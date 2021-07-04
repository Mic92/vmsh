// Copyright 2020 Amazon.com, Inc. or its affiliates. All Rights Reserved.
// Author of further modifications: Peter Okelmann
// SPDX-License-Identifier: Apache-2.0 OR BSD-3-Clause

use std::fs::File;
use std::result;

use nix::sys::uio::RemoteIoVec;
use virtio_queue::{DescriptorChain, Queue};
use vm_memory::{self, GuestAddressSpace};

use crate::devices::virtio::{SignalUsedQueue, QUEUE_MAX_SIZE};

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

// This object is used to process the queue of a block device without making any assumptions
// about the notification mechanism. We're using a specific backend for now (the `StdIoBackend`
// object), but the aim is to have a way of working with generic backends and turn this into
// a more flexible building block. The name comes from processing and returning descriptor
// chains back to the device in the same order they are received.
pub struct InOrderQueueHandler<M: GuestAddressSpace, S: SignalUsedQueue> {
    pub driver_notify: S,
    pub queue: Queue<M>,
    pub console: File,
}

/// Block request parsing errors.
#[derive(Debug)]
pub enum ParseError {
}

impl<M, S> InOrderQueueHandler<M, S>
where
    M: GuestAddressSpace,
    S: SignalUsedQueue,
{
    fn process_chain(&mut self, mut chain: DescriptorChain<M>) -> result::Result<(), Error> {
        log::trace!("process_chain");

        let mut iovs = [RemoteIoVec { base: 0, len: 0 }; QUEUE_MAX_SIZE as usize];
        let mut i = 0;

        for desc in chain {
            iovs[i] = RemoteIoVec { base: desc.addr().0 as usize, len: desc.len() as usize };
            i += 1;
        }

        //self.queue.add_used(chain.head_index(), i as u32)?;

        if self.queue.needs_notification()? {
            log::trace!("notification needed: yes");
            self.driver_notify.signal_used_queue(0);
        } else {
            log::trace!("notification needed: no");
        }

        log::trace!("process_chain done");
        Ok(())
    }

    pub fn process_queue(&mut self) -> result::Result<(), Error> {
        // To see why this is done in a loop, please look at the `Queue::enable_notification`
        // comments in `vm_virtio`.
        loop {
            self.queue.disable_notification()?;

            while let Some(chain) = self.queue.iter()?.next() {
                self.process_chain(chain)?;
            }

            if !self.queue.enable_notification()? {
                break;
            }
        }

        Ok(())
    }
}
