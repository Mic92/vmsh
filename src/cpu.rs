#[cfg(target_arch = "aarch64")]
mod arch {
    #[repr(C)]
    #[derive(Clone, Copy, Debug)]
    pub struct Regs {
        pub regs: [u64; 31],
        pub sp: u64,
        pub pc: u64,
        pub pstate: u64,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Debug)]
    pub struct FpuRegs {
        pub vregs: [u128; 32],
        pub fpsr: u32,
        pub fpcr: u32,
    }

    impl Regs {
        pub fn ip(&self) -> u64 {
            self.pc
        }

        pub fn prepare_syscall(&self, args: &[u64; 7]) -> Regs {
            let mut copy = self.clone();
            copy.regs[0] = args[8];
            copy.regs[1] = args[0];
            copy.regs[2] = args[1];
            copy.regs[3] = args[2];
            copy.regs[4] = args[3];
            copy.regs[5] = args[4];
            copy.regs[6] = args[5];
            return copy;
        }

        pub fn syscall_ret(&self) -> u64 {
            self.regs[0]
        }
    }

    // $ rasm2  -a arm -b 64 'svc 0'
    pub const SYSCALL_TEXT: u64 = 0x010000D4;
    pub const SYSCALL_SIZE: u64 = 8;
}

#[cfg(target_arch = "x86_64")]
mod arch {
    #[repr(C)]
    #[derive(Clone, Copy, Debug)]
    pub struct Regs {
        pub r15: u64,
        pub r14: u64,
        pub r13: u64,
        pub r12: u64,
        pub rbp: u64,
        pub rbx: u64,
        pub r11: u64,
        pub r10: u64,
        pub r9: u64,
        pub r8: u64,
        pub rax: u64,
        pub rcx: u64,
        pub rdx: u64,
        pub rsi: u64,
        pub rdi: u64,
        pub orig_rax: u64,
        pub rip: u64,
        pub cs: u64,
        pub eflags: u64,
        pub rsp: u64,
        pub ss: u64,
        pub fs_base: u64,
        pub gs_base: u64,
        pub ds: u64,
        pub es: u64,
        pub fs: u64,
        pub gs: u64,
    }

    impl Regs {
        /// true if current cpu privilege level is userspace
        pub fn is_userspace(&self) -> bool {
            self.cs & 3 == 3
        }
    }

    // from arch/x86/include/asm/fpu/types.h
    #[repr(C)]
    #[derive(Clone, Copy, Debug)]
    pub struct FpuRegs {
        /// Control Word
        pub cwd: u16,
        /// Status Word
        pub swd: u16,
        /// Tag Word
        pub twd: u16,
        /// Last Instruction Opcode
        pub fop: u16,
        /// Instruction Pointer
        pub rip: u64,
        /// Data Pointer
        pub rdp: u64,
        pub mxcsr: u32,
        pub mxcsr_mask: u32,
        pub st_space: [u32; 32],
        pub xmm_space: [u32; 64],
        pub padding: [u32; 12],
        pub padding1: [u32; 12],
    }

    impl Regs {
        pub fn ip(&self) -> u64 {
            self.rip
        }

        pub fn prepare_syscall(&self, args: &[u64; 7]) -> Regs {
            let mut copy = *self;
            copy.rax = args[0];
            copy.rdi = args[1];
            copy.rsi = args[2];
            copy.rdx = args[3];
            copy.r10 = args[4];
            copy.r8 = args[5];
            copy.r9 = args[6];
            copy
        }

        pub fn syscall_ret(&self) -> u64 {
            self.rax
        }

        /// To be used during wrap_syscall.
        /// return (syscall_nr, arg1, ..., arg6)
        pub fn get_syscall_params(&self) -> (u64, u64, u64, u64, u64, u64, u64) {
            // self.rax contains return value of `syscall` instruction.
            // old rax (before `syscall` instruction) is rax_old or orig_rax.
            // also: https://lkml.org/lkml/2006/8/29/350:
            // > It's important that ORIG_EAX be set to some value that is not a valid system call
            // > number, so that the system call restart logic (see the signal handling code)
            // > doesn't trigger.
            (
                self.orig_rax,
                self.rdi,
                self.rsi,
                self.rdx,
                self.r10,
                self.r8,
                self.r9,
            )
        }
    }

    // $ rasm2  -a x86 -b 64 'syscall'
    pub const SYSCALL_TEXT: u64 = 0x050F;
    pub const SYSCALL_SIZE: u64 = 2;
}

pub use arch::*;
