use bytes::Bytes;
use serde::Serialize;
use std::sync::Arc;

pub const PACKET_VERSION: u8 = 1;
pub const FLAG_KEYFRAME: u8 = 1 << 0;
pub const FLAG_CONFIG: u8 = 1 << 1;
pub const FLAG_DISCONTINUITY: u8 = 1 << 2;
pub const PACKET_HEADER_BYTES: usize = 36;

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

impl FramePacket {
    pub fn header_bytes(&self, discontinuity: bool) -> [u8; PACKET_HEADER_BYTES] {
        let description_length = self.description.as_ref().map_or(0, Bytes::len);
        let mut flags = 0u8;
        if self.is_keyframe {
            flags |= FLAG_KEYFRAME;
        }
        if description_length > 0 {
            flags |= FLAG_CONFIG;
        }
        if discontinuity {
            flags |= FLAG_DISCONTINUITY;
        }

        let mut out = [0u8; PACKET_HEADER_BYTES];
        out[0] = PACKET_VERSION;
        out[1] = flags;
        out[2..4].copy_from_slice(&0u16.to_be_bytes());
        out[4..12].copy_from_slice(&self.frame_sequence.to_be_bytes());
        out[12..20].copy_from_slice(&self.timestamp_us.to_be_bytes());
        out[20..24].copy_from_slice(&self.width.to_be_bytes());
        out[24..28].copy_from_slice(&self.height.to_be_bytes());
        out[28..32].copy_from_slice(&(description_length as u32).to_be_bytes());
        out[32..36].copy_from_slice(&(self.data.len() as u32).to_be_bytes());
        out
    }
}

#[derive(Debug, Serialize)]
pub struct ControlHello {
    pub version: u8,
    pub simulator_udid: String,
    pub width: u32,
    pub height: u32,
    pub codec: Option<String>,
    pub packet_format: &'static str,
}

pub type SharedFrame = Arc<FramePacket>;
