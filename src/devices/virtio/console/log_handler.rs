// Copyright 2020 Amazon.com, Inc. or its affiliates. All Rights Reserved.
// Author of further modifications: Peter Okelmann
// SPDX-License-Identifier: Apache-2.0 OR BSD-3-Clause

use std::fs::File;
use std::io::{Read, Write};
use std::result;
use std::sync::Arc;

use event_manager::EventOps;
use event_manager::EventSet;
use event_manager::Events;
use event_manager::MutEventSubscriber;
use log::error;
use virtio_queue::Queue;
use virtio_queue::{QueueOwnedT, QueueT};
use vm_memory::{self, Bytes, GuestMemoryMmap};

use super::device::{RX_QUEUE_IDX, TX_QUEUE_IDX};
use crate::devices::virtio::SignalUsedQueue;
use crate::kvm::hypervisor::ioevent::IoEvent;

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

pub(crate) struct LogQueueHandler<S: SignalUsedQueue> {
    pub tx_fd: IoEvent,
    pub driver_notify: S,
    #[allow(unused)]
    pub rxq: Queue,
    pub txq: Queue,
    pub console_out: Box<dyn Write + Send>,
    pub console_in: Option<File>,
    pub mem: Arc<GuestMemoryMmap>,
}

impl<S> LogQueueHandler<S>
where
    S: SignalUsedQueue,
{
    fn handle_error<Msg: AsRef<str>>(&self, s: Msg, ops: &mut EventOps) {
        error!("{}", s.as_ref());
        ops.remove(Events::empty(&self.tx_fd))
            .expect("Failed to remove tx ioevent");
    }

    pub fn process_txq(&mut self) -> result::Result<(), Error> {
        // To see why this is done in a loop, please look at the `Queue::enable_notification`
        // comments in `vm_virtio`.
        loop {
            self.txq.disable_notification(self.mem.as_ref())?;

            // Guest console sends (tx), we write to self.console_out fd
            while let Some(mut chain) = self.txq.iter(self.mem.as_ref())?.next() {
                log::debug!("process_chain");

                let mut i = 0;
                while let Some(desc) = chain.next() {
                    log::debug!("chain.next()");
                    let mem = chain.memory();
                    if let Err(e) =
                        mem.write_to(desc.addr(), &mut self.console_out, desc.len() as usize)
                    {
                        error!("error logging console tx (stdout/err): {}", e)
                    }
                    i += 1;
                }
                self.txq
                    .add_used(self.mem.as_ref(), chain.head_index(), i as u32)?;

                if self.txq.needs_notification(self.mem.as_ref())? {
                    log::debug!("notification needed: yes");
                    self.driver_notify.signal_used_queue(0);
                } else {
                    log::debug!("notification needed: no");
                }
            }

            if !self.txq.enable_notification(self.mem.as_ref())? {
                break;
            }
        }
        Ok(())
    }

    pub fn process_rxq(&mut self) -> result::Result<(), Error> {
        // To see why this is done in a loop, please look at the `Queue::enable_notification`
        // comments in `vm_virtio`.
        //loop {
        log::debug!("loop");
        self.rxq.disable_notification(self.mem.as_ref())?;

        if let Some(mut chain) = self.rxq.iter(self.mem.as_ref())?.next() {
            // Guest console reads (rx), we read from self.console_in fd
            log::debug!("process_chain");
            const LEN: usize = 128;
            let mut count = 0;

            if let Some(desc) = chain.next() {
                log::debug!("reading bytes");
                let mem = chain.memory();
                let mut buf = [0u8; LEN];
                let pts = &mut self.console_in.as_mut().expect(
                    "programming error: rx chain cannot be processed if no pts is connected",
                );
                count = match pts.read(&mut buf) {
                    Ok(count) => {
                        log::debug!("read {}", count);
                        count
                    }
                    Err(e) => {
                        log::error!("error reading from console: {}", e);
                        0
                    }
                };
                let buf = &mut buf[..count];
                log::debug!("buf {:?} count {}", buf, count);
                if let Err(e) = mem.write_slice(buf, desc.addr()) {
                    error!("error logging console rx (stdin): {}", e)
                }
            }
            self.rxq
                .add_used(self.mem.as_ref(), chain.head_index(), count as u32)?;

            if self.rxq.needs_notification(self.mem.as_ref())? {
                log::debug!("notification needed: yes");
                self.driver_notify.signal_used_queue(0);
            } else {
                log::debug!("notification needed: no");
            }
        }

        if !self.rxq.enable_notification(self.mem.as_ref())? {
            log::debug!("loop break");
            //break;
        }
        //}
        Ok(())
    }
}

impl<S: SignalUsedQueue> MutEventSubscriber for LogQueueHandler<S> {
    fn process(&mut self, events: Events, ops: &mut EventOps) {
        if events.event_set() != EventSet::IN {
            self.handle_error("Unexpected event_set", ops);
            return;
        }

        match events.data() as u16 {
            RX_QUEUE_IDX => {
                if let Err(e) = self.process_rxq() {
                    self.handle_error(format!("Process rx error {:?}", e), ops);
                }
            }
            TX_QUEUE_IDX => {
                if self.tx_fd.read().is_err() {
                    self.handle_error("Tx ioevent read", ops);
                }
                if let Err(e) = self.process_txq() {
                    self.handle_error(format!("Process tx error {:?}", e), ops);
                }
            }
            _ => self.handle_error("Unexpected data", ops),
        }
    }

    fn init(&mut self, ops: &mut EventOps) {
        if let Some(console) = &self.console_in {
            ops.add(Events::with_data(
                console,
                RX_QUEUE_IDX as u32,
                EventSet::IN,
            ))
            .expect("Failed to register rx ioeventfd for console queue handler");
        }

        ops.add(Events::with_data(
            &self.tx_fd,
            TX_QUEUE_IDX as u32,
            EventSet::IN,
        ))
        .expect("Failed to register tx ioeventfd for console queue handler");
    }
}
