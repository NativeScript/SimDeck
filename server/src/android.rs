use crate::error::AppError;
use bytes::BytesMut;
use http::uri::PathAndQuery;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::env;
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use tonic::metadata::MetadataValue;
use tonic::transport::Endpoint;

const ANDROID_ID_PREFIX: &str = "android:";
const DEFAULT_GRPC_PORT_BASE: u16 = 8554;
const ANDROID_GRPC_FRAME_MESSAGE_LIMIT: usize = 64 * 1024 * 1024;
const ANDROID_TOUCH_SWIPE_THRESHOLD: f64 = 0.025;
const ANDROID_TOUCH_MIN_DURATION_MS: u128 = 80;
const ANDROID_TOUCH_MAX_DURATION_MS: u128 = 1500;
const ANDROID_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const ANDROID_UIAUTOMATOR_DUMP_ATTEMPTS: usize = 10;
const ANDROID_UIAUTOMATOR_DUMP_RETRY_DELAY: Duration = Duration::from_millis(250);
const RUNNING_EMULATOR_CACHE_TTL: Duration = Duration::from_secs(2);
const AVD_GRPC_PORT_CACHE_TTL: Duration = Duration::from_secs(60);
const SCREEN_SIZE_CACHE_TTL: Duration = Duration::from_secs(1);
const MODIFIER_SHIFT: u32 = 1 << 0;
const MODIFIER_CONTROL: u32 = 1 << 1;
const MODIFIER_OPTION: u32 = 1 << 2;
const MODIFIER_COMMAND: u32 = 1 << 3;
const MODIFIER_CAPS_LOCK: u32 = 1 << 4;

type TimedMap<T> = Option<(Instant, HashMap<String, T>)>;
type DisplayMetricsCache = HashMap<String, (Instant, AndroidDisplayMetrics)>;

#[derive(Clone, Copy, Debug, PartialEq)]
struct AndroidDisplayMetrics {
    width: f64,
    height: f64,
    rotation_quarter_turns: u16,
    corner_radii: AndroidCornerRadii,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct AndroidCornerRadii {
    top_left: f64,
    top_right: f64,
    bottom_right: f64,
    bottom_left: f64,
}

impl AndroidCornerRadii {
    const ZERO: Self = Self {
        top_left: 0.0,
        top_right: 0.0,
        bottom_right: 0.0,
        bottom_left: 0.0,
    };

    fn max(self) -> f64 {
        self.top_left
            .max(self.top_right)
            .max(self.bottom_right)
            .max(self.bottom_left)
    }
}

#[derive(Clone, Default)]
pub struct AndroidBridge;

#[derive(Clone, Debug)]
pub struct AndroidDevice {
    pub avd_name: String,
    pub serial: Option<String>,
    pub is_booted: bool,
    pub grpc_port: u16,
}

#[derive(Debug)]
pub struct AndroidFrame {
    pub width: u32,
    pub height: u32,
    pub timestamp_us: u64,
    pub rgba: Vec<u8>,
}

pub struct AndroidGrpcFrameStream {
    inner: tonic::Streaming<grpc::Image>,
    target: Option<AndroidFrameTarget>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AndroidFrameTarget {
    width: u32,
    height: u32,
    rotation_quarter_turns: u16,
}

#[derive(Debug)]
pub struct AndroidTouchGesture {
    started_at: Instant,
    start_x: f64,
    start_y: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AndroidTouchAction {
    None,
    Tap {
        x: f64,
        y: f64,
    },
    Swipe {
        start_x: f64,
        start_y: f64,
        end_x: f64,
        end_y: f64,
        duration_ms: u64,
    },
}

impl AndroidTouchAction {
    pub fn perform(self, bridge: &AndroidBridge, id: &str) -> Result<(), AppError> {
        match self {
            AndroidTouchAction::None => Ok(()),
            AndroidTouchAction::Tap { x, y } => bridge.send_tap_adb(id, x, y),
            AndroidTouchAction::Swipe {
                start_x,
                start_y,
                end_x,
                end_y,
                duration_ms,
            } => bridge.send_swipe_adb(id, start_x, start_y, end_x, end_y, duration_ms),
        }
    }
}

pub fn is_android_id(id: &str) -> bool {
    id.starts_with(ANDROID_ID_PREFIX)
}

pub fn avd_from_id(id: &str) -> Result<String, AppError> {
    id.strip_prefix(ANDROID_ID_PREFIX)
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| AppError::bad_request(format!("Invalid Android emulator id `{id}`.")))
}

pub fn id_for_avd(avd_name: &str) -> String {
    format!("{ANDROID_ID_PREFIX}{avd_name}")
}

pub fn update_touch_gesture(
    active_touch: &mut Option<AndroidTouchGesture>,
    x: f64,
    y: f64,
    phase: &str,
) -> Result<AndroidTouchAction, AppError> {
    if !x.is_finite() || !y.is_finite() {
        return Err(AppError::bad_request(
            "`x` and `y` must be finite normalized numbers.",
        ));
    }
    let x = x.clamp(0.0, 1.0);
    let y = y.clamp(0.0, 1.0);

    match phase {
        "began" => {
            *active_touch = Some(AndroidTouchGesture {
                started_at: Instant::now(),
                start_x: x,
                start_y: y,
            });
            Ok(AndroidTouchAction::None)
        }
        "moved" => Ok(AndroidTouchAction::None),
        "ended" => {
            let touch = active_touch.take().unwrap_or(AndroidTouchGesture {
                started_at: Instant::now(),
                start_x: x,
                start_y: y,
            });
            let distance = ((x - touch.start_x).powi(2) + (y - touch.start_y).powi(2)).sqrt();
            if distance < ANDROID_TOUCH_SWIPE_THRESHOLD {
                return Ok(AndroidTouchAction::Tap { x, y });
            }
            Ok(AndroidTouchAction::Swipe {
                start_x: touch.start_x,
                start_y: touch.start_y,
                end_x: x,
                end_y: y,
                duration_ms: touch
                    .started_at
                    .elapsed()
                    .as_millis()
                    .clamp(ANDROID_TOUCH_MIN_DURATION_MS, ANDROID_TOUCH_MAX_DURATION_MS)
                    as u64,
            })
        }
        "cancelled" => {
            *active_touch = None;
            Ok(AndroidTouchAction::None)
        }
        _ => Ok(AndroidTouchAction::None),
    }
}

impl AndroidBridge {
    pub fn list_devices(&self) -> Result<Vec<AndroidDevice>, AppError> {
        if !self.emulator_path().exists() {
            return Ok(Vec::new());
        }

        let avds = self
            .run_emulator(["-list-avds"])?
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        if avds.is_empty() {
            return Ok(Vec::new());
        }

        let running = self.running_emulators().unwrap_or_default();
        Ok(avds
            .into_iter()
            .enumerate()
            .map(|(index, avd_name)| AndroidDevice {
                serial: running.get(&avd_name).cloned(),
                is_booted: running.contains_key(&avd_name),
                grpc_port: DEFAULT_GRPC_PORT_BASE + index as u16,
                avd_name,
            })
            .collect())
    }

    pub fn enrich_devices(&self, devices: Vec<AndroidDevice>) -> Vec<Value> {
        devices
            .into_iter()
            .map(|device| self.device_value(device))
            .collect()
    }

