// Copyright 2020 Amazon.com, Inc. or its affiliates. All Rights Reserved.
// Author of further modifications: Peter Okelmann
// SPDX-License-Identifier: Apache-2.0 OR BSD-3-Clause

use std::fs::File;
use std::result;

use event_manager::EventOps;
use event_manager::EventSet;
use event_manager::Events;
use event_manager::MutEventSubscriber;
use log::error;
use virtio_queue::{DescriptorChain, Queue};
use vm_memory::Bytes;
use vm_memory::{self, GuestAddressSpace};

use crate::devices::virtio::SignalUsedQueue;
use crate::kvm::hypervisor::IoEventFd;

#[derive(Debug)]
pub enum Error {
    GuestMemory(vm_memory::GuestMemoryError),
    Queue(virtio_queue::Error),
}

impl From<vm_memory::GuestMemoryError> for Error {
    fn from(e: vm_memory::GuestMemoryError) -> Self {
        Error::GuestMemory(e)
    }
}

impl From<virtio_queue::Error> for Error {
    fn from(e: virtio_queue::Error) -> Self {
        Error::Queue(e)
    }
}

const TX_IOEVENT_DATA: u32 = 1;

pub(crate) struct LogQueueHandler<M: GuestAddressSpace, S: SignalUsedQueue> {
    pub tx_fd: IoEventFd,
    pub driver_notify: S,
    #[allow(unused)]
    pub rxq: Queue<M>,
    pub txq: Queue<M>,
    pub console: File,
}

impl<M, S> LogQueueHandler<M, S>
where
    M: GuestAddressSpace,
    S: SignalUsedQueue,
{
    fn handle_error<Msg: AsRef<str>>(&self, s: Msg, ops: &mut EventOps) {
        error!("{}", s.as_ref());
        ops.remove(Events::empty(&self.tx_fd))
            .expect("Failed to remove tx ioevent");
    }

    fn process_chain(&mut self, mut chain: DescriptorChain<M>) -> result::Result<(), Error> {
        log::trace!("process_chain");

        let mut i = 0;
        while let Some(desc) = chain.next() {
            let mem = chain.memory();
            if let Err(e) = mem.write_to(desc.addr(), &mut self.console, desc.len() as usize) {
                error!("error logging console: {}", e)
            }
            i += 1;
        }
        self.txq.add_used(chain.head_index(), i as u32)?;

        if self.txq.needs_notification()? {
            log::trace!("notification needed: yes");
            self.driver_notify.signal_used_queue(0);
        } else {
            log::trace!("notification needed: no");
        }

        Ok(())
    }

    pub fn process_txq(&mut self) -> result::Result<(), Error> {
        // To see why this is done in a loop, please look at the `Queue::enable_notification`
        // comments in `vm_virtio`.
        loop {
            self.txq.disable_notification()?;

            while let Some(chain) = self.txq.iter()?.next() {
                self.process_chain(chain)?;
            }

            if !self.txq.enable_notification()? {
                break;
            }
        }
        Ok(())
    }
}

impl<M: GuestAddressSpace, S: SignalUsedQueue> MutEventSubscriber for LogQueueHandler<M, S> {
    fn process(&mut self, events: Events, ops: &mut EventOps) {
        if events.event_set() != EventSet::IN {
            self.handle_error("Unexpected event_set", ops);
            return;
        }

        if TX_IOEVENT_DATA == events.data() {
            if self.tx_fd.read().is_err() {
                self.handle_error("Tx ioevent read", ops);
            }
            if let Err(e) = self.process_txq() {
                self.handle_error(format!("Process tx error {:?}", e), ops);
            }
        } else {
            self.handle_error("Unexpected data", ops)
        }
    }

    fn init(&mut self, ops: &mut EventOps) {
        ops.add(Events::with_data(
            &self.tx_fd,
            TX_IOEVENT_DATA,
            EventSet::IN,
        ))
        .expect("Failed to register tx ioeventfd for console queue handler");
    }
}
