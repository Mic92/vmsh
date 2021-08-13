from root import MEASURE_RESULTS
import confmeasure
import measure_helpers as util
from measure_helpers import GUEST_JAVDEV, GUEST_QEMUDEV

from typing import List, Any, Optional, Callable
import re


# overwrite the number of samples to take to a minimum
QUICK = False


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


def sample(
    f: Callable[[], Optional[float]], size: int = 16, warmup: int = 0
) -> List[float]:
    if QUICK:
        warmup = 0
        size = 2
    ret = []
    for i in range(0, warmup):
        f()
    for i in range(0, size):
        util.reset_ssd()
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
    with util.testbench(helpers, with_vmsh=True, ioregionfd=False) as vm:
        lsblk(vm)
        measurements["hdparm_attached_wrapsys_qemudev"] = sample(
            lambda: hdparm(vm, GUEST_QEMUDEV)
        )
        measurements["hdparm_attached_wrapsys_javdev"] = sample(
            lambda: hdparm(vm, GUEST_JAVDEV)
        )
        measurements["hdparm_attached_wrapsys_qemudev2"] = sample(
            lambda: hdparm(vm, GUEST_QEMUDEV)
        )

    util.export_lineplot("hdparm_warmup", measurements)
    util.export_barplot("hdparm_warmup_barplot", measurements)
    util.write_stats(f"{MEASURE_RESULTS}/stats.json", measurements)
