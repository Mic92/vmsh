use std::collections::HashMap;
use std::io::IoSlice;
use std::mem::{size_of, size_of_val};
use std::ptr;

use elfloader::arch::x86_64::RelocationTypes;
use elfloader::{
    ElfBinary, ElfLoader, ElfLoaderErr, Entry, Flags, LoadableHeaders, RelocationEntry,
    RelocationType, VAddr,
};
use log::{debug, error, warn};
use nix::sys::mman::ProtFlags;
use nix::sys::uio::{process_vm_writev, RemoteIoVec};
use simple_error::{bail, require_with, try_with};
use stage1_interface::{DeviceState, Stage1Args};
use xmas_elf::sections::{SectionData, SHN_UNDEF};
use xmas_elf::symbol_table::{Binding, DynEntry64};

use crate::guest_mem::MappedMemory;
use crate::kernel::{Kernel, LINUX_KERNEL_KASLR_RANGE};
use crate::kvm::allocator::VirtAlloc;
use crate::kvm::PhysMemAllocator;
use crate::page_math::{page_align, page_start};
use crate::page_table::VirtMem;
use crate::result::Result;
use crate::stage1::{DeviceStatus, DriverStatus};
use crate::try_core_res;

pub struct Loader<'a> {
    /// the linux kernel we link our code against
    kernel: &'a Kernel,
    /// the virtual memory our binary is baked by
    virt_mem: Option<VirtMem>,
    /// To page align elf section we need to pad space before and after each section
    /// These are offsets where within an allocation where the actual section starts
    load_offsets: Vec<usize>,
    allocator: &'a mut PhysMemAllocator,
    /// elf section of type PT_LOAD
    loadables: Vec<Loadable>,
    /// the whole elf file
    binary: &'a [u8],
    /// exported symbols from the elf binary above
    lib_syms: HashMap<&'a str, usize>,
    /// parsed elf header of the binary
    elf: ElfBinary<'a>,
    /// reference to dynamic symbol table section of the elf binary
    dyn_syms: &'a [DynEntry64],
    /// virtual address to `VMSH_STAGE1_ARGS` struct, used to write stage1 arguments
    vmsh_stage1_args: usize,
    /// How much space we need to reserve for strings for stage1_args.
    /// Needs to be page aligned
    string_arg_size: usize,
    /// virtual address of the `vmsh_stage1_init` function
    pub init_func: usize,
}

fn find_loadable(loadables: &mut [Loadable], addr: usize) -> Option<&mut Loadable> {
    loadables
        .iter_mut()
        .find(|loadable| loadable.mapping.contains(addr))
}

impl<'a> Loader<'a> {
    pub fn new(
        binary: &'a [u8],
        kernel: &'a Kernel,
        return_address: usize,
        allocator: &'a mut PhysMemAllocator,
    ) -> Result<Loader<'a>> {
        let elf = try_core_res!(ElfBinary::new(binary), "cannot parse elf binary");
        let dyn_symbol_section = require_with!(
            elf.file.find_section_by_name(".dynsym"),
            "binary has no .dynsym section"
        );
        let dyn_symbol_table = dyn_symbol_section.get_data(&elf.file)?;
        let dyn_syms = match dyn_symbol_table {
            SectionData::DynSymbolTable64(entries) => entries,
            _ => bail!(
                "expected .dynsym to be a DynSymbolTable64, got: {:?}",
                dyn_symbol_table
            ),
        };

        // we use symtab instead of dynsym here because rust does not allow us
        // to export our assembly trampoline (_init_vmsh) from the stage1 build
        // via .dynsym
        let symbol_section = require_with!(
            elf.file.find_section_by_name(".symtab"),
            "binary has no .symtab section"
        );
        let symbol_table = symbol_section.get_data(&elf.file)?;
        let sym_entries = match symbol_table {
            SectionData::SymbolTable64(entries) => entries,
            _ => bail!(
                "expected .symtab to be a SymbolTable64, got: {:?}",
                symbol_table
            ),
        };

        let vbase = kernel.largest_gap.start;

        let mut syms = sym_entries
            .iter()
            .filter(|sym| sym.shndx() != SHN_UNDEF)
            .map(|sym| {
                let name = try_core_res!(sym.get_name(&elf.file), "cannot get name of function");
                Ok((name, vbase + sym.value() as usize))
            })
            .collect::<Result<HashMap<_, _>>>()?;

