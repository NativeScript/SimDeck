use crate::error::AppError;
use crate::native::ffi;
use serde::de::Error as DeError;
use serde::{Deserialize, Serialize};
use std::ffi::{c_void, CStr, CString};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const RECOVERABLE_RESTART_EXIT_CODE: i32 = 75;
const RESTART_ON_CORE_SIMULATOR_MISMATCH_ENV: &str = "SIMDECK_RESTART_ON_CORE_SIMULATOR_MISMATCH";

static RECOVERABLE_RESTART_SCHEDULED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Simulator {
    pub udid: String,
    pub name: String,
    pub state: String,
    #[serde(rename = "isBooted")]
    #[serde(deserialize_with = "deserialize_boolish")]
    pub is_booted: bool,
    #[serde(rename = "isAvailable")]
    #[serde(deserialize_with = "deserialize_boolish")]
    pub is_available: bool,
    #[serde(rename = "lastBootedAt")]
    pub last_booted_at: serde_json::Value,
    #[serde(rename = "dataPath")]
    pub data_path: serde_json::Value,
    #[serde(rename = "logPath")]
    pub log_path: serde_json::Value,
    #[serde(rename = "deviceTypeIdentifier")]
    pub device_type_identifier: serde_json::Value,
    #[serde(rename = "deviceTypeName")]
    pub device_type_name: String,
    #[serde(rename = "runtimeIdentifier")]
    pub runtime_identifier: serde_json::Value,
    #[serde(rename = "runtimeName")]
    pub runtime_name: String,
}

#[derive(Debug, Deserialize)]
struct SimulatorsEnvelope {
    simulators: Vec<Simulator>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: String,
    pub process: String,
    pub pid: serde_json::Value,
    pub subsystem: String,
    pub category: String,
    pub message: String,
}

pub struct LogFilters {
    pub levels: Vec<String>,
    pub processes: Vec<String>,
    pub query: String,
}

impl LogFilters {
    pub fn new(levels: Vec<String>, processes: Vec<String>, query: String) -> Self {
        Self {
            levels,
            processes,
            query,
        }
    }
}

#[derive(Debug, Deserialize)]
struct LogsEnvelope {
    entries: Vec<LogEntry>,
}

fn deserialize_boolish<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Bool(value) => Ok(value),
        serde_json::Value::Number(value) => match value.as_i64() {
            Some(0) => Ok(false),
            Some(1) => Ok(true),
            _ => Err(D::Error::custom("expected 0 or 1 for boolean field")),
        },
        serde_json::Value::String(value) => match value.as_str() {
            "0" | "false" | "False" | "FALSE" => Ok(false),
            "1" | "true" | "True" | "TRUE" => Ok(true),
            _ => Err(D::Error::custom("expected boolean-like string")),
        },
        _ => Err(D::Error::custom("expected boolean-compatible value")),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChromeProfile {
    #[serde(rename = "totalWidth")]
    pub total_width: f64,
    #[serde(rename = "totalHeight")]
    pub total_height: f64,
    #[serde(rename = "screenX")]
    pub screen_x: f64,
    #[serde(rename = "screenY")]
    pub screen_y: f64,
    #[serde(rename = "screenWidth")]
    pub screen_width: f64,
    #[serde(rename = "screenHeight")]
    pub screen_height: f64,
    #[serde(rename = "cornerRadius")]
    pub corner_radius: f64,
    #[serde(rename = "hasScreenMask", default)]
    pub has_screen_mask: bool,
}

#[derive(Default, Clone)]
pub struct NativeBridge;

impl NativeBridge {
    pub fn list_simulators(&self) -> Result<Vec<Simulator>, AppError> {
        let json = unsafe {
            let mut error = ptr::null_mut();
            let raw = ffi::xcw_native_list_simulators(&mut error);
            string_from_raw(raw, error)?
        };
        let payload: SimulatorsEnvelope =
            serde_json::from_str(&json).map_err(|e| AppError::internal(e.to_string()))?;
        Ok(payload.simulators)
    }

