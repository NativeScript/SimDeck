use axum::extract::ws::{Message, WebSocket};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::env;
use std::fs;
use std::io;
#[cfg(unix)]
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, oneshot, Mutex, Notify};
use tokio::time::{timeout, Instant};
use tracing::{debug, warn};

const INSPECTOR_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const INSPECTOR_REGISTRY_HEARTBEAT: Duration = Duration::from_secs(10);
pub const INSPECTOR_REGISTRY_TTL: Duration = Duration::from_secs(45);
const MAX_POLLED_AGENTS: usize = 64;
const POLLED_INFO_REQUEST_ID: u64 = 0;
const POLLED_AGENT_TTL: Duration = Duration::from_secs(60);
const REGISTRY_VERSION: u32 = 1;
static REGISTRY_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

type InspectorResponse = Result<Value, String>;
type PendingResponseSender = oneshot::Sender<InspectorResponse>;
type PendingResponses = Arc<Mutex<HashMap<u64, PendingResponseSender>>>;

#[derive(Clone, Default)]
pub struct InspectorHub {
    inner: Arc<Mutex<InspectorHubState>>,
    registry: Option<InspectorRegistryAdvertisement>,
}

#[derive(Default)]
struct InspectorHubState {
    next_connection_id: u64,
    agents: HashMap<i64, InspectorAgentHandle>,
}

impl InspectorHubState {
    fn prune_stale_polled_agents(&mut self) {
        let now = Instant::now();
        self.agents.retain(|_, agent| {
            !agent.info.is_null() || now.duration_since(agent.created_at) < POLLED_AGENT_TTL
        });
    }
}

