use crate::error::AppError;
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::ffi::c_void;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::process::Command;
use tokio::time::timeout;

const HISTORY_RETENTION_MS: u64 = 10 * 60 * 1000;
const HISTORY_MAX_SAMPLES: usize = 720;
const PROCESS_LIST_TIMEOUT: Duration = Duration::from_secs(2);
const PROCESS_SAMPLE_TIMEOUT: Duration = Duration::from_secs(2);
const NETWORK_SAMPLE_TIMEOUT: Duration = Duration::from_millis(650);
const NETWORK_TOTALS_TIMEOUT: Duration = Duration::from_secs(6);
const STACK_SAMPLE_MAX_BYTES: usize = 256 * 1024;

#[derive(Clone, Default)]
pub struct PerformanceRegistry {
    inner: Arc<Mutex<PerformanceState>>,
}

#[derive(Default)]
struct PerformanceState {
    last_raw: HashMap<i32, RawCounterSnapshot>,
    history: HashMap<i32, VecDeque<PerformanceSample>>,
    hang: HashMap<i32, HangTracker>,
}

#[derive(Clone, Debug)]
struct RawCounterSnapshot {
    sampled_at: Instant,
    timestamp_ms: u64,
    cpu_time_ns: Option<u64>,
    disk_read_bytes: Option<u64>,
    disk_write_bytes: Option<u64>,
    network_received_bytes: Option<u64>,
    network_sent_bytes: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForegroundProcess {
    pub process_identifier: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle_identifier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_name: Option<String>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DisplaySignal {
    pub frame_sequence: u64,
    pub last_frame_at_ms: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct PerformanceQuery {
    pub pid: Option<i32>,
    pub history_window_ms: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulatorPerformanceSnapshot {
    pub udid: String,
    pub sampled_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_pid: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub foreground_process: Option<ForegroundProcess>,
    pub processes: Vec<PerformanceProcess>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current: Option<PerformanceSample>,
    pub history: Vec<PerformanceSample>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformanceProcess {
    pub pid: i32,
    pub parent_pid: i32,
    pub process: String,
    pub role: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle_identifier: Option<String>,
    pub command: String,
    pub is_foreground: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformanceSample {
    pub pid: i32,
    pub timestamp_ms: u64,
    pub cpu_percent: f64,
    pub memory_resident_bytes: Option<u64>,
    pub memory_footprint_bytes: Option<u64>,
    pub memory_peak_footprint_bytes: Option<u64>,
    pub disk_read_bytes: Option<u64>,
    pub disk_write_bytes: Option<u64>,
    pub disk_read_bytes_per_second: Option<f64>,
    pub disk_write_bytes_per_second: Option<f64>,
    pub network_received_bytes: Option<u64>,
    pub network_sent_bytes: Option<u64>,
    pub network_received_bytes_per_second: Option<f64>,
    pub network_sent_bytes_per_second: Option<f64>,
    pub network_connection_count: Option<usize>,
    pub network_established_connection_count: Option<usize>,
    pub network_endpoints: Vec<String>,
    pub hang: HangStatus,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HangStatus {
    pub state: String,
    pub stale_ms: Option<u64>,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StackSampleReport {
    pub pid: i32,
    pub seconds: u64,
    pub sampled_at: u64,
    pub report: String,
    pub stderr: String,
    pub truncated: bool,
}

#[derive(Clone, Debug)]
struct PsProcess {
    pid: i32,
    parent_pid: i32,
    state: String,
    cpu_percent: f64,
    rss_kb: Option<u64>,
    command: String,
}

#[derive(Clone, Debug)]
struct RawPerformanceSample {
    process: PerformanceProcess,
    ps_cpu_percent: f64,
    ps_memory_resident_bytes: Option<u64>,
    rusage: Option<ProcessRUsage>,
    network: Option<NetworkSnapshot>,
}

#[derive(Clone, Copy, Debug)]
struct ProcessRUsage {
    user_time_ns: u64,
    system_time_ns: u64,
    resident_size: u64,
    phys_footprint: u64,
    lifetime_max_phys_footprint: u64,
    disk_read_bytes: u64,
    disk_write_bytes: u64,
}

#[derive(Clone, Debug)]
struct NetworkSnapshot {
    connection_count: usize,
    established_connection_count: usize,
    received_bytes: Option<u64>,
    sent_bytes: Option<u64>,
    endpoints: Vec<String>,
}

#[derive(Clone, Debug, Default)]
struct HangTracker {
    last_frame_sequence: u64,
    last_frame_change_ms: u64,
}

impl PerformanceRegistry {
    pub async fn list_processes(
        &self,
        udid: &str,
        foreground: Option<ForegroundProcess>,
    ) -> Result<Vec<PerformanceProcess>, AppError> {
        let ps = list_ps_processes().await?;
        let mut processes = app_processes_from_ps(udid, foreground.as_ref(), ps);
        if let Some(foreground) = foreground.as_ref() {
            ensure_foreground_process(&mut processes, foreground).await;
        }
        processes.sort_by_key(|process| (!process.is_foreground, process.process.clone()));
        Ok(processes)
    }

    pub async fn snapshot(
        &self,
        udid: &str,
        query: PerformanceQuery,
        foreground: Option<ForegroundProcess>,
        display_signal: DisplaySignal,
    ) -> Result<SimulatorPerformanceSnapshot, AppError> {
        let sampled_at = now_ms();
        let mut warnings = Vec::new();
        let ps = list_ps_processes().await?;
        let mut processes = app_processes_from_ps(udid, foreground.as_ref(), ps);
        if let Some(foreground) = foreground.as_ref() {
            ensure_foreground_process(&mut processes, foreground).await;
        }
        processes.sort_by_key(|process| (!process.is_foreground, process.process.clone()));

        let selected_pid = query
            .pid
            .or_else(|| {
                foreground
                    .as_ref()
                    .map(|process| process.process_identifier as i32)
            })
            .or_else(|| processes.first().map(|process| process.pid));

        if processes.is_empty() {
            warnings.push("No simulator app process matched this UDID yet.".to_owned());
        }

        let mut raw_samples = Vec::new();
        for process in &processes {
            raw_samples
                .push(sample_process(process.clone(), selected_pid == Some(process.pid)).await);
        }

        let selected_pid =
            selected_pid.filter(|pid| raw_samples.iter().any(|sample| sample.process.pid == *pid));
        let (current, history) = self.merge_samples(
            raw_samples,
            selected_pid,
            display_signal,
            query.history_window_ms,
        );

        Ok(SimulatorPerformanceSnapshot {
            udid: udid.to_owned(),
            sampled_at,
            selected_pid,
            foreground_process: foreground,
            processes,
            current,
            history,
            warnings,
        })
    }

    fn merge_samples(
        &self,
        raw_samples: Vec<RawPerformanceSample>,
        selected_pid: Option<i32>,
        display_signal: DisplaySignal,
        history_window_ms: u64,
    ) -> (Option<PerformanceSample>, Vec<PerformanceSample>) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut current = None;
        let now = Instant::now();
        let now_ms = now_ms();

        for raw in raw_samples {
            let pid = raw.process.pid;
            let counter = RawCounterSnapshot {
                sampled_at: now,
                timestamp_ms: now_ms,
                cpu_time_ns: raw
                    .rusage
                    .map(|rusage| rusage.user_time_ns.saturating_add(rusage.system_time_ns)),
                disk_read_bytes: raw.rusage.map(|rusage| rusage.disk_read_bytes),
                disk_write_bytes: raw.rusage.map(|rusage| rusage.disk_write_bytes),
                network_received_bytes: raw
                    .network
                    .as_ref()
                    .and_then(|network| network.received_bytes),
                network_sent_bytes: raw.network.as_ref().and_then(|network| network.sent_bytes),
            };
            let previous = inner.last_raw.insert(pid, counter.clone());
            let cpu_percent = cpu_percent(&raw, previous.as_ref(), &counter);
            let disk_read_bytes_per_second = rate_per_second(
                previous
                    .as_ref()
                    .and_then(|previous| previous.disk_read_bytes),
                counter.disk_read_bytes,
                previous.as_ref(),
                &counter,
            );
            let disk_write_bytes_per_second = rate_per_second(
                previous
                    .as_ref()
                    .and_then(|previous| previous.disk_write_bytes),
                counter.disk_write_bytes,
                previous.as_ref(),
                &counter,
            );
            let network_received_bytes_per_second = rate_per_second(
                previous
                    .as_ref()
                    .and_then(|previous| previous.network_received_bytes),
                counter.network_received_bytes,
                previous.as_ref(),
                &counter,
            );
            let network_sent_bytes_per_second = rate_per_second(
                previous
                    .as_ref()
                    .and_then(|previous| previous.network_sent_bytes),
                counter.network_sent_bytes,
                previous.as_ref(),
                &counter,
            );
            let hang = if selected_pid == Some(pid) {
                hang_status(
                    inner.hang.entry(pid).or_default(),
                    display_signal,
                    now_ms,
                    cpu_percent,
                )
            } else {
                HangStatus {
                    state: "not-selected".to_owned(),
                    stale_ms: None,
                    reason: "Hang signal is tracked for the selected app process.".to_owned(),
                }
            };
            let sample = PerformanceSample {
                pid,
                timestamp_ms: now_ms,
                cpu_percent,
                memory_resident_bytes: raw
                    .rusage
                    .map(|rusage| rusage.resident_size)
                    .or(raw.ps_memory_resident_bytes),
                memory_footprint_bytes: raw.rusage.map(|rusage| rusage.phys_footprint),
                memory_peak_footprint_bytes: raw.rusage.map(|rusage| {
                    rusage
                        .lifetime_max_phys_footprint
                        .max(rusage.phys_footprint)
                }),
                disk_read_bytes: raw.rusage.map(|rusage| rusage.disk_read_bytes),
                disk_write_bytes: raw.rusage.map(|rusage| rusage.disk_write_bytes),
                disk_read_bytes_per_second,
                disk_write_bytes_per_second,
                network_received_bytes: raw
                    .network
                    .as_ref()
                    .and_then(|network| network.received_bytes),
                network_sent_bytes: raw.network.as_ref().and_then(|network| network.sent_bytes),
                network_received_bytes_per_second,
                network_sent_bytes_per_second,
                network_connection_count: raw
                    .network
                    .as_ref()
                    .map(|network| network.connection_count),
                network_established_connection_count: raw
                    .network
                    .as_ref()
                    .map(|network| network.established_connection_count),
                network_endpoints: raw
                    .network
                    .map(|network| network.endpoints)
                    .unwrap_or_default(),
                hang,
            };

            let history = inner.history.entry(pid).or_default();
            history.push_back(sample.clone());
            prune_history(history, now_ms);
            if selected_pid == Some(pid) {
                current = Some(sample);
            }
        }

        let history = selected_pid
            .and_then(|pid| inner.history.get(&pid))
            .map(|history| {
                let window_start = now_ms.saturating_sub(history_window_ms);
                history
                    .iter()
                    .filter(|sample| sample.timestamp_ms >= window_start)
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        (current, history)
    }
}

pub async fn sample_stack(pid: i32, seconds: u64) -> Result<StackSampleReport, AppError> {
    if pid <= 0 {
        return Err(AppError::bad_request("Process id must be positive."));
    }
    let seconds = seconds.clamp(1, 30);
    let sampled_at = now_ms();
    let report_path = std::env::temp_dir().join(format!(
        "simdeck-sample-{pid}-{}-{}.txt",
        std::process::id(),
        sampled_at
    ));
    let result = timeout(
        Duration::from_secs(seconds + 20),
        Command::new("sample")
            .arg(pid.to_string())
            .arg(seconds.to_string())
            .arg("-file")
            .arg(&report_path)
            .output(),
    )
    .await
    .map_err(|_| AppError::native("Timed out sampling process stack."))?
    .map_err(|error| AppError::native(format!("Unable to run sample: {error}")))?;

    let stderr = String::from_utf8_lossy(&result.stderr).trim().to_owned();
    let mut report = tokio::fs::read(&report_path).await.unwrap_or_default();
    let _ = tokio::fs::remove_file(&report_path).await;
    let truncated = report.len() > STACK_SAMPLE_MAX_BYTES;
    if truncated {
        report.truncate(STACK_SAMPLE_MAX_BYTES);
    }

    if !result.status.success() {
        return Err(AppError::native(if stderr.is_empty() {
            format!("sample exited with status {}", result.status)
        } else {
            stderr
        }));
    }

    Ok(StackSampleReport {
        pid,
        seconds,
        sampled_at,
        report: String::from_utf8_lossy(&report).into_owned(),
        stderr,
        truncated,
    })
}

async fn sample_process(
    process: PerformanceProcess,
    include_network: bool,
) -> RawPerformanceSample {
    let pid = process.pid;
    let ps = ps_process_for_pid(pid).await;
    let rusage = read_process_rusage(pid);
    let network = if include_network {
        sample_network(pid).await.ok()
    } else {
        None
    };
    RawPerformanceSample {
        ps_cpu_percent: ps.as_ref().map_or(0.0, |process| process.cpu_percent),
        ps_memory_resident_bytes: ps
            .as_ref()
            .and_then(|process| process.rss_kb)
            .map(|rss_kb| rss_kb.saturating_mul(1024)),
        process,
        rusage,
        network,
    }
}

async fn list_ps_processes() -> Result<Vec<PsProcess>, AppError> {
    let output = timeout(
        PROCESS_LIST_TIMEOUT,
        Command::new("ps")
            .args(["-axo", "pid=,ppid=,state=,%cpu=,rss=,command="])
            .output(),
    )
    .await
    .map_err(|_| AppError::native("Timed out listing host processes."))?
    .map_err(|error| AppError::native(format!("Unable to list host processes: {error}")))?;
    if !output.status.success() {
        return Err(AppError::native("Unable to list host processes."));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(parse_ps_line)
        .collect())
}

async fn ps_process_for_pid(pid: i32) -> Option<PsProcess> {
    let output = timeout(
        PROCESS_SAMPLE_TIMEOUT,
        Command::new("ps")
            .args([
                "-p",
                &pid.to_string(),
                "-o",
                "pid=,ppid=,state=,%cpu=,rss=,command=",
            ])
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(parse_ps_line)
}

fn parse_ps_line(line: &str) -> Option<PsProcess> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut parts = trimmed.split_whitespace();
    let pid = parts.next()?.parse::<i32>().ok()?;
    let parent_pid = parts.next()?.parse::<i32>().ok()?;
    let state = parts.next()?.to_owned();
    let cpu_percent = parts.next()?.parse::<f64>().unwrap_or(0.0);
    let rss_kb = parts.next()?.parse::<u64>().ok();
    let command = parts.collect::<Vec<_>>().join(" ");
    Some(PsProcess {
        pid,
        parent_pid,
        state,
        cpu_percent,
        rss_kb,
        command,
    })
}

fn app_processes_from_ps(
    udid: &str,
    foreground: Option<&ForegroundProcess>,
    processes: Vec<PsProcess>,
) -> Vec<PerformanceProcess> {
    let foreground_pid = foreground.map(|process| process.process_identifier as i32);
    processes
        .into_iter()
        .filter(|process| {
            process.command.contains(udid)
                && (is_relevant_app_process(&process.command)
                    || foreground_pid == Some(process.pid))
        })
        .map(|process| performance_process(process, foreground))
        .collect()
}

async fn ensure_foreground_process(
    processes: &mut Vec<PerformanceProcess>,
    foreground: &ForegroundProcess,
) {
    let pid = foreground.process_identifier as i32;
    if processes.iter().any(|process| process.pid == pid) {
        return;
    }
    let Some(ps) = ps_process_for_pid(pid).await else {
        return;
    };
    processes.push(performance_process(ps, Some(foreground)));
}

fn performance_process(
    ps: PsProcess,
    foreground: Option<&ForegroundProcess>,
) -> PerformanceProcess {
    let app_path = app_bundle_path_from_command(&ps.command);
    let metadata = app_path.as_deref().and_then(app_metadata);
    let fallback_name = process_name_from_command(&ps.command);
    let is_foreground =
        foreground.is_some_and(|process| process.process_identifier as i32 == ps.pid);
    PerformanceProcess {
        pid: ps.pid,
        parent_pid: ps.parent_pid,
        process: metadata
            .as_ref()
            .and_then(|metadata| metadata.app_name.clone())
            .or_else(|| {
                foreground
                    .filter(|foreground| foreground.process_identifier as i32 == ps.pid)
                    .and_then(|foreground| foreground.app_name.clone())
            })
            .unwrap_or(fallback_name),
        role: process_role(&ps.command),
        state: ps.state,
        app_name: metadata
            .as_ref()
            .and_then(|metadata| metadata.app_name.clone())
            .or_else(|| {
                foreground
                    .filter(|foreground| foreground.process_identifier as i32 == ps.pid)
                    .and_then(|foreground| foreground.app_name.clone())
            }),
        bundle_identifier: metadata
            .as_ref()
            .and_then(|metadata| metadata.bundle_identifier.clone())
            .or_else(|| {
                foreground
                    .filter(|foreground| foreground.process_identifier as i32 == ps.pid)
                    .and_then(|foreground| foreground.bundle_identifier.clone())
            }),
        command: ps.command,
        is_foreground,
    }
}

fn is_relevant_app_process(command: &str) -> bool {
    command.contains(".app/")
        || command.contains(".appex/")
        || command.contains("WebContent")
        || command.contains("UIKitApplication")
}

fn process_role(command: &str) -> String {
    if command.contains(".appex/") {
        "extension".to_owned()
    } else if command.contains("WebContent") {
        "web-content".to_owned()
    } else if command.contains(".app/") {
        "app".to_owned()
    } else {
        "helper".to_owned()
    }
}

fn process_name_from_command(command: &str) -> String {
    let executable = command
        .split_whitespace()
        .next()
        .unwrap_or(command)
        .trim_matches('"');
    Path::new(executable)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("unknown")
        .to_owned()
}

fn app_bundle_path_from_command(command: &str) -> Option<String> {
    let marker = if command.contains(".app/") {
        ".app/"
    } else {
        ".appex/"
    };
    let command = command.trim();
    let end = command.find(marker)? + marker.trim_end_matches('/').len();
    let start = command[..end].find('/').unwrap_or(0);
    Some(command[start..end].trim_matches('"').to_owned())
}

struct AppMetadata {
    app_name: Option<String>,
    bundle_identifier: Option<String>,
}

fn app_metadata(app_path: &str) -> Option<AppMetadata> {
    let plist = plist::Value::from_file(Path::new(app_path).join("Info.plist")).ok()?;
    let dictionary = plist.as_dictionary()?;
    let app_name = string_plist_value(dictionary.get("CFBundleDisplayName"))
        .or_else(|| string_plist_value(dictionary.get("CFBundleName")))
        .or_else(|| {
            Path::new(app_path)
                .file_stem()
                .and_then(|name| name.to_str())
                .map(ToOwned::to_owned)
        });
    let bundle_identifier = string_plist_value(dictionary.get("CFBundleIdentifier"));
    Some(AppMetadata {
        app_name,
        bundle_identifier,
    })
}

fn string_plist_value(value: Option<&plist::Value>) -> Option<String> {
    value
        .and_then(plist::Value::as_string)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn cpu_percent(
    raw: &RawPerformanceSample,
    previous: Option<&RawCounterSnapshot>,
    current: &RawCounterSnapshot,
) -> f64 {
    let Some(previous) = previous else {
        return raw.ps_cpu_percent.max(0.0);
    };
    let (Some(previous_cpu), Some(current_cpu)) = (previous.cpu_time_ns, current.cpu_time_ns)
    else {
        return raw.ps_cpu_percent.max(0.0);
    };
    let wall_ns = current
        .sampled_at
        .saturating_duration_since(previous.sampled_at)
        .as_nanos() as f64;
    if wall_ns <= 0.0 {
        return raw.ps_cpu_percent.max(0.0);
    }
    let cpu_ns = current_cpu.saturating_sub(previous_cpu) as f64;
    round_one_decimal((cpu_ns / wall_ns) * 100.0)
}

fn rate_per_second(
    previous_value: Option<u64>,
    current_value: Option<u64>,
    previous: Option<&RawCounterSnapshot>,
    current: &RawCounterSnapshot,
) -> Option<f64> {
    let previous = previous?;
    let previous_value = previous_value?;
    let current_value = current_value?;
    let elapsed = current
        .timestamp_ms
        .saturating_sub(previous.timestamp_ms)
        .max(1) as f64
        / 1000.0;
    Some((current_value.saturating_sub(previous_value) as f64) / elapsed)
}

fn hang_status(
    tracker: &mut HangTracker,
    display_signal: DisplaySignal,
    now_ms: u64,
    cpu_percent: f64,
) -> HangStatus {
    if display_signal.frame_sequence == 0 {
        return HangStatus {
            state: "unknown".to_owned(),
            stale_ms: None,
            reason: "No display frame signal is available yet.".to_owned(),
        };
    }

    if tracker.last_frame_sequence != display_signal.frame_sequence {
        tracker.last_frame_sequence = display_signal.frame_sequence;
        tracker.last_frame_change_ms = now_ms;
    }
    if tracker.last_frame_change_ms == 0 {
        tracker.last_frame_change_ms = display_signal.last_frame_at_ms.max(now_ms);
    }

    let stale_ms = now_ms.saturating_sub(tracker.last_frame_change_ms);
    if stale_ms >= 5_000 && cpu_percent >= 30.0 {
        HangStatus {
            state: "busy".to_owned(),
            stale_ms: Some(stale_ms),
            reason: "Display frames have not changed while the process is using CPU.".to_owned(),
        }
    } else if stale_ms >= 8_000 && cpu_percent < 5.0 {
        HangStatus {
            state: "quiet".to_owned(),
            stale_ms: Some(stale_ms),
            reason: "Display frames have not changed and the process is mostly idle.".to_owned(),
        }
    } else {
        HangStatus {
            state: "responsive".to_owned(),
            stale_ms: Some(stale_ms),
            reason: "Display frame progress is recent.".to_owned(),
        }
    }
}

fn prune_history(history: &mut VecDeque<PerformanceSample>, now_ms: u64) {
    let retention_start = now_ms.saturating_sub(HISTORY_RETENTION_MS);
    while history
        .front()
        .is_some_and(|sample| sample.timestamp_ms < retention_start)
    {
        history.pop_front();
    }
    while history.len() > HISTORY_MAX_SAMPLES {
        history.pop_front();
    }
}

async fn sample_network(pid: i32) -> Result<NetworkSnapshot, AppError> {
    let (received_bytes, sent_bytes) = sample_network_totals(pid).await.unwrap_or((None, None));
    let output = timeout(
        NETWORK_SAMPLE_TIMEOUT,
        Command::new("lsof")
            .args(["-nP", "-a", "-p", &pid.to_string(), "-iTCP", "-iUDP"])
            .output(),
    )
    .await
    .map_err(|_| AppError::native("Timed out reading process network connections."))?
    .map_err(|error| {
        AppError::native(format!(
            "Unable to read process network connections: {error}"
        ))
    })?;
    if !output.status.success() {
        return Ok(NetworkSnapshot {
            connection_count: 0,
            established_connection_count: 0,
            received_bytes,
            sent_bytes,
            endpoints: Vec::new(),
        });
    }
    let mut connection_count = 0;
    let mut established_connection_count = 0;
    let mut endpoints = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines().skip(1) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        connection_count += 1;
        if trimmed.contains("ESTABLISHED") {
            established_connection_count += 1;
        }
        if endpoints.len() < 8 {
            endpoints.push(compact_lsof_endpoint(trimmed));
        }
    }
    Ok(NetworkSnapshot {
        connection_count,
        established_connection_count,
        received_bytes,
        sent_bytes,
        endpoints,
    })
}

async fn sample_network_totals(pid: i32) -> Result<(Option<u64>, Option<u64>), AppError> {
    let output = timeout(
        NETWORK_TOTALS_TIMEOUT,
        Command::new("nettop")
            .args([
                "-P",
                "-L",
                "1",
                "-x",
                "-J",
                "bytes_in,bytes_out",
                "-p",
                &pid.to_string(),
            ])
            .output(),
    )
    .await
    .map_err(|_| AppError::native("Timed out reading process network totals."))?
    .map_err(|error| AppError::native(format!("Unable to read process network totals: {error}")))?;

    if !output.status.success() {
        return Ok((None, None));
    }

    Ok(parse_nettop_network_totals(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

fn parse_nettop_network_totals(output: &str) -> (Option<u64>, Option<u64>) {
    let mut lines = output.lines().filter(|line| !line.trim().is_empty());
    let Some(header) = lines.next() else {
        return (None, None);
    };
    let columns = split_nettop_csv_line(header);
    let Some(received_index) = columns.iter().position(|column| column == "bytes_in") else {
        return (None, None);
    };
    let Some(sent_index) = columns.iter().position(|column| column == "bytes_out") else {
        return (None, None);
    };

    let mut received_total = 0_u64;
    let mut sent_total = 0_u64;
    let mut saw_received = false;
    let mut saw_sent = false;

    for line in lines {
        let fields = split_nettop_csv_line(line);
        if let Some(value) = fields
            .get(received_index)
            .and_then(|field| parse_nettop_byte_value(field))
        {
            received_total = received_total.saturating_add(value);
            saw_received = true;
        }
        if let Some(value) = fields
            .get(sent_index)
            .and_then(|field| parse_nettop_byte_value(field))
        {
            sent_total = sent_total.saturating_add(value);
            saw_sent = true;
        }
    }

    (
        saw_received.then_some(received_total),
        saw_sent.then_some(sent_total),
    )
}

fn split_nettop_csv_line(line: &str) -> Vec<String> {
    line.split(',')
        .map(|field| field.trim().trim_matches('"').to_owned())
        .collect()
}

fn parse_nettop_byte_value(value: &str) -> Option<u64> {
    let value = value.trim().trim_matches('"').replace('_', "");
    if value.is_empty() || value == "-" {
        return None;
    }
    if let Ok(bytes) = value.parse::<u64>() {
        return Some(bytes);
    }

    let number_end = value
        .char_indices()
        .find_map(|(index, character)| {
            (!character.is_ascii_digit() && character != '.').then_some(index)
        })
        .unwrap_or(value.len());
    if number_end == 0 {
        return None;
    }

    let number = value[..number_end].parse::<f64>().ok()?;
    if !number.is_finite() || number < 0.0 {
        return None;
    }
    let suffix = value[number_end..]
        .trim()
        .trim_end_matches("/s")
        .to_ascii_lowercase();
    let multiplier = match suffix.as_str() {
        "" | "b" | "byte" | "bytes" => 1.0,
        "k" | "kb" | "kib" => 1024.0,
        "m" | "mb" | "mib" => 1024.0 * 1024.0,
        "g" | "gb" | "gib" => 1024.0 * 1024.0 * 1024.0,
        "t" | "tb" | "tib" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    Some((number * multiplier).round() as u64)
}

fn compact_lsof_endpoint(line: &str) -> String {
    line.split_whitespace()
        .skip_while(|part| *part != "TCP" && *part != "UDP")
        .collect::<Vec<_>>()
        .join(" ")
}

fn round_one_decimal(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(target_os = "macos")]
fn read_process_rusage(pid: i32) -> Option<ProcessRUsage> {
    let mut info = RUsageInfoV6::default();
    let result = unsafe {
        proc_pid_rusage(
            pid,
            RUSAGE_INFO_V6,
            &mut info as *mut RUsageInfoV6 as *mut c_void,
        )
    };
    (result == 0).then_some(ProcessRUsage {
        user_time_ns: info.ri_user_time,
        system_time_ns: info.ri_system_time,
        resident_size: info.ri_resident_size,
        phys_footprint: info.ri_phys_footprint,
        lifetime_max_phys_footprint: info.ri_lifetime_max_phys_footprint,
        disk_read_bytes: info.ri_diskio_bytesread,
        disk_write_bytes: info.ri_diskio_byteswritten,
    })
}

#[cfg(not(target_os = "macos"))]
fn read_process_rusage(_pid: i32) -> Option<ProcessRUsage> {
    None
}

#[cfg(target_os = "macos")]
const RUSAGE_INFO_V6: i32 = 6;

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Default)]
struct RUsageInfoV6 {
    ri_uuid: [u8; 16],
    ri_user_time: u64,
    ri_system_time: u64,
    ri_pkg_idle_wkups: u64,
    ri_interrupt_wkups: u64,
    ri_pageins: u64,
    ri_wired_size: u64,
    ri_resident_size: u64,
    ri_phys_footprint: u64,
    ri_proc_start_abstime: u64,
    ri_proc_exit_abstime: u64,
    ri_child_user_time: u64,
    ri_child_system_time: u64,
    ri_child_pkg_idle_wkups: u64,
    ri_child_interrupt_wkups: u64,
    ri_child_pageins: u64,
    ri_child_elapsed_abstime: u64,
    ri_diskio_bytesread: u64,
    ri_diskio_byteswritten: u64,
    ri_cpu_time_qos_default: u64,
    ri_cpu_time_qos_maintenance: u64,
    ri_cpu_time_qos_background: u64,
    ri_cpu_time_qos_utility: u64,
    ri_cpu_time_qos_legacy: u64,
    ri_cpu_time_qos_user_initiated: u64,
    ri_cpu_time_qos_user_interactive: u64,
    ri_billed_system_time: u64,
    ri_serviced_system_time: u64,
    ri_logical_writes: u64,
    ri_lifetime_max_phys_footprint: u64,
    ri_instructions: u64,
    ri_cycles: u64,
    ri_billed_energy: u64,
    ri_serviced_energy: u64,
    ri_interval_max_phys_footprint: u64,
    ri_runnable_time: u64,
    ri_flags: u64,
    ri_user_ptime: u64,
    ri_system_ptime: u64,
    ri_pinstructions: u64,
    ri_pcycles: u64,
    ri_energy_nj: u64,
    ri_penergy_nj: u64,
    ri_secure_time_in_system: u64,
    ri_secure_ptime_in_system: u64,
    ri_neural_footprint: u64,
    ri_lifetime_max_neural_footprint: u64,
    ri_interval_max_neural_footprint: u64,
    ri_reserved: [u64; 9],
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn proc_pid_rusage(pid: i32, flavor: i32, buffer: *mut c_void) -> i32;
}

#[cfg(test)]
mod tests {
    use super::{app_bundle_path_from_command, parse_nettop_network_totals, parse_ps_line};

    #[test]
    fn parse_ps_line_keeps_command_tail() {
        let row = parse_ps_line("123 1 S 4.2 8192 /tmp/My App.app/My App --flag value").unwrap();
        assert_eq!(row.pid, 123);
        assert_eq!(row.parent_pid, 1);
        assert_eq!(row.cpu_percent, 4.2);
        assert_eq!(row.rss_kb, Some(8192));
        assert_eq!(row.command, "/tmp/My App.app/My App --flag value");
    }

    #[test]
    fn app_bundle_path_from_command_extracts_app_bundle() {
        assert_eq!(
            app_bundle_path_from_command("/tmp/Fixture.app/Fixture --args"),
            Some("/tmp/Fixture.app".to_owned())
        );
    }

    #[test]
    fn app_bundle_path_from_command_keeps_spaces_in_bundle_path() {
        assert_eq!(
            app_bundle_path_from_command(
                "/Library/Developer/CoreSimulator/Volumes/iOS 26.4.simruntime/Contents/Resources/RuntimeRoot/Applications/MobileSafari.app/MobileSafari"
            ),
            Some(
                "/Library/Developer/CoreSimulator/Volumes/iOS 26.4.simruntime/Contents/Resources/RuntimeRoot/Applications/MobileSafari.app"
                    .to_owned()
            )
        );
    }

    #[test]
    fn parse_nettop_network_totals_reads_process_csv() {
        let output = ",bytes_in,bytes_out,\nFixture.123,100,240,\nHelper.456,2 KB,1.5 KB,\n";
        assert_eq!(
            parse_nettop_network_totals(output),
            (Some(2148), Some(1776))
        );
    }

    #[test]
    fn parse_nettop_network_totals_reads_extended_csv() {
        let output = "time,,interface,state,bytes_in,bytes_out,\n12:00:00,Fixture.123,,,10,20,\n";
        assert_eq!(parse_nettop_network_totals(output), (Some(10), Some(20)));
    }
}
