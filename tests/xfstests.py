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


def println(s: str) -> None:
    print(s, flush=True)


def excludes() -> List[str]:
    """
    Exclude tests based on that they fail on the respective upstream/mainline
    system. Tests which fail on vmsh-blk but succeed on qemu-blk are NOT
    excluded.
    """
    # -g quick on xfs
    # Some tests are skipped because xfsdump is missing. I don't think this is
    # packaged in nixos.
    native_scratch = [
        # this test requires a deprication warning to be absent, but it is present. I dont care about that though.
        "xfs/539",
    ]

    qemu_blk_scratch = [
        # quota stuff, similar to generic/594
        "xfs/050"
        "xfs/144"
        "xfs/153"
    ]

    vmsh_blk_scratch: List[str] = []

    # TEST_DIR=/mnt TEST_DEV=/dev/vdb1 SCRATCH_DEV=/dev/vdb2 SCRATCH_MNT=/scratchmnt xfstests-check

    return []
    return native_scratch + qemu_blk_scratch + vmsh_blk_scratch


def excludes_str() -> str:
    return ",".join(excludes())


def unmount(dev: str) -> None:
    while "target is busy" in str(run(["sudo", "umount", dev], check=False).stderr):
        println(f"umount {dev}: waiting for target not to be busy")
        time.sleep(1)


def format_ssd() -> None:

    unmount(HOST_SSD)
    unmount(HOST_SSDp1)
    unmount(HOST_SSDp2)
    util.blkdiscard()
    run(["sudo", "parted", HOST_SSD, "--", "mklabel", "gpt"])
    run(["sudo", "parted", HOST_SSD, "--", "mkpart", "primary", "0%", "10G"])
    run(["sudo", "parted", HOST_SSD, "--", "mkpart", "primary", "10G", "20G"])
    run(["sudo", f"mkfs.{FS}", HOST_SSDp1])
    run(["sudo", f"mkfs.{FS}", HOST_SSDp2])
    Path(HOST_DIR).mkdir(exist_ok=True)
    Path(HOST_DIR_SCRATCHDEV).mkdir(exist_ok=True)
    run(["sudo", "chown", os.getlogin(), HOST_DIR])
    run(["sudo", "chown", os.getlogin(), HOST_DIR_SCRATCHDEV])
    run(["sudo", "chown", os.getlogin(), HOST_SSD])


def get_failures(fail: str) -> List[str]:
    if fail.startswith("Failures: "):
        failures = fail.split(" ")[1:]
    else:
        failures = []
    return failures


