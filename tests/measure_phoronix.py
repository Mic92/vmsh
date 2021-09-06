from contextlib import contextmanager
import measure_helpers as util
from confmeasure import Helpers
from typing import Iterator, List, Dict, IO
from pathlib import Path
from dataclasses import dataclass
import subprocess
import pandas as pd
import phoronix

from nix import nix_build


@contextmanager
def fresh_fs_ssd() -> Iterator[None]:
    with util.fresh_fs_ssd(nix_build(".#phoronix-image")[0]["outputs"]["out"]):
        yield


def phoronix_test_suite() -> Path:
    p = Path(nix_build(".#phoronix-test-suite")[0]["outputs"]["out"])
    return p.joinpath("bin/phoronix-test-suite")


@dataclass
class Command:
    args: List[str]
    env: Dict[str, str]

    @property
    def env_vars(self) -> List[str]:
        env_vars = []
        for k, v in self.env.items():
            env_vars.append(f"{k}={v}")
        return env_vars


def phoronix_command(phoronix_path: Path, skip_tests: List[str]) -> Command:
    exe = phoronix_test_suite()
    env = dict(
        SKIP_TESTS="iozone",
        PTS_USER_PATH_OVERRIDE=f"{phoronix_path}/",
        PTS_DOWNLOAD_CACHE=f"{phoronix_path.joinpath('download-cache')}/",
        TEST_RESULTS_NAME="vmsh",
        TEST_RESULTS_IDENTIFIER="vmsh",
    )
    # IOzone is marked as deprecated & unmaintained and does not work
    env["SKIP_TESTS"] = ",".join(skip_tests + ["iozone"])

    return Command([str(exe), "run", "pts/disk"], env)


REPORT_PATH = Path("test-results/vmsh/composite.xml")


def parse_result(report_path: Path, identifier: str) -> pd.DataFrame:
    if not report_path.exists():
        raise OSError(f"phoronix did not create a report at {report_path}")
    df = phoronix.parse_xml(report_path)
    df["identifier"] = identifier
    return df


def yes_please() -> IO[bytes]:
    yes_please = subprocess.Popen(["yes"], stdout=subprocess.PIPE)
    assert yes_please.stdout is not None
    return yes_please.stdout


def link_phoronix_test_suite(test_suite: Path) -> List[str]:
    return [
        "ln",
        "-sfn",
        str(test_suite),
        "/root/.phoronix-test-suite",
    ]


def native(skip_tests: List[str]) -> pd.DataFrame:
    with fresh_fs_ssd():
        test_suite = util.HOST_DIR_PATH.joinpath("phoronix-test-suite")
        report_path = test_suite.joinpath(REPORT_PATH)
        cmd = phoronix_command(test_suite, skip_tests)
        # this is gross but so is phoronix test suite
        util.run(["sudo"] + link_phoronix_test_suite(test_suite))
        util.run(
            ["sudo", "taskset", "--cpu-list", "0", "env"] + cmd.env_vars + cmd.args,
            extra_env=cmd.env,
            stdout=None,
            stderr=None,
            stdin=yes_please(),
            # we check at the end if phoronix passed tests when we parse results
            check=False,
        )
        return parse_result(report_path, "native")


def qemu_blk(skip_tests: List[str]) -> pd.DataFrame:
    with fresh_fs_ssd(), util.testbench(
        Helpers(), with_vmsh=False, ioregionfd=False, mounts=True
    ) as vm:
        test_suite = util.GUEST_QEMUBLK_PATH.joinpath("phoronix-test-suite")
        report_path = test_suite.joinpath(REPORT_PATH)
        vm.ssh_cmd(link_phoronix_test_suite(test_suite))
        cmd = phoronix_command(test_suite, skip_tests)
        vm.ssh_cmd(cmd.args, extra_env=cmd.env, stdout=None, stdin=yes_please())
        return parse_result(report_path, "qemu-blk")


# def vmsh_blk(helpers: confmeasure.Helpers, stats: Dict[str, List[Any]]) -> None:
#    with util.fresh_fs_ssd(), util.testbench(
#        helpers, with_vmsh=True, ioregionfd=True, mounts=False
#    ) as vm:
#        pass


STATS_PATH = util.MEASURE_RESULTS.joinpath("phoronix-stats.tsv")


def main() -> None:
    util.check_ssd()
    util.check_intel_turbo()
    benchmarks = [("native", native), ("qemu_blk", qemu_blk)]
    df = None
    if STATS_PATH.exists():
        df = pd.read_csv(STATS_PATH, sep="\t")

    for name, benchmark in benchmarks:
        # skip_tests = "fio,sqlite,fs-mark,dbench,ior,compilebench,postmark".split(",")
        skip_tests = "fio,sqlite,dbench,ior,compilebench,postmark".split(",")
        if df is not None:
            skip_tests = list(df[df.identifier == name].benchmark_name.unique())
            if len(skip_tests) == 6:
                # result of len(df.benchmark_name.unique())
                continue
        new_df = benchmark(skip_tests)
        if df is not None:
            df = pd.concat([df, new_df])
        else:
            df = new_df

        STATS_PATH.parent.mkdir(exist_ok=True, parents=True)
        df.to_csv(STATS_PATH, index=True, sep="\t")


if __name__ == "__main__":
    main()
