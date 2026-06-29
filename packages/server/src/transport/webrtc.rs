use crate::android;
use crate::android_emulation_control::{
    emulator_controller_client::EmulatorControllerClient, image_format, ImageFormat,
};
use crate::api::routes::{
    apply_stream_client_foreground_from_stats, apply_stream_quality_payload,
    bridge_input_session_for_control, run_bridge_multitouch_control_message, run_control_message,
    run_toggle_appearance_control, run_tvos_control_message, stream_quality_limits_for_payload,
    AppState, ControlMessage, StreamQualityPayload, TvosControlTouchGesture,
};
use crate::error::AppError;
use crate::metrics::counters::ClientStreamStats;
use crate::native::ffi;
use crate::transport::packet::{FramePacket, SharedFrame};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::ffi::{c_void, CStr, CString};
use std::net::{SocketAddr, TcpStream};
use std::ptr;
use std::slice;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock, Weak};
use std::thread;
use std::time::Duration;
use tokio::runtime::Builder as TokioRuntimeBuilder;
use tokio::sync::{broadcast, mpsc};
use tokio::task;
use tokio::time::{self, Instant};
use tracing::{info, warn};
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::{MediaEngine, MIME_TYPE_H264};
use webrtc::api::APIBuilder;
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::data_channel::RTCDataChannel;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::policy::ice_transport_policy::RTCIceTransportPolicy;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::rtcp::packet::Packet as RtcpPacket;
use webrtc::rtcp::payload_feedbacks::full_intra_request::FullIntraRequest;
use webrtc::rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
use webrtc::rtp::codecs::h264::H264Payloader;
use webrtc::rtp::packetizer::{new_packetizer, Packetizer};
use webrtc::rtp::sequence::new_random_sequencer;
use webrtc::rtp_transceiver::rtp_codec::{
    RTCRtpCodecCapability, RTCRtpCodecParameters, RTPCodecType,
};
use webrtc::rtp_transceiver::RTCPFeedback;
use webrtc::track::track_local::track_local_static_rtp::TrackLocalStaticRTP;
use webrtc::track::track_local::TrackLocal;
use webrtc::track::track_local::TrackLocalWriter;

const ANNEX_B_START_CODE: &[u8] = &[0, 0, 0, 1];
const DEFAULT_STUN_URL: &str = "stun:stun.l.google.com:19302";
const WEBRTC_CONTROL_CHANNEL_LABEL: &str = "simdeck-control";
const WEBRTC_TELEMETRY_CHANNEL_LABEL: &str = "simdeck-telemetry";
const WEBRTC_DEFAULT_LOCAL_STREAM_FPS: u32 = 60;
const WEBRTC_MAX_LOCAL_STREAM_FPS: u32 = 240;
const WEBRTC_WRITE_TIMEOUT: Duration = Duration::from_millis(120);
const WEBRTC_REALTIME_WRITE_TIMEOUT: Duration = Duration::from_millis(45);
const WEBRTC_REALTIME_KEYFRAME_WRITE_TIMEOUT: Duration = Duration::from_millis(90);
const WEBRTC_INITIAL_KEYFRAME_TIMEOUT: Duration = Duration::from_secs(5);
const WEBRTC_FAST_ICE_GATHER_TIMEOUT: Duration = Duration::from_millis(250);
const WEBRTC_FULL_ICE_GATHER_TIMEOUT: Duration = Duration::from_secs(3);
const WEBRTC_MULTITOUCH_INPUT_IDLE_TIMEOUT: Duration = Duration::from_secs(5);
const WEBRTC_RTP_OUTBOUND_MTU: usize = 1200;
const WEBRTC_PEER_DISCONNECTED_TIMEOUT: Duration = Duration::from_secs(12);
const ANDROID_WEBRTC_FRAME_BROADCAST_CAPACITY: usize = 1;
const ANDROID_WEBRTC_DEFAULT_POLL_FPS: u64 = 120;
const ANDROID_WEBRTC_DEFAULT_MAX_EDGE: u32 = 960;
const ANDROID_WEBRTC_DEFAULT_FPS: u32 = 60;
const ANDROID_WEBRTC_DEFAULT_MIN_BITRATE: u32 = 6_000_000;
const ANDROID_WEBRTC_DEFAULT_BITS_PER_PIXEL: u32 = 5;
const ANDROID_WEBRTC_DEFAULT_VIDEO_CODEC: &str = "software";
const ANDROID_SHARED_VIDEO_RETRY_DELAY: Duration = Duration::from_millis(200);
const ANDROID_GRPC_FALLBACK_RETRY_INTERVAL: Duration = Duration::from_secs(2);
const ANDROID_GRPC_FALLBACK_PROBE_TIMEOUT: Duration = Duration::from_millis(100);
static WEBRTC_MEDIA_STREAMS: OnceLock<Mutex<HashMap<String, Vec<WebRtcMediaStreamToken>>>> =
    OnceLock::new();
static ANDROID_WEBRTC_SOURCES: OnceLock<Mutex<Vec<Weak<AndroidWebRtcSourceInner>>>> =
    OnceLock::new();
static NEXT_ANDROID_WEBRTC_SOURCE_ID: AtomicU64 = AtomicU64::new(1);
const MAX_WEBRTC_MEDIA_STREAMS_PER_UDID: usize = 16;

