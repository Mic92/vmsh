// Copyright 2020 Amazon.com, Inc. or its affiliates. All Rights Reserved.
// Author of further modifications: Peter Okelmann
// SPDX-License-Identifier: Apache-2.0 OR BSD-3-Clause

use std::fs::File;
use std::io::{IoSlice, IoSliceMut};
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::{io, result, slice};

use libc::c_void;
use log::warn;
use nix::sys::mman::{mmap, msync, munmap, MapFlags, MsFlags, ProtFlags};
use nix::sys::uio::{process_vm_readv, process_vm_writev, RemoteIoVec};
use nix::unistd::Pid;
use simple_error::{require_with, try_with};
use std::os::unix::io::AsRawFd;
use virtio_blk::defs::{SECTOR_SHIFT, SECTOR_SIZE};
use virtio_blk::request::{Request, RequestType};
use virtio_blk::stdio_executor::{self, StdIoBackend};
use virtio_queue::{DescriptorChain, Queue, QueueOwnedT, QueueT};
use vm_memory::GuestMemoryMmap;
use vm_memory::{self, Bytes, GuestAddressSpace, GuestMemory, GuestMemoryError};

use crate::devices::virtio::SignalUsedQueue;
use crate::result::Result;

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

pub struct Mmap {
    ptr: *mut c_void,
    len: usize,
}

unsafe impl Send for Mmap {}

impl Mmap {
    pub fn new(file: &File, len: usize) -> Result<Mmap> {
        let len = require_with!(NonZeroUsize::new(len), "lenght is zero");
        let ptr = unsafe {
            try_with!(
                mmap(
                    None,
                    len,
                    ProtFlags::PROT_READ | ProtFlags::PROT_WRITE,
                    MapFlags::MAP_SHARED,
                    file.as_raw_fd(),
                    0,
                ),
                "mmap failed"
            )
        };
        Ok(Mmap {
            ptr,
            len: len.get(),
        })
    }
}

impl Drop for Mmap {
    fn drop(&mut self) {
        if let Err(e) = unsafe { munmap(self.ptr, self.len) } {
            warn!("Failed to munmap block device: {}", e);
        }
    }
}

// This object is used to process the queue of a block device without making any assumptions
// about the notification mechanism. We're using a specific backend for now (the `StdIoBackend`
// object), but the aim is to have a way of working with generic backends and turn this into
// a more flexible building block. The name comes from processing and returning descriptor
// chains back to the device in the same order they are received.
pub struct InOrderQueueHandler<S: SignalUsedQueue> {
    pub driver_notify: S,
    pub queue: Queue,
    pub disk: StdIoBackend<File>,
    pub sectors: u64,
    pub mmap: Mmap,
    //pub guest_memory: Arc<Mutex<Option<M>>>,
    pub pid: Pid,

    // we have those here to safe reallocations across requests
    pub remote_iovs: Vec<RemoteIoVec>,
    pub mem: Arc<GuestMemoryMmap>,
}

unsafe impl<S: SignalUsedQueue> Send for InOrderQueueHandler<S> {}

impl<S: SignalUsedQueue> InOrderQueueHandler<S> {
    fn check_access(&self, mut sectors_count: u64, sector: u64) -> stdio_executor::Result<()> {
        sectors_count = sectors_count
            .checked_add(sector)
            .ok_or(stdio_executor::Error::InvalidAccess)?;
        if sectors_count > self.sectors {
            return Err(stdio_executor::Error::InvalidAccess);
        }
        Ok(())
    }

    fn prepare_iovs(&mut self, request: &Request) -> stdio_executor::Result<()> {
        self.remote_iovs.clear();
        self.remote_iovs.reserve(request.data().len());
        for (data_addr, data_len) in request.data() {
            let hv_addr = match self.mem.memory().get_host_address(*data_addr) {
                // TODO length check
                Ok(hv_addr) => hv_addr,
                Err(e) => {
                    return Err(stdio_executor::Error::GuestMemory(e));
                }
            };

            self.remote_iovs.push(RemoteIoVec {
                base: hv_addr as usize,
                len: *data_len as usize,
            });
        }

        Ok(())
    }

