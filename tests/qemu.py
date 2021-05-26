#!/usr/bin/env python3

import json
import os
import re
import socket
import subprocess
import time
import functools
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path
from queue import Queue
from shlex import quote
from tempfile import TemporaryDirectory
from typing import Any, Dict, Iterator, List, Optional

from root import TEST_ROOT, PROJECT_ROOT


@dataclass
class VmImage:
    kernel: Path
    ## kernel headers & buildsystem
    kerneldir: Path
    squashfs: Path
    initial_ramdisk: Path
    kernel_params: List[str]


@functools.lru_cache(maxsize=None)
def nix_build(what: str) -> Any:
    result = subprocess.run(
        ["nix", "build", "--json", what],
        text=True,
        stdout=subprocess.PIPE,
        check=True,
        cwd=PROJECT_ROOT,
    )
    return json.loads(result.stdout)


def notos_image() -> VmImage:
    data = nix_build(".#not-os-image.json")
    with open(data[0]["outputs"]["out"]) as f:
        data = json.load(f)
        return VmImage(
            kernel=Path(data["kernel"]),
            kerneldir=Path(data["kerneldir"]),
            squashfs=Path(data["squashfs"]),
            initial_ramdisk=Path(data["initialRamdisk"]),
            kernel_params=data["kernelParams"],
        )


class QmpSession:
    def __init__(self, sock: socket.socket) -> None:
        self.sock = sock
        self.pending_events: Queue[Dict[str, Any]] = Queue()
        self.reader = sock.makefile("r")
        self.writer = sock.makefile("w")
        hello = self._result()
        assert "QMP" in hello, f"Unexpected result: {hello}"
        self.send("qmp_capabilities")

    def _readmsg(self) -> Dict[str, Any]:
        line = self.reader.readline()
        return json.loads(line)

    def _raise_unexpected_msg(self, msg: Dict[str, Any]) -> None:
        m = json.dumps(msg, sort_keys=True, indent=4)
        raise RuntimeError(f"Got unexpected qmp response: {m}")

    def _result(self) -> Dict[str, Any]:
        while True:
            # QMP is in the handshake
            res = self._readmsg()
            if "return" in res or "QMP" in res:
                return res
            elif "event" in res:
                self.pending_events.put(res)
                continue
            else:
                self._raise_unexpected_msg(res)

    def events(self) -> Iterator[Dict[str, Any]]:
        while not self.pending_events.empty():
            yield self.pending_events.get()

        res = self._readmsg()

        if "event" not in res:
            self._raise_unexpected_msg(res)
        yield res

    def send(self, cmd: str, args: Dict[str, str] = {}) -> Dict[str, str]:
        data: Dict[str, Any] = dict(execute=cmd)
        if args != {}:
            data["arguments"] = args

        json.dump(data, self.writer)
        self.writer.write("\n")
        self.writer.flush()
        return self._result()


def is_port_open(ip: str, port: int, wait_response: bool = False) -> bool:
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    try:
        s.connect((ip, int(port)))
        if wait_response:
            s.recv(1)
        s.shutdown(2)
        return True
    except Exception:
        return False


@contextmanager
def connect_qmp(path: Path) -> Iterator[QmpSession]:
    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    sock.connect(str(path))

    try:
        yield QmpSession(sock)
    finally:
        sock.close()


def parse_regs(qemu_output: str) -> Dict[str, int]:
    regs = {}
    for match in re.finditer(r"(\S+)\s*=\s*([0-9a-f ]+)", qemu_output):
        name = match.group(1)
        content = match.group(2).replace(" ", "")
        regs[name.lower()] = int(content, 16)
    return regs


def get_ssh_port(session: QmpSession) -> int:
    usernet_info = session.send(
        "human-monitor-command", args={"command-line": "info usernet"}
    )
    ssh_port = None
    for l in usernet_info["return"].splitlines():
        fields = l.split()
        if "TCP[HOST_FORWARD]" in fields and "22" in fields:
            ssh_port = int(l.split()[3])
    assert ssh_port is not None
    return ssh_port


