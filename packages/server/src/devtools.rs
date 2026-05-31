use axum::extract::ws::{Message, WebSocket};
use futures::{future::join_all, SinkExt, StreamExt};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::process::Command;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{header, HeaderValue};
use tokio_tungstenite::tungstenite::Message as UpstreamMessage;
use tracing::debug;

pub type DevToolsQuery = Arc<
    dyn Fn(String, Value) -> Pin<Box<dyn Future<Output = Result<Value, String>> + Send>>
        + Send
        + Sync,
>;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChromeDevToolsTarget {
    pub id: String,
    pub title: String,
    #[serde(rename = "type")]
    pub target_type: String,
    pub url: String,
    pub description: String,
    pub devtools_frontend_url: String,
    pub web_socket_debugger_url: String,
    pub source: String,
    pub process_identifier: i64,
    pub bundle_identifier: Option<String>,
    pub app_name: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChromeDevToolsTargetDiscovery {
    pub udid: String,
    pub targets: Vec<ChromeDevToolsTarget>,
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub foreground_app: Option<ForegroundApp>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForegroundApp {
    pub process_identifier: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle_identifier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_name: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ChromeDevToolsTargetRuntime {
    pub id: String,
    pub title: String,
    pub url: String,
    pub process_identifier: i64,
}

#[derive(Default)]
struct CdpState {
    dom: DomCache,
    execution_context_sent: bool,
}

#[derive(Default)]
struct DomCache {
    document_children: Vec<u64>,
    nodes: HashMap<u64, DomNode>,
    next_node_id: u64,
}

struct DomNode {
    backend_node_id: u64,
    children: Vec<u64>,
    frame: Option<Rect>,
    inspector_id: Option<String>,
    node: Value,
    node_id: u64,
}

#[derive(Clone, Copy)]
struct Rect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

struct CdpResponse {
    events: Vec<Value>,
    result: Value,
}

const DEVTOOLS_HOST: &str = "127.0.0.1";
const DEVTOOLS_DISCOVERY_HOSTS: &[&str] = &["127.0.0.1", "[::1]"];
const DEVTOOLS_DISCOVERY_TIMEOUT: Duration = Duration::from_millis(300);
const DEVTOOLS_LISTENERS_TIMEOUT: Duration = Duration::from_millis(500);
const DEVTOOLS_MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const METRO_ASSET_TIMEOUT: Duration = Duration::from_secs(5);
const METRO_ASSET_MAX_BYTES: usize = 32 * 1024 * 1024;
const DEFAULT_METRO_FRONTEND_PATH: &str = "/debugger-frontend/rn_fusebox.html";
const COMMON_METRO_PORTS: &[u16] = &[8081, 8082, 8083, 19000, 19001, 19002];
const COMMON_CHROME_INSPECTOR_PORTS: &[u16] =
    &[9222, 9223, 9224, 9225, 9226, 9227, 9228, 9229, 9230];
const SOURCE_REACT_NATIVE_METRO: &str = "react-native-metro";
const SOURCE_CHROME_INSPECTOR: &str = "chrome-inspector";

pub fn chrome_devtools_frontend_root() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("SIMDECK_CHROME_DEVTOOLS_FRONTEND_ROOT") {
        let path = PathBuf::from(path);
        if is_devtools_frontend_root(&path) {
            return Some(path);
        }
    }

    let mut roots = Vec::new();
    if let Ok(current_dir) = std::env::current_dir() {
        roots.extend(devtools_frontend_candidates_from(&current_dir));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            roots.extend(devtools_frontend_candidates_from(parent));
        }
    }

    roots
        .into_iter()
        .find(|path| is_devtools_frontend_root(path))
}

fn devtools_frontend_candidates_from(start: &Path) -> Vec<PathBuf> {
    start
        .ancestors()
        .flat_map(|ancestor| {
            [
                ancestor.join(
                    "node_modules/@react-native/debugger-frontend/dist/third-party/front_end",
                ),
                ancestor.join("chrome-devtools-ui"),
                ancestor.join("packages/client/dist/chrome-devtools-ui"),
            ]
        })
        .collect()
}

fn is_devtools_frontend_root(path: &Path) -> bool {
    path.join("inspector.html").is_file()
        && path.join("entrypoints/inspector/inspector.js").is_file()
}

pub fn target_id(process_identifier: i64) -> String {
    format!("sdi-{process_identifier}")
}

pub fn runtime_from_target(target: &ChromeDevToolsTarget) -> ChromeDevToolsTargetRuntime {
    ChromeDevToolsTargetRuntime {
        id: target.id.clone(),
        title: target.title.clone(),
        url: target.url.clone(),
        process_identifier: target.process_identifier,
    }
}

pub fn build_target(
    udid: &str,
    http_origin: Option<&str>,
    info: &Value,
    process_identifier: i64,
    source: &str,
) -> ChromeDevToolsTarget {
    let id = target_id(process_identifier);
    let bundle_identifier = string_value(info, "bundleIdentifier");
    let app_name = string_value(info, "bundleName")
        .or_else(|| bundle_identifier.clone())
        .or_else(|| Some(format!("Process {process_identifier}")));
    let source_label = source_label(source);
    let title = app_name
        .as_deref()
        .map(|name| format!("{source_label}: {name}"))
        .unwrap_or_else(|| format!("{source_label}: Process {process_identifier}"));
    let url = bundle_identifier
        .as_deref()
        .map(|bundle_id| format!("simdeck://{bundle_id}"))
        .unwrap_or_else(|| format!("simdeck://process/{process_identifier}"));
    let web_socket_path = format!("/api/simulators/{udid}/devtools/targets/{id}/socket");
    let web_socket_debugger_url = websocket_url(http_origin.unwrap_or(""), &web_socket_path);
    let devtools_frontend_url = format!(
        "/chrome-devtools-ui/inspector.html?ws={}",
        web_socket_debugger_url
            .trim_start_matches("ws://")
            .trim_start_matches("wss://")
    );

    ChromeDevToolsTarget {
        id,
        title,
        target_type: "page".to_owned(),
        url,
        description: format!("SimDeck {source_label} inspector target"),
        devtools_frontend_url,
        web_socket_debugger_url,
        source: source.to_owned(),
        process_identifier,
        bundle_identifier,
        app_name,
    }
}

pub async fn discover_external_devtools_targets(
    udid: &str,
    http_origin: Option<&str>,
    access_token: Option<&str>,
    simulator_name: Option<&str>,
    simulator_device_type_name: Option<&str>,
) -> (Vec<ChromeDevToolsTarget>, Vec<String>) {
    let mut warnings = Vec::new();
    let mut targets = Vec::new();
    let mut seen_ids = BTreeSet::new();
    let ports = candidate_devtools_ports().await;
    let results = join_all(
        ports
            .into_iter()
            .map(|port| async move { (port, fetch_devtools_target_list(port).await) }),
    )
    .await;
    for (port, result) in results {
        let list = match result {
            Ok(value) => value,
            Err(error) => {
                debug!("DevTools discovery skipped {DEVTOOLS_HOST}:{port}: {error}");
                continue;
            }
        };

        let Some(entries) = list.as_array() else {
            warnings.push(format!(
                "DevTools endpoint on {DEVTOOLS_HOST}:{port} did not return a target list."
            ));
            continue;
        };

        let all_metro_entries = entries
            .iter()
            .filter(|entry| is_react_native_metro_target(entry))
            .collect::<Vec<_>>();
        let preferred_metro_entries = all_metro_entries
            .iter()
            .copied()
            .filter(|entry| is_preferred_react_native_metro_target(entry))
            .collect::<Vec<_>>();
        let metro_entries = if preferred_metro_entries.is_empty() {
            all_metro_entries
        } else {
            preferred_metro_entries
        };
        let mut matched_metro_count = 0;
        for entry in &metro_entries {
            if !metro_target_matches_simulator(entry, simulator_name, simulator_device_type_name) {
                continue;
            }
            let target = build_metro_target(udid, http_origin, access_token, port, entry);
            if seen_ids.insert(target.id.clone()) {
                targets.push(target);
            }
            matched_metro_count += 1;
        }

        if matched_metro_count == 0 && !metro_entries.is_empty() {
            let device_names = metro_entries
                .iter()
                .filter_map(|entry| string_value(entry, "deviceName"))
                .collect::<Vec<_>>();
            if !device_names.is_empty() {
                warnings.push(format!(
                    "Metro on {DEVTOOLS_HOST}:{port} has React Native targets for {}, but none matched simulator {}.",
                    unique_strings(device_names).join(", "),
                    simulator_name.unwrap_or(udid)
                ));
            }
        }

        for entry in entries {
            if is_react_native_metro_target(entry) {
                continue;
            }
            let Some(target) = build_chrome_inspector_target(udid, http_origin, port, entry) else {
                continue;
            };
            if seen_ids.insert(target.id.clone()) {
                targets.push(target);
            }
        }
    }

    (targets, warnings)
}

pub async fn proxied_websocket_url_for_target(target_id: &str) -> Result<String, String> {
    let port = proxied_target_port(target_id)?;
    let list = fetch_devtools_target_list(port).await?;
    let entries = list.as_array().ok_or_else(|| {
        format!("DevTools endpoint on {DEVTOOLS_HOST}:{port} did not return a target list.")
    })?;
    for entry in entries {
        let target = if target_id.starts_with("metro-") {
            build_metro_target("", None, None, port, entry)
        } else if target_id.starts_with("cdp-") {
            let Some(target) = build_chrome_inspector_target("", None, port, entry) else {
                continue;
            };
            target
        } else {
            return Err("Not a proxied DevTools target id.".to_owned());
        };
        if target.id == target_id {
            if target_id.starts_with("metro-") {
                return Ok(metro_websocket_debugger_url(port, entry));
            }
            return chrome_inspector_websocket_debugger_url(port, entry).ok_or_else(|| {
                format!("DevTools target {target_id} does not expose a WebSocket URL.")
            });
        }
    }
    Err(format!(
        "DevTools target {target_id} is no longer available."
    ))
}

pub async fn proxy_websocket(socket: WebSocket, upstream_url: String) {
    if let Err(error) = proxy_websocket_inner(socket, upstream_url).await {
        debug!("DevTools proxy socket closed: {error}");
    }
}

async fn proxy_websocket_inner(socket: WebSocket, upstream_url: String) -> Result<(), String> {
    let mut request = upstream_url
        .as_str()
        .into_client_request()
        .map_err(|error| format!("Invalid DevTools upstream URL: {error}"))?;
    // Metro's React Native dev-middleware rejects inspector WebSocket upgrades
    // whose Origin does not match the dev server itself (HTTP 401). Present the
    // upstream's own origin so the proxied connection is accepted.
    if let Some(origin) = upstream_websocket_origin(&upstream_url) {
        if let Ok(value) = HeaderValue::from_str(&origin) {
            request.headers_mut().insert(header::ORIGIN, value);
        }
    }
    let (upstream, _) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(|error| format!("Unable to connect to DevTools socket: {error}"))?;
    let (mut downstream_writer, mut downstream_reader) = socket.split();
    let (mut upstream_writer, mut upstream_reader) = upstream.split();

    loop {
        tokio::select! {
            downstream = downstream_reader.next() => {
                let Some(message) = downstream else {
                    break;
                };
                let message = message.map_err(|error| format!("Browser WebSocket error: {error}"))?;
                let Some(message) = to_upstream_message(message) else {
                    break;
                };
                upstream_writer
                    .send(message)
                    .await
                    .map_err(|error| format!("Unable to forward browser message to DevTools: {error}"))?;
            }
            upstream = upstream_reader.next() => {
                let Some(message) = upstream else {
                    break;
                };
                let message = message.map_err(|error| format!("DevTools WebSocket error: {error}"))?;
                let Some(message) = to_downstream_message(message) else {
                    break;
                };
                downstream_writer
                    .send(message)
                    .await
                    .map_err(|error| format!("Unable to forward DevTools message to browser: {error}"))?;
            }
        }
    }

    Ok(())
}

fn upstream_websocket_origin(url: &str) -> Option<String> {
    let (scheme, rest) = url
        .strip_prefix("ws://")
        .map(|rest| ("http", rest))
        .or_else(|| url.strip_prefix("wss://").map(|rest| ("https", rest)))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    (!authority.is_empty()).then(|| format!("{scheme}://{authority}"))
}

fn to_upstream_message(message: Message) -> Option<UpstreamMessage> {
    match message {
        Message::Text(text) => Some(UpstreamMessage::Text(text.to_string().into())),
        Message::Binary(bytes) => Some(UpstreamMessage::Binary(bytes)),
        Message::Ping(bytes) => Some(UpstreamMessage::Ping(bytes)),
        Message::Pong(bytes) => Some(UpstreamMessage::Pong(bytes)),
        Message::Close(_) => None,
    }
}

fn to_downstream_message(message: UpstreamMessage) -> Option<Message> {
    match message {
        UpstreamMessage::Text(text) => Some(Message::Text(text.to_string().into())),
        UpstreamMessage::Binary(bytes) => Some(Message::Binary(bytes)),
        UpstreamMessage::Ping(bytes) => Some(Message::Ping(bytes)),
        UpstreamMessage::Pong(bytes) => Some(Message::Pong(bytes)),
        UpstreamMessage::Close(_) | UpstreamMessage::Frame(_) => None,
    }
}

async fn fetch_devtools_target_list(port: u16) -> Result<Value, String> {
    let mut errors = Vec::new();
    for host in DEVTOOLS_DISCOVERY_HOSTS {
        match fetch_devtools_json(host, port, "/json/list").await {
            Ok(value) => return Ok(value),
            Err(list_error) => match fetch_devtools_json(host, port, "/json").await {
                Ok(value) => return Ok(value),
                Err(json_error) => errors.push(format!("{list_error}; {json_error}")),
            },
        }
    }
    Err(errors.join("; "))
}

async fn fetch_devtools_json(host: &str, port: u16, path: &str) -> Result<Value, String> {
    let address = format!("{host}:{port}");
    let mut stream = timeout(DEVTOOLS_DISCOVERY_TIMEOUT, TcpStream::connect(&address))
        .await
        .map_err(|_| format!("Timed out connecting to DevTools endpoint at {address}."))?
        .map_err(|error| format!("Unable to connect to DevTools endpoint at {address}: {error}"))?;

    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {address}\r\nAccept: application/json\r\nConnection: close\r\n\r\n"
    );
    timeout(
        DEVTOOLS_DISCOVERY_TIMEOUT,
        stream.write_all(request.as_bytes()),
    )
    .await
    .map_err(|_| format!("Timed out requesting DevTools {path}."))?
    .map_err(|error| format!("Unable to request DevTools {path}: {error}"))?;

    let mut response = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        let count = timeout(DEVTOOLS_DISCOVERY_TIMEOUT, stream.read(&mut chunk))
            .await
            .map_err(|_| format!("Timed out reading DevTools {path}."))?
            .map_err(|error| format!("Unable to read DevTools {path}: {error}"))?;
        if count == 0 {
            break;
        }
        response.extend_from_slice(&chunk[..count]);
        if response.len() > DEVTOOLS_MAX_RESPONSE_BYTES {
            return Err(format!("DevTools {path} response exceeded the size limit."));
        }
        if response_has_complete_body(&response) {
            break;
        }
    }

    let (headers, body) = split_http_response(&response)
        .ok_or_else(|| format!("DevTools {path} returned a malformed HTTP response."))?;
    let status = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|status| status.parse::<u16>().ok())
        .unwrap_or(0);
    if !(200..300).contains(&status) {
        return Err(format!("DevTools {path} returned HTTP {status}."));
    }

    let body = content_length(&headers)
        .and_then(|length| body.get(..length))
        .unwrap_or(body);
    serde_json::from_slice(body)
        .map_err(|error| format!("DevTools {path} returned malformed JSON: {error}"))
}

