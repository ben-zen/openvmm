# NetVSP & MANA NIC Lifecycle Diagrams

This document describes the lifecycle of the NetVSP synthetic NIC and the
MANA hardware NIC, from VTL2 boot through VF data-path switching and
teardown. Each diagram calls out the key functions involved.

## 1. VTL2 Startup & MANA Initialization

This diagram shows the boot sequence from the OpenHCL VTL2 entry point
through MANA hardware discovery and the creation of the synthetic NIC
offered to the VTL0 guest.

```mermaid
sequenceDiagram
    participant Boot as openhcl_boot
    participant Init as underhill_init
    participant Core as underhill_core
    participant Settings as vtl2_settings_worker
    participant Worker as UhVmNetworkSettings
    participant VFMgr as HclNetworkVFManager
    participant MANA as ManaDevice
    participant GDMA as GdmaDriver
    participant NetVSP as netvsp::Nic
    participant VMBus as VmbusServer

    Boot->>Init: exec /underhill-init
    Note over Init: mount /proc, /sys, /dev<br/>load kernel modules
    Init->>Core: exec /bin/openvmm_hcl

    Core->>Core: underhill_core::main()<br/>→ do_main() → run_control()
    Core->>Core: launch_workers()<br/>→ new_underhill_vm()

    Core->>Settings: InitialControllers::new()
    Settings->>Settings: get_mana_config_from_vtl2_settings()
    Settings->>Settings: wait_for_mana()<br/>PCI uevent for 1414:00ba

    Core->>Worker: add_network() for each NIC

    Worker->>VFMgr: HclNetworkVFManager::new()
    VFMgr->>MANA: create_mana_device()
    Note over VFMgr: VfioDevice::new() opens VFIO handle

    MANA->>GDMA: GdmaDriver::new()
    Note over GDMA: Map BAR0 registers<br/>Write ESTABLISH_HWC<br/>Wait for EQ/interrupt<br/>Read CQ/RQ/SQ/db/pdid/gpa_mkey

    MANA->>GDMA: test_eq()
    MANA->>GDMA: verify_vf_driver_version()
    MANA->>GDMA: list_devices()<br/>→ find GDMA_DEVICE_MANA
    MANA->>GDMA: register_device()
    MANA->>GDMA: BnicDriver::query_dev_config()
    MANA->>GDMA: check_vf_resources()

    VFMgr->>MANA: start_notification_task()<br/>subscribe HWC events
    VFMgr->>MANA: subscribe_vf_reconfig()

    VFMgr->>VFMgr: connect_endpoints()
    Note over VFMgr: For each vport:
    VFMgr->>MANA: device.new_vport()
    VFMgr->>MANA: vport.set_serial_no()
    Note over VFMgr: ManaEndpoint::new(vport)<br/>endpoint_control.connect()

    VFMgr->>VFMgr: HclNetworkVFManagerWorker spawned<br/>→ run() event loop

    Worker->>NetVSP: Nic::builder()<br/>.virtual_function(vf_manager.create_function())<br/>.max_queues() / .limit_ring_buffer()<br/>.build()
    Note over NetVSP: Creates Adapter,<br/>CoordinatorState

    Worker->>VMBus: offer_channel_unit(nic)
    Note over VMBus: NIC channel offered<br/>to VTL0 guest
```

## 2. Adding a Virtual NIC to the VTL0 Guest

Once the VMBus channel is offered, the VTL0 guest opens it and negotiates
the NVSP protocol, sets up ring buffers and RNDIS, and receives the VF
association advertisement.

