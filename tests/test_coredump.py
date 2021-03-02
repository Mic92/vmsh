#!/usr/bin/env python3

import subprocess
import os
import socket
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
        ["nix-build", str(image), "-A", "json"], text=True, stdout=subprocess.PIPE, check=True
    )
    with open(result.stdout.strip("\n")) as f:
        data = json.load(f)
        return Vm(
            kernel=Path(data["kernel"]),
            squashfs=Path(data["squashfs"]),
            initial_ramdisk=Path(data["initialRamdisk"]),
            kernel_params=data["kernelParams"],
        )

def is_open(ip,port):
   s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
   try:
      s.connect((ip, int(port)))
      s.shutdown(2)
      return True
   except:
      return False

class Qemu:
    def __init__(self, vm: Vm):
        self.vm = vm

    def attach(self):
        """
        Attach to qemu session via tmux. This is useful for debugging
        """
        subprocess.run(["tmux", "-L", self.tmux_session, "attach"])

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
            "-netdev", "user,id=n1,hostfwd=tcp:127.0.0.1:0-:22",
            "-device", "virtio-net-pci,netdev=n1",
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


def test_coredump(helpers: conftest.Helpers) -> None:
    vm = rootfs_image(helpers.root().joinpath("../nix/not-os-image.nix"))
    qemu = Qemu(vm)
    with qemu as pid:
        p = subprocess.run(["cargo", "run", "--", "coredump", str(pid)])
        pass
