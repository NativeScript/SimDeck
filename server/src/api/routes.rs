use crate::api::json::json;
use crate::config::Config;
use crate::error::AppError;
use crate::metrics::counters::Metrics;
use crate::simulators::registry::SessionRegistry;
use crate::transport::packet::PACKET_VERSION;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, Method, StatusCode};
use axum::middleware::map_response;
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json as json_value, Value};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub registry: SessionRegistry,
    pub metrics: Arc<Metrics>,
    pub wt_endpoint_template: String,
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

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/metrics", get(metrics))
        .route("/api/simulators", get(list_simulators))
        .route("/api/simulators/{udid}/boot", post(boot_simulator))
        .route("/api/simulators/{udid}/shutdown", post(shutdown_simulator))
        .route("/api/simulators/{udid}/refresh", post(refresh_stream))
        .route("/api/simulators/{udid}/open-url", post(open_url))
        .route("/api/simulators/{udid}/launch", post(launch_bundle))
        .route("/api/simulators/{udid}/touch", post(send_touch))
        .route("/api/simulators/{udid}/key", post(send_key))
        .route("/api/simulators/{udid}/home", post(press_home))
        .route("/api/simulators/{udid}/rotate-right", post(rotate_right))
        .route("/api/simulators/{udid}/chrome-profile", get(chrome_profile))
        .route("/api/simulators/{udid}/chrome.png", get(chrome_png))
        .with_state(state)
        .layer(map_response(append_cors_headers))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_headers(Any)
                .allow_methods([Method::GET, Method::POST, Method::OPTIONS]),
        )
        .layer(TraceLayer::new_for_http())
}

async fn append_cors_headers(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*".parse().unwrap());
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        "GET, POST, OPTIONS".parse().unwrap(),
    );
    headers.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, "*".parse().unwrap());
    response
}

async fn health(State(state): State<AppState>) -> Json<Value> {
    json(json_value!({
        "ok": true,
        "httpPort": state.config.http_port,
        "wtPort": state.config.wt_port,
        "timestamp": SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or(Duration::ZERO).as_secs_f64(),
        "webTransport": {
            "urlTemplate": state.wt_endpoint_template,
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

async fn list_simulators(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let simulators = state.registry.bridge().list_simulators()?;
    Ok(json(json_value!({
        "simulators": state.registry.enrich_simulators(simulators),
    })))
}

async fn boot_simulator(
    State(state): State<AppState>,
    Path(udid): Path<String>,
) -> Result<Json<Value>, AppError> {
    state.registry.bridge().boot_simulator(&udid)?;
    simulator_payload(&state, &udid)
}

async fn shutdown_simulator(
    State(state): State<AppState>,
    Path(udid): Path<String>,
) -> Result<Json<Value>, AppError> {
    state.registry.remove(&udid);
    state.registry.bridge().shutdown_simulator(&udid)?;
    simulator_payload(&state, &udid)
}

async fn refresh_stream(
    State(state): State<AppState>,
    Path(udid): Path<String>,
) -> Result<Json<Value>, AppError> {
    let session = state.registry.get_or_create_async(&udid).await?;
    session.ensure_started_async().await?;
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
    state.registry.bridge().open_url(&udid, &payload.url)?;
    simulator_payload(&state, &udid)
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
    state
        .registry
        .bridge()
        .launch_bundle(&udid, &payload.bundle_id)?;
    simulator_payload(&state, &udid)
}

async fn send_touch(
    State(state): State<AppState>,
    Path(udid): Path<String>,
    Json(payload): Json<TouchPayload>,
) -> Result<Json<Value>, AppError> {
    let session = state.registry.get_or_create(&udid)?;
    session.send_touch(payload.x, payload.y, &payload.phase)?;
    Ok(json(json_value!({ "ok": true })))
}

async fn send_key(
    State(state): State<AppState>,
    Path(udid): Path<String>,
    Json(payload): Json<KeyPayload>,
) -> Result<Json<Value>, AppError> {
    let session = state.registry.get_or_create(&udid)?;
    session.send_key(payload.key_code, payload.modifiers.unwrap_or(0))?;
    Ok(json(json_value!({ "ok": true })))
}

async fn press_home(
    State(state): State<AppState>,
    Path(udid): Path<String>,
) -> Result<Json<Value>, AppError> {
    let session = state.registry.get_or_create(&udid)?;
    session.press_home()?;
    Ok(json(json_value!({ "ok": true })))
}

async fn rotate_right(
    State(state): State<AppState>,
    Path(udid): Path<String>,
) -> Result<Json<Value>, AppError> {
    let session = state.registry.get_or_create(&udid)?;
    session.rotate_right()?;
    session.request_refresh();
    Ok(json(json_value!({ "ok": true })))
}

async fn chrome_profile(
    State(state): State<AppState>,
    Path(udid): Path<String>,
) -> Result<Json<Value>, AppError> {
    let profile = state.registry.bridge().chrome_profile(&udid)?;
    Ok(json(json_value!(profile)))
}

async fn chrome_png(
    State(state): State<AppState>,
    Path(udid): Path<String>,
) -> Result<(StatusCode, HeaderMap, Vec<u8>), AppError> {
    let png = state.registry.bridge().chrome_png(&udid)?;
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, "image/png".parse().unwrap());
    headers.insert(
        header::CACHE_CONTROL,
        "no-cache, no-store, must-revalidate".parse().unwrap(),
    );
    Ok((StatusCode::OK, headers, png))
}

fn simulator_payload(state: &AppState, udid: &str) -> Result<Json<Value>, AppError> {
    let simulators = state.registry.bridge().list_simulators()?;
    let enriched = state.registry.enrich_simulators(simulators);
    let simulator = enriched
        .into_iter()
        .find(|entry| entry.get("udid").and_then(Value::as_str) == Some(udid))
        .ok_or_else(|| AppError::not_found(format!("Unknown simulator {udid}")))?;
    Ok(json(json_value!({ "simulator": simulator })))
}
