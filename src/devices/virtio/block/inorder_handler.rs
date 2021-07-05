// Copyright 2020 Amazon.com, Inc. or its affiliates. All Rights Reserved.
// Author of further modifications: Peter Okelmann
// SPDX-License-Identifier: Apache-2.0 OR BSD-3-Clause

use std::fs::File;
use std::result;

use log::warn;
use virtio_blk::request::Request;
use virtio_blk::stdio_executor::{self, StdIoBackend};
use virtio_queue::{DescriptorChain, Queue};
use vm_memory::{self, Bytes, GuestAddressSpace};

use crate::devices::virtio::SignalUsedQueue;

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
    pub disk: StdIoBackend<File>,
}

impl<M, S> InOrderQueueHandler<M, S>
where
    M: GuestAddressSpace,
    S: SignalUsedQueue,
{
    fn process_chain(&mut self, mut chain: DescriptorChain<M>) -> result::Result<(), Error> {
        let len;

        log::trace!("process_chain");
        match Request::parse(&mut chain) {
            Ok(request) => {
                log::trace!("request: {:?}", request);
                let status = match self.disk.execute(chain.memory(), &request) {
                    Ok(l) => {
                        // TODO: Using `saturating_add` until we consume the recent changes
                        // proposed for the executor upstream.
                        len = l.saturating_add(1);
                        // VIRTIO_BLK_S_OK defined as 0 in the standard.
                        0
                    }
                    Err(e) => {
                        warn!("failed to execute block request: {:?}", e);
                        len = 1;
                        // TODO: add `status` or similar method to executor error.
                        if let stdio_executor::Error::Unsupported(_) = e {
                            // UNSUPP
                            2
                        } else {
                            // IOERR
                            1
                        }
                    }
                };

                chain
                    .memory()
                    .write_obj(status as u8, request.status_addr())?;
            }
            Err(e) => {
                len = 0;
                warn!("block request parse error: {:?}", e);
            }
        }

        self.queue.add_used(chain.head_index(), len)?;

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

// TODO: Figure out which unit tests make sense to add after implementing a generic backend
// abstraction for `InOrderHandler`.
