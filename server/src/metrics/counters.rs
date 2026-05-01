use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

const CLIENT_STREAM_STATS_LIMIT: usize = 48;
const CLIENT_STREAM_STATS_TTL_MS: f64 = 15_000.0;

#[derive(Default)]
pub struct Metrics {
    pub frames_encoded: AtomicU64,
    pub keyframes_encoded: AtomicU64,
    pub frames_sent: AtomicU64,
    pub frames_dropped_server: AtomicU64,
    pub keyframe_requests: AtomicU64,
    pub active_streams: AtomicU64,
    pub subscribers_connected: AtomicU64,
    pub subscribers_disconnected: AtomicU64,
    pub max_send_queue_depth: AtomicU64,
    pub latest_first_frame_ms: AtomicU64,
    client_stream_stats: Mutex<VecDeque<ClientStreamStats>>,
}

#[derive(Debug, Serialize)]
pub struct MetricsSnapshot {
    pub frames_encoded: u64,
    pub keyframes_encoded: u64,
    pub frames_sent: u64,
    pub frames_dropped_server: u64,
    pub keyframe_requests: u64,
    pub active_streams: u64,
    pub subscribers_connected: u64,
    pub subscribers_disconnected: u64,
    pub avg_send_queue_depth: f64,
    pub max_send_queue_depth: u64,
    pub latest_first_frame_ms: u64,
    pub client_streams: Vec<ClientStreamStats>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientStreamStats {
    pub client_id: String,
    pub kind: String,
    pub timestamp_ms: Option<f64>,
    pub udid: Option<String>,
    pub connection_id: Option<u64>,
    pub status: Option<String>,
    pub detail: Option<String>,
    pub error: Option<String>,
    pub ice_connection_state: Option<String>,
    pub peer_connection_state: Option<String>,
    pub ice_gathering_state: Option<String>,
    pub signaling_state: Option<String>,
    pub local_candidate_summary: Option<String>,
    pub remote_candidate_summary: Option<String>,
    pub selected_candidate_pair: Option<String>,
    pub url: Option<String>,
    pub user_agent: Option<String>,
    pub visibility_state: Option<String>,
    pub focused: Option<bool>,
    pub codec: Option<String>,
    pub width: Option<u64>,
    pub height: Option<u64>,
    pub received_packets: Option<u64>,
    pub decoded_frames: Option<u64>,
    pub rendered_frames: Option<u64>,
    pub dropped_frames: Option<u64>,
    pub reconnects: Option<u64>,
    pub frame_sequence: Option<u64>,
    pub decode_queue_size: Option<u64>,
    pub waiting_for_key_frame: Option<bool>,
    pub packet_fps: Option<f64>,
    pub decoded_fps: Option<f64>,
    pub dropped_fps: Option<f64>,
    pub page_fps: Option<f64>,
    pub app_fps: Option<f64>,
    pub latest_render_ms: Option<f64>,
    pub max_render_ms: Option<f64>,
    pub average_render_ms: Option<f64>,
    pub latest_frame_gap_ms: Option<f64>,
}

impl ClientStreamStats {
    fn key(&self) -> (&str, &str) {
        (&self.client_id, &self.kind)
    }
}

impl Metrics {
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            frames_encoded: self.frames_encoded.load(Ordering::Relaxed),
            keyframes_encoded: self.keyframes_encoded.load(Ordering::Relaxed),
            frames_sent: self.frames_sent.load(Ordering::Relaxed),
            frames_dropped_server: self.frames_dropped_server.load(Ordering::Relaxed),
            keyframe_requests: self.keyframe_requests.load(Ordering::Relaxed),
            active_streams: self.active_streams.load(Ordering::Relaxed),
            subscribers_connected: self.subscribers_connected.load(Ordering::Relaxed),
            subscribers_disconnected: self.subscribers_disconnected.load(Ordering::Relaxed),
            avg_send_queue_depth: 1.0,
            max_send_queue_depth: self.max_send_queue_depth.load(Ordering::Relaxed),
            latest_first_frame_ms: self.latest_first_frame_ms.load(Ordering::Relaxed),
            client_streams: self.client_stream_stats_snapshot(),
        }
    }

    pub fn record_client_stream_stats(&self, stats: ClientStreamStats) {
        let mut snapshots = self
            .client_stream_stats
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        prune_stale_client_stream_stats(&mut snapshots);

        if let Some(existing) = snapshots.iter_mut().find(|existing| {
            let (client_id, kind) = existing.key();
            let (next_client_id, next_kind) = stats.key();
            client_id == next_client_id && kind == next_kind
        }) {
            *existing = stats;
        } else {
            snapshots.push_back(stats);
        }

        while snapshots.len() > CLIENT_STREAM_STATS_LIMIT {
            snapshots.pop_front();
        }
    }

    pub fn client_stream_stats_snapshot(&self) -> Vec<ClientStreamStats> {
        let mut snapshots = self
            .client_stream_stats
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        prune_stale_client_stream_stats(&mut snapshots);
        snapshots.iter().cloned().collect()
    }
}

