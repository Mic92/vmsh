"""
# Compare block devices:

- qemu virtio blk (detached_qemublk, direct_detached_qemublk)
- qemu virtio 9p (detached_qemu9p)
- vmsh virtio blk ws (attached_ws_javdev, direct_ws_javdev)
- vmsh virtio blk ioregionfd (attached_iorefd_javdev, direct_iorefd_javdev)

for each:
- best case bw read & write
- worst case iops read & write

# Compare guest performance under vmsh

- native (direct_host)
- detached (direct_detached_qemublk)
- vmsh ws (direct_ws_qemublk)
- vmsh ioregionfd (direct_iorefd_qemublk)
- run via vmsh (in container in vm)
- run via ssh (no container in vm)

for each:
- blkdev
- shell latency
- phoronix
"""

from root import MEASURE_RESULTS
import confmeasure
import measure_helpers as util
from measure_helpers import (
    GUEST_JAVDEV,
    GUEST_QEMUBLK,
    GUEST_QEMU9P,
    GUEST_JAVDEV_MOUNT,
    GUEST_QEMUBLK_MOUNT,
    HOST_SSD,
    run,
)
from qemu import QemuVm
from dataclasses import dataclass

from typing import List, Any, Optional, Callable, DefaultDict
import re
import json
from enum import Enum


# overwrite the number of samples to take to a minimum
# TODO turn this to False for releases. Results look very different.
QUICK = True


def lsblk(vm: QemuVm) -> None:
    term = vm.ssh_cmd(["lsblk"], check=False)
    print(term.stdout)


def hdparm(vm: QemuVm, device: str) -> Optional[float]:
    term = vm.ssh_cmd(["hdparm", "-t", device], check=False)
    if term.returncode != 0:
        return None
    out = term.stdout
    print(out)
    fields = re.sub(" +", " ", out).split(" ")
    mb = float(fields[5])
    sec = float(fields[8])
    return mb / sec


@dataclass
class FioResult:
    read_mean: float
    read_stddev: float
    write_mean: float
    write_stddev: float


class Rw(Enum):
    r = 1
    w = 2
    rw = 3


def fio(
    vm: Optional[QemuVm],
    device: str,
    random: bool = False,
    rw: Rw = Rw.r,
    iops: bool = False,
    file: bool = False,
) -> FioResult:
    """
    inspired by https://docs.oracle.com/en-us/iaas/Content/Block/References/samplefiocommandslinux.htm
    @param random: random vs sequential
    @param iops: return iops vs bandwidth
    @param file: target is file vs blockdevice
    @return (read_mean, stddev, write_mean, stdev) in kiB/s
    """
    runtime = 120
    size = 100  # filesize in GB
    if QUICK:
        runtime = 10
        size = 10
    cmd = []
    if not vm and not file:
        cmd += ["sudo"]
    cmd += ["fio"]

    if file:
        cmd += [f"--filename={device}/file", f"--size={size}GB"]
    else:
        cmd += [f"--filename={device}", "--direct=1"]

    if rw == Rw.r and random:
        cmd += ["--rw=randread"]
    if rw == Rw.w and random:
        cmd += ["--rw=randwrite"]
    elif rw == Rw.rw and random:
        # fio/examples adds rwmixread=60 and rwmixwrite=40 here
        cmd += ["--rw=randrw"]
    elif rw == Rw.r and not random:
        cmd += ["--rw=read"]
    elif rw == Rw.w and not random:
        cmd += ["--rw=write"]
    elif rw == Rw.rw and not random:
        cmd += ["--rw=readwrite"]

    if iops:
        # fio/examples uses 16 here as well
        cmd += ["--bs=4k", "--ioengine=libaio", "--iodepth=64"]
    else:
        cmd += ["--bs=64k", "--ioengine=libaio", "--iodepth=16"]

    cmd += [
        f"--runtime={runtime}",
        "--numjobs=1",
        "--time_based",
        "--group_reporting",
        "--name=generic_name",
        "--eta-newline=1",
    ]

    if not file and rw == Rw.r:
        cmd += ["--readonly"]

    cmd += ["--output-format=json"]

    # print(cmd)
    if vm is None:
        term = run(cmd, check=True)
    else:
        term = vm.ssh_cmd(cmd, check=True)

    out = term.stdout
    # print(out)
    j = json.loads(out)
    read = j["jobs"][0]["read"]
    write = j["jobs"][0]["write"]

    if iops:
        print(
            "IOPS: read",
            read["iops_mean"],
            read["iops_stddev"],
            "write",
            write["iops_mean"],
            write["iops_stddev"],
        )
        return FioResult(
            read["iops_mean"],
            read["iops_stddev"],
            write["iops_mean"],
            write["iops_stddev"],
        )
    else:
        print("Bandwidth read", float(read["bw_mean"]) / 1024 / 1024, "GB/s")
        print("Bandwidth write", float(write["bw_mean"]) / 1024 / 1024, "GB/s")
        return FioResult(
            read["bw_mean"], read["bw_dev"], write["bw_mean"], write["bw_dev"]
        )


