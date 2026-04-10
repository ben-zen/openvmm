# NetVSP & MANA NIC Lifecycle Diagrams

This document describes the lifecycle of the NetVSP synthetic NIC and the
MANA hardware NIC, from VTL2 boot through VF data-path switching and
teardown. Each diagram calls out the key functions involved.

## 1. VTL2 Startup & MANA Initialization

As the OpenHCL kernel boots, it runs `/underhill-init` as its PID0 process; `underhill-init` then spins up processes to serve the various functions of the firmware. `openvmm_hcl` spawns worker tasks for each of the services it offers; for NICs, it creates a separate instance of `HclNetworkVFManager` for each one attached via `HclNetworkVfManager::new`. The diagram picks up here: `HclNetworkVFManager` then creates a `ManaDevice` object, and once _that_ has finished setting up `GdmaDriver`, the VFManager establishes VPorts for all available endpoints. At this point, `UhVmNetworkSettings` offers the NIC to the VTL0 guest.

```mermaid
sequenceDiagram
    participant Worker as UhVmNetworkSettings
    participant VFMgr as HclNetworkVFManager
    participant MANA as ManaDevice
    participant NetVSP as netvsp::Nic
    participant VMBus as VmbusServer

    Worker->>VFMgr: HclNetworkVFManager::new()
    VFMgr->>MANA: create_mana_device()
    Note over VFMgr: VfioDevice::new() opens VFIO handle

    Note over MANA: GdmaDriver::new()<br/>Establish connection<br/>Load resources<br/>Register device
    VFMgr->>MANA: start_notification_task()<br/>subscribe HWC events
    VFMgr->>MANA: subscribe_vf_reconfig()

    VFMgr->>VFMgr: connect_endpoints()
    loop for each vport
        VFMgr->>MANA: device.new_vport()
        VFMgr->>MANA: vport.set_serial_no()
        Note over VFMgr: ManaEndpoint::new(vport)<br/>endpoint_control.connect()
    end

    VFMgr->>VFMgr: HclNetworkVFManagerWorker spawned<br/>→ run() event loop

    Worker->>NetVSP: Nic::builder()<br/>.virtual_function(vf_manager.create_function())<br/>.max_queues() / .limit_ring_buffer()<br/>.build()
    Note over NetVSP: Creates Adapter,<br/>CoordinatorState

    Worker->>VMBus: offer_channel_unit(nic)
    Note over VMBus: NIC channel offered<br/>to VTL0 guest
```

### Components

| Diagram Name | Crate Path | Role |
|---|---|---|
| UhVmNetworkSettings | `underhill_core::worker::UhVmNetworkSettings` | Orchestrates NIC creation in Underhill |
| HclNetworkVFManager | `underhill_core::emuplat::netvsp::HclNetworkVFManager` | Manages MANA VF lifecycle |
| ManaDevice | `mana_driver::mana::ManaDevice` | MANA NIC device abstraction |
| netvsp::Nic | `netvsp::Nic` | Synthetic NIC VMBus device |
| VmbusServer | `vmm_core::vmbus_unit` / `vmbus_server::VmbusServer` | VMBus channel management |

### Citations