    fn execute(&mut self, mem: &GuestMemoryMmap, request: &Request) -> stdio_executor::Result<u32> {
        let offset = request
            .sector()
            .checked_shl(u32::from(SECTOR_SHIFT))
            .ok_or(stdio_executor::Error::InvalidAccess)?;

        let total_len = request.total_data_len();
        // This will count the number of bytes written by the device to the memory. It must fit in
        // an u32 for further writing in the used ring.
        let mut bytes_to_mem: u32 = 0;
        let request_type = request.request_type();

        if (request_type == RequestType::In || request_type == RequestType::Out)
            && (total_len % SECTOR_SIZE != 0)
        {
            return Err(stdio_executor::Error::InvalidDataLength);
        }

        match request_type {
            RequestType::In => {
                self.check_access(total_len / SECTOR_SIZE, request.sector())?;
                // Total data length should fit in an u32 for further writing in the used ring.
                if total_len > u32::MAX as u64 {
                    return Err(stdio_executor::Error::InvalidDataLength);
                }
                self.prepare_iovs(request)?;
                let local_iovs = vec![IoSlice::new(unsafe {
                    slice::from_raw_parts(
                        self.mmap.ptr.add(offset as usize) as *mut u8,
                        request.total_data_len() as usize,
                    )
                })];

                bytes_to_mem =
                    process_vm_writev(self.pid, local_iovs.as_slice(), self.remote_iovs.as_slice())
                        .map_err(|e| {
                            stdio_executor::Error::Read(
                                GuestMemoryError::IOError(io::Error::from_raw_os_error(e as i32)),
                                0,
                            )
                        })? as u32;
            }
            RequestType::Out => {
                self.check_access(total_len / SECTOR_SIZE, request.sector())?;
                self.prepare_iovs(request)?;
                let mut local_iovs = vec![IoSliceMut::new(unsafe {
                    slice::from_raw_parts_mut(
                        self.mmap.ptr.add(offset as usize) as *mut u8,
                        request.total_data_len() as usize,
                    )
                })];
                bytes_to_mem = process_vm_readv(
                    self.pid,
                    local_iovs.as_mut_slice(),
                    self.remote_iovs.as_slice(),
                )
                .map_err(|e| {
                    stdio_executor::Error::Write(GuestMemoryError::IOError(
                        io::Error::from_raw_os_error(e as i32),
                    ))
                })? as u32;
            }
            RequestType::Flush => {
                self.check_access(total_len / SECTOR_SIZE, request.sector())?;
                let res = unsafe {
                    msync(
                        self.mmap.ptr.add(offset as usize),
                        total_len as usize,
                        MsFlags::MS_SYNC,
                    )
                };
                res.map_err(|e| {
                    stdio_executor::Error::Flush(io::Error::from_raw_os_error(e as i32))
                })?
            }
            _ => return self.disk.execute(mem, request),
        }
        Ok(bytes_to_mem)
    }
    fn process_chain(
        &mut self,
        mut chain: DescriptorChain<&GuestMemoryMmap>,
    ) -> result::Result<(), Error> {
        let len;

        log::trace!("process_chain");
        match Request::parse(&mut chain) {
            Ok(request) => {
                log::trace!("request: {:?}", request);
                let status = match self.execute(chain.memory(), &request) {
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

        self.queue
            .add_used(self.mem.as_ref(), chain.head_index(), len)?;

        if self.queue.needs_notification(self.mem.as_ref())? {
            log::trace!("notification needed: yes");
            self.driver_notify.signal_used_queue(0);
        } else {
            log::trace!("notification needed: no");
        }

        log::trace!("process_chain done");
        Ok(())
    }

    pub fn process_queue(&mut self) -> result::Result<(), Error> {
        // manybe expensive?
        let mem = Arc::clone(&self.mem);
        // To see why this is done in a loop, please look at the `Queue::enable_notification`
        // comments in `vm_virtio`.
        loop {
            self.queue.disable_notification(mem.as_ref())?;

            while let Some(chain) = self.queue.iter(mem.as_ref())?.next() {
                self.process_chain(chain)?;
            }

            if !self.queue.enable_notification(mem.as_ref())? {
                break;
            }
        }

        Ok(())
    }
}

// TODO: Figure out which unit tests make sense to add after implementing a generic backend
// abstraction for `InOrderHandler`.
