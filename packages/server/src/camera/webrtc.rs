use super::publish_camera_packet;
use crate::error::AppError;
use bytes::{BufMut, Bytes, BytesMut};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::time;
use tracing::{info, warn};
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::{MediaEngine, MIME_TYPE_H264};
use webrtc::api::APIBuilder;
use webrtc::data_channel::RTCDataChannel;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
use webrtc::rtp::packet::Packet;
use webrtc::rtp_transceiver::rtp_codec::{
    RTCRtpCodecCapability, RTCRtpCodecParameters, RTPCodecType,
};
use webrtc::track::track_remote::TrackRemote;

const CAMERA_DATA_CHANNEL_LABEL: &str = "simdeck-camera";
const H264_FMTP_LINE: &str =
    "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f";
const FAST_ICE_GATHER_TIMEOUT: Duration = Duration::from_millis(250);
const FULL_ICE_GATHER_TIMEOUT: Duration = Duration::from_secs(3);
const REORDER_WINDOW_PACKETS: usize = 8;
const PLI_INTERVAL: Duration = Duration::from_millis(250);
const MAX_FRAME_BYTES: usize = 2 * 1024 * 1024 - 6;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraWebRtcOffer {
    pub client_id: Option<String>,
    pub sdp: String,
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraWebRtcAnswer {
    pub sdp: String,
    #[serde(rename = "type")]
    pub kind: String,
}

struct CameraWebRtcSession {
    peer_connection: Arc<webrtc::peer_connection::RTCPeerConnection>,
    metrics: Arc<CameraWebRtcMetrics>,
}

#[derive(Default)]
struct CameraWebRtcMetrics {
    connected: AtomicBool,
    rtp_packets: AtomicU64,
    rtp_bytes: AtomicU64,
    lost_packets: AtomicU64,
    reordered_packets: AtomicU64,
    assembled_frames: AtomicU64,
    published_frames: AtomicU64,
    dropped_frames: AtomicU64,
    dependency_drops: AtomicU64,
    native_errors: AtomicU64,
    pli_count: AtomicU64,
    queue_high_water: AtomicU64,
}

static CAMERA_WEBRTC_SESSIONS: OnceLock<Mutex<HashMap<String, Arc<CameraWebRtcSession>>>> =
    OnceLock::new();

pub async fn create_answer(
    udid: String,
    payload: CameraWebRtcOffer,
) -> Result<CameraWebRtcAnswer, AppError> {
    if payload.kind != "offer" {
        return Err(AppError::bad_request(
            "Camera WebRTC payload must include type `offer`.",
        ));
    }
    if !payload.sdp.to_ascii_lowercase().contains("h264/90000") {
        return Err(AppError::bad_request(
            "This browser did not offer H.264 camera video.",
        ));
    }

    let mut media_engine = MediaEngine::default();
    media_engine
        .register_codec(
            RTCRtpCodecParameters {
                capability: RTCRtpCodecCapability {
                    mime_type: MIME_TYPE_H264.to_owned(),
                    clock_rate: 90_000,
                    channels: 0,
                    sdp_fmtp_line: H264_FMTP_LINE.to_owned(),
                    rtcp_feedback: crate::transport::webrtc::h264_rtcp_feedback(),
                },
                payload_type: 96,
                ..Default::default()
            },
            RTPCodecType::Video,
        )
        .map_err(|err| AppError::internal(format!("register camera H.264 codec: {err}")))?;
    let mut interceptor_registry = Registry::new();
    interceptor_registry = register_default_interceptors(interceptor_registry, &mut media_engine)
        .map_err(|err| {
        AppError::internal(format!("register camera WebRTC interceptors: {err}"))
    })?;
    let api = APIBuilder::new()
        .with_media_engine(media_engine)
        .with_interceptor_registry(interceptor_registry)
        .build();
    let peer_connection = Arc::new(
        api.new_peer_connection(RTCConfiguration {
            ice_servers: crate::transport::webrtc::ice_servers(),
            ice_transport_policy: crate::transport::webrtc::ice_transport_policy(),
            ..Default::default()
        })
        .await
        .map_err(|err| AppError::internal(format!("create camera WebRTC peer: {err}")))?,
    );
    let metrics = Arc::new(CameraWebRtcMetrics::default());

    let data_udid = udid.clone();
    peer_connection.on_data_channel(Box::new(move |channel: Arc<RTCDataChannel>| {
        let data_udid = data_udid.clone();
        Box::pin(async move {
            if channel.label() != CAMERA_DATA_CHANNEL_LABEL {
                return;
            }
            let ready_channel = channel.clone();
            channel.on_open(Box::new(move || {
                let ready_channel = ready_channel.clone();
                let data_udid = data_udid.clone();
                Box::pin(async move {
                    if let Err(err) = ready_channel
                        .send_text(json!({ "ready": true, "udid": data_udid }).to_string())
                        .await
                    {
                        warn!("Unable to send camera WebRTC readiness: {err}");
                    }
                })
            }));
        })
    }));

    let track_peer = peer_connection.clone();
    let track_metrics = metrics.clone();
    let track_udid = udid.clone();
    peer_connection.on_track(Box::new(move |track, _, _| {
        let track_peer = track_peer.clone();
        let track_metrics = track_metrics.clone();
        let track_udid = track_udid.clone();
        tokio::spawn(async move {
            receive_h264_track(track_udid, track_peer, track, track_metrics).await;
        });
        Box::pin(async {})
    }));

    let state_metrics = metrics.clone();
    let state_udid = udid.clone();
    peer_connection.on_peer_connection_state_change(Box::new(move |state| {
        let state_metrics = state_metrics.clone();
        let state_udid = state_udid.clone();
        Box::pin(async move {
            let connected = state == RTCPeerConnectionState::Connected;
            state_metrics.connected.store(connected, Ordering::Relaxed);
            info!("Camera WebRTC peer state for {state_udid}: {state}");
        })
    }));

    let fast_gather = payload.sdp.lines().any(|line| {
        line.starts_with("a=candidate:") && line.split_whitespace().any(|part| part == "host")
    });
    let offer = RTCSessionDescription::offer(payload.sdp)
        .map_err(|err| AppError::bad_request(format!("invalid camera WebRTC offer: {err}")))?;
    peer_connection
        .set_remote_description(offer)
        .await
        .map_err(|err| AppError::bad_request(format!("set camera WebRTC offer: {err}")))?;
    let answer = peer_connection
        .create_answer(None)
        .await
        .map_err(|err| AppError::internal(format!("create camera WebRTC answer: {err}")))?;
    let mut gather_complete = peer_connection.gathering_complete_promise().await;
    peer_connection
        .set_local_description(answer)
        .await
        .map_err(|err| AppError::internal(format!("set camera WebRTC answer: {err}")))?;
    let gather_timeout = if fast_gather {
        FAST_ICE_GATHER_TIMEOUT
    } else {
        FULL_ICE_GATHER_TIMEOUT
    };
    let gather_result = time::timeout(gather_timeout, gather_complete.recv()).await;
    let mut local_description = peer_connection
        .local_description()
        .await
        .ok_or_else(|| AppError::internal("Camera WebRTC answer was not set."))?;
    if gather_result.is_err()
        && !local_description
            .sdp
            .lines()
            .any(|line| line.starts_with("a=candidate:"))
    {
        let _ = time::timeout(FULL_ICE_GATHER_TIMEOUT, gather_complete.recv()).await;
        local_description = peer_connection
            .local_description()
            .await
            .ok_or_else(|| AppError::internal("Camera WebRTC answer was not set."))?;
    }

    let session = Arc::new(CameraWebRtcSession {
        peer_connection,
        metrics,
    });
    let previous = sessions().lock().unwrap().insert(udid, session);
    if let Some(previous) = previous {
        let _ = previous.peer_connection.close().await;
    }
    let _ = payload.client_id;

    Ok(CameraWebRtcAnswer {
        sdp: local_description.sdp,
        kind: "answer".to_owned(),
    })
}

pub async fn stop(udid: &str) {
    let session = sessions().lock().unwrap().remove(udid);
    if let Some(session) = session {
        let _ = session.peer_connection.close().await;
    }
}

pub fn enrich_status(udid: &str, object: &mut Map<String, Value>) {
    let metrics = sessions()
        .lock()
        .unwrap()
        .get(udid)
        .map(|session| session.metrics.clone());
    let Some(metrics) = metrics else {
        return;
    };
    object.insert(
        "webRtcCamera".to_owned(),
        json!({
            "connected": metrics.connected.load(Ordering::Relaxed),
            "rtpPackets": metrics.rtp_packets.load(Ordering::Relaxed),
            "rtpBytes": metrics.rtp_bytes.load(Ordering::Relaxed),
            "lostPackets": metrics.lost_packets.load(Ordering::Relaxed),
            "reorderedPackets": metrics.reordered_packets.load(Ordering::Relaxed),
            "assembledFrames": metrics.assembled_frames.load(Ordering::Relaxed),
            "publishedFrames": metrics.published_frames.load(Ordering::Relaxed),
            "droppedFrames": metrics.dropped_frames.load(Ordering::Relaxed),
            "dependencyDrops": metrics.dependency_drops.load(Ordering::Relaxed),
            "nativeErrors": metrics.native_errors.load(Ordering::Relaxed),
            "pliCount": metrics.pli_count.load(Ordering::Relaxed),
            "queueHighWater": metrics.queue_high_water.load(Ordering::Relaxed),
        }),
    );
}

fn sessions() -> &'static Mutex<HashMap<String, Arc<CameraWebRtcSession>>> {
    CAMERA_WEBRTC_SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

async fn receive_h264_track(
    udid: String,
    peer_connection: Arc<webrtc::peer_connection::RTCPeerConnection>,
    track: Arc<TrackRemote>,
    metrics: Arc<CameraWebRtcMetrics>,
) {
    if track.kind() != RTPCodecType::Video {
        return;
    }
    let queue = Arc::new(LatestFrameQueue::default());
    let decoder_queue = queue.clone();
    let decoder_metrics = metrics.clone();
    let decoder_udid = udid.clone();
    std::thread::spawn(move || decode_latest_frames(decoder_udid, decoder_queue, decoder_metrics));

    let mut reorder = RtpReorderBuffer::new(REORDER_WINDOW_PACKETS);
    let mut assembler = H264FrameAssembler::default();
    let mut last_pli = Instant::now() - PLI_INTERVAL;
    while let Ok((packet, _)) = track.read_rtp().await {
        metrics.rtp_packets.fetch_add(1, Ordering::Relaxed);
        metrics
            .rtp_bytes
            .fetch_add(packet.payload.len() as u64, Ordering::Relaxed);
        let reordered = reorder.push(packet);
        if reordered.reordered {
            metrics.reordered_packets.fetch_add(1, Ordering::Relaxed);
        }
        if reordered.lost > 0 {
            metrics
                .lost_packets
                .fetch_add(reordered.lost, Ordering::Relaxed);
            assembler.mark_loss();
        }
        for packet in reordered.packets {
            let result = assembler.push(packet);
            if let Some(frame) = result.frame {
                metrics.assembled_frames.fetch_add(1, Ordering::Relaxed);
                queue.push(frame, &metrics);
            } else if result.request_keyframe {
                metrics.dependency_drops.fetch_add(1, Ordering::Relaxed);
            }
            if result.request_keyframe && last_pli.elapsed() >= PLI_INTERVAL {
                let packet = PictureLossIndication {
                    sender_ssrc: 0,
                    media_ssrc: track.ssrc(),
                };
                if peer_connection
                    .write_rtcp(&[Box::new(packet)])
                    .await
                    .is_ok()
                {
                    metrics.pli_count.fetch_add(1, Ordering::Relaxed);
                    last_pli = Instant::now();
                }
            }
        }
    }
    queue.close();
}

fn decode_latest_frames(
    udid: String,
    queue: Arc<LatestFrameQueue>,
    metrics: Arc<CameraWebRtcMetrics>,
) {
    let mut sequence = 0u32;
    while let Some(frame) = queue.pop() {
        if let Some(config) = frame.decoder_config {
            let mut packet = Vec::with_capacity(config.len() + 1);
            packet.push(1);
            packet.extend_from_slice(&config);
            if let Err(err) = publish_camera_packet(&udid, &packet) {
                metrics.native_errors.fetch_add(1, Ordering::Relaxed);
                warn!("Unable to configure camera H.264 decoder for {udid}: {err}");
                continue;
            }
        }
        let mut packet = Vec::with_capacity(frame.data.len() + 6);
        packet.push(2);
        packet.push(u8::from(frame.key_frame));
        packet.extend_from_slice(&sequence.to_be_bytes());
        packet.extend_from_slice(&frame.data);
        sequence = sequence.wrapping_add(1);
        match publish_camera_packet(&udid, &packet) {
            Ok(()) => {
                metrics.published_frames.fetch_add(1, Ordering::Relaxed);
            }
            Err(err) => {
                metrics.native_errors.fetch_add(1, Ordering::Relaxed);
                warn!("Unable to publish camera H.264 frame for {udid}: {err}");
            }
        }
    }
}

#[derive(Default)]
struct LatestFrameQueue {
    state: Mutex<LatestFrameQueueState>,
    changed: Condvar,
}

#[derive(Default)]
struct LatestFrameQueueState {
    frame: Option<CompleteFrame>,
    closed: bool,
}

impl LatestFrameQueue {
    fn push(&self, frame: CompleteFrame, metrics: &CameraWebRtcMetrics) {
        let mut state = self.state.lock().unwrap();
        if state.closed {
            return;
        }
        if state.frame.replace(frame).is_some() {
            metrics.dropped_frames.fetch_add(1, Ordering::Relaxed);
        }
        metrics.queue_high_water.store(1, Ordering::Relaxed);
        self.changed.notify_one();
    }

    fn pop(&self) -> Option<CompleteFrame> {
        let mut state = self.state.lock().unwrap();
        loop {
            if let Some(frame) = state.frame.take() {
                return Some(frame);
            }
            if state.closed {
                return None;
            }
            state = self.changed.wait(state).unwrap();
        }
    }

    fn close(&self) {
        let mut state = self.state.lock().unwrap();
        state.closed = true;
        state.frame = None;
        self.changed.notify_all();
    }
}

struct CompleteFrame {
    data: Bytes,
    key_frame: bool,
    decoder_config: Option<Bytes>,
}

struct H264FrameAssembler {
    timestamp: Option<u32>,
    data: BytesMut,
    fragmented_nal: Option<BytesMut>,
    key_frame: bool,
    damaged: bool,
    waiting_for_keyframe: bool,
    sps: Option<Bytes>,
    pps: Option<Bytes>,
}

impl Default for H264FrameAssembler {
    fn default() -> Self {
        Self {
            timestamp: None,
            data: BytesMut::new(),
            fragmented_nal: None,
            key_frame: false,
            damaged: false,
            waiting_for_keyframe: true,
            sps: None,
            pps: None,
        }
    }
}

struct AssemblyResult {
    frame: Option<CompleteFrame>,
    request_keyframe: bool,
}

impl H264FrameAssembler {
    fn mark_loss(&mut self) {
        self.damaged = true;
        self.waiting_for_keyframe = true;
    }

    fn push(&mut self, packet: Packet) -> AssemblyResult {
        let mut request_keyframe = false;
        if self
            .timestamp
            .is_some_and(|timestamp| timestamp != packet.header.timestamp)
        {
            if !self.data.is_empty() || self.fragmented_nal.is_some() {
                request_keyframe = true;
                self.waiting_for_keyframe = true;
            }
            self.reset_frame(packet.header.timestamp);
        } else if self.timestamp.is_none() {
            self.timestamp = Some(packet.header.timestamp);
        }

        if self.push_payload(&packet.payload).is_err() {
            self.damaged = true;
            self.waiting_for_keyframe = true;
        }
        if !packet.header.marker {
            return AssemblyResult {
                frame: None,
                request_keyframe,
            };
        }
        if self.fragmented_nal.is_some() {
            self.damaged = true;
        }
        let frame = if self.damaged || self.data.is_empty() {
            request_keyframe = true;
            self.waiting_for_keyframe = true;
            None
        } else if self.waiting_for_keyframe && !self.key_frame {
            request_keyframe = true;
            None
        } else {
            let decoder_config = if self.key_frame {
                self.decoder_config()
            } else {
                None
            };
            if self.key_frame && decoder_config.is_none() {
                request_keyframe = true;
                self.waiting_for_keyframe = true;
                None
            } else {
                if self.key_frame {
                    self.waiting_for_keyframe = false;
                }
                Some(CompleteFrame {
                    data: self.data.split().freeze(),
                    key_frame: self.key_frame,
                    decoder_config,
                })
            }
        };
        let next_timestamp = packet.header.timestamp.wrapping_add(1);
        self.reset_frame(next_timestamp);
        AssemblyResult {
            frame,
            request_keyframe,
        }
    }

    fn reset_frame(&mut self, timestamp: u32) {
        self.timestamp = Some(timestamp);
        self.data.clear();
        self.fragmented_nal = None;
        self.key_frame = false;
        self.damaged = false;
    }

    fn push_payload(&mut self, payload: &Bytes) -> Result<(), ()> {
        if payload.len() < 2 {
            return Err(());
        }
        match payload[0] & 0x1f {
            1..=23 => self.push_nal(payload),
            24 => {
                let mut offset = 1;
                while offset + 2 <= payload.len() {
                    let length =
                        u16::from_be_bytes([payload[offset], payload[offset + 1]]) as usize;
                    offset += 2;
                    if length == 0 || offset + length > payload.len() {
                        return Err(());
                    }
                    self.push_nal(&payload.slice(offset..offset + length))?;
                    offset += length;
                }
                if offset == payload.len() {
                    Ok(())
                } else {
                    Err(())
                }
            }
            28 => self.push_fu_a(payload),
            _ => Err(()),
        }
    }

    fn push_fu_a(&mut self, payload: &Bytes) -> Result<(), ()> {
        if payload.len() < 3 {
            return Err(());
        }
        let start = payload[1] & 0x80 != 0;
        let end = payload[1] & 0x40 != 0;
        if start {
            if self.fragmented_nal.is_some() {
                return Err(());
            }
            let mut nal = BytesMut::with_capacity(payload.len() * 2);
            nal.put_u8((payload[0] & 0xe0) | (payload[1] & 0x1f));
            nal.extend_from_slice(&payload[2..]);
            self.fragmented_nal = Some(nal);
        } else if let Some(nal) = self.fragmented_nal.as_mut() {
            nal.extend_from_slice(&payload[2..]);
        } else {
            return Err(());
        }
        if end {
            let nal = self.fragmented_nal.take().ok_or(())?.freeze();
            self.push_nal(&nal)?;
        }
        Ok(())
    }

    fn push_nal(&mut self, nal: &Bytes) -> Result<(), ()> {
        if nal.is_empty() {
            return Err(());
        }
        match nal[0] & 0x1f {
            7 => self.sps = Some(nal.clone()),
            8 => self.pps = Some(nal.clone()),
            9 => {}
            5 => {
                self.key_frame = true;
                self.append_avcc_nal(nal)?;
            }
            _ => self.append_avcc_nal(nal)?,
        }
        Ok(())
    }

    fn append_avcc_nal(&mut self, nal: &Bytes) -> Result<(), ()> {
        if self.data.len() + nal.len() + 4 > MAX_FRAME_BYTES {
            return Err(());
        }
        self.data.put_u32(nal.len() as u32);
        self.data.extend_from_slice(nal);
        Ok(())
    }

    fn decoder_config(&self) -> Option<Bytes> {
        let sps = self.sps.as_ref()?;
        let pps = self.pps.as_ref()?;
        if sps.len() < 4 || sps.len() > u16::MAX as usize || pps.len() > u16::MAX as usize {
            return None;
        }
        let mut config = BytesMut::with_capacity(sps.len() + pps.len() + 16);
        config.put_u8(1);
        config.put_u8(sps[1]);
        config.put_u8(sps[2]);
        config.put_u8(sps[3]);
        config.put_u8(0xff);
        config.put_u8(0xe1);
        config.put_u16(sps.len() as u16);
        config.extend_from_slice(sps);
        config.put_u8(1);
        config.put_u16(pps.len() as u16);
        config.extend_from_slice(pps);
        Some(config.freeze())
    }
}

struct ReorderResult {
    packets: Vec<Packet>,
    lost: u64,
    reordered: bool,
}

struct RtpReorderBuffer {
    expected: Option<u16>,
    pending: HashMap<u16, Packet>,
    window: usize,
}

impl RtpReorderBuffer {
    fn new(window: usize) -> Self {
        Self {
            expected: None,
            pending: HashMap::new(),
            window,
        }
    }

    fn push(&mut self, packet: Packet) -> ReorderResult {
        let sequence = packet.header.sequence_number;
        let expected = self.expected.get_or_insert(sequence);
        let distance = sequence.wrapping_sub(*expected);
        if distance > u16::MAX / 2 {
            return ReorderResult {
                packets: Vec::new(),
                lost: 0,
                reordered: true,
            };
        }
        let marker = packet.header.marker;
        let reordered = distance > 0;
        self.pending.entry(sequence).or_insert(packet);
        let mut packets = self.drain_contiguous();
        let mut lost = 0;
        if self.pending.len() > self.window
            || (marker && packets.last().is_none_or(|p| !p.header.marker))
        {
            if let Some(next) = self.nearest_pending_sequence() {
                let expected = self.expected.unwrap();
                lost = next.wrapping_sub(expected) as u64;
                self.expected = Some(next);
                packets.extend(self.drain_contiguous());
            }
        }
        ReorderResult {
            packets,
            lost,
            reordered,
        }
    }

    fn drain_contiguous(&mut self) -> Vec<Packet> {
        let mut packets = Vec::new();
        while let Some(expected) = self.expected {
            let Some(packet) = self.pending.remove(&expected) else {
                break;
            };
            self.expected = Some(expected.wrapping_add(1));
            packets.push(packet);
        }
        packets
    }

    fn nearest_pending_sequence(&self) -> Option<u16> {
        let expected = self.expected?;
        self.pending
            .keys()
            .copied()
            .min_by_key(|sequence| sequence.wrapping_sub(expected))
    }
}

#[cfg(test)]
mod tests {
    use super::{H264FrameAssembler, RtpReorderBuffer};
    use bytes::Bytes;
    use webrtc::rtp::header::Header;
    use webrtc::rtp::packet::Packet;

    fn packet(sequence: u16, timestamp: u32, marker: bool, payload: &[u8]) -> Packet {
        Packet {
            header: Header {
                sequence_number: sequence,
                timestamp,
                marker,
                ..Default::default()
            },
            payload: Bytes::copy_from_slice(payload),
        }
    }

    #[test]
    fn assembles_avcc_keyframe_and_decoder_configuration() {
        let mut assembler = H264FrameAssembler::default();
        assert!(assembler
            .push(packet(1, 90_000, false, &[0x67, 0x42, 0xe0, 0x1f]))
            .frame
            .is_none());
        assert!(assembler
            .push(packet(2, 90_000, false, &[0x68, 0xce, 0x06, 0xe2]))
            .frame
            .is_none());
        let result = assembler.push(packet(3, 90_000, true, &[0x65, 0xaa, 0xbb]));
        let frame = result.frame.unwrap();

        assert!(frame.key_frame);
        assert_eq!(&frame.data[..], &[0, 0, 0, 3, 0x65, 0xaa, 0xbb]);
        assert!(frame.decoder_config.unwrap().len() > 12);
        assert!(!result.request_keyframe);
    }

    #[test]
    fn assembles_stap_a_parameter_sets_and_fu_a_slice() {
        let mut assembler = H264FrameAssembler::default();
        let stap = [
            0x78, 0, 4, 0x67, 0x42, 0xe0, 0x1f, 0, 4, 0x68, 0xce, 0x06, 0xe2,
        ];
        assert!(assembler
            .push(packet(1, 90_000, false, &stap))
            .frame
            .is_none());
        assert!(assembler
            .push(packet(2, 90_000, false, &[0x7c, 0x85, 0xaa]))
            .frame
            .is_none());
        let result = assembler.push(packet(3, 90_000, true, &[0x7c, 0x45, 0xbb]));

        assert_eq!(
            &result.frame.unwrap().data[..],
            &[0, 0, 0, 3, 0x65, 0xaa, 0xbb]
        );
    }

    #[test]
    fn reorders_packets_and_declares_marker_gaps_lost() {
        let mut reorder = RtpReorderBuffer::new(8);
        assert_eq!(reorder.push(packet(10, 1, false, &[1, 1])).packets.len(), 1);
        let delayed = reorder.push(packet(12, 1, false, &[1, 3]));
        assert!(delayed.packets.is_empty());
        assert!(delayed.reordered);
        let ordered = reorder.push(packet(11, 1, false, &[1, 2]));
        assert_eq!(ordered.packets.len(), 2);
        let loss = reorder.push(packet(14, 1, true, &[1, 4]));
        assert_eq!(loss.lost, 1);
        assert_eq!(loss.packets.len(), 1);
    }
}