    pub fn boot(&self, id: &str) -> Result<bool, AppError> {
        let avd_name = avd_from_id(id)?;
        if self.resolve_serial(&avd_name).is_ok() {
            return Ok(false);
        }
        let grpc_port = self.grpc_port_for_avd(&avd_name)?;
        Command::new(self.emulator_path())
            .args([
                "-avd",
                &avd_name,
                "-no-window",
                "-no-audio",
                "-gpu",
                "swiftshader_indirect",
                "-grpc",
                &grpc_port.to_string(),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| {
                AppError::native(format!(
                    "Unable to start Android emulator `{avd_name}`: {error}"
                ))
            })?;
        Ok(true)
    }

    pub fn shutdown(&self, id: &str) -> Result<(), AppError> {
        let avd_name = avd_from_id(id)?;
        let serial = self.resolve_serial(&avd_name)?;
        let _ = self.run_adb(["-s", &serial, "emu", "kill"])?;
        Ok(())
    }

    pub fn erase(&self, id: &str) -> Result<(), AppError> {
        let avd_name = avd_from_id(id)?;
        if self.resolve_serial(&avd_name).is_ok() {
            return Err(AppError::bad_request(
                "Shutdown the Android emulator before erasing it.",
            ));
        }
        let avd_dir = self.avd_dir(&avd_name);
        for file_name in [
            "userdata-qemu.img",
            "cache.img",
            "data.img",
            "sdcard.img",
            "snapshots.img",
        ] {
            let path = avd_dir.join(file_name);
            if path.exists() {
                std::fs::remove_file(&path).map_err(|error| {
                    AppError::native(format!("Unable to remove {}: {error}", path.display()))
                })?;
            }
        }
        Ok(())
    }

    pub fn wait_until_booted(&self, id: &str, timeout_duration: Duration) -> Result<(), AppError> {
        let avd_name = avd_from_id(id)?;
        let deadline = Instant::now() + timeout_duration;
        loop {
            if let Ok(serial) = self.resolve_serial(&avd_name) {
                if self
                    .run_adb(["-s", &serial, "shell", "getprop", "sys.boot_completed"])
                    .unwrap_or_default()
                    .trim()
                    == "1"
                {
                    return Ok(());
                }
            }
            if Instant::now() >= deadline {
                return Err(AppError::native(format!(
                    "Android emulator `{avd_name}` did not finish booting in time."
                )));
            }
            thread::sleep(Duration::from_millis(500));
        }
    }

    pub fn screenshot_png(&self, id: &str) -> Result<Vec<u8>, AppError> {
        let serial = self.serial_for_id(id)?;
        self.run_adb_bytes(["-s", &serial, "exec-out", "screencap", "-p"])
    }

    pub fn install_app(&self, id: &str, app_path: &str) -> Result<(), AppError> {
        if !app_path.ends_with(".apk") {
            return Err(AppError::bad_request(
                "Android install expects an `.apk` path.",
            ));
        }
        let serial = self.serial_for_id(id)?;
        self.run_adb(["-s", &serial, "install", "-r", app_path])?;
        Ok(())
    }

    pub fn uninstall_app(&self, id: &str, package_name: &str) -> Result<(), AppError> {
        let serial = self.serial_for_id(id)?;
        self.run_adb(["-s", &serial, "uninstall", package_name])?;
        Ok(())
    }

    pub fn open_url(&self, id: &str, url: &str) -> Result<(), AppError> {
        let serial = self.serial_for_id(id)?;
        self.run_adb([
            "-s",
            &serial,
            "shell",
            "am",
            "start",
            "-a",
            "android.intent.action.VIEW",
            "-d",
            url,
        ])?;
        Ok(())
    }

    pub fn launch_package(&self, id: &str, package: &str) -> Result<(), AppError> {
        let serial = self.serial_for_id(id)?;
        self.run_adb([
            "-s",
            &serial,
            "shell",
            "am",
            "start",
            "-a",
            "android.intent.action.MAIN",
            "-c",
            "android.intent.category.LAUNCHER",
            "-p",
            package,
        ])?;
        Ok(())
    }

    pub fn set_pasteboard_text(&self, id: &str, text: &str) -> Result<(), AppError> {
        let serial = self.serial_for_id(id)?;
        let output =
            self.run_adb_shell(&serial, &format!("cmd clipboard set {}", shell_quote(text)))?;
        ensure_android_clipboard_available(&output)?;
        Ok(())
    }

    pub fn pasteboard_text(&self, id: &str) -> Result<String, AppError> {
        let serial = self.serial_for_id(id)?;
        let output = self.run_adb_shell(&serial, "cmd clipboard get")?;
        ensure_android_clipboard_available(&output)?;
        Ok(output.trim_end_matches(['\r', '\n']).to_owned())
    }

    pub fn send_touch(&self, id: &str, x: f64, y: f64, phase: &str) -> Result<(), AppError> {
        match phase {
            "ended" => self.send_tap_adb(id, x, y),
            _ => Ok(()),
        }
    }

    fn send_tap_adb(&self, id: &str, x: f64, y: f64) -> Result<(), AppError> {
        let serial = self.serial_for_id(id)?;
        let (width, height) = self.screen_size_for_serial(&serial)?;
        let px = (x.clamp(0.0, 1.0) * (width - 1.0)).round().max(0.0);
        let py = (y.clamp(0.0, 1.0) * (height - 1.0)).round().max(0.0);
        self.run_adb([
            "-s",
            &serial,
            "shell",
            "input",
            "tap",
            &px.to_string(),
            &py.to_string(),
        ])?;
        Ok(())
    }

    pub fn send_swipe(
        &self,
        id: &str,
        start_x: f64,
        start_y: f64,
        end_x: f64,
        end_y: f64,
        duration_ms: u64,
    ) -> Result<(), AppError> {
        self.send_swipe_adb(id, start_x, start_y, end_x, end_y, duration_ms)
    }

    fn send_swipe_adb(
        &self,
        id: &str,
        start_x: f64,
        start_y: f64,
        end_x: f64,
        end_y: f64,
        duration_ms: u64,
    ) -> Result<(), AppError> {
        let serial = self.serial_for_id(id)?;
        let (width, height) = self.screen_size_for_serial(&serial)?;
        let coords = [start_x, start_y, end_x, end_y]
            .into_iter()
            .enumerate()
            .map(|(index, value)| {
                let max = if index % 2 == 0 {
                    width - 1.0
                } else {
                    height - 1.0
                };
                (value.clamp(0.0, 1.0) * max).round().max(0.0).to_string()
            })
            .collect::<Vec<_>>();
        self.run_adb([
            "-s",
            &serial,
            "shell",
            "input",
            "swipe",
            &coords[0],
            &coords[1],
            &coords[2],
            &coords[3],
            &duration_ms.to_string(),
        ])?;
        Ok(())
    }

    pub fn send_key(&self, id: &str, key_code: u16, modifiers: u32) -> Result<(), AppError> {
        if let Some(text) = hid_text_for_key(key_code, modifiers) {
            return self.type_text_adb(id, &text);
        }

        let serial = self.serial_for_id(id)?;
        let android_key = android_key_code(key_code);
        if has_android_key_modifiers(modifiers) {
            return self.press_android_key_combination(&serial, android_key, modifiers);
        }
        self.press_android_key(&serial, android_key)
    }

    pub fn type_text(&self, id: &str, text: &str) -> Result<(), AppError> {
        self.type_text_adb(id, text)
    }

    pub fn dismiss_keyboard(&self, id: &str) -> Result<(), AppError> {
        let serial = self.serial_for_id(id)?;
        self.press_android_key(&serial, 4)
    }

    fn type_text_adb(&self, id: &str, text: &str) -> Result<(), AppError> {
        let serial = self.serial_for_id(id)?;
        let escaped = android_input_text_arg(text);
        self.run_adb(["-s", &serial, "shell", "input", "text", &escaped])?;
        Ok(())
    }

    fn press_android_key(&self, serial: &str, key_code: u16) -> Result<(), AppError> {
        let key_code = key_code.to_string();
        self.run_adb(["-s", serial, "shell", "input", "keyevent", &key_code])?;
        Ok(())
    }

    fn press_android_key_combination(
        &self,
        serial: &str,
        key_code: u16,
        modifiers: u32,
    ) -> Result<(), AppError> {
        let mut parts = vec!["input".to_owned(), "keycombination".to_owned()];
        parts.extend(
            android_modifier_key_codes(modifiers)
                .into_iter()
                .map(|key| key.to_string()),
        );
        parts.push(key_code.to_string());
        match self.run_adb_shell(serial, &parts.join(" ")) {
            Ok(_) => Ok(()),
            Err(_) => self.press_android_key(serial, key_code),
        }
    }

    pub fn press_home(&self, id: &str) -> Result<(), AppError> {
        let serial = self.serial_for_id(id)?;
        self.run_adb(["-s", &serial, "shell", "input", "keyevent", "3"])?;
        Ok(())
    }

    pub fn open_app_switcher(&self, id: &str) -> Result<(), AppError> {
        let serial = self.serial_for_id(id)?;
        self.run_adb(["-s", &serial, "shell", "input", "keyevent", "187"])?;
        Ok(())
    }

    pub fn press_button(&self, id: &str, button: &str, duration_ms: u32) -> Result<(), AppError> {
        match button {
            "home" => self.press_home(id),
            "lock" | "side-button" => {
                let serial = self.serial_for_id(id)?;
                self.run_adb(["-s", &serial, "shell", "input", "keyevent", "26"])?;
                if duration_ms > 500 {
                    thread::sleep(Duration::from_millis(u64::from(duration_ms)));
                    self.run_adb(["-s", &serial, "shell", "input", "keyevent", "26"])?;
                }
                Ok(())
            }
            "back" => {
                let serial = self.serial_for_id(id)?;
                self.run_adb(["-s", &serial, "shell", "input", "keyevent", "4"])?;
                Ok(())
            }
            _ => Err(AppError::bad_request(format!(
                "Unsupported Android hardware button `{button}`."
            ))),
        }
    }

    pub fn rotate_right(&self, id: &str) -> Result<(), AppError> {
        self.rotate_by_quarter_turns(id, 1)
    }

    pub fn rotate_left(&self, id: &str) -> Result<(), AppError> {
        self.rotate_by_quarter_turns(id, -1)
    }

    fn rotate_by_quarter_turns(&self, id: &str, delta: i16) -> Result<(), AppError> {
        let serial = self.serial_for_id(id)?;
        self.invalidate_display_metrics_for_serial(&serial);
        let current = self
            .display_metrics_for_serial(&serial)
            .map(|metrics| metrics.rotation_quarter_turns)
            .unwrap_or(0);
        let next = (i16::try_from(current).unwrap_or(0) + delta).rem_euclid(4) as u16;
        let rotation = next.to_string();
        let _ = self.run_adb([
            "-s",
            &serial,
            "shell",
            "cmd",
            "window",
            "set-ignore-orientation-request",
            "-d",
            "0",
            "true",
        ]);
        let _ = self.run_adb([
            "-s",
            &serial,
            "shell",
            "cmd",
            "window",
            "fixed-to-user-rotation",
            "-d",
            "0",
            "enabled",
        ]);
        self.run_adb([
            "-s",
            &serial,
            "shell",
            "cmd",
            "window",
            "user-rotation",
            "-d",
            "0",
            "lock",
            &rotation,
        ])?;
        self.invalidate_display_metrics_for_serial(&serial);
        self.wait_for_display_rotation(&serial, next);
        Ok(())
    }

    fn wait_for_display_rotation(&self, serial: &str, rotation: u16) {
        let deadline = Instant::now() + Duration::from_secs(4);
        while Instant::now() < deadline {
            self.invalidate_display_metrics_for_serial(serial);
            if self
                .display_metrics_for_serial(serial)
                .map(|metrics| metrics.rotation_quarter_turns == rotation)
                .unwrap_or(false)
            {
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }
        self.invalidate_display_metrics_for_serial(serial);
    }

    pub fn toggle_appearance(&self, id: &str) -> Result<(), AppError> {
        let serial = self.serial_for_id(id)?;
        let current = self.run_adb_shell(&serial, "cmd uimode night")?;
        let mode = if current.to_lowercase().contains("yes") {
            "no"
        } else {
            "yes"
        };
        self.run_adb(["-s", &serial, "shell", "cmd", "uimode", "night", mode])?;
        Ok(())
    }

    pub fn logs(&self, id: &str, limit: usize) -> Result<Vec<Value>, AppError> {
        let serial = self.serial_for_id(id)?;
        let raw = self.run_adb([
            "-s",
            &serial,
            "logcat",
            "-d",
            "-v",
            "threadtime",
            "-t",
            &limit.max(1).to_string(),
        ])?;
        Ok(raw
            .lines()
            .map(|line| {
                json!({
                    "timestamp": "",
                    "level": android_log_level(line),
                    "process": "",
                    "pid": Value::Null,
                    "subsystem": "android",
                    "category": "logcat",
                    "message": line,
                })
            })
            .collect())
    }

    pub fn chrome_profile(&self, id: &str) -> Result<Value, AppError> {
        let serial = self.serial_for_id(id)?;
        let metrics = self.display_metrics_for_serial(&serial)?;
        let width = metrics.width;
        let height = metrics.height;
        let radii = metrics.corner_radii;
        Ok(json!({
            "totalWidth": width,
            "totalHeight": height,
            "screenX": 0,
            "screenY": 0,
            "screenWidth": width,
            "screenHeight": height,
            "cornerRadius": radii.max(),
            "cornerRadii": {
                "topLeft": radii.top_left,
                "topRight": radii.top_right,
                "bottomRight": radii.bottom_right,
                "bottomLeft": radii.bottom_left,
            },
            "hasScreenMask": false,
        }))
    }

    pub async fn grpc_frame_stream(
        &self,
        id: &str,
        max_edge: Option<u32>,
    ) -> Result<AndroidGrpcFrameStream, AppError> {
        let avd_name = avd_from_id(id)?;
        let port = self.grpc_port_for_avd(&avd_name)?;
        let mut format = grpc::ImageFormat {
            format: grpc::image_format::ImgFormat::Rgba8888 as i32,
            width: 0,
            height: 0,
            display: 0,
            transport: None,
        };
        let target = self
            .resolve_serial(&avd_name)
            .ok()
            .and_then(|serial| self.display_metrics_for_serial(&serial).ok())
            .map(|metrics| AndroidFrameTarget {
                width: metrics.width.round().max(1.0) as u32,
                height: metrics.height.round().max(1.0) as u32,
                rotation_quarter_turns: metrics.rotation_quarter_turns,
            });
        if let (Some(max_edge), Some(target)) = (max_edge, target) {
            let max_edge = max_edge.clamp(240, 2400);
            let largest = target.width.max(target.height);
            if largest > max_edge {
                if target.width >= target.height {
                    format.width = max_edge;
                } else {
                    format.height = max_edge;
                }
            }
        }

        let endpoint = Endpoint::from_shared(format!("http://127.0.0.1:{port}"))
            .map_err(|error| AppError::native(format!("Invalid Android gRPC endpoint: {error}")))?
            .connect()
            .await
            .map_err(|error| {
                AppError::native(format!(
                    "Unable to connect to Android emulator gRPC: {error}"
                ))
            })?;
        let mut grpc = tonic::client::Grpc::new(endpoint)
            .max_decoding_message_size(ANDROID_GRPC_FRAME_MESSAGE_LIMIT);
        grpc.ready().await.map_err(|error| {
            AppError::native(format!("Android emulator gRPC is not ready: {error}"))
        })?;
        let path = PathAndQuery::from_static(
            "/android.emulation.control.EmulatorController/streamScreenshot",
        );
        let mut request = tonic::Request::new(format);
        if let Some(token) = emulator_grpc_token(port) {
            let value = MetadataValue::try_from(format!("Bearer {token}")).map_err(|error| {
                AppError::native(format!("Invalid Android emulator gRPC token: {error}"))
            })?;
            request.metadata_mut().insert("authorization", value);
        }
        let response = grpc
            .server_streaming(request, path, tonic::codec::ProstCodec::default())
            .await
            .map_err(|error| {
                AppError::native(format!(
                    "Android emulator screenshot stream failed: {error}"
                ))
            })?;
        Ok(AndroidGrpcFrameStream {
            inner: response.into_inner(),
            target,
        })
    }

    pub fn accessibility_tree(
        &self,
        id: &str,
        max_depth: Option<usize>,
    ) -> Result<Value, AppError> {
        let serial = self.serial_for_id(id)?;
        let max_depth = max_depth.unwrap_or(80).min(80);
        let mut last_error = None;
        for attempt in 1..=ANDROID_UIAUTOMATOR_DUMP_ATTEMPTS {
            match self.android_accessibility_tree_for_serial(&serial, max_depth) {
                Ok(tree) => return Ok(tree),
                Err(error) => last_error = Some(error),
            }
            if attempt < ANDROID_UIAUTOMATOR_DUMP_ATTEMPTS {
                thread::sleep(ANDROID_UIAUTOMATOR_DUMP_RETRY_DELAY);
            }
        }

        Err(last_error.unwrap_or_else(|| {
            AppError::native("Unable to capture Android UIAutomator hierarchy.")
        }))
    }

    fn android_accessibility_tree_for_serial(
        &self,
        serial: &str,
        max_depth: usize,
    ) -> Result<Value, AppError> {
        let raw = self.run_adb_shell(
            serial,
            "uiautomator dump /sdcard/simdeck_ui.xml >/dev/null && cat /sdcard/simdeck_ui.xml",
        )?;
        let xml = extract_xml(&raw);
        let document = roxmltree::Document::parse(xml).map_err(|error| {
            AppError::native(format!("Unable to parse UIAutomator XML: {error}"))
        })?;
        let root = document.root_element();
        let (width, height) = self.screen_size_for_serial(serial)?;
        let mut roots = Vec::new();
        for child in root.children().filter(|node| node.has_tag_name("node")) {
            roots.push(android_node_value(child, 0, max_depth));
        }
        if roots.is_empty() {
            roots.push(json!({
                "type": "screen",
                "role": "screen",
                "frame": frame_value(0.0, 0.0, width, height),
                "children": [],
            }));
        }
        Ok(json!({
            "source": "android-uiautomator",
            "availableSources": ["android-uiautomator"],
            "roots": roots,
        }))
    }

    fn device_value(&self, device: AndroidDevice) -> Value {
        let id = id_for_avd(&device.avd_name);
        let private_display = if let Some(serial) = device.serial.as_deref() {
            let metrics =
                self.display_metrics_for_serial(serial)
                    .unwrap_or(AndroidDisplayMetrics {
                        width: 0.0,
                        height: 0.0,
                        rotation_quarter_turns: 0,
                        corner_radii: AndroidCornerRadii::ZERO,
                    });
            json!({
                "displayReady": metrics.width > 0.0 && metrics.height > 0.0,
                "displayStatus": "Ready",
                "displayWidth": metrics.width,
                "displayHeight": metrics.height,
                "frameSequence": 0,
                "rotationQuarterTurns": metrics.rotation_quarter_turns,
            })
        } else {
            json!({
                "displayReady": false,
                "displayStatus": "Boot required",
                "displayWidth": 0,
                "displayHeight": 0,
                "frameSequence": 0,
                "rotationQuarterTurns": 0,
            })
        };
        json!({
            "udid": id,
            "id": id,
            "platform": "android-emulator",
            "name": device.avd_name,
            "state": if device.is_booted { "Booted" } else { "Shutdown" },
            "isBooted": device.is_booted,
            "isAvailable": true,
            "lastBootedAt": Value::Null,
            "dataPath": self.avd_dir(&device.avd_name),
            "logPath": Value::Null,
            "deviceTypeIdentifier": "android-emulator",
            "deviceTypeName": "Android Emulator",
            "runtimeIdentifier": "android",
            "runtimeName": "Android",
            "android": {
                "avdName": device.avd_name,
                "serial": device.serial,
                "grpcPort": device.grpc_port,
            },
            "privateDisplay": private_display,
        })
    }

    fn serial_for_id(&self, id: &str) -> Result<String, AppError> {
        self.resolve_serial(&avd_from_id(id)?)
    }

    fn resolve_serial(&self, avd_name: &str) -> Result<String, AppError> {
        self.running_emulators()?.remove(avd_name).ok_or_else(|| {
            AppError::native(format!("Android emulator `{avd_name}` is not running."))
        })
    }

    fn running_emulators(&self) -> Result<HashMap<String, String>, AppError> {
        static CACHE: OnceLock<Mutex<TimedMap<String>>> = OnceLock::new();
        let cache = CACHE.get_or_init(|| Mutex::new(None));
        if let Some((updated_at, running)) = cache.lock().unwrap().as_ref() {
            if updated_at.elapsed() < RUNNING_EMULATOR_CACHE_TTL {
                return Ok(running.clone());
            }
        }
        if !self.adb_path().exists() {
            return Ok(HashMap::new());
        }
        let output = self.run_adb(["devices"])?;
        let mut result = HashMap::new();
        for line in output.lines().skip(1) {
            let mut parts = line.split_whitespace();
            let Some(serial) = parts.next() else { continue };
            let Some(state) = parts.next() else { continue };
            if state != "device" || !serial.starts_with("emulator-") {
                continue;
            }
            if let Some(name) = self.avd_name_for_serial(serial) {
                result.insert(name, serial.to_owned());
            }
        }
        *cache.lock().unwrap() = Some((Instant::now(), result.clone()));
        Ok(result)
    }

    fn avd_name_for_serial(&self, serial: &str) -> Option<String> {
        for property in ["ro.boot.qemu.avd_name", "ro.kernel.qemu.avd_name"] {
            if let Ok(output) = self.run_adb(["-s", serial, "shell", "getprop", property]) {
                let name = output.trim();
                if !name.is_empty() {
                    return Some(name.to_owned());
                }
            }
        }
        self.run_adb(["-s", serial, "emu", "avd", "name"])
            .ok()
            .and_then(|output| {
                output
                    .lines()
                    .map(str::trim)
                    .find(|line| !line.is_empty() && *line != "OK")
                    .map(ToOwned::to_owned)
            })
    }

    fn grpc_port_for_avd(&self, avd_name: &str) -> Result<u16, AppError> {
        static CACHE: OnceLock<Mutex<TimedMap<u16>>> = OnceLock::new();
        let cache = CACHE.get_or_init(|| Mutex::new(None));
        if let Some((updated_at, ports)) = cache.lock().unwrap().as_ref() {
            if updated_at.elapsed() < AVD_GRPC_PORT_CACHE_TTL {
                if let Some(port) = ports.get(avd_name) {
                    return Ok(*port);
                }
            }
        }

        let ports = self
            .run_emulator(["-list-avds"])?
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .enumerate()
            .map(|(index, name)| (name.to_owned(), DEFAULT_GRPC_PORT_BASE + index as u16))
            .collect::<HashMap<_, _>>();
        let port = ports
            .get(avd_name)
            .copied()
            .ok_or_else(|| AppError::not_found(format!("Unknown Android AVD `{avd_name}`.")))?;
        *cache.lock().unwrap() = Some((Instant::now(), ports));
        Ok(port)
    }

    fn screen_size_for_serial(&self, serial: &str) -> Result<(f64, f64), AppError> {
        let metrics = self.display_metrics_for_serial(serial)?;
        Ok((metrics.width, metrics.height))
    }

    fn display_metrics_for_serial(&self, serial: &str) -> Result<AndroidDisplayMetrics, AppError> {
        let cache = android_display_metrics_cache();
        if let Some((updated_at, metrics)) = cache.lock().unwrap().get(serial) {
            if updated_at.elapsed() < SCREEN_SIZE_CACHE_TTL {
                return Ok(*metrics);
            }
        }
        let output = self.run_adb(["-s", serial, "shell", "dumpsys", "display"])?;
        let metrics = parse_android_display_metrics(&output)
            .or_else(|| self.wm_display_metrics_for_serial(serial).ok())
            .ok_or_else(|| AppError::native("Android emulator did not report display metrics."))?;
        cache
            .lock()
            .unwrap()
            .insert(serial.to_owned(), (Instant::now(), metrics));
        Ok(metrics)
    }

    fn wm_display_metrics_for_serial(
        &self,
        serial: &str,
    ) -> Result<AndroidDisplayMetrics, AppError> {
        let output = self.run_adb(["-s", serial, "shell", "wm", "size"])?;
        let size = output
            .split_whitespace()
            .find(|part| part.contains('x'))
            .ok_or_else(|| AppError::native("Android emulator did not report a screen size."))?;
        let (width, height) = size
            .split_once('x')
            .ok_or_else(|| AppError::native("Android emulator reported an invalid screen size."))?;
        let width = width
            .parse::<f64>()
            .map_err(|_| AppError::native("Android emulator reported an invalid width."))?;
        let height = height
            .parse::<f64>()
            .map_err(|_| AppError::native("Android emulator reported an invalid height."))?;
        Ok(AndroidDisplayMetrics {
            width,
            height,
            rotation_quarter_turns: 0,
            corner_radii: AndroidCornerRadii::ZERO,
        })
    }

    fn invalidate_display_metrics_for_serial(&self, serial: &str) {
        android_display_metrics_cache()
            .lock()
            .unwrap()
            .remove(serial);
    }

    fn run_adb_shell(&self, serial: &str, script: &str) -> Result<String, AppError> {
        self.run_adb(["-s", serial, "shell", script])
    }

    fn run_adb<const N: usize>(&self, args: [&str; N]) -> Result<String, AppError> {
        run_command_text(self.adb_path(), args)
    }

    fn run_adb_bytes<const N: usize>(&self, args: [&str; N]) -> Result<Vec<u8>, AppError> {
        run_command_bytes(self.adb_path(), args)
    }

    fn run_emulator<const N: usize>(&self, args: [&str; N]) -> Result<String, AppError> {
        run_command_text(self.emulator_path(), args)
    }

    fn adb_path(&self) -> PathBuf {
        sdk_root().join("platform-tools/adb")
    }

    fn emulator_path(&self) -> PathBuf {
        sdk_root().join("emulator/emulator")
    }

    fn avd_dir(&self, avd_name: &str) -> PathBuf {
        home_dir().join(format!(".android/avd/{avd_name}.avd"))
    }
}

impl AndroidGrpcFrameStream {
    pub async fn next_frame(&mut self) -> Result<Option<AndroidFrame>, AppError> {
        let Some(image) = self.inner.message().await.map_err(|error| {
            AppError::native(format!(
                "Android emulator screenshot stream failed: {error}"
            ))
        })?
        else {
            return Ok(None);
        };
        let format = image.format.ok_or_else(|| {
            AppError::native("Android emulator screenshot did not include an image format.")
        })?;
        let width = if format.width > 0 {
            format.width
        } else {
            image.width
        };
        let height = if format.height > 0 {
            format.height
        } else {
            image.height
        };
        if width == 0 || height == 0 {
            return Err(AppError::native(
                "Android emulator screenshot did not include dimensions.",
            ));
        }
        let rgba = rgba_display_order(
            &image.image,
            width,
            height,
            grpc::image_format::ImgFormat::try_from(format.format)
                .unwrap_or(grpc::image_format::ImgFormat::Rgba8888),
        )?;
        let (width, height, mut rgba) =
            normalize_android_frame_orientation(width, height, rgba, self.target);
        flatten_android_frame_alpha(&mut rgba, width, height);
        Ok(Some(AndroidFrame {
            width,
            height,
            timestamp_us: image.timestamp_us,
            rgba,
        }))
    }
}

fn normalize_android_frame_orientation(
    width: u32,
    height: u32,
    mut rgba: Vec<u8>,
    target: Option<AndroidFrameTarget>,
) -> (u32, u32, Vec<u8>) {
    let Some(target) = target else {
        return (width, height, rgba);
    };
    if width == 0 || height == 0 || target.width == 0 || target.height == 0 {
        return (width, height, rgba);
    }

    let (width, height) = if (width > height) == (target.width > target.height) {
        (width, height)
    } else {
        rgba = if target.rotation_quarter_turns == 3 {
            rotate_rgba_counterclockwise(&rgba, width, height)
        } else {
            rotate_rgba_clockwise(&rgba, width, height)
        };
        (height, width)
    };

    if target.width > target.height {
        rotate_rgba_180_in_place(&mut rgba, width, height);
    }

    (width, height, rgba)
}

fn rotate_rgba_clockwise(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let width = width as usize;
    let height = height as usize;
    let mut out = vec![0; rgba.len()];
    for y in 0..height {
        for x in 0..width {
            let src = (y * width + x) * 4;
            let dst_x = height - 1 - y;
            let dst_y = x;
            let dst = (dst_y * height + dst_x) * 4;
            out[dst..dst + 4].copy_from_slice(&rgba[src..src + 4]);
        }
    }
    out
}

fn rotate_rgba_180_in_place(rgba: &mut [u8], width: u32, height: u32) {
    let pixel_count = width as usize * height as usize;
    for pixel in 0..(pixel_count / 2) {
        let opposite = pixel_count - 1 - pixel;
        for channel in 0..4 {
            rgba.swap(pixel * 4 + channel, opposite * 4 + channel);
        }
    }
}

fn rotate_rgba_counterclockwise(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let width = width as usize;
    let height = height as usize;
    let mut out = vec![0; rgba.len()];
    for y in 0..height {
        for x in 0..width {
            let src = (y * width + x) * 4;
            let dst_x = y;
            let dst_y = width - 1 - x;
            let dst = (dst_y * height + dst_x) * 4;
            out[dst..dst + 4].copy_from_slice(&rgba[src..src + 4]);
        }
    }
    out
}

fn flatten_android_frame_alpha(rgba: &mut [u8], width: u32, height: u32) {
    if !rgba.chunks_exact(4).any(|pixel| pixel[3] != 255) {
        return;
    }

    let width = width as usize;
    let height = height as usize;
    let Some(default_fill) = first_opaque_rgb(rgba) else {
        for pixel in rgba.chunks_exact_mut(4) {
            pixel[3] = 255;
        }
        return;
    };

    for y in 0..height {
        let row_start = y * width * 4;
        let row = &mut rgba[row_start..row_start + width * 4];
        let mut fill = first_opaque_rgb(row).unwrap_or(default_fill);
        for pixel in row.chunks_exact_mut(4) {
            if pixel[3] == 255 {
                fill = [pixel[0], pixel[1], pixel[2]];
                continue;
            }
            composite_pixel_over_rgb(pixel, fill);
        }
    }
}

fn first_opaque_rgb(rgba: &[u8]) -> Option<[u8; 3]> {
    rgba.chunks_exact(4)
        .find(|pixel| pixel[3] == 255)
        .map(|pixel| [pixel[0], pixel[1], pixel[2]])
}

fn composite_pixel_over_rgb(pixel: &mut [u8], background: [u8; 3]) {
    let alpha = u32::from(pixel[3]);
    if alpha == 0 {
        pixel[0] = background[0];
        pixel[1] = background[1];
        pixel[2] = background[2];
        pixel[3] = 255;
        return;
    }

    for channel in 0..3 {
        pixel[channel] = ((u32::from(pixel[channel]) * alpha
            + u32::from(background[channel]) * (255 - alpha)
            + 127)
            / 255) as u8;
    }
    pixel[3] = 255;
}

fn run_command_text<const N: usize>(program: PathBuf, args: [&str; N]) -> Result<String, AppError> {
    let output = run_command(program, args)?;
    String::from_utf8(output)
        .map_err(|error| AppError::native(format!("Command returned non-UTF8 output: {error}")))
}

fn run_command_bytes<const N: usize>(
    program: PathBuf,
    args: [&str; N],
) -> Result<Vec<u8>, AppError> {
    run_command(program, args)
}

fn run_command<const N: usize>(program: PathBuf, args: [&str; N]) -> Result<Vec<u8>, AppError> {
    if !program.exists() {
        return Err(AppError::native(format!(
            "Android SDK binary not found at {}.",
            program.display()
        )));
    }
    let mut child = Command::new(&program)
        .args(args)
        .env("ANDROID_HOME", sdk_root())
        .env("ANDROID_SDK_ROOT", sdk_root())
        .env("JAVA_HOME", java_home())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            AppError::native(format!("Unable to run {}: {error}", program.display()))
        })?;
    let stdout = child.stdout.take().ok_or_else(|| {
        AppError::native(format!(
            "Unable to capture stdout from {}.",
            program.display()
        ))
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        AppError::native(format!(
            "Unable to capture stderr from {}.",
            program.display()
        ))
    })?;
    let stdout_reader = thread::spawn(move || read_command_stream(stdout));
    let stderr_reader = thread::spawn(move || read_command_stream(stderr));
    let deadline = Instant::now() + ANDROID_COMMAND_TIMEOUT;
    let status = loop {
        match child.try_wait().map_err(|error| {
            AppError::native(format!("Unable to wait for {}: {error}", program.display()))
        })? {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait().map_err(|error| {
                    AppError::native(format!(
                        "Unable to wait for timed-out {}: {error}",
                        program.display()
                    ))
                })?;
                let stdout = join_command_reader(stdout_reader, &program, "stdout")?;
                let stderr = join_command_reader(stderr_reader, &program, "stderr")?;
                let stderr_detail = command_stream_summary(&stderr)
                    .map(|summary| format!(": {summary}"))
                    .unwrap_or_default();
                return Err(AppError::native(format!(
                    "{} timed out after {}s{} (stdout {} bytes, stderr {} bytes)",
                    command_name(&program),
                    ANDROID_COMMAND_TIMEOUT.as_secs(),
                    stderr_detail,
                    stdout.len(),
                    stderr.len()
                )));
            }
            None => thread::sleep(Duration::from_millis(25)),
        }
    };
    let stdout = join_command_reader(stdout_reader, &program, "stdout")?;
    let stderr = join_command_reader(stderr_reader, &program, "stderr")?;
    if status.success() {
        let stderr_text = String::from_utf8_lossy(&stderr);
        if stderr_text.contains("No shell command implementation") {
            return Err(AppError::native(stderr_text.trim().to_owned()));
        }
        return Ok(stdout);
    }
    let stderr = command_stream_summary(&stderr).unwrap_or_default();
    let stdout = command_stream_summary(&stdout);
    Err(AppError::native(format!(
        "{} failed: {}{}",
        command_name(&program),
        stderr,
        stdout.map(|value| format!(" {value}")).unwrap_or_default()
    )))
}

fn read_command_stream(mut stream: impl Read) -> std::io::Result<Vec<u8>> {
    let mut buffer = Vec::new();
    stream.read_to_end(&mut buffer)?;
    Ok(buffer)
}

fn join_command_reader(
    reader: thread::JoinHandle<std::io::Result<Vec<u8>>>,
    program: &Path,
    stream_name: &str,
) -> Result<Vec<u8>, AppError> {
    reader
        .join()
        .map_err(|_| {
            AppError::native(format!(
                "Unable to read {stream_name} from {}.",
                program.display()
            ))
        })?
        .map_err(|error| {
            AppError::native(format!(
                "Unable to read {stream_name} from {}: {error}",
                program.display()
            ))
        })
}

fn command_name(program: &Path) -> &str {
    program
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Android command")
}

fn command_stream_summary(bytes: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(bytes);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut summary = trimmed.chars().take(2000).collect::<String>();
    if trimmed.chars().nth(2000).is_some() {
        summary.push_str("...");
    }
    Some(summary)
}

fn sdk_root() -> PathBuf {
    env::var_os("ANDROID_HOME")
        .or_else(|| env::var_os("ANDROID_SDK_ROOT"))
        .map(PathBuf::from)
        .filter(|path| path.exists())
        .unwrap_or_else(|| home_dir().join("Library/Android/sdk"))
}

fn java_home() -> OsString {
    env::var_os("JAVA_HOME").unwrap_or_else(|| OsString::from("/opt/homebrew/opt/openjdk"))
}

fn home_dir() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new("/").to_path_buf())
}

