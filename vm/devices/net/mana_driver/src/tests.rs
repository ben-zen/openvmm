// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! This module drives the MANA emuulator with the MANA driver to test the
//! end-to-end flow.

use crate::bnic_driver::BnicDriver;
use crate::bnic_driver::RxConfig;
use crate::bnic_driver::WqConfig;
use crate::gdma_driver::GdmaDriver;
use crate::mana::ManaDevice;
use crate::mana::ResourceArena;
use crate::mana::Vport;
use chipset_device::mmio::ExternallyManagedMmioIntercepts;
use gdma::VportConfig;
use gdma_defs::GdmaDevType;
use gdma_defs::GdmaQueueType;
use gdma_defs::bnic::ManaQueryDeviceCfgResp;
use net_backend::null::NullEndpoint;
use page_pool_alloc::PagePoolAllocator;
use pal_async::DefaultDriver;
use pal_async::async_test;
use pci_core::msi::MsiConnection;
use std::sync::Arc;
use test_with_tracing::test;
use user_driver::DeviceBacking;
use user_driver::memory::PAGE_SIZE;
use user_driver_emulated_mock::DeviceTestMemory;
use user_driver_emulated_mock::EmulatedDevice;
use vmcore::vm_task::SingleDriverBackend;
use vmcore::vm_task::VmTaskDriverSource;

#[async_test]
async fn test_gdma(driver: DefaultDriver) {
    let mem = DeviceTestMemory::new(128, false, "test_gdma");
    let msi_conn = MsiConnection::new();
    let device = gdma::GdmaDevice::new(
        &VmTaskDriverSource::new(SingleDriverBackend::new(driver.clone())),
        mem.guest_memory(),
        msi_conn.target(),
        vec![VportConfig {
            mac_address: [1, 2, 3, 4, 5, 6].into(),
            endpoint: Box::new(NullEndpoint::new()),
        }],
        &mut ExternallyManagedMmioIntercepts,
    );
    let dma_client = mem.dma_client();
    let device = EmulatedDevice::new(device, msi_conn, dma_client);
    let dma_client = device.dma_client();
    let buffer = dma_client.allocate_dma_buffer(6 * PAGE_SIZE).unwrap();

    let mut gdma = GdmaDriver::new(&driver, device, 1, Some(buffer))
        .await
        .unwrap();
    gdma.test_eq().await.unwrap();
    gdma.verify_vf_driver_version().await.unwrap();
    let dev_id = gdma
        .list_devices()
        .await
        .unwrap()
        .iter()
        .copied()
        .find(|dev_id| dev_id.ty == GdmaDevType::GDMA_DEVICE_MANA)
        .unwrap();

    let device_props = gdma.register_device(dev_id).await.unwrap();
    let mut bnic = BnicDriver::new(&mut gdma, dev_id);
    let _dev_config = bnic.query_dev_config().await.unwrap();
    let port_config = bnic.query_vport_config(0).await.unwrap();
    let vport = port_config.vport;
    let buffer = Arc::new(
        gdma.device()
            .dma_client()
            .allocate_dma_buffer(0x5000)
            .unwrap(),
    );
    let mut arena = ResourceArena::new();
    let eq_gdma_region = gdma
        .create_dma_region(&mut arena, dev_id, buffer.subblock(0, PAGE_SIZE))
        .await
        .unwrap();
    let rq_gdma_region = gdma
        .create_dma_region(&mut arena, dev_id, buffer.subblock(PAGE_SIZE, PAGE_SIZE))
        .await
        .unwrap();
    let rq_cq_gdma_region = gdma
        .create_dma_region(
            &mut arena,
            dev_id,
            buffer.subblock(2 * PAGE_SIZE, PAGE_SIZE),
        )
        .await
        .unwrap();
    let sq_gdma_region = gdma
        .create_dma_region(
            &mut arena,
            dev_id,
            buffer.subblock(3 * PAGE_SIZE, PAGE_SIZE),
        )
        .await
        .unwrap();
    let sq_cq_gdma_region = gdma
        .create_dma_region(
            &mut arena,
            dev_id,
            buffer.subblock(4 * PAGE_SIZE, PAGE_SIZE),
        )
        .await
        .unwrap();
    let (eq_id, _) = gdma
        .create_eq(
            &mut arena,
            dev_id,
            eq_gdma_region,
            PAGE_SIZE as u32,
            device_props.pdid,
            device_props.db_id,
            0,
        )
        .await
        .unwrap();
    let mut bnic = BnicDriver::new(&mut gdma, dev_id);
    let _rq_cfg = bnic
        .create_wq_obj(
            &mut arena,
            vport,
            GdmaQueueType::GDMA_RQ,
            &WqConfig {
                wq_gdma_region: rq_gdma_region,
                cq_gdma_region: rq_cq_gdma_region,
                wq_size: PAGE_SIZE as u32,
                cq_size: PAGE_SIZE as u32,
                cq_moderation_ctx_id: 0,
                eq_id,
            },
        )
        .await
        .unwrap();
    let _sq_cfg = bnic
        .create_wq_obj(
            &mut arena,
            vport,
            GdmaQueueType::GDMA_SQ,
            &WqConfig {
                wq_gdma_region: sq_gdma_region,
                cq_gdma_region: sq_cq_gdma_region,
                wq_size: PAGE_SIZE as u32,
                cq_size: PAGE_SIZE as u32,
                cq_moderation_ctx_id: 0,
                eq_id,
            },
        )
        .await
        .unwrap();
    bnic.config_vport_tx(vport, 0, 0).await.unwrap();
    bnic.config_vport_rx(
        vport,
        &RxConfig {
            rx_enable: Some(true),
            rss_enable: Some(false),
            hash_key: None,
            default_rxobj: None,
            indirection_table: None,
        },
    )
    .await
    .unwrap();
    arena.destroy(&mut gdma).await;
}

