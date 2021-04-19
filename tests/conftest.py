#!/usr/bin/env python3

import contextlib
import json
import os
import subprocess
import sys
import threading
from contextlib import contextmanager
from pathlib import Path
from queue import Queue
from shlex import quote
from typing import Any, Iterator, List, Type, Union

import pytest
from qemu import QemuVm, VmImage, spawn_qemu
from root import PROJECT_ROOT, TEST_ROOT

sys.path.append(str(TEST_ROOT.parent))

build_artifacts = Path("/dev/null")  # folder with cargo-built executables


def cargo_build() -> Path:
    subprocess.run(["cargo", "build"], cwd=PROJECT_ROOT)
    return PROJECT_ROOT.joinpath("target", "debug")


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


EOF = 1


class VmshPopen(subprocess.Popen):
    def process_stdout(self) -> None:
        self.lines: Queue[Union[str, int]] = Queue()
        threading.Thread(target=self.print_stdout_with_prefix).start()

    def __exit__(self, exc_type: Any, exc_value: Any, traceback: Any) -> None:
        # we cannot kill sudo, but we can stop vmsh as it drops privileges to our user
        subprocess.run(["pkill", "--parent", str(self.pid)])
        super().__exit__(exc_type, exc_value, traceback)

    def print_stdout_with_prefix(self) -> None:
        buf = ""
        while True:
            assert self.stdout is not None
            res = self.stdout.read(1)

            if len(res) > 0:
                if res == "\n":
                    print(f"vmsh[{self.pid}] {buf}")
                    self.lines.put(buf)
                    buf = ""
                else:
                    buf += res
            else:
                if buf != "":
                    print(f"vmsh[{self.pid}] {buf}", flush=True)
                self.lines.put(EOF)
                return

    def wait_until_line(self, line: str) -> None:
        """
        blocks until line is printed
        @param line example: "pause\n"
        """
        print(f"wait for '{line}'...")
        while True:
            l = self.lines.get()

            if l == EOF:
                raise Exception("reach end of stdout output before process finished")

            if l == line:
                return


def spawn_vmsh_command(
    args: List[str], cargo_executable: str = "vmsh", stdout: Any = None
) -> VmshPopen:
    if not os.path.isdir("/sys/module/kheaders"):
        subprocess.run(["sudo", "modprobe", "kheaders"])
    uid = os.getuid()
    gid = os.getuid()
    groups = ",".join(map(str, os.getgroups()))
    with ensure_debugfs_access():
        cmd = [str(os.path.join(build_artifacts, cargo_executable))]
        cmd += args
        cmd_quoted = " ".join(map(quote, cmd))

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
            cmd_quoted,
        ]
        print("$ " + " ".join(map(quote, cmd)))
        p = VmshPopen(cmd, stdout=subprocess.PIPE, text=True)
        p.process_stdout()
        return p


class Helpers:
    @staticmethod
    def root() -> Path:
        return TEST_ROOT

    @staticmethod
    def notos_image() -> VmImage:
        return rootfs_image(TEST_ROOT.joinpath("../nix/not-os-image.nix"))

    @staticmethod
    def spawn_vmsh_command(
        args: List[str], cargo_executable: str = "vmsh", stdout: Any = None
    ) -> VmshPopen:
        return spawn_vmsh_command(args, cargo_executable, stdout=stdout)

    @staticmethod
    def run_vmsh_command(args: List[str], cargo_executable: str = "vmsh") -> None:
        proc = spawn_vmsh_command(args, cargo_executable)
        assert proc.wait() == 0

    @staticmethod
    def spawn_qemu(image: VmImage) -> "contextlib._GeneratorContextManager[QemuVm]":
        return spawn_qemu(image)


@pytest.fixture
def helpers() -> Type[Helpers]:
    return Helpers


build_artifacts = cargo_build()
