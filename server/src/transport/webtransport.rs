use crate::api::routes::AppState;
use crate::auth;
use crate::metrics::counters::Metrics;
use crate::simulators::session::SimulatorSession;
use crate::transport::packet::{ControlHello, SharedFrame, PACKET_VERSION};
use anyhow::Context;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use tracing::warn;
use wtransport::endpoint::endpoint_side::Server;
use wtransport::{Endpoint, Identity, ServerConfig, VarInt};

static WEBTRANSPORT_MEDIA_STREAMS: OnceLock<Mutex<HashMap<String, broadcast::Sender<()>>>> =
    OnceLock::new();

pub struct WebTransportRuntime {
    pub endpoint_url_template: String,
    pub certificate_hash_hex: String,
}

pub async fn prepare(
    config: &crate::config::Config,
) -> anyhow::Result<(WebTransportRuntime, Endpoint<Server>)> {
    let identity = Identity::self_signed(config.certificate_subject_alt_names())?;
    let certificate_hash_hex =
        hex::encode(identity.certificate_chain().as_slice()[0].hash().as_ref());

    let server_config = ServerConfig::builder()
        .with_bind_address(config.wt_addr())
        .with_identity(identity)
        .keep_alive_interval(Some(Duration::from_secs(3)))
        .build();

    let endpoint = Endpoint::server(server_config)?;
    let runtime = WebTransportRuntime {
        endpoint_url_template: config.wt_endpoint_template(),
        certificate_hash_hex,
    };
    Ok((runtime, endpoint))
}

pub async fn serve(endpoint: Endpoint<Server>, state: AppState) -> anyhow::Result<()> {
    loop {
        let incoming_session = endpoint.accept().await;
        let incoming_request = incoming_session.await?;
        let path = incoming_request.path().to_owned();
        let origin = incoming_request.origin().map(str::to_owned);
        if !auth::webtransport_request_authorized(&state.config, &path, origin.as_deref()) {
            incoming_request.forbidden().await;
            warn!("rejected unauthorized WebTransport session: {path}");
            continue;
        }
        let session = incoming_request.accept().await?;
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(error) = handle_session(state, path, session).await {
                warn!("webtransport session failed: {error:#}");
            }
        });
    }
}

async fn handle_session(
    state: AppState,
    path: String,
    connection: wtransport::Connection,
) -> anyhow::Result<()> {
    let path_without_query = auth::strip_query(&path);
    let Some(udid) = path_without_query.strip_prefix("/wt/simulators/") else {
        anyhow::bail!("unexpected WebTransport path: {path}");
    };
    let session = started_session(&state, udid).await?;
    session.request_refresh();
    let (cancellation_token, mut cancellation) = replace_webtransport_media_stream(udid);
    let _stream_cancellation =
        WebTransportMediaStreamGuard::new(udid.to_owned(), cancellation_token);

    let hello_frame = session.wait_for_keyframe(Duration::from_secs(3)).await;
    let Some(hello_frame_ref) = hello_frame.as_ref() else {
        state.registry.remove(udid);
        anyhow::bail!("timed out waiting for initial simulator keyframe for {udid}");
    };
    let width = hello_frame_ref.width;
    let height = hello_frame_ref.height;
    let hello = ControlHello {
        version: PACKET_VERSION,
        simulator_udid: udid.to_owned(),
        width,
        height,
        codec: hello_frame_ref.codec.clone(),
        packet_format: "binary-video-v1",
    };

    let mut control = connection
        .open_uni()
        .await
        .context("open control stream")?
        .await
        .context("accept control stream")?;
    control
        .write_all(serde_json::to_vec(&hello)?.as_slice())
        .await
        .context("write control hello")?;
    control.finish().await.context("finish control hello")?;

    let mut video = connection
        .open_uni()
        .await
        .context("open video stream")?
        .await
        .context("accept video stream")?;
    let started_at = Instant::now();
    let mut first_frame_sent = false;
    let mut last_sequence = 0u64;
    let mut waiting_for_keyframe = false;
    let mut rx = session.subscribe();
    let _stream_metrics = StreamMetricsGuard::new(state.metrics.clone());

    if let Some(frame) = hello_frame {
        send_frame(&mut video, &session, &state.metrics, &frame, true).await?;
        last_sequence = frame.frame_sequence;
        first_frame_sent = true;
        state
            .metrics
            .latest_first_frame_ms
            .store(started_at.elapsed().as_millis() as u64, Ordering::Relaxed);
    }

    loop {
        let frame = tokio::select! {
            _ = cancellation.recv() => {
                connection.close(VarInt::from_u32(0), b"stream replaced");
                return Ok(());
            }
            result = rx.recv() => match result {
                Ok(frame) => frame,
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    state
                        .metrics
                        .frames_dropped_server
                        .fetch_add(skipped, Ordering::Relaxed);
                    waiting_for_keyframe = true;
                    last_sequence = 0;
                    session.request_refresh();
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => anyhow::bail!("frame stream closed"),
            },
        };

        if waiting_for_keyframe && !frame.is_keyframe {
            state
                .metrics
                .frames_dropped_server
                .fetch_add(1, Ordering::Relaxed);
            continue;
        }

        if last_sequence != 0 && frame.frame_sequence > last_sequence + 1 && !frame.is_keyframe {
            state
                .metrics
                .frames_dropped_server
                .fetch_add(1, Ordering::Relaxed);
            waiting_for_keyframe = true;
            last_sequence = 0;
            session.request_refresh();
            continue;
        }

        let discontinuity = last_sequence != 0 && frame.frame_sequence > last_sequence + 1;
        let discontinuity = discontinuity || waiting_for_keyframe;
        send_frame(&mut video, &session, &state.metrics, &frame, discontinuity).await?;
        last_sequence = frame.frame_sequence;
        waiting_for_keyframe = false;

        if !first_frame_sent {
            first_frame_sent = true;
            state
                .metrics
                .latest_first_frame_ms
                .store(started_at.elapsed().as_millis() as u64, Ordering::Relaxed);
        }
    }
}

