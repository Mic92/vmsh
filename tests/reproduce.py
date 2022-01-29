#!/usr/bin/env python3

import sys

if sys.version_info < (3, 7, 0):
    print("This script assumes at least python3.7")
    sys.exit(1)

import os
import shutil
from typing import IO, Any, Callable, List, Dict, Optional, Text
import subprocess
from pathlib import Path

ROOT = Path(__file__).parent.parent.resolve()
HAS_TTY = sys.stderr.isatty()


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
) -> "subprocess.CompletedProcess[Text]":
    env = os.environ.copy()
    env.update(extra_env)
    env_string = []
    for k, v in extra_env.items():
        env_string.append(f"{k}={v}")
    info(f"$ {' '.join(env_string)} {' '.join(cmd)}")
    return subprocess.run(
        cmd, cwd=cwd, check=check, env=env, text=True, input=input, timeout=60 * 60
    )


def nix_develop(command: List[str], extra_env: Dict[str, str]) -> None:
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
    )


# medium
def robustness(extra_env: Dict[str, str]) -> None:
    nix_develop(["python", "tests/xfstests.py"], extra_env=extra_env)


# medium
def generality_hypervisors(extra_env: Dict[str, str]) -> None:
    pass


# medium
def generality_kernels(extra_env: Dict[str, str]) -> None:
    pass


# hässlich
def throughput(extra_env: Dict[str, str]) -> None:
    pass


# easy
def iops(extra_env: Dict[str, str]) -> None:
    pass


# easy
def console(extra_env: Dict[str, str]) -> None:
    pass


# hässlich
def docker_hub(extra_env: Dict[str, str]) -> None:
    pass


# hässlich
def usecase1(extra_env: Dict[str, str]) -> None:
    pass


# easy
def usecase2(extra_env: Dict[str, str]) -> None:
    pass


# easy
def usecase3(extra_env: Dict[str, str]) -> None:
    pass


def evaluation(extra_env: Dict[str, str]) -> None:
    info("Run evaluations")
    experiments = {
        "6.1 Robustness (xfstests)": robustness,
        "6.2 Generality, hypervisors": generality_hypervisors,
        "6.2 Generality, kernels": generality_kernels,
        "Figure 6 a) IO bandwidth/throughput. Best-case scenario.": throughput,
        "Figure 6 b) IO operations per second (IOPS). Worst case scenario.": iops,
        "Figure 7. Loki-console responsiveness compared to SSH.": console,
        "Figure 8. VM size reduction for the top-40 Docker images (average reduction: 60%).": docker_hub,
        "usecase #1: : Serverless debug shell": usecase1,
        "usecase #2: : VM rescue system": usecase2,
        "usecase #3: : Package security scanner": usecase3,
    }
    for figure, function in experiments.items():
        info(figure)
        for i in range(3):
            try:
                function(extra_env)
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
    results = ROOT.joinpath("results")
    if results.exists():
        shutil.rmtree(results)
    results.mkdir()
    tsv_files = ["foo.tsv"]
    for f in tsv_files:
        result = ROOT.joinpath(f)
        if not result.exists():
            warn(
                f"tsv file {result} does not exists! It should have been created during evaluation"
            )
        shutil.copyfile(result, results.joinpath(f))
    graphs = ROOT.joinpath("graphs.py")

    run(
        [
            "nix-shell",
            "--run",
            f"cd {results} && python {graphs} foo.tsv",
        ]
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