#[derive(Clone)]
pub struct ConnectedInspector {
    pub process_identifier: i64,
    pub info: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishedInspector {
    pub access_token: String,
    pub available_sources: Vec<String>,
    pub daemon_id: String,
    pub info: Value,
    pub process_identifier: i64,
    pub server_url: String,
    pub updated_at_unix_ms: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct InspectorRegistryFile {
    version: u32,
    inspectors: Vec<PublishedInspector>,
}

impl Default for InspectorRegistryFile {
    fn default() -> Self {
        Self {
            version: REGISTRY_VERSION,
            inspectors: Vec::new(),
        }
    }
}

#[derive(Clone)]
pub struct InspectorRegistryAdvertisement {
    access_token: String,
    daemon_id: String,
    path: PathBuf,
    server_url: String,
}

#[derive(Clone)]
struct InspectorAgentHandle {
    connection_id: u64,
    created_at: Instant,
    info: Value,
    outgoing: mpsc::Sender<Value>,
    outbox: Arc<Mutex<VecDeque<Value>>>,
    outbox_notify: Arc<Notify>,
    pending: PendingResponses,
    next_request_id: Arc<AtomicU64>,
}

impl InspectorHub {
    pub fn with_registry(registry: InspectorRegistryAdvertisement) -> Self {
        Self {
            inner: Arc::default(),
            registry: Some(registry),
        }
    }

    pub async fn handle_socket(&self, socket: WebSocket) {
        let connection_id = self.allocate_connection_id().await;
        let (mut sender, mut receiver) = socket.split();
        let (outgoing_tx, mut outgoing_rx) = mpsc::channel::<Value>(32);
        let outbox = Arc::new(Mutex::new(VecDeque::new()));
        let outbox_notify = Arc::new(Notify::new());
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let process_identifier = Arc::new(Mutex::new(None::<i64>));

        let writer = tokio::spawn(async move {
            while let Some(message) = outgoing_rx.recv().await {
                if sender
                    .send(Message::Text(message.to_string().into()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });

        let handle = InspectorAgentHandle {
            connection_id,
            created_at: Instant::now(),
            info: Value::Null,
            outgoing: outgoing_tx,
            outbox,
            outbox_notify,
            pending,
            next_request_id: Arc::new(AtomicU64::new(1)),
        };

        let reader_hub = self.clone();
        let reader_handle = handle.clone();
        let reader_pending = reader_handle.pending.clone();
        let reader_process_identifier = process_identifier.clone();
        tokio::spawn(async move {
            while let Some(message) = receiver.next().await {
                match message {
                    Ok(Message::Text(text)) => {
                        handle_incoming_message(
                            &reader_hub,
                            &reader_handle,
                            &reader_pending,
                            &reader_process_identifier,
                            text.as_str(),
                        )
                        .await;
                    }
                    Ok(Message::Binary(bytes)) => {
                        if let Ok(text) = std::str::from_utf8(&bytes) {
                            handle_incoming_message(
                                &reader_hub,
                                &reader_handle,
                                &reader_pending,
                                &reader_process_identifier,
                                text,
                            )
                            .await;
                        }
                    }
                    Ok(Message::Close(_)) => break,
                    Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => {}
                    Err(error) => {
                        debug!("NativeScript inspector WebSocket closed: {error}");
                        break;
                    }
                }
            }

            if let Some(pid) = *reader_process_identifier.lock().await {
                reader_hub.unregister(pid, connection_id).await;
            }
            fail_all_pending(&reader_pending, "NativeScript inspector disconnected.").await;
            writer.abort();
        });

        match handle
            .query("Inspector.getInfo", Value::Null, INSPECTOR_REQUEST_TIMEOUT)
            .await
        {
            Ok(info) => {
                let Some(pid) = info.get("processIdentifier").and_then(Value::as_i64) else {
                    warn!("NativeScript inspector did not report processIdentifier.");
                    return;
                };
                *process_identifier.lock().await = Some(pid);
                self.register(pid, handle.with_info(info)).await;
            }
            Err(error) => {
                if process_identifier.lock().await.is_none() {
                    warn!("NativeScript inspector registration failed: {error}");
                }
            }
        }
    }

    pub async fn connected(&self) -> Vec<ConnectedInspector> {
        self.inner
            .lock()
            .await
            .agents
            .iter()
            .filter(|(_, agent)| !agent.info.is_null())
            .map(|(process_identifier, agent)| ConnectedInspector {
                process_identifier: *process_identifier,
                info: agent.info.clone(),
            })
            .collect()
    }

    pub async fn published_inspectors(&self) -> Vec<PublishedInspector> {
        let Some(registry) = self.registry.as_ref() else {
            return Vec::new();
        };
        registry.read_live_entries().await
    }

    pub async fn ensure_polled_agent(&self, process_identifier: i64) -> Result<(), String> {
        {
            let mut inner = self.inner.lock().await;
            inner.prune_stale_polled_agents();
            if inner.agents.contains_key(&process_identifier) {
                return Ok(());
            }
            if inner.agents.len() >= MAX_POLLED_AGENTS {
                return Err("Too many pending NativeScript inspector agents.".to_owned());
            }
        }

        let connection_id = self.allocate_connection_id().await;
        let (outgoing, _receiver) = mpsc::channel::<Value>(1);
        let outbox = Arc::new(Mutex::new(VecDeque::new()));
        outbox.lock().await.push_back(json!({
            "id": POLLED_INFO_REQUEST_ID,
            "method": "Inspector.getInfo",
            "params": Value::Null,
        }));

        let agent = InspectorAgentHandle {
            connection_id,
            created_at: Instant::now(),
            info: Value::Null,
            outgoing,
            outbox,
            outbox_notify: Arc::new(Notify::new()),
            pending: Arc::new(Mutex::new(HashMap::new())),
            next_request_id: Arc::new(AtomicU64::new(1)),
        };

        self.register(process_identifier, agent).await;
        Ok(())
    }

    pub async fn query_with_timeout(
        &self,
        process_identifier: i64,
        method: &str,
        params: Value,
        wait: Duration,
    ) -> Result<Value, String> {
        let agent = self
            .inner
            .lock()
            .await
            .agents
            .get(&process_identifier)
            .cloned()
            .ok_or_else(|| {
                format!("NativeScript inspector for process {process_identifier} is not connected.")
            })?;
        agent.query(method, params, wait).await
    }

    pub async fn poll(
        &self,
        process_identifier: i64,
        wait: Duration,
    ) -> Result<Option<Value>, String> {
        let agent = self
            .inner
            .lock()
            .await
            .agents
            .get(&process_identifier)
            .cloned()
            .ok_or_else(|| {
                format!("NativeScript inspector for process {process_identifier} is not connected.")
            })?;
        agent.poll(wait).await
    }

    pub async fn complete_response(
        &self,
        process_identifier: i64,
        response: Value,
    ) -> Result<(), String> {
        let agent = self
            .inner
            .lock()
            .await
            .agents
            .get(&process_identifier)
            .cloned()
            .ok_or_else(|| {
                format!("NativeScript inspector for process {process_identifier} is not connected.")
            })?;
        self.update_agent_info_from_response(process_identifier, &response)
            .await;
        agent.complete_response(response).await;
        Ok(())
    }

    async fn allocate_connection_id(&self) -> u64 {
        let mut inner = self.inner.lock().await;
        inner.next_connection_id = inner.next_connection_id.saturating_add(1);
        inner.next_connection_id
    }

    async fn register(&self, process_identifier: i64, agent: InspectorAgentHandle) {
        debug!(
            "Registered NativeScript inspector for process {}.",
            process_identifier
        );
        let connection_id = agent.connection_id;
        let info = agent.info.clone();
        self.inner
            .lock()
            .await
            .agents
            .insert(process_identifier, agent);
        if !info.is_null() {
            self.publish_inspector(process_identifier, &info).await;
            self.start_registry_heartbeat(process_identifier, connection_id);
        }
    }

    async fn unregister(&self, process_identifier: i64, connection_id: u64) {
        let mut inner = self.inner.lock().await;
        let removed = if inner
            .agents
            .get(&process_identifier)
            .map(|agent| agent.connection_id)
            == Some(connection_id)
        {
            inner.agents.remove(&process_identifier);
            true
        } else {
            false
        };
        drop(inner);
        if removed {
            if let Some(registry) = self.registry.as_ref() {
                if let Err(error) = registry.remove(process_identifier).await {
                    debug!("Failed to remove SimDeck inspector registry entry: {error}");
                }
            }
        }
    }

    async fn update_agent_info_from_response(&self, process_identifier: i64, response: &Value) {
        let Some(info) = inspector_info_from_response(response) else {
            return;
        };
        if info.get("processIdentifier").and_then(Value::as_i64) != Some(process_identifier) {
            return;
        }

        let mut heartbeat_connection_id = None;
        let mut inner = self.inner.lock().await;
        if let Some(agent) = inner.agents.get_mut(&process_identifier) {
            if agent.info.is_null() {
                heartbeat_connection_id = Some(agent.connection_id);
            }
            agent.info = info.clone();
        }
        drop(inner);
        self.publish_inspector(process_identifier, &info).await;
        if let Some(connection_id) = heartbeat_connection_id {
            self.start_registry_heartbeat(process_identifier, connection_id);
        }
    }

    async fn publish_inspector(&self, process_identifier: i64, info: &Value) {
        let Some(registry) = self.registry.as_ref() else {
            return;
        };
        if let Err(error) = registry.upsert(process_identifier, info.clone()).await {
            debug!("Failed to publish SimDeck inspector registry entry: {error}");
        }
    }

    fn start_registry_heartbeat(&self, process_identifier: i64, connection_id: u64) {
        let hub = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(INSPECTOR_REGISTRY_HEARTBEAT);
            loop {
                interval.tick().await;
                let info = {
                    let inner = hub.inner.lock().await;
                    let Some(agent) = inner.agents.get(&process_identifier) else {
                        break;
                    };
                    if agent.connection_id != connection_id {
                        break;
                    }
                    agent.info.clone()
                };
                if info.is_null() {
                    continue;
                }
                hub.publish_inspector(process_identifier, &info).await;
            }
        });
    }
}

impl InspectorRegistryAdvertisement {
    pub fn new(config: &crate::config::Config) -> Self {
        Self {
            access_token: config.access_token.clone(),
            daemon_id: format!("{}:{}", process::id(), config.http_port),
            path: inspector_registry_path(),
            server_url: registry_server_url(config),
        }
    }

    #[cfg(test)]
    pub fn for_test(
        path: PathBuf,
        daemon_id: impl Into<String>,
        server_url: impl Into<String>,
        access_token: impl Into<String>,
    ) -> Self {
        Self {
            access_token: access_token.into(),
            daemon_id: daemon_id.into(),
            path,
            server_url: server_url.into(),
        }
    }

    async fn upsert(&self, process_identifier: i64, info: Value) -> io::Result<()> {
        let _lock = RegistryFileLock::acquire(&self.path)?;
        let mut registry = read_registry_file(&self.path);
        prune_registry_file(&mut registry);
        registry.inspectors.retain(|entry| {
            !(entry.process_identifier == process_identifier && entry.daemon_id == self.daemon_id)
        });
        registry.inspectors.push(PublishedInspector {
            access_token: self.access_token.clone(),
            available_sources: inspector_available_sources(&info),
            daemon_id: self.daemon_id.clone(),
            info,
            process_identifier,
            server_url: self.server_url.clone(),
            updated_at_unix_ms: now_unix_ms(),
        });
        write_registry_file(&self.path, &registry)
    }

    async fn remove(&self, process_identifier: i64) -> io::Result<()> {
        let _lock = RegistryFileLock::acquire(&self.path)?;
        let mut registry = read_registry_file(&self.path);
        let original_len = registry.inspectors.len();
        registry.inspectors.retain(|entry| {
            !(entry.process_identifier == process_identifier && entry.daemon_id == self.daemon_id)
        });
        prune_registry_file(&mut registry);
        if registry.inspectors.len() != original_len {
            write_registry_file(&self.path, &registry)?;
        }
        Ok(())
    }

    async fn read_live_entries(&self) -> Vec<PublishedInspector> {
        let _lock = RegistryFileLock::acquire(&self.path).ok();
        let mut registry = read_registry_file(&self.path);
        prune_registry_file(&mut registry);
        registry.inspectors
    }
}

struct RegistryFileLock {
    #[cfg(unix)]
    file: fs::File,
}

impl RegistryFileLock {
    fn acquire(path: &Path) -> io::Result<Self> {
        #[cfg(unix)]
        {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let lock_path = path.with_extension("json.lock");
            let file = fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(false)
                .open(lock_path)?;
            let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
            if result != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self { file })
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            Ok(Self {})
        }
    }
}

impl Drop for RegistryFileLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        unsafe {
            let _ = libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

fn inspector_registry_path() -> PathBuf {
    env::var_os("SIMDECK_INSPECTOR_REGISTRY_PATH")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".simdeck")))
        .unwrap_or_else(|| env::temp_dir().join("simdeck"))
        .join("inspectors.json")
}