fn replace_webtransport_media_stream(
    udid: &str,
) -> (broadcast::Sender<()>, broadcast::Receiver<()>) {
    let (tx, rx) = broadcast::channel(1);
    let streams = WEBTRANSPORT_MEDIA_STREAMS.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(previous) = streams.lock().unwrap().insert(udid.to_owned(), tx.clone()) {
        let _ = previous.send(());
    }
    (tx, rx)
}

fn clear_webtransport_media_stream(udid: &str, token: &broadcast::Sender<()>) {
    if let Some(streams) = WEBTRANSPORT_MEDIA_STREAMS.get() {
        let mut streams = streams.lock().unwrap();
        if streams
            .get(udid)
            .is_some_and(|current| current.same_channel(token))
        {
            streams.remove(udid);
        }
    }
}

pub fn cancel_media_stream(udid: &str) -> bool {
    let Some(streams) = WEBTRANSPORT_MEDIA_STREAMS.get() else {
        return false;
    };
    let Some(stream) = streams.lock().unwrap().get(udid).cloned() else {
        return false;
    };
    let _ = stream.send(());
    true
}

pub fn has_media_stream(udid: &str) -> bool {
    WEBTRANSPORT_MEDIA_STREAMS
        .get()
        .is_some_and(|streams| streams.lock().unwrap().contains_key(udid))
}

struct WebTransportMediaStreamGuard {
    udid: String,
    token: broadcast::Sender<()>,
}

impl WebTransportMediaStreamGuard {
    fn new(udid: String, token: broadcast::Sender<()>) -> Self {
        Self { udid, token }
    }
}

impl Drop for WebTransportMediaStreamGuard {
    fn drop(&mut self) {
        clear_webtransport_media_stream(&self.udid, &self.token);
    }
}

struct StreamMetricsGuard {
    metrics: Arc<Metrics>,
}

impl StreamMetricsGuard {
    fn new(metrics: Arc<Metrics>) -> Self {
        metrics
            .subscribers_connected
            .fetch_add(1, Ordering::Relaxed);
        let active_streams = metrics.active_streams.fetch_add(1, Ordering::Relaxed) + 1;
        metrics
            .max_send_queue_depth
            .fetch_max(active_streams, Ordering::Relaxed);
        Self { metrics }
    }
}

impl Drop for StreamMetricsGuard {
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

async fn send_frame(
    stream: &mut wtransport::SendStream,
    session: &SimulatorSession,
    metrics: &Arc<Metrics>,
    frame: &SharedFrame,
    discontinuity: bool,
) -> anyhow::Result<()> {
    let header = frame.header_bytes(discontinuity);
    let description = frame
        .description
        .as_ref()
        .map(bytes::Bytes::as_ref)
        .unwrap_or(&[]);
    stream.write_all(&header).await?;
    if !description.is_empty() {
        stream.write_all(description).await?;
    }
    let data = frame.data.as_ref();
    if !data.is_empty() {
        stream.write_all(data).await?;
    }
    metrics.frames_sent.fetch_add(1, Ordering::Relaxed);
    if discontinuity && !frame.is_keyframe {
        session.request_refresh();
    }
    Ok(())
}

async fn started_session(state: &AppState, udid: &str) -> anyhow::Result<SimulatorSession> {
    let session = state.registry.get_or_create_async(udid).await?;
    match session.ensure_started_async().await {
        Ok(()) => Ok(session),
        Err(first_error) => {
            state.registry.remove(udid);
            let session = state.registry.get_or_create_async(udid).await?;
            session.ensure_started_async().await.with_context(|| {
                format!("failed to start fresh simulator display session after: {first_error}")
            })?;
            Ok(session)
        }
    }
}
