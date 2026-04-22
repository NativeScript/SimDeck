use crate::api::routes::AppState;
use crate::metrics::counters::Metrics;
use crate::simulators::session::SimulatorSession;
use crate::transport::packet::{ControlHello, ForeignBytes, SharedFrame, PACKET_VERSION};
use anyhow::Context;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::warn;
use wtransport::endpoint::endpoint_side::Server;
use wtransport::{Endpoint, Identity, ServerConfig};

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
    let Some(udid) = path.strip_prefix("/wt/simulators/") else {
        anyhow::bail!("unexpected WebTransport path: {path}");
    };
    let session = state.registry.get_or_create_async(udid).await?;
    session.ensure_started_async().await?;
    session.request_refresh_async().await;

    let hello_frame = session.wait_for_keyframe(Duration::from_secs(3)).await;
    let Some(hello_frame_ref) = hello_frame.as_ref() else {
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
    let mut rx = session.subscribe();

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
        rx.changed().await?;
        let Some(frame) = rx.borrow().clone() else {
            continue;
        };

        if last_sequence != 0 && frame.frame_sequence > last_sequence + 1 && !frame.is_keyframe {
            state
                .metrics
                .frames_dropped_server
                .fetch_add(1, Ordering::Relaxed);
            session.request_refresh();
            continue;
        }

        let discontinuity = last_sequence != 0 && frame.frame_sequence > last_sequence + 1;
        send_frame(&mut video, &session, &state.metrics, &frame, discontinuity).await?;
        last_sequence = frame.frame_sequence;

        if !first_frame_sent {
            first_frame_sent = true;
            state
                .metrics
                .latest_first_frame_ms
                .store(started_at.elapsed().as_millis() as u64, Ordering::Relaxed);
        }
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
        .map(ForeignBytes::as_slice)
        .unwrap_or(&[]);
    stream.write_all(&header).await?;
    if !description.is_empty() {
        stream.write_all(description).await?;
    }
    let data = frame.data.as_slice();
    if !data.is_empty() {
        stream.write_all(data).await?;
    }
    metrics.frames_sent.fetch_add(1, Ordering::Relaxed);
    if discontinuity {
        session.request_refresh();
    }
    Ok(())
}
