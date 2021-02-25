// Copyright 2020 Amazon.com, Inc. or its affiliates. All Rights Reserved.
// Author of further modifications: Peter Okelmann
// SPDX-License-Identifier: Apache-2.0 OR BSD-3-Clause

// We're only providing virtio over MMIO devices for now, but we aim to add PCI support as well.

pub mod block;

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use event_manager::{EventManager, MutEventSubscriber};
//use kvm_ioctls::VmFd; FIXME remove
use crate::kvm::Hypervisor;

//use linux_loader::cmdline::Cmdline; FIXME remove
use vm_device::bus::MmioRange;
use vmm_sys_util::eventfd::EventFd;

// TODO: Move virtio-related defines from the local modules to the `vm-virtio` crate upstream.

// TODO: Add MMIO-specific module when we add support for something like PCI as well.

// Device-independent virtio features.
mod features {
    pub const VIRTIO_F_RING_EVENT_IDX: u64 = 29;
    pub const VIRTIO_F_VERSION_1: u64 = 32;
    pub const VIRTIO_F_IN_ORDER: u64 = 35;
}

// This bit is set on the device interrupt status when notifying the driver about used
// queue events.
// TODO: There seem to be similar semantics when the PCI transport is used with MSI-X cap
// disabled. Let's figure out at some point if having MMIO as part of the name is necessary.
const VIRTIO_MMIO_INT_VRING: u8 = 0x01;

// The driver will write to the register at this offset in the MMIO region to notify the device
// about available queue events.
const VIRTIO_MMIO_QUEUE_NOTIFY_OFFSET: u64 = 0x50;

// TODO: Make configurable for each device maybe?
const QUEUE_MAX_SIZE: u16 = 256;

#[derive(Copy, Clone)]
pub struct MmioConfig {
    pub range: MmioRange,
    // The interrupt assigned to the device.
    pub gsi: u32,
}

// These arguments are common for all virtio devices. We're always passing a mmio_cfg object
// for now, and we'll re-evaluate the layout of this struct when adding more transport options.
pub struct CommonArgs<'a, M, B> {
    // The objects used for guest memory accesses and other operations.
    pub mem: M,
    // Used by the devices to register ioevents and irqfds.
    //pub vm_fd: Arc<VmFd>, FIXME remove
    pub vmm: Arc<Hypervisor>,
    // Mutable handle to the event manager the device is supposed to register with. There could be
    // more if we decide to use more than just one thread for device model emulation.
    pub event_mgr: &'a mut EventManager<Arc<Mutex<dyn MutEventSubscriber + Send>>>,
    // This stands for something that implements `MmioManager`, and can be passed as a reference
    // or smart pointer (such as a `Mutex` guard).
    pub mmio_mgr: B,
    // The virtio MMIO device parameters (MMIO range and interrupt to be used).
    pub mmio_cfg: MmioConfig,
    // We pass a mutable reference to the kernel cmdline `String` so the device can add any
    // required arguments (i.e. for virtio over MMIO discovery). This means we need to create
    // the devices before loading he kernel cmdline into memory, but that's not a significant
    // limitation.
    //pub kernel_cmdline: &'a mut Cmdline, FIXME remove
}

/// Simple trait to model the operation of signalling the driver about used events
/// for the specified queue.
// TODO: Does this need renaming to be relevant for packed queues as well?
pub trait SignalUsedQueue {
    // TODO: Should this return an error? This failing is not really recoverable at the interface
    // level so the expectation is the implementation handles that transparently somehow.
    fn signal_used_queue(&self, index: u16);
}

/// Uses a single irqfd as the basis of signalling any queue (useful for the MMIO transport,
/// where a single interrupt is shared for everything).
pub struct SingleFdSignalQueue {
    pub irqfd: Arc<EventFd>,
    pub interrupt_status: Arc<AtomicU8>,
}

impl SignalUsedQueue for SingleFdSignalQueue {
    fn signal_used_queue(&self, _index: u16) {
        self.interrupt_status
            .fetch_or(VIRTIO_MMIO_INT_VRING, Ordering::SeqCst);
        self.irqfd
            .write(1)
            .expect("Failed write to eventfd when signalling queue");
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use vm_device::bus::MmioAddress;
    use vm_device::device_manager::IoManager;
    use vm_memory::{GuestAddress, GuestMemoryMmap};

    use super::*;

    pub type MockMem = Arc<GuestMemoryMmap>;

    // Can be used in other modules to test functionality that requires a `CommonArgs` struct as
    // input. The `args` method below generates an instance of `CommonArgs` based on the members
    // below.
    pub struct CommonArgsMock {
        pub mem: MockMem,
        //pub vm_fd: Arc<VmFd>, FIXME remove
        pub vmm: Arc<Hypervisor>,
        pub event_mgr: EventManager<Arc<Mutex<dyn MutEventSubscriber + Send>>>,
        pub mmio_mgr: IoManager,
        pub mmio_cfg: MmioConfig,
        //pub kernel_cmdline: Cmdline, FIXME remove
    }

    impl CommonArgsMock {
        pub fn new() -> Self {
            let mem =
                Arc::new(GuestMemoryMmap::from_ranges(&[(GuestAddress(0), 0x1000_0000)]).unwrap());

            let kvm = kvm_ioctls::Kvm::new().unwrap();
            let vm_fd = Arc::new(kvm.create_vm().unwrap());

            let range = MmioRange::new(MmioAddress(0x1_0000_0000), 0x1000).unwrap();
            let mmio_cfg = MmioConfig { range, gsi: 5 };

            // Required so the vm_fd can be used to register irqfds.
            vm_fd.create_irq_chip().unwrap();

            CommonArgsMock {
                mem,
                vm_fd,
                event_mgr: EventManager::new().unwrap(),
                mmio_mgr: IoManager::new(),
                mmio_cfg,
                // `4096` seems large enough for testing.
                //kernel_cmdline: Cmdline::new(4096), FIXME remove
            }
        }

        pub fn args(&mut self) -> CommonArgs<MockMem, &mut IoManager> {
            CommonArgs {
                mem: self.mem.clone(),
                vm_fd: self.vm_fd.clone(),
                event_mgr: &mut self.event_mgr,
                mmio_mgr: &mut self.mmio_mgr,
                mmio_cfg: self.mmio_cfg,
                kernel_cmdline: &mut self.kernel_cmdline,
            }
        }
    }
}
