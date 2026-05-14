use crate::error::AppError;
use axum::extract::ws::{Message, WebSocket};
use futures::{SinkExt, StreamExt};
use plist::{Dictionary, Value};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::process::Command;
use tokio::sync::{Notify, RwLock};
use tokio::time::{sleep, timeout, Instant};
use tracing::{debug, warn};

const WEBINSPECTORD_SOCKET_NAME: &str = "com.apple.webinspectord_sim.socket";
const WEBKIT_PACKET_MAX_LEN: usize = 64 * 1024 * 1024;
const WEBKIT_DISCOVERY_TIMEOUT: Duration = Duration::from_millis(2200);
const WEBKIT_DISCOVERY_REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const WEBKIT_SOCKET_ACTIVATION_DELAY: Duration = Duration::from_millis(200);
const WEBKIT_TARGET_ATTACH_TIMEOUT: Duration = Duration::from_millis(2200);
const WEBKIT_IO_TIMEOUT: Duration = Duration::from_secs(4);
const WEBKIT_SOCKET_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(5);
const WEBKIT_DISCOVERY_RECONNECT_DELAY: Duration = Duration::from_millis(500);

static WEBKIT_ID_COUNTER: AtomicU64 = AtomicU64::new(1);
static WEBKIT_DISCOVERY_MONITORS: OnceLock<Mutex<HashMap<String, Arc<WebKitDiscoveryMonitor>>>> =
    OnceLock::new();

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebKitTarget {
    pub id: String,
    pub app_id: String,
    pub app_name: Option<String>,
    pub app_active: bool,
    pub page_active: bool,
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
    bundle_identifier: Option<String>,
    active: bool,
    is_proxy: bool,
}

#[derive(Clone, Debug)]
struct WebKitPage {
    app_id: String,
    page_id: u64,
    title: Option<String>,
    url: Option<String>,
    connection_id: Option<String>,
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

#[derive(Clone, Default)]
struct WebKitDiscoveryState {
    socket_path: Option<String>,
    applications: BTreeMap<String, WebKitApplication>,
    pages: BTreeMap<(String, u64), WebKitPage>,
    warnings: Vec<String>,
}

struct WebKitDiscoveryMonitor {
    udid: String,
    running: AtomicBool,
    state: RwLock<WebKitDiscoveryState>,
    notify: Notify,
}

pub async fn discover_targets(
    udid: &str,
    http_origin: Option<&str>,
) -> Result<WebKitTargetDiscovery, AppError> {
    let monitor = webkit_discovery_monitor(udid);
    monitor.clone().ensure_started();

    let deadline = Instant::now() + WEBKIT_DISCOVERY_TIMEOUT;
    loop {
        let discovery = monitor.discovery(http_origin).await;
        if !discovery.targets.is_empty() || Instant::now() >= deadline {
            return Ok(discovery);
        }

        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return Ok(discovery);
        };
        if timeout(
            remaining.min(Duration::from_millis(250)),
            monitor.notify.notified(),
        )
        .await
        .is_err()
        {
            continue;
        }
    }
}

fn webkit_discovery_monitor(udid: &str) -> Arc<WebKitDiscoveryMonitor> {
    let monitors = WEBKIT_DISCOVERY_MONITORS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut monitors = monitors
        .lock()
        .expect("WebKit discovery monitor lock poisoned");
    monitors
        .entry(udid.to_owned())
        .or_insert_with(|| Arc::new(WebKitDiscoveryMonitor::new(udid.to_owned())))
        .clone()
}

impl WebKitDiscoveryMonitor {
    fn new(udid: String) -> Self {
        Self {
            udid,
            running: AtomicBool::new(false),
            state: RwLock::new(WebKitDiscoveryState::default()),
            notify: Notify::new(),
        }
    }

    fn ensure_started(self: Arc<Self>) {
        if self.running.swap(true, Ordering::AcqRel) {
            return;
        }
        tokio::spawn(async move {
            self.run().await;
        });
    }

    async fn discovery(&self, http_origin: Option<&str>) -> WebKitTargetDiscovery {
        let state = self.state.read().await.clone();
        self.discovery_from_state(state, http_origin)
    }

