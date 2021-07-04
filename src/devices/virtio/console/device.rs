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

use crate::devices::virtio::features::{
    VIRTIO_F_IN_ORDER, VIRTIO_F_RING_EVENT_IDX, VIRTIO_F_VERSION_1,
};
use crate::devices::virtio::console::inorder_handler::InOrderQueueHandler;
use crate::devices::virtio::{
    IrqAckHandler, MmioConfig, SingleFdSignalQueue, QUEUE_MAX_SIZE, VIRTIO_MMIO_QUEUE_NOTIFY_OFFSET,
};
use crate::kvm::hypervisor::Hypervisor;

use super::{CONSOLE_DEVICE_ID, ConsoleArgs, Error, Result, build_config_space};
use super::queue_handler::QueueHandler;
use crate::tracer::inject_syscall;
use crate::tracer::wrap_syscall::KvmRunWrapper;
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

// Does host provide console size?
const VIRTIO_CONSOLE_F_SIZE: u32 = 0;
// Does host provide multiple ports?
const VIRTIO_CONSOLE_F_MULTIPORT: u32 = 1;
// Does host support emergency write?
const VIRTIO_CONSOLE_F_EMERG_WRITE: u32 = 2;


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
        let mut device_features =
            1 << VIRTIO_F_VERSION_1 | 1 << VIRTIO_F_IN_ORDER | 1 << VIRTIO_F_RING_EVENT_IDX | 1 << VIRTIO_CONSOLE_F_SIZE;

        // A console device has two queue.
        let queues = vec![Queue::new(args.common.mem, QUEUE_MAX_SIZE)];
        let config_space = build_config_space();
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

        // Register the queue event fd. Something like this, but in a pirate fashion.
        // let ioeventfd = EventFd::new(EFD_NONBLOCK).map_err(Error::EventFd)?;
        // self.vm_fd
        //     .register_ioevent(
        //         &ioeventfd,
        //         &IoEventAddress::Mmio(
        //             self.mmio_cfg.range.base().0 + VIRTIO_MMIO_QUEUE_NOTIFY_OFFSET,
        //         ),
        //         0u32,
        //     )
        //     .map_err(Error::RegisterIoevent)?;
        let ioeventfd;
        {
            let mut wrapper_go =
                map_err_with!(self.vmm.wrapper.lock(), "cannot obtain wrapper mutex")
                    .map_err(Error::Simple)?;

            // wrapper -> injector
            {
                let wrapper = wrapper_go.take().unwrap();
                let mut tracee = self.vmm.tracee_write_guard().map_err(Error::Simple)?;

                let err =
                    "cannot re-attach injector after having detached it favour of KvmRunWrapper";
                let injector = map_err_with!(
                    inject_syscall::from_tracer(wrapper.into_tracer().map_err(Error::Simple)?),
                    &err
                )
                .map_err(Error::Simple)?;
                map_err_with!(tracee.attach_to(injector), &err).map_err(Error::Simple)?;
            }

            // we need to drop tracee for ioeventfd_
            ioeventfd = self
                .vmm
                .ioeventfd_(
                    self.mmio_cfg.range.base().0 + VIRTIO_MMIO_QUEUE_NOTIFY_OFFSET,
                    4,
                    Some(0),
                )
                .map_err(Error::Simple)?;

            // injector -> wrapper
            {
                let mut tracee = self.vmm.tracee_write_guard().map_err(Error::Simple)?;
                // we may unwrap because we just attached it.
                let injector = tracee.detach().unwrap();
                let wrapper = KvmRunWrapper::from_tracer(
                    inject_syscall::into_tracer(injector, self.vmm.vcpu_maps[0].clone())
                        .map_err(Error::Simple)?,
                )
                .map_err(Error::Simple)?;
                let _ = wrapper_go.replace(wrapper);
            }
        }


        let mut features = self.virtio_cfg.driver_features;

        let driver_notify = SingleFdSignalQueue {
            irqfd: self.irqfd.clone(),
            interrupt_status: self.virtio_cfg.interrupt_status.clone(),
            ack_handler: self.irq_ack_handler.clone(),
        };

        // FIXME replace with actual console
        let console = map_err_with!(OpenOptions::new().read(true).write(true).open("/proc/self/fd/0"),
                                "could not open console").map_err(Error::Simple)?;

        let inner = InOrderQueueHandler {
            driver_notify,
            queue: self.virtio_cfg.queues[0].clone(),
            console,
        };

        let handler = Arc::new(Mutex::new(QueueHandler { inner, ioeventfd }));

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
