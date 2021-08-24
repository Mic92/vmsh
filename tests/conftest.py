#!/usr/bin/env python3

import contextlib
import sys
from pathlib import Path
from typing import List, Type

import pytest
from qemu import QemuVm, VmImage, spawn_qemu
from nix import notos_image, busybox_image
from root import TEST_ROOT
from vmsh import spawn_vmsh_command, VmshPopen

sys.path.append(str(TEST_ROOT.parent))


class Helpers:
    @staticmethod
    def root() -> Path:
        return TEST_ROOT

    @staticmethod
    def notos_image() -> VmImage:
        return notos_image()

    @staticmethod
    def busybox_image() -> "contextlib._GeneratorContextManager[Path]":
        return busybox_image()

    @staticmethod
    def spawn_vmsh_command(
        args: List[str], cargo_executable: str = "vmsh"
    ) -> VmshPopen:
        return spawn_vmsh_command(args, cargo_executable)

    @staticmethod
    def run_vmsh_command(args: List[str], cargo_executable: str = "vmsh") -> VmshPopen:
        proc = spawn_vmsh_command(args, cargo_executable)
        assert proc.wait() == 0
        return proc

    @staticmethod
    def spawn_qemu(
        image: VmImage, extra_args: List[str] = []
    ) -> "contextlib._GeneratorContextManager[QemuVm]":
        return spawn_qemu(image, extra_args)


@pytest.fixture
def helpers() -> Type[Helpers]:
    return Helpers
