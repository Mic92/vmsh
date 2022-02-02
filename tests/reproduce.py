#!/usr/bin/env python3

import sys

if sys.version_info < (3, 7, 0):
    print("This script assumes at least python3.7")
    sys.exit(1)

from timeit import default_timer as timer
import os
import shutil
import time
from typing import IO, Any, Callable, List, Dict, Optional, Text
import subprocess
from pathlib import Path

ROOT = Path(__file__).parent.parent.resolve()
HAS_TTY = sys.stderr.isatty()

sys.path.append(str(ROOT.joinpath("tests")))

from measure_helpers import fresh_fs_ssd, HOST_DIR


def color_text(code: int, file: IO[Any] = sys.stdout) -> Callable[[str], None]:
    def wrapper(text: str) -> None:
        if HAS_TTY:
            print(f"\x1b[{code}m{text}\x1b[0m", file=file)
        else:
            print(text, file=file)

    return wrapper


warn = color_text(31, file=sys.stderr)
info = color_text(32)


def run(
    cmd: List[str],
    extra_env: Dict[str, str] = {},
    input: Optional[str] = None,
    check: bool = True,
    cwd: str = str(ROOT),
    shell: bool = False,
) -> "subprocess.CompletedProcess[Text]":
    env = os.environ.copy()
    env.update(extra_env)
    env_string = []
    for k, v in extra_env.items():
        env_string.append(f"{k}={v}")
    info(f"$ {' '.join(env_string)} {' '.join(cmd)}")
    if cwd != os.getcwd():
        info(f"cd {cwd}")
    return subprocess.run(
        cmd, cwd=cwd, check=check, env=env, text=True, input=input, shell=shell
    )


def nix_develop(
    command: List[str], extra_env: Dict[str, str], cwd: str = str(ROOT)
) -> None:
    run(
        [
            "nix",
            "develop",
            "--extra-experimental-features",
            "flakes nix-command",
            "--command",
        ]
        + command,
        extra_env=extra_env,
        cwd=cwd,
    )


def robustness(extra_env: Dict[str, str]) -> None:
    nix_develop(["python", "tests/xfstests.py"], extra_env=extra_env)


def generality_hypervisors(extra_env: Dict[str, str]) -> None:
    info("run unittest for all hypervisors")
    result_file = ROOT.joinpath("tests", "measurements", "hypervisor_test.ok")
    if result_file.exists():
        print("skip hypervisor tests")
        return
    nix_develop(["pytest", "-s", "tests/test_hypervisor.py"], extra_env=extra_env)
    with open(result_file, "w") as f:
        f.write("YES")


def generality_kernels(extra_env: Dict[str, str]) -> None:
    info("run unittest for all kernel versions")
    result_file = ROOT.joinpath("tests", "measurements", "kernel_test.ok")
    if result_file.exists():
        print("skip kernel tests")
        return
    nix_develop(["pytest", "-s", "tests/test_attach.py"], extra_env=extra_env)
    with open(result_file, "w") as f:
        f.write("YES")


def phoronix(extra_env: Dict[str, str]) -> None:
    nix_develop(["python", "tests/measure_phoronix.py"], extra_env=extra_env)


def block(extra_env: Dict[str, str]) -> None:
    result_file = ROOT.joinpath("tests", "measurements", "measure_block.ok")
    if result_file.exists():
        print("skip block device benchmark")
        return
    nix_develop(["python", "tests/measure_block.py"], extra_env=extra_env)
    with open(result_file, "w") as f:
        f.write("YES")


def console(extra_env: Dict[str, str]) -> None:
    nix_develop(["python", "tests/measure_console.py"], extra_env=extra_env)


def docker_hub(extra_env: Dict[str, str]) -> None:
    result_file = ROOT.joinpath("tests", "measurements", "docker-hub.ok")
    if result_file.exists():
        print("skip docker-hub test")
        return
    with fresh_fs_ssd():
        runq_path = Path(HOST_DIR).joinpath("runq")
        run(
            [
                "git",
                "clone",
                "--recursive",
                "https://github.com/Mic92/runq",
                str(runq_path),
            ]
        )
        images = ROOT.joinpath("tests", "measurements", "docker-images.json")
        if not images.exists():
            shutil.copyfile(ROOT.joinpath("tests", "docker-images.json"), images)
        run(
            ["nix-shell", "--run", f"python shrink_containers.py {images}"],
            extra_env=extra_env,
            cwd=str(runq_path),
        )

        run(
            [
                f"sudo lsof {HOST_DIR} | awk '{{print $2}}' | tail -n +2 | sudo xargs -r kill"
            ],
            shell=True,
        )
        time.sleep(3)
        run(
            [
                f"sudo lsof {HOST_DIR} | awk '{{print $2}}' | tail -n +2 | sudo xargs -r kill -9"
            ],
            shell=True,
        )
    with open(result_file, "w") as f:
        f.write("YES")


