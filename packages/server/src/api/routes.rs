use crate::accessibility::{interactive_accessibility_snapshot, AccessibilitySource};
use crate::android::{self, AndroidBridge, AndroidEmulatorSpec};
use crate::api::json::json;
use crate::auth;
use crate::camera::{self, CameraStartRequest, CameraSwitchRequest};
use crate::config::Config;
use crate::devtools;
use crate::error::AppError;
use crate::inspector::{InspectorHub, PublishedInspector};
use crate::logs::LogRegistry;
use crate::metrics::counters::{ClientStreamStats, Metrics};
use crate::native::bridge::{
    tvos_remote_key_for_touch_motion, tvos_remote_key_for_touch_phase, LogFilters, NativeBridge,
    NativeInputSession, NativePairedWatchSpec, HID_KEY_ENTER,
};
use crate::performance::{
    sample_stack, DisplaySignal, ForegroundProcess, PerformanceQuery, PerformanceRegistry,
};
use crate::simulators::registry::SessionRegistry;
use crate::simulators::session::SimulatorSession;
use crate::static_files;
use crate::webkit;
use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, DefaultBodyLimit, Path, Query, State};
use axum::http::{header, HeaderMap, Method, Request, StatusCode, Uri};
use axum::middleware::{from_fn_with_state, Next};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{any, get, post};
use axum::{Json, Router};
use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use regex::Regex;
use serde::Deserialize;
use serde_json::Map;
use serde_json::{json as json_value, Value};
use std::collections::{HashMap, VecDeque};
use std::env;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::process::Command;
use tokio::sync::{mpsc, Mutex};
use tokio::task;
use tokio::time::timeout;
use tower_http::trace::{DefaultMakeSpan, DefaultOnFailure, TraceLayer};
use tracing::Level;

const SIMULATOR_INVENTORY_CACHE_TTL: Duration = Duration::from_secs(5);
const SIMULATOR_INVENTORY_TIMEOUT: Duration = Duration::from_secs(8);
const SIMULATOR_INVENTORY_FORCE_REFRESH_TIMEOUT: Duration = Duration::from_secs(90);
const STREAM_CLIENT_FOREGROUND_TTL: Duration = Duration::from_secs(30);
const CHROME_DEVTOOLS_DISCOVERY_TIMEOUT: Duration = Duration::from_millis(900);
const MULTITOUCH_INPUT_IDLE_TIMEOUT: Duration = Duration::from_secs(5);
const FOREGROUND_APP_CACHE_TTL: Duration = Duration::from_secs(3);
const INSPECTOR_FOREGROUND_APP_CACHE_TTL: Duration = Duration::from_millis(500);
const FOREGROUND_APP_STALE_TTL: Duration = Duration::from_secs(30);
const FOREGROUND_APP_ROUTE_TIMEOUT: Duration = Duration::from_millis(1200);
const APP_UPLOAD_FILE_NAME_HEADER: &str = "x-simdeck-filename";
const MAX_APP_UPLOAD_BYTES: usize = 1024 * 1024 * 1024;
const FOREGROUND_PROCESS_PROBE_TIMEOUT: Duration = Duration::from_millis(750);
const ACCESSIBILITY_SOURCE_DISCOVERY_TIMEOUT: Duration = Duration::from_millis(250);
const ACCESSIBILITY_TREE_CACHE_TTL: Duration = Duration::from_secs(5);
const NATIVE_AX_SNAPSHOT_RETRY_ATTEMPTS: usize = 5;
const NATIVE_AX_SNAPSHOT_RETRY_DELAY: Duration = Duration::from_millis(100);
const NATIVE_AX_SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(8);

static FOREGROUND_APP_CACHE: OnceLock<StdMutex<HashMap<String, CachedForegroundApp>>> =
    OnceLock::new();

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub registry: SessionRegistry,
    pub logs: LogRegistry,
    pub inspectors: InspectorHub,
    pub metrics: Arc<Metrics>,
    pub performance: PerformanceRegistry,
    pub stream_clients: StreamClientForegroundRegistry,
    pub simulator_inventory: SimulatorInventoryCache,
    pub accessibility_cache: AccessibilitySnapshotCache,
    pub android: AndroidBridge,
}

#[derive(Clone)]
struct CachedForegroundApp {
    cached_at: Instant,
    foreground_app: devtools::ForegroundApp,
}

#[derive(Clone, Default)]
pub struct AccessibilitySnapshotCache {
    inner: Arc<StdMutex<HashMap<AccessibilitySnapshotCacheKey, CachedAccessibilitySnapshot>>>,
    generations: Arc<StdMutex<HashMap<String, u64>>>,
    warming: Arc<StdMutex<HashMap<String, u64>>>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct AccessibilitySnapshotCacheKey {
    udid: String,
    source: String,
    max_depth: Option<usize>,
    include_hidden: bool,
    interactive_only: bool,
}

#[derive(Clone)]
struct CachedAccessibilitySnapshot {
    cached_at: Instant,
    snapshot: Value,
}

impl AccessibilitySnapshotCache {
    fn get_compatible(
        &self,
        key: &AccessibilitySnapshotCacheKey,
    ) -> Option<(AccessibilitySnapshotCacheKey, Value)> {
        let mut cache = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let now = Instant::now();
        cache.retain(|_, cached| {
            now.duration_since(cached.cached_at) <= ACCESSIBILITY_TREE_CACHE_TTL
        });
        cache
            .iter()
            .filter(|(cached_key, _)| {
                cached_key.udid == key.udid
                    && cached_key.source == key.source
                    && cached_key.include_hidden == key.include_hidden
                    && cached_key.interactive_only == key.interactive_only
                    && cached_depth_covers(cached_key.max_depth, key.max_depth)
            })
            .min_by_key(|(cached_key, _)| cached_depth_rank(cached_key.max_depth))
            .map(|(cached_key, cached)| (cached_key.clone(), cached.snapshot.clone()))
    }

    #[cfg(test)]
    fn insert(&self, key: AccessibilitySnapshotCacheKey, snapshot: &Value) {
        let generation = self.generation(&key.udid);
        self.insert_if_generation(key, snapshot, generation);
    }

    fn insert_if_generation(
        &self,
        key: AccessibilitySnapshotCacheKey,
        snapshot: &Value,
        generation: u64,
    ) {
        if !cacheable_accessibility_snapshot(snapshot) {
            return;
        }
        let generations = self
            .generations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if generations.get(&key.udid).copied().unwrap_or(0) != generation {
            return;
        }
        let mut cache = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cache.insert(
            key,
            CachedAccessibilitySnapshot {
                cached_at: Instant::now(),
                snapshot: snapshot.clone(),
            },
        );
    }

    fn latest_interactive(&self, udid: &str) -> Option<Value> {
        let mut cache = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let now = Instant::now();
        cache.retain(|_, cached| {
            now.duration_since(cached.cached_at) <= ACCESSIBILITY_TREE_CACHE_TTL
        });
        cache
            .iter()
            .filter(|(key, _)| key.udid == udid && key.interactive_only && !key.include_hidden)
            .max_by_key(|(_, cached)| cached.cached_at)
            .map(|(_, cached)| cached.snapshot.clone())
    }

    fn invalidate(&self, udid: &str) {
        let mut generations = self
            .generations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let generation = generations.entry(udid.to_owned()).or_insert(0);
        *generation = generation.saturating_add(1);
        let mut cache = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cache.retain(|key, _| key.udid != udid);
    }

    fn generation(&self, udid: &str) -> u64 {
        self.generations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(udid)
            .copied()
            .unwrap_or(0)
    }

    fn begin_warming(&self, udid: &str, generation: u64) -> bool {
        let mut warming = self
            .warming
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match warming.get(udid).copied() {
            Some(active_generation) if active_generation >= generation => false,
            _ => {
                warming.insert(udid.to_owned(), generation);
                true
            }
        }
    }

    fn finish_warming(&self, udid: &str, generation: u64) {
        let mut warming = self
            .warming
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if warming.get(udid).copied() == Some(generation) {
            warming.remove(udid);
        }
    }
}

fn cacheable_accessibility_snapshot(snapshot: &Value) -> bool {
    snapshot.get("fallbackReason").is_none()
}

fn cached_depth_covers(cached: Option<usize>, requested: Option<usize>) -> bool {
    match (cached, requested) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(cached), Some(requested)) => cached >= requested,
    }
}

fn cached_depth_rank(depth: Option<usize>) -> usize {
    depth.unwrap_or(usize::MAX)
}

#[derive(Clone, Default)]
pub struct StreamClientForegroundRegistry {
    inner: Arc<StdMutex<HashMap<(String, String), StreamClientForegroundState>>>,
}

#[derive(Clone, Copy)]
struct StreamClientForegroundState {
    foreground: bool,
    updated_at: Instant,
}

impl StreamClientForegroundRegistry {
    pub fn record(&self, udid: &str, client_id: &str, foreground: bool) -> (bool, bool) {
        let now = Instant::now();
        let mut clients = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        clients.retain(|_, state| {
            now.duration_since(state.updated_at) <= STREAM_CLIENT_FOREGROUND_TTL
        });
        let previous = any_foreground_client_for_udid(&clients, udid);
        clients.insert(
            (udid.to_owned(), client_id.to_owned()),
            StreamClientForegroundState {
                foreground,
                updated_at: now,
            },
        );
        let next = any_foreground_client_for_udid(&clients, udid).unwrap_or(true);
        (next, previous != Some(next))
    }

    pub fn remove(&self, udid: &str, client_id: &str) -> (bool, bool) {
        let now = Instant::now();
        let mut clients = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        clients.retain(|_, state| {
            now.duration_since(state.updated_at) <= STREAM_CLIENT_FOREGROUND_TTL
        });
        let previous = any_foreground_client_for_udid(&clients, udid);
        clients.remove(&(udid.to_owned(), client_id.to_owned()));
        let next = any_foreground_client_for_udid(&clients, udid).unwrap_or(false);
        (next, previous != Some(next))
    }
}

fn any_foreground_client_for_udid(
    clients: &HashMap<(String, String), StreamClientForegroundState>,
    udid: &str,
) -> Option<bool> {
    let mut saw_client = false;
    let mut saw_foreground = false;
    for ((client_udid, _), state) in clients {
        if client_udid == udid {
            saw_client = true;
            saw_foreground |= state.foreground;
        }
    }
    saw_client.then_some(saw_foreground)
}

#[derive(Clone, Default)]
pub struct SimulatorInventoryCache {
    inner: Arc<Mutex<SimulatorInventoryState>>,
}