async fn connect_loopback_devtools_service(
    port: u16,
    timeout_duration: Duration,
    service_name: &str,
) -> Result<(String, TcpStream), String> {
    let mut errors = Vec::new();
    for host in DEVTOOLS_DISCOVERY_HOSTS {
        let address = format!("{host}:{port}");
        match timeout(timeout_duration, TcpStream::connect(&address)).await {
            Ok(Ok(stream)) => return Ok((address, stream)),
            Ok(Err(error)) => errors.push(format!("{address}: {error}")),
            Err(_) => errors.push(format!("{address}: timed out")),
        }
    }
    Err(format!(
        "Unable to connect to {service_name} on loopback port {port}: {}",
        errors.join("; ")
    ))
}

async fn candidate_devtools_ports() -> Vec<u16> {
    let mut ports = BTreeSet::new();
    ports.extend(COMMON_METRO_PORTS.iter().copied());
    ports.extend(COMMON_CHROME_INSPECTOR_PORTS.iter().copied());
    ports.extend(discover_listening_devtools_ports().await);
    ports.into_iter().collect()
}

async fn discover_listening_devtools_ports() -> Vec<u16> {
    let output = match timeout(
        DEVTOOLS_LISTENERS_TIMEOUT,
        Command::new("lsof")
            .args(["-nP", "-iTCP", "-sTCP:LISTEN"])
            .output(),
    )
    .await
    {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            debug!("Unable to discover local DevTools listener ports with lsof: {error}");
            return Vec::new();
        }
        Err(_) => {
            debug!("Timed out discovering local DevTools listener ports with lsof");
            return Vec::new();
        }
    };

    if !output.status.success() {
        debug!(
            "lsof DevTools listener discovery exited with status {}",
            output.status
        );
        return Vec::new();
    }

    parse_lsof_devtools_ports(&String::from_utf8_lossy(&output.stdout))
}

fn parse_lsof_devtools_ports(output: &str) -> Vec<u16> {
    let mut ports = BTreeSet::new();
    for line in output.lines().skip(1) {
        if !is_devtools_listener_process(line) {
            continue;
        }
        if let Some(port) = tcp_listener_port(line) {
            ports.insert(port);
        }
    }
    ports.into_iter().collect()
}

