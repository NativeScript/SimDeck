use crate::api::routes::AppState;
use crate::error::AppError;
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use tracing::warn;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::{MediaEngine, MIME_TYPE_H264};
use webrtc::api::APIBuilder;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::interceptor::registry::Registry;
use webrtc::media::Sample;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;
use webrtc::track::track_local::TrackLocal;

const DEFAULT_STUN_URL: &str = "stun:stun.l.google.com:19302";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebRtcOfferPayload {
    pub sdp: String,
    #[serde(rename = "type")]
    pub kind: String,
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
    session.request_refresh();

    let first_frame = session
        .wait_for_keyframe(Duration::from_secs(3))
        .await
        .ok_or_else(|| AppError::native("Timed out waiting for a simulator keyframe."))?;
    let codec = first_frame
        .codec
        .as_deref()
        .unwrap_or_default()
        .to_lowercase();
    if !codec.contains("h264") {
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

    let video_track = Arc::new(TrackLocalStaticSample::new(
        RTCRtpCodecCapability {
            mime_type: MIME_TYPE_H264.to_owned(),
            clock_rate: 90_000,
            channels: 0,
            sdp_fmtp_line: "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f"
                .to_owned(),
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
    let _guard = WebRtcMetricsGuard::new(state.metrics.clone());
    if let Err(error) = write_frame_sample(&video_track, &first_frame).await {
        warn!("failed to write initial WebRTC frame for {udid}: {error}");
    }

    loop {
        let frame = match rx.recv().await {
            Ok(frame) => frame,
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                state
                    .metrics
                    .frames_dropped_server
                    .fetch_add(skipped, Ordering::Relaxed);
                session.request_refresh();
                continue;
            }
            Err(broadcast::error::RecvError::Closed) => break,
        };
        if let Err(error) = write_frame_sample(&video_track, &frame).await {
            warn!("WebRTC frame write failed for {udid}: {error}");
            break;
        }
        state.metrics.frames_sent.fetch_add(1, Ordering::Relaxed);
    }

    let _ = peer_connection.close().await;
}

async fn write_frame_sample(
    video_track: &TrackLocalStaticSample,
    frame: &crate::transport::packet::SharedFrame,
) -> anyhow::Result<()> {
    let mut data = Vec::new();
    if frame.is_keyframe {
        if let Some(description) = frame.description.as_ref() {
            data.extend_from_slice(description.as_slice());
        }
    }
    data.extend_from_slice(frame.data.as_slice());
    video_track
        .write_sample(&Sample {
            data: Bytes::from(data),
            duration: Duration::from_millis(16),
            ..Default::default()
        })
        .await?;
    Ok(())
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