fn registry_server_url(config: &crate::config::Config) -> String {
    let host = if config.bind_ip.is_loopback() || config.bind_ip.is_unspecified() {
        "127.0.0.1".to_owned()
    } else {
        config.advertise_host.clone()
    };
    format!("http://{}:{}", http_host(&host), config.http_port)
}

fn http_host(host: &str) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_owned()
    }
}

fn read_registry_file(path: &Path) -> InspectorRegistryFile {
    let Ok(data) = fs::read_to_string(path) else {
        return InspectorRegistryFile::default();
    };
    serde_json::from_str(&data).unwrap_or_default()
}

fn write_registry_file(path: &Path, registry: &InspectorRegistryFile) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary_path = path.with_extension(format!(
        "json.{}.{}.tmp",
        process::id(),
        REGISTRY_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let data = serde_json::to_vec_pretty(registry).map_err(io::Error::other)?;
    fs::write(&temporary_path, data)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&temporary_path, fs::Permissions::from_mode(0o600));
    }
    fs::rename(temporary_path, path)
}

fn prune_registry_file(registry: &mut InspectorRegistryFile) {
    let cutoff = now_unix_ms().saturating_sub(INSPECTOR_REGISTRY_TTL.as_millis() as u64);
    registry.version = REGISTRY_VERSION;
    registry
        .inspectors
        .retain(|entry| entry.updated_at_unix_ms >= cutoff);
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn inspector_available_sources(info: &Value) -> Vec<String> {
    let mut sources = Vec::new();
    let react_native_available = info
        .get("reactNative")
        .and_then(|value| value.get("available"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if react_native_available {
        sources.push("react-native".to_owned());
    }
    let flutter_available = info
        .get("flutter")
        .and_then(|value| value.get("available"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if flutter_available {
        sources.push("flutter".to_owned());
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
            "nativescript" => sources.push("nativescript".to_owned()),
            "react-native" => push_unique_source(&mut sources, "react-native"),
            "flutter" => push_unique_source(&mut sources, "flutter"),
            "swiftui" => push_unique_source(&mut sources, "swiftui"),
            _ => {}
        }
    }
    let uikit_available = info
        .get("uikit")
        .and_then(|value| value.get("available"))
        .and_then(Value::as_bool)
        .unwrap_or(!(react_native_available || flutter_available));
    if uikit_available {
        sources.push("in-app-inspector".to_owned());
    }
    sources
}

fn push_unique_source(sources: &mut Vec<String>, source: &str) {
    if !sources.iter().any(|value| value == source) {
        sources.push(source.to_owned());
    }
}

impl InspectorAgentHandle {
    async fn query(&self, method: &str, params: Value, wait: Duration) -> Result<Value, String> {
        let id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let request = json!({
            "id": id,
            "method": method,
            "params": params,
        });
        let (response_tx, response_rx) = oneshot::channel();
        self.pending.lock().await.insert(id, response_tx);

        self.outbox.lock().await.push_back(request.clone());
        self.outbox_notify.notify_waiters();
        let _ = self.outgoing.send(request).await;

        match timeout(wait, response_rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("NativeScript inspector response channel closed.".to_owned()),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(format!(
                    "Timed out waiting for NativeScript inspector method {method}."
                ))
            }
        }
    }

    async fn poll(&self, wait: Duration) -> Result<Option<Value>, String> {
        let deadline = Instant::now() + wait;
        loop {
            if let Some(request) = self.outbox.lock().await.pop_front() {
                return Ok(Some(request));
            }

            let now = Instant::now();
            if now >= deadline {
                return Ok(None);
            }

            if timeout(deadline - now, self.outbox_notify.notified())
                .await
                .is_err()
            {
                return Ok(None);
            }
        }
    }

    async fn complete_response(&self, response: Value) {
        complete_pending_response_value(&self.pending, response).await;
    }

    fn with_info(&self, info: Value) -> Self {
        Self {
            connection_id: self.connection_id,
            created_at: self.created_at,
            info,
            outgoing: self.outgoing.clone(),
            outbox: self.outbox.clone(),
            outbox_notify: self.outbox_notify.clone(),
            pending: self.pending.clone(),
            next_request_id: self.next_request_id.clone(),
        }
    }
}

async fn handle_incoming_message(
    hub: &InspectorHub,
    handle: &InspectorAgentHandle,
    pending: &PendingResponses,
    process_identifier: &Arc<Mutex<Option<i64>>>,
    text: &str,
) {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return;
    };

    if value.get("id").and_then(Value::as_u64).is_some() {
        complete_pending_response_value(pending, value).await;
        return;
    }

    let method = value
        .get("method")
        .or_else(|| value.get("event"))
        .and_then(Value::as_str);
    if method != Some("Inspector.ready") {
        return;
    }

    let info = value
        .get("params")
        .or_else(|| value.get("info"))
        .cloned()
        .unwrap_or(Value::Null);
    let Some(pid) = info.get("processIdentifier").and_then(Value::as_i64) else {
        warn!("NativeScript inspector ready event did not report processIdentifier.");
        return;
    };

    *process_identifier.lock().await = Some(pid);
    hub.register(pid, handle.with_info(info)).await;
}

async fn complete_pending_response_value(pending: &PendingResponses, value: Value) {
    let Some(id) = value.get("id").and_then(Value::as_u64) else {
        return;
    };
    let Some(response_tx) = pending.lock().await.remove(&id) else {
        return;
    };

    let result = if let Some(error) = value.get("error") {
        Err(error
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| error.to_string()))
    } else {
        value
            .get("result")
            .cloned()
            .ok_or_else(|| "Inspector response did not include result.".to_owned())
    };
    let _ = response_tx.send(result);
}

fn inspector_info_from_response(response: &Value) -> Option<Value> {
    let result = response.get("result")?;
    if result
        .get("protocolVersion")
        .and_then(Value::as_str)
        .is_some()
        && result
            .get("processIdentifier")
            .and_then(Value::as_i64)
            .is_some()
    {
        Some(result.clone())
    } else {
        None
    }
}

async fn fail_all_pending(pending: &PendingResponses, message: &str) {
    let mut pending = pending.lock().await;
    for (_, response_tx) in pending.drain() {
        let _ = response_tx.send(Err(message.to_owned()));
    }
}

#[cfg(test)]
mod tests {
    use super::{
        now_unix_ms, write_registry_file, InspectorHub, InspectorRegistryAdvertisement,
        InspectorRegistryFile, PublishedInspector, INSPECTOR_REGISTRY_TTL, REGISTRY_VERSION,
    };
    use serde_json::{json, Value};
    use std::{
        env, fs,
        path::{Path, PathBuf},
        process,
        time::Duration,
    };

    fn registry_test_path(name: &str) -> PathBuf {
        env::temp_dir().join(format!(
            "simdeck-inspector-registry-{name}-{}-{}.json",
            process::id(),
            now_unix_ms()
        ))
    }

    fn registry_entry(path: &Path) -> InspectorRegistryAdvertisement {
        InspectorRegistryAdvertisement::for_test(
            path.to_path_buf(),
            "daemon-a",
            "http://127.0.0.1:4310",
            "token-a",
        )
    }

    fn inspector_info() -> Value {
        json!({
            "protocolVersion": "1.0",
            "bundleIdentifier": "com.example.App",
            "processIdentifier": 123,
            "appHierarchy": {
                "available": true,
                "source": "nativescript"
            },
            "uikit": { "available": true }
        })
    }

    fn flutter_inspector_info() -> Value {
        json!({
            "protocolVersion": "1.0",
            "bundleIdentifier": "com.example.FlutterApp",
            "processIdentifier": 456,
            "flutter": {
                "available": true,
                "widgetCreationTracked": true
            },
            "appHierarchy": {
                "available": true,
                "source": "flutter"
            },
            "uikit": { "available": false }
        })
    }

    #[tokio::test]
    async fn registry_advertisement_publishes_and_removes_live_inspector() {
        let path = registry_test_path("publish");
        let registry = registry_entry(&path);

        registry.upsert(123, inspector_info()).await.unwrap();

        let entries = registry.read_live_entries().await;
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.process_identifier, 123);
        assert_eq!(entry.daemon_id, "daemon-a");
        assert_eq!(entry.server_url, "http://127.0.0.1:4310");
        assert_eq!(entry.access_token, "token-a");
        assert_eq!(
            entry.available_sources,
            vec!["nativescript".to_owned(), "in-app-inspector".to_owned()]
        );
        assert_eq!(entry.info["bundleIdentifier"], "com.example.App");

        registry.remove(123).await.unwrap();
        assert!(registry.read_live_entries().await.is_empty());

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("json.lock"));
    }

    #[tokio::test]
    async fn registry_advertisement_publishes_flutter_source() {
        let path = registry_test_path("publish-flutter");
        let registry = registry_entry(&path);

        registry
            .upsert(456, flutter_inspector_info())
            .await
            .unwrap();

        let entries = registry.read_live_entries().await;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].process_identifier, 456);
        assert_eq!(entries[0].available_sources, vec!["flutter".to_owned()]);

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("json.lock"));
    }

    #[tokio::test]
    async fn polled_inspector_info_response_publishes_registry_entry() {
        let path = registry_test_path("polled");
        let registry = registry_entry(&path);
        let hub = InspectorHub::with_registry(registry.clone());

        hub.ensure_polled_agent(123).await.unwrap();
        let request = hub.poll(123, Duration::ZERO).await.unwrap().unwrap();
        assert_eq!(request["method"], "Inspector.getInfo");

        hub.complete_response(
            123,
            json!({
                "id": request["id"].clone(),
                "result": inspector_info()
            }),
        )
        .await
        .unwrap();

        let entries = registry.read_live_entries().await;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].process_identifier, 123);
        assert_eq!(entries[0].info["bundleIdentifier"], "com.example.App");

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("json.lock"));
    }

    #[tokio::test]
    async fn registry_reader_ignores_stale_inspectors() {
        let path = registry_test_path("stale");
        let registry = registry_entry(&path);
        let stale_time = now_unix_ms()
            .saturating_sub(INSPECTOR_REGISTRY_TTL.as_millis() as u64)
            .saturating_sub(1);
        write_registry_file(
            &path,
            &InspectorRegistryFile {
                version: REGISTRY_VERSION,
                inspectors: vec![
                    PublishedInspector {
                        access_token: "stale-token".to_owned(),
                        available_sources: vec!["nativescript".to_owned()],
                        daemon_id: "stale-daemon".to_owned(),
                        info: inspector_info(),
                        process_identifier: 111,
                        server_url: "http://127.0.0.1:4311".to_owned(),
                        updated_at_unix_ms: stale_time,
                    },
                    PublishedInspector {
                        access_token: "fresh-token".to_owned(),
                        available_sources: vec!["react-native".to_owned()],
                        daemon_id: "fresh-daemon".to_owned(),
                        info: inspector_info(),
                        process_identifier: 222,
                        server_url: "http://127.0.0.1:4312".to_owned(),
                        updated_at_unix_ms: now_unix_ms(),
                    },
                ],
            },
        )
        .unwrap();

        let entries = registry.read_live_entries().await;

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].process_identifier, 222);
        assert_eq!(entries[0].daemon_id, "fresh-daemon");

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("json.lock"));
    }
}