fn is_devtools_listener_process(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    [
        "node", "bun", "deno", "chrome", "chromium", "google", "electron", "metro", "react",
        "native", "tns",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn tcp_listener_port(line: &str) -> Option<u16> {
    let tcp = line.split_once("TCP ")?.1;
    let endpoint = tcp.split_whitespace().next()?;
    endpoint.rsplit(':').next()?.parse::<u16>().ok()
}

fn split_http_response(response: &[u8]) -> Option<(String, &[u8])> {
    let separator = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")?;
    let headers = String::from_utf8_lossy(&response[..separator]).into_owned();
    Some((headers, &response[separator + 4..]))
}

fn response_has_complete_body(response: &[u8]) -> bool {
    let Some((headers, body)) = split_http_response(response) else {
        return false;
    };
    content_length(&headers).is_some_and(|length| body.len() >= length)
}

fn content_length(headers: &str) -> Option<usize> {
    headers.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if !name.trim().eq_ignore_ascii_case("content-length") {
            return None;
        }
        value.trim().parse::<usize>().ok()
    })
}

fn metro_target_matches_simulator(
    entry: &Value,
    simulator_name: Option<&str>,
    simulator_device_type_name: Option<&str>,
) -> bool {
    let Some(device_name) = string_value(entry, "deviceName") else {
        return true;
    };
    let device_name = normalized_device_name(&device_name);
    [simulator_name, simulator_device_type_name]
        .into_iter()
        .flatten()
        .map(normalized_device_name)
        .any(|candidate| candidate == device_name)
}

fn is_react_native_metro_target(entry: &Value) -> bool {
    entry.get("reactNative").is_some()
        || string_value(entry, "devtoolsFrontendUrl")
            .is_some_and(|url| url.contains("/debugger-frontend/"))
        || string_value(entry, "webSocketDebuggerUrl")
            .is_some_and(|url| url.contains("/inspector/debug"))
        || string_value(entry, "description")
            .is_some_and(|description| description.to_ascii_lowercase().contains("react native"))
}

fn is_preferred_react_native_metro_target(entry: &Value) -> bool {
    entry
        .pointer("/reactNative/capabilities/prefersFuseboxFrontend")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || string_value(entry, "devtoolsFrontendUrl")
            .is_some_and(|url| url.contains("/rn_fusebox.html"))
}

fn build_metro_target(
    udid: &str,
    http_origin: Option<&str>,
    access_token: Option<&str>,
    port: u16,
    entry: &Value,
) -> ChromeDevToolsTarget {
    let metro_id = string_value(entry, "id").unwrap_or_else(|| "target".to_owned());
    let id = format!("metro-{port}-{}", path_safe_id(&metro_id));
    let app_id = string_value(entry, "appId");
    let title = string_value(entry, "title")
        .or_else(|| app_id.clone())
        .unwrap_or_else(|| "React Native".to_owned());
    let description = string_value(entry, "description")
        .unwrap_or_else(|| "React Native Metro DevTools target".to_owned());
    let web_socket_path = format!("/api/simulators/{udid}/devtools/targets/{id}/socket");
    let web_socket_path = websocket_path_with_access_token(web_socket_path, access_token);
    let web_socket_debugger_url = websocket_url(http_origin.unwrap_or(""), &web_socket_path);
    let devtools_frontend_url = metro_devtools_frontend_url(port, entry, &web_socket_debugger_url);
    let app_name = app_id.clone().or_else(|| Some(title.clone()));

    ChromeDevToolsTarget {
        id,
        title,
        target_type: string_value(entry, "type").unwrap_or_else(|| "node".to_owned()),
        url: app_id
            .as_deref()
            .map(|app_id| format!("metro://{app_id}/{udid}"))
            .unwrap_or_else(|| format!("metro://{udid}/{metro_id}")),
        description,
        devtools_frontend_url,
        web_socket_debugger_url,
        source: SOURCE_REACT_NATIVE_METRO.to_owned(),
        process_identifier: 0,
        bundle_identifier: app_id,
        app_name,
    }
}

fn metro_websocket_debugger_url(port: u16, entry: &Value) -> String {
    chrome_inspector_websocket_debugger_url(port, entry)
        .unwrap_or_else(|| format!("ws://{DEVTOOLS_HOST}:{port}/inspector/debug"))
}

fn build_chrome_inspector_target(
    udid: &str,
    http_origin: Option<&str>,
    port: u16,
    entry: &Value,
) -> Option<ChromeDevToolsTarget> {
    let target_key = string_value(entry, "id")
        .or_else(|| string_value(entry, "webSocketDebuggerUrl"))
        .or_else(|| string_value(entry, "url"))
        .or_else(|| string_value(entry, "title"))?;
    chrome_inspector_websocket_debugger_url(port, entry)?;
    let id = format!("cdp-{port}-{}", path_safe_id(&target_key));
    let title = string_value(entry, "title")
        .or_else(|| string_value(entry, "url"))
        .unwrap_or_else(|| format!("DevTools target on {DEVTOOLS_HOST}:{port}"));
    let description = string_value(entry, "description")
        .unwrap_or_else(|| format!("Chrome DevTools target on {DEVTOOLS_HOST}:{port}"));
    let web_socket_path = format!("/api/simulators/{udid}/devtools/targets/{id}/socket");
    let proxied_web_socket_url = websocket_url(http_origin.unwrap_or(""), &web_socket_path);
    let devtools_frontend_url = chrome_inspector_frontend_url(&proxied_web_socket_url);
    let url = string_value(entry, "url")
        .unwrap_or_else(|| format!("devtools://{DEVTOOLS_HOST}:{port}/{target_key}"));

    Some(ChromeDevToolsTarget {
        id,
        title,
        target_type: string_value(entry, "type").unwrap_or_else(|| "page".to_owned()),
        url,
        description,
        devtools_frontend_url,
        web_socket_debugger_url: proxied_web_socket_url,
        source: SOURCE_CHROME_INSPECTOR.to_owned(),
        process_identifier: 0,
        bundle_identifier: None,
        app_name: None,
    })
}

fn chrome_inspector_websocket_debugger_url(port: u16, entry: &Value) -> Option<String> {
    string_value(entry, "webSocketDebuggerUrl")
        .map(|url| normalize_upstream_websocket_url(port, &url))
        .or_else(|| {
            string_value(entry, "id")
                .map(|id| format!("ws://{DEVTOOLS_HOST}:{port}/devtools/page/{id}"))
        })
}

fn normalize_upstream_websocket_url(port: u16, value: &str) -> String {
    if value.starts_with("ws://") || value.starts_with("wss://") {
        return value.to_owned();
    }
    if value.starts_with('/') {
        return format!("ws://{DEVTOOLS_HOST}:{port}{value}");
    }
    format!("ws://{DEVTOOLS_HOST}:{port}/{value}")
}

fn chrome_inspector_frontend_url(web_socket_debugger_url: &str) -> String {
    let socket_param = web_socket_debugger_url
        .trim_start_matches("ws://")
        .trim_start_matches("wss://");
    format!(
        "/chrome-devtools-ui/inspector.html?ws={}",
        percent_encode_query_component(socket_param)
    )
}

fn proxied_target_port(target_id: &str) -> Result<u16, String> {
    let rest = target_id
        .strip_prefix("metro-")
        .or_else(|| target_id.strip_prefix("cdp-"))
        .ok_or_else(|| "Not a proxied DevTools target id.".to_owned())?;
    rest.split('-')
        .next()
        .and_then(|port| port.parse::<u16>().ok())
        .ok_or_else(|| "Invalid proxied DevTools target id.".to_owned())
}

fn metro_devtools_frontend_url(port: u16, entry: &Value, web_socket_debugger_url: &str) -> String {
    let frontend = string_value(entry, "devtoolsFrontendUrl");
    let asset_path = metro_frontend_asset_path(frontend.as_deref());
    let query = frontend
        .as_deref()
        .and_then(|value| split_path_query(value).1);
    // Reverse-proxy Metro's own (version-matched) Fusebox frontend through the
    // SimDeck origin so the LAN client and Studio app can reach it, with the
    // socket rewritten to the proxied inspector path.
    format!(
        "{}?{}",
        metro_frontend_proxy_base(port, &asset_path),
        metro_frontend_query_with_socket(query, web_socket_debugger_url)
    )
}

fn metro_frontend_proxy_base(port: u16, asset_path: &str) -> String {
    format!("/api/metro/{port}{asset_path}")
}

fn metro_frontend_asset_path(frontend: Option<&str>) -> String {
    let Some(frontend) = frontend else {
        return DEFAULT_METRO_FRONTEND_PATH.to_owned();
    };
    let (path, _) = split_path_query(frontend);
    let path = url_path_component(path);
    if is_metro_frontend_path(path) {
        path.to_owned()
    } else {
        DEFAULT_METRO_FRONTEND_PATH.to_owned()
    }
}

fn url_path_component(value: &str) -> &str {
    let rest = value
        .strip_prefix("http://")
        .or_else(|| value.strip_prefix("https://"));
    match rest {
        Some(rest) => rest.find('/').map(|index| &rest[index..]).unwrap_or("/"),
        None => value,
    }
}

pub fn is_metro_frontend_path(path: &str) -> bool {
    path.starts_with("/debugger-frontend/") || path.starts_with("/rozenite/")
}