#[async_test]
async fn test_gdma_save_restore(driver: DefaultDriver) {
    let mem = DeviceTestMemory::new(128, false, "test_gdma");
    let msi_conn = MsiConnection::new();
    let device = gdma::GdmaDevice::new(
        &VmTaskDriverSource::new(SingleDriverBackend::new(driver.clone())),
        mem.guest_memory(),
        msi_conn.target(),
        vec![VportConfig {
            mac_address: [1, 2, 3, 4, 5, 6].into(),
            endpoint: Box::new(NullEndpoint::new()),
        }],
        &mut ExternallyManagedMmioIntercepts,
    );
    let dma_client = mem.dma_client();

    let device = EmulatedDevice::new(device, msi_conn, dma_client);
    let cloned_device = device.clone();

    let dma_client = device.dma_client();
    let gdma_buffer = dma_client.allocate_dma_buffer(6 * PAGE_SIZE).unwrap();

    let saved_state = {
        let mut gdma = GdmaDriver::new(&driver, device, 1, Some(gdma_buffer.clone()))
            .await
            .unwrap();

        gdma.test_eq().await.unwrap();
        gdma.verify_vf_driver_version().await.unwrap();
        gdma.save().await.unwrap()
    };

    let mut new_gdma = GdmaDriver::restore(saved_state, cloned_device, gdma_buffer)
        .await
        .unwrap();

    // Validate that the new driver still works after restoration.
    new_gdma.test_eq().await.unwrap();
}

#[async_test]
async fn test_gdma_reconfig_vf(driver: DefaultDriver) {
    let mem = DeviceTestMemory::new(128, false, "test_gdma");
    let msi_conn = MsiConnection::new();
    let device = gdma::GdmaDevice::new(
        &VmTaskDriverSource::new(SingleDriverBackend::new(driver.clone())),
        mem.guest_memory(),
        msi_conn.target(),
        vec![VportConfig {
            mac_address: [1, 2, 3, 4, 5, 6].into(),
            endpoint: Box::new(NullEndpoint::new()),
        }],
        &mut ExternallyManagedMmioIntercepts,
    );
    let dma_client = mem.dma_client();
    let device = EmulatedDevice::new(device, msi_conn, dma_client);
    let dma_client = device.dma_client();
    let buffer = dma_client.allocate_dma_buffer(6 * PAGE_SIZE).unwrap();

    let mut gdma = GdmaDriver::new(&driver, device, 1, Some(buffer))
        .await
        .unwrap();

    assert!(
        !gdma.get_vf_reconfiguration_pending(),
        "vf_reconfiguration_pending should be false"
    );

    // Get the device ID while HWC is still alive (needed for deregister later).
    let dev_id = gdma
        .list_devices()
        .await
        .unwrap()
        .iter()
        .copied()
        .find(|dev_id| dev_id.ty == GdmaDevType::GDMA_DEVICE_MANA)
        .unwrap();

    // Trigger the reconfig event (EQE 135).
    gdma.generate_reconfig_vf_event().await.unwrap();

    assert!(
        gdma.get_vf_reconfiguration_pending(),
        "vf_reconfiguration_pending should be true after reconfig event"
    );

    // Deregister should fail immediately because vf_reconfiguration_pending is set.
    let deregister_result = gdma.deregister_device(dev_id).await;
    let err = deregister_result.expect_err("deregister_device should fail after EQE 135");
    let err_msg = format!("{err:#}");
    assert!(
        err_msg.contains("VF reconfiguration pending"),
        "unexpected error: {err_msg}"
    );
    assert!(
        gdma.get_vf_reconfiguration_pending(),
        "vf_reconfiguration_pending should remain true after deregister_device"
    );
}

