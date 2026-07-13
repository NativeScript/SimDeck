use crate::error::AppError;
use crate::metrics::counters::Metrics;
use crate::native::bridge::{
    digital_crown_error, fixed_orientation_error, simulator_has_fixed_orientation,
    simulator_is_tvos, simulator_is_watchos, NativeBridge, NativeSession,
};
use crate::native::ffi;
use crate::simulators::state::SessionState;
use crate::transport::packet::{FramePacket, SharedFrame};
use bytes::Bytes;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;
use tokio::task;
use tokio::time::{sleep_until, timeout, Instant};
use tracing::debug;

// This channel carries encoded H.264 access units. Subscribers must not miss
// ordinary P-frames: dropping compressed references creates decoder artifacts
// even on a perfect localhost link. Coalescing is only safe before encoding.
const FRAME_BROADCAST_CAPACITY: usize = 128;
const MIN_KEYFRAME_INTERVAL_MS: u64 = 250;
const DEFAULT_SHARED_REFRESH_FPS: u64 = 60;
const MIN_SHARED_REFRESH_FPS: u64 = 15;
const MAX_SHARED_REFRESH_FPS: u64 = 240;
const MIN_REFRESH_INTERVAL_US: u64 = 1_000_000 / MAX_SHARED_REFRESH_FPS;

pub struct SimulatorSession {
    inner: Arc<SimulatorSessionInner>,
    callback_user_data: usize,
}

struct SimulatorSessionInner {
    udid: String,
    native: NativeSession,
    is_tvos: bool,
    is_watchos: bool,
    has_fixed_orientation: bool,
    metrics: Arc<Metrics>,
    sender: broadcast::Sender<SharedFrame>,
    latest_keyframe: RwLock<Option<SharedFrame>>,
    state: Mutex<SessionState>,
    start_condvar: Condvar,
    display_ready: AtomicBool,
    display_width: AtomicU64,
    display_height: AtomicU64,
    frame_sequence: AtomicU64,
    last_frame_ms: AtomicU64,
    last_refresh_us: AtomicU64,
    last_keyframe_ms: AtomicU64,
    last_activity_ms: AtomicU64,
    active_frame_subscribers: AtomicU64,
    refresh_pump_running: AtomicBool,
}

pub struct FrameSubscription {
    inner: Arc<SimulatorSessionInner>,
    receiver: broadcast::Receiver<SharedFrame>,
}

impl FrameSubscription {
    pub async fn recv(&mut self) -> Result<SharedFrame, broadcast::error::RecvError> {
        self.receiver.recv().await
    }
}

