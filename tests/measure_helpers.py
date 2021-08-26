import confmeasure
from confmeasure import NOW
from root import MEASURE_RESULTS

import os
import json
from typing import List, Any, Iterator, Dict, DefaultDict, Optional, Text
from collections import defaultdict
from contextlib import contextmanager
import subprocess
import pandas as pd
from pathlib import Path
import time


HOST_SSD = "/dev/nvme0n1"
HOST_DIR = "/mnt/nvme"
GUEST_JAVDEV = "/dev/vdc"
GUEST_QEMUBLK = "/dev/vdb"
GUEST_QEMU9P = "/9p"
GUEST_JAVDEV_MOUNT = "/javdev"
GUEST_QEMUBLK_MOUNT = "/blk"


@contextmanager
def testbench_console(
    helpers: confmeasure.Helpers,
    pts: str,
    guest_cmd: List[str] = [
        "/bin/sh",
        "-c",
        "echo works",
    ],
    mmio: str = "wrap_syscall",
) -> Iterator[Any]:
    with helpers.busybox_image() as img, helpers.spawn_qemu(
        helpers.notos_image(),
    ) as vm:
        vm.wait_for_ssh()
        vmshcmd = [
            "attach",
            "--backing-file",
            str(img),
            "--mmio",
            mmio,
            "--pts",
            pts,
            str(vm.pid),
            "--",
        ]
        vmshcmd += guest_cmd
        vmsh = helpers.spawn_vmsh_command(vmshcmd)

        with vmsh:
            try:
                vmsh.wait_until_line(
                    "stage1 driver started",
                    lambda l: "stage1 driver started" in l,
                )
            finally:
                yield vm

        try:
            os.kill(vmsh.pid, 0)
        except ProcessLookupError:
            pass
        else:
            assert False, "vmsh was not terminated properly"


@contextmanager
def testbench(
    helpers: confmeasure.Helpers,
    with_vmsh: bool = True,
    ioregionfd: bool = False,
    mounts: bool = True,
) -> Iterator[Any]:
    if ioregionfd:
        mmiomode = "ioregionfd"
    else:
        mmiomode = "wrap_syscall"

    if mounts:
        virtio_9p: Optional[str] = HOST_DIR
    else:
        virtio_9p = None

    with helpers.spawn_qemu(
        helpers.notos_image(),
        virtio_blk=HOST_SSD,
        virtio_9p=virtio_9p,
    ) as vm:
        vm.wait_for_ssh()
        if mounts:
            print(vm.ssh_cmd(["mkdir", "-p", GUEST_QEMU9P]).stdout)
            print(
                vm.ssh_cmd(
                    [
                        "mount",
                        "-t",
                        "9p",
                        "-o",
                        "trans=virtio,msize=104857600",
                        "measure9p",
                        GUEST_QEMU9P,
                    ]
                ).stdout
            )
            # print(vm.ssh_cmd(["ls", "-la", GUEST_QEMU9P]).stdout)

            print(vm.ssh_cmd(["mkdir", "-p", GUEST_QEMUBLK_MOUNT]).stdout)
            print(
                vm.ssh_cmd(
                    [
                        "mount",
                        GUEST_QEMUBLK,
                        GUEST_QEMUBLK_MOUNT,
                    ]
                ).stdout
            )

        if not with_vmsh:
            yield vm
        else:
            vmsh = helpers.spawn_vmsh_command(
                [
                    "attach",
                    "--backing-file",
                    str(HOST_SSD),
                    str(vm.pid),
                    "--mmio",
                    mmiomode,
                    "--",
                    "/bin/sh",
                    "-c",
                    "echo works",
                ]
            )

            with vmsh:
                try:
                    vmsh.wait_until_line(
                        "stage1 driver started",
                        lambda l: "stage1 driver started" in l,
                    )
                finally:
                    if mounts:
                        print(vm.ssh_cmd(["mkdir", "-p", GUEST_JAVDEV_MOUNT]).stdout)
                        print(
                            vm.ssh_cmd(
                                [
                                    "mount",
                                    GUEST_JAVDEV,
                                    GUEST_JAVDEV_MOUNT,
                                ]
                            ).stdout
                        )
                    yield vm
                    if mounts:
                        print(vm.ssh_cmd(["umount", GUEST_JAVDEV_MOUNT]).stdout)

            try:
                os.kill(vmsh.pid, 0)
            except ProcessLookupError:
                pass
            else:
                assert False, "vmsh was not terminated properly"
        if mounts:
            print(vm.ssh_cmd(["umount", GUEST_QEMUBLK_MOUNT]).stdout)
            print(vm.ssh_cmd(["umount", GUEST_QEMU9P]).stdout)