#[derive(Default)]
struct SimulatorInventoryState {
    simulators: Option<Vec<crate::native::bridge::Simulator>>,
    updated_at: Option<Instant>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StreamQualityPayload {
    pub(crate) profile: Option<String>,
    #[serde(rename = "videoCodec")]
    pub(crate) video_codec: Option<String>,
    pub(crate) max_edge: Option<u32>,
    pub(crate) fps: Option<u32>,
    pub(crate) min_bitrate: Option<u32>,
    pub(crate) bits_per_pixel: Option<u32>,
}

impl StreamQualityPayload {
    fn has_any_value(&self) -> bool {
        self.profile
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
            || self
                .video_codec
                .as_deref()
                .map(str::trim)
                .is_some_and(|value| !value.is_empty())
            || self.max_edge.is_some()
            || self.fps.is_some()
            || self.min_bitrate.is_some()
            || self.bits_per_pixel.is_some()
    }
}

#[derive(Clone, Copy)]
struct StreamQualityProfile {
    id: &'static str,
    label: &'static str,
    max_edge: u32,
    fps: u32,
    min_bitrate: u32,
    bits_per_pixel: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ActiveStreamQualityState {
    profile: String,
    max_edge: u32,
    fps: u32,
    min_bitrate: u32,
    bits_per_pixel: u32,
    video_codec: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct StreamQualityLimits {
    pub(crate) max_edge: u32,
    pub(crate) fps: u32,
    pub(crate) min_bitrate: u32,
    pub(crate) bits_per_pixel: u32,
}

const STREAM_QUALITY_PROFILES: &[StreamQualityProfile] = &[
    StreamQualityProfile {
        id: "ci-software",
        label: "CI Software",
        max_edge: 960,
        fps: 24,
        min_bitrate: 1_200_000,
        bits_per_pixel: 2,
    },
    StreamQualityProfile {
        id: "quality",
        label: "Quality",
        max_edge: 4096,
        fps: 60,
        min_bitrate: 60_000_000,
        bits_per_pixel: 10,
    },
    StreamQualityProfile {
        id: "full",
        label: "Full",
        max_edge: 4096,
        fps: 60,
        min_bitrate: 12_000_000,
        bits_per_pixel: 4,
    },
    StreamQualityProfile {
        id: "balanced",
        label: "Balanced",
        max_edge: 1280,
        fps: 60,
        min_bitrate: 6_000_000,
        bits_per_pixel: 5,
    },
    StreamQualityProfile {
        id: "fast",
        label: "Fast",
        max_edge: 960,
        fps: 30,
        min_bitrate: 2_500_000,
        bits_per_pixel: 3,
    },
    StreamQualityProfile {
        id: "smooth",
        label: "Smooth",
        max_edge: 4096,
        fps: 60,
        min_bitrate: 4_000_000,
        bits_per_pixel: 5,
    },
    StreamQualityProfile {
        id: "economy",
        label: "Economy",
        max_edge: 1080,
        fps: 30,
        min_bitrate: 3_500_000,
        bits_per_pixel: 6,
    },
    StreamQualityProfile {
        id: "low",
        label: "Low",
        max_edge: 720,
        fps: 30,
        min_bitrate: 2_000_000,
        bits_per_pixel: 5,
    },
    StreamQualityProfile {
        id: "tiny",
        label: "Tiny",
        max_edge: 540,
        fps: 30,
        min_bitrate: 1_200_000,
        bits_per_pixel: 4,
    },
];

const VISIBLE_STREAM_QUALITY_PROFILE_IDS: &[&str] =
    &["full", "balanced", "smooth", "economy", "low", "tiny"];

static STREAM_CONFIG_LOCK: OnceLock<StdMutex<()>> = OnceLock::new();

#[derive(Deserialize)]
struct InstallPayload {
    #[serde(rename = "appPath")]
    app_path: String,
}

#[derive(Deserialize)]
struct UninstallPayload {
    #[serde(rename = "bundleId")]
    bundle_id: String,
}

#[derive(Deserialize)]
struct PasteboardPayload {
    text: String,
}

#[derive(Deserialize)]
struct ScreenshotPngQuery {
    bezel: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScreenRecordingPayload {
    seconds: Option<f64>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateSimulatorPayload {
    platform: Option<String>,
    name: String,
    device_type_identifier: String,
    runtime_identifier: Option<String>,
    paired_watch: Option<CreatePairedWatchPayload>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreatePairedWatchPayload {
    name: String,
    device_type_identifier: String,
    runtime_identifier: Option<String>,
}

#[derive(Deserialize, Clone)]
struct TouchSequenceEvent {
    x: f64,
    y: f64,
    phase: String,
    #[serde(rename = "delayMsAfter")]
    delay_ms_after: Option<u64>,
}

#[derive(Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum ControlMessage {
    Touch {
        x: f64,
        y: f64,
        phase: String,
    },
    EdgeTouch {
        x: f64,
        y: f64,
        phase: String,
        edge: String,
    },
    MultiTouch {
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        phase: String,
    },
    Key {
        key_code: u16,
        modifiers: Option<u32>,
    },
    Button {
        button: String,
        duration_ms: Option<u32>,
        phase: Option<String>,
        usage_page: Option<u32>,
        usage: Option<u32>,
    },
    Crown {
        delta: f64,
    },
    Scroll {
        delta_x: f64,
        delta_y: f64,
        x: Option<f64>,
        y: Option<f64>,
    },
    DismissKeyboard,
    ToggleSoftwareKeyboard,
    Home,
    AppSwitcher,
    RotateLeft,
    RotateRight,
    ToggleAppearance,
}

#[derive(Default)]
pub(crate) struct TvosControlTouchGesture {
    start: Option<(f64, f64)>,
    last: Option<(f64, f64)>,
}

impl TvosControlTouchGesture {
    fn update(&mut self, x: f64, y: f64, phase: &str) -> Result<Option<u16>, AppError> {
        let point = (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0));
        match phase.trim().to_ascii_lowercase().as_str() {
            "began" | "down" => {
                self.start = Some(point);
                self.last = Some(point);
                Ok(None)
            }
            "moved" => {
                if self.start.is_none() {
                    self.start = Some(point);
                }
                self.last = Some(point);
                Ok(None)
            }
            "ended" | "cancelled" | "up" => {
                let start = self.start.take().unwrap_or(point);
                let end = self.last.take().unwrap_or(point);
                Ok(Some(tvos_remote_key_for_touch_motion(
                    start.0, start.1, end.0, end.1,
                )))
            }
            _ => Err(AppError::bad_request(
                "`phase` must be `began`, `moved`, `ended`, `cancelled`, `down`, or `up`.",
            )),
        }
    }
}

fn tvos_touch_sequence_key(events: &[TouchSequenceEvent]) -> Result<u16, AppError> {
    let first = events
        .first()
        .ok_or_else(|| AppError::bad_request("Touch sequence requires events."))?;
    let mut start = (first.x.clamp(0.0, 1.0), first.y.clamp(0.0, 1.0));
    let mut end = start;

    for event in events {
        let point = (event.x.clamp(0.0, 1.0), event.y.clamp(0.0, 1.0));
        match event.phase.trim().to_ascii_lowercase().as_str() {
            "began" | "down" => {
                start = point;
                end = point;
            }
            "moved" => {
                end = point;
            }
            "ended" | "cancelled" | "up" => {
                end = point;
                return Ok(tvos_remote_key_for_touch_motion(
                    start.0, start.1, end.0, end.1,
                ));
            }
            _ => {
                return Err(AppError::bad_request(
                    "`phase` must be `began`, `moved`, `ended`, `cancelled`, `down`, or `up`.",
                ))
            }
        }
    }

    Ok(tvos_remote_key_for_touch_motion(
        start.0, start.1, end.0, end.1,
    ))
}

fn bridge_simulator_is_tvos(bridge: &NativeBridge, udid: &str) -> bool {
    bridge.simulator_is_tvos(udid).unwrap_or(false)
}

fn press_tvos_remote_key(bridge: &NativeBridge, udid: &str, key_code: u16) -> Result<(), AppError> {
    bridge.send_key(udid, key_code, 0)
}

fn handle_tvos_touch_phase(bridge: &NativeBridge, udid: &str, phase: &str) -> Result<(), AppError> {
    if let Some(key_code) = tvos_remote_key_for_touch_phase(phase)? {
        press_tvos_remote_key(bridge, udid, key_code)?;
    }
    Ok(())
}

#[derive(Deserialize)]
struct ChromePngQuery {
    buttons: Option<String>,
}

#[derive(Deserialize)]
struct ChromeButtonPngQuery {
    pressed: Option<String>,
}

include!("action_types.rs");

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccessibilityPointQuery {
    x: f64,
    y: f64,
    max_depth: Option<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccessibilityTreeQuery {
    source: Option<String>,
    max_depth: Option<usize>,
    include_hidden: Option<bool>,
    interactive_only: Option<bool>,
}

#[derive(Deserialize)]
struct LogsQuery {
    backfill: Option<bool>,
    seconds: Option<f64>,
    limit: Option<usize>,
    levels: Option<String>,
    processes: Option<String>,
    q: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PerformanceRequestQuery {
    pid: Option<i32>,
    window_ms: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StackSampleRequestQuery {
    seconds: Option<u64>,
}

#[derive(Deserialize)]
struct InspectorRequestPayload {
    method: String,
    params: Option<Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InspectorDirectRequestPayload {
    process_identifier: i64,
    method: String,
    params: Option<Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InspectorPollQuery {
    #[serde(alias = "pid")]
    process_identifier: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InspectorResponsePayload {
    process_identifier: i64,
    id: u64,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<Value>,
}

const INSPECTOR_AGENT_HOST: &str = "127.0.0.1";
const INSPECTOR_AGENT_DEFAULT_PORT: u16 = 47370;
const INSPECTOR_AGENT_PORT_SCAN_LIMIT: u16 = 32;
const INSPECTOR_AGENT_TIMEOUT: Duration = Duration::from_millis(900);
const CONNECTED_INSPECTOR_HIERARCHY_TIMEOUT: Duration = Duration::from_secs(8);
const SOURCE_NATIVE_AX: &str = "native-ax";
const SOURCE_NATIVE_SCRIPT: &str = "nativescript";
const SOURCE_REACT_NATIVE: &str = "react-native";
const SOURCE_FLUTTER: &str = "flutter";
const SOURCE_SWIFTUI: &str = "swiftui";
const SOURCE_UIKIT: &str = "in-app-inspector";

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/pair", post(pair_browser))
        .route("/api/health", get(health))
        .route("/api/metrics", get(metrics))
        .route(
            "/api/stream-quality",
            get(stream_quality).post(set_stream_quality),
        )
        .route(
            "/api/client-stream-stats",
            get(client_stream_stats).post(record_client_stream_stats),
        )
        .route("/api/inspector/connect", get(native_inspector_connect))
        .route("/api/inspector/poll", get(inspector_poll))
        .route("/api/inspector/request", post(inspector_direct_request))
        .route("/api/inspector/response", post(inspector_response))
        .route("/chrome-devtools-ui", get(chrome_devtools_ui_redirect))
        .route("/chrome-devtools-ui/{*path}", get(chrome_devtools_ui_file))
        .route("/api/metro/{port}", any(metro_proxy_root))
        .route("/api/metro/{port}/{*path}", any(metro_proxy_asset))
        .route("/api/metro-frontend/{port}/{*path}", any(metro_proxy_asset))
        .route("/webkit-inspector-ui", get(webkit_inspector_ui_redirect))
        .route(
            "/webkit-inspector-ui/{*path}",
            get(webkit_inspector_ui_file),
        )
        .route(
            "/api/simulators",
            get(list_simulators).post(create_simulator),
        )
        .route(
            "/api/simulators/create-options",
            get(simulator_create_options),
        )
        .route("/api/camera/webcams", get(camera_webcams))
        .route("/api/simulators/{udid}/state", get(simulator_state))
        .route("/api/simulators/{udid}/processes", get(simulator_processes))
        .route(
            "/api/simulators/{udid}/performance",
            get(simulator_performance),
        )
        .route(
            "/api/simulators/{udid}/processes/{pid}/performance",
            get(simulator_process_performance),
        )
        .route(
            "/api/simulators/{udid}/processes/{pid}/sample",
            post(sample_process_stack),
        )
        .route("/api/simulators/{udid}/boot", post(boot_simulator))
        .route("/api/simulators/{udid}/shutdown", post(shutdown_simulator))
        .route("/api/simulators/{udid}/erase", post(erase_simulator))
        .route("/api/simulators/{udid}/install", post(install_app))
        .route(
            "/api/simulators/{udid}/install-upload",
            post(upload_install_app).layer(DefaultBodyLimit::max(MAX_APP_UPLOAD_BYTES)),
        )
        .route("/api/simulators/{udid}/uninstall", post(uninstall_app))
        .route(
            "/api/simulators/{udid}/pasteboard",
            get(get_pasteboard).post(set_pasteboard),
        )
        .route("/api/simulators/{udid}/screenshot.png", get(screenshot_png))
        .route(
            "/api/simulators/{udid}/screen-recording",
            post(screen_recording),
        )
        .route(
            "/api/simulators/{udid}/screen-recording/start",
            post(start_screen_recording),
        )
        .route(
            "/api/simulators/{udid}/screen-recording/{recording_id}/stop",
            post(stop_screen_recording),
        )
        .route("/api/simulators/{udid}/refresh", post(refresh_stream))
        .route(
            "/api/simulators/{udid}/camera",
            get(camera_status).post(start_camera).delete(stop_camera),
        )
        .route(
            "/api/simulators/{udid}/camera/source",
            post(switch_camera_source),
        )
        .route("/api/simulators/{udid}/action", post(simulator_action))
        .route("/api/simulators/{udid}/control", get(control_socket))
        .route("/api/simulators/{udid}/input", get(control_socket))
        .route("/api/simulators/{udid}/h264", get(removed_h264_stream))
        .route("/api/simulators/{udid}/webrtc/offer", post(webrtc_offer))
        .route("/api/simulators/{udid}/chrome-profile", get(chrome_profile))
        .route("/api/simulators/{udid}/chrome.png", get(chrome_png))
        .route(
            "/api/simulators/{udid}/chrome-button/{button}",
            get(chrome_button_png),
        )
        .route(
            "/api/simulators/{udid}/screen-mask.png",
            get(screen_mask_png),
        )
        .route(
            "/api/simulators/{udid}/accessibility-tree",
            get(accessibility_tree),
        )
        .route(
            "/api/simulators/{udid}/accessibility-point",
            get(accessibility_point),
        )
        .route(
            "/api/simulators/{udid}/inspector/request",
            post(inspector_request),
        )
        .route("/api/simulators/{udid}/webkit/targets", get(webkit_targets))
        .route(
            "/api/simulators/{udid}/webkit/targets/{target_id}/socket",
            get(webkit_target_socket),
        )
        .route(
            "/api/simulators/{udid}/devtools/targets",
            get(chrome_devtools_targets),
        )
        .route(
            "/api/simulators/{udid}/devtools/targets/{target_id}/socket",
            get(chrome_devtools_target_socket),
        )
        .route("/api/simulators/{udid}/logs", get(simulator_logs))
        .route_layer(from_fn_with_state(state.clone(), require_api_auth))
        .with_state(state)
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().level(Level::WARN))
                .on_failure(DefaultOnFailure::new().level(Level::WARN)),
        )
}

async fn require_api_auth(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if request.method() == Method::OPTIONS {
        return auth::preflight_response(&state.config, request.headers());
    }

    if is_inspector_agent_transport_path(request.uri().path())
        || request.uri().path() == "/api/pair"
    {
        return next.run(request).await;
    }

    let peer_is_loopback = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(address)| address.ip().is_loopback())
        .unwrap_or(false);

    if !auth::api_request_authorized(
        &state.config,
        request.method(),
        request.headers(),
        peer_is_loopback,
        request.uri().query(),
    ) {
        return auth::unauthorized_response(&state.config, request.headers());
    }

    let request_headers = request.headers().clone();
    let mut response = next.run(request).await;
    auth::append_cors_headers(&state.config, &request_headers, response.headers_mut());
    if peer_is_loopback {
        auth::append_access_cookie(response.headers_mut(), &state.config.access_token);
    }
    response
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PairBrowserPayload {
    code: String,
}

async fn pair_browser(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<PairBrowserPayload>,
) -> Response {
    if !auth::pairing_code_matches(&state.config, &payload.code) {
        return auth::unauthorized_response(&state.config, &headers);
    }
    let mut response = Json(json_value!({
        "ok": true,
        "accessToken": state.config.access_token,
    }))
    .into_response();
    auth::append_cors_headers(&state.config, &headers, response.headers_mut());
    auth::append_access_cookie(response.headers_mut(), &state.config.access_token);
    response
}

fn is_inspector_agent_transport_path(path: &str) -> bool {
    matches!(
        path,
        "/api/inspector/connect" | "/api/inspector/poll" | "/api/inspector/response"
    )
}

async fn health(State(state): State<AppState>) -> Json<Value> {
    let video_codec = active_video_codec(&state.config);
    let stream_quality =
        stream_quality_state_value(&current_stream_quality_state(video_codec.clone()));
    json(json_value!({
        "ok": true,
        "serverId": crate::auth::server_identity(&state.config),
        "advertiseHost": state.config.advertise_host,
        "hostId": state.config.host_id,
        "hostName": state.config.host_name,
        "httpPort": state.config.http_port,
        "serverKind": state.config.server_kind.as_str(),
        "timestamp": SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or(Duration::ZERO).as_secs_f64(),
        "videoCodec": video_codec,
        "androidGpu": active_android_gpu(),
        "lowLatency": state.config.low_latency,
        "realtimeStream": crate::transport::webrtc::realtime_stream_enabled(),
        "localStreamFps": env_u32("SIMDECK_LOCAL_STREAM_FPS", 60, 15, 240),
        "streamQuality": stream_quality,
        "webRtc": {
            "iceServers": crate::transport::webrtc::client_ice_servers(),
            "iceTransportPolicy": crate::transport::webrtc::ice_transport_policy_label()
        }
    }))
}

fn active_video_codec(config: &Config) -> String {
    std::env::var("SIMDECK_VIDEO_CODEC")
        .ok()
        .and_then(|value| normalize_video_codec(&value).map(ToOwned::to_owned))
        .unwrap_or_else(|| config.video_codec.clone())
}

fn active_android_gpu() -> String {
    std::env::var("SIMDECK_ANDROID_GPU")
        .ok()
        .and_then(|value| normalize_android_gpu(&value).map(ToOwned::to_owned))
        .unwrap_or_else(|| "host".to_owned())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as u64
}

fn normalize_video_codec(codec: &str) -> Option<&'static str> {
    match codec.trim().to_ascii_lowercase().as_str() {
        "auto" => Some("auto"),
        "hardware" => Some("hardware"),
        "software" => Some("software"),
        _ => None,
    }
}

fn normalize_android_gpu(mode: &str) -> Option<&'static str> {
    match mode.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "auto" => Some("auto"),
        "host" => Some("host"),
        "software" => Some("software"),
        "lavapipe" => Some("lavapipe"),
        "swiftshader" => Some("swiftshader"),
        "swangle" => Some("swangle"),
        "swiftshader_indirect" => Some("swiftshader_indirect"),
        _ => None,
    }
}

async fn metrics(State(state): State<AppState>) -> Json<Value> {
    let mut snapshot = json_value!(state.metrics.snapshot());
    if let Some(object) = snapshot.as_object_mut() {
        object.insert(
            "encoders".to_owned(),
            json_value!(state.registry.encoder_snapshots()),
        );
    }
    json(snapshot)
}

async fn webkit_targets(
    Path(udid): Path<String>,
    headers: HeaderMap,
) -> Result<Json<webkit::WebKitTargetDiscovery>, AppError> {
    let origin = request_origin(&headers);
    let discovery = webkit::discover_targets(&udid, origin.as_deref()).await?;
    Ok(Json(discovery))
}

async fn webkit_target_socket(
    Path((udid, target_id)): Path<(String, String)>,
    ws: WebSocketUpgrade,
) -> Response {
    ws.on_upgrade(move |socket| webkit::attach_websocket(udid, target_id, socket))
}

fn simulator_logical_screen_size(state: &AppState, udid: &str) -> Option<(f64, f64)> {
    let snapshot = state.registry.get(udid)?.snapshot();
    let width = snapshot.get("displayWidth")?.as_f64()?;
    let height = snapshot.get("displayHeight")?.as_f64()?;
    logical_screen_size_from_display_pixels(width, height)
}

fn logical_screen_size_from_display_pixels(width: f64, height: f64) -> Option<(f64, f64)> {
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return None;
    }
    let short_edge = width.min(height);
    let long_edge = width.max(height);
    let scale = if short_edge <= 1320.0 && long_edge >= 1800.0 {
        3.0
    } else if short_edge >= 700.0 && long_edge >= 1000.0 {
        2.0
    } else {
        1.0
    };
    Some((width / scale, height / scale))
}

async fn chrome_devtools_targets(
    State(state): State<AppState>,
    Path(udid): Path<String>,
    headers: HeaderMap,
) -> Result<Json<devtools::ChromeDevToolsTargetDiscovery>, AppError> {
    let origin = request_origin(&headers);
    let mut warnings = Vec::new();
    let simulator = match list_simulators_cached(state.clone(), false).await {
        Ok(simulators) => simulators
            .into_iter()
            .find(|simulator| simulator.udid == udid),
        Err(error) => {
            warnings.push(format!(
                "Unable to load simulator metadata for DevTools discovery: {error}"
            ));
            None
        }
    };
    let foreground_app_future = timeout(
        FOREGROUND_APP_ROUTE_TIMEOUT,
        foreground_app_for_simulator_with_cache_ttl(
            &state,
            &udid,
            INSPECTOR_FOREGROUND_APP_CACHE_TTL,
        ),
    );
    let external_targets_future = timeout(
        CHROME_DEVTOOLS_DISCOVERY_TIMEOUT,
        devtools::discover_external_devtools_targets(
            &udid,
            origin.as_deref(),
            Some(&state.config.access_token),
            simulator.as_ref().map(|simulator| simulator.name.as_str()),
            simulator
                .as_ref()
                .map(|simulator| simulator.device_type_name.as_str()),
        ),
    );
    let (foreground_app_result, external_targets_result) =
        tokio::join!(foreground_app_future, external_targets_future);
    let foreground_app = match foreground_app_result {
        Ok(Ok(foreground_app)) => foreground_app,
        Ok(Err(error)) => {
            tracing::debug!("Unable to load foreground app for DevTools discovery: {error}");
            stale_cached_foreground_app(&udid)
        }
        Err(_) => {
            tracing::debug!("Timed out loading foreground app for DevTools discovery.");
            stale_cached_foreground_app(&udid)
        }
    };
    let (mut external_targets, external_warnings) = match external_targets_result {
        Ok(result) => result,
        Err(_) => {
            warnings.push("Timed out loading Chrome DevTools targets.".to_owned());
            (Vec::new(), Vec::new())
        }
    };
    let mut targets = Vec::new();
    targets.append(&mut external_targets);
    warnings.extend(external_warnings);
    Ok(Json(devtools::ChromeDevToolsTargetDiscovery {
        udid,
        targets,
        warnings,
        foreground_app,
    }))
}

async fn chrome_devtools_target_socket(
    State(state): State<AppState>,
    Path((udid, target_id)): Path<(String, String)>,
    websocket: WebSocketUpgrade,
) -> Response {
    websocket.on_upgrade(move |socket| async move {
        if target_id.starts_with("metro-") || target_id.starts_with("cdp-") {
            match devtools::proxied_websocket_url_for_target(&target_id).await {
                Ok(upstream_url) => devtools::proxy_websocket(socket, upstream_url).await,
                Err(error) => {
                    tracing::debug!(
                        "Proxied DevTools target socket failed for {udid}/{target_id}: {error}"
                    );
                }
            }
        } else {
            match chrome_devtools_socket_session(&state, &udid, &target_id).await {
                Ok((runtime, query)) => devtools::handle_socket(socket, runtime, query).await,
                Err(error) => {
                    tracing::debug!(
                        "Chrome DevTools target socket failed for {udid}/{target_id}: {error}"
                    );
                }
            }
        }
    })
}

async fn chrome_devtools_socket_session(
    state: &AppState,
    udid: &str,
    target_id: &str,
) -> Result<
    (
        devtools::ChromeDevToolsTargetRuntime,
        devtools::DevToolsQuery,
    ),
    String,
> {
    let process_identifier = target_id
        .strip_prefix("sdi-")
        .and_then(|value| value.parse::<i64>().ok())
        .ok_or_else(|| "Invalid Chrome DevTools target id.".to_owned())?;
    let session = inspector_session_for_process(state, udid, process_identifier).await?;
    let source = chrome_devtools_source_for_session(&session)
        .ok_or_else(|| "This app inspector does not expose a Chrome DevTools target.".to_owned())?;
    let target = devtools::build_target(
        udid,
        None,
        &session.info,
        session.process_identifier,
        source,
    );
    let runtime = devtools::runtime_from_target(&target);
    let query_state = state.clone();
    let query_session = session.clone();
    let query: devtools::DevToolsQuery = Arc::new(move |method, params| {
        let state = query_state.clone();
        let session = query_session.clone();
        Box::pin(async move { query_inspector_session(&state, &session, &method, params).await })
    });
    Ok((runtime, query))
}

fn chrome_devtools_source_for_session(session: &InspectorSession) -> Option<&str> {
    if session
        .available_sources
        .iter()
        .any(|source| source == SOURCE_REACT_NATIVE)
    {
        return Some(SOURCE_REACT_NATIVE);
    }
    if session
        .available_sources
        .iter()
        .any(|source| source == SOURCE_NATIVE_SCRIPT)
    {
        return Some(SOURCE_NATIVE_SCRIPT);
    }
    None
}

async fn chrome_devtools_ui_redirect() -> Redirect {
    Redirect::temporary("/chrome-devtools-ui/inspector.html")
}

async fn chrome_devtools_ui_file(method: Method, uri: Uri) -> Response {
    let Some(root) = devtools::chrome_devtools_frontend_root() else {
        return AppError::not_found("Chrome DevTools frontend resources are not available.")
            .into_response();
    };
    match static_files::serve_static_under(root, "/chrome-devtools-ui", method, uri, None).await {
        Ok(response) => response,
        Err(status) => status.into_response(),
    }
}

async fn webkit_inspector_ui_redirect() -> Redirect {
    Redirect::temporary("/webkit-inspector-ui/Main.html")
}

async fn metro_proxy_root(
    Path(port): Path<u16>,
    method: Method,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Response {
    metro_proxy_response(port, "/", uri.query(), method, headers, body).await
}

async fn metro_proxy_asset(
    Path((port, path)): Path<(u16, String)>,
    method: Method,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Response {
    metro_proxy_response(
        port,
        &format!("/{path}"),
        uri.query(),
        method,
        headers,
        body,
    )
    .await
}

async fn metro_proxy_response(
    port: u16,
    asset_path: &str,
    query: Option<&str>,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok());
    match devtools::fetch_metro_resource(
        port,
        asset_path,
        query,
        method.as_str(),
        Some(body.as_ref()),
        content_type,
    )
    .await
    {
        Ok(asset) => {
            let status = StatusCode::from_u16(asset.status).unwrap_or(StatusCode::BAD_GATEWAY);
            let body = devtools::rewrite_metro_proxy_asset(
                port,
                asset_path,
                asset.content_type.as_deref(),
                asset.body,
            );
            let mut builder = Response::builder()
                .status(status)
                .header(header::CACHE_CONTROL, "no-store");
            if let Some(content_type) = asset.content_type {
                builder = builder.header(header::CONTENT_TYPE, content_type);
            }
            builder
                .body(Body::from(body))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
        Err(error) => {
            tracing::debug!("Metro proxy failed for {port}{asset_path}: {error}");
            AppError::not_found("Metro resource is not available through the proxy.")
                .into_response()
        }
    }
}

async fn webkit_inspector_ui_file(method: Method, uri: Uri) -> Response {
    let Some(root) = webkit::webkit_inspector_ui_root() else {
        return AppError::not_found("WebInspectorUI resources are not available on this Mac.")
            .into_response();
    };
    if uri.path().trim_end_matches('/') == "/webkit-inspector-ui/Main.html" {
        if method != Method::GET && method != Method::HEAD {
            return StatusCode::METHOD_NOT_ALLOWED.into_response();
        }
        let main_html = match tokio::fs::read_to_string(root.join("Main.html")).await {
            Ok(main_html) => main_html,
            Err(_) => return StatusCode::NOT_FOUND.into_response(),
        };
        let body = if method == Method::HEAD {
            Body::empty()
        } else {
            Body::from(webkit::inject_frontend_host(&main_html))
        };
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .header(
                header::CACHE_CONTROL,
                "no-store, no-cache, must-revalidate, max-age=0",
            )
            .header(header::PRAGMA, "no-cache")
            .header(header::EXPIRES, "0")
            .body(body)
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }
    match static_files::serve_static_under(root, "/webkit-inspector-ui", method, uri, None).await {
        Ok(response) => response,
        Err(status) => status.into_response(),
    }
}

fn request_origin(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
        .or_else(|| {
            headers
                .get(header::HOST)
                .and_then(|value| value.to_str().ok())
                .map(|host| format!("http://{host}"))
        })
}

async fn stream_quality(State(state): State<AppState>) -> Json<Value> {
    json(json_value!(stream_quality_response(&state.config)))
}

async fn set_stream_quality(
    State(state): State<AppState>,
    Json(payload): Json<StreamQualityPayload>,
) -> Result<Json<Value>, AppError> {
    apply_stream_quality_payload(&state, &payload).map(json)
}

pub(crate) fn apply_stream_quality_payload(
    state: &AppState,
    payload: &StreamQualityPayload,
) -> Result<Value, AppError> {
    if !payload.has_any_value() {
        return Ok(stream_quality_response(&state.config));
    }
    let video_codec = payload
        .video_codec
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            normalize_video_codec(value)
                .ok_or_else(|| AppError::bad_request(format!("Unknown video codec `{value}`.")))
        })
        .transpose()?;
    let profile = payload
        .profile
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(stream_quality_profile)
        .transpose()?;
    let limits = resolved_stream_quality_limits(payload, profile);

    let _stream_config_guard = STREAM_CONFIG_LOCK
        .get_or_init(|| StdMutex::new(()))
        .lock()
        .unwrap();
    let current = current_stream_quality_state(active_video_codec(&state.config));
    let next_video_codec = video_codec.unwrap_or(current.video_codec.as_str());
    let next_profile = profile.map(|profile| profile.id).unwrap_or("custom");
    if current.max_edge == limits.max_edge
        && current.fps == limits.fps
        && current.min_bitrate == limits.min_bitrate
        && current.bits_per_pixel == limits.bits_per_pixel
        && current.profile == next_profile
        && current.video_codec == next_video_codec
    {
        return Ok(stream_quality_response(&state.config));
    }

    env::set_var("SIMDECK_REALTIME_MAX_EDGE", limits.max_edge.to_string());
    env::set_var("SIMDECK_REALTIME_FPS", limits.fps.to_string());
    env::set_var("SIMDECK_LOCAL_STREAM_FPS", limits.fps.to_string());
    env::set_var(
        "SIMDECK_REALTIME_MIN_BITRATE",
        limits.min_bitrate.to_string(),
    );
    env::set_var(
        "SIMDECK_REALTIME_BITS_PER_PIXEL",
        limits.bits_per_pixel.to_string(),
    );
    if let Some(profile) = profile {
        env::set_var("SIMDECK_STREAM_QUALITY_PROFILE", profile.id);
    } else {
        env::set_var("SIMDECK_STREAM_QUALITY_PROFILE", "custom");
    }
    if let Some(video_codec) = video_codec {
        env::set_var("SIMDECK_VIDEO_CODEC", video_codec);
    }

    state.registry.reconfigure_video_encoders();
    Ok(stream_quality_response(&state.config))
}

fn stream_quality_response(config: &Config) -> Value {
    let video_codec = active_video_codec(config);
    let quality = current_stream_quality_state(video_codec.clone());
    json_value!({
        "ok": true,
        "quality": stream_quality_state_value(&quality),
        "videoCodec": video_codec,
        "profiles": STREAM_QUALITY_PROFILES
            .iter()
            .filter(|profile| VISIBLE_STREAM_QUALITY_PROFILE_IDS.contains(&profile.id))
            .map(stream_quality_profile_value)
            .collect::<Vec<_>>()
    })
}

fn current_stream_quality_state(video_codec: String) -> ActiveStreamQualityState {
    let configured_profile = env::var("SIMDECK_STREAM_QUALITY_PROFILE")
        .ok()
        .and_then(|value| stream_quality_profile(value.trim()).ok());
    let fallback = configured_profile.unwrap_or(StreamQualityProfile {
        id: "custom",
        label: "Custom",
        max_edge: 1440,
        fps: 30,
        min_bitrate: 3_000_000,
        bits_per_pixel: 4,
    });
    let max_edge = env_u32("SIMDECK_REALTIME_MAX_EDGE", fallback.max_edge, 320, 4096);
    let fps = env_u32("SIMDECK_REALTIME_FPS", fallback.fps, 10, 240);
    let min_bitrate = env_u32(
        "SIMDECK_REALTIME_MIN_BITRATE",
        fallback.min_bitrate,
        200_000,
        60_000_000,
    );
    let bits_per_pixel = env_u32(
        "SIMDECK_REALTIME_BITS_PER_PIXEL",
        fallback.bits_per_pixel,
        1,
        10,
    );
    let profile = env::var("SIMDECK_STREAM_QUALITY_PROFILE")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            STREAM_QUALITY_PROFILES
                .iter()
                .find(|candidate| {
                    candidate.max_edge == max_edge
                        && candidate.fps == fps
                        && candidate.min_bitrate == min_bitrate
                        && candidate.bits_per_pixel == bits_per_pixel
                })
                .map(|candidate| candidate.id.to_owned())
                .unwrap_or_else(|| "custom".to_owned())
        });
    ActiveStreamQualityState {
        profile,
        max_edge,
        fps,
        min_bitrate,
        bits_per_pixel,
        video_codec,
    }
}

fn stream_quality_state_value(state: &ActiveStreamQualityState) -> Value {
    json_value!({
        "profile": state.profile,
        "maxEdge": state.max_edge,
        "fps": state.fps,
        "minBitrate": state.min_bitrate,
        "bitsPerPixel": state.bits_per_pixel,
        "videoCodec": state.video_codec,
    })
}

fn stream_quality_profile(id: &str) -> Result<StreamQualityProfile, AppError> {
    STREAM_QUALITY_PROFILES
        .iter()
        .copied()
        .find(|profile| profile.id == id)
        .ok_or_else(|| AppError::bad_request(format!("Unknown stream quality profile `{id}`.")))
}

fn stream_quality_profile_value(profile: &StreamQualityProfile) -> Value {
    json_value!({
        "id": profile.id,
        "label": profile.label,
        "maxEdge": profile.max_edge,
        "fps": profile.fps,
        "minBitrate": profile.min_bitrate,
        "bitsPerPixel": profile.bits_per_pixel,
    })
}

pub(crate) fn stream_quality_limits_for_payload(
    payload: &StreamQualityPayload,
) -> Result<StreamQualityLimits, AppError> {
    let profile = payload
        .profile
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(stream_quality_profile)
        .transpose()?;
    Ok(resolved_stream_quality_limits(payload, profile))
}

fn resolved_stream_quality_limits(
    payload: &StreamQualityPayload,
    profile: Option<StreamQualityProfile>,
) -> StreamQualityLimits {
    StreamQualityLimits {
        max_edge: profile
            .map(|profile| profile.max_edge)
            .or(payload.max_edge)
            .unwrap_or(1440)
            .clamp(320, 4096),
        fps: payload
            .fps
            .or_else(|| profile.map(|profile| profile.fps))
            .unwrap_or(30)
            .clamp(10, 240),
        min_bitrate: payload
            .min_bitrate
            .or_else(|| profile.map(|profile| profile.min_bitrate))
            .unwrap_or(3_000_000)
            .clamp(200_000, 60_000_000),
        bits_per_pixel: payload
            .bits_per_pixel
            .or_else(|| profile.map(|profile| profile.bits_per_pixel))
            .unwrap_or(4)
            .clamp(1, 10),
    }
}

fn env_u32(name: &str, fallback: u32, minimum: u32, maximum: u32) -> u32 {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(fallback)
        .clamp(minimum, maximum)
}

async fn client_stream_stats(State(state): State<AppState>) -> Json<Value> {
    json(json_value!({
        "clientStreams": state.metrics.client_stream_stats_snapshot(),
    }))
}

async fn record_client_stream_stats(
    State(state): State<AppState>,
    Json(payload): Json<ClientStreamStats>,
) -> Result<Json<Value>, AppError> {
    if payload.client_id.trim().is_empty() || payload.kind.trim().is_empty() {
        return Err(AppError::bad_request(
            "Request body must include `clientId` and `kind`.",
        ));
    }

    apply_stream_client_foreground_from_stats(&state, &payload);
    state.metrics.record_client_stream_stats(payload);
    Ok(json(json_value!({ "ok": true })))
}

pub(crate) fn apply_stream_client_foreground_from_stats(
    state: &AppState,
    stats: &ClientStreamStats,
) {
    let Some(foreground) = client_stats_foreground(stats) else {
        return;
    };
    let Some(udid) = stats
        .udid
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    let client_id = stats.client_id.trim();
    if client_id.is_empty() {
        return;
    }
    let (any_foreground, changed) = state.stream_clients.record(udid, client_id, foreground);
    if changed {
        if let Some(session) = state.registry.get(udid) {
            session.set_client_foreground(any_foreground);
        }
    }
}

fn client_stats_foreground(stats: &ClientStreamStats) -> Option<bool> {
    if stats.kind != "page" {
        return None;
    }
    Some(stats.visibility_state.as_deref()? == "visible")
}

async fn native_inspector_connect(
    State(state): State<AppState>,
    websocket: WebSocketUpgrade,
) -> impl IntoResponse {
    websocket.on_upgrade(move |socket| async move {
        state.inspectors.handle_socket(socket).await;
    })
}

async fn inspector_poll(
    State(state): State<AppState>,
    Query(query): Query<InspectorPollQuery>,
) -> Result<Response, AppError> {
    if query.process_identifier <= 0 {
        return Err(AppError::bad_request(
            "`processIdentifier` must be a positive process id.",
        ));
    }
    state
        .inspectors
        .ensure_polled_agent(query.process_identifier)
        .await
        .map_err(AppError::native)?;
    match state
        .inspectors
        .poll(query.process_identifier, Duration::from_secs(25))
        .await
        .map_err(AppError::native)?
    {
        Some(request) => Ok(Json(request).into_response()),
        None => Ok(StatusCode::NO_CONTENT.into_response()),
    }
}

async fn inspector_direct_request(
    State(state): State<AppState>,
    Json(payload): Json<InspectorDirectRequestPayload>,
) -> Result<Json<Value>, AppError> {
    if payload.process_identifier <= 0 {
        return Err(AppError::bad_request(
            "`processIdentifier` must be a positive process id.",
        ));
    }
    let method = payload.method.trim();
    if !is_allowed_inspector_proxy_method(method) {
        return Err(AppError::bad_request(format!(
            "Unsupported inspector proxy method `{method}`."
        )));
    }

    let wait = inspector_request_timeout(method);
    let result = state
        .inspectors
        .query_with_timeout(
            payload.process_identifier,
            method,
            payload.params.unwrap_or(Value::Null),
            wait,
        )
        .await
        .map_err(AppError::native)?;

    Ok(json(json_value!({ "result": result })))
}

async fn inspector_response(
    State(state): State<AppState>,
    Json(payload): Json<InspectorResponsePayload>,
) -> Result<StatusCode, AppError> {
    let mut response = Map::new();
    response.insert("id".to_owned(), Value::Number(payload.id.into()));
    if let Some(error) = payload.error {
        response.insert("error".to_owned(), error);
    } else {
        response.insert("result".to_owned(), payload.result.unwrap_or(Value::Null));
    }

    state
        .inspectors
        .complete_response(payload.process_identifier, Value::Object(response))
        .await
        .map_err(AppError::native)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_simulators(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let simulators = all_device_values(state.clone(), false).await?;
    Ok(json(json_value!({
        "simulators": simulators,
    })))
}

async fn simulator_create_options(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let mut options =
        run_bridge_action(state.clone(), |bridge| bridge.simulator_creation_options()).await?;
    let android_options =
        match run_android_action(state, |android| android.creation_options()).await {
            Ok(options) => options,
            Err(error) => json_value!({
                "deviceTypes": [],
                "systemImages": [],
                "unavailableReason": error.to_string(),
            }),
        };
    if let Some(map) = options.as_object_mut() {
        map.insert("android".to_owned(), android_options);
    }
    Ok(json(options))
}

async fn create_simulator(
    State(state): State<AppState>,
    Json(payload): Json<CreateSimulatorPayload>,
) -> Result<Json<Value>, AppError> {
    let platform = payload.platform.as_deref().map(str::trim).unwrap_or("ios");
    let name = payload.name.trim().to_owned();
    let device_type_identifier = payload.device_type_identifier.trim().to_owned();
    if name.is_empty() {
        return Err(AppError::bad_request("Request body must include `name`."));
    }
    if device_type_identifier.is_empty() {
        return Err(AppError::bad_request(
            "Request body must include `deviceTypeIdentifier`.",
        ));
    }

    let runtime_identifier = trimmed_optional_string(payload.runtime_identifier);
    if platform.eq_ignore_ascii_case("android") {
        let system_image_identifier = runtime_identifier.ok_or_else(|| {
            AppError::bad_request("Android emulator creation requires `runtimeIdentifier`.")
        })?;
        if payload.paired_watch.is_some() {
            return Err(AppError::bad_request(
                "Android emulator creation does not support `pairedWatch`.",
            ));
        }
        let spec = AndroidEmulatorSpec {
            name,
            device_profile_identifier: device_type_identifier,
            system_image_identifier,
        };
        let created =
            run_android_action(state.clone(), move |android| android.create_emulator(spec)).await?;
        let udid = created
            .get("udid")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::internal("Android create did not return an emulator ID."))?
            .to_owned();
        boot_android_device(state.clone(), udid.clone()).await?;
        let devices = all_device_values(state, true).await?;
        let simulator = devices
            .iter()
            .find(|entry| entry.get("udid").and_then(Value::as_str) == Some(udid.as_str()))
            .cloned()
            .ok_or_else(|| {
                AppError::not_found(format!("Created emulator {udid} was not found."))
            })?;
        return Ok(json(json_value!({
            "ok": true,
            "created": created,
            "simulator": simulator,
            "pairedWatchSimulator": null,
        })));
    }

    let paired_watch = payload
        .paired_watch
        .map(|watch| {
            let watch_name = watch.name.trim().to_owned();
            let watch_device_type_identifier = watch.device_type_identifier.trim().to_owned();
            if watch_name.is_empty() {
                return Err(AppError::bad_request(
                    "Paired watch creation requires `pairedWatch.name`.",
                ));
            }
            if watch_device_type_identifier.is_empty() {
                return Err(AppError::bad_request(
                    "Paired watch creation requires `pairedWatch.deviceTypeIdentifier`.",
                ));
            }
            Ok(NativePairedWatchSpec {
                name: watch_name,
                device_type_identifier: watch_device_type_identifier,
                runtime_identifier: trimmed_optional_string(watch.runtime_identifier),
            })
        })
        .transpose()?;
    let action_name = name.clone();
    let action_device_type_identifier = device_type_identifier.clone();
    let action_runtime_identifier = runtime_identifier.clone();
    let action_paired_watch = paired_watch.clone();
    let created = run_bridge_action(state.clone(), move |bridge| {
        bridge.create_simulator(
            &action_name,
            &action_device_type_identifier,
            action_runtime_identifier.as_deref(),
            action_paired_watch.as_ref(),
        )
    })
    .await?;

    let udid = created
        .get("udid")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::internal("Native create did not return a simulator UDID."))?
        .to_owned();
    let paired_watch_udid = created
        .get("pairedWatchUDID")
        .and_then(Value::as_str)
        .map(str::to_owned);
    boot_ios_device(state.clone(), udid.clone()).await?;
    if let Some(watch_udid) = paired_watch_udid.clone() {
        boot_ios_device(state.clone(), watch_udid).await?;
    }
    let devices = all_device_values(state, true).await?;
    let simulator = devices
        .iter()
        .find(|entry| entry.get("udid").and_then(Value::as_str) == Some(udid.as_str()))
        .cloned()
        .ok_or_else(|| AppError::not_found(format!("Created simulator {udid} was not found.")))?;
    let paired_watch_simulator = paired_watch_udid.as_deref().and_then(|watch_udid| {
        devices
            .iter()
            .find(|entry| entry.get("udid").and_then(Value::as_str) == Some(watch_udid))
            .cloned()
    });