class QemuVm:
    def __init__(self, qmp_session: QmpSession, tmux_session: str, pid: int) -> None:
        self.qmp_session = qmp_session
        self.tmux_session = tmux_session
        self.pid = pid
        self.ssh_port = get_ssh_port(qmp_session)

    def events(self) -> Iterator[Dict[str, Any]]:
        return self.qmp_session.events()

    def wait_for_ssh(self) -> None:
        """
        Block until ssh port is accessible
        """
        print(f"wait for ssh on {self.ssh_port}")
        while True:
            if (
                self.ssh_cmd(
                    ["echo", "ok"], check=False, stderr=subprocess.DEVNULL
                ).returncode
                == 0
            ):
                break
            time.sleep(0.1)

    def ssh_cmd(
        self,
        argv: List[str],
        check: bool = True,
        stdout: Optional[int] = subprocess.PIPE,
        stderr: Optional[int] = None,
        text: bool = True,
    ) -> subprocess.CompletedProcess:
        """
        @return: CompletedProcess.stderr/stdout contains output of `cmd` which
        is run in the vm via ssh.
        """
        cmd = " ".join(map(quote, argv))
        key_path = TEST_ROOT.joinpath("..", "nix", "ssh_key")
        key_path.chmod(0o400)
        return subprocess.run(
            [
                "ssh",
                "-i",
                str(key_path),
                "-p",
                str(self.ssh_port),
                "-oBatchMode=yes",
                "-oStrictHostKeyChecking=no",
                "-oConnectTimeout=5",
                "-oUserKnownHostsFile=/dev/null",
                "root@127.0.1",
                cmd,
            ],
            stdout=stdout,
            stderr=stderr,
            check=check,
            text=text,
        )

    def regs(self) -> Dict[str, int]:
        """
        Get cpu register:
        TODO: add support for multiple cpus
        """
        res = self.send(
            "human-monitor-command", args={"command-line": "info registers"}
        )
        return parse_regs(res["return"])

    def dump_physical_memory(self, addr: int, num_bytes: int) -> bytes:
        res = self.send(
            "human-monitor-command",
            args={"command-line": f"xp/{num_bytes}bx 0x{addr:x}"},
        )
        hexval = "".join(
            m.group(1) for m in re.finditer("0x([0-9a-f]{2})", res["return"])
        )
        return bytes.fromhex(hexval)

    def attach(self) -> None:
        """
        Attach to qemu session via tmux. This is useful for debugging
        """
        subprocess.run(["tmux", "-L", self.tmux_session, "attach"])

    def send(self, cmd: str, args: Dict[str, str] = {}) -> Dict[str, str]:
        """
        Send a Qmp command (https://wiki.qemu.org/Documentation/QMP)
        """
        return self.qmp_session.send(cmd, args)


def qemu_command(image: VmImage, qmp_socket: Path, ssh_port: int = 0) -> List:
    """
    @image VM image to boot
    @qmp_socket unixsocket path of the QEMU qmp control socket
    @ssh_port host port bound to vm guest port (0 means dynamic port)
    """
    params = " ".join(image.kernel_params)
    return [
        "qemu-kvm",
        "-name",
        "test-os",
        "-m",
        "512",
        "-drive",
        f"id=drive1,file={image.squashfs},readonly=on,media=cdrom,format=raw,if=none",
        "-device",
        "virtio-blk-pci,drive=drive1,bootindex=1",
        "-kernel",
        f"{image.kernel}/bzImage",
        "-initrd",
        f"{image.initial_ramdisk}/initrd",
        "-nographic",
        "-append",
        f"console=ttyS0 {params} quiet panic=-1",
        "-netdev",
        f"user,id=n1,hostfwd=tcp:127.0.0.1:{ssh_port}-:22",
        "-device",
        "virtio-net-pci,netdev=n1",
        "-virtfs",
        f"local,path={PROJECT_ROOT},security_model=none,mount_tag=vmsh",
        "-qmp",
        f"unix:{str(qmp_socket)},server,nowait",
        "-no-reboot",
        "-device",
        "virtio-rng-pci",
    ]


@contextmanager
def spawn_qemu(image: VmImage, extra_args: List[str] = []) -> Iterator[QemuVm]:
    with TemporaryDirectory() as tempdir:
        qmp_socket = Path(tempdir).joinpath("qmp.sock")
        cmd = qemu_command(image, qmp_socket)
        cmd += extra_args

        tmux_session = f"pytest-{os.getpid()}"
        tmux = [
            "tmux",
            "-L",
            tmux_session,
            "new-session",
            "-d",
            " ".join(map(quote, cmd)),
        ]
        print("$ " + " ".join(map(quote, tmux)))
        subprocess.run(tmux, check=True)
        try:
            proc = subprocess.run(
                [
                    "tmux",
                    "-L",
                    tmux_session,
                    "list-panes",
                    "-a",
                    "-F",
                    "#{pane_pid}",
                ],
                stdout=subprocess.PIPE,
                check=True,
            )
            qemu_pid = int(proc.stdout)
            while not qmp_socket.exists():
                try:
                    os.kill(qemu_pid, 0)
                    time.sleep(0.1)
                except ProcessLookupError:
                    raise Exception("qemu vm was terminated")
            with connect_qmp(qmp_socket) as session:
                yield QemuVm(session, tmux_session, qemu_pid)
        finally:
            subprocess.run(["tmux", "-L", tmux_session, "kill-server"])
