// Copyright 2020 Amazon.com, Inc. or its affiliates. All Rights Reserved.
// Author of further modifications: Peter Okelmann
// SPDX-License-Identifier: Apache-2.0 OR BSD-3-Clause

use std::borrow::{Borrow, BorrowMut};
use std::fs::OpenOptions;
use std::ops::DerefMut;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use virtio_device::{VirtioDevice, VirtioDeviceType};

use event_manager::{MutEventSubscriber, RemoteEndpoint, Result as EvmgrResult, SubscriberId};
use virtio_blk::stdio_executor::StdIoBackend;
use virtio_device::{VirtioConfig, VirtioDeviceActions, VirtioMmioDevice};
use virtio_queue::Queue;
use vm_device::bus::MmioAddress;
use vm_device::device_manager::MmioManager;
use vm_device::{DeviceMmio, MutDeviceMmio};
use vm_memory::GuestAddressSpace;
use vmm_sys_util::eventfd::EventFd;

use crate::devices::virtio::block::{BLOCK_DEVICE_ID, VIRTIO_BLK_F_FLUSH, VIRTIO_BLK_F_RO};
use crate::devices::virtio::features::{
    VIRTIO_F_IN_ORDER, VIRTIO_F_RING_EVENT_IDX, VIRTIO_F_VERSION_1,
};
use crate::devices::virtio::{IrqAckHandler, MmioConfig, SingleFdSignalQueue, QUEUE_MAX_SIZE};
use crate::kvm::hypervisor::Hypervisor;

use super::super::register_ioeventfd;
use super::inorder_handler::InOrderQueueHandler;
use super::queue_handler::QueueHandler;
use super::{build_config_space, BlockArgs, Error, Result};

// This Block device can only use the MMIO transport for now, but we plan to reuse large parts of
// the functionality when we implement virtio PCI as well, for example by having a base generic
// type, and then separate concrete instantiations for `MmioConfig` and `PciConfig`.
pub struct Block<M: GuestAddressSpace> {
    virtio_cfg: VirtioConfig<M>,
    pub mmio_cfg: MmioConfig,
    endpoint: RemoteEndpoint<Arc<Mutex<dyn MutEventSubscriber + Send>>>,
    pub irq_ack_handler: Arc<Mutex<IrqAckHandler>>,
    vmm: Arc<Hypervisor>,
    irqfd: Arc<EventFd>,
    file_path: PathBuf,
    read_only: bool,
    sub_id: Option<SubscriberId>,

    // Before resetting we return the handler to the mmio thread for cleanup
    #[allow(dead_code)]
    handler: Option<Arc<Mutex<dyn MutEventSubscriber + Send>>>,
    // We'll prob need to remember this for state save/restore unless we pass the info from
    // the outside.
    _root_device: bool,
}

