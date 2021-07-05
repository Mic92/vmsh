// Copyright 2020 Amazon.com, Inc. or its affiliates. All Rights Reserved.
// Author of further modifications: Peter Okelmann
// SPDX-License-Identifier: Apache-2.0 OR BSD-3-Clause

use std::borrow::{Borrow, BorrowMut};
use std::fs::OpenOptions;
use std::ops::DerefMut;
use std::sync::{Arc, Mutex};
use virtio_device::{VirtioDevice, VirtioDeviceType};

use event_manager::{MutEventSubscriber, RemoteEndpoint, Result as EvmgrResult, SubscriberId};
use virtio_device::{VirtioConfig, VirtioDeviceActions, VirtioMmioDevice};
use virtio_queue::Queue;
use vm_device::bus::MmioAddress;
use vm_device::device_manager::MmioManager;
use vm_device::{DeviceMmio, MutDeviceMmio};
use vm_memory::GuestAddressSpace;
use vmm_sys_util::eventfd::EventFd;

use crate::devices::virtio::console::VIRTIO_CONSOLE_F_SIZE;
use crate::devices::virtio::console::log_handler::LogQueueHandler;
use crate::devices::virtio::features::{
    VIRTIO_F_IN_ORDER, VIRTIO_F_RING_EVENT_IDX, VIRTIO_F_VERSION_1,
};
use crate::devices::virtio::{IrqAckHandler, MmioConfig, QUEUE_MAX_SIZE, SingleFdSignalQueue, register_ioeventfd};
use crate::kvm::hypervisor::Hypervisor;

//use super::queue_handler::QueueHandler;
use super::{build_config_space, ConsoleArgs, Error, Result, CONSOLE_DEVICE_ID};
use simple_error::map_err_with;

pub struct Console<M: GuestAddressSpace> {
    virtio_cfg: VirtioConfig<M>,
    pub mmio_cfg: MmioConfig,
    endpoint: RemoteEndpoint<Arc<Mutex<dyn MutEventSubscriber + Send>>>,
    pub irq_ack_handler: Arc<Mutex<IrqAckHandler>>,
    vmm: Arc<Hypervisor>,
    irqfd: Arc<EventFd>,
    sub_id: Option<SubscriberId>,

    // Before resetting we return the handler to the mmio thread for cleanup
    #[allow(dead_code)]
    handler: Option<Arc<Mutex<dyn MutEventSubscriber + Send>>>,
}

impl<M> Console<M>
where
    M: GuestAddressSpace + Clone + Send + 'static,
{
    pub fn new<B>(mut args: ConsoleArgs<M, B>) -> Result<Arc<Mutex<Self>>>
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
        let queues = vec![Queue::new(args.common.mem.clone(), QUEUE_MAX_SIZE); 2];

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

        let console = Arc::new(Mutex::new(Console {
            virtio_cfg,
            mmio_cfg,
            endpoint: args.common.event_mgr.remote_endpoint(),
            irq_ack_handler,
            vmm: args.common.vmm.clone(),
            irqfd,
            sub_id: None,
            handler: None,
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

        // FIXME replace with actual console
        let console = map_err_with!(
            OpenOptions::new()
                .read(true)
                .write(true)
                .open("/proc/self/fd/0"),
            "could not open console"
        )
            .map_err(Error::Simple)?;

        //let rx_fd = register_ioeventfd(&self.vmm, &self.mmio_cfg, 0).map_err(Error::Simple)?;
        let tx_fd = register_ioeventfd(&self.vmm, &self.mmio_cfg, 1).map_err(Error::Simple)?;

        let handler = Arc::new(Mutex::new(LogQueueHandler {
            driver_notify,
            tx_fd,
            rxq: self.virtio_cfg.queues[0].clone(),
            txq: self.virtio_cfg.queues[1].clone(),
            console,
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

// We now implement `WithVirtioConfig` and `WithDeviceOps` to get the automatic implementation
// for `VirtioDevice`.
impl<M: GuestAddressSpace + Clone + Send + 'static> VirtioDeviceType for Console<M> {
    fn device_type(&self) -> u32 {
        CONSOLE_DEVICE_ID
    }
}

impl<M: GuestAddressSpace + Clone + Send + 'static> Borrow<VirtioConfig<M>> for Console<M> {
    fn borrow(&self) -> &VirtioConfig<M> {
        &self.virtio_cfg
    }
}

impl<M: GuestAddressSpace + Clone + Send + 'static> BorrowMut<VirtioConfig<M>> for Console<M> {
    fn borrow_mut(&mut self) -> &mut VirtioConfig<M> {
        &mut self.virtio_cfg
    }
}

impl<M: GuestAddressSpace + Clone + Send + 'static> VirtioDeviceActions for Console<M> {
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

impl<M: GuestAddressSpace + Clone + Send + 'static> VirtioMmioDevice<M> for Console<M> {}

impl<M: GuestAddressSpace + Clone + Send + 'static> MutDeviceMmio for Console<M> {
    fn mmio_read(&mut self, _base: MmioAddress, offset: u64, data: &mut [u8]) {
        self.read(offset, data);
    }

    fn mmio_write(&mut self, _base: MmioAddress, offset: u64, data: &[u8]) {
        self.write(offset, data);
    }
}
