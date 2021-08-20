// Copyright 2020 Amazon.com, Inc. or its affiliates. All Rights Reserved.
// Author of further modifications: Peter Okelmann
// SPDX-License-Identifier: Apache-2.0 OR BSD-3-Clause

use std::fs::File;
use std::io::Read;
use std::result;

use event_manager::EventOps;
use event_manager::EventSet;
use event_manager::Events;
use event_manager::MutEventSubscriber;
use log::error;
use virtio_queue::{DescriptorChain, Queue};
use vm_memory::Bytes;
use vm_memory::{self, GuestAddressSpace};
use vmm_sys_util::eventfd::EventFd;

use crate::devices::virtio::SignalUsedQueue;
use crate::kvm::hypervisor::ioevent::IoEvent;
use super::device::{RX_QUEUE_IDX, TX_QUEUE_IDX};

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

pub(crate) struct LogQueueHandler<M: GuestAddressSpace, S: SignalUsedQueue> {
    pub tx_fd: IoEvent,
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

    /// Guest console sends (tx), we write to self.console fd
    fn process_tx_chain(&mut self, mut chain: DescriptorChain<M>) -> result::Result<(), Error> {
        log::debug!("process_chain");

        let mut i = 0;
        while let Some(desc) = chain.next() {
            log::debug!("chain.next()");
            let mem = chain.memory();
            if let Err(e) = mem.write_to(desc.addr(), &mut self.console, desc.len() as usize) {
                error!("error logging console tx TODO: {}", e)
            }
            i += 1;
        }
        self.txq.add_used(chain.head_index(), i as u32)?;

        if self.txq.needs_notification()? {
            log::debug!("notification needed: yes");
            self.driver_notify.signal_used_queue(0);
        } else {
            log::debug!("notification needed: no");
        }

        Ok(())
    }

    pub fn process_txq(&mut self) -> result::Result<(), Error> {
        // To see why this is done in a loop, please look at the `Queue::enable_notification`
        // comments in `vm_virtio`.
        loop {
            //log::warn!("loop");
            self.txq.disable_notification()?;

            while let Some(chain) = self.txq.iter()?.next() {
                self.process_tx_chain(chain)?;
            }

            if !self.txq.enable_notification()? {
                //log::warn!("loop break");
                break;
            }
        }
        Ok(())
    }

    /// Guest console reads (rx), we read from self.console fd
    fn process_rx_chain(&mut self, mut chain: DescriptorChain<M>) -> result::Result<(), Error> {
        log::debug!("process_chain");

        let mut i = 0;
        if let Some(desc) = chain.next() {
            log::warn!("reading {}", 1); //  desc.len());
            let mem = chain.memory();
            if let Err(e) = mem.read_from(desc.addr(), &mut self.console, 1) {
                error!("error logging console rx TODO: {}", e)
            }
            i += 1;
        }
        self.rxq.add_used(chain.head_index(), i as u32)?;

        if self.rxq.needs_notification()? {
            log::debug!("notification needed: yes");
            self.driver_notify.signal_used_queue(0);
        } else {
            log::debug!("notification needed: no");
        }

        Ok(())
    }

    pub fn process_rxq(&mut self) -> result::Result<(), Error> {
        // To see why this is done in a loop, please look at the `Queue::enable_notification`
        // comments in `vm_virtio`.
        //loop {
            log::debug!("loop");
            self.rxq.disable_notification()?;

            if let Some(chain) = self.rxq.iter()?.next() {
                self.process_rx_chain(chain)?;
            }

            if !self.rxq.enable_notification()? {
                log::debug!("loop break");
                //break;
            }
        //}
        Ok(())
    }
}

impl<M: GuestAddressSpace, S: SignalUsedQueue> MutEventSubscriber for LogQueueHandler<M, S> {
    fn process(&mut self, events: Events, ops: &mut EventOps) {
        if events.event_set() != EventSet::IN {
            self.handle_error("Unexpected event_set", ops);
            return;
        }

        match events.data() as u16 {
            RX_QUEUE_IDX => {
                log::warn!("rx");
                if let Err(e) = self.process_rxq() {
                    self.handle_error(format!("Process rx error {:?}", e), ops);
                }
            },
            TX_QUEUE_IDX => {
                if self.tx_fd.read().is_err() {
                    self.handle_error("Tx ioevent read", ops);
                }
                if let Err(e) = self.process_txq() {
                    self.handle_error(format!("Process tx error {:?}", e), ops);
                }
            },
            _ => self.handle_error("Unexpected data", ops)
        }
    }

    fn init(&mut self, ops: &mut EventOps) {
        ops
        .add(Events::with_data(
            &self.console,
            RX_QUEUE_IDX as u32,
            EventSet::IN,
        ))
        .expect("Failed to register rx ioeventfd for console queue handler");

        ops
        .add(Events::with_data(
            &self.tx_fd,
            TX_QUEUE_IDX as u32,
            EventSet::IN,
        ))
        .expect("Failed to register tx ioeventfd for console queue handler");
    }
}
