// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! `pcapng` compatible packet capture endpoint implementation.

#![expect(missing_docs)]
#![forbid(unsafe_code)]

use async_trait::async_trait;
use futures::FutureExt;
use futures::StreamExt;
use futures::lock::Mutex;
use futures_concurrency::future::Race;
use inspect::InspectMut;
use mesh::error::RemoteError;
use mesh::rpc::FailableRpc;
use mesh::rpc::RpcSend;
use net_backend::BufferAccess;
use net_backend::Endpoint;
use net_backend::EndpointAction;
use net_backend::MultiQueueSupport;
use net_backend::OobSource;
use net_backend::Queue;
use net_backend::QueueConfig;
use net_backend::RawOob;
use net_backend::RssConfig;
use net_backend::RxBufferSegment;
use net_backend::RxId;
use net_backend::RxMetadata;
use net_backend::TxError;
use net_backend::TxId;
use net_backend::TxOffloadSupport;
use net_backend::TxSegment;
use net_backend::next_packet;
use pcap_file::DataLink;
use pcap_file::PcapError;
use pcap_file::PcapResult;
use pcap_file::pcapng::PcapNgWriter;
use pcap_file::pcapng::blocks::enhanced_packet::EnhancedPacketBlock;
use pcap_file::pcapng::blocks::enhanced_packet::EnhancedPacketOption;
use pcap_file::pcapng::blocks::interface_description::InterfaceDescriptionBlock;
use pcap_file::pcapng::blocks::opt_common::CommonOption;
use pcap_file::pcapng::blocks::opt_common::CustomBinaryOption;
use std::borrow::Cow;
use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::task::Context;
use std::task::Poll;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

/// IANA Private Enterprise Number for Microsoft, used to scope the custom
/// pcapng option carrying raw NetVSP/MANA OOB bytes.
const MICROSOFT_PEN: u32 = 311;

/// Version of the `[schema_version][OobSource tag]` header prepended to the
/// custom pcapng option's value. Bump this if the encoding changes.
const OOB_OPTION_SCHEMA_VERSION: u8 = 1;

/// Maps an [`OobSource`] to the single-byte tag stored in the custom option
/// payload, so a companion Wireshark dissector can dispatch on it.
fn oob_source_tag(source: OobSource) -> u8 {
    match source {
        OobSource::NetvspRndisPpi => 1,
        OobSource::ManaRxcompOob => 2,
        OobSource::ManaTxOob => 3,
    }
}

/// Builds the pcapng `EnhancedPacketOption` carrying `raw_oob`, encoded as
/// `[schema_version][OobSource tag][raw backend bytes...]` in a standard
/// custom binary option (code `0x0BAD`) scoped to [`MICROSOFT_PEN`].
fn raw_oob_option(raw_oob: &RawOob) -> EnhancedPacketOption<'static> {
    let mut value = Vec::with_capacity(2 + raw_oob.data.len());
    value.push(OOB_OPTION_SCHEMA_VERSION);
    value.push(oob_source_tag(raw_oob.source));
    value.extend_from_slice(&raw_oob.data);

    EnhancedPacketOption::Common(CommonOption::CustomBinaryCopiable(CustomBinaryOption {
        pen: MICROSOFT_PEN,
        value: Cow::Owned(value),
    }))
}

/// Defines packet capture operations.
#[derive(Debug, PartialEq, mesh::MeshPayload)]
pub enum PacketCaptureOperation {
    /// Query details.
    Query,
    /// Start packet capture.
    Start,
    /// Stop packet capture.
    Stop,
}

/// Defines start operation data.
#[derive(Debug, mesh::MeshPayload)]
pub struct StartData<W: Write> {
    pub snaplen: u32,
    pub writers: Vec<W>,
}

/// Defines operational data.
#[derive(Debug, mesh::MeshPayload)]
pub enum OperationData<W: Write> {
    OpQueryData(u32),
    OpStartData(StartData<W>),
}