fn prune_stale_client_stream_stats(snapshots: &mut VecDeque<ClientStreamStats>) {
    let now_ms = current_time_ms();
    snapshots.retain(|stats| {
        stats
            .timestamp_ms
            .is_some_and(|timestamp| now_ms - timestamp <= CLIENT_STREAM_STATS_TTL_MS)
    });
}

fn current_time_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::{current_time_ms, ClientStreamStats, Metrics};

    fn stats(client_id: &str, kind: &str, timestamp_ms: Option<f64>) -> ClientStreamStats {
        ClientStreamStats {
            client_id: client_id.to_owned(),
            kind: kind.to_owned(),
            timestamp_ms,
            udid: None,
            connection_id: None,
            status: None,
            detail: None,
            error: None,
            ice_connection_state: None,
            peer_connection_state: None,
            ice_gathering_state: None,
            signaling_state: None,
            local_candidate_summary: None,
            remote_candidate_summary: None,
            selected_candidate_pair: None,
            url: None,
            user_agent: None,
            visibility_state: None,
            focused: None,
            codec: None,
            width: None,
            height: None,
            received_packets: None,
            decoded_frames: None,
            rendered_frames: None,
            dropped_frames: None,
            reconnects: None,
            frame_sequence: None,
            decode_queue_size: None,
            waiting_for_key_frame: None,
            packet_fps: None,
            decoded_fps: None,
            dropped_fps: None,
            page_fps: None,
            app_fps: None,
            latest_render_ms: None,
            max_render_ms: None,
            average_render_ms: None,
            latest_frame_gap_ms: None,
        }
    }

    #[test]
    fn client_stream_stats_replace_matching_client_and_kind() {
        let metrics = Metrics::default();
        let now = current_time_ms();
        let mut first = stats("client-1", "webrtc", Some(now));
        first.status = Some("connecting".to_owned());
        let mut second = stats("client-1", "webrtc", Some(now + 1.0));
        second.status = Some("connected".to_owned());

        metrics.record_client_stream_stats(first);
        metrics.record_client_stream_stats(second);

        let snapshots = metrics.client_stream_stats_snapshot();
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].status.as_deref(), Some("connected"));
    }

    #[test]
    fn client_stream_stats_keep_distinct_kinds_for_same_client() {
        let metrics = Metrics::default();
        let now = current_time_ms();

        metrics.record_client_stream_stats(stats("client-1", "webrtc", Some(now)));
        metrics.record_client_stream_stats(stats("client-1", "worker", Some(now)));

        let snapshots = metrics.client_stream_stats_snapshot();
        assert_eq!(snapshots.len(), 2);
    }

    #[test]
    fn client_stream_stats_prune_missing_and_stale_timestamps() {
        let metrics = Metrics::default();
        let now = current_time_ms();

        metrics.record_client_stream_stats(stats("missing", "webrtc", None));
        metrics.record_client_stream_stats(stats("stale", "webrtc", Some(now - 20_000.0)));
        metrics.record_client_stream_stats(stats("fresh", "webrtc", Some(now)));

        let snapshots = metrics.client_stream_stats_snapshot();
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].client_id, "fresh");
    }

    #[test]
    fn client_stream_stats_keep_latest_limit() {
        let metrics = Metrics::default();
        let now = current_time_ms();

        for index in 0..60 {
            metrics.record_client_stream_stats(stats(
                &format!("client-{index}"),
                "webrtc",
                Some(now + index as f64),
            ));
        }

        let snapshots = metrics.client_stream_stats_snapshot();
        assert_eq!(snapshots.len(), 48);
        assert_eq!(snapshots[0].client_id, "client-12");
        assert_eq!(snapshots[47].client_id, "client-59");
    }
}