        syms.insert("VMSH_STAGE1_PC", return_address);

        Ok(Loader {
            kernel,
            virt_mem: None,
            load_offsets: vec![],
            allocator,
            loadables: vec![],
            binary,
            elf,
            dyn_syms,
            init_func: *require_with!(syms.get("_init_vmsh"), "no _init_vmsh symbol found"),
            vmsh_stage1_args: *require_with!(
                syms.get("VMSH_STAGE1_ARGS"),
                "no cleanup_vmsh_stage1 symbol found"
            ),
            lib_syms: syms,
            string_arg_size: 0,
        })
    }

    fn upload_binary(&self) -> Result<()> {
        let mut local_iovec = vec![];
        let mut remote_iovec = vec![];
        let mut len = 0;
        for l in self.loadables.iter() {
            local_iovec.push(IoSlice::new(&l.content));
            len += l.content.len();
            remote_iovec.push(RemoteIoVec {
                base: l.mapping.phys_start.host_addr() + l.virt_offset,
                len: l.content.len(),
            });
        }
        let written = try_with!(
            process_vm_writev(
                self.allocator.hv.pid,
                local_iovec.as_slice(),
                remote_iovec.as_slice()
            ),
            "cannot write to process"
        );
        if written != len {
            bail!("short write, expected {}, written: {}", len, written);
        }
        Ok(())
    }

    fn vbase(&self) -> usize {
        self.kernel.largest_gap.start
    }

    fn write_stage1_args(
        &mut self,
        command: &[String],
        irq_num: usize,
        mmio_ranges: Vec<u64>,
    ) -> Result<(DeviceStatus, DriverStatus)> {
        let virt_mem = require_with!(self.virt_mem.as_ref(), "no virtual memory assigned");
        let string_mapping =
            require_with!(virt_mem.mappings.last(), "no virtual mappings found").clone();

        let mut strings: Vec<u8> = Vec::with_capacity(self.string_arg_size);

        let mut argv = command
            .iter()
            .map(|arg| {
                let ptr = strings.len() + string_mapping.virt_start;
                strings.extend_from_slice(arg.as_bytes());
                // make string null-terminated
                strings.push(b'\0');
                ptr as *mut libc::c_char
            })
            .collect::<Vec<_>>();

        self.loadables.push(Loadable {
            content: strings,
            mapping: string_mapping,
            virt_offset: 0,
        });
        // make argv null-terminated
        argv.push(ptr::null_mut());

        let addr = self.vmsh_stage1_args;
        let loadable = require_with!(
            find_loadable(&mut self.loadables, addr),
            "could not find elf loadable for vmsh_stage1_args"
        );
        let start = addr - (loadable.mapping.virt_start + loadable.virt_offset);
        let range = start..(start + size_of::<Stage1Args>());
        if range.end > loadable.content.len() {
            if range.end > loadable.mapping.len {
                bail!(
                    "stage1 args exceeds section by {:#x} bytes",
                    loadable.mapping.len - range.end
                );
            }
            loadable.content.resize(range.end, 0);
        }
        let stage1_args = loadable.content[range].as_mut_ptr() as *mut Stage1Args;
        let stage1_args = unsafe { &mut (*stage1_args) };

        stage1_args.argv[0..argv.len()].clone_from_slice(argv.as_slice());
        stage1_args.device_addrs[0..mmio_ranges.len()].clone_from_slice(&mmio_ranges);
        stage1_args.device_status = DeviceState::Initializing;
        stage1_args.irq_num = irq_num;

        let stage1_args_addr = stage1_args as *const Stage1Args as usize;

        let dev_offset =
            &stage1_args.device_status as *const DeviceState as usize - stage1_args_addr;
        let drv_offset =
            &stage1_args.driver_status as *const DeviceState as usize - stage1_args_addr;
        let host_offset =
            addr - loadable.mapping.virt_start + loadable.mapping.phys_start.host_addr();
        Ok((
            DeviceStatus {
                host_addr: host_offset + dev_offset,
            },
            DriverStatus {
                host_addr: host_offset + drv_offset,
            },
        ))
    }

    pub fn load_binary(
        &mut self,
        command: &[String],
        irq_num: usize,
        mmio_ranges: Vec<u64>,
    ) -> Result<(VirtMem, DeviceStatus, DriverStatus)> {
        let binary = try_core_res!(ElfBinary::new(self.binary), "cannot parse elf binary");

        self.string_arg_size = page_align(command.iter().map(|c| c.len() + 1).sum());
        try_core_res!(binary.load(self), "cannot load elf binary");

        let (device_status, driver_status) = try_with!(
            self.write_stage1_args(command, irq_num, mmio_ranges),
            "failed to write stage1 arguments"
        );

        try_with!(self.upload_binary(), "failed to upload binary to vm");
        let mem = require_with!(self.virt_mem.take(), "BUG, no virtual memory assigned");
        Ok((mem, device_status, driver_status))
    }
}