def run(
    cmd: List[str],
    extra_env: Dict[str, str] = {},
    stdout: Optional[int] = subprocess.PIPE,
    stderr: Optional[int] = subprocess.PIPE,
    input: Optional[str] = None,
    stdin: Optional[int] = None,
    check: bool = True,
) -> "subprocess.CompletedProcess[Text]":
    env = os.environ.copy()
    env.update(extra_env)
    env_string = []
    for k, v in extra_env.items():
        env_string.append(f"{k}={v}")
    print(f"$ {' '.join(env_string + cmd)}")
    return subprocess.run(
        cmd,
        stdout=stdout,
        stderr=stderr,
        check=check,
        env=env,
        text=True,
        input=input,
        stdin=stdin,
    )


def blkdiscard() -> Any:
    run(["sudo", "blkdiscard", "-f", HOST_SSD])


@contextmanager
def fresh_fs_ssd(image: Optional[Path]) -> Iterator[Any]:
    while "target is busy" in run(["sudo", "umount", HOST_SSD], check=False).stderr:
        print("umount: waiting for target not to be busy")
        time.sleep(1)
    blkdiscard()
    if image:
        run(
            [
                "sudo",
                "dd",
                "status=progress",
                "bs=128M",
                "iflag=direct",
                "oflag=direct",
                "conv=fdatasync",
                str(image),
                HOST_SSD,
            ]
        )
        run(["sudo", "resize2fs", HOST_SSD])
    else:
        run(["sudo", "mkfs.ext4", HOST_SSD])
    Path(HOST_DIR).mkdir(exist_ok=True)
    run(["sudo", "mount", HOST_SSD, HOST_DIR])
    run(["sudo", "chown", os.getlogin(), HOST_DIR])
    run(["sudo", "chown", os.getlogin(), HOST_SSD])
    try:
        run(["touch", f"{HOST_DIR}/file"], check=True)
        yield
    finally:
        run(["sudo", "chown", "0", HOST_SSD])
        if Path(HOST_DIR).is_mount():
            run(["sudo", "umount", HOST_DIR], check=False)


def check_ssd() -> None:
    print(subprocess.check_output(["lsblk"]).decode())
    input_ = "y"
    input_ = input(f"Delete {HOST_SSD} to use for benchmark? [Y/n] ")
    if input_ != "Y" and input_ != "y" and input_ != "":
        print("Aborting.")
        exit(1)


def check_intel_turbo() -> None:
    path = "/sys/devices/system/cpu/intel_pstate/no_turbo"
    if os.path.exists(path):
        with open(path) as f:
            if f.readline() != "1\n":
                print(
                    """Please run: sudo su -c 'echo "1" > /sys/devices/system/cpu/intel_pstate/no_turbo'"""
                )
                exit(1)


# look at those caches getting warm
def export_lineplot(name: str, data: Dict[str, List[float]]) -> None:
    frame = pd.DataFrame(data)
    path = f"{MEASURE_RESULTS}/{name}-{NOW}.tsv"
    print(path)
    frame.to_csv(path, index=False, sep="\t")
    frame.to_csv(f"{MEASURE_RESULTS}/{name}-latest.tsv", index=False, sep="\t")


def export_barplot(name: str, data: Dict[str, List[float]]) -> None:
    frame = pd.DataFrame(data)
    frame = frame.describe()
    path = f"{MEASURE_RESULTS}/{name}-{NOW}.tsv"
    print(path)
    frame.to_csv(path, sep="\t")
    frame.to_csv(f"{MEASURE_RESULTS}/{name}-latest.tsv", index=True, sep="\t")


def export_fio(name: str, data: Dict[str, List[float]]) -> None:
    os.makedirs(MEASURE_RESULTS, exist_ok=True)
    df = pd.DataFrame(data)
    print(df.describe())
    path = f"{MEASURE_RESULTS}/{name}-{NOW}.tsv"
    print(path)
    df.to_csv(path, index=True, sep="\t")
    df.to_csv(f"{MEASURE_RESULTS}/{name}-latest.tsv", index=True, sep="\t")


def read_stats(path: Path) -> DefaultDict[str, List]:
    stats: DefaultDict[str, List] = defaultdict(list)
    if not os.path.exists(path):
        return stats
    with open(path) as f:
        return json.load(f)


def write_stats(path: Path, stats: Dict[str, List]) -> None:
    path.parent.mkdir(exist_ok=True, parents=True)
    with open(path, "w") as f:
        json.dump(
            stats,
            f,
            indent=4,
            sort_keys=True,
        )
