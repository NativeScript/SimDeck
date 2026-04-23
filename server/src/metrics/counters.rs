use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

const CLIENT_STREAM_STATS_LIMIT: usize = 48;

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
        self.client_stream_stats
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .cloned()
            .collect()
    }
}
