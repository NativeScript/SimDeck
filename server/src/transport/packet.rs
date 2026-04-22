use crate::native::ffi;
use serde::Serialize;
use std::ffi::c_void;
use std::fmt;
use std::sync::Arc;

pub const PACKET_VERSION: u8 = 1;
pub const FLAG_KEYFRAME: u8 = 1 << 0;
pub const FLAG_CONFIG: u8 = 1 << 1;
pub const FLAG_DISCONTINUITY: u8 = 1 << 2;
pub const PACKET_HEADER_BYTES: usize = 36;

pub struct ForeignBytes {
    data: *const u8,
    length: usize,
    owner: *const c_void,
}

impl ForeignBytes {
    pub unsafe fn from_ffi(bytes: ffi::xcw_native_shared_bytes) -> Option<Self> {
        if bytes.data.is_null() || bytes.length == 0 {
            if !bytes.owner.is_null() {
                unsafe {
                    ffi::xcw_native_release_shared_bytes(bytes);
                }
            }
            return None;
        }

        Some(Self {
            data: bytes.data,
            length: bytes.length,
            owner: bytes.owner,
        })
    }

    pub fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.data, self.length) }
    }

    pub fn len(&self) -> usize {
        self.length
    }
}

impl AsRef<[u8]> for ForeignBytes {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl Drop for ForeignBytes {
    fn drop(&mut self) {
        unsafe {
            ffi::xcw_native_release_shared_bytes(ffi::xcw_native_shared_bytes {
                data: self.data,
                length: self.length,
                owner: self.owner,
            });
        }
    }
}

impl fmt::Debug for ForeignBytes {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ForeignBytes")
            .field("length", &self.length)
            .finish()
    }
}

unsafe impl Send for ForeignBytes {}
unsafe impl Sync for ForeignBytes {}

#[derive(Debug)]
pub struct FramePacket {
    pub frame_sequence: u64,
    pub timestamp_us: u64,
    pub is_keyframe: bool,
    pub width: u32,
    pub height: u32,
    pub codec: Option<String>,
    pub description: Option<ForeignBytes>,
    pub data: ForeignBytes,
}

impl FramePacket {
    pub fn header_bytes(&self, discontinuity: bool) -> [u8; PACKET_HEADER_BYTES] {
        let description_length = self.description.as_ref().map_or(0, ForeignBytes::len);
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