/// Additional parameters provided as part of a network packet capture trace.
#[derive(Debug, mesh::MeshPayload)]
pub struct PacketCaptureParams<W: Write> {
    /// Indicates the network capture operation.
    pub operation: PacketCaptureOperation,
    /// Operational data that is specific to the given operation.
    pub op_data: Option<OperationData<W>>,
}

trait PcapWriter: Send + Sync {
    /// Writes a EnhancedPacketBlocke
    fn write_pcapng_block_eb(&mut self, block: EnhancedPacketBlock<'_>) -> PcapResult<usize>;

    /// Writes a InterfaceDescriptionBlock
    fn write_pcapng_block_id(&mut self, block: InterfaceDescriptionBlock<'_>) -> PcapResult<usize>;
}

struct LocalPcapWriter<W: Write> {
    inner: PcapNgWriter<W>,
}

impl<W: Write + Send + Sync> PcapWriter for LocalPcapWriter<W> {
    fn write_pcapng_block_eb(&mut self, block: EnhancedPacketBlock<'_>) -> PcapResult<usize> {
        self.inner.write_pcapng_block(block)
    }

    fn write_pcapng_block_id(&mut self, block: InterfaceDescriptionBlock<'_>) -> PcapResult<usize> {
        self.inner.write_pcapng_block(block)
    }
}

struct PacketCaptureOptions {
    operation: PacketCaptureOperation,
    snaplen: usize,
    writer: Option<Box<dyn PcapWriter>>,
}

impl PacketCaptureOptions {
    fn new_with_start<W: Write + Send + Sync + 'static>(snaplen: u32, writer: W) -> Self {
        //TODO: Native endianness?
        let pcap_ng_writer =
            PcapNgWriter::with_endianness(writer, pcap_file::Endianness::Big).unwrap();

        let local_writer = LocalPcapWriter {
            inner: pcap_ng_writer,
        };

        Self {
            operation: PacketCaptureOperation::Start,
            snaplen: snaplen as usize,
            writer: Some(Box::new(local_writer)),
        }
    }

    fn new_with_stop() -> Self {
        Self {
            operation: PacketCaptureOperation::Stop,
            snaplen: 0,
            writer: None,
        }
    }
}

enum PacketCaptureEndpointCommand {
    PacketCapture(FailableRpc<PacketCaptureOptions, ()>),
}

pub struct PacketCaptureEndpointControl {
    control_tx: mesh::Sender<PacketCaptureEndpointCommand>,
}

impl PacketCaptureEndpointControl {
    pub async fn packet_capture<W: Write + Send + Sync + 'static>(
        &self,
        params: PacketCaptureParams<W>,
    ) -> anyhow::Result<PacketCaptureParams<W>> {
        let mut params = params;
        let options = match params.operation {
            PacketCaptureOperation::Query | PacketCaptureOperation::Start => {
                let Some(op_data) = &mut params.op_data else {
                    anyhow::bail!(
                        "Invalid input parameter. Expecting operational data, but none provided"
                    );
                };

                match op_data {
                    OperationData::OpQueryData(num_streams) => {
                        return Ok(PacketCaptureParams {
                            operation: params.operation,
                            op_data: Some(OperationData::OpQueryData(*num_streams + 1)),
                        });
                    }
                    OperationData::OpStartData(data) => {
                        if data.writers.is_empty() {
                            anyhow::bail!("Insufficient streams");
                        }
                        let socket = data.writers.remove(0);
                        PacketCaptureOptions::new_with_start(data.snaplen, socket)
                    }
                }
            }
            PacketCaptureOperation::Stop => PacketCaptureOptions::new_with_stop(),
        };

        self.control_tx
            .call_failable(PacketCaptureEndpointCommand::PacketCapture, options)
            .await?;

        Ok(params)
    }
}

pub struct PacketCaptureEndpoint {
    /// Some identifier that this endpoint can identify itself using for things
    /// like tracing, filtering etc..
    id: String,
    endpoint: Box<dyn Endpoint>,
    control_rx: Arc<Mutex<mesh::Receiver<PacketCaptureEndpointCommand>>>,
    pcap: Arc<Pcap>,
}

