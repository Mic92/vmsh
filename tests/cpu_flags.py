# from linux: arch/x86/include/uapi/asm/processor-flags.h
#
# Basic CPU control in CR0
#


#
# Paging options in CR3
#
X86_CR3_PWT = 0x00000008  # Page Write Through
X86_CR3_PCD = 0x00000010  # Page Cache Disable
X86_CR3_PCID_MASK = 0x00000FFF  # PCID Mask

#
# Intel CPU features in CR4
#
X86_CR4_VME = 0x00000001  # enable vm86 extensions
X86_CR4_PVI = 0x00000002  # virtual interrupts flag enable
X86_CR4_TSD = 0x00000004  # disable time stamp at ipl 3
X86_CR4_DE = 0x00000008  # enable debugging extensions
X86_CR4_PSE = 0x00000010  # enable page size extensions
X86_CR4_PAE = 0x00000020  # enable physical address extensions
X86_CR4_MCE = 0x00000040  # Machine check enable
X86_CR4_PGE = 0x00000080  # enable global pages
X86_CR4_PCE = 0x00000100  # enable performance counters at ipl 3
X86_CR4_OSFXSR = 0x00000200  # enable fast FPU save and restore
X86_CR4_OSXMMEXCPT = 0x00000400  # enable unmasked SSE exceptions
X86_CR4_VMXE = 0x00002000  # enable VMX virtualization */
X86_CR4_RDWRGSFS = 0x00010000  # enable RDWRGSFS support */
X86_CR4_PCIDE = 0x00020000  # enable PCID support */
X86_CR4_OSXSAVE = 0x00040000  # enable xsave and xrestore */
X86_CR4_SMEP = 0x00100000  # enable SMEP support */
X86_CR4_SMAP = 0x00200000  # enable SMAP support */
X86_CR4_PKE = 0x400000  # Protection keys support (since linux 4.6)

X86_CR0_PE = 0x00000001  # Protection Enable
X86_CR0_MP = 0x00000002  # Monitor Coprocessor
X86_CR0_EM = 0x00000004  # Emulation
X86_CR0_TS = 0x00000008  # Task Switched
X86_CR0_ET = 0x00000010  # Extension Type
X86_CR0_NE = 0x00000020  # Numeric Error
X86_CR0_WP = 0x00010000  # Write Protect
X86_CR0_AM = 0x00040000  # Alignment Mask
X86_CR0_NW = 0x20000000  # Not Write-through
X86_CR0_CD = 0x40000000  # Cache Disable
X86_CR0_PG = 0x80000000  # Paging


# EFER bits
_EFER_SCE = 0  # SYSCALL/SYSRET
_EFER_LME = 8  # Long mode enable
_EFER_LMA = 10  # Long mode active (read-only)
_EFER_NX = 11  # No execute enable
_EFER_SVME = 12  # Enable virtualization
_EFER_LMSLE = 13  # Long Mode Segment Limit Enable
_EFER_FFXSR = 14  # Enable Fast FXSAVE/FXRSTOR
EFER_SCE = 1 << _EFER_SCE
EFER_LME = 1 << _EFER_LME
EFER_LMA = 1 << _EFER_LMA
EFER_NX = 1 << _EFER_NX
EFER_SVME = 1 << _EFER_SVME
EFER_LMSLE = 1 << _EFER_LMSLE
EFER_FFXSR = 1 << _EFER_FFXSR

_PAGE_PRESENT = 1  # is present
_PAGE_RW = 2  # writable
_PAGE_USER = 4  # userspace addressable
_PAGE_PWT = 8  # page write through
_PAGE_PCD = 16  # page cache disabled
_PAGE_ACCESSED = 32  # was accessed (raised by CPU)
_PAGE_DIRTY = 64  # was written to (raised by CPU)
_PAGE_PSE = 128  # 4 MB (or 2MB) page
_PAGE_GLOBAL = 256  # Global TLB entry PPro+
_PAGE_SOFTW1 = 512  # available for programmer
_PAGE_SOFTW2 = 1024  # ^
_PAGE_SOFTW3 = 2048  # ^
_PAGE_SOFTW4 = 288230376151711744  # ^
_PAGE_PAT = 128  # on 4KB pages
_PAGE_PAT_LARGE = 4096  # On 2MB or 1GB pages
_PAGE_SPECIAL = _PAGE_SOFTW1
_PAGE_CPA_TEST = _PAGE_SOFTW2
_PAGE_NX = 9223372036854775808  # only on 64-bit
_PAGE_DEVMAP = _PAGE_SOFTW4  # only on 64-bit
_PAGE_SOFT_DIRTY = _PAGE_SOFTW3
