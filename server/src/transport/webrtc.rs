use crate::api::routes::{run_control_message, AppState, ControlMessage};
use crate::error::AppError;
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast::{self, error::TryRecvError};
use tokio::time;
use tracing::{info, warn};
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::{MediaEngine, MIME_TYPE_H264};
use webrtc::api::APIBuilder;
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::data_channel::data_channel_state::RTCDataChannelState;
use webrtc::data_channel::RTCDataChannel;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::interceptor::registry::Registry;
use webrtc::media::Sample;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;
use webrtc::track::track_local::TrackLocal;

const ANNEX_B_START_CODE: &[u8] = &[0, 0, 0, 1];
const DEFAULT_STUN_URL: &str = "stun:stun.l.google.com:19302";
const WEBRTC_CONTROL_CHANNEL_LABEL: &str = "simdeck-control";
const WEBRTC_JPEG_CHANNEL_LABEL: &str = "simdeck-video-jpeg";
const WEBRTC_BOOTSTRAP_KEYFRAME_INTERVAL: Duration = Duration::from_millis(250);
const WEBRTC_BOOTSTRAP_KEYFRAME_REPEATS: u8 = 4;
const WEBRTC_MIN_REFRESH_INTERVAL: Duration = Duration::from_millis(16);
const WEBRTC_MAX_REFRESH_INTERVAL: Duration = Duration::from_millis(100);
const WEBRTC_WRITE_TIMEOUT: Duration = Duration::from_millis(120);
const JPEG_CHUNK_MAGIC: &[u8; 4] = b"SDJF";
const JPEG_CHUNK_VERSION: u8 = 1;
const JPEG_CHUNK_HEADER_BYTES: usize = 40;
const JPEG_CHUNK_PAYLOAD_BYTES: usize = 60 * 1024;
const JPEG_CHANNEL_MIN_BUFFERED_BYTES: usize = 2 * 1024 * 1024;
const JPEG_CHANNEL_BUFFERED_FRAME_MULTIPLIER: usize = 2;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebRtcOfferPayload {
    pub sdp: String,
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
}

