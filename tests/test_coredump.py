import conftest
import os
import subprocess
from tempfile import TemporaryDirectory


def test_coredump(helpers: conftest.Helpers) -> None:
    with TemporaryDirectory() as temp, helpers.spawn_qemu(helpers.notos_image()) as vm:
        core = os.path.join(temp, "core")
        helpers.run_vmsh_command(["coredump", str(vm.pid), core])
        subprocess.run(["readelf", "-a", core], check=True)