```mermaid
sequenceDiagram
    participant Guest as VTL0 Guest
    participant VMBus as VMBus Channel
    participant NicDev as Nic (VmbusDevice)
    participant Coord as Coordinator
    participant PriWorker as Primary Worker
    participant Channel as NetChannel
    participant VF as VirtualFunction

    Guest->>VMBus: Open primary channel (idx=0)
    VMBus->>NicDev: open(channel_idx=0)

    NicDev->>Coord: insert_coordinator()
    Note over Coord: Creates Coordinator task<br/>with endpoint + VF state

    NicDev->>PriWorker: insert_worker(idx=0)<br/>state = WorkerState::Init
    NicDev->>Coord: coordinator.start()

    PriWorker->>Channel: NetChannel::initialize()
    Note over Channel: 1. Negotiate NVSP version<br/>   (Version::V1 through V61)<br/>2. Exchange NDIS version/config<br/>3. Setup receive buffer GPADL<br/>   → ReceiveBuffer::new()<br/>4. Setup send buffer GPADL<br/>   → SendBuffer::new()

    Channel-->>PriWorker: Init complete
    PriWorker->>PriWorker: state → WaitingForCoordinator

    Coord->>Coord: restart_queues()
    Note over Coord: Query data path state<br/>restore_guest_vf_state()

    Coord->>PriWorker: CoordinatorMessage::Restart
    PriWorker->>PriWorker: state → Ready

    Note over PriWorker: Begin processing ring buffer:<br/>TX (RndisPacket) / RX (RndisPacketComplete)

    Guest->>Channel: RNDIS Initialize
    Channel->>Channel: handle_rndis_control_message()<br/>rndis_state → Operational

    Guest->>Channel: RNDIS Set (OID_GEN_CURRENT_PACKET_FILTER)
    Note over Channel: Packet filter applied<br/>→ CoordinatorMessage::Update

    Guest->>VMBus: SubChannelRequest(num_sub_channels)
    Channel->>Channel: PacketData::SubChannelRequest
    Note over Channel: Approve up to max_queues-1<br/>subchannels

    Coord->>Coord: CoordinatorMessage::Restart
    Note over Coord: Open subchannels,<br/>start subchannel workers

    loop For each subchannel
        Guest->>VMBus: Open subchannel (idx=N)
        VMBus->>NicDev: open(channel_idx=N)
        NicDev->>PriWorker: insert_worker(idx=N)<br/>state = WorkerState::Ready
    end

    Note over Coord: All channels opened

    Coord->>Channel: guest_send_indirection_table()
    Channel->>Guest: MESSAGE5_TYPE_SEND_INDIRECTION_TABLE

    Note over Coord: Check VF availability
    Coord->>VF: vf.id() → Some(vfid)
    Note over Coord: guest_vf_state:<br/>Initializing → Available{vfid}

    Coord->>PriWorker: stop worker[0]
    PriWorker->>Channel: handle_state_change()
    Channel->>Channel: guest_vf_is_available(vfid)
    Channel->>Guest: MESSAGE4_TYPE_SEND_VF_ASSOCIATION<br/>{vf_allocated: 1, serial_number}
    Note over Channel: guest_vf_state:<br/>Available → AvailableAdvertised

    Coord->>Coord: CoordinatorStatePendingVfState::Delay<br/>wait VF_DEVICE_DELAY
    Note over Coord: Timer expires → OfferVfDevice
    Coord->>Coord: guest_vf_state:<br/>AvailableAdvertised → Ready
    Coord->>Coord: pending_vf_state → Pending
    Coord->>VF: guest_ready_for_device()
```

## 3. VF Data Path Switch: Synthetic → VF (Accelerated Networking)

When the guest is ready and the VF hardware is available, the data path
switches from the synthetic NetVSP path through VTL2 to direct VF
passthrough to the VTL0 guest.