impl InspectMut for PacketCaptureEndpoint {
    fn inspect_mut(&mut self, req: inspect::Request<'_>) {
        self.current_mut().inspect_mut(req)
    }
}

impl PacketCaptureEndpoint {
    pub fn new(endpoint: Box<dyn Endpoint>, id: String) -> (Self, PacketCaptureEndpointControl) {
        let (control_tx, control_rx) = mesh::channel();
        let control = PacketCaptureEndpointControl {
            control_tx: control_tx.clone(),
        };
        let pcap = Arc::new(Pcap::new(control_tx.clone()));
        (
            Self {
                id,
                endpoint,
                control_rx: Arc::new(Mutex::new(control_rx)),
                pcap,
            },
            control,
        )
    }

    fn current(&self) -> &dyn Endpoint {
        self.endpoint.as_ref()
    }

    fn current_mut(&mut self) -> &mut dyn Endpoint {
        self.endpoint.as_mut()
    }
}

#[async_trait]
impl Endpoint for PacketCaptureEndpoint {
    fn endpoint_type(&self) -> &'static str {
        self.current().endpoint_type()
    }

    async fn get_queues(
        &mut self,
        config: Vec<QueueConfig>,
        rss: Option<&RssConfig<'_>>,
        queues: &mut Vec<Box<dyn Queue>>,
    ) -> anyhow::Result<()> {
        if self.pcap.enabled.load(Ordering::Relaxed) {
            tracing::trace!("using packet capture queues");
            let mut queues_inner: Vec<Box<dyn Queue>> = Vec::new();
            self.current_mut()
                .get_queues(config, rss, &mut queues_inner)
                .await?;
            while let Some(inner) = queues_inner.pop() {
                queues.push(Box::new(PacketCaptureQueue {
                    queue: inner,
                    pcap: self.pcap.clone(),
                    scratch_segments: Vec::new(),
                    rx_oob: HashMap::new(),
                }));
            }
        } else {
            tracing::trace!("using inner queues");
            self.current_mut().get_queues(config, rss, queues).await?;
        }
        Ok(())
    }

    async fn stop(&mut self) {
        self.current_mut().stop().await
    }

    fn is_ordered(&self) -> bool {
        self.current().is_ordered()
    }

    fn tx_offload_support(&self) -> TxOffloadSupport {
        self.current().tx_offload_support()
    }

    fn multiqueue_support(&self) -> MultiQueueSupport {
        self.current().multiqueue_support()
    }

    fn tx_fast_completions(&self) -> bool {
        self.current().tx_fast_completions()
    }

    async fn set_data_path_to_guest_vf(&self, use_vf: bool) -> anyhow::Result<()> {
        self.current().set_data_path_to_guest_vf(use_vf).await
    }

    async fn get_data_path_to_guest_vf(&self) -> anyhow::Result<bool> {
        self.current().get_data_path_to_guest_vf().await
    }

    async fn wait_for_endpoint_action(&mut self) -> EndpointAction {
        enum Message {
            PacketCaptureEndpointCommand(PacketCaptureEndpointCommand),
            UpdateFromEndpoint(EndpointAction),
        }
        loop {
            let receiver = self.control_rx.clone();
            let mut receive_update = receiver.lock().await;
            let update = async {
                match receive_update.next().await {
                    Some(m) => Message::PacketCaptureEndpointCommand(m),
                    None => {
                        std::future::pending::<()>().await;
                        unreachable!()
                    }
                }
            };
            let ep_update = self
                .current_mut()
                .wait_for_endpoint_action()
                .map(Message::UpdateFromEndpoint);
            let m = (update, ep_update).race().await;
            match m {
                Message::PacketCaptureEndpointCommand(
                    PacketCaptureEndpointCommand::PacketCapture(rpc),
                ) => {
                    let (options, response) = rpc.split();
                    let result = async {
                        let id = &self.id;
                        let start = match options.operation {
                            PacketCaptureOperation::Start => {
                                tracing::info!(id, "starting trace");
                                true
                            }
                            PacketCaptureOperation::Stop => {
                                tracing::info!(id, "stopping trace");
                                false
                            }
                            _ => Err(anyhow::anyhow!("Unexpected packet capture option {id}"))?,
                        };

                        // Keep the lock until all values are being set to make the update atomic.
                        let mut pcap_writer = self.pcap.pcap_writer.lock();
                        let restart_required = start != self.pcap.enabled.load(Ordering::Relaxed);
                        self.pcap.snaplen.store(options.snaplen, Ordering::Relaxed);
                        self.pcap
                            .interface_descriptor_written
                            .store(false, Ordering::Relaxed);
                        self.pcap.enabled.store(start, Ordering::Relaxed);
                        *pcap_writer = options.writer;
                        anyhow::Ok(restart_required)
                    }
                    .await;
                    let (result, restart_required) = match result {
                        Err(e) => (Err(e), false),
                        Ok(value) => (Ok(()), value),
                    };
                    response.complete(result.map_err(RemoteError::new));
                    if restart_required {
                        break EndpointAction::RestartRequired;
                    }
                }
                Message::UpdateFromEndpoint(update) => break update,
            }
        }
    }

    fn link_speed(&self) -> u64 {
        self.current().link_speed()
    }
}

