// Copyright 2020 Amazon.com, Inc. or its affiliates. All Rights Reserved.
// Author of further modifications: Peter Okelmann
// SPDX-License-Identifier: Apache-2.0 OR BSD-3-Clause

use std::fs::File;
use std::result;

use nix::sys::uio::RemoteIoVec;
use virtio_queue::{DescriptorChain, Queue};
use vm_memory::Bytes;
use vm_memory::{self, GuestAddressSpace};

use crate::devices::virtio::{SignalUsedQueue, QUEUE_MAX_SIZE};
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

// This handler only reads output from the guest and forwards it to the file
pub struct LogQueueHandler<M: GuestAddressSpace, S: SignalUsedQueue> {
    pub driver_notify: S,
    pub rxq: Queue<M>,
    pub txq: Queue<M>,
    pub console: File,
}

impl<M, S> LogQueueHandler<M, S>
where
    M: GuestAddressSpace,
    S: SignalUsedQueue,
{
    fn process_chain(&mut self, mut chain: DescriptorChain<M>) -> result::Result<(), Error> {
        log::trace!("process_chain");

        let mut i = 0;
        while let Some(desc) = chain.next() {
            chain
                .memory()
                .write_to(desc.addr(), &mut self.console, desc.len() as usize);
            i += 1;
        }
        self.txq.add_used(chain.head_index(), i as u32)?;

        //let mut iovs = [RemoteIoVec { base: 0, len: 0 }; QUEUE_MAX_SIZE as usize];
        //let mut i = 0;
        //for desc in chain.into_iter() {
        //    //desc.memory().write_to(self.console, desc.len());
        //    //iovs[i] = dbg!(RemoteIoVec { base: desc.addr().0 as usize, len: desc.len() as usize });
        //    i += 1;
        //}

        if self.txq.needs_notification()? {
            log::trace!("notification needed: yes");
            self.driver_notify.signal_used_queue(0);
        } else {
            log::trace!("notification needed: no");
        }

        Ok(())
    }

    pub fn process_rxq(&mut self) -> result::Result<(), Error> {
        warn!("unexpected rxq event");
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

impl<M, S> LogQueueHandler<M, S>
where
    M: GuestAddressSpace,
    S: SignalUsedQueue,
{
    fn process_chain(&mut self, mut chain: DescriptorChain<M>) -> result::Result<(), Error> {
        log::trace!("process_chain");

        let mut i = 0;
        while let Some(desc) = chain.next() {
            chain
                .memory()
                .write_to(desc.addr(), &mut self.console, desc.len() as usize);
            i += 1;
        }
        self.txq.add_used(chain.head_index(), i as u32)?;

        //let mut iovs = [RemoteIoVec { base: 0, len: 0 }; QUEUE_MAX_SIZE as usize];
        //let mut i = 0;
        //for desc in chain.into_iter() {
        //    //desc.memory().write_to(self.console, desc.len());
        //    //iovs[i] = dbg!(RemoteIoVec { base: desc.addr().0 as usize, len: desc.len() as usize });
        //    i += 1;
        //}

        if self.txq.needs_notification()? {
            log::trace!("notification needed: yes");
            self.driver_notify.signal_used_queue(0);
        } else {
            log::trace!("notification needed: no");
        }

        Ok(())
    }

    pub fn process_rxq(&mut self) -> result::Result<(), Error> {
        warn!("unexpected rxq event");
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