    pub fn boot_simulator(&self, udid: &str) -> Result<(), AppError> {
        unsafe {
            let udid = CString::new(udid).map_err(|e| AppError::bad_request(e.to_string()))?;
            let mut error = ptr::null_mut();
            bool_result(
                ffi::xcw_native_boot_simulator(udid.as_ptr(), &mut error),
                error,
            )
        }
    }

    pub fn shutdown_simulator(&self, udid: &str) -> Result<(), AppError> {
        unsafe {
            let udid = CString::new(udid).map_err(|e| AppError::bad_request(e.to_string()))?;
            let mut error = ptr::null_mut();
            bool_result(
                ffi::xcw_native_shutdown_simulator(udid.as_ptr(), &mut error),
                error,
            )
        }
    }

    pub fn toggle_appearance(&self, udid: &str) -> Result<(), AppError> {
        unsafe {
            let udid = CString::new(udid).map_err(|e| AppError::bad_request(e.to_string()))?;
            let mut error = ptr::null_mut();
            bool_result(
                ffi::xcw_native_toggle_appearance(udid.as_ptr(), &mut error),
                error,
            )
        }
    }

    pub fn open_url(&self, udid: &str, url: &str) -> Result<(), AppError> {
        let udid = CString::new(udid).map_err(|e| AppError::bad_request(e.to_string()))?;
        let url = CString::new(url).map_err(|e| AppError::bad_request(e.to_string()))?;
        unsafe {
            let mut error = ptr::null_mut();
            bool_result(
                ffi::xcw_native_open_url(udid.as_ptr(), url.as_ptr(), &mut error),
                error,
            )
        }
    }

    pub fn launch_bundle(&self, udid: &str, bundle_id: &str) -> Result<(), AppError> {
        let udid = CString::new(udid).map_err(|e| AppError::bad_request(e.to_string()))?;
        let bundle = CString::new(bundle_id).map_err(|e| AppError::bad_request(e.to_string()))?;
        unsafe {
            let mut error = ptr::null_mut();
            bool_result(
                ffi::xcw_native_launch_bundle(udid.as_ptr(), bundle.as_ptr(), &mut error),
                error,
            )
        }
    }

    pub fn chrome_profile(&self, udid: &str) -> Result<ChromeProfile, AppError> {
        let udid = CString::new(udid).map_err(|e| AppError::bad_request(e.to_string()))?;
        let json = unsafe {
            let mut error = ptr::null_mut();
            let raw = ffi::xcw_native_get_chrome_profile(udid.as_ptr(), &mut error);
            string_from_raw(raw, error)?
        };
        serde_json::from_str(&json).map_err(|e| AppError::internal(e.to_string()))
    }

    pub fn chrome_png(&self, udid: &str) -> Result<Vec<u8>, AppError> {
        let udid = CString::new(udid).map_err(|e| AppError::bad_request(e.to_string()))?;
        unsafe {
            let mut error = ptr::null_mut();
            let bytes = ffi::xcw_native_render_chrome_png(udid.as_ptr(), &mut error);
            if bytes.data.is_null() {
                return Err(
                    take_error(error).unwrap_or_else(|| AppError::native("Unknown native error."))
                );
            }
            let data = std::slice::from_raw_parts(bytes.data, bytes.length).to_vec();
            ffi::xcw_native_free_bytes(bytes);
            Ok(data)
        }
    }

    pub fn screen_mask_png(&self, udid: &str) -> Result<Vec<u8>, AppError> {
        let udid = CString::new(udid).map_err(|e| AppError::bad_request(e.to_string()))?;
        unsafe {
            let mut error = ptr::null_mut();
            let bytes = ffi::xcw_native_render_screen_mask_png(udid.as_ptr(), &mut error);
            if bytes.data.is_null() {
                return Err(
                    take_error(error).unwrap_or_else(|| AppError::native("Unknown native error."))
                );
            }
            let data = std::slice::from_raw_parts(bytes.data, bytes.length).to_vec();
            ffi::xcw_native_free_bytes(bytes);
            Ok(data)
        }
    }