struct Pcap {
    // N.B Lock/update semantics: Keep the `pcap_writer` lock while updating
    //  the other fields.
    pcap_writer: parking_lot::Mutex<Option<Box<dyn PcapWriter>>>,
    interface_descriptor_written: AtomicBool,
    enabled: AtomicBool,
    snaplen: AtomicUsize,
    endpoint_control: mesh::Sender<PacketCaptureEndpointCommand>,
}

impl Pcap {
    fn new(endpoint_control: mesh::Sender<PacketCaptureEndpointCommand>) -> Self {
        Self {
            enabled: AtomicBool::new(false),
            snaplen: AtomicUsize::new(65535),
            pcap_writer: parking_lot::Mutex::new(None),
            interface_descriptor_written: AtomicBool::new(false),
            endpoint_control,
        }
    }

    fn write_packet(
        &self,
        buf: &[u8],
        original_len: u32,
        snaplen: u32,
        timestamp: &Duration,
        raw_oob: Option<&RawOob>,
    ) -> bool {
        let mut locked_writer = self.pcap_writer.lock();
        let Some(pcap_writer) = &mut *locked_writer else {
            return false;
        };

        let handle_write_result = |r: PcapResult<usize>| match r {
            // Writer gone unexpectedly; disable packet capture.
            Err(PcapError::IoError(_)) => {
                // No particular benefit of using compare_exchange atomic here
                // as the pcap writer lock is held.
                if self.enabled.load(Ordering::Relaxed) {
                    self.enabled.store(false, Ordering::Relaxed);
                    let stop = PacketCaptureOptions::new_with_stop();
                    // Best effort.
                    drop(
                        self.endpoint_control
                            .call(PacketCaptureEndpointCommand::PacketCapture, stop),
                    );
                }
                Err(())
            }
            _ => Ok(()),
        };

        if !self.interface_descriptor_written.load(Ordering::Relaxed) {
            let interface = InterfaceDescriptionBlock {
                linktype: DataLink::ETHERNET,
                snaplen,
                options: vec![],
            };
            if handle_write_result(pcap_writer.write_pcapng_block_id(interface)).is_err() {
                *locked_writer = None;
                return false;
            }
            self.interface_descriptor_written
                .store(true, Ordering::Relaxed);
        }

        let options = match raw_oob {
            Some(raw_oob) => vec![raw_oob_option(raw_oob)],
            None => vec![],
        };

        let packet = EnhancedPacketBlock {
            interface_id: 0,
            timestamp: *timestamp,
            original_len,
            data: Cow::Borrowed(buf),
            options,
        };

        if handle_write_result(pcap_writer.write_pcapng_block_eb(packet)).is_err() {
            *locked_writer = None;
            return false;
        }

        true
    }
}