# hässlich
def usecase1(extra_env: Dict[str, str]) -> None:
    pass


def usecase2(extra_env: Dict[str, str]) -> None:
    result_file = ROOT.joinpath("tests", "measurements", "usecase2.ok")
    if result_file.exists():
        print("skip usecase2 test")
        return
    nix_develop(
        [
            "pytest",
            "-s",
            "tests/test_hypervisor.py",
            "-k",
            "test_qemu_and_change_password",
        ],
        extra_env=extra_env,
    )
    with open(result_file, "w") as f:
        f.write("YES")


def usecase3(extra_env: Dict[str, str]) -> None:
    result_file = ROOT.joinpath("tests", "measurements", "usecase3.ok")
    if result_file.exists():
        print("skip usecase3 test")
        return
    nix_develop(
        [
            "pytest",
            "-s",
            "tests/test_alpine.py",
        ],
        extra_env=extra_env,
    )
    with open(result_file, "w") as f:
        f.write("YES")


def evaluation(extra_env: Dict[str, str]) -> None:
    info("Run evaluations")
    experiments = {
        "6.1 Robustness (xfstests)": robustness,
        "6.2 Generality, hypervisors": generality_hypervisors,
        "6.2 Generality, kernels": generality_kernels,
        "Figure 5. Relative performance of vmsh-blk for the Phoronix Test Suite compared to qemu-blk.": phoronix,
        "Figure 6. fio with different configurations featuring qemu-blk and vmsh-blk with direct IO, and file IO with qemu-9p.": block,
        "Figure 7. Loki-console responsiveness compared to SSH.": console,
        # "usecase #1: : Serverless debug shell": usecase1,
        "usecase #2: : VM rescue system": usecase2,
        "usecase #3: : Package security scanner": usecase3,
        "Figure 8. VM size reduction for the top-40 Docker images (average reduction: 60%).": docker_hub,
    }
    for figure, function in experiments.items():
        info(figure)
        for i in range(3):
            try:
                start = timer()
                function(extra_env)
                end = timer()
                print(f"{figure} took {(end - start) / 60} minutes")
                break
            except subprocess.TimeoutExpired:
                warn(f"'{figure}' took too long to run: retry ({i + 1}/3)!")
                if i == 2:
                    sys.exit(1)
            except subprocess.CalledProcessError:
                warn(f"'{figure}' failed to run: retry ({i + 1}/3)!")
                if i == 2:
                    sys.exit(1)


def generate_graphs() -> None:
    results = ROOT.joinpath("tests", "graphs")
    if results.exists():
        shutil.rmtree(results)
    results.mkdir()
    tsv_files = ["console-latest.tsv", "fio-latest.tsv", "phoronix-stats.tsv"]
    for f in tsv_files:
        result = ROOT.joinpath("tests", "measurements", f)
        if not result.exists():
            warn(
                f"tsv file {result} does not exists! It should have been created during evaluation"
            )
        shutil.copyfile(result, results.joinpath(f))
    graphs = ROOT.joinpath("tests", "graphs.py")

    nix_develop(
        ["bash", "-c", f"cd {results} && python {str(graphs)} {' '.join(tsv_files)}"],
        extra_env=dict(),
    )
    info(f"Result and graphs data have been written to {results}")


def main() -> None:
    nix_shell = shutil.which("nix-shell", mode=os.F_OK | os.X_OK)
    if nix_shell is None:
        warn(
            "For reproducibility this script requires the nix package manager to be installed: https://nixos.org/download.html"
        )
        sys.exit(1)
    sudo = shutil.which("sudo", mode=os.F_OK | os.X_OK)
    if sudo is None:
        warn("During the evaluation we need the 'sudo' command")
        sys.exit(1)

    run(["sudo", "tee", "/sys/devices/system/cpu/intel_pstate/no_turbo"], input="1\n")

    host_ssd = os.environ.get("HOST_SSD")
    if not host_ssd:
        warn("HOST_SSD environment variable is not set. Not running evaluation!")
        sys.exit(1)

    evaluation(extra_env=dict(HOST_SSD=host_ssd))
    generate_graphs()


if __name__ == "__main__":
    main()
