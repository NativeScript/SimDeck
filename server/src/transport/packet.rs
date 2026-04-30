use bytes::Bytes;
use std::sync::Arc;

#[derive(Debug)]
pub struct FramePacket {
    pub frame_sequence: u64,
    pub timestamp_us: u64,
    pub is_keyframe: bool,
    pub width: u32,
    pub height: u32,
    pub codec: Option<String>,
    pub description: Option<Bytes>,
    pub data: Bytes,
}

pub type SharedFrame = Arc<FramePacket>;
