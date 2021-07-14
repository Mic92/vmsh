// Copyright 2020 Amazon.com, Inc. or its affiliates. All Rights Reserved.
// Author of further modifications: Peter Okelmann
// SPDX-License-Identifier: Apache-2.0 OR BSD-3-Clause

// We're only providing virtio over MMIO devices for now, but we aim to add PCI support as well.

pub mod block;
pub mod console;

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::kvm::hypervisor::{Hypervisor, IoEventFd};
use crate::result::Result;
use crate::tracer::inject_syscall;
use crate::tracer::wrap_syscall::KvmRunWrapper;
use event_manager::{EventManager, MutEventSubscriber};
use log::error;

use simple_error::try_with;
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
    pub ack_handler: Arc<Mutex<IrqAckHandler>>,
}

impl SignalUsedQueue for SingleFdSignalQueue {
    fn signal_used_queue(&self, _index: u16) {
        log::trace!("irqfd << {}", _index);
        self.interrupt_status
            .fetch_or(VIRTIO_MMIO_INT_VRING, Ordering::SeqCst);
        if let Err(e) = self.irqfd.write(1) {
            error!("Failed write to eventfd when signalling queue: {}", e);
        } else {
            match self.ack_handler.lock() {
                Ok(mut handler) => handler.irq_sent(),
                Err(e) => error!("Failed to lock IrqAckHandler: {}", e),
            }
        }
    }
}

/// Note: `device::threads::EVENT_LOOP_TIMEOUT_MS` typically determines how often the irq ack
/// timeout is handled and thus is typically the lower bound.
const INTERRUPT_ACK_TIMEOUT: Duration = Duration::from_millis(1);

pub struct IrqAckHandler {
    last_sent: Instant,
    interrupt_status: Arc<AtomicU8>,
    irqfd: Arc<EventFd>,
    total_sent: usize,
    total_ack_timeouted: usize,
}

impl IrqAckHandler {
    pub fn new(interrupt_status: Arc<AtomicU8>, irqfd: Arc<EventFd>) -> Self {
        IrqAckHandler {
            last_sent: Instant::now(),
            interrupt_status,
            irqfd,
            total_sent: 0,
            total_ack_timeouted: 0,
        }
    }

    /// Must be called whenever a new irq is sent for which an ack is expected.
    pub fn irq_sent(&mut self) {
        self.total_sent += 1;
        self.last_sent = Instant::now();
    }

    /// Must be called regularly to handle ack timeouts and re-send irqs.
    pub fn handle_timeouts(&mut self) {
        let passed = Instant::now().duration_since(self.last_sent);
        let unacked = self.interrupt_status.load(Ordering::Acquire) != 0;
        if passed >= INTERRUPT_ACK_TIMEOUT && unacked {
            // interrupt timed out && has not been acked
            if let Err(e) = self.irqfd.write(1) {
                log::error!("Failed write to eventfd when signalling queue: {}", e);
            } else {
                self.total_ack_timeouted += 1;
                log::debug!(
                    "re-sending lost interrupt after {:.1}ms. Total lost {:.0}% ({}/{})",
                    passed.as_micros() as f64 / 1000.0,
                    100.0 * self.total_ack_timeouted as f64 / self.total_sent as f64,
                    self.total_ack_timeouted,
                    self.total_sent,
                );
            }
        }
    }
}

// TODO move all of the following to kvm::?hypervisor?

use crate::kvm::hypervisor::{IoEvent};
use crate::devices::USE_IOREGIONFD;
use vmm_sys_util::eventfd::{EFD_NONBLOCK};
use std::os::unix::io::AsRawFd;
pub fn _register_ioevent(
    vmm: &Arc<Hypervisor>,
    mmio_cfg: &MmioConfig,
    queue_idx: u64,
) -> Result<IoEvent> {
    if !USE_IOREGIONFD {
        let ioeventfd = register_ioeventfd(vmm, mmio_cfg, queue_idx)?;
        Ok(IoEvent::IoEventFd(ioeventfd))
    } else {
        let eventfd = try_with!(EventFd::new(EFD_NONBLOCK), "foo");
        log::info!(
            "eventfd {:?} for ioregionfd",
            eventfd.as_raw_fd(),
        );
        Ok(IoEvent::EventFd(eventfd))
    }
}

use log::*;
pub fn register_ioeventfd_ioregion(
    vmm: &Arc<Hypervisor>,
    mmio_cfg: &MmioConfig,
    queue_idx: u64,
) -> Result<IoEventFd> {
    warn!("ioeventfd 1");
    vmm.stop()?;
    // we need to drop tracee for ioeventfd_
    let ret = vmm.ioeventfd_(
        mmio_cfg.range.base().0 + VIRTIO_MMIO_QUEUE_NOTIFY_OFFSET,
        4,
        Some(queue_idx),
    );
    warn!("ioeventfd 2");
    vmm.resume()?;
    ret
}
pub fn register_ioeventfd(
    vmm: &Arc<Hypervisor>,
    mmio_cfg: &MmioConfig,
    queue_idx: u64,
) -> Result<IoEventFd> {
    // Register the queue event fd. Something like this, but in a pirate fashion.
    // let ioeventfd = EventFd::new(EFD_NONBLOCK).map_err(Error::EventFd)?;
    // self.vm_fd
    //     .register_ioevent(
    //         &ioeventfd,
    //         &IoEventAddress::Mmio(
    //             self.mmio_cfg.range.base().0 + VIRTIO_MMIO_QUEUE_NOTIFY_OFFSET,
    //         ),
    //         0u32,
    //     )
    //     .map_err(Error::RegisterIoevent)?;
    let mut wrapper_go = try_with!(vmm.wrapper.lock(), "cannot obtain wrapper mutex");

    // wrapper -> injector
    {
        let wrapper = wrapper_go.take().unwrap();
        let mut tracee = vmm.tracee_write_guard()?;

        let err = "cannot re-attach injector after having detached it favour of KvmRunWrapper";
        let injector = try_with!(inject_syscall::from_tracer(wrapper.into_tracer()?), err);
        try_with!(tracee.attach_to(injector), &err);
    }

    // we need to drop tracee for ioeventfd_
    let ioeventfd = vmm.ioeventfd_(
        mmio_cfg.range.base().0 + VIRTIO_MMIO_QUEUE_NOTIFY_OFFSET,
        4,
        Some(queue_idx),
    )?;

    // injector -> wrapper
    {
        let mut tracee = vmm.tracee_write_guard()?;
        // we may unwrap because we just attached it.
        let injector = tracee.detach().unwrap();
        let wrapper = KvmRunWrapper::from_tracer(inject_syscall::into_tracer(
            injector,
            vmm.vcpu_maps[0].clone(),
        )?)?;
        let _ = wrapper_go.replace(wrapper);
    }
    Ok(ioeventfd)
}