    pub fn screenshot_png(&self, udid: &str) -> Result<Vec<u8>, AppError> {
        let udid = CString::new(udid).map_err(|e| AppError::bad_request(e.to_string()))?;
        unsafe {
            let mut error = ptr::null_mut();
            let bytes = ffi::xcw_native_screenshot_png(udid.as_ptr(), &mut error);
            if bytes.data.is_null() {
                return Err(
                    take_error(error).unwrap_or_else(|| AppError::native("Unknown native error."))
                );
            }
            let data = std::slice::from_raw_parts(bytes.data, bytes.length).to_vec();
            ffi::xcw_native_free_bytes(bytes);
            Ok(data)
        }
    }

    pub fn recent_logs(
        &self,
        udid: &str,
        seconds: f64,
        limit: usize,
        filters: &LogFilters,
    ) -> Result<Vec<LogEntry>, AppError> {
        let udid = CString::new(udid).map_err(|e| AppError::bad_request(e.to_string()))?;
        let json = unsafe {
            let mut error = ptr::null_mut();
            let raw = ffi::xcw_native_recent_logs(udid.as_ptr(), seconds, limit, &mut error);
            string_from_raw(raw, error)?
        };
        let payload: LogsEnvelope =
            serde_json::from_str(&json).map_err(|e| AppError::internal(e.to_string()))?;
        let mut entries: Vec<LogEntry> = payload
            .entries
            .into_iter()
            .filter(|entry| log_entry_matches(entry, filters))
            .collect();
        if entries.len() > limit {
            entries = entries.split_off(entries.len() - limit);
        }
        Ok(entries)
    }

    pub fn accessibility_snapshot(
        &self,
        udid: &str,
        point: Option<(f64, f64)>,
    ) -> Result<serde_json::Value, AppError> {
        self.accessibility_snapshot_with_max_depth(udid, point, None)
    }

    pub fn accessibility_snapshot_with_max_depth(
        &self,
        udid: &str,
        point: Option<(f64, f64)>,
        max_depth: Option<usize>,
    ) -> Result<serde_json::Value, AppError> {
        let udid = CString::new(udid).map_err(|e| AppError::bad_request(e.to_string()))?;
        let max_depth = max_depth.unwrap_or(80).min(80);
        let json = match native_accessibility_snapshot_json(&udid, point, max_depth) {
            Ok(json) => json,
            Err(error) if is_core_simulator_service_mismatch(&error.to_string()) => {
                std::thread::sleep(Duration::from_millis(250));
                native_accessibility_snapshot_json(&udid, point, max_depth)?
            }
            Err(error) => return Err(error),
        };
        let snapshot: serde_json::Value =
            serde_json::from_str(&json).map_err(|e| AppError::internal(e.to_string()))?;
        Ok(snapshot)
    }

    pub fn send_touch(&self, udid: &str, x: f64, y: f64, phase: &str) -> Result<(), AppError> {
        let udid = CString::new(udid).map_err(|e| AppError::bad_request(e.to_string()))?;
        let phase = CString::new(phase).map_err(|e| AppError::bad_request(e.to_string()))?;
        unsafe {
            let mut error = ptr::null_mut();
            bool_result(
                ffi::xcw_native_send_touch(udid.as_ptr(), x, y, phase.as_ptr(), &mut error),
                error,
            )
        }
    }

    pub fn send_key(&self, udid: &str, key_code: u16, modifiers: u32) -> Result<(), AppError> {
        let udid = CString::new(udid).map_err(|e| AppError::bad_request(e.to_string()))?;
        unsafe {
            let mut error = ptr::null_mut();
            bool_result(
                ffi::xcw_native_send_key(udid.as_ptr(), key_code, modifiers, &mut error),
                error,
            )
        }
    }

    pub fn press_home(&self, udid: &str) -> Result<(), AppError> {
        let udid = CString::new(udid).map_err(|e| AppError::bad_request(e.to_string()))?;
        unsafe {
            let mut error = ptr::null_mut();
            bool_result(ffi::xcw_native_press_home(udid.as_ptr(), &mut error), error)
        }
    }

    pub fn open_app_switcher(&self, udid: &str) -> Result<(), AppError> {
        let udid = CString::new(udid).map_err(|e| AppError::bad_request(e.to_string()))?;
        unsafe {
            let mut error = ptr::null_mut();
            bool_result(
                ffi::xcw_native_open_app_switcher(udid.as_ptr(), &mut error),
                error,
            )
        }
    }

