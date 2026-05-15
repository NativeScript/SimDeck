use crate::error::AppError;
use crate::native::ffi;
use anyhow::{anyhow, Context};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::ffi::{c_char, CStr, CString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const INJECTOR_NAME: &str = "libSimDeckCameraInjector.dylib";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CameraSourceKind {
    Placeholder,
    Image,
    Video,
    Webcam,
}

impl CameraSourceKind {
    fn as_native_arg(&self) -> &'static str {
        match self {
            Self::Placeholder => "placeholder",
            Self::Image => "image",
            Self::Video => "video",
            Self::Webcam => "webcam",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraSource {
    pub kind: CameraSourceKind,
    #[serde(default)]
    pub arg: Option<String>,
}

impl Default for CameraSource {
    fn default() -> Self {
        Self {
            kind: CameraSourceKind::Placeholder,
            arg: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraStartRequest {
    #[serde(default)]
    pub bundle_id: Option<String>,
    #[serde(default)]
    pub source: CameraSource,
    #[serde(default)]
    pub mirror: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraSwitchRequest {
    pub source: CameraSource,
    #[serde(default)]
    pub mirror: Option<String>,
}

#[derive(Clone, Debug)]
pub struct CameraStartOptions {
    pub udid: String,
    pub bundle_id: Option<String>,
    pub source: CameraSource,
    pub mirror: Option<String>,
}

pub fn list_webcams_value() -> Result<Value, AppError> {
    let mut error_message = std::ptr::null_mut();
    let raw = unsafe { ffi::simdeck_camera_list_webcams_json(&mut error_message) };
    native_json(raw, error_message, "Unable to list Mac cameras.")
}

pub fn start_camera(options: CameraStartOptions) -> Result<Value, AppError> {
    validate_udid(&options.udid)?;
    let source = normalize_source(options.source)?;
    let mirror = normalize_mirror(options.mirror.as_deref())?;
    fs::create_dir_all(camera_state_dir()).map_err(app_internal)?;

    let shm_name = shm_name_for_udid(&options.udid);
    native_start_camera(&options.udid, &shm_name, &source, &mirror)?;

    if let Some(bundle_id) = options
        .bundle_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        validate_bundle_id(bundle_id)?;
        launch_with_injector(&options.udid, bundle_id, &shm_name, &mirror)?;
        record_injected_bundle(&options.udid, bundle_id)?;
    }

    let mut status = native_status(&options.udid)?;
    enrich_status(&options.udid, &mut status);
    Ok(status)
}

pub fn switch_camera(
    udid: &str,
    source: CameraSource,
    mirror: Option<String>,
) -> Result<Value, AppError> {
    validate_udid(udid)?;
    let source = normalize_source(source)?;
    let mirror = match mirror {
        Some(value) => Some(normalize_mirror(Some(&value))?),
        None => None,
    };
    let mut status = native_switch_camera(udid, &source, mirror.as_deref())?;
    enrich_status(udid, &mut status);
    Ok(status)
}

pub fn camera_status(udid: &str) -> Result<Value, AppError> {
    validate_udid(udid)?;
    let mut status = native_status(udid)?;
    enrich_status(udid, &mut status);
    Ok(status)
}

pub fn stop_camera(udid: &str) -> Result<Value, AppError> {
    validate_udid(udid)?;
    native_stop_camera(udid)?;
    let _ = fs::remove_file(injected_bundles_file(udid));
    Ok(json!({ "ok": true, "udid": udid, "alive": false }))
}

fn native_start_camera(
    udid: &str,
    shm_name: &str,
    source: &CameraSource,
    mirror: &str,
) -> Result<(), AppError> {
    let udid = cstring("simulator UDID", udid)?;
    let shm_name = cstring("shared memory name", shm_name)?;
    let source_name = cstring("camera source", source.kind.as_native_arg())?;
    let source_arg = cstring(
        "camera source argument",
        source.arg.as_deref().unwrap_or(""),
    )?;
    let mirror = cstring("camera mirror", mirror)?;
    let mut error_message = std::ptr::null_mut();
    let ok = unsafe {
        ffi::simdeck_camera_start(
            udid.as_ptr(),
            shm_name.as_ptr(),
            source_name.as_ptr(),
            source_arg.as_ptr(),
            mirror.as_ptr(),
            &mut error_message,
        )
    };
    if ok {
        Ok(())
    } else {
        Err(native_error(
            error_message,
            "Unable to start daemon camera simulation.",
        ))
    }
}

fn native_status(udid: &str) -> Result<Value, AppError> {
    let udid = cstring("simulator UDID", udid)?;
    let mut error_message = std::ptr::null_mut();
    let raw = unsafe { ffi::simdeck_camera_status(udid.as_ptr(), &mut error_message) };
    native_json(raw, error_message, "Unable to read camera status.")
}

fn native_switch_camera(
    udid: &str,
    source: &CameraSource,
    mirror: Option<&str>,
) -> Result<Value, AppError> {
    let udid = cstring("simulator UDID", udid)?;
    let source_name = cstring("camera source", source.kind.as_native_arg())?;
    let source_arg = cstring(
        "camera source argument",
        source.arg.as_deref().unwrap_or(""),
    )?;
    let mirror = cstring("camera mirror", mirror.unwrap_or(""))?;
    let mut error_message = std::ptr::null_mut();
    let raw = unsafe {
        ffi::simdeck_camera_switch(
            udid.as_ptr(),
            source_name.as_ptr(),
            source_arg.as_ptr(),
            mirror.as_ptr(),
            &mut error_message,
        )
    };
    native_json(raw, error_message, "Unable to switch camera source.")
}

fn native_stop_camera(udid: &str) -> Result<(), AppError> {
    let udid = cstring("simulator UDID", udid)?;
    let mut error_message = std::ptr::null_mut();
    let ok = unsafe { ffi::simdeck_camera_stop(udid.as_ptr(), &mut error_message) };
    if ok {
        Ok(())
    } else {
        Err(native_error(
            error_message,
            "Unable to stop daemon camera simulation.",
        ))
    }
}

fn native_json(
    raw: *mut c_char,
    error_message: *mut c_char,
    fallback: &'static str,
) -> Result<Value, AppError> {
    if raw.is_null() {
        return Err(native_error(error_message, fallback));
    }
    let text = take_native_string(raw);
    serde_json::from_str(&text)
        .map_err(|error| AppError::internal(format!("Unable to parse camera JSON. {error}")))
}

fn native_error(raw: *mut c_char, fallback: &'static str) -> AppError {
    if raw.is_null() {
        return AppError::native(fallback);
    }
    let message = take_native_string(raw);
    if message.trim().is_empty() {
        AppError::native(fallback)
    } else {
        AppError::native(message)
    }
}

fn take_native_string(raw: *mut c_char) -> String {
    let value = unsafe { CStr::from_ptr(raw).to_string_lossy().into_owned() };
    unsafe { ffi::xcw_native_free_string(raw) };
    value
}

fn cstring(name: &str, value: &str) -> Result<CString, AppError> {
    CString::new(value).map_err(|_| AppError::bad_request(format!("{name} contains NUL byte.")))
}

fn launch_with_injector(
    udid: &str,
    bundle_id: &str,
    shm_name: &str,
    mirror: &str,
) -> Result<(), AppError> {
    let dylib = camera_injector_path().map_err(app_internal)?;
    let _ = Command::new("/usr/bin/xcrun")
        .args(["simctl", "terminate", udid, bundle_id])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let app_log = camera_app_log_file(udid);
    let mut child = Command::new("/usr/bin/xcrun")
        .arg("simctl")
        .arg("launch")
        .arg(format!("--stdout={}", app_log.display()))
        .arg(format!("--stderr={}", app_log.display()))
        .arg(udid)
        .arg(bundle_id)
        .env("SIMCTL_CHILD_DYLD_INSERT_LIBRARIES", dylib)
        .env("SIMCTL_CHILD_SIMDECK_CAMERA_SHM_NAME", shm_name)
        .env("SIMCTL_CHILD_SIMDECK_CAMERA_MIRROR", mirror)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| AppError::native(format!("Unable to launch camera app. {error}")))?;
    let start = Instant::now();
    let output = loop {
        match child.try_wait() {
            Ok(Some(_)) => break child.wait_with_output().map_err(app_internal)?,
            Ok(None) => {
                if start.elapsed() > Duration::from_secs(180) {
                    let _ = child.kill();
                    let output = child.wait_with_output().map_err(app_internal)?;
                    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
                    return Err(AppError::native(if stderr.is_empty() {
                        "xcrun simctl launch timed out after 180s.".to_owned()
                    } else {
                        format!("xcrun simctl launch timed out after 180s. {stderr}")
                    }));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(error) => {
                return Err(AppError::native(format!(
                    "Unable to wait for camera app launch. {error}"
                )))
            }
        }
    };
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        Err(AppError::native(if stderr.is_empty() {
            stdout
        } else {
            stderr
        }))
    }
}

fn normalize_source(mut source: CameraSource) -> Result<CameraSource, AppError> {
    if let Some(arg) = source.arg.as_deref() {
        let trimmed = arg.trim();
        source.arg = (!trimmed.is_empty()).then(|| trimmed.to_owned());
    }
    match source.kind {
        CameraSourceKind::Placeholder => {
            source.arg = None;
        }
        CameraSourceKind::Image | CameraSourceKind::Video => {
            let arg = source.arg.as_deref().ok_or_else(|| {
                AppError::bad_request("Camera file or stream source requires `arg`.")
            })?;
            if !is_url(arg) {
                let path = Path::new(arg);
                if !path.is_absolute() {
                    return Err(AppError::bad_request(
                        "Camera file source must be an absolute path.",
                    ));
                }
                if !path.exists() {
                    return Err(AppError::not_found(format!(
                        "Camera media source does not exist: {}",
                        path.display()
                    )));
                }
            }
        }
        CameraSourceKind::Webcam => {}
    }
    Ok(source)
}

pub fn file_source(path_or_url: &str) -> CameraSource {
    let kind = if is_video_source(path_or_url) {
        CameraSourceKind::Video
    } else {
        CameraSourceKind::Image
    };
    CameraSource {
        kind,
        arg: Some(path_or_url.to_owned()),
    }
}

fn is_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://") || value.starts_with("file://")
}

fn is_video_source(value: &str) -> bool {
    if is_url(value) {
        return true;
    }
    let ext = Path::new(value)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(
        ext.as_str(),
        "mp4" | "m4v" | "mov" | "qt" | "avi" | "mkv" | "webm" | "mpg" | "mpeg" | "3gp" | "3g2"
    )
}

fn normalize_mirror(value: Option<&str>) -> Result<String, AppError> {
    let normalized = value.unwrap_or("auto").trim().to_ascii_lowercase();
    match normalized.as_str() {
        "auto" | "on" | "off" => Ok(normalized),
        _ => Err(AppError::bad_request(
            "Camera mirror must be one of `auto`, `on`, or `off`.",
        )),
    }
}

fn validate_udid(udid: &str) -> Result<(), AppError> {
    if udid.trim().is_empty() || udid.contains('/') || udid.contains('\0') {
        return Err(AppError::bad_request("Invalid simulator UDID."));
    }
    Ok(())
}

fn validate_bundle_id(bundle_id: &str) -> Result<(), AppError> {
    let valid = !bundle_id.is_empty()
        && bundle_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_');
    if valid {
        Ok(())
    } else {
        Err(AppError::bad_request("Invalid bundle identifier."))
    }
}

fn enrich_status(udid: &str, status: &mut Value) {
    let Some(object) = status.as_object_mut() else {
        return;
    };
    object.insert("udid".to_owned(), Value::String(udid.to_owned()));
    if let Some(pid) = object
        .get("processId")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
    {
        object.insert("daemonPid".to_owned(), json!(pid));
    }
    object.insert("bundleIds".to_owned(), json!(read_injected_bundles(udid)));
    object.insert(
        "appLogPath".to_owned(),
        Value::String(camera_app_log_file(udid).display().to_string()),
    );
}

fn record_injected_bundle(udid: &str, bundle_id: &str) -> Result<(), AppError> {
    let mut bundle_ids = read_injected_bundles(udid);
    if !bundle_ids.iter().any(|current| current == bundle_id) {
        bundle_ids.push(bundle_id.to_owned());
    }
    let payload = json!({
        "daemonPid": std::process::id(),
        "bundleIds": bundle_ids,
    });
    fs::write(
        injected_bundles_file(udid),
        serde_json::to_vec(&payload).map_err(app_internal)?,
    )
    .map_err(app_internal)
}

fn read_injected_bundles(udid: &str) -> Vec<String> {
    let path = injected_bundles_file(udid);
    let Ok(data) = fs::read(path) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_slice::<Value>(&data) else {
        return Vec::new();
    };
    let stored_pid = value.get("daemonPid").and_then(Value::as_u64).unwrap_or(0) as u32;
    if stored_pid != std::process::id() {
        return Vec::new();
    }
    value
        .get("bundleIds")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(ToOwned::to_owned)
        .collect()
}

fn camera_state_dir() -> PathBuf {
    std::env::temp_dir().join("simdeck-camera")
}

fn camera_app_log_file(udid: &str) -> PathBuf {
    camera_state_dir().join(format!("{}.app.log", short_hash(udid)))
}

fn injected_bundles_file(udid: &str) -> PathBuf {
    camera_state_dir().join(format!("{}.bundles.json", short_hash(udid)))
}

fn shm_name_for_udid(udid: &str) -> String {
    format!("/sd-cam-{}", short_hash(udid))
}

fn short_hash(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    hex::encode(&digest[..6])
}

fn camera_injector_path() -> anyhow::Result<PathBuf> {
    camera_artifact_path(INJECTOR_NAME)
}

fn camera_artifact_path(name: &str) -> anyhow::Result<PathBuf> {
    let exe = std::env::current_exe().context("resolve current executable")?;
    let exe_dir = exe
        .parent()
        .ok_or_else(|| anyhow!("current executable has no parent directory"))?;
    let cwd = std::env::current_dir().context("resolve current directory")?;
    let candidates = [
        exe_dir.join("camera").join(name),
        exe_dir.join(name),
        cwd.join("build").join("camera").join(name),
        cwd.join("cli").join("camera").join("build").join(name),
    ];
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| {
            anyhow!(
                "Camera artifact `{}` is missing. Run `npm run build:cli` from the SimDeck checkout.",
                name
            )
        })
}

fn app_internal(error: impl std::fmt::Display) -> AppError {
    AppError::internal(error.to_string())
}
