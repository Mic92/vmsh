// Copyright 2020 Amazon.com, Inc. or its affiliates. All Rights Reserved.
// Author of further modifications: Peter Okelmann
// SPDX-License-Identifier: Apache-2.0 OR BSD-3-Clause

use event_manager::{EventOps, Events, MutEventSubscriber};
use log::error;
use vm_memory::GuestAddressSpace;
use vmm_sys_util::epoll::EventSet;

use crate::devices::virtio::console::console_handler::ConsoleQueueHandler;
use crate::devices::virtio::SingleFdSignalQueue;
use crate::kvm::hypervisor::IoEventFd;

const RX_IOEVENT_DATA: u32 = 0;
const TX_IOEVENT_DATA: u32 = 1;

// This object simply combines the more generic `InOrderQueueHandler` with a concrete queue
// signalling implementation based on `EventFd`s, and then also implements `MutEventSubscriber`
// to interact with the event manager. `ioeventfd` is the `EventFd` connected to queue
// notifications coming from the driver.
pub(crate) struct QueueHandler<M: GuestAddressSpace> {
    pub inner: ConsoleQueueHandler<M, SingleFdSignalQueue>,
    pub rx_fd: IoEventFd,
    pub tx_fd: IoEventFd,
}

impl<M: GuestAddressSpace> QueueHandler<M> {
    fn handle_error<S: AsRef<str>>(&self, s: S, ops: &mut EventOps) {
        error!("{}", s.as_ref());
        ops.remove(Events::empty(&self.rx_fd))
            .expect("Failed to remove rx ioevent");
        ops.remove(Events::empty(&self.tx_fd))
            .expect("Failed to remove tx ioevent");
    }
}

impl<M: GuestAddressSpace> MutEventSubscriber for QueueHandler<M> {
    fn process(&mut self, events: Events, ops: &mut EventOps) {
        if events.event_set() != EventSet::IN {
            self.handle_error("Unexpected event_set", ops);
            return;
        }
        match events.data() {
            RX_IOEVENT_DATA => {
                if self.rx_fd.read().is_err() {
                    self.handle_error("Rx ioevent read", ops);
                } else if let Err(e) = self.inner.process_rxq() {
                    self.handle_error(format!("Process rx error {:?}", e), ops);
                }
            }
            TX_IOEVENT_DATA => {
                if self.tx_fd.read().is_err() {
                    self.handle_error("Tx ioevent read", ops);
                }
                if let Err(e) = self.inner.process_txq() {
                    self.handle_error(format!("Process tx error {:?}", e), ops);
                }
            }
            _ => self.handle_error("Unexpected data", ops),
        }
    }

    fn init(&mut self, ops: &mut EventOps) {
        ops.add(Events::with_data(
            &self.rx_fd,
            RX_IOEVENT_DATA,
            EventSet::IN,
        ))
        .expect("Failed to register rx ioeventfd for console queue handler");
        ops.add(Events::with_data(
            &self.tx_fd,
            TX_IOEVENT_DATA,
            EventSet::IN,
        ))
        .expect("Failed to register tx ioeventfd for console queue handler");
    }
}
