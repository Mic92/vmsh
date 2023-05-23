// Copyright 2020 Amazon.com, Inc. or its affiliates. All Rights Reserved.
// Author of further modifications: Peter Okelmann
// SPDX-License-Identifier: Apache-2.0 OR BSD-3-Clause

use nix::unistd::Pid;
use simple_error::SimpleError;
use std::borrow::{Borrow, BorrowMut};
use std::fs::OpenOptions;
use std::io::{Seek, SeekFrom};
use std::ops::DerefMut;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use virtio_device::{VirtioDevice, VirtioDeviceType};

use event_manager::{MutEventSubscriber, RemoteEndpoint, Result as EvmgrResult, SubscriberId};
use virtio_blk::stdio_executor::StdIoBackend;
use virtio_device::{VirtioConfig, VirtioDeviceActions, VirtioMmioDevice, VirtioQueueNotifiable};
use virtio_queue::Queue;
use virtio_queue::QueueT;
use vm_device::bus::MmioAddress;
use vm_device::device_manager::MmioManager;
use vm_device::{DeviceMmio, MutDeviceMmio};
use vm_memory::GuestMemoryMmap;
use vmm_sys_util::eventfd::EventFd;

use crate::devices::use_ioregionfd;
use crate::devices::virtio::block::inorder_handler::Mmap;
use crate::devices::virtio::block::{
    BLOCK_DEVICE_ID, SECTOR_SHIFT, VIRTIO_BLK_F_FLUSH, VIRTIO_BLK_F_RO,
};
use crate::devices::virtio::features::{
    VIRTIO_F_IN_ORDER, VIRTIO_F_RING_EVENT_IDX, VIRTIO_F_VERSION_1,
};
use crate::devices::virtio::{IrqAckHandler, MmioConfig, SingleFdSignalQueue, QUEUE_MAX_SIZE};
use crate::devices::MaybeIoRegionFd;
use crate::kvm::hypervisor::{
    ioevent::IoEvent, ioregionfd::IoRegionFd, userspaceioeventfd::UserspaceIoEventFd,
};

use super::inorder_handler::InOrderQueueHandler;
use super::queue_handler::QueueHandler;
use super::{build_config_space, BlockArgs, Error, Result};

// This Block device can only use the MMIO transport for now, but we plan to reuse large parts of
// the functionality when we implement virtio PCI as well, for example by having a base generic
// type, and then separate concrete instantiations for `MmioConfig` and `PciConfig`.
pub struct Block {
    virtio_cfg: VirtioConfig<Queue>,
    pub mmio_cfg: MmioConfig,
    endpoint: RemoteEndpoint<Arc<Mutex<dyn MutEventSubscriber + Send>>>,
    pub irq_ack_handler: Arc<Mutex<IrqAckHandler>>,
    irqfd: Arc<EventFd>,
    pub ioregionfd: Option<IoRegionFd>,
    ioeventfd: Option<IoEvent>,
    pub uioefd: UserspaceIoEventFd,
    /// only used when ioregionfd != None
    file_path: PathBuf,
    read_only: bool,
    sub_id: Option<SubscriberId>,
    guest_memory: Arc<GuestMemoryMmap>,
    pid: Pid,

    // Before resetting we return the handler to the mmio thread for cleanup
    #[allow(dead_code)]
    handler: Option<Arc<Mutex<dyn MutEventSubscriber + Send>>>,
    // We'll prob need to remember this for state save/restore unless we pass the info from
    // the outside.
    _root_device: bool,
}

