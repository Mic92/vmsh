import os
import time
from tempfile import TemporaryDirectory

import conftest
from coredump import ElfCore


MSR_EFER = 0xC0000080


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
        assert qemu_regs["rip"] != 0 and qemu_regs2["rip"] == qemu_regs["rip"]
        core_path = os.path.join(temp, "core")
        helpers.run_vmsh_command(["coredump", str(vm.pid), core_path])
        with open(core_path, "rb") as fd:
            core = ElfCore(fd)
            assert len(core.regs) > 0
            assert len(core.fpu_regs) > 0
            assert len(core.special_regs) > 0
            assert core.regs[0].rip == qemu_regs["rip"]
            for name in ["cr0", "cr2", "cr3", "cr4"]:
                assert getattr(core.special_regs[0], name) == qemu_regs[name]
            assert core.msrs[0][0].index == MSR_EFER
