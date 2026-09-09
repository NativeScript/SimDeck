//! Software H.264 encoding for builds without the macOS native bridge.
//!
//! macOS encodes Android emulator frames through `XCWH264Encoder` (VideoToolbox
//! or x264) behind the native bridge C ABI. Windows and Linux link
//! `native_stubs.c` instead, so this module does the same job in Rust with
//! Cisco's OpenH264, compiled from source through the `openh264` crate. It
//! produces Annex B baseline H.264, which is what the WebRTC packetizer in
//! `transport::webrtc` and every browser H.264 decoder expect.
//!
//! The encoder is created lazily from the first frame because the output size
//! is only known once the emulator delivers a frame, and it is recreated
//! whenever the size or the quality-derived bitrate changes.

use crate::android::AndroidH264StreamQuality;
use crate::error::AppError;
use bytes::Bytes;
use openh264::encoder::{
    BitRate, Complexity, Encoder, EncoderConfig, FrameRate, FrameType, IntraFramePeriod, Profile,
    RateControlMode, SpsPpsStrategy, UsageType,
};
use openh264::formats::{BgraSliceU8, RgbaSliceU8, YUVBuffer};
use openh264::OpenH264API;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

/// Name reported through `/api/health` and encoder stats.
pub(crate) const SOFTWARE_H264_ENCODER_NAME: &str = "openh264";
/// Matches the macOS realtime keyframe interval; viewers request keyframes on
/// demand through RTCP, so periodic IDR frames only bound recovery time.
const KEYFRAME_INTERVAL_SECONDS: u32 = 60;
const MIN_BITRATE_BPS: u32 = 200_000;
const MAX_BITRATE_BPS: u32 = 60_000_000;
const DEFAULT_MIN_BITRATE_BPS: u32 = 3_000_000;
const DEFAULT_BITS_PER_PIXEL: u32 = 5;
const MAX_ENCODER_THREADS: usize = 4;
const BYTES_PER_PIXEL: usize = 4;

/// Encoder parameters derived from the stream quality and the frame size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SoftwareH264Settings {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) fps: u32,
    pub(crate) bitrate_bps: u32,
    pub(crate) keyframe_interval_frames: u32,
    pub(crate) threads: u16,
}

/// One encoded access unit in Annex B form.
pub(crate) struct EncodedH264Frame {
    pub(crate) is_keyframe: bool,
    pub(crate) data: Bytes,
}

pub(crate) struct SoftwareH264Encoder {
    quality: AndroidH264StreamQuality,
    fps: u32,
    threads: u16,
    encoder: Option<(SoftwareH264Settings, Encoder)>,
    yuv: Option<YUVBuffer>,
    yuv_dimensions: (u32, u32),
    keyframe_pending: bool,
    reinitializations: u64,
    output_frames: u64,
    keyframe_outputs: u64,
    skipped_frames: u64,
    encode_failures: u64,
    latest_encode_us: u64,
    average_encode_us: f64,
    peak_encode_us: u64,
}

impl SoftwareH264Encoder {
    pub(crate) fn new(quality: AndroidH264StreamQuality, fps: u32) -> Self {
        Self {
            quality,
            fps: fps.max(1),
            threads: encoder_threads(),
            encoder: None,
            yuv: None,
            yuv_dimensions: (0, 0),
            keyframe_pending: true,
            reinitializations: 0,
            output_frames: 0,
            keyframe_outputs: 0,
            skipped_frames: 0,
            encode_failures: 0,
            latest_encode_us: 0,
            average_encode_us: 0.0,
            peak_encode_us: 0,
        }
    }

    /// Applies a new quality target. The encoder is rebuilt on the next frame
    /// when the derived bitrate or frame rate changes, and always emits a
    /// keyframe so viewers resynchronize.
    pub(crate) fn reconfigure(&mut self, quality: AndroidH264StreamQuality, fps: u32) {
        self.quality = quality;
        self.fps = fps.max(1);
        self.keyframe_pending = true;
    }

    pub(crate) fn request_keyframe(&mut self) {
        self.keyframe_pending = true;
    }