pub fn is_metro_proxy_path(path: &str) -> bool {
    path.starts_with('/')
        && !path.starts_with("//")
        && !path
            .bytes()
            .any(|byte| byte < 0x20 || byte == 0x7f || byte == b'\\')
        && !path.split('/').any(|part| part == "..")
}

pub fn is_metro_proxy_method(method: &str) -> bool {
    matches!(method, "GET" | "HEAD" | "POST")
}

fn metro_frontend_query_with_socket(query: Option<&str>, web_socket_debugger_url: &str) -> String {
    let socket_param = web_socket_debugger_url
        .trim_start_matches("ws://")
        .trim_start_matches("wss://");
    let mut params = vec![format!(
        "ws={}",
        percent_encode_query_component(socket_param)
    )];
    if let Some(query) = query {
        let metadata_params = metro_frontend_metadata_params(query);
        params.extend(
            query
                .split('&')
                .filter(|param| {
                    !param.is_empty() && !param.starts_with("ws=") && !param.starts_with("wss=")
                })
                .map(ToOwned::to_owned),
        );
        for (key, param) in metadata_params {
            if !query_has_param(query, &key) {
                params.push(param);
            }
        }
    }
    params.join("&")
}

fn metro_frontend_metadata_params(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter_map(|param| {
            let (key, value) = param.split_once('=')?;
            matches!(key, "ws" | "wss").then_some(value)
        })
        .flat_map(|value| {
            let decoded = percent_decode_query_component(value);
            split_path_query(&decoded)
                .1
                .map(metro_frontend_inspector_params)
                .unwrap_or_default()
        })
        .collect()
}

fn metro_frontend_inspector_params(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter_map(|param| {
            let (key, value) = param.split_once('=')?;
            matches!(key, "device" | "page").then(|| {
                (
                    key.to_owned(),
                    format!("{key}={}", percent_encode_query_component(value)),
                )
            })
        })
        .collect()
}

fn query_has_param(query: &str, needle: &str) -> bool {
    query.split('&').any(|param| {
        param
            .split_once('=')
            .map(|(key, _)| key == needle)
            .unwrap_or(false)
    })
}

fn websocket_path_with_access_token(path: String, access_token: Option<&str>) -> String {
    let Some(access_token) = access_token
        .map(str::trim)
        .filter(|access_token| !access_token.is_empty())
    else {
        return path;
    };
    let separator = if path.contains('?') { '&' } else { '?' };
    format!(
        "{path}{separator}simdeckToken={}",
        percent_encode_query_component(access_token)
    )
}

/// Reverse-proxies a Metro HTTP path over the SimDeck origin. The upstream
/// request omits the browser `Origin` so Metro's dev-middleware does not reject
/// DevTools resources.
pub async fn fetch_metro_resource(
    port: u16,
    path: &str,
    query: Option<&str>,
    method: &str,
    body: Option<&[u8]>,
    content_type: Option<&str>,
) -> Result<ProxiedAsset, String> {
    if !is_metro_proxy_path(path) {
        return Err("Not a safe Metro proxy path.".to_owned());
    }
    if !is_metro_proxy_method(method) {
        return Err("Not an allowed Metro proxy method.".to_owned());
    }
    let target = match query {
        Some(query) => format!("{path}?{query}"),
        None => path.to_owned(),
    };
    let body = body.unwrap_or(&[]);
    let (address, mut stream) =
        connect_loopback_devtools_service(port, METRO_ASSET_TIMEOUT, "Metro").await?;
    let mut request = format!(
        "{method} {target} HTTP/1.1\r\nHost: {address}\r\nAccept: */*\r\nConnection: close\r\n"
    );
    if !body.is_empty() {
        request.push_str(&format!("Content-Length: {}\r\n", body.len()));
        if let Some(content_type) = content_type {
            request.push_str(&format!("Content-Type: {content_type}\r\n"));
        }
    }
    request.push_str("\r\n");
    timeout(METRO_ASSET_TIMEOUT, stream.write_all(request.as_bytes()))
        .await
        .map_err(|_| "Timed out requesting Metro asset.".to_owned())?
        .map_err(|error| format!("Unable to request Metro asset: {error}"))?;
    if !body.is_empty() {
        timeout(METRO_ASSET_TIMEOUT, stream.write_all(body))
            .await
            .map_err(|_| "Timed out writing Metro request body.".to_owned())?
            .map_err(|error| format!("Unable to write Metro request body: {error}"))?;
    }

    let mut response = Vec::new();
    let mut chunk = [0_u8; 16384];
    loop {
        let count = timeout(METRO_ASSET_TIMEOUT, stream.read(&mut chunk))
            .await
            .map_err(|_| "Timed out reading Metro asset.".to_owned())?
            .map_err(|error| format!("Unable to read Metro asset: {error}"))?;
        if count == 0 {
            break;
        }
        response.extend_from_slice(&chunk[..count]);
        if response.len() > METRO_ASSET_MAX_BYTES {
            return Err("Metro asset exceeded the size limit.".to_owned());
        }
        if response_has_complete_body(&response) {
            break;
        }
    }

    let (headers, body) = split_http_response(&response)
        .ok_or_else(|| "Metro returned a malformed HTTP response.".to_owned())?;
    let status = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|status| status.parse::<u16>().ok())
        .unwrap_or(0);
    let content_type = header_value(&headers, "content-type");
    let body = decoded_http_body(&headers, body)?;
    Ok(ProxiedAsset {
        status,
        content_type,
        body,
    })
}

pub struct ProxiedAsset {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

pub fn rewrite_metro_proxy_asset(
    port: u16,
    asset_path: &str,
    content_type: Option<&str>,
    body: Vec<u8>,
) -> Vec<u8> {
    if !is_rewritable_metro_proxy_asset(asset_path, content_type) {
        return body;
    }

    match String::from_utf8(body) {
        Ok(text) => rewrite_metro_proxy_text(port, &text).into_bytes(),
        Err(error) => error.into_bytes(),
    }
}

fn is_rewritable_metro_proxy_asset(asset_path: &str, content_type: Option<&str>) -> bool {
    let path = asset_path
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(asset_path)
        .to_ascii_lowercase();
    if matches!(
        Path::new(&path)
            .extension()
            .and_then(|extension| extension.to_str()),
        Some("css" | "html" | "js" | "json" | "map" | "mjs")
    ) {
        return true;
    }

    let Some(content_type) = content_type else {
        return false;
    };
    let media_type = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    media_type.starts_with("text/")
        || matches!(
            media_type.as_str(),
            "application/ecmascript"
                | "application/javascript"
                | "application/json"
                | "application/manifest+json"
                | "application/x-javascript"
        )
}

fn rewrite_metro_proxy_text(port: u16, text: &str) -> String {
    let proxy_prefix = format!("/api/metro/{port}");
    let text = rewrite_loopback_metro_origins(port, text, &proxy_prefix);
    rewrite_absolute_metro_paths(&text, &proxy_prefix)
}

fn rewrite_loopback_metro_origins(port: u16, text: &str, proxy_prefix: &str) -> String {
    [
        format!("http://127.0.0.1:{port}"),
        format!("http://localhost:{port}"),
        format!("http://[::1]:{port}"),
    ]
    .into_iter()
    .fold(text.to_owned(), |rewritten, origin| {
        rewritten.replace(&origin, proxy_prefix)
    })
}

fn rewrite_absolute_metro_paths(text: &str, proxy_prefix: &str) -> String {
    let mut rewritten = String::with_capacity(text.len());
    let mut index = 0;
    while index < text.len() {
        let remaining = &text[index..];
        let metro_path = ["/rozenite/", "/debugger-frontend/"]
            .into_iter()
            .find(|prefix| remaining.starts_with(prefix));
        if let Some(metro_path) = metro_path {
            let previous = text[..index].chars().next_back();
            if previous
                .map(is_absolute_metro_path_boundary)
                .unwrap_or(true)
            {
                rewritten.push_str(proxy_prefix);
                rewritten.push_str(metro_path);
                index += metro_path.len();
                continue;
            }
        }

        let character = remaining
            .chars()
            .next()
            .expect("remaining text is non-empty");
        rewritten.push(character);
        index += character.len_utf8();
    }
    rewritten
}

fn is_absolute_metro_path_boundary(character: char) -> bool {
    character.is_ascii_whitespace()
        || matches!(
            character,
            '"' | '\'' | '`' | '(' | '[' | '{' | '=' | ':' | ',' | ';'
        )
}

fn header_value(headers: &str, name: &str) -> Option<String> {
    headers.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim()
            .eq_ignore_ascii_case(name)
            .then(|| value.trim().to_owned())
    })
}

fn decoded_http_body(headers: &str, body: &[u8]) -> Result<Vec<u8>, String> {
    if header_value(headers, "transfer-encoding").is_some_and(|value| {
        value
            .split(',')
            .any(|encoding| encoding.trim().eq_ignore_ascii_case("chunked"))
    }) {
        return decode_chunked_body(body);
    }
    Ok(content_length(headers)
        .and_then(|length| body.get(..length))
        .unwrap_or(body)
        .to_vec())
}