```mermaid
sequenceDiagram
    participant Guest as VTL0 Guest
    participant NetVSP as NetVSP Worker
    participant Coord as Coordinator
    participant VFTrait as VirtualFunction
    participant VFMgrInst as HclNetworkVFManagerInstance
    participant VFMgrWkr as HclNetworkVFManagerWorker
    participant ManaEP as ManaEndpoint
    participant Vport as mana_driver::Vport
    participant BNIC as BnicDriver (HWC)

    Note over VFMgrWkr: VF available, VTL2 device started
    VFMgrWkr->>VFMgrWkr: startup_vtl2_device()<br/>→ connect_endpoints()
    VFMgrWkr->>VFMgrWkr: notify_vtl0_vf_arrival()

    Note over Coord: Coordinator receives<br/>UpdateFromVf notification
    Coord->>Coord: update_guest_vf_state()

    Note over Coord: VF advertised to guest<br/>(see Diagram 2)

    Coord->>VFTrait: guest_ready_for_device()
    VFTrait->>VFMgrInst: guest_ready_for_device()
    VFMgrInst->>VFMgrInst: set_vport_ready_and_get_vf_state()
    VFMgrInst->>VFMgrWkr: send(AddVtl0VF)

    VFMgrWkr->>VFMgrWkr: Handle AddVtl0VF
    Note over VFMgrWkr: Vtl0Bus::Present →<br/>offer_device()
    VFMgrWkr->>Guest: VTL0 VF PCI device offered
    Note over VFMgrWkr: guest_state.offered_to_guest = true

    Guest->>Guest: Guest OS enumerates VF PCI device<br/>(netvsc driver detects VF arrival)

    Guest->>NetVSP: PacketData::SwitchDataPath<br/>{active_data_path: VF}
    NetVSP->>NetVSP: switch_data_path(use_guest_vf=true)
    Note over NetVSP: guest_vf_state:<br/>Ready → DataPathSwitchPending<br/>{to_guest: true}
    NetVSP->>Coord: send_coordinator_update_vf()

    Coord->>Coord: handle_coordinator_message(Update)
    Coord->>Coord: update_guest_vf_state()

    Note over Coord: Stop primary worker,<br/>process pending switch
    Coord->>ManaEP: endpoint.set_data_path_to_guest_vf(true)
    ManaEP->>Vport: vport.move_filter(1)
    Note over Vport: direction_to_vtl0 = true
    Vport->>BNIC: BnicDriver::move_vport_filter()<br/>→ MANA_VTL2_MOVE_FILTER
    Note over BNIC: MAC filter ownership<br/>moves to VTL0 VF vport

    BNIC-->>Vport: Success
    Vport-->>ManaEP: Ok
    ManaEP-->>Coord: Ok
    Note over Coord: is_data_path_switched = Some(true)

    Coord->>NetVSP: Restart primary worker
    NetVSP->>NetVSP: handle_state_change()
    Note over NetVSP: DataPathSwitchPending{result: Some(true)}<br/>→ send_completion to guest<br/>→ guest_vf_state: DataPathSwitched

    NetVSP->>Guest: Completion (success)

    Note over Guest: Data now flows directly<br/>Guest ↔ MANA VF hardware<br/>(bypasses VTL2 synthetic path)
```

## 4. VF Data Path Switch Back: VF → Synthetic (Failback)

This can happen due to guest-initiated switchback, VF removal (live
migration, servicing), or hardware reconfiguration. Two sub-flows
are shown.

### 4a. Guest-Initiated Switchback

```mermaid
sequenceDiagram
    participant Guest as VTL0 Guest
    participant NetVSP as NetVSP Worker
    participant Coord as Coordinator
    participant ManaEP as ManaEndpoint
    participant Vport as mana_driver::Vport
    participant BNIC as BnicDriver (HWC)

    Guest->>NetVSP: PacketData::SwitchDataPath<br/>{active_data_path: SYNTHETIC}
    NetVSP->>NetVSP: switch_data_path(use_guest_vf=false)
    Note over NetVSP: guest_vf_state:<br/>DataPathSwitched →<br/>DataPathSwitchPending<br/>{to_guest: false}
    NetVSP->>Coord: send_coordinator_update_vf()

    Coord->>Coord: update_guest_vf_state()
    Coord->>ManaEP: endpoint.set_data_path_to_guest_vf(false)
    ManaEP->>Vport: vport.move_filter(0)
    Note over Vport: direction_to_vtl0 = false
    Vport->>BNIC: BnicDriver::move_vport_filter()<br/>→ MANA_VTL2_MOVE_FILTER
    Note over BNIC: MAC filter ownership<br/>returns to VTL2 VF vport

    BNIC-->>Vport: Success
    Vport-->>ManaEP: Ok
    ManaEP-->>Coord: Ok
    Note over Coord: is_data_path_switched = Some(false)

    Coord->>NetVSP: Restart primary worker
    NetVSP->>NetVSP: handle_state_change()
    Note over NetVSP: DataPathSwitchPending{to_guest:false, result:true}<br/>→ guest_vf_state: Ready
    NetVSP->>Guest: Completion (success)

    Note over Guest: Data now flows through<br/>VTL2 synthetic (NetVSP) path
```

