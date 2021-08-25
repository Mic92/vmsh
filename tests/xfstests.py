import confmeasure
import measure_helpers as util
from measure_helpers import (
    GUEST_JAVDEV,
    GUEST_QEMUBLK,
    HOST_SSD,
    HOST_DIR,
    run,
)

from typing import Dict
import os
from pathlib import Path


# when true only a single test is run instead of the full suite
QUICK = True


HOST_SSDp1 = f"{HOST_SSD}p1"
HOST_SSDp2 = f"{HOST_SSD}p2"


def format_ssd() -> None:
    import time

    while "target is busy" in run(["sudo", "umount", HOST_SSD], check=False).stderr:
        print("umount: waiting for target not to be busy")
        time.sleep(1)
    util.blkdiscard()
    run(["sudo", "parted", HOST_SSD, "--", "mklabel", "gpt"])
    run(["sudo", "parted", HOST_SSD, "--", "mkpart", "primary", "0%", "50%"])
    run(["sudo", "parted", HOST_SSD, "--", "mkpart", "primary", "50%", "100%"])
    run(["sudo", "mkfs.ext4", HOST_SSDp1])
    run(["sudo", "mkfs.ext4", HOST_SSDp2])
    Path(HOST_DIR).mkdir(exist_ok=True)
    run(["sudo", "chown", os.getlogin(), HOST_DIR])
    run(["sudo", "chown", os.getlogin(), HOST_SSD])


def native(stats: Dict[str, str]) -> None:
    env = {"TEST_DIR": HOST_DIR, "TEST_DEV": HOST_SSDp1}
    # , "SCRATCH_DEV": HOST_SSDp2, "SCRATCH_MNT": "/tmp/scratchmnt"}
    if QUICK:
        run(
            ["sudo", "-E", "xfstests-check", "ext4/001"],
            stdout=None,
            stderr=None,
            extra_env=env,
            check=False,
        )
    else:
        run(
            ["sudo", "-E", "xfstests-check", "-g", "quick"],
            stdout=None,
            stderr=None,
            extra_env=env,
            check=False,
        )

    with open("results/check.log", "r") as f:
        stats["native"] = f.readlines()[-1].strip()


def qemu_blk(helpers: confmeasure.Helpers, stats: Dict[str, str]) -> None:
    with util.testbench(helpers, with_vmsh=False, ioregionfd=False, mounts=False) as vm:
        # breakpoint()
        vm.ssh_cmd(["mkdir", "-p", "/mnt"], check=True)
        if QUICK:
            vm.ssh_cmd(
                [
                    "sh",
                    "-c",
                    f"TEST_DIR=/mnt TEST_DEV={GUEST_QEMUBLK}1 xfstests-check generic/484",
                ],
                stdout=None,
                check=False,
            )
        else:
            vm.ssh_cmd(
                [
                    "sh",
                    "-c",
                    f"TEST_DIR=/mnt TEST_DEV={GUEST_QEMUBLK}1 xfstests-check -g quick",
                ],
                stdout=None,
                check=False,
            )
        lines = vm.ssh_cmd(["tail", "results/check.log"]).stdout
        stats["qemu-blk"] = lines.split("\n")[-2].strip()

        # failing:
        # generic/484
        # generic/099 i think this fixed itself.


def vmsh_blk(helpers: confmeasure.Helpers, stats: Dict[str, str]) -> None:
    with util.testbench(helpers, with_vmsh=True, ioregionfd=False, mounts=False) as vm:
        # breakpoint()
        vm.ssh_cmd(["mkdir", "-p", "/mnt"], check=True)
        if QUICK:
            vm.ssh_cmd(
                [
                    "sh",
                    "-c",
                    f"TEST_DIR=/mnt TEST_DEV={GUEST_JAVDEV}1 xfstests-check ext4/001",
                ],
                stdout=None,
                check=False,
            )
        else:
            vm.ssh_cmd(
                [
                    "sh",
                    "-c",
                    f"TEST_DIR=/mnt TEST_DEV={GUEST_JAVDEV}1 xfstests-check -g quick",
                ],
                stdout=None,
                check=False,
            )
        lines = vm.ssh_cmd(["tail", "results/check.log"]).stdout
        stats["vmsh-blk"] = lines.split("\n")[-2].strip()


def main() -> None:
    util.check_ssd()
    helpers = confmeasure.Helpers()
    format_ssd()
    stats: Dict[str, str] = {}

    native(stats)
    qemu_blk(helpers, stats)
    vmsh_blk(helpers, stats)

    print(stats)


if __name__ == "__main__":
    main()
