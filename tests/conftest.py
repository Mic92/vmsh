#!/usr/bin/env python3

import sys
import os
from pathlib import Path
import socket
import time
from dataclasses import dataclass
import subprocess
import json
from typing import Any, Dict, Iterator, List, Type
from shlex import quote
from tempfile import TemporaryDirectory
from contextlib import contextmanager
import contextlib

import pytest

TEST_ROOT = Path(__file__).parent.resolve()
sys.path.append(str(TEST_ROOT.parent))


@dataclass
class VmImage:
    kernel: Path
    squashfs: Path
    initial_ramdisk: Path
    kernel_params: List[str]


def rootfs_image(image: Path) -> VmImage:
    result = subprocess.run(
        ["nix-build", str(image), "-A", "json"],
        text=True,
        stdout=subprocess.PIPE,
        check=True,
    )
    with open(result.stdout.strip("\n")) as f:
        data = json.load(f)
        return VmImage(
            kernel=Path(data["kernel"]),
            squashfs=Path(data["squashfs"]),
            initial_ramdisk=Path(data["initialRamdisk"]),
            kernel_params=data["kernelParams"],
        )


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
    def __init__(self, tmux_session: str, pid: int, ssh_port: int) -> None:
        self.tmux_session = tmux_session
        self.pid = pid
        self.ssh_port = ssh_port

    def attach(self) -> None:
        """
        Attach to qemu session via tmux. This is useful for debugging
        """
        subprocess.run(["tmux", "-L", self.tmux_session, "attach"])


@contextmanager
def spawn_qemu(image: VmImage) -> Iterator[QemuVm]:
    with TemporaryDirectory() as tempdir:
        qmp_socket = Path(tempdir).joinpath("qmp.sock")
        params = " ".join(image.kernel_params)
        cmd = [
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
            yield QemuVm(tmux_session, qemu_pid, ssh_port)
        finally:
            subprocess.run(["tmux", "-L", tmux_session, "kill-server"])


@contextmanager
def ensure_debugfs_access() -> Iterator[None]:
    uid = os.getuid()
    if os.stat("/sys/kernel/debug/").st_uid != uid:
        subprocess.run(
            ["sudo", "chown", "-R", str(uid), "/sys/kernel/debug"], check=True
        )
        try:
            yield
        finally:
            subprocess.run(["sudo", "chown", "-R", "0", "/sys/kernel/debug"])
    else:
        yield


# cargo_options: additional args to pass to cargo. Example: "--bin test_ioctls"
# to run a non-default binary
def run_vmsh_command(args: List[str], cargo_options: List[str] = []) -> None:
    if not os.path.isdir("/sys/module/kheaders"):
        subprocess.run(["sudo", "modprobe", "kheaders"])
    uid = os.getuid()
    gid = os.getuid()
    groups = ",".join(map(str, os.getgroups()))
    with ensure_debugfs_access():
        cargoArgs = [
            "cargo",
            "run",
        ]
        cargoArgs += cargo_options
        cargoArgs += ["--"]
        cargoArgs += args
        cargoCmd = " ".join(map(quote, cargoArgs))

        cmd = [
            "sudo",
            "-E",
            "capsh",
            "--caps=cap_sys_ptrace,cap_sys_admin,cap_sys_resource+epi cap_setpcap,cap_setuid,cap_setgid+ep",
            "--keep=1",
            f"--groups={groups}",
            f"--gid={gid}",
            f"--uid={uid}",
            "--addamb=cap_sys_resource",
            "--addamb=cap_sys_admin",
            "--addamb=cap_sys_ptrace",
            "--",
            "-c",
            cargoCmd,
        ]
        print("$ " + " ".join(map(quote, cmd)))
        subprocess.run(cmd, check=True)


class Helpers:
    @staticmethod
    def root() -> Path:
        return TEST_ROOT

    @staticmethod
    def notos_image() -> VmImage:
        return rootfs_image(TEST_ROOT.joinpath("../nix/not-os-image.nix"))

    @staticmethod
    def run_vmsh_command(args: List[str], cargo_options: List[str] = []) -> None:
        return run_vmsh_command(args, cargo_options)

    @staticmethod
    def spawn_qemu(image: VmImage) -> "contextlib._GeneratorContextManager[QemuVm]":
        return spawn_qemu(image)


@pytest.fixture
def helpers() -> Type[Helpers]:
    return Helpers
