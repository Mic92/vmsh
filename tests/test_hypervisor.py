import os
import subprocess
import time
from contextlib import contextmanager
from typing import Iterator, Optional, Tuple

import conftest

from nix import nix_build, passwd_image


def find_hypervisor_by_tty(tty: str, command: str) -> Optional[int]:
    command_bytes = command.encode("utf-8")
    for fname in os.listdir("/proc"):
        try:
            pid = int(fname)
        except ValueError:
            continue
        try:
            target = os.readlink(f"/proc/{pid}/fd/1")
            if target == tty:
                with open(f"/proc/{pid}/cmdline", "rb") as f:
                    argv0 = f.read().split(b"\0")[0]
                    if os.path.basename(argv0) == command_bytes:
                        return pid
        except OSError:  # Permission error/ENOENT
            pass
    return None


def tmux_logs(tmux_session: str) -> str:
    p = subprocess.run(
        ["tmux", "-L", tmux_session, "capture-pane", ";", "show-buffer"],
        text=True,
        stdout=subprocess.PIPE,
        check=False,
    )
    return p.stdout


@contextmanager
def run_hypervisor(flake_name: str, command: str) -> Iterator[Tuple[int, str]]:
    executable = nix_build(flake_name)[0]["outputs"]["out"]
    tmux_session = f"pytest-{os.getpid()}"
    # only in tmux the serial works, don't now why ptys did not work
    tmux = [
        "tmux",
        "-L",
        tmux_session,
        "new-session",
        "-d",
        f"{executable}/bin/microvm-run",
    ]
    subprocess.run(tmux, check=True)
    pid = None
    try:
        proc = subprocess.run(
            [
                "tmux",
                "-L",
                tmux_session,
                "list-panes",
                "-a",
                "-F",
                "#{pane_tty}",
            ],
            stdout=subprocess.PIPE,
            check=True,
            text=True,
        )
        tty = proc.stdout.strip()
        for i in range(20):
            pid = find_hypervisor_by_tty(tty, command)
            if pid is not None:
                break
            time.sleep(1)
        else:
            try:
                print(tmux_logs(tmux_session))
            except OSError:
                pass
            breakpoint()
            raise Exception(f"timeout: process {command} not found/started")

        yield (pid, tmux_session)
    finally:
        subprocess.run(["tmux", "-L", tmux_session, "kill-server"])
        while True:
            try:
                if pid:
                    os.kill(pid, 0)
                else:
                    break
            except ProcessLookupError:
                break
            else:
                print(f"waiting for {command} to stop")
                time.sleep(1)


EOF = 1


def wait_for_tmux_output(tmux_session: str, needle: str) -> None:
    # this is super ugly, but could not find a better way :(
    output = ""
    for i in range(60):
        output = tmux_logs(tmux_session)
        if needle in output:
            break
        time.sleep(1)
    else:
        print(output)
        assert False, "Machine takes too long to boot"


def hypervisor_test(helpers: conftest.Helpers, flake_name: str, command: str) -> None:
    print(f"test {command}")
    with run_hypervisor(flake_name, command) as (
        pid,
        tmux_session,
    ), helpers.busybox_image() as img:
        wait_for_tmux_output(tmux_session, "Welcome to NixOS")

        vmsh = helpers.spawn_vmsh_command(
            [
                "attach",
                "--backing-file",
                str(img),
                str(pid),
                "--",
                "/bin/sh",
                "-c",
                "echo works",
            ]
        )

        with vmsh:
            try:
                vmsh.wait_until_line(
                    "works",
                    lambda l: "works" in l,
                )
            except Exception:
                print(tmux_logs(tmux_session))
                raise


def test_firecracker(helpers: conftest.Helpers) -> None:
    hypervisor_test(helpers, ".#firecracker-example", "firecracker")


def test_crosvm(helpers: conftest.Helpers) -> None:
    hypervisor_test(helpers, ".#crosvm-example", "crosvm")


def test_kvmtool(helpers: conftest.Helpers) -> None:
    hypervisor_test(helpers, ".#kvmtool-example", "lkvm")


# also tests qemu
def test_qemu_and_change_password(helpers: conftest.Helpers) -> None:
    # XXX not portable name
    with run_hypervisor(".#qemu-example", "qemu-system-x86_64") as (
        pid,
        tmux_session,
    ), passwd_image() as img:
        wait_for_tmux_output(tmux_session, "Welcome to NixOS")

        vmsh = helpers.spawn_vmsh_command(
            [
                "attach",
                "--backing-file",
                str(img),
                str(pid),
                "--",
                "/bin/sh",
                "-c",
                "echo 'root:foo' | /bin/chpasswd -R /var/lib/vmsh",
            ]
        )

        with vmsh:
            try:
                vmsh.wait_until_line(
                    "process finished with exit status: 0",
                    lambda l: "process finished with exit status: 0" in l,
                )
            except Exception:
                print(tmux_logs(tmux_session))
                raise
