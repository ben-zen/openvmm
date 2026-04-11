# NetVSP

NetVSP is the VMBus synthetic network adapter emulator. It presents
a virtual NIC to the guest over a VMBus channel and implements the
NVSP (Network Virtualization Service Protocol) for packet
transport between the guest and the host networking stack.

## Overview

NetVSP implements the Hyper-V synthetic networking protocol — a
VMBus-based transport that carries RNDIS-encapsulated Ethernet
frames between the guest's `netvsc` driver and the host. The
protocol is defined in
[`netvsp::protocol`](https://openvmm.dev/rustdoc/linux/netvsp/index.html).

In OpenHCL, NetVSP runs in VTL2 and mediates between the VTL0
guest and the physical MANA NIC hardware. When Accelerated
Networking is available, the data path can be switched from the
synthetic (VTL2-mediated) path to direct VF passthrough to the
guest.

## Key crates

| Crate | Role |
|---|---|
| [`netvsp`](https://openvmm.dev/rustdoc/linux/netvsp/index.html) | Synthetic NIC device — NVSP protocol, coordinator, workers |
| [`mana_driver`](https://openvmm.dev/rustdoc/linux/mana_driver/index.html) | MANA hardware NIC driver — GDMA, vports, filter management |
| [`net_mana`](https://openvmm.dev/rustdoc/linux/net_mana/index.html) | `ManaEndpoint` — bridges `netvsp` endpoints to MANA vports |
| `underhill_core::emuplat::netvsp` | OpenHCL VF manager — lifecycle, VTL0 bus control, save/restore |

## Architecture

The synthetic NIC is built from several cooperating components:

- **`Nic`** — the top-level VMBus device; handles channel
  open/close and spawns workers
- **`Coordinator`** — manages VF state, data path switching, and
  worker lifecycle across primary and sub-channels
- **`Worker`** — processes ring buffer I/O (TX and RX) on each
  VMBus channel
- **`NetChannel`** — handles NVSP protocol negotiation, RNDIS
  message processing, and VF advertisement to the guest

In OpenHCL deployments, `HclNetworkVFManager` orchestrates the
MANA VF lifecycle — device discovery, vport creation, VTL0 bus
offering, and teardown.

## Data path modes

NetVSP supports two data path modes:

1. **Synthetic** — all packets flow through VTL2 via the VMBus
   ring buffers. The `Coordinator` manages packet filter state
   and the `Worker` processes TX/RX on each channel.

2. **VF passthrough (Accelerated Networking)** — the MAC filter
   is moved from the VTL2 vport to the VTL0 vport via
   `Vport::move_filter()`, and packets flow directly between the
   guest and MANA hardware, bypassing VTL2.

Switching between modes is coordinated by the
`PrimaryChannelGuestVfState` state machine in `netvsp`.

## Further reading

- [NetVSP & MANA Lifecycle Diagrams](./netvsp_lifecycle.md) —
  sequence diagrams covering VTL2 startup, guest NIC addition, VF
  data path switching, failback, and the VF state machine
