use crate::error::AppError;
use crate::native::ffi;
use serde::de::Error as DeError;
use serde::{Deserialize, Serialize};
use std::ffi::{c_void, CStr, CString};
use std::ptr;

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

pub struct NativeSession {
    handle: *mut c_void,
}

unsafe impl Send for NativeSession {}
unsafe impl Sync for NativeSession {}

impl NativeSession {
    pub fn session_info(&self) -> Result<serde_json::Value, AppError> {
        let json = unsafe {
            let mut error = ptr::null_mut();
            let raw = ffi::xcw_native_session_info(self.handle, &mut error);
            string_from_raw(raw, error)?
        };
        serde_json::from_str(&json).map_err(|e| AppError::internal(e.to_string()))
    }

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

    pub fn press_home(&self) -> Result<(), AppError> {
        unsafe {
            let mut error = ptr::null_mut();
            bool_result(
                ffi::xcw_native_session_press_home(self.handle, &mut error),
                error,
            )
        }
    }

    pub fn rotate_right(&self) -> Result<(), AppError> {
        unsafe {
            let mut error = ptr::null_mut();
            bool_result(
                ffi::xcw_native_session_rotate_right(self.handle, &mut error),
                error,
            )
        }
    }

    pub fn rotate_left(&self) -> Result<(), AppError> {
        unsafe {
            let mut error = ptr::null_mut();
            bool_result(
                ffi::xcw_native_session_rotate_left(self.handle, &mut error),
                error,
            )
        }
    }

    pub unsafe fn set_frame_callback(
        &self,
        callback: Option<ffi::xcw_native_frame_callback>,
        user_data: *mut c_void,
    ) {
        ffi::xcw_native_session_set_frame_callback(self.handle, callback, user_data);
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
    Some(AppError::native(message))
}
