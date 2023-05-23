// Copyright 2020 Amazon.com, Inc. or its affiliates. All Rights Reserved.
// Author of further modifications: Peter Okelmann
// SPDX-License-Identifier: Apache-2.0 OR BSD-3-Clause

use std::borrow::{Borrow, BorrowMut};
use std::fs::OpenOptions;
use std::io;
use std::io::Write;
use std::ops::DerefMut;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use event_manager::{MutEventSubscriber, RemoteEndpoint, Result as EvmgrResult, SubscriberId};
use virtio_device::{VirtioConfig, VirtioDeviceActions, VirtioMmioDevice, VirtioQueueNotifiable};
use virtio_device::{VirtioDevice, VirtioDeviceType};
use virtio_queue::Queue;
use virtio_queue::QueueT;
use vm_device::bus::MmioAddress;
use vm_device::device_manager::MmioManager;
use vm_device::{DeviceMmio, MutDeviceMmio};
use vm_memory::GuestMemoryMmap;
use vmm_sys_util::eventfd::EventFd;

use crate::devices::use_ioregionfd;
use crate::devices::virtio::console::log_handler::LogQueueHandler;
use crate::devices::virtio::console::VIRTIO_CONSOLE_F_SIZE;
use crate::devices::virtio::features::{
    VIRTIO_F_IN_ORDER, VIRTIO_F_RING_EVENT_IDX, VIRTIO_F_VERSION_1,
};
use crate::devices::virtio::{IrqAckHandler, MmioConfig, SingleFdSignalQueue, QUEUE_MAX_SIZE};
use crate::devices::MaybeIoRegionFd;
use crate::kvm::hypervisor::{
    ioevent::IoEvent, ioregionfd::IoRegionFd, userspaceioeventfd::UserspaceIoEventFd,
};

//use super::queue_handler::QueueHandler;
use super::{build_config_space, ConsoleArgs, Error, Result, CONSOLE_DEVICE_ID};
use simple_error::{map_err_with, SimpleError};

pub(super) const RX_QUEUE_IDX: u16 = 0;
pub(super) const TX_QUEUE_IDX: u16 = 1;

pub struct Console {
    virtio_cfg: VirtioConfig<Queue>,
    pub mmio_cfg: MmioConfig,
    endpoint: RemoteEndpoint<Arc<Mutex<dyn MutEventSubscriber + Send>>>,
    pub irq_ack_handler: Arc<Mutex<IrqAckHandler>>,
    irqfd: Arc<EventFd>,
    pub ioregionfd: Option<IoRegionFd>,
    pub uioefd: UserspaceIoEventFd,
    mem: Arc<GuestMemoryMmap>,
    tx_fd: Option<IoEvent>,
    /// only used when ioregionfd != None
    sub_id: Option<SubscriberId>,
    pts: Option<PathBuf>,

    // Before resetting we return the handler to the mmio thread for cleanup
    #[allow(dead_code)]
    handler: Option<Arc<Mutex<dyn MutEventSubscriber + Send>>>,
}

impl Console {
    pub fn new<B>(mut args: ConsoleArgs<B>) -> Result<Arc<Mutex<Self>>>
    where
        // We're using this (more convoluted) bound so we can pass both references and smart
        // pointers such as mutex guards here.
        B: DerefMut,
        B::Target: MmioManager<D = Arc<dyn DeviceMmio + Send + Sync>>,
    {
        // The queue handling logic for this device uses the buffers in order, so we enable the
        // corresponding feature as well.
        let device_features = 1 << VIRTIO_F_VERSION_1
            | 1 << VIRTIO_F_IN_ORDER
            | 1 << VIRTIO_F_RING_EVENT_IDX
            | 1 << VIRTIO_CONSOLE_F_SIZE;

        // A console device has two queue.
        let queues = vec![
            Queue::new(QUEUE_MAX_SIZE).map_err(Error::QueueCreation)?,
            Queue::new(QUEUE_MAX_SIZE).map_err(Error::QueueCreation)?,
        ];

        let config_space = build_config_space();
        let virtio_cfg = VirtioConfig::new(device_features, queues, config_space);

        // Used to send notifications to the driver.
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
            Arc::clone(&irqfd),
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

        let pts = args.pts;
        log::info!("pts is {:?}", pts);

        //let rx_fd = IoEvent::register(&self.vmm, &mut self.uioefd, &self.mmio_cfg, RX_QUEUE_IDX as u64)
        //.map_err(Error::Simple)?;
        let mut uioefd = UserspaceIoEventFd::default();
        let tx_fd = IoEvent::register(
            &args.common.vmm,
            &mut uioefd,
            &mmio_cfg,
            TX_QUEUE_IDX as u64,
        )
        .map_err(Error::Simple)?;

        let console = Arc::new(Mutex::new(Console {
            virtio_cfg,
            mmio_cfg,
            endpoint: args.common.event_mgr.remote_endpoint(),
            irq_ack_handler,
            irqfd,
            ioregionfd,
            mem: Arc::clone(&args.common.mem),
            tx_fd: Some(tx_fd),
            uioefd,
            sub_id: None,
            handler: None,
            pts,
        }));

        // Register the device on the MMIO bus.
        args.common
            .mmio_mgr
            .register_mmio(mmio_cfg.range, console.clone())
            .map_err(Error::Bus)?;

        Ok(console)
    }

