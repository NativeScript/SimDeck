use crate::error::AppError;
use anyhow::{anyhow, Context};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::net::Shutdown;
#[cfg(unix)]
use std::os::unix::net::UnixStream;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const HELPER_NAME: &str = "simdeck-camera-helper";
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
    fn as_helper_arg(&self) -> &'static str {
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
    let helper = camera_helper_path().map_err(app_internal)?;
    let output = Command::new(helper)
        .arg("--list-webcams-json")
        .output()
        .map_err(|error| AppError::native(format!("Unable to list Mac cameras. {error}")))?;
    if !output.status.success() {
        return Err(AppError::native(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| AppError::internal(format!("Unable to parse camera list. {error}")))
}

pub fn start_camera(options: CameraStartOptions) -> Result<Value, AppError> {
    validate_udid(&options.udid)?;
    let source = normalize_source(options.source)?;
    let mirror = normalize_mirror(options.mirror.as_deref())?;
    let helper = ensure_helper(&options.udid, &source, &mirror)?;
    let mut status = helper_status(&options.udid).unwrap_or_else(|_| json!({ "alive": true }));
    if let Some(bundle_id) = options
        .bundle_id
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        validate_bundle_id(bundle_id)?;
        launch_with_injector(&options.udid, bundle_id, &helper.shm_name, &mirror)?;
        record_injected_bundle(&options.udid, bundle_id, helper.pid)?;
    }
    enrich_status(&options.udid, &mut status, Some(helper.pid));
    Ok(status)
}

pub fn switch_camera(
    udid: &str,
    source: CameraSource,
    mirror: Option<String>,
) -> Result<Value, AppError> {
    validate_udid(udid)?;
    let source = normalize_source(source)?;
    let reply = send_helper_command(
        udid,
        &json!({
            "action": "switch",
            "source": source.kind.as_helper_arg(),
            "arg": source.arg,
        }),
    )?;
    if !reply.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        return Err(AppError::native(
            reply
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("Camera helper rejected source switch."),
        ));
    }
    if let Some(mirror) = mirror {
        let mirror = normalize_mirror(Some(&mirror))?;
        set_mirror(udid, &mirror)?;
    }
    let mut status = helper_status(udid)?;
    enrich_status(udid, &mut status, None);
    Ok(status)
}

pub fn camera_status(udid: &str) -> Result<Value, AppError> {
    validate_udid(udid)?;
    let mut status = if is_helper_alive(udid) {
        helper_status(udid)?
    } else {
        json!({ "ok": true, "alive": false })
    };
    enrich_status(udid, &mut status, None);
    Ok(status)
}

pub fn stop_camera(udid: &str) -> Result<Value, AppError> {
    validate_udid(udid)?;
    let pid = read_helper_pid(udid);
    if is_helper_alive(udid) {
        let _ = send_helper_command(udid, &json!({ "action": "shutdown" }));
    }
    if let Some(pid) = pid {
        wait_for_process_exit(pid, Duration::from_millis(1200));
        if process_alive(pid) {
            let _ = kill_process(pid);
        }
    }
    let _ = fs::remove_file(helper_pid_file(udid));
    let _ = fs::remove_file(helper_socket_file(udid));
    let _ = fs::remove_file(injected_bundles_file(udid));
    Ok(json!({ "ok": true, "udid": udid, "alive": false }))
}

struct HelperState {
    pid: u32,
    shm_name: String,
}