pub async fn create_answer(
    state: AppState,
    udid: String,
    payload: WebRtcOfferPayload,
) -> Result<WebRtcAnswerPayload, AppError> {
    if payload.kind != "offer" {
        return Err(AppError::bad_request(
            "WebRTC payload must include type `offer`.",
        ));
    }

    let session = state.registry.get_or_create_async(&udid).await?;
    if let Err(error) = session.ensure_started_async().await {
        state.registry.remove(&udid);
        return Err(error);
    }
    session.request_keyframe();
    info!(
        "WebRTC offer for {udid}: remote_candidates={} remote_candidate_types={} ice_servers={}",
        count_sdp_candidates(&payload.sdp),
        summarize_sdp_candidate_types(&payload.sdp),
        std::env::var("SIMDECK_WEBRTC_ICE_SERVERS")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_STUN_URL.to_owned())
    );

    let first_frame = session
        .wait_for_keyframe(Duration::from_secs(3))
        .await
        .ok_or_else(|| AppError::native("Timed out waiting for a simulator keyframe."))?;
    if payload.transport.as_deref() == Some("data-channel") {
        return create_data_channel_answer(state, udid, session, first_frame, payload).await;
    }

    let codec = first_frame
        .codec
        .as_deref()
        .unwrap_or_default()
        .to_lowercase();
    if !is_h264_codec(&codec) {
        return Err(AppError::bad_request(
            "WebRTC preview requires H.264. Restart SimDeck with `--video-codec h264-software`.",
        ));
    }

    let mut media_engine = MediaEngine::default();
    media_engine
        .register_default_codecs()
        .map_err(|error| AppError::internal(format!("register WebRTC codecs: {error}")))?;
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
            ..Default::default()
        })
        .await
        .map_err(|error| AppError::internal(format!("create WebRTC peer connection: {error}")))?,
    );
    register_diagnostics(&peer_connection, &udid);
    register_control_data_channel(&peer_connection, session.clone(), udid.clone());

    let video_track = Arc::new(TrackLocalStaticSample::new(
        RTCRtpCodecCapability {
            mime_type: MIME_TYPE_H264.to_owned(),
            clock_rate: 90_000,
            channels: 0,
            sdp_fmtp_line: h264_sdp_fmtp_line(&codec),
            rtcp_feedback: vec![],
        },
        "simdeck-video".to_owned(),
        "simdeck".to_owned(),
    ));

    let rtp_sender = peer_connection
        .add_track(video_track.clone() as Arc<dyn TrackLocal + Send + Sync>)
        .await
        .map_err(|error| AppError::internal(format!("add WebRTC video track: {error}")))?;
    tokio::spawn(async move {
        let mut buffer = vec![0u8; 1500];
        while rtp_sender.read(&mut buffer).await.is_ok() {}
    });

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
    let _ = gather_complete.recv().await;
    let local_description = peer_connection
        .local_description()
        .await
        .ok_or_else(|| AppError::internal("WebRTC local description was not set."))?;
    info!(
        "WebRTC answer for {udid}: local_candidates={} local_candidate_types={}",
        count_sdp_candidates(&local_description.sdp),
        summarize_sdp_candidate_types(&local_description.sdp)
    );

    tokio::spawn(stream_h264_frames(
        state,
        udid,
        session,
        first_frame,
        peer_connection,
        video_track,
    ));

    Ok(WebRtcAnswerPayload {
        sdp: local_description.sdp,
        kind: "answer".to_owned(),
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

async fn create_data_channel_answer(
    state: AppState,
    udid: String,
    session: crate::simulators::session::SimulatorSession,
    first_frame: crate::transport::packet::SharedFrame,
    payload: WebRtcOfferPayload,
) -> Result<WebRtcAnswerPayload, AppError> {
    let codec = first_frame
        .codec
        .as_deref()
        .unwrap_or_default()
        .to_lowercase();
    if codec != "jpeg" {
        return Err(AppError::bad_request(
            "WebRTC data-channel preview requires `--video-codec jpeg`.",
        ));
    }

    let mut media_engine = MediaEngine::default();
    media_engine
        .register_default_codecs()
        .map_err(|error| AppError::internal(format!("register WebRTC codecs: {error}")))?;
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
            ..Default::default()
        })
        .await
        .map_err(|error| AppError::internal(format!("create WebRTC peer connection: {error}")))?,
    );
    register_diagnostics(&peer_connection, &udid);
    register_data_channel_streams(
        &peer_connection,
        state.clone(),
        session.clone(),
        udid.clone(),
        first_frame,
    );

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
    let _ = gather_complete.recv().await;
    let local_description = peer_connection
        .local_description()
        .await
        .ok_or_else(|| AppError::internal("WebRTC local description was not set."))?;
    tokio::spawn(hold_peer_connection_until_closed(peer_connection));

    Ok(WebRtcAnswerPayload {
        sdp: local_description.sdp,
        kind: "answer".to_owned(),
    })
}

async fn hold_peer_connection_until_closed(
    peer_connection: Arc<webrtc::peer_connection::RTCPeerConnection>,
) {
    loop {
        if matches!(
            peer_connection.connection_state(),
            RTCPeerConnectionState::Closed | RTCPeerConnectionState::Failed
        ) {
            break;
        }
        time::sleep(Duration::from_secs(5)).await;
    }
}

fn register_data_channel_streams(
    peer_connection: &Arc<webrtc::peer_connection::RTCPeerConnection>,
    state: AppState,
    session: crate::simulators::session::SimulatorSession,
    udid: String,
    first_frame: crate::transport::packet::SharedFrame,
) {
    peer_connection.on_data_channel(Box::new(move |channel: Arc<RTCDataChannel>| {
        let state = state.clone();
        let session = session.clone();
        let udid = udid.clone();
        let first_frame = first_frame.clone();
        Box::pin(async move {
            match channel.label() {
                WEBRTC_CONTROL_CHANNEL_LABEL => {
                    attach_control_data_channel(channel, session, udid);
                }
                WEBRTC_JPEG_CHANNEL_LABEL => {
                    let state_for_stream = state.clone();
                    let session_for_stream = session.clone();
                    let udid_for_stream = udid.clone();
                    let stream_channel = channel.clone();
                    channel.on_open(Box::new(move || {
                        let channel = stream_channel.clone();
                        let state = state_for_stream.clone();
                        let session = session_for_stream.clone();
                        let udid = udid_for_stream.clone();
                        let first_frame = first_frame.clone();
                        Box::pin(async move {
                            tokio::spawn(stream_jpeg_data_channel_frames(
                                state,
                                udid,
                                session,
                                first_frame,
                                channel,
                            ));
                        })
                    }));
                }
                _ => {}
            }
        })
    }));
}

