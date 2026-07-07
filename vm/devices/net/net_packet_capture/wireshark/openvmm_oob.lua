-- Copyright (c) Microsoft Corporation.
-- Licensed under the MIT License.
--
-- Wireshark Lua dissector for the raw NetVSP/MANA out-of-band (OOB) /
-- per-packet-info (PPI) bytes that `net_packet_capture`
-- (vm/devices/net/net_packet_capture/src/lib.rs) attaches to captured
-- packets as a pcapng "custom binary option" (option code 0x0BAD/0x4BAD),
-- scoped to Microsoft's IANA Private Enterprise Number (311).
--
-- Install: copy this file into Wireshark's Personal Lua Plugins folder
-- (Help > About Wireshark > Folders > Personal Lua Plugins), or load it
-- via `-X lua_script:openvmm_oob.lua`.
--
-- Encoding written by net_packet_capture (see `raw_oob_option` in
-- src/lib.rs): the pcapng custom option's value is
--   [u8 schema_version][u8 OobSource tag][raw backend-native OOB bytes...]
-- where OobSource tag is:
--   1 = NetVSP RNDIS per-packet-info (rndisprot::PerPacketInfo entries)
--   2 = MANA ManaRxcompOob (gdma_defs::bnic::ManaRxcompOob)
--   3 = MANA ManaTxOob (gdma_defs::bnic::ManaTxOob) -- currently unused;
--       see plan notes on why MANA TX raw OOB isn't captured today.

local MICROSOFT_PEN = 311

local SOURCE_NETVSP_RNDIS_PPI = 1
local SOURCE_MANA_RXCOMP_OOB = 2
local SOURCE_MANA_TX_OOB = 3

local source_names = {
    [SOURCE_NETVSP_RNDIS_PPI] = "NetVSP RNDIS PPI",
    [SOURCE_MANA_RXCOMP_OOB] = "MANA ManaRxcompOob",
    [SOURCE_MANA_TX_OOB] = "MANA ManaTxOob",
}

local p_oob = Proto("openvmm_oob", "OpenVMM NetVSP/MANA raw OOB")

-- Top-level fields.
local f_schema_version =
    ProtoField.uint8("openvmm_oob.schema_version", "Schema Version", base.DEC)
local f_source =
    ProtoField.uint8("openvmm_oob.source", "OOB Source", base.DEC, source_names)
local f_raw_bytes = ProtoField.bytes("openvmm_oob.raw", "Raw OOB Bytes")

-- NetVSP RNDIS PPI entry fields (rndisprot::PerPacketInfo).
local ppi_type_names = {
    [0] = "PPI_TCP_IP_CHECKSUM",
    [2] = "PPI_LSO",
    [6] = "PPI_VLAN",
}
local f_ppi_size = ProtoField.uint32("openvmm_oob.netvsp_ppi.size", "Entry Size", base.DEC)
local f_ppi_type =
    ProtoField.uint32("openvmm_oob.netvsp_ppi.type", "PPI Type", base.HEX, ppi_type_names)
local f_ppi_info_offset = ProtoField.uint32(
    "openvmm_oob.netvsp_ppi.info_offset",
    "Per-Packet Info Offset",
    base.DEC
)
local f_ppi_payload = ProtoField.bytes("openvmm_oob.netvsp_ppi.payload", "PPI Payload")

-- MANA ManaRxcompOob fields (gdma_defs::bnic::ManaRxcompOob).
local f_mana_rx_cqe_hdr = ProtoField.uint32("openvmm_oob.mana_rx.cqe_hdr", "CQE Header", base.HEX)
local f_mana_rx_flags = ProtoField.uint32("openvmm_oob.mana_rx.flags", "Flags", base.HEX)
local f_mana_rx_vlan_id = ProtoField.uint16("openvmm_oob.mana_rx.vlan_id", "VLAN ID", base.DEC)
local f_mana_rx_vlantag_present =
    ProtoField.bool("openvmm_oob.mana_rx.vlantag_present", "VLAN Tag Present")
local f_mana_rx_hashtype = ProtoField.uint16("openvmm_oob.mana_rx.hashtype", "RX Hash Type", base.DEC)
local f_mana_rx_pkt_len0 =
    ProtoField.uint16("openvmm_oob.mana_rx.ppi0_pkt_len", "PPI[0] Packet Length", base.DEC)
local f_mana_rx_pkt_hash0 = ProtoField.uint32(
    "openvmm_oob.mana_rx.ppi0_pkt_hash",
    "PPI[0] Packet Hash (RSS)",
    base.HEX
)