type ElfResult = std::result::Result<(), ElfLoaderErr>;

struct Loadable {
    content: Vec<u8>,
    mapping: MappedMemory,
    virt_offset: usize,
}

macro_rules! try_elf {
    ($expr: expr, $str: expr) => {
        match $expr {
            Ok(val) => val,
            Err(e) => {
                error!("{}: {}", $str, e);
                return Err(ElfLoaderErr::ElfParser { source: $str });
            }
        }
    };
}
macro_rules! require_elf {
    ($expr: expr, $str: expr) => {
        match $expr {
            Some(val) => val,
            None => {
                return Err(ElfLoaderErr::ElfParser { source: $str });
            }
        }
    };
}

fn resolve_symbol(
    name: &str,
    syms: &HashMap<String, usize>,
    lib_syms: &HashMap<&str, usize>,
) -> Option<usize> {
    if let Some(sym) = syms.get(name) {
        return Some(*sym);
    }

    if let Some(sym) = lib_syms.get(name) {
        return Some(*sym);
    }

    // usleep_range/_printk were introduced in linux 5.16
    if name == "usleep_range_state" {
        // not 100% api compatible but usleep_range_state just takes an
        // additional argument, that is ignored by usleep_range
        syms.get("usleep_range").copied()
    } else if name == "_printk" {
        syms.get("printk").copied()
    } else {
        None
    }
}

impl<'a> ElfLoader for Loader<'a> {
    fn allocate(&mut self, headers: LoadableHeaders) -> ElfResult {
        let allocs = headers.map(|h| {
            debug!(
                "allocate base = {:#x} size = {:#x} flags = {}",
                h.virtual_addr(),
                h.mem_size(),
                h.flags()
            );
            let mut prot = ProtFlags::PROT_READ;
            if h.flags().is_execute() {
                prot |= ProtFlags::PROT_EXEC;
            }
            if h.flags().is_write() {
                prot |= ProtFlags::PROT_WRITE;
            }
            let virtual_addr = h.virtual_addr() as usize;
            let start = page_start(virtual_addr);

            VirtAlloc {
                virt_start: self.vbase() + start,
                virt_offset: virtual_addr - start,
                len: page_align(virtual_addr + h.mem_size() as usize) - start,
                prot,
            }
        });
        let mut allocs = allocs.collect::<Vec<_>>();
        allocs.sort_by_key(|k| k.virt_start);
        let last_addr = match allocs.last() {
            Some(a) => a.virt_end(),
            None => {
                return Err(ElfLoaderErr::ElfParser {
                    source: "elf has no section",
                })
            }
        };

        // put strings for stage1 args before elf binary
        let last = VirtAlloc {
            virt_start: last_addr,
            virt_offset: 0,
            len: self.string_arg_size,
            prot: ProtFlags::PROT_READ | ProtFlags::PROT_WRITE,
        };
        if !LINUX_KERNEL_KASLR_RANGE.contains(&(last.virt_start + last.len)) {
            error!("virtual memory allocation ({:#x}-{:#x}) does not fit into kernel aslr range ({:#x}-{:#x}).",
                  last.virt_start, last.virt_start + last.len,
                  LINUX_KERNEL_KASLR_RANGE.start, LINUX_KERNEL_KASLR_RANGE.end
            );
            allocs.iter().for_each(|a| {
                error!(
                    "{:#x}-{:#x} ({:?})",
                    a.virt_start,
                    a.virt_start + a.len,
                    a.prot
                );
            });
            return Err(ElfLoaderErr::ElfParser {
                source: "virtual memory allocation does not fit into kernel aslr range",
            });
        }
        allocs.push(last);

        allocs.iter().for_each(|a| {
            if a.virt_start > self.vbase() {
                debug!(
                    "{:#x}-{:#x} ({:?})",
                    a.virt_start - self.vbase(),
                    a.virt_start + a.len - self.vbase(),
                    a.prot
                );
            }
        });

        if allocs.is_empty() {
            return Err(ElfLoaderErr::ElfParser {
                source: "no loadable sections found in elf file",
            });
        }
        self.virt_mem = Some(try_elf!(
            self.allocator.virt_alloc(&allocs),
            "cannot allocate memory"
        ));
        self.load_offsets = allocs.iter().map(|v| v.virt_offset).collect::<Vec<_>>();

        Ok(())
    }

