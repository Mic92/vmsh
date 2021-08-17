use std::fs::File;

use event_manager::MutEventSubscriber;
use virtio_queue::Queue;
use vm_memory::GuestAddressSpace;

use crate::{devices::virtio::SignalUsedQueue, kvm::hypervisor::ioevent::IoEvent};

pub struct StdinStdoutHandler<M: GuestAddressSpace, S: SignalUsedQueue> {
    /// ioevent fd to indicate new data added to the tx queue
    pub tx_fd: IoEvent,
    /// Notify driver about used buffers
    pub driver_notify: S,
    /// rx queue for sending data to the guest
    pub rxq: Queue<M>,
    /// tx queue for receiving data from the guest
    pub txq: Queue<M>,
    /// host side stream
    pub console: File,
}

impl<M: GuestAddressSpace, S: SignalUsedQueue> StdinStdoutHandler<M, S> {}

impl<M: GuestAddressSpace, S: SignalUsedQueue> MutEventSubscriber for StdinStdoutHandler<M, S> {
    fn process(&mut self, events: event_manager::Events, ops: &mut event_manager::EventOps) {}

    fn init(&mut self, ops: &mut event_manager::EventOps) {}
}