fn ensure_helper(udid: &str, source: &CameraSource, mirror: &str) -> Result<HelperState, AppError> {
    let shm_name = shm_name_for_udid(udid);
    if is_helper_alive(udid) {
        let reply = send_helper_command(
            udid,
            &json!({
                "action": "switch",
                "source": source.kind.as_helper_arg(),
                "arg": source.arg,
            }),
        )?;
        if !reply.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            return Err(AppError::native(
                reply
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("Camera helper rejected source switch."),
            ));
        }
        set_mirror(udid, mirror)?;
        return Ok(HelperState {
            pid: read_helper_pid(udid).unwrap_or(0),
            shm_name,
        });
    }

    let _ = stop_camera(udid);
    fs::create_dir_all(camera_state_dir()).map_err(app_internal)?;
    let helper_path = camera_helper_path().map_err(app_internal)?;
    let socket_path = helper_socket_file(udid);
    let log_path = helper_log_file(udid);
    let mut helper_args = vec![
        OsString::from("--shm"),
        OsString::from(shm_name.as_str()),
        OsString::from("--socket"),
        socket_path.as_os_str().to_owned(),
        OsString::from("--source"),
        OsString::from(source.kind.as_helper_arg()),
        OsString::from("--mirror"),
        OsString::from(mirror),
    ];
    if let Some(arg) = source.arg.as_deref().filter(|value| !value.is_empty()) {
        helper_args.push(OsString::from("--arg"));
        helper_args.push(OsString::from(arg));
    }

    let app_bundle = helper_app_bundle(&helper_path);
    let launched_app_bundle = app_bundle.is_some();
    let mut command = if let Some(app_bundle) = app_bundle {
        let mut command = Command::new("/usr/bin/open");
        command
            .arg("-n")
            .arg("-W")
            .arg("-o")
            .arg(&log_path)
            .arg("--stderr")
            .arg(&log_path)
            .arg(app_bundle)
            .arg("--args")
            .args(&helper_args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    } else {
        let stdout = append_log(&log_path)?;
        let stderr = append_log(&log_path)?;
        let mut command = Command::new(&helper_path);
        command
            .args(&helper_args)
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        #[cfg(unix)]
        unsafe {
            command.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        command
    };
    let mut child = command
        .spawn()
        .map_err(|error| AppError::native(format!("Unable to start camera helper. {error}")))?;
    let launcher_pid = child.id();
    fs::write(helper_pid_file(udid), launcher_pid.to_string()).map_err(app_internal)?;
    let start = Instant::now();
    let ready_timeout = match source.kind {
        CameraSourceKind::Webcam => Duration::from_secs(70),
        _ => Duration::from_secs(4),
    };
    let mut saw_socket = false;
    while start.elapsed() < ready_timeout {
        if !launched_app_bundle && !process_alive(launcher_pid) {
            let _ = child.try_wait();
            return Err(AppError::native(format!(
                "Camera helper exited before it was ready. See {}",
                log_path.display()
            )));
        }
        if socket_path.exists() {
            saw_socket = true;
            if let Ok(status) = send_helper_command(udid, &json!({ "action": "status" })) {
                if helper_status_matches_source(&status, source) {
                    let pid = status_helper_pid(&status).unwrap_or(launcher_pid);
                    fs::write(helper_pid_file(udid), pid.to_string()).map_err(app_internal)?;
                    return Ok(HelperState { pid, shm_name });
                }
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let wait_target = if saw_socket {
        format!("camera helper source `{}`", source.kind.as_helper_arg())
    } else {
        "camera helper socket".to_owned()
    };
    Err(AppError::native(format!(
        "Timed out waiting for {wait_target}. See {}",
        log_path.display()
    )))
}

fn helper_status_matches_source(status: &Value, source: &CameraSource) -> bool {
    status.get("ok").and_then(Value::as_bool).unwrap_or(false)
        && status
            .get("source")
            .and_then(Value::as_str)
            .is_some_and(|value| value == source.kind.as_helper_arg())
}

fn status_helper_pid(status: &Value) -> Option<u32> {
    status
        .get("processId")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
}

fn helper_status(udid: &str) -> Result<Value, AppError> {
    let mut status = send_helper_command(udid, &json!({ "action": "status" }))?;
    enrich_status(udid, &mut status, None);
    Ok(status)
}

fn set_mirror(udid: &str, mirror: &str) -> Result<(), AppError> {
    let reply = send_helper_command(udid, &json!({ "action": "mirror", "mode": mirror }))?;
    if reply.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        Ok(())
    } else {
        Err(AppError::native(
            reply
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("Unable to set camera mirror mode."),
        ))
    }
}

#[cfg(unix)]
fn send_helper_command(udid: &str, command: &Value) -> Result<Value, AppError> {
    let socket_path = helper_socket_file(udid);
    if !socket_path.exists() {
        return Err(AppError::not_found("Camera helper is not running."));
    }
    let mut stream = UnixStream::connect(&socket_path)
        .map_err(|error| AppError::native(format!("Unable to connect camera helper. {error}")))?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));
    let line = serde_json::to_vec(command).map_err(app_internal)?;
    stream.write_all(&line).map_err(app_internal)?;
    stream.write_all(b"\n").map_err(app_internal)?;
    let _ = stream.shutdown(Shutdown::Write);
    let mut reader = BufReader::new(stream);
    let mut response = String::new();
    reader.read_line(&mut response).map_err(app_internal)?;
    serde_json::from_str(response.trim()).map_err(|error| {
        AppError::internal(format!("Unable to parse camera helper response. {error}"))
    })
}

#[cfg(not(unix))]
fn send_helper_command(_udid: &str, _command: &Value) -> Result<Value, AppError> {
    Err(AppError::native(
        "Camera simulation is only supported on macOS.",
    ))
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
    let app_log = helper_app_log_file(udid);
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

fn enrich_status(udid: &str, status: &mut Value, helper_pid: Option<u32>) {
    let Some(object) = status.as_object_mut() else {
        return;
    };
    object.insert("udid".to_owned(), Value::String(udid.to_owned()));
    let status_pid = object
        .get("processId")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok());
    if let Some(pid) = helper_pid.or(status_pid).or_else(|| read_helper_pid(udid)) {
        object.insert("helperPid".to_owned(), json!(pid));
    }
    object.insert("bundleIds".to_owned(), json!(read_injected_bundles(udid)));
    object.insert(
        "helperLogPath".to_owned(),
        Value::String(helper_log_file(udid).display().to_string()),
    );
}

fn record_injected_bundle(udid: &str, bundle_id: &str, helper_pid: u32) -> Result<(), AppError> {
    let mut bundle_ids = read_injected_bundles(udid);
    if !bundle_ids.iter().any(|current| current == bundle_id) {
        bundle_ids.push(bundle_id.to_owned());
    }
    let payload = json!({
        "helperPid": helper_pid,
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
    let stored_pid = value.get("helperPid").and_then(Value::as_u64).unwrap_or(0) as u32;
    if Some(stored_pid) != read_helper_pid(udid) {
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

fn append_log(path: &Path) -> Result<File, AppError> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| AppError::native(format!("Unable to open camera helper log. {error}")))
}

fn is_helper_alive(udid: &str) -> bool {
    if !helper_socket_file(udid).exists() {
        return false;
    }
    if read_helper_pid(udid).is_some_and(process_alive) {
        return true;
    }
    send_helper_command(udid, &json!({ "action": "status" }))
        .ok()
        .and_then(|status| status.get("alive").and_then(Value::as_bool))
        .unwrap_or(false)
}

fn read_helper_pid(udid: &str) -> Option<u32> {
    fs::read_to_string(helper_pid_file(udid))
        .ok()?
        .trim()
        .parse::<u32>()
        .ok()
}

fn wait_for_process_exit(pid: u32, timeout: Duration) {
    let start = Instant::now();
    while start.elapsed() < timeout && process_alive(pid) {
        std::thread::sleep(Duration::from_millis(40));
    }
}

fn process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    unsafe {
        libc::kill(pid as libc::pid_t, 0) == 0
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

fn kill_process(pid: u32) -> Result<(), AppError> {
    #[cfg(unix)]
    unsafe {
        if libc::kill(pid as libc::pid_t, libc::SIGTERM) == 0 {
            Ok(())
        } else {
            Err(AppError::native(
                std::io::Error::last_os_error().to_string(),
            ))
        }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        Ok(())
    }
}

fn camera_state_dir() -> PathBuf {
    std::env::temp_dir().join("simdeck-camera")
}

fn helper_pid_file(udid: &str) -> PathBuf {
    camera_state_dir().join(format!("{}.pid", short_hash(udid)))
}

fn helper_log_file(udid: &str) -> PathBuf {
    camera_state_dir().join(format!("{}.log", short_hash(udid)))
}

fn helper_app_log_file(udid: &str) -> PathBuf {
    camera_state_dir().join(format!("{}.app.log", short_hash(udid)))
}

fn helper_socket_file(udid: &str) -> PathBuf {
    std::env::temp_dir().join(format!("simdeck-cam-{}.sock", short_hash(udid)))
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

fn camera_helper_path() -> anyhow::Result<PathBuf> {
    let exe = std::env::current_exe().context("resolve current executable")?;
    let exe_dir = exe
        .parent()
        .ok_or_else(|| anyhow!("current executable has no parent directory"))?;
    let cwd = std::env::current_dir().context("resolve current directory")?;
    let bundled = Path::new("SimDeckCameraHelper.app")
        .join("Contents")
        .join("MacOS")
        .join(HELPER_NAME);
    let candidates = [
        exe_dir.join("camera").join(&bundled),
        cwd.join("build").join("camera").join(&bundled),
        exe_dir.join("camera").join(HELPER_NAME),
        exe_dir.join(HELPER_NAME),
        cwd.join("build").join("camera").join(HELPER_NAME),
        cwd.join("cli")
            .join("camera")
            .join("build")
            .join(HELPER_NAME),
    ];
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| {
            anyhow!(
                "Camera artifact `{}` is missing. Run `npm run build:cli` from the SimDeck checkout.",
                HELPER_NAME
            )
        })
}

fn helper_app_bundle(helper_path: &Path) -> Option<PathBuf> {
    let macos_dir = helper_path.parent()?;
    if macos_dir.file_name()? != "MacOS" {
        return None;
    }
    let contents_dir = macos_dir.parent()?;
    if contents_dir.file_name()? != "Contents" {
        return None;
    }
    let app_bundle = contents_dir.parent()?;
    (app_bundle.extension()? == "app").then(|| app_bundle.to_path_buf())
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
