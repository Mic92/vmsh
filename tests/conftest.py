#!/usr/bin/env python3

import contextlib
import json
import os
import subprocess
import sys
from contextlib import contextmanager
from pathlib import Path
from shlex import quote
from typing import Iterator, List, Type, Optional, Any

import pytest
from qemu import QemuVm, VmImage, spawn_qemu
from root import TEST_ROOT, PROJECT_ROOT

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


def print_stdout_with_prefix(proc: subprocess.Popen, prefix: str) -> None:
    print("vmsh: ", end="", flush=True)
    while True:
        assert proc.stdout is not None
        res = proc.stdout.read(1)

        if len(res) > 0:
            print(f"{res}", end="", flush=True)
            if res == "\n":
                print(prefix, end="", flush=True)
        else:
            print("", flush=True)
            return


class VmshPopen(subprocess.Popen):
    def __exit__(self, exc_type: Any, exc_value: Any, traceback: Any) -> None:
        # we cannot kill sudo, but we can stop vmsh as it drops privileges to our user
        subprocess.run(["pkill", "--parent", str(self.pid)])
        print_stdout_with_prefix(self, "vmsh: ")
        super().__exit__(exc_type, exc_value, traceback)

    def print_stdout_until(self: subprocess.Popen, until_line: Optional[str]) -> None:
        """
        blocks until until_line is printed
        @param until_line example: "pause\n"
        """
        while True:
            assert self.stdout is not None
            line = self.stdout.readline()

            if len(line) > 0:
                print(f"vmsh: {line}", end="", flush=True)
                if line == until_line:
                    print("print_stdout_until fulfilled")
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
        return VmshPopen(cmd, stdout=subprocess.PIPE, text=True)


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