    fn discovery_from_state(
        &self,
        state: WebKitDiscoveryState,
        http_origin: Option<&str>,
    ) -> WebKitTargetDiscovery {
        let origin = http_origin.unwrap_or("");
        let mut targets = state
            .pages
            .into_values()
            .filter(|page| !is_incomplete_or_transient_page(page))
            .map(|page| {
                let app = state.applications.get(&page.app_id);
                webkit_target(&self.udid, origin, page, app)
            })
            .collect::<Vec<_>>();
        targets.retain(is_inspectable_webkit_target);
        targets.sort_by(|lhs, rhs| {
            lhs.app_name
                .cmp(&rhs.app_name)
                .then(lhs.title.cmp(&rhs.title))
                .then(lhs.url.cmp(&rhs.url))
                .then(lhs.page_id.cmp(&rhs.page_id))
        });

        WebKitTargetDiscovery {
            udid: self.udid.clone(),
            socket_path: state.socket_path,
            targets,
            warnings: state.warnings,
        }
    }

    async fn run(self: Arc<Self>) {
        loop {
            match discover_webinspector_socket(&self.udid).await {
                Ok(Some(socket)) => {
                    self.set_socket_state(Some(socket.path.clone()), Vec::new())
                        .await;
                    if let Err(error) = self.run_connection(socket).await {
                        debug!(
                            "WebKit discovery connection ended for {}: {error}",
                            self.udid
                        );
                    }
                }
                Ok(None) => {
                    self.set_socket_state(
                        None,
                        vec![format!(
                            "No WebKit Remote Inspector socket was found for simulator {}. Boot the simulator and open an inspectable Safari page or WKWebView.",
                            self.udid
                        )],
                    )
                    .await;
                }
                Err(error) => {
                    self.set_socket_state(None, vec![error.to_string()]).await;
                }
            }
            sleep(WEBKIT_DISCOVERY_RECONNECT_DELAY).await;
        }
    }

    async fn set_socket_state(&self, socket_path: Option<String>, warnings: Vec<String>) {
        let mut state = self.state.write().await;
        state.socket_path = socket_path;
        state.warnings = warnings;
        if state.socket_path.is_none() {
            state.applications.clear();
            state.pages.clear();
        }
        drop(state);
        self.notify.notify_waiters();
    }

    async fn publish_listing(
        &self,
        socket_path: &str,
        applications: &BTreeMap<String, WebKitApplication>,
        pages: &BTreeMap<(String, u64), WebKitPage>,
    ) {
        let mut state = self.state.write().await;
        state.socket_path = Some(socket_path.to_owned());
        state.applications = applications.clone();
        state.pages = pages.clone();
        state.warnings.clear();
        drop(state);
        self.notify.notify_waiters();
    }