p_oob.fields = {
    f_schema_version,
    f_source,
    f_raw_bytes,
    f_ppi_size,
    f_ppi_type,
    f_ppi_info_offset,
    f_ppi_payload,
    f_mana_rx_cqe_hdr,
    f_mana_rx_flags,
    f_mana_rx_vlan_id,
    f_mana_rx_vlantag_present,
    f_mana_rx_hashtype,
    f_mana_rx_pkt_len0,
    f_mana_rx_pkt_hash0,
}

-- Decodes one or more concatenated `rndisprot::PerPacketInfo` entries:
--   struct PerPacketInfo { u32 size; u32 typ; u32 per_packet_information_offset; }
-- followed by `size - 12` bytes of type-specific payload.
local function dissect_netvsp_ppi(buffer, tree)
    local offset = 0
    local len = buffer:len()
    while offset + 12 <= len do
        local entry_size = buffer(offset, 4):le_uint()
        if entry_size < 12 then
            break
        end
        local clamped_size = math.min(entry_size, len - offset)
        local entry = buffer(offset, clamped_size)
        local subtree = tree:add(p_oob, entry, "PPI Entry")
        subtree:add_le(f_ppi_size, entry(0, 4))
        subtree:add_le(f_ppi_type, entry(4, 4))
        subtree:add_le(f_ppi_info_offset, entry(8, 4))
        if clamped_size > 12 then
            subtree:add(f_ppi_payload, entry(12, clamped_size - 12))
        end
        offset = offset + entry_size
    end
end

-- Decodes the leading fields of `gdma_defs::bnic::ManaRxcompOob`:
--   cqe_hdr: u32, flags: u32 (bitfield), ppi[0]: { u16 pkt_len; u16
--   reserved1; u32 reserved2; u32 pkt_hash; }, ...
--
-- Bit extraction uses plain arithmetic (not Lua 5.3+ bitwise operators or
-- TvbRange:bitfield(), which assumes MSB-first numbering) to stay portable
-- across Wireshark's bundled Lua versions and match the LE bit-packed
-- layout produced by the `bitfield_struct` crate (LSB-first).
local function dissect_mana_rxcomp_oob(buffer, tree)
    if buffer:len() < 8 then
        return
    end
    tree:add_le(f_mana_rx_cqe_hdr, buffer(0, 4))
    local flags_range = buffer(4, 4)
    tree:add_le(f_mana_rx_flags, flags_range)
    local flags_val = flags_range:le_uint()
    -- rx_vlan_id: bits 0..11 (12 bits)
    local vlan_id = flags_val % 4096
    -- rx_vlantag_present: bit 12
    local vlantag_present = math.floor(flags_val / 4096) % 2
    -- rx_hashtype: bits 16..24 (9 bits)
    local hashtype = math.floor(flags_val / 65536) % 512
    tree:add(f_mana_rx_vlan_id, vlan_id)
    tree:add(f_mana_rx_vlantag_present, vlantag_present == 1)
    tree:add(f_mana_rx_hashtype, hashtype)

    if buffer:len() >= 20 then
        tree:add_le(f_mana_rx_pkt_len0, buffer(8, 2))
        tree:add_le(f_mana_rx_pkt_hash0, buffer(16, 4))
    end
end

function p_oob.dissector(buffer, pinfo, tree)
    if buffer:len() < 2 then
        return
    end

    local subtree = tree:add(p_oob, buffer(), "OpenVMM raw NetVSP/MANA OOB")
    subtree:add(f_schema_version, buffer(0, 1))
    local source = buffer(1, 1):uint()
    subtree:add(f_source, buffer(1, 1))

    if buffer:len() > 2 then
        local payload = buffer(2)
        if source == SOURCE_NETVSP_RNDIS_PPI then
            dissect_netvsp_ppi(payload, subtree)
        elseif source == SOURCE_MANA_RXCOMP_OOB then
            dissect_mana_rxcomp_oob(payload, subtree)
        else
            subtree:add(f_raw_bytes, payload)
        end
    end
end

-- Register against Wireshark's pcapng custom-option dissector table, keyed
-- by our Private Enterprise Number. Wireshark invokes this dissector with
-- the option's value bytes (the PEN itself already consumed) whenever it
-- encounters a custom binary option (0x0BAD/0x4BAD) scoped to this PEN on
-- any pcapng block -- including the EnhancedPacketBlock that
-- net_packet_capture attaches this option to.
--
-- Guarded with pcall: older Wireshark builds that don't expose this
-- dissector table will simply skip registration rather than erroring out
-- when this script is loaded.
local ok, custom_option_table = pcall(DissectorTable.get, "pcapng_custom_option")
if ok and custom_option_table ~= nil then
    custom_option_table:add(MICROSOFT_PEN, p_oob)
end
