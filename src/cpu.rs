// struct Regs was taken from https://golang.org/pkg/syscall/#PtraceGetRegs

#[cfg(target_arch = "aarch64")]
mod arch {
    #[repr(C)]
    #[derive(Clone)]
    pub struct Regs {
        regs: [u64; 31],
        sp: u64,
        pc: u64,
        pstate: u64,
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
}

#[cfg(target_arch = "x86_64")]
mod arch {
    #[repr(C)]
    #[derive(Clone)]
    pub struct Regs {
        r15: u64,
        r14: u64,
        r13: u64,
        r12: u64,
        rbp: u64,
        rbx: u64,
        r11: u64,
        r10: u64,
        r9: u64,
        r8: u64,
        rax: u64,
        rcx: u64,
        rdx: u64,
        rsi: u64,
        rdi: u64,
        orig_rax: u64,
        rip: u64,
        cs: u64,
        eflags: u64,
        rsp: u64,
        ss: u64,
        fs_base: u64,
        gs_base: u64,
        ds: u64,
        es: u64,
        fs: u64,
        gs: u64,
    }

    impl Regs {
        pub fn ip(&self) -> u64 {
            self.rip
        }

        pub fn prepare_syscall(&self, args: &[u64; 7]) -> Regs {
            let mut copy = self.clone();
            copy.rax = args[0];
            copy.rdi = args[1];
            copy.rsi = args[2];
            copy.rdx = args[3];
            copy.r10 = args[4];
            copy.r8 = args[5];
            copy.r9 = args[6];
            return copy;
        }

        pub fn syscall_ret(&self) -> u64 {
            self.rax
        }
    }

    // $ rasm2  -a x86 -b 64 'syscall'
    pub const SYSCALL_TEXT: u64 = 0x050F;
}

pub use arch::*;