/// A [`BufferAccess`] proxy that intercepts `write_data`/`write_header`/
/// `write_packet` calls made by the wrapped backend queue, stashing each
/// packet's [`RxMetadata::raw_oob`] (keyed by [`RxId`]) so it can enrich the
/// pcap after `rx_poll` returns, then forwards unchanged to the real `pool`.
///
/// This closes a gap in the wrapping design: `PacketCaptureQueue::rx_poll`
/// otherwise forwards `pool` straight into the wrapped backend, so any
/// `RxMetadata` the backend hands to `pool.write_header`/`write_packet` is
/// never observed by packet capture.
struct CapturingBufferAccess<'a> {
    inner: &'a mut dyn BufferAccess,
    captured: &'a mut HashMap<u32, RawOob>,
}

impl BufferAccess for CapturingBufferAccess<'_> {
    fn guest_memory(&self) -> &guestmem::GuestMemory {
        self.inner.guest_memory()
    }

    fn write_data(&mut self, id: RxId, data: &[u8]) {
        self.inner.write_data(id, data)
    }

    fn push_guest_addresses(&self, id: RxId, buf: &mut Vec<RxBufferSegment>) {
        self.inner.push_guest_addresses(id, buf)
    }

    fn capacity(&self, id: RxId) -> u32 {
        self.inner.capacity(id)
    }

    fn write_header(&mut self, id: RxId, metadata: &RxMetadata) {
        if let Some(raw_oob) = &metadata.raw_oob {
            self.captured.insert(id.0, raw_oob.clone());
        }
        self.inner.write_header(id, metadata)
    }

    fn write_packet(&mut self, id: RxId, metadata: &RxMetadata, data: &[u8]) {
        if let Some(raw_oob) = &metadata.raw_oob {
            self.captured.insert(id.0, raw_oob.clone());
        }
        self.inner.write_packet(id, metadata, data)
    }
}

struct PacketCaptureQueue {
    queue: Box<dyn Queue>,
    pcap: Arc<Pcap>,
    scratch_segments: Vec<RxBufferSegment>,
    /// Raw OOB bytes captured via [`CapturingBufferAccess`] during the most
    /// recent `rx_poll`, keyed by [`RxId::0`](RxId). Entries are consumed
    /// (removed) as each packet is written to the pcap.
    rx_oob: HashMap<u32, RawOob>,
}

impl PacketCaptureQueue {
    fn current_mut(&mut self) -> &mut dyn Queue {
        self.queue.as_mut()
    }
}

#[async_trait]
impl Queue for PacketCaptureQueue {
    async fn update_target_vp(&mut self, target_vp: u32) {
        self.current_mut().update_target_vp(target_vp).await
    }

    fn poll_ready(&mut self, cx: &mut Context<'_>, pool: &mut dyn BufferAccess) -> Poll<()> {
        self.current_mut().poll_ready(cx, pool)
    }

    fn rx_avail(&mut self, pool: &mut dyn BufferAccess, done: &[RxId]) {
        self.current_mut().rx_avail(pool, done)
    }

