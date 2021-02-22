use bcc::perf_event::PerfMapBuilder;
use bcc::{BPFBuilder, Kprobe, BPF};
use core::slice::from_raw_parts as make_slice;
use libc::{c_ulong, size_t};
use nix::unistd::Pid;
use nix::unistd::{sysconf, SysconfVar};
use simple_error::bail;
use simple_error::try_with;
use std::sync::mpsc::channel;
use std::time::Duration;
use std::{fmt, ptr};

use crate::kvm::Hypervisor;
use crate::proc::{self, Mapping};
use crate::result::Result;

#[derive(Clone, Debug)]
#[repr(C)]
pub struct MemSlot {
    base_gfn: u64,
    npages: c_ulong,
    userspace_addr: c_ulong,
}

fn page_size() -> c_ulong {
    sysconf(SysconfVar::PAGE_SIZE).unwrap().unwrap() as u64
}

impl MemSlot {
    pub fn start(&self) -> u64 {
        self.userspace_addr
    }

    pub fn size(&self) -> u64 {
        self.npages * page_size()
    }

    pub fn end(&self) -> u64 {
        self.start() + self.size()
    }

    pub fn physical_start(&self) -> u64 {
        self.base_gfn * page_size()
    }
}

impl fmt::Display for MemSlot {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "Mapping {{ start=0x{:x}, end=0x{:x}, size=0x{:x}, physical_start=0x{:x} }}",
            self.start(),
            self.end(),
            self.size(),
            self.physical_start(),
        )
    }
}

const BPF_TEXT: &str = r#"
#include <linux/kvm_host.h>

struct memslot {
    gfn_t base_gfn;
    unsigned long npages;
    unsigned long userspace_addr;
};

typedef struct {
  size_t used_slots;
  struct memslot memslots[KVM_MEM_SLOTS_NUM];
} out_t;

BPF_PERCPU_ARRAY(slots, out_t, 1);

BPF_PERF_OUTPUT(memslots);

void kvm_vm_ioctl(struct pt_regs *ctx, struct file *filp) {
    struct kvm *kvm = (struct kvm *)filp->private_data;

    u32 pid = bpf_get_current_pid_tgid() >> 32;
    if (pid != TARGET_PID) {
        return;
    }

    u32 idx = 0;
    out_t *out = slots.lookup(&idx);
    if (!out) {
      return;
    }

    // On x86 there is also a second address space for system management mode in memslots[1]
    // however we dont care about about this one
    out->used_slots = kvm->memslots[0]->used_slots;
    for (size_t i = 0; i < KVM_MEM_SLOTS_NUM; i++) {
      struct kvm_memory_slot *in_slot = &kvm->memslots[0]->memslots[i];
      struct memslot *out_slot = &out->memslots[i];

      out_slot->base_gfn = in_slot->base_gfn;
      out_slot->npages = in_slot->npages;
      out_slot->userspace_addr = in_slot->userspace_addr;
    }
    memslots.perf_submit(ctx, out, sizeof(*out));
}"#;

fn bpf_prog(pid: Pid) -> Result<BPF> {
    let builder = try_with!(BPFBuilder::new(BPF_TEXT), "cannot compile bpf program");
    let cflags = &[format!("-DTARGET_PID={}", pid)];
    let builder_with_cflags = try_with!(builder.cflags(cflags), "could not pass cflags");
    Ok(try_with!(builder_with_cflags.build(), "build failed"))
}

pub fn get_maps(hv: Hypervisor) -> Result<Vec<Mapping>> {
    let mut module = bpf_prog(hv.pid)?;
    try_with!(
        Kprobe::new()
            .handler("kvm_vm_ioctl")
            .function("kvm_vm_ioctl")
            .attach(&mut module),
        "failed to install kprobe"
    );
    let table = try_with!(module.table("memslots"), "failed to get perf event table");

    let (sender, receiver) = channel();
    let builder = PerfMapBuilder::new(table, move || {
        let sender = sender.clone();
        Box::new(move |x| {
            let head = x.as_ptr() as *const size_t;
            let size = unsafe { ptr::read(head) };
            let memslots_slice = unsafe { make_slice(head.add(1) as *const MemSlot, size) };
            sender.send(memslots_slice.to_vec()).unwrap();
        })
    });
    let mut perf_map = try_with!(builder.build(), "could not install perf event handler");
    let tracee = hv.attach()?;
    try_with!(tracee.check_extension(0), "cannot query kvm extensions");

    perf_map.poll(0);
    let memslots = try_with!(
        receiver.recv_timeout(Duration::from_secs(0)),
        "could not receive memslots from kernel"
    );
    memslots
        .iter()
        .map(
            |slot| match proc::find_mapping(&hv.mappings, slot.start()) {
                Some(v) => Ok(v),
                None => bail!(
                    "No mapping of memslot {} found in hypervisor (/proc/{}/maps)",
                    slot,
                    hv.pid
                ),
            },
        )
        .collect()
}