    pub(crate) fn encode_rgba(
        &mut self,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> Result<Option<EncodedH264Frame>, AppError> {
        validate_frame(rgba.len(), width, height)?;
        self.prepare_yuv(width, height);
        if let Some(yuv) = self.yuv.as_mut() {
            yuv.read_rgba8(RgbaSliceU8::new(rgba, (width as usize, height as usize)));
        }
        self.encode_prepared(width, height)
    }

    pub(crate) fn encode_bgra(
        &mut self,
        bgra: &[u8],
        width: u32,
        height: u32,
    ) -> Result<Option<EncodedH264Frame>, AppError> {
        validate_frame(bgra.len(), width, height)?;
        self.prepare_yuv(width, height);
        if let Some(yuv) = self.yuv.as_mut() {
            yuv.read_bgra8(BgraSliceU8::new(bgra, (width as usize, height as usize)));
        }
        self.encode_prepared(width, height)
    }

    pub(crate) fn stats(&self) -> Value {
        let settings = self.encoder.as_ref().map(|(settings, _)| *settings);
        json!({
            "encoder": SOFTWARE_H264_ENCODER_NAME,
            "hardware": false,
            "transportCodec": "h264",
            "profile": "baseline",
            "width": settings.map(|settings| settings.width),
            "height": settings.map(|settings| settings.height),
            "fps": settings.map(|settings| settings.fps),
            "bitrateBps": settings.map(|settings| settings.bitrate_bps),
            "keyFrameIntervalFrames": settings.map(|settings| settings.keyframe_interval_frames),
            "threads": self.threads,
            "reinitializations": self.reinitializations,
            "outputFrames": self.output_frames,
            "keyFrameOutputs": self.keyframe_outputs,
            "skippedFrames": self.skipped_frames,
            "encodeFailures": self.encode_failures,
            "keyFramePending": self.keyframe_pending,
            "latestEncodeLatencyUs": self.latest_encode_us,
            "averageEncodeLatencyUs": self.average_encode_us.round() as u64,
            "peakEncodeLatencyUs": self.peak_encode_us,
        })
    }

    fn prepare_yuv(&mut self, width: u32, height: u32) {
        if self.yuv.is_none() || self.yuv_dimensions != (width, height) {
            self.yuv = Some(YUVBuffer::new(width as usize, height as usize));
            self.yuv_dimensions = (width, height);
        }
    }

    fn ensure_encoder(&mut self, settings: SoftwareH264Settings) -> Result<(), AppError> {
        if matches!(&self.encoder, Some((current, _)) if *current == settings) {
            return Ok(());
        }
        let encoder =
            Encoder::with_api_config(OpenH264API::from_source(), encoder_config(settings))
                .map_err(|error| {
                    AppError::native(format!("OpenH264 encoder creation failed: {error}"))
                })?;
        if self.encoder.is_some() {
            self.reinitializations += 1;
        }
        self.encoder = Some((settings, encoder));
        self.keyframe_pending = true;
        Ok(())
    }

    fn encode_prepared(
        &mut self,
        width: u32,
        height: u32,
    ) -> Result<Option<EncodedH264Frame>, AppError> {
        let settings = software_h264_settings(self.quality, self.fps, self.threads, width, height);
        self.ensure_encoder(settings)?;
        let force_keyframe = std::mem::take(&mut self.keyframe_pending);
        let started = Instant::now();
        let encoded = {
            let (Some((_, encoder)), Some(yuv)) = (self.encoder.as_mut(), self.yuv.as_ref()) else {
                return Err(AppError::native(
                    "OpenH264 encoder state is missing a frame buffer.",
                ));
            };
            if force_keyframe {
                encoder.force_intra_frame();
            }
            encoder
                .encode(yuv)
                .map(|bitstream| (bitstream.frame_type(), bitstream.to_vec()))
        };
        let (frame_type, data) = match encoded {
            Ok(encoded) => encoded,
            Err(error) => {
                self.encode_failures += 1;
                self.keyframe_pending = true;
                return Err(AppError::native(format!("OpenH264 encode error: {error}")));
            }
        };
        self.record_encode_duration(started.elapsed());
        if data.is_empty() || matches!(frame_type, FrameType::Skip | FrameType::Invalid) {
            self.skipped_frames += 1;
            if force_keyframe {
                self.keyframe_pending = true;
            }
            return Ok(None);
        }
        let is_keyframe = matches!(frame_type, FrameType::IDR);
        self.output_frames += 1;
        if is_keyframe {
            self.keyframe_outputs += 1;
        }
        Ok(Some(EncodedH264Frame {
            is_keyframe,
            data: Bytes::from(data),
        }))
    }

    fn record_encode_duration(&mut self, duration: Duration) {
        let micros = duration.as_micros().min(u128::from(u64::MAX)) as u64;
        self.latest_encode_us = micros;
        self.peak_encode_us = self.peak_encode_us.max(micros);
        self.average_encode_us = if self.output_frames == 0 && self.skipped_frames == 0 {
            micros as f64
        } else {
            self.average_encode_us * 0.9 + micros as f64 * 0.1
        };
    }
}

/// Mirrors `XCWAverageBitRateForDimensions` in the macOS encoder: budget a
/// number of bits per pixel per second, never below the configured minimum.
pub(crate) fn software_h264_settings(
    quality: AndroidH264StreamQuality,
    fps: u32,
    threads: u16,
    width: u32,
    height: u32,
) -> SoftwareH264Settings {
    let fps = fps.max(1);
    let bits_per_pixel = quality
        .bits_per_pixel
        .unwrap_or(DEFAULT_BITS_PER_PIXEL)
        .max(1);
    let min_bitrate = quality.min_bitrate.unwrap_or(DEFAULT_MIN_BITRATE_BPS);
    let computed = u64::from(width) * u64::from(height) * u64::from(bits_per_pixel);
    let bitrate_bps = computed
        .max(u64::from(min_bitrate))
        .clamp(u64::from(MIN_BITRATE_BPS), u64::from(MAX_BITRATE_BPS)) as u32;
    SoftwareH264Settings {
        width,
        height,
        fps,
        bitrate_bps,
        keyframe_interval_frames: fps.saturating_mul(KEYFRAME_INTERVAL_SECONDS).max(1),
        threads,
    }
}

fn encoder_config(settings: SoftwareH264Settings) -> EncoderConfig {
    EncoderConfig::new()
        .usage_type(UsageType::ScreenContentRealTime)
        .profile(Profile::Baseline)
        .rate_control_mode(RateControlMode::Bitrate)
        .bitrate(BitRate::from_bps(settings.bitrate_bps))
        .max_frame_rate(FrameRate::from_hz(settings.fps as f32))
        .intra_frame_period(IntraFramePeriod::from_num_frames(
            settings.keyframe_interval_frames,
        ))
        .skip_frames(false)
        .complexity(Complexity::Low)
        .sps_pps_strategy(SpsPpsStrategy::ConstantId)
        .num_threads(settings.threads)
}

fn encoder_threads() -> u16 {
    std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1)
        .div_ceil(2)
        .clamp(1, MAX_ENCODER_THREADS) as u16
}

