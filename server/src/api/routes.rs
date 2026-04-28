use crate::api::json::json;
use crate::auth;
use crate::config::Config;
use crate::error::AppError;
use crate::inspector::InspectorHub;
use crate::logs::LogRegistry;
use crate::metrics::counters::{ClientStreamStats, Metrics};
use crate::native::bridge::{LogFilters, NativeBridge};
use crate::simulators::registry::SessionRegistry;
use crate::simulators::session::SimulatorSession;
use crate::transport::packet::PACKET_VERSION;
use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{header, HeaderMap, Method, Request, StatusCode};
use axum::middleware::{from_fn_with_state, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::Map;
use serde_json::{json as json_value, Value};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::process::Command;
use tokio::task;
use tokio::time::timeout;
use tower_http::trace::TraceLayer;

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub registry: SessionRegistry,
    pub logs: LogRegistry,
    pub inspectors: InspectorHub,
    pub metrics: Arc<Metrics>,
    pub certificate_hash_hex: String,
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
enum ControlMessage {
    Touch {
        x: f64,
        y: f64,
        phase: String,
    },
    Key {
        key_code: u16,
        modifiers: Option<u32>,
    },
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
struct InspectorRequestPayload {
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
const SOURCE_UIKIT: &str = "in-app-inspector";

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/metrics", get(metrics))
        .route(
            "/api/client-stream-stats",
            get(client_stream_stats).post(record_client_stream_stats),
        )
        .route("/api/inspector/connect", get(native_inspector_connect))
        .route("/api/inspector/poll", get(inspector_poll))
        .route("/api/inspector/response", post(inspector_response))
        .route("/api/simulators", get(list_simulators))
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
        .route("/api/simulators/{udid}/logs", get(simulator_logs))
        .route_layer(from_fn_with_state(state.clone(), require_api_auth))
        .with_state(state)
        .layer(TraceLayer::new_for_http())
}

async fn require_api_auth(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if is_inspector_agent_transport_path(request.uri().path()) {
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

fn is_inspector_agent_transport_path(path: &str) -> bool {
    matches!(
        path,
        "/api/inspector/connect" | "/api/inspector/poll" | "/api/inspector/response"
    )
}

async fn health(State(state): State<AppState>) -> Json<Value> {
    json(json_value!({
        "ok": true,
        "httpPort": state.config.http_port,
        "wtPort": state.config.wt_port,
        "timestamp": SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or(Duration::ZERO).as_secs_f64(),
        "videoCodec": state.config.video_codec,
        "webTransport": {
            "urlTemplate": auth::tokenized_webtransport_template(&state.config),
            "certificateHash": {
                "algorithm": "sha-256",
                "value": state.certificate_hash_hex,
            },
            "packetVersion": PACKET_VERSION,
        }
    }))
}

async fn metrics(State(state): State<AppState>) -> Json<Value> {
    json(json_value!(state.metrics.snapshot()))
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

    state.metrics.record_client_stream_stats(payload);
    Ok(json(json_value!({ "ok": true })))
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
    let simulators = run_bridge_action(state.clone(), |bridge| bridge.list_simulators()).await?;
    Ok(json(json_value!({
        "simulators": state.registry.enrich_simulators(simulators),
    })))
}

async fn boot_simulator(
    State(state): State<AppState>,
    Path(udid): Path<String>,
) -> Result<Json<Value>, AppError> {
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
    let text = run_bridge_action(state, move |bridge| bridge.pasteboard_text(&udid)).await?;
    Ok(json(json_value!({ "text": text })))
}

async fn set_pasteboard(
    State(state): State<AppState>,
    Path(udid): Path<String>,
    Json(payload): Json<PasteboardPayload>,
) -> Result<Json<Value>, AppError> {
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
    let png = run_bridge_action(state, move |bridge| bridge.screenshot_png(&udid)).await?;
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
    let action_udid = udid.clone();
    run_bridge_action(state, move |bridge| bridge.toggle_appearance(&action_udid)).await?;
    Ok(json(json_value!({ "ok": true })))
}

async fn refresh_stream(
    State(state): State<AppState>,
    Path(udid): Path<String>,
) -> Result<Json<Value>, AppError> {
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
    websocket.on_upgrade(move |socket| handle_control_socket(state, udid, socket))
}

async fn webrtc_offer(
    State(state): State<AppState>,
    Path(udid): Path<String>,
    Json(payload): Json<crate::transport::webrtc::WebRtcOfferPayload>,
) -> Result<Json<crate::transport::webrtc::WebRtcAnswerPayload>, AppError> {
    crate::transport::webrtc::create_answer(state, udid, payload)
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
        if let Err(error) = run_control_message(session.clone(), control_message).await {
            tracing::debug!("Control message failed for {udid}: {error}");
        }
    }
}

async fn run_control_message(
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
        ControlMessage::Key {
            key_code,
            modifiers,
        } => session.send_key(key_code, modifiers.unwrap_or(0)),
    })
    .await
    .map_err(|error| AppError::internal(format!("Failed to join control task: {error}")))?
}

async fn send_key(
    State(state): State<AppState>,
    Path(udid): Path<String>,
    Json(payload): Json<KeyPayload>,
) -> Result<Json<Value>, AppError> {
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
    run_bridge_action(state, move |bridge| {
        bridge.press_button(&udid, &payload.button, payload.duration_ms.unwrap_or(0))
    })
    .await?;
    Ok(json(json_value!({ "ok": true })))
}

async fn press_home(
    State(state): State<AppState>,
    Path(udid): Path<String>,
) -> Result<Json<Value>, AppError> {
    run_bridge_action(state, move |bridge| bridge.press_home(&udid)).await?;
    Ok(json(json_value!({ "ok": true })))
}

async fn open_app_switcher(
    State(state): State<AppState>,
    Path(udid): Path<String>,
) -> Result<Json<Value>, AppError> {
    run_bridge_action(state, move |bridge| {
        bridge.press_home(&udid)?;
        std::thread::sleep(Duration::from_millis(140));
        bridge.press_home(&udid)
    })
    .await?;
    Ok(json(json_value!({ "ok": true })))
}

async fn rotate_right(
    State(state): State<AppState>,
    Path(udid): Path<String>,
) -> Result<Json<Value>, AppError> {
    run_bridge_action(state, move |bridge| bridge.rotate_right(&udid)).await?;
    Ok(json(json_value!({ "ok": true })))
}

async fn rotate_left(
    State(state): State<AppState>,
    Path(udid): Path<String>,
) -> Result<Json<Value>, AppError> {
    run_bridge_action(state, move |bridge| bridge.rotate_left(&udid)).await?;
    Ok(json(json_value!({ "ok": true })))
}

async fn chrome_profile(
    State(state): State<AppState>,
    Path(udid): Path<String>,
) -> Result<Json<Value>, AppError> {
    let profile = run_bridge_action(state, move |bridge| bridge.chrome_profile(&udid)).await?;
    Ok(json(json_value!(profile)))
}

async fn chrome_png(
    State(state): State<AppState>,
    Path(udid): Path<String>,
) -> Result<(StatusCode, HeaderMap, Vec<u8>), AppError> {
    let png = run_bridge_action(state, move |bridge| bridge.chrome_png(&udid)).await?;
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, "image/png".parse().unwrap());
    headers.insert(
        header::CACHE_CONTROL,
        "no-cache, no-store, must-revalidate".parse().unwrap(),
    );
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
            run_bridge_action(state, move |bridge| {
                bridge.press_button(&udid, &button, duration_ms.unwrap_or(0))
            })
            .await?;
            Ok(json_value!({ "action": "button" }))
        }
        BatchStep::Launch { bundle_id } => {
            run_bridge_action(state, move |bridge| bridge.launch_bundle(&udid, &bundle_id)).await?;
            Ok(json_value!({ "action": "launch" }))
        }
        BatchStep::OpenUrl { url } => {
            run_bridge_action(state, move |bridge| bridge.open_url(&udid, &url)).await?;
            Ok(json_value!({ "action": "openUrl" }))
        }
        BatchStep::Home => {
            run_bridge_action(state, move |bridge| bridge.press_home(&udid)).await?;
            Ok(json_value!({ "action": "home" }))
        }
        BatchStep::DismissKeyboard => {
            run_bridge_action(state, move |bridge| bridge.send_key(&udid, 41, 0)).await?;
            Ok(json_value!({ "action": "dismissKeyboard" }))
        }
        BatchStep::AppSwitcher => {
            run_bridge_action(state, move |bridge| {
                bridge.press_home(&udid)?;
                std::thread::sleep(Duration::from_millis(140));
                bridge.press_home(&udid)
            })
            .await?;
            Ok(json_value!({ "action": "appSwitcher" }))
        }
        BatchStep::RotateLeft => {
            run_bridge_action(state, move |bridge| bridge.rotate_left(&udid)).await?;
            Ok(json_value!({ "action": "rotateLeft" }))
        }
        BatchStep::RotateRight => {
            run_bridge_action(state, move |bridge| bridge.rotate_right(&udid)).await?;
            Ok(json_value!({ "action": "rotateRight" }))
        }
        BatchStep::ToggleAppearance => {
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
        "Runtime.ping"
            | "View.get"
            | "View.evaluateScript"
            | "View.getHierarchy"
            | "View.getProperties"
            | "View.setProperty"
            | "View.listActions"
            | "View.perform"
    )
}