impl Block {
    pub fn new<B>(mut args: BlockArgs<B>) -> Result<Arc<Mutex<Self>>>
    where
        // We're using this (more convoluted) bound so we can pass both references and smart
        // pointers such as mutex guards here.
        B: DerefMut,
        B::Target: MmioManager<D = Arc<dyn DeviceMmio + Send + Sync>>,
    {
        // The queue handling logic for this device uses the buffers in order, so we enable the
        // corresponding feature as well.
        let mut device_features =
            1 << VIRTIO_F_VERSION_1 | 1 << VIRTIO_F_IN_ORDER | 1 << VIRTIO_F_RING_EVENT_IDX;

        if args.read_only {
            device_features |= 1 << VIRTIO_BLK_F_RO;
        }

        if args.advertise_flush {
            device_features |= 1 << VIRTIO_BLK_F_FLUSH;
        }

        // A block device has a single queue.
        let mem = args.common.mem.clone();
        let queues = vec![Queue::new(QUEUE_MAX_SIZE).map_err(Error::QueueCreation)?];
        let config_space = build_config_space(&args.file_path)?;
        let virtio_cfg = VirtioConfig::new(device_features, queues, config_space);

        // Used to send notifications to the driver.
        //let irqfd = EventFd::new(EFD_NONBLOCK).map_err(Error::EventFd)?;
        log::debug!("register irqfd on gsi {}", args.common.mmio_cfg.gsi);
        let irqfd = Arc::new(
            args.common
                .vmm
                .irqfd(args.common.mmio_cfg.gsi)
                .map_err(Error::Simple)?,
        );

        let mmio_cfg = args.common.mmio_cfg;

        let irq_ack_handler = Arc::new(Mutex::new(IrqAckHandler::new(
            virtio_cfg.interrupt_status.clone(),
            irqfd.clone(),
        )));

        let mut ioregionfd = None;
        if use_ioregionfd() {
            ioregionfd = Some(
                args.common
                    .vmm
                    .ioregionfd(mmio_cfg.range.base().0, mmio_cfg.range.size() as usize)
                    .map_err(Error::Simple)?,
            );
        }
        let mut uioefd = UserspaceIoEventFd::default();
        let ioeventfd = IoEvent::register(&args.common.vmm, &mut uioefd, &mmio_cfg, 0)
            .map_err(Error::Simple)?;

        let block = Arc::new(Mutex::new(Block {
            virtio_cfg,
            mmio_cfg,
            endpoint: args.common.event_mgr.remote_endpoint(),
            irq_ack_handler,
            irqfd,
            ioregionfd,
            ioeventfd: Some(ioeventfd),
            uioefd,
            file_path: args.file_path,
            read_only: args.read_only,
            pid: args.common.vmm.pid,
            sub_id: None,
            handler: None,
            _root_device: args.root_device,
            guest_memory: mem,
        }));

        // Register the device on the MMIO bus.
        args.common
            .mmio_mgr
            .register_mmio(mmio_cfg.range, block.clone())
            .map_err(Error::Bus)?;

        Ok(block)
    }