    pub fn press_button(&self, udid: &str, button: &str, duration_ms: u32) -> Result<(), AppError> {
        let udid = CString::new(udid).map_err(|e| AppError::bad_request(e.to_string()))?;
        let button = CString::new(button).map_err(|e| AppError::bad_request(e.to_string()))?;
        unsafe {
            let mut error = ptr::null_mut();
            bool_result(
                ffi::xcw_native_press_button(
                    udid.as_ptr(),
                    button.as_ptr(),
                    duration_ms,
                    &mut error,
                ),
                error,
            )
        }
    }

    pub fn rotate_right(&self, udid: &str) -> Result<(), AppError> {
        let udid = CString::new(udid).map_err(|e| AppError::bad_request(e.to_string()))?;
        unsafe {
            let mut error = ptr::null_mut();
            bool_result(
                ffi::xcw_native_rotate_right(udid.as_ptr(), &mut error),
                error,
            )
        }
    }

    pub fn rotate_left(&self, udid: &str) -> Result<(), AppError> {
        let udid = CString::new(udid).map_err(|e| AppError::bad_request(e.to_string()))?;
        unsafe {
            let mut error = ptr::null_mut();
            bool_result(
                ffi::xcw_native_rotate_left(udid.as_ptr(), &mut error),
                error,
            )
        }
    }

    pub fn erase_simulator(&self, udid: &str) -> Result<(), AppError> {
        let udid = CString::new(udid).map_err(|e| AppError::bad_request(e.to_string()))?;
        unsafe {
            let mut error = ptr::null_mut();
            bool_result(
                ffi::xcw_native_erase_simulator(udid.as_ptr(), &mut error),
                error,
            )
        }
    }

    pub fn install_app(&self, udid: &str, app_path: &str) -> Result<(), AppError> {
        let udid = CString::new(udid).map_err(|e| AppError::bad_request(e.to_string()))?;
        let app_path = CString::new(app_path).map_err(|e| AppError::bad_request(e.to_string()))?;
        unsafe {
            let mut error = ptr::null_mut();
            bool_result(
                ffi::xcw_native_install_app(udid.as_ptr(), app_path.as_ptr(), &mut error),
                error,
            )
        }
    }

    pub fn uninstall_app(&self, udid: &str, bundle_id: &str) -> Result<(), AppError> {
        let udid = CString::new(udid).map_err(|e| AppError::bad_request(e.to_string()))?;
        let bundle_id =
            CString::new(bundle_id).map_err(|e| AppError::bad_request(e.to_string()))?;
        unsafe {
            let mut error = ptr::null_mut();
            bool_result(
                ffi::xcw_native_uninstall_app(udid.as_ptr(), bundle_id.as_ptr(), &mut error),
                error,
            )
        }
    }

    pub fn set_pasteboard_text(&self, udid: &str, text: &str) -> Result<(), AppError> {
        let udid = CString::new(udid).map_err(|e| AppError::bad_request(e.to_string()))?;
        let text = CString::new(text).map_err(|e| AppError::bad_request(e.to_string()))?;
        unsafe {
            let mut error = ptr::null_mut();
            bool_result(
                ffi::xcw_native_set_pasteboard_text(udid.as_ptr(), text.as_ptr(), &mut error),
                error,
            )
        }
    }

    pub fn pasteboard_text(&self, udid: &str) -> Result<String, AppError> {
        let udid = CString::new(udid).map_err(|e| AppError::bad_request(e.to_string()))?;
        unsafe {
            let mut error = ptr::null_mut();
            let raw = ffi::xcw_native_get_pasteboard_text(udid.as_ptr(), &mut error);
            string_from_raw(raw, error)
        }
    }

    pub fn create_input_session(&self, udid: &str) -> Result<NativeInputSession, AppError> {
        let udid = CString::new(udid).map_err(|e| AppError::bad_request(e.to_string()))?;
        unsafe {
            let mut error = ptr::null_mut();
            let handle = ffi::xcw_native_input_create(udid.as_ptr(), &mut error);
            if handle.is_null() {
                return Err(take_error(error).unwrap_or_else(|| {
                    AppError::native("Unable to create native input session.")
                }));
            }
            Ok(NativeInputSession { handle })
        }
    }