    async fn run_connection(&self, socket: WebKitSocket) -> Result<(), AppError> {
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
        send_get_connected_applications(&mut stream, &connection_id).await?;

        let mut next_listing_refresh = Instant::now() + WEBKIT_DISCOVERY_REFRESH_INTERVAL;
        let mut applications: BTreeMap<String, WebKitApplication> = BTreeMap::new();
        let mut requested_listings: HashSet<String> = HashSet::new();
        let mut pages: BTreeMap<(String, u64), WebKitPage> = BTreeMap::new();

        loop {
            let read_timeout = Duration::from_millis(500);
            let packet = match timeout(read_timeout, read_packet(&mut stream)).await {
                Ok(Ok(packet)) => packet,
                Ok(Err(error)) => return Err(error),
                Err(_) => {
                    if Instant::now() >= next_listing_refresh {
                        send_get_connected_applications(&mut stream, &connection_id).await?;
                        for app_id in applications.keys() {
                            send_forward_get_listing(&mut stream, &connection_id, app_id).await?;
                        }
                        next_listing_refresh = Instant::now() + WEBKIT_DISCOVERY_REFRESH_INTERVAL;
                    }
                    continue;
                }
            };

            let message = match parse_rpc_message(&packet) {
                Ok(message) => message,
                Err(error) => {
                    debug!("Ignoring malformed WebKit discovery packet: {error}");
                    continue;
                }
            };
            debug!(
                selector = %message.selector,
                "Received WebKit discovery selector"
            );

            match message.selector.as_str() {
                "_rpc_reportConnectedApplicationList:" => {
                    let live_apps = parse_application_list(&message.args);
                    let live_ids = live_apps
                        .iter()
                        .map(|app| app.id.clone())
                        .collect::<HashSet<_>>();
                    applications.retain(|app_id, _| live_ids.contains(app_id));
                    pages.retain(|(app_id, _), _| live_ids.contains(app_id));
                    requested_listings.retain(|app_id| live_ids.contains(app_id));
                    for app in live_apps {
                        if requested_listings.insert(app.id.clone()) {
                            send_forward_get_listing(&mut stream, &connection_id, &app.id).await?;
                        }
                        applications.insert(app.id.clone(), app);
                    }
                    self.publish_listing(&socket.path, &applications, &pages)
                        .await;
                }
                "_rpc_applicationConnected:" => {
                    if let Some(app) = parse_application(&message.args) {
                        if requested_listings.insert(app.id.clone()) {
                            send_forward_get_listing(&mut stream, &connection_id, &app.id).await?;
                        }
                        applications.insert(app.id.clone(), app);
                        self.publish_listing(&socket.path, &applications, &pages)
                            .await;
                    }
                }
                "_rpc_applicationUpdated:" => {
                    if let Some(app_id) = string_value(&message.args, "WIRApplicationIdentifierKey")
                    {
                        if requested_listings.insert(app_id.clone()) {
                            send_forward_get_listing(&mut stream, &connection_id, &app_id).await?;
                        }
                        let app = applications
                            .entry(app_id.clone())
                            .or_insert(WebKitApplication {
                                id: app_id,
                                name: None,
                                bundle_identifier: None,
                                active: false,
                                is_proxy: false,
                            });
                        if let Some(name) = string_value(&message.args, "WIRApplicationNameKey") {
                            app.name = Some(name);
                        }
                        if let Some(bundle_identifier) =
                            string_value(&message.args, "WIRApplicationBundleIdentifierKey")
                        {
                            app.bundle_identifier = Some(bundle_identifier);
                        }
                        if let Some(active) =
                            integer_value(&message.args, "WIRIsApplicationActiveKey")
                        {
                            app.active = active > 0;
                        }
                        if let Some(is_proxy) =
                            bool_value(&message.args, "WIRIsApplicationProxyKey")
                        {
                            app.is_proxy = is_proxy;
                        }
                        self.publish_listing(&socket.path, &applications, &pages)
                            .await;
                    }
                }
                "_rpc_applicationSentListing:" => {
                    apply_page_listing(&mut pages, &message.args);
                    self.publish_listing(&socket.path, &applications, &pages)
                        .await;
                }
                "_rpc_applicationDisconnected:" => {
                    if let Some(app) = parse_application(&message.args) {
                        applications.remove(&app.id);
                        requested_listings.remove(&app.id);
                        pages.retain(|(app_id, _), _| app_id != &app.id);
                        self.publish_listing(&socket.path, &applications, &pages)
                            .await;
                    }
                }
                "_rpc_reportSetup:" => {
                    send_get_connected_applications(&mut stream, &connection_id).await?;
                }
                "_rpc_reportConnectedDriverList:" | "_rpc_reportCurrentState:" => {}
                selector => debug!("Ignoring WebKit inspector discovery selector {selector}."),
            }
        }
    }
}

fn is_incomplete_or_transient_page(page: &WebKitPage) -> bool {
    let url = page.url.as_deref().map(str::trim).unwrap_or_default();
    if url.is_empty() {
        return true;
    }
    url == "about:blank"
        && page
            .title
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .is_empty()
}

