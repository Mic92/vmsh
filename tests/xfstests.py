import confmeasure
import measure_helpers as util
from measure_helpers import (
    GUEST_JAVDEV,
    GUEST_QEMUBLK,
    HOST_SSD,
    HOST_DIR,
    run,
)

from typing import Dict, List
import os
from pathlib import Path
import time


# when true only a single test is run instead of the full suite
# (quick and with_scratch) takes ~2h
QUICK = False
WITH_SCRATCH = True

FS = "xfs"  # options: xfs, ext4

HOST_SSDp1 = f"{HOST_SSD}p1"
HOST_SSDp2 = f"{HOST_SSD}p2"


HOST_DIR_SCRATCHDEV = "/tmp/xfstests_scratchdev"


def excludes() -> List[str]:
    """
    Exclude tests based on that they fail on the respective upstream/mainline
    system. Tests which fail on vmsh-blk but succeed on qemu-blk are NOT
    excluded.
    """

    # -g quick on ext4

    native_noscratch = [
        # missing SCRATCH_MNT? Test impl error. Works with scratch.
        "generic/628",
        "generic/629",
    ]
    _ = native_noscratch
    native_scratch = [
        # -pwrite: No space left on device
        "ext4/306",
        # ?
        "generic/079",
        "generic/452",
        # The following are skipped in qemu, mostly because of missing kernel
        # features. (disk encryption)
        "ext4/051",
        "generic/548",
        "generic/549",
        "generic/550",
        "generic/582",
        "generic/583",
        "generic/584",
        "generic/592",
        "generic/602",
    ]
    qemu_blk_noscratch = [
        # "please ensure that /mnt is a shared mountpoint" (sounds like testimpl error)
        "generic/632",
    ]
    qemu_blk_scratch = [
        # blocks indefinitely >~5h
        "generic/397",
        # +/tmp/xfstests.szrcLx/tests/ext4/024: line 41: /scratchmnt/edir/file: No such file or directory
        "ext4/024",
        # -Write backwards sync leaving holes - defrag should do nothing
        "generic/018",
        # "group quota on SCRATCH_MNT (SCRATCH_DEV) is off" when it should on
        "generic/082",
        # -cp: failed to clone 'SCRATCH_MNT/test-356/file2' from 'SCRATCH_MNT/test-356/file1': Text file busy
        # -Tear it down
        # +./common/rc: line 2553: /dev/fd/62: No such file or directory
        "generic/356",
        # ...
        "generic/357",
        "generic/398",
        "generic/419",
        "generic/421",
        "generic/440",
        "generic/472",
        "generic/493",
        "generic/494",
        "generic/495",
        "generic/496",
        "generic/497",
        "generic/554",
        "generic/569",
        "generic/636",
        "generic/641",
    ]

    # -g xfs/quick on xfs
    # Some tests are skipped because xfsdump is missing. I don't think this is
    # packaged in nixos.
    native_scratch = [
        # kernel needs XFS_ONLINE_SCRUB
        "xfs/506",
        # this test requires a deprication warning to be absent, but it is present. I dont care about that though.
        "xfs/539",



        # works on ext4 but not on xfs
        # -Block grace time: 00:10; Inode grace time: 00:20
        # +Block grace time: DEF_TIME; Inode grace time: DEF_TIME
        "generic/594",
        # set grace to n but got grace n-2
        "generic/600",

        # Fixes:

        # as with ext4:
        "generic/079",
        # -chgrp: changing group of 'SCRATCH_MNT/dummy/foo': Disk quota exceeded
        # +chgrp: changing group of 'SCRATCH_MNT/dummy/foo': Operation not permitted
        "generic/566",
        # Fixed
        # +/tmp/xfstests_scratchdev/ls_on_scratch: unknown program 'ls_on_scratch'
        "generic/452",
        # Works now: (because of added user?)
        # -pwrite: Disk quota exceeded
        # a bunch of the previous ones have that error as well
        "generic/603",
        # the rest is mostly disjuct:
        "generic/230",
        "generic/328",
        "generic/355",
        "generic/382",

        # Ignorable errors:
        # device or resource busy. This is probably racy because it works when run individually.
        "generic/261",
    ]
    # Failures: 
    # known
    # generic/594 generic/600 
    # xfs/539
    # xfs/506

    # return native_scratch + qemu_blk_noscratch + qemu_blk_scratch
    return []


