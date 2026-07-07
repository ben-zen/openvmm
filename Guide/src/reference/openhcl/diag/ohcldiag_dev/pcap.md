# Network packet capture (PCAP)

PCAP is an industry standard format for capturing network packets. OpenHCL now
supports PCAP based packet capture for the network packets that are going through it.

## Prerequisites

* PCAP based packet capture support in OpenHCL came in around Nov 2023. The easiest
way to check whether the OpenHCL version you are running has PCAP support or not is by
running `ohcldiag-dev -h` and if the output shows an option for `packet-capture`, then
the support is there. Otherwise, pick a newer version of OpenHCL.
* OpenHCL PCAP support is only for the synthetic network path. It will likely not
show any packets captured if a vNIC is operating in accelerated networking mode. If you
would like to capture the network packets for a given vNIC in OpenHCL, disable
accelerated networking on the vNIC first.

## Packet capture options

To see the options for packet capture, run the help command using:

```cmd
ohcldiag-dev packet-capture -h
```

The help should be self explanatory, but further below are some sample commands
for reference purposes.

## How to stop running packet capture

There are two ways of controlling how long the packet capture runs.

1. Use the `-G` or `--seconds` to specify for how many seconds to run the packet capture
for in the command line. If not specified, it runs for the default value, which you can
see from the output of the help command above. This option can be handy for example, when
doing packet capture on the TiP node, where interacting with the console using keys like
`Ctrl+c` is not possible.
2. If you would like to keep the packet capture running indefinitely, specify a big value
for the `-G` option. You can then stop the capture at any time using the `Ctrl+c` key.

## Packet capture traces

The packet capture command will generate a pcap file for each vNIC. You can control the
name of the pcap file generated using the `-w` option. If not specified, the default
value for the file name is shown by the help command above. The index of the vNIC is
appended to the file name. So, for example, the pcap file for the first vNIC would be
`<default value>-0.pcap`, the second one `<default-value>-1.pcap`, so on and so forth.

## Loading the pcap file for analysis

There are many software that are available to load the pcap file. The most commonly
used one is `wireshark`. Copy the `*.pcap` files generated on the test machine and then
open them up on the desired software.

## Out-of-band (OOB) / per-packet-info (PPI) enrichment

In addition to the raw Ethernet frame, captured packets are enriched with the
out-of-band/per-packet-info (OOB/PPI) data that NetVSP and MANA pass alongside
the frame -- things like checksum offload state, LSO/USO segmentation
parameters, VLAN tag, and (for MANA) the RSS hash. This data would otherwise
be invisible in the pcap, since it never appears in the wire bytes.

The raw, backend-native bytes (e.g. NetVSP's RNDIS per-packet-info entries,
or MANA's `ManaRxcompOob`) are attached verbatim to each captured packet as a
standard pcapng "custom binary option" (option code `0x0BAD`), scoped to
Microsoft's IANA Private Enterprise Number (311). This is a lossless,
spec-conformant pcapng extension: tools that don't understand it (including
older Wireshark versions) simply ignore it, so existing pcap workflows are
unaffected.

Coverage today:

* **NetVSP TX**: the full, raw per-packet-info byte range is always captured
  when the guest sends PPI (checksum/LSO/VLAN info), independent of whether
  the individual PPI type is one NetVSP currently parses.
* **MANA RX**: raw `ManaRxcompOob` bytes (including the RSS hash, which has
  no equivalent in the normalized capture path) are captured when OOB
  capture is enabled on that queue.
* NetVSP RX and MANA TX are not separately captured this way, since their
  OOB data is fully derived from information already visible through the
  normal capture path -- nothing is lost by omitting them.

### Viewing OOB data in Wireshark

To decode and display the OOB fields in Wireshark, install the companion
Lua dissector at
[`vm/devices/net/net_packet_capture/wireshark/openvmm_oob.lua`](https://github.com/microsoft/openvmm/blob/main/vm/devices/net/net_packet_capture/wireshark/openvmm_oob.lua):

1. Copy the file into Wireshark's Personal Lua Plugins folder (find the
   exact path via Wireshark's **Help > About Wireshark > Folders**), or load
   it for a single session with `-X lua_script:openvmm_oob.lua`.
2. Reload the capture (or restart Wireshark). Packets carrying OOB data will
   show an "OpenVMM raw NetVSP/MANA OOB" section in the packet details pane.

Without the plugin loaded, the OOB option is still present in the file (and
visible as raw bytes in Wireshark's generic option display) -- the plugin is
only needed to decode it into structured, named fields.

## Example packet capture commands

In all of the below commands, `$vmname` should be replaced with the actual VM name. On
Azure, the VM name is the same as the container ID.

* Most basic command; run packet capture with all defaults. This will run packet capture
for the default values, including the default time (see the help command above for
default values).

```cmd
ohcldiag-dev.exe $vmname packet-capture
```

* Run packet capture indefinitely and use Ctrl+c to stop.

```cmd
ohcldiag-dev.exe $vmname packet-capture -G 655555
```

* Run packet capture with the location of the output location using the `-w` option.
By default, the traces are captured in the current working dir. That may not always be
desirable, especially on TiP. Let's say you want all the output pcap files
to go to the `c:\test` folder, then you can do something like:

```cmd
ohcldiag-dev.exe ubuntu packet-capture -w c:\test\nic
```

The output files will be of the form `c:\test\nic-*.pcap`

* Specify the length of the packet to capture using the `-s` or `--snaplen` option.
By default, the length of the packet captured can be big and can cause the size
of the pcap files to be quite large. It is advisable to only capture the packets
for the length that is of interest. For example, to specify only capturing 128 bytes
of the packet (which will generally give you the TCP and IP headers), do:

```cmd
ohcldiag-dev.exe ubuntu packet-capture -s 128
```

* Run the packet capture for the specified duration in seconds using the `-G` option.
For example, to capture packets for 2min, do:

```cmd
ohcldiag-dev.exe ubuntu packet-capture -G 120
```