fn is_inspectable_webkit_target(target: &WebKitTarget) -> bool {
    let url = target.url.as_deref().map(str::trim).unwrap_or_default();
    if url.is_empty() {
        return false;
    }
    !(url == "about:blank"
        && target
            .title
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .is_empty())
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
    let sender_id = connection_id.clone();
    let (mut inspector_reader, mut inspector_writer) = stream.into_split();
    send_rpc(
        &mut inspector_writer,
        "_rpc_reportIdentifier:",
        rpc_args(&connection_id),
    )
    .await?;
    sleep(WEBKIT_SOCKET_ACTIVATION_DELAY).await;
    prepare_webkit_target_for_attach(
        &mut inspector_reader,
        &mut inspector_writer,
        &connection_id,
        &app_id,
        page_id,
    )
    .await?;
    send_forward_indicate_webview(
        &mut inspector_writer,
        &connection_id,
        &app_id,
        page_id,
        true,
    )
    .await?;
    send_forward_indicate_webview(
        &mut inspector_writer,
        &connection_id,
        &app_id,
        page_id,
        false,
    )
    .await?;
    send_forward_socket_setup(
        &mut inspector_writer,
        &connection_id,
        &app_id,
        page_id,
        &sender_id,
    )
    .await?;
    let initial_messages =
        wait_for_webkit_socket_setup(&mut inspector_reader, &app_id, page_id, &sender_id).await?;

    let (mut client_writer, mut client_reader) = socket.split();
    for message in initial_messages {
        client_writer
            .send(Message::Text(message.into()))
            .await
            .map_err(|error| AppError::internal(format!("WebSocket send failed: {error}")))?;
    }
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

async fn prepare_webkit_target_for_attach<R, W>(
    inspector_reader: &mut R,
    inspector_writer: &mut W,
    connection_id: &str,
    app_id: &str,
    page_id: u64,
) -> Result<(), AppError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    send_forward_get_listing(inspector_writer, connection_id, app_id).await?;
    let deadline = Instant::now() + WEBKIT_TARGET_ATTACH_TIMEOUT;
    let mut released_connections = HashSet::new();

    loop {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return Err(AppError::native(format!(
                "Timed out preparing WebKit target {app_id}/{page_id} for inspection."
            )));
        };
        let packet = timeout(
            remaining.min(Duration::from_millis(500)),
            read_packet(inspector_reader),
        )
        .await;
        let packet = match packet {
            Ok(Ok(packet)) => packet,
            Ok(Err(error)) => return Err(error),
            Err(_) => continue,
        };

        let message = parse_rpc_message(&packet)?;
        match message.selector.as_str() {
            "_rpc_applicationSentListing:" => {
                if string_value(&message.args, "WIRApplicationIdentifierKey").as_deref()
                    != Some(app_id)
                {
                    continue;
                }

                let Some(page) = parse_page_listing(&message.args)
                    .into_iter()
                    .find(|page| page.page_id == page_id)
                else {
                    return Err(AppError::not_found(format!(
                        "WebKit target {app_id}/{page_id} is no longer available."
                    )));
                };

                let Some(existing_connection_id) = page.connection_id else {
                    return Ok(());
                };
                if existing_connection_id == connection_id {
                    return Ok(());
                }
                if !released_connections.insert(existing_connection_id.clone()) {
                    send_forward_get_listing(inspector_writer, connection_id, app_id).await?;
                    continue;
                }

                debug!(
                    app_id,
                    page_id,
                    existing_connection_id,
                    "Releasing stale WebKit inspector target owner"
                );
                send_forward_did_close(
                    inspector_writer,
                    &existing_connection_id,
                    app_id,
                    page_id,
                    &existing_connection_id,
                )
                .await?;
                send_forward_get_listing(inspector_writer, connection_id, app_id).await?;
            }
            "_rpc_applicationDisconnected:" => {
                if string_value(&message.args, "WIRApplicationIdentifierKey").as_deref()
                    == Some(app_id)
                {
                    return Err(AppError::not_found(format!(
                        "WebKit application {app_id} disconnected before inspection could start."
                    )));
                }
            }
            "_rpc_reportSetup:" => {
                send_get_connected_applications(inspector_writer, connection_id).await?;
                send_forward_get_listing(inspector_writer, connection_id, app_id).await?;
            }
            "_rpc_reportCurrentState:"
            | "_rpc_reportConnectedApplicationList:"
            | "_rpc_reportConnectedDriverList:"
            | "_rpc_applicationUpdated:" => {}
            selector => debug!("Ignoring WebKit inspector attach preflight selector {selector}."),
        }
    }
}

async fn wait_for_webkit_socket_setup<R>(
    inspector_reader: &mut R,
    app_id: &str,
    page_id: u64,
    sender_id: &str,
) -> Result<Vec<String>, AppError>
where
    R: AsyncRead + Unpin,
{
    let deadline = Instant::now() + WEBKIT_TARGET_ATTACH_TIMEOUT;
    loop {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return Err(AppError::native(format!(
                "WebKit target {app_id}/{page_id} did not acknowledge inspector socket setup."
            )));
        };
        let packet = timeout(
            remaining.min(Duration::from_millis(500)),
            read_packet(inspector_reader),
        )
        .await;
        let packet = match packet {
            Ok(Ok(packet)) => packet,
            Ok(Err(error)) => return Err(error),
            Err(_) => continue,
        };

        let message = parse_rpc_message(&packet)?;
        match message.selector.as_str() {
            "_rpc_applicationSentData:" => {
                if string_value(&message.args, "WIRDestinationKey").as_deref() == Some(sender_id) {
                    if let Some(data) = data_value(&message.args, "WIRMessageDataKey") {
                        return Ok(vec![String::from_utf8(data).unwrap_or_default()]);
                    }
                }
            }
            "_rpc_applicationDisconnected:" => {
                if string_value(&message.args, "WIRApplicationIdentifierKey").as_deref()
                    == Some(app_id)
                {
                    return Err(AppError::not_found(format!(
                        "WebKit application {app_id} disconnected before inspection could start."
                    )));
                }
            }
            "_rpc_applicationSentListing:"
            | "_rpc_applicationUpdated:"
            | "_rpc_reportSetup:"
            | "_rpc_reportCurrentState:"
            | "_rpc_reportConnectedApplicationList:"
            | "_rpc_reportConnectedDriverList:" => {}
            selector => debug!("Ignoring WebKit inspector setup selector {selector}."),
        }
    }
}