### 4b. VF Removal / Hardware Reconfiguration (Host-Initiated)

```mermaid
sequenceDiagram
    participant Host as Host / Hardware
    participant UEvent as UEvent Listener
    participant VFMgrWkr as HclNetworkVFManagerWorker
    participant VFMgrInst as VirtualFunction Instance
    participant Coord as Coordinator
    participant NetVSP as NetVSP Worker
    participant ManaEP as ManaEndpoint
    participant Vport as mana_driver::Vport
    participant Guest as VTL0 Guest

    Host->>UEvent: PCI device removal event
    UEvent->>VFMgrWkr: ManaDeviceRemoved

    VFMgrWkr->>VFMgrWkr: try_notify_guest_and_revoke_vtl0_vf()
    Note over VFMgrWkr: send_vf_state_change_notifications()<br/>→ notify netvsp coordinators

    VFMgrWkr->>VFMgrInst: HclNetworkVFUpdateNotification::Update
    VFMgrInst-->>Coord: wait_for_state_change() resolves

    Coord->>Coord: UpdateFromVf message
    Coord->>Coord: update_guest_vf_state()
    Note over Coord: VF id() → None<br/>guest_vf_state transitions to<br/>Unavailable variants

    alt Data path was switched to VF
        Note over Coord: guest_vf_state:<br/>→ UnavailableFromDataPathSwitched
        Coord->>NetVSP: Stop worker, handle_state_change()

        NetVSP->>NetVSP: guest_vf_data_path_switched_to_synthetic()
        NetVSP->>Guest: MESSAGE4_TYPE_SWITCH_DATA_PATH<br/>{active_data_path: SYNTHETIC}
        Note over NetVSP: → UnavailableFromAvailable

        NetVSP->>NetVSP: guest_vf_is_available(None)
        NetVSP->>Guest: MESSAGE4_TYPE_SEND_VF_ASSOCIATION<br/>{vf_allocated: 0}
        Note over NetVSP: → Unavailable
    else Data path was synthetic
        Note over Coord: guest_vf_state:<br/>→ UnavailableFromAvailable
        Coord->>NetVSP: Stop worker, handle_state_change()
        NetVSP->>NetVSP: guest_vf_is_available(None)
        NetVSP->>Guest: MESSAGE4_TYPE_SEND_VF_ASSOCIATION<br/>{vf_allocated: 0}
        Note over NetVSP: → Unavailable
    end

    Note over VFMgrWkr: On notification error,<br/>force synthetic fallback:

    opt Notification failed
        VFMgrWkr->>ManaEP: endpoint.set_data_path_to_guest_vf(false)
        ManaEP->>Vport: vport.move_filter(0)
        Note over VFMgrWkr: Force MAC filter to VTL2
    end

    VFMgrWkr->>VFMgrWkr: vtl0_bus_control.revoke_device()
    Note over VFMgrWkr: guest_state.offered_to_guest = false

    VFMgrWkr->>VFMgrWkr: shutdown_vtl2_device()
    Note over VFMgrWkr: disconnect_all_endpoints()<br/>→ ManaDevice::shutdown()

    Note over Guest: All traffic now flows<br/>through VTL2 synthetic path<br/>(or NIC is fully removed)
```

## 5. VF State Machine Summary

The `PrimaryChannelGuestVfState` enum in `netvsp` drives all VF-related
guest interactions. Here is the complete state machine:

