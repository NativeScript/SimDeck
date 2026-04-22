use crate::error::AppError;
use crate::metrics::counters::Metrics;
use crate::native::bridge::{NativeBridge, NativeSession};
use crate::native::ffi;
use crate::simulators::state::SessionState;
use crate::transport::packet::{ForeignBytes, FramePacket, SharedFrame};
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::watch;
use tokio::task;
use tokio::time::{timeout, Instant};
use tracing::debug;

pub struct SimulatorSession {
    inner: Arc<SimulatorSessionInner>,
    callback_user_data: usize,
}

struct SimulatorSessionInner {
    udid: String,
    native: NativeSession,
    metrics: Arc<Metrics>,
    sender: watch::Sender<Option<SharedFrame>>,
    latest_keyframe: RwLock<Option<SharedFrame>>,
    state: Mutex<SessionState>,
    display_ready: AtomicBool,
    display_width: AtomicU64,
    display_height: AtomicU64,
    frame_sequence: AtomicU64,
    last_refresh_ms: AtomicU64,
}

impl SimulatorSession {
    pub fn new(
        bridge: &NativeBridge,
        udid: String,
        metrics: Arc<Metrics>,
    ) -> Result<Self, AppError> {
        let native = bridge.create_session(&udid)?;
        let (sender, _) = watch::channel(None);
        let inner = Arc::new(SimulatorSessionInner {
            udid,
            native,
            metrics,
            sender,
            latest_keyframe: RwLock::new(None),
            state: Mutex::new(SessionState::Detached),
            display_ready: AtomicBool::new(false),
            display_width: AtomicU64::new(0),
            display_height: AtomicU64::new(0),
            frame_sequence: AtomicU64::new(0),
            last_refresh_ms: AtomicU64::new(0),
        });

        let user_data = Arc::into_raw(inner.clone()) as *mut c_void;
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
        {
            let mut state = self.inner.state.lock().unwrap();
            if matches!(*state, SessionState::Ready | SessionState::Streaming) {
                return Ok(());
            }
            *state = SessionState::Attaching;
        }

        if let Err(error) = self.inner.native.start() {
            *self.inner.state.lock().unwrap() = SessionState::Failed;
            return Err(error);
        }
        *self.inner.state.lock().unwrap() = SessionState::Ready;
        Ok(())
    }

    pub async fn ensure_started_async(&self) -> Result<(), AppError> {
        let session = self.clone();
        task::spawn_blocking(move || session.ensure_started())
            .await
            .map_err(|error| AppError::internal(format!("Failed to join start task: {error}")))?
    }

    pub fn subscribe(&self) -> watch::Receiver<Option<SharedFrame>> {
        self.inner
            .metrics
            .subscribers_connected
            .fetch_add(1, Ordering::Relaxed);
        self.inner
            .metrics
            .max_send_queue_depth
            .fetch_max(1, Ordering::Relaxed);
        *self.inner.state.lock().unwrap() = SessionState::Streaming;
        self.inner.sender.subscribe()
    }

    pub fn latest_keyframe(&self) -> Option<SharedFrame> {
        self.inner.latest_keyframe.read().unwrap().clone()
    }

    pub async fn wait_for_keyframe(&self, timeout_duration: Duration) -> Option<SharedFrame> {
        if let Some(frame) = self.latest_keyframe() {
            return Some(frame);
        }

        let deadline = Instant::now() + timeout_duration;
        let mut rx = self.inner.sender.subscribe();
        self.request_refresh_async().await;

        loop {
            if let Some(frame) = self.latest_keyframe() {
                return Some(frame);
            }

            let now = Instant::now();
            if now >= deadline {
                return self.latest_keyframe();
            }

            let remaining = deadline - now;
            match timeout(remaining, rx.changed()).await {
                Ok(Ok(())) => {
                    self.request_refresh_async().await;
                }
                Ok(Err(_)) | Err(_) => return self.latest_keyframe(),
            }
        }
    }

    pub fn request_refresh(&self) {
        let now = now_ms();
        let previous = self.inner.last_refresh_ms.load(Ordering::Relaxed);
        if now.saturating_sub(previous) < 200 {
            return;
        }
        self.inner.last_refresh_ms.store(now, Ordering::Relaxed);
        self.inner
            .metrics
            .keyframe_requests
            .fetch_add(1, Ordering::Relaxed);
        self.inner.native.request_refresh();
    }

    pub async fn request_refresh_async(&self) {
        self.request_refresh();
    }

    pub fn send_touch(&self, x: f64, y: f64, phase: &str) -> Result<(), AppError> {
        self.ensure_started()?;
        self.inner.native.send_touch(x, y, phase)
    }

    pub fn send_key(&self, key_code: u16, modifiers: u32) -> Result<(), AppError> {
        self.ensure_started()?;
        self.inner.native.send_key(key_code, modifiers)
    }

    pub fn press_home(&self) -> Result<(), AppError> {
        self.ensure_started()?;
        self.inner.native.press_home()
    }

    pub fn rotate_right(&self) -> Result<(), AppError> {
        self.ensure_started()?;
        self.inner.native.rotate_right()
    }

    pub fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "displayReady": self.inner.display_ready.load(Ordering::Relaxed),
            "displayStatus": self.inner.state.lock().unwrap().as_str(),
            "displayWidth": self.inner.display_width.load(Ordering::Relaxed),
            "displayHeight": self.inner.display_height.load(Ordering::Relaxed),
            "frameSequence": self.inner.frame_sequence.load(Ordering::Relaxed),
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
                let _ = Arc::from_raw(self.callback_user_data as *const SimulatorSessionInner);
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

    let inner = Arc::from_raw(user_data as *const SimulatorSessionInner);
    inner.handle_frame(&*frame);
    let _ = Arc::into_raw(inner);
}

impl SimulatorSessionInner {
    fn handle_frame(&self, frame: &ffi::xcw_native_frame) {
        let description = unsafe { ForeignBytes::from_ffi(frame.description) };
        let Some(data) = (unsafe { ForeignBytes::from_ffi(frame.data) }) else {
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
        let _ = self.sender.send_replace(Some(packet));
        if matches!(*self.state.lock().unwrap(), SessionState::Attaching) {
            *self.state.lock().unwrap() = SessionState::Ready;
        }
    }
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