fn validate_frame(length: usize, width: u32, height: u32) -> Result<(), AppError> {
    if width < 2 || height < 2 || !width.is_multiple_of(2) || !height.is_multiple_of(2) {
        return Err(AppError::native(format!(
            "OpenH264 needs even frame dimensions; got {width}x{height}."
        )));
    }
    let expected = width as usize * height as usize * BYTES_PER_PIXEL;
    if length != expected {
        return Err(AppError::native(format!(
            "Android frame buffer holds {length} bytes but {width}x{height} needs {expected}."
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quality(min_bitrate: u32, bits_per_pixel: u32) -> AndroidH264StreamQuality {
        AndroidH264StreamQuality {
            max_edge: Some(960),
            fps: Some(30),
            min_bitrate: Some(min_bitrate),
            bits_per_pixel: Some(bits_per_pixel),
        }
    }

    fn solid_rgba(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
        let mut frame = Vec::with_capacity(width as usize * height as usize * 4);
        for _ in 0..(width * height) {
            frame.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
        }
        frame
    }

    fn nal_unit_types(annex_b: &[u8]) -> Vec<u8> {
        let mut types = Vec::new();
        let mut index = 0;
        while index + 3 < annex_b.len() {
            let four = annex_b[index..index + 4] == [0, 0, 0, 1];
            let three = annex_b[index..index + 3] == [0, 0, 1];
            if four || three {
                let header = index + if four { 4 } else { 3 };
                if header < annex_b.len() {
                    types.push(annex_b[header] & 0x1f);
                }
                index = header;
            } else {
                index += 1;
            }
        }
        types
    }

    #[test]
    fn settings_budget_bits_per_pixel_above_the_minimum() {
        let settings = software_h264_settings(quality(1_000_000, 5), 30, 2, 432, 960);
        assert_eq!(settings.bitrate_bps, 432 * 960 * 5);
        assert_eq!(settings.fps, 30);
        assert_eq!(settings.keyframe_interval_frames, 30 * 60);
        assert_eq!(settings.threads, 2);
    }

    #[test]
    fn settings_respect_the_minimum_and_the_ceiling() {
        let floor = software_h264_settings(quality(6_000_000, 1), 60, 1, 64, 64);
        assert_eq!(floor.bitrate_bps, 6_000_000);
        let ceiling = software_h264_settings(quality(1_000_000, 10), 60, 1, 4096, 4096);
        assert_eq!(ceiling.bitrate_bps, MAX_BITRATE_BPS);
        let defaults = software_h264_settings(Default::default(), 0, 1, 64, 64);
        assert_eq!(defaults.fps, 1);
        assert_eq!(defaults.bitrate_bps, DEFAULT_MIN_BITRATE_BPS);
    }

    #[test]
    fn frames_must_be_even_and_fully_populated() {
        assert!(validate_frame(64 * 64 * 4, 64, 64).is_ok());
        assert!(validate_frame(63 * 64 * 4, 63, 64).is_err());
        assert!(validate_frame(64 * 64 * 4 - 1, 64, 64).is_err());
        assert!(validate_frame(0, 0, 0).is_err());
    }

    #[test]
    fn encodes_annex_b_keyframes_and_honors_keyframe_requests() {
        let mut encoder = SoftwareH264Encoder::new(quality(500_000, 2), 30);
        let first = encoder
            .encode_rgba(&solid_rgba(64, 64, [200, 30, 30]), 64, 64)
            .expect("first encode")
            .expect("first frame is emitted");
        assert!(first.is_keyframe);
        assert!(first.data.starts_with(&[0, 0, 0, 1]) || first.data.starts_with(&[0, 0, 1]));
        let types = nal_unit_types(&first.data);
        assert!(types.contains(&7), "keyframe carries SPS: {types:?}");
        assert!(types.contains(&8), "keyframe carries PPS: {types:?}");
        assert!(
            types.contains(&5),
            "keyframe carries an IDR slice: {types:?}"
        );

        let second = encoder
            .encode_rgba(&solid_rgba(64, 64, [30, 200, 30]), 64, 64)
            .expect("second encode");
        if let Some(second) = second {
            assert!(!second.is_keyframe);
        }

        encoder.request_keyframe();
        let third = encoder
            .encode_bgra(&solid_rgba(64, 64, [30, 30, 200]), 64, 64)
            .expect("third encode")
            .expect("forced keyframe is emitted");
        assert!(third.is_keyframe);
        assert!(nal_unit_types(&third.data).contains(&5));

        let stats = encoder.stats();
        assert_eq!(stats["encoder"], Value::String("openh264".to_owned()));
        assert_eq!(stats["width"], Value::from(64));
        assert!(stats["keyFrameOutputs"].as_u64().unwrap_or(0) >= 2);
        assert_eq!(stats["encodeFailures"], Value::from(0));
    }

    #[test]
    fn resizing_and_reconfiguring_rebuilds_the_encoder() {
        let mut encoder = SoftwareH264Encoder::new(quality(500_000, 2), 30);
        encoder
            .encode_rgba(&solid_rgba(64, 64, [0, 0, 0]), 64, 64)
            .expect("first size");
        let resized = encoder
            .encode_rgba(&solid_rgba(96, 64, [0, 0, 0]), 96, 64)
            .expect("second size")
            .expect("resized keyframe is emitted");
        assert!(resized.is_keyframe);
        assert_eq!(encoder.stats()["reinitializations"], Value::from(1));

        encoder.reconfigure(quality(4_000_000, 4), 60);
        let reconfigured = encoder
            .encode_rgba(&solid_rgba(96, 64, [0, 0, 0]), 96, 64)
            .expect("reconfigured encode")
            .expect("reconfigured keyframe is emitted");
        assert!(reconfigured.is_keyframe);
        assert_eq!(encoder.stats()["reinitializations"], Value::from(2));
        assert_eq!(encoder.stats()["fps"], Value::from(60));
    }
}