    Ok(json(json_value!({
        "ok": true,
        "created": created,
        "simulator": simulator,
        "pairedWatchSimulator": paired_watch_simulator,
    })))
}

async fn simulator_state(
    State(state): State<AppState>,
    Path(udid): Path<String>,
) -> Result<Json<Value>, AppError> {
    let simulator = if android::is_android_id(&udid) {
        let android_devices =
            run_android_action(state.clone(), |android| android.list_devices()).await?;
        state
            .android
            .enrich_devices(android_devices)
            .into_iter()
            .find(|entry| entry.get("udid").and_then(Value::as_str) == Some(udid.as_str()))
            .ok_or_else(|| AppError::not_found(format!("Unknown Android emulator {udid}")))?
    } else {
        all_device_values(state.clone(), true)
            .await?
            .into_iter()
            .find(|entry| entry.get("udid").and_then(Value::as_str) == Some(udid.as_str()))
            .ok_or_else(|| AppError::not_found(format!("Unknown simulator {udid}")))?
    };

    let display = simulator.get("privateDisplay");
    let frame_sequence = display
        .and_then(|value| value.get("frameSequence"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let last_frame_at = display
        .and_then(|value| value.get("lastFrameAt"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let last_frame_age_ms = if last_frame_at > 0 {
        Some(now_ms().saturating_sub(last_frame_at))
    } else {
        None
    };
    let is_booted = simulator
        .get("isBooted")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let foreground_app = if is_booted && !android::is_android_id(&udid) {
        foreground_app_for_simulator(&state, &udid)
            .await
            .ok()
            .flatten()
    } else {
        None
    };

    Ok(json(json_value!({
        "udid": udid,
        "booted": is_booted,
        "displayReady": display
            .and_then(|value| value.get("displayReady"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        "displayStatus": display
            .and_then(|value| value.get("displayStatus"))
            .and_then(Value::as_str)
            .unwrap_or("Unknown"),
        "frameSequence": frame_sequence,
        "lastFrameAt": last_frame_at,
        "lastFrameAgeMs": last_frame_age_ms,
        "foregroundApp": foreground_app,
        "simulator": simulator,
    })))
}

async fn simulator_processes(
    State(state): State<AppState>,
    Path(udid): Path<String>,
) -> Result<Json<Value>, AppError> {
    if android::is_android_id(&udid) {
        return Err(AppError::bad_request(
            "Performance gauges are only supported for iOS simulators.",
        ));
    }
    let foreground = performance_foreground_process(&state, &udid).await;
    let processes = state
        .performance
        .list_processes(&udid, foreground.clone())
        .await?;
    Ok(json(json_value!({
        "udid": udid,
        "foregroundProcess": foreground,
        "processes": processes,
    })))
}

async fn simulator_performance(
    State(state): State<AppState>,
    Path(udid): Path<String>,
    Query(query): Query<PerformanceRequestQuery>,
) -> Result<Json<Value>, AppError> {
    simulator_performance_payload(state, udid, query.pid, query.window_ms).await
}

async fn simulator_process_performance(
    State(state): State<AppState>,
    Path((udid, pid)): Path<(String, i32)>,
    Query(query): Query<PerformanceRequestQuery>,
) -> Result<Json<Value>, AppError> {
    simulator_performance_payload(state, udid, Some(pid), query.window_ms).await
}

async fn simulator_performance_payload(
    state: AppState,
    udid: String,
    pid: Option<i32>,
    window_ms: Option<u64>,
) -> Result<Json<Value>, AppError> {
    if android::is_android_id(&udid) {
        return Err(AppError::bad_request(
            "Performance gauges are only supported for iOS simulators.",
        ));
    }
    let foreground = performance_foreground_process(&state, &udid).await;
    let display_signal = simulator_display_signal(state.clone(), &udid).await;
    let snapshot = state
        .performance
        .snapshot(
            &udid,
            PerformanceQuery {
                pid,
                history_window_ms: window_ms.unwrap_or(120_000).clamp(10_000, 10 * 60 * 1000),
            },
            foreground,
            display_signal,
        )
        .await?;
    let events = performance_log_events(&state, &udid, &snapshot).await;
    let mut value = serde_json::to_value(snapshot).map_err(|error| {
        AppError::internal(format!("Unable to encode performance data: {error}"))
    })?;
    if let Some(object) = value.as_object_mut() {
        object.insert("events".to_owned(), Value::Array(events));
    }
    Ok(json(value))
}

async fn sample_process_stack(
    State(state): State<AppState>,
    Path((udid, pid)): Path<(String, i32)>,
    Query(query): Query<StackSampleRequestQuery>,
) -> Result<Json<Value>, AppError> {
    if android::is_android_id(&udid) {
        return Err(AppError::bad_request(
            "Performance sampling is only supported for iOS simulators.",
        ));
    }
    let foreground = performance_foreground_process(&state, &udid).await;
    let processes = state.performance.list_processes(&udid, foreground).await?;
    if !processes.iter().any(|process| process.pid == pid) {
        return Err(AppError::bad_request(format!(
            "Process {pid} does not belong to simulator {udid}."
        )));
    }
    let report = sample_stack(pid, query.seconds.unwrap_or(3)).await?;
    Ok(json(json_value!({
        "udid": udid,
        "sample": report,
    })))
}

async fn performance_foreground_process(state: &AppState, udid: &str) -> Option<ForegroundProcess> {
    let foreground = foreground_app_for_simulator(state, udid)
        .await
        .ok()
        .flatten();
    foreground.map(|foreground| ForegroundProcess {
        process_identifier: foreground.process_identifier,
        bundle_identifier: foreground.bundle_identifier,
        app_name: foreground.app_name,
    })
}

async fn simulator_display_signal(state: AppState, udid: &str) -> DisplaySignal {
    all_device_values(state, false)
        .await
        .ok()
        .and_then(|simulators| {
            simulators
                .into_iter()
                .find(|entry| entry.get("udid").and_then(Value::as_str) == Some(udid))
        })
        .and_then(|simulator| {
            let display = simulator.get("privateDisplay")?;
            Some(DisplaySignal {
                frame_sequence: display
                    .get("frameSequence")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                last_frame_at_ms: display
                    .get("lastFrameAt")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
            })
        })
        .unwrap_or_default()
}

async fn performance_log_events(
    state: &AppState,
    udid: &str,
    snapshot: &crate::performance::SimulatorPerformanceSnapshot,
) -> Vec<Value> {
    let Some(current) = snapshot.current.as_ref() else {
        return Vec::new();
    };
    let process_name = snapshot
        .processes
        .iter()
        .find(|process| process.pid == current.pid)
        .map(|process| process.process.as_str())
        .unwrap_or("");
    let filters = LogFilters::new(Vec::new(), Vec::new(), String::new());
    if state.logs.ensure_started(udid).await.is_err() {
        return Vec::new();
    }
    state
        .logs
        .snapshot(udid, &filters, 800)
        .await
        .into_iter()
        .rev()
        .filter(|entry| performance_log_entry_matches(entry, current.pid, process_name))
        .take(12)
        .map(|entry| {
            json_value!({
                "timestamp": entry.timestamp,
                "level": entry.level,
                "process": entry.process,
                "pid": entry.pid,
                "subsystem": entry.subsystem,
                "category": entry.category,
                "message": entry.message,
            })
        })
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn performance_log_entry_matches(
    entry: &crate::native::bridge::LogEntry,
    pid: i32,
    process_name: &str,
) -> bool {
    let pid_matches = entry.pid.as_i64() == Some(pid as i64);
    let process_matches = !process_name.is_empty() && entry.process == process_name;
    if !pid_matches && !process_matches {
        return false;
    }
    let haystack = format!(
        "{} {} {} {}",
        entry.level, entry.subsystem, entry.category, entry.message
    )
    .to_lowercase();
    [
        "abort",
        "crash",
        "exception",
        "exited",
        "jetsam",
        "killed",
        "signal",
        "terminat",
    ]
    .iter()
    .any(|needle| haystack.contains(needle))
}

async fn boot_android_device(state: AppState, udid: String) -> Result<(), AppError> {
    run_android_action(state, move |android| {
        android.boot(&udid)?;
        android.wait_until_booted(&udid, Duration::from_secs(240))?;
        Ok(())
    })
    .await
}

async fn boot_ios_device(state: AppState, udid: String) -> Result<(), AppError> {
    forget_lifecycle_session(&state, &udid);
    let action_udid = udid.clone();
    run_bridge_action(state.clone(), move |bridge| {
        bridge.boot_simulator(&action_udid)
    })
    .await?;
    let generation = state.accessibility_cache.generation(&udid);
    warm_accessibility_cache(state, udid, generation).await;
    Ok(())
}

async fn boot_simulator(
    State(state): State<AppState>,
    Path(udid): Path<String>,
) -> Result<Json<Value>, AppError> {
    if android::is_android_id(&udid) {
        boot_android_device(state.clone(), udid.clone()).await?;
        return android_simulator_payload(state, udid).await;
    }
    boot_ios_device(state.clone(), udid.clone()).await?;
    simulator_payload(state, udid).await
}

async fn shutdown_simulator(
    State(state): State<AppState>,
    Path(udid): Path<String>,
) -> Result<Json<Value>, AppError> {
    if android::is_android_id(&udid) {
        let action_udid = udid.clone();
        run_android_action(state.clone(), move |android| android.shutdown(&action_udid)).await?;
        return android_simulator_payload(state, udid).await;
    }
    forget_lifecycle_session(&state, &udid);
    let action_udid = udid.clone();
    run_bridge_action(state.clone(), move |bridge| {
        bridge.shutdown_simulator(&action_udid)
    })
    .await?;
    simulator_payload(state, udid).await
}

async fn erase_simulator(
    State(state): State<AppState>,
    Path(udid): Path<String>,
) -> Result<Json<Value>, AppError> {
    if android::is_android_id(&udid) {
        let action_udid = udid.clone();
        run_android_action(state, move |android| android.erase(&action_udid)).await?;
        return Ok(json(json_value!({ "ok": true })));
    }
    forget_lifecycle_session(&state, &udid);
    let action_udid = udid.clone();
    run_bridge_action(state, move |bridge| bridge.erase_simulator(&action_udid)).await?;
    Ok(json(json_value!({ "ok": true })))
}

fn forget_lifecycle_session(state: &AppState, udid: &str) {
    // SimulatorKit can reset the server-side connection if a cached private
    // display session is destructed while CoreSimulator is booting, shutting
    // down, or erasing the same device. Detach it from the registry without
    // running Objective-C teardown on the lifecycle response path.
    state.accessibility_cache.invalidate(udid);
    state.registry.forget(udid);
}

async fn install_app(
    State(state): State<AppState>,
    Path(udid): Path<String>,
    Json(payload): Json<InstallPayload>,
) -> Result<Json<Value>, AppError> {
    if payload.app_path.trim().is_empty() {
        return Err(AppError::bad_request(
            "Request body must include `appPath`.",
        ));
    }
    install_app_path(state, udid, payload.app_path).await?;
    Ok(json(json_value!({ "ok": true })))
}

async fn upload_install_app(
    State(state): State<AppState>,
    Path(udid): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, AppError> {
    if body.is_empty() {
        return Err(AppError::bad_request("Uploaded app file is empty."));
    }
    let file_name = uploaded_app_file_name(&headers)?;
    let app_kind = uploaded_app_kind(&file_name)?;
    validate_uploaded_app_target(&udid, app_kind)?;
    let upload_path = write_uploaded_app_file(&file_name, &body).await?;
    let app_path = upload_path.to_string_lossy().to_string();
    let install_result = install_app_path(state, udid.clone(), app_path).await;
    let _ = tokio::fs::remove_file(&upload_path).await;
    install_result?;
    Ok(json(json_value!({
        "ok": true,
        "udid": udid,
        "action": "install",
        "fileName": file_name,
    })))
}

async fn install_app_path(state: AppState, udid: String, app_path: String) -> Result<(), AppError> {
    state.accessibility_cache.invalidate(&udid);
    if android::is_android_id(&udid) {
        let action_udid = udid.clone();
        let action_path = app_path.clone();
        run_android_action(state, move |android| {
            android.install_app(&action_udid, &action_path)
        })
        .await?;
        return Ok(());
    }
    let action_udid = udid.clone();
    let action_path = app_path.clone();
    run_bridge_action(state.clone(), move |bridge| {
        bridge.install_app(&action_udid, &action_path)
    })
    .await?;
    spawn_accessibility_warmup(state, udid);
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UploadedAppKind {
    Apk,
    Ipa,
}

fn uploaded_app_file_name(headers: &HeaderMap) -> Result<String, AppError> {
    let raw_name = headers
        .get(APP_UPLOAD_FILE_NAME_HEADER)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("app-upload");
    let file_name = sanitize_upload_file_name(raw_name);
    if file_name.is_empty() {
        return Err(AppError::bad_request(
            "Uploaded app must include a valid file name.",
        ));
    }
    Ok(file_name)
}

fn sanitize_upload_file_name(raw_name: &str) -> String {
    let candidate = raw_name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(raw_name)
        .trim();
    let mut sanitized = String::with_capacity(candidate.len().min(160));
    for ch in candidate.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
            sanitized.push(ch);
        } else if ch.is_ascii_whitespace() {
            sanitized.push('-');
        }
    }
    while sanitized.starts_with('.') {
        sanitized.remove(0);
    }
    while sanitized.contains("..") {
        sanitized = sanitized.replace("..", ".");
    }
    if sanitized.len() > 160 {
        let extension = std::path::Path::new(&sanitized)
            .extension()
            .and_then(|value| value.to_str())
            .map(ToOwned::to_owned);
        sanitized.truncate(140);
        if let Some(extension) = extension {
            let suffix = format!(".{extension}");
            if !sanitized.ends_with(&suffix) {
                sanitized.push_str(&suffix);
            }
        }
    }
    sanitized
}

fn uploaded_app_kind(file_name: &str) -> Result<UploadedAppKind, AppError> {
    let extension = std::path::Path::new(file_name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match extension.as_str() {
        "apk" => Ok(UploadedAppKind::Apk),
        "ipa" => Ok(UploadedAppKind::Ipa),
        _ => Err(AppError::bad_request(
            "Drop an `.ipa` for iOS simulators or an `.apk` for Android emulators.",
        )),
    }
}

fn validate_uploaded_app_target(udid: &str, app_kind: UploadedAppKind) -> Result<(), AppError> {
    match (android::is_android_id(udid), app_kind) {
        (true, UploadedAppKind::Apk) | (false, UploadedAppKind::Ipa) => Ok(()),
        (true, UploadedAppKind::Ipa) => Err(AppError::bad_request(
            "Android emulators can only install `.apk` uploads.",
        )),
        (false, UploadedAppKind::Apk) => Err(AppError::bad_request(
            "iOS simulators can only install `.ipa` uploads.",
        )),
    }
}

async fn write_uploaded_app_file(
    file_name: &str,
    body: &Bytes,
) -> Result<std::path::PathBuf, AppError> {
    let upload_dir = env::temp_dir().join("simdeck").join("uploads");
    tokio::fs::create_dir_all(&upload_dir)
        .await
        .map_err(|error| {
            AppError::internal(format!(
                "Unable to create upload directory {}: {error}",
                upload_dir.display()
            ))
        })?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_nanos();
    let path = upload_dir.join(format!("{}-{}-{file_name}", std::process::id(), timestamp));
    tokio::fs::write(&path, body.as_ref())
        .await
        .map_err(|error| {
            AppError::internal(format!(
                "Unable to save uploaded app {}: {error}",
                path.display()
            ))
        })?;
    Ok(path)
}

async fn uninstall_app(
    State(state): State<AppState>,
    Path(udid): Path<String>,
    Json(payload): Json<UninstallPayload>,
) -> Result<Json<Value>, AppError> {
    state.accessibility_cache.invalidate(&udid);
    if payload.bundle_id.trim().is_empty() {
        return Err(AppError::bad_request(
            "Request body must include `bundleId`.",
        ));
    }
    if android::is_android_id(&udid) {
        let action_udid = udid.clone();
        run_android_action(state, move |android| {
            android.uninstall_app(&action_udid, &payload.bundle_id)
        })
        .await?;
        return Ok(json(json_value!({ "ok": true })));
    }
    let action_udid = udid.clone();
    run_bridge_action(state.clone(), move |bridge| {
        bridge.uninstall_app(&action_udid, &payload.bundle_id)
    })
    .await?;
    spawn_accessibility_warmup(state, udid);
    Ok(json(json_value!({ "ok": true })))
}

async fn get_pasteboard(
    State(state): State<AppState>,
    Path(udid): Path<String>,
) -> Result<Json<Value>, AppError> {
    if android::is_android_id(&udid) {
        let text = run_android_action(state, move |android| android.pasteboard_text(&udid)).await?;
        return Ok(json(json_value!({ "text": text })));
    }
    let text = run_bridge_action(state, move |bridge| bridge.pasteboard_text(&udid)).await?;
    Ok(json(json_value!({ "text": text })))
}

async fn set_pasteboard(
    State(state): State<AppState>,
    Path(udid): Path<String>,
    Json(payload): Json<PasteboardPayload>,
) -> Result<Json<Value>, AppError> {
    if android::is_android_id(&udid) {
        run_android_action(state, move |android| {
            android.set_pasteboard_text(&udid, &payload.text)
        })
        .await?;
        return Ok(json(json_value!({ "ok": true })));
    }
    run_bridge_action(state, move |bridge| {
        bridge.set_pasteboard_text(&udid, &payload.text)
    })
    .await?;
    Ok(json(json_value!({ "ok": true })))
}

async fn screenshot_png(
    State(state): State<AppState>,
    Path(udid): Path<String>,
    Query(query): Query<ScreenshotPngQuery>,
) -> Result<(StatusCode, HeaderMap, Vec<u8>), AppError> {
    let include_bezel = query
        .bezel
        .as_deref()
        .map(parse_asset_bool)
        .unwrap_or(false);
    let png = if android::is_android_id(&udid) {
        if include_bezel {
            return Err(AppError::bad_request(
                "Android emulators do not support bezeled screenshots.",
            ));
        }
        run_android_action(state, move |android| android.screenshot_png(&udid)).await?
    } else {
        run_bridge_action(state, move |bridge| {
            bridge.screenshot_png(&udid, include_bezel)
        })
        .await?
    };
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, "image/png".parse().unwrap());
    headers.insert(
        header::CACHE_CONTROL,
        "no-cache, no-store, must-revalidate".parse().unwrap(),
    );
    Ok((StatusCode::OK, headers, png))
}

async fn screen_recording(
    State(state): State<AppState>,
    Path(udid): Path<String>,
    Json(payload): Json<ScreenRecordingPayload>,
) -> Result<(StatusCode, HeaderMap, Vec<u8>), AppError> {
    let seconds = validate_screen_recording_seconds(payload.seconds)?;
    if android::is_android_id(&udid) {
        return Err(AppError::bad_request(
            "Screen recording is currently supported for iOS simulators only.",
        ));
    }
    let mp4 = run_bridge_action(state, move |bridge| {
        bridge.screen_recording_mp4(&udid, seconds)
    })
    .await?;
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, "video/mp4".parse().unwrap());
    headers.insert(
        header::CACHE_CONTROL,
        "no-cache, no-store, must-revalidate".parse().unwrap(),
    );
    Ok((StatusCode::OK, headers, mp4))
}

async fn start_screen_recording(
    State(state): State<AppState>,
    Path(udid): Path<String>,
) -> Result<Json<Value>, AppError> {
    if android::is_android_id(&udid) {
        return Err(AppError::bad_request(
            "Screen recording is currently supported for iOS simulators only.",
        ));
    }
    let recording_id =
        run_bridge_action(state, move |bridge| bridge.start_screen_recording(&udid)).await?;
    Ok(Json(json_value!({
        "ok": true,
        "recordingId": recording_id,
    })))
}

async fn stop_screen_recording(
    State(state): State<AppState>,
    Path((udid, recording_id)): Path<(String, String)>,
) -> Result<(StatusCode, HeaderMap, Vec<u8>), AppError> {
    if android::is_android_id(&udid) {
        return Err(AppError::bad_request(
            "Screen recording is currently supported for iOS simulators only.",
        ));
    }
    let mp4 = run_bridge_action(state, move |bridge| {
        bridge.stop_screen_recording(&recording_id)
    })
    .await?;
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, "video/mp4".parse().unwrap());
    headers.insert(
        header::CACHE_CONTROL,
        "no-cache, no-store, must-revalidate".parse().unwrap(),
    );
    Ok((StatusCode::OK, headers, mp4))
}

fn validate_screen_recording_seconds(seconds: Option<f64>) -> Result<f64, AppError> {
    let seconds = seconds.unwrap_or(5.0);
    if !seconds.is_finite() || seconds <= 0.0 {
        return Err(AppError::bad_request(
            "`seconds` must be finite and greater than zero.",
        ));
    }
    if seconds > 120.0 {
        return Err(AppError::bad_request(
            "`seconds` must be 120 or less for API screen recordings.",
        ));
    }
    Ok(seconds)
}

async fn refresh_stream(
    State(state): State<AppState>,
    Path(udid): Path<String>,
) -> Result<Json<Value>, AppError> {
    if android::is_android_id(&udid) {
        return Ok(json(json_value!({ "ok": true, "stream": "screenshot" })));
    }
    let session = state.registry.get_or_create_async(&udid).await?;
    if let Err(error) = session.ensure_started_async().await {
        state.registry.remove(&udid);
        return Err(error);
    }
    session.request_refresh();
    Ok(json(json_value!({ "ok": true })))
}

async fn camera_webcams() -> Result<Json<Value>, AppError> {
    let webcams = task::spawn_blocking(camera::list_webcams_value)
        .await
        .map_err(|error| AppError::internal(format!("Camera task failed. {error}")))??;
    Ok(json(webcams))
}

async fn camera_status(Path(udid): Path<String>) -> Result<Json<Value>, AppError> {
    if android::is_android_id(&udid) {
        return Err(AppError::bad_request(
            "Camera simulation is only supported for iOS simulators.",
        ));
    }
    let status = task::spawn_blocking(move || camera::camera_status(&udid))
        .await
        .map_err(|error| AppError::internal(format!("Camera task failed. {error}")))??;
    Ok(json(status))
}

async fn start_camera(
    Path(udid): Path<String>,
    Json(payload): Json<CameraStartRequest>,
) -> Result<Json<Value>, AppError> {
    if android::is_android_id(&udid) {
        return Err(AppError::bad_request(
            "Camera simulation is only supported for iOS simulators.",
        ));
    }
    let status = task::spawn_blocking(move || {
        camera::start_camera(camera::CameraStartOptions {
            udid,
            bundle_id: payload.bundle_id,
            source: payload.source,
            mirror: payload.mirror,
        })
    })
    .await
    .map_err(|error| AppError::internal(format!("Camera task failed. {error}")))??;
    Ok(json(status))
}

async fn switch_camera_source(
    Path(udid): Path<String>,
    Json(payload): Json<CameraSwitchRequest>,
) -> Result<Json<Value>, AppError> {
    if android::is_android_id(&udid) {
        return Err(AppError::bad_request(
            "Camera simulation is only supported for iOS simulators.",
        ));
    }
    let status =
        task::spawn_blocking(move || camera::switch_camera(&udid, payload.source, payload.mirror))
            .await
            .map_err(|error| AppError::internal(format!("Camera task failed. {error}")))??;
    Ok(json(status))
}

async fn stop_camera(Path(udid): Path<String>) -> Result<Json<Value>, AppError> {
    if android::is_android_id(&udid) {
        return Err(AppError::bad_request(
            "Camera simulation is only supported for iOS simulators.",
        ));
    }
    let status = task::spawn_blocking(move || camera::stop_camera(&udid))
        .await
        .map_err(|error| AppError::internal(format!("Camera task failed. {error}")))??;
    Ok(json(status))
}

async fn simulator_action(
    State(state): State<AppState>,
    Path(udid): Path<String>,
    Json(payload): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let action = payload
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if action.eq_ignore_ascii_case("batch") {
        let payload: BatchPayload = serde_json::from_value(payload).map_err(|error| {
            AppError::bad_request(format!("Invalid batch action payload: {error}"))
        })?;
        let result = run_batch_steps(state, udid, payload).await?;
        return Ok(json(result));
    }

    let step: BatchStep = serde_json::from_value(payload)
        .map_err(|error| AppError::bad_request(format!("Invalid action payload: {error}")))?;
    let invalidates_ax_cache = batch_step_invalidates_ax_cache(&step);
    let should_warm_ax = batch_step_should_warm_ax(&step);
    if invalidates_ax_cache {
        state.accessibility_cache.invalidate(&udid);
    }
    let result = run_batch_step(state.clone(), udid.clone(), step).await?;
    if should_warm_ax {
        spawn_accessibility_warmup(state, udid);
    }
    Ok(json(action_response_value(result)))
}

fn action_response_value(result: Value) -> Value {
    match result {
        Value::Object(mut object) => {
            object
                .entry("ok".to_owned())
                .or_insert_with(|| Value::Bool(true));
            Value::Object(object)
        }
        value => json_value!({ "ok": true, "result": value }),
    }
}

async fn control_socket(
    State(state): State<AppState>,
    Path(udid): Path<String>,
    websocket: WebSocketUpgrade,
) -> impl IntoResponse {
    if android::is_android_id(&udid) {
        return websocket
            .on_upgrade(move |socket| handle_android_control_socket(state, udid, socket));
    }
    websocket.on_upgrade(move |socket| handle_control_socket(state, udid, socket))
}

async fn removed_h264_stream(Path(_udid): Path<String>) -> impl IntoResponse {
    (
        StatusCode::GONE,
        Json(json_value!({
            "ok": false,
            "error": "H.264 WebSocket video streaming has been removed. Use /api/simulators/{udid}/webrtc/offer.",
        })),
    )
}

async fn handle_android_control_socket(state: AppState, udid: String, socket: WebSocket) {
    let (mut sender, mut receiver) = socket.split();
    let _ = sender
        .send(Message::Text(
            json_value!({ "type": "ready", "udid": udid, "platform": "android-emulator" })
                .to_string()
                .into(),
        ))
        .await;
    while let Some(message) = receiver.next().await {
        let text = match message {
            Ok(Message::Text(text)) => text,
            Ok(Message::Binary(bytes)) => match String::from_utf8(bytes.to_vec()) {
                Ok(text) => text.into(),
                Err(_) => continue,
            },
            Ok(Message::Close(_)) => break,
            Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => continue,
            Err(_) => break,
        };
        let Ok(control_message) = serde_json::from_str::<ControlMessage>(&text) else {
            continue;
        };
        let state = state.clone();
        let udid = udid.clone();
        let _ = run_android_control_message(state, udid, control_message).await;
    }
}

async fn run_android_control_message(
    state: AppState,
    udid: String,
    message: ControlMessage,
) -> Result<(), AppError> {
    match message {
        ControlMessage::Touch { x, y, phase } => {
            handle_android_control_touch(state, udid, x, y, phase).await
        }
        ControlMessage::EdgeTouch { x, y, phase, .. } => {
            handle_android_control_touch(state, udid, x, y, phase).await
        }
        ControlMessage::MultiTouch { x1, y1, phase, .. } => {
            handle_android_control_touch(state, udid, x1, y1, phase).await
        }
        other => {
            run_android_action(state, move |android| match other {
                ControlMessage::Key {
                    key_code,
                    modifiers,
                } => android.send_key(&udid, key_code, modifiers.unwrap_or(0)),
                ControlMessage::Button {
                    button,
                    duration_ms,
                    phase,
                    ..
                } => match phase.as_deref() {
                    Some("down" | "began") => Ok(()),
                    Some("up" | "ended" | "cancelled") | None => {
                        android.press_button(&udid, &button, duration_ms.unwrap_or(0))
                    }
                    Some(_) => Err(AppError::bad_request(
                        "`phase` must be `down`, `up`, `began`, `ended`, or `cancelled`.",
                    )),
                },
                ControlMessage::DismissKeyboard => android.dismiss_keyboard(&udid),
                ControlMessage::ToggleSoftwareKeyboard => Err(AppError::bad_request(
                    "Software keyboard toggle is only available for iOS simulators.",
                )),
                ControlMessage::Home => android.press_home(&udid),
                ControlMessage::AppSwitcher => android.open_app_switcher(&udid),
                ControlMessage::RotateLeft => android.rotate_left(&udid),
                ControlMessage::RotateRight => android.rotate_right(&udid),
                ControlMessage::Crown { .. } => Err(AppError::bad_request(
                    "Digital Crown rotation is only available for Apple Watch simulators.",
                )),
                ControlMessage::Scroll { .. } => Err(AppError::bad_request(
                    "Native scroll wheel input is only available for iOS simulators.",
                )),
                ControlMessage::ToggleAppearance => android.toggle_appearance(&udid),
                ControlMessage::Touch { .. }
                | ControlMessage::EdgeTouch { .. }
                | ControlMessage::MultiTouch { .. } => Ok(()),
            })
            .await
        }
    }
}

async fn handle_android_control_touch(
    state: AppState,
    udid: String,
    x: f64,
    y: f64,
    phase: String,
) -> Result<(), AppError> {
    run_android_action(state, move |android| {
        android.send_touch(&udid, x, y, &phase)
    })
    .await
}

async fn webrtc_offer(
    State(state): State<AppState>,
    ConnectInfo(address): ConnectInfo<SocketAddr>,
    Path(udid): Path<String>,
    Json(payload): Json<crate::transport::webrtc::WebRtcOfferPayload>,
) -> Result<Json<crate::transport::webrtc::WebRtcAnswerPayload>, AppError> {
    crate::transport::webrtc::create_answer(state, udid, payload, address.ip().is_loopback())
        .await
        .map(Json)
}

async fn handle_control_socket(state: AppState, udid: String, socket: WebSocket) {
    let session = match state.registry.get_or_create_async(&udid).await {
        Ok(session) => session,
        Err(error) => {
            tracing::debug!("Failed to create control session for {udid}: {error}");
            return;
        }
    };
    if let Err(error) = session.ensure_started_async().await {
        tracing::debug!("Failed to start control session for {udid}: {error}");
        return;
    }

    let (mut sender, mut receiver) = socket.split();
    let _ = sender
        .send(Message::Text(
            json_value!({ "type": "ready", "udid": udid })
                .to_string()
                .into(),
        ))
        .await;
    let (control_tx, control_rx) = mpsc::unbounded_channel::<ControlMessage>();
    let bridge = state.registry.bridge().clone();
    let control_task = task::spawn(run_control_queue(session, bridge, udid.clone(), control_rx));

    while let Some(message) = receiver.next().await {
        let text = match message {
            Ok(Message::Text(text)) => text,
            Ok(Message::Binary(bytes)) => match String::from_utf8(bytes.to_vec()) {
                Ok(text) => text.into(),
                Err(_) => continue,
            },
            Ok(Message::Close(_)) => break,
            Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => continue,
            Err(error) => {
                tracing::debug!("Control WebSocket closed for {udid}: {error}");
                break;
            }
        };

        let control_message = match serde_json::from_str::<ControlMessage>(&text) {
            Ok(message) => message,
            Err(error) => {
                tracing::debug!("Invalid control message for {udid}: {error}");
                continue;
            }
        };
        if control_tx.send(control_message).is_err() {
            break;
        }
    }
    drop(control_tx);
    let _ = control_task.await;
}

async fn run_control_queue(
    session: SimulatorSession,
    bridge: NativeBridge,
    udid: String,
    mut receiver: mpsc::UnboundedReceiver<ControlMessage>,
) {
    let mut pending = VecDeque::new();
    let mut tvos_touch = TvosControlTouchGesture::default();
    let mut multitouch_input_session: Option<Arc<NativeInputSession>> = None;
    loop {
        let mut message = match pending.pop_front() {
            Some(message) => message,
            None => {
                if multitouch_input_session.is_some() {
                    tokio::select! {
                        message = receiver.recv() => match message {
                            Some(message) => message,
                            None => break,
                        },
                        _ = tokio::time::sleep(MULTITOUCH_INPUT_IDLE_TIMEOUT) => {
                            multitouch_input_session = None;
                            continue;
                        }
                    }
                } else {
                    match receiver.recv().await {
                        Some(message) => message,
                        None => break,
                    }
                }
            }
        };
        if control_message_is_move(&message) {
            while let Ok(next_message) = receiver.try_recv() {
                if control_message_is_move(&next_message) {
                    message = next_message;
                } else {
                    pending.push_back(next_message);
                    break;
                }
            }
        }
        if multitouch_input_session.is_some()
            && !matches!(message, ControlMessage::MultiTouch { .. })
        {
            multitouch_input_session = None;
        }
        let result = match message {
            ControlMessage::ToggleAppearance => {
                run_toggle_appearance_control(bridge.clone(), udid.clone()).await
            }
            message if session.is_tvos() => {
                run_tvos_control_message(session.clone(), message, &mut tvos_touch).await
            }
            message @ ControlMessage::MultiTouch { .. } => {
                let should_clear_input = control_message_ends_touch(&message);
                let result = match bridge_input_session_for_control(
                    &mut multitouch_input_session,
                    bridge.clone(),
                    &udid,
                )
                .await
                {
                    Ok(input) => run_bridge_multitouch_control_message(input, message).await,
                    Err(error) => Err(error),
                };
                if should_clear_input {
                    multitouch_input_session = None;
                }
                result
            }
            message => run_control_message(session.clone(), message).await,
        };
        if let Err(error) = result {
            tracing::debug!("Control message failed for {udid}: {error}");
        }
    }
}

fn control_message_is_move(message: &ControlMessage) -> bool {
    matches!(
        message,
        ControlMessage::Touch { phase, .. }
            | ControlMessage::EdgeTouch { phase, .. }
            if phase == "moved"
    )
}

fn edge_name_to_hid_value(edge: &str) -> Option<u32> {
    let edge = edge.trim().to_ascii_lowercase();
    match edge.as_str() {
        "left" => Some(1),
        "top" => Some(2),
        "bottom" => Some(3),
        "right" => Some(4),
        "none" => Some(0),
        _ => None,
    }
}

fn control_message_ends_touch(message: &ControlMessage) -> bool {
    matches!(
        message,
        ControlMessage::Touch { phase, .. }
            | ControlMessage::EdgeTouch { phase, .. }
            | ControlMessage::MultiTouch { phase, .. }
            if phase == "ended" || phase == "cancelled"
    )
}

pub(crate) async fn run_control_message(
    session: SimulatorSession,
    message: ControlMessage,
) -> Result<(), AppError> {
    task::spawn_blocking(move || match message {
        ControlMessage::Touch { x, y, phase } => {
            if !x.is_finite() || !y.is_finite() {
                return Err(AppError::bad_request(
                    "`x` and `y` must be finite normalized numbers.",
                ));
            }
            session.send_touch(x.clamp(0.0, 1.0), y.clamp(0.0, 1.0), &phase)
        }
        ControlMessage::EdgeTouch { x, y, phase, edge } => {
            if !x.is_finite() || !y.is_finite() {
                return Err(AppError::bad_request(
                    "`x` and `y` must be finite normalized numbers.",
                ));
            }
            let edge = edge_name_to_hid_value(edge.as_str()).ok_or_else(|| {
                AppError::bad_request("`edge` must be `left`, `top`, `bottom`, `right`, or `none`.")
            })?;
            session.send_edge_touch(x.clamp(0.0, 1.0), y.clamp(0.0, 1.0), &phase, edge)
        }
        ControlMessage::MultiTouch {
            x1,
            y1,
            x2,
            y2,
            phase,
        } => {
            if !x1.is_finite() || !y1.is_finite() || !x2.is_finite() || !y2.is_finite() {
                return Err(AppError::bad_request(
                    "`x1`, `y1`, `x2`, and `y2` must be finite normalized numbers.",
                ));
            }
            session.send_multitouch(x1, y1, x2, y2, &phase)
        }
        ControlMessage::Key {
            key_code,
            modifiers,
        } => session.send_key(key_code, modifiers.unwrap_or(0)),
        ControlMessage::Button {
            button,
            duration_ms,
            phase,
            usage_page,
            usage,
        } => {
            if let Some(phase) = phase {
                let pressed = match phase.as_str() {
                    "down" | "began" => true,
                    "up" | "ended" | "cancelled" => false,
                    _ => {
                        return Err(AppError::bad_request(
                            "`phase` must be `down`, `up`, `began`, `ended`, or `cancelled`.",
                        ))
                    }
                };
                session.send_button(&button, pressed, usage_page, usage)
            } else {
                session.press_button(&button, duration_ms.unwrap_or(0))
            }
        }
        ControlMessage::Crown { delta } => session.rotate_crown(delta),
        ControlMessage::Scroll {
            delta_x,
            delta_y,
            x,
            y,
        } => {
            if !delta_x.is_finite() || !delta_y.is_finite() {
                return Err(AppError::bad_request("Scroll deltas must be finite."));
            }
            session.send_scroll(delta_x, delta_y, x.unwrap_or(0.5), y.unwrap_or(0.5))
        }
        ControlMessage::DismissKeyboard => session.send_key(41, 0),
        ControlMessage::ToggleSoftwareKeyboard => session.press_button("software-keyboard", 0),
        ControlMessage::Home => session.press_home(),
        ControlMessage::AppSwitcher => session.open_app_switcher(),
        ControlMessage::RotateLeft => session.rotate_left(),
        ControlMessage::RotateRight => session.rotate_right(),
        ControlMessage::ToggleAppearance => Err(AppError::bad_request(
            "`toggleAppearance` requires a native bridge control handler.",
        )),
    })
    .await
    .map_err(|error| AppError::internal(format!("Failed to join control task: {error}")))?
}

pub(crate) async fn bridge_input_session_for_control(
    input_session: &mut Option<Arc<NativeInputSession>>,
    bridge: NativeBridge,
    udid: &str,
) -> Result<Arc<NativeInputSession>, AppError> {
    if let Some(input) = input_session {
        return Ok(input.clone());
    }

    let udid = udid.to_string();
    let input = task::spawn_blocking(move || bridge.create_input_session(&udid))
        .await
        .map_err(|error| {
            AppError::internal(format!(
                "Failed to join bridge input creation task: {error}"
            ))
        })??;
    let input = Arc::new(input);
    *input_session = Some(input.clone());
    Ok(input)
}

pub(crate) async fn run_bridge_multitouch_control_message(
    input: Arc<NativeInputSession>,
    message: ControlMessage,
) -> Result<(), AppError> {
    task::spawn_blocking(move || match message {
        ControlMessage::MultiTouch {
            x1,
            y1,
            x2,
            y2,
            phase,
        } => {
            if !x1.is_finite() || !y1.is_finite() || !x2.is_finite() || !y2.is_finite() {
                return Err(AppError::bad_request(
                    "`x1`, `y1`, `x2`, and `y2` must be finite normalized numbers.",
                ));
            }
            input.send_multitouch(x1, y1, x2, y2, &phase)
        }
        _ => Err(AppError::bad_request(
            "Bridge input control only supports multi-touch messages.",
        )),
    })
    .await
    .map_err(|error| AppError::internal(format!("Failed to join bridge input task: {error}")))?
}

pub(crate) async fn run_tvos_control_message(
    session: SimulatorSession,
    message: ControlMessage,
    active_touch: &mut TvosControlTouchGesture,
) -> Result<(), AppError> {
    let key_code = match message {
        ControlMessage::Touch { x, y, phase } => {
            if !x.is_finite() || !y.is_finite() {
                return Err(AppError::bad_request(
                    "`x` and `y` must be finite normalized numbers.",
                ));
            }
            active_touch.update(x, y, &phase)?
        }
        ControlMessage::EdgeTouch { x, y, phase, .. } => {
            if !x.is_finite() || !y.is_finite() {
                return Err(AppError::bad_request(
                    "`x` and `y` must be finite normalized numbers.",
                ));
            }
            active_touch.update(x, y, &phase)?
        }
        ControlMessage::MultiTouch { x1, y1, phase, .. } => {
            if !x1.is_finite() || !y1.is_finite() {
                return Err(AppError::bad_request(
                    "`x1` and `y1` must be finite normalized numbers.",
                ));
            }
            active_touch.update(x1, y1, &phase)?
        }
        other => return run_control_message(session, other).await,
    };

    if let Some(key_code) = key_code {
        task::spawn_blocking(move || session.send_key(key_code, 0))
            .await
            .map_err(|error| AppError::internal(format!("Failed to join control task: {error}")))?
    } else {
        Ok(())
    }
}

pub(crate) async fn run_toggle_appearance_control(
    bridge: NativeBridge,
    udid: String,
) -> Result<(), AppError> {
    task::spawn_blocking(move || bridge.toggle_appearance(&udid))
        .await
        .map_err(|error| AppError::internal(format!("Failed to join control task: {error}")))?
}

async fn chrome_profile(
    State(state): State<AppState>,
    Path(udid): Path<String>,
) -> Result<Json<Value>, AppError> {
    if android::is_android_id(&udid) {
        let profile =
            run_android_action(state, move |android| android.chrome_profile(&udid)).await?;
        return Ok(json(profile));
    }
    let profile = run_bridge_action(state, move |bridge| bridge.chrome_profile(&udid)).await?;
    Ok(json(json_value!(profile)))
}

fn chrome_asset_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, "image/png".parse().unwrap());
    headers.insert(
        header::CACHE_CONTROL,
        "private, max-age=86400".parse().unwrap(),
    );
    headers
}

async fn chrome_png(
    State(state): State<AppState>,
    Path(udid): Path<String>,
    Query(query): Query<ChromePngQuery>,
) -> Result<(StatusCode, HeaderMap, Vec<u8>), AppError> {
    if android::is_android_id(&udid) {
        return Err(AppError::not_found(
            "Android emulators do not expose device chrome assets.",
        ));
    }
    let include_buttons = query
        .buttons
        .as_deref()
        .map(|value| !value.eq_ignore_ascii_case("false"))
        .unwrap_or(true);
    let png = run_bridge_action(state, move |bridge| {
        bridge.chrome_png_with_buttons(&udid, include_buttons)
    })
    .await?;
    let headers = chrome_asset_headers();
    Ok((StatusCode::OK, headers, png))
}

fn parse_asset_bool(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

async fn chrome_button_png(
    State(state): State<AppState>,
    Path((udid, button)): Path<(String, String)>,
    Query(query): Query<ChromeButtonPngQuery>,
) -> Result<(StatusCode, HeaderMap, Vec<u8>), AppError> {
    let button_name = button.strip_suffix(".png").unwrap_or(&button).to_owned();
    let png = if let Some(pressed) = query.pressed.as_deref().map(parse_asset_bool) {
        run_bridge_action(state, move |bridge| {
            bridge.chrome_button_png(&udid, &button_name, pressed)
        })
        .await?
    } else if let Some(base_name) = button_name.strip_suffix("-down").map(str::to_owned) {
        let exact_udid = udid.clone();
        let exact_name = button_name.clone();
        match run_bridge_action(state.clone(), move |bridge| {
            bridge.chrome_button_png(&exact_udid, &exact_name, false)
        })
        .await
        {
            Ok(png) => png,
            Err(_) => {
                run_bridge_action(state, move |bridge| {
                    bridge.chrome_button_png(&udid, &base_name, true)
                })
                .await?
            }
        }
    } else {
        run_bridge_action(state, move |bridge| {
            bridge.chrome_button_png(&udid, &button_name, false)
        })
        .await?
    };
    let headers = chrome_asset_headers();
    Ok((StatusCode::OK, headers, png))
}

async fn screen_mask_png(
    State(state): State<AppState>,
    Path(udid): Path<String>,
) -> Result<(StatusCode, HeaderMap, Vec<u8>), AppError> {
    if android::is_android_id(&udid) {
        return Err(AppError::not_found(
            "Android emulators do not expose screen mask assets.",
        ));
    }
    let png = run_bridge_action(state, move |bridge| bridge.screen_mask_png(&udid)).await?;
    let headers = chrome_asset_headers();
    Ok((StatusCode::OK, headers, png))
}

async fn accessibility_tree(
    State(state): State<AppState>,
    Path(udid): Path<String>,
    Query(query): Query<AccessibilityTreeQuery>,
) -> Result<Json<Value>, AppError> {
    let snapshot = cached_accessibility_tree_value(
        state,
        udid,
        query.source.as_deref(),
        query.max_depth,
        query.include_hidden.unwrap_or(false),
        query.interactive_only.unwrap_or(false),
    )
    .await?;
    Ok(json(snapshot))
}

async fn cached_accessibility_tree_value(
    state: AppState,
    udid: String,
    source: Option<&str>,
    max_depth: Option<usize>,
    include_hidden: bool,
    interactive_only: bool,
) -> Result<Value, AppError> {
    let cache_key =
        accessibility_cache_key(&udid, source, max_depth, include_hidden, interactive_only);
    if let Some(cache_key) = cache_key.as_ref() {
        if let Some((cached_key, snapshot)) = state.accessibility_cache.get_compatible(cache_key) {
            return Ok(if cached_key.max_depth != cache_key.max_depth {
                trim_tree_depth(snapshot, cache_key.max_depth)
            } else {
                snapshot
            });
        }
    }

    refresh_accessibility_tree_value(
        state,
        udid,
        source,
        max_depth,
        include_hidden,
        interactive_only,
    )
    .await
}

async fn refresh_accessibility_tree_value(
    state: AppState,
    udid: String,
    source: Option<&str>,
    max_depth: Option<usize>,
    include_hidden: bool,
    interactive_only: bool,
) -> Result<Value, AppError> {
    let cache_key =
        accessibility_cache_key(&udid, source, max_depth, include_hidden, interactive_only);
    let generation = state.accessibility_cache.generation(&udid);
    let snapshot = accessibility_tree_value(
        state.clone(),
        udid.clone(),
        source,
        max_depth,
        include_hidden,
        interactive_only,
    )
    .await?;
    if let Some(cache_key) = cache_key {
        state
            .accessibility_cache
            .insert_if_generation(cache_key, &snapshot, generation);
    }
    Ok(snapshot)
}

fn accessibility_cache_key(
    udid: &str,
    source: Option<&str>,
    max_depth: Option<usize>,
    include_hidden: bool,
    interactive_only: bool,
) -> Option<AccessibilitySnapshotCacheKey> {
    let source = AccessibilitySource::parse(source).ok()?;
    if source != AccessibilitySource::NativeAX
        && !(interactive_only && source == AccessibilitySource::Auto)
    {
        return None;
    }
    Some(AccessibilitySnapshotCacheKey {
        udid: udid.to_owned(),
        source: source.as_query_value().to_owned(),
        max_depth: max_depth.map(|depth| depth.min(80)),
        include_hidden,
        interactive_only,
    })
}

fn spawn_accessibility_warmup(state: AppState, udid: String) {
    let generation = state.accessibility_cache.generation(&udid);
    if android::is_android_id(&udid) || !state.accessibility_cache.begin_warming(&udid, generation)
    {
        return;
    }
    tokio::spawn(async move {
        warm_accessibility_cache(state.clone(), udid.clone(), generation).await;
        state.accessibility_cache.finish_warming(&udid, generation);
    });
}

async fn warm_accessibility_cache(state: AppState, udid: String, generation: u64) {
    if android::is_android_id(&udid) {
        return;
    }
    let session = match state.registry.get_or_create_async(&udid).await {
        Ok(session) => session,
        Err(error) => {
            tracing::debug!("AX warmup skipped session creation for {udid}: {error}");
            return;
        }
    };
    if let Err(error) = session.ensure_started_async().await {
        tracing::debug!("AX warmup skipped display start for {udid}: {error}");
    }

    for (max_depth, interactive_only) in [(Some(8), true)] {
        let Ok(snapshot) = native_ax_accessibility_tree_value(
            state.clone(),
            udid.clone(),
            max_depth,
            false,
            interactive_only,
        )
        .await
        else {
            continue;
        };
        state.accessibility_cache.insert_if_generation(
            AccessibilitySnapshotCacheKey {
                udid: udid.clone(),
                source: AccessibilitySource::NativeAX.as_query_value().to_owned(),
                max_depth,
                include_hidden: false,
                interactive_only,
            },
            &snapshot,
            generation,
        );
        if interactive_only {
            state.accessibility_cache.insert_if_generation(
                AccessibilitySnapshotCacheKey {
                    udid: udid.clone(),
                    source: AccessibilitySource::Auto.as_query_value().to_owned(),
                    max_depth,
                    include_hidden: false,
                    interactive_only,
                },
                &snapshot,
                generation,
            );
        }
    }

    tokio::time::sleep(Duration::from_millis(500)).await;
    if state.accessibility_cache.generation(&udid) != generation {
        return;
    }
    let Ok(snapshot) =
        native_ax_accessibility_tree_value(state.clone(), udid.clone(), Some(8), false, false)
            .await
    else {
        return;
    };
    state.accessibility_cache.insert_if_generation(
        AccessibilitySnapshotCacheKey {
            udid,
            source: AccessibilitySource::NativeAX.as_query_value().to_owned(),
            max_depth: Some(8),
            include_hidden: false,
            interactive_only: false,
        },
        &snapshot,
        generation,
    );
}

async fn accessibility_tree_value(
    state: AppState,
    udid: String,
    source: Option<&str>,
    max_depth: Option<usize>,
    include_hidden: bool,
    interactive_only: bool,
) -> Result<Value, AppError> {
    if android::is_android_id(&udid) {
        let requested_source = source
            .filter(|source| *source != "auto")
            .map(|source| source.to_owned());
        return run_android_action(state, move |android| {
            let mut tree = android.accessibility_tree(&udid, max_depth)?;
            if include_hidden {
                tree["includeHidden"] = Value::Bool(true);
            }
            if let Some(source) = requested_source {
                tree["requestedSource"] = Value::String(source);
            }
            if interactive_only {
                tree = interactive_accessibility_snapshot(&tree);
            }
            Ok(tree)
        })
        .await;
    }
    let requested_source = AccessibilitySource::parse(source).map_err(AppError::bad_request)?;
    if requested_source == AccessibilitySource::AndroidUiautomator {
        return Err(AppError::bad_request(
            "`android-uiautomator` source is only available for Android emulator IDs.",
        ));
    }
    let max_depth = max_depth.map(|depth| depth.min(80));

    if requested_source == AccessibilitySource::NativeAX
        || (interactive_only && requested_source == AccessibilitySource::Auto)
    {
        return native_ax_accessibility_tree_value(
            state,
            udid,
            max_depth,
            include_hidden,
            interactive_only,
        )
        .await;
    }

    match inspector_session_for_state(&state, &udid).await {
        Ok(session) => {
            let hierarchy_source = match requested_source {
                AccessibilitySource::Auto => InAppHierarchySource::Automatic,
                AccessibilitySource::NativeScript => InAppHierarchySource::Automatic,
                AccessibilitySource::ReactNative => InAppHierarchySource::Automatic,
                AccessibilitySource::Flutter => InAppHierarchySource::Automatic,
                AccessibilitySource::SwiftUI => InAppHierarchySource::Automatic,
                AccessibilitySource::UIKit => InAppHierarchySource::UIKit,
                AccessibilitySource::NativeAX => unreachable!(),
                AccessibilitySource::AndroidUiautomator => unreachable!(),
            };
            match run_in_app_inspector_hierarchy(
                &state,
                &session,
                hierarchy_source,
                max_depth,
                include_hidden,
                interactive_only,
            )
            .await
            {
                Ok(snapshot) => {
                    let base_sources = available_sources_with_native_ax(Some(&session));
                    let available_sources =
                        available_sources_for_snapshot(&base_sources, &snapshot);
                    let snapshot_source = snapshot.get("source").and_then(Value::as_str);
                    let fallback_reason = if requested_source == AccessibilitySource::NativeScript
                        && snapshot_source != Some(SOURCE_NATIVE_SCRIPT)
                    {
                        Some("NativeScript hierarchy is not published by the app.".to_owned())
                    } else if requested_source == AccessibilitySource::ReactNative
                        && snapshot_source != Some(SOURCE_REACT_NATIVE)
                    {
                        Some("React Native hierarchy is not published by the app.".to_owned())
                    } else if requested_source == AccessibilitySource::Flutter
                        && snapshot_source != Some(SOURCE_FLUTTER)
                    {
                        Some("Flutter hierarchy is not published by the app.".to_owned())
                    } else if requested_source == AccessibilitySource::SwiftUI
                        && snapshot_source != Some(SOURCE_SWIFTUI)
                    {
                        Some("SwiftUI hierarchy is not published by the app.".to_owned())
                    } else {
                        None
                    };
                    let snapshot =
                        attach_tree_metadata(snapshot, &available_sources, fallback_reason);
                    Ok(if interactive_only {
                        interactive_accessibility_snapshot(&snapshot)
                    } else {
                        snapshot
                    })
                }
                Err(_inspector_error) => {
                    let mut available_sources = available_sources_with_native_ax(Some(&session));
                    if requested_source == AccessibilitySource::UIKit {
                        if let Ok(snapshot) = run_in_app_inspector_hierarchy(
                            &state,
                            &session,
                            InAppHierarchySource::Automatic,
                            Some(0),
                            include_hidden,
                            false,
                        )
                        .await
                        {
                            available_sources =
                                available_sources_for_snapshot(&available_sources, &snapshot);
                        }
                    }
                    match accessibility_snapshot(state.clone(), udid.clone(), None, max_depth).await
                    {
                        Ok(native_snapshot) => {
                            let snapshot = attach_available_sources(
                                trim_tree_depth(native_snapshot, max_depth),
                                &available_sources,
                            );
                            Ok(if interactive_only {
                                interactive_accessibility_snapshot(&snapshot)
                            } else {
                                snapshot
                            })
                        }
                        Err(native_ax_error) => {
                            let snapshot = empty_accessibility_tree(
                                SOURCE_NATIVE_AX,
                                &available_sources,
                                suppress_native_ax_translation_error(&native_ax_error.to_string()),
                            );
                            Ok(if interactive_only {
                                interactive_accessibility_snapshot(&snapshot)
                            } else {
                                snapshot
                            })
                        }
                    }
                }
            }
        }
        Err(_inspector_error) => {
            let available_sources = available_sources_with_native_ax(None);
            match accessibility_snapshot(state.clone(), udid.clone(), None, max_depth).await {
                Ok(native_snapshot) => {
                    let snapshot = attach_available_sources(
                        trim_tree_depth(native_snapshot, max_depth),
                        &available_sources,
                    );
                    Ok(if interactive_only {
                        interactive_accessibility_snapshot(&snapshot)
                    } else {
                        snapshot
                    })
                }
                Err(native_ax_error) => {
                    let snapshot = empty_accessibility_tree(
                        SOURCE_NATIVE_AX,
                        &available_sources,
                        suppress_native_ax_translation_error(&native_ax_error.to_string()),
                    );
                    Ok(if interactive_only {
                        interactive_accessibility_snapshot(&snapshot)
                    } else {
                        snapshot
                    })
                }
            }
        }
    }
}

async fn native_ax_accessibility_tree_value(
    state: AppState,
    udid: String,
    max_depth: Option<usize>,
    include_hidden: bool,
    interactive_only: bool,
) -> Result<Value, AppError> {
    let available_sources = native_ax_available_sources(&state, &udid).await;
    match accessibility_snapshot_with_retries(state, udid, None, max_depth, interactive_only).await
    {
        Ok(native_snapshot) => {
            let mut snapshot = attach_available_sources(
                trim_tree_depth(native_snapshot, max_depth),
                &available_sources,
            );
            if include_hidden {
                snapshot["includeHidden"] = Value::Bool(true);
            }
            Ok(if interactive_only {
                interactive_accessibility_snapshot(&snapshot)
            } else {
                snapshot
            })
        }
        Err(native_ax_error) => {
            let snapshot = empty_accessibility_tree(
                SOURCE_NATIVE_AX,
                &available_sources,
                suppress_native_ax_translation_error(&native_ax_error.to_string()),
            );
            Ok(if interactive_only {
                interactive_accessibility_snapshot(&snapshot)
            } else {
                snapshot
            })
        }
    }
}

async fn native_ax_available_sources(state: &AppState, udid: &str) -> Vec<String> {
    match timeout(
        ACCESSIBILITY_SOURCE_DISCOVERY_TIMEOUT,
        inspector_session_for_state(state, udid),
    )
    .await
    {
        Ok(Ok(session)) => available_sources_with_native_ax(Some(&session)),
        Ok(Err(error)) => {
            tracing::debug!("Native AX source discovery found no inspector for {udid}: {error}");
            available_sources_with_native_ax(None)
        }
        Err(_) => {
            tracing::debug!("Native AX source discovery timed out for {udid}");
            available_sources_with_native_ax(None)
        }
    }
}

async fn accessibility_point(
    State(state): State<AppState>,
    Path(udid): Path<String>,
    Query(query): Query<AccessibilityPointQuery>,
) -> Result<Json<Value>, AppError> {
    if !query.x.is_finite() || !query.y.is_finite() || query.x < 0.0 || query.y < 0.0 {
        return Err(AppError::bad_request(
            "`x` and `y` must be finite non-negative numbers.",
        ));
    }

    if android::is_android_id(&udid) {
        let snapshot = run_android_action(state, move |android| {
            android.accessibility_tree(&udid, None)
        })
        .await?;
        return Ok(json(accessibility_point_snapshot(
            &snapshot, query.x, query.y,
        )?));
    }
    let snapshot = accessibility_snapshot(
        state.clone(),
        udid.clone(),
        Some((query.x, query.y)),
        query.max_depth,
    )
    .await?;
    if point_snapshot_looks_like_local_widget_coordinates(&snapshot, query.x, query.y) {
        if let Ok(full_snapshot) =
            accessibility_snapshot(state, udid, None, query.max_depth.or(Some(4))).await
        {
            if let Ok(point_snapshot) =
                accessibility_point_snapshot(&full_snapshot, query.x, query.y)
            {
                return Ok(json(point_snapshot));
            }
        }
    }
    Ok(json(snapshot))
}

include!("action_execution.rs");

include!("accessibility_query.rs");

async fn inspector_request(
    State(state): State<AppState>,
    Path(udid): Path<String>,
    Json(payload): Json<InspectorRequestPayload>,
) -> Result<Json<Value>, AppError> {
    let method = payload.method.trim();
    if !is_allowed_inspector_proxy_method(method) {
        return Err(AppError::bad_request(format!(
            "Unsupported inspector proxy method `{method}`."
        )));
    }

    let session = inspector_session_for_state(&state, &udid)
        .await
        .map_err(AppError::native)?;
    let result = query_inspector_session(
        &state,
        &session,
        method,
        payload.params.unwrap_or(Value::Null),
    )
    .await
    .map_err(AppError::native)?;

    Ok(json(json_value!({
        "result": result,
        "inspector": inspector_metadata(&session.info, &Value::Null, session.process_identifier, &session.transport),
    })))
}

fn is_allowed_inspector_proxy_method(method: &str) -> bool {
    matches!(
        method,
        "Inspector.getInfo"
            | "Runtime.ping"
            | "View.get"
            | "View.evaluateScript"
            | "View.getHierarchy"
            | "View.getProperties"
            | "View.setProperty"
            | "View.listActions"
            | "View.perform"
    )
}

fn inspector_request_timeout(method: &str) -> Duration {
    if method == "View.getHierarchy" {
        CONNECTED_INSPECTOR_HIERARCHY_TIMEOUT
    } else {
        Duration::from_secs(10)
    }
}

async fn simulator_logs(
    State(state): State<AppState>,
    Path(udid): Path<String>,
    Query(query): Query<LogsQuery>,
) -> Result<Json<Value>, AppError> {
    let limit = query.limit.unwrap_or(250).clamp(1, 1000);
    if android::is_android_id(&udid) {
        let entries = run_android_action(state, move |android| android.logs(&udid, limit)).await?;
        return Ok(json(json_value!({ "entries": entries })));
    }
    let filters = LogFilters::new(
        split_filter_values(query.levels.as_deref()),
        split_filter_values(query.processes.as_deref()),
        query.q.as_deref().unwrap_or("").trim().to_lowercase(),
    );
    let entries = if query.backfill.unwrap_or(false) {
        let seconds = query.seconds.unwrap_or(30.0).clamp(1.0, 1800.0);
        run_bridge_action(state.clone(), move |bridge| {
            bridge.recent_logs(&udid, seconds, limit, &filters)
        })
        .await?
    } else {
        state.logs.ensure_started(&udid).await?;
        state.logs.snapshot(&udid, &filters, limit).await
    };
    Ok(json(json_value!({
        "entries": entries,
    })))
}

#[derive(Clone, Copy)]
enum InAppHierarchySource {
    Automatic,
    UIKit,
}

#[derive(Clone)]
struct InspectorSession {
    transport: InspectorSessionTransport,
    available_sources: Vec<String>,
    info: Value,
    process_identifier: i64,
}

#[derive(Clone)]
enum InspectorSessionTransport {
    Connected,
    Tcp {
        port: u16,
    },
    RemoteService {
        server_url: String,
        access_token: String,
    },
}

async fn inspector_session_for_state(
    state: &AppState,
    udid: &str,
) -> Result<InspectorSession, String> {
    let frontmost_pid = inspector_frontmost_process_identifier(state, udid).await;
    let connected_error = match connected_inspector_session(state, udid, frontmost_pid).await {
        Ok(session) => return Ok(session),
        Err(error) => error,
    };
    let registry_error = match registry_inspector_session(state, udid, frontmost_pid).await {
        Ok(session) => return Ok(session),
        Err(error) => error,
    };

    match inspector_session(udid, frontmost_pid).await {
        Ok(session) => Ok(session),
        Err(tcp_error) => Err(format!("{connected_error} {registry_error} {tcp_error}")),
    }
}

async fn inspector_frontmost_process_identifier(state: &AppState, udid: &str) -> Option<i64> {
    if let Ok(Some(process_identifier)) = frontmost_process_identifier(state, udid).await {
        return Some(process_identifier);
    }

    foreground_app_for_simulator_with_cache_ttl(state, udid, INSPECTOR_FOREGROUND_APP_CACHE_TTL)
        .await
        .ok()
        .flatten()
        .map(|foreground| foreground.process_identifier)
}

async fn inspector_session_for_process(
    state: &AppState,
    udid: &str,
    process_identifier: i64,
) -> Result<InspectorSession, String> {
    let connected_error =
        match connected_inspector_session(state, udid, Some(process_identifier)).await {
            Ok(session) => return Ok(session),
            Err(error) => error,
        };

    match inspector_session(udid, Some(process_identifier)).await {
        Ok(session) => Ok(session),
        Err(tcp_error) => Err(format!("{connected_error} {tcp_error}")),
    }
}

async fn connected_inspector_session(
    state: &AppState,
    udid: &str,
    frontmost_pid: Option<i64>,
) -> Result<InspectorSession, String> {
    let mut probed_inspectors = Vec::new();
    let mut candidates = Vec::new();
    for inspector in state.inspectors.connected().await {
        if frontmost_pid.is_some_and(|pid| pid != inspector.process_identifier) {
            probed_inspectors.push(format!(
                "background process {}",
                inspector.process_identifier
            ));
            continue;
        }
        if inspector_process_belongs_to_udid(udid, inspector.process_identifier).await? {
            candidates.push(InspectorSession {
                transport: InspectorSessionTransport::Connected,
                available_sources: inspector_available_sources(&inspector.info),
                info: inspector.info,
                process_identifier: inspector.process_identifier,
            });
            continue;
        }

        probed_inspectors.push(format!("process {}", inspector.process_identifier));
    }

    if let Some(session) = best_inspector_session(candidates) {
        return Ok(session);
    }

    if probed_inspectors.is_empty() {
        Err(format!(
            "No connected WebSocket inspector found for simulator {udid}."
        ))
    } else {
        Err(format!(
            "No connected WebSocket inspector matched simulator {udid}. Found inspectors for {}.",
            probed_inspectors.join(", ")
        ))
    }
}

async fn registry_inspector_session(
    state: &AppState,
    udid: &str,
    frontmost_pid: Option<i64>,
) -> Result<InspectorSession, String> {
    let mut probed_inspectors = Vec::new();
    let mut candidates = Vec::new();
    for inspector in state.inspectors.published_inspectors().await {
        if frontmost_pid.is_some_and(|pid| pid != inspector.process_identifier) {
            probed_inspectors.push(format!(
                "background registry process {}",
                inspector.process_identifier
            ));
            continue;
        }
        if !inspector_process_belongs_to_udid(udid, inspector.process_identifier).await? {
            probed_inspectors.push(format!("registry process {}", inspector.process_identifier));
            continue;
        }
        let session = inspector_session_from_published(inspector);
        if query_inspector_session(state, &session, "Runtime.ping", Value::Null)
            .await
            .is_err()
        {
            probed_inspectors.push(format!(
                "unreachable registry process {}",
                session.process_identifier
            ));
            continue;
        }
        candidates.push(session);
    }

    if let Some(session) = best_inspector_session(candidates) {
        return Ok(session);
    }

    if probed_inspectors.is_empty() {
        Err(format!(
            "No published app inspector found for simulator {udid}."
        ))
    } else {
        Err(format!(
            "No published app inspector matched simulator {udid}. Found inspectors for {}.",
            probed_inspectors.join(", ")
        ))
    }
}

fn inspector_session_from_published(inspector: PublishedInspector) -> InspectorSession {
    let mut available_sources = inspector_available_sources(&inspector.info);
    for source in inspector.available_sources {
        if source == SOURCE_UIKIT && !inspector_info_allows_uikit(&inspector.info) {
            continue;
        }
        push_unique_source(&mut available_sources, &source);
    }
    InspectorSession {
        transport: InspectorSessionTransport::RemoteService {
            server_url: inspector.server_url,
            access_token: inspector.access_token,
        },
        available_sources,
        info: inspector.info,
        process_identifier: inspector.process_identifier,
    }
}

async fn inspector_session(
    udid: &str,
    frontmost_pid: Option<i64>,
) -> Result<InspectorSession, String> {
    let mut probed_inspectors = Vec::new();
    let mut probe_errors = Vec::new();

    if let Some(session) = find_inspector_session_on_ports(
        udid,
        frontmost_pid,
        inspector_agent_ports().collect(),
        &mut probed_inspectors,
        &mut probe_errors,
    )
    .await?
    {
        return Ok(session);
    }

    let discovered_ports = match discover_simulator_listener_ports(udid).await {
        Ok(ports) => ports
            .into_iter()
            .filter(|port| !inspector_agent_ports().contains(port))
            .collect::<Vec<_>>(),
        Err(error) => {
            probe_errors.push(format!("listener discovery: {error}"));
            Vec::new()
        }
    };

    if let Some(session) = find_inspector_session_on_ports(
        udid,
        frontmost_pid,
        discovered_ports,
        &mut probed_inspectors,
        &mut probe_errors,
    )
    .await?
    {
        return Ok(session);
    }

    if !probed_inspectors.is_empty() {
        return Err(format!(
            "No in-app inspector matched simulator {udid}. Found inspectors on {}.",
            probed_inspectors.join(", ")
        ));
    }

    let first_port = INSPECTOR_AGENT_DEFAULT_PORT;
    let last_port = inspector_agent_last_port();
    let detail = probe_errors
        .first()
        .map(|error| format!(" First probe error: {error}"))
        .unwrap_or_default();
    Err(format!(
        "No in-app inspector found for simulator {udid} on ports {first_port}-{last_port} or simulator-local listener ports.{detail}"
    ))
}

async fn find_inspector_session_on_ports(
    udid: &str,
    frontmost_pid: Option<i64>,
    ports: Vec<u16>,
    probed_inspectors: &mut Vec<String>,
    probe_errors: &mut Vec<String>,
) -> Result<Option<InspectorSession>, String> {
    let mut candidates = Vec::new();
    for port in ports {
        let info = match query_inspector_agent_on_port(port, "Inspector.getInfo", Value::Null).await
        {
            Ok(info) => info,
            Err(error) => {
                probe_errors.push(format!("{port}: {error}"));
                continue;
            }
        };

        let process_identifier = match info.get("processIdentifier").and_then(Value::as_i64) {
            Some(process_identifier) => process_identifier,
            None => {
                probe_errors.push(format!(
                    "{port}: Inspector agent did not report a process identifier."
                ));
                continue;
            }
        };

        if frontmost_pid.is_some_and(|pid| pid != process_identifier) {
            probed_inspectors.push(format!("{port}: background process {process_identifier}"));
            continue;
        }

        if inspector_process_belongs_to_udid(udid, process_identifier).await? {
            candidates.push(InspectorSession {
                transport: InspectorSessionTransport::Tcp { port },
                available_sources: inspector_available_sources(&info),
                info,
                process_identifier,
            });
            continue;
        }

        probed_inspectors.push(format!("{port}: process {process_identifier}"));
    }

    Ok(best_inspector_session(candidates))
}

fn best_inspector_session(mut candidates: Vec<InspectorSession>) -> Option<InspectorSession> {
    candidates.sort_by_key(inspector_session_score);
    candidates.into_iter().next()
}

fn inspector_session_score(session: &InspectorSession) -> u8 {
    if session
        .available_sources
        .iter()
        .any(|source| source == SOURCE_REACT_NATIVE)
    {
        return 0;
    }
    if session
        .available_sources
        .iter()
        .any(|source| source == SOURCE_FLUTTER)
    {
        return 1;
    }
    if session
        .available_sources
        .iter()
        .any(|source| source == SOURCE_NATIVE_SCRIPT)
    {
        return 2;
    }
    if session
        .available_sources
        .iter()
        .any(|source| source == SOURCE_SWIFTUI)
    {
        return 3;
    }
    if session
        .available_sources
        .iter()
        .any(|source| source == SOURCE_UIKIT)
    {
        return 4;
    }
    5
}

async fn frontmost_process_identifier(state: &AppState, udid: &str) -> Result<Option<i64>, String> {
    let snapshot = accessibility_snapshot(state.clone(), udid.to_owned(), None, Some(0))
        .await
        .map_err(|error| error.to_string())?;
    if let Some(process_identifier) = process_identifier_from_accessibility_snapshot(&snapshot) {
        return Ok(Some(process_identifier));
    }
    frontmost_process_identifier_from_points(state, udid).await
}

async fn frontmost_process_identifier_from_points(
    state: &AppState,
    udid: &str,
) -> Result<Option<i64>, String> {
    let probe_points = foreground_process_probe_points(state, udid);
    let mut last_error: Option<String> = None;
    for point in probe_points {
        match timeout(
            FOREGROUND_PROCESS_PROBE_TIMEOUT,
            accessibility_snapshot(state.clone(), udid.to_owned(), Some(point), Some(0)),
        )
        .await
        {
            Ok(Ok(snapshot)) => {
                if let Some(process_identifier) =
                    process_identifier_from_accessibility_snapshot(&snapshot)
                {
                    return Ok(Some(process_identifier));
                }
            }
            Ok(Err(error)) => last_error = Some(error.to_string()),
            Err(_) => {}
        }
    }
    if let Some(error) = last_error {
        Err(error)
    } else {
        Ok(None)
    }
}

fn foreground_process_probe_points(state: &AppState, udid: &str) -> Vec<(f64, f64)> {
    let (screen_width, screen_height) =
        simulator_logical_screen_size(state, udid).unwrap_or((402.0, 874.0));
    let center_x = (screen_width * 0.5).max(1.0);
    let center_y = (screen_height * 0.5).clamp(1.0, screen_height.max(1.0));
    let bottom_address_y = (screen_height - 54.0).clamp(1.0, screen_height.max(1.0));
    let bottom_title_y = (screen_height - 28.0).clamp(1.0, screen_height.max(1.0));
    vec![
        (center_x, bottom_address_y),
        (center_x, bottom_title_y),
        (center_x, 92.0_f64.min((screen_height * 0.18).max(1.0))),
        (center_x, center_y),
    ]
}

fn process_identifier_from_accessibility_snapshot(snapshot: &Value) -> Option<i64> {
    let roots = snapshot.get("roots").and_then(Value::as_array)?;
    roots
        .iter()
        .find_map(process_identifier_from_accessibility_node)
}

fn process_identifier_from_accessibility_node(node: &Value) -> Option<i64> {
    if let Some(process_identifier) = node.get("pid").and_then(Value::as_i64) {
        return Some(process_identifier);
    }
    node.get("children")
        .and_then(Value::as_array)
        .and_then(|children| {
            children
                .iter()
                .find_map(process_identifier_from_accessibility_node)
        })
}

async fn foreground_app_metadata(
    state: &AppState,
    udid: &str,
) -> Result<Option<devtools::ForegroundApp>, String> {
    let Some(process_identifier) = frontmost_process_identifier(state, udid).await? else {
        return Ok(None);
    };
    let command = process_command(process_identifier).await?;
    if command.contains(".appex/") || command.contains("WebContent") {
        return Ok(None);
    }
    let app_path = app_bundle_path_from_command(&command);
    let bundle_identifier = match app_path.as_deref() {
        Some(path) => app_bundle_identifier(path).await.ok().flatten(),
        None => None,
    };
    let app_name = app_path
        .as_deref()
        .and_then(|path| {
            std::path::Path::new(path)
                .file_stem()
                .and_then(|name| name.to_str())
                .map(ToOwned::to_owned)
        })
        .or_else(|| bundle_identifier.clone());
    Ok(Some(devtools::ForegroundApp {
        process_identifier,
        bundle_identifier,
        app_name,
    }))
}

async fn foreground_app_for_simulator(
    state: &AppState,
    udid: &str,
) -> Result<Option<devtools::ForegroundApp>, String> {
    foreground_app_for_simulator_with_cache_ttl(state, udid, FOREGROUND_APP_CACHE_TTL).await
}

async fn foreground_app_for_simulator_with_cache_ttl(
    state: &AppState,
    udid: &str,
    cache_ttl: Duration,
) -> Result<Option<devtools::ForegroundApp>, String> {
    if let Some(foreground) = cached_foreground_app_with_ttl(udid, cache_ttl) {
        return Ok(Some(foreground));
    }

    let mut last_error: Option<String> = None;

    // DevTools selection needs the app currently under the private display.
    // launchctl can leave recently active UIKit services marked as active, so
    // prefer the frontmost accessibility root when it is available.
    match foreground_app_metadata(state, udid).await {
        Ok(Some(foreground)) => {
            cache_foreground_app(udid, &foreground);
            return Ok(Some(foreground));
        }
        Ok(None) => {}
        Err(error) => last_error = Some(error),
    }

    match foreground_app_from_launchctl(udid).await {
        Ok(Some(foreground)) => {
            cache_foreground_app(udid, &foreground);
            Ok(Some(foreground))
        }
        Ok(None) => Ok(stale_cached_foreground_app(udid)),
        Err(error) => stale_cached_foreground_app(udid)
            .map(Some)
            .ok_or_else(|| last_error.unwrap_or(error)),
    }
}

fn cache_foreground_app(udid: &str, foreground_app: &devtools::ForegroundApp) {
    let cache = FOREGROUND_APP_CACHE.get_or_init(|| StdMutex::new(HashMap::new()));
    let Ok(mut cache) = cache.lock() else {
        return;
    };
    cache.insert(
        udid.to_owned(),
        CachedForegroundApp {
            cached_at: Instant::now(),
            foreground_app: foreground_app.clone(),
        },
    );
}

fn stale_cached_foreground_app(udid: &str) -> Option<devtools::ForegroundApp> {
    cached_foreground_app_with_ttl(udid, FOREGROUND_APP_STALE_TTL)
}

fn cached_foreground_app_with_ttl(udid: &str, ttl: Duration) -> Option<devtools::ForegroundApp> {
    let cache = FOREGROUND_APP_CACHE.get()?;
    let Ok(cache) = cache.lock() else {
        return None;
    };
    let cached = cache.get(udid)?;
    (cached.cached_at.elapsed() <= ttl).then(|| cached.foreground_app.clone())
}

#[derive(Clone, Debug)]
struct UIKitApplicationService {
    pid: i64,
    service_name: String,
}

#[derive(Clone, Debug)]
struct UIKitApplicationServiceDetails {
    active_count: u64,
    app_name: Option<String>,
    bundle_identifier: Option<String>,
    process_identifier: i64,
    spawn_role: String,
}

async fn foreground_app_from_launchctl(
    udid: &str,
) -> Result<Option<devtools::ForegroundApp>, String> {
    let deadline = Instant::now() + FOREGROUND_APP_ROUTE_TIMEOUT;
    let services = simulator_ui_application_services(udid).await?;
    let mut best: Option<UIKitApplicationServiceDetails> = None;
    let mut skipped_details = 0_u32;
    for service in &services {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            break;
        };
        let details_result =
            match timeout(remaining, ui_application_service_details(udid, service)).await {
                Ok(result) => result,
                Err(_) => {
                    skipped_details += 1;
                    break;
                }
            };
        let Some(details) = (match details_result {
            Ok(details) => details,
            Err(error) => {
                skipped_details += 1;
                tracing::debug!("Skipping UIKit application foreground candidate: {error}");
                None
            }
        }) else {
            continue;
        };
        let details_score = ui_application_foreground_score(&details);
        let best_score = best
            .as_ref()
            .map(ui_application_foreground_score)
            .unwrap_or((0, 0));
        if details_score > best_score {
            best = Some(details);
        }
        if best.as_ref().is_some_and(is_decisive_foreground_app) {
            break;
        }
    }

    if skipped_details > 0
        && best
            .as_ref()
            .is_some_and(|details| ui_application_foreground_score(details) < (2, 3))
    {
        return Ok(None);
    }

    Ok(best.map(|details| devtools::ForegroundApp {
        process_identifier: details.process_identifier,
        bundle_identifier: details.bundle_identifier,
        app_name: details.app_name,
    }))
}

fn is_decisive_foreground_app(details: &UIKitApplicationServiceDetails) -> bool {
    if ui_application_foreground_score(details).0 < 2 {
        return false;
    }
    let bundle_identifier = details.bundle_identifier.as_deref().unwrap_or_default();
    !bundle_identifier.contains("ViewService")
        && !bundle_identifier.contains("WidgetRenderer")
        && !bundle_identifier.contains(".appex")
}

async fn simulator_ui_application_services(
    udid: &str,
) -> Result<Vec<UIKitApplicationService>, String> {
    let output = timeout(
        Duration::from_secs(1),
        Command::new("xcrun")
            .args(["simctl", "spawn", udid, "launchctl", "print", "user/501"])
            .output(),
    )
    .await
    .map_err(|_| "Timed out listing simulator UIKit applications.".to_owned())?
    .map_err(|error| format!("Unable to list simulator UIKit applications: {error}"))?;
    if !output.status.success() {
        return Ok(Vec::new());
    }

    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(parse_ui_application_service_line)
        .collect())
}

fn parse_ui_application_service_line(line: &str) -> Option<UIKitApplicationService> {
    let trimmed = line.trim();
    if !trimmed.contains("UIKitApplication:") {
        return None;
    }
    let mut parts = trimmed.split_whitespace();
    let pid = parts.next()?.parse::<i64>().ok()?;
    let separator = parts.next()?;
    let service_name = parts.next()?.to_owned();
    if separator != "-" || pid <= 0 || !service_name.starts_with("UIKitApplication:") {
        return None;
    }
    Some(UIKitApplicationService { pid, service_name })
}

async fn ui_application_service_details(
    udid: &str,
    service: &UIKitApplicationService,
) -> Result<Option<UIKitApplicationServiceDetails>, String> {
    let output = timeout(
        Duration::from_secs(1),
        Command::new("xcrun")
            .args([
                "simctl",
                "spawn",
                udid,
                "launchctl",
                "print",
                &format!("user/501/{}", service.service_name),
            ])
            .output(),
    )
    .await
    .map_err(|_| "Timed out reading simulator UIKit application state.".to_owned())?
    .map_err(|error| format!("Unable to read simulator UIKit application state: {error}"))?;
    if !output.status.success() {
        return Ok(None);
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let active_count = launchctl_numeric_value(&text, "active count").unwrap_or(0);
    let process_identifier = launchctl_numeric_value(&text, "pid")
        .map(|pid| pid as i64)
        .unwrap_or(service.pid);
    if process_identifier <= 0
        || launchctl_value(&text, "state").is_none_or(|value| value != "running")
    {
        return Ok(None);
    }
    let spawn_role = launchctl_value(&text, "spawn role").unwrap_or_default();
    let program = launchctl_value(&text, "program");
    let bundle_identifier = launchctl_value(&text, "bundle id");
    let app_name = program
        .as_deref()
        .and_then(app_bundle_path_from_command)
        .and_then(|path| {
            std::path::Path::new(&path)
                .file_stem()
                .and_then(|name| name.to_str())
                .map(ToOwned::to_owned)
        })
        .or_else(|| bundle_identifier.clone());

    Ok(Some(UIKitApplicationServiceDetails {
        active_count,
        app_name,
        bundle_identifier,
        process_identifier,
        spawn_role,
    }))
}

fn ui_application_foreground_score(details: &UIKitApplicationServiceDetails) -> (u8, u64) {
    let role_score = if details.spawn_role.contains("ui focal") {
        2
    } else if details.spawn_role.contains("ui") {
        1
    } else {
        0
    };
    (role_score, details.active_count)
}

fn launchctl_value(output: &str, key: &str) -> Option<String> {
    let prefix = format!("{key} = ");
    output.lines().find_map(|line| {
        let value = line.trim().strip_prefix(&prefix)?.trim();
        (!value.is_empty()).then_some(value.to_owned())
    })
}

fn launchctl_numeric_value(output: &str, key: &str) -> Option<u64> {
    launchctl_value(output, key)?.parse::<u64>().ok()
}

async fn process_command(pid: i64) -> Result<String, String> {
    let output = timeout(
        Duration::from_secs(1),
        Command::new("ps")
            .arg("-p")
            .arg(pid.to_string())
            .arg("-o")
            .arg("command=")
            .output(),
    )
    .await
    .map_err(|_| "Timed out reading process command.".to_owned())?
    .map_err(|error| format!("Unable to read process command: {error}"))?;

    if !output.status.success() {
        return Err(format!("Process {pid} is not running."));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn app_bundle_path_from_command(command: &str) -> Option<String> {
    let command = command.trim();
    let app_marker = ".app/";
    let end = command.find(app_marker)? + ".app".len();
    let start = command[..end].find('/').unwrap_or(0);
    Some(command[start..end].to_owned())
}

async fn app_bundle_identifier(app_path: &str) -> Result<Option<String>, String> {
    let plist_path = std::path::Path::new(app_path).join("Info.plist");
    let output = timeout(
        Duration::from_secs(1),
        Command::new("plutil")
            .args(["-extract", "CFBundleIdentifier", "raw", "-o", "-"])
            .arg(&plist_path)
            .output(),
    )
    .await
    .map_err(|_| "Timed out reading app bundle identifier.".to_owned())?
    .map_err(|error| format!("Unable to read app bundle identifier: {error}"))?;

    if !output.status.success() {
        return Ok(None);
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    Ok((!value.is_empty()).then_some(value))
}

async fn run_in_app_inspector_hierarchy(
    state: &AppState,
    session: &InspectorSession,
    source: InAppHierarchySource,
    max_depth: Option<usize>,
    include_hidden: bool,
    interactive_only: bool,
) -> Result<Value, String> {
    let max_depth = max_depth.unwrap_or(80);
    let params = match source {
        InAppHierarchySource::Automatic => json_value!({
            "includeHidden": include_hidden,
            "maxDepth": max_depth,
            "interactiveOnly": interactive_only,
        }),
        InAppHierarchySource::UIKit => json_value!({
            "includeHidden": include_hidden,
            "maxDepth": max_depth,
            "interactiveOnly": interactive_only,
            "source": "uikit",
        }),
    };
    let hierarchy = query_inspector_session(state, session, "View.getHierarchy", params).await?;
    let source = hierarchy
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or(SOURCE_UIKIT);
    if framework_source(source) {
        return Ok(json_value!({
            "roots": hierarchy.get("roots").cloned().unwrap_or_else(|| Value::Array(Vec::new())),
            "source": source,
            "inspector": inspector_metadata(&session.info, &hierarchy, session.process_identifier, &session.transport),
        }));
    }

    let roots = hierarchy
        .get("roots")
        .and_then(Value::as_array)
        .ok_or_else(|| "Inspector agent hierarchy response did not include roots.".to_owned())?
        .iter()
        .map(|node| normalize_inspector_node(node, Some(session.process_identifier)))
        .collect::<Vec<_>>();

    Ok(json_value!({
        "roots": roots,
        "source": SOURCE_UIKIT,
        "inspector": inspector_metadata(&session.info, &hierarchy, session.process_identifier, &session.transport),
    }))
}

async fn query_inspector_session(
    state: &AppState,
    session: &InspectorSession,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    match &session.transport {
        InspectorSessionTransport::Connected => {
            let wait = inspector_request_timeout(method);
            state
                .inspectors
                .query_with_timeout(session.process_identifier, method, params, wait)
                .await
        }
        InspectorSessionTransport::Tcp { port } => {
            query_inspector_agent_on_port(*port, method, params).await
        }
        InspectorSessionTransport::RemoteService {
            server_url,
            access_token,
        } => {
            query_remote_service_inspector(
                server_url,
                access_token,
                session.process_identifier,
                method,
                params,
            )
            .await
        }
    }
}

fn inspector_available_sources(info: &Value) -> Vec<String> {
    let mut sources = Vec::new();
    let snapshot_source = info.get("source").and_then(Value::as_str).unwrap_or("");
    let react_native_available = info
        .get("reactNative")
        .and_then(|value| value.get("available"))
        .and_then(Value::as_bool)
        .unwrap_or(snapshot_source == SOURCE_REACT_NATIVE);
    if react_native_available {
        sources.push(SOURCE_REACT_NATIVE.to_owned());
    }
    let flutter_available = info
        .get("flutter")
        .and_then(|value| value.get("available"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if flutter_available {
        sources.push(SOURCE_FLUTTER.to_owned());
    }
    match snapshot_source {
        SOURCE_NATIVE_SCRIPT => push_unique_source(&mut sources, SOURCE_NATIVE_SCRIPT),
        SOURCE_SWIFTUI => push_unique_source(&mut sources, SOURCE_SWIFTUI),
        _ => {}
    }
    let app_hierarchy = info.get("appHierarchy");
    let app_hierarchy_available = app_hierarchy
        .and_then(|value| value.get("available"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let app_hierarchy_source = app_hierarchy
        .and_then(|value| value.get("source"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if app_hierarchy_available {
        match app_hierarchy_source {
            SOURCE_NATIVE_SCRIPT => push_unique_source(&mut sources, SOURCE_NATIVE_SCRIPT),
            SOURCE_REACT_NATIVE => push_unique_source(&mut sources, SOURCE_REACT_NATIVE),
            SOURCE_FLUTTER => push_unique_source(&mut sources, SOURCE_FLUTTER),
            SOURCE_SWIFTUI => push_unique_source(&mut sources, SOURCE_SWIFTUI),
            _ => {}
        }
    }
    let uikit_available = info
        .get("uikit")
        .and_then(|value| value.get("available"))
        .and_then(Value::as_bool)
        .unwrap_or_else(|| {
            !(react_native_available
                || flutter_available
                || snapshot_source == SOURCE_NATIVE_SCRIPT
                || app_hierarchy_source == SOURCE_REACT_NATIVE
                || app_hierarchy_source == SOURCE_FLUTTER
                || snapshot_source == SOURCE_REACT_NATIVE
                || snapshot_source == SOURCE_FLUTTER)
        });
    if uikit_available {
        sources.push(SOURCE_UIKIT.to_owned());
    }
    sources
}

fn inspector_info_allows_uikit(info: &Value) -> bool {
    info.get("uikit")
        .and_then(|value| value.get("available"))
        .and_then(Value::as_bool)
        .unwrap_or_else(|| {
            let app_hierarchy_source = info
                .get("appHierarchy")
                .and_then(|value| value.get("source"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let snapshot_source = info.get("source").and_then(Value::as_str).unwrap_or("");
            let react_native_available = info
                .get("reactNative")
                .and_then(|value| value.get("available"))
                .and_then(Value::as_bool)
                .unwrap_or(snapshot_source == SOURCE_REACT_NATIVE);
            let flutter_available = info
                .get("flutter")
                .and_then(|value| value.get("available"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            !(react_native_available
                || flutter_available
                || snapshot_source == SOURCE_NATIVE_SCRIPT
                || app_hierarchy_source == SOURCE_REACT_NATIVE
                || app_hierarchy_source == SOURCE_FLUTTER
                || snapshot_source == SOURCE_REACT_NATIVE
                || snapshot_source == SOURCE_FLUTTER)
        })
}

fn push_unique_source(sources: &mut Vec<String>, source: &str) {
    if !sources.iter().any(|value| value == source) {
        sources.push(source.to_owned());
    }
}

fn available_sources_for_session(session: &InspectorSession) -> Vec<String> {
    let mut sources = Vec::new();
    if session
        .available_sources
        .iter()
        .any(|source| source == SOURCE_NATIVE_SCRIPT)
    {
        push_unique_source(&mut sources, SOURCE_NATIVE_SCRIPT);
    }
    if session
        .available_sources
        .iter()
        .any(|source| source == SOURCE_REACT_NATIVE)
    {
        push_unique_source(&mut sources, SOURCE_REACT_NATIVE);
    }
    if session
        .available_sources
        .iter()
        .any(|source| source == SOURCE_FLUTTER)
    {
        push_unique_source(&mut sources, SOURCE_FLUTTER);
    }
    if session
        .available_sources
        .iter()
        .any(|source| source == SOURCE_SWIFTUI)
    {
        push_unique_source(&mut sources, SOURCE_SWIFTUI);
    }
    if session
        .available_sources
        .iter()
        .any(|source| source == SOURCE_UIKIT)
    {
        push_unique_source(&mut sources, SOURCE_UIKIT);
    }
    sources
}

fn available_sources_with_native_ax(session: Option<&InspectorSession>) -> Vec<String> {
    let mut sources = session
        .map(available_sources_for_session)
        .unwrap_or_default();
    push_unique_source(&mut sources, SOURCE_NATIVE_AX);
    sources
}

fn available_sources_for_snapshot(base_sources: &[String], snapshot: &Value) -> Vec<String> {
    let mut sources = base_sources.to_owned();
    let Some(source) = snapshot.get("source").and_then(Value::as_str) else {
        return sources;
    };
    if source == SOURCE_REACT_NATIVE || source == SOURCE_FLUTTER {
        sources.retain(|candidate| candidate != SOURCE_UIKIT);
    }
    if framework_source(source) && !sources.iter().any(|value| value == source) {
        sources.insert(0, source.to_owned());
    }
    if source == SOURCE_UIKIT && !sources.iter().any(|value| value == SOURCE_UIKIT) {
        let insert_at = usize::from(
            sources
                .first()
                .map(|value| framework_source(value))
                .unwrap_or(false),
        );
        sources.insert(insert_at, SOURCE_UIKIT.to_owned());
    }
    sources
}

fn framework_source(source: &str) -> bool {
    source == SOURCE_NATIVE_SCRIPT
        || source == SOURCE_REACT_NATIVE
        || source == SOURCE_FLUTTER
        || source == SOURCE_SWIFTUI
}

fn attach_available_sources(snapshot: Value, available_sources: &[String]) -> Value {
    attach_tree_metadata(snapshot, available_sources, None)
}

fn trim_tree_depth(mut snapshot: Value, max_depth: Option<usize>) -> Value {
    let Some(max_depth) = max_depth else {
        return snapshot;
    };
    if let Some(roots) = snapshot.get_mut("roots").and_then(Value::as_array_mut) {
        for root in roots {
            trim_node_depth(root, 0, max_depth);
        }
    }
    snapshot
}

fn trim_node_depth(node: &mut Value, depth: usize, max_depth: usize) {
    let Some(object) = node.as_object_mut() else {
        return;
    };
    if depth >= max_depth {
        object.insert("children".to_owned(), Value::Array(Vec::new()));
        return;
    }
    if let Some(children) = object.get_mut("children").and_then(Value::as_array_mut) {
        for child in children {
            trim_node_depth(child, depth + 1, max_depth);
        }
    }
}

fn empty_accessibility_tree(
    source: &str,
    available_sources: &[String],
    fallback_reason: Option<String>,
) -> Value {
    attach_tree_metadata(
        json_value!({
            "roots": [],
            "source": source,
        }),
        available_sources,
        fallback_reason,
    )
}

fn suppress_native_ax_translation_error(message: &str) -> Option<String> {
    if message.contains("No translation object returned for simulator")
        || is_core_simulator_service_mismatch(message)
    {
        return None;
    }
    Some(message.to_owned())
}

fn is_transient_native_ax_snapshot_error(message: &str) -> bool {
    message.contains("No application accessibility root returned for simulator")
        || message.contains("No translation object returned for simulator")
        || is_core_simulator_service_mismatch(message)
}

fn is_core_simulator_service_mismatch(message: &str) -> bool {
    message.contains("CoreSimulator.framework was changed while the process was running")
        || message.contains("Service version")
            && message.contains("does not match expected service version")
}

fn attach_tree_metadata(
    mut snapshot: Value,
    available_sources: &[String],
    fallback_reason: Option<String>,
) -> Value {
    if let Value::Object(ref mut object) = snapshot {
        object.insert(
            "availableSources".to_owned(),
            Value::Array(
                available_sources
                    .iter()
                    .map(|source| Value::String(source.clone()))
                    .collect(),
            ),
        );
        if let Some(reason) = fallback_reason {
            object.insert("fallbackReason".to_owned(), Value::String(reason));
            if object.get("source").and_then(Value::as_str) == Some(SOURCE_NATIVE_AX) {
                object.insert(
                    "fallbackSource".to_owned(),
                    Value::String(SOURCE_NATIVE_AX.to_owned()),
                );
            }
        }
    }
    snapshot
}

fn inspector_metadata(
    info: &Value,
    hierarchy: &Value,
    process_identifier: i64,
    transport: &InspectorSessionTransport,
) -> Value {
    let (transport_name, port, service_url) = match transport {
        InspectorSessionTransport::Connected => ("websocket", Value::Null, Value::Null),
        InspectorSessionTransport::Tcp { port } => {
            ("tcp+ndjson", Value::Number((*port).into()), Value::Null)
        }
        InspectorSessionTransport::RemoteService { server_url, .. } => (
            "remote-websocket",
            Value::Null,
            Value::String(server_url.clone()),
        ),
    };
    json_value!({
        "bundleIdentifier": info.get("bundleIdentifier").cloned().unwrap_or(Value::Null),
        "bundleName": info.get("bundleName").cloned().unwrap_or(Value::Null),
        "coordinateSpace": hierarchy.get("coordinateSpace").cloned().unwrap_or(Value::Null),
        "displayScale": hierarchy.get("displayScale").cloned().unwrap_or_else(|| info.get("displayScale").cloned().unwrap_or(Value::Null)),
        "serviceUrl": service_url,
        "host": INSPECTOR_AGENT_HOST,
        "port": port,
        "processIdentifier": process_identifier,
        "protocolVersion": info.get("protocolVersion").cloned().unwrap_or(Value::Null),
        "sourceRoot": hierarchy.get("sourceRoot").cloned().unwrap_or_else(|| info.get("sourceRoot").cloned().unwrap_or(Value::Null)),
        "transport": transport_name,
    })
}

fn inspector_agent_ports() -> std::ops::RangeInclusive<u16> {
    INSPECTOR_AGENT_DEFAULT_PORT..=inspector_agent_last_port()
}

fn inspector_agent_last_port() -> u16 {
    INSPECTOR_AGENT_DEFAULT_PORT.saturating_add(INSPECTOR_AGENT_PORT_SCAN_LIMIT)
}

async fn query_inspector_agent_on_port(
    port: u16,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let address = format!("{INSPECTOR_AGENT_HOST}:{port}");
    let mut stream = timeout(INSPECTOR_AGENT_TIMEOUT, TcpStream::connect(&address))
        .await
        .map_err(|_| format!("Timed out connecting to in-app inspector at {address}."))?
        .map_err(|error| format!("Unable to connect to in-app inspector at {address}: {error}"))?;

    let request = json_value!({
        "id": 1,
        "method": method,
        "params": params,
    });
    stream
        .write_all(request.to_string().as_bytes())
        .await
        .map_err(|error| format!("Unable to write inspector request: {error}"))?;
    stream
        .write_all(b"\n")
        .await
        .map_err(|error| format!("Unable to finish inspector request: {error}"))?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    for _ in 0..16 {
        line.clear();
        let byte_count = timeout(INSPECTOR_AGENT_TIMEOUT, reader.read_line(&mut line))
            .await
            .map_err(|_| format!("Timed out waiting for in-app inspector method {method}."))?
            .map_err(|error| format!("Unable to read inspector response: {error}"))?;
        if byte_count == 0 {
            break;
        }

        let value: Value = serde_json::from_str(line.trim()).map_err(|error| {
            format!("Inspector returned malformed JSON for method {method}: {error}")
        })?;
        if value.get("id").and_then(Value::as_i64) != Some(1) {
            continue;
        }
        if let Some(error) = value.get("error") {
            return Err(format!("Inspector method {method} failed: {error}"));
        }
        return value
            .get("result")
            .cloned()
            .ok_or_else(|| format!("Inspector method {method} did not include a result."));
    }

    Err(format!(
        "Inspector connection closed before method {method} returned a response."
    ))
}

async fn query_remote_service_inspector(
    server_url: &str,
    access_token: &str,
    process_identifier: i64,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let endpoint = InspectorServiceEndpoint::parse(server_url)?;
    let body = json_value!({
        "processIdentifier": process_identifier,
        "method": method,
        "params": params,
    })
    .to_string();
    let request = format!(
        "POST /api/inspector/request HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\n{}: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        endpoint.host_header(),
        crate::auth::ACCESS_TOKEN_HEADER,
        access_token,
        body.len(),
        body
    );
    let mut stream = timeout(
        INSPECTOR_AGENT_TIMEOUT,
        TcpStream::connect((endpoint.host.as_str(), endpoint.port)),
    )
    .await
    .map_err(|_| format!("Timed out connecting to published SimDeck service at {server_url}."))?
    .map_err(|error| {
        format!("Unable to connect to published SimDeck service at {server_url}: {error}")
    })?;
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|error| format!("Unable to write published inspector request: {error}"))?;

    let mut response = Vec::new();
    timeout(INSPECTOR_AGENT_TIMEOUT, stream.read_to_end(&mut response))
        .await
        .map_err(|_| format!("Timed out waiting for published inspector method {method}."))?
        .map_err(|error| format!("Unable to read published inspector response: {error}"))?;
    let response = String::from_utf8_lossy(&response);
    let (headers, body) = response
        .split_once("\r\n\r\n")
        .ok_or_else(|| "Published inspector returned malformed HTTP.".to_owned())?;
    let status_line = headers.lines().next().unwrap_or_default();
    if !status_line.contains(" 2") {
        return Err(format!(
            "Published inspector method {method} failed with {status_line}: {body}"
        ));
    }
    let value: Value = serde_json::from_str(body.trim()).map_err(|error| {
        format!("Published inspector returned malformed JSON for method {method}: {error}")
    })?;
    value
        .get("result")
        .cloned()
        .ok_or_else(|| format!("Published inspector method {method} did not include a result."))
}

struct InspectorServiceEndpoint {
    host: String,
    port: u16,
}

impl InspectorServiceEndpoint {
    fn parse(server_url: &str) -> Result<Self, String> {
        let authority = server_url
            .trim()
            .strip_prefix("http://")
            .ok_or_else(|| format!("Published inspector URL must use http://: {server_url}"))?
            .split('/')
            .next()
            .unwrap_or_default();
        let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
            let (host, rest) = rest
                .split_once(']')
                .ok_or_else(|| format!("Published inspector URL has invalid host: {server_url}"))?;
            let port = rest
                .strip_prefix(':')
                .ok_or_else(|| format!("Published inspector URL is missing a port: {server_url}"))?
                .parse::<u16>()
                .map_err(|error| format!("Published inspector URL has invalid port: {error}"))?;
            (host.to_owned(), port)
        } else {
            let (host, port) = authority.rsplit_once(':').ok_or_else(|| {
                format!("Published inspector URL is missing a port: {server_url}")
            })?;
            let port = port
                .parse::<u16>()
                .map_err(|error| format!("Published inspector URL has invalid port: {error}"))?;
            (host.to_owned(), port)
        };
        if host.trim().is_empty() {
            return Err(format!(
                "Published inspector URL has empty host: {server_url}"
            ));
        }
        Ok(Self { host, port })
    }

    fn host_header(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
}

async fn inspector_process_belongs_to_udid(udid: &str, pid: i64) -> Result<bool, String> {
    Ok(process_command(pid)
        .await
        .is_ok_and(|command| command.contains(udid)))
}

async fn discover_simulator_listener_ports(udid: &str) -> Result<Vec<u16>, String> {
    let output = timeout(
        Duration::from_secs(2),
        Command::new("lsof")
            .arg("-nP")
            .arg("-iTCP")
            .arg("-sTCP:LISTEN")
            .output(),
    )
    .await
    .map_err(|_| "Timed out discovering simulator listener ports.".to_owned())?
    .map_err(|error| format!("Unable to discover simulator listener ports: {error}"))?;

    if !output.status.success() {
        return Err(format!(
            "`lsof` exited with status {} while discovering simulator listener ports.",
            output.status
        ));
    }

    let mut ports = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Some((pid, port)) = parse_lsof_tcp_listener(line) else {
            continue;
        };
        if inspector_process_belongs_to_udid(udid, pid).await? && !ports.contains(&port) {
            ports.push(port);
        }
    }
    Ok(ports)
}

fn parse_lsof_tcp_listener(line: &str) -> Option<(i64, u16)> {
    if !line.contains(" (LISTEN)") {
        return None;
    }

    let mut columns = line.split_whitespace();
    columns.next()?;
    let pid = columns.next()?.parse::<i64>().ok()?;
    let endpoint = line.split("TCP ").nth(1)?.split(" (LISTEN)").next()?;
    let port = endpoint.rsplit(':').next()?.parse::<u16>().ok()?;
    Some((pid, port))
}

fn normalize_inspector_node(node: &Value, pid: Option<i64>) -> Value {
    let Some(object) = node.as_object() else {
        return json_value!({
            "type": "View",
            "title": "Invalid inspector node",
            "children": [],
        });
    };

    let accessibility = object.get("accessibility").and_then(Value::as_object);
    let swiftui = object.get("swiftUI").and_then(Value::as_object);
    let view_controller = object.get("viewController").and_then(Value::as_object);
    let control = object.get("control").and_then(Value::as_object);
    let children = object
        .get("children")
        .and_then(Value::as_array)
        .map(|children| {
            children
                .iter()
                .map(|child| normalize_inspector_node(child, pid))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let class_name = object_string(object, "className").unwrap_or_else(|| "UIView".to_owned());
    let display_name = object_string(object, "displayName")
        .or_else(|| object_string(object, "type"))
        .unwrap_or_else(|| class_name.clone());
    let source = object_string(object, "source").unwrap_or_else(|| "in-app-inspector".to_owned());
    let inspector_id = object_string(object, "id");
    let accessibility_label =
        object_string(object, "AXLabel").or_else(|| nested_string(accessibility, "label"));
    let text = object_string(object, "text");
    let placeholder = object_string(object, "placeholder");
    let swiftui_tag = nested_string(swiftui, "tag");
    let view_controller_title = nested_string(view_controller, "title");
    let image_name = object_string(object, "imageName").filter(|_| {
        source != "nativescript" || display_name.to_ascii_lowercase().contains("image")
    });
    let object_title = object_string(object, "title").filter(|title| title != &display_name);
    let title = first_non_empty_string([
        swiftui_tag.clone(),
        object_title,
        text.clone(),
        view_controller_title,
        image_name.clone(),
        Some(display_name.clone()),
    ]);
    let role = if nested_bool(swiftui, "isProbe").unwrap_or(false) {
        "SwiftUI Probe"
    } else if nested_bool(swiftui, "isHost").unwrap_or(false) {
        "SwiftUI Host"
    } else {
        "UIKit View"
    };
    let custom_actions = control
        .and_then(|control| control.get("actions"))
        .and_then(Value::as_array)
        .map(|actions| {
            actions
                .iter()
                .filter_map(Value::as_str)
                .map(|action| Value::String(action.to_owned()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let mut normalized = Map::new();
    normalized.insert("type".to_owned(), Value::String(display_name.clone()));
    normalized.insert("className".to_owned(), Value::String(class_name.clone()));
    normalized.insert("role".to_owned(), Value::String(role.to_owned()));
    normalized.insert("title".to_owned(), Value::String(title));
    normalized.insert("children".to_owned(), Value::Array(children));
    normalized.insert("source".to_owned(), Value::String(source));

    if let Some(value) = inspector_id {
        normalized.insert("AXUniqueId".to_owned(), Value::String(value.clone()));
        normalized.insert("inspectorId".to_owned(), Value::String(value));
    }
    if let Some(value) =
        nested_string(accessibility, "identifier").or_else(|| nested_string(swiftui, "tagId"))
    {
        normalized.insert("AXIdentifier".to_owned(), Value::String(value));
    }
    if let Some(value) = accessibility_label.or(text.clone()) {
        normalized.insert("AXLabel".to_owned(), Value::String(value));
    }
    if let Some(value) = object_string(object, "AXValue")
        .or_else(|| nested_string(accessibility, "value"))
        .or(placeholder.clone())
    {
        normalized.insert("AXValue".to_owned(), Value::String(value));
    }
    if let Some(value) =
        object_string(object, "help").or_else(|| nested_string(accessibility, "hint"))
    {
        normalized.insert("help".to_owned(), Value::String(value));
    }
    if let Some(frame) = object
        .get("frameInScreen")
        .or_else(|| object.get("frame"))
        .cloned()
    {
        normalized.insert("frame".to_owned(), frame);
    }
    if let Some(pid) = pid {
        normalized.insert("pid".to_owned(), Value::Number(pid.into()));
    }
    if !custom_actions.is_empty() {
        normalized.insert("custom_actions".to_owned(), Value::Array(custom_actions));
    }

    copy_fields(
        object,
        &mut normalized,
        &[
            "alpha",
            "backgroundColor",
            "bounds",
            "center",
            "clipsToBounds",
            "debugDescription",
            "frameInScreen",
            "isHidden",
            "isOpaque",
            "isUserInteractionEnabled",
            "moduleName",
            "tintColor",
            "transform",
        ],
    );
    copy_optional_field(object, &mut normalized, "swiftUI");
    copy_optional_field(object, &mut normalized, "viewController");
    copy_optional_field(object, &mut normalized, "scroll");
    copy_optional_field(object, &mut normalized, "control");
    copy_optional_field(object, &mut normalized, "nativeScript");
    copy_optional_field(object, &mut normalized, "uikitScript");
    copy_optional_field(object, &mut normalized, "sourceLocation");
    copy_optional_field(object, &mut normalized, "sourceLocations");
    copy_optional_field(object, &mut normalized, "sourceFile");
    copy_optional_field(object, &mut normalized, "sourceLine");
    copy_optional_field(object, &mut normalized, "sourceColumn");
    copy_optional_field(object, &mut normalized, "text");
    copy_optional_field(object, &mut normalized, "placeholder");
    if let Some(image_name) = image_name {
        normalized.insert("imageName".to_owned(), Value::String(image_name));
    }

    if let Some(enabled) = view_enabled(object) {
        normalized.insert("enabled".to_owned(), Value::Bool(enabled));
    }

    Value::Object(normalized)
}

fn view_enabled(object: &Map<String, Value>) -> Option<bool> {
    let user_interaction = object.get("isUserInteractionEnabled")?.as_bool()?;
    let hidden = object
        .get("isHidden")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let alpha = object.get("alpha").and_then(Value::as_f64).unwrap_or(1.0);
    Some(user_interaction && !hidden && alpha > 0.01)
}

fn copy_fields(source: &Map<String, Value>, target: &mut Map<String, Value>, fields: &[&str]) {
    for field in fields {
        copy_optional_field(source, target, field);
    }
}

fn copy_optional_field(source: &Map<String, Value>, target: &mut Map<String, Value>, field: &str) {
    if let Some(value) = source.get(field) {
        target.insert(field.to_owned(), value.clone());
    }
}

fn object_string(object: &Map<String, Value>, key: &str) -> Option<String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn nested_string(object: Option<&Map<String, Value>>, key: &str) -> Option<String> {
    object
        .and_then(|object| object.get(key))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn nested_bool(object: Option<&Map<String, Value>>, key: &str) -> Option<bool> {
    object
        .and_then(|object| object.get(key))
        .and_then(Value::as_bool)
}

fn first_non_empty_string(values: impl IntoIterator<Item = Option<String>>) -> String {
    values
        .into_iter()
        .flatten()
        .map(|value| value.trim().to_owned())
        .find(|value| !value.is_empty())
        .unwrap_or_default()
}

fn trimmed_optional_string(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn split_filter_values(value: Option<&str>) -> Vec<String> {
    value
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| part.to_lowercase())
        .collect()
}

async fn run_bridge_action<F, T>(state: AppState, action: F) -> Result<T, AppError>
where
    F: FnOnce(NativeBridge) -> Result<T, AppError> + Send + 'static,
    T: Send + 'static,
{
    let bridge = state.registry.bridge().clone();
    task::spawn_blocking(move || action(bridge))
        .await
        .map_err(|error| {
            AppError::internal(format!("Failed to join native bridge task: {error}"))
        })?
}

async fn run_android_action<F, T>(state: AppState, action: F) -> Result<T, AppError>
where
    F: FnOnce(AndroidBridge) -> Result<T, AppError> + Send + 'static,
    T: Send + 'static,
{
    let android = state.android.clone();
    task::spawn_blocking(move || action(android))
        .await
        .map_err(|error| {
            AppError::internal(format!("Failed to join Android bridge task: {error}"))
        })?
}

async fn all_device_values(state: AppState, force_refresh: bool) -> Result<Vec<Value>, AppError> {
    let ios = list_simulators_cached(state.clone(), force_refresh).await?;
    let mut values = state.registry.enrich_simulators(ios);
    let android_devices =
        run_android_action(state.clone(), |android| android.list_devices()).await?;
    values.extend(state.android.enrich_devices(android_devices));
    Ok(booted_first(values))
}

fn booted_first(values: Vec<Value>) -> Vec<Value> {
    let mut indexed_values = values.into_iter().enumerate().collect::<Vec<_>>();
    indexed_values.sort_by(|(left_index, left), (right_index, right)| {
        let left_booted = left
            .get("isBooted")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let right_booted = right
            .get("isBooted")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        right_booted
            .cmp(&left_booted)
            .then_with(|| left_index.cmp(right_index))
    });
    indexed_values.into_iter().map(|(_, value)| value).collect()
}

async fn list_simulators_cached(
    state: AppState,
    force_refresh: bool,
) -> Result<Vec<crate::native::bridge::Simulator>, AppError> {
    {
        let guard = state.simulator_inventory.inner.lock().await;
        if !force_refresh {
            if let (Some(simulators), Some(updated_at)) = (&guard.simulators, guard.updated_at) {
                if updated_at.elapsed() <= SIMULATOR_INVENTORY_CACHE_TTL {
                    return Ok(simulators.clone());
                }
            }
        }
    }

    let inventory_timeout = if force_refresh {
        SIMULATOR_INVENTORY_FORCE_REFRESH_TIMEOUT
    } else {
        SIMULATOR_INVENTORY_TIMEOUT
    };

    let simulators = match timeout(
        inventory_timeout,
        run_bridge_action(state.clone(), |bridge| bridge.list_simulators()),
    )
    .await
    {
        Ok(result) => result?,
        Err(_) => {
            tracing::warn!(
                timeout_seconds = inventory_timeout.as_secs(),
                force_refresh,
                "Timed out listing iOS simulators; returning cached inventory."
            );
            let guard = state.simulator_inventory.inner.lock().await;
            return Ok(guard.simulators.clone().unwrap_or_default());
        }
    };

    let mut guard = state.simulator_inventory.inner.lock().await;
    guard.simulators = Some(simulators.clone());
    guard.updated_at = Some(Instant::now());
    Ok(simulators)
}

async fn android_simulator_payload(state: AppState, udid: String) -> Result<Json<Value>, AppError> {
    let android_devices =
        run_android_action(state.clone(), |android| android.list_devices()).await?;
    let simulator = state
        .android
        .enrich_devices(android_devices)
        .into_iter()
        .find(|entry| entry.get("udid").and_then(Value::as_str) == Some(udid.as_str()))
        .ok_or_else(|| AppError::not_found(format!("Unknown Android emulator {udid}")))?;
    Ok(json(json_value!({ "simulator": simulator })))
}

async fn simulator_payload(state: AppState, udid: String) -> Result<Json<Value>, AppError> {
    if android::is_android_id(&udid) {
        return android_simulator_payload(state, udid).await;
    }
    let enriched = all_device_values(state.clone(), true).await?;
    let simulator = enriched
        .into_iter()
        .find(|entry| entry.get("udid").and_then(Value::as_str) == Some(udid.as_str()))
        .ok_or_else(|| AppError::not_found(format!("Unknown simulator {udid}")))?;
    Ok(json(json_value!({ "simulator": simulator })))
}

async fn accessibility_snapshot(
    state: AppState,
    udid: String,
    point: Option<(f64, f64)>,
    max_depth: Option<usize>,
) -> Result<Value, AppError> {
    accessibility_snapshot_with_retries(state, udid, point, max_depth, false).await
}

async fn accessibility_snapshot_with_retries(
    state: AppState,
    udid: String,
    point: Option<(f64, f64)>,
    max_depth: Option<usize>,
    interactive_only: bool,
) -> Result<Value, AppError> {
    let attempts = if point.is_none() {
        NATIVE_AX_SNAPSHOT_RETRY_ATTEMPTS
    } else {
        1
    };
    for attempt in 0..attempts {
        match accessibility_snapshot_with_options(
            state.clone(),
            udid.clone(),
            point,
            max_depth,
            interactive_only,
        )
        .await
        {
            Ok(snapshot) => return Ok(snapshot),
            Err(error) => {
                let message = error.to_string();
                if attempt + 1 >= attempts || !is_transient_native_ax_snapshot_error(&message) {
                    return Err(error);
                }
                tokio::time::sleep(NATIVE_AX_SNAPSHOT_RETRY_DELAY).await;
            }
        }
    }
    unreachable!("native AX snapshot retry loop always returns")
}

async fn accessibility_snapshot_with_options(
    state: AppState,
    udid: String,
    point: Option<(f64, f64)>,
    max_depth: Option<usize>,
    interactive_only: bool,
) -> Result<Value, AppError> {
    let bridge = state.registry.bridge().clone();
    let metrics = state.metrics.clone();
    let started = Instant::now();
    let task = task::spawn_blocking(move || {
        bridge.accessibility_snapshot_with_options(&udid, point, max_depth, interactive_only)
    });
    let result = match timeout(NATIVE_AX_SNAPSHOT_TIMEOUT, task).await {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => Err(AppError::internal(format!(
            "Failed to join native accessibility snapshot task: {error}"
        ))),
        Err(_) => Err(AppError::native(format!(
            "Native accessibility snapshot timed out after {}ms.",
            NATIVE_AX_SNAPSHOT_TIMEOUT.as_millis()
        ))),
    };
    let duration_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
    metrics.record_accessibility_snapshot(
        duration_ms,
        result.is_ok(),
        duration_ms >= NATIVE_AX_SNAPSHOT_TIMEOUT.as_millis() as u64,
    );
    result
}

#[cfg(test)]
mod tests {
    use super::{
        accessibility_point_snapshot, attach_tree_metadata, available_sources_for_snapshot,
        available_sources_with_native_ax, best_inspector_session,
        chrome_devtools_source_for_session, client_stats_foreground,
        compact_accessibility_snapshot, element_matches_selector, first_matching_element,
        inspector_available_sources, inspector_metadata, inspector_session_from_published,
        inspector_session_score, is_inspector_agent_transport_path,
        is_transient_native_ax_snapshot_error, logical_screen_size_from_display_pixels,
        normalize_inspector_node, normalize_screen_point_from_snapshot,
        normalized_gesture_coordinates, parse_lsof_tcp_listener, parse_ui_application_service_line,
        process_identifier_from_accessibility_snapshot, resolved_stream_quality_limits,
        scroll_input_plan_for_udid, split_filter_values, stream_quality_profile,
        suppress_native_ax_translation_error, tap_point_from_snapshot, trim_tree_depth,
        ui_application_foreground_score, AccessibilitySnapshotCache, AccessibilitySnapshotCacheKey,
        AccessibilitySource, BatchStep, ElementSelectorPayload, InspectorSession,
        InspectorSessionTransport, ScrollInputBackend, ScrollUntilVisiblePayload,
        StreamClientForegroundRegistry, StreamQualityLimits, StreamQualityPayload,
        UIKitApplicationServiceDetails, SOURCE_FLUTTER, SOURCE_NATIVE_AX, SOURCE_NATIVE_SCRIPT,
        SOURCE_REACT_NATIVE, SOURCE_SWIFTUI, SOURCE_UIKIT,
    };
    use crate::inspector::PublishedInspector;
    use crate::metrics::counters::ClientStreamStats;
    use serde_json::{json, Value};

    fn selector() -> ElementSelectorPayload {
        ElementSelectorPayload {
            text: None,
            id: Some("continue-button".to_owned()),
            label: Some("Continue".to_owned()),
            value: None,
            element_type: Some("Button".to_owned()),
            index: None,
            enabled: None,
            checked: None,
            focused: None,
            selected: None,
            regex: None,
        }
    }

    fn accessibility_snapshot() -> Value {
        json!({
            "roots": [{
                "type": "Window",
                "frame": { "x": 0.0, "y": 0.0, "width": 400.0, "height": 800.0 },
                "children": [{
                    "type": "Button",
                    "AXIdentifier": "continue-button",
                    "AXLabel": "Continue",
                    "frame": { "x": 100.0, "y": 200.0, "width": 80.0, "height": 40.0 },
                    "children": []
                }]
            }]
        })
    }

    #[test]
    fn accessibility_snapshot_pid_search_reads_nested_point_results() {
        let snapshot = json!({
            "roots": [{
                "type": "Window",
                "children": [{
                    "type": "TextField",
                    "pid": 24218,
                    "children": []
                }]
            }]
        });

        assert_eq!(
            process_identifier_from_accessibility_snapshot(&snapshot),
            Some(24218)
        );
    }

    #[test]
    fn logical_screen_size_infers_simulator_point_scale() {
        assert_eq!(
            logical_screen_size_from_display_pixels(1206.0, 2622.0),
            Some((402.0, 874.0))
        );
        assert_eq!(
            logical_screen_size_from_display_pixels(750.0, 1334.0),
            Some((375.0, 667.0))
        );
        assert_eq!(
            logical_screen_size_from_display_pixels(1668.0, 2388.0),
            Some((834.0, 1194.0))
        );
    }

    #[test]
    fn stream_client_foreground_remove_pauses_when_last_visible_client_leaves() {
        let registry = StreamClientForegroundRegistry::default();

        assert_eq!(registry.record("udid", "visible", true), (true, true));
        assert_eq!(registry.record("udid", "hidden", false), (true, false));
        assert_eq!(registry.remove("udid", "visible"), (false, true));
        assert_eq!(registry.remove("udid", "hidden"), (false, false));
    }

    #[test]
    fn client_stats_foreground_uses_page_visibility() {
        let page_stats =
            |visibility_state: Option<&str>, focused: Option<bool>| ClientStreamStats {
                client_id: "client".to_owned(),
                kind: "page".to_owned(),
                visibility_state: visibility_state.map(ToOwned::to_owned),
                focused,
                ..Default::default()
            };

        assert_eq!(
            client_stats_foreground(&page_stats(Some("visible"), Some(true))),
            Some(true)
        );
        assert_eq!(
            client_stats_foreground(&page_stats(Some("visible"), Some(false))),
            Some(true)
        );
        assert_eq!(
            client_stats_foreground(&page_stats(Some("hidden"), Some(true))),
            Some(false)
        );
        assert_eq!(
            client_stats_foreground(&page_stats(Some("visible"), None)),
            Some(true)
        );
        assert_eq!(client_stats_foreground(&page_stats(None, Some(true))), None);
        assert_eq!(
            client_stats_foreground(&ClientStreamStats {
                client_id: "client".to_owned(),
                kind: "webrtc".to_owned(),
                focused: Some(true),
                visibility_state: Some("visible".to_owned()),
                ..Default::default()
            }),
            None
        );
    }

    #[test]
    fn selector_matching_uses_identifier_label_and_type_aliases() {
        let snapshot = accessibility_snapshot();
        let node = &snapshot["roots"][0]["children"][0];

        assert!(element_matches_selector(node, &selector()));
        assert!(!element_matches_selector(
            node,
            &ElementSelectorPayload {
                label: Some("Cancel".to_owned()),
                ..selector()
            }
        ));
    }

    #[test]
    fn named_stream_quality_profile_controls_resolution_over_stale_max_edge() {
        let payload = StreamQualityPayload {
            profile: Some("quality".to_owned()),
            video_codec: None,
            max_edge: Some(1280),
            fps: None,
            min_bitrate: None,
            bits_per_pixel: None,
        };

        assert_eq!(
            resolved_stream_quality_limits(
                &payload,
                Some(stream_quality_profile("quality").unwrap())
            ),
            StreamQualityLimits {
                max_edge: 4096,
                fps: 60,
                min_bitrate: 60_000_000,
                bits_per_pixel: 10,
            }
        );
    }

    #[test]
    fn first_matching_element_searches_descendants() {
        let found = first_matching_element(&accessibility_snapshot(), &selector()).unwrap();

        assert_eq!(found["AXIdentifier"], "continue-button");
    }

    #[test]
    fn tap_point_from_snapshot_returns_normalized_element_center() {
        let point = tap_point_from_snapshot(&accessibility_snapshot(), &selector()).unwrap();

        assert_eq!(point, (0.35, 0.275));
    }

    #[test]
    fn tap_point_from_snapshot_uses_agent_ref_index_order() {
        let point = tap_point_from_snapshot(
            &accessibility_snapshot(),
            &ElementSelectorPayload {
                index: Some(1),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(point, (0.35, 0.275));
    }

    #[test]
    fn accessibility_cache_returns_latest_interactive_snapshot() {
        let cache = AccessibilitySnapshotCache::default();
        cache.insert(
            AccessibilitySnapshotCacheKey {
                udid: "sim-1".to_owned(),
                source: "auto".to_owned(),
                max_depth: Some(4),
                include_hidden: false,
                interactive_only: true,
            },
            &json!({ "name": "first" }),
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
        cache.insert(
            AccessibilitySnapshotCacheKey {
                udid: "sim-1".to_owned(),
                source: "native-ax".to_owned(),
                max_depth: None,
                include_hidden: false,
                interactive_only: true,
            },
            &json!({ "name": "latest" }),
        );

        assert_eq!(cache.latest_interactive("sim-1").unwrap()["name"], "latest");
        assert!(cache.latest_interactive("sim-2").is_none());
    }

    #[test]
    fn accessibility_cache_skips_fallback_snapshots() {
        let cache = AccessibilitySnapshotCache::default();
        let key = AccessibilitySnapshotCacheKey {
            udid: "sim-1".to_owned(),
            source: "native-ax".to_owned(),
            max_depth: Some(4),
            include_hidden: false,
            interactive_only: false,
        };
        cache.insert(
            key.clone(),
            &json!({
                "source": SOURCE_NATIVE_AX,
                "fallbackReason": "Native accessibility snapshot timed out."
            }),
        );
        cache.insert(
            key.clone(),
            &json!({ "source": SOURCE_NATIVE_AX, "roots": [{ "name": "ready" }] }),
        );

        assert_eq!(
            cache.get_compatible(&key).unwrap().1["roots"][0]["name"],
            "ready"
        );
    }

    #[test]
    fn normalize_screen_point_clamps_to_root_bounds() {
        let point =
            normalize_screen_point_from_snapshot(&accessibility_snapshot(), 500.0, -20.0).unwrap();

        assert_eq!(point, (1.0, 0.0));
    }

    #[test]
    fn accessibility_point_snapshot_returns_deepest_node() {
        let snapshot = json!({
            "source": "android-uiautomator",
            "availableSources": ["android-uiautomator"],
            "roots": [{
                "type": "FrameLayout",
                "frame": { "x": 0.0, "y": 0.0, "width": 400.0, "height": 800.0 },
                "children": [{
                    "type": "ViewGroup",
                    "AXIdentifier": "container",
                    "frame": { "x": 0.0, "y": 100.0, "width": 400.0, "height": 300.0 },
                    "children": [{
                        "type": "Button",
                        "AXIdentifier": "child-button",
                        "frame": { "x": 120.0, "y": 140.0, "width": 80.0, "height": 60.0 },
                        "children": []
                    }]
                }]
            }]
        });

        let point = accessibility_point_snapshot(&snapshot, 150.0, 160.0).unwrap();

        assert_eq!(point["source"], "android-uiautomator");
        assert_eq!(point["roots"][0]["AXIdentifier"], "child-button");
        assert!(point["roots"][0].get("children").is_none());
    }

    #[test]
    fn gesture_presets_clamp_delta_and_reject_unknown_names() {
        assert_eq!(
            normalized_gesture_coordinates("scroll-down", Some(2.0)).unwrap(),
            (0.5, 0.975, 0.5, 0.025000000000000022, 500)
        );
        assert!(normalized_gesture_coordinates("orbit", None).is_err());
    }

    #[test]
    fn scroll_until_visible_plans_android_swipe_for_android_ids() {
        let payload = ScrollUntilVisiblePayload {
            selector: ElementSelectorPayload::default(),
            source: Some("android-uiautomator".to_owned()),
            max_depth: None,
            include_hidden: None,
            timeout_ms: None,
            poll_ms: None,
            direction: Some("down".to_owned()),
            duration_ms: Some(225),
            steps: Some(7),
        };

        let plan = scroll_input_plan_for_udid("android:Pixel_8", &payload).unwrap();

        assert_eq!(plan.backend, ScrollInputBackend::Android);
        assert_eq!(plan.swipe.start_y, 0.78);
        assert_eq!(plan.swipe.end_y, 0.22);
        assert_eq!(plan.swipe.duration_ms, 225);
        assert_eq!(plan.swipe.steps, 7);
    }

    #[test]
    fn compact_accessibility_snapshot_removes_nested_noise_but_keeps_identity() {
        let compact = compact_accessibility_snapshot(&accessibility_snapshot());

        assert_eq!(compact["roots"][0]["children"][0]["id"], "continue-button");
        assert_eq!(compact["roots"][0]["children"][0]["label"], "Continue");
        assert!(compact["roots"][0]["children"][0].get("frame").is_some());
    }

    #[test]
    fn trim_tree_depth_drops_children_at_requested_depth() {
        let trimmed = trim_tree_depth(accessibility_snapshot(), Some(0));

        assert_eq!(trimmed["roots"][0]["children"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn inspector_source_detection_prefers_framework_specific_sources() {
        let sources = inspector_available_sources(&json!({
            "reactNative": { "available": true },
            "flutter": { "available": true },
            "appHierarchy": { "available": true, "source": "nativescript" },
            "uikit": { "available": true }
        }));

        assert_eq!(
            sources,
            vec![
                SOURCE_REACT_NATIVE.to_owned(),
                SOURCE_FLUTTER.to_owned(),
                SOURCE_NATIVE_SCRIPT.to_owned(),
                SOURCE_UIKIT.to_owned()
            ]
        );
    }

    #[test]
    fn chrome_devtools_source_only_allows_cdp_capable_app_inspectors() {
        let react_native = InspectorSession {
            transport: InspectorSessionTransport::Connected,
            available_sources: vec![SOURCE_REACT_NATIVE.to_owned(), SOURCE_SWIFTUI.to_owned()],
            info: json!({}),
            process_identifier: 1,
        };
        let native_script = InspectorSession {
            transport: InspectorSessionTransport::Connected,
            available_sources: vec![SOURCE_NATIVE_SCRIPT.to_owned()],
            info: json!({}),
            process_identifier: 2,
        };
        let swiftui = InspectorSession {
            transport: InspectorSessionTransport::Connected,
            available_sources: vec![SOURCE_SWIFTUI.to_owned(), SOURCE_UIKIT.to_owned()],
            info: json!({}),
            process_identifier: 3,
        };

        assert_eq!(
            chrome_devtools_source_for_session(&react_native),
            Some(SOURCE_REACT_NATIVE)
        );
        assert_eq!(
            chrome_devtools_source_for_session(&native_script),
            Some(SOURCE_NATIVE_SCRIPT)
        );
        assert_eq!(chrome_devtools_source_for_session(&swiftui), None);
    }

    #[test]
    fn best_inspector_session_prioritizes_react_native_then_nativescript() {
        let uikit = InspectorSession {
            transport: InspectorSessionTransport::Connected,
            available_sources: vec![SOURCE_UIKIT.to_owned()],
            info: json!({}),
            process_identifier: 1,
        };
        let react_native = InspectorSession {
            transport: InspectorSessionTransport::Tcp { port: 47370 },
            available_sources: vec![SOURCE_REACT_NATIVE.to_owned()],
            info: json!({}),
            process_identifier: 2,
        };

        let best = best_inspector_session(vec![uikit, react_native]).unwrap();

        assert_eq!(best.process_identifier, 2);
    }

    #[test]
    fn published_inspector_session_uses_remote_service_transport() {
        let session = inspector_session_from_published(PublishedInspector {
            access_token: "secret-token".to_owned(),
            available_sources: vec![SOURCE_REACT_NATIVE.to_owned()],
            service_id: "service-a".to_owned(),
            info: json!({
                "bundleIdentifier": "com.example.App",
                "protocolVersion": "1.0"
            }),
            process_identifier: 42,
            server_url: "http://127.0.0.1:4310".to_owned(),
            updated_at_unix_ms: 1,
        });

        assert_eq!(inspector_session_score(&session), 0);
        let InspectorSessionTransport::RemoteService {
            server_url,
            access_token,
        } = &session.transport
        else {
            panic!("published inspector should use remote service transport");
        };
        assert_eq!(server_url, "http://127.0.0.1:4310");
        assert_eq!(access_token, "secret-token");

        let metadata = inspector_metadata(
            &session.info,
            &json!({ "displayScale": 3.0 }),
            session.process_identifier,
            &session.transport,
        );
        assert_eq!(metadata["transport"], "remote-websocket");
        assert_eq!(metadata["serviceUrl"], "http://127.0.0.1:4310");
        assert!(metadata["port"].is_null());
    }

    #[test]
    fn published_inspector_session_filters_disallowed_uikit_source() {
        let session = inspector_session_from_published(PublishedInspector {
            access_token: "secret-token".to_owned(),
            available_sources: vec![SOURCE_UIKIT.to_owned()],
            service_id: "service-a".to_owned(),
            info: json!({
                "bundleIdentifier": "com.example.FlutterApp",
                "processIdentifier": 42,
                "flutter": { "available": true },
                "appHierarchy": { "available": true, "source": SOURCE_FLUTTER },
                "uikit": { "available": false }
            }),
            process_identifier: 42,
            server_url: "http://127.0.0.1:4310".to_owned(),
            updated_at_unix_ms: 1,
        });

        assert_eq!(session.available_sources, vec![SOURCE_FLUTTER.to_owned()]);
        assert_eq!(inspector_session_score(&session), 1);
    }

    #[test]
    fn inspector_source_detection_does_not_invent_uikit_for_flutter_hierarchy() {
        let sources = inspector_available_sources(&json!({
            "appHierarchy": { "available": true, "source": SOURCE_FLUTTER }
        }));

        assert_eq!(sources, vec![SOURCE_FLUTTER.to_owned()]);
    }

    #[test]
    fn inspector_source_detection_uses_react_native_snapshot_source() {
        let sources = inspector_available_sources(&json!({
            "source": SOURCE_REACT_NATIVE,
            "roots": []
        }));

        assert_eq!(sources, vec![SOURCE_REACT_NATIVE.to_owned()]);
    }

    #[test]
    fn inspector_source_detection_uses_nativescript_snapshot_source() {
        let sources = inspector_available_sources(&json!({
            "source": SOURCE_NATIVE_SCRIPT,
            "roots": []
        }));

        assert_eq!(sources, vec![SOURCE_NATIVE_SCRIPT.to_owned()]);
    }

    #[test]
    fn direct_inspector_request_endpoint_requires_api_auth() {
        assert!(is_inspector_agent_transport_path("/api/inspector/connect"));
        assert!(!is_inspector_agent_transport_path("/api/inspector/request"));
    }

    #[test]
    fn available_sources_for_react_native_snapshot_removes_uikit_fallback() {
        let sources = available_sources_for_snapshot(
            &[SOURCE_UIKIT.to_owned(), SOURCE_NATIVE_AX.to_owned()],
            &json!({ "source": SOURCE_REACT_NATIVE }),
        );

        assert_eq!(
            sources,
            vec![SOURCE_REACT_NATIVE.to_owned(), SOURCE_NATIVE_AX.to_owned()]
        );
    }

    #[test]
    fn native_ax_available_sources_preserve_inspector_sources() {
        let session = InspectorSession {
            transport: InspectorSessionTransport::Connected,
            available_sources: vec![SOURCE_REACT_NATIVE.to_owned()],
            info: Value::Null,
            process_identifier: 42,
        };

        assert_eq!(
            available_sources_with_native_ax(Some(&session)),
            vec![SOURCE_REACT_NATIVE.to_owned(), SOURCE_NATIVE_AX.to_owned()]
        );
    }

    #[test]
    fn available_sources_for_flutter_snapshot_removes_uikit_fallback() {
        let sources = available_sources_for_snapshot(
            &[SOURCE_UIKIT.to_owned(), SOURCE_NATIVE_AX.to_owned()],
            &json!({ "source": SOURCE_FLUTTER }),
        );

        assert_eq!(
            sources,
            vec![SOURCE_FLUTTER.to_owned(), SOURCE_NATIVE_AX.to_owned()]
        );
    }

    #[test]
    fn native_ax_expected_translation_failures_are_suppressed() {
        assert_eq!(
            suppress_native_ax_translation_error(
                "No translation object returned for simulator SIM"
            ),
            None
        );
        assert!(suppress_native_ax_translation_error("Bridge failed").is_some());
    }

    #[test]
    fn transient_native_ax_snapshot_errors_are_retryable() {
        assert!(is_transient_native_ax_snapshot_error(
            "No application accessibility root returned for simulator. The simulator may be between lifecycle states."
        ));
        assert!(is_transient_native_ax_snapshot_error(
            "No translation object returned for simulator SIM"
        ));
        assert!(!is_transient_native_ax_snapshot_error("Bridge failed"));
    }

    #[test]
    fn parse_lsof_tcp_listener_extracts_pid_and_port() {
        assert_eq!(
            parse_lsof_tcp_listener("Fixture 123 dj 12u IPv4 0x1 0t0 TCP 127.0.0.1:47370 (LISTEN)"),
            Some((123, 47370))
        );
        assert_eq!(
            parse_lsof_tcp_listener(
                "Fixture 123 dj 12u IPv4 0x1 0t0 TCP 127.0.0.1:47370 (ESTABLISHED)"
            ),
            None
        );
    }

    #[test]
    fn parse_ui_application_service_line_extracts_pid_and_service() {
        let service = parse_ui_application_service_line(
            "   41210      - \tUIKitApplication:com.apple.mobilesafari[2777][rb-running]",
        )
        .unwrap();
        assert_eq!(service.pid, 41210);
        assert_eq!(
            service.service_name,
            "UIKitApplication:com.apple.mobilesafari[2777][rb-running]"
        );
    }

    #[test]
    fn ui_application_foreground_score_prefers_focal_then_active_count() {
        let focal = UIKitApplicationServiceDetails {
            active_count: 1,
            app_name: None,
            bundle_identifier: None,
            process_identifier: 1,
            spawn_role: "ui focal (1)".to_owned(),
        };
        let background = UIKitApplicationServiceDetails {
            active_count: 10,
            app_name: None,
            bundle_identifier: None,
            process_identifier: 2,
            spawn_role: "non-ui (3)".to_owned(),
        };
        assert!(
            ui_application_foreground_score(&focal) > ui_application_foreground_score(&background)
        );
    }

    #[test]
    fn normalize_inspector_node_maps_runtime_metadata_to_accessibility_fields() {
        let normalized = normalize_inspector_node(
            &json!({
                "id": "node-1",
                "className": "UIButton",
                "displayName": "Button",
                "accessibility": {
                    "identifier": "continue-button",
                    "label": "Continue",
                    "value": "Ready"
                },
                "frameInScreen": { "x": 10.0, "y": 20.0, "width": 30.0, "height": 40.0 },
                "isUserInteractionEnabled": true,
                "isHidden": false,
                "alpha": 1.0,
                "children": []
            }),
            Some(42),
        );

        assert_eq!(normalized["AXUniqueId"], "node-1");
        assert_eq!(normalized["AXIdentifier"], "continue-button");
        assert_eq!(normalized["AXLabel"], "Continue");
        assert_eq!(normalized["AXValue"], "Ready");
        assert_eq!(normalized["enabled"], true);
        assert_eq!(normalized["pid"], 42);
    }

    #[test]
    fn tree_metadata_attaches_available_sources_and_fallback_reason() {
        let metadata = attach_tree_metadata(
            json!({ "roots": [], "source": SOURCE_NATIVE_AX }),
            &[SOURCE_SWIFTUI.to_owned(), SOURCE_NATIVE_AX.to_owned()],
            Some("native accessibility unavailable".to_owned()),
        );

        assert_eq!(metadata["availableSources"][0], SOURCE_SWIFTUI);
        assert_eq!(metadata["fallbackSource"], SOURCE_NATIVE_AX);
        assert_eq!(
            metadata["fallbackReason"],
            "native accessibility unavailable"
        );
    }

    #[test]
    fn accessibility_source_parser_accepts_documented_aliases() {
        assert!(matches!(
            AccessibilitySource::parse(Some("rn")).unwrap(),
            AccessibilitySource::ReactNative
        ));
        assert!(matches!(
            AccessibilitySource::parse(Some("flutter")).unwrap(),
            AccessibilitySource::Flutter
        ));
        assert!(matches!(
            AccessibilitySource::parse(Some("swift-ui")).unwrap(),
            AccessibilitySource::SwiftUI
        ));
        assert!(AccessibilitySource::parse(Some("unknown")).is_err());
    }

    #[test]
    fn single_action_payloads_deserialize_like_batch_steps() {
        let tap: BatchStep = serde_json::from_value(json!({
            "action": "tap",
            "selector": { "label": "Continue" },
            "durationMs": 1,
        }))
        .unwrap();
        match tap {
            BatchStep::Tap(payload) => {
                assert_eq!(payload.selector.label.as_deref(), Some("Continue"));
                assert_eq!(payload.duration_ms, Some(1));
            }
            _ => panic!("tap payload should decode as BatchStep::Tap"),
        }

        let open_url: BatchStep = serde_json::from_value(json!({
            "action": "openUrl",
            "url": "https://example.com",
        }))
        .unwrap();
        match open_url {
            BatchStep::OpenUrl { url } => assert_eq!(url, "https://example.com"),
            _ => panic!("openUrl payload should decode as BatchStep::OpenUrl"),
        }
    }

    #[test]
    fn split_filter_values_trims_lowercases_and_omits_empty_parts() {
        assert_eq!(
            split_filter_values(Some(" Error, SpringBoard ,, DEBUG ")),
            vec!["error", "springboard", "debug"]
        );
    }
}