fn decode_chunked_body(mut body: &[u8]) -> Result<Vec<u8>, String> {
    let mut decoded = Vec::new();
    loop {
        let line_end = body
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or_else(|| "Metro returned malformed chunked HTTP.".to_owned())?;
        let size_line = std::str::from_utf8(&body[..line_end])
            .map_err(|_| "Metro returned non-UTF-8 chunk metadata.".to_owned())?;
        let size_text = size_line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_text, 16)
            .map_err(|_| "Metro returned invalid chunk size.".to_owned())?;
        body = &body[line_end + 2..];
        if size == 0 {
            return Ok(decoded);
        }
        if body.len() < size + 2 || body.get(size..size + 2) != Some(b"\r\n") {
            return Err("Metro returned truncated chunked HTTP.".to_owned());
        }
        decoded.extend_from_slice(&body[..size]);
        body = &body[size + 2..];
    }
}

fn split_path_query(value: &str) -> (&str, Option<&str>) {
    match value.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (value, None),
    }
}

fn normalized_device_name(value: &str) -> String {
    value
        .trim()
        .strip_prefix("SimDeck ")
        .unwrap_or_else(|| value.trim())
        .to_ascii_lowercase()
}

fn path_safe_id(value: &str) -> String {
    let safe = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    if safe.trim_matches('-').is_empty() {
        "target".to_owned()
    } else {
        safe
    }
}

