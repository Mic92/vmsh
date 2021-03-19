import ctypes as ct
import os
import time
from tempfile import TemporaryDirectory
from typing import Dict

import conftest
from coredump import core_user, elf_fpregset_t, elf_prstatus
from elftools.elf.elffile import ELFFile
from elftools.elf.segments import NoteSegment

NT_PRXFPREG = 1189489535


def validate_elf_structure(core: ELFFile, qemu_regs: Dict[str, int]) -> None:
    note_segment = next(core.iter_segments())
    assert isinstance(note_segment, NoteSegment)
    regs = []
    fpu_regs = []
    special_regs = []
    for note in note_segment.iter_notes():
        if note.n_type == "NT_PRSTATUS":
            assert note.n_descsz == ct.sizeof(elf_prstatus)
            regs.append(
                elf_prstatus.from_buffer_copy(note.n_desc.encode("latin-1")).pr_reg
            )
        elif note.n_type == NT_PRXFPREG:
            assert note.n_descsz == ct.sizeof(elf_fpregset_t)
            fpu_regs.append(
                elf_fpregset_t.from_buffer_copy(note.n_desc.encode("latin-1"))
            )
        elif note.n_type == "NT_TASKSTRUCT":
            assert note.n_descsz == ct.sizeof(core_user)
            special_regs.append(
                core_user.from_buffer_copy(note.n_desc.encode("latin1")).sregs
            )
    assert len(regs) > 0
    assert len(fpu_regs) > 0
    assert len(special_regs) > 0
    assert regs[0].rip == qemu_regs["rip"]


def validate_coredump(core: str, regs: Dict[str, int]) -> None:
    with open(core, "rb") as fd:
        validate_elf_structure(ELFFile(fd), regs)


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
        regs = vm.regs()
        time.sleep(0.01)
        # sanity check if we really stopped the vm
        regs2 = vm.regs()
        assert regs["rip"] != 0 and regs2["rip"] == regs["rip"]
        core = os.path.join(temp, "core")
        helpers.run_vmsh_command(["coredump", str(vm.pid), core])
        validate_coredump(core, regs)
