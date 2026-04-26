use crate::api::json::json;
use crate::auth;
use crate::config::Config;
use crate::error::AppError;
use crate::inspector::InspectorHub;
use crate::logs::LogRegistry;
use crate::metrics::counters::{ClientStreamStats, Metrics};
use crate::native::bridge::{LogFilters, NativeBridge};
use crate::simulators::registry::SessionRegistry;
use crate::transport::packet::PACKET_VERSION;
use axum::body::Body;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{header, HeaderMap, Method, Request, StatusCode};
use axum::middleware::{from_fn_with_state, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::Map;
use serde_json::{json as json_value, Value};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
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
struct TouchPayload {
    x: f64,
    y: f64,
    phase: String,
}

#[derive(Deserialize)]
struct KeyPayload {
    #[serde(rename = "keyCode")]
    key_code: u16,
    modifiers: Option<u32>,
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
const SOURCE_NATIVE_AX: &str = "native-ax";
const SOURCE_NATIVE_SCRIPT: &str = "nativescript";
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
        .route(
            "/api/simulators/{udid}/toggle-appearance",
            post(toggle_appearance),
        )
        .route("/api/simulators/{udid}/refresh", post(refresh_stream))
        .route("/api/simulators/{udid}/open-url", post(open_url))
        .route("/api/simulators/{udid}/launch", post(launch_bundle))
        .route("/api/simulators/{udid}/touch", post(send_touch))
        .route("/api/simulators/{udid}/key", post(send_key))
        .route(
            "/api/simulators/{udid}/dismiss-keyboard",
            post(dismiss_keyboard),
        )
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
    state.registry.remove(&udid);
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
    state.registry.remove(&udid);
    let action_udid = udid.clone();
    run_bridge_action(state.clone(), move |bridge| {
        bridge.shutdown_simulator(&action_udid)
    })
    .await?;
    simulator_payload(state, udid).await
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
    run_bridge_action(state, move |bridge| bridge.send_touch(&udid, x, y, &phase)).await?;
    Ok(json(json_value!({ "ok": true })))
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

async fn dismiss_keyboard(
    State(state): State<AppState>,
    Path(udid): Path<String>,
) -> Result<Json<Value>, AppError> {
    run_bridge_action(state, move |bridge| bridge.send_key(&udid, 41, 0)).await?;
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
    let requested_source = AccessibilityHierarchySource::parse(query.source.as_deref())?;
    let max_depth = query.max_depth.map(|depth| depth.min(80));
    let include_hidden = query.include_hidden.unwrap_or(false);

    if requested_source == AccessibilityHierarchySource::NativeAX {
        let native_snapshot = match accessibility_snapshot(state.clone(), udid.clone(), None).await
        {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return Ok(json(empty_accessibility_tree(
                    SOURCE_NATIVE_AX,
                    &available_sources_with_native_ax(None),
                    suppress_native_ax_translation_error(&error.to_string()),
                )));
            }
        };
        let availability = inspector_session_for_state(&state, &udid).await.ok();
        let available_sources =
            foreground_available_sources(&native_snapshot, availability.as_ref());
        let snapshot = attach_available_sources(
            trim_tree_depth(native_snapshot, max_depth),
            &available_sources,
        );
        return Ok(json(snapshot));
    }

    match inspector_session_for_state(&state, &udid).await {
        Ok(session) => {
            let hierarchy_source = match requested_source {
                AccessibilityHierarchySource::Auto => InAppHierarchySource::Automatic,
                AccessibilityHierarchySource::NativeScript => InAppHierarchySource::Automatic,
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
                    let fallback_reason = if requested_source
                        == AccessibilityHierarchySource::NativeScript
                        && snapshot.get("source").and_then(Value::as_str)
                            != Some(SOURCE_NATIVE_SCRIPT)
                    {
                        Some("NativeScript hierarchy is not published by the app.".to_owned())
                    } else {
                        None
                    };
                    Ok(json(attach_tree_metadata(
                        snapshot,
                        &available_sources,
                        fallback_reason,
                    )))
                }
                Err(_inspector_error) => {
                    let available_sources = available_sources_with_native_ax(Some(&session));
                    match accessibility_snapshot(state.clone(), udid.clone(), None).await {
                        Ok(native_snapshot) => Ok(json(attach_available_sources(
                            trim_tree_depth(native_snapshot, max_depth),
                            &available_sources,
                        ))),
                        Err(native_ax_error) => Ok(json(empty_accessibility_tree(
                            SOURCE_NATIVE_AX,
                            &available_sources,
                            suppress_native_ax_translation_error(&native_ax_error.to_string()),
                        ))),
                    }
                }
            }
        }
        Err(_inspector_error) => {
            let available_sources = available_sources_with_native_ax(None);
            match accessibility_snapshot(state.clone(), udid.clone(), None).await {
                Ok(native_snapshot) => Ok(json(attach_available_sources(
                    trim_tree_depth(native_snapshot, max_depth),
                    &available_sources,
                ))),
                Err(native_ax_error) => Ok(json(empty_accessibility_tree(
                    SOURCE_NATIVE_AX,
                    &available_sources,
                    suppress_native_ax_translation_error(&native_ax_error.to_string()),
                ))),
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

    let snapshot = accessibility_snapshot(state, udid, Some((query.x, query.y))).await?;
    Ok(json(snapshot))
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
    UIKit,
    NativeAX,
}

impl AccessibilityHierarchySource {
    fn parse(value: Option<&str>) -> Result<Self, AppError> {
        match value.unwrap_or("auto").trim().to_lowercase().as_str() {
            "" | "auto" => Ok(Self::Auto),
            "nativescript" | "ns" => Ok(Self::NativeScript),
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
    let connected_error = match connected_inspector_session(state, udid).await {
        Ok(session) => return Ok(session),
        Err(error) => error,
    };

    match inspector_session(udid).await {
        Ok(session) => Ok(session),
        Err(tcp_error) => Err(format!("{connected_error} {tcp_error}")),
    }
}

async fn connected_inspector_session(
    state: &AppState,
    udid: &str,
) -> Result<InspectorSession, String> {
    let mut probed_inspectors = Vec::new();
    for inspector in state.inspectors.connected().await {
        if inspector_process_belongs_to_udid(udid, inspector.process_identifier).await? {
            return Ok(InspectorSession {
                transport: InspectorSessionTransport::Connected,
                available_sources: inspector_available_sources(&inspector.info),
                info: inspector.info,
                process_identifier: inspector.process_identifier,
            });
        }

        probed_inspectors.push(format!("process {}", inspector.process_identifier));
    }

    if probed_inspectors.is_empty() {
        Err(format!(
            "No connected NativeScript inspector found for simulator {udid}."
        ))
    } else {
        Err(format!(
            "No connected NativeScript inspector matched simulator {udid}. Found inspectors for {}.",
            probed_inspectors.join(", ")
        ))
    }
}

async fn inspector_session(udid: &str) -> Result<InspectorSession, String> {
    let mut probed_inspectors = Vec::new();
    let mut probe_errors = Vec::new();

    if let Some(session) = find_inspector_session_on_ports(
        udid,
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
    ports: Vec<u16>,
    probed_inspectors: &mut Vec<String>,
    probe_errors: &mut Vec<String>,
) -> Result<Option<InspectorSession>, String> {
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

        if inspector_process_belongs_to_udid(udid, process_identifier).await? {
            return Ok(Some(InspectorSession {
                transport: InspectorSessionTransport::Tcp { port },
                available_sources: inspector_available_sources(&info),
                info,
                process_identifier,
            }));
        }

        probed_inspectors.push(format!("{port}: process {process_identifier}"));
    }

    Ok(None)
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
    if source == SOURCE_NATIVE_SCRIPT {
        return Ok(json_value!({
            "roots": hierarchy.get("roots").cloned().unwrap_or_else(|| Value::Array(Vec::new())),
            "source": SOURCE_NATIVE_SCRIPT,
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
                INSPECTOR_AGENT_TIMEOUT
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
    let app_hierarchy = info.get("appHierarchy");
    let app_hierarchy_available = app_hierarchy
        .and_then(|value| value.get("available"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let app_hierarchy_source = app_hierarchy
        .and_then(|value| value.get("source"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if app_hierarchy_available && app_hierarchy_source == SOURCE_NATIVE_SCRIPT {
        sources.push(SOURCE_NATIVE_SCRIPT.to_owned());
    }
    sources.push(SOURCE_UIKIT.to_owned());
    sources
}

fn push_unique_source(sources: &mut Vec<String>, source: &str) {
    if !sources.iter().any(|value| value == source) {
        sources.push(source.to_owned());
    }
}

fn available_sources_for_session(session: &InspectorSession) -> Vec<String> {
    let mut sources = Vec::new();
    if matches!(session.transport, InspectorSessionTransport::Connected)
        || session
            .available_sources
            .iter()
            .any(|source| source == SOURCE_NATIVE_SCRIPT)
    {
        push_unique_source(&mut sources, SOURCE_NATIVE_SCRIPT);
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

fn foreground_available_sources(
    native_ax_snapshot: &Value,
    session: Option<&InspectorSession>,
) -> Vec<String> {
    if let Some(session) = session {
        if native_ax_snapshot_contains_pid(native_ax_snapshot, session.process_identifier) {
            return available_sources_with_native_ax(Some(session));
        }
    }
    available_sources_with_native_ax(None)
}

fn available_sources_for_snapshot(base_sources: &[String], snapshot: &Value) -> Vec<String> {
    let mut sources = base_sources.to_owned();
    let Some(source) = snapshot.get("source").and_then(Value::as_str) else {
        return sources;
    };
    if source == SOURCE_NATIVE_SCRIPT && !sources.iter().any(|value| value == SOURCE_NATIVE_SCRIPT)
    {
        sources.insert(0, SOURCE_NATIVE_SCRIPT.to_owned());
    }
    if source == SOURCE_UIKIT && !sources.iter().any(|value| value == SOURCE_UIKIT) {
        let insert_at = usize::from(
            sources
                .first()
                .map(|value| value == SOURCE_NATIVE_SCRIPT)
                .unwrap_or(false),
        );
        sources.insert(insert_at, SOURCE_UIKIT.to_owned());
    }
    sources
}

fn native_ax_snapshot_contains_pid(snapshot: &Value, pid: i64) -> bool {
    match snapshot {
        Value::Array(values) => values
            .iter()
            .any(|value| native_ax_snapshot_contains_pid(value, pid)),
        Value::Object(object) => {
            object.get("pid").and_then(Value::as_i64) == Some(pid)
                || object
                    .get("children")
                    .map(|children| native_ax_snapshot_contains_pid(children, pid))
                    .unwrap_or(false)
                || object
                    .get("roots")
                    .map(|roots| native_ax_snapshot_contains_pid(roots, pid))
                    .unwrap_or(false)
        }
        _ => false,
    }
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
) -> Result<Value, AppError> {
    run_bridge_action(state, move |bridge| {
        bridge.accessibility_snapshot(&udid, point)
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