impl<M> Block<M>
where
    M: GuestAddressSpace + Clone + Send + 'static,
{
    pub fn new<B>(mut args: BlockArgs<M, B>) -> Result<Arc<Mutex<Self>>>
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
        let queues = vec![Queue::new(args.common.mem, QUEUE_MAX_SIZE)];
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

        let block = Arc::new(Mutex::new(Block {
            virtio_cfg,
            mmio_cfg,
            endpoint: args.common.event_mgr.remote_endpoint(),
            irq_ack_handler,
            vmm: args.common.vmm.clone(),
            irqfd,
            file_path: args.file_path,
            read_only: args.read_only,
            sub_id: None,
            handler: None,
            _root_device: args.root_device,
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

        let ioeventfd = register_ioeventfd(&self.vmm, &self.mmio_cfg, 0).map_err(Error::Simple)?;

        let file = OpenOptions::new()
            .read(true)
            .write(!self.read_only)
            .open(&self.file_path)
            .map_err(Error::OpenFile)?;

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

        let inner = InOrderQueueHandler {
            driver_notify,
            queue: self.virtio_cfg.queues[0].clone(),
            disk,
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
impl<M: GuestAddressSpace + Clone + Send + 'static> VirtioDeviceType for Block<M> {
    fn device_type(&self) -> u32 {
        BLOCK_DEVICE_ID
    }
}

impl<M: GuestAddressSpace + Clone + Send + 'static> Borrow<VirtioConfig<M>> for Block<M> {
    fn borrow(&self) -> &VirtioConfig<M> {
        &self.virtio_cfg
    }
}

impl<M: GuestAddressSpace + Clone + Send + 'static> BorrowMut<VirtioConfig<M>> for Block<M> {
    fn borrow_mut(&mut self) -> &mut VirtioConfig<M> {
        &mut self.virtio_cfg
    }
}

impl<M: GuestAddressSpace + Clone + Send + 'static> VirtioDeviceActions for Block<M> {
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

impl<M: GuestAddressSpace + Clone + Send + 'static> VirtioMmioDevice<M> for Block<M> {}

impl<M: GuestAddressSpace + Clone + Send + 'static> MutDeviceMmio for Block<M> {
    fn mmio_read(&mut self, _base: MmioAddress, offset: u64, data: &mut [u8]) {
        self.read(offset, data);
    }

    fn mmio_write(&mut self, _base: MmioAddress, offset: u64, data: &[u8]) {
        self.write(offset, data);
    }
}

//#[cfg(test)]
//mod tests {
//    use vmm_sys_util::tempfile::TempFile;
//
//    use crate::virtio::tests::CommonArgsMock;
//
//    use super::*;
//
//    // Restricting this for now, because registering irqfds does not work on Arm without properly
//    // setting up the equivalent of the irqchip first (as part of `CommonArgsContext::new`).
//    #[cfg_attr(target_arch = "aarch64", ignore)]
//    #[test]
//    fn test_device() {
//        let tmp = TempFile::new().unwrap();
//
//        {
//            let mut mock = CommonArgsMock::new();
//            let common_args = mock.args();
//            let args = BlockArgs {
//                common: common_args,
//                file_path: tmp.as_path().to_path_buf(),
//                read_only: false,
//                root_device: true,
//                advertise_flush: true,
//            };
//
//            let block_mutex = Block::new(args).unwrap();
//            let mut block = block_mutex.lock().unwrap();
//
//            // The read-only feature should not be present.
//            assert_eq!(block.virtio_cfg.device_features & (1 << VIRTIO_BLK_F_RO), 0);
//            // The flush feature should be present.
//            assert_ne!(
//                block.virtio_cfg.device_features & (1 << VIRTIO_BLK_F_FLUSH),
//                0
//            );
//
//            // Some quick sanity checks. Most of the functionality around the device should be
//            // exercised/validated via integration tests.
//
//            let range = mock.mmio_cfg.range;
//
//            let bus_range = mock.mmio_mgr.mmio_device(range.base()).unwrap().0;
//            assert_eq!(bus_range.base(), range.base());
//            assert_eq!(bus_range.size(), range.size());
//
//            assert_eq!(
//                mock.kernel_cmdline.as_str(),
//                format!(
//                    "virtio_mmio.device=4K@0x{:x}:{} root=/dev/vda",
//                    range.base().0,
//                    mock.mmio_cfg.gsi
//                )
//            );
//
//            assert_eq!(block.device_type(), BLOCK_DEVICE_ID);
//
//            assert!(matches!(block.activate(), Err(Error::QueuesNotValid)));
//
//            block.virtio_config_mut().device_activated = true;
//            assert_eq!(block.virtio_config().device_activated, true);
//            assert!(matches!(block.activate(), Err(Error::AlreadyActivated)));
//        }
//
//        // Test a read-only root device.
//        {
//            let mut mock = CommonArgsMock::new();
//            let common_args = mock.args();
//            let args = BlockArgs {
//                common: common_args,
//                file_path: tmp.as_path().to_path_buf(),
//                read_only: true,
//                root_device: true,
//                advertise_flush: true,
//            };
//
//            let block_mutex = Block::new(args).unwrap();
//            let block = block_mutex.lock().unwrap();
//
//            // The read-only feature should be present.
//            assert_ne!(block.virtio_cfg.device_features & (1 << VIRTIO_BLK_F_RO), 0);
//
//            assert_eq!(
//                mock.kernel_cmdline.as_str(),
//                format!(
//                    "virtio_mmio.device=4K@0x{:x}:{} root=/dev/vda ro",
//                    mock.mmio_cfg.range.base().0,
//                    mock.mmio_cfg.gsi
//                )
//            );
//        }
//
//        // Test a block device with root and advertise flush not enabled.
//        {
//            {
//                let mut mock = CommonArgsMock::new();
//                let common_args = mock.args();
//                let args = BlockArgs {
//                    common: common_args,
//                    file_path: tmp.as_path().to_path_buf(),
//                    read_only: true,
//                    root_device: false,
//                    advertise_flush: false,
//                };
//
//                let block_mutex = Block::new(args).unwrap();
//                let block = block_mutex.lock().unwrap();
//
//                // The read-only feature should be present.
//                assert_ne!(block.virtio_cfg.device_features & (1 << VIRTIO_BLK_F_RO), 0);
//                // The flush feature should not be present.
//                assert_eq!(
//                    block.virtio_cfg.device_features & (1 << VIRTIO_BLK_F_FLUSH),
//                    0
//                );
//
//                assert_eq!(
//                    mock.kernel_cmdline.as_str(),
//                    format!(
//                        "virtio_mmio.device=4K@0x{:x}:{}",
//                        mock.mmio_cfg.range.base().0,
//                        mock.mmio_cfg.gsi
//                    )
//                );
//            }
//        }
//    }
//}