    fn rx_poll(
        &mut self,
        pool: &mut dyn BufferAccess,
        packets: &mut [RxId],
    ) -> anyhow::Result<usize> {
        let enabled = self.pcap.enabled.load(Ordering::Relaxed);
        // Tell the wrapped backend whether to preserve raw OOB bytes. This
        // is a no-op for backends that don't support it.
        self.queue.set_oob_capture(enabled);
        let n = if enabled {
            // Wrap `pool` so that any `RxMetadata` the backend writes is
            // observed here, not just by the frontend.
            let mut proxy = CapturingBufferAccess {
                inner: &mut *pool,
                captured: &mut self.rx_oob,
            };
            self.queue.rx_poll(&mut proxy, packets)?
        } else {
            self.queue.rx_poll(pool, packets)?
        };
        if enabled {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::new(0, 0));
            let snaplen = self.pcap.snaplen.load(Ordering::Relaxed);
            for id in &packets[..n] {
                let mut buf = vec![0; snaplen];
                let mut len = 0;
                let mut pkt_len = 0;
                self.scratch_segments.clear();
                pool.push_guest_addresses(*id, &mut self.scratch_segments);
                for segment in &self.scratch_segments {
                    pkt_len += segment.len;
                    if len == buf.len() {
                        continue;
                    }

                    let copy_length = std::cmp::min(buf.len() - len, segment.len as usize);
                    let _ = pool.guest_memory().read_at(segment.gpa, &mut buf[len..]);
                    len += copy_length;
                }

                if len == 0 {
                    continue;
                }

                let raw_oob = self.rx_oob.remove(&id.0);
                if !self.pcap.write_packet(
                    &buf[..len],
                    pkt_len,
                    snaplen as u32,
                    &timestamp,
                    raw_oob.as_ref(),
                ) {
                    break;
                }
            }
            // Drop any stashed OOB bytes for buffers that weren't reported
            // as done this poll (e.g. dropped/truncated), so the map
            // doesn't grow unbounded.
            if !self.rx_oob.is_empty() {
                self.rx_oob.clear();
            }
        }
        Ok(n)
    }

    fn tx_avail(
        &mut self,
        pool: &mut dyn BufferAccess,
        segments: &[TxSegment],
    ) -> anyhow::Result<(bool, usize)> {
        if self.pcap.enabled.load(Ordering::Relaxed) {
            let mut segments = segments;
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::new(0, 0));
            let snaplen = self.pcap.snaplen.load(Ordering::Relaxed);
            while !segments.is_empty() {
                let (metadata, this, rest) = next_packet(segments);
                segments = rest;
                if metadata.len == 0 {
                    continue;
                }
                let mut buf = vec![0; snaplen];
                let mut len = 0;
                for segment in this {
                    if len == buf.len() {
                        break;
                    }

                    let copy_length = std::cmp::min(buf.len() - len, segment.len as usize);
                    let _ = pool.guest_memory().read_at(segment.gpa, &mut buf[len..]);
                    len += copy_length;
                }

                if len == 0 {
                    continue;
                }

                if !self.pcap.write_packet(
                    &buf[..len],
                    metadata.len,
                    snaplen as u32,
                    &timestamp,
                    metadata.raw_oob.as_ref(),
                ) {
                    break;
                }
            }
        }
        self.current_mut().tx_avail(pool, segments)
    }

    fn tx_poll(
        &mut self,
        pool: &mut dyn BufferAccess,
        done: &mut [TxId],
    ) -> Result<usize, TxError> {
        self.current_mut().tx_poll(pool, done)
    }
}

