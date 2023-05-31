use bcc::perf_event::PerfMapBuilder;
use bcc::{BPFBuilder, Kprobe, BPF};
use core::slice::from_raw_parts as make_slice;
use libc::{c_ulong, size_t};
use log::warn;
use nix::sys::utsname::uname;
use nix::unistd::Pid;
use simple_error::bail;
use simple_error::require_with;
use simple_error::simple_error;
use simple_error::try_with;
use std::sync::mpsc::channel;
use std::time::Duration;
use std::{fmt, ptr};

use crate::kvm::hypervisor;
use crate::result::Result;
use crate::tracer::proc::openpid;
use crate::tracer::proc::{self, Mapping};
use crate::{kvm::tracee::Tracee, page_math::page_size};

#[derive(Clone, Debug)]
#[repr(C)]
pub struct MemSlot {
    base_gfn: u64,
    npages: c_ulong,
    userspace_addr: c_ulong,
}

impl MemSlot {
    pub fn start(&self) -> usize {
        self.userspace_addr as usize
    }

    pub fn size(&self) -> usize {
        (self.npages as usize) * page_size()
    }

    pub fn end(&self) -> usize {
        self.start() + self.size()
    }

    pub fn physical_start(&self) -> usize {
        (self.base_gfn as usize) * page_size()
    }
}

impl fmt::Display for MemSlot {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "MemSlot {{ start={:#x}, end={:#x}, size={:#x}, physical_start={:#x}, physical_end = {:#x} }}",
            self.start(),
            self.end(),
            self.size(),
            self.physical_start(),
            self.physical_start() + self.size(),
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

// KVM_MEM_SLOTS_NUM became to big to handle it in ebpf
#define MAX_SLOTS 1024

typedef struct {
  size_t used_slots;
  struct memslot memslots[MAX_SLOTS];
} out_t;

BPF_PERCPU_ARRAY(slots, out_t, 1);
BPF_TABLE("array", int, struct rb_node*, queue, 128);

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

 #if KERNEL_MAJOR == 5 && KERNEL_MINOR >= 16 || KERNEL_MAJOR > 5
    // On x86 there is also a second address space for system management mode in memslots[1]
    // however we dont care about about this one
    struct rb_node *root = kvm->memslots[0]->gfn_tree.rb_node;
  	int node_idx = kvm->memslots[0]->node_idx;
    u32 head = 0, head_copy = 0, tail = 1;
    queue.update(&head, &root);

    for (uint32_t head = 0; head < MAX_SLOTS && head < tail; head++) {
        // create a copy so that the verifier can see that it is not modified
        head_copy = head;
        struct rb_node **node_ptr = queue.lookup(&head_copy);
        if (node_ptr == NULL) {
            break;
        }
        struct rb_node *node = *node_ptr;
        struct kvm_memory_slot *slot = 
          (struct kvm_memory_slot *)((void*)node - offsetof(struct kvm_memory_slot, gfn_node[node_idx]));
        if (out->used_slots >= MAX_SLOTS) {
          break;
        }
        struct memslot *out_slot = &out->memslots[out->used_slots];
        out_slot->base_gfn = slot->base_gfn;
        out_slot->npages = slot->npages;
        out_slot->userspace_addr = slot->userspace_addr;
        out->used_slots++;

        struct rb_node* left_child = node->rb_left;
        if (node->rb_left) {
          queue.update(&tail, &left_child);
          tail++;
        }

        struct rb_node* right_child = node->rb_right;
        if (node->rb_right) {
          queue.update(&tail, &right_child);
          tail++;
        }
    }
#else
    out->used_slots = kvm->memslots[0]->used_slots;
    for (size_t i = 0; i < MAX_SLOTS && i < out->used_slots; i++) {
      struct kvm_memory_slot *in_slot = &kvm->memslots[0]->memslots[i];
      struct memslot *out_slot = &out->memslots[i];
      
      out_slot->base_gfn = in_slot->base_gfn;
      out_slot->npages = in_slot->npages;
      out_slot->userspace_addr = in_slot->userspace_addr;
    }
#endif

    memslots.perf_submit(ctx, out, sizeof(*out));
}"#;

