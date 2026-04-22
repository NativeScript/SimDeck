use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default)]
pub struct Metrics {
    pub frames_encoded: AtomicU64,
    pub keyframes_encoded: AtomicU64,
    pub frames_sent: AtomicU64,
    pub frames_dropped_server: AtomicU64,
    pub keyframe_requests: AtomicU64,
    pub subscribers_connected: AtomicU64,
    pub subscribers_disconnected: AtomicU64,
    pub max_send_queue_depth: AtomicU64,
    pub latest_first_frame_ms: AtomicU64,
}

#[derive(Debug, Serialize)]
pub struct MetricsSnapshot {
    pub frames_encoded: u64,
    pub keyframes_encoded: u64,
    pub frames_sent: u64,
    pub frames_dropped_server: u64,
    pub keyframe_requests: u64,
    pub subscribers_connected: u64,
    pub subscribers_disconnected: u64,
    pub avg_send_queue_depth: f64,
    pub max_send_queue_depth: u64,
    pub latest_first_frame_ms: u64,
}

impl Metrics {
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            frames_encoded: self.frames_encoded.load(Ordering::Relaxed),
            keyframes_encoded: self.keyframes_encoded.load(Ordering::Relaxed),
            frames_sent: self.frames_sent.load(Ordering::Relaxed),
            frames_dropped_server: self.frames_dropped_server.load(Ordering::Relaxed),
            keyframe_requests: self.keyframe_requests.load(Ordering::Relaxed),
            subscribers_connected: self.subscribers_connected.load(Ordering::Relaxed),
            subscribers_disconnected: self.subscribers_disconnected.load(Ordering::Relaxed),
            avg_send_queue_depth: 1.0,
            max_send_queue_depth: self.max_send_queue_depth.load(Ordering::Relaxed),
            latest_first_frame_ms: self.latest_first_frame_ms.load(Ordering::Relaxed),
        }
    }
}
