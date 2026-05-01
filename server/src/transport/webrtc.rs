use crate::api::routes::{run_control_message, AppState, ControlMessage};
use crate::error::AppError;
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::time;
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
const WEBRTC_BOOTSTRAP_KEYFRAME_INTERVAL: Duration = Duration::from_millis(150);
const WEBRTC_BOOTSTRAP_KEYFRAME_REPEATS: u8 = 8;
const WEBRTC_MIN_REFRESH_INTERVAL: Duration = Duration::from_millis(16);
const WEBRTC_MAX_REFRESH_INTERVAL: Duration = Duration::from_millis(100);
const WEBRTC_LOW_LATENCY_REFRESH_INTERVAL: Duration = Duration::from_millis(67);
const WEBRTC_LOW_LATENCY_MAX_REFRESH_INTERVAL: Duration = Duration::from_millis(250);
const WEBRTC_WRITE_TIMEOUT: Duration = Duration::from_millis(120);
const WEBRTC_REALTIME_WRITE_TIMEOUT: Duration = Duration::from_millis(45);
const WEBRTC_REALTIME_KEYFRAME_WRITE_TIMEOUT: Duration = Duration::from_millis(90);
const WEBRTC_RTP_OUTBOUND_MTU: usize = 1200;
static WEBRTC_MEDIA_STREAMS: OnceLock<Mutex<HashMap<String, Vec<broadcast::Sender<()>>>>> =
    OnceLock::new();
const MAX_WEBRTC_MEDIA_STREAMS_PER_UDID: usize = 3;

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
    if payload.transport.is_some() {
        return Err(AppError::bad_request(
            "WebRTC preview supports media tracks only.",
        ));
    }
    session.request_keyframe();
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

    let first_frame = session
        .wait_for_keyframe(Duration::from_secs(3))
        .await
        .ok_or_else(|| AppError::native("Timed out waiting for a simulator keyframe."))?;
    let codec = first_frame
        .codec
        .as_deref()
        .unwrap_or_default()
        .to_lowercase();
    if !is_h264_codec(&codec) {
        return Err(AppError::bad_request(
            "WebRTC preview requires H.264. Restart SimDeck with `--video-codec h264-software` or `h264`.",
        ));
    }

    let h264_fmtp_line = h264_sdp_fmtp_line(&codec);
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
    register_control_data_channel(&peer_connection, session.clone(), udid.clone());

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

    let (cancellation_token, cancellation) = register_webrtc_media_stream(&udid);
    tokio::spawn(
        WebRtcMediaStream {
            state,
            udid,
            session,
            first_frame,
            peer_connection,
            video_track,
            cancellation_token,
            cancellation,
        }
        .run(),
    );

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

fn h264_rtcp_feedback() -> Vec<RTCPFeedback> {
    let mut feedback = vec![
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
            parameter: "pli".to_owned(),
        },
    ];
    if !realtime_stream_enabled() {
        feedback.insert(
            3,
            RTCPFeedback {
                typ: "nack".to_owned(),
                parameter: String::new(),
            },
        );
    }
    feedback
}

fn register_webrtc_media_stream(udid: &str) -> (broadcast::Sender<()>, broadcast::Receiver<()>) {
    let (tx, rx) = broadcast::channel(1);
    let streams = WEBRTC_MEDIA_STREAMS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut streams = streams.lock().unwrap();
    let active_streams = streams.entry(udid.to_owned()).or_default();
    while active_streams.len() >= MAX_WEBRTC_MEDIA_STREAMS_PER_UDID {
        if let Some(stale_stream) = active_streams.first().cloned() {
            let _ = stale_stream.send(());
        }
        active_streams.remove(0);
    }
    active_streams.push(tx.clone());
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
    register_webrtc_media_stream(udid)
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
            active_streams.retain(|current| !current.same_channel(token));
            if active_streams.is_empty() {
                streams.remove(udid);
            }
        }
    }
}

