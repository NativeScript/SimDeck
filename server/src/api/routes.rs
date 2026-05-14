use crate::android::{self, AndroidBridge, AndroidEmulatorSpec};
use crate::api::json::json;
use crate::auth;
use crate::config::Config;
use crate::devtools;
use crate::error::AppError;
use crate::inspector::{InspectorHub, PublishedInspector};
use crate::logs::LogRegistry;
use crate::metrics::counters::{ClientStreamStats, Metrics};
use crate::native::bridge::{LogFilters, NativeBridge, NativePairedWatchSpec};
use crate::performance::{
    sample_stack, DisplaySignal, ForegroundProcess, PerformanceQuery, PerformanceRegistry,
};
use crate::simulators::registry::SessionRegistry;
use crate::simulators::session::SimulatorSession;
use crate::static_files;
use crate::transport::packet::FramePacket;
use crate::transport::webrtc::AndroidWebRtcSource;
use crate::webkit;
use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{header, HeaderMap, Method, Request, StatusCode, Uri};
use axum::middleware::{from_fn_with_state, Next};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::{Bytes, BytesMut};
use futures::{future::join_all, SinkExt, StreamExt};
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
use tokio::sync::{broadcast, mpsc, Mutex};
use tokio::task;
use tokio::time::timeout;
use tower_http::trace::{DefaultMakeSpan, DefaultOnFailure, TraceLayer};
use tracing::Level;

const SIMULATOR_INVENTORY_CACHE_TTL: Duration = Duration::from_secs(5);
const SIMULATOR_INVENTORY_TIMEOUT: Duration = Duration::from_secs(8);
const SIMULATOR_INVENTORY_FORCE_REFRESH_TIMEOUT: Duration = Duration::from_secs(90);
const H264_WS_MAGIC: &[u8; 4] = b"SDH1";
const H264_WS_HEADER_LEN: usize = 40;
const H264_WS_FLAG_KEYFRAME: u8 = 1 << 0;
const H264_WS_FLAG_CONFIG: u8 = 1 << 1;
const H264_WS_SEND_TIMEOUT: Duration = Duration::from_secs(2);
const H264_WS_KEYFRAME_WAIT_TIMEOUT: Duration = Duration::from_secs(3);
const STREAM_CLIENT_FOREGROUND_TTL: Duration = Duration::from_secs(30);
const CHROME_DEVTOOLS_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(1);
const FOREGROUND_APP_CACHE_TTL: Duration = Duration::from_secs(3);
const FOREGROUND_APP_STALE_TTL: Duration = Duration::from_secs(30);
const SAFARI_ACTIVE_TAB_HINT_TIMEOUT: Duration = Duration::from_millis(750);

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
    pub android: AndroidBridge,
}