fn percent_encode_query_component(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn percent_decode_query_component(value: &str) -> String {
    let mut decoded = Vec::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let high = hex_value(bytes[index + 1]);
            let low = hex_value(bytes[index + 2]);
            if let (Some(high), Some(low)) = (high, low) {
                decoded.push((high << 4) | low);
                index += 3;
                continue;
            }
        }
        if bytes[index] == b'+' {
            decoded.push(b' ');
        } else {
            decoded.push(bytes[index]);
        }
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn unique_strings(values: Vec<String>) -> Vec<String> {
    values.into_iter().fold(Vec::new(), |mut unique, value| {
        if !unique.contains(&value) {
            unique.push(value);
        }
        unique
    })
}

pub async fn handle_socket(
    socket: WebSocket,
    target: ChromeDevToolsTargetRuntime,
    query: DevToolsQuery,
) {
    if let Err(error) = handle_socket_inner(socket, target, query).await {
        debug!("Chrome DevTools socket closed: {error}");
    }
}

async fn handle_socket_inner(
    socket: WebSocket,
    target: ChromeDevToolsTargetRuntime,
    query: DevToolsQuery,
) -> Result<(), String> {
    let (mut writer, mut reader) = socket.split();
    let mut state = CdpState::default();

    while let Some(message) = reader.next().await {
        let text = match message {
            Ok(Message::Text(text)) => text.to_string(),
            Ok(Message::Binary(bytes)) => String::from_utf8(bytes.to_vec())
                .map_err(|error| format!("Invalid UTF-8 DevTools frame: {error}"))?,
            Ok(Message::Close(_)) => break,
            Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => continue,
            Err(error) => return Err(format!("WebSocket client error: {error}")),
        };

        let request = serde_json::from_str::<Value>(&text)
            .map_err(|error| format!("Malformed DevTools JSON: {error}"))?;
        let id = request.get("id").cloned();
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let params = request.get("params").cloned().unwrap_or(Value::Null);
        let outcome =
            match handle_cdp_method(&mut state, &target, query.clone(), &method, params).await {
                Ok(outcome) => outcome,
                Err(message) => {
                    let response = json!({
                        "id": id.unwrap_or(Value::Null),
                        "error": {
                            "code": -32000,
                            "message": message,
                        },
                    });
                    writer
                        .send(Message::Text(response.to_string().into()))
                        .await
                        .map_err(|error| format!("Unable to send DevTools response: {error}"))?;
                    continue;
                }
            };
        let response = json!({
            "id": id.unwrap_or(Value::Null),
            "result": outcome.result,
        });

        writer
            .send(Message::Text(response.to_string().into()))
            .await
            .map_err(|error| format!("Unable to send DevTools response: {error}"))?;

        for event in outcome.events {
            writer
                .send(Message::Text(event.to_string().into()))
                .await
                .map_err(|error| format!("Unable to send DevTools event: {error}"))?;
        }
    }

    Ok(())
}

async fn handle_cdp_method(
    state: &mut CdpState,
    target: &ChromeDevToolsTargetRuntime,
    query: DevToolsQuery,
    method: &str,
    params: Value,
) -> Result<CdpResponse, String> {
    let mut events = Vec::new();
    let result = match method {
        "Browser.getVersion" => json!({
            "protocolVersion": "1.3",
            "product": "SimDeck",
            "revision": "simdeck",
            "userAgent": "SimDeck Chrome DevTools Adapter",
            "jsVersion": "0",
        }),
        "Schema.getDomains" => json!({
            "domains": [
                { "name": "Runtime", "version": "1.3" },
                { "name": "DOM", "version": "1.3" },
                { "name": "Page", "version": "1.3" },
                { "name": "CSS", "version": "1.3" },
                { "name": "Log", "version": "1.3" },
                { "name": "Target", "version": "1.3" }
            ],
        }),
        "Target.getTargets" => json!({
            "targetInfos": [target_info(target)],
        }),
        "Target.setDiscoverTargets" | "Target.setAutoAttach" => json!({}),
        "Runtime.enable" => {
            if !state.execution_context_sent {
                events.push(json!({
                    "method": "Runtime.executionContextCreated",
                    "params": {
                        "context": execution_context(target),
                    },
                }));
                state.execution_context_sent = true;
            }
            json!({})
        }
        "Runtime.getIsolateId" => json!({ "id": "simdeck" }),
        "Runtime.runIfWaitingForDebugger"
        | "Runtime.releaseObject"
        | "Runtime.releaseObjectGroup"
        | "Runtime.discardConsoleEntries"
        | "Debugger.enable"
        | "Debugger.disable"
        | "Debugger.setAsyncCallStackDepth"
        | "Debugger.setPauseOnExceptions"
        | "Debugger.setBlackboxPatterns"
        | "Debugger.setBreakpointsActive"
        | "Page.enable"
        | "Page.disable"
        | "Page.setLifecycleEventsEnabled"
        | "Log.enable"
        | "Log.clear"
        | "Console.enable"
        | "Network.enable"
        | "Network.disable"
        | "Network.setCacheDisabled"
        | "Network.setBypassServiceWorker"
        | "DOM.enable"
        | "CSS.enable"
        | "CSS.disable"
        | "Overlay.enable"
        | "Overlay.disable"
        | "Overlay.hideHighlight"
        | "Security.enable"
        | "Performance.enable"
        | "Inspector.enable"
        | "Audits.enable"
        | "Emulation.setFocusEmulationEnabled" => json!({}),
        "Page.getFrameTree" | "Page.getResourceTree" => json!({
            "frameTree": {
                "frame": frame(target),
                "resources": [],
            },
        }),
        "Page.getNavigationHistory" => json!({
            "currentIndex": 0,
            "entries": [{
                "id": 1,
                "url": target.url,
                "userTypedURL": target.url,
                "title": target.title,
                "transitionType": "typed",
            }],
        }),
        "Page.getResourceContent" => json!({
            "content": "",
            "base64Encoded": false,
        }),
        "Runtime.evaluate" => runtime_evaluate(query, &params).await,
        "Runtime.awaitPromise" => json!({
            "result": remote_object(&Value::Null),
        }),
        "Runtime.callFunctionOn" => json!({
            "result": remote_object(&Value::Null),
        }),
        "Runtime.getProperties" => runtime_get_properties(state, &params),
        "Debugger.getScriptSource" => json!({
            "scriptSource": "",
        }),
        "DOM.getDocument" => {
            state.dom.refresh(query.clone()).await?;
            let depth = params
                .get("depth")
                .and_then(Value::as_i64)
                .unwrap_or(2)
                .max(-1);
            json!({
                "root": state.dom.document_node(depth),
            })
        }
        "DOM.getFlattenedDocument" => {
            state.dom.refresh(query.clone()).await?;
            let depth = params
                .get("depth")
                .and_then(Value::as_i64)
                .unwrap_or(-1)
                .max(-1);
            json!({
                "nodes": state.dom.flattened_nodes(depth),
            })
        }
        "DOM.requestChildNodes" => {
            state.dom.refresh(query.clone()).await?;
            let node_id = params.get("nodeId").and_then(Value::as_u64).unwrap_or(1);
            events.push(json!({
                "method": "DOM.setChildNodes",
                "params": {
                    "parentId": node_id,
                    "nodes": state.dom.children_for_node(node_id, 1),
                },
            }));
            json!({})
        }
        "DOM.describeNode" => {
            state.dom.refresh(query.clone()).await?;
            let node_id = params.get("nodeId").and_then(Value::as_u64).unwrap_or(1);
            json!({
                "node": state.dom.describe_node(node_id),
            })
        }
        "DOM.resolveNode" => {
            state.dom.refresh(query.clone()).await?;
            let node_id = params.get("nodeId").and_then(Value::as_u64).unwrap_or(1);
            json!({
                "object": state.dom.remote_node_object(node_id),
            })
        }
        "DOM.getBoxModel" => {
            state.dom.refresh(query.clone()).await?;
            let node_id = params.get("nodeId").and_then(Value::as_u64).unwrap_or(1);
            json!({
                "model": state.dom.box_model(node_id),
            })
        }
        "DOM.pushNodesByBackendIdsToFrontend" => json!({
            "nodeIds": params
                .get("backendNodeIds")
                .and_then(Value::as_array)
                .map(|ids| ids.iter().filter_map(Value::as_u64).collect::<Vec<_>>())
                .unwrap_or_default(),
        }),
        "DOM.querySelector" => json!({ "nodeId": 0 }),
        "DOM.querySelectorAll" => json!({ "nodeIds": [] }),
        "CSS.getMatchedStylesForNode" => json!({
            "matchedCSSRules": [],
            "pseudoElements": [],
            "inherited": [],
            "cssKeyframesRules": [],
        }),
        "CSS.getComputedStyleForNode" => json!({
            "computedStyle": [],
        }),
        "CSS.getPlatformFontsForNode" => json!({
            "fonts": [],
        }),
        "Overlay.highlightNode" | "Overlay.highlightRect" => json!({}),
        _ => json!({}),
    };

    Ok(CdpResponse { events, result })
}

async fn runtime_evaluate(query: DevToolsQuery, params: &Value) -> Value {
    let expression = params
        .get("expression")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if expression.is_empty() {
        return json!({ "result": remote_object(&Value::Null) });
    }

    let hierarchy = match query(
        "View.getHierarchy".to_owned(),
        json!({
            "includeHidden": true,
            "maxDepth": 0,
        }),
    )
    .await
    {
        Ok(value) => value,
        Err(error) => return exception_result(error),
    };
    let Some(root_id) = first_inspector_id(&hierarchy) else {
        return exception_result("No inspectable root view is available.".to_owned());
    };
    match query(
        "View.evaluateScript".to_owned(),
        json!({
            "id": root_id,
            "script": expression,
        }),
    )
    .await
    {
        Ok(value) => json!({ "result": remote_object(value.get("result").unwrap_or(&value)) }),
        Err(error) => exception_result(error),
    }
}

fn runtime_get_properties(state: &mut CdpState, params: &Value) -> Value {
    let object_id = params
        .get("objectId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if let Some(node_id) = object_id
        .strip_prefix("node:")
        .and_then(|id| id.parse().ok())
    {
        return json!({
            "result": state.dom.node_properties(node_id),
            "internalProperties": [],
            "privateProperties": [],
        });
    }
    json!({
        "result": [],
        "internalProperties": [],
        "privateProperties": [],
    })
}

fn exception_result(message: String) -> Value {
    json!({
        "result": {
            "type": "undefined",
            "description": "undefined",
        },
        "exceptionDetails": {
            "text": message,
            "exception": {
                "type": "string",
                "value": message,
                "description": message,
            },
        },
    })
}

impl DomCache {
    async fn refresh(&mut self, query: DevToolsQuery) -> Result<(), String> {
        let hierarchy = query(
            "View.getHierarchy".to_owned(),
            json!({
                "includeHidden": true,
                "maxDepth": 30,
            }),
        )
        .await?;
        self.rebuild(&hierarchy);
        Ok(())
    }

    fn rebuild(&mut self, hierarchy: &Value) {
        self.document_children.clear();
        self.nodes.clear();
        self.next_node_id = 2;
        if let Some(roots) = hierarchy.get("roots").and_then(Value::as_array) {
            for root in roots {
                let node_id = self.insert_node(root);
                self.document_children.push(node_id);
            }
        }
    }

    fn insert_node(&mut self, node: &Value) -> u64 {
        let node_id = self.next_node_id;
        self.next_node_id += 1;
        let children_values = node
            .get("children")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let inspector_id = node
            .get("inspectorId")
            .or_else(|| node.get("id"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let frame = rect_from_node(node);
        self.nodes.insert(
            node_id,
            DomNode {
                backend_node_id: node_id,
                children: Vec::new(),
                frame,
                inspector_id,
                node: node.clone(),
                node_id,
            },
        );
        let children = children_values
            .iter()
            .map(|child| self.insert_node(child))
            .collect::<Vec<_>>();
        if let Some(node) = self.nodes.get_mut(&node_id) {
            node.children = children;
        }
        node_id
    }

    fn document_node(&self, depth: i64) -> Value {
        json!({
            "nodeId": 1,
            "backendNodeId": 1,
            "nodeType": 9,
            "nodeName": "#document",
            "localName": "",
            "nodeValue": "",
            "childNodeCount": self.document_children.len(),
            "children": self.document_children
                .iter()
                .map(|node_id| self.node_value(*node_id, depth))
                .collect::<Vec<_>>(),
        })
    }

    fn flattened_nodes(&self, depth: i64) -> Vec<Value> {
        let mut nodes = vec![self.document_node(0)];
        for node_id in &self.document_children {
            self.append_flattened(*node_id, depth, &mut nodes);
        }
        nodes
    }

    fn append_flattened(&self, node_id: u64, depth: i64, nodes: &mut Vec<Value>) {
        nodes.push(self.node_value(node_id, 0));
        if depth == 0 {
            return;
        }
        if let Some(node) = self.nodes.get(&node_id) {
            let next_depth = if depth < 0 { -1 } else { depth - 1 };
            for child_id in &node.children {
                self.append_flattened(*child_id, next_depth, nodes);
            }
        }
    }

    fn describe_node(&self, node_id: u64) -> Value {
        if node_id == 1 {
            return self.document_node(0);
        }
        self.node_value(node_id, 1)
    }

    fn children_for_node(&self, node_id: u64, depth: i64) -> Vec<Value> {
        if node_id == 1 {
            return self
                .document_children
                .iter()
                .map(|child_id| self.node_value(*child_id, depth))
                .collect();
        }
        self.nodes
            .get(&node_id)
            .map(|node| {
                node.children
                    .iter()
                    .map(|child_id| self.node_value(*child_id, depth))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn node_value(&self, node_id: u64, depth: i64) -> Value {
        let Some(node) = self.nodes.get(&node_id) else {
            return json!({
                "nodeId": node_id,
                "backendNodeId": node_id,
                "nodeType": 1,
                "nodeName": "UNKNOWN",
                "localName": "unknown",
                "nodeValue": "",
                "childNodeCount": 0,
                "attributes": [],
            });
        };
        let node_name = node_name(&node.node);
        let mut value = json!({
            "nodeId": node.node_id,
            "backendNodeId": node.backend_node_id,
            "nodeType": 1,
            "nodeName": node_name.to_ascii_uppercase(),
            "localName": node_name.to_ascii_lowercase(),
            "nodeValue": "",
            "childNodeCount": node.children.len(),
            "attributes": node_attributes(&node.node, node.inspector_id.as_deref()),
        });
        if depth != 0 {
            let next_depth = if depth < 0 { -1 } else { depth - 1 };
            if let Some(object) = value.as_object_mut() {
                object.insert(
                    "children".to_owned(),
                    Value::Array(
                        node.children
                            .iter()
                            .map(|child_id| self.node_value(*child_id, next_depth))
                            .collect(),
                    ),
                );
            }
        }
        value
    }

    fn remote_node_object(&self, node_id: u64) -> Value {
        let description = self
            .nodes
            .get(&node_id)
            .map(|node| node_description(&node.node))
            .unwrap_or_else(|| "#document".to_owned());
        json!({
            "type": "object",
            "subtype": "node",
            "className": "SimDeckNode",
            "description": description,
            "objectId": format!("node:{node_id}"),
        })
    }

    fn box_model(&self, node_id: u64) -> Value {
        let rect = self
            .nodes
            .get(&node_id)
            .and_then(|node| node.frame)
            .unwrap_or(Rect {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 0.0,
            });
        let quad = rect_quad(rect);
        json!({
            "content": quad,
            "padding": quad,
            "border": quad,
            "margin": quad,
            "width": rect.width,
            "height": rect.height,
        })
    }

    fn node_properties(&self, node_id: u64) -> Vec<Value> {
        let Some(node) = self.nodes.get(&node_id) else {
            return Vec::new();
        };
        let mut properties = Vec::new();
        let object = node.node.as_object().cloned().unwrap_or_default();
        for (name, value) in object {
            if name == "children" {
                continue;
            }
            properties.push(json!({
                "name": name,
                "value": remote_object(&value),
                "enumerable": true,
                "configurable": true,
                "writable": false,
            }));
        }
        properties
    }
}

fn target_info(target: &ChromeDevToolsTargetRuntime) -> Value {
    json!({
        "targetId": target.id,
        "type": "page",
        "title": target.title,
        "url": target.url,
        "attached": true,
        "canAccessOpener": false,
        "browserContextId": "simdeck",
    })
}

fn execution_context(target: &ChromeDevToolsTargetRuntime) -> Value {
    json!({
        "id": 1,
        "origin": target.url,
        "name": target.title,
        "uniqueId": format!("simdeck-{}", target.process_identifier),
        "auxData": {
            "isDefault": true,
            "frameId": "simdeck-frame",
            "type": "default",
        },
    })
}

fn frame(target: &ChromeDevToolsTargetRuntime) -> Value {
    json!({
        "id": "simdeck-frame",
        "loaderId": "simdeck-loader",
        "url": target.url,
        "domainAndRegistry": "",
        "securityOrigin": target.url,
        "mimeType": "text/html",
        "title": target.title,
    })
}

fn remote_object(value: &Value) -> Value {
    match value {
        Value::Null => json!({ "type": "object", "subtype": "null", "value": null }),
        Value::Bool(value) => json!({ "type": "boolean", "value": value }),
        Value::Number(value) => json!({
            "type": "number",
            "value": value,
            "description": value.to_string(),
        }),
        Value::String(value) => json!({
            "type": "string",
            "value": value,
            "description": value,
        }),
        Value::Array(_) | Value::Object(_) => json!({
            "type": "object",
            "description": compact_json(value),
            "value": value,
        }),
    }
}

fn compact_json(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| String::new())
}

fn first_inspector_id(hierarchy: &Value) -> Option<String> {
    hierarchy
        .get("roots")
        .and_then(Value::as_array)
        .and_then(|roots| roots.first())
        .and_then(|root| {
            root.get("inspectorId")
                .or_else(|| root.get("id"))
                .and_then(Value::as_str)
        })
        .map(ToOwned::to_owned)
}

fn node_name(node: &Value) -> String {
    string_value(node, "type")
        .or_else(|| string_value(node, "className"))
        .or_else(|| string_value(node, "source"))
        .unwrap_or_else(|| "view".to_owned())
        .replace(
            |character: char| !character.is_ascii_alphanumeric() && character != '-',
            "-",
        )
}

fn node_description(node: &Value) -> String {
    let name = node_name(node);
    let title = string_value(node, "title")
        .or_else(|| string_value(node, "text"))
        .or_else(|| string_value(node, "AXLabel"));
    if let Some(title) = title {
        format!("{name} \"{title}\"")
    } else {
        name
    }
}

fn node_attributes(node: &Value, inspector_id: Option<&str>) -> Vec<Value> {
    let mut attributes = Vec::new();
    push_attribute(&mut attributes, "data-simdeck-id", inspector_id);
    for (name, key) in [
        ("source", "source"),
        ("title", "title"),
        ("text", "text"),
        ("label", "AXLabel"),
        ("value", "AXValue"),
        ("class", "className"),
        ("testid", "reactNative.testID"),
        ("nativeid", "reactNative.nativeID"),
    ] {
        push_attribute(
            &mut attributes,
            name,
            nested_string_value(node, key).as_deref(),
        );
    }
    if let Some(location) = source_location_label(node) {
        push_attribute(&mut attributes, "source-location", Some(&location));
    }
    attributes
}

fn push_attribute(attributes: &mut Vec<Value>, name: &str, value: Option<&str>) {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    attributes.push(Value::String(name.to_owned()));
    attributes.push(Value::String(value.to_owned()));
}

fn source_location_label(node: &Value) -> Option<String> {
    let location = node.get("sourceLocation").and_then(Value::as_object)?;
    let file = location.get("file").and_then(Value::as_str)?;
    let line = location.get("line").and_then(Value::as_i64);
    let column = location.get("column").and_then(Value::as_i64);
    Some(match (line, column) {
        (Some(line), Some(column)) => format!("{file}:{line}:{column}"),
        (Some(line), None) => format!("{file}:{line}"),
        _ => file.to_owned(),
    })
}

fn rect_from_node(node: &Value) -> Option<Rect> {
    ["frameInScreen", "frame", "bounds"]
        .into_iter()
        .filter_map(|key| node.get(key).and_then(rect_from_value))
        .next()
}

fn rect_from_value(value: &Value) -> Option<Rect> {
    let object = value.as_object()?;
    let x = object.get("x").and_then(Value::as_f64)?;
    let y = object.get("y").and_then(Value::as_f64)?;
    let width = object.get("width").and_then(Value::as_f64)?;
    let height = object.get("height").and_then(Value::as_f64)?;
    Some(Rect {
        x,
        y,
        width,
        height,
    })
}

fn rect_quad(rect: Rect) -> Vec<Value> {
    [
        rect.x,
        rect.y,
        rect.x + rect.width,
        rect.y,
        rect.x + rect.width,
        rect.y + rect.height,
        rect.x,
        rect.y + rect.height,
    ]
    .into_iter()
    .map(|value| json!(value))
    .collect()
}

fn string_value(object: &Value, key: &str) -> Option<String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn nested_string_value(object: &Value, key_path: &str) -> Option<String> {
    let mut current = object;
    for key in key_path.split('.') {
        current = current.get(key)?;
    }
    current
        .as_str()
        .map(ToOwned::to_owned)
        .or_else(|| current.as_i64().map(|value| value.to_string()))
        .or_else(|| current.as_u64().map(|value| value.to_string()))
        .or_else(|| current.as_bool().map(|value| value.to_string()))
}

fn source_label(source: &str) -> &'static str {
    match source {
        "react-native" => "React Native",
        SOURCE_REACT_NATIVE_METRO => "React Native Metro",
        SOURCE_CHROME_INSPECTOR => "Chrome Inspector",
        "nativescript" => "NativeScript",
        "swiftui" => "SwiftUI",
        "in-app-inspector" => "UIKit",
        _ => "App",
    }
}

fn websocket_url(http_origin: &str, path: &str) -> String {
    if http_origin.starts_with("https://") {
        format!(
            "wss://{}{}",
            http_origin.trim_start_matches("https://"),
            path
        )
    } else if http_origin.starts_with("http://") {
        format!("ws://{}{}", http_origin.trim_start_matches("http://"), path)
    } else {
        path.to_owned()
    }
}

#[allow(dead_code)]
fn timestamp_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs_f64()
        * 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[test]
    fn upstream_websocket_origin_matches_metro_dev_server() {
        assert_eq!(
            upstream_websocket_origin("ws://127.0.0.1:8082/inspector/debug?device=abc&page=1"),
            Some("http://127.0.0.1:8082".to_owned())
        );
        assert_eq!(
            upstream_websocket_origin("wss://localhost:8081/inspector/debug"),
            Some("https://localhost:8081".to_owned())
        );
        assert_eq!(upstream_websocket_origin("http://127.0.0.1:8082"), None);
    }

    #[test]
    fn metro_devtools_frontend_url_proxies_rozenite_frontend() {
        let entry = json!({
            "devtoolsFrontendUrl": "/rozenite/rn_fusebox.html?ws=127.0.0.1:8081/inspector/debug&device=ios",
            "webSocketDebuggerUrl": "ws://127.0.0.1:8081/inspector/debug"
        });

        let url = metro_devtools_frontend_url(
            8081,
            &entry,
            "ws://127.0.0.1:4310/api/simulators/ABC/devtools/targets/metro-8081-target/socket",
        );

        assert_eq!(
            url,
            "/api/metro/8081/rozenite/rn_fusebox.html?ws=127.0.0.1%3A4310%2Fapi%2Fsimulators%2FABC%2Fdevtools%2Ftargets%2Fmetro-8081-target%2Fsocket&device=ios"
        );
    }

    #[test]
    fn metro_devtools_frontend_url_preserves_rozenite_inspector_metadata_from_encoded_socket() {
        let entry = json!({
            "devtoolsFrontendUrl": "/rozenite/rn_fusebox.html?ws=%2Finspector%2Fdebug%3Fdevice%3D8067dc%26page%3D1&sources.hide_add_folder=true",
            "webSocketDebuggerUrl": "ws://127.0.0.1:8081/inspector/debug?device=8067dc&page=1"
        });

        let url = metro_devtools_frontend_url(
            8081,
            &entry,
            "ws://127.0.0.1:4310/api/simulators/ABC/devtools/targets/metro-8081-target/socket",
        );

        assert_eq!(
            url,
            "/api/metro/8081/rozenite/rn_fusebox.html?ws=127.0.0.1%3A4310%2Fapi%2Fsimulators%2FABC%2Fdevtools%2Ftargets%2Fmetro-8081-target%2Fsocket&sources.hide_add_folder=true&device=8067dc&page=1"
        );
    }

    #[test]
    fn metro_devtools_frontend_url_proxies_absolute_rozenite_origin() {
        let entry = json!({
            "devtoolsFrontendUrl": "http://localhost:8081/rozenite/rn_fusebox.html?panel=redux&ws=localhost:8081/inspector/debug",
            "webSocketDebuggerUrl": "ws://127.0.0.1:8081/inspector/debug"
        });

        let url = metro_devtools_frontend_url(
            8081,
            &entry,
            "ws://simdeck.local:4310/api/simulators/ABC/devtools/targets/metro-8081-target/socket",
        );

        assert_eq!(
            url,
            "/api/metro/8081/rozenite/rn_fusebox.html?ws=simdeck.local%3A4310%2Fapi%2Fsimulators%2FABC%2Fdevtools%2Ftargets%2Fmetro-8081-target%2Fsocket&panel=redux"
        );
    }

    #[test]
    fn metro_devtools_frontend_url_proxies_debugger_frontend() {
        let entry = json!({
            "devtoolsFrontendUrl": "/debugger-frontend/rn_fusebox.html?ws=127.0.0.1:8081/inspector/debug&device=ios",
            "webSocketDebuggerUrl": "ws://127.0.0.1:8081/inspector/debug"
        });

        let url = metro_devtools_frontend_url(
            8081,
            &entry,
            "ws://127.0.0.1:4310/api/simulators/ABC/devtools/targets/metro-8081-target/socket",
        );

        assert_eq!(
            url,
            "/api/metro/8081/debugger-frontend/rn_fusebox.html?ws=127.0.0.1%3A4310%2Fapi%2Fsimulators%2FABC%2Fdevtools%2Ftargets%2Fmetro-8081-target%2Fsocket&device=ios"
        );
    }

    #[test]
    fn metro_devtools_frontend_url_defaults_to_debugger_frontend_when_metro_omits_it() {
        let entry = json!({
            "webSocketDebuggerUrl": "ws://127.0.0.1:8081/inspector/debug"
        });

        let url = metro_devtools_frontend_url(
            8081,
            &entry,
            "ws://127.0.0.1:4310/api/simulators/ABC/devtools/targets/metro-8081-target/socket",
        );

        assert_eq!(
            url,
            "/api/metro/8081/debugger-frontend/rn_fusebox.html?ws=127.0.0.1%3A4310%2Fapi%2Fsimulators%2FABC%2Fdevtools%2Ftargets%2Fmetro-8081-target%2Fsocket"
        );
    }

    #[test]
    fn metro_frontend_asset_path_rejects_unexpected_paths() {
        assert_eq!(
            metro_frontend_asset_path(Some("/inspector/debug?device=x")),
            "/debugger-frontend/rn_fusebox.html"
        );
        assert_eq!(
            metro_frontend_asset_path(Some(
                "http://localhost:8081/debugger-frontend/rn_fusebox.html?ws=x"
            )),
            "/debugger-frontend/rn_fusebox.html"
        );
        assert!(is_metro_frontend_path("/rozenite/panel.js"));
        assert!(!is_metro_frontend_path("/json/list"));
    }

    #[test]
    fn build_metro_target_adds_access_token_to_proxied_socket() {
        let entry = json!({
            "id": "target-1",
            "devtoolsFrontendUrl": "/debugger-frontend/rn_fusebox.html?ws=127.0.0.1:8081/inspector/debug",
            "webSocketDebuggerUrl": "ws://127.0.0.1:8081/inspector/debug"
        });

        let target = build_metro_target(
            "ABC",
            Some("http://127.0.0.1:4310"),
            Some("secret token"),
            8081,
            &entry,
        );

        assert!(target.web_socket_debugger_url.ends_with(
            "/api/simulators/ABC/devtools/targets/metro-8081-target-1/socket?simdeckToken=secret%20token"
        ));
        assert!(target
            .devtools_frontend_url
            .starts_with("/api/metro/8081/debugger-frontend/rn_fusebox.html?"));
        assert!(target.devtools_frontend_url.contains(
            "ws=127.0.0.1%3A4310%2Fapi%2Fsimulators%2FABC%2Fdevtools%2Ftargets%2Fmetro-8081-target-1%2Fsocket%3FsimdeckToken%3Dsecret%2520token"
        ));
    }

    #[test]
    fn metro_proxy_path_allows_full_metro_server_surface() {
        assert!(is_metro_proxy_path("/debugger-frontend/rn_fusebox.html"));
        assert!(is_metro_proxy_path("/rozenite/panel.js"));
        assert!(is_metro_proxy_path("/json/list"));
        assert!(is_metro_proxy_path("/symbolicate"));
        assert!(is_metro_proxy_path("/index.bundle"));
        assert!(!is_metro_proxy_path(""));
        assert!(!is_metro_proxy_path("http://localhost:8081/json/list"));
        assert!(!is_metro_proxy_path("/../Info.plist"));
        assert!(is_metro_proxy_method("GET"));
        assert!(is_metro_proxy_method("HEAD"));
        assert!(is_metro_proxy_method("POST"));
        assert!(!is_metro_proxy_method("DELETE"));
    }

    #[test]
    fn metro_proxy_rewrites_absolute_rozenite_assets_to_port_scoped_proxy() {
        let body = br#"
            import * as UI from "/rozenite/ui/legacy/legacy.js";
            import "/debugger-frontend/models/react_native/react_native.js";
            const url = new URL(location.href);
            url.pathname = "/rozenite/plugins/@rozenite_controls-plugin";
            const nested = "https://example.com/rozenite/docs";
            const alreadyProxied = "/api/metro/8091/rozenite/host.js";
        "#;

        let rewritten = rewrite_metro_proxy_asset(
            8091,
            "/rozenite/host.js",
            Some("application/javascript"),
            body.to_vec(),
        );
        let rewritten = String::from_utf8(rewritten).unwrap();

        assert!(rewritten.contains("from \"/api/metro/8091/rozenite/ui/legacy/legacy.js\""));
        assert!(rewritten.contains(
            "import \"/api/metro/8091/debugger-frontend/models/react_native/react_native.js\""
        ));
        assert!(rewritten.contains(
            "url.pathname = \"/api/metro/8091/rozenite/plugins/@rozenite_controls-plugin\""
        ));
        assert!(rewritten.contains("https://example.com/rozenite/docs"));
        assert!(rewritten.contains("\"/api/metro/8091/rozenite/host.js\""));
    }

    #[test]
    fn metro_proxy_rewrites_loopback_metro_origins() {
        let rewritten = rewrite_metro_proxy_asset(
            8091,
            "/rozenite/plugin.json",
            Some("application/json"),
            br#"{"script":"http://localhost:8091/rozenite/plugin.js","ipv4":"http://127.0.0.1:8091/debugger-frontend/main.js"}"#.to_vec(),
        );
        let rewritten = String::from_utf8(rewritten).unwrap();

        assert!(rewritten.contains("\"/api/metro/8091/rozenite/plugin.js\""));
        assert!(rewritten.contains("\"/api/metro/8091/debugger-frontend/main.js\""));
    }

    #[tokio::test]
    async fn fetch_metro_asset_proxies_full_path_without_origin_header() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![0_u8; 1024];
            let count = socket.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..count]);
            assert!(request.starts_with("GET /json/list?platform=ios HTTP/1.1\r\n"));
            assert!(!request.to_ascii_lowercase().contains("\r\norigin:"));
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n[]",
                )
                .await
                .unwrap();
        });

        let asset =
            fetch_metro_resource(port, "/json/list", Some("platform=ios"), "GET", None, None)
                .await
                .unwrap();

        server.await.unwrap();
        assert_eq!(asset.status, 200);
        assert_eq!(asset.content_type.as_deref(), Some("application/json"));
        assert_eq!(asset.body, b"[]");
    }

    #[tokio::test]
    async fn fetch_metro_resource_falls_back_to_ipv6_loopback() {
        let listener = TcpListener::bind("[::1]:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![0_u8; 1024];
            let count = socket.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..count]);
            assert!(request.starts_with("GET /rozenite/rn_fusebox.html HTTP/1.1\r\n"));
            assert!(request.contains(&format!("\r\nHost: [::1]:{port}\r\n")));
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 11\r\n\r\nroze-ready!",
                )
                .await
                .unwrap();
        });

        let asset =
            fetch_metro_resource(port, "/rozenite/rn_fusebox.html", None, "GET", None, None)
                .await
                .unwrap();

        server.await.unwrap();
        assert_eq!(asset.status, 200);
        assert_eq!(asset.content_type.as_deref(), Some("text/html"));
        assert_eq!(asset.body, b"roze-ready!");
    }

    #[tokio::test]
    async fn fetch_metro_resource_proxies_post_body_for_symbolication() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut chunk = [0_u8; 1024];
            loop {
                let count = socket.read(&mut chunk).await.unwrap();
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..count]);
                if request
                    .windows(12)
                    .any(|window| window == b"{\"stack\":[]}")
                {
                    break;
                }
            }
            let request = String::from_utf8_lossy(&request);
            assert!(request.starts_with("POST /symbolicate HTTP/1.1\r\n"));
            assert!(request.contains("\r\nContent-Length: 12\r\n"));
            assert!(request.contains("\r\nContent-Type: application/json\r\n"));
            assert!(request.ends_with("{\"stack\":[]}"));
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r\n0\r\n\r\n",
                )
                .await
                .unwrap();
        });

        let asset = fetch_metro_resource(
            port,
            "/symbolicate",
            None,
            "POST",
            Some(br#"{"stack":[]}"#),
            Some("application/json"),
        )
        .await
        .unwrap();

        server.await.unwrap();
        assert_eq!(asset.status, 200);
        assert_eq!(asset.content_type.as_deref(), Some("application/json"));
        assert_eq!(asset.body, b"{}");
    }

    #[test]
    fn decoded_http_body_decodes_chunked_metro_responses() {
        let body = decoded_http_body(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n",
            b"5\r\nhello\r\n6;ext=1\r\n world\r\n0\r\n\r\n",
        )
        .unwrap();

        assert_eq!(body, b"hello world");
    }
}
