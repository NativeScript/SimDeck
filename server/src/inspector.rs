use axum::extract::ws::{Message, WebSocket};
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, Mutex, Notify};
use tokio::time::{timeout, Instant};
use tracing::{debug, warn};

const INSPECTOR_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const POLLED_INFO_REQUEST_ID: u64 = 0;

type InspectorResponse = Result<Value, String>;
type PendingResponseSender = oneshot::Sender<InspectorResponse>;
type PendingResponses = Arc<Mutex<HashMap<u64, PendingResponseSender>>>;

#[derive(Clone, Default)]
pub struct InspectorHub {
    inner: Arc<Mutex<InspectorHubState>>,
}

#[derive(Default)]
struct InspectorHubState {
    next_connection_id: u64,
    agents: HashMap<i64, InspectorAgentHandle>,
}

#[derive(Clone)]
pub struct ConnectedInspector {
    pub process_identifier: i64,
    pub info: Value,
}

#[derive(Clone)]
struct InspectorAgentHandle {
    connection_id: u64,
    info: Value,
    outgoing: mpsc::Sender<Value>,
    outbox: Arc<Mutex<VecDeque<Value>>>,
    outbox_notify: Arc<Notify>,
    pending: PendingResponses,
    next_request_id: Arc<AtomicU64>,
}

impl InspectorHub {
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

    pub async fn ensure_polled_agent(&self, process_identifier: i64) {
        {
            let inner = self.inner.lock().await;
            if inner.agents.contains_key(&process_identifier) {
                return;
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
            info: Value::Null,
            outgoing,
            outbox,
            outbox_notify: Arc::new(Notify::new()),
            pending: Arc::new(Mutex::new(HashMap::new())),
            next_request_id: Arc::new(AtomicU64::new(1)),
        };

        self.register(process_identifier, agent).await;
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
        self.inner
            .lock()
            .await
            .agents
            .insert(process_identifier, agent);
    }

    async fn unregister(&self, process_identifier: i64, connection_id: u64) {
        let mut inner = self.inner.lock().await;
        if inner
            .agents
            .get(&process_identifier)
            .map(|agent| agent.connection_id)
            == Some(connection_id)
        {
            inner.agents.remove(&process_identifier);
        }
    }

    async fn update_agent_info_from_response(&self, process_identifier: i64, response: &Value) {
        let Some(info) = inspector_info_from_response(response) else {
            return;
        };
        if info.get("processIdentifier").and_then(Value::as_i64) != Some(process_identifier) {
            return;
        }

        let mut inner = self.inner.lock().await;
        if let Some(agent) = inner.agents.get_mut(&process_identifier) {
            agent.info = info;
        }
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
