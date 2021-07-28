use std::mem::size_of_val;

use elfloader::{
    ElfBinary, ElfLoader, ElfLoaderErr, Entry, Flags, LoadableHeaders, Rela, TypeRela64, VAddr, P64,
};
use log::{debug, error, warn};
use nix::sys::mman::ProtFlags;
use nix::sys::uio::{process_vm_writev, IoVec, RemoteIoVec};
use simple_error::{bail, try_with};
use xmas_elf::sections::SectionData;
use xmas_elf::symbol_table::{Binding, DynEntry64};

use crate::guest_mem::MappedMemory;
use crate::kernel::Kernel;
use crate::kvm::allocator::VirtAlloc;
use crate::kvm::PhysMemAllocator;
use crate::page_math::{page_align, page_start};
use crate::page_table::VirtMem;
use crate::result::Result;

pub struct Loader<'a> {
    kernel: &'a Kernel,
    virt_mem: Option<VirtMem>,
    load_offsets: Vec<usize>,
    allocator: &'a mut PhysMemAllocator,
    loadables: Vec<Loadable>,
    binary: &'a [u8],
    elf: ElfBinary<'a>,
    dyn_syms: &'a [DynEntry64],
}

impl<'a> Loader<'a> {
    pub fn new(
        binary: &'a [u8],
        kernel: &'a Kernel,
        allocator: &'a mut PhysMemAllocator,
    ) -> Result<Loader<'a>> {
        let elf = match ElfBinary::new(binary) {
            Err(e) => bail!("cannot parse elf binary: {}", e),
            Ok(v) => v,
        };
        let dyn_symbol_section = elf.file.find_section_by_name(".dynsym").unwrap();
        let dyn_symbol_table = dyn_symbol_section.get_data(&elf.file)?;
        let dyn_syms = match dyn_symbol_table {
            SectionData::DynSymbolTable64(entries) => entries,
            _ => bail!(
                "expected .dynsym to be a DynSymbolTable64, got: {:?}",
                dyn_symbol_table
            ),
        };

        Ok(Loader {
            kernel,
            virt_mem: None,
            load_offsets: vec![],
            loadables: vec![],
            allocator,
            binary,
            elf,
            dyn_syms,
        })
    }

    fn upload_binary(&self) -> Result<()> {
        let mut local_iovec = vec![];
        let mut remote_iovec = vec![];
        let mut len = 0;
        for l in self.loadables.iter() {
            local_iovec.push(IoVec::from_slice(&l.content));
            len += l.mapping.len;
            remote_iovec.push(RemoteIoVec {
                base: l.mapping.phys_start.host_addr(),
                len: l.mapping.len,
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
        self.kernel.range.end
    }

    pub fn load_binary(&mut self) -> Result<VirtMem> {
        let binary = match ElfBinary::new(self.binary) {
            Err(e) => bail!("cannot parse elf binary: {}", e),
            Ok(v) => v,
        };

        if let Err(e) = binary.load(self) {
            bail!("cannot load elf binary: {}", e);
        };
        try_with!(self.upload_binary(), "failed to upload binary to vm");
        Ok(self.virt_mem.take().unwrap())
    }
}

type ElfResult = std::result::Result<(), ElfLoaderErr>;

struct Loadable {
    content: Vec<u8>,
    mapping: MappedMemory,
    load_offset: usize,
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
        //allocs.iter().for_each(|a| {
        //    info!("{:#x}", a.virt_start);
        //});

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
                    "received loadable that was not allocated before at {:#x}",
                    start
                );
                "BUG: received loadable that was not allocated before"
            }
        );
        self.loadables.push(Loadable {
            content: region.to_vec(),
            mapping: mapping.clone(),
            load_offset: self.load_offsets[idx],
        });
        Ok(())
    }

    fn relocate(&mut self, entry: &Rela<P64>) -> ElfResult {
        let typ = TypeRela64::from(entry.get_type());
        let addr = self.vbase() + entry.get_offset() as usize;
        let syms = &self.kernel.symbols;
        let vbase = self.vbase();
        let loadable = self
            .loadables
            .iter_mut()
            .find(|loadable| loadable.mapping.contains(addr));
        let loadable = require_elf!(loadable, {
            error!("no loadable found for relocation address: {}", addr);
            "no loadable found for relocation address: {}"
        });
        let start = addr - (loadable.mapping.virt_start + loadable.load_offset);

        match typ {
            TypeRela64::R_RELATIVE => {
                // This is a relative relocation, add the offset (where we put our
                // binary in the vspace) to the addend and we're done.
                let dest_addr = vbase + entry.get_addend() as usize;
                debug!("R_RELATIVE *{:#x} = {:#x}", addr, dest_addr);
                let range = start..(start + size_of_val(&dest_addr));
                loadable.content[range].clone_from_slice(&dest_addr.to_ne_bytes());
                Ok(())
            }
            TypeRela64::R_GLOB_DAT => {
                let sym = &self.dyn_syms[entry.get_symbol_table_index() as usize];
                if sym.get_binding()? == Binding::Weak {
                    // we have some weak symbols that are included by default
                    // but not used for anything in the kernel.
                    // Seem to be safe to ignore
                    return Ok(());
                }

                let sym_name = sym.get_name(&self.elf.file)?;
                debug!("R_GLOB_DAT *{:#x} = @ {}", addr, sym_name);

                let symbol = require_elf!(syms.get(sym_name), {
                    error!("binary contains unknown symbol: {}", sym_name);
                    "cannot find symbol"
                });
                let dest_addr = (symbol + entry.get_addend() as usize).to_ne_bytes();
                let range = start..(start + size_of_val(symbol));
                loadable.content[range].clone_from_slice(&dest_addr);

                Ok(())
            }
            other => {
                warn!("loader: unhandled relocation: {:?}", other);
                Err(ElfLoaderErr::UnsupportedRelocationEntry)
            }
        }
    }
}