#[derive(Clone)]
struct CachedForegroundApp {
    cached_at: Instant,
    foreground_app: devtools::ForegroundApp,
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

#[derive(Deserialize)]
struct OpenUrlPayload {
    url: String,
}

#[derive(Deserialize)]
struct LaunchPayload {
    #[serde(rename = "bundleId")]
    bundle_id: String,
}

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

#[derive(Deserialize)]
struct TouchPayload {
    x: f64,
    y: f64,
    phase: String,
}

#[derive(Deserialize)]
struct TouchSequencePayload {
    events: Vec<TouchSequenceEvent>,
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

#[derive(Debug, Eq, PartialEq)]
struct StreamQualityLimits {
    max_edge: u32,
    fps: u32,
    min_bitrate: u32,
    bits_per_pixel: u32,
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
        max_edge: 1170,
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

const VISIBLE_STREAM_QUALITY_PROFILE_IDS: &[&str] = &["full", "balanced", "economy", "low", "tiny"];

static STREAM_CONFIG_LOCK: OnceLock<StdMutex<()>> = OnceLock::new();

#[derive(Deserialize, Clone)]
struct TouchSequenceEvent {
    x: f64,
    y: f64,
    phase: String,
    #[serde(rename = "delayMsAfter")]
    delay_ms_after: Option<u64>,
}

#[derive(Deserialize)]
struct KeyPayload {
    #[serde(rename = "keyCode")]
    key_code: u16,
    modifiers: Option<u32>,
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
    DismissKeyboard,
    Home,
    AppSwitcher,
    RotateLeft,
    RotateRight,
    ToggleAppearance,
}

#[derive(Deserialize)]
struct KeySequencePayload {
    #[serde(rename = "keyCodes")]
    key_codes: Vec<u16>,
    #[serde(rename = "delayMs")]
    delay_ms: Option<u64>,
}

#[derive(Deserialize)]
struct ButtonPayload {
    button: String,
    #[serde(rename = "durationMs")]
    duration_ms: Option<u32>,
    phase: Option<String>,
    #[serde(rename = "usagePage")]
    usage_page: Option<u32>,
    usage: Option<u32>,
}

#[derive(Deserialize)]
struct CrownPayload {
    delta: f64,
}

#[derive(Deserialize)]
struct ChromePngQuery {
    buttons: Option<String>,
}

#[derive(Deserialize)]
struct ChromeButtonPngQuery {
    pressed: Option<String>,
}

#[derive(Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
struct ElementSelectorPayload {
    id: Option<String>,
    label: Option<String>,
    value: Option<String>,
    #[serde(alias = "type")]
    element_type: Option<String>,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct AccessibilityQueryPayload {
    #[serde(default)]
    selector: ElementSelectorPayload,
    source: Option<String>,
    max_depth: Option<usize>,
    include_hidden: Option<bool>,
    limit: Option<usize>,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct WaitForPayload {
    #[serde(default)]
    selector: ElementSelectorPayload,
    source: Option<String>,
    max_depth: Option<usize>,
    include_hidden: Option<bool>,
    timeout_ms: Option<u64>,
    poll_ms: Option<u64>,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct TapElementPayload {
    x: Option<f64>,
    y: Option<f64>,
    normalized: Option<bool>,
    #[serde(default)]
    selector: ElementSelectorPayload,
    source: Option<String>,
    max_depth: Option<usize>,
    include_hidden: Option<bool>,
    wait_timeout_ms: Option<u64>,
    poll_ms: Option<u64>,
    duration_ms: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BatchPayload {
    steps: Vec<BatchStep>,
    continue_on_error: Option<bool>,
}

#[derive(Deserialize, Clone)]
#[serde(
    tag = "action",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum BatchStep {
    Sleep {
        ms: Option<u64>,
        seconds: Option<f64>,
    },
    Tap(TapElementPayload),
    WaitFor(WaitForPayload),
    Assert(WaitForPayload),
    Key {
        key_code: u16,
        modifiers: Option<u32>,
    },
    KeySequence {
        key_codes: Vec<u16>,
        delay_ms: Option<u64>,
    },
    Touch {
        x: f64,
        y: f64,
        phase: Option<String>,
        down: Option<bool>,
        up: Option<bool>,
        delay_ms: Option<u64>,
    },
    TouchSequence {
        events: Vec<TouchSequenceEvent>,
    },
    Swipe {
        start_x: f64,
        start_y: f64,
        end_x: f64,
        end_y: f64,
        duration_ms: Option<u64>,
        steps: Option<u32>,
    },
    Gesture {
        preset: String,
        duration_ms: Option<u64>,
        delta: Option<f64>,
        steps: Option<u32>,
    },
    Type {
        text: String,
        delay_ms: Option<u64>,
    },
    Button {
        button: String,
        duration_ms: Option<u32>,
    },
    Crown {
        delta: f64,
    },
    Launch {
        bundle_id: String,
    },
    OpenUrl {
        url: String,
    },
    Home,
    DismissKeyboard,
    AppSwitcher,
    RotateLeft,
    RotateRight,
    ToggleAppearance,
    Describe {
        source: Option<String>,
        max_depth: Option<usize>,
        include_hidden: Option<bool>,
    },
}

#[derive(Deserialize)]
struct AccessibilityPointQuery {
    x: f64,
    y: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccessibilityTreeQuery {
    source: Option<String>,
    max_depth: Option<usize>,
    include_hidden: Option<bool>,
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
        .route("/api/simulators/{udid}/uninstall", post(uninstall_app))
        .route(
            "/api/simulators/{udid}/pasteboard",
            get(get_pasteboard).post(set_pasteboard),
        )
        .route("/api/simulators/{udid}/screenshot.png", get(screenshot_png))
        .route(
            "/api/simulators/{udid}/toggle-appearance",
            post(toggle_appearance),
        )
        .route("/api/simulators/{udid}/refresh", post(refresh_stream))
        .route("/api/simulators/{udid}/open-url", post(open_url))
        .route("/api/simulators/{udid}/launch", post(launch_bundle))
        .route("/api/simulators/{udid}/tap", post(tap_element))
        .route("/api/simulators/{udid}/query", post(accessibility_query))
        .route("/api/simulators/{udid}/wait-for", post(wait_for_element))
        .route("/api/simulators/{udid}/assert", post(assert_element))
        .route("/api/simulators/{udid}/batch", post(run_batch))
        .route("/api/simulators/{udid}/touch", post(send_touch))
        .route("/api/simulators/{udid}/control", get(control_socket))
        .route("/api/simulators/{udid}/input", get(control_socket))
        .route("/api/simulators/{udid}/h264", get(h264_socket))
        .route("/api/simulators/{udid}/webrtc/offer", post(webrtc_offer))
        .route(
            "/api/simulators/{udid}/touch-sequence",
            post(send_touch_sequence),
        )
        .route("/api/simulators/{udid}/key", post(send_key))
        .route(
            "/api/simulators/{udid}/key-sequence",
            post(send_key_sequence),
        )
        .route(
            "/api/simulators/{udid}/dismiss-keyboard",
            post(dismiss_keyboard),
        )
        .route("/api/simulators/{udid}/button", post(press_button))
        .route("/api/simulators/{udid}/crown", post(rotate_crown))
        .route("/api/simulators/{udid}/home", post(press_home))
        .route(
            "/api/simulators/{udid}/app-switcher",
            post(open_app_switcher),
        )
        .route("/api/simulators/{udid}/rotate-left", post(rotate_left))
        .route("/api/simulators/{udid}/rotate-right", post(rotate_right))
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
    if is_inspector_agent_transport_path(request.uri().path())
        || request.uri().path() == "/api/pair"
    {
        return next.run(request).await;
    }

    if request.method() == Method::OPTIONS {
        return auth::preflight_response(&state.config, request.headers());
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
    let mut response = Json(json_value!({ "ok": true })).into_response();
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
        "httpPort": state.config.http_port,
        "timestamp": SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or(Duration::ZERO).as_secs_f64(),
        "videoCodec": video_codec,
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
    State(state): State<AppState>,
    Path(udid): Path<String>,
    headers: HeaderMap,
) -> Result<Json<webkit::WebKitTargetDiscovery>, AppError> {
    let origin = request_origin(&headers);
    let mut discovery = webkit::discover_targets(&udid, origin.as_deref()).await?;
    let foreground_app = foreground_app_for_simulator(&state, &udid)
        .await
        .ok()
        .flatten();
    if foreground_app
        .as_ref()
        .is_some_and(is_safari_foreground_app)
    {
        if let Some(active_url_hint) = safari_active_url_hint(&state, &udid).await {
            mark_safari_active_webkit_target(&mut discovery.targets, &active_url_hint);
        }
    }
    Ok(Json(discovery))
}

async fn webkit_target_socket(
    Path((udid, target_id)): Path<(String, String)>,
    ws: WebSocketUpgrade,
) -> Response {
    ws.on_upgrade(move |socket| webkit::attach_websocket(udid, target_id, socket))
}

async fn safari_active_url_hint(state: &AppState, udid: &str) -> Option<String> {
    let probe_points = safari_active_url_probe_points(state.clone(), udid.to_owned()).await;
    for point in probe_points {
        let Ok(Ok(snapshot)) = timeout(
            SAFARI_ACTIVE_TAB_HINT_TIMEOUT,
            accessibility_snapshot(state.clone(), udid.to_owned(), Some(point), Some(0)),
        )
        .await
        else {
            continue;
        };
        if let Some(hint) = active_url_hint_from_accessibility_snapshot(&snapshot) {
            return Some(hint);
        }
    }
    None
}

async fn safari_active_url_probe_points(state: AppState, udid: String) -> Vec<(f64, f64)> {
    let profile = run_bridge_action(state, move |bridge| bridge.chrome_profile(&udid)).await;
    let (screen_width, screen_height) = profile
        .map(|profile| (profile.screen_width, profile.screen_height))
        .unwrap_or((402.0, 874.0));
    let center_x = (screen_width * 0.5).max(1.0);
    let bottom_address_y = (screen_height - 54.0).clamp(1.0, screen_height.max(1.0));
    let bottom_title_y = (screen_height - 28.0).clamp(1.0, screen_height.max(1.0));
    let top_address_y = 92.0_f64.min((screen_height * 0.18).max(1.0));
    vec![
        (center_x, bottom_address_y),
        (center_x, bottom_title_y),
        (center_x, top_address_y),
    ]
}

fn active_url_hint_from_accessibility_snapshot(snapshot: &Value) -> Option<String> {
    let roots = snapshot.get("roots").and_then(Value::as_array)?;
    roots
        .iter()
        .find_map(active_url_hint_from_accessibility_node)
}

fn active_url_hint_from_accessibility_node(node: &Value) -> Option<String> {
    for key in [
        "AXValue", "value", "url", "AXLabel", "label", "title", "name",
    ] {
        if let Some(hint) = node
            .get(key)
            .and_then(Value::as_str)
            .and_then(sanitize_active_url_hint)
        {
            return Some(hint);
        }
    }
    node.get("children")
        .and_then(Value::as_array)
        .and_then(|children| {
            children
                .iter()
                .find_map(active_url_hint_from_accessibility_node)
        })
}

fn sanitize_active_url_hint(value: &str) -> Option<String> {
    let cleaned = value
        .chars()
        .filter(|character| {
            !character.is_control()
                && !matches!(
                    *character,
                    '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
                )
        })
        .collect::<String>()
        .trim()
        .to_owned();
    if cleaned.is_empty() || cleaned.eq_ignore_ascii_case("address") {
        return None;
    }
    let lower = cleaned.to_ascii_lowercase();
    if lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("file://")
        || cleaned.contains('.')
    {
        return Some(cleaned);
    }
    None
}

fn mark_safari_active_webkit_target(targets: &mut [webkit::WebKitTarget], url_hint: &str) {
    let Some((active_index, _score)) = targets
        .iter()
        .enumerate()
        .filter(|(_, target)| target.kind == "safari-page")
        .filter_map(|(index, target)| {
            let score = safari_active_url_match_score(url_hint, target);
            (score > 0).then_some((index, score))
        })
        .max_by_key(|(_, score)| *score)
    else {
        return;
    };

    for (index, target) in targets.iter_mut().enumerate() {
        target.page_active = index == active_index;
    }
    targets.sort_by(|lhs, rhs| {
        rhs.page_active
            .cmp(&lhs.page_active)
            .then(lhs.app_name.cmp(&rhs.app_name))
            .then(lhs.title.cmp(&rhs.title))
            .then(lhs.url.cmp(&rhs.url))
            .then(lhs.page_id.cmp(&rhs.page_id))
    });
}

fn safari_active_url_match_score(url_hint: &str, target: &webkit::WebKitTarget) -> u8 {
    let Some(target_url) = target.url.as_deref() else {
        return 0;
    };
    let hint_key = normalized_url_key(url_hint);
    let target_key = normalized_url_key(target_url);
    if hint_key.is_empty() || target_key.is_empty() {
        return 0;
    }
    if hint_key == target_key {
        return 100;
    }

    let hint_host = normalized_url_host(&hint_key);
    let target_host = normalized_url_host(&target_key);
    if !hint_host.is_empty() && hint_host == target_host {
        return 90;
    }
    if target_key.contains(&hint_key)
        || (!target_host.is_empty() && hint_key.contains(&target_host))
    {
        return 80;
    }
    0
}

fn normalized_url_key(value: &str) -> String {
    let mut key = value
        .trim()
        .trim_matches('"')
        .trim_end_matches('/')
        .to_ascii_lowercase();
    for prefix in ["https://", "http://", "file://"] {
        if let Some(stripped) = key.strip_prefix(prefix) {
            key = stripped.to_owned();
            break;
        }
    }
    if let Some(stripped) = key.strip_prefix("www.") {
        key = stripped.to_owned();
    }
    key.trim_end_matches('/').to_owned()
}

fn normalized_url_host(value: &str) -> String {
    normalized_url_key(value)
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .trim_start_matches("www.")
        .to_owned()
}

fn is_safari_foreground_app(foreground: &devtools::ForegroundApp) -> bool {
    foreground.bundle_identifier.as_deref() == Some("com.apple.mobilesafari")
        || foreground.app_name.as_deref() == Some("Safari")
        || foreground.app_name.as_deref() == Some("MobileSafari")
}

async fn chrome_devtools_targets(
    State(state): State<AppState>,
    Path(udid): Path<String>,
    headers: HeaderMap,
) -> Result<Json<devtools::ChromeDevToolsTargetDiscovery>, AppError> {
    let origin = request_origin(&headers);
    let mut warnings = Vec::new();
    let foreground_app = foreground_app_for_simulator(&state, &udid)
        .await
        .ok()
        .flatten();
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
    let (mut external_targets, external_warnings) = match timeout(
        CHROME_DEVTOOLS_DISCOVERY_TIMEOUT,
        devtools::discover_external_devtools_targets(
            &udid,
            origin.as_deref(),
            simulator.as_ref().map(|simulator| simulator.name.as_str()),
            simulator
                .as_ref()
                .map(|simulator| simulator.device_type_name.as_str()),
        ),
    )
    .await
    {
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

async fn boot_simulator(
    State(state): State<AppState>,
    Path(udid): Path<String>,
) -> Result<Json<Value>, AppError> {
    if android::is_android_id(&udid) {
        let action_udid = udid.clone();
        run_android_action(state.clone(), move |android| {
            android.boot(&action_udid)?;
            android.wait_until_booted(&action_udid, Duration::from_secs(240))?;
            Ok(())
        })
        .await?;
        return android_simulator_payload(state, udid).await;
    }
    forget_lifecycle_session(&state, &udid);
    let action_udid = udid.clone();
    run_bridge_action(state.clone(), move |bridge| {
        bridge.boot_simulator(&action_udid)
    })
    .await?;
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
    if android::is_android_id(&udid) {
        let action_udid = udid.clone();
        run_android_action(state, move |android| {
            android.install_app(&action_udid, &payload.app_path)
        })
        .await?;
        return Ok(json(json_value!({ "ok": true })));
    }
    let action_udid = udid.clone();
    run_bridge_action(state, move |bridge| {
        bridge.install_app(&action_udid, &payload.app_path)
    })
    .await?;
    Ok(json(json_value!({ "ok": true })))
}

async fn uninstall_app(
    State(state): State<AppState>,
    Path(udid): Path<String>,
    Json(payload): Json<UninstallPayload>,
) -> Result<Json<Value>, AppError> {
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
    run_bridge_action(state, move |bridge| {
        bridge.uninstall_app(&action_udid, &payload.bundle_id)
    })
    .await?;
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
) -> Result<(StatusCode, HeaderMap, Vec<u8>), AppError> {
    let png = if android::is_android_id(&udid) {
        run_android_action(state, move |android| android.screenshot_png(&udid)).await?
    } else {
        run_bridge_action(state, move |bridge| bridge.screenshot_png(&udid)).await?
    };
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, "image/png".parse().unwrap());
    headers.insert(
        header::CACHE_CONTROL,
        "no-cache, no-store, must-revalidate".parse().unwrap(),
    );
    Ok((StatusCode::OK, headers, png))
}

async fn toggle_appearance(
    State(state): State<AppState>,
    Path(udid): Path<String>,
) -> Result<Json<Value>, AppError> {
    if android::is_android_id(&udid) {
        run_android_action(state, move |android| android.toggle_appearance(&udid)).await?;
        return Ok(json(json_value!({ "ok": true })));
    }
    let action_udid = udid.clone();
    run_bridge_action(state, move |bridge| bridge.toggle_appearance(&action_udid)).await?;
    Ok(json(json_value!({ "ok": true })))
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

async fn open_url(
    State(state): State<AppState>,
    Path(udid): Path<String>,
    Json(payload): Json<OpenUrlPayload>,
) -> Result<Json<Value>, AppError> {
    if payload.url.trim().is_empty() {
        return Err(AppError::bad_request("Request body must include `url`."));
    }
    if android::is_android_id(&udid) {
        let action_udid = udid.clone();
        run_android_action(state, move |android| {
            android.open_url(&action_udid, &payload.url)
        })
        .await?;
        return Ok(json(json_value!({ "ok": true })));
    }
    let action_udid = udid.clone();
    run_bridge_action(state, move |bridge| {
        bridge.open_url(&action_udid, &payload.url)
    })
    .await?;
    Ok(json(json_value!({ "ok": true })))
}

async fn launch_bundle(
    State(state): State<AppState>,
    Path(udid): Path<String>,
    Json(payload): Json<LaunchPayload>,
) -> Result<Json<Value>, AppError> {
    if payload.bundle_id.trim().is_empty() {
        return Err(AppError::bad_request(
            "Request body must include `bundleId`.",
        ));
    }
    if android::is_android_id(&udid) {
        let action_udid = udid.clone();
        run_android_action(state, move |android| {
            android.launch_package(&action_udid, &payload.bundle_id)
        })
        .await?;
        return Ok(json(json_value!({ "ok": true })));
    }
    let action_udid = udid.clone();
    run_bridge_action(state, move |bridge| {
        bridge.launch_bundle(&action_udid, &payload.bundle_id)
    })
    .await?;
    Ok(json(json_value!({ "ok": true })))
}

async fn tap_element(
    State(state): State<AppState>,
    Path(udid): Path<String>,
    Json(payload): Json<TapElementPayload>,
) -> Result<Json<Value>, AppError> {
    perform_tap_payload(state, udid, payload).await?;
    Ok(json(json_value!({ "ok": true })))
}

async fn accessibility_query(
    State(state): State<AppState>,
    Path(udid): Path<String>,
    Json(payload): Json<AccessibilityQueryPayload>,
) -> Result<Json<Value>, AppError> {
    let snapshot = accessibility_tree_value(
        state,
        udid,
        payload.source.as_deref(),
        payload.max_depth,
        payload.include_hidden.unwrap_or(false),
    )
    .await?;
    let matches = query_compact_elements(
        &snapshot,
        &payload.selector,
        payload.limit.unwrap_or(64).clamp(1, 512),
    );
    Ok(json(json_value!({
        "ok": true,
        "source": snapshot.get("source").cloned().unwrap_or(Value::Null),
        "count": matches.len(),
        "matches": matches,
    })))
}

async fn wait_for_element(
    State(state): State<AppState>,
    Path(udid): Path<String>,
    Json(payload): Json<WaitForPayload>,
) -> Result<Json<Value>, AppError> {
    wait_for_element_payload(state, udid, payload).await
}

async fn assert_element(
    State(state): State<AppState>,
    Path(udid): Path<String>,
    Json(payload): Json<WaitForPayload>,
) -> Result<Json<Value>, AppError> {
    wait_for_element_payload(state, udid, payload).await
}

async fn run_batch(
    State(state): State<AppState>,
    Path(udid): Path<String>,
    Json(payload): Json<BatchPayload>,
) -> Result<Json<Value>, AppError> {
    if payload.steps.is_empty() {
        return Err(AppError::bad_request(
            "Request body must include at least one batch step.",
        ));
    }
    if payload.steps.len() > 256 {
        return Err(AppError::bad_request(
            "Batch cannot contain more than 256 steps.",
        ));
    }

    let continue_on_error = payload.continue_on_error.unwrap_or(false);
    let mut results = Vec::new();
    let mut failure_count = 0usize;
    for (index, step) in payload.steps.into_iter().enumerate() {
        let started = Instant::now();
        let result = run_batch_step(state.clone(), udid.clone(), step).await;
        let elapsed_ms = started.elapsed().as_millis() as u64;
        match result {
            Ok(value) => {
                results.push(json_value!({
                    "index": index,
                    "ok": true,
                    "elapsedMs": elapsed_ms,
                    "result": value,
                }));
            }
            Err(error) => {
                failure_count += 1;
                let message = error.to_string();
                results.push(json_value!({
                    "index": index,
                    "ok": false,
                    "elapsedMs": elapsed_ms,
                    "error": message,
                }));
                if !continue_on_error {
                    return Err(AppError::bad_request(format!(
                        "Batch step {} failed: {}",
                        index + 1,
                        message
                    )));
                }
            }
        }
    }
    Ok(json(json_value!({
        "ok": failure_count == 0,
        "failureCount": failure_count,
        "steps": results,
    })))
}

async fn send_touch(
    State(state): State<AppState>,
    Path(udid): Path<String>,
    Json(payload): Json<TouchPayload>,
) -> Result<Json<Value>, AppError> {
    if !payload.x.is_finite() || !payload.y.is_finite() {
        return Err(AppError::bad_request(
            "`x` and `y` must be finite normalized numbers.",
        ));
    }
    let x = payload.x.clamp(0.0, 1.0);
    let y = payload.y.clamp(0.0, 1.0);
    let phase = payload.phase;
    if android::is_android_id(&udid) {
        run_android_action(state, move |android| {
            android.send_touch(&udid, x, y, &phase)
        })
        .await?;
        return Ok(json(json_value!({ "ok": true })));
    }
    run_bridge_action(state, move |bridge| {
        let input = bridge.create_input_session(&udid)?;
        input.send_touch(x, y, &phase)
    })
    .await?;
    Ok(json(json_value!({ "ok": true })))
}

async fn send_touch_sequence(
    State(state): State<AppState>,
    Path(udid): Path<String>,
    Json(payload): Json<TouchSequencePayload>,
) -> Result<Json<Value>, AppError> {
    if payload.events.is_empty() {
        return Err(AppError::bad_request(
            "Request body must include at least one touch event.",
        ));
    }
    if payload.events.len() > 64 {
        return Err(AppError::bad_request(
            "Touch sequence cannot contain more than 64 events.",
        ));
    }
    for event in &payload.events {
        if !event.x.is_finite() || !event.y.is_finite() {
            return Err(AppError::bad_request(
                "`x` and `y` must be finite normalized numbers.",
            ));
        }
    }
    if android::is_android_id(&udid) {
        run_android_action(state, move |android| {
            for event in payload.events {
                android.send_touch(
                    &udid,
                    event.x.clamp(0.0, 1.0),
                    event.y.clamp(0.0, 1.0),
                    &event.phase,
                )?;
                if let Some(delay_ms) = event.delay_ms_after.filter(|delay_ms| *delay_ms > 0) {
                    std::thread::sleep(Duration::from_millis(delay_ms));
                }
            }
            Ok(())
        })
        .await?;
        return Ok(json(json_value!({ "ok": true })));
    }
    run_bridge_action(state, move |bridge| {
        let input = bridge.create_input_session(&udid)?;
        for event in payload.events {
            input.send_touch(
                event.x.clamp(0.0, 1.0),
                event.y.clamp(0.0, 1.0),
                &event.phase,
            )?;
            if let Some(delay_ms) = event.delay_ms_after.filter(|delay_ms| *delay_ms > 0) {
                std::thread::sleep(Duration::from_millis(delay_ms));
            }
        }
        Ok(())
    })
    .await?;
    Ok(json(json_value!({ "ok": true })))
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

async fn h264_socket(
    State(state): State<AppState>,
    Path(udid): Path<String>,
    Query(query): Query<StreamQualityPayload>,
    websocket: WebSocketUpgrade,
) -> impl IntoResponse {
    websocket.on_upgrade(move |socket| handle_h264_socket(state, udid, query, socket))
}

async fn handle_android_control_socket(state: AppState, udid: String, socket: WebSocket) {
    let (mut sender, mut receiver) = socket.split();
    let mut active_touch: Option<android::AndroidTouchGesture> = None;
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
        let _ = run_android_control_message(state, udid, control_message, &mut active_touch).await;
    }
}

async fn run_android_control_message(
    state: AppState,
    udid: String,
    message: ControlMessage,
    active_touch: &mut Option<android::AndroidTouchGesture>,
) -> Result<(), AppError> {
    match message {
        ControlMessage::Touch { x, y, phase } => {
            handle_android_control_touch(state, udid, x, y, phase, active_touch).await
        }
        ControlMessage::EdgeTouch { x, y, phase, .. } => {
            handle_android_control_touch(state, udid, x, y, phase, active_touch).await
        }
        ControlMessage::MultiTouch { x1, y1, phase, .. } => {
            handle_android_control_touch(state, udid, x1, y1, phase, active_touch).await
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
                ControlMessage::Home => android.press_home(&udid),
                ControlMessage::AppSwitcher => android.open_app_switcher(&udid),
                ControlMessage::RotateLeft => android.rotate_left(&udid),
                ControlMessage::RotateRight => android.rotate_right(&udid),
                ControlMessage::Crown { .. } => Err(AppError::bad_request(
                    "Digital Crown rotation is only available for Apple Watch simulators.",
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
    active_touch: &mut Option<android::AndroidTouchGesture>,
) -> Result<(), AppError> {
    let action = android::update_touch_gesture(active_touch, x, y, &phase)?;
    if matches!(action, android::AndroidTouchAction::None) {
        return Ok(());
    }
    run_android_action(state, move |android| action.perform(&android, &udid)).await
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

async fn handle_h264_socket(
    state: AppState,
    udid: String,
    initial_quality: StreamQualityPayload,
    socket: WebSocket,
) {
    if android::is_android_id(&udid) {
        handle_android_h264_socket(state, udid, initial_quality, socket).await;
        return;
    }

    if initial_quality.has_any_value() {
        if let Err(error) = apply_stream_quality_payload(&state, &initial_quality) {
            tracing::debug!("Failed to apply H264 WebSocket stream quality for {udid}: {error}");
        }
    }
    let session = match state.registry.get_or_create_async(&udid).await {
        Ok(session) => session,
        Err(error) => {
            tracing::debug!("Failed to create H264 WebSocket session for {udid}: {error}");
            return;
        }
    };
    if let Err(error) = session.ensure_started_async().await {
        tracing::debug!("Failed to start H264 WebSocket session for {udid}: {error}");
        return;
    }

    let mut subscription = session.subscribe();
    let (mut sender, mut receiver) = socket.split();
    let mut decoder_synced = false;
    let mut last_sent_sequence: Option<u64> = None;

    let initial_keyframe = if let Some(keyframe) = session
        .latest_keyframe()
        .filter(|frame| h264_ws_frame_is_decoder_sync(frame))
    {
        Some(keyframe)
    } else {
        session
            .wait_for_keyframe(H264_WS_KEYFRAME_WAIT_TIMEOUT)
            .await
            .filter(|frame| h264_ws_frame_is_decoder_sync(frame))
    };

    if let Some(keyframe) = initial_keyframe {
        if h264_ws_frame_is_supported(&keyframe) {
            let message_bytes = h264_ws_frame_message(&keyframe);
            let message = Message::Binary(message_bytes);
            if timeout(H264_WS_SEND_TIMEOUT, sender.send(message))
                .await
                .ok()
                .and_then(Result::ok)
                .is_none()
            {
                return;
            }
            last_sent_sequence = Some(keyframe.frame_sequence);
            decoder_synced = true;
        }
    } else {
        session.request_keyframe();
    }

    loop {
        tokio::select! {
            received = receiver.next() => {
                let Some(received) = received else { break };
                let message = match received {
                    Ok(message) => message,
                    Err(error) => {
                        tracing::debug!("H264 WebSocket closed for {udid}: {error}");
                        break;
                    }
                };
                if !handle_h264_socket_message(&state, &session, &message) {
                    break;
                }
            }
            frame = subscription.recv() => {
                let frame = match frame {
                    Ok(frame) => frame,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        decoder_synced = false;
                        session.request_keyframe();
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                };
                if !h264_ws_frame_is_supported(&frame) {
                    continue;
                }
                if last_sent_sequence
                    .map(|sequence| frame.frame_sequence <= sequence)
                    .unwrap_or(false)
                {
                    continue;
                }
                if !decoder_synced && !frame.is_keyframe {
                    session.request_keyframe();
                    continue;
                }
                let is_keyframe = frame.is_keyframe;
                let message_bytes = h264_ws_frame_message(&frame);
                let message = Message::Binary(message_bytes);
                if timeout(H264_WS_SEND_TIMEOUT, sender.send(message))
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .is_none()
                {
                    break;
                }
                last_sent_sequence = Some(frame.frame_sequence);
                if is_keyframe {
                    decoder_synced = true;
                }
            }
        }
    }
}

async fn handle_android_h264_socket(
    state: AppState,
    udid: String,
    initial_quality: StreamQualityPayload,
    socket: WebSocket,
) {
    let source = match AndroidWebRtcSource::start(
        state.android.clone(),
        state.metrics.clone(),
        udid.clone(),
        initial_quality.max_edge,
        true,
    )
    .await
    {
        Ok(source) => source,
        Err(error) => {
            tracing::debug!("Failed to create Android H264 WebSocket source for {udid}: {error}");
            return;
        }
    };

    let mut subscription = source.subscribe();
    let (mut sender, mut receiver) = socket.split();
    let mut decoder_synced = false;
    let mut last_sent_sequence: Option<u64> = None;

    let initial_keyframe = source
        .wait_for_keyframe(H264_WS_KEYFRAME_WAIT_TIMEOUT)
        .await
        .filter(|frame| h264_ws_frame_is_decoder_sync(frame));

    if let Some(keyframe) = initial_keyframe {
        if h264_ws_frame_is_supported(&keyframe) {
            let message_bytes = h264_ws_frame_message(&keyframe);
            let message = Message::Binary(message_bytes);
            if timeout(H264_WS_SEND_TIMEOUT, sender.send(message))
                .await
                .ok()
                .and_then(Result::ok)
                .is_none()
            {
                return;
            }
            last_sent_sequence = Some(keyframe.frame_sequence);
            decoder_synced = true;
        }
    } else {
        source.request_keyframe();
    }

    loop {
        tokio::select! {
            received = receiver.next() => {
                let Some(received) = received else { break };
                let message = match received {
                    Ok(message) => message,
                    Err(error) => {
                        tracing::debug!("Android H264 WebSocket closed for {udid}: {error}");
                        break;
                    }
                };
                if !handle_android_h264_socket_message(&state, &source, &message) {
                    break;
                }
            }
            frame = subscription.recv() => {
                let frame = match frame {
                    Ok(frame) => frame,
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        decoder_synced = false;
                        source.request_keyframe();
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                };
                if !h264_ws_frame_is_supported(&frame) {
                    continue;
                }
                if last_sent_sequence
                    .map(|sequence| frame.frame_sequence <= sequence)
                    .unwrap_or(false)
                {
                    continue;
                }
                if !decoder_synced && !frame.is_keyframe {
                    source.request_keyframe();
                    continue;
                }
                let is_keyframe = frame.is_keyframe;
                let message_bytes = h264_ws_frame_message(&frame);
                let message = Message::Binary(message_bytes);
                if timeout(H264_WS_SEND_TIMEOUT, sender.send(message))
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .is_none()
                {
                    break;
                }
                last_sent_sequence = Some(frame.frame_sequence);
                if is_keyframe {
                    decoder_synced = true;
                }
            }
        }
    }
}

fn handle_h264_socket_message(
    state: &AppState,
    session: &SimulatorSession,
    message: &Message,
) -> bool {
    let text = match message {
        Message::Text(text) => text.as_str(),
        Message::Binary(bytes) => match std::str::from_utf8(bytes) {
            Ok(text) => text,
            Err(_) => return true,
        },
        Message::Close(_) => return false,
        Message::Ping(_) | Message::Pong(_) => return true,
    };
    let Ok(message) = serde_json::from_str::<H264SocketMessage>(text) else {
        return true;
    };
    match message {
        H264SocketMessage::ClientStats { stats } => {
            if !stats.client_id.trim().is_empty() && !stats.kind.trim().is_empty() {
                apply_stream_client_foreground_from_stats(state, &stats);
                state.metrics.record_client_stream_stats(*stats);
            }
        }
        H264SocketMessage::StreamControl {
            client_id,
            force_keyframe,
            foreground,
            snapshot,
        } => {
            apply_stream_client_foreground(state, session, &client_id, foreground);
            if force_keyframe.unwrap_or(false) {
                session.request_keyframe();
            }
            if snapshot.unwrap_or(false) {
                session.request_refresh();
            }
        }
        H264SocketMessage::StreamQuality { config } => {
            if let Err(error) = apply_stream_quality_payload(state, &config) {
                tracing::debug!("Failed to apply H264 WebSocket stream quality: {error}");
            } else {
                session.request_keyframe();
            }
        }
    }
    true
}

fn apply_stream_client_foreground(
    state: &AppState,
    session: &SimulatorSession,
    client_id: &Option<String>,
    foreground: Option<bool>,
) {
    let Some(foreground) = foreground else {
        return;
    };
    let Some(client_id) = client_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    let (any_foreground, changed) =
        state
            .stream_clients
            .record(session.udid(), client_id, foreground);
    if changed {
        session.set_client_foreground(any_foreground);
    }
}

fn handle_android_h264_socket_message(
    state: &AppState,
    source: &AndroidWebRtcSource,
    message: &Message,
) -> bool {
    let text = match message {
        Message::Text(text) => text.as_str(),
        Message::Binary(bytes) => match std::str::from_utf8(bytes) {
            Ok(text) => text,
            Err(_) => return true,
        },
        Message::Close(_) => return false,
        Message::Ping(_) | Message::Pong(_) => return true,
    };
    let Ok(message) = serde_json::from_str::<H264SocketMessage>(text) else {
        return true;
    };
    match message {
        H264SocketMessage::ClientStats { stats } => {
            if !stats.client_id.trim().is_empty() && !stats.kind.trim().is_empty() {
                state.metrics.record_client_stream_stats(*stats);
            }
        }
        H264SocketMessage::StreamControl {
            client_id: _,
            force_keyframe,
            foreground: _,
            snapshot,
        } => {
            if force_keyframe.unwrap_or(false) {
                source.request_keyframe();
            }
            if snapshot.unwrap_or(false) {
                source.request_refresh();
            }
        }
        H264SocketMessage::StreamQuality { config: _ } => {
            source.request_keyframe();
        }
    }
    true
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum H264SocketMessage {
    ClientStats {
        stats: Box<ClientStreamStats>,
    },
    StreamControl {
        #[serde(rename = "clientId")]
        client_id: Option<String>,
        #[serde(rename = "forceKeyframe")]
        force_keyframe: Option<bool>,
        foreground: Option<bool>,
        snapshot: Option<bool>,
    },
    StreamQuality {
        config: StreamQualityPayload,
    },
}

fn h264_ws_frame_is_supported(frame: &FramePacket) -> bool {
    frame
        .codec
        .as_deref()
        .map(|codec| {
            let codec = codec.to_ascii_lowercase();
            codec == "h264" || codec.starts_with("avc1")
        })
        .unwrap_or(true)
}

fn h264_ws_frame_is_decoder_sync(frame: &FramePacket) -> bool {
    h264_ws_frame_is_supported(frame)
        && frame.is_keyframe
        && frame
            .description
            .as_ref()
            .map(|description| !description.is_empty())
            .unwrap_or(false)
}

fn h264_ws_frame_message(frame: &FramePacket) -> Bytes {
    let config = frame.description.as_deref().unwrap_or(&[]);
    let flags = if frame.is_keyframe {
        H264_WS_FLAG_KEYFRAME
    } else {
        0
    } | if config.is_empty() {
        0
    } else {
        H264_WS_FLAG_CONFIG
    };
    let mut message = BytesMut::with_capacity(H264_WS_HEADER_LEN + config.len() + frame.data.len());
    message.extend_from_slice(H264_WS_MAGIC);
    message.extend_from_slice(&[1, flags]);
    message.extend_from_slice(&(H264_WS_HEADER_LEN as u16).to_be_bytes());
    message.extend_from_slice(&frame.frame_sequence.to_be_bytes());
    message.extend_from_slice(&frame.timestamp_us.to_be_bytes());
    message.extend_from_slice(&frame.width.to_be_bytes());
    message.extend_from_slice(&frame.height.to_be_bytes());
    message.extend_from_slice(&(config.len() as u32).to_be_bytes());
    message.extend_from_slice(&(frame.data.len() as u32).to_be_bytes());
    message.extend_from_slice(config);
    message.extend_from_slice(&frame.data);
    message.freeze()
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
    loop {
        let mut message = match pending.pop_front() {
            Some(message) => message,
            None => match receiver.recv().await {
                Some(message) => message,
                None => break,
            },
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
        let result = match message {
            ControlMessage::ToggleAppearance => {
                run_toggle_appearance_control(bridge.clone(), udid.clone()).await
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
            | ControlMessage::MultiTouch { phase, .. }
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
            session.send_multitouch(
                x1.clamp(0.0, 1.0),
                y1.clamp(0.0, 1.0),
                x2.clamp(0.0, 1.0),
                y2.clamp(0.0, 1.0),
                &phase,
            )
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
        ControlMessage::DismissKeyboard => session.send_key(41, 0),
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

pub(crate) async fn run_toggle_appearance_control(
    bridge: NativeBridge,
    udid: String,
) -> Result<(), AppError> {
    task::spawn_blocking(move || bridge.toggle_appearance(&udid))
        .await
        .map_err(|error| AppError::internal(format!("Failed to join control task: {error}")))?
}

async fn send_key(
    State(state): State<AppState>,
    Path(udid): Path<String>,
    Json(payload): Json<KeyPayload>,
) -> Result<Json<Value>, AppError> {
    if android::is_android_id(&udid) {
        run_android_action(state, move |android| {
            android.send_key(&udid, payload.key_code, payload.modifiers.unwrap_or(0))
        })
        .await?;
        return Ok(json(json_value!({ "ok": true })));
    }
    run_bridge_action(state, move |bridge| {
        bridge.send_key(&udid, payload.key_code, payload.modifiers.unwrap_or(0))
    })
    .await?;
    Ok(json(json_value!({ "ok": true })))
}

async fn send_key_sequence(
    State(state): State<AppState>,
    Path(udid): Path<String>,
    Json(payload): Json<KeySequencePayload>,
) -> Result<Json<Value>, AppError> {
    if payload.key_codes.is_empty() {
        return Err(AppError::bad_request(
            "Request body must include at least one key code.",
        ));
    }
    if payload.key_codes.len() > 512 {
        return Err(AppError::bad_request(
            "Key sequence cannot contain more than 512 key codes.",
        ));
    }
    if android::is_android_id(&udid) {
        run_android_action(state, move |android| {
            let delay_ms = payload.delay_ms.unwrap_or(0);
            let key_count = payload.key_codes.len();
            for (index, key_code) in payload.key_codes.into_iter().enumerate() {
                android.send_key(&udid, key_code, 0)?;
                if delay_ms > 0 && index + 1 < key_count {
                    std::thread::sleep(Duration::from_millis(delay_ms));
                }
            }
            Ok(())
        })
        .await?;
        return Ok(json(json_value!({ "ok": true })));
    }
    run_bridge_action(state, move |bridge| {
        let input = bridge.create_input_session(&udid)?;
        let delay_ms = payload.delay_ms.unwrap_or(0);
        let key_count = payload.key_codes.len();
        for (index, key_code) in payload.key_codes.into_iter().enumerate() {
            input.send_key(key_code, 0)?;
            if delay_ms > 0 && index + 1 < key_count {
                std::thread::sleep(Duration::from_millis(delay_ms));
            }
        }
        Ok(())
    })
    .await?;
    Ok(json(json_value!({ "ok": true })))
}

async fn dismiss_keyboard(
    State(state): State<AppState>,
    Path(udid): Path<String>,
) -> Result<Json<Value>, AppError> {
    if android::is_android_id(&udid) {
        run_android_action(state, move |android| android.dismiss_keyboard(&udid)).await?;
        return Ok(json(json_value!({ "ok": true })));
    }
    run_bridge_action(state, move |bridge| bridge.send_key(&udid, 41, 0)).await?;
    Ok(json(json_value!({ "ok": true })))
}

async fn press_button(
    State(state): State<AppState>,
    Path(udid): Path<String>,
    Json(payload): Json<ButtonPayload>,
) -> Result<Json<Value>, AppError> {
    if payload.button.trim().is_empty() {
        return Err(AppError::bad_request("Request body must include `button`."));
    }
    if android::is_android_id(&udid) {
        run_android_action(state, move |android| {
            android.press_button(&udid, &payload.button, payload.duration_ms.unwrap_or(0))
        })
        .await?;
        return Ok(json(json_value!({ "ok": true })));
    }
    if let Some(phase) = payload.phase.as_deref() {
        let pressed = match phase {
            "down" | "began" => true,
            "up" | "ended" | "cancelled" => false,
            _ => {
                return Err(AppError::bad_request(
                    "`phase` must be `down`, `up`, `began`, `ended`, or `cancelled`.",
                ))
            }
        };
        run_bridge_action(state, move |bridge| {
            bridge.send_button(
                &udid,
                &payload.button,
                pressed,
                payload.usage_page,
                payload.usage,
            )
        })
        .await?;
    } else {
        run_bridge_action(state, move |bridge| {
            bridge.press_button(&udid, &payload.button, payload.duration_ms.unwrap_or(0))
        })
        .await?;
    }
    Ok(json(json_value!({ "ok": true })))
}

async fn rotate_crown(
    State(state): State<AppState>,
    Path(udid): Path<String>,
    Json(payload): Json<CrownPayload>,
) -> Result<Json<Value>, AppError> {
    if !payload.delta.is_finite() {
        return Err(AppError::bad_request(
            "Request body must include finite `delta`.",
        ));
    }
    run_bridge_action(state, move |bridge| {
        bridge.rotate_crown(&udid, payload.delta)
    })
    .await?;
    Ok(json(json_value!({ "ok": true })))
}

async fn press_home(
    State(state): State<AppState>,
    Path(udid): Path<String>,
) -> Result<Json<Value>, AppError> {
    if android::is_android_id(&udid) {
        run_android_action(state, move |android| android.press_home(&udid)).await?;
        return Ok(json(json_value!({ "ok": true })));
    }
    run_bridge_action(state, move |bridge| bridge.press_home(&udid)).await?;
    Ok(json(json_value!({ "ok": true })))
}

async fn open_app_switcher(
    State(state): State<AppState>,
    Path(udid): Path<String>,
) -> Result<Json<Value>, AppError> {
    if android::is_android_id(&udid) {
        run_android_action(state, move |android| android.open_app_switcher(&udid)).await?;
        return Ok(json(json_value!({ "ok": true })));
    }
    run_bridge_action(state, move |bridge| bridge.open_app_switcher(&udid)).await?;
    Ok(json(json_value!({ "ok": true })))
}

async fn rotate_right(
    State(state): State<AppState>,
    Path(udid): Path<String>,
) -> Result<Json<Value>, AppError> {
    if android::is_android_id(&udid) {
        run_android_action(state, move |android| android.rotate_right(&udid)).await?;
        return Ok(json(json_value!({ "ok": true })));
    }
    run_bridge_action(state, move |bridge| bridge.rotate_right(&udid)).await?;
    Ok(json(json_value!({ "ok": true })))
}

async fn rotate_left(
    State(state): State<AppState>,
    Path(udid): Path<String>,
) -> Result<Json<Value>, AppError> {
    if android::is_android_id(&udid) {
        run_android_action(state, move |android| android.rotate_left(&udid)).await?;
        return Ok(json(json_value!({ "ok": true })));
    }
    run_bridge_action(state, move |bridge| bridge.rotate_left(&udid)).await?;
    Ok(json(json_value!({ "ok": true })))
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
    Ok(json(
        accessibility_tree_value(
            state,
            udid,
            query.source.as_deref(),
            query.max_depth,
            query.include_hidden.unwrap_or(false),
        )
        .await?,
    ))
}

async fn accessibility_tree_value(
    state: AppState,
    udid: String,
    source: Option<&str>,
    max_depth: Option<usize>,
    include_hidden: bool,
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
            Ok(tree)
        })
        .await;
    }
    let requested_source = AccessibilityHierarchySource::parse(source)?;
    let max_depth = max_depth.map(|depth| depth.min(80));

    if requested_source == AccessibilityHierarchySource::NativeAX {
        let inspector_session = inspector_session_for_state(&state, &udid).await.ok();
        let mut available_sources = available_sources_with_native_ax(inspector_session.as_ref());
        let native_snapshot =
            match accessibility_snapshot(state.clone(), udid.clone(), None, max_depth).await {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    return Ok(empty_accessibility_tree(
                        SOURCE_NATIVE_AX,
                        &available_sources,
                        suppress_native_ax_translation_error(&error.to_string()),
                    ));
                }
            };
        merge_connected_sources_for_pid(
            &state,
            &udid,
            root_process_identifier(&native_snapshot),
            &mut available_sources,
        )
        .await;
        let snapshot = attach_available_sources(native_snapshot, &available_sources);
        return Ok(snapshot);
    }

    match inspector_session_for_state(&state, &udid).await {
        Ok(session) => {
            let hierarchy_source = match requested_source {
                AccessibilityHierarchySource::Auto => InAppHierarchySource::Automatic,
                AccessibilityHierarchySource::NativeScript => InAppHierarchySource::Automatic,
                AccessibilityHierarchySource::ReactNative => InAppHierarchySource::Automatic,
                AccessibilityHierarchySource::Flutter => InAppHierarchySource::Automatic,
                AccessibilityHierarchySource::SwiftUI => InAppHierarchySource::Automatic,
                AccessibilityHierarchySource::UIKit => InAppHierarchySource::UIKit,
                AccessibilityHierarchySource::NativeAX => unreachable!(),
            };
            match run_in_app_inspector_hierarchy(
                &state,
                &session,
                hierarchy_source,
                max_depth,
                include_hidden,
            )
            .await
            {
                Ok(snapshot) => {
                    let base_sources = available_sources_with_native_ax(Some(&session));
                    let available_sources =
                        available_sources_for_snapshot(&base_sources, &snapshot);
                    let snapshot_source = snapshot.get("source").and_then(Value::as_str);
                    let fallback_reason = if requested_source
                        == AccessibilityHierarchySource::NativeScript
                        && snapshot_source != Some(SOURCE_NATIVE_SCRIPT)
                    {
                        Some("NativeScript hierarchy is not published by the app.".to_owned())
                    } else if requested_source == AccessibilityHierarchySource::ReactNative
                        && snapshot_source != Some(SOURCE_REACT_NATIVE)
                    {
                        Some("React Native hierarchy is not published by the app.".to_owned())
                    } else if requested_source == AccessibilityHierarchySource::Flutter
                        && snapshot_source != Some(SOURCE_FLUTTER)
                    {
                        Some("Flutter hierarchy is not published by the app.".to_owned())
                    } else if requested_source == AccessibilityHierarchySource::SwiftUI
                        && snapshot_source != Some(SOURCE_SWIFTUI)
                    {
                        Some("SwiftUI hierarchy is not published by the app.".to_owned())
                    } else {
                        None
                    };
                    Ok(attach_tree_metadata(
                        snapshot,
                        &available_sources,
                        fallback_reason,
                    ))
                }
                Err(_inspector_error) => {
                    let mut available_sources = available_sources_with_native_ax(Some(&session));
                    if requested_source == AccessibilityHierarchySource::UIKit {
                        if let Ok(snapshot) = run_in_app_inspector_hierarchy(
                            &state,
                            &session,
                            InAppHierarchySource::Automatic,
                            Some(0),
                            include_hidden,
                        )
                        .await
                        {
                            available_sources =
                                available_sources_for_snapshot(&available_sources, &snapshot);
                        }
                    }
                    match accessibility_snapshot(state.clone(), udid.clone(), None, max_depth).await
                    {
                        Ok(native_snapshot) => Ok(attach_available_sources(
                            trim_tree_depth(native_snapshot, max_depth),
                            &available_sources,
                        )),
                        Err(native_ax_error) => Ok(empty_accessibility_tree(
                            SOURCE_NATIVE_AX,
                            &available_sources,
                            suppress_native_ax_translation_error(&native_ax_error.to_string()),
                        )),
                    }
                }
            }
        }
        Err(_inspector_error) => {
            let available_sources = available_sources_with_native_ax(None);
            match accessibility_snapshot(state.clone(), udid.clone(), None, max_depth).await {
                Ok(native_snapshot) => Ok(attach_available_sources(
                    trim_tree_depth(native_snapshot, max_depth),
                    &available_sources,
                )),
                Err(native_ax_error) => Ok(empty_accessibility_tree(
                    SOURCE_NATIVE_AX,
                    &available_sources,
                    suppress_native_ax_translation_error(&native_ax_error.to_string()),
                )),
            }
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
    let snapshot = accessibility_snapshot(state, udid, Some((query.x, query.y)), None).await?;
    Ok(json(snapshot))
}

async fn perform_tap_payload(
    state: AppState,
    udid: String,
    payload: TapElementPayload,
) -> Result<(), AppError> {
    let duration_ms = payload.duration_ms.unwrap_or(60);
    let (x, y) = if selector_is_empty(&payload.selector) {
        let x = payload
            .x
            .ok_or_else(|| AppError::bad_request("Tap requires `x` and `y` or a selector."))?;
        let y = payload
            .y
            .ok_or_else(|| AppError::bad_request("Tap requires `x` and `y` or a selector."))?;
        if !x.is_finite() || !y.is_finite() || x < 0.0 || y < 0.0 {
            return Err(AppError::bad_request(
                "Tap coordinates must be finite non-negative numbers.",
            ));
        }
        if payload.normalized.unwrap_or(true) {
            (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0))
        } else {
            let snapshot = accessibility_tree_value(
                state.clone(),
                udid.clone(),
                payload.source.as_deref(),
                payload.max_depth,
                payload.include_hidden.unwrap_or(false),
            )
            .await?;
            normalize_screen_point_from_snapshot(&snapshot, x, y)?
        }
    } else {
        let wait_payload = WaitForPayload {
            selector: payload.selector.clone(),
            source: payload.source.clone(),
            max_depth: payload.max_depth,
            include_hidden: payload.include_hidden,
            timeout_ms: payload.wait_timeout_ms,
            poll_ms: payload.poll_ms,
        };
        let snapshot = wait_for_snapshot_match(state.clone(), udid.clone(), wait_payload).await?;
        tap_point_from_snapshot(&snapshot, &payload.selector)?
    };

    if android::is_android_id(&udid) {
        return run_android_action(state, move |android| {
            android.send_touch(&udid, x, y, "began")?;
            if duration_ms > 0 {
                std::thread::sleep(Duration::from_millis(duration_ms));
            }
            android.send_touch(&udid, x, y, "ended")
        })
        .await;
    }

    run_bridge_action(state, move |bridge| {
        let input = bridge.create_input_session(&udid)?;
        input.send_touch(x, y, "began")?;
        if duration_ms > 0 {
            std::thread::sleep(Duration::from_millis(duration_ms));
        }
        input.send_touch(x, y, "ended")
    })
    .await
}

async fn wait_for_element_payload(
    state: AppState,
    udid: String,
    payload: WaitForPayload,
) -> Result<Json<Value>, AppError> {
    let started = Instant::now();
    let snapshot = wait_for_snapshot_match(state, udid, payload.clone()).await?;
    let found = first_matching_element(&snapshot, &payload.selector)
        .ok_or_else(|| AppError::not_found("No accessibility element matched."))?;
    Ok(json(json_value!({
        "ok": true,
        "elapsedMs": started.elapsed().as_millis() as u64,
        "match": compact_accessibility_node(&found),
    })))
}

async fn wait_for_snapshot_match(
    state: AppState,
    udid: String,
    payload: WaitForPayload,
) -> Result<Value, AppError> {
    let timeout_ms = payload.timeout_ms.unwrap_or(5_000);
    let poll_ms = payload.poll_ms.unwrap_or(100).max(10);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        let snapshot = accessibility_tree_value(
            state.clone(),
            udid.clone(),
            payload.source.as_deref(),
            payload.max_depth,
            payload.include_hidden.unwrap_or(false),
        )
        .await?;
        if first_matching_element(&snapshot, &payload.selector).is_some() {
            return Ok(snapshot);
        }
        if timeout_ms == 0 || Instant::now() >= deadline {
            return Err(AppError::not_found("No accessibility element matched."));
        }
        tokio::time::sleep(Duration::from_millis(poll_ms)).await;
    }
}

async fn run_batch_step(state: AppState, udid: String, step: BatchStep) -> Result<Value, AppError> {
    match step {
        BatchStep::Sleep { ms, seconds } => {
            let duration =
                ms.unwrap_or_else(|| ((seconds.unwrap_or(0.0) * 1000.0).max(0.0)) as u64);
            tokio::time::sleep(Duration::from_millis(duration)).await;
            Ok(json_value!({ "action": "sleep", "durationMs": duration }))
        }
        BatchStep::Tap(payload) => {
            perform_tap_payload(state, udid, payload).await?;
            Ok(json_value!({ "action": "tap" }))
        }
        BatchStep::WaitFor(payload) => {
            let snapshot = wait_for_snapshot_match(state, udid, payload.clone()).await?;
            let found = first_matching_element(&snapshot, &payload.selector)
                .ok_or_else(|| AppError::not_found("No accessibility element matched."))?;
            Ok(json_value!({ "action": "waitFor", "match": compact_accessibility_node(&found) }))
        }
        BatchStep::Assert(payload) => {
            let snapshot = wait_for_snapshot_match(state, udid, payload.clone()).await?;
            let found = first_matching_element(&snapshot, &payload.selector)
                .ok_or_else(|| AppError::not_found("No accessibility element matched."))?;
            Ok(json_value!({ "action": "assert", "match": compact_accessibility_node(&found) }))
        }
        BatchStep::Key {
            key_code,
            modifiers,
        } => {
            if android::is_android_id(&udid) {
                run_android_action(state, move |android| {
                    android.send_key(&udid, key_code, modifiers.unwrap_or(0))
                })
                .await?;
                return Ok(json_value!({ "action": "key" }));
            }
            run_bridge_action(state, move |bridge| {
                bridge.send_key(&udid, key_code, modifiers.unwrap_or(0))
            })
            .await?;
            Ok(json_value!({ "action": "key" }))
        }
        BatchStep::KeySequence {
            key_codes,
            delay_ms,
        } => {
            if key_codes.is_empty() {
                return Err(AppError::bad_request("keySequence requires keyCodes."));
            }
            if key_codes.len() > 512 {
                return Err(AppError::bad_request(
                    "keySequence cannot contain more than 512 key codes.",
                ));
            }
            if android::is_android_id(&udid) {
                run_android_action(state, move |android| {
                    let delay_ms = delay_ms.unwrap_or(0);
                    let key_count = key_codes.len();
                    for (index, key_code) in key_codes.into_iter().enumerate() {
                        android.send_key(&udid, key_code, 0)?;
                        if delay_ms > 0 && index + 1 < key_count {
                            std::thread::sleep(Duration::from_millis(delay_ms));
                        }
                    }
                    Ok(())
                })
                .await?;
                return Ok(json_value!({ "action": "keySequence" }));
            }
            run_bridge_action(state, move |bridge| {
                let input = bridge.create_input_session(&udid)?;
                let delay_ms = delay_ms.unwrap_or(0);
                let key_count = key_codes.len();
                for (index, key_code) in key_codes.into_iter().enumerate() {
                    input.send_key(key_code, 0)?;
                    if delay_ms > 0 && index + 1 < key_count {
                        std::thread::sleep(Duration::from_millis(delay_ms));
                    }
                }
                Ok(())
            })
            .await?;
            Ok(json_value!({ "action": "keySequence" }))
        }
        BatchStep::Touch {
            x,
            y,
            phase,
            down,
            up,
            delay_ms,
        } => {
            if !x.is_finite() || !y.is_finite() {
                return Err(AppError::bad_request(
                    "touch requires finite normalized x and y.",
                ));
            }
            if android::is_android_id(&udid) {
                run_android_action(state, move |android| {
                    let x = x.clamp(0.0, 1.0);
                    let y = y.clamp(0.0, 1.0);
                    if down.unwrap_or(false) || up.unwrap_or(false) {
                        if down.unwrap_or(false) {
                            android.send_touch(&udid, x, y, "began")?;
                        }
                        if down.unwrap_or(false) && up.unwrap_or(false) {
                            std::thread::sleep(Duration::from_millis(delay_ms.unwrap_or(100)));
                        }
                        if up.unwrap_or(false) {
                            android.send_touch(&udid, x, y, "ended")?;
                        }
                    } else {
                        android.send_touch(&udid, x, y, phase.as_deref().unwrap_or("began"))?;
                    }
                    Ok(())
                })
                .await?;
                return Ok(json_value!({ "action": "touch" }));
            }
            run_bridge_action(state, move |bridge| {
                let input = bridge.create_input_session(&udid)?;
                let x = x.clamp(0.0, 1.0);
                let y = y.clamp(0.0, 1.0);
                if down.unwrap_or(false) || up.unwrap_or(false) {
                    if down.unwrap_or(false) {
                        input.send_touch(x, y, "began")?;
                    }
                    if down.unwrap_or(false) && up.unwrap_or(false) {
                        std::thread::sleep(Duration::from_millis(delay_ms.unwrap_or(100)));
                    }
                    if up.unwrap_or(false) {
                        input.send_touch(x, y, "ended")?;
                    }
                } else {
                    input.send_touch(x, y, phase.as_deref().unwrap_or("began"))?;
                }
                Ok(())
            })
            .await?;
            Ok(json_value!({ "action": "touch" }))
        }
        BatchStep::TouchSequence { events } => {
            if events.is_empty() {
                return Err(AppError::bad_request("touchSequence requires events."));
            }
            if events.len() > 64 {
                return Err(AppError::bad_request(
                    "touchSequence cannot contain more than 64 events.",
                ));
            }
            if android::is_android_id(&udid) {
                run_android_action(state, move |android| {
                    for event in events {
                        if !event.x.is_finite() || !event.y.is_finite() {
                            return Err(AppError::bad_request(
                                "touchSequence requires finite normalized x and y.",
                            ));
                        }
                        android.send_touch(
                            &udid,
                            event.x.clamp(0.0, 1.0),
                            event.y.clamp(0.0, 1.0),
                            &event.phase,
                        )?;
                        if let Some(delay_ms) =
                            event.delay_ms_after.filter(|delay_ms| *delay_ms > 0)
                        {
                            std::thread::sleep(Duration::from_millis(delay_ms));
                        }
                    }
                    Ok(())
                })
                .await?;
                return Ok(json_value!({ "action": "touchSequence" }));
            }
            run_bridge_action(state, move |bridge| {
                let input = bridge.create_input_session(&udid)?;
                for event in events {
                    if !event.x.is_finite() || !event.y.is_finite() {
                        return Err(AppError::bad_request(
                            "touchSequence requires finite normalized x and y.",
                        ));
                    }
                    input.send_touch(
                        event.x.clamp(0.0, 1.0),
                        event.y.clamp(0.0, 1.0),
                        &event.phase,
                    )?;
                    if let Some(delay_ms) = event.delay_ms_after.filter(|delay_ms| *delay_ms > 0) {
                        std::thread::sleep(Duration::from_millis(delay_ms));
                    }
                }
                Ok(())
            })
            .await?;
            Ok(json_value!({ "action": "touchSequence" }))
        }
        BatchStep::Swipe {
            start_x,
            start_y,
            end_x,
            end_y,
            duration_ms,
            steps,
        } => {
            if !start_x.is_finite()
                || !start_y.is_finite()
                || !end_x.is_finite()
                || !end_y.is_finite()
            {
                return Err(AppError::bad_request(
                    "swipe requires finite normalized coordinates.",
                ));
            }
            if android::is_android_id(&udid) {
                run_android_action(state, move |android| {
                    android.send_swipe(
                        &udid,
                        start_x,
                        start_y,
                        end_x,
                        end_y,
                        duration_ms.unwrap_or(350),
                    )
                })
                .await?;
                return Ok(json_value!({ "action": "swipe" }));
            }
            run_bridge_action(state, move |bridge| {
                let step_count = steps.unwrap_or(12).max(1);
                let delay =
                    Duration::from_millis(duration_ms.unwrap_or(350) / u64::from(step_count));
                let input = bridge.create_input_session(&udid)?;
                let start_x = start_x.clamp(0.0, 1.0);
                let start_y = start_y.clamp(0.0, 1.0);
                let end_x = end_x.clamp(0.0, 1.0);
                let end_y = end_y.clamp(0.0, 1.0);
                input.send_touch(start_x, start_y, "began")?;
                for step in 1..step_count {
                    let t = f64::from(step) / f64::from(step_count);
                    input.send_touch(
                        start_x + (end_x - start_x) * t,
                        start_y + (end_y - start_y) * t,
                        "moved",
                    )?;
                    std::thread::sleep(delay);
                }
                input.send_touch(end_x, end_y, "ended")
            })
            .await?;
            Ok(json_value!({ "action": "swipe" }))
        }
        BatchStep::Gesture {
            preset,
            duration_ms,
            delta,
            steps,
        } => {
            let (start_x, start_y, end_x, end_y, default_duration_ms) =
                normalized_gesture_coordinates(&preset, delta)?;
            if android::is_android_id(&udid) {
                run_android_action(state, move |android| {
                    android.send_swipe(
                        &udid,
                        start_x,
                        start_y,
                        end_x,
                        end_y,
                        duration_ms.unwrap_or(default_duration_ms),
                    )
                })
                .await?;
                return Ok(json_value!({ "action": "gesture", "preset": preset }));
            }
            run_bridge_action(state, move |bridge| {
                let step_count = steps.unwrap_or(12).max(1);
                let delay = Duration::from_millis(
                    duration_ms.unwrap_or(default_duration_ms) / u64::from(step_count),
                );
                let input = bridge.create_input_session(&udid)?;
                input.send_touch(start_x, start_y, "began")?;
                for step in 1..step_count {
                    let t = f64::from(step) / f64::from(step_count);
                    input.send_touch(
                        start_x + (end_x - start_x) * t,
                        start_y + (end_y - start_y) * t,
                        "moved",
                    )?;
                    std::thread::sleep(delay);
                }
                input.send_touch(end_x, end_y, "ended")
            })
            .await?;
            Ok(json_value!({ "action": "gesture", "preset": preset }))
        }
        BatchStep::Type { text, delay_ms } => {
            if android::is_android_id(&udid) {
                run_android_action(state, move |android| {
                    if delay_ms.is_some() {
                        for character in text.chars() {
                            android.type_text(&udid, &character.to_string())?;
                            if let Some(delay_ms) = delay_ms.filter(|delay_ms| *delay_ms > 0) {
                                std::thread::sleep(Duration::from_millis(delay_ms));
                            }
                        }
                        Ok(())
                    } else {
                        android.type_text(&udid, &text)
                    }
                })
                .await?;
                return Ok(json_value!({ "action": "type" }));
            }
            run_bridge_action(state, move |bridge| {
                let input = bridge.create_input_session(&udid)?;
                for character in text.chars() {
                    let Some((key_code, modifiers)) = hid_for_character(character) else {
                        return Err(AppError::bad_request(format!(
                            "Unsupported character for HID typing: {character:?}"
                        )));
                    };
                    input.send_key(key_code, modifiers)?;
                    if let Some(delay_ms) = delay_ms.filter(|delay_ms| *delay_ms > 0) {
                        std::thread::sleep(Duration::from_millis(delay_ms));
                    }
                }
                Ok(())
            })
            .await?;
            Ok(json_value!({ "action": "type" }))
        }
        BatchStep::Button {
            button,
            duration_ms,
        } => {
            if android::is_android_id(&udid) {
                run_android_action(state, move |android| {
                    android.press_button(&udid, &button, duration_ms.unwrap_or(0))
                })
                .await?;
                return Ok(json_value!({ "action": "button" }));
            }
            run_bridge_action(state, move |bridge| {
                bridge.press_button(&udid, &button, duration_ms.unwrap_or(0))
            })
            .await?;
            Ok(json_value!({ "action": "button" }))
        }
        BatchStep::Crown { delta } => {
            run_bridge_action(state, move |bridge| bridge.rotate_crown(&udid, delta)).await?;
            Ok(json_value!({ "action": "crown" }))
        }
        BatchStep::Launch { bundle_id } => {
            if android::is_android_id(&udid) {
                run_android_action(state, move |android| {
                    android.launch_package(&udid, &bundle_id)
                })
                .await?;
                return Ok(json_value!({ "action": "launch" }));
            }
            run_bridge_action(state, move |bridge| bridge.launch_bundle(&udid, &bundle_id)).await?;
            Ok(json_value!({ "action": "launch" }))
        }
        BatchStep::OpenUrl { url } => {
            if android::is_android_id(&udid) {
                run_android_action(state, move |android| android.open_url(&udid, &url)).await?;
                return Ok(json_value!({ "action": "openUrl" }));
            }
            run_bridge_action(state, move |bridge| bridge.open_url(&udid, &url)).await?;
            Ok(json_value!({ "action": "openUrl" }))
        }
        BatchStep::Home => {
            if android::is_android_id(&udid) {
                run_android_action(state, move |android| android.press_home(&udid)).await?;
                return Ok(json_value!({ "action": "home" }));
            }
            run_bridge_action(state, move |bridge| bridge.press_home(&udid)).await?;
            Ok(json_value!({ "action": "home" }))
        }
        BatchStep::DismissKeyboard => {
            if android::is_android_id(&udid) {
                run_android_action(state, move |android| android.dismiss_keyboard(&udid)).await?;
                return Ok(json_value!({ "action": "dismissKeyboard" }));
            }
            run_bridge_action(state, move |bridge| bridge.send_key(&udid, 41, 0)).await?;
            Ok(json_value!({ "action": "dismissKeyboard" }))
        }
        BatchStep::AppSwitcher => {
            if android::is_android_id(&udid) {
                run_android_action(state, move |android| android.open_app_switcher(&udid)).await?;
                return Ok(json_value!({ "action": "appSwitcher" }));
            }
            run_bridge_action(state, move |bridge| bridge.open_app_switcher(&udid)).await?;
            Ok(json_value!({ "action": "appSwitcher" }))
        }
        BatchStep::RotateLeft => {
            if android::is_android_id(&udid) {
                run_android_action(state, move |android| android.rotate_left(&udid)).await?;
                return Ok(json_value!({ "action": "rotateLeft" }));
            }
            run_bridge_action(state, move |bridge| bridge.rotate_left(&udid)).await?;
            Ok(json_value!({ "action": "rotateLeft" }))
        }
        BatchStep::RotateRight => {
            if android::is_android_id(&udid) {
                run_android_action(state, move |android| android.rotate_right(&udid)).await?;
                return Ok(json_value!({ "action": "rotateRight" }));
            }
            run_bridge_action(state, move |bridge| bridge.rotate_right(&udid)).await?;
            Ok(json_value!({ "action": "rotateRight" }))
        }
        BatchStep::ToggleAppearance => {
            if android::is_android_id(&udid) {
                run_android_action(state, move |android| android.toggle_appearance(&udid)).await?;
                return Ok(json_value!({ "action": "toggleAppearance" }));
            }
            run_bridge_action(state, move |bridge| bridge.toggle_appearance(&udid)).await?;
            Ok(json_value!({ "action": "toggleAppearance" }))
        }
        BatchStep::Describe {
            source,
            max_depth,
            include_hidden,
        } => {
            let snapshot = accessibility_tree_value(
                state,
                udid,
                source.as_deref(),
                max_depth,
                include_hidden.unwrap_or(false),
            )
            .await?;
            Ok(json_value!({
                "action": "describe",
                "snapshot": compact_accessibility_snapshot(&snapshot),
            }))
        }
    }
}

fn query_compact_elements(
    snapshot: &Value,
    selector: &ElementSelectorPayload,
    limit: usize,
) -> Vec<Value> {
    let mut matches = Vec::new();
    if let Some(roots) = snapshot.get("roots").and_then(Value::as_array) {
        for root in roots {
            collect_query_matches(root, selector, limit, &mut matches);
            if matches.len() >= limit {
                break;
            }
        }
    }
    matches
}

fn collect_query_matches(
    node: &Value,
    selector: &ElementSelectorPayload,
    limit: usize,
    matches: &mut Vec<Value>,
) {
    if matches.len() >= limit {
        return;
    }
    if element_matches_selector(node, selector) {
        matches.push(compact_accessibility_node(node));
    }
    if let Some(children) = node.get("children").and_then(Value::as_array) {
        for child in children {
            collect_query_matches(child, selector, limit, matches);
            if matches.len() >= limit {
                return;
            }
        }
    }
}

fn first_matching_element(snapshot: &Value, selector: &ElementSelectorPayload) -> Option<Value> {
    let roots = snapshot.get("roots")?.as_array()?;
    for root in roots {
        if let Some(found) = first_matching_node(root, selector) {
            return Some(found.clone());
        }
    }
    None
}

fn first_matching_node<'a>(
    node: &'a Value,
    selector: &ElementSelectorPayload,
) -> Option<&'a Value> {
    if element_matches_selector(node, selector) {
        return Some(node);
    }
    for child in node
        .get("children")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(found) = first_matching_node(child, selector) {
            return Some(found);
        }
    }
    None
}

fn element_matches_selector(node: &Value, selector: &ElementSelectorPayload) -> bool {
    if selector_is_empty(selector) {
        return true;
    }
    selector
        .element_type
        .as_ref()
        .is_none_or(|expected| string_fields_match(node, expected, &["type", "role", "className"]))
        && selector.id.as_ref().is_none_or(|expected| {
            string_fields_match(
                node,
                expected,
                &[
                    "AXIdentifier",
                    "AXUniqueId",
                    "inspectorId",
                    "id",
                    "identifier",
                ],
            )
        })
        && selector.label.as_ref().is_none_or(|expected| {
            string_fields_match(
                node,
                expected,
                &["AXLabel", "label", "title", "text", "name"],
            )
        })
        && selector
            .value
            .as_ref()
            .is_none_or(|expected| string_fields_match(node, expected, &["AXValue", "value"]))
}

fn selector_is_empty(selector: &ElementSelectorPayload) -> bool {
    selector.id.is_none()
        && selector.label.is_none()
        && selector.value.is_none()
        && selector.element_type.is_none()
}

fn string_fields_match(node: &Value, expected: &str, fields: &[&str]) -> bool {
    fields
        .iter()
        .filter_map(|field| node.get(*field).and_then(Value::as_str))
        .any(|value| value == expected)
}

fn tap_point_from_snapshot(
    snapshot: &Value,
    selector: &ElementSelectorPayload,
) -> Result<(f64, f64), AppError> {
    let roots = snapshot
        .get("roots")
        .and_then(Value::as_array)
        .ok_or_else(|| AppError::not_found("Accessibility snapshot does not contain roots."))?;
    for root in roots {
        let root_frame = root
            .get("frame")
            .or_else(|| root.get("frameInScreen"))
            .ok_or_else(|| AppError::not_found("Accessibility root does not expose a frame."))?;
        let root_width = number_field(root_frame, "width")?;
        let root_height = number_field(root_frame, "height")?;
        if let Some(node) = first_matching_node(root, selector) {
            let frame = node
                .get("frame")
                .or_else(|| node.get("frameInScreen"))
                .ok_or_else(|| AppError::not_found("Matched element does not expose a frame."))?;
            let x = number_field(frame, "x")? + number_field(frame, "width")? / 2.0;
            let y = number_field(frame, "y")? + number_field(frame, "height")? / 2.0;
            return Ok((
                (x / root_width).clamp(0.0, 1.0),
                (y / root_height).clamp(0.0, 1.0),
            ));
        }
    }
    Err(AppError::not_found("No accessibility element matched."))
}

fn normalize_screen_point_from_snapshot(
    snapshot: &Value,
    x: f64,
    y: f64,
) -> Result<(f64, f64), AppError> {
    let root = snapshot
        .get("roots")
        .and_then(Value::as_array)
        .and_then(|roots| roots.first())
        .ok_or_else(|| AppError::not_found("Accessibility snapshot does not contain a root."))?;
    let frame = root
        .get("frame")
        .or_else(|| root.get("frameInScreen"))
        .ok_or_else(|| AppError::not_found("Accessibility root does not expose a frame."))?;
    let width = number_field(frame, "width")?;
    let height = number_field(frame, "height")?;
    if width <= 0.0 || height <= 0.0 {
        return Err(AppError::not_found("Accessibility root frame is empty."));
    }
    Ok(((x / width).clamp(0.0, 1.0), (y / height).clamp(0.0, 1.0)))
}

fn accessibility_point_snapshot(snapshot: &Value, x: f64, y: f64) -> Result<Value, AppError> {
    let roots = snapshot
        .get("roots")
        .and_then(Value::as_array)
        .ok_or_else(|| AppError::not_found("Accessibility snapshot does not contain roots."))?;
    let node = roots
        .iter()
        .rev()
        .find_map(|root| deepest_node_at_point(root, x, y))
        .ok_or_else(|| AppError::not_found("No accessibility element contains the point."))?;
    let mut node = node.clone();
    if let Some(object) = node.as_object_mut() {
        object.remove("children");
    }

    let mut response = Map::new();
    for key in [
        "source",
        "availableSources",
        "requestedSource",
        "fallbackReason",
        "inspector",
        "includeHidden",
    ] {
        if let Some(value) = snapshot.get(key) {
            response.insert(key.to_owned(), value.clone());
        }
    }
    response.insert("roots".to_owned(), Value::Array(vec![node]));
    Ok(Value::Object(response))
}

fn deepest_node_at_point(node: &Value, x: f64, y: f64) -> Option<&Value> {
    let has_frame = node
        .get("frame")
        .or_else(|| node.get("frameInScreen"))
        .is_some();
    if has_frame && !node_frame_contains_point(node, x, y).unwrap_or(false) {
        return None;
    }
    for child in node
        .get("children")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .rev()
    {
        if let Some(found) = deepest_node_at_point(child, x, y) {
            return Some(found);
        }
    }
    has_frame.then_some(node)
}

fn node_frame_contains_point(node: &Value, x: f64, y: f64) -> Result<bool, AppError> {
    let frame = node
        .get("frame")
        .or_else(|| node.get("frameInScreen"))
        .ok_or_else(|| AppError::not_found("Accessibility node does not expose a frame."))?;
    let frame_x = number_field(frame, "x")?;
    let frame_y = number_field(frame, "y")?;
    let width = number_field(frame, "width")?;
    let height = number_field(frame, "height")?;
    Ok(width > 0.0
        && height > 0.0
        && x >= frame_x
        && y >= frame_y
        && x < frame_x + width
        && y < frame_y + height)
}

fn number_field(value: &Value, field: &str) -> Result<f64, AppError> {
    value
        .get(field)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
        .ok_or_else(|| AppError::not_found(format!("Missing numeric frame field `{field}`.")))
}

fn hid_for_character(character: char) -> Option<(u16, u32)> {
    let shift: u32 = 1;
    let mapping = match character {
        'a'..='z' => (character as u16 - b'a' as u16 + 4, 0),
        'A'..='Z' => (character as u16 - b'A' as u16 + 4, shift),
        '1' => (30, 0),
        '!' => (30, shift),
        '2' => (31, 0),
        '@' => (31, shift),
        '3' => (32, 0),
        '#' => (32, shift),
        '4' => (33, 0),
        '$' => (33, shift),
        '5' => (34, 0),
        '%' => (34, shift),
        '6' => (35, 0),
        '^' => (35, shift),
        '7' => (36, 0),
        '&' => (36, shift),
        '8' => (37, 0),
        '*' => (37, shift),
        '9' => (38, 0),
        '(' => (38, shift),
        '0' => (39, 0),
        ')' => (39, shift),
        '\n' | '\r' => (40, 0),
        '\t' => (43, 0),
        ' ' => (44, 0),
        '-' => (45, 0),
        '_' => (45, shift),
        '=' => (46, 0),
        '+' => (46, shift),
        '[' => (47, 0),
        '{' => (47, shift),
        ']' => (48, 0),
        '}' => (48, shift),
        '\\' => (49, 0),
        '|' => (49, shift),
        ';' => (51, 0),
        ':' => (51, shift),
        '\'' => (52, 0),
        '"' => (52, shift),
        '`' => (53, 0),
        '~' => (53, shift),
        ',' => (54, 0),
        '<' => (54, shift),
        '.' => (55, 0),
        '>' => (55, shift),
        '/' => (56, 0),
        '?' => (56, shift),
        _ => return None,
    };
    Some(mapping)
}

fn normalized_gesture_coordinates(
    preset: &str,
    delta: Option<f64>,
) -> Result<(f64, f64, f64, f64, u64), AppError> {
    let center_x = 0.5;
    let center_y = 0.5;
    let distance = delta.unwrap_or(0.45).clamp(0.05, 0.95);
    let edge = 0.02;
    let coordinates = match preset {
        "scroll-up" => (
            center_x,
            center_y - distance / 2.0,
            center_x,
            center_y + distance / 2.0,
            500,
        ),
        "scroll-down" => (
            center_x,
            center_y + distance / 2.0,
            center_x,
            center_y - distance / 2.0,
            500,
        ),
        "scroll-left" => (
            center_x - distance / 2.0,
            center_y,
            center_x + distance / 2.0,
            center_y,
            500,
        ),
        "scroll-right" => (
            center_x + distance / 2.0,
            center_y,
            center_x - distance / 2.0,
            center_y,
            500,
        ),
        "swipe-from-left-edge" => (edge, center_y, 1.0 - edge, center_y, 300),
        "swipe-from-right-edge" => (1.0 - edge, center_y, edge, center_y, 300),
        "swipe-from-top-edge" => (center_x, edge, center_x, 1.0 - edge, 300),
        "swipe-from-bottom-edge" => (center_x, 1.0 - edge, center_x, edge, 300),
        _ => {
            return Err(AppError::bad_request(format!(
                "Unsupported gesture preset `{preset}`."
            )))
        }
    };
    Ok(coordinates)
}

fn compact_accessibility_snapshot(snapshot: &Value) -> Value {
    let roots = snapshot
        .get("roots")
        .and_then(Value::as_array)
        .map(|roots| {
            roots
                .iter()
                .map(compact_accessibility_node)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    json_value!({
        "source": snapshot.get("source").cloned().unwrap_or(Value::Null),
        "roots": roots,
    })
}

fn compact_accessibility_node(node: &Value) -> Value {
    let mut object = Map::new();
    copy_first_string(node, &mut object, "role", &["type", "role", "className"]);
    copy_first_string(
        node,
        &mut object,
        "id",
        &[
            "AXIdentifier",
            "AXUniqueId",
            "inspectorId",
            "id",
            "identifier",
        ],
    );
    copy_first_string(
        node,
        &mut object,
        "label",
        &["AXLabel", "label", "title", "text", "name"],
    );
    copy_first_string(node, &mut object, "value", &["AXValue", "value"]);
    if let Some(frame) = node.get("frame").or_else(|| node.get("frameInScreen")) {
        object.insert("frame".to_owned(), frame.clone());
    }
    if let Some(children) = node.get("children").and_then(Value::as_array) {
        let children = children
            .iter()
            .map(compact_accessibility_node)
            .collect::<Vec<_>>();
        if !children.is_empty() {
            object.insert("children".to_owned(), Value::Array(children));
        }
    }
    Value::Object(object)
}

fn copy_first_string(
    source: &Value,
    target: &mut Map<String, Value>,
    output_key: &str,
    input_keys: &[&str],
) {
    if let Some(value) = input_keys
        .iter()
        .filter_map(|key| source.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .find(|value| !value.is_empty())
    {
        target.insert(output_key.to_owned(), Value::String(value.to_owned()));
    }
}

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

#[derive(Clone, Copy, PartialEq, Eq)]
enum AccessibilityHierarchySource {
    Auto,
    NativeScript,
    ReactNative,
    Flutter,
    SwiftUI,
    UIKit,
    NativeAX,
}

impl AccessibilityHierarchySource {
    fn parse(value: Option<&str>) -> Result<Self, AppError> {
        match value.unwrap_or("auto").trim().to_lowercase().as_str() {
            "" | "auto" => Ok(Self::Auto),
            "nativescript" | "ns" => Ok(Self::NativeScript),
            "react-native" | "reactnative" | "rn" => Ok(Self::ReactNative),
            "flutter" | "fl" => Ok(Self::Flutter),
            "swiftui" | "swift-ui" => Ok(Self::SwiftUI),
            "uikit" | "in-app-inspector" => Ok(Self::UIKit),
            "ax" | "native-ax" | "native-accessibility" => Ok(Self::NativeAX),
            source => Err(AppError::bad_request(format!(
                "Unsupported accessibility hierarchy source `{source}`."
            ))),
        }
    }
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
    RemoteDaemon {
        server_url: String,
        access_token: String,
    },
}

async fn inspector_session_for_state(
    state: &AppState,
    udid: &str,
) -> Result<InspectorSession, String> {
    let frontmost_pid = foreground_app_for_simulator(state, udid)
        .await
        .ok()
        .flatten()
        .map(|foreground| foreground.process_identifier);
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
        push_unique_source(&mut available_sources, &source);
    }
    InspectorSession {
        transport: InspectorSessionTransport::RemoteDaemon {
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
    let probe_points = foreground_process_probe_points(state.clone(), udid.to_owned()).await;
    let mut last_error: Option<String> = None;
    for point in probe_points {
        match timeout(
            SAFARI_ACTIVE_TAB_HINT_TIMEOUT,
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

async fn foreground_process_probe_points(state: AppState, udid: String) -> Vec<(f64, f64)> {
    let profile = run_bridge_action(state, move |bridge| bridge.chrome_profile(&udid)).await;
    let (screen_width, screen_height) = profile
        .map(|profile| (profile.screen_width, profile.screen_height))
        .unwrap_or((402.0, 874.0));
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
    if let Some(foreground) = cached_foreground_app(udid) {
        return Ok(Some(foreground));
    }

    let mut last_error: Option<String> = None;
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

fn cached_foreground_app(udid: &str) -> Option<devtools::ForegroundApp> {
    cached_foreground_app_with_ttl(udid, FOREGROUND_APP_CACHE_TTL)
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
    let services = simulator_ui_application_services(udid).await?;
    let mut best: Option<UIKitApplicationServiceDetails> = None;
    let detail_results = join_all(
        services
            .iter()
            .map(|service| ui_application_service_details(udid, service)),
    )
    .await;
    let mut skipped_details = 0_u32;
    for result in detail_results {
        let Some(details) = (match result {
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

async fn simulator_ui_application_services(
    udid: &str,
) -> Result<Vec<UIKitApplicationService>, String> {
    let output = timeout(
        Duration::from_secs(3),
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
        Duration::from_secs(3),
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
) -> Result<Value, String> {
    let max_depth = max_depth.unwrap_or(80);
    let params = match source {
        InAppHierarchySource::Automatic => json_value!({
            "includeHidden": include_hidden,
            "maxDepth": max_depth,
        }),
        InAppHierarchySource::UIKit => json_value!({
            "includeHidden": include_hidden,
            "maxDepth": max_depth,
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
        InspectorSessionTransport::RemoteDaemon {
            server_url,
            access_token,
        } => {
            query_remote_daemon_inspector(
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
    let react_native_available = info
        .get("reactNative")
        .and_then(|value| value.get("available"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
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
            SOURCE_NATIVE_SCRIPT => sources.push(SOURCE_NATIVE_SCRIPT.to_owned()),
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
        .unwrap_or(!(react_native_available || flutter_available));
    if uikit_available {
        sources.push(SOURCE_UIKIT.to_owned());
    }
    sources
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

async fn merge_connected_sources_for_pid(
    state: &AppState,
    udid: &str,
    process_identifier: Option<i64>,
    sources: &mut Vec<String>,
) {
    for inspector in state.inspectors.connected().await {
        if process_identifier.is_some_and(|pid| pid != inspector.process_identifier) {
            continue;
        }
        if inspector_process_belongs_to_udid(udid, inspector.process_identifier)
            .await
            .unwrap_or(false)
        {
            for source in inspector_available_sources(&inspector.info) {
                push_unique_source(sources, &source);
            }
        }
    }
    if sources
        .iter()
        .any(|source| source == SOURCE_REACT_NATIVE || source == SOURCE_FLUTTER)
    {
        sources.retain(|source| source != SOURCE_UIKIT);
    }
}

fn root_process_identifier(snapshot: &Value) -> Option<i64> {
    snapshot
        .get("roots")
        .and_then(Value::as_array)
        .and_then(|roots| roots.first())
        .and_then(|root| root.get("pid"))
        .and_then(Value::as_i64)
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
    let (transport_name, port, daemon_url) = match transport {
        InspectorSessionTransport::Connected => ("websocket", Value::Null, Value::Null),
        InspectorSessionTransport::Tcp { port } => {
            ("tcp+ndjson", Value::Number((*port).into()), Value::Null)
        }
        InspectorSessionTransport::RemoteDaemon { server_url, .. } => (
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
        "daemonUrl": daemon_url,
        "host": INSPECTOR_AGENT_HOST,
        "port": port,
        "processIdentifier": process_identifier,
        "protocolVersion": info.get("protocolVersion").cloned().unwrap_or(Value::Null),
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

async fn query_remote_daemon_inspector(
    server_url: &str,
    access_token: &str,
    process_identifier: i64,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let endpoint = InspectorDaemonEndpoint::parse(server_url)?;
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
    .map_err(|_| format!("Timed out connecting to published SimDeck daemon at {server_url}."))?
    .map_err(|error| {
        format!("Unable to connect to published SimDeck daemon at {server_url}: {error}")
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

struct InspectorDaemonEndpoint {
    host: String,
    port: u16,
}

impl InspectorDaemonEndpoint {
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
    let inspector_id = object_string(object, "id");
    let accessibility_label = nested_string(accessibility, "label");
    let text = object_string(object, "text");
    let placeholder = object_string(object, "placeholder");
    let swiftui_tag = nested_string(swiftui, "tag");
    let view_controller_title = nested_string(view_controller, "title");
    let image_name = object_string(object, "imageName");
    let title = first_non_empty_string([
        swiftui_tag.clone(),
        text.clone(),
        view_controller_title,
        image_name,
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
    normalized.insert(
        "source".to_owned(),
        Value::String("in-app-inspector".to_owned()),
    );

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
    if let Some(value) = nested_string(accessibility, "value").or(placeholder.clone()) {
        normalized.insert("AXValue".to_owned(), Value::String(value));
    }
    if let Some(value) = nested_string(accessibility, "hint") {
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
    copy_optional_field(object, &mut normalized, "text");
    copy_optional_field(object, &mut normalized, "placeholder");
    copy_optional_field(object, &mut normalized, "imageName");

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
    Ok(values)
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
    run_bridge_action(state, move |bridge| {
        bridge.accessibility_snapshot_with_max_depth(&udid, point, max_depth)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::{
        accessibility_point_snapshot, active_url_hint_from_accessibility_snapshot,
        attach_tree_metadata, available_sources_for_snapshot, best_inspector_session,
        chrome_devtools_source_for_session, client_stats_foreground,
        compact_accessibility_snapshot, element_matches_selector, first_matching_element,
        inspector_available_sources, inspector_metadata, inspector_session_from_published,
        inspector_session_score, is_inspector_agent_transport_path,
        mark_safari_active_webkit_target, normalize_inspector_node,
        normalize_screen_point_from_snapshot, normalized_gesture_coordinates,
        parse_lsof_tcp_listener, parse_ui_application_service_line,
        process_identifier_from_accessibility_snapshot, resolved_stream_quality_limits,
        split_filter_values, stream_quality_profile, suppress_native_ax_translation_error,
        tap_point_from_snapshot, trim_tree_depth, ui_application_foreground_score,
        AccessibilityHierarchySource, ElementSelectorPayload, InspectorSession,
        InspectorSessionTransport, StreamClientForegroundRegistry, StreamQualityLimits,
        StreamQualityPayload, UIKitApplicationServiceDetails, SOURCE_FLUTTER, SOURCE_NATIVE_AX,
        SOURCE_NATIVE_SCRIPT, SOURCE_REACT_NATIVE, SOURCE_SWIFTUI, SOURCE_UIKIT,
    };
    use crate::inspector::PublishedInspector;
    use crate::metrics::counters::ClientStreamStats;
    use crate::transport::packet::FramePacket;
    use bytes::Bytes;
    use serde_json::{json, Value};

    fn selector() -> ElementSelectorPayload {
        ElementSelectorPayload {
            id: Some("continue-button".to_owned()),
            label: Some("Continue".to_owned()),
            value: None,
            element_type: Some("Button".to_owned()),
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

    fn webkit_target(title: &str, url: &str, page_id: u64) -> crate::webkit::WebKitTarget {
        crate::webkit::WebKitTarget {
            id: format!("target-{page_id}"),
            app_id: "com.apple.mobilesafari".to_owned(),
            app_name: Some("Safari".to_owned()),
            app_active: true,
            page_active: false,
            page_id,
            title: Some(title.to_owned()),
            url: Some(url.to_owned()),
            kind: "safari-page".to_owned(),
            inspector_url: format!("/webkit/{page_id}"),
            web_socket_url: format!("ws://localhost/{page_id}"),
        }
    }

    #[test]
    fn active_safari_url_hint_reads_bidi_stripped_ax_value() {
        let snapshot = json!({
            "roots": [{
                "type": "TextField",
                "AXLabel": "Address",
                "AXValue": "\u{200e}webkit.org",
                "children": []
            }]
        });

        assert_eq!(
            active_url_hint_from_accessibility_snapshot(&snapshot),
            Some("webkit.org".to_owned())
        );
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
    fn mark_safari_active_webkit_target_promotes_matching_tab() {
        let mut targets = vec![
            webkit_target("Example Domain", "https://example.com/", 2),
            webkit_target("WebKit", "https://webkit.org/", 8),
            webkit_target("SimDeck", "https://simdeck.nativescript.org/", 1),
        ];

        mark_safari_active_webkit_target(&mut targets, "webkit.org");

        assert_eq!(
            targets.first().and_then(|target| target.url.as_deref()),
            Some("https://webkit.org/")
        );
        assert!(targets[0].page_active);
        assert!(targets.iter().skip(1).all(|target| !target.page_active));
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
    fn published_inspector_session_uses_remote_daemon_transport() {
        let session = inspector_session_from_published(PublishedInspector {
            access_token: "secret-token".to_owned(),
            available_sources: vec![SOURCE_REACT_NATIVE.to_owned()],
            daemon_id: "daemon-a".to_owned(),
            info: json!({
                "bundleIdentifier": "com.example.App",
                "protocolVersion": "1.0"
            }),
            process_identifier: 42,
            server_url: "http://127.0.0.1:4310".to_owned(),
            updated_at_unix_ms: 1,
        });

        assert_eq!(inspector_session_score(&session), 0);
        let InspectorSessionTransport::RemoteDaemon {
            server_url,
            access_token,
        } = &session.transport
        else {
            panic!("published inspector should use remote daemon transport");
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
        assert_eq!(metadata["daemonUrl"], "http://127.0.0.1:4310");
        assert!(metadata["port"].is_null());
    }

    #[test]
    fn published_inspector_session_merges_sources_from_info() {
        let session = inspector_session_from_published(PublishedInspector {
            access_token: "secret-token".to_owned(),
            available_sources: vec![SOURCE_UIKIT.to_owned()],
            daemon_id: "daemon-a".to_owned(),
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

        assert_eq!(
            session.available_sources,
            vec![SOURCE_FLUTTER.to_owned(), SOURCE_UIKIT.to_owned()]
        );
        assert_eq!(inspector_session_score(&session), 1);
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
            "   41210      - \tUIKitApplication:com.apple.mobilesafari[2777][rb-legacy]",
        )
        .unwrap();
        assert_eq!(service.pid, 41210);
        assert_eq!(
            service.service_name,
            "UIKitApplication:com.apple.mobilesafari[2777][rb-legacy]"
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
            AccessibilityHierarchySource::parse(Some("rn")).unwrap(),
            AccessibilityHierarchySource::ReactNative
        ));
        assert!(matches!(
            AccessibilityHierarchySource::parse(Some("flutter")).unwrap(),
            AccessibilityHierarchySource::Flutter
        ));
        assert!(matches!(
            AccessibilityHierarchySource::parse(Some("swift-ui")).unwrap(),
            AccessibilityHierarchySource::SwiftUI
        ));
        assert!(AccessibilityHierarchySource::parse(Some("unknown")).is_err());
    }

    #[test]
    fn split_filter_values_trims_lowercases_and_omits_empty_parts() {
        assert_eq!(
            split_filter_values(Some(" Error, SpringBoard ,, DEBUG ")),
            vec!["error", "springboard", "debug"]
        );
    }

    #[test]
    fn h264_ws_frame_message_uses_fixed_binary_header() {
        let frame = FramePacket {
            frame_sequence: 9,
            timestamp_us: 456,
            is_keyframe: true,
            width: 390,
            height: 844,
            codec: Some("h264".to_owned()),
            description: Some(Bytes::from_static(b"avcc")),
            data: Bytes::from_static(b"h264-sample"),
        };

        let message = super::h264_ws_frame_message(&frame);

        assert_eq!(&message[0..4], b"SDH1");
        assert_eq!(message[4], 1);
        assert_eq!(
            message[5],
            super::H264_WS_FLAG_KEYFRAME | super::H264_WS_FLAG_CONFIG
        );
        assert_eq!(u16::from_be_bytes([message[6], message[7]]), 40);
        assert_eq!(u64::from_be_bytes(message[8..16].try_into().unwrap()), 9);
        assert_eq!(u64::from_be_bytes(message[16..24].try_into().unwrap()), 456);
        assert_eq!(u32::from_be_bytes(message[24..28].try_into().unwrap()), 390);
        assert_eq!(u32::from_be_bytes(message[28..32].try_into().unwrap()), 844);
        assert_eq!(u32::from_be_bytes(message[32..36].try_into().unwrap()), 4);
        assert_eq!(u32::from_be_bytes(message[36..40].try_into().unwrap()), 11);
        assert_eq!(&message[40..44], b"avcc");
        assert_eq!(&message[44..], b"h264-sample");
    }
}
