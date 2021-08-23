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
            "vm mem: {:#x} -> {:#x} (physical: {:#x}, flags: {:?} | {:?}) @@ {}",
            map.start, map.end, map.phys_addr, map.prot_flags, map.map_flags, map.pathname
        )
    }

    info!("vcpu maps");
    for map in vm.get_vcpu_maps()? {
        info!(
            "vm cpu mem: {:#x} -> {:#x} (physical: {:#x}, flags: {:?} | {:?}) @@ {}",
            map.start, map.end, map.phys_addr, map.prot_flags, map.map_flags, map.pathname
        );

        let map_ptr = map.start as *const kvm_bindings::kvm_run;
        let kvm_run: kvm_bindings::kvm_run =
            kvm::hypervisor::memory::process_read(opts.pid, map_ptr as *const libc::c_void)?;
        info!("kvm_run: exit_reason {}", kvm_run.exit_reason);

        let reason_ptr: *const u32 = unsafe { &((*map_ptr).exit_reason) };
        let reason: u32 =
            kvm::hypervisor::memory::process_read(opts.pid, reason_ptr as *const libc::c_void)?;
        info!("reason ptr = {:?}", reason_ptr);
        info!("reason = {}", reason);
    }

    let mem = GuestMem::new(&vm)?;

    match find_kernel(&mem, &vm) {
        Ok(kernel) => {
            let sections = &kernel.memory_sections;
            info!(
                "found kernel at {:#x}-{:#x} (free space before: {} kib, free space after: {} kib)",
                kernel.range.start,
                kernel.range.end,
                kernel.space_before() / 1024,
                kernel.space_after() / 1024,
            );
            info!("kernel sections:");
            for m in sections {
                info!("{:#x} ({}kb, {:?})", m.virt_start, m.len / 1024, m.prot)
            }
            info!("{} found kernel symbols", kernel.symbols.len());
        }
        Err(e) => info!("could not find kernel: {}", e),
    }

    let pic1 = vm.get_irqchip(0)?;
    info!("pic1: {:?}", unsafe { pic1.chip.pic });
    let pic2 = vm.get_irqchip(1)?;
    info!("pic2: {:?}", unsafe { pic2.chip.pic });
    let ioapic = vm.get_irqchip(2)?;
    let ioa = unsafe { ioapic.chip.ioapic };
    info!(
        "ioapic: base_address={:x} ioregsel={:x} id={:x} irr={:x}",
        ioa.base_address, ioa.ioregsel, ioa.id, ioa.irr
    );
    // this is quite verbose
    //for (i, field) in ioa.redirtbl.iter().enumerate() {
    //    info!("ioapic[{}]=bits={:x}: fields={:?}", i, unsafe { field.bits }, unsafe { field.fields });
    //}

    Ok(())
}