def excludes_str() -> str:
    return ",".join(excludes())


def unmount(dev: str) -> None:
    while "target is busy" in run(["sudo", "umount", dev], check=False).stderr:
        print(f"umount {dev}: waiting for target not to be busy")
        time.sleep(1)


def format_ssd() -> None:

    unmount(HOST_SSD)
    unmount(HOST_SSDp1)
    unmount(HOST_SSDp2)
    util.blkdiscard()
    run(["sudo", "parted", HOST_SSD, "--", "mklabel", "gpt"])
    run(["sudo", "parted", HOST_SSD, "--", "mkpart", "primary", "0%", "50%"])
    run(["sudo", "parted", HOST_SSD, "--", "mkpart", "primary", "50%", "100%"])
    run(["sudo", f"mkfs.{FS}", HOST_SSDp1])
    run(["sudo", f"mkfs.{FS}", HOST_SSDp2])
    Path(HOST_DIR).mkdir(exist_ok=True)
    Path(HOST_DIR_SCRATCHDEV).mkdir(exist_ok=True)
    run(["sudo", "chown", os.getlogin(), HOST_DIR])
    run(["sudo", "chown", os.getlogin(), HOST_DIR_SCRATCHDEV])
    run(["sudo", "chown", os.getlogin(), HOST_SSD])


def native(stats: Dict[str, str]) -> None:
    env = {"TEST_DIR": HOST_DIR, "TEST_DEV": HOST_SSDp1}
    env_scratch = {"SCRATCH_DEV": HOST_SSDp2, "SCRATCH_MNT": HOST_DIR_SCRATCHDEV}
    if WITH_SCRATCH:
        env = dict(env, **env_scratch)
    if QUICK:
        run(
            ["sudo", "-E", "xfstests-check", "-e", excludes_str(), "ext4/001"],
            stdout=None,
            stderr=None,
            extra_env=env,
            check=False,
        )
    else:
        run(
            ["sudo", "-E", "xfstests-check", "-e", excludes_str(), "-g", "quick"],
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
        vm.ssh_cmd(["mkdir", "-p", "/scratchmnt"], check=True)
        env = f"TEST_DIR=/mnt TEST_DEV={GUEST_QEMUBLK}1"
        if WITH_SCRATCH:
            env += f" SCRATCH_DEV={GUEST_QEMUBLK}2 SCRATCH_MNT=/scratchmnt"
        if QUICK:
            vm.ssh_cmd(
                [
                    "sh",
                    "-c",
                    f"{env} xfstests-check -e {excludes_str()} generic/484",
                ],
                stdout=None,
                check=False,
            )
        else:
            vm.ssh_cmd(
                [
                    "sh",
                    "-c",
                    f"{env} xfstests-check -e {excludes_str()} -g quick",
                ],
                stdout=None,
                check=False,
            )
        lines = vm.ssh_cmd(["tail", "results/check.log"]).stdout
        stats["qemu-blk"] = lines.split("\n")[-2].strip()


def vmsh_blk(helpers: confmeasure.Helpers, stats: Dict[str, str]) -> None:
    with util.testbench(helpers, with_vmsh=True, ioregionfd=False, mounts=False) as vm:
        vm.ssh_cmd(["mkdir", "-p", "/mnt"], check=True)
        vm.ssh_cmd(["mkdir", "-p", "/scratchmnt"], check=True)
        env = f"TEST_DIR=/mnt TEST_DEV={GUEST_JAVDEV}1"
        if WITH_SCRATCH:
            env += f" SCRATCH_DEV={GUEST_JAVDEV}2 SCRATCH_MNT=/scratchmnt"
        if QUICK:
            vm.ssh_cmd(
                [
                    "sh",
                    "-c",
                    f"{env} xfstests-check -e {excludes_str()} ext4/001",
                ],
                stdout=None,
                check=False,
            )
        else:
            vm.ssh_cmd(
                [
                    "sh",
                    "-c",
                    f"{env} xfstests-check -e {excludes_str()} -g quick",
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

    stats["excluded"] = str(len(excludes()))
    print(stats)


if __name__ == "__main__":
    main()
