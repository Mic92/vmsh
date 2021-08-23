from root import MEASURE_RESULTS
import confmeasure
import measure_helpers as util

from typing import List, Any, Optional, Callable
import time
from pty import openpty
import os


# overwrite the number of samples to take to a minimum
# TODO turn this to False for releases. Results look very different.
QUICK = True


SIZE = 32
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


def expect(fd: int, timeout: int, until: Optional[str] = None) -> bool:
    """
    @return true if terminated because of until
    """
    if QUICK:
        print("begin readall until", until)
    import select

    buf = ""
    ret = False
    (r, _, _) = select.select([fd], [], [], timeout)
    # print("selected")
    if QUICK:
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
    if QUICK:
        print(buf.replace("\n", "\n[readall] "), end="")
        print("\x1b[0m")
    # print(list(buf))
    if QUICK and not buf.endswith("\n"):
        print("")
    # if until == "\hello world\n":
    # print(list(buf))
    return ret


def assertline(ptmfd: int, value: str) -> None:
    assert expect(
        ptmfd, 2, f"\n{value}\r"
    )  # not sure why and when how many \r's occur.


def writeall(fd: int, content: str) -> None:
    if QUICK:
        print("[writeall]", content.strip())
    c = os.write(fd, str.encode(content))
    if c != len(content):
        raise Exception("TODO implement writeall")


def echo(ptmfd: int, prompt: str, cp1: str) -> float:
    print("measuring echo")
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


def vmsh_console(helpers: confmeasure.Helpers, stats: Any) -> None:
    name = "vmsh-console"
    if name in stats.keys():
        print(f"skip {name}")
        return

    (ptmfd, ptsfd) = openpty()
    pts = os.readlink(f"/proc/self/fd/{ptsfd}")
    os.close(ptsfd)
    # pts = "/proc/self/fd/1"
    print("pts: ", pts)
    cmd = ["/bin/sh"]
    with util.testbench_console(helpers, pts, guest_cmd=cmd) as _:
        # breakpoint()
        assert expect(ptmfd, 2, "~ #")
        samples = sample(lambda: echo(ptmfd, "~ #", "\necho hello world\r"))
        print("samples:", samples)
        # print(f"echo: {echo(ptmfd, ptm)}s")

    os.close(ptmfd)

    stats[name] = samples
    util.write_stats(STATS_PATH, stats)


def native(helpers: confmeasure.Helpers, stats: Any) -> None:
    name = "native"
    if name in stats.keys():
        print(f"skip {name}")
        return
    (ptmfd, ptsfd) = openpty()
    import subprocess

    sh = subprocess.Popen(["/bin/sh"], stdin=ptsfd, stdout=ptsfd, stderr=ptsfd)
    assert expect(ptmfd, 2, "sh-4.4$")
    samples = sample(lambda: echo(ptmfd, "sh-4.4$", " echo hello world\r"))
    print("samples:", samples)
    sh.kill()
    sh.wait()
    os.close(ptmfd)
    os.close(ptsfd)

    stats[name] = samples
    util.write_stats(STATS_PATH, stats)


def ssh(helpers: confmeasure.Helpers, stats: Any) -> None:
    name = "ssh"
    if name in stats.keys():
        print(f"skip {name}")
        return
    (ptmfd, ptsfd) = openpty()
    (ptmfd_stub, ptsfd_stub) = openpty()
    pts_stub = os.readlink(f"/proc/self/fd/{ptsfd_stub}")
    os.close(ptsfd_stub)
    with util.testbench_console(helpers, pts_stub, guest_cmd=["/bin/ls"]) as vm:
        # breakpoint()
        sh = vm.ssh_Popen(stdin=ptsfd, stdout=ptsfd, stderr=ptsfd)
        assert expect(ptmfd, 2, "~]$")
        samples = sample(lambda: echo(ptmfd, "~]$", "\necho hello world\r"))
        sh.kill()
        sh.wait()
        print("samples:", samples)

    os.close(ptmfd_stub)
    os.close(ptsfd)
    os.close(ptmfd)

    stats[name] = samples
    util.write_stats(STATS_PATH, stats)


def main() -> None:
    """
    not quick: 5 * fio_suite(5min) + 2 * sample(5min) = 35min
    """
    util.check_system()
    helpers = confmeasure.Helpers()

    stats = util.read_stats(STATS_PATH)

    native(helpers, stats)
    ssh(helpers, stats)
    vmsh_console(helpers, stats)

    util.export_fio("console", stats)  # TODO rename


if __name__ == "__main__":
    main()
