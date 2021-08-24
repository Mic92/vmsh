#!/usr/bin/env python3

import contextlib
import sys
from datetime import datetime
from pathlib import Path
from typing import List, Type, Optional

import pytest
from qemu import QemuVm, VmImage, spawn_qemu
from nix import notos_image, busybox_image
from root import TEST_ROOT
from vmsh import spawn_vmsh_command, VmshPopen

sys.path.append(str(TEST_ROOT.parent))


NOW = datetime.now().strftime("%Y%m%d-%H%M%S")

# passed to numactl, starts with 0
CORES_VMSH = "1-3"
CORES_QEMU = "4-7"


class Helpers:
    @staticmethod
    def root() -> Path:
        return TEST_ROOT

    @staticmethod
    def notos_image() -> VmImage:
        return notos_image(nix=".#measurement-image.json")

    @staticmethod
    def busybox_image() -> "contextlib._GeneratorContextManager[Path]":
        # return busybox_image(nix=".#measurement-image")
        return busybox_image()

    @staticmethod
    def spawn_vmsh_command(
        args: List[str], cargo_executable: str = "vmsh"
    ) -> VmshPopen:
        return spawn_vmsh_command(
            args, cargo_executable, target="release", pin_cores=CORES_VMSH
        )

    @staticmethod
    def run_vmsh_command(args: List[str], cargo_executable: str = "vmsh") -> VmshPopen:
        proc = spawn_vmsh_command(
            args, cargo_executable, target="release", pin_cores=CORES_VMSH
        )
        assert proc.wait() == 0
        return proc

    @staticmethod
    def spawn_qemu(
        image: VmImage,
        virtio_blk: Optional[str] = None,
        virtio_9p: Optional[str] = None,
        extra_args: List[str] = [],
    ) -> "contextlib._GeneratorContextManager[QemuVm]":
        extra_args_pre = [
            "numactl",
            "-C",
            CORES_QEMU,
        ]
        extra_args_ = [  # TODO more CPUs
            "-m",
            "3583",
        ]
        extra_args_ += extra_args
        if virtio_blk is not None:
            extra_args_ += [
                "-drive",
                f"id=drive2,file={virtio_blk},format=raw,if=none",
                "-device",
                "virtio-blk-pci,drive=drive2",  # TODO mmio
            ]
        if virtio_9p is not None:
            extra_args_ += [
                "-virtfs",
                f"local,path={virtio_9p},security_model=none,mount_tag=measure9p",
            ]
        return spawn_qemu(image, extra_args_, extra_args_pre)


@pytest.fixture
def helpers() -> Type[Helpers]:
    return Helpers