fn emulator_grpc_token(port: u16) -> Option<String> {
    per_instance_grpc_token(port).or_else(global_grpc_token)
}

fn per_instance_grpc_token(port: u16) -> Option<String> {
    let running_dir = home_dir().join("Library/Caches/TemporaryItems/avd/running");
    let entries = std::fs::read_dir(running_dir).ok()?;
    let port_value = port.to_string();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("ini") {
            continue;
        }
        let contents = std::fs::read_to_string(path).ok()?;
        let fields = parse_ini(&contents);
        if fields.get("grpc.port") == Some(&port_value) {
            if let Some(token) = fields.get("grpc.token").filter(|token| !token.is_empty()) {
                return Some(token.to_owned());
            }
        }
    }
    None
}

fn global_grpc_token() -> Option<String> {
    std::fs::read_to_string(home_dir().join(".emulator_console_auth_token"))
        .ok()
        .map(|token| token.trim().to_owned())
        .filter(|token| !token.is_empty())
}

fn parse_ini(contents: &str) -> HashMap<String, String> {
    contents
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let (key, value) = line.split_once('=')?;
            Some((key.trim().to_owned(), value.trim().to_owned()))
        })
        .collect()
}

fn rgba_display_order(
    image: &[u8],
    width: u32,
    height: u32,
    format: grpc::image_format::ImgFormat,
) -> Result<Vec<u8>, AppError> {
    let width = width as usize;
    let height = height as usize;
    match format {
        grpc::image_format::ImgFormat::Rgba8888 => {
            let row_len = width * 4;
            if image.len() < row_len * height {
                return Err(AppError::native(
                    "Android emulator returned a truncated RGBA frame.",
                ));
            }
            Ok(image[..row_len * height].to_vec())
        }
        grpc::image_format::ImgFormat::Rgb888 => {
            let src_row_len = width * 3;
            if image.len() < src_row_len * height {
                return Err(AppError::native(
                    "Android emulator returned a truncated RGB frame.",
                ));
            }
            let mut out = BytesMut::with_capacity(width * height * 4);
            out.resize(width * height * 4, 255);
            for y in 0..height {
                let src_row = y * src_row_len;
                let dst_row = y * width * 4;
                for x in 0..width {
                    let src = src_row + x * 3;
                    let dst = dst_row + x * 4;
                    out[dst] = image[src];
                    out[dst + 1] = image[src + 1];
                    out[dst + 2] = image[src + 2];
                    out[dst + 3] = 255;
                }
            }
            Ok(out.to_vec())
        }
        grpc::image_format::ImgFormat::Png => Err(AppError::native(
            "Android emulator gRPC returned PNG instead of raw pixels.",
        )),
    }
}