    fn _activate(&mut self) -> Result<()> {
        if self.virtio_cfg.device_activated {
            return Err(Error::AlreadyActivated);
        }

        // We do not support legacy drivers.
        if self.virtio_cfg.driver_features & (1 << VIRTIO_F_VERSION_1) == 0 {
            return Err(Error::BadFeatures(self.virtio_cfg.driver_features));
        }

        let driver_notify = SingleFdSignalQueue {
            irqfd: self.irqfd.clone(),
            interrupt_status: self.virtio_cfg.interrupt_status.clone(),
            ack_handler: self.irq_ack_handler.clone(),
        };

        let console_in;
        let console_out: Box<dyn Write + Send>;
        match &self.pts {
            Some(pts) => {
                console_in = Some(
                    map_err_with!(
                        OpenOptions::new().read(true).open(pts),
                        "could not open read console"
                    )
                    .map_err(Error::Simple)?,
                );
                console_out = Box::new(
                    map_err_with!(
                        OpenOptions::new().write(true).open(pts),
                        "could not open write console"
                    )
                    .map_err(Error::Simple)?,
                );
            }
            None => {
                console_in = None;
                console_out = Box::new(io::stdout());
            }
        };

        let rxq = self.virtio_cfg.queues.remove(RX_QUEUE_IDX.into());
        let txq = self.virtio_cfg.queues.remove(RX_QUEUE_IDX.into());

        let handler = Arc::new(Mutex::new(LogQueueHandler {
            driver_notify,
            tx_fd: match self.tx_fd.take() {
                Some(tx_fd) => tx_fd,
                None => return Err(Error::Simple(SimpleError::new("no tx_fd set"))),
            },
            mem: Arc::clone(&self.mem),
            rxq,
            txq,
            console_out,
            console_in,
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

impl MaybeIoRegionFd for Console {
    fn get_ioregionfd(&mut self) -> &mut Option<IoRegionFd> {
        &mut self.ioregionfd
    }
}

// We now implement `WithVirtioConfig` and `WithDeviceOps` to get the automatic implementation
// for `VirtioDevice`.
impl VirtioDeviceType for Console {
    fn device_type(&self) -> u32 {
        CONSOLE_DEVICE_ID
    }
}

impl Borrow<VirtioConfig<Queue>> for Console {
    fn borrow(&self) -> &VirtioConfig<Queue> {
        &self.virtio_cfg
    }
}

impl BorrowMut<VirtioConfig<Queue>> for Console {
    fn borrow_mut(&mut self) -> &mut VirtioConfig<Queue> {
        &mut self.virtio_cfg
    }
}

impl VirtioDeviceActions for Console {
    type E = Error;

    /// make sure to set self.vmm.wrapper to Some() before activating. Typically this is done by
    /// activating during vmm.kvmrun_wrapped()
    fn activate(&mut self) -> Result<()> {
        let ret = self._activate();
        if let Err(ref e) = ret {
            log::warn!("failed to activate console device: {:?}", e);
        }
        ret
    }

    fn reset(&mut self) -> Result<()> {
        self.set_device_status(0);
        self._reset()?;
        Ok(())
    }
}

impl VirtioQueueNotifiable for Console {
    fn queue_notify(&mut self, val: u32) {
        if use_ioregionfd() {
            self.uioefd.queue_notify(val);
            log::trace!("queue_notify {}", val);
        }
    }
}

impl VirtioMmioDevice for Console {}

impl MutDeviceMmio for Console {
    fn mmio_read(&mut self, _base: MmioAddress, offset: u64, data: &mut [u8]) {
        self.read(offset, data);
    }

    fn mmio_write(&mut self, _base: MmioAddress, offset: u64, data: &[u8]) {
        self.write(offset, data);
    }
}
