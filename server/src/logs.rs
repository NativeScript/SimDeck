use crate::error::AppError;
use crate::native::bridge::{log_entry_matches, LogEntry, LogFilters};
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, ChildStdout, Command};
use tokio::sync::Mutex;

const MAX_LOG_ENTRIES: usize = 5_000;
type LogStreamSpawner = Arc<dyn Fn(&str) -> Result<Child, AppError> + Send + Sync>;

#[derive(Clone)]
pub struct LogRegistry {
    streams: Arc<Mutex<HashMap<String, Arc<LogStreamState>>>>,
    spawn_log_stream: LogStreamSpawner,
}

impl Default for LogRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Default)]
struct LogStreamState {
    entries: Mutex<VecDeque<LogEntry>>,
    status: Mutex<LogStreamStatus>,
}

#[derive(Default)]
struct LogStreamStatus {
    phase: LogStreamPhase,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum LogStreamPhase {
    #[default]
    Idle,
    Starting,
    Running,
}

async fn run_start_if_idle<T, F, Fut>(
    state: &Arc<LogStreamState>,
    start: F,
) -> Result<Option<T>, AppError>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T, AppError>>,
{
    {
        let mut status = state.status.lock().await;
        if !matches!(status.phase, LogStreamPhase::Idle) {
            return Ok(None);
        }
        status.phase = LogStreamPhase::Starting;
    }

    match start().await {
        Ok(value) => {
            state.status.lock().await.phase = LogStreamPhase::Running;
            Ok(Some(value))
        }
        Err(error) => {
            state.status.lock().await.phase = LogStreamPhase::Idle;
            Err(error)
        }
    }
}

impl LogRegistry {
    pub fn new() -> Self {
        Self {
            streams: Arc::new(Mutex::new(HashMap::new())),
            spawn_log_stream: Arc::new(spawn_log_stream_process),
        }
    }

    #[cfg(test)]
    fn new_for_tests(spawn_log_stream: LogStreamSpawner) -> Self {
        Self {
            streams: Arc::new(Mutex::new(HashMap::new())),
            spawn_log_stream,
        }
    }