SIZE = 16
WARMUP = 0
if QUICK:
    WARMUP = 0
    SIZE = 2


# QUICK: 20s else: 5min
def sample(
    f: Callable[[], Optional[float]], size: int = SIZE, warmup: int = WARMUP
) -> List[float]:
    ret = []
    for i in range(0, warmup):
        f()
    for i in range(0, size):
        r = f()
        if r is None:
            return []
        ret += [r]
    return ret


STATS_PATH = MEASURE_RESULTS.joinpath("fio-stats.json")


def fio_read_write(
    vm: Optional[QemuVm],
    device: str,
    random: bool = False,
    iops: bool = False,
    file: bool = False,
) -> FioResult:
    read = fio(
        vm,
        device,
        random=random,
        rw=Rw.r,
        iops=iops,
        file=file,
    )
    write = fio(
        vm,
        device,
        random=random,
        rw=Rw.w,
        iops=iops,
        file=file,
    )
    return FioResult(
        read.read_mean, read.read_stddev, write.write_mean, write.write_stddev
    )


# QUICK: ? else: ~2*2.5min
def fio_suite(
    vm: Optional[QemuVm],
    stats: DefaultDict[str, List[Any]],
    device: str,
    name: str,
    file: bool = True,
) -> None:
    if name in stats["system"]:
        print(f"skip {name}")
        return
    print(f"run {name}")

    if not file:
        util.blkdiscard()

    bw = fio_read_write(
        vm,
        device,
        random=False,
        iops=False,
        file=file,
    )

    if not file:
        util.blkdiscard()

    iops = fio_read_write(
        vm,
        device,
        random=True,
        iops=True,
        file=file,
    )

    results = [
        (
            "best-case-bw-seperate",
            bw,
        ),
        (
            "worst-case-iops-seperate",
            iops,
        ),
    ]
    for benchmark, result in results:
        stats["system"].append(name)
        stats["benchmark"].append(benchmark)
        stats["read_mean"].append(result.read_mean)
        stats["read_stddev"].append(result.read_stddev)
        stats["write_mean"].append(result.write_mean)
        stats["write_stddev"].append(result.write_stddev)
    util.write_stats(STATS_PATH, stats)


def main() -> None:
    """
    not quick: 5 * fio_suite(5min) + 2 * sample(5min) = 35min
    """
    util.check_ssd()
    util.check_intel_turbo()
    helpers = confmeasure.Helpers()

    fio_stats = util.read_stats(STATS_PATH)

    fio_suite(None, fio_stats, HOST_SSD, "direct_host1", file=False)
    fio_suite(None, fio_stats, HOST_SSD, "direct_host2", file=False)

    with util.testbench(helpers, with_vmsh=False, ioregionfd=False, mounts=False) as vm:
        fio_suite(vm, fio_stats, GUEST_QEMUBLK, "direct_detached_qemublk", file=False)
    with util.testbench(helpers, with_vmsh=True, ioregionfd=False, mounts=False) as vm:
        fio_suite(vm, fio_stats, GUEST_QEMUBLK, "direct_ws_qemublk", file=False)
        fio_suite(vm, fio_stats, GUEST_JAVDEV, "direct_ws_javdev", file=False)
    with util.testbench(helpers, with_vmsh=True, ioregionfd=True, mounts=False) as vm:
        fio_suite(vm, fio_stats, GUEST_QEMUBLK, "direct_iorefd_qemublk", file=False)
        fio_suite(vm, fio_stats, GUEST_JAVDEV, "direct_iorefd_javdev", file=False)

    # file based benchmarks don't blkdiscard on their own, so we do it as often as possible
    with util.fresh_fs_ssd():
        with util.testbench(helpers, with_vmsh=False, ioregionfd=False) as vm:
            lsblk(vm)
            fio_suite(vm, fio_stats, GUEST_QEMUBLK_MOUNT, "detached_qemublk")
    with util.fresh_fs_ssd():
        with util.testbench(helpers, with_vmsh=False, ioregionfd=False) as vm:
            fio_suite(vm, fio_stats, GUEST_QEMU9P, "detached_qemu9p")

    with util.fresh_fs_ssd():
        with util.testbench(helpers, with_vmsh=True, ioregionfd=False) as vm:
            fio_suite(vm, fio_stats, GUEST_JAVDEV_MOUNT, "attached_ws_javdev")

    with util.fresh_fs_ssd():
        with util.testbench(helpers, with_vmsh=True, ioregionfd=True) as vm:
            fio_suite(vm, fio_stats, GUEST_JAVDEV_MOUNT, "attached_iorefd_javdev")

    util.export_fio("fio", fio_stats)


if __name__ == "__main__":
    main()
