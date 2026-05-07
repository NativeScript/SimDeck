use crate::error::AppError;
use crate::metrics::counters::Metrics;
use crate::native::bridge::{NativeBridge, NativeSession};
use crate::native::ffi;
use crate::simulators::state::SessionState;
use crate::transport::packet::{FramePacket, JpegFramePacket, SharedFrame, SharedJpegFrame};
use bytes::Bytes;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;
use tokio::task;
use tokio::time::{sleep, timeout, Instant};
use tracing::debug;

// This channel carries encoded H.264 access units. Subscribers must not miss
// ordinary P-frames: dropping compressed references creates decoder artifacts
// even on a perfect localhost link. Coalescing is only safe before encoding.
const FRAME_BROADCAST_CAPACITY: usize = 128;
const MIN_REFRESH_INTERVAL_MS: u64 = 16;
const MIN_KEYFRAME_INTERVAL_MS: u64 = 250;
const DEFAULT_SHARED_REFRESH_FPS: u64 = 60;
const MIN_SHARED_REFRESH_FPS: u64 = 15;
const MAX_SHARED_REFRESH_FPS: u64 = 240;
const JPEG_FRAME_BROADCAST_CAPACITY: usize = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JpegStreamConfig {
    pub max_edge: u32,
    pub quality_percent: u32,
}

pub struct SimulatorSession {
    inner: Arc<SimulatorSessionInner>,
    callback_user_data: usize,
    jpeg_callback_user_data: usize,
}

struct SimulatorSessionInner {
    udid: String,
    native: NativeSession,
    metrics: Arc<Metrics>,
    sender: broadcast::Sender<SharedFrame>,
    jpeg_sender: broadcast::Sender<SharedJpegFrame>,
    latest_keyframe: RwLock<Option<SharedFrame>>,
    state: Mutex<SessionState>,
    start_condvar: Condvar,
    display_ready: AtomicBool,
    display_width: AtomicU64,
    display_height: AtomicU64,
    frame_sequence: AtomicU64,
    last_refresh_ms: AtomicU64,
    last_keyframe_ms: AtomicU64,
    active_frame_subscribers: AtomicU64,
    active_jpeg_subscribers: AtomicU64,
    jpeg_config: Mutex<Option<JpegStreamConfig>>,
    refresh_pump_running: AtomicBool,
}

pub struct FrameSubscription {
    inner: Arc<SimulatorSessionInner>,
    receiver: broadcast::Receiver<SharedFrame>,
}

pub struct JpegFrameSubscription {
    inner: Arc<SimulatorSessionInner>,
    receiver: broadcast::Receiver<SharedJpegFrame>,
}

impl JpegFrameSubscription {
    pub async fn recv(&mut self) -> Result<SharedJpegFrame, broadcast::error::RecvError> {
        self.receiver.recv().await
    }
}

impl Drop for JpegFrameSubscription {
    fn drop(&mut self) {
        if self
            .inner
            .active_jpeg_subscribers
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                Some(value.saturating_sub(1))
            })
            .unwrap_or(0)
            <= 1
        {
            *self.inner.jpeg_config.lock().unwrap() = None;
            unsafe {
                self.inner
                    .native
                    .set_jpeg_frame_callback(None, std::ptr::null_mut(), 0, 0.0);
            }
        }
    }
}

impl FrameSubscription {
    pub async fn recv(&mut self) -> Result<SharedFrame, broadcast::error::RecvError> {
        self.receiver.recv().await
    }
}

impl Drop for FrameSubscription {
    fn drop(&mut self) {
        self.inner
            .active_frame_subscribers
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                Some(value.saturating_sub(1))
            })
            .ok();
    }
}

impl SimulatorSession {
    pub fn udid(&self) -> &str {
        &self.inner.udid
    }

    pub fn new(
        bridge: &NativeBridge,
        udid: String,
        metrics: Arc<Metrics>,
    ) -> Result<Self, AppError> {
        let native = bridge.create_session(&udid)?;
        let (sender, _) = broadcast::channel(FRAME_BROADCAST_CAPACITY);
        let (jpeg_sender, _) = broadcast::channel(JPEG_FRAME_BROADCAST_CAPACITY);
        let inner = Arc::new(SimulatorSessionInner {
            udid,
            native,
            metrics,
            sender,
            jpeg_sender,
            latest_keyframe: RwLock::new(None),
            state: Mutex::new(SessionState::Detached),
            start_condvar: Condvar::new(),
            display_ready: AtomicBool::new(false),
            display_width: AtomicU64::new(0),
            display_height: AtomicU64::new(0),
            frame_sequence: AtomicU64::new(0),
            last_refresh_ms: AtomicU64::new(0),
            last_keyframe_ms: AtomicU64::new(0),
            active_frame_subscribers: AtomicU64::new(0),
            active_jpeg_subscribers: AtomicU64::new(0),
            jpeg_config: Mutex::new(None),
            refresh_pump_running: AtomicBool::new(false),
        });

        let user_data = Weak::into_raw(Arc::downgrade(&inner)) as *mut c_void;
        let jpeg_user_data = Weak::into_raw(Arc::downgrade(&inner)) as *mut c_void;
        unsafe {
            inner
                .native
                .set_frame_callback(Some(native_frame_callback), user_data);
        }

        Ok(Self {
            inner,
            callback_user_data: user_data as usize,
            jpeg_callback_user_data: jpeg_user_data as usize,
        })
    }