async fn simulator_logs(
    State(state): State<AppState>,
    Path(udid): Path<String>,
    Query(query): Query<LogsQuery>,
) -> Result<Json<Value>, AppError> {
    let limit = query.limit.unwrap_or(250).clamp(1, 1000);
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
    UIKit,
    NativeAX,
}

impl AccessibilityHierarchySource {
    fn parse(value: Option<&str>) -> Result<Self, AppError> {
        match value.unwrap_or("auto").trim().to_lowercase().as_str() {
            "" | "auto" => Ok(Self::Auto),
            "nativescript" | "ns" => Ok(Self::NativeScript),
            "react-native" | "reactnative" | "rn" => Ok(Self::ReactNative),
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

#[derive(Clone, Copy)]
enum InspectorSessionTransport {
    Connected,
    Tcp { port: u16 },
}

async fn inspector_session_for_state(
    state: &AppState,
    udid: &str,
) -> Result<InspectorSession, String> {
    let frontmost_pid = frontmost_process_identifier(state, udid)
        .await
        .ok()
        .flatten();
    let connected_error = match connected_inspector_session(state, udid, frontmost_pid).await {
        Ok(session) => return Ok(session),
        Err(error) => error,
    };

    match inspector_session(udid, frontmost_pid).await {
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
        .any(|source| source == SOURCE_NATIVE_SCRIPT)
    {
        return 1;
    }
    if session
        .available_sources
        .iter()
        .any(|source| source == SOURCE_UIKIT)
    {
        return 2;
    }
    3
}

async fn frontmost_process_identifier(state: &AppState, udid: &str) -> Result<Option<i64>, String> {
    let snapshot = accessibility_snapshot(state.clone(), udid.to_owned(), None, Some(0))
        .await
        .map_err(|error| error.to_string())?;
    Ok(snapshot
        .get("roots")
        .and_then(Value::as_array)
        .and_then(|roots| roots.first())
        .and_then(|root| root.get("pid"))
        .and_then(Value::as_i64))
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
    if source == SOURCE_NATIVE_SCRIPT || source == SOURCE_REACT_NATIVE {
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
    match session.transport {
        InspectorSessionTransport::Connected => {
            let wait = if method == "View.getHierarchy" {
                CONNECTED_INSPECTOR_HIERARCHY_TIMEOUT
            } else {
                Duration::from_secs(10)
            };
            state
                .inspectors
                .query_with_timeout(session.process_identifier, method, params, wait)
                .await
        }
        InspectorSessionTransport::Tcp { port } => {
            query_inspector_agent_on_port(port, method, params).await
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
            _ => {}
        }
    }
    let uikit_available = info
        .get("uikit")
        .and_then(|value| value.get("available"))
        .and_then(Value::as_bool)
        .unwrap_or(!react_native_available);
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
    if source == SOURCE_REACT_NATIVE {
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
    if sources.iter().any(|source| source == SOURCE_REACT_NATIVE) {
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
    source == SOURCE_NATIVE_SCRIPT || source == SOURCE_REACT_NATIVE
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
    let (transport_name, port) = match transport {
        InspectorSessionTransport::Connected => ("websocket", Value::Null),
        InspectorSessionTransport::Tcp { port } => ("tcp+ndjson", Value::Number((*port).into())),
    };
    json_value!({
        "bundleIdentifier": info.get("bundleIdentifier").cloned().unwrap_or(Value::Null),
        "bundleName": info.get("bundleName").cloned().unwrap_or(Value::Null),
        "coordinateSpace": hierarchy.get("coordinateSpace").cloned().unwrap_or(Value::Null),
        "displayScale": hierarchy.get("displayScale").cloned().unwrap_or_else(|| info.get("displayScale").cloned().unwrap_or(Value::Null)),
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

async fn inspector_process_belongs_to_udid(udid: &str, pid: i64) -> Result<bool, String> {
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
    .map_err(|_| "Timed out validating inspector process simulator.".to_owned())?
    .map_err(|error| format!("Unable to validate inspector process simulator: {error}"))?;

    if !output.status.success() {
        return Ok(false);
    }

    Ok(String::from_utf8_lossy(&output.stdout).contains(udid))
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

async fn simulator_payload(state: AppState, udid: String) -> Result<Json<Value>, AppError> {
    let simulators = run_bridge_action(state.clone(), |bridge| bridge.list_simulators()).await?;
    let enriched = state.registry.enrich_simulators(simulators);
    let simulator = enriched
        .into_iter()
        .find(|entry| entry.get("udid").and_then(Value::as_str) == Some(udid.as_str()))
        .ok_or_else(|| AppError::not_found(format!("Unknown simulator {udid}")))?;
    Ok(json(json_value!({ "simulator": simulator })))
}
