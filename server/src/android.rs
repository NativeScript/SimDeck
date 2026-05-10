use crate::error::AppError;
use bytes::BytesMut;
use http::uri::PathAndQuery;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::env;
use std::ffi::OsString;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use tonic::metadata::MetadataValue;
use tonic::transport::{Channel, Endpoint};

const ANDROID_ID_PREFIX: &str = "android:";
const DEFAULT_GRPC_PORT_BASE: u16 = 8554;
const ANDROID_GRPC_FRAME_MESSAGE_LIMIT: usize = 64 * 1024 * 1024;
const ANDROID_TOUCH_IDENTIFIER: i32 = 1;
const RUNNING_EMULATOR_CACHE_TTL: Duration = Duration::from_secs(2);
const AVD_GRPC_PORT_CACHE_TTL: Duration = Duration::from_secs(60);
const SCREEN_SIZE_CACHE_TTL: Duration = Duration::from_secs(60);

type TimedMap<T> = Option<(Instant, HashMap<String, T>)>;
type ScreenSizeCache = HashMap<String, (Instant, (f64, f64))>;

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

    pub fn boot(&self, id: &str) -> Result<(), AppError> {
        let avd_name = avd_from_id(id)?;
        if self.resolve_serial(&avd_name).is_ok() {
            return Ok(());
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
        Ok(())
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
            "monkey",
            "-p",
            package,
            "-c",
            "android.intent.category.LAUNCHER",
            "1",
        ])?;
        Ok(())
    }

    pub fn set_pasteboard_text(&self, id: &str, text: &str) -> Result<(), AppError> {
        let serial = self.serial_for_id(id)?;
        self.run_adb_shell(&serial, &format!("cmd clipboard set {}", shell_quote(text)))?;
        Ok(())
    }

    pub fn pasteboard_text(&self, id: &str) -> Result<String, AppError> {
        let serial = self.serial_for_id(id)?;
        self.run_adb_shell(&serial, "cmd clipboard get")
    }

    pub fn send_touch(&self, id: &str, x: f64, y: f64, phase: &str) -> Result<(), AppError> {
        if self.send_touch_grpc(id, x, y, phase).is_ok() {
            return Ok(());
        }
        if phase != "ended" && phase != "cancelled" {
            return Ok(());
        }
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
        if self
            .send_swipe_grpc(id, start_x, start_y, end_x, end_y, duration_ms)
            .is_ok()
        {
            return Ok(());
        }
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

    pub fn send_key(&self, id: &str, key_code: u16, _modifiers: u32) -> Result<(), AppError> {
        if self
            .send_key_grpc(id, grpc::KeyboardEvent::usb_keypress(i32::from(key_code)))
            .is_ok()
        {
            return Ok(());
        }
        let serial = self.serial_for_id(id)?;
        let android_key = android_key_code(key_code);
        self.run_adb([
            "-s",
            &serial,
            "shell",
            "input",
            "keyevent",
            &android_key.to_string(),
        ])?;
        Ok(())
    }

    pub fn type_text(&self, id: &str, text: &str) -> Result<(), AppError> {
        if self
            .send_key_grpc(id, grpc::KeyboardEvent::text(text.to_owned()))
            .is_ok()
        {
            return Ok(());
        }
        let serial = self.serial_for_id(id)?;
        let escaped = text.replace('%', "%25").replace(' ', "%s");
        self.run_adb(["-s", &serial, "shell", "input", "text", &escaped])?;
        Ok(())
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
        let serial = self.serial_for_id(id)?;
        self.run_adb(["-s", &serial, "emu", "rotate"])?;
        Ok(())
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
        let (width, height) = self.screen_size_for_serial(&serial)?;
        Ok(json!({
            "totalWidth": width,
            "totalHeight": height,
            "screenX": 0,
            "screenY": 0,
            "screenWidth": width,
            "screenHeight": height,
            "cornerRadius": 0,
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
        if let (Some(max_edge), Ok(serial)) = (max_edge, self.resolve_serial(&avd_name)) {
            if let Ok((width, height)) = self.screen_size_for_serial(&serial) {
                let max_edge = max_edge.clamp(240, 2400) as f64;
                let largest = width.max(height);
                if largest > max_edge {
                    let scale = max_edge / largest;
                    format.width = (width * scale).round().max(1.0) as u32;
                    format.height = (height * scale).round().max(1.0) as u32;
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
        })
    }

    pub fn accessibility_tree(
        &self,
        id: &str,
        max_depth: Option<usize>,
    ) -> Result<Value, AppError> {
        let serial = self.serial_for_id(id)?;
        let raw = self.run_adb_shell(
            &serial,
            "uiautomator dump /sdcard/simdeck_ui.xml >/dev/null && cat /sdcard/simdeck_ui.xml",
        )?;
        let xml = extract_xml(&raw);
        let document = roxmltree::Document::parse(xml).map_err(|error| {
            AppError::native(format!("Unable to parse UIAutomator XML: {error}"))
        })?;
        let mut roots = Vec::new();
        let root = document.root_element();
        let max_depth = max_depth.unwrap_or(80).min(80);
        for child in root.children().filter(|node| node.has_tag_name("node")) {
            roots.push(android_node_value(child, 0, max_depth));
        }
        let (width, height) = self.screen_size_for_serial(&serial)?;
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

    fn send_touch_grpc(&self, id: &str, x: f64, y: f64, phase: &str) -> Result<(), AppError> {
        self.block_on_grpc(self.send_touch_grpc_async(id, x, y, phase))
    }

    async fn send_touch_grpc_async(
        &self,
        id: &str,
        x: f64,
        y: f64,
        phase: &str,
    ) -> Result<(), AppError> {
        let avd_name = avd_from_id(id)?;
        let serial = self.resolve_serial(&avd_name)?;
        let (width, height) = self.screen_size_for_serial(&serial)?;
        let pressure = match phase {
            "began" | "moved" => 1,
            "ended" | "cancelled" => 0,
            _ => return Ok(()),
        };
        let event = grpc::TouchEvent {
            touches: vec![grpc::Touch {
                x: normalized_to_pixel(x, width),
                y: normalized_to_pixel(y, height),
                identifier: ANDROID_TOUCH_IDENTIFIER,
                pressure,
                touch_major: 8,
                touch_minor: 8,
                expiration: grpc::touch::EventExpiration::NeverExpire as i32,
                orientation: 0,
            }],
            display: 0,
        };
        self.grpc_unary_for_avd::<grpc::TouchEvent, grpc::Empty>(
            &avd_name,
            "/android.emulation.control.EmulatorController/sendTouch",
            event,
        )
        .await?;
        Ok(())
    }

    fn send_swipe_grpc(
        &self,
        id: &str,
        start_x: f64,
        start_y: f64,
        end_x: f64,
        end_y: f64,
        duration_ms: u64,
    ) -> Result<(), AppError> {
        let duration_ms = duration_ms.clamp(50, 1500);
        let steps = (duration_ms / 8).clamp(4, 120);
        self.send_touch_grpc(id, start_x, start_y, "began")?;
        for step in 1..steps {
            let t = step as f64 / steps as f64;
            self.send_touch_grpc(
                id,
                start_x + (end_x - start_x) * t,
                start_y + (end_y - start_y) * t,
                "moved",
            )?;
            thread::sleep(Duration::from_millis((duration_ms / steps).max(1)));
        }
        self.send_touch_grpc(id, end_x, end_y, "ended")
    }

    fn send_key_grpc(&self, id: &str, event: grpc::KeyboardEvent) -> Result<(), AppError> {
        self.block_on_grpc(async {
            let avd_name = avd_from_id(id)?;
            self.grpc_unary_for_avd::<grpc::KeyboardEvent, grpc::Empty>(
                &avd_name,
                "/android.emulation.control.EmulatorController/sendKey",
                event,
            )
            .await?;
            Ok(())
        })
    }

    fn block_on_grpc<F, T>(&self, future: F) -> Result<T, AppError>
    where
        F: Future<Output = Result<T, AppError>>,
    {
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            return handle.block_on(future);
        }
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| AppError::internal(format!("Unable to create gRPC runtime: {error}")))?
            .block_on(future)
    }

    async fn grpc_unary_for_avd<Req, Resp>(
        &self,
        avd_name: &str,
        path: &'static str,
        request: Req,
    ) -> Result<Resp, AppError>
    where
        Req: prost::Message + Default + Send + 'static,
        Resp: prost::Message + Default + Send + 'static,
    {
        let port = self.grpc_port_for_avd(avd_name)?;
        let channel = grpc_channel_for_port(port)?;
        let mut grpc = tonic::client::Grpc::new(channel);
        grpc.ready().await.map_err(|error| {
            AppError::native(format!("Android emulator gRPC is not ready: {error}"))
        })?;
        let mut request = tonic::Request::new(request);
        if let Some(token) = emulator_grpc_token(port) {
            let value = MetadataValue::try_from(format!("Bearer {token}")).map_err(|error| {
                AppError::native(format!("Invalid Android emulator gRPC token: {error}"))
            })?;
            request.metadata_mut().insert("authorization", value);
        }
        let response = grpc
            .unary(
                request,
                PathAndQuery::from_static(path),
                tonic::codec::ProstCodec::default(),
            )
            .await
            .map_err(|error| {
                AppError::native(format!("Android emulator gRPC input failed: {error}"))
            })?;
        Ok(response.into_inner())
    }

    fn device_value(&self, device: AndroidDevice) -> Value {
        let id = id_for_avd(&device.avd_name);
        let private_display = if let Some(serial) = device.serial.as_deref() {
            let (width, height) = self.screen_size_for_serial(serial).unwrap_or((0.0, 0.0));
            json!({
                "displayReady": width > 0.0 && height > 0.0,
                "displayStatus": "Ready",
                "displayWidth": width,
                "displayHeight": height,
                "frameSequence": 0,
                "rotationQuarterTurns": 0,
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
            if let Ok(name_output) = self.run_adb(["-s", serial, "emu", "avd", "name"]) {
                if let Some(name) = name_output
                    .lines()
                    .map(str::trim)
                    .find(|line| !line.is_empty() && *line != "OK")
                {
                    result.insert(name.to_owned(), serial.to_owned());
                }
            }
        }
        *cache.lock().unwrap() = Some((Instant::now(), result.clone()));
        Ok(result)
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
        static CACHE: OnceLock<Mutex<ScreenSizeCache>> = OnceLock::new();
        let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
        if let Some((updated_at, size)) = cache.lock().unwrap().get(serial) {
            if updated_at.elapsed() < SCREEN_SIZE_CACHE_TTL {
                return Ok(*size);
            }
        }
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
        cache
            .lock()
            .unwrap()
            .insert(serial.to_owned(), (Instant::now(), (width, height)));
        Ok((width, height))
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
        Ok(Some(AndroidFrame {
            width,
            height,
            timestamp_us: image.timestamp_us,
            rgba,
        }))
    }
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
    let output = Command::new(&program)
        .args(args)
        .env("ANDROID_HOME", sdk_root())
        .env("ANDROID_SDK_ROOT", sdk_root())
        .env("JAVA_HOME", java_home())
        .output()
        .map_err(|error| {
            AppError::native(format!("Unable to run {}: {error}", program.display()))
        })?;
    if output.status.success() {
        return Ok(output.stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    Err(AppError::native(format!(
        "{} failed: {}{}",
        program
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Android command"),
        stderr.trim(),
        if stdout.trim().is_empty() {
            String::new()
        } else {
            format!(" {}", stdout.trim())
        }
    )))
}

fn grpc_channel_for_port(port: u16) -> Result<Channel, AppError> {
    static CHANNELS: OnceLock<Mutex<HashMap<u16, Channel>>> = OnceLock::new();
    let channels = CHANNELS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut channels = channels.lock().unwrap();
    if let Some(channel) = channels.get(&port) {
        return Ok(channel.clone());
    }
    let endpoint = Endpoint::from_shared(format!("http://127.0.0.1:{port}"))
        .map_err(|error| AppError::native(format!("Invalid Android gRPC endpoint: {error}")))?;
    let channel = endpoint.connect_lazy();
    channels.insert(port, channel.clone());
    Ok(channel)
}

fn normalized_to_pixel(value: f64, extent: f64) -> i32 {
    (value.clamp(0.0, 1.0) * (extent - 1.0).max(0.0))
        .round()
        .max(0.0) as i32
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
        40 => 66,
        41 => 111,
        42 => 67,
        43 => 61,
        44 => 62,
        79 => 22,
        80 => 21,
        81 => 20,
        82 => 19,
        _ => hid,
    }
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
}

#[allow(dead_code)]
fn _dedupe(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

mod grpc {
    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct Empty {}

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct Touch {
        #[prost(int32, tag = "1")]
        pub x: i32,
        #[prost(int32, tag = "2")]
        pub y: i32,
        #[prost(int32, tag = "3")]
        pub identifier: i32,
        #[prost(int32, tag = "4")]
        pub pressure: i32,
        #[prost(int32, tag = "5")]
        pub touch_major: i32,
        #[prost(int32, tag = "6")]
        pub touch_minor: i32,
        #[prost(enumeration = "touch::EventExpiration", tag = "7")]
        pub expiration: i32,
        #[prost(int32, tag = "8")]
        pub orientation: i32,
    }

    pub mod touch {
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration,
        )]
        #[repr(i32)]
        pub enum EventExpiration {
            Unspecified = 0,
            NeverExpire = 1,
        }
    }

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct TouchEvent {
        #[prost(message, repeated, tag = "1")]
        pub touches: Vec<Touch>,
        #[prost(int32, tag = "2")]
        pub display: i32,
    }

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct KeyboardEvent {
        #[prost(enumeration = "keyboard_event::KeyCodeType", tag = "1")]
        pub code_type: i32,
        #[prost(enumeration = "keyboard_event::KeyEventType", tag = "2")]
        pub event_type: i32,
        #[prost(int32, tag = "3")]
        pub key_code: i32,
        #[prost(string, tag = "4")]
        pub key: String,
        #[prost(string, tag = "5")]
        pub text: String,
    }

    impl KeyboardEvent {
        pub fn usb_keypress(key_code: i32) -> Self {
            Self {
                code_type: keyboard_event::KeyCodeType::Usb as i32,
                event_type: keyboard_event::KeyEventType::Keypress as i32,
                key_code,
                key: String::new(),
                text: String::new(),
            }
        }

        pub fn text(text: String) -> Self {
            Self {
                code_type: keyboard_event::KeyCodeType::Usb as i32,
                event_type: keyboard_event::KeyEventType::Keypress as i32,
                key_code: 0,
                key: String::new(),
                text,
            }
        }
    }

    pub mod keyboard_event {
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration,
        )]
        #[repr(i32)]
        pub enum KeyCodeType {
            Usb = 0,
            Evdev = 1,
            Xkb = 2,
            Win = 3,
            Mac = 4,
        }

        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration,
        )]
        #[repr(i32)]
        pub enum KeyEventType {
            Keydown = 0,
            Keyup = 1,
            Keypress = 2,
        }
    }

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