    pub fn ensure_started(&self) -> Result<(), AppError> {
        loop {
            let mut state = self.inner.state.lock().unwrap();
            match *state {
                SessionState::Ready | SessionState::Streaming => return Ok(()),
                SessionState::Attaching => {
                    drop(self.inner.start_condvar.wait(state).unwrap());
                }
                _ => {
                    *state = SessionState::Attaching;
                    break;
                }
            }
        }

        if let Err(error) = self.inner.native.start() {
            *self.inner.state.lock().unwrap() = SessionState::Failed;
            self.inner.start_condvar.notify_all();
            return Err(error);
        }
        *self.inner.state.lock().unwrap() = SessionState::Ready;
        self.inner.start_condvar.notify_all();
        Ok(())
    }

    pub async fn ensure_started_async(&self) -> Result<(), AppError> {
        let session = self.clone();
        task::spawn_blocking(move || session.ensure_started())
            .await
            .map_err(|error| AppError::internal(format!("Failed to join start task: {error}")))?
    }

    pub fn subscribe(&self) -> FrameSubscription {
        *self.inner.state.lock().unwrap() = SessionState::Streaming;
        self.inner
            .active_frame_subscribers
            .fetch_add(1, Ordering::Relaxed);
        self.inner.start_refresh_pump();
        FrameSubscription {
            inner: self.inner.clone(),
            receiver: self.inner.sender.subscribe(),
        }
    }

    pub fn subscribe_jpeg(&self, config: JpegStreamConfig) -> JpegFrameSubscription {
        *self.inner.state.lock().unwrap() = SessionState::Streaming;
        let previous_subscribers = self
            .inner
            .active_jpeg_subscribers
            .fetch_add(1, Ordering::Relaxed);
        let mut current_config = self.inner.jpeg_config.lock().unwrap();
        if current_config.is_none() || previous_subscribers == 0 {
            *current_config = Some(config);
            unsafe {
                self.inner.native.set_jpeg_frame_callback(
                    Some(native_jpeg_frame_callback),
                    self.jpeg_callback_user_data as *mut c_void,
                    config.max_edge,
                    (config.quality_percent as f64 / 100.0).clamp(0.2, 0.95),
                );
            }
        }
        drop(current_config);
        self.request_refresh();
        JpegFrameSubscription {
            inner: self.inner.clone(),
            receiver: self.inner.jpeg_sender.subscribe(),
        }
    }

    pub fn update_jpeg_config(&self, config: JpegStreamConfig) {
        if self.inner.active_jpeg_subscribers.load(Ordering::Relaxed) == 0 {
            return;
        }
        let mut current_config = self.inner.jpeg_config.lock().unwrap();
        if current_config.as_ref() == Some(&config) {
            return;
        }
        *current_config = Some(config);
        unsafe {
            self.inner.native.set_jpeg_frame_callback(
                Some(native_jpeg_frame_callback),
                self.jpeg_callback_user_data as *mut c_void,
                config.max_edge,
                (config.quality_percent as f64 / 100.0).clamp(0.2, 0.95),
            );
        }
    }

    pub fn latest_keyframe(&self) -> Option<SharedFrame> {
        self.inner.latest_keyframe.read().unwrap().clone()
    }

    pub async fn wait_for_keyframe(&self, timeout_duration: Duration) -> Option<SharedFrame> {
        let deadline = Instant::now() + timeout_duration;
        let baseline_sequence = self
            .latest_keyframe()
            .map_or(0, |frame| frame.frame_sequence);
        let mut rx = self.inner.sender.subscribe();
        self.request_keyframe_immediate();

        loop {
            if let Some(frame) = self.latest_keyframe() {
                if frame.frame_sequence > baseline_sequence {
                    return Some(frame);
                }
            }

            let now = Instant::now();
            if now >= deadline {
                return None;
            }

            let remaining = deadline - now;
            match timeout(remaining, rx.recv()).await {
                Ok(Ok(frame)) if frame.is_keyframe && frame.frame_sequence > baseline_sequence => {
                    return Some(frame)
                }
                Ok(Ok(_)) => self.request_keyframe(),
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => {
                    self.request_keyframe();
                }
                Ok(Err(_)) | Err(_) => return None,
            }
        }
    }

