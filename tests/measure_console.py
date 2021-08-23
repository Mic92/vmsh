from root import MEASURE_RESULTS
import confmeasure
import measure_helpers as util
from measure_helpers import (
    run,
)
from qemu import QemuVm
from dataclasses import dataclass

from typing import List, Any, Optional, Callable, DefaultDict
import io
import re
import json
import time
import timeit
from enum import Enum
from pty import openpty
import os


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
    out = re.sub(" +", " ", out).split(" ")
    mb = float(out[5])
    sec = float(out[8])
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


STATS_PATH = MEASURE_RESULTS.joinpath("console-stats.json")


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


def expect(fd: int, timeout: int, until: Optional[str] = None) -> bool:
    """
    @return true if terminated because of until
    """
    print("begin readall until", until)
    import select
    buf = ""
    ret = False
    (r, _, _) = select.select([fd], [], [], timeout)
    # print("selected")
    print("[readall] ", end="")
    while fd in r:
        # print("reading")
        out = os.read(fd, 1).decode()
        # print("len", len(out))
        buf += out
        if until is not None and until in buf:
            ret = True
            break
        (r, _, _) = select.select([fd], [], [], timeout)
        if len(out) == 0:
            break
    print(buf.replace("\n", "\n[readall] "), end="")
    # print(list(buf))
    if not buf.endswith("\n"):
        print("")
    # if until == "\hello world\n":
        # print(list(buf))
    return ret


def assertline(ptmfd: int, value: str) -> None:
    assert expect(ptmfd, 2, f"\n{value}\r")  # not sure why and when how many \r's occur.


def writeall(fd: int, content: str) -> None:
    print("[writeall]", content.strip())
    c = os.write(fd, str.encode(content))
    if c != len(content):
        raise Exception("TODO implement writeall")


def echo(ptmfd: int, prompt: str, cp1: str) -> float:
    sw = time.monotonic()
    writeall(ptmfd, "echo hello world\n")

    # assert expect(ptmfd, 2, cp1)
    # sw = time.monotonic() - sw
    assertline(ptmfd, "hello world")
    # assert ptm.readline().strip() == "hello world"
    sw = time.monotonic() - sw

    assert expect(ptmfd, 2, prompt)
    time.sleep(0.5)
    return sw


def main() -> None:
    """
    not quick: 5 * fio_suite(5min) + 2 * sample(5min) = 35min
    """
    util.check_system()
    helpers = confmeasure.Helpers()

    console_stats = util.read_stats(STATS_PATH)

    (ptmfd, ptsfd) = openpty()
    import subprocess
    sh = subprocess.Popen(["/bin/sh"], stdin=ptsfd, stdout=ptsfd, stderr=ptsfd)
    assert expect(ptmfd, 2, "sh-4.4$")
    samples = sample(lambda: echo(ptmfd, "sh-4.4$", " echo hello world\r"), size=2)
    print("samples:", samples)
    sh.kill()
    sh.wait()
    os.close(ptmfd)
    os.close(ptsfd)

    (ptmfd, ptsfd) = openpty()
    pts = os.readlink(f"/proc/self/fd/{ptsfd}")
    os.close(ptsfd)
    # pts = "/proc/self/fd/1"
    print("pts: ", pts)
    cmd = ["/bin/sh"]
    with util.testbench_console(helpers, pts, guest_cmd=cmd) as _:
        # breakpoint()
        assert expect(ptmfd, 2, "~ #")
        samples = sample(lambda: echo(ptmfd, "~ #", "\necho hello world\r"), size=2)
        print("samples:", samples)
        # print(f"echo: {echo(ptmfd, ptm)}s")

    os.close(ptmfd)

    util.export_fio("console", console_stats)  # TODO rename


if __name__ == "__main__":
    main()