    pub async fn ensure_started(&self, udid: &str) -> Result<(), AppError> {
        let state = {
            let mut streams = self.streams.lock().await;
            streams
                .entry(udid.to_owned())
                .or_insert_with(|| Arc::new(LogStreamState::default()))
                .clone()
        };
        let spawn_log_stream = self.spawn_log_stream.clone();

        let Some((child, stdout)) = run_start_if_idle(&state, || async move {
            let mut child = spawn_log_stream(udid)?;

            let stdout = child
                .stdout
                .take()
                .ok_or_else(|| AppError::native("Simulator log stream did not expose stdout."))?;

            Ok((child, stdout))
        })
        .await?
        else {
            return Ok(());
        };

        let udid = udid.to_owned();
        tokio::spawn(async move {
            let result = read_log_stream(child, stdout, state.clone()).await;
            let mut status = state.status.lock().await;
            status.phase = LogStreamPhase::Idle;
            if let Err(error) = result {
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

fn spawn_log_stream_process(udid: &str) -> Result<Child, AppError> {
    Command::new("xcrun")
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
        .map_err(|error| AppError::native(format!("Unable to start simulator log stream. {error}")))
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

#[cfg(test)]
mod tests {
    use super::{run_start_if_idle, LogRegistry, LogStreamPhase, LogStreamSpawner, LogStreamState};
    use crate::error::AppError;
    use crate::native::bridge::LogFilters;
    use std::process::Stdio;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::process::Command;
    use tokio::sync::Barrier;
    use tokio::time::{sleep, Duration, Instant};

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_start_only_runs_one_spawn_action() {
        let state = Arc::new(LogStreamState::default());
        let calls = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(Barrier::new(17));
        let mut handles = Vec::new();

        for _ in 0..16 {
            let state = state.clone();
            let calls = calls.clone();
            let barrier = barrier.clone();
            handles.push(tokio::spawn(async move {
                barrier.wait().await;
                run_start_if_idle(&state, move || {
                    let calls = calls.clone();
                    async move {
                        let call = calls.fetch_add(1, Ordering::SeqCst) + 1;
                        sleep(Duration::from_millis(20)).await;
                        Ok::<_, AppError>(call)
                    }
                })
                .await
            }));
        }

        barrier.wait().await;

        let mut started = 0;
        for handle in handles {
            if let Some(call) = handle.await.unwrap().unwrap() {
                started += 1;
                assert_eq!(call, 1);
            }
        }

        assert_eq!(started, 1);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(state.status.lock().await.phase, LogStreamPhase::Running);
    }

    #[tokio::test]
    async fn failed_start_returns_to_idle_and_can_retry() {
        let state = Arc::new(LogStreamState::default());

        let error = run_start_if_idle(&state, || async {
            Err::<usize, _>(AppError::native("failed to spawn"))
        })
        .await
        .unwrap_err();
        assert_eq!(error.to_string(), "failed to spawn");
        assert_eq!(state.status.lock().await.phase, LogStreamPhase::Idle);

        let started = run_start_if_idle(&state, || async { Ok::<_, AppError>(7usize) })
            .await
            .unwrap();
        assert_eq!(started, Some(7));
        assert_eq!(state.status.lock().await.phase, LogStreamPhase::Running);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn ensure_started_only_spawns_one_process_and_streams_logs() {
        let calls = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(Barrier::new(17));
        let spawner: LogStreamSpawner = Arc::new({
            let calls = calls.clone();
            move |_| {
                calls.fetch_add(1, Ordering::SeqCst);
                Command::new("/bin/sh")
                    .args([
                        "-c",
                        "printf '%s\n' '{\"timestamp\":\"2026-04-23T12:00:00Z\",\"messageType\":\"Info\",\"processImagePath\":\"/Applications/Test.app/Test\",\"processID\":42,\"subsystem\":\"dev.xcw\",\"category\":\"stream\",\"eventMessage\":\"hello\"}'; sleep 0.05",
                    ])
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|error| {
                        AppError::native(format!(
                            "Unable to start simulator log stream. {error}"
                        ))
                    })
            }
        });
        let registry = LogRegistry::new_for_tests(spawner);

        let mut handles = Vec::new();
        for _ in 0..16 {
            let registry = registry.clone();
            let barrier = barrier.clone();
            handles.push(tokio::spawn(async move {
                barrier.wait().await;
                registry.ensure_started("booted-sim").await
            }));
        }

        barrier.wait().await;

        for handle in handles {
            handle.await.unwrap().unwrap();
        }

        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let filters = LogFilters::new(Vec::new(), Vec::new(), String::new());
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let entries = registry.snapshot("booted-sim", &filters, 10).await;
            if let Some(entry) = entries.last() {
                assert_eq!(entry.process, "Test");
                assert_eq!(entry.message, "hello");
                assert_eq!(entry.level, "Info");
                break;
            }

            assert!(
                Instant::now() < deadline,
                "timed out waiting for streamed log entry"
            );
            sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn ensure_started_can_retry_after_spawn_failure() {
        let calls = Arc::new(AtomicUsize::new(0));
        let spawner: LogStreamSpawner = Arc::new({
            let calls = calls.clone();
            move |_| {
                let call = calls.fetch_add(1, Ordering::SeqCst);
                if call == 0 {
                    return Err(AppError::native("failed to spawn"));
                }

                Command::new("/bin/sh")
                    .args(["-c", "sleep 0.05"])
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|error| {
                        AppError::native(format!("Unable to start simulator log stream. {error}"))
                    })
            }
        });
        let registry = LogRegistry::new_for_tests(spawner);

        let error = registry.ensure_started("booted-sim").await.unwrap_err();
        assert_eq!(error.to_string(), "failed to spawn");
        registry.ensure_started("booted-sim").await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
