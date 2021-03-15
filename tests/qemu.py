#!/usr/bin/env python3

import json
import os
import socket
import subprocess
import time
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path
from shlex import quote
from tempfile import TemporaryDirectory
from typing import Any, Dict, Iterator, List


@dataclass
class VmImage:
    kernel: Path
    squashfs: Path
    initial_ramdisk: Path
    kernel_params: List[str]


class QmpSession:
    def __init__(self, sock: socket.socket) -> None:
        self.sock = sock
        self.reader = sock.makefile("r")
        self.writer = sock.makefile("w")
        hello = self._result()
        assert "QMP" in hello, f"Unexpected result: {hello}"
        self.send("qmp_capabilities")

    def _result(self) -> Dict[str, Any]:
        while True:
            line = self.reader.readline()
            res = json.loads(line)
            # QMP is in the handshake
            if "return" in res or "QMP" in res:
                return res
            elif "event" in res or "QMP" in res:
                continue
            else:
                raise RuntimeError(f"Got unexpected qmp response: {line}")

    def send(self, cmd: str, args: Dict[str, str] = {}) -> Dict[str, str]:
        data: Dict[str, Any] = dict(execute=cmd)
        if args != {}:
            data["arguments"] = args

        json.dump(data, self.writer)
        self.writer.write("\n")
        self.writer.flush()
        return self._result()


@contextmanager
def connect_qmp(path: Path) -> Iterator[QmpSession]:
    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    sock.connect(str(path))

    try:
        yield QmpSession(sock)
    finally:
        sock.close()


class QemuVm:
    def __init__(
        self, qmp_session: QmpSession, tmux_session: str, pid: int, ssh_port: int
    ) -> None:
        self.qmp_session = qmp_session
        self.tmux_session = tmux_session
        self.pid = pid
        self.ssh_port = ssh_port

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


def qemu_command(image: VmImage, qmp_socket: Path) -> List:
    params = " ".join(image.kernel_params)
    return [
        "qemu-kvm",
        "-name",
        "test-os",
        "-m",
        "512",
        "-drive",
        f"index=0,id=drive1,file={image.squashfs},readonly,media=cdrom,format=raw,if=virtio",
        "-kernel",
        f"{image.kernel}/bzImage",
        "-initrd",
        f"{image.initial_ramdisk}/initrd",
        "-nographic",
        "-append",
        f"console=ttyS0 {params} quiet panic=-1",
        "-netdev",
        "user,id=n1,hostfwd=tcp:127.0.0.1:0-:22",
        "-device",
        "virtio-net-pci,netdev=n1",
        "-qmp",
        f"unix:{str(qmp_socket)},server,nowait",
        "-no-reboot",
        "-device",
        "virtio-rng-pci",
    ]


@contextmanager
def spawn_qemu(image: VmImage) -> Iterator[QemuVm]:
    with TemporaryDirectory() as tempdir:
        qmp_socket = Path(tempdir).joinpath("qmp.sock")
        cmd = qemu_command(image, qmp_socket)

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
                usernet_info = session.send(
                    "human-monitor-command", args={"command-line": "info usernet"}
                )
                ssh_port = None
                for l in usernet_info["return"].splitlines():
                    fields = l.split()
                    if "TCP[HOST_FORWARD]" in fields and "22" in fields:
                        ssh_port = int(l.split()[3])
                assert ssh_port is not None
                yield QemuVm(session, tmux_session, qemu_pid, ssh_port)
        finally:
            subprocess.run(["tmux", "-L", tmux_session, "kill-server"])