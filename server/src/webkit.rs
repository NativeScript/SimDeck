use crate::error::AppError;
use axum::extract::ws::{Message, WebSocket};
use futures::{SinkExt, StreamExt};
use plist::{Dictionary, Value};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::process::Command;
use tokio::time::{sleep, timeout, Instant};
use tracing::{debug, warn};

const WEBINSPECTORD_SOCKET_NAME: &str = "com.apple.webinspectord_sim.socket";
const WEBKIT_PACKET_MAX_LEN: usize = 64 * 1024 * 1024;
const WEBKIT_DISCOVERY_TIMEOUT: Duration = Duration::from_millis(5000);
const WEBKIT_DISCOVERY_ATTEMPT_TIMEOUT: Duration = Duration::from_millis(1600);
const WEBKIT_DISCOVERY_REFRESH_INTERVAL: Duration = Duration::from_millis(400);
const WEBKIT_DISCOVERY_CACHE_TTL: Duration = Duration::from_secs(30);
const WEBKIT_SOCKET_ACTIVATION_DELAY: Duration = Duration::from_millis(200);
const WEBKIT_IO_TIMEOUT: Duration = Duration::from_secs(4);

static WEBKIT_ID_COUNTER: AtomicU64 = AtomicU64::new(1);
static WEBKIT_DISCOVERY_CACHE: OnceLock<Mutex<HashMap<String, CachedWebKitDiscovery>>> =
    OnceLock::new();

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebKitTarget {
    pub id: String,
    pub app_id: String,
    pub app_name: Option<String>,
    pub page_id: u64,
    pub title: Option<String>,
    pub url: Option<String>,
    pub kind: String,
    pub inspector_url: String,
    pub web_socket_url: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebKitTargetDiscovery {
    pub udid: String,
    pub socket_path: Option<String>,
    pub targets: Vec<WebKitTarget>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug)]
struct WebKitApplication {
    id: String,
    name: Option<String>,
    is_proxy: bool,
}

#[derive(Clone, Debug)]
struct WebKitPage {
    app_id: String,
    page_id: u64,
    title: Option<String>,
    url: Option<String>,
}

#[derive(Clone, Debug)]
struct WebKitSocket {
    path: String,
}

#[derive(Default)]
struct LsofProcess {
    belongs_to_udid: bool,
    sockets: Vec<String>,
}

#[derive(Clone)]
struct CachedWebKitDiscovery {
    discovery: WebKitTargetDiscovery,
    cached_at: SystemTime,
}

pub async fn discover_targets(
    udid: &str,
    http_origin: Option<&str>,
) -> Result<WebKitTargetDiscovery, AppError> {
    let socket = match discover_webinspector_socket(udid).await? {
        Some(socket) => socket,
        None => {
            return Ok(WebKitTargetDiscovery {
                udid: udid.to_owned(),
                socket_path: None,
                targets: Vec::new(),
                warnings: vec![format!(
                    "No WebKit Remote Inspector socket was found for simulator {udid}. Boot the simulator and open an inspectable Safari page or WKWebView."
                )],
            });
        }
    };

    let deadline = Instant::now() + WEBKIT_DISCOVERY_TIMEOUT;
    let mut attempts = 0usize;
    let mut last_discovery: Option<WebKitTargetDiscovery> = None;
    let mut accumulated_warnings = Vec::new();
    loop {
        attempts += 1;
        match discover_targets_once(udid, http_origin, &socket).await {
            Ok(discovery) if !discovery.targets.is_empty() => {
                cache_webkit_discovery(&discovery);
                return Ok(discovery);
            }
            Ok(discovery) => {
                for warning in &discovery.warnings {
                    push_unique_warning(&mut accumulated_warnings, warning);
                }
                last_discovery = Some(discovery);
            }
            Err(error) => {
                push_unique_warning(&mut accumulated_warnings, error.to_string());
            }
        }

        if Instant::now() >= deadline {
            break;
        }
        sleep(Duration::from_millis(250)).await;
    }

    let mut discovery = last_discovery.unwrap_or_else(|| WebKitTargetDiscovery {
        udid: udid.to_owned(),
        socket_path: Some(socket.path.clone()),
        targets: Vec::new(),
        warnings: Vec::new(),
    });
    discovery.warnings = accumulated_warnings;
    if attempts > 1 {
        push_unique_warning(
            &mut discovery.warnings,
            format!("Retried WebKit target discovery {attempts} times while simulator webinspectord was settling."),
        );
    }

    if let Some(cached) = cached_webkit_discovery(
        udid,
        discovery.socket_path.clone(),
        discovery.warnings.clone(),
    ) {
        return Ok(cached);
    }

    Ok(discovery)
}