fn register_control_data_channel(
    peer_connection: &Arc<webrtc::peer_connection::RTCPeerConnection>,
    session: crate::simulators::session::SimulatorSession,
    udid: String,
) {
    peer_connection.on_data_channel(Box::new(move |channel: Arc<RTCDataChannel>| {
        let session = session.clone();
        let udid = udid.clone();
        Box::pin(async move {
            if channel.label() != WEBRTC_CONTROL_CHANNEL_LABEL {
                return;
            }
            attach_control_data_channel(channel, session, udid);
        })
    }));
}

fn attach_control_data_channel(
    channel: Arc<RTCDataChannel>,
    session: crate::simulators::session::SimulatorSession,
    udid: String,
) {
    channel.on_message(Box::new(move |message: DataChannelMessage| {
        let session = session.clone();
        let udid = udid.clone();
        Box::pin(async move {
            let Ok(text) = std::str::from_utf8(&message.data) else {
                warn!("Invalid WebRTC control message bytes for {udid}");
                return;
            };
            let control_message = match serde_json::from_str::<ControlMessage>(text) {
                Ok(message) => message,
                Err(error) => {
                    warn!("Invalid WebRTC control message for {udid}: {error}");
                    return;
                }
            };
            if let Err(error) = run_control_message(session, control_message).await {
                warn!("WebRTC control message failed for {udid}: {error}");
            }
        })
    }));
}

fn is_h264_codec(codec: &str) -> bool {
    let codec = codec.trim().to_ascii_lowercase();
    codec.contains("h264") || codec.starts_with("avc1.") || codec.starts_with("avc3.")
}

fn h264_sdp_fmtp_line(codec: &str) -> String {
    let profile_level_id = codec
        .split_once('.')
        .map(|(_, value)| value)
        .filter(|value| value.len() >= 6)
        .map(|value| &value[..6])
        .filter(|value| value.chars().all(|ch| ch.is_ascii_hexdigit()))
        .unwrap_or("42e01f");
    format!("level-asymmetry-allowed=1;packetization-mode=1;profile-level-id={profile_level_id}")
}

fn ice_servers() -> Vec<RTCIceServer> {
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
    vec![RTCIceServer {
        urls,
        ..Default::default()
    }]
}