fn extract_xml(output: &str) -> &str {
    output
        .find("<?xml")
        .or_else(|| output.find("<hierarchy"))
        .map(|index| &output[index..])
        .unwrap_or(output)
}

fn android_node_value(node: roxmltree::Node<'_, '_>, depth: usize, max_depth: usize) -> Value {
    let bounds = parse_bounds(node.attribute("bounds").unwrap_or(""));
    let class_name = node.attribute("class").unwrap_or("");
    let short_class = class_name.rsplit('.').next().unwrap_or(class_name);
    let text = node.attribute("text").unwrap_or("");
    let content_desc = node.attribute("content-desc").unwrap_or("");
    let label = if !text.is_empty() { text } else { content_desc };
    let resource_id = node.attribute("resource-id").unwrap_or("");
    let role = android_role(node, short_class);
    let mut children = Vec::new();
    if depth < max_depth {
        for child in node.children().filter(|child| child.has_tag_name("node")) {
            children.push(android_node_value(child, depth + 1, max_depth));
        }
    }
    json!({
        "source": "android-uiautomator",
        "type": android_type(short_class, class_name),
        "role": role,
        "className": class_name,
        "AXIdentifier": resource_id,
        "AXLabel": label,
        "AXValue": text,
        "androidClass": class_name,
        "androidPackage": node.attribute("package").unwrap_or(""),
        "androidResourceId": resource_id,
        "checkable": bool_attr(node, "checkable"),
        "checked": bool_attr(node, "checked"),
        "clickable": bool_attr(node, "clickable"),
        "focusable": bool_attr(node, "focusable"),
        "focused": bool_attr(node, "focused"),
        "longClickable": bool_attr(node, "long-clickable"),
        "password": bool_attr(node, "password"),
        "scrollable": bool_attr(node, "scrollable"),
        "selected": bool_attr(node, "selected"),
        "text": text,
        "title": label,
        "enabled": bool_attr(node, "enabled"),
        "isHidden": node.attribute("visible-to-user") == Some("false"),
        "frame": frame_value(bounds.0, bounds.1, bounds.2, bounds.3),
        "frameInScreen": frame_value(bounds.0, bounds.1, bounds.2, bounds.3),
        "children": children,
    })
}