async fn discover_targets_once(
    udid: &str,
    http_origin: Option<&str>,
    socket: &WebKitSocket,
) -> Result<WebKitTargetDiscovery, AppError> {
    let mut stream = timeout(WEBKIT_IO_TIMEOUT, UnixStream::connect(&socket.path))
        .await
        .map_err(|_| AppError::native("Timed out connecting to simulator webinspectord."))?
        .map_err(|error| {
            AppError::native(format!(
                "Unable to connect to simulator webinspectord at {}: {error}",
                socket.path
            ))
        })?;

    let connection_id = new_remote_inspector_id();
    send_rpc(
        &mut stream,
        "_rpc_reportIdentifier:",
        rpc_args(&connection_id),
    )
    .await?;
    sleep(WEBKIT_SOCKET_ACTIVATION_DELAY).await;
    send_rpc(
        &mut stream,
        "_rpc_getConnectedApplications:",
        rpc_args(&connection_id),
    )
    .await?;

    let deadline = Instant::now() + WEBKIT_DISCOVERY_ATTEMPT_TIMEOUT;
    let mut next_listing_refresh = Instant::now() + WEBKIT_DISCOVERY_REFRESH_INTERVAL;
    let mut applications: BTreeMap<String, WebKitApplication> = BTreeMap::new();
    let mut requested_listings: HashSet<String> = HashSet::new();
    let mut pages: BTreeMap<(String, u64), WebKitPage> = BTreeMap::new();
    let mut warnings = Vec::new();

    while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        let read_timeout = remaining.min(Duration::from_millis(250));
        let packet = match timeout(read_timeout, read_packet(&mut stream)).await {
            Ok(Ok(packet)) => packet,
            Ok(Err(error)) => {
                warnings.push(error.to_string());
                break;
            }
            Err(_) if !pages.is_empty() => break,
            Err(_) if Instant::now() < deadline => {
                if Instant::now() >= next_listing_refresh {
                    send_rpc(
                        &mut stream,
                        "_rpc_getConnectedApplications:",
                        rpc_args(&connection_id),
                    )
                    .await?;
                    for app_id in applications.keys() {
                        send_forward_get_listing(&mut stream, &connection_id, app_id).await?;
                    }
                    next_listing_refresh = Instant::now() + WEBKIT_DISCOVERY_REFRESH_INTERVAL;
                }
                continue;
            }
            Err(_) => break,
        };

        let message = match parse_rpc_message(&packet) {
            Ok(message) => message,
            Err(error) => {
                warnings.push(error.to_string());
                continue;
            }
        };

        match message.selector.as_str() {
            "_rpc_reportConnectedApplicationList:" => {
                for app in parse_application_list(&message.args) {
                    if requested_listings.insert(app.id.clone()) {
                        send_forward_get_listing(&mut stream, &connection_id, &app.id).await?;
                    }
                    applications.insert(app.id.clone(), app);
                }
            }
            "_rpc_applicationConnected:" => {
                if let Some(app) = parse_application(&message.args) {
                    if requested_listings.insert(app.id.clone()) {
                        send_forward_get_listing(&mut stream, &connection_id, &app.id).await?;
                    }
                    applications.insert(app.id.clone(), app);
                }
            }
            "_rpc_applicationUpdated:" => {
                if let Some(app_id) = string_value(&message.args, "WIRApplicationIdentifierKey") {
                    if requested_listings.insert(app_id.clone()) {
                        send_forward_get_listing(&mut stream, &connection_id, &app_id).await?;
                    }
                    applications
                        .entry(app_id.clone())
                        .or_insert(WebKitApplication {
                            id: app_id,
                            name: string_value(&message.args, "WIRApplicationNameKey"),
                            is_proxy: bool_value(&message.args, "WIRIsApplicationProxyKey")
                                .unwrap_or(false),
                        });
                }
            }
            "_rpc_applicationSentListing:" => {
                for page in parse_page_listing(&message.args) {
                    pages.insert((page.app_id.clone(), page.page_id), page);
                }
            }
            "_rpc_applicationDisconnected:" => {
                if let Some(app) = parse_application(&message.args) {
                    applications.remove(&app.id);
                    pages.retain(|(app_id, _), _| app_id != &app.id);
                }
            }
            "_rpc_reportSetup:"
            | "_rpc_reportConnectedDriverList:"
            | "_rpc_reportCurrentState:" => {}
            selector => debug!("Ignoring WebKit inspector discovery selector {selector}."),
        }
    }

    let origin = http_origin.unwrap_or("");
    let mut targets = pages
        .into_values()
        .map(|page| {
            let app = applications.get(&page.app_id);
            webkit_target(udid, origin, page, app)
        })
        .collect::<Vec<_>>();
    targets.sort_by(|lhs, rhs| {
        lhs.app_name
            .cmp(&rhs.app_name)
            .then(lhs.title.cmp(&rhs.title))
            .then(lhs.url.cmp(&rhs.url))
            .then(lhs.page_id.cmp(&rhs.page_id))
    });

    let discovery = WebKitTargetDiscovery {
        udid: udid.to_owned(),
        socket_path: Some(socket.path.clone()),
        targets,
        warnings,
    };

    Ok(discovery)
}