```mermaid
stateDiagram-v2
    [*] --> Initializing

    Initializing --> Available: VF id() returns Some(vfid)
    Initializing --> Unavailable: VF id() returns None

    Available --> AvailableAdvertised: guest_vf_is_available(vfid)<br/>sends SEND_VF_ASSOCIATION

    AvailableAdvertised --> Ready: VF_DEVICE_DELAY expires<br/>guest_ready_for_device()

    Ready --> DataPathSwitchPending_toVF: Guest sends SwitchDataPath{VF}<br/>switch_data_path(true)
    Ready --> DataPathSynthetic: External state change

    DataPathSwitchPending_toVF --> DataPathSwitched: move_filter(1) succeeds<br/>send_completion()
    DataPathSwitchPending_toVF --> DataPathSynthetic: move_filter(1) fails<br/>send_completion()
    DataPathSwitchPending_toVF --> UnavailableFromDPSwitchPending: VF removed

    DataPathSwitched --> DataPathSwitchPending_toSynth: Guest sends SwitchDataPath{SYNTHETIC}<br/>switch_data_path(false)
    DataPathSwitched --> UnavailableFromDataPathSwitched: VF removed

    DataPathSwitchPending_toSynth --> Ready: move_filter(0) succeeds
    DataPathSwitchPending_toSynth --> DataPathSwitched: move_filter(0) fails

    DataPathSynthetic --> Ready: guest_vf_data_path_switched_to_synthetic()<br/>notifies guest

    UnavailableFromDataPathSwitched --> UnavailableFromAvailable: guest_vf_data_path_switched_to_synthetic()
    UnavailableFromDPSwitchPending --> UnavailableFromDataPathSwitched: send_completion() (was to_guest)
    UnavailableFromDPSwitchPending --> UnavailableFromAvailable: send_completion() (was to_synth)
    UnavailableFromAvailable --> Unavailable: guest_vf_is_available(None)<br/>sends SEND_VF_ASSOCIATION{0}

    Available --> UnavailableFromAvailable: VF removed
    AvailableAdvertised --> UnavailableFromAvailable: VF removed

    Unavailable --> Available: VF arrives again
    Unavailable --> [*]: NIC shutdown
```

## Key Source Locations

| Component | File | Key Functions |
|-----------|------|---------------|
| VTL2 entry | `openhcl/underhill_entry/src/lib.rs` | `underhill_main()` |
| VM worker setup | `openhcl/underhill_core/src/worker.rs` | `new_underhill_vm()`, `add_network()`, `new_underhill_nic()` |
| MANA PCI discovery | `openhcl/underhill_core/src/dispatch/vtl2_settings_worker.rs` | `wait_for_mana()`, `InitialControllers::new()` |
| VF Manager | `openhcl/underhill_core/src/emuplat/netvsp.rs` | `HclNetworkVFManager::new()`, `HclNetworkVFManagerWorker::run()` |
| VF Manager lifecycle | `openhcl/underhill_core/src/emuplat/netvsp.rs` | `startup_vtl2_device()`, `connect_endpoints()`, `shutdown_vtl2_device()` |
| VF Manager guest ops | `openhcl/underhill_core/src/emuplat/netvsp.rs` | `try_notify_guest_and_revoke_vtl0_vf()`, `notify_vtl0_vf_arrival()` |
| MANA driver init | `vm/devices/net/mana_driver/src/mana.rs` | `ManaDevice::new()`, `start_notification_task()` |
| GDMA driver | `vm/devices/net/mana_driver/src/gdma_driver.rs` | `GdmaDriver::new()`, `GdmaDriver::restore()` |
| MANA vport / filter | `vm/devices/net/mana_driver/src/mana.rs` | `Vport::move_filter()`, `Vport::query_filter_state()` |
| MANA endpoint | `vm/devices/net/net_mana/src/lib.rs` | `ManaEndpoint::new()`, `set_data_path_to_guest_vf()` |
| NetVSP NIC | `vm/devices/net/netvsp/src/lib.rs` | `Nic::builder()`, `NicBuilder::build()` |
| NetVSP VMBus device | `vm/devices/net/netvsp/src/lib.rs` | `open()`, `close()`, `start()`, `stop()` |
| NetVSP coordinator | `vm/devices/net/netvsp/src/lib.rs` | `Coordinator::process()`, `update_guest_vf_state()`, `restore_guest_vf_state()` |
| NetVSP worker | `vm/devices/net/netvsp/src/lib.rs` | `Worker::process()`, `handle_state_change()`, `switch_data_path()` |
| NetVSP VF messaging | `vm/devices/net/netvsp/src/lib.rs` | `guest_vf_is_available()`, `guest_vf_data_path_switched_to_synthetic()` |
| NVSP protocol | `vm/devices/net/netvsp/src/protocol.rs` | `Message4SendVfAssociation`, `Message4SwitchDataPath`, `DataPath` |