    pub fn request_refresh(&self) {
        self.inner.request_refresh();
    }

    pub fn request_keyframe(&self) {
        let now = now_ms();
        let previous = self.inner.last_keyframe_ms.load(Ordering::Relaxed);
        if now.saturating_sub(previous) < MIN_KEYFRAME_INTERVAL_MS {
            self.request_refresh();
            return;
        }
        self.inner.last_keyframe_ms.store(now, Ordering::Relaxed);
        self.inner.last_refresh_ms.store(now, Ordering::Relaxed);
        self.inner
            .metrics
            .keyframe_requests
            .fetch_add(1, Ordering::Relaxed);
        self.inner.native.request_keyframe();
    }

    fn request_keyframe_immediate(&self) {
        let now = now_ms();
        self.inner.last_keyframe_ms.store(now, Ordering::Relaxed);
        self.inner.last_refresh_ms.store(now, Ordering::Relaxed);
        self.inner
            .metrics
            .keyframe_requests
            .fetch_add(1, Ordering::Relaxed);
        self.inner.native.request_keyframe();
    }

    pub fn reconfigure_video_encoder(&self) {
        self.inner.last_keyframe_ms.store(0, Ordering::Relaxed);
        self.inner.last_refresh_ms.store(0, Ordering::Relaxed);
        self.inner.native.reconfigure_video_encoder();
    }

    pub fn send_touch(&self, x: f64, y: f64, phase: &str) -> Result<(), AppError> {
        self.inner.native.send_touch(x, y, phase)
    }

    pub fn send_key(&self, key_code: u16, modifiers: u32) -> Result<(), AppError> {
        self.inner.native.send_key(key_code, modifiers)
    }

    pub fn press_home(&self) -> Result<(), AppError> {
        self.inner.native.press_home()
    }

    pub fn open_app_switcher(&self) -> Result<(), AppError> {
        self.inner.native.open_app_switcher()
    }

    pub fn rotate_left(&self) -> Result<(), AppError> {
        self.inner.native.rotate_left()
    }

    pub fn rotate_right(&self) -> Result<(), AppError> {
        self.inner.native.rotate_right()
    }

    pub fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "displayReady": self.inner.display_ready.load(Ordering::Relaxed),
            "displayStatus": self.inner.state.lock().unwrap().as_str(),
            "displayWidth": self.inner.display_width.load(Ordering::Relaxed),
            "displayHeight": self.inner.display_height.load(Ordering::Relaxed),
            "frameSequence": self.inner.frame_sequence.load(Ordering::Relaxed),
            "rotationQuarterTurns": self.inner.native.rotation_quarter_turns(),
            "encoder": self.inner.native.video_encoder_stats(),
        })
    }
}

impl Drop for SimulatorSession {
    fn drop(&mut self) {
        if Arc::strong_count(&self.inner) == 1 {
            *self.inner.state.lock().unwrap() = SessionState::ShuttingDown;
            unsafe {
                self.inner
                    .native
                    .set_frame_callback(None, std::ptr::null_mut());
                self.inner
                    .native
                    .set_jpeg_frame_callback(None, std::ptr::null_mut(), 0, 0.0);
                let _ = Weak::from_raw(self.callback_user_data as *const SimulatorSessionInner);
                let _ =
                    Weak::from_raw(self.jpeg_callback_user_data as *const SimulatorSessionInner);
            }
        }
    }
}

impl Clone for SimulatorSession {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            callback_user_data: self.callback_user_data,
            jpeg_callback_user_data: self.jpeg_callback_user_data,
        }
    }
}

unsafe extern "C" fn native_frame_callback(
    frame: *const ffi::xcw_native_frame,
    user_data: *mut c_void,
) {
    if frame.is_null() || user_data.is_null() {
        return;
    }

    let weak = Weak::from_raw(user_data as *const SimulatorSessionInner);
    if let Some(inner) = weak.upgrade() {
        inner.handle_frame(&*frame);
    }
    let _ = Weak::into_raw(weak);
}

unsafe extern "C" fn native_jpeg_frame_callback(
    frame: *const ffi::xcw_native_jpeg_frame,
    user_data: *mut c_void,
) {
    if frame.is_null() || user_data.is_null() {
        return;
    }

    let weak = Weak::from_raw(user_data as *const SimulatorSessionInner);
    if let Some(inner) = weak.upgrade() {
        inner.handle_jpeg_frame(&*frame);
    }
    let _ = Weak::into_raw(weak);
}

