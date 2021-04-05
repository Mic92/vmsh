#!/usr/bin/env python3

import contextlib
import json
import os
import subprocess
import sys
from contextlib import contextmanager
from pathlib import Path
from shlex import quote
from typing import Iterator, List, Type

import pytest
from qemu import QemuVm, VmImage, spawn_qemu
from root import TEST_ROOT

sys.path.append(str(TEST_ROOT.parent))

build_artifacts = Path("/dev/null")  # folder with cargo-built executables


def cargo_build() -> Path:
    subprocess.run(["cargo", "build"])
    return Path("target/debug")


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


def run_vmsh_command(args: List[str], cargo_executable: str = "vmsh") -> None:
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
        subprocess.run(cmd, check=True)


class Helpers:
    @staticmethod
    def root() -> Path:
        return TEST_ROOT

    @staticmethod
    def notos_image() -> VmImage:
        return rootfs_image(TEST_ROOT.joinpath("../nix/not-os-image.nix"))

    @staticmethod
    def run_vmsh_command(args: List[str], cargo_executable: str = "vmsh") -> None:
        return run_vmsh_command(args, cargo_executable=cargo_executable)

    @staticmethod
    def spawn_qemu(image: VmImage) -> "contextlib._GeneratorContextManager[QemuVm]":
        return spawn_qemu(image)


@pytest.fixture
def helpers() -> Type[Helpers]:
    return Helpers


build_artifacts = cargo_build()
