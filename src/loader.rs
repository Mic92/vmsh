use elfloader::{
    ElfBinary, ElfLoader, ElfLoaderErr, Entry, Flags, LoadableHeaders, Rela, TypeRela64, VAddr, P64,
};
use log::{debug, warn};
use simple_error::bail;
use xmas_elf::{
    sections::SectionData,
    symbol_table::{Binding, DynEntry64},
};

use crate::result::Result;

pub struct Loader<'a> {
    vbase: u64,
    binary: &'a [u8],
    elf: ElfBinary<'a>,
    dyn_syms: &'a [DynEntry64],
}

impl<'a> Loader<'a> {
    pub fn new(binary: &[u8]) -> Result<Loader> {
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
            vbase: 0,
            binary,
            elf,
            dyn_syms,
        })
    }
    pub fn load_binary(&mut self) -> Result<()> {
        let binary = match ElfBinary::new(self.binary) {
            Err(e) => bail!("cannot parse elf binary: {}", e),
            Ok(v) => v,
        };

        if let Err(e) = binary.load(self) {
            bail!("cannot load elf binary: {}", e);
        };
        Ok(())
    }
}

type ElfResult = std::result::Result<(), ElfLoaderErr>;

impl<'a> ElfLoader for Loader<'a> {
    fn allocate(&mut self, load_headers: LoadableHeaders) -> ElfResult {
        for header in load_headers {
            debug!(
                "allocate base = {:#x} size = {:#x} flags = {}",
                header.virtual_addr(),
                header.mem_size(),
                header.flags()
            );
        }
        Ok(())
    }

    fn load(&mut self, flags: Flags, base: VAddr, region: &[u8]) -> ElfResult {
        let start = self.vbase + base;
        let end = self.vbase + base + region.len() as u64;
        debug!(
            "load region into = {:#x} -- {:#x} ({:?})",
            start, end, flags
        );
        Ok(())
    }

    fn relocate(&mut self, entry: &Rela<P64>) -> ElfResult {
        let typ = TypeRela64::from(entry.get_type());
        let addr: *mut u64 = (self.vbase + entry.get_offset()) as *mut u64;

        match typ {
            TypeRela64::R_RELATIVE => {
                // This is a relative relocation, add the offset (where we put our
                // binary in the vspace) to the addend and we're done.
                debug!(
                    "R_RELATIVE *{:p} = {:#x}",
                    addr,
                    self.vbase + entry.get_addend()
                );
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

                debug!("R_GLOB_DAT *{:p} = @ {}", addr, sym_name);

                Ok(())
            }
            other => {
                warn!("loader: unhandled relocation: {:?}", other);
                Err(ElfLoaderErr::UnsupportedRelocationEntry)
            }
        }
    }
}