impl SimulatorSessionInner {
    fn start_refresh_pump(self: &Arc<Self>) {
        if self
            .refresh_pump_running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }

        let inner = self.clone();
        tokio::spawn(async move {
            loop {
                if inner.active_frame_subscribers.load(Ordering::Relaxed) == 0 {
                    inner.refresh_pump_running.store(false, Ordering::Release);
                    if inner.active_frame_subscribers.load(Ordering::Relaxed) == 0 {
                        break;
                    }
                    if inner
                        .refresh_pump_running
                        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                        .is_err()
                    {
                        break;
                    }
                }

                inner.request_refresh();
                sleep(shared_refresh_interval()).await;
            }
        });
    }

    fn request_refresh(&self) {
        let now = now_ms();
        let previous = self.last_refresh_ms.load(Ordering::Relaxed);
        if now.saturating_sub(previous) < MIN_REFRESH_INTERVAL_MS {
            return;
        }
        self.last_refresh_ms.store(now, Ordering::Relaxed);
        self.native.request_refresh();
    }

    fn handle_frame(&self, frame: &ffi::xcw_native_frame) {
        let description = unsafe { copy_ffi_bytes(frame.description) };
        let Some(data) = (unsafe { copy_ffi_bytes(frame.data) }) else {
            return;
        };
        let packet = Arc::new(FramePacket {
            frame_sequence: frame.frame_sequence,
            timestamp_us: frame.timestamp_us,
            is_keyframe: frame.is_keyframe,
            width: frame.width,
            height: frame.height,
            codec: c_string(frame.codec),
            description,
            data,
        });

        self.metrics.frames_encoded.fetch_add(1, Ordering::Relaxed);
        if packet.is_keyframe {
            self.metrics
                .keyframes_encoded
                .fetch_add(1, Ordering::Relaxed);
            *self.latest_keyframe.write().unwrap() = Some(packet.clone());
        }

        self.display_ready.store(true, Ordering::Relaxed);
        self.display_width
            .store(packet.width as u64, Ordering::Relaxed);
        self.display_height
            .store(packet.height as u64, Ordering::Relaxed);
        self.frame_sequence
            .store(packet.frame_sequence, Ordering::Relaxed);
        debug!(
            udid = %self.udid,
            sequence = packet.frame_sequence,
            keyframe = packet.is_keyframe,
            "native frame received"
        );
        let _ = self.sender.send(packet);
        if matches!(*self.state.lock().unwrap(), SessionState::Attaching) {
            *self.state.lock().unwrap() = SessionState::Ready;
            self.start_condvar.notify_all();
        }
    }

    fn handle_jpeg_frame(&self, frame: &ffi::xcw_native_jpeg_frame) {
        let Some(data) = (unsafe { copy_ffi_bytes(frame.data) }) else {
            return;
        };
        let packet = Arc::new(JpegFramePacket {
            frame_sequence: frame.frame_sequence,
            timestamp_us: frame.timestamp_us,
            width: frame.width,
            height: frame.height,
            data,
        });

        self.display_ready.store(true, Ordering::Relaxed);
        self.display_width
            .store(packet.width as u64, Ordering::Relaxed);
        self.display_height
            .store(packet.height as u64, Ordering::Relaxed);
        self.frame_sequence
            .store(packet.frame_sequence, Ordering::Relaxed);
        let _ = self.jpeg_sender.send(packet);
        if matches!(*self.state.lock().unwrap(), SessionState::Attaching) {
            *self.state.lock().unwrap() = SessionState::Ready;
            self.start_condvar.notify_all();
        }
    }
}

unsafe fn copy_ffi_bytes(bytes: ffi::xcw_native_shared_bytes) -> Option<Bytes> {
    if bytes.data.is_null() || bytes.length == 0 {
        if !bytes.owner.is_null() {
            unsafe {
                ffi::xcw_native_release_shared_bytes(bytes);
            }
        }
        return None;
    }

    let copied =
        unsafe { Bytes::copy_from_slice(std::slice::from_raw_parts(bytes.data, bytes.length)) };
    unsafe {
        ffi::xcw_native_release_shared_bytes(bytes);
    }
    Some(copied)
}

fn c_string(ptr: *const i8) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let value = unsafe { std::ffi::CStr::from_ptr(ptr) }
        .to_string_lossy()
        .trim()
        .to_owned();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as u64
}

fn shared_refresh_interval() -> Duration {
    let fps = std::env::var("SIMDECK_REALTIME_FPS")
        .or_else(|_| std::env::var("SIMDECK_LOCAL_STREAM_FPS"))
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(DEFAULT_SHARED_REFRESH_FPS)
        .clamp(MIN_SHARED_REFRESH_FPS, MAX_SHARED_REFRESH_FPS);
    Duration::from_micros(1_000_000 / fps)
}