impl Drop for FrameSubscription {
    fn drop(&mut self) {
        let previous = self
            .inner
            .active_frame_subscribers
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                Some(value.saturating_sub(1))
            })
            .unwrap_or(0);
        if previous <= 1 {
            *self.inner.state.lock().unwrap() = SessionState::Ready;
            self.inner.native.set_client_foreground(false);
            *self.inner.latest_keyframe.write().unwrap() = None;
            self.inner.last_keyframe_ms.store(0, Ordering::Relaxed);
            self.inner.last_refresh_us.store(0, Ordering::Relaxed);
            self.inner.native.reconfigure_video_encoder();
            self.inner
                .metrics
                .stream_pipeline_resets
                .fetch_add(1, Ordering::Relaxed);
        }
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
        let simulator = bridge.simulator(&udid).ok().flatten();
        let is_tvos = simulator.as_ref().map(simulator_is_tvos).unwrap_or(false);
        let is_watchos = simulator
            .as_ref()
            .map(simulator_is_watchos)
            .unwrap_or(false);
        let has_fixed_orientation = simulator
            .as_ref()
            .map(simulator_has_fixed_orientation)
            .unwrap_or(false);
        let (sender, _) = broadcast::channel(FRAME_BROADCAST_CAPACITY);
        let now = now_ms();
        let inner = Arc::new(SimulatorSessionInner {
            udid,
            native,
            is_tvos,
            is_watchos,
            has_fixed_orientation,
            metrics,
            sender,
            latest_keyframe: RwLock::new(None),
            state: Mutex::new(SessionState::Detached),
            start_condvar: Condvar::new(),
            display_ready: AtomicBool::new(false),
            display_width: AtomicU64::new(0),
            display_height: AtomicU64::new(0),
            frame_sequence: AtomicU64::new(0),
            last_frame_ms: AtomicU64::new(0),
            last_refresh_us: AtomicU64::new(0),
            last_keyframe_ms: AtomicU64::new(0),
            last_activity_ms: AtomicU64::new(now),
            active_frame_subscribers: AtomicU64::new(0),
            refresh_pump_running: AtomicBool::new(false),
        });

        let user_data = Weak::into_raw(Arc::downgrade(&inner)) as *mut c_void;
        unsafe {
            inner
                .native
                .set_frame_callback(Some(native_frame_callback), user_data);
        }

        Ok(Self {
            inner,
            callback_user_data: user_data as usize,
        })
    }

    pub fn ensure_started(&self) -> Result<(), AppError> {
        self.mark_activity();
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
        if self.inner.active_frame_subscribers.load(Ordering::Relaxed) == 0 {
            self.inner.native.set_client_foreground(false);
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
        self.mark_activity();
        *self.inner.state.lock().unwrap() = SessionState::Streaming;
        let previous = self
            .inner
            .active_frame_subscribers
            .fetch_add(1, Ordering::Relaxed);
        if previous == 0 {
            self.inner.native.set_client_foreground(true);
        }
        self.inner.start_refresh_pump();
        FrameSubscription {
            inner: self.inner.clone(),
            receiver: self.inner.sender.subscribe(),
        }
    }

    pub fn latest_keyframe(&self) -> Option<SharedFrame> {
        self.inner.latest_keyframe.read().unwrap().clone()
    }

    pub async fn wait_for_keyframe(&self, timeout_duration: Duration) -> Option<SharedFrame> {
        self.mark_activity();
        self.inner.native.set_client_foreground(true);
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
        self.mark_activity();
        self.inner.request_refresh();
    }

    pub fn request_keyframe(&self) {
        self.mark_activity();
        let now = now_ms();
        let previous = self.inner.last_keyframe_ms.load(Ordering::Relaxed);
        if now.saturating_sub(previous) < MIN_KEYFRAME_INTERVAL_MS {
            self.request_refresh();
            return;
        }
        self.inner.last_keyframe_ms.store(now, Ordering::Relaxed);
        self.inner
            .last_refresh_us
            .store(now_us(), Ordering::Relaxed);
        self.inner
            .metrics
            .keyframe_requests
            .fetch_add(1, Ordering::Relaxed);
        self.inner.native.request_keyframe();
    }

    fn request_keyframe_immediate(&self) {
        let now = now_ms();
        self.inner.last_keyframe_ms.store(now, Ordering::Relaxed);
        self.inner
            .last_refresh_us
            .store(now_us(), Ordering::Relaxed);
        self.inner
            .metrics
            .keyframe_requests
            .fetch_add(1, Ordering::Relaxed);
        self.inner.native.request_keyframe();
    }

    pub fn reconfigure_video_encoder(&self) {
        self.mark_activity();
        *self.inner.latest_keyframe.write().unwrap() = None;
        self.inner.last_keyframe_ms.store(0, Ordering::Relaxed);
        self.inner.last_refresh_us.store(0, Ordering::Relaxed);
        self.inner.native.reconfigure_video_encoder();
    }

    pub fn set_client_foreground(&self, foreground: bool) {
        self.mark_activity();
        self.inner.native.set_client_foreground(foreground);
        if foreground {
            self.request_keyframe();
        }
    }

    pub fn is_tvos(&self) -> bool {
        self.inner.is_tvos
    }

    pub fn has_fixed_orientation(&self) -> bool {
        self.inner.has_fixed_orientation
    }

    pub fn is_watchos(&self) -> bool {
        self.inner.is_watchos
    }

    pub fn send_touch(&self, x: f64, y: f64, phase: &str) -> Result<(), AppError> {
        self.mark_activity();
        if self.is_tvos() {
            return Err(AppError::bad_request(
                "tvOS simulators do not support direct screen touch. Use Enter and arrow keys instead.",
            ));
        }
        self.inner.native.send_touch(x, y, phase)
    }

    pub fn send_edge_touch(&self, x: f64, y: f64, phase: &str, edge: u32) -> Result<(), AppError> {
        self.mark_activity();
        if self.is_tvos() {
            return Err(AppError::bad_request(
                "tvOS simulators do not support direct screen touch. Use Enter and arrow keys instead.",
            ));
        }
        self.inner.native.send_edge_touch(x, y, phase, edge)
    }

    pub fn send_multitouch(
        &self,
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        phase: &str,
    ) -> Result<(), AppError> {
        self.mark_activity();
        if self.is_tvos() {
            return Err(AppError::bad_request(
                "tvOS simulators do not support direct screen touch. Use Enter and arrow keys instead.",
            ));
        }
        self.inner.native.send_multitouch(x1, y1, x2, y2, phase)
    }

    pub fn send_key(&self, key_code: u16, modifiers: u32) -> Result<(), AppError> {
        self.mark_activity();
        self.inner.native.send_key(key_code, modifiers)
    }

    pub fn press_home(&self) -> Result<(), AppError> {
        self.mark_activity();
        self.inner.native.press_home()
    }

    pub fn press_button(&self, button: &str, duration_ms: u32) -> Result<(), AppError> {
        self.mark_activity();
        self.inner.native.press_button(button, duration_ms)
    }

    pub fn send_button(
        &self,
        button: &str,
        pressed: bool,
        usage_page: Option<u32>,
        usage: Option<u32>,
    ) -> Result<(), AppError> {
        self.mark_activity();
        self.inner
            .native
            .send_button(button, pressed, usage_page, usage)
    }

    pub fn rotate_crown(&self, delta: f64) -> Result<(), AppError> {
        self.mark_activity();
        if !self.is_watchos() {
            return Err(digital_crown_error());
        }
        self.inner.native.rotate_crown(delta)
    }

    pub fn open_app_switcher(&self) -> Result<(), AppError> {
        self.mark_activity();
        self.inner.native.open_app_switcher()
    }

    pub fn rotate_left(&self) -> Result<(), AppError> {
        self.mark_activity();
        if self.has_fixed_orientation() {
            return Err(fixed_orientation_error());
        }
        self.inner.native.rotate_left()
    }

    pub fn rotate_right(&self) -> Result<(), AppError> {
        self.mark_activity();
        if self.has_fixed_orientation() {
            return Err(fixed_orientation_error());
        }
        self.inner.native.rotate_right()
    }

    pub fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "displayReady": self.inner.display_ready.load(Ordering::Relaxed),
            "displayStatus": self.inner.state.lock().unwrap().as_str(),
            "displayWidth": self.inner.display_width.load(Ordering::Relaxed),
            "displayHeight": self.inner.display_height.load(Ordering::Relaxed),
            "frameSequence": self.inner.frame_sequence.load(Ordering::Relaxed),
            "lastFrameAt": self.inner.last_frame_ms.load(Ordering::Relaxed),
            "rotationQuarterTurns": self.inner.native.rotation_quarter_turns(),
            "encoder": self.inner.native.video_encoder_stats(),
        })
    }

    /// Current display rotation in quarter turns (0..=3). Odd values mean the
    /// interface is landscape relative to the device's native orientation.
    pub fn rotation_quarter_turns(&self) -> i32 {
        self.inner.native.rotation_quarter_turns()
    }

    pub fn is_idle_for(&self, idle_timeout: Duration) -> bool {
        if Arc::strong_count(&self.inner) > 1 {
            return false;
        }
        if self.inner.active_frame_subscribers.load(Ordering::Relaxed) > 0 {
            return false;
        }
        let idle_ms = idle_timeout.as_millis().try_into().unwrap_or(u64::MAX);
        now_ms().saturating_sub(self.inner.last_activity_ms.load(Ordering::Relaxed)) >= idle_ms
    }

    fn mark_activity(&self) {
        self.inner.mark_activity();
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
                let _ = Weak::from_raw(self.callback_user_data as *const SimulatorSessionInner);
            }
        }
    }
}