#[derive(Clone)]
struct WebRtcMediaStreamToken {
    client_id: Option<String>,
    cancellation: broadcast::Sender<()>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebRtcOfferPayload {
    pub client_id: Option<String>,
    pub sdp: String,
    #[serde(rename = "streamConfig")]
    pub stream_config: Option<StreamQualityPayload>,
    #[serde(rename = "type")]
    pub kind: String,
    pub transport: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebRtcAnswerPayload {
    pub sdp: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub video: WebRtcVideoMetadata,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebRtcVideoMetadata {
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AndroidH264StreamConfig {
    quality: android::AndroidH264StreamQuality,
    video_codec: String,
}

fn android_h264_config_from_payload(
    payload: Option<&StreamQualityPayload>,
) -> Result<AndroidH264StreamConfig, AppError> {
    let video_codec = android_video_codec_from_payload(payload)?;
    let Some(payload) = payload else {
        return Ok(AndroidH264StreamConfig {
            quality: default_android_h264_quality(),
            video_codec,
        });
    };
    let limits = stream_quality_limits_for_payload(payload)?;
    let profile = payload
        .profile
        .as_deref()
        .map(str::trim)
        .filter(|profile| !profile.is_empty());
    let max_edge = match profile {
        Some("full" | "quality") => None,
        Some("balanced" | "smooth") => Some(ANDROID_WEBRTC_DEFAULT_MAX_EDGE),
        _ => Some(limits.max_edge),
    };
    Ok(AndroidH264StreamConfig {
        quality: android::AndroidH264StreamQuality {
            max_edge,
            fps: Some(limits.fps),
            min_bitrate: Some(limits.min_bitrate),
            bits_per_pixel: Some(limits.bits_per_pixel),
        },
        video_codec,
    })
}

fn android_video_codec_from_payload(
    payload: Option<&StreamQualityPayload>,
) -> Result<String, AppError> {
    if let Some(video_codec) = payload
        .and_then(|payload| payload.video_codec.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return crate::api::routes::normalize_video_codec(video_codec)
            .map(ToOwned::to_owned)
            .ok_or_else(|| AppError::bad_request(format!("Unknown video codec `{video_codec}`.")));
    }
    Ok(std::env::var("SIMDECK_ANDROID_VIDEO_CODEC")
        .ok()
        .and_then(|value| crate::api::routes::normalize_video_codec(&value).map(ToOwned::to_owned))
        .unwrap_or_else(|| ANDROID_WEBRTC_DEFAULT_VIDEO_CODEC.to_owned()))
}

fn default_android_h264_quality() -> android::AndroidH264StreamQuality {
    android::AndroidH264StreamQuality {
        max_edge: Some(ANDROID_WEBRTC_DEFAULT_MAX_EDGE),
        fps: Some(ANDROID_WEBRTC_DEFAULT_FPS),
        min_bitrate: Some(ANDROID_WEBRTC_DEFAULT_MIN_BITRATE),
        bits_per_pixel: Some(ANDROID_WEBRTC_DEFAULT_BITS_PER_PIXEL),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientIceServer {
    pub urls: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
}

pub async fn create_answer(
    state: AppState,
    udid: String,
    payload: WebRtcOfferPayload,
    _peer_is_loopback: bool,
) -> Result<WebRtcAnswerPayload, AppError> {
    if payload.kind != "offer" {
        return Err(AppError::bad_request(
            "WebRTC payload must include type `offer`.",
        ));
    }
    let is_android = android::is_android_id(&udid);
    if payload.transport.is_some() {
        return Err(AppError::bad_request(
            "Unsupported WebRTC transport. SimDeck streams WebRTC video over media tracks.",
        ));
    }
    if !is_android {
        if let Some(stream_config) = payload.stream_config.as_ref() {
            apply_stream_quality_payload(&state, stream_config)?;
        }
    }

    let source = if is_android {
        let stream_config = android_h264_config_from_payload(payload.stream_config.as_ref())?;
        WebRtcVideoSource::Android(
            AndroidWebRtcSource::start(
                state.android.clone(),
                state.metrics.clone(),
                udid.clone(),
                stream_config,
            )
            .await?,
        )
    } else {
        let session = state.registry.get_or_create_async(&udid).await?;
        if let Err(error) = session.ensure_started_async().await {
            state.registry.remove(&udid);
            return Err(error);
        }
        apply_stream_client_foreground(&state, &session, &payload.client_id, Some(true));
        WebRtcVideoSource::Simulator(session)
    };

    info!(
        "WebRTC offer for {udid}: remote_candidates={} remote_candidate_types={} ice_servers={} ice_transport_policy={}",
        count_sdp_candidates(&payload.sdp),
        summarize_sdp_candidate_types(&payload.sdp),
        std::env::var("SIMDECK_WEBRTC_ICE_SERVERS")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_STUN_URL.to_owned()),
        ice_transport_policy_label()
    );

    let first_frame = wait_for_h264_sync_keyframe(&source, WEBRTC_INITIAL_KEYFRAME_TIMEOUT)
        .await
        .ok_or_else(|| AppError::native("Timed out waiting for a device H.264 keyframe."))?;
    let codec = first_frame
        .codec
        .as_deref()
        .unwrap_or_default()
        .to_lowercase();
    if !is_h264_codec(&codec) {
        return Err(AppError::bad_request(
            "WebRTC preview requires H.264. Restart SimDeck with `--video-codec auto`, `hardware`, or `software`.",
        ));
    }

    let h264_fmtp_line = h264_sdp_fmtp_line(&codec, &payload.sdp);
    let mut media_engine = MediaEngine::default();
    media_engine
        .register_codec(
            RTCRtpCodecParameters {
                capability: RTCRtpCodecCapability {
                    mime_type: MIME_TYPE_H264.to_owned(),
                    clock_rate: 90_000,
                    channels: 0,
                    sdp_fmtp_line: h264_fmtp_line.clone(),
                    rtcp_feedback: h264_rtcp_feedback(),
                },
                payload_type: 96,
                ..Default::default()
            },
            RTPCodecType::Video,
        )
        .map_err(|error| AppError::internal(format!("register WebRTC H.264 codec: {error}")))?;
    let mut registry = Registry::new();
    registry = register_default_interceptors(registry, &mut media_engine)
        .map_err(|error| AppError::internal(format!("register WebRTC interceptors: {error}")))?;
    let api = APIBuilder::new()
        .with_media_engine(media_engine)
        .with_interceptor_registry(registry)
        .build();

    let peer_connection = Arc::new(
        api.new_peer_connection(RTCConfiguration {
            ice_servers: ice_servers(),
            ice_transport_policy: ice_transport_policy(),
            ..Default::default()
        })
        .await
        .map_err(|error| AppError::internal(format!("create WebRTC peer connection: {error}")))?,
    );
    register_diagnostics(&peer_connection, &udid);
    let (stream_control_tx, stream_control_rx) = mpsc::unbounded_channel();
    match &source {
        WebRtcVideoSource::Simulator(session) => register_control_data_channel(
            &peer_connection,
            session.clone(),
            state.clone(),
            udid.clone(),
            stream_control_tx,
        ),
        WebRtcVideoSource::Android(source) => register_android_data_channel(
            &peer_connection,
            source.clone(),
            state.clone(),
            udid.clone(),
            stream_control_tx,
        ),
    }

    let video_track = Arc::new(TrackLocalStaticRTP::new(
        RTCRtpCodecCapability {
            mime_type: MIME_TYPE_H264.to_owned(),
            clock_rate: 90_000,
            channels: 0,
            sdp_fmtp_line: h264_fmtp_line,
            rtcp_feedback: h264_rtcp_feedback(),
        },
        "simdeck-video".to_owned(),
        "simdeck".to_owned(),
    ));

    let rtp_sender = peer_connection
        .add_track(video_track.clone() as Arc<dyn TrackLocal + Send + Sync>)
        .await
        .map_err(|error| AppError::internal(format!("add WebRTC video track: {error}")))?;
    let rtcp_source = source.clone();
    let rtcp_udid = udid.clone();
    tokio::spawn(async move {
        while let Ok((packets, _attributes)) = rtp_sender.read_rtcp().await {
            if packets
                .iter()
                .any(|packet| rtcp_packet_requests_keyframe(packet.as_ref()))
            {
                info!("WebRTC RTCP requested keyframe for {rtcp_udid}");
                rtcp_source.request_keyframe();
            }
        }
    });

    let fast_gather =
        has_sdp_candidate_type(&payload.sdp, "host") && ice_transport_policy_label() == "all";
    let offer = RTCSessionDescription::offer(payload.sdp)
        .map_err(|error| AppError::bad_request(format!("invalid WebRTC offer: {error}")))?;
    peer_connection
        .set_remote_description(offer)
        .await
        .map_err(|error| AppError::bad_request(format!("set remote WebRTC offer: {error}")))?;

    let answer = peer_connection
        .create_answer(None)
        .await
        .map_err(|error| AppError::internal(format!("create WebRTC answer: {error}")))?;
    let mut gather_complete = peer_connection.gathering_complete_promise().await;
    peer_connection
        .set_local_description(answer)
        .await
        .map_err(|error| AppError::internal(format!("set WebRTC answer: {error}")))?;
    let gather_timeout = if fast_gather {
        WEBRTC_FAST_ICE_GATHER_TIMEOUT
    } else {
        WEBRTC_FULL_ICE_GATHER_TIMEOUT
    };
    let gather_result = time::timeout(gather_timeout, gather_complete.recv()).await;
    let mut local_description = peer_connection
        .local_description()
        .await
        .ok_or_else(|| AppError::internal("WebRTC local description was not set."))?;
    if gather_result.is_err() && count_sdp_candidates(&local_description.sdp) == 0 {
        let _ = time::timeout(WEBRTC_FULL_ICE_GATHER_TIMEOUT, gather_complete.recv()).await;
        local_description = peer_connection
            .local_description()
            .await
            .ok_or_else(|| AppError::internal("WebRTC local description was not set."))?;
    }
    info!(
        "WebRTC answer for {udid}: local_candidates={} local_candidate_types={}",
        count_sdp_candidates(&local_description.sdp),
        summarize_sdp_candidate_types(&local_description.sdp)
    );

    let first_frame_width = first_frame.width;
    let first_frame_height = first_frame.height;
    let client_id = payload.client_id.clone();
    let (cancellation_token, cancellation) =
        register_webrtc_media_stream(&udid, payload.client_id.as_deref(), true, false);
    tokio::spawn(
        WebRtcMediaStream {
            state,
            udid,
            client_id,
            source,
            first_frame,
            peer_connection,
            video_track,
            cancellation_token,
            cancellation,
            stream_control_rx,
        }
        .run(),
    );

    Ok(WebRtcAnswerPayload {
        sdp: local_description.sdp,
        kind: "answer".to_owned(),
        video: WebRtcVideoMetadata {
            width: first_frame_width,
            height: first_frame_height,
        },
    })
}

fn register_diagnostics(
    peer_connection: &Arc<webrtc::peer_connection::RTCPeerConnection>,
    udid: &str,
) {
    let candidate_udid = udid.to_owned();
    peer_connection.on_ice_candidate(Box::new(move |candidate| {
        let candidate_udid = candidate_udid.clone();
        Box::pin(async move {
            match candidate {
                Some(candidate) => {
                    info!(
                        "WebRTC local candidate for {candidate_udid}: type={} protocol={} address={} port={} related={}:{} tcp={}",
                        candidate.typ,
                        candidate.protocol,
                        redact_candidate_address(&candidate.address),
                        candidate.port,
                        redact_candidate_address(&candidate.related_address),
                        candidate.related_port,
                        candidate.tcp_type
                    );
                }
                None => {
                    info!("WebRTC local candidate gathering complete for {candidate_udid}");
                }
            }
        })
    }));

    let gathering_udid = udid.to_owned();
    peer_connection.on_ice_gathering_state_change(Box::new(move |state| {
        let gathering_udid = gathering_udid.clone();
        Box::pin(async move {
            info!("WebRTC ICE gathering state for {gathering_udid}: {state}");
        })
    }));

    let ice_udid = udid.to_owned();
    peer_connection.on_ice_connection_state_change(Box::new(move |state| {
        let ice_udid = ice_udid.clone();
        Box::pin(async move {
            info!("WebRTC ICE connection state for {ice_udid}: {state}");
        })
    }));

    let peer_udid = udid.to_owned();
    peer_connection.on_peer_connection_state_change(Box::new(move |state| {
        let peer_udid = peer_udid.clone();
        Box::pin(async move {
            info!("WebRTC peer connection state for {peer_udid}: {state}");
        })
    }));
}

fn count_sdp_candidates(sdp: &str) -> usize {
    sdp.lines()
        .filter(|line| line.starts_with("a=candidate:"))
        .count()
}

fn has_sdp_candidate_type(sdp: &str, candidate_type: &str) -> bool {
    sdp.lines()
        .filter(|line| line.starts_with("a=candidate:"))
        .any(|line| {
            line.split_whitespace()
                .collect::<Vec<_>>()
                .windows(2)
                .any(|pair| pair[0] == "typ" && pair[1] == candidate_type)
        })
}

fn summarize_sdp_candidate_types(sdp: &str) -> String {
    let mut host = 0usize;
    let mut srflx = 0usize;
    let mut prflx = 0usize;
    let mut relay = 0usize;
    let mut other = 0usize;
    for line in sdp.lines().filter(|line| line.starts_with("a=candidate:")) {
        match line
            .split_whitespace()
            .collect::<Vec<_>>()
            .windows(2)
            .find_map(|pair| {
                if pair[0] == "typ" {
                    Some(pair[1])
                } else {
                    None
                }
            }) {
            Some("host") => host += 1,
            Some("srflx") => srflx += 1,
            Some("prflx") => prflx += 1,
            Some("relay") => relay += 1,
            Some(_) | None => other += 1,
        }
    }
    format!("host={host},srflx={srflx},prflx={prflx},relay={relay},other={other}")
}

fn redact_candidate_address(address: &str) -> String {
    if address.is_empty() {
        return String::new();
    }
    if address.parse::<std::net::IpAddr>().is_ok() {
        return "<ip>".to_owned();
    }
    "<host>".to_owned()
}

fn register_control_data_channel(
    peer_connection: &Arc<webrtc::peer_connection::RTCPeerConnection>,
    session: crate::simulators::session::SimulatorSession,
    state: AppState,
    udid: String,
    stream_control_tx: mpsc::UnboundedSender<WebRtcStreamCommand>,
) {
    peer_connection.on_data_channel(Box::new(move |channel: Arc<RTCDataChannel>| {
        let session = session.clone();
        let state = state.clone();
        let udid = udid.clone();
        let stream_control_tx = stream_control_tx.clone();
        Box::pin(async move {
            let label = channel.label();
            if label != WEBRTC_CONTROL_CHANNEL_LABEL && label != WEBRTC_TELEMETRY_CHANNEL_LABEL {
                return;
            }
            attach_control_data_channel(channel, session, state, udid, stream_control_tx);
        })
    }));
}

fn attach_control_data_channel(
    channel: Arc<RTCDataChannel>,
    session: crate::simulators::session::SimulatorSession,
    state: AppState,
    udid: String,
    stream_control_tx: mpsc::UnboundedSender<WebRtcStreamCommand>,
) {
    let (control_tx, control_rx) = mpsc::unbounded_channel::<ControlMessage>();
    task::spawn(run_webrtc_control_queue(
        session.clone(),
        state.clone(),
        udid.clone(),
        stream_control_tx.clone(),
        control_rx,
    ));
    channel.on_message(Box::new(move |message: DataChannelMessage| {
        let session = session.clone();
        let state = state.clone();
        let udid = udid.clone();
        let stream_control_tx = stream_control_tx.clone();
        let control_tx = control_tx.clone();
        Box::pin(async move {
            let Ok(text) = std::str::from_utf8(&message.data) else {
                warn!("Invalid WebRTC control message bytes for {udid}");
                return;
            };
            if let Ok(message) = serde_json::from_str::<WebRtcDataChannelMessage>(text) {
                match message {
                    WebRtcDataChannelMessage::ClientStats { stats } => {
                        if !stats.client_id.trim().is_empty() && !stats.kind.trim().is_empty() {
                            apply_stream_client_foreground_from_stats(&state, &stats);
                            state.metrics.record_client_stream_stats(*stats);
                        }
                    }
                    WebRtcDataChannelMessage::StreamControl {
                        client_id,
                        force_keyframe,
                        foreground,
                        snapshot,
                    } => {
                        apply_stream_client_foreground(&state, &session, &client_id, foreground);
                        let command = WebRtcStreamCommand {
                            force_keyframe: force_keyframe.unwrap_or(false),
                            snapshot: snapshot.unwrap_or(false),
                        };
                        if command.force_keyframe || command.snapshot {
                            session.request_keyframe();
                        }
                        let _ = stream_control_tx.send(command);
                    }
                    WebRtcDataChannelMessage::StreamQuality { config } => {
                        if let Err(error) = apply_stream_quality_payload(&state, &config) {
                            warn!("WebRTC stream quality update failed for {udid}: {error}");
                        } else {
                            session.request_keyframe();
                        }
                    }
                }
                return;
            }
            let control_message = match serde_json::from_str::<ControlMessage>(text) {
                Ok(message) => message,
                Err(error) => {
                    warn!("Invalid WebRTC control message for {udid}: {error}");
                    return;
                }
            };
            if control_tx.send(control_message).is_err() {
                warn!("WebRTC control queue closed for {udid}");
            }
        })
    }));
}

fn apply_stream_client_foreground(
    state: &AppState,
    session: &crate::simulators::session::SimulatorSession,
    client_id: &Option<String>,
    foreground: Option<bool>,
) {
    let Some(foreground) = foreground else {
        return;
    };
    let Some(client_id) = client_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    let (any_foreground, changed) =
        state
            .stream_clients
            .record(session.udid(), client_id, foreground);
    if changed {
        session.set_client_foreground(any_foreground);
    }
}

fn remove_stream_client_foreground(
    state: &AppState,
    source: &WebRtcVideoSource,
    udid: &str,
    client_id: &Option<String>,
) {
    let Some(client_id) = client_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    let (any_foreground, changed) = state.stream_clients.remove(udid, client_id);
    if changed {
        source.set_client_foreground(any_foreground);
    }
}

fn register_android_data_channel(
    peer_connection: &Arc<webrtc::peer_connection::RTCPeerConnection>,
    source: AndroidWebRtcSource,
    state: AppState,
    udid: String,
    stream_control_tx: mpsc::UnboundedSender<WebRtcStreamCommand>,
) {
    peer_connection.on_data_channel(Box::new(move |channel: Arc<RTCDataChannel>| {
        let source = source.clone();
        let state = state.clone();
        let udid = udid.clone();
        let stream_control_tx = stream_control_tx.clone();
        Box::pin(async move {
            let label = channel.label();
            if label != WEBRTC_CONTROL_CHANNEL_LABEL && label != WEBRTC_TELEMETRY_CHANNEL_LABEL {
                return;
            }
            attach_android_data_channel(channel, source, state, udid, stream_control_tx);
        })
    }));
}

fn attach_android_data_channel(
    channel: Arc<RTCDataChannel>,
    source: AndroidWebRtcSource,
    state: AppState,
    udid: String,
    stream_control_tx: mpsc::UnboundedSender<WebRtcStreamCommand>,
) {
    let (control_tx, control_rx) = mpsc::unbounded_channel::<ControlMessage>();
    task::spawn(run_android_webrtc_control_queue(
        state.clone(),
        udid.clone(),
        control_rx,
    ));
    channel.on_message(Box::new(move |message: DataChannelMessage| {
        let source = source.clone();
        let state = state.clone();
        let udid = udid.clone();
        let stream_control_tx = stream_control_tx.clone();
        let control_tx = control_tx.clone();
        Box::pin(async move {
            let Ok(text) = std::str::from_utf8(&message.data) else {
                warn!("Invalid Android WebRTC control message bytes for {udid}");
                return;
            };
            if let Ok(message) = serde_json::from_str::<WebRtcDataChannelMessage>(text) {
                match message {
                    WebRtcDataChannelMessage::ClientStats { stats } => {
                        if !stats.client_id.trim().is_empty() && !stats.kind.trim().is_empty() {
                            state.metrics.record_client_stream_stats(*stats);
                        }
                    }
                    WebRtcDataChannelMessage::StreamControl {
                        client_id: _,
                        force_keyframe,
                        foreground: _,
                        snapshot,
                    } => {
                        let command = WebRtcStreamCommand {
                            force_keyframe: force_keyframe.unwrap_or(false),
                            snapshot: snapshot.unwrap_or(false),
                        };
                        if command.force_keyframe || command.snapshot {
                            source.request_keyframe();
                        }
                        let _ = stream_control_tx.send(command);
                    }
                    WebRtcDataChannelMessage::StreamQuality { config } => {
                        match android_h264_config_from_payload(Some(&config)) {
                            Ok(config) => source.reconfigure_h264(config.quality),
                            Err(error) => {
                                warn!(
                                    "Android WebRTC stream quality update failed for {udid}: {error}"
                                );
                                source.request_keyframe();
                            }
                        }
                    }
                }
                return;
            }

            let control_message = match serde_json::from_str::<ControlMessage>(text) {
                Ok(message) => message,
                Err(error) => {
                    warn!("Invalid Android WebRTC control message for {udid}: {error}");
                    return;
                }
            };
            if control_tx.send(control_message).is_err() {
                warn!("Android WebRTC control queue closed for {udid}");
            }
        })
    }));
}

async fn run_android_webrtc_control_queue(
    state: AppState,
    udid: String,
    mut receiver: mpsc::UnboundedReceiver<ControlMessage>,
) {
    let mut pending = VecDeque::new();
    loop {
        let mut message = match pending.pop_front() {
            Some(message) => message,
            None => match receiver.recv().await {
                Some(message) => message,
                None => break,
            },
        };
        if webrtc_control_message_is_move(&message) {
            while let Ok(next_message) = receiver.try_recv() {
                if webrtc_control_message_is_move(&next_message) {
                    message = next_message;
                } else {
                    pending.push_back(next_message);
                    break;
                }
            }
        }

        if let Err(error) =
            run_android_webrtc_control_message(state.clone(), udid.clone(), message).await
        {
            warn!("Android WebRTC control message failed for {udid}: {error}");
        }
    }
}

async fn run_android_webrtc_control_message(
    state: AppState,
    udid: String,
    message: ControlMessage,
) -> Result<(), AppError> {
    match message {
        ControlMessage::Touch { x, y, phase } => {
            if !x.is_finite() || !y.is_finite() {
                return Err(AppError::bad_request(
                    "`x` and `y` must be finite normalized numbers.",
                ));
            }
            return handle_android_webrtc_touch(
                state,
                udid,
                x.clamp(0.0, 1.0),
                y.clamp(0.0, 1.0),
                phase,
            )
            .await;
        }
        ControlMessage::EdgeTouch { x, y, phase, .. } => {
            if !x.is_finite() || !y.is_finite() {
                return Err(AppError::bad_request(
                    "`x` and `y` must be finite normalized numbers.",
                ));
            }
            return handle_android_webrtc_touch(
                state,
                udid,
                x.clamp(0.0, 1.0),
                y.clamp(0.0, 1.0),
                phase,
            )
            .await;
        }
        ControlMessage::MultiTouch { x1, y1, phase, .. } => {
            if !x1.is_finite() || !y1.is_finite() {
                return Err(AppError::bad_request(
                    "`x1` and `y1` must be finite normalized numbers.",
                ));
            }
            return handle_android_webrtc_touch(
                state,
                udid,
                x1.clamp(0.0, 1.0),
                y1.clamp(0.0, 1.0),
                phase,
            )
            .await;
        }
        _ => {}
    }

    task::spawn_blocking(move || match message {
        ControlMessage::Key {
            key_code,
            modifiers,
        } => state
            .android
            .send_key(&udid, key_code, modifiers.unwrap_or(0)),
        ControlMessage::Button {
            button,
            duration_ms,
            phase,
            ..
        } => match phase.as_deref() {
            Some("down" | "began") => Ok(()),
            Some("up" | "ended" | "cancelled") | None => {
                state
                    .android
                    .press_button(&udid, &button, duration_ms.unwrap_or(0))
            }
            Some(_) => Err(AppError::bad_request(
                "`phase` must be `down`, `up`, `began`, `ended`, or `cancelled`.",
            )),
        },
        ControlMessage::DismissKeyboard => state.android.dismiss_keyboard(&udid),
        ControlMessage::ToggleSoftwareKeyboard => Err(AppError::bad_request(
            "Software keyboard toggle is only available for iOS simulators.",
        )),
        ControlMessage::Home => state.android.press_home(&udid),
        ControlMessage::AppSwitcher => state.android.open_app_switcher(&udid),
        ControlMessage::RotateLeft => state.android.rotate_left(&udid),
        ControlMessage::RotateRight => state.android.rotate_right(&udid),
        ControlMessage::Crown { .. } => Err(AppError::bad_request(
            "Digital Crown rotation is only available for Apple Watch simulators.",
        )),
        ControlMessage::Scroll { .. } => Err(AppError::bad_request(
            "Native scroll wheel input is disabled because SimulatorKit scroll packets can destabilize iOS runtimes. Send touch drag input instead.",
        )),
        ControlMessage::ToggleAppearance => state.android.toggle_appearance(&udid),
        ControlMessage::Touch { .. }
        | ControlMessage::EdgeTouch { .. }
        | ControlMessage::MultiTouch { .. } => Ok(()),
    })
    .await
    .map_err(|error| AppError::internal(format!("Failed to join Android control task: {error}")))?
}

async fn handle_android_webrtc_touch(
    state: AppState,
    udid: String,
    x: f64,
    y: f64,
    phase: String,
) -> Result<(), AppError> {
    task::spawn_blocking(move || state.android.send_touch(&udid, x, y, &phase))
        .await
        .map_err(|error| {
            AppError::internal(format!("Failed to join Android touch task: {error}"))
        })?
}

async fn run_webrtc_control_queue(
    session: crate::simulators::session::SimulatorSession,
    state: AppState,
    udid: String,
    _stream_control_tx: mpsc::UnboundedSender<WebRtcStreamCommand>,
    mut receiver: mpsc::UnboundedReceiver<ControlMessage>,
) {
    let mut pending = VecDeque::new();
    let mut tvos_touch = TvosControlTouchGesture::default();
    let mut multitouch_input_session = None;
    loop {
        let mut message = match pending.pop_front() {
            Some(message) => message,
            None => {
                if multitouch_input_session.is_some() {
                    tokio::select! {
                        message = receiver.recv() => match message {
                            Some(message) => message,
                            None => break,
                        },
                        _ = time::sleep(WEBRTC_MULTITOUCH_INPUT_IDLE_TIMEOUT) => {
                            multitouch_input_session = None;
                            continue;
                        }
                    }
                } else {
                    match receiver.recv().await {
                        Some(message) => message,
                        None => break,
                    }
                }
            }
        };
        if webrtc_control_message_is_move(&message) {
            while let Ok(next_message) = receiver.try_recv() {
                if webrtc_control_message_is_move(&next_message) {
                    message = next_message;
                } else {
                    pending.push_back(next_message);
                    break;
                }
            }
        }
        if multitouch_input_session.is_some()
            && !matches!(message, ControlMessage::MultiTouch { .. })
        {
            multitouch_input_session = None;
        }
        match message {
            ControlMessage::ToggleAppearance => {
                let bridge = state.registry.bridge().clone();
                let action_udid = udid.clone();
                let result = run_toggle_appearance_control(bridge, action_udid).await;
                if let Err(error) = result {
                    warn!("WebRTC control message failed for {udid}: {error}");
                }
            }
            message @ ControlMessage::MultiTouch { .. } if !session.is_tvos() => {
                let should_clear_input = webrtc_control_message_ends_touch(&message);
                let bridge = state.registry.bridge().clone();
                let result = match bridge_input_session_for_control(
                    &mut multitouch_input_session,
                    bridge,
                    &udid,
                )
                .await
                {
                    Ok(input) => run_bridge_multitouch_control_message(input, message).await,
                    Err(error) => Err(error),
                };
                if should_clear_input {
                    multitouch_input_session = None;
                }
                if let Err(error) = result {
                    warn!("WebRTC control message failed for {udid}: {error}");
                }
            }
            message => {
                let result = if session.is_tvos() {
                    run_tvos_control_message(session.clone(), message, &mut tvos_touch).await
                } else {
                    run_control_message(session.clone(), message).await
                };
                if let Err(error) = result {
                    warn!("WebRTC control message failed for {udid}: {error}");
                }
            }
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum WebRtcDataChannelMessage {
    ClientStats {
        stats: Box<ClientStreamStats>,
    },
    StreamControl {
        #[serde(rename = "clientId")]
        client_id: Option<String>,
        #[serde(rename = "forceKeyframe")]
        force_keyframe: Option<bool>,
        foreground: Option<bool>,
        snapshot: Option<bool>,
    },
    StreamQuality {
        config: StreamQualityPayload,
    },
}

#[derive(Clone, Debug)]
struct WebRtcStreamCommand {
    force_keyframe: bool,
    snapshot: bool,
}

fn webrtc_control_message_is_move(message: &ControlMessage) -> bool {
    matches!(
        message,
        ControlMessage::Touch { phase, .. }
            | ControlMessage::EdgeTouch { phase, .. }
            if phase == "moved"
    )
}

fn webrtc_control_message_ends_touch(message: &ControlMessage) -> bool {
    matches!(
        message,
        ControlMessage::Touch { phase, .. }
            | ControlMessage::EdgeTouch { phase, .. }
            | ControlMessage::MultiTouch { phase, .. }
            if phase == "ended" || phase == "cancelled"
    )
}

fn is_h264_codec(codec: &str) -> bool {
    let codec = codec.trim().to_ascii_lowercase();
    codec.contains("h264") || codec.starts_with("avc1.") || codec.starts_with("avc3.")
}

fn h264_sdp_fmtp_line(codec: &str, offer_sdp: &str) -> String {
    let codec_profile_level_id = codec
        .split_once('.')
        .map(|(_, value)| value)
        .filter(|value| value.len() >= 6)
        .map(|value| &value[..6])
        .filter(|value| value.chars().all(|ch| ch.is_ascii_hexdigit()))
        .map(|value| value.to_ascii_lowercase());
    let offered_profile_level_ids = offer_h264_profile_level_ids(offer_sdp);
    let profile_level_id = negotiated_h264_profile_level_id(
        codec_profile_level_id.as_deref(),
        &offered_profile_level_ids,
    );
    format!("level-asymmetry-allowed=1;packetization-mode=1;profile-level-id={profile_level_id}")
}

fn negotiated_h264_profile_level_id(
    encoder_profile_level_id: Option<&str>,
    offered_profile_level_ids: &[String],
) -> String {
    if offered_profile_level_ids.is_empty() {
        return encoder_profile_level_id.unwrap_or("42e01f").to_owned();
    }

    if let Some(encoder_profile_level_id) = encoder_profile_level_id {
        if offered_profile_level_ids
            .iter()
            .any(|offered| offered == encoder_profile_level_id)
        {
            return encoder_profile_level_id.to_owned();
        }
        if offered_profile_level_ids
            .iter()
            .any(|offered| h264_profile_level_ids_are_compatible(encoder_profile_level_id, offered))
        {
            return encoder_profile_level_id.to_owned();
        }
    }

    for baseline_profile in ["42e01f", "42001f"] {
        if offered_profile_level_ids
            .iter()
            .any(|offered| offered == baseline_profile)
        {
            return baseline_profile.to_owned();
        }
    }

    offered_profile_level_ids
        .first()
        .cloned()
        .unwrap_or_else(|| "42e01f".to_owned())
}

fn h264_profile_level_ids_are_compatible(encoder: &str, offered: &str) -> bool {
    let encoder = encoder.to_ascii_lowercase();
    let offered = offered.to_ascii_lowercase();
    if encoder.len() < 4 || offered.len() < 4 {
        return false;
    }
    let encoder_profile = &encoder[..2];
    let offered_profile = &offered[..2];
    if encoder_profile != offered_profile {
        return false;
    }
    // Baseline and constrained-baseline offers commonly differ only in constraint
    // bits while level-asymmetry allows a sender to use a higher level.
    if encoder_profile == "42" {
        return true;
    }
    encoder[..4] == offered[..4]
}

fn offer_h264_profile_level_ids(sdp: &str) -> Vec<String> {
    let h264_payload_types = sdp
        .lines()
        .filter_map(|line| line.strip_prefix("a=rtpmap:"))
        .filter_map(|line| {
            let (payload_type, codec) = line.split_once(' ')?;
            codec
                .to_ascii_lowercase()
                .starts_with("h264/")
                .then(|| payload_type.to_owned())
        })
        .collect::<Vec<_>>();
    if h264_payload_types.is_empty() {
        return Vec::new();
    }

    sdp.lines()
        .filter_map(|line| line.strip_prefix("a=fmtp:"))
        .filter_map(|line| {
            let (payload_type, fmtp) = line.split_once(' ')?;
            h264_payload_types
                .iter()
                .any(|candidate| candidate == payload_type)
                .then_some(fmtp)
        })
        .flat_map(|fmtp| fmtp.split(';'))
        .filter_map(|parameter| parameter.trim().split_once('='))
        .filter_map(|(name, value)| {
            if name.eq_ignore_ascii_case("profile-level-id")
                && value.len() >= 6
                && value[..6].chars().all(|ch| ch.is_ascii_hexdigit())
            {
                Some(value[..6].to_ascii_lowercase())
            } else {
                None
            }
        })
        .collect()
}

fn h264_rtcp_feedback() -> Vec<RTCPFeedback> {
    vec![
        RTCPFeedback {
            typ: "goog-remb".to_owned(),
            parameter: String::new(),
        },
        RTCPFeedback {
            typ: "transport-cc".to_owned(),
            parameter: String::new(),
        },
        RTCPFeedback {
            typ: "ccm".to_owned(),
            parameter: "fir".to_owned(),
        },
        RTCPFeedback {
            typ: "nack".to_owned(),
            parameter: String::new(),
        },
        RTCPFeedback {
            typ: "nack".to_owned(),
            parameter: "pli".to_owned(),
        },
    ]
}

fn rtcp_packet_requests_keyframe(packet: &(dyn RtcpPacket + Send + Sync)) -> bool {
    packet.as_any().is::<PictureLossIndication>() || packet.as_any().is::<FullIntraRequest>()
}

fn register_webrtc_media_stream(
    udid: &str,
    client_id: Option<&str>,
    evict_anonymous: bool,
    replace_existing_for_udid: bool,
) -> (broadcast::Sender<()>, broadcast::Receiver<()>) {
    let (tx, rx) = broadcast::channel(1);
    let streams = WEBRTC_MEDIA_STREAMS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut streams = streams.lock().unwrap();
    let active_streams = streams.entry(udid.to_owned()).or_default();
    let client_id = client_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    if replace_existing_for_udid {
        for stream in active_streams.drain(..) {
            let _ = stream.cancellation.send(());
        }
    } else if let Some(client_id) = &client_id {
        active_streams.retain(|stream| {
            let is_same_client = stream.client_id.as_ref() == Some(client_id);
            let is_anonymous = evict_anonymous && stream.client_id.is_none();
            if is_same_client || is_anonymous {
                let _ = stream.cancellation.send(());
                false
            } else {
                true
            }
        });
    }
    while active_streams.len() >= MAX_WEBRTC_MEDIA_STREAMS_PER_UDID {
        if let Some(stale_stream) = active_streams.first().cloned() {
            let _ = stale_stream.cancellation.send(());
        }
        active_streams.remove(0);
    }
    active_streams.push(WebRtcMediaStreamToken {
        client_id,
        cancellation: tx.clone(),
    });
    drop(streams);
    (tx, rx)
}

#[cfg(test)]
fn active_webrtc_media_stream_count(udid: &str) -> usize {
    WEBRTC_MEDIA_STREAMS
        .get()
        .and_then(|streams| streams.lock().ok()?.get(udid).map(Vec::len))
        .unwrap_or(0)
}

#[cfg(test)]
fn register_webrtc_media_stream_for_test(
    udid: &str,
) -> (broadcast::Sender<()>, broadcast::Receiver<()>) {
    register_webrtc_media_stream(udid, None, false, false)
}

#[cfg(test)]
fn register_webrtc_media_stream_for_client_test(
    udid: &str,
    client_id: &str,
) -> (broadcast::Sender<()>, broadcast::Receiver<()>) {
    register_webrtc_media_stream(udid, Some(client_id), false, false)
}

#[cfg(test)]
fn register_webrtc_media_stream_evicting_anonymous_for_test(
    udid: &str,
    client_id: &str,
) -> (broadcast::Sender<()>, broadcast::Receiver<()>) {
    register_webrtc_media_stream(udid, Some(client_id), true, false)
}

#[cfg(test)]
fn register_webrtc_media_stream_replacing_udid_for_test(
    udid: &str,
    client_id: Option<&str>,
) -> (broadcast::Sender<()>, broadcast::Receiver<()>) {
    register_webrtc_media_stream(udid, client_id, true, true)
}

#[cfg(test)]
fn clear_webrtc_media_stream_for_test(udid: &str, token: &broadcast::Sender<()>) {
    clear_webrtc_media_stream(udid, token);
}

#[cfg(test)]
fn reset_webrtc_media_streams_for_test(udid: &str) {
    if let Some(streams) = WEBRTC_MEDIA_STREAMS.get() {
        streams.lock().unwrap().remove(udid);
    }
}

fn clear_webrtc_media_stream(udid: &str, token: &broadcast::Sender<()>) {
    if let Some(streams) = WEBRTC_MEDIA_STREAMS.get() {
        let mut streams = streams.lock().unwrap();
        if let Some(active_streams) = streams.get_mut(udid) {
            active_streams.retain(|current| !current.cancellation.same_channel(token));
            if active_streams.is_empty() {
                streams.remove(udid);
            }
        }
    }
}

#[cfg(test)]
pub fn has_media_stream(udid: &str) -> bool {
    WEBRTC_MEDIA_STREAMS.get().is_some_and(|streams| {
        streams
            .lock()
            .unwrap()
            .get(udid)
            .is_some_and(|streams| !streams.is_empty())
    })
}

pub fn client_ice_servers() -> Vec<ClientIceServer> {
    let mut urls = std::env::var("SIMDECK_WEBRTC_ICE_SERVERS")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_STUN_URL.to_owned())
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if urls.is_empty() {
        urls.push(DEFAULT_STUN_URL.to_owned());
    }
    let username = std::env::var("SIMDECK_WEBRTC_ICE_USERNAME")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let credential = std::env::var("SIMDECK_WEBRTC_ICE_CREDENTIAL")
        .ok()
        .filter(|value| !value.trim().is_empty());
    vec![ClientIceServer {
        urls,
        username,
        credential,
    }]
}

pub fn ice_transport_policy_label() -> String {
    match std::env::var("SIMDECK_WEBRTC_ICE_TRANSPORT_POLICY")
        .ok()
        .as_deref()
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("relay") => "relay".to_owned(),
        _ => "all".to_owned(),
    }
}

fn ice_servers() -> Vec<RTCIceServer> {
    client_ice_servers()
        .into_iter()
        .map(|server| RTCIceServer {
            urls: server.urls,
            username: server.username.unwrap_or_default(),
            credential: server.credential.unwrap_or_default(),
        })
        .collect()
}

fn ice_transport_policy() -> RTCIceTransportPolicy {
    match ice_transport_policy_label().as_str() {
        "relay" => RTCIceTransportPolicy::Relay,
        _ => RTCIceTransportPolicy::All,
    }
}

#[derive(Clone)]
pub(crate) struct AndroidWebRtcSource {
    inner: Arc<AndroidWebRtcSourceInner>,
}

struct AndroidWebRtcSourceInner {
    source_id: u64,
    udid: String,
    video_codec: String,
    shutdown_tx: broadcast::Sender<()>,
    sender: broadcast::Sender<SharedFrame>,
    latest_keyframe: RwLock<Option<SharedFrame>>,
    frame_sequence: AtomicU64,
    quality: Mutex<android::AndroidH264StreamQuality>,
    source_kind: RwLock<&'static str>,
    encoder_handle: AtomicUsize,
    callback_user_data: AtomicUsize,
    shared_frames_read: AtomicU64,
    encode_submissions: AtomicU64,
    encode_failures: AtomicU64,
    latest_shared_frame_copy_us: AtomicU64,
    latest_encode_submit_us: AtomicU64,
    latest_source_frame_gap_us: AtomicU64,
    latest_source_timestamp_us: AtomicU64,
    source_sequence: AtomicU64,
    source_fps: AtomicU64,
    source_width: AtomicU64,
    source_height: AtomicU64,
    output_width: AtomicU64,
    output_height: AtomicU64,
    metrics: Arc<crate::metrics::counters::Metrics>,
}

struct AndroidSourceFrameRecord {
    sequence: u64,
    timestamp_us: u64,
    source_fps: u64,
    source_width: u64,
    source_height: u64,
    output_width: u64,
    output_height: u64,
    copy_duration: Duration,
}

unsafe impl Send for AndroidWebRtcSourceInner {}
unsafe impl Sync for AndroidWebRtcSourceInner {}

impl AndroidWebRtcSource {
    pub(crate) async fn start(
        bridge: android::AndroidBridge,
        metrics: Arc<crate::metrics::counters::Metrics>,
        udid: String,
        config: AndroidH264StreamConfig,
    ) -> Result<Self, AppError> {
        let sources = ANDROID_WEBRTC_SOURCES.get_or_init(|| Mutex::new(Vec::new()));
        let mut sources = sources.lock().unwrap();
        if let Some(inner) = reusable_android_webrtc_source(&mut sources, &udid, &config) {
            return Ok(Self { inner });
        }

        let (sender, _) = broadcast::channel(ANDROID_WEBRTC_FRAME_BROADCAST_CAPACITY);
        let (shutdown_tx, _) = broadcast::channel(1);
        let inner = Arc::new(AndroidWebRtcSourceInner {
            source_id: NEXT_ANDROID_WEBRTC_SOURCE_ID.fetch_add(1, Ordering::Relaxed),
            udid: udid.clone(),
            video_codec: config.video_codec,
            shutdown_tx,
            sender,
            latest_keyframe: RwLock::new(None),
            frame_sequence: AtomicU64::new(0),
            quality: Mutex::new(config.quality),
            source_kind: RwLock::new("shared-video"),
            encoder_handle: AtomicUsize::new(0),
            callback_user_data: AtomicUsize::new(0),
            shared_frames_read: AtomicU64::new(0),
            encode_submissions: AtomicU64::new(0),
            encode_failures: AtomicU64::new(0),
            latest_shared_frame_copy_us: AtomicU64::new(0),
            latest_encode_submit_us: AtomicU64::new(0),
            latest_source_frame_gap_us: AtomicU64::new(0),
            latest_source_timestamp_us: AtomicU64::new(0),
            source_sequence: AtomicU64::new(0),
            source_fps: AtomicU64::new(0),
            source_width: AtomicU64::new(0),
            source_height: AtomicU64::new(0),
            output_width: AtomicU64::new(0),
            output_height: AtomicU64::new(0),
            metrics,
        });

        let source = Self { inner };
        source.inner.create_native_encoder()?;
        sources.push(Arc::downgrade(&source.inner));
        drop(sources);
        spawn_android_shared_video_encoder(bridge, &source.inner);
        Ok(source)
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<SharedFrame> {
        self.inner.sender.subscribe()
    }

    pub(crate) async fn wait_for_keyframe(
        &self,
        timeout_duration: Duration,
    ) -> Option<SharedFrame> {
        let deadline = Instant::now() + timeout_duration;
        let baseline_sequence = self
            .inner
            .latest_keyframe
            .read()
            .unwrap()
            .as_ref()
            .map_or(0, |frame| frame.frame_sequence);
        let mut receiver = self.inner.sender.subscribe();
        self.request_keyframe();

        loop {
            if let Some(frame) = self.inner.latest_keyframe.read().unwrap().clone() {
                if frame.frame_sequence > baseline_sequence {
                    return Some(frame);
                }
            }
            let remaining = deadline.checked_duration_since(Instant::now())?;
            match time::timeout(remaining, receiver.recv()).await {
                Ok(Ok(frame)) if frame.is_keyframe && frame.frame_sequence > baseline_sequence => {
                    return Some(frame)
                }
                Ok(Ok(_)) | Ok(Err(broadcast::error::RecvError::Lagged(_))) => {
                    self.request_keyframe();
                }
                Ok(Err(_)) | Err(_) => return None,
            }
        }
    }

    pub(crate) fn request_refresh(&self) {}

    pub(crate) fn request_keyframe(&self) {
        self.inner.request_keyframe();
    }

    pub(crate) fn reconfigure_h264(&self, quality: android::AndroidH264StreamQuality) {
        self.inner.reconfigure_h264(quality);
    }
}

fn reusable_android_webrtc_source(
    sources: &mut Vec<Weak<AndroidWebRtcSourceInner>>,
    udid: &str,
    config: &AndroidH264StreamConfig,
) -> Option<Arc<AndroidWebRtcSourceInner>> {
    let mut reusable = None;
    sources.retain(|source| {
        let Some(existing) = source.upgrade() else {
            return false;
        };

        if existing.udid != udid {
            return true;
        }

        if reusable.is_none() && existing.video_codec == config.video_codec {
            existing.reconfigure_h264(config.quality);
            reusable = Some(existing);
            return true;
        }

        if existing.source_id != reusable.as_ref().map_or(0, |source| source.source_id) {
            let _ = existing.shutdown_tx.send(());
        }
        false
    });
    reusable
}

pub(crate) fn android_encoder_snapshots() -> Vec<Value> {
    let mut sources = ANDROID_WEBRTC_SOURCES
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap();
    let mut snapshots = Vec::new();
    sources.retain(|source| {
        if let Some(inner) = source.upgrade() {
            snapshots.push(inner.stats_snapshot());
            true
        } else {
            false
        }
    });
    snapshots
}

fn spawn_android_shared_video_encoder(
    bridge: android::AndroidBridge,
    inner: &Arc<AndroidWebRtcSourceInner>,
) {
    let weak_inner = Arc::downgrade(inner);
    let udid = inner.udid.clone();
    let mut shutdown_rx = inner.shutdown_tx.subscribe();
    thread::spawn(move || loop {
        if android_shutdown_requested(&mut shutdown_rx) || weak_inner.upgrade().is_none() {
            break;
        }
        if let Some(grpc_port) = android_grpc_port_for_udid(&bridge, &udid) {
            match TokioRuntimeBuilder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| {
                    AppError::native(format!("Unable to start Android gRPC runtime: {error}"))
                })
                .and_then(|runtime| {
                    runtime.block_on(run_android_grpc_screenshot_encoder(
                        grpc_port,
                        weak_inner.clone(),
                        &mut shutdown_rx,
                    ))
                }) {
                Ok(()) => return,
                Err(error) => {
                    warn!(
                        "Android gRPC screenshot stream failed for {udid} on port {grpc_port}: {error}; falling back to shared video"
                    );
                }
            }
        }
        let mut stream = match bridge.shared_video_frame_stream(&udid) {
            Ok(stream) => stream,
            Err(error) => {
                warn!("Android shared-video stream is not ready for {udid}: {error}");
                thread::sleep(ANDROID_SHARED_VIDEO_RETRY_DELAY);
                continue;
            }
        };
        info!("Android shared-video stream attached for {udid}");
        let poll_interval = android_webrtc_poll_interval();
        let mut next_poll_at = Instant::now();
        let mut next_frame_at = Instant::now();
        let mut next_grpc_probe_at = Instant::now()
            .checked_add(ANDROID_GRPC_FALLBACK_RETRY_INTERVAL)
            .unwrap_or_else(Instant::now);
        loop {
            if android_shutdown_requested(&mut shutdown_rx) {
                return;
            }
            let Some(inner) = weak_inner.upgrade() else {
                return;
            };
            let now = Instant::now();
            if now >= next_grpc_probe_at {
                next_grpc_probe_at = now
                    .checked_add(ANDROID_GRPC_FALLBACK_RETRY_INTERVAL)
                    .unwrap_or(now);
                if let Some(grpc_port) = android_grpc_port_for_udid(&bridge, &udid) {
                    if android_grpc_endpoint_accepts_connections(grpc_port) {
                        info!(
                            "Android gRPC endpoint became available for {udid} on port {grpc_port}; leaving shared-video fallback"
                        );
                        break;
                    }
                }
            }
            let quality = inner.h264_quality();
            let source_read_interval = android_source_read_interval(quality);
            if let Some(interval) = source_read_interval {
                if Instant::now() < next_frame_at {
                    sleep_until_next_android_poll(&mut next_poll_at, poll_interval);
                    continue;
                }
                schedule_next_android_frame(&mut next_frame_at, interval);
            }
            let read_started = Instant::now();
            match stream.next_frame(quality) {
                Ok(Some(frame)) => {
                    inner.record_shared_video_frame(&frame, read_started.elapsed());
                    if let Err(error) = inner.encode_android_shared_video_frame(&frame) {
                        warn!("Android shared-video encode failed for {udid}: {error}");
                        thread::sleep(ANDROID_SHARED_VIDEO_RETRY_DELAY);
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    warn!("Android shared-video frame read failed for {udid}: {error}");
                    break;
                }
            }
            sleep_until_next_android_poll(&mut next_poll_at, poll_interval);
        }
    });
}

fn android_grpc_port_for_udid(bridge: &android::AndroidBridge, udid: &str) -> Option<u16> {
    let avd_name = udid.strip_prefix("android:")?;
    bridge.grpc_port_for_avd(avd_name).ok()
}

fn android_grpc_endpoint_accepts_connections(grpc_port: u16) -> bool {
    let address = SocketAddr::from(([127, 0, 0, 1], grpc_port));
    TcpStream::connect_timeout(&address, ANDROID_GRPC_FALLBACK_PROBE_TIMEOUT).is_ok()
}

async fn run_android_grpc_screenshot_encoder(
    grpc_port: u16,
    weak_inner: Weak<AndroidWebRtcSourceInner>,
    shutdown_rx: &mut broadcast::Receiver<()>,
) -> Result<(), AppError> {
    let Some(inner) = weak_inner.upgrade() else {
        return Ok(());
    };
    inner.set_source_kind("grpc-screenshot");
    let quality = inner.h264_quality();
    let requested_edge = quality.max_edge;
    let request = ImageFormat {
        format: image_format::ImgFormat::Rgba8888 as i32,
        width: requested_edge.unwrap_or(0),
        height: requested_edge.unwrap_or(0),
        ..Default::default()
    };
    let endpoint = format!("http://127.0.0.1:{grpc_port}");
    let mut client = time::timeout(
        Duration::from_secs(2),
        EmulatorControllerClient::connect(endpoint),
    )
    .await
    .map_err(|_| AppError::native("Android emulator gRPC connect timed out."))?
    .map_err(|error| AppError::native(format!("Android emulator gRPC connect failed: {error}")))?;
    let mut stream = client
        .stream_screenshot(request)
        .await
        .map_err(|error| AppError::native(format!("Android emulator gRPC stream failed: {error}")))?
        .into_inner();

    loop {
        if android_shutdown_requested(shutdown_rx) || weak_inner.strong_count() == 0 {
            return Ok(());
        }
        let Some(image) = stream.message().await.map_err(|error| {
            AppError::native(format!("Android emulator gRPC frame failed: {error}"))
        })?
        else {
            return Err(AppError::native(
                "Android emulator gRPC screenshot stream ended.",
            ));
        };
        let Some(inner) = weak_inner.upgrade() else {
            return Ok(());
        };
        if inner.h264_quality().max_edge != requested_edge {
            return Err(AppError::native(
                "Android stream quality changed; reconnecting gRPC screenshot stream.",
            ));
        }
        let read_started = Instant::now();
        let (width, height) = android_grpc_image_dimensions(&image)?;
        if width == 0 || height == 0 || image.image.is_empty() {
            continue;
        }
        let (rgba, output_width, output_height) =
            android_grpc_top_down_even_rgba(&image.image, width, height)?;
        inner.record_android_source_frame(AndroidSourceFrameRecord {
            sequence: u64::from(image.seq),
            timestamp_us: image.timestamp_us,
            source_fps: u64::from(android_h264_target_fps(inner.h264_quality())),
            source_width: u64::from(width),
            source_height: u64::from(height),
            output_width: u64::from(output_width),
            output_height: u64::from(output_height),
            copy_duration: read_started.elapsed(),
        });
        if let Err(error) =
            inner.encode_android_rgba_frame(&rgba, output_width, output_height, image.timestamp_us)
        {
            warn!(
                "Android gRPC screenshot encode failed for {}: {error}",
                inner.udid
            );
            time::sleep(ANDROID_SHARED_VIDEO_RETRY_DELAY).await;
        }
    }
}

fn android_grpc_image_dimensions(
    image: &crate::android_emulation_control::Image,
) -> Result<(u32, u32), AppError> {
    let (width, height) = image
        .format
        .as_ref()
        .map(|format| (format.width, format.height))
        .unwrap_or((image.width, image.height));
    if width == 0 || height == 0 {
        return Ok((0, 0));
    }
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| AppError::native("Android gRPC screenshot frame size overflowed."))?;
    if pixels > 256 * 1024 * 1024 {
        return Err(AppError::native(
            "Android gRPC screenshot frame size was outside supported bounds.",
        ));
    }
    Ok((width, height))
}

fn android_grpc_top_down_even_rgba(
    source_rgba: &[u8],
    width: u32,
    height: u32,
) -> Result<(Vec<u8>, u32, u32), AppError> {
    let row_bytes = usize::try_from(width)
        .ok()
        .and_then(|width| width.checked_mul(4))
        .ok_or_else(|| AppError::native("Android gRPC screenshot row size overflowed."))?;
    let expected_len = row_bytes
        .checked_mul(usize::try_from(height).unwrap_or(usize::MAX))
        .ok_or_else(|| AppError::native("Android gRPC screenshot frame size overflowed."))?;
    if source_rgba.len() != expected_len {
        return Err(AppError::native(format!(
            "Android gRPC screenshot frame size mismatch: got {}, expected {expected_len}.",
            source_rgba.len()
        )));
    }
    let output_width = if width.is_multiple_of(2) {
        width
    } else {
        width + 1
    };
    let output_height = if height.is_multiple_of(2) {
        height
    } else {
        height + 1
    };
    let output_row_bytes = usize::try_from(output_width)
        .ok()
        .and_then(|width| width.checked_mul(4))
        .ok_or_else(|| AppError::native("Android gRPC screenshot output row size overflowed."))?;
    let output_len = output_row_bytes
        .checked_mul(usize::try_from(output_height).unwrap_or(usize::MAX))
        .ok_or_else(|| AppError::native("Android gRPC screenshot output frame size overflowed."))?;
    let mut top_down = vec![0u8; output_len];
    let height = usize::try_from(height)
        .map_err(|_| AppError::native("Android gRPC screenshot height overflowed."))?;
    for y in 0..height {
        let source = y * row_bytes;
        let target = y * output_row_bytes;
        top_down[target..target + row_bytes]
            .copy_from_slice(&source_rgba[source..source + row_bytes]);
        if output_width != width {
            let last_pixel = target + row_bytes - 4;
            let padded_pixel = target + row_bytes;
            top_down.copy_within(last_pixel..last_pixel + 4, padded_pixel);
        }
    }
    if output_height != u32::try_from(height).unwrap_or(u32::MAX) {
        let last_row = (height - 1) * output_row_bytes;
        let padded_row = height * output_row_bytes;
        top_down.copy_within(last_row..last_row + output_row_bytes, padded_row);
    }
    Ok((top_down, output_width, output_height))
}

fn android_shutdown_requested(receiver: &mut broadcast::Receiver<()>) -> bool {
    !matches!(
        receiver.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    )
}

impl Drop for AndroidWebRtcSourceInner {
    fn drop(&mut self) {
        let _ = self.shutdown_tx.send(());
        let handle = self.encoder_handle.swap(0, Ordering::AcqRel);
        if handle != 0 {
            unsafe {
                ffi::xcw_native_h264_encoder_destroy(handle as *mut c_void);
            }
        }
        let user_data = self.callback_user_data.swap(0, Ordering::AcqRel);
        if user_data != 0 {
            unsafe {
                let _ = Weak::from_raw(user_data as *const AndroidWebRtcSourceInner);
            }
        }
    }
}

impl AndroidWebRtcSourceInner {
    fn create_native_encoder(self: &Arc<Self>) -> Result<(), AppError> {
        let weak = Arc::downgrade(self);
        let user_data = Weak::into_raw(weak) as *mut c_void;
        let video_codec = CString::new(self.video_codec.as_str())
            .map_err(|_| AppError::bad_request("Android video codec contains a NUL byte."))?;
        let mut error = ptr::null_mut();
        let handle = unsafe {
            ffi::xcw_native_h264_encoder_create_with_video_codec(
                Some(android_h264_encoder_frame_callback),
                user_data,
                video_codec.as_ptr(),
                &mut error,
            )
        };
        if handle.is_null() {
            unsafe {
                let _ = Weak::from_raw(user_data as *const AndroidWebRtcSourceInner);
            }
            return Err(unsafe {
                take_native_error(error, "Unable to create Android native H.264 encoder.")
            });
        }
        self.callback_user_data
            .store(user_data as usize, Ordering::Release);
        self.encoder_handle
            .store(handle as usize, Ordering::Release);
        Ok(())
    }

    fn h264_quality(&self) -> android::AndroidH264StreamQuality {
        *self.quality.lock().unwrap()
    }

    fn record_shared_video_frame(
        &self,
        frame: &android::AndroidSharedVideoFrame,
        copy_duration: Duration,
    ) {
        self.set_source_kind("shared-video");
        self.record_android_source_frame(AndroidSourceFrameRecord {
            sequence: u64::from(frame.source_sequence),
            timestamp_us: frame.timestamp_us,
            source_fps: u64::from(frame.source_fps),
            source_width: u64::from(frame.source_width),
            source_height: u64::from(frame.source_height),
            output_width: u64::from(frame.width),
            output_height: u64::from(frame.height),
            copy_duration,
        });
    }

    fn set_source_kind(&self, source_kind: &'static str) {
        *self.source_kind.write().unwrap() = source_kind;
    }

    fn record_android_source_frame(&self, record: AndroidSourceFrameRecord) {
        self.shared_frames_read.fetch_add(1, Ordering::Relaxed);
        self.latest_shared_frame_copy_us
            .store(duration_us(record.copy_duration), Ordering::Relaxed);
        let previous_timestamp = self
            .latest_source_timestamp_us
            .swap(record.timestamp_us, Ordering::Relaxed);
        if previous_timestamp != 0 && record.timestamp_us > previous_timestamp {
            self.latest_source_frame_gap_us
                .store(record.timestamp_us - previous_timestamp, Ordering::Relaxed);
        }
        self.source_sequence
            .store(record.sequence, Ordering::Relaxed);
        self.source_fps.store(record.source_fps, Ordering::Relaxed);
        self.source_width
            .store(record.source_width, Ordering::Relaxed);
        self.source_height
            .store(record.source_height, Ordering::Relaxed);
        self.output_width
            .store(record.output_width, Ordering::Relaxed);
        self.output_height
            .store(record.output_height, Ordering::Relaxed);
    }

    fn reconfigure_h264(&self, quality: android::AndroidH264StreamQuality) {
        let mut current = self.quality.lock().unwrap();
        if *current != quality {
            *current = quality;
            *self.latest_keyframe.write().unwrap() = None;
        }
        drop(current);
        self.request_keyframe();
    }

    fn request_keyframe(&self) {
        self.metrics
            .keyframe_requests
            .fetch_add(1, Ordering::Relaxed);
        let handle = self.encoder_handle.load(Ordering::Acquire);
        if handle != 0 {
            unsafe {
                ffi::xcw_native_h264_encoder_request_keyframe(handle as *mut c_void);
            }
        }
    }

    fn stats_snapshot(&self) -> Value {
        let quality = self.h264_quality();
        json!({
            "sourceId": self.source_id,
            "udid": self.udid.as_str(),
            "quality": {
                "maxEdge": quality.max_edge,
                "fps": quality.fps,
                "minBitrate": quality.min_bitrate,
                "bitsPerPixel": quality.bits_per_pixel,
                "videoCodec": self.video_codec.as_str(),
            },
            "sharedVideo": {
                "sourceKind": *self.source_kind.read().unwrap(),
                "framesRead": self.shared_frames_read.load(Ordering::Relaxed),
                "sourceSequence": self.source_sequence.load(Ordering::Relaxed),
                "sourceFps": self.source_fps.load(Ordering::Relaxed),
                "sourceWidth": self.source_width.load(Ordering::Relaxed),
                "sourceHeight": self.source_height.load(Ordering::Relaxed),
                "outputWidth": self.output_width.load(Ordering::Relaxed),
                "outputHeight": self.output_height.load(Ordering::Relaxed),
                "latestSourceTimestampUs": self.latest_source_timestamp_us.load(Ordering::Relaxed),
                "latestSourceFrameGapMs": micros_to_millis_value(self.latest_source_frame_gap_us.load(Ordering::Relaxed)),
                "latestFrameCopyMs": micros_to_millis_value(self.latest_shared_frame_copy_us.load(Ordering::Relaxed)),
            },
            "encoder": {
                "submissions": self.encode_submissions.load(Ordering::Relaxed),
                "failures": self.encode_failures.load(Ordering::Relaxed),
                "latestSubmitMs": micros_to_millis_value(self.latest_encode_submit_us.load(Ordering::Relaxed)),
                "native": self.native_encoder_stats(),
            },
        })
    }

    fn native_encoder_stats(&self) -> Value {
        let handle = self.encoder_handle.load(Ordering::Acquire);
        if handle == 0 {
            return json!({});
        }
        unsafe {
            let mut error = ptr::null_mut();
            let raw = ffi::xcw_native_h264_encoder_stats(handle as *mut c_void, &mut error);
            if !error.is_null() {
                ffi::xcw_native_free_string(error);
            }
            take_native_string(raw)
                .and_then(|json| serde_json::from_str(&json).ok())
                .unwrap_or_else(|| json!({}))
        }
    }

    fn encode_android_shared_video_frame(
        &self,
        frame: &android::AndroidSharedVideoFrame,
    ) -> Result<(), AppError> {
        let handle = self.encoder_handle.load(Ordering::Acquire);
        if handle == 0 {
            return Err(AppError::native(
                "Android native H.264 encoder is not available.",
            ));
        }
        let mut error = ptr::null_mut();
        let submit_started = Instant::now();
        self.encode_submissions.fetch_add(1, Ordering::Relaxed);
        let ok = unsafe {
            ffi::xcw_native_h264_encoder_encode_bgra(
                handle as *mut c_void,
                frame.bgra.as_ptr(),
                frame.bgra.len(),
                frame.width,
                frame.height,
                frame.timestamp_us,
                &mut error,
            )
        };
        self.latest_encode_submit_us
            .store(duration_us(submit_started.elapsed()), Ordering::Relaxed);
        if !ok {
            self.encode_failures.fetch_add(1, Ordering::Relaxed);
            return Err(unsafe { take_native_error(error, "Android native H.264 encode failed.") });
        }
        Ok(())
    }

    fn encode_android_rgba_frame(
        &self,
        rgba: &[u8],
        width: u32,
        height: u32,
        timestamp_us: u64,
    ) -> Result<(), AppError> {
        let handle = self.encoder_handle.load(Ordering::Acquire);
        if handle == 0 {
            return Err(AppError::native(
                "Android native H.264 encoder is not available.",
            ));
        }
        let mut error = ptr::null_mut();
        let submit_started = Instant::now();
        self.encode_submissions.fetch_add(1, Ordering::Relaxed);
        let ok = unsafe {
            ffi::xcw_native_h264_encoder_encode_rgba(
                handle as *mut c_void,
                rgba.as_ptr(),
                rgba.len(),
                width,
                height,
                timestamp_us,
                &mut error,
            )
        };
        self.latest_encode_submit_us
            .store(duration_us(submit_started.elapsed()), Ordering::Relaxed);
        if !ok {
            self.encode_failures.fetch_add(1, Ordering::Relaxed);
            return Err(unsafe { take_native_error(error, "Android native H.264 encode failed.") });
        }
        Ok(())
    }

    fn handle_encoded_frame(&self, frame: &ffi::xcw_native_frame) {
        let Some(data) = (unsafe { copy_native_shared_bytes(frame.data) }) else {
            return;
        };
        let description = unsafe { copy_native_shared_bytes(frame.description) };
        let codec = unsafe { native_c_string(frame.codec) };
        let frame_sequence = self.frame_sequence.fetch_add(1, Ordering::Relaxed) + 1;
        let packet = Arc::new(FramePacket {
            frame_sequence,
            timestamp_us: frame.timestamp_us,
            is_keyframe: frame.is_keyframe,
            width: frame.width,
            height: frame.height,
            codec,
            description,
            data,
        });
        self.metrics.frames_encoded.fetch_add(1, Ordering::Relaxed);
        if packet.is_keyframe {
            self.metrics
                .keyframes_encoded
                .fetch_add(1, Ordering::Relaxed);
            *self.latest_keyframe.write().unwrap() = Some(packet.clone());
        }
        let _ = self.sender.send(packet);
    }
}

unsafe extern "C" fn android_h264_encoder_frame_callback(
    frame: *const ffi::xcw_native_frame,
    user_data: *mut c_void,
) {
    if frame.is_null() || user_data.is_null() {
        return;
    }

    let weak = Weak::from_raw(user_data as *const AndroidWebRtcSourceInner);
    if let Some(inner) = weak.upgrade() {
        inner.handle_encoded_frame(&*frame);
    }
    let _ = Weak::into_raw(weak);
}

unsafe fn copy_native_shared_bytes(bytes: ffi::xcw_native_shared_bytes) -> Option<Bytes> {
    let copied = if bytes.data.is_null() || bytes.length == 0 {
        None
    } else {
        Some(Bytes::copy_from_slice(slice::from_raw_parts(
            bytes.data,
            bytes.length,
        )))
    };
    ffi::xcw_native_release_shared_bytes(bytes);
    copied
}

unsafe fn native_c_string(value: *const i8) -> Option<String> {
    if value.is_null() {
        return None;
    }
    CStr::from_ptr(value).to_str().ok().map(ToOwned::to_owned)
}

unsafe fn take_native_string(value: *mut i8) -> Option<String> {
    if value.is_null() {
        return None;
    }
    let string = CStr::from_ptr(value).to_str().ok().map(ToOwned::to_owned);
    ffi::xcw_native_free_string(value);
    string
}

unsafe fn take_native_error(error: *mut i8, fallback: &str) -> AppError {
    if error.is_null() {
        return AppError::native(fallback);
    }
    let message = CStr::from_ptr(error)
        .to_str()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|_| fallback.to_owned());
    ffi::xcw_native_free_string(error);
    AppError::native(message)
}

fn duration_us(duration: Duration) -> u64 {
    duration.as_micros().min(u128::from(u64::MAX)) as u64
}

fn micros_to_millis_value(micros: u64) -> Option<f64> {
    (micros > 0).then_some(micros as f64 / 1000.0)
}

fn android_webrtc_poll_interval() -> Duration {
    let fps = std::env::var("SIMDECK_ANDROID_SHARED_VIDEO_POLL_FPS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(ANDROID_WEBRTC_DEFAULT_POLL_FPS)
        .clamp(60, u64::from(WEBRTC_MAX_LOCAL_STREAM_FPS));
    Duration::from_micros(1_000_000 / fps)
}

fn android_h264_target_fps(quality: android::AndroidH264StreamQuality) -> u32 {
    quality
        .fps
        .unwrap_or(ANDROID_WEBRTC_DEFAULT_FPS)
        .clamp(10, WEBRTC_MAX_LOCAL_STREAM_FPS)
}

fn android_source_read_interval(quality: android::AndroidH264StreamQuality) -> Option<Duration> {
    let fps = android_h264_target_fps(quality);
    if fps >= ANDROID_WEBRTC_DEFAULT_FPS {
        return None;
    }
    Some(android_h264_frame_interval(quality))
}

fn android_h264_frame_interval(quality: android::AndroidH264StreamQuality) -> Duration {
    let fps = android_h264_target_fps(quality);
    Duration::from_micros(1_000_000 / u64::from(fps))
}

fn schedule_next_android_frame(next_frame_at: &mut Instant, interval: Duration) {
    let target = next_frame_at
        .checked_add(interval)
        .unwrap_or_else(Instant::now);
    let now = Instant::now();
    *next_frame_at = if target > now { target } else { now };
}

fn sleep_until_next_android_poll(next_poll_at: &mut Instant, interval: Duration) {
    let target = next_poll_at
        .checked_add(interval)
        .unwrap_or_else(Instant::now);
    *next_poll_at = target;
    let now = Instant::now();
    if target > now {
        thread::sleep(target - now);
    } else {
        *next_poll_at = now;
    }
}

#[derive(Clone)]
enum WebRtcVideoSource {
    Simulator(crate::simulators::session::SimulatorSession),
    Android(AndroidWebRtcSource),
}

impl WebRtcVideoSource {
    fn uses_frame_timestamps_for_realtime(&self) -> bool {
        false
    }

    fn subscribe(&self) -> WebRtcFrameReceiver {
        match self {
            Self::Simulator(session) => WebRtcFrameReceiver::Simulator(session.subscribe()),
            Self::Android(source) => WebRtcFrameReceiver::Android(source.subscribe()),
        }
    }

    async fn wait_for_keyframe(&self, timeout_duration: Duration) -> Option<SharedFrame> {
        match self {
            Self::Simulator(session) => session.wait_for_keyframe(timeout_duration).await,
            Self::Android(source) => source.wait_for_keyframe(timeout_duration).await,
        }
    }

    fn request_refresh(&self) {
        match self {
            Self::Simulator(session) => session.request_refresh(),
            Self::Android(source) => source.request_refresh(),
        }
    }

    fn request_keyframe(&self) {
        match self {
            Self::Simulator(session) => session.request_keyframe(),
            Self::Android(source) => source.request_keyframe(),
        }
    }

    fn set_client_foreground(&self, foreground: bool) {
        if let Self::Simulator(session) = self {
            session.set_client_foreground(foreground);
        }
    }
}

enum WebRtcFrameReceiver {
    Simulator(crate::simulators::session::FrameSubscription),
    Android(broadcast::Receiver<SharedFrame>),
}

impl WebRtcFrameReceiver {
    async fn recv(&mut self) -> Result<SharedFrame, broadcast::error::RecvError> {
        match self {
            Self::Simulator(receiver) => receiver.recv().await,
            Self::Android(receiver) => receiver.recv().await,
        }
    }
}

async fn wait_for_h264_sync_keyframe(
    source: &WebRtcVideoSource,
    timeout_duration: Duration,
) -> Option<SharedFrame> {
    let deadline = time::Instant::now() + timeout_duration;
    loop {
        let remaining = deadline.checked_duration_since(time::Instant::now())?;
        let frame = source.wait_for_keyframe(remaining).await?;
        if h264_frame_is_decoder_sync(&frame) {
            return Some(frame);
        }
        source.request_keyframe();
    }
}

struct WebRtcMediaStream {
    state: AppState,
    source: WebRtcVideoSource,
    udid: String,
    client_id: Option<String>,
    first_frame: SharedFrame,
    peer_connection: Arc<webrtc::peer_connection::RTCPeerConnection>,
    video_track: Arc<TrackLocalStaticRTP>,
    cancellation_token: broadcast::Sender<()>,
    cancellation: broadcast::Receiver<()>,
    stream_control_rx: mpsc::UnboundedReceiver<WebRtcStreamCommand>,
}

impl WebRtcMediaStream {
    async fn run(self) {
        let Self {
            state,
            source,
            udid,
            client_id,
            first_frame,
            peer_connection,
            video_track,
            cancellation_token,
            mut cancellation,
            mut stream_control_rx,
        } = self;
        let mut rx = source.subscribe();
        let mut send_timing = WebRtcSendTiming::new();
        let mut peer_state_interval = time::interval(Duration::from_millis(250));
        let realtime_stream = realtime_stream_enabled();
        let mut packetizer = new_packetizer(
            WEBRTC_RTP_OUTBOUND_MTU,
            96,
            0,
            Box::<H264Payloader>::default(),
            Box::new(new_random_sequencer()),
            90_000,
        );
        let mut waiting_for_keyframe = false;
        let mut peer_disconnected_since: Option<time::Instant> = None;
        let _guard = WebRtcMetricsGuard::new(state.metrics.clone());
        let uses_frame_timestamps_for_realtime = source.uses_frame_timestamps_for_realtime();
        let first_frame_duration = send_timing.duration_for(
            &first_frame,
            realtime_stream,
            uses_frame_timestamps_for_realtime,
        );

        match write_frame_sample_with_timeout(
            &video_track,
            &mut packetizer,
            &first_frame,
            first_frame_duration,
            realtime_stream,
        )
        .await
        {
            Ok(true) => {
                state.metrics.frames_sent.fetch_add(1, Ordering::Relaxed);
            }
            Ok(false) => {
                state
                    .metrics
                    .frames_dropped_server
                    .fetch_add(1, Ordering::Relaxed);
                if recovery_action_for_write_timeout(realtime_stream)
                    == FrameRecoveryAction::Refresh
                {
                    source.request_refresh();
                } else {
                    waiting_for_keyframe = true;
                    source.request_keyframe();
                }
            }
            Err(error) => {
                warn!("WebRTC initial keyframe write failed for {udid}: {error}");
                let _ = peer_connection.close().await;
                return;
            }
        }

        loop {
            tokio::select! {
                _ = cancellation.recv() => {
                    warn!("WebRTC media stream replaced for {udid}");
                    break;
                }
                _ = peer_state_interval.tick() => {
                    let peer_state = peer_connection.connection_state();
                    if matches!(peer_state, RTCPeerConnectionState::Closed | RTCPeerConnectionState::Failed) {
                        warn!("WebRTC media stream closing for {udid}: peer state {peer_state}");
                        break;
                    }
                    if peer_state == RTCPeerConnectionState::Disconnected {
                        let disconnected_since =
                            peer_disconnected_since.get_or_insert_with(time::Instant::now);
                        if disconnected_since.elapsed() >= WEBRTC_PEER_DISCONNECTED_TIMEOUT {
                            warn!("WebRTC media stream closing for {udid}: peer state {peer_state}");
                            break;
                        }
                    } else {
                        peer_disconnected_since = None;
                    }
                }
                command = stream_control_rx.recv() => {
                    let Some(command) = command else {
                        continue;
                    };
                    if command.force_keyframe || command.snapshot {
                        waiting_for_keyframe = true;
                        source.request_keyframe();
                    } else {
                        source.request_refresh();
                    }
                }
                frame = rx.recv() => {
                    let frame = match frame {
                        Ok(frame) => frame,
                        Err(broadcast::error::RecvError::Lagged(skipped)) => {
                            state
                                .metrics
                                .frames_dropped_server
                                .fetch_add(skipped, Ordering::Relaxed);
                            waiting_for_keyframe = true;
                            source.request_keyframe();
                            continue;
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            warn!("WebRTC media stream closing for {udid}: frame channel closed");
                            break;
                        }
                    };
                    if waiting_for_keyframe && !frame.is_keyframe {
                        state.metrics.frames_dropped_server.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                    if h264_frame_is_decoder_sync(&frame) {
                        waiting_for_keyframe = false;
                    } else if frame.is_keyframe {
                        waiting_for_keyframe = true;
                        source.request_keyframe();
                        state.metrics.frames_dropped_server.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                    let duration = send_timing.duration_for(
                        &frame,
                        realtime_stream,
                        uses_frame_timestamps_for_realtime,
                    );
                    let write_result = write_frame_sample_with_timeout(
                        &video_track,
                        &mut packetizer,
                        &frame,
                        duration,
                        realtime_stream,
                    )
                    .await;
                    match write_result {
                        Ok(true) => {
                            state.metrics.frames_sent.fetch_add(1, Ordering::Relaxed);
                        }
                        Ok(false) => {
                            state
                                .metrics
                                .frames_dropped_server
                                .fetch_add(1, Ordering::Relaxed);
                            let recovery_action = recovery_action_for_write_timeout(realtime_stream);
                            waiting_for_keyframe = recovery_action == FrameRecoveryAction::Keyframe;
                            if recovery_action == FrameRecoveryAction::Refresh {
                                source.request_refresh();
                            } else {
                                source.request_keyframe();
                            }
                        }
                        Err(error) => {
                            warn!("WebRTC frame write failed for {udid}: {error}");
                            break;
                        }
                    }
                }
            }
        }

        warn!("WebRTC media stream ended for {udid}");
        clear_webrtc_media_stream(&udid, &cancellation_token);
        remove_stream_client_foreground(&state, &source, &udid, &client_id);
        let _ = peer_connection.close().await;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FrameRecoveryAction {
    Refresh,
    Keyframe,
}

fn recovery_action_for_write_timeout(realtime_stream: bool) -> FrameRecoveryAction {
    if realtime_stream {
        FrameRecoveryAction::Refresh
    } else {
        FrameRecoveryAction::Keyframe
    }
}

async fn write_frame_sample<P: Packetizer>(
    video_track: &TrackLocalStaticRTP,
    packetizer: &mut P,
    frame: &crate::transport::packet::SharedFrame,
    duration: Duration,
    realtime_stream: bool,
) -> anyhow::Result<()> {
    let data = h264_annex_b_sample(frame)?;
    let samples = (duration.as_secs_f64() * 90_000.0).max(1.0) as u32;
    let packets = packetizer.packetize(&Bytes::from(data), samples)?;
    let packet_count = packets.len();
    let pacing = rtp_packet_pacing(duration, packet_count, realtime_stream);
    for (index, packet) in packets.into_iter().enumerate() {
        video_track.write_rtp(&packet).await?;
        if let Some((batch_size, delay)) = pacing.filter(|_| index + 1 < packet_count) {
            if (index + 1) % batch_size == 0 {
                time::sleep(delay).await;
            }
        }
    }
    Ok(())
}

async fn write_frame_sample_with_timeout<P: Packetizer>(
    video_track: &TrackLocalStaticRTP,
    packetizer: &mut P,
    frame: &crate::transport::packet::SharedFrame,
    duration: Duration,
    realtime_stream: bool,
) -> anyhow::Result<bool> {
    let slow_write_threshold = if realtime_stream {
        if frame.is_keyframe {
            WEBRTC_REALTIME_KEYFRAME_WRITE_TIMEOUT
        } else {
            WEBRTC_REALTIME_WRITE_TIMEOUT.max(realtime_sample_duration() * 2)
        }
    } else {
        WEBRTC_WRITE_TIMEOUT
    };
    let started_at = time::Instant::now();
    let write_result = time::timeout(
        slow_write_threshold,
        write_frame_sample(video_track, packetizer, frame, duration, realtime_stream),
    )
    .await;
    let elapsed = started_at.elapsed();

    match write_result {
        Ok(Ok(())) => Ok(true),
        Ok(Err(error)) => Err(error),
        Err(_) => {
            warn!(
                "WebRTC frame write timed out: elapsed_ms={} threshold_ms={} keyframe={} realtime={}",
                elapsed.as_millis(),
                slow_write_threshold.as_millis(),
                frame.is_keyframe,
                realtime_stream
            );
            Ok(false)
        }
    }
}

fn rtp_packet_pacing(
    duration: Duration,
    packet_count: usize,
    realtime_stream: bool,
) -> Option<(usize, Duration)> {
    if realtime_stream {
        return None;
    }
    let min_paced_packets = if realtime_stream { 2 } else { 12 };
    if packet_count < min_paced_packets {
        return None;
    }
    let pacing_ticks = ((duration.as_millis() / 4).max(1) as usize).min(packet_count - 1);
    let batch_size = packet_count.div_ceil(pacing_ticks).max(1);
    let delay =
        (duration / pacing_ticks as u32).clamp(Duration::from_millis(1), Duration::from_millis(5));
    Some((batch_size, delay))
}

pub fn realtime_stream_enabled() -> bool {
    std::env::var("SIMDECK_REALTIME_STREAM")
        .ok()
        .is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
}

pub fn h264_annex_b_sample(
    frame: &crate::transport::packet::FramePacket,
) -> anyhow::Result<Vec<u8>> {
    let data = frame.data.as_ref();
    let description = frame.description.as_ref().map(bytes::Bytes::as_ref);
    let mut sample = Vec::with_capacity(data.len() + description.map_or(0, |bytes| bytes.len()));

    if frame.is_keyframe {
        if let Some(avcc) = description {
            append_avcc_parameter_sets(avcc, &mut sample)?;
        }
    }

    if is_annex_b(data) {
        sample.extend_from_slice(data);
        return Ok(sample);
    }

    let sample_len_before_data = sample.len();
    let preferred_nal_length_size = description.and_then(avcc_nal_length_size);
    let fallback_nal_length_size = detect_length_prefixed_nal_length_size(data);
    let nal_length_size = preferred_nal_length_size
        .or(fallback_nal_length_size)
        .unwrap_or(4);
    if let Err(error) = append_length_prefixed_nalus(data, nal_length_size, &mut sample) {
        let Some(fallback_nal_length_size) = fallback_nal_length_size else {
            return Err(error);
        };
        if Some(fallback_nal_length_size) == preferred_nal_length_size {
            return Err(error);
        }
        sample.truncate(sample_len_before_data);
        append_length_prefixed_nalus(data, fallback_nal_length_size, &mut sample)?;
    }
    Ok(sample)
}

fn h264_frame_has_idr(frame: &crate::transport::packet::FramePacket) -> bool {
    let data = frame.data.as_ref();
    if is_annex_b(data) {
        return annex_b_sample_has_idr(data);
    }
    let preferred_nal_length_size = frame
        .description
        .as_ref()
        .and_then(|description| avcc_nal_length_size(description.as_ref()));
    if preferred_nal_length_size
        .map(|nal_length_size| length_prefixed_sample_has_idr(data, nal_length_size))
        .unwrap_or(false)
    {
        return true;
    }
    detect_length_prefixed_nal_length_size(data)
        .map(|nal_length_size| length_prefixed_sample_has_idr(data, nal_length_size))
        .unwrap_or(false)
}

fn h264_frame_is_decoder_sync(frame: &crate::transport::packet::FramePacket) -> bool {
    frame.is_keyframe && h264_frame_has_idr(frame)
}

fn annex_b_sample_has_idr(data: &[u8]) -> bool {
    let mut offset = 0usize;
    while let Some((nal_start, nal_end)) = next_annex_b_nal_range(data, offset) {
        if h264_nal_type(data[nal_start]) == 5 {
            return true;
        }
        offset = nal_end;
    }
    false
}

fn next_annex_b_nal_range(data: &[u8], offset: usize) -> Option<(usize, usize)> {
    let (start_code, start_code_len) = find_annex_b_start_code(data, offset)?;
    let nal_start = start_code + start_code_len;
    if nal_start >= data.len() {
        return None;
    }
    let nal_end = find_annex_b_start_code(data, nal_start)
        .map(|(next_start, _)| next_start)
        .unwrap_or(data.len());
    Some((nal_start, nal_end))
}

fn find_annex_b_start_code(data: &[u8], offset: usize) -> Option<(usize, usize)> {
    let mut index = offset;
    while index + 3 <= data.len() {
        if data[index..].starts_with(ANNEX_B_START_CODE) {
            return Some((index, ANNEX_B_START_CODE.len()));
        }
        if data[index..].starts_with(&[0, 0, 1]) {
            return Some((index, 3));
        }
        index += 1;
    }
    None
}

fn length_prefixed_sample_has_idr(data: &[u8], nal_length_size: usize) -> bool {
    if !(1..=4).contains(&nal_length_size) {
        return false;
    }
    let mut offset = 0usize;
    while offset + nal_length_size <= data.len() {
        let mut length = 0usize;
        for byte in &data[offset..offset + nal_length_size] {
            length = (length << 8) | (*byte as usize);
        }
        offset += nal_length_size;
        if length == 0 {
            continue;
        }
        if offset + length > data.len() {
            return false;
        }
        if h264_nal_type(data[offset]) == 5 {
            return true;
        }
        offset += length;
    }
    false
}

fn detect_length_prefixed_nal_length_size(data: &[u8]) -> Option<usize> {
    [4usize, 2, 1, 3]
        .into_iter()
        .find(|nal_length_size| length_prefixed_sample_is_well_formed(data, *nal_length_size))
}

fn length_prefixed_sample_is_well_formed(data: &[u8], nal_length_size: usize) -> bool {
    if data.is_empty() || !(1..=4).contains(&nal_length_size) {
        return false;
    }
    let mut offset = 0usize;
    let mut nal_count = 0usize;
    while offset + nal_length_size <= data.len() {
        let mut length = 0usize;
        for byte in &data[offset..offset + nal_length_size] {
            length = (length << 8) | (*byte as usize);
        }
        offset += nal_length_size;
        if length == 0 || offset + length > data.len() {
            return false;
        }
        if !is_plausible_h264_nal_type(h264_nal_type(data[offset])) {
            return false;
        }
        offset += length;
        nal_count += 1;
    }
    offset == data.len() && nal_count > 0
}

fn h264_nal_type(header: u8) -> u8 {
    header & 0x1f
}

fn is_plausible_h264_nal_type(nal_type: u8) -> bool {
    (1..=23).contains(&nal_type)
}

fn is_annex_b(data: &[u8]) -> bool {
    data.starts_with(&[0, 0, 1]) || data.starts_with(ANNEX_B_START_CODE)
}

fn avcc_nal_length_size(avcc: &[u8]) -> Option<usize> {
    if avcc.len() < 5 {
        return None;
    }
    Some(((avcc[4] & 0x03) + 1) as usize)
}

fn append_avcc_parameter_sets(avcc: &[u8], output: &mut Vec<u8>) -> anyhow::Result<()> {
    if avcc.len() < 7 {
        return Ok(());
    }

    let sps_count = (avcc[5] & 0x1f) as usize;
    let mut offset = 6usize;
    for _ in 0..sps_count {
        append_avcc_nal(avcc, &mut offset, output)?;
    }

    if offset >= avcc.len() {
        return Ok(());
    }

    let pps_count = avcc[offset] as usize;
    offset += 1;
    for _ in 0..pps_count {
        append_avcc_nal(avcc, &mut offset, output)?;
    }
    Ok(())
}

fn append_avcc_nal(avcc: &[u8], offset: &mut usize, output: &mut Vec<u8>) -> anyhow::Result<()> {
    if *offset + 2 > avcc.len() {
        anyhow::bail!("truncated H.264 decoder configuration record");
    }
    let length = u16::from_be_bytes([avcc[*offset], avcc[*offset + 1]]) as usize;
    *offset += 2;
    if *offset + length > avcc.len() {
        anyhow::bail!("truncated H.264 decoder configuration NAL unit");
    }
    if length > 0 {
        output.extend_from_slice(ANNEX_B_START_CODE);
        output.extend_from_slice(&avcc[*offset..*offset + length]);
    }
    *offset += length;
    Ok(())
}

fn append_length_prefixed_nalus(
    data: &[u8],
    nal_length_size: usize,
    output: &mut Vec<u8>,
) -> anyhow::Result<()> {
    if !(1..=4).contains(&nal_length_size) {
        anyhow::bail!("invalid NAL length size {nal_length_size}");
    }

    let mut offset = 0usize;
    while offset < data.len() {
        if offset + nal_length_size > data.len() {
            anyhow::bail!("truncated NAL length prefix");
        }

        let mut length = 0usize;
        for byte in &data[offset..offset + nal_length_size] {
            length = (length << 8) | (*byte as usize);
        }
        offset += nal_length_size;
        if length == 0 {
            continue;
        }
        if offset + length > data.len() {
            anyhow::bail!("truncated NAL unit");
        }
        output.extend_from_slice(ANNEX_B_START_CODE);
        output.extend_from_slice(&data[offset..offset + length]);
        offset += length;
    }
    Ok(())
}

struct WebRtcSendTiming {
    last_timestamp_us: Option<u64>,
}

impl WebRtcSendTiming {
    fn new() -> Self {
        Self {
            last_timestamp_us: None,
        }
    }

    fn duration_for(
        &mut self,
        frame: &crate::transport::packet::FramePacket,
        realtime_stream: bool,
        use_frame_timestamps_for_realtime: bool,
    ) -> Duration {
        const MIN_FRAME_DURATION_US: u64 = 1_000;
        const DEFAULT_FRAME_DURATION_US: u64 = 16_667;
        const MAX_FRAME_DURATION_US: u64 = 100_000;
        let default_duration = if realtime_stream {
            realtime_sample_duration()
        } else {
            Duration::from_micros(DEFAULT_FRAME_DURATION_US)
        };
        let default_duration_us = default_duration
            .as_micros()
            .try_into()
            .unwrap_or(DEFAULT_FRAME_DURATION_US);

        if realtime_stream && !use_frame_timestamps_for_realtime {
            self.last_timestamp_us = Some(frame.timestamp_us);
            return default_duration;
        }

        let duration_us = self
            .last_timestamp_us
            .and_then(|previous| frame.timestamp_us.checked_sub(previous))
            .filter(|duration| *duration > 0)
            .unwrap_or(default_duration_us)
            .clamp(MIN_FRAME_DURATION_US, MAX_FRAME_DURATION_US);
        self.last_timestamp_us = Some(frame.timestamp_us);
        Duration::from_micros(duration_us)
    }
}

fn realtime_sample_duration() -> Duration {
    let fps = std::env::var("SIMDECK_REALTIME_FPS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(u64::from(WEBRTC_DEFAULT_LOCAL_STREAM_FPS))
        .clamp(15, u64::from(WEBRTC_MAX_LOCAL_STREAM_FPS));
    Duration::from_micros(1_000_000 / fps)
}

struct WebRtcMetricsGuard {
    metrics: Arc<crate::metrics::counters::Metrics>,
}

impl WebRtcMetricsGuard {
    fn new(metrics: Arc<crate::metrics::counters::Metrics>) -> Self {
        metrics
            .subscribers_connected
            .fetch_add(1, Ordering::Relaxed);
        metrics.active_streams.fetch_add(1, Ordering::Relaxed);
        Self { metrics }
    }
}

impl Drop for WebRtcMetricsGuard {
    fn drop(&mut self) {
        self.metrics
            .subscribers_disconnected
            .fetch_add(1, Ordering::Relaxed);
        let _ = self.metrics.active_streams.fetch_update(
            Ordering::Relaxed,
            Ordering::Relaxed,
            |current| Some(current.saturating_sub(1)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{
        android_h264_config_from_payload, append_avcc_parameter_sets, append_length_prefixed_nalus,
        h264_annex_b_sample, h264_frame_has_idr, h264_frame_is_decoder_sync, h264_sdp_fmtp_line,
        is_annex_b, is_h264_codec, rtcp_packet_requests_keyframe, rtp_packet_pacing,
        WebRtcMetricsGuard, WebRtcSendTiming, ANNEX_B_START_CODE,
    };
    use crate::api::routes::StreamQualityPayload;
    use crate::metrics::counters::Metrics;
    use crate::transport::packet::FramePacket;
    use bytes::Bytes;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, RwLock};
    use std::time::Duration;
    use webrtc::rtcp::payload_feedbacks::full_intra_request::FullIntraRequest;
    use webrtc::rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
    use webrtc::rtcp::sender_report::SenderReport;

    #[test]
    fn accepts_browser_h264_codec_strings() {
        assert!(is_h264_codec("h264"));
        assert!(is_h264_codec("avc1.42e01f"));
        assert!(is_h264_codec("avc3.640028"));
        assert!(!is_h264_codec("hvc1.1.6.L123.B0"));
        assert!(!is_h264_codec(""));
    }

    #[test]
    fn uses_h264_profile_level_id_when_available() {
        assert!(h264_sdp_fmtp_line("avc1.42e01f", "").contains("profile-level-id=42e01f"));
        assert!(h264_sdp_fmtp_line("h264", "").contains("profile-level-id=42e01f"));
        assert!(h264_sdp_fmtp_line(
            "avc1.640028",
            "a=rtpmap:99 H264/90000\r\na=fmtp:99 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f\r\n"
        )
        .contains("profile-level-id=42e01f"));
        assert!(h264_sdp_fmtp_line(
            "avc1.42e01f",
            "a=rtpmap:99 H264/90000\r\na=fmtp:99 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=640c1f\r\n"
        )
        .contains("profile-level-id=640c1f"));
        assert!(h264_sdp_fmtp_line(
            "avc1.42e01f",
            "a=rtpmap:99 H264/90000\r\na=fmtp:99 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=640c1f\r\na=rtpmap:100 H264/90000\r\na=fmtp:100 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f\r\n"
        )
        .contains("profile-level-id=42e01f"));
        assert!(h264_sdp_fmtp_line(
            "avc1.42c034",
            "a=rtpmap:99 H264/90000\r\na=fmtp:99 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f\r\n"
        )
        .contains("profile-level-id=42c034"));
    }

    #[test]
    fn detects_rtcp_keyframe_requests() {
        assert!(rtcp_packet_requests_keyframe(
            &PictureLossIndication::default()
        ));
        assert!(rtcp_packet_requests_keyframe(&FullIntraRequest::default()));
        assert!(!rtcp_packet_requests_keyframe(&SenderReport::default()));
    }

    #[test]
    fn android_full_size_quality_keeps_native_dimensions() {
        for (profile, min_bitrate, bits_per_pixel) in
            [("full", 12_000_000, 4), ("quality", 60_000_000, 10)]
        {
            let payload = StreamQualityPayload {
                profile: Some(profile.to_owned()),
                video_codec: None,
                max_edge: None,
                fps: Some(60),
                min_bitrate: None,
                bits_per_pixel: None,
            };

            let config = android_h264_config_from_payload(Some(&payload)).unwrap();
            let quality = config.quality;

            assert_eq!(quality.max_edge, None);
            assert_eq!(quality.fps, Some(60));
            assert_eq!(quality.min_bitrate, Some(min_bitrate));
            assert_eq!(quality.bits_per_pixel, Some(bits_per_pixel));
            assert_eq!(config.video_codec, "software");
        }
    }

    #[test]
    fn android_default_config_is_scaled_for_software_local_streaming() {
        let config = android_h264_config_from_payload(None).unwrap();
        let quality = config.quality;

        assert_eq!(quality.max_edge, Some(960));
        assert_eq!(quality.fps, Some(60));
        assert_eq!(quality.min_bitrate, Some(6_000_000));
        assert_eq!(quality.bits_per_pixel, Some(5));
        assert_eq!(config.video_codec, "software");
    }

    #[test]
    fn android_smooth_quality_scales_to_android_default_edge() {
        let payload = StreamQualityPayload {
            profile: Some("smooth".to_owned()),
            video_codec: None,
            max_edge: None,
            fps: Some(60),
            min_bitrate: None,
            bits_per_pixel: None,
        };

        let config = android_h264_config_from_payload(Some(&payload)).unwrap();
        let quality = config.quality;

        assert_eq!(quality.max_edge, Some(960));
        assert_eq!(quality.fps, Some(60));
        assert_eq!(quality.min_bitrate, Some(4_000_000));
        assert_eq!(quality.bits_per_pixel, Some(5));
        assert_eq!(config.video_codec, "software");
    }

    #[test]
    fn android_balanced_quality_scales_to_android_default_edge() {
        let payload = StreamQualityPayload {
            profile: Some("balanced".to_owned()),
            video_codec: None,
            max_edge: None,
            fps: Some(60),
            min_bitrate: None,
            bits_per_pixel: None,
        };

        let config = android_h264_config_from_payload(Some(&payload)).unwrap();
        let quality = config.quality;

        assert_eq!(quality.max_edge, Some(960));
        assert_eq!(quality.fps, Some(60));
    }

    #[test]
    fn android_stream_config_honors_requested_video_codec() {
        let payload = StreamQualityPayload {
            profile: Some("balanced".to_owned()),
            video_codec: Some("hardware".to_owned()),
            max_edge: None,
            fps: Some(60),
            min_bitrate: None,
            bits_per_pixel: None,
        };

        let config = android_h264_config_from_payload(Some(&payload)).unwrap();

        assert_eq!(config.video_codec, "hardware");
        assert_eq!(config.quality.max_edge, Some(960));
    }

    #[test]
    fn android_default_source_reads_are_not_pre_throttled() {
        let quality = crate::android::AndroidH264StreamQuality {
            fps: Some(60),
            ..Default::default()
        };

        assert_eq!(super::android_source_read_interval(quality), None);
    }

    #[test]
    fn android_low_fps_source_reads_are_pre_throttled() {
        let quality = crate::android::AndroidH264StreamQuality {
            fps: Some(30),
            ..Default::default()
        };

        assert_eq!(
            super::android_source_read_interval(quality),
            Some(Duration::from_micros(33_333))
        );
    }

    #[test]
    fn android_grpc_rgba_padding_preserves_top_down_rows() {
        let pixels = [
            [1, 0, 0, 255],
            [2, 0, 0, 255],
            [3, 0, 0, 255],
            [4, 0, 0, 255],
            [5, 0, 0, 255],
            [6, 0, 0, 255],
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();

        let (rgba, width, height) = super::android_grpc_top_down_even_rgba(&pixels, 3, 2).unwrap();

        assert_eq!((width, height), (4, 2));
        let red_values = rgba
            .chunks_exact(4)
            .map(|pixel| pixel[0])
            .collect::<Vec<_>>();
        assert_eq!(red_values, vec![1, 2, 3, 3, 4, 5, 6, 6]);
    }

    fn android_source_inner_for_test(
        udid: &str,
        source_id: u64,
        video_codec: &str,
    ) -> Arc<super::AndroidWebRtcSourceInner> {
        let (shutdown_tx, _) = tokio::sync::broadcast::channel(1);
        let (sender, _) =
            tokio::sync::broadcast::channel(super::ANDROID_WEBRTC_FRAME_BROADCAST_CAPACITY);
        Arc::new(super::AndroidWebRtcSourceInner {
            source_id,
            udid: udid.to_owned(),
            video_codec: video_codec.to_owned(),
            shutdown_tx,
            sender,
            latest_keyframe: RwLock::new(None),
            frame_sequence: AtomicU64::new(0),
            quality: Mutex::new(crate::android::AndroidH264StreamQuality::default()),
            source_kind: RwLock::new("shared-video"),
            encoder_handle: AtomicUsize::new(0),
            callback_user_data: AtomicUsize::new(0),
            shared_frames_read: AtomicU64::new(0),
            encode_submissions: AtomicU64::new(0),
            encode_failures: AtomicU64::new(0),
            latest_shared_frame_copy_us: AtomicU64::new(0),
            latest_encode_submit_us: AtomicU64::new(0),
            latest_source_frame_gap_us: AtomicU64::new(0),
            latest_source_timestamp_us: AtomicU64::new(0),
            source_sequence: AtomicU64::new(0),
            source_fps: AtomicU64::new(0),
            source_width: AtomicU64::new(0),
            source_height: AtomicU64::new(0),
            output_width: AtomicU64::new(0),
            output_height: AtomicU64::new(0),
            metrics: Arc::new(Metrics::default()),
        })
    }

    #[test]
    fn android_source_registry_reuses_same_udid_and_codec() {
        let udid = format!("test-android-source-{}", std::process::id());
        let config = super::AndroidH264StreamConfig {
            quality: crate::android::AndroidH264StreamQuality {
                fps: Some(30),
                ..Default::default()
            },
            video_codec: "software".to_owned(),
        };
        let first = android_source_inner_for_test(&udid, 1, "software");
        let mut first_shutdown = first.shutdown_tx.subscribe();
        let mut sources = vec![Arc::downgrade(&first)];

        let reused = super::reusable_android_webrtc_source(&mut sources, &udid, &config).unwrap();

        assert!(Arc::ptr_eq(&first, &reused));
        assert!(first_shutdown.try_recv().is_err());
        assert_eq!(sources.len(), 1);
        assert_eq!(first.h264_quality().fps, Some(30));
    }

    #[test]
    fn android_source_registry_replaces_same_udid_when_codec_changes() {
        let udid = format!("test-android-source-codec-{}", std::process::id());
        let config = super::AndroidH264StreamConfig {
            quality: crate::android::AndroidH264StreamQuality::default(),
            video_codec: "hardware".to_owned(),
        };
        let first = android_source_inner_for_test(&udid, 1, "software");
        let mut first_shutdown = first.shutdown_tx.subscribe();
        let mut sources = vec![Arc::downgrade(&first)];

        let reused = super::reusable_android_webrtc_source(&mut sources, &udid, &config);

        assert!(reused.is_none());
        assert!(first_shutdown.try_recv().is_ok());
        assert!(sources.is_empty());
    }

    #[test]
    fn realtime_h264_advertises_retransmission_feedback() {
        let feedback = super::h264_rtcp_feedback();
        assert!(feedback
            .iter()
            .any(|item| item.typ == "nack" && item.parameter.is_empty()));
        assert!(feedback
            .iter()
            .any(|item| item.typ == "nack" && item.parameter == "pli"));
        assert!(feedback
            .iter()
            .any(|item| item.typ == "ccm" && item.parameter == "fir"));
        assert!(feedback
            .iter()
            .any(|item| item.typ == "transport-cc" && item.parameter.is_empty()));
    }

    #[test]
    fn peer_disconnected_grace_covers_remote_ice_wobbles() {
        assert!(super::WEBRTC_PEER_DISCONNECTED_TIMEOUT >= Duration::from_secs(10));
    }

    #[test]
    fn registering_second_webrtc_stream_does_not_cancel_first() {
        let udid = format!("test-{}", std::process::id());
        super::reset_webrtc_media_streams_for_test(&udid);
        let (first_token, mut first_rx) = super::register_webrtc_media_stream_for_test(&udid);
        let (second_token, mut second_rx) = super::register_webrtc_media_stream_for_test(&udid);

        assert!(super::has_media_stream(&udid));
        assert!(first_rx.try_recv().is_err());
        assert!(second_rx.try_recv().is_err());
        assert_eq!(super::active_webrtc_media_stream_count(&udid), 2);

        super::clear_webrtc_media_stream_for_test(&udid, &first_token);
        super::clear_webrtc_media_stream_for_test(&udid, &second_token);
        assert!(!super::has_media_stream(&udid));
    }

    #[test]
    fn clearing_webrtc_stream_is_scoped_and_idempotent() {
        let udid = format!("test-clear-{}", std::process::id());
        super::reset_webrtc_media_streams_for_test(&udid);
        let (first_token, mut first_rx) = super::register_webrtc_media_stream_for_test(&udid);
        let (second_token, mut second_rx) = super::register_webrtc_media_stream_for_test(&udid);

        super::clear_webrtc_media_stream_for_test(&udid, &first_token);
        super::clear_webrtc_media_stream_for_test(&udid, &first_token);

        assert!(first_rx.try_recv().is_err());
        assert!(second_rx.try_recv().is_err());
        assert_eq!(super::active_webrtc_media_stream_count(&udid), 1);
        assert!(super::has_media_stream(&udid));

        super::clear_webrtc_media_stream_for_test(&udid, &second_token);
        assert!(!super::has_media_stream(&udid));
    }

    #[test]
    fn registering_same_client_webrtc_stream_replaces_old_stream() {
        let udid = format!("test-client-cap-{}", std::process::id());
        super::reset_webrtc_media_streams_for_test(&udid);
        let (_first_token, mut first_rx) =
            super::register_webrtc_media_stream_for_client_test(&udid, "page-1");
        let (second_token, mut second_rx) =
            super::register_webrtc_media_stream_for_client_test(&udid, "page-1");

        assert!(first_rx.try_recv().is_ok());
        assert!(second_rx.try_recv().is_err());
        assert_eq!(super::active_webrtc_media_stream_count(&udid), 1);

        super::clear_webrtc_media_stream_for_test(&udid, &second_token);
        assert!(!super::has_media_stream(&udid));
    }

    #[test]
    fn registering_identified_client_evicts_anonymous_streams() {
        let udid = format!("test-anonymous-cap-{}", std::process::id());
        super::reset_webrtc_media_streams_for_test(&udid);
        let (_anonymous_token, mut anonymous_rx) =
            super::register_webrtc_media_stream_for_test(&udid);
        let (identified_token, mut identified_rx) =
            super::register_webrtc_media_stream_evicting_anonymous_for_test(&udid, "page-1");

        assert!(anonymous_rx.try_recv().is_ok());
        assert!(identified_rx.try_recv().is_err());
        assert_eq!(super::active_webrtc_media_stream_count(&udid), 1);

        super::clear_webrtc_media_stream_for_test(&udid, &identified_token);
        assert!(!super::has_media_stream(&udid));
    }

    #[test]
    fn registering_replacement_stream_cancels_all_existing_udid_streams() {
        let udid = format!("test-replace-{}", std::process::id());
        super::reset_webrtc_media_streams_for_test(&udid);
        let (_anonymous_token, mut anonymous_rx) =
            super::register_webrtc_media_stream_for_test(&udid);
        let (_first_client_token, mut first_client_rx) =
            super::register_webrtc_media_stream_for_client_test(&udid, "page-1");
        let (replacement_token, mut replacement_rx) =
            super::register_webrtc_media_stream_replacing_udid_for_test(&udid, Some("page-2"));

        assert!(anonymous_rx.try_recv().is_ok());
        assert!(first_client_rx.try_recv().is_ok());
        assert!(replacement_rx.try_recv().is_err());
        assert_eq!(super::active_webrtc_media_stream_count(&udid), 1);

        super::clear_webrtc_media_stream_for_test(&udid, &replacement_token);
        assert!(!super::has_media_stream(&udid));
    }

    #[test]
    fn registering_more_than_max_webrtc_streams_cancels_oldest() {
        let udid = format!("test-cap-{}", std::process::id());
        super::reset_webrtc_media_streams_for_test(&udid);
        let (_first_token, mut first_rx) = super::register_webrtc_media_stream_for_test(&udid);
        let mut retained = Vec::new();
        for _ in 0..super::MAX_WEBRTC_MEDIA_STREAMS_PER_UDID {
            retained.push(super::register_webrtc_media_stream_for_test(&udid));
        }

        assert!(first_rx.try_recv().is_ok());
        for (_token, rx) in retained.iter_mut() {
            assert!(rx.try_recv().is_err());
        }
        assert_eq!(
            super::active_webrtc_media_stream_count(&udid),
            super::MAX_WEBRTC_MEDIA_STREAMS_PER_UDID
        );

        for (token, _rx) in retained {
            super::clear_webrtc_media_stream_for_test(&udid, &token);
        }
        assert!(!super::has_media_stream(&udid));
    }

    #[test]
    fn metrics_guard_balances_stream_connect_and_disconnect_counts() {
        let metrics = Arc::new(Metrics::default());

        {
            let _guard = WebRtcMetricsGuard::new(metrics.clone());
            assert_eq!(metrics.subscribers_connected.load(Ordering::Relaxed), 1);
            assert_eq!(metrics.active_streams.load(Ordering::Relaxed), 1);
        }

        assert_eq!(metrics.subscribers_disconnected.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.active_streams.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn metrics_guard_does_not_underflow_active_streams() {
        let metrics = Arc::new(Metrics::default());
        let guard = WebRtcMetricsGuard::new(metrics.clone());
        metrics.active_streams.store(0, Ordering::Relaxed);

        drop(guard);

        assert_eq!(metrics.active_streams.load(Ordering::Relaxed), 0);
        assert_eq!(metrics.subscribers_disconnected.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn send_timing_uses_frame_timestamps_for_non_realtime_streams() {
        let mut timing = WebRtcSendTiming::new();
        let first = FramePacket {
            frame_sequence: 1,
            timestamp_us: 10_000,
            is_keyframe: true,
            width: 100,
            height: 100,
            codec: Some("h264".to_owned()),
            description: None,
            data: Bytes::from_static(&[0, 0, 1, 0x65]),
        };
        let second = FramePacket {
            frame_sequence: 2,
            timestamp_us: 43_333,
            is_keyframe: false,
            width: 100,
            height: 100,
            codec: Some("h264".to_owned()),
            description: None,
            data: Bytes::from_static(&[0, 0, 1, 0x41]),
        };

        assert_eq!(
            timing.duration_for(&first, false, false),
            Duration::from_micros(16_667)
        );
        assert_eq!(
            timing.duration_for(&second, false, false),
            Duration::from_micros(33_333)
        );
    }

    #[test]
    fn send_timing_clamps_non_realtime_timestamp_gaps() {
        let mut timing = WebRtcSendTiming::new();
        let first = FramePacket {
            frame_sequence: 1,
            timestamp_us: 100_000,
            is_keyframe: true,
            width: 100,
            height: 100,
            codec: Some("h264".to_owned()),
            description: None,
            data: Bytes::from_static(&[0, 0, 1, 0x65]),
        };
        let backwards = FramePacket {
            timestamp_us: 90_000,
            ..first.clone_for_test(2)
        };
        let huge_gap = FramePacket {
            timestamp_us: 1_000_000,
            ..first.clone_for_test(3)
        };

        assert_eq!(
            timing.duration_for(&first, false, false),
            Duration::from_micros(16_667)
        );
        assert_eq!(
            timing.duration_for(&backwards, false, false),
            Duration::from_micros(16_667)
        );
        assert_eq!(
            timing.duration_for(&huge_gap, false, false),
            Duration::from_micros(100_000)
        );
    }

    #[test]
    fn write_timeout_recovery_preserves_non_realtime_h264_chain() {
        assert_eq!(
            super::recovery_action_for_write_timeout(false),
            super::FrameRecoveryAction::Keyframe
        );
        assert_eq!(
            super::recovery_action_for_write_timeout(true),
            super::FrameRecoveryAction::Refresh
        );
    }

    #[test]
    fn realtime_send_timing_uses_fixed_sample_duration() {
        let mut timing = WebRtcSendTiming::new();
        let first = FramePacket {
            frame_sequence: 1,
            timestamp_us: 10_000,
            is_keyframe: true,
            width: 100,
            height: 100,
            codec: Some("h264".to_owned()),
            description: None,
            data: Bytes::from_static(&[0, 0, 1, 0x65]),
        };
        let second = FramePacket {
            frame_sequence: 2,
            timestamp_us: 25_500,
            is_keyframe: false,
            width: 100,
            height: 100,
            codec: Some("h264".to_owned()),
            description: None,
            data: Bytes::from_static(&[0, 0, 1, 0x41]),
        };

        assert_eq!(
            timing.duration_for(&first, true, false),
            super::realtime_sample_duration()
        );
        assert_eq!(
            timing.duration_for(&second, true, false),
            super::realtime_sample_duration()
        );
    }

    #[test]
    fn realtime_android_send_timing_ignores_source_timestamp_jitter() {
        let mut timing = WebRtcSendTiming::new();
        let first = FramePacket {
            frame_sequence: 1,
            timestamp_us: 10_000,
            is_keyframe: true,
            width: 100,
            height: 100,
            codec: Some("h264".to_owned()),
            description: None,
            data: Bytes::from_static(&[0, 0, 1, 0x65]),
        };
        let second = FramePacket {
            frame_sequence: 2,
            timestamp_us: 118_333,
            is_keyframe: false,
            width: 100,
            height: 100,
            codec: Some("h264".to_owned()),
            description: None,
            data: Bytes::from_static(&[0, 0, 1, 0x41]),
        };

        assert_eq!(
            timing.duration_for(&first, true, false),
            super::realtime_sample_duration()
        );
        assert_eq!(
            timing.duration_for(&second, true, false),
            super::realtime_sample_duration()
        );
    }

    #[test]
    fn rtp_packet_pacing_batches_large_frames() {
        assert_eq!(rtp_packet_pacing(Duration::from_millis(20), 10, true), None);
        assert_eq!(
            rtp_packet_pacing(Duration::from_millis(20), 10, false),
            None
        );
        assert_eq!(rtp_packet_pacing(Duration::from_millis(20), 1, true), None);
        assert_eq!(
            rtp_packet_pacing(Duration::from_millis(33), 60, false),
            Some((8, Duration::from_micros(4125)))
        );
    }

    #[test]
    fn converts_avcc_parameter_sets_to_annex_b() {
        let avcc = [
            1, 0x42, 0xe0, 0x1f, 0xff, 0xe1, 0, 3, 0x67, 0x42, 0x00, 1, 0, 2, 0x68, 0xce,
        ];
        let mut output = Vec::new();

        append_avcc_parameter_sets(&avcc, &mut output).unwrap();

        assert_eq!(
            output,
            [
                ANNEX_B_START_CODE,
                &[0x67, 0x42, 0x00],
                ANNEX_B_START_CODE,
                &[0x68, 0xce],
            ]
            .concat()
        );
    }

    #[test]
    fn converts_length_prefixed_h264_sample_to_annex_b() {
        let sample = [0, 0, 0, 2, 0x65, 0x88, 0, 0, 0, 2, 0x41, 0x9a];
        let mut output = Vec::new();

        append_length_prefixed_nalus(&sample, 4, &mut output).unwrap();

        assert_eq!(
            output,
            [
                ANNEX_B_START_CODE,
                &[0x65, 0x88],
                ANNEX_B_START_CODE,
                &[0x41, 0x9a],
            ]
            .concat()
        );
        assert!(is_annex_b(&output));
    }

    #[test]
    fn detects_idr_nalus_in_h264_samples() {
        let avcc_frame = FramePacket {
            codec: Some("avc1.42e01f".to_owned()),
            data: Bytes::from_static(&[0, 0, 0, 2, 0x41, 0x9a, 0, 0, 0, 2, 0x65, 0x88]),
            description: Some(Bytes::from_static(&[1, 0x42, 0xe0, 0x1f, 0xff])),
            frame_sequence: 1,
            height: 1,
            is_keyframe: true,
            timestamp_us: 0,
            width: 1,
        };
        let annex_b_frame = FramePacket {
            codec: Some("avc1.42e01f".to_owned()),
            data: Bytes::from_static(&[0, 0, 1, 0x67, 0x42, 0, 0, 0, 1, 0x65, 0x88]),
            description: None,
            frame_sequence: 2,
            height: 1,
            is_keyframe: true,
            timestamp_us: 0,
            width: 1,
        };
        let non_idr_frame = FramePacket {
            codec: Some("avc1.42e01f".to_owned()),
            data: Bytes::from_static(&[0, 0, 0, 2, 0x41, 0x9a]),
            description: Some(Bytes::from_static(&[1, 0x42, 0xe0, 0x1f, 0xff])),
            frame_sequence: 3,
            height: 1,
            is_keyframe: true,
            timestamp_us: 0,
            width: 1,
        };

        assert!(h264_frame_has_idr(&avcc_frame));
        assert!(h264_frame_has_idr(&annex_b_frame));
        assert!(!h264_frame_has_idr(&non_idr_frame));
    }

    #[test]
    fn detects_length_prefixed_idr_when_avcc_length_size_is_wrong() {
        let two_byte_length_prefixed_idr = FramePacket {
            codec: Some("avc1.42e01f".to_owned()),
            data: Bytes::from_static(&[0, 2, 0x65, 0x88, 0, 2, 0x41, 0x9a]),
            description: Some(Bytes::from_static(&[1, 0x42, 0xe0, 0x1f, 0xff])),
            frame_sequence: 1,
            height: 1,
            is_keyframe: true,
            timestamp_us: 0,
            width: 1,
        };
        let annex_b = h264_annex_b_sample(&two_byte_length_prefixed_idr).unwrap();

        assert!(h264_frame_has_idr(&two_byte_length_prefixed_idr));
        assert!(h264_frame_is_decoder_sync(&two_byte_length_prefixed_idr));
        assert_eq!(
            annex_b,
            [
                ANNEX_B_START_CODE,
                &[0x65, 0x88],
                ANNEX_B_START_CODE,
                &[0x41, 0x9a],
            ]
            .concat()
        );
    }

    #[test]
    fn rejects_truncated_h264_decoder_config_records() {
        let mut output = Vec::new();

        let result =
            append_avcc_parameter_sets(&[1, 0x42, 0xe0, 0x1f, 0xff, 0xe1, 0, 4, 0x67], &mut output);

        assert!(result.is_err());
    }

    #[test]
    fn rejects_truncated_length_prefixed_h264_samples() {
        let mut output = Vec::new();

        let result = append_length_prefixed_nalus(&[0, 0, 0, 4, 0x65], 4, &mut output);

        assert!(result.is_err());
    }

    #[test]
    fn keyframes_include_decoder_config_before_sample_nalus() {
        let frame = FramePacket {
            frame_sequence: 1,
            timestamp_us: 0,
            is_keyframe: true,
            width: 100,
            height: 100,
            codec: Some("avc1.42e01f".to_owned()),
            description: Some(Bytes::from_static(&[
                1, 0x42, 0xe0, 0x1f, 0xff, 0xe1, 0, 3, 0x67, 0x42, 0x00, 1, 0, 2, 0x68, 0xce,
            ])),
            data: Bytes::from_static(&[0, 0, 0, 2, 0x65, 0x88]),
        };

        let sample = h264_annex_b_sample(&frame).unwrap();

        assert_eq!(
            sample,
            [
                ANNEX_B_START_CODE,
                &[0x67, 0x42, 0x00],
                ANNEX_B_START_CODE,
                &[0x68, 0xce],
                ANNEX_B_START_CODE,
                &[0x65, 0x88],
            ]
            .concat()
        );
    }

    trait CloneFrameForTest {
        fn clone_for_test(&self, frame_sequence: u64) -> Self;
    }

    impl CloneFrameForTest for FramePacket {
        fn clone_for_test(&self, frame_sequence: u64) -> Self {
            Self {
                frame_sequence,
                timestamp_us: self.timestamp_us,
                is_keyframe: self.is_keyframe,
                width: self.width,
                height: self.height,
                codec: self.codec.clone(),
                description: self.description.clone(),
                data: self.data.clone(),
            }
        }
    }
}
