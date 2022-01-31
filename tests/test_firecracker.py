import subprocess
import conftest

from threading import Thread

from typing import Iterator, Union, IO, Callable, Optional, Tuple, Any
from pathlib import Path

import os
import sys
from shlex import quote
import termios
import pty
import json
from queue import Queue
from tempfile import NamedTemporaryFile
from contextlib import contextmanager
from nix import nix_build


# {
#  "boot-source": {
#    "kernel_image_path": "Image",
#    "initrd_path": "initramfs.img.lz4",
#    "boot_args": "console=ttyS0 reboot=k panic=-1 pci=off"
#  },
#  "drives": [],
#  "machine-config": {
#    "vcpu_count": 1,
#    "mem_size_mib": 512,
#    "ht_enabled": false
#  }
# }

def set_tty_raw(fd: int) -> None:
    new = termios.tcgetattr(fd)
    new[3] = new[3] & ~termios.ECHO
    termios.tcsetattr(fd, termios.TCSADRAIN, new)

@contextmanager
def run_firecracker() -> Iterator[Tuple[subprocess.Popen[str], IO[str]]]:
    image = Path(nix_build(".#alpine-image")[0]["outputs"]["out"])
    with NamedTemporaryFile(mode="w") as f:
        json.dump(
            {
                "boot-source": {
                    "kernel_image_path": str(image.joinpath("Image")),
                    "initrd_path": str(image.joinpath("initramfs.img.lz4")),
                    "boot_args": "console=ttyS0 reboot=k panic=-1 pci=off",
                },
                "drives": [],
                "machine-config": {
                    "vcpu_count": 1,
                    "mem_size_mib": 512,
                    "ht_enabled": False,
                },
            },
            f,
        )
        f.flush()
        firecracker = [
            "tmux",
            "-c", f"firecracker --no-api --config-file {str(f.name)} --no-seccomp"
        ]
        tmux_session = f"pytest-{os.getpid()}"
        tmux = [
            "tmux",
            "-L",
            tmux_session,
            "new-session",
            "-d",
            " ".join(map(quote, firecracker)),
        ]

        with subprocess.Popen(
                firecracker, text=True, stdout=subprocess.PIPE
        ) as p:
            master_file = None
            try:
                #master_file = os.fdopen(master, "r")
                #yield (p, master_file)
                yield (p, p.stdout)
            finally:
                try:
                    if master_file is not None:
                        master_file.close()
                finally:
                    p.kill()


EOF = 1


class ProcessStdout:
    def __init__(self, proc: subprocess.Popen[str], output: IO[str]) -> None:
        self.proc = proc
        self.lines: Queue[Union[str, int]] = Queue()
        Thread(target=self.print_output, args=(output, )).start()

    def print_output(self, output: IO[str]) -> None:
        buf = ""
        while True:
            res = output.read(1)
            if len(res) > 0:
                if "\n" in res:
                    print(f"[{self.proc.pid}] {buf}")
                    self.lines.put(buf)
                    buf = ""
                else:
                    buf += res
            else:
                if buf != "":
                    print(f"[{self.proc.pid}] {buf}", flush=True)
                self.lines.put(EOF)
                return

    def wait_until_line(self, tag: str, condition: Optional[Callable[[str], bool]] = None) -> None:
        """
        blocks until a line matching the given condition is printed
        Example: `vm.wait_until_line(lambda line: line == "foobar")`
        @param tag: printable, human readable tag
        """
        if condition is None:
            condition = lambda l: tag in l
        print(f"wait for '{tag}'...")
        while True:
            l = self.lines.get()

            if l == EOF:
                raise Exception("reach end of stdout output before process finished")

            if condition(str(l)):
                return


def test_attach(
    helpers: conftest.Helpers,
) -> None:
    with run_firecracker() as (proc, master_fd):
        p = ProcessStdout(proc, master_fd)
        p.wait_until_line("finished booting")
        breakpoint()
    # with helpers.busybox_image() as img, helpers.spawn_qemu(helpers.notos_image()) as vm:
    # vm.wait_for_ssh()
    # vmsh = helpers.spawn_vmsh_command(
    #     [
    #         "attach",
    #         "--backing-file",
    #         str(img),
    #         str(vm.pid),
    #         "--",
    #         "/bin/sh",
    #         "-c",
    #         "echo works",
    #     ]
    # )

    # with vmsh:
    #     try:
    #         vmsh.wait_until_line(
    #             "stage1 driver started",
    #             lambda l: "stage1 driver started" in l,
    #         )
    #     finally:
    #         res = vm.ssh_cmd(["dmesg"], check=False)
    #         print("stdout:\n", res.stdout)
    #     assert res.returncode == 0

    #     assert (
    #         res.stdout.find(
    #             "EXT4-fs (vdb): mounted filesystem with ordered data mode"
    #         )
    #         > 0
    #     )
    # try:
    #     os.kill(vmsh.pid, 0)
    # except ProcessLookupError:
    #     pass
    # else:
    #     assert False, "vmsh was not terminated properly"

    # # See that the VM is still alive after detaching
    # res = vm.ssh_cmd(["echo", "ping"], check=False)
    # assert res.stdout == "ping\n"
    # assert res.returncode == 0