def native(stats: Dict[str, str]) -> None:
    env = {"TEST_DIR": HOST_DIR, "TEST_DEV": HOST_SSDp1}
    env_scratch = {"SCRATCH_DEV": HOST_SSDp2, "SCRATCH_MNT": HOST_DIR_SCRATCHDEV}
    if WITH_SCRATCH:
        env = dict(env, **env_scratch)
    if QUICK:
        run(
            ["sudo", "-E", "xfstests-check", "-e", excludes_str(), "generic/600"],
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
        lines = f.readlines()
        stats["native"] = lines[-1].strip()
        fail = lines[-2].strip()
        failures = get_failures(fail)

    recoveries: List[str] = []
    for failure in failures:
        time.sleep(1)
        run(
            ["sudo", "-E", "xfstests-check", "-e", excludes_str(), failure],
            stdout=None,
            stderr=None,
            extra_env=env,
            check=False,
        )
        with open("results/check.log", "r") as f:
            lines = f.readlines()
            if "Passed all 1 tests" in lines[-1].strip():
                recoveries += [failure]

    println(f"Failures detected: {failures}")
    println(f"Failure recoveries: {recoveries}")
    stats["native_recoveries"] = f"{len(recoveries)}/{len(failures)}"


def qemu_blk(helpers: confmeasure.Helpers, stats: Dict[str, str]) -> None:
    with util.testbench(helpers, with_vmsh=False, ioregionfd=False, mounts=False) as vm:
        vm.ssh_cmd(["mkdir", "-p", "/mnt"], check=True)
        vm.ssh_cmd(["mkdir", "-p", "/scratchmnt"], check=True)
        # breakpoint()
        env = f"TEST_DIR=/mnt TEST_DEV={GUEST_QEMUBLK}1"
        if WITH_SCRATCH:
            env += f" SCRATCH_DEV={GUEST_QEMUBLK}2 SCRATCH_MNT=/scratchmnt"
        if QUICK:
            vm.ssh_cmd(
                [
                    "sh",
                    "-c",
                    f"{env} xfstests-check -e '{excludes_str()}' generic/623",
                ],
                stdout=None,
                check=False,
            )
        else:
            vm.ssh_cmd(
                [
                    "sh",
                    "-c",
                    f"{env} xfstests-check -e '{excludes_str()}' -g quick",
                ],
                stdout=None,
                check=False,
            )

        lines = vm.ssh_cmd(["tail", "results/check.log"]).stdout.split("\n")
        stats["qemu-blk"] = lines[-2].strip()

        fail = lines[-3].strip()
        failures = get_failures(fail)

        recoveries: List[str] = []
        for failure in failures:
            time.sleep(1)
            vm.ssh_cmd(
                [
                    "sh",
                    "-c",
                    f"{env} xfstests-check -e '{excludes_str()}' {failure}",
                ],
                stdout=None,
                check=False,
            )
            lines = vm.ssh_cmd(["tail", "results/check.log"]).stdout.split("\n")
            if "Passed all 1 tests" in lines[-2].strip():
                recoveries += [failure]

        println(f"Failures detected: {failures}")
        println(f"Failure recoveries: {recoveries}")
        stats["qemu-blk_recoveries"] = f"{len(recoveries)}/{len(failures)}"


def vmsh_blk(helpers: confmeasure.Helpers, stats: Dict[str, str]) -> None:
    with util.testbench(helpers, with_vmsh=True, ioregionfd=False, mounts=False) as vm:
        vm.ssh_cmd(["mkdir", "-p", "/mnt"], check=True)
        vm.ssh_cmd(["mkdir", "-p", "/scratchmnt"], check=True)
        # breakpoint()
        env = f"TEST_DIR=/mnt TEST_DEV={GUEST_JAVDEV}1"
        if WITH_SCRATCH:
            env += f" SCRATCH_DEV={GUEST_JAVDEV}2 SCRATCH_MNT=/scratchmnt"
        if QUICK:
            vm.ssh_cmd(
                [
                    "sh",
                    "-c",
                    f"{env} xfstests-check -e '{excludes_str()}' xfs/539",
                ],
                stdout=None,
                check=False,
            )
        else:
            vm.ssh_cmd(
                [
                    "sh",
                    "-c",
                    f"{env} xfstests-check -e '{excludes_str()}' -g quick",
                ],
                stdout=None,
                check=False,
            )
        lines = vm.ssh_cmd(["tail", "results/check.log"]).stdout.split("\n")
        stats["vmsh-blk"] = lines[-2].strip()

        fail = lines[-3].strip()
        failures = get_failures(fail)

        recoveries: List[str] = []
        for failure in failures:
            time.sleep(1)
            vm.ssh_cmd(
                [
                    "sh",
                    "-c",
                    f"{env} xfstests-check -e '{excludes_str()}' {failure}",
                ],
                stdout=None,
                check=False,
            )
            lines = vm.ssh_cmd(["tail", "results/check.log"]).stdout.split("\n")
            if "Passed all 1 tests" in lines[-2].strip():
                recoveries += [failure]

        println(f"Failures detected: {failures}")
        println(f"Failure recoveries: {recoveries}")
        stats["vmsh-blk_recoveries"] = f"{len(recoveries)}/{len(failures)}"


def main() -> None:
    util.check_ssd()
    helpers = confmeasure.Helpers()
    stats: Dict[str, str] = {}

    println("")
    println("================================ native test ========================")
    println("")
    format_ssd()
    native(stats)
    println("")
    println("================================ qemu-blk test ========================")
    println("")
    format_ssd()
    qemu_blk(helpers, stats)
    println("")
    println("================================ vmsh-blk test ========================")
    println("")
    format_ssd()
    vmsh_blk(helpers, stats)

    stats["excluded"] = str(len(excludes()))
    println(str(stats))


if __name__ == "__main__":
    main()