/// Creates a ManaDevice, obtains a Vport, then shuts down the device so that
/// the Vport's `inner_weak` can no longer be upgraded.
async fn create_orphaned_vport(
    driver: &DefaultDriver,
) -> Vport<EmulatedDevice<gdma::GdmaDevice, PagePoolAllocator>> {
    let mem = DeviceTestMemory::new(128, false, "test_vport_orphan");
    let msi_conn = MsiConnection::new();
    let device = gdma::GdmaDevice::new(
        &VmTaskDriverSource::new(SingleDriverBackend::new(driver.clone())),
        mem.guest_memory(),
        msi_conn.target(),
        vec![VportConfig {
            mac_address: [1, 2, 3, 4, 5, 6].into(),
            endpoint: Box::new(NullEndpoint::new()),
        }],
        &mut ExternallyManagedMmioIntercepts,
    );
    let dma_client = mem.dma_client();
    let device = EmulatedDevice::new(device, msi_conn, dma_client);
    let dev_config = ManaQueryDeviceCfgResp {
        pf_cap_flags1: 0.into(),
        pf_cap_flags2: 0,
        pf_cap_flags3: 0,
        pf_cap_flags4: 0,
        max_num_vports: 1,
        reserved: 0,
        max_num_eqs: 64,
    };
    let mana = ManaDevice::new(driver, device, 1, 1, None).await.unwrap();
    let vport = mana.new_vport(0, None, &dev_config).await.unwrap();
    let (result, _device) = mana.shutdown().await;
    result.expect("shutdown should succeed");
    vport
}

#[async_test]
async fn test_vport_accessors_after_device_shutdown(driver: DefaultDriver) {
    let vport = create_orphaned_vport(&driver).await;

    // Config-based accessors don't depend on inner_weak and should still work.
    assert_eq!(vport.id(), 0);
    let _mac = vport.mac_address();
    let _tx_queues = vport.max_tx_queues();
    let _rx_queues = vport.max_rx_queues();
    let _ent = vport.num_indirection_ent();
    assert_eq!(vport.get_direction_to_vtl0().await, None);
}

#[async_test]
async fn test_vport_operations_fail_after_device_shutdown(driver: DefaultDriver) {
    let vport = create_orphaned_vport(&driver).await;

    // gpa_mkey is the thinnest realize_inner wrapper; verify the error message.
    let err = vport.gpa_mkey().unwrap_err();
    assert!(
        format!("{err}").contains("VPort 0 is invalid"),
        "unexpected error: {err}"
    );

    // All async methods that go through realize_inner() should also return Err.
    assert!(vport.config_tx().await.is_err());
    assert!(
        vport
            .config_rx(&RxConfig {
                rx_enable: None,
                rss_enable: None,
                hash_key: None,
                default_rxobj: None,
                indirection_table: None,
            })
            .await
            .is_err()
    );
    let mut arena = ResourceArena::new();
    assert!(vport.new_eq(&mut arena, PAGE_SIZE as u32, 0).await.is_err());
    assert!(
        vport
            .new_wq(&mut arena, true, PAGE_SIZE as u32, PAGE_SIZE as u32, 0)
            .await
            .is_err()
    );
    assert!(vport.set_serial_no(42).await.is_err());
    assert!(vport.query_stats().await.is_err());
    assert!(vport.query_filter_state(0).await.is_err());
    assert!(vport.retarget_interrupt(0, 0).await.is_err());

    // move_filter with unknown cached state reaches realize_inner and fails.
    assert!(vport.move_filter(1).await.is_err());
}

#[async_test]
async fn test_vport_move_filter_cached_after_device_shutdown(driver: DefaultDriver) {
    let mem = DeviceTestMemory::new(128, false, "test_vport_move_filter");
    let msi_conn = MsiConnection::new();
    let device = gdma::GdmaDevice::new(
        &VmTaskDriverSource::new(SingleDriverBackend::new(driver.clone())),
        mem.guest_memory(),
        msi_conn.target(),
        vec![VportConfig {
            mac_address: [1, 2, 3, 4, 5, 6].into(),
            endpoint: Box::new(NullEndpoint::new()),
        }],
        &mut ExternallyManagedMmioIntercepts,
    );
    let dma_client = mem.dma_client();
    let device = EmulatedDevice::new(device, msi_conn, dma_client);
    let dev_config = ManaQueryDeviceCfgResp {
        pf_cap_flags1: 0.into(),
        pf_cap_flags2: 0,
        pf_cap_flags3: 0,
        pf_cap_flags4: 0,
        max_num_vports: 1,
        reserved: 0,
        max_num_eqs: 64,
    };
    let mana = ManaDevice::new(&driver, device, 1, 1, None).await.unwrap();

    // Pre-populate the cached filter direction via VportState.
    let vport_state = crate::mana::VportState::new(Some(true), None);
    let vport = mana
        .new_vport(0, Some(vport_state), &dev_config)
        .await
        .unwrap();
    assert_eq!(vport.get_direction_to_vtl0().await, Some(true));

    let (result, _device) = mana.shutdown().await;
    result.expect("shutdown should succeed");

    // Same direction uses cached state — returns Ok even after shutdown.
    vport.move_filter(1).await.unwrap();

    // Opposite direction needs realize_inner — returns Err.
    assert!(vport.move_filter(0).await.is_err());
}

#[async_test]
async fn test_vport_graceful_noop_after_device_shutdown(driver: DefaultDriver) {
    let vport = create_orphaned_vport(&driver).await;

    // destroy() and register_link_status_notifier() should not panic when
    // inner_weak is invalid — they handle the None case gracefully.
    let (sender, _receiver) = mesh::channel();
    vport.register_link_status_notifier(sender).await;
    vport.destroy(ResourceArena::new()).await;
}