fn cache_webkit_discovery(discovery: &WebKitTargetDiscovery) {
    if discovery.targets.is_empty() {
        return;
    }
    let cache = WEBKIT_DISCOVERY_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let Ok(mut cache) = cache.lock() else {
        return;
    };
    cache.insert(
        discovery.udid.clone(),
        CachedWebKitDiscovery {
            discovery: discovery.clone(),
            cached_at: SystemTime::now(),
        },
    );
}

fn cached_webkit_discovery(
    udid: &str,
    socket_path: Option<String>,
    warnings: Vec<String>,
) -> Option<WebKitTargetDiscovery> {
    let cache = WEBKIT_DISCOVERY_CACHE.get()?;
    let Ok(cache) = cache.lock() else {
        return None;
    };
    let cached = cache.get(udid)?;
    if cached.discovery.targets.is_empty()
        || cached.cached_at.elapsed().ok()? > WEBKIT_DISCOVERY_CACHE_TTL
    {
        return None;
    }

    let mut discovery = cached.discovery.clone();
    if socket_path.is_some() {
        discovery.socket_path = socket_path;
    }
    discovery.warnings = warnings;
    push_unique_warning(
        &mut discovery.warnings,
        "WebKit target discovery returned an empty listing; showing the last known targets while simulator webinspectord settles.",
    );
    for warning in &cached.discovery.warnings {
        push_unique_warning(&mut discovery.warnings, warning);
    }
    Some(discovery)
}

fn push_unique_warning(warnings: &mut Vec<String>, warning: impl AsRef<str>) {
    let warning = warning.as_ref();
    if warnings.iter().any(|existing| existing == warning) {
        return;
    }
    warnings.push(warning.to_owned());
}

pub async fn attach_websocket(udid: String, target_id: String, socket: WebSocket) {
    if let Err(error) = attach_websocket_inner(&udid, &target_id, socket).await {
        warn!("WebKit inspector socket failed for {udid}/{target_id}: {error}");
    }
}

async fn attach_websocket_inner(
    udid: &str,
    target_id: &str,
    socket: WebSocket,
) -> Result<(), AppError> {
    let (app_id, page_id) = decode_target_id(target_id)?;
    let webkit_socket = discover_webinspector_socket(udid).await?.ok_or_else(|| {
        AppError::not_found(format!(
            "No WebKit Remote Inspector socket was found for simulator {udid}."
        ))
    })?;
    let stream = timeout(WEBKIT_IO_TIMEOUT, UnixStream::connect(&webkit_socket.path))
        .await
        .map_err(|_| AppError::native("Timed out connecting to simulator webinspectord."))?
        .map_err(|error| {
            AppError::native(format!(
                "Unable to connect to simulator webinspectord at {}: {error}",
                webkit_socket.path
            ))
        })?;

    let connection_id = new_remote_inspector_id();
    let sender_id = new_remote_inspector_id();
    let (mut inspector_reader, mut inspector_writer) = stream.into_split();
    send_rpc(
        &mut inspector_writer,
        "_rpc_reportIdentifier:",
        rpc_args(&connection_id),
    )
    .await?;
    sleep(WEBKIT_SOCKET_ACTIVATION_DELAY).await;
    send_forward_socket_setup(
        &mut inspector_writer,
        &connection_id,
        &app_id,
        page_id,
        &sender_id,
    )
    .await?;

    let (mut client_writer, mut client_reader) = socket.split();
    let mut closed_cleanly = false;
    loop {
        tokio::select! {
            client_message = client_reader.next() => {
                match client_message {
                    Some(Ok(Message::Text(text))) => {
                        send_forward_socket_data(
                            &mut inspector_writer,
                            &connection_id,
                            &app_id,
                            page_id,
                            &sender_id,
                            text.as_bytes(),
                        ).await?;
                    }
                    Some(Ok(Message::Binary(bytes))) => {
                        send_forward_socket_data(
                            &mut inspector_writer,
                            &connection_id,
                            &app_id,
                            page_id,
                            &sender_id,
                            bytes.as_ref(),
                        ).await?;
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        closed_cleanly = true;
                        break;
                    }
                    Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => {}
                    Some(Err(error)) => return Err(AppError::internal(format!("WebSocket client error: {error}"))),
                }
            }
            inspector_packet = read_packet(&mut inspector_reader) => {
                let packet = inspector_packet?;
                let message = parse_rpc_message(&packet)?;
                match message.selector.as_str() {
                    "_rpc_applicationSentData:" => {
                        if string_value(&message.args, "WIRDestinationKey").as_deref() == Some(sender_id.as_str()) {
                            if let Some(data) = data_value(&message.args, "WIRMessageDataKey") {
                                let text = String::from_utf8(data).unwrap_or_default();
                                client_writer
                                    .send(Message::Text(text.into()))
                                    .await
                                    .map_err(|error| AppError::internal(format!("WebSocket send failed: {error}")))?;
                            }
                        }
                    }
                    "_rpc_applicationDisconnected:" => break,
                    selector => debug!("Ignoring WebKit inspector attach selector {selector}."),
                }
            }
        }
    }

    let _ = send_forward_did_close(
        &mut inspector_writer,
        &connection_id,
        &app_id,
        page_id,
        &sender_id,
    )
    .await;

    if !closed_cleanly {
        let _ = client_writer.send(Message::Close(None)).await;
    }
    Ok(())
}

