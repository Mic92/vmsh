#!/usr/bin/env python3

import subprocess
import os
import json
from shlex import quote
from pathlib import Path
from typing import List, Any
from dataclasses import dataclass

import conftest


@dataclass
class Vm:
    kernel: Path
    squashfs: Path
    initial_ramdisk: Path
    kernel_params: List[str]


def rootfs_image(image: Path):
    result = subprocess.run(
        ["nix-build", str(image), "-A", "json"], text=True, stdout=subprocess.PIPE
    )
    with open(result.stdout.strip("\n")) as f:
        data = json.load(f)
        return Vm(
            kernel=Path(data["kernel"]),
            squashfs=Path(data["squashfs"]),
            initial_ramdisk=Path(data["initialRamdisk"]),
            kernel_params=data["kernelParams"],
        )


class Qemu:
    def __init__(self, vm: Vm):
        self.vm = vm

    def __enter__(self) -> int:
        params = " ".join(self.vm.kernel_params)
        cmd = [
            "qemu-kvm",
            "-name",
            "test-os",
            "-m",
            "512",
            "-drive",
            f"index=0,id=drive1,file={self.vm.squashfs},readonly,media=cdrom,format=raw,if=virtio",
            "-kernel",
            f"{self.vm.kernel}/bzImage",
            "-initrd",
            f"{self.vm.initial_ramdisk}/initrd",
            "-nographic",
            "-append",
            f"console=ttyS0 {params} quiet panic=-1",
            "-no-reboot",
            "-device",
            "virtio-rng-pci",
        ]

        self.tmux_session = f"pytest-{os.getpid()}"
        tmux = ["tmux",  "-L", self.tmux_session, "new-session", "-d", " ".join(map(quote, cmd))]
        print("$ " + " ".join(map(quote, tmux)))
        subprocess.run(tmux, check=True)
        proc = subprocess.run(["tmux", "-L", self.tmux_session, "list-panes", "-a", "-F", "#{pane_pid}"], stdout=subprocess.PIPE, check=True)
        return int(proc.stdout)

    def __exit__(self, type: Any, value: Any, traceback: Any) -> None:
        subprocess.run(["tmux", "-L", self.tmux_session, "kill-server"])


def test_update(helpers: conftest.Helpers) -> None:
    vm = rootfs_image(helpers.root().joinpath("../nix/not-os-image.nix"))
    qemu = Qemu(vm)
    with qemu as pid:
        pass
