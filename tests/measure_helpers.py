import confmeasure
from confmeasure import NOW
from root import MEASURE_RESULTS

import os
import json
from typing import List, Any, Iterator, Dict, DefaultDict
from collections import defaultdict
from contextlib import contextmanager
import subprocess
import pandas as pd


HOST_SSD = "/dev/nvme0n1"
GUEST_JAVDEV = "/dev/vdb"
GUEST_QEMUDEV = "/dev/vdc"


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
        extra_drive=HOST_SSD,
    ) as vm:
        vm.wait_for_ssh()

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


def reset_ssd() -> None:
    term = subprocess.run(["blkdiscard", "-f", HOST_SSD])
    print(term.stdout)
    if term.returncode != 0:
        print(term.stderr)
        print("blkdiscard failed.")
        exit(1)


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


def read_stats(path: str) -> DefaultDict[str, List]:
    stats: DefaultDict[str, List] = defaultdict(list)
    if not os.path.exists(path):
        return stats
    with open(path) as f:
        raw_stats = json.load(f)
        for key, value in raw_stats.items():
            stats[key] = value
    return stats


def write_stats(path: str, stats: DefaultDict[str, List]) -> None:
    with open(path, "w") as f:
        json.dump(stats, f)