| Diagram Action | Source |
|---|---|
| `HclNetworkVFManager::new()` | [`HclNetworkVFManager::new` in netvsp.rs @ 1462](https://github.com/microsoft/openvmm/blob/main/openhcl/underhill_core/src/emuplat/netvsp.rs#L1462) |
| `create_mana_device()` | [`create_mana_device` in netvsp.rs @ 79](https://github.com/microsoft/openvmm/blob/main/openhcl/underhill_core/src/emuplat/netvsp.rs#L79) |
| `VfioDevice::new()` | [`VfioDevice::new` in vfio.rs @ 91](https://github.com/microsoft/openvmm/blob/main/vm/devices/user_driver/src/vfio.rs#L91) |
| `GdmaDriver::new()` | [`GdmaDriver::new` in gdma_driver.rs @ 285](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/mana_driver/src/gdma_driver.rs#L285) |
| `start_notification_task()` | [`ManaDevice::start_notification_task` in mana.rs @ 208](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/mana_driver/src/mana.rs#L208) |
| `subscribe_vf_reconfig()` | [`ManaDevice::subscribe_vf_reconfig` in mana.rs @ 289](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/mana_driver/src/mana.rs#L289) |
| `connect_endpoints()` | [`HclNetworkVFManagerWorker::connect_endpoints` in netvsp.rs @ 380](https://github.com/microsoft/openvmm/blob/main/openhcl/underhill_core/src/emuplat/netvsp.rs#L380) |
| `device.new_vport()` | [`ManaDevice::new_vport` in mana.rs @ 254](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/mana_driver/src/mana.rs#L254) |
| `vport.set_serial_no()` | [`Vport::set_serial_no` in mana.rs @ 546](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/mana_driver/src/mana.rs#L546) |
| `ManaEndpoint::new(vport)` | [`ManaEndpoint::new` in lib.rs (net_mana) @ 122](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/net_mana/src/lib.rs#L122) |
| `endpoint_control.connect()` | [`HclNetworkVFManagerWorker::connect_endpoints` in netvsp.rs @ 413](https://github.com/microsoft/openvmm/blob/main/openhcl/underhill_core/src/emuplat/netvsp.rs#L413) |
| `HclNetworkVFManagerWorker::run()` | [`HclNetworkVFManagerWorker::run` in netvsp.rs @ 714](https://github.com/microsoft/openvmm/blob/main/openhcl/underhill_core/src/emuplat/netvsp.rs#L714) |
| `Nic::builder()` | [`Nic::builder` in lib.rs (netvsp) @ 1217](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/netvsp/src/lib.rs#L1217) |
| `NicBuilder::build()` | [`NicBuilder::build` in lib.rs (netvsp) @ 1091](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/netvsp/src/lib.rs#L1091) |
| `offer_channel_unit(nic)` | [`offer_channel_unit` in vmbus_unit.rs @ 112](https://github.com/microsoft/openvmm/blob/main/vmm_core/src/vmbus_unit.rs#L112) |

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

    Guest->>VMBus: Open primary channel (idx=0)
    VMBus->>NicDev: open(channel_idx=0)

    NicDev->>Coord: insert_coordinator()
    Note over Coord: Creates Coordinator task<br/>with endpoint + VF state

    NicDev->>PriWorker: insert_worker(idx=0)<br/>state = WorkerState::Init
    NicDev->>Coord: coordinator.start()

    PriWorker->>Channel: NetChannel::initialize()
    Note over Channel: 1. Negotiate NVSP version<br/>2. Exchange NDIS version/config<br/>3. Setup send & receive buffer GPADLs

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

    Note over Coord: Check VF availability:<br/>if Some(vfid), set guest_vf_state:<br/>Initializing → Available{vfid}

    Coord->>PriWorker: stop worker[0]
    PriWorker->>Channel: handle_state_change()
    Channel->>Channel: guest_vf_is_available(vfid)
    Channel->>Guest: MESSAGE4_TYPE_SEND_VF_ASSOCIATION<br/>{vf_allocated: 1, serial_number}
    Note over Channel: guest_vf_state:<br/>Available → AvailableAdvertised

    Coord->>Coord: CoordinatorStatePendingVfState::Delay<br/>wait VF_DEVICE_DELAY
    Note over Coord: Timer expires → OfferVfDevice
    Coord->>Coord: guest_vf_state:<br/>AvailableAdvertised → Ready
    Coord->>Coord: pending_vf_state → Pending
    Note over Coord: Inform VF via<br/>guest_ready_for_device()
```

### Components

| Diagram Name | Crate Path | Role |
|---|---|---|
| VTL0 Guest | _(external)_ | VTL0 guest OS |
| VMBus Channel | `vmbus_server::VmbusServer` | VMBus transport |
| Nic (VmbusDevice) | `netvsp::Nic` (`VmbusDevice` impl) | NetVSP device — open / close / start / stop |
| Coordinator | `netvsp::Coordinator` | Coordinates VF state and worker lifecycle |
| Primary Worker | `netvsp::Worker` | Processes ring buffer I/O on primary channel |
| NetChannel | `netvsp::NetChannel` | NVSP protocol negotiation and RNDIS handling |

### Citations

| Diagram Action | Source |
|---|---|
| `open(channel_idx=0)` | [`Nic::open` in lib.rs (netvsp) @ 1264](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/netvsp/src/lib.rs#L1264) |
| `insert_coordinator()` | [`Nic::insert_coordinator` in lib.rs (netvsp) @ 1494](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/netvsp/src/lib.rs#L1494) |
| `insert_worker(idx=0)` | [`Nic::insert_worker` in lib.rs (netvsp) @ 1430](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/netvsp/src/lib.rs#L1430) |
| `coordinator.start()` | [`Nic::start` in lib.rs (netvsp) @ 1382](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/netvsp/src/lib.rs#L1382) |
| `NetChannel::initialize()` | [`NetChannel::initialize` in lib.rs (netvsp) @ 4805](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/netvsp/src/lib.rs#L4805) |
| `restart_queues()` | [`Coordinator::restart_queues` in lib.rs (netvsp) @ 4413](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/netvsp/src/lib.rs#L4413) |
| `restore_guest_vf_state()` | [`Coordinator::restore_guest_vf_state` in lib.rs (netvsp) @ 4195](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/netvsp/src/lib.rs#L4195) |
| `CoordinatorMessage::Restart` | [`enum CoordinatorMessage` in lib.rs (netvsp) @ 156](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/netvsp/src/lib.rs#L156) |
| `handle_rndis_control_message()` | [`NetChannel::handle_rndis_control_message` in lib.rs (netvsp) @ 2869](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/netvsp/src/lib.rs#L2869) |
| `PacketData::SubChannelRequest` | [`Worker::process` in lib.rs (netvsp) @ 5480](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/netvsp/src/lib.rs#L5480) |
| `guest_send_indirection_table()` | [`NetChannel::guest_send_indirection_table` in lib.rs (netvsp) @ 2677](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/netvsp/src/lib.rs#L2677) |
| `guest_vf_is_available(vfid)` | [`NetChannel::guest_vf_is_available` in lib.rs (netvsp) @ 2623](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/netvsp/src/lib.rs#L2623) |
| `MESSAGE4_TYPE_SEND_VF_ASSOCIATION` | [`Message4SendVfAssociation` in protocol.rs @ 446](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/netvsp/src/protocol.rs#L446) |
| `handle_state_change()` | [`Worker::handle_state_change` in lib.rs (netvsp) @ 2767](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/netvsp/src/lib.rs#L2767) |
| `guest_ready_for_device()` | [`VirtualFunction::guest_ready_for_device` in lib.rs (netvsp) @ 336](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/netvsp/src/lib.rs#L336) |

## 3. VF Data Path Switch: Synthetic → VF (Accelerated Networking)

When the guest is ready and the VF hardware is available, the data path
switches from the synthetic NetVSP path through VTL2 to direct VF
passthrough to the VTL0 guest. This procedure has been divided into two diagrams, since the halves of the operation involve mostly independent components.

### Components

| Diagram Name | Crate Path | Role |
|---|---|---|
| HclNetworkVFManagerWorker | `underhill_core::emuplat::netvsp::HclNetworkVFManagerWorker` | Event loop for VF lifecycle messages |
| Coordinator | `netvsp::Coordinator` | VF state coordinator |
| HclNetworkVFManagerInstance | `underhill_core::emuplat::netvsp::HclNetworkVFManagerInstance` | Per-NIC `VirtualFunction` trait impl |
| ManaEndpoint | `net_mana::ManaEndpoint` | Endpoint adapter for MANA VF operations |
| mana_driver::Vport | `mana_driver::mana::Vport` | MANA virtual port — filter ownership |
| BnicDriver (HWC) | `mana_driver::bnic_driver::BnicDriver` | Hardware command channel driver |
| NetVSP Worker | `netvsp::Worker` | Primary channel worker |
| VTL0 Guest | _(external)_ | VTL0 guest OS |

### 3a. MANA device arrives & VTL2 prepares

```mermaid
sequenceDiagram
    box VTL2
    participant VFMgrWkr as HclNetworkVFManagerWorker
    participant Coord as Coordinator
    participant VFMgrInst as HclNetworkVFManagerInstance
    end
    box VTL0
    participant Guest as VTL0 Guest
    end

    Note over VFMgrWkr: VF available, VTL2 device started
    VFMgrWkr->>VFMgrWkr: startup_vtl2_device()<br/>→ connect_endpoints()
    VFMgrWkr->>VFMgrInst: notify_vtl0_vf_arrival()

    Note over Coord: Coordinator receives<br/>UpdateFromVf notification
    Coord->>Coord: update_guest_vf_state()

    Note over Coord: VF advertised to guest<br/>(see Diagram 2)

    Coord->>VFMgrInst: guest_ready_for_device()
    Note over VFMgrInst: via VirtualFunction trait
    VFMgrInst->>VFMgrInst: set_vport_ready_and_get_vf_state()
    VFMgrInst->>VFMgrWkr: send(AddVtl0VF)

    VFMgrWkr->>VFMgrWkr: Handle AddVtl0VF
    Note over VFMgrWkr: Vtl0Bus::Present →<br/>offer_device()
    VFMgrWkr->>Guest: VTL0 VF PCI device offered
    Note over VFMgrWkr: guest_state.offered_to_guest = true
```

#### Citations

| Diagram Action | Source |
|---|---|
| `startup_vtl2_device()` | [`HclNetworkVFManagerWorker::startup_vtl2_device` in netvsp.rs @ 656](https://github.com/microsoft/openvmm/blob/main/openhcl/underhill_core/src/emuplat/netvsp.rs#L656) |
| `connect_endpoints()` | [`HclNetworkVFManagerWorker::connect_endpoints` in netvsp.rs @ 380](https://github.com/microsoft/openvmm/blob/main/openhcl/underhill_core/src/emuplat/netvsp.rs#L380) |
| `notify_vtl0_vf_arrival()` | [`HclNetworkVFManagerWorker::notify_vtl0_vf_arrival` in netvsp.rs @ 536](https://github.com/microsoft/openvmm/blob/main/openhcl/underhill_core/src/emuplat/netvsp.rs#L536) |
| `update_guest_vf_state()` | [`Coordinator::update_guest_vf_state` in lib.rs (netvsp) @ 4639](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/netvsp/src/lib.rs#L4639) |
| `guest_ready_for_device()` (trait) | [`VirtualFunction::guest_ready_for_device` in lib.rs (netvsp) @ 336](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/netvsp/src/lib.rs#L336) |
| `guest_ready_for_device()` (impl) | [`HclNetworkVFManagerInstance::guest_ready_for_device` in netvsp.rs @ 1753](https://github.com/microsoft/openvmm/blob/main/openhcl/underhill_core/src/emuplat/netvsp.rs#L1753) |
| `set_vport_ready_and_get_vf_state()` | [`HclNetworkVFManagerInstance` (field callback)` in netvsp.rs @ 1755](https://github.com/microsoft/openvmm/blob/main/openhcl/underhill_core/src/emuplat/netvsp.rs#L1755) |
| `send(AddVtl0VF)` | [`HclNetworkVfManagerMessage::AddVtl0VF` in netvsp.rs @ 1768](https://github.com/microsoft/openvmm/blob/main/openhcl/underhill_core/src/emuplat/netvsp.rs#L1768) |
| Handle `AddVtl0VF` → `offer_device()` | [`HclNetworkVFManagerWorker::run` match arm` in netvsp.rs @ 836](https://github.com/microsoft/openvmm/blob/main/openhcl/underhill_core/src/emuplat/netvsp.rs#L836) |
| `offer_device()` (VTL0 bus) | [`HclVpciBusControl::offer_device` in vpci.rs @ 53](https://github.com/microsoft/openvmm/blob/main/openhcl/underhill_core/src/vpci.rs#L53) |

### 3b. VTL0 accepts MANA VF & VTL2 switches data paths

```mermaid
sequenceDiagram
    box VTL0
    participant Guest as VTL0 Guest
    end

    box VTL2
    participant NetVSP as NetVSP Worker
    participant Coord as Coordinator
    participant ManaEP as ManaEndpoint
    participant Vport as mana_driver::Vport
    participant BNIC as BnicDriver (HWC)
    end


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

#### Citations

| Diagram Action | Source |
|---|---|
| `PacketData::SwitchDataPath {VF}` | [`Message4SwitchDataPath` in protocol.rs @ 473](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/netvsp/src/protocol.rs#L473) |
| `switch_data_path(true)` | [`Worker::switch_data_path` in lib.rs (netvsp) @ 5360](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/netvsp/src/lib.rs#L5360) |
| `send_coordinator_update_vf()` | [`NetChannel::send_coordinator_update_vf` in lib.rs (netvsp) @ 3197](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/netvsp/src/lib.rs#L3197) |
| `handle_coordinator_message(Update)` | [`Coordinator::handle_coordinator_message` in lib.rs (netvsp) @ 4144](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/netvsp/src/lib.rs#L4144) |
| `set_data_path_to_guest_vf(true)` | [`ManaEndpoint::set_data_path_to_guest_vf` in lib.rs (net_mana) @ 541](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/net_mana/src/lib.rs#L541) |
| `vport.move_filter(1)` | [`Vport::move_filter` in mana.rs @ 519](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/mana_driver/src/mana.rs#L519) |
| `BnicDriver::move_vport_filter()` | [`BnicDriver::move_vport_filter` in bnic_driver.rs @ 239](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/mana_driver/src/bnic_driver.rs#L239) |
| `handle_state_change()` | [`Worker::handle_state_change` in lib.rs (netvsp) @ 2767](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/netvsp/src/lib.rs#L2767) |

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
#### Components

| Diagram Name | Crate Path | Role |
|---|---|---|
| VTL0 Guest | _(external)_ | VTL0 guest OS |
| NetVSP Worker | `netvsp::Worker` | Primary channel worker |
| Coordinator | `netvsp::Coordinator` | VF state coordinator |
| ManaEndpoint | `net_mana::ManaEndpoint` | Endpoint adapter |
| mana_driver::Vport | `mana_driver::mana::Vport` | MANA virtual port |
| BnicDriver (HWC) | `mana_driver::bnic_driver::BnicDriver` | HW command channel |

#### Citations

| Diagram Action | Source |
|---|---|
| `PacketData::SwitchDataPath {SYNTHETIC}` | [`Message4SwitchDataPath` / `DataPath` in protocol.rs @ 473 / 462](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/netvsp/src/protocol.rs#L473) |
| `switch_data_path(false)` | [`Worker::switch_data_path` in lib.rs (netvsp) @ 5360](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/netvsp/src/lib.rs#L5360) |
| `send_coordinator_update_vf()` | [`NetChannel::send_coordinator_update_vf` in lib.rs (netvsp) @ 3197](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/netvsp/src/lib.rs#L3197) |
| `update_guest_vf_state()` | [`Coordinator::update_guest_vf_state` in lib.rs (netvsp) @ 4639](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/netvsp/src/lib.rs#L4639) |
| `set_data_path_to_guest_vf(false)` | [`ManaEndpoint::set_data_path_to_guest_vf` in lib.rs (net_mana) @ 541](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/net_mana/src/lib.rs#L541) |
| `vport.move_filter(0)` | [`Vport::move_filter` in mana.rs @ 519](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/mana_driver/src/mana.rs#L519) |
| `BnicDriver::move_vport_filter()` | [`BnicDriver::move_vport_filter` in bnic_driver.rs @ 239](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/mana_driver/src/bnic_driver.rs#L239) |
| `handle_state_change()` | [`Worker::handle_state_change` in lib.rs (netvsp) @ 2767](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/netvsp/src/lib.rs#L2767) |

### 4b. VF Removal / Hardware Reconfiguration (Host-Initiated)

```mermaid
sequenceDiagram
    participant Host as Host / Hardware
    participant VFMgrWkr as HclNetworkVFManagerWorker
    participant VFMgrInst as VirtualFunction Instance
    participant Coord as Coordinator
    participant NetVSP as NetVSP Worker
    participant ManaEP as ManaEndpoint
    participant Vport as mana_driver::Vport
    participant Guest as VTL0 Guest

    Host->>VFMgrWkr: PCI device removal event
    Note over VFMgrWkr: ManaDeviceRemoved<br/>(via UEvent listener)

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

> [!NOTE]
> The filter-move sequence in the `opt` block above
> (`ManaEndpoint` → `Vport`) mirrors the full chain shown in
> Diagram 4a. See that diagram for the complete
> `set_data_path_to_guest_vf` → `move_filter` → `BnicDriver` flow.

#### Components

| Diagram Name | Crate Path | Role |
|---|---|---|
| Host / Hardware | _(external)_ | Physical host / NIC hardware |
| HclNetworkVFManagerWorker | `underhill_core::emuplat::netvsp::HclNetworkVFManagerWorker` | VF lifecycle event loop (receives UEvents) |
| VirtualFunction Instance | `underhill_core::emuplat::netvsp::HclNetworkVFManagerInstance` | Per-NIC `VirtualFunction` trait impl |
| Coordinator | `netvsp::Coordinator` | VF state coordinator |
| NetVSP Worker | `netvsp::Worker` | Primary channel worker |
| ManaEndpoint | `net_mana::ManaEndpoint` | Endpoint adapter (fallback path) |
| mana_driver::Vport | `mana_driver::mana::Vport` | MANA virtual port (fallback path) |
| VTL0 Guest | _(external)_ | VTL0 guest OS |

#### Citations

| Diagram Action | Source |
|---|---|
| PCI removal event (UEvent) | [`HclNetworkVFManagerWorker::run` in netvsp.rs @ 714](https://github.com/microsoft/openvmm/blob/main/openhcl/underhill_core/src/emuplat/netvsp.rs#L714) |
| `try_notify_guest_and_revoke_vtl0_vf()` | [`HclNetworkVFManagerWorker::try_notify_guest_and_revoke_vtl0_vf` in netvsp.rs @ 454](https://github.com/microsoft/openvmm/blob/main/openhcl/underhill_core/src/emuplat/netvsp.rs#L454) |
| `send_vf_state_change_notifications()` | [`HclNetworkVFManagerWorker::send_vf_state_change_notifications` in netvsp.rs @ 437](https://github.com/microsoft/openvmm/blob/main/openhcl/underhill_core/src/emuplat/netvsp.rs#L437) |
| `HclNetworkVFUpdateNotification::Update` | [`enum HclNetworkVFUpdateNotification` in netvsp.rs @ 1323](https://github.com/microsoft/openvmm/blob/main/openhcl/underhill_core/src/emuplat/netvsp.rs#L1323) |
| `wait_for_state_change()` | [`HclNetworkVFManagerInstance::wait_for_state_change` in netvsp.rs @ 1776](https://github.com/microsoft/openvmm/blob/main/openhcl/underhill_core/src/emuplat/netvsp.rs#L1776) |
| `update_guest_vf_state()` | [`Coordinator::update_guest_vf_state` in lib.rs (netvsp) @ 4639](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/netvsp/src/lib.rs#L4639) |
| `guest_vf_data_path_switched_to_synthetic()` | [`NetChannel::guest_vf_data_path_switched_to_synthetic` in lib.rs (netvsp) @ 2735](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/netvsp/src/lib.rs#L2735) |
| `guest_vf_is_available(None)` | [`NetChannel::guest_vf_is_available` in lib.rs (netvsp) @ 2623](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/netvsp/src/lib.rs#L2623) |
| `MESSAGE4_TYPE_SEND_VF_ASSOCIATION {0}` | [`Message4SendVfAssociation` in protocol.rs @ 446](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/netvsp/src/protocol.rs#L446) |
| `MESSAGE4_TYPE_SWITCH_DATA_PATH` | [`Message4SwitchDataPath` in protocol.rs @ 473](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/netvsp/src/protocol.rs#L473) |
| `set_data_path_to_guest_vf(false)` (opt) | [`ManaEndpoint::set_data_path_to_guest_vf` in lib.rs (net_mana) @ 541](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/net_mana/src/lib.rs#L541) |
| `vport.move_filter(0)` (opt) | [`Vport::move_filter` in mana.rs @ 519](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/mana_driver/src/mana.rs#L519) |
| `revoke_device()` | [`HclVpciBusControl::revoke_device` in vpci.rs @ 58](https://github.com/microsoft/openvmm/blob/main/openhcl/underhill_core/src/vpci.rs#L58) |
| `disconnect_all_endpoints()` | [`HclNetworkVFManagerWorker::disconnect_all_endpoints` in netvsp.rs @ 618](https://github.com/microsoft/openvmm/blob/main/openhcl/underhill_core/src/emuplat/netvsp.rs#L618) |
| `shutdown_vtl2_device()` | [`HclNetworkVFManagerWorker::shutdown_vtl2_device` in netvsp.rs @ 543](https://github.com/microsoft/openvmm/blob/main/openhcl/underhill_core/src/emuplat/netvsp.rs#L543) |
| `ManaDevice::shutdown()` | [`ManaDevice::shutdown` in mana.rs @ 302](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/mana_driver/src/mana.rs#L302) |

## 5. VF State Machine Summary

The [`netvsp::PrimaryChannelGuestVfState`](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/netvsp/src/lib.rs#L528) enum drives all VF-related guest interactions. Transitions are driven by methods on [`netvsp::Worker`](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/netvsp/src/lib.rs#L166) and [`netvsp::Coordinator`](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/netvsp/src/lib.rs#L3841) Here is the complete state machine:

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

### Citations

| State / Transition | Source |
|---|---|
| `PrimaryChannelGuestVfState` enum | [`PrimaryChannelGuestVfState` in lib.rs (netvsp) @ 528](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/netvsp/src/lib.rs#L528) |
| `guest_vf_is_available()` | [`NetChannel::guest_vf_is_available` in lib.rs (netvsp) @ 2623](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/netvsp/src/lib.rs#L2623) |
| `guest_ready_for_device()` | [`VirtualFunction::guest_ready_for_device` in lib.rs (netvsp) @ 336](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/netvsp/src/lib.rs#L336) |
| `switch_data_path()` | [`Worker::switch_data_path` in lib.rs (netvsp) @ 5360](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/netvsp/src/lib.rs#L5360) |
| `move_filter()` | [`Vport::move_filter` in mana.rs @ 519](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/mana_driver/src/mana.rs#L519) |
| `send_completion()` | [`Worker::handle_state_change` in lib.rs (netvsp) @ 2767](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/netvsp/src/lib.rs#L2767) |
| `guest_vf_data_path_switched_to_synthetic()` | [`NetChannel::guest_vf_data_path_switched_to_synthetic` in lib.rs (netvsp) @ 2735](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/netvsp/src/lib.rs#L2735) |

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
