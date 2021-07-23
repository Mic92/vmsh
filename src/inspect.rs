//mod device;

use crate::guest_mem::GuestMem;
use crate::kernel::find_kernel;
use crate::result::Result;
use log::*;
use nix::unistd::Pid;
use simple_error::try_with;

use crate::kvm;

pub struct InspectOptions {
    pub pid: Pid,
}

pub fn inspect(opts: &InspectOptions) -> Result<()> {
    let vm = try_with!(
        kvm::hypervisor::get_hypervisor(opts.pid),
        "cannot get vms for process {}",
        opts.pid
    );
    vm.stop()?;

    for map in vm.get_maps()? {
        info!(
            "vm mem: 0x{:x} -> 0x{:x} (physical: 0x{:x}, flags: {:?} | {:?}) @@ {}",
            map.start, map.end, map.phys_addr, map.prot_flags, map.map_flags, map.pathname
        )
    }

    info!("vcpu maps");
    for map in vm.get_vcpu_maps()? {
        info!(
            "vm cpu mem: 0x{:x} -> 0x{:x} (physical: 0x{:x}, flags: {:?} | {:?}) @@ {}",
            map.start, map.end, map.phys_addr, map.prot_flags, map.map_flags, map.pathname
        );

        let map_ptr = map.start as *const kvm_bindings::kvm_run;
        let kvm_run: kvm_bindings::kvm_run =
            kvm::hypervisor::process_read(opts.pid, map_ptr as *const libc::c_void)?;
        info!("kvm_run: exit_reason {}", kvm_run.exit_reason);

        let reason_ptr: *const u32 = unsafe { &((*map_ptr).exit_reason) };
        let reason: u32 =
            kvm::hypervisor::process_read(opts.pid, reason_ptr as *const libc::c_void)?;
        info!("reason ptr = {:?}", reason_ptr);
        info!("reason = {}", reason);
    }

    let mem = GuestMem::new(&vm)?;

    match find_kernel(&mem, &vm) {
        Ok(kernel) => {
            let sections = &kernel.memory_sections;
            info!(
                "found kernel at 0x{:x}-0x{:x} (free space before: {} kib, free space after: {} kib)",
                kernel.range.start,
                kernel.range.end,
                kernel.space_before() / 1024,
                kernel.space_after() / 1024,
            );
            info!("kernel sections:");
            for m in sections {
                info!("0x{:x} ({}kb, {:?})", m.virt_start, m.len / 1024, m.prot)
            }
            info!("{} found kernel symbols", kernel.symbols.len());
        }
        Err(e) => info!("could not find kernel: {}", e),
    }

    Ok(())
}