    fn load(&mut self, flags: Flags, base: VAddr, region: &[u8]) -> ElfResult {
        let start = self.vbase() + base as usize;
        let end = self.vbase() + base as usize + region.len() as usize;
        debug!(
            "load region into = {:#x} -- {:#x} ({:?})",
            start, end, flags
        );
        let mem = require_elf!(
            self.virt_mem.as_ref(),
            "BUG: no virtual memory was allocated"
        );
        let (idx, mapping) = require_elf!(
            mem.mappings
                .iter()
                .enumerate()
                .find(|(_, mapping)| mapping.virt_start == page_start(start)),
            {
                error!(
                    "received loadable that was not allocated before at {:#x} ({:#x})",
                    start, base
                );
                "BUG: received loadable that was not allocated before"
            }
        );
        self.loadables.push(Loadable {
            content: region.to_vec(),
            mapping: mapping.clone(),
            virt_offset: self.load_offsets[idx],
        });
        Ok(())
    }

    fn relocate(&mut self, entry: RelocationEntry) -> ElfResult {
        let addr = self.vbase() + entry.offset as usize;
        let syms = &self.kernel.symbols;
        let lib_syms = &self.lib_syms;
        let vbase = self.vbase();
        let loadable = require_elf!(find_loadable(&mut self.loadables, addr), {
            error!(
                "no loadable found for relocation address: {:#x} ({:#x})",
                addr, entry.offset as usize
            );
            "no loadable found for relocation address"
        });
        let start = addr - (loadable.mapping.virt_start + loadable.virt_offset);

        match entry.rtype {
            RelocationType::x86_64(RelocationTypes::R_AMD64_RELATIVE) => {
                // This is a relative relocation, add the offset (where we put our
                // binary in the vspace) to the addend and we're done.
                let dest_addr = vbase + entry.addend.unwrap_or(0) as usize;
                debug!("R_RELATIVE *{:#x} = {:#x}", addr, dest_addr);
                let range = start..(start + size_of_val(&dest_addr));
                loadable.content[range].clone_from_slice(&dest_addr.to_ne_bytes());
                Ok(())
            }
            RelocationType::x86_64(RelocationTypes::R_AMD64_GLOB_DAT) => {
                let sym = &self.dyn_syms[entry.index as usize];
                if sym.get_binding()? == Binding::Weak {
                    // we have some weak symbols that are included by default
                    // but not used for anything in the kernel.
                    // Seem to be safe to ignore
                    return Ok(());
                }

                let sym_name = sym.get_name(&self.elf.file)?;
                debug!("R_GLOB_DAT *{:#x} = @ {}", addr, sym_name);
                let res = resolve_symbol(sym_name, syms, lib_syms);
                let symbol = require_elf!(res, {
                    error!("binary requires unknown symbol: {}", sym_name);
                    "cannot find symbol"
                });
                let dest_addr = (symbol + entry.addend.unwrap_or(0) as usize).to_ne_bytes();
                let range = start..(start + size_of_val(&symbol));
                loadable.content[range].clone_from_slice(&dest_addr);

                Ok(())
            }
            other => {
                warn!("loader: unhandled relocation: {:?}", other);
                Err(ElfLoaderErr::UnsupportedRelocationEntry)
            }
        }
    }

    fn tls(
        &mut self,
        _tdata_start: VAddr,
        _tdata_length: u64,
        _total_size: u64,
        _align: u64,
    ) -> std::result::Result<(), ElfLoaderErr> {
        Ok(())
    }

    fn make_readonly(
        &mut self,
        _base: VAddr,
        _size: usize,
    ) -> std::result::Result<(), ElfLoaderErr> {
        Ok(())
    }
}