fn parse_bounds(value: &str) -> (f64, f64, f64, f64) {
    let numbers = value
        .replace("][", ",")
        .replace(['[', ']'], "")
        .split(',')
        .filter_map(|part| part.parse::<f64>().ok())
        .collect::<Vec<_>>();
    if numbers.len() != 4 {
        return (0.0, 0.0, 0.0, 0.0);
    }
    (
        numbers[0],
        numbers[1],
        (numbers[2] - numbers[0]).max(0.0),
        (numbers[3] - numbers[1]).max(0.0),
    )
}

fn frame_value(x: f64, y: f64, width: f64, height: f64) -> Value {
    json!({ "x": x, "y": y, "width": width, "height": height })
}

fn bool_attr(node: roxmltree::Node<'_, '_>, name: &str) -> bool {
    node.attribute(name) == Some("true")
}

fn android_type(short_class: &str, class_name: &str) -> String {
    let fallback = if short_class.is_empty() {
        class_name
    } else {
        short_class
    };
    if fallback.is_empty() {
        "View".to_owned()
    } else {
        fallback.to_owned()
    }
}

fn android_display_metrics_cache() -> &'static Mutex<DisplayMetricsCache> {
    static CACHE: OnceLock<Mutex<DisplayMetricsCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn parse_android_display_metrics(output: &str) -> Option<AndroidDisplayMetrics> {
    let rotation = parse_android_display_rotation(output).unwrap_or(0);
    let corner_radii = parse_android_rounded_corners(output).unwrap_or(AndroidCornerRadii::ZERO);
    if let Some(line) = output
        .lines()
        .find(|line| line.contains("mOverrideDisplayInfo=DisplayInfo"))
    {
        if let Some((width, height)) = parse_display_info_app_size(line) {
            return Some(AndroidDisplayMetrics {
                width,
                height,
                rotation_quarter_turns: rotation,
                corner_radii,
            });
        }
    }
    if let Some((width, height)) = output.lines().find_map(parse_logical_frame_size) {
        return Some(AndroidDisplayMetrics {
            width,
            height,
            rotation_quarter_turns: rotation,
            corner_radii,
        });
    }
    None
}