async fn stream_h264_frames(
    state: AppState,
    udid: String,
    session: crate::simulators::session::SimulatorSession,
    first_frame: crate::transport::packet::SharedFrame,
    peer_connection: Arc<webrtc::peer_connection::RTCPeerConnection>,
    video_track: Arc<TrackLocalStaticSample>,
) {
    let mut rx = session.subscribe();
    let mut latest_keyframe = first_frame;
    let mut last_sequence = 0u64;
    let mut send_timing = WebRtcSendTiming::new();
    let mut bootstrap_interval = time::interval(WEBRTC_BOOTSTRAP_KEYFRAME_INTERVAL);
    let mut refresh_sleep = Box::pin(time::sleep(WEBRTC_MIN_REFRESH_INTERVAL));
    let mut bootstrap_frames_remaining = WEBRTC_BOOTSTRAP_KEYFRAME_REPEATS;
    let mut adaptive_refresh_interval = WEBRTC_MIN_REFRESH_INTERVAL;
    let mut waiting_for_keyframe = false;
    let _guard = WebRtcMetricsGuard::new(state.metrics.clone());

    loop {
        tokio::select! {
            _ = &mut refresh_sleep => {
                session.request_refresh();
                refresh_sleep
                    .as_mut()
                    .reset(time::Instant::now() + adaptive_refresh_interval);
            }
            _ = bootstrap_interval.tick(), if bootstrap_frames_remaining > 0 => {
                if let Err(error) = write_frame_sample(
                    &video_track,
                    &latest_keyframe,
                    WEBRTC_BOOTSTRAP_KEYFRAME_INTERVAL,
                ).await {
                    warn!("WebRTC bootstrap keyframe write failed for {udid}: {error}");
                    break;
                }
                state.metrics.frames_sent.fetch_add(1, Ordering::Relaxed);
                bootstrap_frames_remaining = bootstrap_frames_remaining.saturating_sub(1);
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
                        session.request_keyframe();
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                };
                let (frame, skipped) = freshest_available_frame(frame, &mut rx);
                if skipped > 0 {
                    state
                        .metrics
                        .frames_dropped_server
                        .fetch_add(skipped, Ordering::Relaxed);
                    if !frame.is_keyframe {
                        waiting_for_keyframe = true;
                        session.request_keyframe();
                        continue;
                    }
                }
                if last_sequence != 0 && frame.frame_sequence > last_sequence + 1 && !frame.is_keyframe {
                    state
                        .metrics
                        .frames_dropped_server
                        .fetch_add(frame.frame_sequence - last_sequence - 1, Ordering::Relaxed);
                    waiting_for_keyframe = true;
                    session.request_keyframe();
                    continue;
                }
                if waiting_for_keyframe && !frame.is_keyframe {
                    state.metrics.frames_dropped_server.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                if frame.is_keyframe {
                    latest_keyframe = frame.clone();
                    waiting_for_keyframe = false;
                }
                let duration = send_timing.duration_for(&frame);
                let started_at = time::Instant::now();
                let write_result = time::timeout(
                    WEBRTC_WRITE_TIMEOUT,
                    write_frame_sample(&video_track, &frame, duration),
                ).await;
                adaptive_refresh_interval = adaptive_interval_for_write(started_at.elapsed());
                if let Err(error) = write_result
                    .map_err(|_| anyhow::anyhow!(
                        "timed out writing WebRTC frame after {}ms",
                        WEBRTC_WRITE_TIMEOUT.as_millis()
                    ))
                    .and_then(|result| result)
                {
                    warn!("WebRTC frame write failed for {udid}: {error}");
                    break;
                }
                last_sequence = frame.frame_sequence;
                state.metrics.frames_sent.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    let _ = peer_connection.close().await;
}

fn adaptive_interval_for_write(write_elapsed: Duration) -> Duration {
    let target_ms = (write_elapsed.as_millis() as u64).saturating_mul(2).clamp(
        WEBRTC_MIN_REFRESH_INTERVAL.as_millis() as u64,
        WEBRTC_MAX_REFRESH_INTERVAL.as_millis() as u64,
    );
    Duration::from_millis(target_ms)
}

async fn stream_jpeg_data_channel_frames(
    state: AppState,
    udid: String,
    session: crate::simulators::session::SimulatorSession,
    first_frame: crate::transport::packet::SharedFrame,
    channel: Arc<RTCDataChannel>,
) {
    let mut rx = session.subscribe();
    let _guard = WebRtcMetricsGuard::new(state.metrics.clone());

    if let Err(error) = write_jpeg_frame_chunks(&channel, &first_frame).await {
        warn!("WebRTC JPEG first frame write failed for {udid}: {error}");
        return;
    }
    let mut last_sequence = first_frame.frame_sequence;
    state.metrics.frames_sent.fetch_add(1, Ordering::Relaxed);

    loop {
        if channel.ready_state() != RTCDataChannelState::Open {
            break;
        }

        let frame = match rx.recv().await {
            Ok(frame) => frame,
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                state
                    .metrics
                    .frames_dropped_server
                    .fetch_add(skipped, Ordering::Relaxed);
                continue;
            }
            Err(broadcast::error::RecvError::Closed) => break,
        };
        let (frame, skipped) = freshest_available_frame(frame, &mut rx);
        if skipped > 0 {
            state
                .metrics
                .frames_dropped_server
                .fetch_add(skipped, Ordering::Relaxed);
        }
        if frame
            .codec
            .as_deref()
            .is_some_and(|codec| !codec.eq_ignore_ascii_case("jpeg"))
        {
            warn!("WebRTC JPEG channel received non-JPEG frame for {udid}");
            break;
        }

        let buffered = channel.buffered_amount().await;
        let max_buffered = (frame.data.len() * JPEG_CHANNEL_BUFFERED_FRAME_MULTIPLIER)
            .max(JPEG_CHANNEL_MIN_BUFFERED_BYTES);
        if buffered > max_buffered {
            state
                .metrics
                .frames_dropped_server
                .fetch_add(1 + skipped, Ordering::Relaxed);
            continue;
        }

        if last_sequence != 0 && frame.frame_sequence > last_sequence + 1 {
            state
                .metrics
                .frames_dropped_server
                .fetch_add(frame.frame_sequence - last_sequence - 1, Ordering::Relaxed);
        }

        if let Err(error) = write_jpeg_frame_chunks(&channel, &frame).await {
            warn!("WebRTC JPEG frame write failed for {udid}: {error}");
            break;
        }
        last_sequence = frame.frame_sequence;
        state.metrics.frames_sent.fetch_add(1, Ordering::Relaxed);
    }
}

async fn write_jpeg_frame_chunks(
    channel: &RTCDataChannel,
    frame: &crate::transport::packet::FramePacket,
) -> anyhow::Result<()> {
    let payload = frame.data.as_ref();
    let chunk_count = payload.len().div_ceil(JPEG_CHUNK_PAYLOAD_BYTES);
    if chunk_count == 0 || chunk_count > u16::MAX as usize {
        anyhow::bail!("invalid JPEG frame chunk count {chunk_count}");
    }

    for chunk_index in 0..chunk_count {
        let start = chunk_index * JPEG_CHUNK_PAYLOAD_BYTES;
        let end = (start + JPEG_CHUNK_PAYLOAD_BYTES).min(payload.len());
        let chunk_payload = &payload[start..end];
        let mut message = Vec::with_capacity(JPEG_CHUNK_HEADER_BYTES + chunk_payload.len());
        message.extend_from_slice(JPEG_CHUNK_MAGIC);
        message.push(JPEG_CHUNK_VERSION);
        message.push(if frame.is_keyframe { 1 } else { 0 });
        message.extend_from_slice(&(chunk_index as u16).to_be_bytes());
        message.extend_from_slice(&(chunk_count as u16).to_be_bytes());
        message.extend_from_slice(&0u16.to_be_bytes());
        message.extend_from_slice(&frame.frame_sequence.to_be_bytes());
        message.extend_from_slice(&frame.timestamp_us.to_be_bytes());
        message.extend_from_slice(&frame.width.to_be_bytes());
        message.extend_from_slice(&frame.height.to_be_bytes());
        message.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        message.extend_from_slice(chunk_payload);
        channel.send(&Bytes::from(message)).await?;
    }
    Ok(())
}

async fn write_frame_sample(
    video_track: &TrackLocalStaticSample,
    frame: &crate::transport::packet::SharedFrame,
    duration: Duration,
) -> anyhow::Result<()> {
    let data = h264_annex_b_sample(frame)?;
    video_track
        .write_sample(&Sample {
            data: Bytes::from(data),
            duration,
            ..Default::default()
        })
        .await?;
    Ok(())
}

fn h264_annex_b_sample(frame: &crate::transport::packet::FramePacket) -> anyhow::Result<Vec<u8>> {
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

    let nal_length_size = description.and_then(avcc_nal_length_size).unwrap_or(4);
    append_length_prefixed_nalus(data, nal_length_size, &mut sample)?;
    Ok(sample)
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
        anyhow::bail!("invalid H.264 NAL length size {nal_length_size}");
    }

    let mut offset = 0usize;
    while offset < data.len() {
        if offset + nal_length_size > data.len() {
            anyhow::bail!("truncated H.264 NAL length prefix");
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
            anyhow::bail!("truncated H.264 NAL unit");
        }
        output.extend_from_slice(ANNEX_B_START_CODE);
        output.extend_from_slice(&data[offset..offset + length]);
        offset += length;
    }
    Ok(())
}

fn freshest_available_frame(
    mut frame: crate::transport::packet::SharedFrame,
    rx: &mut broadcast::Receiver<crate::transport::packet::SharedFrame>,
) -> (crate::transport::packet::SharedFrame, u64) {
    let mut skipped = 0u64;
    loop {
        match rx.try_recv() {
            Ok(next) => {
                skipped += 1;
                frame = next;
            }
            Err(TryRecvError::Lagged(count)) => {
                skipped += count;
            }
            Err(TryRecvError::Empty) | Err(TryRecvError::Closed) => return (frame, skipped),
        }
    }
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

    fn duration_for(&mut self, frame: &crate::transport::packet::FramePacket) -> Duration {
        const MIN_FRAME_DURATION_US: u64 = 1_000;
        const DEFAULT_FRAME_DURATION_US: u64 = 16_667;
        const MAX_FRAME_DURATION_US: u64 = 100_000;

        let duration_us = self
            .last_timestamp_us
            .and_then(|previous| frame.timestamp_us.checked_sub(previous))
            .filter(|duration| *duration > 0)
            .unwrap_or(DEFAULT_FRAME_DURATION_US)
            .clamp(MIN_FRAME_DURATION_US, MAX_FRAME_DURATION_US);
        self.last_timestamp_us = Some(frame.timestamp_us);
        Duration::from_micros(duration_us)
    }
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
        append_avcc_parameter_sets, append_length_prefixed_nalus, h264_sdp_fmtp_line, is_annex_b,
        is_h264_codec, ANNEX_B_START_CODE,
    };

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
        assert!(h264_sdp_fmtp_line("avc1.42e01f").contains("profile-level-id=42e01f"));
        assert!(h264_sdp_fmtp_line("h264").contains("profile-level-id=42e01f"));
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
}