async fn discover_webinspector_socket(udid: &str) -> Result<Option<WebKitSocket>, AppError> {
    let output = timeout(
        Duration::from_secs(2),
        Command::new("/usr/sbin/lsof")
            .args(["-nP", "-c", "webinspectord", "-F", "pn"])
            .output(),
    )
    .await
    .map_err(|_| AppError::native("Timed out listing webinspectord sockets."))?
    .map_err(|error| AppError::native(format!("Unable to run lsof: {error}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::native(format!(
            "Unable to list webinspectord sockets: {}",
            stderr.trim()
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut processes: HashMap<String, LsofProcess> = HashMap::new();
    let udid_marker = format!("/Devices/{udid}/");
    let mut current_pid: Option<String> = None;
    for line in stdout.lines() {
        if let Some(pid) = line.strip_prefix('p') {
            current_pid = Some(pid.to_owned());
            processes.entry(pid.to_owned()).or_default();
            continue;
        }
        let Some(name) = line.strip_prefix('n') else {
            continue;
        };
        let Some(pid) = current_pid.as_ref() else {
            continue;
        };
        let process = processes.entry(pid.clone()).or_default();
        if name.contains(&udid_marker) {
            process.belongs_to_udid = true;
        }
        if name.ends_with(WEBINSPECTORD_SOCKET_NAME) {
            process.sockets.push(name.to_owned());
        }
    }

    let mut candidates = processes
        .into_values()
        .filter(|process| process.belongs_to_udid)
        .flat_map(|process| process.sockets)
        .filter(|path| Path::new(path).exists())
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.dedup();

    if let Some(path) = candidates.into_iter().next() {
        return Ok(Some(WebKitSocket { path }));
    }

    discover_launchd_webinspector_socket(udid).await
}

async fn discover_launchd_webinspector_socket(
    udid: &str,
) -> Result<Option<WebKitSocket>, AppError> {
    let output = match timeout(
        Duration::from_secs(2),
        Command::new("xcrun")
            .args([
                "simctl",
                "spawn",
                udid,
                "launchctl",
                "getenv",
                "RWI_LISTEN_SOCKET",
            ])
            .output(),
    )
    .await
    {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            debug!("Unable to query simulator launchd WebKit socket for {udid}: {error}");
            return Ok(None);
        }
        Err(_) => {
            debug!("Timed out querying simulator launchd WebKit socket for {udid}.");
            return Ok(None);
        }
    };

    if !output.status.success() {
        debug!(
            "Simulator launchd did not report WebKit socket for {udid}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        return Ok(None);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let Some(path) = sanitize_webinspectord_socket_path(&stdout) else {
        return Ok(None);
    };

    if !Path::new(path).exists() {
        debug!("Simulator launchd WebKit socket for {udid} does not exist: {path}");
        return Ok(None);
    }

    Ok(Some(WebKitSocket {
        path: path.to_owned(),
    }))
}

fn sanitize_webinspectord_socket_path(raw: &str) -> Option<&str> {
    let path = raw.trim();
    if path.is_empty() || !path.ends_with(WEBINSPECTORD_SOCKET_NAME) {
        return None;
    }
    Some(path)
}

async fn send_forward_get_listing<W: AsyncWrite + Unpin>(
    writer: &mut W,
    connection_id: &str,
    app_id: &str,
) -> Result<(), AppError> {
    let mut args = rpc_args(connection_id);
    args.insert(
        "WIRApplicationIdentifierKey".to_owned(),
        Value::String(app_id.to_owned()),
    );
    send_rpc(writer, "_rpc_forwardGetListing:", args).await
}

async fn send_forward_socket_setup<W: AsyncWrite + Unpin>(
    writer: &mut W,
    connection_id: &str,
    app_id: &str,
    page_id: u64,
    sender_id: &str,
) -> Result<(), AppError> {
    let mut args = rpc_args(connection_id);
    args.insert(
        "WIRApplicationIdentifierKey".to_owned(),
        Value::String(app_id.to_owned()),
    );
    args.insert("WIRAutomaticallyPause".to_owned(), Value::Boolean(false));
    args.insert(
        "WIRPageIdentifierKey".to_owned(),
        Value::Integer(page_id.into()),
    );
    args.insert(
        "WIRSenderKey".to_owned(),
        Value::String(sender_id.to_owned()),
    );
    send_rpc(writer, "_rpc_forwardSocketSetup:", args).await
}

async fn send_forward_socket_data<W: AsyncWrite + Unpin>(
    writer: &mut W,
    connection_id: &str,
    app_id: &str,
    page_id: u64,
    sender_id: &str,
    data: &[u8],
) -> Result<(), AppError> {
    let mut args = rpc_args(connection_id);
    args.insert(
        "WIRApplicationIdentifierKey".to_owned(),
        Value::String(app_id.to_owned()),
    );
    args.insert(
        "WIRPageIdentifierKey".to_owned(),
        Value::Integer(page_id.into()),
    );
    args.insert(
        "WIRSenderKey".to_owned(),
        Value::String(sender_id.to_owned()),
    );
    args.insert("WIRSocketDataKey".to_owned(), Value::Data(data.to_vec()));
    send_rpc(writer, "_rpc_forwardSocketData:", args).await
}

async fn send_forward_did_close<W: AsyncWrite + Unpin>(
    writer: &mut W,
    connection_id: &str,
    app_id: &str,
    page_id: u64,
    sender_id: &str,
) -> Result<(), AppError> {
    let mut args = rpc_args(connection_id);
    args.insert(
        "WIRApplicationIdentifierKey".to_owned(),
        Value::String(app_id.to_owned()),
    );
    args.insert(
        "WIRPageIdentifierKey".to_owned(),
        Value::Integer(page_id.into()),
    );
    args.insert(
        "WIRSenderKey".to_owned(),
        Value::String(sender_id.to_owned()),
    );
    send_rpc(writer, "_rpc_forwardDidClose:", args).await
}

async fn send_rpc<W: AsyncWrite + Unpin>(
    writer: &mut W,
    selector: &str,
    args: Dictionary,
) -> Result<(), AppError> {
    let mut message = Dictionary::new();
    message.insert("__selector".to_owned(), Value::String(selector.to_owned()));
    message.insert("__argument".to_owned(), Value::Dictionary(args));

    let mut payload = Vec::new();
    plist::to_writer_binary(&mut payload, &Value::Dictionary(message))
        .map_err(|error| AppError::internal(format!("Unable to encode WebKit plist: {error}")))?;
    if payload.len() > WEBKIT_PACKET_MAX_LEN {
        return Err(AppError::internal("WebKit plist packet is too large."));
    }

    let mut packet = Vec::with_capacity(payload.len() + 4);
    packet.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    packet.extend_from_slice(&payload);
    writer
        .write_all(&packet)
        .await
        .map_err(|error| AppError::native(format!("Unable to write WebKit packet: {error}")))?;
    Ok(())
}

async fn read_packet<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Vec<u8>, AppError> {
    let mut header = [0u8; 4];
    reader.read_exact(&mut header).await.map_err(|error| {
        AppError::native(format!("Unable to read WebKit packet header: {error}"))
    })?;
    let length = u32::from_be_bytes(header) as usize;
    if length > WEBKIT_PACKET_MAX_LEN {
        return Err(AppError::bad_request(format!(
            "WebKit packet length {length} exceeds maximum {WEBKIT_PACKET_MAX_LEN}."
        )));
    }
    let mut payload = vec![0u8; length];
    reader
        .read_exact(&mut payload)
        .await
        .map_err(|error| AppError::native(format!("Unable to read WebKit packet body: {error}")))?;
    Ok(payload)
}

struct RpcMessage {
    selector: String,
    args: Dictionary,
}

fn parse_rpc_message(payload: &[u8]) -> Result<RpcMessage, AppError> {
    let value = Value::from_reader(Cursor::new(payload))
        .map_err(|error| AppError::internal(format!("Unable to parse WebKit plist: {error}")))?;
    let dict = value
        .as_dictionary()
        .ok_or_else(|| AppError::internal("WebKit packet was not a dictionary."))?;
    let selector = dict
        .get("__selector")
        .and_then(Value::as_string)
        .ok_or_else(|| AppError::internal("WebKit packet did not include __selector."))?
        .to_owned();
    let args = dict
        .get("__argument")
        .and_then(Value::as_dictionary)
        .cloned()
        .unwrap_or_default();
    Ok(RpcMessage { selector, args })
}

fn parse_application_list(args: &Dictionary) -> Vec<WebKitApplication> {
    let Some(apps) = args
        .get("WIRApplicationDictionaryKey")
        .and_then(Value::as_dictionary)
    else {
        return Vec::new();
    };

    apps.values()
        .filter_map(Value::as_dictionary)
        .filter_map(parse_application)
        .collect()
}

fn parse_application(args: &Dictionary) -> Option<WebKitApplication> {
    let id = string_value(args, "WIRApplicationIdentifierKey")?;
    Some(WebKitApplication {
        id,
        name: string_value(args, "WIRApplicationNameKey"),
        is_proxy: bool_value(args, "WIRIsApplicationProxyKey").unwrap_or(false),
    })
}

fn parse_page_listing(args: &Dictionary) -> Vec<WebKitPage> {
    let Some(app_id) = string_value(args, "WIRApplicationIdentifierKey") else {
        return Vec::new();
    };
    let Some(listing) = args.get("WIRListingKey").and_then(Value::as_dictionary) else {
        return Vec::new();
    };

    listing
        .values()
        .filter_map(Value::as_dictionary)
        .filter_map(|page| {
            let page_id = integer_value(page, "WIRPageIdentifierKey")?;
            Some(WebKitPage {
                app_id: app_id.clone(),
                page_id,
                title: string_value(page, "WIRTitleKey"),
                url: string_value(page, "WIRURLKey"),
            })
        })
        .collect()
}

fn rpc_args(connection_id: &str) -> Dictionary {
    let mut args = Dictionary::new();
    args.insert(
        "WIRConnectionIdentifierKey".to_owned(),
        Value::String(connection_id.to_owned()),
    );
    args
}

fn webkit_target(
    udid: &str,
    http_origin: &str,
    page: WebKitPage,
    app: Option<&WebKitApplication>,
) -> WebKitTarget {
    let id = encode_target_id(&page.app_id, page.page_id);
    let web_socket_url = format!("/api/simulators/{udid}/webkit/targets/{id}/socket");
    let absolute_ws = websocket_url(http_origin, &web_socket_url);
    let inspector_url = format!(
        "/webkit-inspector-ui/Main.html?ws={}",
        absolute_ws
            .trim_start_matches("ws://")
            .trim_start_matches("wss://")
    );
    let app_name = app.and_then(|app| app.name.clone());
    let normalized_app_id = page.app_id.to_ascii_lowercase();
    let normalized_app_name = app_name.as_deref().map(str::to_ascii_lowercase);
    let kind = if normalized_app_id.contains("mobilesafari")
        || normalized_app_name.as_deref() == Some("safari")
    {
        "safari-page"
    } else if app.is_some_and(|app| app.is_proxy) {
        "web-content-proxy"
    } else {
        "app-web-content"
    }
    .to_owned();

    WebKitTarget {
        id,
        app_id: page.app_id,
        app_name,
        page_id: page.page_id,
        title: page.title,
        url: page.url,
        kind,
        inspector_url,
        web_socket_url,
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

fn encode_target_id(app_id: &str, page_id: u64) -> String {
    format!("{}-{page_id}", hex::encode(app_id.as_bytes()))
}

fn decode_target_id(target_id: &str) -> Result<(String, u64), AppError> {
    let Some((encoded_app_id, page_id)) = target_id.rsplit_once('-') else {
        return Err(AppError::bad_request("Invalid WebKit target id."));
    };
    let app_id_bytes = hex::decode(encoded_app_id)
        .map_err(|_| AppError::bad_request("Invalid WebKit target app id."))?;
    let app_id = String::from_utf8(app_id_bytes)
        .map_err(|_| AppError::bad_request("Invalid WebKit target app id encoding."))?;
    let page_id = page_id
        .parse::<u64>()
        .map_err(|_| AppError::bad_request("Invalid WebKit target page id."))?;
    Ok((app_id, page_id))
}

fn new_remote_inspector_id() -> String {
    let counter = WEBKIT_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_nanos();
    let pid = std::process::id() as u128;
    let value = now ^ (pid << 48) ^ counter as u128;
    let hex = format!("{value:032X}");
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

fn string_value(args: &Dictionary, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_string)
        .map(str::to_owned)
        .filter(|value| !value.is_empty())
}

fn bool_value(args: &Dictionary, key: &str) -> Option<bool> {
    args.get(key).and_then(Value::as_boolean)
}

fn integer_value(args: &Dictionary, key: &str) -> Option<u64> {
    let value = args.get(key)?;
    value.as_unsigned_integer().or_else(|| {
        value
            .as_signed_integer()
            .and_then(|value| value.try_into().ok())
    })
}

fn data_value(args: &Dictionary, key: &str) -> Option<Vec<u8>> {
    args.get(key)
        .and_then(Value::as_data)
        .map(ToOwned::to_owned)
}

pub fn webkit_inspector_ui_root() -> Option<PathBuf> {
    [
        "/System/Cryptexes/OS/System/Library/PrivateFrameworks/WebInspectorUI.framework/Versions/A/Resources",
        "/System/Library/PrivateFrameworks/WebInspectorUI.framework/Resources",
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|path| path.join("Main.html").is_file())
}

pub fn inject_frontend_host(main_html: &str) -> String {
    let localized_strings = r#"<script src="en.lproj/localizedStrings.js"></script>"#;
    let compatibility_style = format!("<style>{}</style>", browser_frontend_compatibility_css());
    let shim = format!("<script>{}</script>", browser_frontend_host_script());
    main_html.replacen(
        "<script src=\"Main.js\"></script>",
        &format!(
            "{localized_strings}\n    {compatibility_style}\n    {shim}\n    <script src=\"Main.js\"></script>"
        ),
        1,
    )
}

fn browser_frontend_compatibility_css() -> &'static str {
    r#"
html.simdeck-browser-host,
html.simdeck-browser-host body {
    width: 100%;
    height: 100%;
    margin: 0;
    overflow: hidden;
}

html.simdeck-browser-host {
    color-scheme: light dark;
}

html.simdeck-browser-host #main,
html.simdeck-browser-host #content,
html.simdeck-browser-host #tab-browser {
    min-width: 0;
    min-height: 0;
}

html.simdeck-browser-host #docked-resizer {
    display: none !important;
}

html.simdeck-browser-host body.mac-platform {
    --selected-foreground-color: white;
    --selected-background-color: hsl(212, 92%, 54%);
    --selected-text-background-color: hsl(210, 98%, 93%);
    --breakpoint-color: hsl(211, 100%, 50%);
    --breakpoint-color-disabled: hsl(211, 82%, 82%);
    --glyph-color-active: hsl(212, 92%, 54%);
    --glyph-color-active-pressed: hsl(218, 85%, 52%);
}

@media (prefers-color-scheme: dark) {
    html.simdeck-browser-host body.mac-platform {
        --selected-foreground-color: hsl(0, 0%, 100%);
        --selected-background-color: hsl(219, 80%, 43%);
        --selected-text-background-color: hsl(230, 51%, 36%);
        --breakpoint-color: hsl(212, 100%, 71%);
        --breakpoint-color-disabled: hsl(212, 35%, 48%);
        --glyph-color-active: hsl(212, 100%, 71%);
        --glyph-color-active-pressed: hsl(212, 92%, 74%);
    }
}
"#
}

fn browser_frontend_host_script() -> &'static str {
    r#"
(function () {
    document.documentElement.classList.add("simdeck-browser-host");

    function queryValue(name) {
        return new URLSearchParams(window.location.search).get(name);
    }

    function websocketUrl() {
        const raw = queryValue("ws");
        if (!raw)
            return null;
        if (raw.startsWith("ws://") || raw.startsWith("wss://"))
            return raw;
        return (window.location.protocol === "https:" ? "wss://" : "ws://") + raw;
    }

    function dispatchBackendMessage(message) {
        if (window.InspectorBackend && typeof InspectorBackend.dispatch === "function") {
            InspectorBackend.dispatch(message);
            return;
        }
        (window.__simdeckInspectorBackendQueue ||= []).push(message);
    }

    function flushBackendQueue() {
        const queue = window.__simdeckInspectorBackendQueue || [];
        window.__simdeckInspectorBackendQueue = [];
        for (const message of queue)
            dispatchBackendMessage(message);
    }

    function copyText(text) {
        if (navigator.clipboard && navigator.clipboard.writeText) {
            navigator.clipboard.writeText(text).catch(() => {});
            return;
        }
        const textarea = document.createElement("textarea");
        textarea.value = text;
        textarea.style.position = "fixed";
        textarea.style.opacity = "0";
        document.body.appendChild(textarea);
        textarea.select();
        document.execCommand("copy");
        textarea.remove();
    }

    const pendingMessages = [];
    let reconnectDelay = 500;
    let reconnectTimer = 0;
    let socket = null;

    function clearReconnectTimer() {
        if (!reconnectTimer)
            return;
        clearTimeout(reconnectTimer);
        reconnectTimer = 0;
    }

    function scheduleReconnect() {
        if (reconnectTimer || !websocketUrl())
            return;
        const delay = reconnectDelay;
        reconnectDelay = Math.min(Math.ceil(reconnectDelay * 1.5), 3000);
        reconnectTimer = setTimeout(() => {
            reconnectTimer = 0;
            connectSocket();
        }, delay);
    }

    function enqueueBackendMessage(message) {
        if (pendingMessages.length > 500)
            pendingMessages.shift();
        pendingMessages.push(message);
    }

    function connectSocket() {
        const url = websocketUrl();
        if (!url)
            return;
        if (socket && (socket.readyState === WebSocket.OPEN || socket.readyState === WebSocket.CONNECTING))
            return;
        clearReconnectTimer();
        const nextSocket = new WebSocket(url);
        socket = nextSocket;
        nextSocket.addEventListener("message", (event) => dispatchBackendMessage(event.data));
        nextSocket.addEventListener("open", () => {
            if (socket !== nextSocket)
                return;
            reconnectDelay = 500;
            while (pendingMessages.length)
                nextSocket.send(pendingMessages.shift());
        });
        nextSocket.addEventListener("close", () => {
            if (socket !== nextSocket)
                return;
            socket = null;
            scheduleReconnect();
        });
        nextSocket.addEventListener("error", (event) => console.error("SimDeck WebKit Inspector socket error", event));
    }

    window.InspectorFrontendHost = {
        supportsShowCertificate: false,
        isRemote: true,
        inspectionLevel: 1,
        debuggableInfo: {
            debuggableType: "web-page",
            targetPlatformName: "iOS",
            targetBuildVersion: undefined,
            targetProductVersion: undefined,
            targetIsSimulator: true,
        },
        get platform() {
            const match = navigator.platform.match(/mac|win|linux/i);
            if (!match)
                return "unknown";
            return match[0].toLowerCase() === "win" ? "windows" : match[0].toLowerCase();
        },
        platformVersionName: "",
        supportsDiagnosticLogging: false,
        supportsWebExtensions: false,
        localizedStringsURL: "en.lproj/localizedStrings.js",
        connect() {
            connectSocket();
        },
        loaded() {
            if (window.WI && typeof WI.updateVisibilityState === "function")
                WI.updateVisibilityState(true);
            flushBackendQueue();
        },
        closeWindow() {},
        reopen() { window.location.reload(); },
        reset() { window.location.reload(); },
        bringToFront() {},
        inspectedURLChanged(title) {
            if (title)
                document.title = title;
        },
        showCertificate() {},
        setZoomFactor() {},
        zoomFactor() { return 1; },
        setForcedAppearance() {},
        userInterfaceLayoutDirection() { return "ltr"; },
        supportsDockSide() { return false; },
        requestDockSide() {},
        requestSetDockSide() {},
        setAttachedWindowHeight() {},
        setAttachedWindowWidth() {},
        setSheetRect() {},
        startWindowDrag() {},
        moveWindowBy() {},
        copyText,
        killText: copyText,
        openURLExternally(url) { window.open(url, "_blank", "noopener"); },
        canSave() { return false; },
        save() {},
        canLoad() { return false; },
        load() { throw new Error("Loading local files is not supported in SimDeck WebKit Inspector."); },
        getPath() { return null; },
        canPickColorFromScreen() { return false; },
        pickColorFromScreen() { throw new Error("Picking colors from screen is not supported in SimDeck WebKit Inspector."); },
        revealFileExternally() {},
        getCurrentX() { return 0; },
        getCurrentY() { return 0; },
        setPath() {},
        showContextMenu(event, items) {
            if (window.WI && WI.SoftContextMenu) {
                new WI.SoftContextMenu(items).show(event);
            }
        },
        dispatchEventAsContextMenuEvent(event) {
            event.target?.dispatchEvent(new MouseEvent("contextmenu", event));
        },
        sendMessageToBackend(message) {
            if (socket && socket.readyState === WebSocket.OPEN) {
                socket.send(message);
            } else {
                enqueueBackendMessage(message);
                connectSocket();
            }
        },
        unbufferedLog(message) { console.log(message); },
        isUnderTest() { return false; },
        beep() {},
        inspectInspector() {},
        isBeingInspected() { return false; },
        setAllowsInspectingInspector() {},
        logDiagnosticEvent() {},
        didShowExtensionTab() {},
        didHideExtensionTab() {},
        didNavigateExtensionTab() {},
        inspectedPageDidNavigate() {},
        evaluateScriptInExtensionTab() {},
        engineeringSettingsAllowed() { return false; },
    };

    function getOrInsert(key, value) {
        const existing = this.get(key);
        if (existing !== undefined)
            return existing;
        this.set(key, value);
        return value;
    }

    function getOrInsertComputed(key, callback) {
        const existing = this.get(key);
        if (existing !== undefined)
            return existing;
        const value = callback();
        this.set(key, value);
        return value;
    }

    Map.prototype.getOrInsert ||= getOrInsert;
    WeakMap.prototype.getOrInsert ||= getOrInsert;
    Map.prototype.getOrInsertComputed ||= getOrInsertComputed;
    WeakMap.prototype.getOrInsertComputed ||= getOrInsertComputed;
})();
"#
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_id_round_trips_app_and_page() {
        let id = encode_target_id("PID:71571", 42);
        assert_eq!(decode_target_id(&id).unwrap(), ("PID:71571".to_owned(), 42));
    }

    #[test]
    fn websocket_url_uses_request_scheme() {
        assert_eq!(
            websocket_url("http://127.0.0.1:4310", "/api/socket"),
            "ws://127.0.0.1:4310/api/socket"
        );
        assert_eq!(
            websocket_url("https://example.test", "/api/socket"),
            "wss://example.test/api/socket"
        );
    }

    #[test]
    fn inject_frontend_host_precedes_main_script() {
        let html = r#"<script src="CodeMirror.js"></script>
    <script src="Main.js"></script>"#;
        let injected = inject_frontend_host(html);
        let strings_index = injected.find("en.lproj/localizedStrings.js").unwrap();
        let shim_index = injected.find("window.InspectorFrontendHost").unwrap();
        let main_index = injected.find(r#"<script src="Main.js"></script>"#).unwrap();
        assert!(strings_index < shim_index);
        assert!(shim_index < main_index);
    }

    #[test]
    fn sanitize_webinspectord_socket_path_accepts_launchd_env_output() {
        assert_eq!(
            sanitize_webinspectord_socket_path(
                "/private/var/tmp/com.apple.launchd.test/com.apple.webinspectord_sim.socket\n"
            ),
            Some("/private/var/tmp/com.apple.launchd.test/com.apple.webinspectord_sim.socket")
        );
        assert_eq!(sanitize_webinspectord_socket_path("\n"), None);
        assert_eq!(
            sanitize_webinspectord_socket_path(
                "/private/var/tmp/com.apple.launchd.test/other.socket"
            ),
            None
        );
    }
}
