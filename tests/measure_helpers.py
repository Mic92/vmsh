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
GUEST_JAVDEV = "/dev/vdb"
GUEST_QEMUBLK = "/dev/vdc"
GUEST_QEMU9P = "/9p"


@contextmanager
def testbench(
    helpers: confmeasure.Helpers, with_vmsh: bool = True, ioregionfd: bool = False
) -> Iterator[Any]:
    if ioregionfd:
        mmiomode = "ioregionfd"
    else:
        mmiomode = "wrap_syscall"

    with helpers.spawn_qemu(
        helpers.notos_image(),
        virtio_blk=HOST_SSD,
        virtio_9p=HOST_DIR,
    ) as vm:
        vm.wait_for_ssh()
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
        print(vm.ssh_cmd(["ls", "-la", GUEST_QEMU9P]).stdout)

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
                    yield vm

            try:
                os.kill(vmsh.pid, 0)
            except ProcessLookupError:
                pass
            else:
                assert False, "vmsh was not terminated properly"


def run(
    cmd: List[str],
    extra_env: Dict[str, str] = {},
    stdout: int = subprocess.PIPE,
    stderr: int = subprocess.PIPE,
    input: Optional[str] = None,
    stdin: Optional[int] = None,
    check: bool = True,
) -> "subprocess.CompletedProcess[Text]":
    env = os.environ.copy()
    env.update(extra_env)
    env_string = []
    for k, v in extra_env.items():
        env_string.append(f"{k}={v}")
    print(f"$ {' '.join(env_string)} {' '.join(cmd)}")
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


@contextmanager
def fresh_ssd() -> Iterator[Any]:
    try:
        while "target is busy" in run(["sudo", "umount", HOST_SSD], check=False).stderr:
            print("umount: waiting for target not to be busy")
            time.sleep(1)
        run(["blkdiscard", "-f", HOST_SSD], check=True)
        run(["mkfs.ext4", HOST_SSD], check=True)
        Path(HOST_DIR).mkdir(exist_ok=True)
        run(["sudo", "mount", HOST_SSD, HOST_DIR], check=True)
        run(["sudo", "chown", os.getlogin(), HOST_DIR], check=True)
    except Exception:
        pass
    yield
    run(["sudo", "umount", HOST_SSD], check=False)


def check_ssd() -> None:
    print(subprocess.check_output(["lsblk"]).decode())
    input_ = "y"
    input_ = input(f"Delete {HOST_SSD} to use for benchmark? [Y/n] ")
    if input_ != "Y" and input_ != "y" and input_ != "":
        print("Aborting.")
        exit(1)


def check_system() -> None:
    try:
        with open("/sys/devices/system/cpu/intel_pstate/no_turbo") as f:
            if f.readline() != "1\n":
                print(
                    """Please run: sudo su -c 'echo "1" > /sys/devices/system/cpu/intel_pstate/no_turbo'"""
                )
                exit(1)
    finally:
        return


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


def export_fio(name: str, data_: Dict[str, List[float]]) -> None:
    data: Any = data_.copy()
    data["index"] = ["read", "stddev", "write", "stddev"]
    frame = pd.DataFrame(data)
    frame = frame.set_index("index")
    path = f"{MEASURE_RESULTS}/{name}-{NOW}.tsv"
    print(path)
    frame.to_csv(path, index=True, sep="\t")
    frame.to_csv(f"{MEASURE_RESULTS}/{name}-latest.tsv", index=True, sep="\t")


def read_stats(path: str) -> DefaultDict[str, Dict[str, List]]:
    stats: DefaultDict[str, Dict[str, List]] = defaultdict(Dict[str, list])
    if not os.path.exists(path):
        return stats
    with open(path) as f:
        raw_stats = json.load(f)
        for key, value in raw_stats.items():
            stats[key] = value
    return stats


def write_stats(path: str, stats: DefaultDict[str, Dict[str, List]]) -> None:
    with open(path, "w") as f:
        json.dump(stats, f)
