from contextlib import contextmanager
import confmeasure
import measure_helpers as util
from typing import Iterator, List, Any, Dict
from collections import defaultdict

from nix import nix_build


@contextmanager
def fresh_fs_ssd() -> Iterator[None]:
    with util.fresh_fs_ssd(nix_build("phoronix-image")):
        yield


def native(stats: Dict[str, List[Any]]) -> None:
    with fresh_fs_ssd():
        breakpoint()


def qemu_blk(helpers: confmeasure.Helpers, stats: Dict[str, List[Any]]) -> None:
    with util.fresh_fs_ssd(), util.testbench(
        helpers, with_vmsh=False, ioregionfd=False, mounts=False
    ) as vm:
        pass


def vmsh_blk(helpers: confmeasure.Helpers, stats: Dict[str, List[Any]]) -> None:
    with util.fresh_fs_ssd(), util.testbench(
        helpers, with_vmsh=True, ioregionfd=True, mounts=False
    ) as vm:
        pass


def main() -> None:
    util.check_ssd()
    util.check_intel_turbo()
    stats = defaultdict(list)
    native(stats)
    # SKIP_TESTS=iozone DEFAULT_TEST_DIRECTORY=/tmp/ TEST_RESULTS_NAME=vmsh TEST_RESULTS_IDENTIFIER=zfs4 ./phoronix-test-suite/phoronix-test-suite run pts/disk
    # yes | DEFAULT_TEST_DIRECTORY=/tmp/ TEST_RESULTS_NAME=vmsh TEST_RESULTS_IDENTIFIER=zfs3 ./phoronix-test-suite/phoronix-test-suite install pts/disk
    # yes = subprocess.run(["yes"], stdout=subprocess.PIPE)
    # subprocess.run([""], stdin=yes, check=True)
    # helpers = confmeasure.Helpers()


if __name__ == "__main__":
    main()