    pub fn create_session(&self, udid: &str) -> Result<NativeSession, AppError> {
        let udid = CString::new(udid).map_err(|e| AppError::bad_request(e.to_string()))?;
        unsafe {
            let mut error = ptr::null_mut();
            let handle = ffi::xcw_native_session_create(udid.as_ptr(), &mut error);
            if handle.is_null() {
                return Err(take_error(error)
                    .unwrap_or_else(|| AppError::native("Unable to create native session.")));
            }
            Ok(NativeSession { handle })
        }
    }
}

pub fn log_entry_matches(entry: &LogEntry, filters: &LogFilters) -> bool {
    if !filters.levels.is_empty()
        && !filters
            .levels
            .iter()
            .any(|level| log_level_matches(&entry.level, level))
    {
        return false;
    }

    if !filters.processes.is_empty()
        && !filters
            .processes
            .iter()
            .any(|process| entry.process.eq_ignore_ascii_case(process))
    {
        return false;
    }

    if !filters.query.is_empty() {
        let haystack = format!(
            "{} {} {} {} {}",
            entry.process, entry.message, entry.subsystem, entry.category, entry.level
        )
        .to_lowercase();
        if !haystack.contains(&filters.query) {
            return false;
        }
    }

    true
}

fn log_level_matches(entry_level: &str, filter: &str) -> bool {
    match filter {
        "error" => {
            entry_level.to_lowercase().contains("error")
                || entry_level.to_lowercase().contains("fault")
        }
        "debug" => entry_level.to_lowercase().contains("debug"),
        "info" => entry_level.to_lowercase().contains("info"),
        "default" => {
            let level = entry_level.to_lowercase();
            !level.contains("error")
                && !level.contains("fault")
                && !level.contains("debug")
                && !level.contains("info")
        }
        _ => true,
    }
}

pub struct NativeInputSession {
    handle: *mut c_void,
}

unsafe impl Send for NativeInputSession {}
unsafe impl Sync for NativeInputSession {}

impl NativeInputSession {
    pub fn display_size(&self) -> Option<(f64, f64)> {
        unsafe {
            let mut width = 0.0;
            let mut height = 0.0;
            if ffi::xcw_native_input_display_size(self.handle, &mut width, &mut height)
                && width.is_finite()
                && height.is_finite()
                && width > 0.0
                && height > 0.0
            {
                Some((width, height))
            } else {
                None
            }
        }
    }

    pub fn send_touch(&self, x: f64, y: f64, phase: &str) -> Result<(), AppError> {
        let phase = CString::new(phase).map_err(|e| AppError::bad_request(e.to_string()))?;
        unsafe {
            let mut error = ptr::null_mut();
            bool_result(
                ffi::xcw_native_input_send_touch(self.handle, x, y, phase.as_ptr(), &mut error),
                error,
            )
        }
    }

    pub fn send_multitouch(
        &self,
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        phase: &str,
    ) -> Result<(), AppError> {
        let phase = CString::new(phase).map_err(|e| AppError::bad_request(e.to_string()))?;
        unsafe {
            let mut error = ptr::null_mut();
            bool_result(
                ffi::xcw_native_input_send_multitouch(
                    self.handle,
                    x1,
                    y1,
                    x2,
                    y2,
                    phase.as_ptr(),
                    &mut error,
                ),
                error,
            )
        }
    }

    pub fn send_key(&self, key_code: u16, modifiers: u32) -> Result<(), AppError> {
        unsafe {
            let mut error = ptr::null_mut();
            bool_result(
                ffi::xcw_native_input_send_key(self.handle, key_code, modifiers, &mut error),
                error,
            )
        }
    }

    pub fn send_key_event(&self, key_code: u16, down: bool) -> Result<(), AppError> {
        unsafe {
            let mut error = ptr::null_mut();
            bool_result(
                ffi::xcw_native_input_send_key_event(self.handle, key_code, down, &mut error),
                error,
            )
        }
    }
}

impl Drop for NativeInputSession {
    fn drop(&mut self) {
        unsafe {
            ffi::xcw_native_input_destroy(self.handle);
        }
    }
}

pub struct NativeSession {
    handle: *mut c_void,
}

