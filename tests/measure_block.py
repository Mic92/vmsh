from root import MEASURE_RESULTS
import confmeasure
import measure_helpers as util
from measure_helpers import GUEST_JAVDEV, GUEST_QEMUBLK

from typing import List, Any, Optional, Callable, Tuple
import re
import json


# overwrite the number of samples to take to a minimum
QUICK = True


def lsblk(vm: Any) -> None:
    term = vm.ssh_cmd(["lsblk"], check=False)
    print(term.stdout)


def hdparm(vm: Any, device: str) -> Optional[float]:
    term = vm.ssh_cmd(["hdparm", "-t", device], check=False)
    if term.returncode != 0:
        return None
    out = term.stdout
    print(out)
    out = re.sub(" +", " ", out).split(" ")
    mb = float(out[5])
    sec = float(out[8])
    return mb / sec


def fio(
    vm: Any,
    target: str,
    random: bool = False,
    readonly: bool = True,
    iops: bool = False,
    file: bool = False,
) -> Tuple[float, float, float, float]:
    """
    inspired by https://docs.oracle.com/en-us/iaas/Content/Block/References/samplefiocommandslinux.htm
    @param random: random vs sequential
    @param readonly: readonly vs read+write
    @param iops: return iops vs bandwidth
    @param file: target is file vs blockdevice
    @return (read_mean, stddev, write_mean, stdev) in kiB/s
    """
    runtime = 120
    if QUICK:
        runtime = 10

    cmd = ["fio", f"--filename={target}"]

    if file:
        cmd += ["--size=500GB"]

    cmd += ["--direct=1"]

    if readonly and random:
        cmd += ["--rw=randread"]
    elif not readonly and random:
        # fio/examples adds rwmixread=60 and rwmixwrite=40 here
        cmd += ["--rw=randrw"]
    elif readonly and not random:
        cmd += ["--rw=read"]
    elif not readonly and not random:
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

    if readonly:
        cmd += ["--readonly"]

    cmd += ["--output-format=json"]

    print(cmd)
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
        return (
            read["iops_mean"],
            read["iops_stddev"],
            write["iops_mean"],
            write["iops_stddev"],
        )
    else:
        print("Bandwidth read", float(read["bw_mean"]) / 1024 / 1024, "GB/s")
        print("Bandwidth write", float(write["bw_mean"]) / 1024 / 1024, "GB/s")
        return (read["bw_mean"], read["bw_dev"], write["bw_mean"], write["bw_dev"])


SIZE = 16
WARMUP = 0
if QUICK:
    WARMUP = 0
    SIZE = 2


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


if __name__ == "__main__":
    util.check_ssd()
    util.check_system()
    helpers = confmeasure.Helpers()

    measurements = util.read_stats(f"{MEASURE_RESULTS}/stats.json")
    measurements["fio"] = {}
    measurements["hdparm"] = {}
    with util.fresh_ssd():
        with util.testbench(helpers, with_vmsh=True, ioregionfd=False) as vm:
            lsblk(vm)
            measurements["fio"]["foo"] = list(
                fio(
                    vm,
                    GUEST_QEMUBLK,
                    random=False,
                    readonly=True,
                    iops=False,
                    file=False,
                )
            )
            measurements["hdparm"]["attached_wrapsys_qemudev"] = sample(
                lambda: hdparm(vm, GUEST_QEMUBLK)
            )
            measurements["hdparm"]["attached_wrapsys_javdev"] = sample(
                lambda: hdparm(vm, GUEST_JAVDEV)
            )
    #
    #        with util.testbench(helpers, with_vmsh=False, ioregionfd=False) as vm:
    #            lsblk(vm)
    #            measurements["hdparm_detached_9p"] = sample(
    #                lambda: hdparm(vm, GUEST_QEMUBLK)
    #            )

    util.export_lineplot("hdparm_warmup", measurements["hdparm"])
    util.export_barplot("hdparm_warmup_barplot", measurements["hdparm"])
    util.export_fio("fio", measurements["fio"])
    util.write_stats(f"{MEASURE_RESULTS}/stats.json", measurements)