    fn _activate(&mut self) -> Result<()> {
        if self.virtio_cfg.device_activated {
            return Err(Error::AlreadyActivated);
        }

        // We do not support legacy drivers.
        if self.virtio_cfg.driver_features & (1 << VIRTIO_F_VERSION_1) == 0 {
            return Err(Error::BadFeatures(self.virtio_cfg.driver_features));
        }

        let mut file = OpenOptions::new()
            .read(true)
            .write(!self.read_only)
            .open(&self.file_path)
            .map_err(Error::OpenFile)?;

        let disk_size = file.seek(SeekFrom::End(0)).map_err(Error::Seek)?;

        let mmap = match Mmap::new(&file, disk_size as usize) {
            Ok(m) => m,
            Err(e) => {
                return Err(Error::Simple(SimpleError::new(format!(
                    "cannot mmap disk: {:?}",
                    e
                ))))
            }
        };

        let mut features = self.virtio_cfg.driver_features;
        if self.read_only {
            // Not sure if the driver is expected to explicitly acknowledge the `RO` feature,
            // so adding it explicitly here when present just in case.
            features |= 1 << VIRTIO_BLK_F_RO;
        }

        // TODO: Create the backend earlier (as part of `Block::new`)?
        let disk = StdIoBackend::new(file, features)
            .map_err(Error::Backend)?
            .with_device_id(*b"vmsh0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0");

        let driver_notify = SingleFdSignalQueue {
            irqfd: self.irqfd.clone(),
            interrupt_status: self.virtio_cfg.interrupt_status.clone(),
            ack_handler: self.irq_ack_handler.clone(),
        };

        let queue = self.virtio_cfg.queues.remove(0);
        let inner = InOrderQueueHandler {
            pid: self.pid,
            driver_notify,
            queue,
            disk,
            sectors: disk_size >> SECTOR_SHIFT,
            mmap,
            mem: Arc::clone(&self.guest_memory),
            remote_iovs: vec![],
        };
        let handler = Arc::new(Mutex::new(QueueHandler {
            inner,
            ioeventfd: match self.ioeventfd.take() {
                Some(fd) => fd,
                None => return Err(Error::Simple(SimpleError::new("ioeventfd not set"))),
            },
        }));

        // Register the queue handler with the `EventManager`. We record the `sub_id`
        // (and/or keep a handler clone) to remove the subscriber when resetting the device
        let sub_id = self
            .endpoint
            .call_blocking(move |mgr| -> EvmgrResult<SubscriberId> {
                Ok(mgr.add_subscriber(handler))
            })
            .map_err(|e| {
                log::warn!("{}", e);
                Error::Endpoint(e)
            })?;
        self.sub_id = Some(sub_id);

        log::debug!("activating device: ok");
        self.virtio_cfg.device_activated = true;

        Ok(())
    }
    fn _reset(&mut self) -> Result<()> {
        // we remove the handler here, since we need to free up the ioeventfd resources
        // in the mmio thread rather the eventmanager thread.
        if let Some(sub_id) = self.sub_id.take() {
            let handler = self
                .endpoint
                .call_blocking(move |mgr| mgr.remove_subscriber(sub_id))
                .map_err(|e| {
                    log::warn!("{}", e);
                    Error::Endpoint(e)
                })?;
            self.handler = Some(handler);
        }
        Ok(())
    }
}

impl MaybeIoRegionFd for Block {
    fn get_ioregionfd(&mut self) -> &mut Option<IoRegionFd> {
        &mut self.ioregionfd
    }
}

// We now implement `WithVirtioConfig` and `WithDeviceOps` to get the automatic implementation
// for `VirtioDevice`.
impl VirtioDeviceType for Block {
    fn device_type(&self) -> u32 {
        BLOCK_DEVICE_ID
    }
}

impl Borrow<VirtioConfig<Queue>> for Block {
    fn borrow(&self) -> &VirtioConfig<Queue> {
        &self.virtio_cfg
    }
}

impl BorrowMut<VirtioConfig<Queue>> for Block {
    fn borrow_mut(&mut self) -> &mut VirtioConfig<Queue> {
        &mut self.virtio_cfg
    }
}

impl VirtioDeviceActions for Block {
    type E = Error;

    /// make sure to set self.vmm.wrapper to Some() before activating. Typically this is done by
    /// activating during vmm.kvmrun_wrapped()
    fn activate(&mut self) -> Result<()> {
        let ret = self._activate();
        if let Err(ref e) = ret {
            log::warn!("failed to activate block device: {:?}", e);
        }
        ret
    }

    fn reset(&mut self) -> Result<()> {
        self.set_device_status(0);
        self._reset()?;
        Ok(())
    }
}

impl VirtioQueueNotifiable for Block {
    fn queue_notify(&mut self, val: u32) {
        if use_ioregionfd() {
            self.uioefd.queue_notify(val);
            log::trace!("queue_notify {}", val);
        }
    }
}

impl VirtioMmioDevice for Block {}

impl MutDeviceMmio for Block {
    fn mmio_read(&mut self, _base: MmioAddress, offset: u64, data: &mut [u8]) {
        self.read(offset, data);
    }

    fn mmio_write(&mut self, _base: MmioAddress, offset: u64, data: &[u8]) {
        self.write(offset, data);
    }
}
