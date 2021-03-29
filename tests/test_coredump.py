import os
import time
from tempfile import TemporaryDirectory
from typing import Dict, IO

import conftest
from coredump import ElfCore
from qemu import QemuVm


MSR_EFER = 0xC0000080


def check_coredump(fd: IO[bytes], qemu_regs: Dict[str, int], vm: QemuVm) -> None:
    core = ElfCore(fd)
    assert len(core.regs) > 0
    assert len(core.fpu_regs) > 0
    assert len(core.special_regs) > 0
    assert core.regs[0].rip == qemu_regs["rip"]
    for name in ["cr0", "cr2", "cr3", "cr4"]:
        # print(f"{name} = 0x{getattr(core.special_regs[0], name):x}")
        assert getattr(core.special_regs[0], name) == qemu_regs[name]
    assert core.msrs[0][0].index == MSR_EFER
    cr3 = core.special_regs[0].cr3
    page_table_segment = core.find_segment_by_addr(cr3)
    assert page_table_segment
    data = core.map_segment(page_table_segment)
    res = vm.send("human-monitor-command", args={"command-line": f"xp 0x{cr3:x}"})
    value = int(res["return"].split(": ")[1].strip(), 16)
    assert data[cr3] == value


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
            check_coredump(fd, qemu_regs, vm)