impl Clone for SimulatorSession {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            callback_user_data: self.callback_user_data,
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

impl SimulatorSessionInner {
    fn mark_activity(&self) {
        self.last_activity_ms.store(now_ms(), Ordering::Relaxed);
    }

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
            let mut next_tick = Instant::now();
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
                let refresh_interval = shared_refresh_interval();
                next_tick += refresh_interval;
                let now = Instant::now();
                if next_tick <= now {
                    next_tick = now + refresh_interval;
                }
                sleep_until(next_tick).await;
            }
        });
    }

    fn request_refresh(&self) {
        let now = now_us();
        let previous = self.last_refresh_us.load(Ordering::Relaxed);
        if now.saturating_sub(previous) < MIN_REFRESH_INTERVAL_US {
            return;
        }
        self.last_refresh_us.store(now, Ordering::Relaxed);
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
        self.last_frame_ms.store(now_ms(), Ordering::Relaxed);
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

fn now_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_micros() as u64
}

fn shared_refresh_interval() -> Duration {
    let target_fps = std::env::var("SIMDECK_REALTIME_FPS")
        .or_else(|_| std::env::var("SIMDECK_LOCAL_STREAM_FPS"))
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(DEFAULT_SHARED_REFRESH_FPS)
        .clamp(MIN_SHARED_REFRESH_FPS, MAX_SHARED_REFRESH_FPS);
    let fps = if realtime_stream_enabled() {
        target_fps.saturating_mul(2)
    } else {
        target_fps
    }
    .clamp(MIN_SHARED_REFRESH_FPS, MAX_SHARED_REFRESH_FPS);
    Duration::from_micros(1_000_000 / fps)
}

fn realtime_stream_enabled() -> bool {
    std::env::var("SIMDECK_REALTIME_STREAM")
        .map(|value| {
            let value = value.trim();
            value == "1"
                || value.eq_ignore_ascii_case("true")
                || value.eq_ignore_ascii_case("yes")
                || value.eq_ignore_ascii_case("on")
        })
        .unwrap_or(false)
}
