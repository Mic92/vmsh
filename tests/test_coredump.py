import os
import time
from tempfile import TemporaryDirectory

import conftest
from coredump import ElfCore


def test_coredump(helpers: conftest.Helpers) -> None:
    with TemporaryDirectory() as temp, helpers.spawn_qemu(helpers.notos_image()) as vm:
        while True:
            regs = vm.regs()
            # TODO make this arch indepentent
            if "eip" not in regs:
                break
            # wait till CPU is in 32-bit on boot
            time.sleep(0.01)

        vm.send("stop")
        qemu_regs = vm.regs()
        time.sleep(0.01)
        # sanity check if we really stopped the vm
        qemu_regs2 = vm.regs()
        assert regs["rip"] != 0 and qemu_regs2["rip"] == regs["rip"]
        core_path = os.path.join(temp, "core")
        helpers.run_vmsh_command(["coredump", str(vm.pid), core_path])
        with open(core_path, "rb") as fd:
            core = ElfCore(fd)
            assert len(core.regs) > 0
            assert len(core.fpu_regs) > 0
            assert len(core.special_regs) > 0
            assert core.regs[0].rip == qemu_regs["rip"]
