use crate::error::AppError;
use crate::native::bridge::{log_entry_matches, LogEntry, LogFilters};
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, ChildStdout, Command};
use tokio::sync::Mutex;

const MAX_LOG_ENTRIES: usize = 5_000;

#[derive(Clone, Default)]
pub struct LogRegistry {
    streams: Arc<Mutex<HashMap<String, Arc<LogStreamState>>>>,
}

#[derive(Default)]
struct LogStreamState {
    entries: Mutex<VecDeque<LogEntry>>,
    status: Mutex<LogStreamStatus>,
}

#[derive(Default)]
struct LogStreamStatus {
    running: bool,
    last_error: Option<String>,
}

impl LogRegistry {
    pub async fn ensure_started(&self, udid: &str) -> Result<(), AppError> {
        let state = {
            let mut streams = self.streams.lock().await;
            streams
                .entry(udid.to_owned())
                .or_insert_with(|| Arc::new(LogStreamState::default()))
                .clone()
        };

        {
            let status = state.status.lock().await;
            if status.running {
                return Ok(());
            }
        }

        let mut child = Command::new("xcrun")
            .args([
                "simctl",
                "spawn",
                udid,
                "log",
                "stream",
                "--style",
                "ndjson",
                "--level",
                "debug",
                "--type",
                "log",
                "--ignore-dropped",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| {
                AppError::native(format!("Unable to start simulator log stream. {error}"))
            })?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AppError::native("Simulator log stream did not expose stdout."))?;

        {
            let mut status = state.status.lock().await;
            status.running = true;
            status.last_error = None;
        }

        let udid = udid.to_owned();
        tokio::spawn(async move {
            let result = read_log_stream(child, stdout, state.clone()).await;
            let mut status = state.status.lock().await;
            status.running = false;
            status.last_error = result.err();
            if let Some(error) = &status.last_error {
                tracing::warn!(%udid, %error, "simulator log stream stopped");
            }
        });

        Ok(())
    }

    pub async fn snapshot(&self, udid: &str, filters: &LogFilters, limit: usize) -> Vec<LogEntry> {
        let state = {
            let streams = self.streams.lock().await;
            streams.get(udid).cloned()
        };
        let Some(state) = state else {
            return Vec::new();
        };

        let entries = state.entries.lock().await;
        let mut matching: Vec<LogEntry> = entries
            .iter()
            .filter(|entry| log_entry_matches(entry, filters))
            .cloned()
            .collect();
        if matching.len() > limit {
            matching = matching.split_off(matching.len() - limit);
        }
        matching
    }
}

async fn read_log_stream(
    mut child: Child,
    stdout: ChildStdout,
    state: Arc<LogStreamState>,
) -> Result<(), String> {
    let mut lines = BufReader::new(stdout).lines();
    while let Some(line) = lines.next_line().await.map_err(|error| error.to_string())? {
        let Some(entry) = parse_log_entry(&line) else {
            continue;
        };

        let mut entries = state.entries.lock().await;
        entries.push_back(entry);
        while entries.len() > MAX_LOG_ENTRIES {
            entries.pop_front();
        }
    }

    if matches!(child.try_wait(), Ok(None)) {
        let _ = child.kill().await;
    }
    let _ = child.wait().await;
    Ok(())
}

fn parse_log_entry(line: &str) -> Option<LogEntry> {
    let trimmed = line.trim();
    if !trimmed.starts_with('{') {
        return None;
    }

    let payload: Value = serde_json::from_str(trimmed).ok()?;
    let process_path = payload
        .get("processImagePath")
        .and_then(Value::as_str)
        .unwrap_or("");
    let process = process_path
        .rsplit('/')
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown")
        .to_owned();

    Some(LogEntry {
        timestamp: string_field(&payload, "timestamp"),
        level: non_empty_string_field(&payload, "messageType")
            .unwrap_or_else(|| "Default".to_owned()),
        process,
        pid: payload.get("processID").cloned().unwrap_or(Value::Null),
        subsystem: string_field(&payload, "subsystem"),
        category: string_field(&payload, "category"),
        message: non_empty_string_field(&payload, "eventMessage")
            .or_else(|| non_empty_string_field(&payload, "formatString"))
            .unwrap_or_default(),
    })
}

fn string_field(payload: &Value, key: &str) -> String {
    non_empty_string_field(payload, key).unwrap_or_default()
}

fn non_empty_string_field(payload: &Value, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}
