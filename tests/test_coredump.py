import conftest
import os
import subprocess
import re
from tempfile import TemporaryDirectory
from typing import Dict


def parse_regs(qemu_output: str) -> Dict[str, int]:
    regs = {}
    for match in re.finditer(r"(\S+)\s*=\s*([0-9a-f ]+)", qemu_output):
        name = match.group(1)
        content = match.group(2).replace(" ", "")
        print(f"{name}={content}")
        regs[name.lower()] = int(content, 16)
    return regs


def test_coredump(helpers: conftest.Helpers) -> None:
    with TemporaryDirectory() as temp, helpers.spawn_qemu(helpers.notos_image()) as vm:
        core = os.path.join(temp, "core")
        vm.send("stop")
        res = vm.send("human-monitor-command", args={"command-line": "info registers"})
        regs = parse_regs(res["return"])
        # TODO make this arch indepentent
        assert regs["rip"] != 0
        helpers.run_vmsh_command(["coredump", str(vm.pid), core])
        subprocess.run(["readelf", "-a", core], check=True)