fn parse_android_rounded_corners(output: &str) -> Option<AndroidCornerRadii> {
    let mut radii = AndroidCornerRadii::ZERO;
    let mut found = false;

    for chunk in output.split("RoundedCorner{").skip(1) {
        let section = chunk.split('}').next().unwrap_or(chunk);
        let Some(position) = parse_named_value(section, "position=") else {
            continue;
        };
        let Some(radius) =
            parse_named_value(section, "radius=").and_then(|value| value.parse::<f64>().ok())
        else {
            continue;
        };
        match position {
            "TopLeft" => radii.top_left = radius,
            "TopRight" => radii.top_right = radius,
            "BottomRight" => radii.bottom_right = radius,
            "BottomLeft" => radii.bottom_left = radius,
            _ => continue,
        }
        found = true;
    }

    found.then_some(radii)
}

fn parse_named_value<'a>(input: &'a str, key: &str) -> Option<&'a str> {
    let (_, value) = input.split_once(key)?;
    Some(value.split([',', '}', ']']).next().unwrap_or(value).trim())
}

fn parse_android_display_rotation(output: &str) -> Option<u16> {
    output
        .lines()
        .find(|line| line.contains("mOverrideDisplayInfo=DisplayInfo"))
        .and_then(parse_display_info_rotation)
        .or_else(|| {
            output
                .lines()
                .find_map(|line| {
                    line.split_once("mCurrentOrientation=")
                        .map(|(_, value)| value)
                })
                .and_then(parse_rotation_token)
        })
}

fn parse_display_info_app_size(line: &str) -> Option<(f64, f64)> {
    let (_, value) = line.rsplit_once(", app ")?;
    parse_size_prefix(value)
}

fn parse_display_info_rotation(line: &str) -> Option<u16> {
    let (_, value) = line.rsplit_once(", rotation ")?;
    parse_rotation_token(value)
}

fn parse_rotation_token(value: &str) -> Option<u16> {
    let digits = value
        .trim()
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect::<String>();
    let rotation = digits.parse::<u16>().ok()?;
    Some(rotation % 4)
}

fn parse_size_prefix(value: &str) -> Option<(f64, f64)> {
    let mut parts = value.split_whitespace();
    let width = parts.next()?.trim_end_matches(',').parse::<f64>().ok()?;
    if parts.next()? != "x" {
        return None;
    }
    let height = parts.next()?.trim_end_matches(',').parse::<f64>().ok()?;
    Some((width, height))
}

fn parse_logical_frame_size(line: &str) -> Option<(f64, f64)> {
    let (_, value) = line.split_once("logicalFrame=Rect(")?;
    let (frame, _) = value.split_once(')')?;
    let (_, max_values) = frame.split_once(" - ")?;
    let (width, height) = max_values.split_once(',')?;
    Some((width.trim().parse().ok()?, height.trim().parse().ok()?))
}