async fn discover_webinspector_socket(udid: &str) -> Result<Option<WebKitSocket>, AppError> {
    let output = timeout(
        WEBKIT_SOCKET_DISCOVERY_TIMEOUT,
        Command::new("/usr/sbin/lsof")
            .args(["-nP", "-U", "-F", "pn"])
            .output(),
    )
    .await
    .map_err(|_| AppError::native("Timed out listing webinspectord sockets."))?
    .map_err(|error| AppError::native(format!("Unable to run lsof: {error}")))?;

    if !output.status.success() {
        debug!(
            "Unable to list webinspectord sockets for {udid}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        return discover_launchd_webinspector_socket(udid).await;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut processes: HashMap<String, LsofProcess> = HashMap::new();
    let device_marker = format!("/Devices/{udid}/");
    let launchd_marker = format!("CoreSimulator.SimDevice.{udid}/");
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
        if name.contains(&device_marker) || name.contains(&launchd_marker) {
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
        WEBKIT_SOCKET_DISCOVERY_TIMEOUT,
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

async fn send_get_connected_applications<W: AsyncWrite + Unpin>(
    writer: &mut W,
    connection_id: &str,
) -> Result<(), AppError> {
    send_rpc(
        writer,
        "_rpc_getConnectedApplications:",
        rpc_args(connection_id),
    )
    .await
}

async fn send_forward_indicate_webview<W: AsyncWrite + Unpin>(
    writer: &mut W,
    connection_id: &str,
    app_id: &str,
    page_id: u64,
    enabled: bool,
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
    args.insert("WIRIndicateEnabledKey".to_owned(), Value::Boolean(enabled));
    send_rpc(writer, "_rpc_forwardIndicateWebView:", args).await
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

    apps.iter()
        .filter_map(|(app_id, value)| {
            let app = value.as_dictionary()?;
            Some(WebKitApplication {
                id: string_value(app, "WIRApplicationIdentifierKey")
                    .unwrap_or_else(|| app_id.clone()),
                name: string_value(app, "WIRApplicationNameKey"),
                bundle_identifier: string_value(app, "WIRApplicationBundleIdentifierKey"),
                active: integer_value(app, "WIRIsApplicationActiveKey").unwrap_or(0) > 0,
                is_proxy: bool_value(app, "WIRIsApplicationProxyKey").unwrap_or(false),
            })
        })
        .collect()
}

fn parse_application(args: &Dictionary) -> Option<WebKitApplication> {
    let id = string_value(args, "WIRApplicationIdentifierKey")?;
    Some(WebKitApplication {
        id,
        name: string_value(args, "WIRApplicationNameKey"),
        bundle_identifier: string_value(args, "WIRApplicationBundleIdentifierKey"),
        active: integer_value(args, "WIRIsApplicationActiveKey").unwrap_or(0) > 0,
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
        .iter()
        .filter_map(|(page_key, value)| {
            let page = value.as_dictionary()?;
            let page_id = integer_value(page, "WIRPageIdentifierKey")
                .or_else(|| page_key.parse::<u64>().ok())?;
            Some(WebKitPage {
                app_id: app_id.clone(),
                page_id,
                title: string_value(page, "WIRTitleKey"),
                url: string_value(page, "WIRURLKey"),
                connection_id: string_value(page, "WIRConnectionIdentifierKey"),
            })
        })
        .collect()
}

fn apply_page_listing(pages: &mut BTreeMap<(String, u64), WebKitPage>, listing_args: &Dictionary) {
    if let Some(app_id) = string_value(listing_args, "WIRApplicationIdentifierKey") {
        pages.retain(|(page_app_id, _), _| page_app_id != &app_id);
    }
    for page in parse_page_listing(listing_args) {
        pages.insert((page.app_id.clone(), page.page_id), page);
    }
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
    let app_active = app.map(|app| app.active).unwrap_or(false);
    let normalized_app_id = page.app_id.to_ascii_lowercase();
    let normalized_app_name = app_name.as_deref().map(str::to_ascii_lowercase);
    let normalized_bundle_id = app
        .and_then(|app| app.bundle_identifier.as_deref())
        .map(str::to_ascii_lowercase);
    let kind = if normalized_app_id.contains("mobilesafari")
        || normalized_app_name.as_deref() == Some("safari")
        || normalized_bundle_id.as_deref() == Some("com.apple.mobilesafari")
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
        app_active,
        page_active: false,
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
    let mut bytes = value.to_be_bytes();
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex = hex::encode(bytes);
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
    let localized_string_fixups =
        r#"<script>localizedStrings["Refresh layers"] ||= "Refresh layers";</script>"#;
    let compatibility_style = format!("<style>{}</style>", browser_frontend_compatibility_css());
    let shim = format!("<script>{}</script>", browser_frontend_host_script());
    main_html.replacen(
        "<script src=\"Main.js\"></script>",
        &format!(
            "{localized_strings}\n    {localized_string_fixups}\n    {compatibility_style}\n    {shim}\n    <script src=\"Main.js\"></script>"
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

    function installCSSCompatibilityFallbacks() {
        const cssManager = window.WI?.cssManager;
        if (!cssManager || cssManager.propertyNameCompletions || !window.WI?.CSSPropertyNameCompletions)
            return;

        const propertyMap = window.WI.CSSKeywordCompletions?._propertyKeywordMap || {};
        const propertyNames = new Set(Object.keys(propertyMap));
        for (const name of [
            "background",
            "color",
            "display",
            "font-family",
            "height",
            "margin",
            "width",
        ])
            propertyNames.add(name);

        cssManager._propertyNameCompletions = new WI.CSSPropertyNameCompletions(
            Array.from(propertyNames, (name) => ({ name }))
        );
    }

    function installNetworkManagerCompatibilityFallbacks() {
        const prototype = window.WI?.NetworkManager?.prototype;
        if (!prototype || prototype.__simdeckMainFrameResourceTreePatch)
            return;
        const original = prototype._processMainFrameResourceTreePayload;
        if (typeof original !== "function")
            return;

        prototype.__simdeckMainFrameResourceTreePatch = true;
        prototype._processMainFrameResourceTreePayload = function (error, mainFramePayload) {
            if (this._transitioningPageTarget && !this._mainFrame)
                this._transitioningPageTarget = false;
            return original.call(this, error, mainFramePayload);
        };
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

    function notifySocketState(state) {
        if (window.parent === window)
            return;
        window.parent.postMessage({
            type: "simdeck:webkit-inspector:socket",
            state,
        }, "*");
    }

    function clearReconnectTimer() {
        if (!reconnectTimer)
            return;
        clearTimeout(reconnectTimer);
        reconnectTimer = 0;
    }

    function scheduleReconnect() {
        if (reconnectTimer)
            return;
        if (!websocketUrl()) {
            notifySocketState("disconnected");
            return;
        }
        notifySocketState("reconnecting");
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
        notifySocketState("connecting");
        const nextSocket = new WebSocket(url);
        socket = nextSocket;
        nextSocket.addEventListener("message", (event) => dispatchBackendMessage(event.data));
        nextSocket.addEventListener("open", () => {
            if (socket !== nextSocket)
                return;
            reconnectDelay = 500;
            notifySocketState("connected");
            while (pendingMessages.length)
                nextSocket.send(pendingMessages.shift());
        });
        nextSocket.addEventListener("close", () => {
            if (socket !== nextSocket)
                return;
            socket = null;
            scheduleReconnect();
        });
        nextSocket.addEventListener("error", (event) => {
            if (socket === nextSocket)
                notifySocketState("failed");
            console.error("SimDeck WebKit Inspector socket error", event);
        });
    }

    const unsupportedOptionalTargetCommands = new Set([
        "Page.setShowRulers",
    ]);

    function maybeHandleUnsupportedFrontendCommand(message) {
        let payload;
        try {
            payload = JSON.parse(message);
        } catch {
            return false;
        }

        if (unsupportedOptionalTargetCommands.has(payload.method)) {
            dispatchBackendMessage(JSON.stringify({ id: payload.id, result: {} }));
            return true;
        }

        if (payload.method !== "Target.sendMessageToTarget" || !payload.params)
            return false;
        const targetId = payload.params.targetId;
        if (!targetId || typeof payload.params.message !== "string")
            return false;

        let targetPayload;
        try {
            targetPayload = JSON.parse(payload.params.message);
        } catch {
            return false;
        }
        if (!unsupportedOptionalTargetCommands.has(targetPayload.method))
            return false;

        dispatchBackendMessage(JSON.stringify({
            method: "Target.dispatchMessageFromTarget",
            params: {
                targetId,
                message: JSON.stringify({ id: targetPayload.id, result: {} }),
            },
        }));
        return true;
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
        localizedStringsURL: null,
        connect() {
            connectSocket();
        },
        loaded() {
            if (window.WI && typeof WI.updateVisibilityState === "function")
                WI.updateVisibilityState(true);
            installCSSCompatibilityFallbacks();
            installNetworkManagerCompatibilityFallbacks();
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
            if (maybeHandleUnsupportedFrontendCommand(message))
                return;
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
    fn parses_webkit_listing_ids_from_dictionary_keys() {
        let mut app = Dictionary::new();
        app.insert(
            "WIRApplicationNameKey".to_owned(),
            Value::String("Safari".to_owned()),
        );
        app.insert(
            "WIRApplicationBundleIdentifierKey".to_owned(),
            Value::String("com.apple.mobilesafari".to_owned()),
        );
        app.insert(
            "WIRIsApplicationActiveKey".to_owned(),
            Value::Integer(2.into()),
        );
        let mut app_dictionary = Dictionary::new();
        app_dictionary.insert("PID:42".to_owned(), Value::Dictionary(app));
        let mut app_args = Dictionary::new();
        app_args.insert(
            "WIRApplicationDictionaryKey".to_owned(),
            Value::Dictionary(app_dictionary),
        );

        let applications = parse_application_list(&app_args);
        assert_eq!(applications.len(), 1);
        assert_eq!(applications[0].id, "PID:42");
        assert_eq!(applications[0].name.as_deref(), Some("Safari"));
        assert_eq!(
            applications[0].bundle_identifier.as_deref(),
            Some("com.apple.mobilesafari")
        );
        assert!(applications[0].active);

        let mut page = Dictionary::new();
        page.insert(
            "WIRTitleKey".to_owned(),
            Value::String("Example Domain".to_owned()),
        );
        page.insert(
            "WIRURLKey".to_owned(),
            Value::String("https://example.com/".to_owned()),
        );
        page.insert(
            "WIRConnectionIdentifierKey".to_owned(),
            Value::String("existing-connection".to_owned()),
        );
        let mut listing = Dictionary::new();
        listing.insert("7".to_owned(), Value::Dictionary(page));
        let mut page_args = Dictionary::new();
        page_args.insert(
            "WIRApplicationIdentifierKey".to_owned(),
            Value::String("PID:42".to_owned()),
        );
        page_args.insert("WIRListingKey".to_owned(), Value::Dictionary(listing));

        let pages = parse_page_listing(&page_args);
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].app_id, "PID:42");
        assert_eq!(pages[0].page_id, 7);
        assert_eq!(pages[0].url.as_deref(), Some("https://example.com/"));
        assert_eq!(
            pages[0].connection_id.as_deref(),
            Some("existing-connection")
        );
    }

    #[test]
    fn page_listing_replaces_previous_pages_for_application() {
        let mut pages = BTreeMap::new();
        pages.insert(
            ("PID:42".to_owned(), 1),
            WebKitPage {
                app_id: "PID:42".to_owned(),
                page_id: 1,
                title: Some("Old Example".to_owned()),
                url: Some("https://example.com/old".to_owned()),
                connection_id: None,
            },
        );
        pages.insert(
            ("PID:99".to_owned(), 1),
            WebKitPage {
                app_id: "PID:99".to_owned(),
                page_id: 1,
                title: Some("Other App".to_owned()),
                url: Some("https://other.example/".to_owned()),
                connection_id: None,
            },
        );

        let mut page = Dictionary::new();
        page.insert(
            "WIRTitleKey".to_owned(),
            Value::String("Example Domain".to_owned()),
        );
        page.insert(
            "WIRURLKey".to_owned(),
            Value::String("https://example.com/".to_owned()),
        );
        let mut listing = Dictionary::new();
        listing.insert("2".to_owned(), Value::Dictionary(page));
        let mut listing_args = Dictionary::new();
        listing_args.insert(
            "WIRApplicationIdentifierKey".to_owned(),
            Value::String("PID:42".to_owned()),
        );
        listing_args.insert("WIRListingKey".to_owned(), Value::Dictionary(listing));

        apply_page_listing(&mut pages, &listing_args);

        assert!(!pages.contains_key(&("PID:42".to_owned(), 1)));
        assert!(pages.contains_key(&("PID:42".to_owned(), 2)));
        assert!(pages.contains_key(&("PID:99".to_owned(), 1)));
    }

    #[test]
    fn webkit_target_uses_application_activity_and_bundle_kind() {
        let app = WebKitApplication {
            id: "PID:42".to_owned(),
            name: None,
            bundle_identifier: Some("com.apple.mobilesafari".to_owned()),
            active: true,
            is_proxy: false,
        };
        let page = WebKitPage {
            app_id: app.id.clone(),
            page_id: 7,
            title: Some("Example Domain".to_owned()),
            url: Some("https://example.com/".to_owned()),
            connection_id: None,
        };

        let target = webkit_target("UDID", "http://127.0.0.1:4311", page, Some(&app));
        assert_eq!(target.kind, "safari-page");
        assert!(target.app_active);
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
        assert!(injected.contains("simdeck:webkit-inspector:socket"));
        assert!(injected.contains("notifySocketState(\"reconnecting\")"));
        assert!(injected.contains("Page.setShowRulers"));
        assert!(injected.contains("maybeHandleUnsupportedFrontendCommand(message)"));
        assert!(injected.contains("installNetworkManagerCompatibilityFallbacks()"));
    }

    #[test]
    fn webkit_attach_uses_page_activation_before_socket_setup() {
        let source = include_str!("webkit.rs");
        let preflight_index = source
            .find("prepare_webkit_target_for_attach(")
            .expect("attach should preflight target ownership before socket setup");
        let indicate_index = source
            .find("send_forward_indicate_webview(")
            .expect("attach should indicate the WebView before socket setup");
        let setup_index = source
            .find("send_forward_socket_setup(")
            .expect("attach should still set up the forwarding socket");
        assert!(preflight_index < setup_index);
        assert!(indicate_index < setup_index);
        assert!(source.contains("_rpc_forwardIndicateWebView:"));
        assert!(source.contains("Releasing stale WebKit inspector target owner"));
    }

    #[test]
    fn webkit_discovery_requests_connected_applications() {
        let source = include_str!("webkit.rs");
        let identifier_index = source
            .find("\"_rpc_reportIdentifier:\"")
            .expect("discovery should report a WebKit connection identifier");
        let connected_applications_index = source
            .find("send_get_connected_applications(")
            .expect("discovery should explicitly request connected applications");
        assert!(identifier_index < connected_applications_index);
        assert!(source.contains("_rpc_getConnectedApplications:"));
    }

    #[test]
    fn incomplete_and_transient_pages_are_not_advertised() {
        let incomplete_page = WebKitPage {
            app_id: "PID:1".to_owned(),
            page_id: 1,
            title: Some("Safari".to_owned()),
            url: None,
            connection_id: None,
        };
        let blank_page = WebKitPage {
            app_id: "PID:1".to_owned(),
            page_id: 1,
            title: None,
            url: Some("about:blank".to_owned()),
            connection_id: None,
        };
        let titled_blank_page = WebKitPage {
            app_id: "PID:1".to_owned(),
            page_id: 1,
            title: Some("Blank".to_owned()),
            url: Some("about:blank".to_owned()),
            connection_id: None,
        };
        let real_page = WebKitPage {
            app_id: "PID:1".to_owned(),
            page_id: 1,
            title: Some("SimDeck".to_owned()),
            url: Some("https://simdeck.nativescript.org/".to_owned()),
            connection_id: None,
        };

        assert!(is_incomplete_or_transient_page(&incomplete_page));
        assert!(is_incomplete_or_transient_page(&blank_page));
        assert!(!is_incomplete_or_transient_page(&titled_blank_page));
        assert!(!is_incomplete_or_transient_page(&real_page));

        assert!(!is_inspectable_webkit_target(&WebKitTarget {
            id: "target".to_owned(),
            app_id: "PID:1".to_owned(),
            app_name: Some("Safari".to_owned()),
            app_active: true,
            page_active: false,
            page_id: 1,
            title: Some("Safari".to_owned()),
            url: None,
            kind: "safari-page".to_owned(),
            inspector_url: "/inspector".to_owned(),
            web_socket_url: "/socket".to_owned(),
        }));
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

    #[test]
    fn simulator_launchd_socket_marker_matches_udid() {
        let udid = "2B3B4CA8-6F57-44D8-8AAE-1394456282B7";
        let launchd_marker = format!("CoreSimulator.SimDevice.{udid}/");
        assert!(
            "/private/var/tmp/com.apple.CoreSimulator.SimDevice.2B3B4CA8-6F57-44D8-8AAE-1394456282B7/syslogsock"
                .contains(&launchd_marker)
        );
    }
}