fn bpf_prog(pid: Pid) -> Result<BPF> {
    let uts_name = try_with!(uname(), "could not get uts name");
    let raw_kernel_release = uts_name.release().to_string_lossy();
    let kernel_release = try_with!(
        raw_kernel_release
            .split('.')
            .map(|s| s.parse::<u32>())
            .collect::<std::result::Result<Vec<u32>, _>>(),
        "could not parse kernel release: {:?}",
        raw_kernel_release
    );
    if kernel_release.len() < 2 {
        return Err(simple_error!(
            "kernel release has not enough numbers: {:?}",
            raw_kernel_release
        ));
    }

    let builder = try_with!(BPFBuilder::new(BPF_TEXT), "cannot compile bpf program");
    let cflags = &[
        format!("-DTARGET_PID={}", pid),
        format!("-DKERNEL_MAJOR={}", kernel_release[0]),
        format!("-DKERNEL_MINOR={}", kernel_release[1]),
    ];
    let builder_with_cflags = try_with!(builder.cflags(cflags), "could not pass cflags");
    Ok(try_with!(
        builder_with_cflags.build(),
        "build failed. This might happen if vmsh was started without root (or cap_sys_admin)"
    ))
}

pub fn fetch_mappings(pid: Pid) -> Result<Vec<Mapping>> {
    let handle = try_with!(openpid(pid), "cannot open handle in proc");
    let mappings = try_with!(handle.maps(), "cannot read process maps");
    Ok(mappings)
}

pub fn get_maps(tracee: &Tracee) -> Result<Vec<Mapping>> {
    let mut module = bpf_prog(tracee.pid())?;
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
            sender
                .send(memslots_slice.to_vec())
                .expect("failed send memslots back");
        })
    });
    let mut perf_map = try_with!(builder.build(), "could not install perf event handler");
    try_with!(tracee.check_extension(0), "cannot query kvm extensions");

    perf_map.poll(0);
    let memslots = try_with!(
        receiver.recv_timeout(Duration::from_secs(0)),
        "could not receive memslots from kernel"
    );
    if memslots.len() == 1024 {
        warn!(
            "Reached capacity of kvm memslots we can extract from the kernel.
We might miss physical memory allocations."
        );
    }
    let mappings = fetch_mappings(tracee.pid())?;
    memslots
        .iter()
        .map(|slot| match proc::find_mapping(&mappings, slot.start()) {
            Some(mut m) => {
                m.start = slot.start();
                m.end = slot.end();
                m.phys_addr = slot.physical_start();
                Ok(m)
            }
            None => bail!(
                "No mapping of memslot {} found in hypervisor (/proc/{}/maps)",
                slot,
                tracee.pid()
            ),
        })
        .collect()
}

/// ordered list of the hypervisor memory mapped to [vcpu0fd, vcpu1fd, ...]
pub fn get_vcpu_maps(pid: Pid) -> Result<Vec<Mapping>> {
    let mappings = fetch_mappings(pid)?;
    let vcpu_maps = mappings.into_iter().filter(|m| {
        m.pathname
            .starts_with(hypervisor::VCPUFD_INODE_NAME_STARTS_WITH)
    });

    // we need a for loop, because we can not return errors from within a .sort() lambda.
    let mut taged_maps = vec![]; // (vcpunr, vcpu_map)
    for vcpu_map in vcpu_maps {
        let ao: Option<&str> = vcpu_map
            .pathname
            .strip_prefix(hypervisor::VCPUFD_INODE_NAME_STARTS_WITH);
        let astr: &str = require_with!(
            ao,
            "vcpufd {} does not start with expected prefix",
            vcpu_map.pathname,
        );
        let ai = try_with!(
            astr.parse::<u64>(),
            "vcpufd {} has unexpected postfix {}",
            vcpu_map.pathname,
            astr,
        );
        taged_maps.push((ai, vcpu_map));
    }

    taged_maps.sort_unstable_by_key(|(i, _map)| *i);
    let sorted_maps = taged_maps.into_iter().map(|(_i, map)| map).collect();
    Ok(sorted_maps)
}