fn android_role(node: roxmltree::Node<'_, '_>, class_name: &str) -> &'static str {
    let clickable = bool_attr(node, "clickable");
    let scrollable = bool_attr(node, "scrollable");
    match class_name {
        "Button" | "ImageButton" | "FloatingActionButton" => "button",
        "EditText" => "textField",
        "TextView" => "staticText",
        "ImageView" => "image",
        "CheckBox" => "checkBox",
        "RadioButton" => "radioButton",
        "Switch" | "ToggleButton" => "switch",
        "SeekBar" => "slider",
        "RecyclerView" | "ListView" | "GridView" => "collection",
        "ScrollView" | "HorizontalScrollView" | "NestedScrollView" | "ViewPager" => "scrollView",
        "WebView" => "webView",
        "ProgressBar" => "progressIndicator",
        "Spinner" => "popUpButton",
        "TabWidget" | "TabLayout" => "tabGroup",
        "Toolbar" | "ActionBar" => "toolbar",
        "ViewGroup" | "FrameLayout" | "LinearLayout" | "RelativeLayout" | "ConstraintLayout"
        | "CoordinatorLayout" | "DrawerLayout" => "container",
        _ if scrollable => "scrollView",
        _ if clickable => "button",
        _ => "view",
    }
}

fn android_key_code(hid: u16) -> u16 {
    match hid {
        4..=29 => 29 + (hid - 4),
        30 => 8,
        31 => 9,
        32 => 10,
        33 => 11,
        34 => 12,
        35 => 13,
        36 => 14,
        37 => 15,
        38 => 16,
        39 => 7,
        40 => 66,
        41 => 111,
        42 => 67,
        43 => 61,
        44 => 62,
        45 => 69,
        46 => 70,
        47 => 71,
        48 => 72,
        49 => 73,
        51 => 74,
        52 => 75,
        53 => 68,
        54 => 55,
        55 => 56,
        56 => 76,
        57 => 115,
        58..=69 => 131 + (hid - 58),
        73 => 124,
        74 => 122,
        75 => 92,
        76 => 112,
        77 => 123,
        78 => 93,
        79 => 22,
        80 => 21,
        81 => 20,
        82 => 19,
        _ => hid,
    }
}

fn hid_text_for_key(hid: u16, modifiers: u32) -> Option<String> {
    if modifiers & (MODIFIER_CONTROL | MODIFIER_OPTION | MODIFIER_COMMAND) != 0 {
        return None;
    }

    let shifted = modifiers & MODIFIER_SHIFT != 0;
    let caps_locked = modifiers & MODIFIER_CAPS_LOCK != 0;

    if (4..=29).contains(&hid) {
        let offset = (hid - 4) as u8;
        let base = if shifted ^ caps_locked { b'A' } else { b'a' };
        return Some(char::from(base + offset).to_string());
    }

    let text = match (hid, shifted) {
        (30, false) => "1",
        (30, true) => "!",
        (31, false) => "2",
        (31, true) => "@",
        (32, false) => "3",
        (32, true) => "#",
        (33, false) => "4",
        (33, true) => "$",
        (34, false) => "5",
        (34, true) => "%",
        (35, false) => "6",
        (35, true) => "^",
        (36, false) => "7",
        (36, true) => "&",
        (37, false) => "8",
        (37, true) => "*",
        (38, false) => "9",
        (38, true) => "(",
        (39, false) => "0",
        (39, true) => ")",
        (44, _) => " ",
        (45, false) => "-",
        (45, true) => "_",
        (46, false) => "=",
        (46, true) => "+",
        (47, false) => "[",
        (47, true) => "{",
        (48, false) => "]",
        (48, true) => "}",
        (49, false) => "\\",
        (49, true) => "|",
        (51, false) => ";",
        (51, true) => ":",
        (52, false) => "'",
        (52, true) => "\"",
        (53, false) => "`",
        (53, true) => "~",
        (54, false) => ",",
        (54, true) => "<",
        (55, false) => ".",
        (55, true) => ">",
        (56, false) => "/",
        (56, true) => "?",
        _ => return None,
    };
    Some(text.to_owned())
}

fn android_input_text_arg(text: &str) -> String {
    let mut escaped = String::new();
    for character in text.chars() {
        match character {
            ' ' => escaped.push_str("%s"),
            '%' => escaped.push_str("%25"),
            '&' | '(' | ')' | '<' | '>' | ';' | '|' | '*' | '\\' | '"' | '\'' | '`' | '$' => {
                escaped.push('\\');
                escaped.push(character);
            }
            _ => escaped.push(character),
        }
    }
    escaped
}

fn has_android_key_modifiers(modifiers: u32) -> bool {
    modifiers & (MODIFIER_SHIFT | MODIFIER_CONTROL | MODIFIER_OPTION | MODIFIER_COMMAND) != 0
}

fn android_modifier_key_codes(modifiers: u32) -> Vec<u16> {
    let mut keys = Vec::new();
    if modifiers & MODIFIER_CONTROL != 0 {
        keys.push(113);
    }
    if modifiers & MODIFIER_OPTION != 0 {
        keys.push(57);
    }
    if modifiers & MODIFIER_SHIFT != 0 {
        keys.push(59);
    }
    if modifiers & MODIFIER_COMMAND != 0 {
        keys.push(117);
    }
    keys
}