impl InspectMut for PacketCaptureQueue {
    fn inspect_mut(&mut self, req: inspect::Request<'_>) {
        self.current_mut().inspect_mut(req)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pcap_file::pcapng::Block;
    use pcap_file::pcapng::PcapNgReader;
    use pcap_file::pcapng::blocks::opt_common::CommonOption;
    use std::io::Result as IoResult;

    #[test]
    fn oob_source_tag_mapping_is_stable() {
        // The Wireshark companion dissector (wireshark/openvmm_oob.lua)
        // hardcodes these values; changing them requires updating both
        // sides in lockstep.
        assert_eq!(oob_source_tag(OobSource::NetvspRndisPpi), 1);
        assert_eq!(oob_source_tag(OobSource::ManaRxcompOob), 2);
        assert_eq!(oob_source_tag(OobSource::ManaTxOob), 3);
    }

    #[test]
    fn raw_oob_option_encodes_pen_and_header() {
        let raw_oob = RawOob {
            source: OobSource::ManaRxcompOob,
            data: vec![0xAA, 0xBB, 0xCC],
        };
        let option = raw_oob_option(&raw_oob);
        let EnhancedPacketOption::Common(CommonOption::CustomBinaryCopiable(custom)) = option
        else {
            panic!("expected a custom binary option");
        };
        assert_eq!(custom.pen, MICROSOFT_PEN);
        assert_eq!(
            custom.value.as_ref(),
            &[OOB_OPTION_SCHEMA_VERSION, 2, 0xAA, 0xBB, 0xCC]
        );
    }

    /// A `Write` sink that appends to a shared buffer, so the bytes written
    /// by a `PcapWriter` can be inspected/re-read after the fact.
    #[derive(Clone)]
    struct SharedBuf(Arc<parking_lot::Mutex<Vec<u8>>>);

    impl Write for SharedBuf {
        fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
            self.0.lock().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> IoResult<()> {
            Ok(())
        }
    }

    /// Writes one packet with the given `raw_oob` through `Pcap::write_packet`,
    /// then re-parses the resulting pcapng bytes with `PcapNgReader` and
    /// returns the decoded custom option's `(pen, value)`, if present. This
    /// confirms the encoding round-trips through an independent, spec-
    /// conformant pcapng parser -- not just through our own writer code.
    fn round_trip_raw_oob(raw_oob: &RawOob) -> Option<(u32, Vec<u8>)> {
        let (endpoint_control, _rx) = mesh::channel();
        let pcap = Pcap::new(endpoint_control);

        let shared = SharedBuf(Arc::new(parking_lot::Mutex::new(Vec::new())));
        let options = PacketCaptureOptions::new_with_start(1500, shared.clone());
        *pcap.pcap_writer.lock() = options.writer;
        pcap.enabled.store(true, Ordering::Relaxed);

        let wrote = pcap.write_packet(&[0u8; 14], 14, 1500, &Duration::from_secs(1), Some(raw_oob));
        assert!(wrote, "write_packet should succeed");

        let bytes = shared.0.lock().clone();
        let mut reader = PcapNgReader::new(std::io::Cursor::new(bytes)).unwrap();
        while let Some(block) = reader.next_block() {
            let block = block.unwrap().into_owned();
            if let Block::EnhancedPacket(epb) = block {
                for opt in &epb.options {
                    if let EnhancedPacketOption::Common(CommonOption::CustomBinaryCopiable(
                        custom,
                    )) = opt
                    {
                        return Some((custom.pen, custom.value.clone().into_owned()));
                    }
                }
            }
        }
        None
    }

    #[test]
    fn raw_oob_round_trips_through_pcapng_reader() {
        let raw_oob = RawOob {
            source: OobSource::NetvspRndisPpi,
            data: vec![1, 2, 3, 4, 5, 6, 7, 8],
        };
        let (pen, value) = round_trip_raw_oob(&raw_oob).expect("custom option should be present");
        assert_eq!(pen, MICROSOFT_PEN);
        assert_eq!(
            value,
            vec![OOB_OPTION_SCHEMA_VERSION, 1, 1, 2, 3, 4, 5, 6, 7, 8]
        );
    }

    #[test]
    fn no_custom_option_when_raw_oob_absent() {
        let (endpoint_control, _rx) = mesh::channel();
        let pcap = Pcap::new(endpoint_control);

        let shared = SharedBuf(Arc::new(parking_lot::Mutex::new(Vec::new())));
        let options = PacketCaptureOptions::new_with_start(1500, shared.clone());
        *pcap.pcap_writer.lock() = options.writer;
        pcap.enabled.store(true, Ordering::Relaxed);

        assert!(pcap.write_packet(&[0u8; 14], 14, 1500, &Duration::from_secs(1), None));

        let bytes = shared.0.lock().clone();
        let mut reader = PcapNgReader::new(std::io::Cursor::new(bytes)).unwrap();
        let mut saw_epb = false;
        while let Some(block) = reader.next_block() {
            let block = block.unwrap().into_owned();
            if let Block::EnhancedPacket(epb) = block {
                saw_epb = true;
                assert!(epb.options.is_empty());
            }
        }
        assert!(saw_epb, "expected an EnhancedPacketBlock to be written");
    }
}