unsafe impl Send for NativeSession {}
unsafe impl Sync for NativeSession {}

impl NativeSession {
    pub fn start(&self) -> Result<(), AppError> {
        unsafe {
            let mut error = ptr::null_mut();
            bool_result(
                ffi::xcw_native_session_start(self.handle, &mut error),
                error,
            )
        }
    }

    pub fn request_refresh(&self) {
        unsafe {
            ffi::xcw_native_session_request_refresh(self.handle);
        }
    }

    pub fn request_keyframe(&self) {
        unsafe {
            ffi::xcw_native_session_request_keyframe(self.handle);
        }
    }

    pub unsafe fn set_frame_callback(
        &self,
        callback: Option<ffi::xcw_native_frame_callback>,
        user_data: *mut c_void,
    ) {
        ffi::xcw_native_session_set_frame_callback(self.handle, callback, user_data);
    }

    pub fn send_touch(&self, x: f64, y: f64, phase: &str) -> Result<(), AppError> {
        let phase = CString::new(phase).map_err(|e| AppError::bad_request(e.to_string()))?;
        unsafe {
            let mut error = ptr::null_mut();
            bool_result(
                ffi::xcw_native_session_send_touch(self.handle, x, y, phase.as_ptr(), &mut error),
                error,
            )
        }
    }

    pub fn send_key(&self, key_code: u16, modifiers: u32) -> Result<(), AppError> {
        unsafe {
            let mut error = ptr::null_mut();
            bool_result(
                ffi::xcw_native_session_send_key(self.handle, key_code, modifiers, &mut error),
                error,
            )
        }
    }
}

impl Drop for NativeSession {
    fn drop(&mut self) {
        unsafe {
            ffi::xcw_native_session_set_frame_callback(self.handle, None, ptr::null_mut());
            ffi::xcw_native_session_destroy(self.handle);
        }
    }
}

fn native_accessibility_snapshot_json(
    udid: &CString,
    point: Option<(f64, f64)>,
    max_depth: usize,
) -> Result<String, AppError> {
    unsafe {
        let mut error = ptr::null_mut();
        let (has_point, x, y) = point
            .map(|(x, y)| (true, x, y))
            .unwrap_or((false, 0.0, 0.0));
        let raw = ffi::xcw_native_accessibility_snapshot(
            udid.as_ptr(),
            has_point,
            x,
            y,
            max_depth,
            &mut error,
        );
        string_from_raw(raw, error)
    }
}

fn is_core_simulator_service_mismatch(message: &str) -> bool {
    message.contains("CoreSimulator.framework was changed while the process was running")
        || message.contains("Service version")
            && message.contains("does not match expected service version")
}

unsafe fn string_from_raw(raw: *mut i8, error: *mut i8) -> Result<String, AppError> {
    if raw.is_null() {
        return Err(take_error(error).unwrap_or_else(|| AppError::native("Unknown native error.")));
    }
    let value = CStr::from_ptr(raw).to_string_lossy().into_owned();
    ffi::xcw_native_free_string(raw);
    Ok(value)
}

unsafe fn bool_result(result: bool, error: *mut i8) -> Result<(), AppError> {
    if result {
        Ok(())
    } else {
        Err(take_error(error).unwrap_or_else(|| AppError::native("Unknown native error.")))
    }
}

unsafe fn take_error(raw: *mut i8) -> Option<AppError> {
    if raw.is_null() {
        return None;
    }
    let message = CStr::from_ptr(raw).to_string_lossy().into_owned();
    ffi::xcw_native_free_string(raw);
    schedule_recoverable_restart_if_needed(&message);
    Some(AppError::native(message))
}

fn schedule_recoverable_restart_if_needed(message: &str) {
    if std::env::var_os(RESTART_ON_CORE_SIMULATOR_MISMATCH_ENV).is_none()
        || !is_core_simulator_service_mismatch(message)
        || RECOVERABLE_RESTART_SCHEDULED.swap(true, Ordering::SeqCst)
    {
        return;
    }

    eprintln!("CoreSimulator service mismatch detected; restarting simdeck server process.");
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(100));
        std::process::exit(RECOVERABLE_RESTART_EXIT_CODE);
    });
}