fn android_log_level(line: &str) -> &'static str {
    if line.contains(" E ") {
        "error"
    } else if line.contains(" W ") {
        "warning"
    } else if line.contains(" D ") {
        "debug"
    } else {
        "info"
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn ensure_android_clipboard_available(output: &str) -> Result<(), AppError> {
    if output.contains("No shell command implementation") {
        return Err(AppError::native(
            "Android clipboard shell service is not implemented on this emulator image.",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn android_nodes_keep_class_type_and_semantic_role() {
        let document = roxmltree::Document::parse(
            r#"<node class="android.view.ViewGroup" package="com.example" resource-id="com.example:id/hotseat" bounds="[0,1873][1080,2400]" enabled="true" visible-to-user="true" clickable="false" scrollable="false" />"#,
        )
        .unwrap();

        let value = android_node_value(document.root_element(), 0, 10);

        assert_eq!(value["type"], "ViewGroup");
        assert_eq!(value["role"], "container");
        assert_eq!(value["AXIdentifier"], "com.example:id/hotseat");
        assert_eq!(value["androidClass"], "android.view.ViewGroup");
        assert_eq!(value["androidResourceId"], "com.example:id/hotseat");
        assert_eq!(value["enabled"], true);
    }

    #[test]
    fn clickable_unknown_android_nodes_are_buttons() {
        let document = roxmltree::Document::parse(
            r#"<node class="com.example.CustomTile" bounds="[10,20][110,70]" enabled="true" visible-to-user="true" clickable="true" />"#,
        )
        .unwrap();

        let value = android_node_value(document.root_element(), 0, 10);

        assert_eq!(value["type"], "CustomTile");
        assert_eq!(value["role"], "button");
    }

    #[test]
    fn android_touch_gesture_resolves_tap_on_end() {
        let mut active = None;

        assert_eq!(
            update_touch_gesture(&mut active, 0.4, 0.6, "began").unwrap(),
            AndroidTouchAction::None
        );
        assert_eq!(
            update_touch_gesture(&mut active, 0.41, 0.6, "ended").unwrap(),
            AndroidTouchAction::Tap { x: 0.41, y: 0.6 }
        );
    }

    #[test]
    fn android_touch_gesture_resolves_swipe_on_end() {
        let mut active = None;

        assert_eq!(
            update_touch_gesture(&mut active, 0.1, 0.2, "began").unwrap(),
            AndroidTouchAction::None
        );
        assert_eq!(
            update_touch_gesture(&mut active, 0.8, 0.2, "ended").unwrap(),
            AndroidTouchAction::Swipe {
                start_x: 0.1,
                start_y: 0.2,
                end_x: 0.8,
                end_y: 0.2,
                duration_ms: 80,
            }
        );
    }

    #[test]
    fn android_key_code_maps_usb_hid_keyboard_usages() {
        assert_eq!(android_key_code(4), 29);
        assert_eq!(android_key_code(29), 54);
        assert_eq!(android_key_code(30), 8);
        assert_eq!(android_key_code(39), 7);
        assert_eq!(android_key_code(40), 66);
        assert_eq!(android_key_code(42), 67);
        assert_eq!(android_key_code(58), 131);
        assert_eq!(android_key_code(69), 142);
        assert_eq!(android_key_code(73), 124);
        assert_eq!(android_key_code(79), 22);
    }

    #[test]
    fn hid_text_for_key_uses_shift_and_caps_for_printable_input() {
        assert_eq!(hid_text_for_key(4, 0).as_deref(), Some("a"));
        assert_eq!(hid_text_for_key(4, MODIFIER_SHIFT).as_deref(), Some("A"));
        assert_eq!(
            hid_text_for_key(4, MODIFIER_CAPS_LOCK).as_deref(),
            Some("A")
        );
        assert_eq!(
            hid_text_for_key(4, MODIFIER_SHIFT | MODIFIER_CAPS_LOCK).as_deref(),
            Some("a")
        );
        assert_eq!(hid_text_for_key(30, 0).as_deref(), Some("1"));
        assert_eq!(hid_text_for_key(30, MODIFIER_SHIFT).as_deref(), Some("!"));
        assert_eq!(hid_text_for_key(56, MODIFIER_SHIFT).as_deref(), Some("?"));
        assert_eq!(hid_text_for_key(80, 0), None);
        assert_eq!(hid_text_for_key(4, MODIFIER_COMMAND), None);
    }

    #[test]
    fn android_input_text_arg_escapes_adb_shell_text() {
        assert_eq!(android_input_text_arg("hello world"), "hello%sworld");
        assert_eq!(android_input_text_arg("100%"), "100%25");
        assert_eq!(android_input_text_arg("a&b"), "a\\&b");
    }

    #[test]
    fn android_modifier_key_codes_match_android_meta_keys() {
        assert_eq!(android_modifier_key_codes(MODIFIER_CONTROL), vec![113]);
        assert_eq!(android_modifier_key_codes(MODIFIER_OPTION), vec![57]);
        assert_eq!(android_modifier_key_codes(MODIFIER_SHIFT), vec![59]);
        assert_eq!(android_modifier_key_codes(MODIFIER_COMMAND), vec![117]);
        assert_eq!(
            android_modifier_key_codes(MODIFIER_CONTROL | MODIFIER_SHIFT),
            vec![113, 59]
        );
    }

    #[test]
    fn parse_android_display_metrics_prefers_current_app_size() {
        let output = r#"
  mViewports=[DisplayViewport{type=INTERNAL, logicalFrame=Rect(0, 0 - 2400, 1080), physicalFrame=Rect(0, 0 - 2400, 1080)}]
    mCurrentOrientation=3
    mOverrideDisplayInfo=DisplayInfo{"Built-in Screen", real 2400 x 1080, largest app 2400 x 2400, smallest app 1080 x 1080, rotation 3, state ON, app 2400 x 1080, density 420}
"#;

        assert_eq!(
            parse_android_display_metrics(output),
            Some(AndroidDisplayMetrics {
                width: 2400.0,
                height: 1080.0,
                rotation_quarter_turns: 3,
                corner_radii: AndroidCornerRadii::ZERO,
            })
        );
    }

    #[test]
    fn parse_android_display_metrics_falls_back_to_logical_frame() {
        let output = r#"
  mViewports=[DisplayViewport{type=INTERNAL, logicalFrame=Rect(0, 0 - 1080, 2400), physicalFrame=Rect(0, 0 - 1080, 2400)}]
    mCurrentOrientation=0
"#;

        assert_eq!(
            parse_android_display_metrics(output),
            Some(AndroidDisplayMetrics {
                width: 1080.0,
                height: 2400.0,
                rotation_quarter_turns: 0,
                corner_radii: AndroidCornerRadii::ZERO,
            })
        );
    }

    #[test]
    fn parse_android_display_metrics_reads_rounded_corners() {
        let output = r#"
DisplayDeviceInfo{"Built-in Screen", 1080 x 2400, roundedCorners RoundedCorners{[RoundedCorner{position=TopLeft, radius=104, center=Point(104, 104)}, RoundedCorner{position=TopRight, radius=104, center=Point(976, 104)}, RoundedCorner{position=BottomRight, radius=102, center=Point(978, 2298)}, RoundedCorner{position=BottomLeft, radius=102, center=Point(102, 2298)}]}}
  mViewports=[DisplayViewport{type=INTERNAL, logicalFrame=Rect(0, 0 - 1080, 2400), physicalFrame=Rect(0, 0 - 1080, 2400)}]
    mCurrentOrientation=0
"#;

        assert_eq!(
            parse_android_display_metrics(output),
            Some(AndroidDisplayMetrics {
                width: 1080.0,
                height: 2400.0,
                rotation_quarter_turns: 0,
                corner_radii: AndroidCornerRadii {
                    top_left: 104.0,
                    top_right: 104.0,
                    bottom_right: 102.0,
                    bottom_left: 102.0,
                },
            })
        );
    }

    #[test]
    fn android_frame_orientation_rotates_when_stream_aspect_is_swapped() {
        let rgba = vec![
            1, 0, 0, 255, 2, 0, 0, 255, 3, 0, 0, 255, //
            4, 0, 0, 255, 5, 0, 0, 255, 6, 0, 0, 255,
        ];

        let (width, height, rotated) = normalize_android_frame_orientation(
            3,
            2,
            rgba,
            Some(AndroidFrameTarget {
                width: 2,
                height: 3,
                rotation_quarter_turns: 0,
            }),
        );

        assert_eq!((width, height), (2, 3));
        assert_eq!(
            rotated,
            vec![
                4, 0, 0, 255, 1, 0, 0, 255, //
                5, 0, 0, 255, 2, 0, 0, 255, //
                6, 0, 0, 255, 3, 0, 0, 255,
            ]
        );
    }

    #[test]
    fn android_frame_orientation_keeps_matching_aspect() {
        let rgba = vec![1, 2, 3, 255, 5, 6, 7, 255];

        let (width, height, out) = normalize_android_frame_orientation(
            1,
            2,
            rgba.clone(),
            Some(AndroidFrameTarget {
                width: 1080,
                height: 2400,
                rotation_quarter_turns: 0,
            }),
        );

        assert_eq!((width, height), (1, 2));
        assert_eq!(out, rgba);
    }

    #[test]
    fn android_frame_orientation_flips_landscape_streams() {
        let rgba = vec![
            1, 0, 0, 255, 2, 0, 0, 255, 3, 0, 0, 255, //
            4, 0, 0, 255, 5, 0, 0, 255, 6, 0, 0, 255,
        ];

        let (width, height, rotated) = normalize_android_frame_orientation(
            3,
            2,
            rgba,
            Some(AndroidFrameTarget {
                width: 3,
                height: 2,
                rotation_quarter_turns: 1,
            }),
        );

        assert_eq!((width, height), (3, 2));
        assert_eq!(
            rotated,
            vec![
                6, 0, 0, 255, 5, 0, 0, 255, 4, 0, 0, 255, //
                3, 0, 0, 255, 2, 0, 0, 255, 1, 0, 0, 255,
            ]
        );
    }

    #[test]
    fn android_frame_alpha_flattens_transparent_edges_to_nearest_row_pixel() {
        let mut rgba = vec![0, 0, 0, 0, 10, 20, 30, 255, 40, 50, 60, 255, 0, 0, 0, 0];

        flatten_android_frame_alpha(&mut rgba, 4, 1);

        assert_eq!(
            rgba,
            vec![10, 20, 30, 255, 10, 20, 30, 255, 40, 50, 60, 255, 40, 50, 60, 255,]
        );
    }

    #[test]
    fn android_frame_alpha_composites_partial_alpha() {
        let mut rgba = vec![100, 50, 0, 255, 200, 200, 200, 128];

        flatten_android_frame_alpha(&mut rgba, 2, 1);

        assert_eq!(rgba, vec![100, 50, 0, 255, 150, 125, 100, 255]);
    }

    #[test]
    fn android_frame_orientation_rotates_counterclockwise_then_flips_reverse_landscape() {
        let rgba = vec![
            1, 0, 0, 255, 2, 0, 0, 255, 3, 0, 0, 255, //
            4, 0, 0, 255, 5, 0, 0, 255, 6, 0, 0, 255,
        ];

        let (width, height, rotated) = normalize_android_frame_orientation(
            2,
            3,
            rgba,
            Some(AndroidFrameTarget {
                width: 3,
                height: 2,
                rotation_quarter_turns: 3,
            }),
        );

        assert_eq!((width, height), (3, 2));
        assert_eq!(
            rotated,
            vec![
                5, 0, 0, 255, 3, 0, 0, 255, 1, 0, 0, 255, //
                6, 0, 0, 255, 4, 0, 0, 255, 2, 0, 0, 255,
            ]
        );
    }
}

mod grpc {
    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct ImageTransport {
        #[prost(enumeration = "image_transport::TransportChannel", tag = "1")]
        pub channel: i32,
        #[prost(string, tag = "2")]
        pub handle: String,
    }

    pub mod image_transport {
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration,
        )]
        #[repr(i32)]
        pub enum TransportChannel {
            Unspecified = 0,
            Mmap = 1,
        }
    }

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct ImageFormat {
        #[prost(enumeration = "image_format::ImgFormat", tag = "1")]
        pub format: i32,
        #[prost(uint32, tag = "3")]
        pub width: u32,
        #[prost(uint32, tag = "4")]
        pub height: u32,
        #[prost(uint32, tag = "5")]
        pub display: u32,
        #[prost(message, optional, tag = "6")]
        pub transport: Option<ImageTransport>,
    }

    pub mod image_format {
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration,
        )]
        #[repr(i32)]
        pub enum ImgFormat {
            Png = 0,
            Rgba8888 = 1,
            Rgb888 = 2,
        }
    }

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct Image {
        #[prost(message, optional, tag = "1")]
        pub format: Option<ImageFormat>,
        #[prost(uint32, tag = "2")]
        pub width: u32,
        #[prost(uint32, tag = "3")]
        pub height: u32,
        #[prost(bytes = "vec", tag = "4")]
        pub image: Vec<u8>,
        #[prost(uint32, tag = "5")]
        pub seq: u32,
        #[prost(uint64, tag = "6")]
        pub timestamp_us: u64,
    }
}