#[cfg(test)]
pub fn cancel_media_stream(udid: &str) -> bool {
    let Some(streams) = WEBRTC_MEDIA_STREAMS.get() else {
        return false;
    };
    let Some(active_streams) = streams.lock().unwrap().get(udid).cloned() else {
        return false;
    };
    for stream in &active_streams {
        let _ = stream.send(());
    }
    true
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

struct WebRtcMediaStream {
    state: AppState,
    session: crate::simulators::session::SimulatorSession,
    udid: String,
    first_frame: crate::transport::packet::SharedFrame,
    peer_connection: Arc<webrtc::peer_connection::RTCPeerConnection>,
    video_track: Arc<TrackLocalStaticRTP>,
    cancellation_token: broadcast::Sender<()>,
    cancellation: broadcast::Receiver<()>,
}

impl WebRtcMediaStream {
    async fn run(self) {
        let Self {
            state,
            session,
            udid,
            first_frame,
            peer_connection,
            video_track,
            cancellation_token,
            mut cancellation,
        } = self;
        let mut rx = session.subscribe();
        let mut latest_keyframe = first_frame.clone();
        let mut send_timing = WebRtcSendTiming::new();
        let mut peer_state_interval = time::interval(Duration::from_millis(250));
        let mut bootstrap_sleep = Box::pin(time::sleep(WEBRTC_BOOTSTRAP_KEYFRAME_INTERVAL));
        let realtime_stream = realtime_stream_enabled();
        let mut packetizer = new_packetizer(
            WEBRTC_RTP_OUTBOUND_MTU,
            96,
            0,
            Box::<H264Payloader>::default(),
            Box::new(new_random_sequencer()),
            90_000,
        );
        let refresh_floor = refresh_floor_for_low_latency(state.config.low_latency);
        let refresh_ceiling = refresh_ceiling_for_low_latency(state.config.low_latency);
        let mut refresh_sleep = Box::pin(time::sleep(refresh_floor));
        let mut adaptive_refresh_interval = refresh_floor;
        let mut bootstrap_frames_remaining = WEBRTC_BOOTSTRAP_KEYFRAME_REPEATS;
        let mut waiting_for_keyframe = false;
        let _guard = WebRtcMetricsGuard::new(state.metrics.clone());

        match write_frame_sample_with_timeout(
            &video_track,
            &mut packetizer,
            &first_frame,
            refresh_floor,
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
                session.request_keyframe();
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
                }
                _ = &mut bootstrap_sleep, if bootstrap_frames_remaining > 0 => {
                    match write_frame_sample_with_timeout(
                        &video_track,
                        &mut packetizer,
                        &latest_keyframe,
                        WEBRTC_BOOTSTRAP_KEYFRAME_INTERVAL,
                        realtime_stream,
                    ).await {
                        Ok(true) => {
                            state.metrics.frames_sent.fetch_add(1, Ordering::Relaxed);
                        }
                        Ok(false) => {
                            state
                                .metrics
                                .frames_dropped_server
                                .fetch_add(1, Ordering::Relaxed);
                            session.request_keyframe();
                        }
                        Err(error) => {
                            warn!("WebRTC bootstrap keyframe write failed for {udid}: {error}");
                            break;
                        }
                    }
                    bootstrap_frames_remaining = bootstrap_frames_remaining.saturating_sub(1);
                    bootstrap_sleep
                        .as_mut()
                        .reset(time::Instant::now() + WEBRTC_BOOTSTRAP_KEYFRAME_INTERVAL);
                }
                _ = &mut refresh_sleep => {
                    session.request_refresh();
                    refresh_sleep
                        .as_mut()
                        .reset(time::Instant::now() + adaptive_refresh_interval);
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
                        Err(broadcast::error::RecvError::Closed) => {
                            warn!("WebRTC media stream closing for {udid}: frame channel closed");
                            break;
                        }
                    };
                    let (frame, stale_frames) = drain_to_latest_frame(&mut rx, frame, &state.metrics);
                    if stale_frames > 0 {
                        session.request_keyframe();
                    }
                    if stale_frames > 0 && !realtime_stream && !frame.is_keyframe {
                        waiting_for_keyframe = true;
                        continue;
                    }
                    if !realtime_stream && waiting_for_keyframe && !frame.is_keyframe {
                        state.metrics.frames_dropped_server.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                    if frame.is_keyframe {
                        latest_keyframe = frame.clone();
                        waiting_for_keyframe = false;
                    }
                    let duration = send_timing.duration_for(&frame, realtime_stream);
                    let started_at = time::Instant::now();
                    let write_result = write_frame_sample_with_timeout(
                        &video_track,
                        &mut packetizer,
                        &frame,
                        duration,
                        realtime_stream,
                    )
                    .await;
                    adaptive_refresh_interval =
                        adaptive_interval_for_write(started_at.elapsed(), refresh_floor, refresh_ceiling);
                    match write_result {
                        Ok(true) => {
                            state.metrics.frames_sent.fetch_add(1, Ordering::Relaxed);
                        }
                        Ok(false) => {
                            state
                                .metrics
                                .frames_dropped_server
                                .fetch_add(1, Ordering::Relaxed);
                            waiting_for_keyframe = !realtime_stream;
                            adaptive_refresh_interval = refresh_ceiling;
                            session.request_keyframe();
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
        let _ = peer_connection.close().await;
    }
}

fn drain_to_latest_frame(
    rx: &mut broadcast::Receiver<crate::transport::packet::SharedFrame>,
    mut frame: crate::transport::packet::SharedFrame,
    metrics: &Arc<crate::metrics::counters::Metrics>,
) -> (crate::transport::packet::SharedFrame, u64) {
    let mut stale_frames = 0u64;
    loop {
        match rx.try_recv() {
            Ok(next_frame) => {
                stale_frames += 1;
                frame = next_frame;
            }
            Err(broadcast::error::TryRecvError::Lagged(skipped)) => {
                stale_frames = stale_frames.saturating_add(skipped);
            }
            Err(broadcast::error::TryRecvError::Empty | broadcast::error::TryRecvError::Closed) => {
                break;
            }
        }
    }
    if stale_frames > 0 {
        metrics
            .frames_dropped_server
            .fetch_add(stale_frames, Ordering::Relaxed);
    }
    (frame, stale_frames)
}

fn refresh_floor_for_low_latency(low_latency: bool) -> Duration {
    if low_latency {
        WEBRTC_LOW_LATENCY_REFRESH_INTERVAL
    } else {
        WEBRTC_MIN_REFRESH_INTERVAL
    }
}

fn refresh_ceiling_for_low_latency(low_latency: bool) -> Duration {
    if low_latency {
        WEBRTC_LOW_LATENCY_MAX_REFRESH_INTERVAL
    } else {
        WEBRTC_MAX_REFRESH_INTERVAL
    }
}

fn adaptive_interval_for_write(
    write_elapsed: Duration,
    refresh_floor: Duration,
    refresh_ceiling: Duration,
) -> Duration {
    let target_ms = (write_elapsed.as_millis() as u64).saturating_mul(2).clamp(
        refresh_floor.as_millis() as u64,
        refresh_ceiling.as_millis() as u64,
    );
    Duration::from_millis(target_ms)
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
    let pacing = realtime_packet_pacing(duration, packet_count, realtime_stream);
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
    let timeout = if realtime_stream {
        if frame.is_keyframe {
            WEBRTC_REALTIME_KEYFRAME_WRITE_TIMEOUT
        } else {
            WEBRTC_REALTIME_WRITE_TIMEOUT.max(realtime_sample_duration() * 2)
        }
    } else {
        WEBRTC_WRITE_TIMEOUT
    };
    match time::timeout(
        timeout,
        write_frame_sample(video_track, packetizer, frame, duration, realtime_stream),
    )
    .await
    {
        Ok(result) => result.map(|()| true),
        Err(_) => Ok(false),
    }
}

fn realtime_packet_pacing(
    duration: Duration,
    packet_count: usize,
    realtime_stream: bool,
) -> Option<(usize, Duration)> {
    if !realtime_stream || packet_count <= 1 {
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
    ) -> Duration {
        if realtime_stream {
            self.last_timestamp_us = Some(frame.timestamp_us);
            return realtime_sample_duration();
        }

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

fn realtime_sample_duration() -> Duration {
    let fps = std::env::var("SIMDECK_REALTIME_FPS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(30)
        .clamp(15, 60);
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
    fn registering_second_webrtc_stream_does_not_cancel_first() {
        let udid = format!("test-{}", std::process::id());
        super::reset_webrtc_media_streams_for_test(&udid);
        let (first_token, mut first_rx) = super::register_webrtc_media_stream_for_test(&udid);
        let (second_token, mut second_rx) = super::register_webrtc_media_stream_for_test(&udid);

        assert!(super::has_media_stream(&udid));
        assert!(first_rx.try_recv().is_err());
        assert!(second_rx.try_recv().is_err());

        assert!(super::cancel_media_stream(&udid));
        assert!(first_rx.try_recv().is_ok());
        assert!(second_rx.try_recv().is_ok());

        super::clear_webrtc_media_stream_for_test(&udid, &first_token);
        super::clear_webrtc_media_stream_for_test(&udid, &second_token);
        assert!(!super::has_media_stream(&udid));
    }

    #[test]
    fn registering_fourth_webrtc_stream_cancels_oldest() {
        let udid = format!("test-cap-{}", std::process::id());
        super::reset_webrtc_media_streams_for_test(&udid);
        let (_first_token, mut first_rx) = super::register_webrtc_media_stream_for_test(&udid);
        let (second_token, mut second_rx) = super::register_webrtc_media_stream_for_test(&udid);
        let (third_token, mut third_rx) = super::register_webrtc_media_stream_for_test(&udid);
        let (fourth_token, mut fourth_rx) = super::register_webrtc_media_stream_for_test(&udid);

        assert!(first_rx.try_recv().is_ok());
        assert!(second_rx.try_recv().is_err());
        assert!(third_rx.try_recv().is_err());
        assert!(fourth_rx.try_recv().is_err());
        assert_eq!(super::active_webrtc_media_stream_count(&udid), 3);

        super::clear_webrtc_media_stream_for_test(&udid, &second_token);
        super::clear_webrtc_media_stream_for_test(&udid, &third_token);
        super::clear_webrtc_media_stream_for_test(&udid, &fourth_token);
        assert!(!super::has_media_stream(&udid));
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
