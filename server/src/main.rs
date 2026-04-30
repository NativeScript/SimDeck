mod api;
mod auth;
mod config;
mod core_simulator;
mod error;
mod inspector;
mod logging;
mod logs;
mod metrics;
mod native;
mod service;
mod simulators;
mod static_files;
mod transport;

use anyhow::Context;
use api::routes::{router, AppState};
use axum::Router;
use clap::{Parser, Subcommand, ValueEnum};
use config::Config;
use inspector::InspectorHub;
use logs::LogRegistry;
use metrics::counters::Metrics;
use native::bridge::{NativeBridge, NativeInputSession};
use native::ffi;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use simulators::registry::SessionRegistry;
use std::collections::hash_map::DefaultHasher;
use std::env;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::{self, IsTerminal, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{info, warn};

const RECOVERABLE_RESTART_EXIT_CODE: i32 = 75;
const RESTART_ON_CORE_SIMULATOR_MISMATCH_ENV: &str = "SIMDECK_RESTART_ON_CORE_SIMULATOR_MISMATCH";
const SERVER_FD_RESTART_THRESHOLD: usize = 4096;
const SERVER_HEALTH_WATCHDOG_INITIAL_DELAY: Duration = Duration::from_secs(15);
const SERVER_HEALTH_WATCHDOG_INTERVAL: Duration = Duration::from_secs(5);
const SERVER_HEALTH_WATCHDOG_PROBE_TIMEOUT: Duration = Duration::from_secs(3);
const SERVER_HEALTH_WATCHDOG_STALE_HEARTBEAT: Duration = Duration::from_secs(10);
const SERVER_HEALTH_WATCHDOG_FAILURE_THRESHOLD: usize = 3;

#[derive(Parser)]
#[command(name = "simdeck")]
#[command(bin_name = "simdeck")]
#[command(about = "Project-local iOS Simulator devtool")]
#[command(
    override_usage = "simdeck [SIMULATOR_NAME_OR_UDID]\n       simdeck [-d|--detached]\n       simdeck [-k|--kill]\n       simdeck [-r|--restart]\n       simdeck <COMMAND> [OPTIONS]"
)]
#[command(
    after_help = "Run without a subcommand to start a foreground workspace daemon. Pass a simulator name or UDID as the only argument to select it in the UI. Use -d/--detached, -k/--kill, or -r/--restart for shorthand daemon lifecycle commands."
)]
#[command(version)]
struct Cli {
    #[arg(long, global = true, hide = true)]
    server_url: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Ui {
        #[arg(long, default_value_t = 4310)]
        port: u16,
        #[arg(long, default_value_t = IpAddr::V4(Ipv4Addr::LOCALHOST))]
        bind: IpAddr,
        #[arg(long)]
        advertise_host: Option<String>,
        #[arg(long)]
        client_root: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = VideoCodecMode::H264Software)]
        video_codec: VideoCodecMode,
        #[arg(long)]
        low_latency: bool,
        #[arg(long)]
        open: bool,
    },
    Daemon {
        #[command(subcommand)]
        command: DaemonCommand,
    },
    #[command(hide = true)]
    Serve {
        #[arg(long, default_value_t = 4310)]
        port: u16,
        #[arg(long, default_value_t = IpAddr::V4(Ipv4Addr::LOCALHOST))]
        bind: IpAddr,
        #[arg(long)]
        advertise_host: Option<String>,
        #[arg(long)]
        client_root: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = VideoCodecMode::H264Software)]
        video_codec: VideoCodecMode,
        #[arg(long)]
        low_latency: bool,
        #[arg(long)]
        access_token: Option<String>,
        #[arg(long)]
        pairing_code: Option<String>,
    },
    Service {
        #[command(subcommand)]
        command: ServiceCommand,
    },
    #[command(name = "core-simulator", visible_alias = "simctl-service")]
    CoreSimulator {
        #[command(subcommand)]
        command: CoreSimulatorCommand,
    },
    List,
    Boot {
        udid: String,
    },
    Shutdown {
        udid: String,
    },
    OpenUrl {
        udid: String,
        url: String,
    },
    Launch {
        udid: String,
        bundle_id: String,
    },
    ToggleAppearance {
        udid: String,
    },
    Erase {
        udid: String,
    },
    Install {
        udid: String,
        app_path: String,
    },
    Uninstall {
        udid: String,
        bundle_id: String,
    },
    Pasteboard {
        #[command(subcommand)]
        command: PasteboardCommand,
    },
    Logs {
        udid: String,
        #[arg(long, default_value_t = 30.0)]
        seconds: f64,
        #[arg(long, default_value_t = 200)]
        limit: usize,
    },
    Screenshot {
        udid: String,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long)]
        stdout: bool,
    },
    #[command(name = "describe")]
    DescribeUi {
        udid: String,
        #[arg(long, value_parser = parse_point)]
        point: Option<(f64, f64)>,
        #[arg(long, value_enum, default_value_t = DescribeUiFormat::Json)]
        format: DescribeUiFormat,
        #[arg(long, value_enum, default_value_t = DescribeUiSource::Auto)]
        source: DescribeUiSource,
        #[arg(long)]
        max_depth: Option<usize>,
        #[arg(long)]
        include_hidden: bool,
        #[arg(long)]
        direct: bool,
    },
    Touch {
        udid: String,
        x: f64,
        y: f64,
        #[arg(long, default_value = "began")]
        phase: String,
        #[arg(long)]
        normalized: bool,
        #[arg(long)]
        down: bool,
        #[arg(long)]
        up: bool,
        #[arg(long, default_value_t = 100)]
        delay_ms: u64,
    },
    Tap {
        udid: String,
        x: Option<f64>,
        y: Option<f64>,
        #[arg(long)]
        id: Option<String>,
        #[arg(long)]
        label: Option<String>,
        #[arg(long)]
        value: Option<String>,
        #[arg(long)]
        element_type: Option<String>,
        #[arg(long, default_value_t = 0)]
        wait_timeout_ms: u64,
        #[arg(long, default_value_t = 100)]
        poll_interval_ms: u64,
        #[arg(long)]
        normalized: bool,
        #[arg(long, default_value_t = 60)]
        duration_ms: u64,
        #[arg(long, default_value_t = 0)]
        pre_delay_ms: u64,
        #[arg(long, default_value_t = 0)]
        post_delay_ms: u64,
    },
    Swipe {
        udid: String,
        start_x: f64,
        start_y: f64,
        end_x: f64,
        end_y: f64,
        #[arg(long)]
        normalized: bool,
        #[arg(long, default_value_t = 350)]
        duration_ms: u64,
        #[arg(long, default_value_t = 12)]
        steps: u32,
        #[arg(long, default_value_t = 0)]
        pre_delay_ms: u64,
        #[arg(long, default_value_t = 0)]
        post_delay_ms: u64,
    },
    Gesture {
        udid: String,
        preset: String,
        #[arg(long)]
        screen_width: Option<f64>,
        #[arg(long)]
        screen_height: Option<f64>,
        #[arg(long)]
        normalized: bool,
        #[arg(long)]
        duration_ms: Option<u64>,
        #[arg(long)]
        delta: Option<f64>,
        #[arg(long, default_value_t = 0)]
        pre_delay_ms: u64,
        #[arg(long, default_value_t = 0)]
        post_delay_ms: u64,
    },
    Pinch {
        udid: String,
        center_x: Option<f64>,
        center_y: Option<f64>,
        #[arg(long, default_value_t = 160.0)]
        start_distance: f64,
        #[arg(long, default_value_t = 80.0)]
        end_distance: f64,
        #[arg(long, default_value_t = 0.0)]
        angle_degrees: f64,
        #[arg(long)]
        normalized: bool,
        #[arg(long, default_value_t = 450)]
        duration_ms: u64,
        #[arg(long, default_value_t = 12)]
        steps: u32,
    },
    RotateGesture {
        udid: String,
        center_x: Option<f64>,
        center_y: Option<f64>,
        #[arg(long, default_value_t = 100.0)]
        radius: f64,
        #[arg(long, default_value_t = 90.0)]
        degrees: f64,
        #[arg(long)]
        normalized: bool,
        #[arg(long, default_value_t = 500)]
        duration_ms: u64,
        #[arg(long, default_value_t = 12)]
        steps: u32,
    },
    Key {
        udid: String,
        key: String,
        #[arg(long, default_value_t = 0)]
        modifiers: u32,
        #[arg(long, default_value_t = 0)]
        duration_ms: u64,
        #[arg(long, default_value_t = 0)]
        pre_delay_ms: u64,
        #[arg(long, default_value_t = 0)]
        post_delay_ms: u64,
    },
    KeySequence {
        udid: String,
        #[arg(long = "keycodes", alias = "keys")]
        keycodes: String,
        #[arg(long, default_value_t = 100)]
        delay_ms: u64,
    },
    KeyCombo {
        udid: String,
        #[arg(long)]
        modifiers: String,
        #[arg(long)]
        key: String,
    },
    Type {
        udid: String,
        text: Option<String>,
        #[arg(long)]
        stdin: bool,
        #[arg(long)]
        file: Option<PathBuf>,
        #[arg(long, default_value_t = 12)]
        delay_ms: u64,
    },
    Button {
        udid: String,
        button: String,
        #[arg(long, default_value_t = 0)]
        duration_ms: u32,
    },
    Batch {
        udid: String,
        #[arg(long = "step")]
        steps: Vec<String>,
        #[arg(long)]
        file: Option<PathBuf>,
        #[arg(long)]
        stdin: bool,
        #[arg(long)]
        continue_on_error: bool,
    },
    DismissKeyboard {
        udid: String,
    },
    Home {
        udid: String,
    },
    AppSwitcher {
        udid: String,
    },
    RotateLeft {
        udid: String,
    },
    RotateRight {
        udid: String,
    },
    ChromeProfile {
        udid: String,
    },
}

#[derive(Subcommand)]
enum DaemonCommand {
    Start {
        #[arg(long, default_value_t = 4310)]
        port: u16,
        #[arg(long, default_value_t = IpAddr::V4(Ipv4Addr::LOCALHOST))]
        bind: IpAddr,
        #[arg(long)]
        advertise_host: Option<String>,
        #[arg(long)]
        client_root: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = VideoCodecMode::H264Software)]
        video_codec: VideoCodecMode,
        #[arg(long)]
        low_latency: bool,
    },
    Stop,
    Status,
    #[command(hide = true)]
    Run {
        #[arg(long)]
        project_root: PathBuf,
        #[arg(long)]
        metadata_path: PathBuf,
        #[arg(long)]
        port: u16,
        #[arg(long, default_value_t = IpAddr::V4(Ipv4Addr::LOCALHOST))]
        bind: IpAddr,
        #[arg(long)]
        advertise_host: Option<String>,
        #[arg(long)]
        client_root: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = VideoCodecMode::H264Software)]
        video_codec: VideoCodecMode,
        #[arg(long)]
        low_latency: bool,
        #[arg(long)]
        access_token: String,
        #[arg(long)]
        pairing_code: Option<String>,
    },
}

#[derive(Subcommand)]
enum ServiceCommand {
    On {
        #[arg(long, default_value_t = 4310)]
        port: u16,
        #[arg(long, default_value_t = IpAddr::V4(Ipv4Addr::LOCALHOST))]
        bind: IpAddr,
        #[arg(long)]
        advertise_host: Option<String>,
        #[arg(long)]
        client_root: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = VideoCodecMode::H264Software)]
        video_codec: VideoCodecMode,
        #[arg(long)]
        low_latency: bool,
        #[arg(long)]
        access_token: Option<String>,
    },
    Restart {
        #[arg(long, default_value_t = 4310)]
        port: u16,
        #[arg(long, default_value_t = IpAddr::V4(Ipv4Addr::LOCALHOST))]
        bind: IpAddr,
        #[arg(long)]
        advertise_host: Option<String>,
        #[arg(long)]
        client_root: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = VideoCodecMode::H264Software)]
        video_codec: VideoCodecMode,
        #[arg(long)]
        low_latency: bool,
        #[arg(long)]
        access_token: Option<String>,
    },
    Off,
}

#[derive(Subcommand)]
enum CoreSimulatorCommand {
    Start,
    Shutdown,
    Restart,
}

#[derive(Subcommand)]
enum PasteboardCommand {
    Get {
        udid: String,
    },
    Set {
        udid: String,
        text: Option<String>,
        #[arg(long)]
        stdin: bool,
        #[arg(long)]
        file: Option<PathBuf>,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum VideoCodecMode {
    Hevc,
    H264,
    H264Software,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum DescribeUiFormat {
    Json,
    CompactJson,
    Agent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum DescribeUiSource {
    Auto,
    Nativescript,
    ReactNative,
    Uikit,
    NativeAx,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DaemonMetadata {
    project_root: PathBuf,
    pid: u32,
    http_url: String,
    access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pairing_code: Option<String>,
    binary_path: PathBuf,
    started_at: u64,
}

#[derive(Clone, Debug)]
struct DaemonLaunchOptions {
    port: u16,
    bind: IpAddr,
    advertise_host: Option<String>,
    client_root: Option<PathBuf>,
    video_codec: VideoCodecMode,
    low_latency: bool,
}

impl VideoCodecMode {
    fn as_env_value(self) -> &'static str {
        match self {
            Self::Hevc => "hevc",
            Self::H264 => "h264",
            Self::H264Software => "h264-software",
        }
    }
}

fn command_service_url(explicit: Option<String>) -> anyhow::Result<String> {
    if let Some(url) = explicit
        .or_else(|| env::var("SIMDECK_SERVER_URL").ok())
        .filter(|value| !value.trim().is_empty())
    {
        return Ok(url);
    }
    Ok(ensure_project_daemon(DaemonLaunchOptions::default())?.http_url)
}

impl Default for DaemonLaunchOptions {
    fn default() -> Self {
        Self {
            port: 4310,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            advertise_host: None,
            client_root: None,
            video_codec: VideoCodecMode::H264Software,
            low_latency: false,
        }
    }
}

fn ensure_project_daemon(options: DaemonLaunchOptions) -> anyhow::Result<DaemonMetadata> {
    Ok(ensure_project_daemon_with_status(options)?.0)
}

fn ensure_project_daemon_with_status(
    options: DaemonLaunchOptions,
) -> anyhow::Result<(DaemonMetadata, bool)> {
    if let Some(metadata) = read_daemon_metadata().ok().flatten() {
        if daemon_is_healthy(&metadata) {
            return Ok((metadata, false));
        }
    }
    Ok((start_project_daemon(options)?, true))
}

fn start_project_daemon(options: DaemonLaunchOptions) -> anyhow::Result<DaemonMetadata> {
    let project_root = project_root()?;
    let metadata_path = daemon_metadata_path_for_root(&project_root)?;
    let port = choose_daemon_port(options.port)?;
    let access_token = auth::generate_access_token();
    let pairing_code = auth::generate_pairing_code();
    let executable = env::current_exe().context("resolve simdeck executable")?;
    let mut args = vec![
        "daemon".to_owned(),
        "run".to_owned(),
        "--project-root".to_owned(),
        project_root.to_string_lossy().into_owned(),
        "--metadata-path".to_owned(),
        metadata_path.to_string_lossy().into_owned(),
        "--port".to_owned(),
        port.to_string(),
        "--bind".to_owned(),
        options.bind.to_string(),
        "--access-token".to_owned(),
        access_token.clone(),
        "--pairing-code".to_owned(),
        pairing_code.clone(),
        "--video-codec".to_owned(),
        options.video_codec.as_env_value().to_owned(),
    ];
    if options.low_latency {
        args.push("--low-latency".to_owned());
    }
    if let Some(advertise_host) = options.advertise_host {
        args.push("--advertise-host".to_owned());
        args.push(advertise_host);
    }
    if let Some(client_root) = options.client_root {
        args.push("--client-root".to_owned());
        args.push(client_root.to_string_lossy().into_owned());
    }

    let child = ProcessCommand::new(&executable)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("start project SimDeck daemon")?;

    let metadata = DaemonMetadata {
        project_root,
        pid: child.id(),
        http_url: format!("http://127.0.0.1:{port}"),
        access_token,
        pairing_code: Some(pairing_code),
        binary_path: executable,
        started_at: now_secs(),
    };
    write_daemon_metadata(&metadata)?;
    wait_for_daemon(&metadata, Duration::from_secs(15))?;
    Ok(metadata)
}

fn stop_project_daemon() -> anyhow::Result<()> {
    let Some(metadata) = read_daemon_metadata()? else {
        println_json(&serde_json::json!({ "ok": true, "running": false }))?;
        return Ok(());
    };
    terminate_daemon_metadata(&metadata)?;
    println_json(&serde_json::json!({
        "ok": true,
        "running": false,
        "pid": metadata.pid,
        "killedPid": metadata.pid
    }))
}

fn terminate_daemon_metadata(metadata: &DaemonMetadata) -> anyhow::Result<()> {
    let _ = ProcessCommand::new("kill")
        .arg(metadata.pid.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    wait_for_process_exit(metadata.pid, Duration::from_secs(3));
    let _ = fs::remove_file(daemon_metadata_path_for_root(&metadata.project_root)?);
    Ok(())
}

fn wait_for_process_exit(pid: u32, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !process_exists(pid) {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn process_exists(pid: u32) -> bool {
    ProcessCommand::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn remove_daemon_metadata_if_current(root: &Path, pid: u32) -> anyhow::Result<()> {
    let path = daemon_metadata_path_for_root(root)?;
    let should_remove = fs::read_to_string(&path)
        .ok()
        .and_then(|data| serde_json::from_str::<DaemonMetadata>(&data).ok())
        .is_some_and(|metadata| metadata.pid == pid);
    if should_remove {
        let _ = fs::remove_file(path);
    }
    Ok(())
}

fn daemon_status() -> anyhow::Result<()> {
    let metadata = read_daemon_metadata()?;
    let running = metadata.as_ref().is_some_and(daemon_is_healthy);
    println_json(&serde_json::json!({ "running": running, "daemon": metadata }))
}

fn print_daemon_start_result(metadata: &DaemonMetadata, started: bool) -> anyhow::Result<()> {
    println_json(&serde_json::json!({
        "ok": true,
        "projectRoot": metadata.project_root,
        "pid": metadata.pid,
        "url": metadata.http_url,
        "pairingCode": metadata.pairing_code,
        "started": started
    }))
}

fn wait_for_daemon(metadata: &DaemonMetadata, timeout: Duration) -> anyhow::Result<()> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if daemon_is_healthy(metadata) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    anyhow::bail!(
        "Timed out waiting for SimDeck daemon at {}",
        metadata.http_url
    )
}

fn daemon_is_healthy(metadata: &DaemonMetadata) -> bool {
    http_get_json(&metadata.http_url, "/api/health").is_ok()
}

fn read_daemon_metadata() -> anyhow::Result<Option<DaemonMetadata>> {
    let path = daemon_metadata_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let data = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    Ok(Some(serde_json::from_str(&data).with_context(|| {
        format!("parse daemon metadata {}", path.display())
    })?))
}

fn write_daemon_metadata(metadata: &DaemonMetadata) -> anyhow::Result<()> {
    let path = daemon_metadata_path_for_root(&metadata.project_root)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, serde_json::to_vec_pretty(metadata)?)
        .with_context(|| format!("write {}", path.display()))
}

fn daemon_metadata_path() -> anyhow::Result<PathBuf> {
    daemon_metadata_path_for_root(&project_root()?)
}

fn daemon_metadata_path_for_root(root: &Path) -> anyhow::Result<PathBuf> {
    let mut hasher = DefaultHasher::new();
    root.to_string_lossy().hash(&mut hasher);
    Ok(env::temp_dir()
        .join("simdeck")
        .join(format!("{:016x}.json", hasher.finish())))
}

fn project_root() -> anyhow::Result<PathBuf> {
    let mut current = env::current_dir().context("resolve current directory")?;
    loop {
        if current.join(".simdeck").exists()
            || current.join(".git").exists()
            || current.join("package.json").exists()
            || current.join("xcodeproj").exists()
        {
            return Ok(current);
        }
        if !current.pop() {
            return env::current_dir().context("resolve current directory");
        }
    }
}

fn choose_daemon_port(preferred: u16) -> anyhow::Result<u16> {
    let start = preferred.max(1024);
    for port in start..start.saturating_add(200) {
        if port_available(port) && port_available(port.saturating_add(1)) {
            return Ok(port);
        }
    }
    anyhow::bail!("No available SimDeck daemon port near {preferred}")
}

fn port_available(port: u16) -> bool {
    TcpListener::bind((Ipv4Addr::LOCALHOST, port)).is_ok()
}

fn open_browser(url: &str) -> anyhow::Result<()> {
    ProcessCommand::new("open")
        .arg(url)
        .status()
        .context("open SimDeck UI")?;
    Ok(())
}

enum NoCommandAction {
    Foreground(Option<String>),
    Detached,
    Kill,
    Restart,
}

fn no_command_action_from_args() -> Option<NoCommandAction> {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.as_slice() {
        [] => Some(NoCommandAction::Foreground(None)),
        [flag] if flag == "-d" || flag == "--detached" => Some(NoCommandAction::Detached),
        [flag] if flag == "-k" || flag == "--kill" => Some(NoCommandAction::Kill),
        [flag] if flag == "-r" || flag == "--restart" => Some(NoCommandAction::Restart),
        [selector] if !selector.starts_with('-') && !is_known_command(selector) => {
            Some(NoCommandAction::Foreground(Some(selector.clone())))
        }
        _ => None,
    }
}

fn is_known_command(value: &str) -> bool {
    matches!(
        value,
        "ui" | "daemon"
            | "service"
            | "core-simulator"
            | "simctl-service"
            | "list"
            | "boot"
            | "shutdown"
            | "open-url"
            | "launch"
            | "toggle-appearance"
            | "erase"
            | "install"
            | "uninstall"
            | "pasteboard"
            | "logs"
            | "screenshot"
            | "describe"
            | "touch"
            | "tap"
            | "swipe"
            | "gesture"
            | "pinch"
            | "rotate-gesture"
            | "key"
            | "key-sequence"
            | "key-combo"
            | "type"
            | "button"
            | "batch"
            | "dismiss-keyboard"
            | "home"
            | "app-switcher"
            | "rotate-left"
            | "rotate-right"
            | "chrome-profile"
            | "help"
    )
}

fn run_no_command_action(action: NoCommandAction) -> anyhow::Result<()> {
    match action {
        NoCommandAction::Foreground(selector) => run_foreground_ui(selector),
        NoCommandAction::Detached => start_detached_daemon(DaemonLaunchOptions::default()),
        NoCommandAction::Kill => stop_project_daemon(),
        NoCommandAction::Restart => restart_detached_daemon(DaemonLaunchOptions::default()),
    }
}

fn start_detached_daemon(options: DaemonLaunchOptions) -> anyhow::Result<()> {
    let (metadata, started) = ensure_project_daemon_with_status(options)?;
    print_daemon_start_result(&metadata, started)
}

fn restart_detached_daemon(options: DaemonLaunchOptions) -> anyhow::Result<()> {
    if let Some(metadata) = read_daemon_metadata()? {
        terminate_daemon_metadata(&metadata)?;
    }
    start_detached_daemon(options)
}

fn run_foreground_ui(selector: Option<String>) -> anyhow::Result<()> {
    if let Some(metadata) = read_daemon_metadata().ok().flatten() {
        if daemon_is_healthy(&metadata) {
            terminate_daemon_metadata(&metadata)?;
        }
    }

    let project_root = project_root()?;
    let port = choose_daemon_port(4310)?;
    let bind = IpAddr::V4(Ipv4Addr::UNSPECIFIED);
    let video_codec = VideoCodecMode::H264Software;
    let low_latency = false;
    let advertise_host = detect_lan_ip()
        .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST))
        .to_string();
    let access_token = auth::generate_access_token();
    let pairing_code = auth::generate_pairing_code();
    let executable = env::current_exe().context("resolve simdeck executable")?;
    let metadata = DaemonMetadata {
        project_root: project_root.clone(),
        pid: std::process::id(),
        http_url: format!("http://127.0.0.1:{port}"),
        access_token: access_token.clone(),
        pairing_code: Some(pairing_code.clone()),
        binary_path: executable,
        started_at: now_secs(),
    };
    write_daemon_metadata(&metadata)?;

    let local_url = ui_url("127.0.0.1", port, selector.as_deref());
    let network_url = ui_url(&advertise_host, port, selector.as_deref());
    println!("🚀 SimDeck is ready");
    println!();
    println!("{:>12}   {local_url}", "Local:");
    println!("{:>12}   {network_url}", "Network:");
    println!("{:>12}   {}", "Pair:", format_pairing_code(&pairing_code));
    println!();
    println!("q or ^C to stop server");
    let _ = io::stdout().flush();

    let result = serve_with_appkit(
        port,
        bind,
        Some(advertise_host),
        None,
        video_codec,
        low_latency,
        Some(access_token),
        Some(pairing_code),
    );
    let _ = remove_daemon_metadata_if_current(&project_root, std::process::id());
    result
}

fn detect_lan_ip() -> Option<IpAddr> {
    for target in ["8.8.8.8:80", "1.1.1.1:80"] {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
        if socket.connect(target).is_err() {
            continue;
        }
        let ip = socket.local_addr().ok()?.ip();
        if !ip.is_loopback() && !ip.is_unspecified() {
            return Some(ip);
        }
    }
    None
}

fn ui_url(host: &str, port: u16, selector: Option<&str>) -> String {
    let mut url = format!("http://{host}:{port}");
    if let Some(selector) = selector.filter(|value| !value.trim().is_empty()) {
        url.push_str(&format!("/?device={}", percent_encode(selector.trim())));
    }
    url
}

fn format_pairing_code(pairing_code: &str) -> String {
    if pairing_code.len() == 6 {
        format!("{} {}", &pairing_code[..3], &pairing_code[3..])
    } else {
        pairing_code.to_owned()
    }
}

fn percent_encode(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(*byte as char);
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

fn main() -> anyhow::Result<()> {
    logging::init();
    if let Some(action) = no_command_action_from_args() {
        return run_no_command_action(action);
    }

    let cli = Cli::parse();
    let explicit_server_url = cli.server_url.clone();
    let service_url = explicit_server_url
        .clone()
        .or_else(|| env::var("SIMDECK_SERVER_URL").ok())
        .filter(|value| !value.trim().is_empty());
    let bridge = NativeBridge;

    match cli.command {
        Command::Ui {
            port,
            bind,
            advertise_host,
            client_root,
            video_codec,
            low_latency,
            open,
        } => {
            let (metadata, started) = ensure_project_daemon_with_status(DaemonLaunchOptions {
                port,
                bind,
                advertise_host,
                client_root,
                video_codec,
                low_latency,
            })?;
            if open {
                open_browser(&metadata.http_url)?;
            }
            print_daemon_start_result(&metadata, started)?;
            Ok(())
        }
        Command::Daemon { command } => match command {
            DaemonCommand::Start {
                port,
                bind,
                advertise_host,
                client_root,
                video_codec,
                low_latency,
            } => {
                let (metadata, started) = ensure_project_daemon_with_status(DaemonLaunchOptions {
                    port,
                    bind,
                    advertise_host,
                    client_root,
                    video_codec,
                    low_latency,
                })?;
                print_daemon_start_result(&metadata, started)
            }
            DaemonCommand::Stop => stop_project_daemon(),
            DaemonCommand::Status => daemon_status(),
            DaemonCommand::Run {
                project_root,
                metadata_path,
                port,
                bind,
                advertise_host,
                client_root,
                video_codec,
                low_latency,
                access_token,
                pairing_code,
            } => {
                env::set_current_dir(&project_root).with_context(|| {
                    format!("set daemon project root to {}", project_root.display())
                })?;
                write_daemon_metadata(&DaemonMetadata {
                    project_root,
                    pid: std::process::id(),
                    http_url: format!("http://127.0.0.1:{port}"),
                    access_token: access_token.clone(),
                    pairing_code: pairing_code.clone(),
                    binary_path: env::current_exe().context("resolve daemon executable")?,
                    started_at: now_secs(),
                })?;
                let result = serve_with_appkit(
                    port,
                    bind,
                    advertise_host,
                    client_root,
                    video_codec,
                    low_latency,
                    Some(access_token),
                    pairing_code,
                );
                let _ = fs::remove_file(metadata_path);
                result
            }
        },
        Command::Serve {
            port,
            bind,
            advertise_host,
            client_root,
            video_codec,
            low_latency,
            access_token,
            pairing_code,
        } => serve_with_appkit(
            port,
            bind,
            advertise_host,
            client_root,
            video_codec,
            low_latency,
            access_token,
            pairing_code,
        ),
        Command::Service { command } => match command {
            ServiceCommand::On {
                port,
                bind,
                advertise_host,
                client_root,
                video_codec,
                low_latency,
                access_token,
            } => service::enable(ServiceOptions {
                port,
                bind,
                advertise_host,
                client_root,
                video_codec,
                low_latency,
                access_token,
                pairing_code: None,
            }),
            ServiceCommand::Restart {
                port,
                bind,
                advertise_host,
                client_root,
                video_codec,
                low_latency,
                access_token,
            } => service::restart(ServiceOptions {
                port,
                bind,
                advertise_host,
                client_root,
                video_codec,
                low_latency,
                access_token,
                pairing_code: None,
            }),
            ServiceCommand::Off => service::disable(),
        },
        Command::CoreSimulator { command } => match command {
            CoreSimulatorCommand::Start => core_simulator::start(),
            CoreSimulatorCommand::Shutdown => core_simulator::shutdown(),
            CoreSimulatorCommand::Restart => core_simulator::restart(),
        },
        Command::List => {
            let service_url = command_service_url(explicit_server_url.clone())?;
            let simulators = service_get_json(&service_url, "/api/simulators")?
                .get("simulators")
                .cloned()
                .unwrap_or(Value::Array(Vec::new()));
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({ "simulators": simulators }))?
            );
            Ok(())
        }
        Command::Boot { udid } => {
            let service_url = command_service_url(explicit_server_url.clone())?;
            service_post_ok(&service_url, &udid, "boot", &Value::Null)?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({ "ok": true, "udid": udid, "action": "boot" })
                )?
            );
            Ok(())
        }
        Command::Shutdown { udid } => {
            let service_url = command_service_url(explicit_server_url.clone())?;
            service_post_ok(&service_url, &udid, "shutdown", &Value::Null)?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({ "ok": true, "udid": udid, "action": "shutdown" })
                )?
            );
            Ok(())
        }
        Command::OpenUrl { udid, url } => {
            let service_url = command_service_url(explicit_server_url.clone())?;
            service_open_url(&service_url, &udid, &url)?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({ "ok": true, "udid": udid, "url": url })
                )?
            );
            Ok(())
        }
        Command::Launch { udid, bundle_id } => {
            let service_url = command_service_url(explicit_server_url.clone())?;
            service_launch(&service_url, &udid, &bundle_id)?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({ "ok": true, "udid": udid, "bundleId": bundle_id })
                )?
            );
            Ok(())
        }
        Command::ToggleAppearance { udid } => {
            let service_url = command_service_url(explicit_server_url.clone())?;
            service_post_ok(&service_url, &udid, "toggle-appearance", &Value::Null)?;
            println_json(
                &serde_json::json!({ "ok": true, "udid": udid, "action": "toggle-appearance" }),
            )?;
            Ok(())
        }
        Command::Erase { udid } => {
            let service_url = command_service_url(explicit_server_url.clone())?;
            service_post_ok(&service_url, &udid, "erase", &Value::Null)?;
            println_json(&serde_json::json!({ "ok": true, "udid": udid, "action": "erase" }))?;
            Ok(())
        }
        Command::Install { udid, app_path } => {
            let service_url = command_service_url(explicit_server_url.clone())?;
            service_post_ok(
                &service_url,
                &udid,
                "install",
                &serde_json::json!({ "appPath": app_path }),
            )?;
            println_json(
                &serde_json::json!({ "ok": true, "udid": udid, "action": "install", "appPath": app_path }),
            )?;
            Ok(())
        }
        Command::Uninstall { udid, bundle_id } => {
            let service_url = command_service_url(explicit_server_url.clone())?;
            service_post_ok(
                &service_url,
                &udid,
                "uninstall",
                &serde_json::json!({ "bundleId": bundle_id }),
            )?;
            println_json(
                &serde_json::json!({ "ok": true, "udid": udid, "action": "uninstall", "bundleId": bundle_id }),
            )?;
            Ok(())
        }
        Command::Pasteboard { command } => match command {
            PasteboardCommand::Get { udid } => {
                let service_url = command_service_url(explicit_server_url.clone())?;
                let text = service_get_json(
                    &service_url,
                    &format!("/api/simulators/{}/pasteboard", url_path_component(&udid)),
                )?
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
                println_json(&serde_json::json!({ "udid": udid, "text": text }))?;
                Ok(())
            }
            PasteboardCommand::Set {
                udid,
                text,
                stdin,
                file,
            } => {
                let service_url = command_service_url(explicit_server_url.clone())?;
                let text = read_text_input(text, stdin, file)?;
                service_post_ok(
                    &service_url,
                    &udid,
                    "pasteboard",
                    &serde_json::json!({ "text": text }),
                )?;
                println_json(
                    &serde_json::json!({ "ok": true, "udid": udid, "action": "pasteboard-set" }),
                )?;
                Ok(())
            }
        },
        Command::Logs {
            udid,
            seconds,
            limit,
        } => {
            let service_url = command_service_url(explicit_server_url.clone())?;
            let filters = native::bridge::LogFilters::new(Vec::new(), Vec::new(), String::new());
            let _ = filters;
            let entries = service_get_json(
                &service_url,
                &format!(
                    "/api/simulators/{}/logs?seconds={seconds}&limit={limit}",
                    url_path_component(&udid)
                ),
            )?
            .get("entries")
            .cloned()
            .unwrap_or(Value::Array(Vec::new()));
            println_json(&serde_json::json!({ "entries": entries }))?;
            Ok(())
        }
        Command::Screenshot {
            udid,
            output,
            stdout,
        } => {
            let service_url = command_service_url(explicit_server_url.clone())?;
            let png = service_get_bytes(
                &service_url,
                &format!(
                    "/api/simulators/{}/screenshot.png",
                    url_path_component(&udid)
                ),
            )?;
            if stdout {
                io::stdout().write_all(&png)?;
            } else {
                let output = output.unwrap_or_else(|| default_screenshot_path(&udid));
                if let Some(parent) = output
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&output, &png)?;
                println_json(
                    &serde_json::json!({ "ok": true, "udid": udid, "action": "screenshot", "output": output }),
                )?;
            }
            Ok(())
        }
        Command::DescribeUi {
            udid,
            point,
            format,
            source,
            max_depth,
            include_hidden,
            direct,
        } => {
            let service_url = command_service_url(explicit_server_url.clone())?;
            let snapshot = describe_ui_snapshot(
                &bridge,
                &udid,
                point,
                source,
                max_depth,
                include_hidden,
                direct,
                &service_url,
            )?;
            print_describe_ui(&snapshot, format)?;
            Ok(())
        }
        Command::Touch {
            udid,
            x,
            y,
            phase,
            normalized,
            down,
            up,
            delay_ms,
        } => {
            if let Some(server_url) = service_url.as_deref().filter(|_| normalized) {
                if down || up {
                    let mut events = Vec::new();
                    if down {
                        events.push(service_touch_event(
                            x,
                            y,
                            "began",
                            if up { delay_ms } else { 0 },
                        ));
                    }
                    if up {
                        events.push(service_touch_event(x, y, "ended", 0));
                    }
                    if !events.is_empty() {
                        service_touch_sequence(server_url, &udid, events)?;
                    }
                } else {
                    service_touch(server_url, &udid, x, y, &phase)?;
                }
            } else {
                let (x, y) = resolve_touch_point(&bridge, &udid, x, y, normalized)?;
                if down || up {
                    let input = bridge.create_input_session(&udid)?;
                    if down {
                        input.send_touch(x, y, "began")?;
                    }
                    if down && up {
                        std::thread::sleep(Duration::from_millis(delay_ms));
                    }
                    if up {
                        input.send_touch(x, y, "ended")?;
                    }
                } else {
                    bridge.send_touch(&udid, x, y, &phase)?;
                }
            }
            println_json(&serde_json::json!({ "ok": true, "udid": udid, "action": "touch" }))?;
            Ok(())
        }
        Command::Tap {
            udid,
            x,
            y,
            id,
            label,
            value,
            element_type,
            wait_timeout_ms,
            poll_interval_ms,
            normalized,
            duration_ms,
            pre_delay_ms,
            post_delay_ms,
        } => {
            if let (Some(server_url), Some(x), Some(y), true, None, None, None, None) = (
                service_url.as_deref(),
                x,
                y,
                normalized,
                id.as_ref(),
                label.as_ref(),
                value.as_ref(),
                element_type.as_ref(),
            ) {
                sleep_ms(pre_delay_ms);
                service_tap(server_url, &udid, x, y, duration_ms)?;
                sleep_ms(post_delay_ms);
            } else if let Some(server_url) = service_url.as_deref() {
                sleep_ms(pre_delay_ms);
                service_tap_element(
                    server_url,
                    &udid,
                    serde_json::json!({
                        "x": x,
                        "y": y,
                        "normalized": normalized,
                        "selector": {
                            "id": id,
                            "label": label,
                            "value": value,
                            "elementType": element_type,
                        },
                        "waitTimeoutMs": wait_timeout_ms,
                        "pollMs": poll_interval_ms,
                        "durationMs": duration_ms,
                    }),
                )?;
                sleep_ms(post_delay_ms);
            } else {
                let target = resolve_tap_target(
                    &bridge,
                    &udid,
                    TapTargetRequest {
                        x,
                        y,
                        normalized,
                        selector: ElementSelector {
                            id,
                            label,
                            value,
                            element_type,
                        },
                        wait_timeout_ms,
                        poll_interval_ms,
                    },
                )?;
                sleep_ms(pre_delay_ms);
                if let Some(input) = target.input.as_ref() {
                    perform_tap_with_input(input, target.x, target.y, duration_ms)?;
                } else {
                    perform_tap(&bridge, &udid, target.x, target.y, duration_ms)?;
                }
                sleep_ms(post_delay_ms);
            }
            println_json(&serde_json::json!({ "ok": true, "udid": udid, "action": "tap" }))?;
            Ok(())
        }
        Command::Swipe {
            udid,
            start_x,
            start_y,
            end_x,
            end_y,
            normalized,
            duration_ms,
            steps,
            pre_delay_ms,
            post_delay_ms,
        } => {
            if let Some(server_url) = service_url.as_deref().filter(|_| normalized) {
                sleep_ms(pre_delay_ms);
                service_swipe(
                    server_url,
                    &udid,
                    start_x,
                    start_y,
                    end_x,
                    end_y,
                    duration_ms,
                    steps,
                )?;
                sleep_ms(post_delay_ms);
            } else {
                let (start_x, start_y) =
                    resolve_touch_point(&bridge, &udid, start_x, start_y, normalized)?;
                let (end_x, end_y) = resolve_touch_point(&bridge, &udid, end_x, end_y, normalized)?;
                sleep_ms(pre_delay_ms);
                perform_swipe(
                    &bridge,
                    &udid,
                    GestureCoordinates {
                        start_x,
                        start_y,
                        end_x,
                        end_y,
                        duration_ms,
                    },
                    steps,
                )?;
                sleep_ms(post_delay_ms);
            }
            println_json(&serde_json::json!({ "ok": true, "udid": udid, "action": "swipe" }))?;
            Ok(())
        }
        Command::Gesture {
            udid,
            preset,
            screen_width,
            screen_height,
            normalized,
            duration_ms,
            delta,
            pre_delay_ms,
            post_delay_ms,
        } => {
            if let Some(server_url) = service_url.as_deref().filter(|_| normalized) {
                let gesture = gesture_coordinates(
                    &bridge,
                    &udid,
                    &preset,
                    screen_width,
                    screen_height,
                    normalized,
                    delta,
                )?;
                sleep_ms(pre_delay_ms);
                service_swipe(
                    server_url,
                    &udid,
                    gesture.start_x,
                    gesture.start_y,
                    gesture.end_x,
                    gesture.end_y,
                    duration_ms.unwrap_or(gesture.duration_ms),
                    4,
                )?;
                sleep_ms(post_delay_ms);
                println_json(
                    &serde_json::json!({ "ok": true, "udid": udid, "action": "gesture", "preset": preset }),
                )?;
                return Ok(());
            }
            let gesture = gesture_coordinates(
                &bridge,
                &udid,
                &preset,
                screen_width,
                screen_height,
                normalized,
                delta,
            )?;
            sleep_ms(pre_delay_ms);
            perform_swipe(
                &bridge,
                &udid,
                GestureCoordinates {
                    duration_ms: duration_ms.unwrap_or(gesture.duration_ms),
                    ..gesture
                },
                4,
            )?;
            sleep_ms(post_delay_ms);
            println_json(
                &serde_json::json!({ "ok": true, "udid": udid, "action": "gesture", "preset": preset }),
            )?;
            Ok(())
        }
        Command::Pinch {
            udid,
            center_x,
            center_y,
            start_distance,
            end_distance,
            angle_degrees,
            normalized,
            duration_ms,
            steps,
        } => {
            let frames = pinch_frames(
                &bridge,
                &udid,
                center_x,
                center_y,
                start_distance,
                end_distance,
                angle_degrees,
                normalized,
                steps,
            )?;
            run_multitouch_frames(&bridge, &udid, frames, duration_ms)?;
            println_json(&serde_json::json!({ "ok": true, "udid": udid, "action": "pinch" }))?;
            Ok(())
        }
        Command::RotateGesture {
            udid,
            center_x,
            center_y,
            radius,
            degrees,
            normalized,
            duration_ms,
            steps,
        } => {
            let frames = rotate_gesture_frames(
                &bridge,
                &udid,
                RotateGestureRequest {
                    center_x,
                    center_y,
                    radius,
                    degrees,
                    normalized,
                    steps,
                },
            )?;
            run_multitouch_frames(&bridge, &udid, frames, duration_ms)?;
            println_json(
                &serde_json::json!({ "ok": true, "udid": udid, "action": "rotate-gesture" }),
            )?;
            Ok(())
        }
        Command::Key {
            udid,
            key,
            modifiers,
            duration_ms,
            pre_delay_ms,
            post_delay_ms,
        } => {
            let key_code = parse_hid_key(&key)?;
            sleep_ms(pre_delay_ms);
            if let Some(server_url) = service_url.as_deref().filter(|_| duration_ms == 0) {
                service_key(server_url, &udid, key_code, modifiers)?;
            } else if duration_ms > 0 && modifiers == 0 {
                let input = bridge.create_input_session(&udid)?;
                input.send_key_event(key_code, true)?;
                sleep_ms(duration_ms);
                input.send_key_event(key_code, false)?;
            } else {
                bridge.send_key(&udid, key_code, modifiers)?;
            }
            sleep_ms(post_delay_ms);
            println_json(&serde_json::json!({ "ok": true, "udid": udid, "action": "key" }))?;
            Ok(())
        }
        Command::KeySequence {
            udid,
            keycodes,
            delay_ms,
        } => {
            let keys = parse_key_list(&keycodes)?;
            if let Some(server_url) = service_url.as_deref() {
                service_key_sequence(server_url, &udid, &keys, delay_ms)?;
            } else {
                let input = bridge.create_input_session(&udid)?;
                for (index, key) in keys.iter().enumerate() {
                    input.send_key(*key, 0)?;
                    if index + 1 < keys.len() {
                        sleep_ms(delay_ms);
                    }
                }
            }
            println_json(
                &serde_json::json!({ "ok": true, "udid": udid, "action": "key-sequence" }),
            )?;
            Ok(())
        }
        Command::KeyCombo {
            udid,
            modifiers,
            key,
        } => {
            let modifier_mask = parse_modifier_mask(&modifiers)?;
            let key_code = parse_hid_key(&key)?;
            if let Some(server_url) = service_url.as_deref() {
                service_key(server_url, &udid, key_code, modifier_mask)?;
            } else {
                bridge.send_key(&udid, key_code, modifier_mask)?;
            }
            println_json(&serde_json::json!({ "ok": true, "udid": udid, "action": "key-combo" }))?;
            Ok(())
        }
        Command::Type {
            udid,
            text,
            stdin,
            file,
            delay_ms,
        } => {
            let text = read_text_input(text, stdin, file)?;
            type_text(&bridge, &udid, &text, delay_ms)?;
            println_json(&serde_json::json!({ "ok": true, "udid": udid, "action": "type" }))?;
            Ok(())
        }
        Command::Button {
            udid,
            button,
            duration_ms,
        } => {
            if let Some(server_url) = service_url.as_deref() {
                service_button(server_url, &udid, &button, duration_ms)?;
            } else {
                bridge.press_button(&udid, &button, duration_ms)?;
            }
            println_json(
                &serde_json::json!({ "ok": true, "udid": udid, "action": "button", "button": button }),
            )?;
            Ok(())
        }
        Command::Batch {
            udid,
            steps,
            file,
            stdin,
            continue_on_error,
        } => {
            let report = if let Some(server_url) = service_url.as_deref() {
                let step_lines = read_batch_steps(steps, file, stdin)?;
                service_batch(
                    server_url,
                    &udid,
                    batch_lines_to_json_steps(&step_lines)?,
                    continue_on_error,
                )?
            } else {
                run_batch(&bridge, &udid, steps, file, stdin, continue_on_error)?
            };
            println_json(&report)?;
            Ok(())
        }
        Command::DismissKeyboard { udid } => {
            if let Some(server_url) = service_url.as_deref() {
                service_post_ok(server_url, &udid, "dismiss-keyboard", &Value::Null)?;
            } else {
                bridge.send_key(&udid, 41, 0)?;
            }
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({ "ok": true, "udid": udid, "action": "dismiss-keyboard" })
                )?
            );
            Ok(())
        }
        Command::Home { udid } => {
            if let Some(server_url) = service_url.as_deref() {
                service_post_ok(server_url, &udid, "home", &Value::Null)?;
            } else {
                bridge.press_home(&udid)?;
            }
            println_json(&serde_json::json!({ "ok": true, "udid": udid, "action": "home" }))?;
            Ok(())
        }
        Command::AppSwitcher { udid } => {
            if let Some(server_url) = service_url.as_deref() {
                service_post_ok(server_url, &udid, "app-switcher", &Value::Null)?;
            } else {
                bridge.press_home(&udid)?;
                std::thread::sleep(Duration::from_millis(140));
                bridge.press_home(&udid)?;
            }
            println_json(
                &serde_json::json!({ "ok": true, "udid": udid, "action": "app-switcher" }),
            )?;
            Ok(())
        }
        Command::RotateLeft { udid } => {
            if let Some(server_url) = service_url.as_deref() {
                service_post_ok(server_url, &udid, "rotate-left", &Value::Null)?;
            } else {
                bridge.rotate_left(&udid)?;
            }
            println_json(
                &serde_json::json!({ "ok": true, "udid": udid, "action": "rotate-left" }),
            )?;
            Ok(())
        }
        Command::RotateRight { udid } => {
            if let Some(server_url) = service_url.as_deref() {
                service_post_ok(server_url, &udid, "rotate-right", &Value::Null)?;
            } else {
                bridge.rotate_right(&udid)?;
            }
            println_json(
                &serde_json::json!({ "ok": true, "udid": udid, "action": "rotate-right" }),
            )?;
            Ok(())
        }
        Command::ChromeProfile { udid } => {
            let service_url = command_service_url(explicit_server_url.clone())?;
            let profile = service_get_json(
                &service_url,
                &format!(
                    "/api/simulators/{}/chrome-profile",
                    url_path_component(&udid)
                ),
            )?;
            println_json(&profile)?;
            Ok(())
        }
    }
}

#[derive(Clone, Debug)]
struct ServiceOptions {
    port: u16,
    bind: IpAddr,
    advertise_host: Option<String>,
    client_root: Option<PathBuf>,
    video_codec: VideoCodecMode,
    low_latency: bool,
    access_token: Option<String>,
    pairing_code: Option<String>,
}

#[allow(clippy::too_many_arguments)]
fn serve_with_appkit(
    port: u16,
    bind: IpAddr,
    advertise_host: Option<String>,
    client_root: Option<PathBuf>,
    video_codec: VideoCodecMode,
    low_latency: bool,
    access_token: Option<String>,
    pairing_code: Option<String>,
) -> anyhow::Result<()> {
    std::env::set_var("SIMDECK_VIDEO_CODEC", video_codec.as_env_value());
    std::env::set_var("SIMDECK_LOW_LATENCY", if low_latency { "1" } else { "0" });
    std::env::set_var(RESTART_ON_CORE_SIMULATOR_MISMATCH_ENV, "1");
    start_fd_pressure_watchdog();
    unsafe {
        ffi::xcw_native_initialize_app();
    }

    let (result_tx, result_rx) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .context("build tokio runtime");
        let result = match runtime {
            Ok(runtime) => runtime.block_on(serve(
                port,
                bind,
                advertise_host,
                client_root,
                video_codec,
                low_latency,
                access_token,
                pairing_code,
            )),
            Err(error) => Err(error),
        };
        let _ = result_tx.send(result);
    });

    loop {
        match result_rx.try_recv() {
            Ok(result) => return result,
            Err(mpsc::TryRecvError::Disconnected) => {
                anyhow::bail!("server runtime thread exited unexpectedly");
            }
            Err(mpsc::TryRecvError::Empty) => unsafe {
                ffi::xcw_native_run_main_loop_slice(0.05);
            },
        }
    }
}

fn start_fd_pressure_watchdog() {
    std::thread::spawn(|| loop {
        std::thread::sleep(Duration::from_secs(1));
        let Ok(fd_count) = open_fd_count() else {
            continue;
        };
        if fd_count <= SERVER_FD_RESTART_THRESHOLD {
            continue;
        }
        eprintln!(
            "Open file descriptor count reached {fd_count}; restarting simdeck server process."
        );
        std::process::exit(RECOVERABLE_RESTART_EXIT_CODE);
    });
}

fn open_fd_count() -> io::Result<usize> {
    fs::read_dir("/dev/fd").map(|entries| entries.count())
}

fn start_server_health_watchdog(http_addr: SocketAddr, heartbeat: Arc<AtomicU64>) {
    std::thread::spawn(move || {
        std::thread::sleep(SERVER_HEALTH_WATCHDOG_INITIAL_DELAY);
        let mut consecutive_failures = 0usize;

        loop {
            std::thread::sleep(SERVER_HEALTH_WATCHDOG_INTERVAL);

            let heartbeat_age = now_secs().saturating_sub(heartbeat.load(Ordering::Relaxed));
            let heartbeat_stale = heartbeat_age > SERVER_HEALTH_WATCHDOG_STALE_HEARTBEAT.as_secs();
            let health_ok = http_health_probe(http_addr, SERVER_HEALTH_WATCHDOG_PROBE_TIMEOUT);

            if heartbeat_stale || !health_ok {
                consecutive_failures += 1;
            } else {
                consecutive_failures = 0;
            }

            if consecutive_failures >= SERVER_HEALTH_WATCHDOG_FAILURE_THRESHOLD {
                eprintln!(
                    "SimDeck server health watchdog failed {consecutive_failures} consecutive checks \
(heartbeat_age={heartbeat_age}s, http_health_ok={health_ok}); restarting server process."
                );
                std::process::exit(RECOVERABLE_RESTART_EXIT_CODE);
            }
        }
    });
}

fn http_health_probe(address: SocketAddr, timeout: Duration) -> bool {
    let Ok(mut stream) = std::net::TcpStream::connect_timeout(&address, timeout) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));

    let request = b"GET /api/health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
    if stream.write_all(request).is_err() {
        return false;
    }

    let mut response = [0u8; 128];
    let Ok(read) = stream.read(&mut response) else {
        return false;
    };
    read > 12 && response[..read].starts_with(b"HTTP/1.1 200")
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

#[derive(Clone, Debug, Default)]
struct ElementSelector {
    id: Option<String>,
    label: Option<String>,
    value: Option<String>,
    element_type: Option<String>,
}

#[derive(Clone, Copy, Debug)]
struct GestureCoordinates {
    start_x: f64,
    start_y: f64,
    end_x: f64,
    end_y: f64,
    duration_ms: u64,
}

#[derive(Clone, Debug)]
struct TapTargetRequest {
    x: Option<f64>,
    y: Option<f64>,
    normalized: bool,
    selector: ElementSelector,
    wait_timeout_ms: u64,
    poll_interval_ms: u64,
}

struct ResolvedTapTarget {
    x: f64,
    y: f64,
    input: Option<NativeInputSession>,
}

#[derive(Clone, Copy, Debug)]
struct ElementTapTarget {
    x: f64,
    y: f64,
    root_width: f64,
    root_height: f64,
}

#[derive(Clone, Copy, Debug)]
struct MultiTouchFrame {
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
}

#[derive(Clone, Copy, Debug)]
struct RotateGestureRequest {
    center_x: Option<f64>,
    center_y: Option<f64>,
    radius: f64,
    degrees: f64,
    normalized: bool,
    steps: u32,
}

fn run_multitouch_frames(
    bridge: &NativeBridge,
    udid: &str,
    frames: Vec<MultiTouchFrame>,
    duration_ms: u64,
) -> Result<(), crate::error::AppError> {
    let Some(first) = frames.first().copied() else {
        return Err(crate::error::AppError::bad_request(
            "Multi-touch gesture requires at least one frame.",
        ));
    };
    let step_delay = if frames.len() > 1 {
        duration_ms / (frames.len() as u64 - 1)
    } else {
        duration_ms
    };
    let input = bridge.create_input_session(udid)?;
    input.send_multitouch(first.x1, first.y1, first.x2, first.y2, "began")?;
    for frame in frames
        .iter()
        .copied()
        .skip(1)
        .take(frames.len().saturating_sub(2))
    {
        sleep_ms(step_delay);
        input.send_multitouch(frame.x1, frame.y1, frame.x2, frame.y2, "moved")?;
    }
    if let Some(last) = frames.last().copied() {
        sleep_ms(step_delay);
        input.send_multitouch(last.x1, last.y1, last.x2, last.y2, "ended")?;
    }
    Ok(())
}

fn sleep_ms(duration_ms: u64) {
    if duration_ms > 0 {
        std::thread::sleep(Duration::from_millis(duration_ms));
    }
}

fn perform_tap(
    bridge: &NativeBridge,
    udid: &str,
    x: f64,
    y: f64,
    duration_ms: u64,
) -> Result<(), crate::error::AppError> {
    let input = bridge.create_input_session(udid)?;
    perform_tap_with_input(&input, x, y, duration_ms)
}

fn perform_tap_with_input(
    input: &NativeInputSession,
    x: f64,
    y: f64,
    duration_ms: u64,
) -> Result<(), crate::error::AppError> {
    input.send_touch(x, y, "began")?;
    sleep_ms(duration_ms);
    input.send_touch(x, y, "ended")
}

fn perform_swipe(
    bridge: &NativeBridge,
    udid: &str,
    gesture: GestureCoordinates,
    steps: u32,
) -> Result<(), crate::error::AppError> {
    let step_count = steps.max(1);
    let delay = Duration::from_millis(gesture.duration_ms / u64::from(step_count));
    let input = bridge.create_input_session(udid)?;
    input.send_touch(gesture.start_x, gesture.start_y, "began")?;
    for step in 1..step_count {
        let t = f64::from(step) / f64::from(step_count);
        input.send_touch(
            lerp(gesture.start_x, gesture.end_x, t),
            lerp(gesture.start_y, gesture.end_y, t),
            "moved",
        )?;
        std::thread::sleep(delay);
    }
    input.send_touch(gesture.end_x, gesture.end_y, "ended")
}

fn type_text(
    bridge: &NativeBridge,
    udid: &str,
    text: &str,
    delay_ms: u64,
) -> Result<(), crate::error::AppError> {
    let input = bridge.create_input_session(udid)?;
    for character in text.chars() {
        let Some((key_code, modifiers)) = hid_for_character(character) else {
            return Err(crate::error::AppError::bad_request(format!(
                "Unsupported character for HID typing: {character:?}"
            )));
        };
        input.send_key(key_code, modifiers)?;
        sleep_ms(delay_ms);
    }
    Ok(())
}

fn read_text_input(
    text: Option<String>,
    use_stdin: bool,
    file: Option<PathBuf>,
) -> anyhow::Result<String> {
    let sources =
        usize::from(text.is_some()) + usize::from(use_stdin) + usize::from(file.is_some());
    if sources != 1 {
        return Err(crate::error::AppError::bad_request(
            "Specify exactly one input source: text argument, --stdin, or --file.",
        )
        .into());
    }
    if use_stdin {
        let mut buffer = String::new();
        io::stdin().read_to_string(&mut buffer)?;
        return Ok(buffer);
    }
    if let Some(file) = file {
        return Ok(fs::read_to_string(file)?);
    }
    Ok(text.unwrap_or_default())
}

fn default_screenshot_path(udid: &str) -> PathBuf {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    PathBuf::from(format!("Simulator Screenshot - {udid} - {timestamp}.png"))
}

#[allow(clippy::too_many_arguments)]
fn describe_ui_snapshot(
    bridge: &NativeBridge,
    udid: &str,
    point: Option<(f64, f64)>,
    source: DescribeUiSource,
    max_depth: Option<usize>,
    include_hidden: bool,
    direct: bool,
    server_url: &str,
) -> anyhow::Result<Value> {
    if point.is_none() && !direct {
        match fetch_service_accessibility_tree(udid, source, max_depth, include_hidden, server_url)
        {
            Ok(snapshot) => return Ok(snapshot),
            Err(error) if source != DescribeUiSource::Auto => return Err(error),
            Err(_) => {}
        }
    }

    if source != DescribeUiSource::Auto && source != DescribeUiSource::NativeAx {
        anyhow::bail!(
            "The `{}` hierarchy source requires a running SimDeck daemon. Start it with `simdeck daemon start --port 4310`, or use --source native-ax.",
            source.as_query_value()
        );
    }

    Ok(bridge.accessibility_snapshot_with_max_depth(udid, point, max_depth)?)
}

fn fetch_service_accessibility_tree(
    udid: &str,
    source: DescribeUiSource,
    max_depth: Option<usize>,
    include_hidden: bool,
    server_url: &str,
) -> anyhow::Result<Value> {
    let mut query = vec![format!("source={}", source.as_query_value())];
    if let Some(max_depth) = max_depth {
        query.push(format!("maxDepth={}", max_depth.min(80)));
    }
    if include_hidden {
        query.push("includeHidden=true".to_owned());
    }
    let path = format!(
        "/api/simulators/{}/accessibility-tree?{}",
        url_path_component(udid),
        query.join("&")
    );
    http_get_json(server_url, &path)
}

fn http_get_json(server_url: &str, path: &str) -> anyhow::Result<Value> {
    http_request_json(server_url, "GET", path, None)
}

fn service_get_json(server_url: &str, path: &str) -> anyhow::Result<Value> {
    http_request_json(server_url, "GET", path, None)
}

fn service_get_bytes(server_url: &str, path: &str) -> anyhow::Result<Vec<u8>> {
    http_request(server_url, "GET", path, None)
}

fn service_open_url(server_url: &str, udid: &str, url: &str) -> anyhow::Result<()> {
    service_post_ok(
        server_url,
        udid,
        "open-url",
        &serde_json::json!({ "url": url }),
    )
}

fn service_launch(server_url: &str, udid: &str, bundle_id: &str) -> anyhow::Result<()> {
    service_post_ok(
        server_url,
        udid,
        "launch",
        &serde_json::json!({ "bundleId": bundle_id }),
    )
}

fn service_touch(server_url: &str, udid: &str, x: f64, y: f64, phase: &str) -> anyhow::Result<()> {
    service_post_ok(
        server_url,
        udid,
        "touch",
        &serde_json::json!({ "x": x, "y": y, "phase": phase }),
    )
}

fn service_tap(
    server_url: &str,
    udid: &str,
    x: f64,
    y: f64,
    duration_ms: u64,
) -> anyhow::Result<()> {
    service_touch_sequence(
        server_url,
        udid,
        vec![
            service_touch_event(x, y, "began", duration_ms),
            service_touch_event(x, y, "ended", 0),
        ],
    )
}

fn service_tap_element(server_url: &str, udid: &str, body: Value) -> anyhow::Result<()> {
    service_post_ok(server_url, udid, "tap", &body)
}

fn service_batch(
    server_url: &str,
    udid: &str,
    steps: Vec<Value>,
    continue_on_error: bool,
) -> anyhow::Result<Value> {
    let path = format!("/api/simulators/{}/batch", url_path_component(udid));
    http_request_json(
        server_url,
        "POST",
        &path,
        Some(&serde_json::json!({
            "steps": steps,
            "continueOnError": continue_on_error,
        })),
    )
}

#[allow(clippy::too_many_arguments)]
fn service_swipe(
    server_url: &str,
    udid: &str,
    start_x: f64,
    start_y: f64,
    end_x: f64,
    end_y: f64,
    duration_ms: u64,
    steps: u32,
) -> anyhow::Result<()> {
    let step_count = steps.max(2);
    let delay_ms = duration_ms / u64::from(step_count);
    let mut events = vec![service_touch_event(start_x, start_y, "began", 0)];
    for index in 1..step_count {
        let progress = f64::from(index) / f64::from(step_count);
        let x = start_x + (end_x - start_x) * progress;
        let y = start_y + (end_y - start_y) * progress;
        events.push(service_touch_event(x, y, "moved", delay_ms));
    }
    events.push(service_touch_event(end_x, end_y, "ended", 0));
    service_touch_sequence(server_url, udid, events)
}

fn service_touch_event(x: f64, y: f64, phase: &str, delay_ms_after: u64) -> Value {
    serde_json::json!({
        "x": x,
        "y": y,
        "phase": phase,
        "delayMsAfter": delay_ms_after,
    })
}

fn service_touch_sequence(server_url: &str, udid: &str, events: Vec<Value>) -> anyhow::Result<()> {
    service_post_ok(
        server_url,
        udid,
        "touch-sequence",
        &serde_json::json!({ "events": events }),
    )
}

fn service_key(server_url: &str, udid: &str, key_code: u16, modifiers: u32) -> anyhow::Result<()> {
    service_post_ok(
        server_url,
        udid,
        "key",
        &serde_json::json!({ "keyCode": key_code, "modifiers": modifiers }),
    )
}

fn service_key_sequence(
    server_url: &str,
    udid: &str,
    keys: &[u16],
    delay_ms: u64,
) -> anyhow::Result<()> {
    service_post_ok(
        server_url,
        udid,
        "key-sequence",
        &serde_json::json!({ "keyCodes": keys, "delayMs": delay_ms }),
    )
}

fn service_button(
    server_url: &str,
    udid: &str,
    button: &str,
    duration_ms: u32,
) -> anyhow::Result<()> {
    service_post_ok(
        server_url,
        udid,
        "button",
        &serde_json::json!({ "button": button, "durationMs": duration_ms }),
    )
}

fn service_post_ok(server_url: &str, udid: &str, action: &str, body: &Value) -> anyhow::Result<()> {
    let path = format!("/api/simulators/{}/{}", url_path_component(udid), action);
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        match http_request_json(server_url, "POST", &path, Some(body)) {
            Ok(_) => return Ok(()),
            Err(error)
                if Instant::now() < deadline
                    && error
                        .to_string()
                        .contains("Resource temporarily unavailable") =>
            {
                std::thread::sleep(Duration::from_millis(250));
            }
            Err(error) => return Err(error),
        }
    }
}

fn http_request_json(
    server_url: &str,
    method: &str,
    path: &str,
    body: Option<&Value>,
) -> anyhow::Result<Value> {
    let body = http_request(server_url, method, path, body)?;
    serde_json::from_slice(&body).context("parse SimDeck service JSON response")
}

fn http_request(
    server_url: &str,
    method: &str,
    path: &str,
    body: Option<&Value>,
) -> anyhow::Result<Vec<u8>> {
    let endpoint = HttpEndpoint::parse(server_url)?;
    let mut stream = std::net::TcpStream::connect((endpoint.host.as_str(), endpoint.port))
        .with_context(|| format!("connect to SimDeck service at {server_url}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(120)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let body = body.map(serde_json::to_vec).transpose()?;
    let request = if let Some(body) = body.as_ref() {
        format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nOrigin: {}\r\nAccept: application/json\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            endpoint.host_header(),
            endpoint.origin(),
            body.len(),
        )
    } else {
        format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nOrigin: {}\r\nAccept: application/json\r\nConnection: close\r\n\r\n",
            endpoint.host_header(),
            endpoint.origin(),
        )
    };
    stream.write_all(request.as_bytes())?;
    if let Some(body) = body.as_ref() {
        stream.write_all(body)?;
    }

    let mut response = Vec::new();
    stream.read_to_end(&mut response)?;
    let (status, headers, body) = parse_http_response(&response)?;
    let body = if response_is_chunked(&headers) {
        decode_chunked_body(body)?
    } else {
        body.to_vec()
    };
    if !(200..300).contains(&status) {
        let message = String::from_utf8_lossy(&body).trim().to_owned();
        anyhow::bail!(
            "SimDeck service returned HTTP {status}{}",
            if message.is_empty() {
                String::new()
            } else {
                format!(": {message}")
            }
        );
    }
    Ok(body)
}

struct HttpEndpoint {
    host: String,
    port: u16,
}

type HttpHeaders = Vec<(String, String)>;

impl HttpEndpoint {
    fn parse(server_url: &str) -> anyhow::Result<Self> {
        let without_scheme = server_url
            .trim_end_matches('/')
            .strip_prefix("http://")
            .ok_or_else(|| anyhow::anyhow!("Only http:// server URLs are supported."))?;
        let authority = without_scheme
            .split('/')
            .next()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Server URL must include a host."))?;
        let (host, port) = if let Some(host) = authority.strip_prefix('[') {
            let (host, rest) = host
                .split_once(']')
                .ok_or_else(|| anyhow::anyhow!("Invalid IPv6 server URL host."))?;
            let port = rest
                .strip_prefix(':')
                .map(parse_port)
                .transpose()?
                .unwrap_or(80);
            (host.to_owned(), port)
        } else if let Some((host, port)) = authority.rsplit_once(':') {
            (host.to_owned(), parse_port(port)?)
        } else {
            (authority.to_owned(), 80)
        };
        Ok(Self { host, port })
    }

    fn host_header(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }

    fn origin(&self) -> String {
        format!("http://{}", self.host_header())
    }
}

fn parse_port(value: &str) -> anyhow::Result<u16> {
    value
        .parse::<u16>()
        .with_context(|| format!("parse port `{value}`"))
}

fn parse_http_response(response: &[u8]) -> anyhow::Result<(u16, HttpHeaders, &[u8])> {
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| anyhow::anyhow!("SimDeck service returned a malformed HTTP response."))?;
    let header_bytes = &response[..header_end];
    let body = &response[header_end + 4..];
    let header_text = std::str::from_utf8(header_bytes).context("parse HTTP headers as UTF-8")?;
    let mut lines = header_text.lines();
    let status_line = lines
        .next()
        .ok_or_else(|| anyhow::anyhow!("SimDeck service returned an empty HTTP response."))?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("HTTP response did not include a status code."))?
        .parse::<u16>()
        .context("parse HTTP status code")?;
    let headers = lines
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        })
        .collect();
    Ok((status, headers, body))
}

fn response_is_chunked(headers: &[(String, String)]) -> bool {
    headers.iter().any(|(name, value)| {
        name == "transfer-encoding"
            && value
                .split(',')
                .any(|part| part.trim().eq_ignore_ascii_case("chunked"))
    })
}

fn decode_chunked_body(mut body: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut decoded = Vec::new();
    loop {
        let line_end = body
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or_else(|| anyhow::anyhow!("Chunked response ended before a chunk size."))?;
        let size_text = std::str::from_utf8(&body[..line_end])
            .context("parse chunk size as UTF-8")?
            .split(';')
            .next()
            .unwrap_or("")
            .trim();
        let size = usize::from_str_radix(size_text, 16).context("parse chunk size")?;
        body = &body[line_end + 2..];
        if size == 0 {
            return Ok(decoded);
        }
        if body.len() < size + 2 {
            anyhow::bail!("Chunked response ended before a full chunk.");
        }
        decoded.extend_from_slice(&body[..size]);
        body = &body[size + 2..];
    }
}

fn url_path_component(value: &str) -> String {
    value
        .bytes()
        .flat_map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                vec![byte as char]
            }
            _ => format!("%{byte:02X}").chars().collect(),
        })
        .collect()
}

impl DescribeUiSource {
    fn as_query_value(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Nativescript => "nativescript",
            Self::ReactNative => "react-native",
            Self::Uikit => "uikit",
            Self::NativeAx => "native-ax",
        }
    }
}

fn print_describe_ui(snapshot: &Value, format: DescribeUiFormat) -> anyhow::Result<()> {
    match format {
        DescribeUiFormat::Json => println_json(snapshot),
        DescribeUiFormat::CompactJson => {
            println!(
                "{}",
                serde_json::to_string(&compact_accessibility_snapshot(snapshot))?
            );
            Ok(())
        }
        DescribeUiFormat::Agent => {
            print!("{}", render_agent_accessibility_tree(snapshot));
            Ok(())
        }
    }
}

fn compact_accessibility_snapshot(snapshot: &Value) -> Value {
    let roots = snapshot
        .get("roots")
        .and_then(Value::as_array)
        .map(|roots| {
            roots
                .iter()
                .map(compact_accessibility_node)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut object = serde_json::Map::new();
    object.insert(
        "source".to_owned(),
        snapshot
            .get("source")
            .cloned()
            .unwrap_or_else(|| Value::String("unknown".to_owned())),
    );
    object.insert("roots".to_owned(), Value::Array(roots));
    for field in ["availableSources", "fallbackReason", "fallbackSource"] {
        if let Some(value) = snapshot.get(field) {
            object.insert(field.to_owned(), value.clone());
        }
    }
    Value::Object(object)
}

fn compact_accessibility_node(node: &Value) -> Value {
    let mut object = serde_json::Map::new();
    insert_string_alias(node, &mut object, "role", &["type", "role", "className"]);
    insert_string_alias(
        node,
        &mut object,
        "id",
        &["AXIdentifier", "AXUniqueId", "inspectorId", "id"],
    );
    insert_string_alias(
        node,
        &mut object,
        "label",
        &["AXLabel", "label", "title", "text", "name"],
    );
    insert_string_alias(
        node,
        &mut object,
        "value",
        &["AXValue", "value", "placeholder"],
    );
    if let Some(frame) = compact_frame(node.get("frame").or_else(|| node.get("frameInScreen"))) {
        object.insert("frame".to_owned(), frame);
    }
    if truthy_field(node, "hidden").unwrap_or(false)
        || truthy_field(node, "isHidden").unwrap_or(false)
    {
        object.insert("hidden".to_owned(), Value::Bool(true));
    }
    if let Some(false) = truthy_field(node, "enabled") {
        object.insert("enabled".to_owned(), Value::Bool(false));
    }
    if let Some(actions) = node
        .get("custom_actions")
        .or_else(|| {
            node.get("control")
                .and_then(|control| control.get("actions"))
        })
        .and_then(Value::as_array)
    {
        let actions = actions
            .iter()
            .filter_map(Value::as_str)
            .map(|action| Value::String(action.to_owned()))
            .collect::<Vec<_>>();
        if !actions.is_empty() {
            object.insert("actions".to_owned(), Value::Array(actions));
        }
    }
    if let Some(source_location) = node.get("sourceLocation").filter(|value| !value.is_null()) {
        object.insert("sourceLocation".to_owned(), source_location.clone());
    }
    let children = node
        .get("children")
        .and_then(Value::as_array)
        .map(|children| {
            children
                .iter()
                .map(compact_accessibility_node)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !children.is_empty() {
        object.insert("children".to_owned(), Value::Array(children));
    }
    Value::Object(object)
}

fn insert_string_alias(
    source: &Value,
    target: &mut serde_json::Map<String, Value>,
    output_key: &str,
    input_keys: &[&str],
) {
    if let Some(value) = input_keys
        .iter()
        .filter_map(|key| source.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .find(|value| !value.is_empty())
    {
        target.insert(output_key.to_owned(), Value::String(value.to_owned()));
    }
}

fn compact_frame(frame: Option<&Value>) -> Option<Value> {
    let frame = frame?;
    let x = frame.get("x")?.as_f64()?;
    let y = frame.get("y")?.as_f64()?;
    let width = frame.get("width")?.as_f64()?;
    let height = frame.get("height")?.as_f64()?;
    Some(serde_json::json!([
        round_frame_value(x),
        round_frame_value(y),
        round_frame_value(width),
        round_frame_value(height)
    ]))
}

fn round_frame_value(value: f64) -> Value {
    let rounded = (value * 10.0).round() / 10.0;
    serde_json::Number::from_f64(rounded)
        .map(Value::Number)
        .unwrap_or(Value::Null)
}

fn truthy_field(node: &Value, field: &str) -> Option<bool> {
    node.get(field).and_then(Value::as_bool)
}

fn render_agent_accessibility_tree(snapshot: &Value) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "source: {}",
        snapshot
            .get("source")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
    ));
    if let Some(sources) = snapshot.get("availableSources").and_then(Value::as_array) {
        let sources = sources
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(",");
        if !sources.is_empty() {
            lines.push(format!("available: {sources}"));
        }
    }
    if let Some(reason) = snapshot.get("fallbackReason").and_then(Value::as_str) {
        lines.push(format!("fallback: {}", compact_text(reason)));
    }
    if let Some(roots) = snapshot.get("roots").and_then(Value::as_array) {
        for root in roots {
            render_agent_node(root, 0, &mut lines);
        }
    }
    lines.push(String::new());
    lines.join("\n")
}

fn render_agent_node(node: &Value, depth: usize, lines: &mut Vec<String>) {
    let compact = compact_accessibility_node(node);
    let object = compact.as_object();
    let field = |name| {
        object
            .and_then(|object| object.get(name))
            .and_then(Value::as_str)
            .map(compact_text)
            .filter(|value| !value.is_empty())
    };
    let role = field("role").unwrap_or_else(|| "View".to_owned());
    let id = field("id");
    let label = field("label");
    let value = field("value");
    let mut line = format!("{}- {}", "  ".repeat(depth), role);
    if let Some(id) = id {
        line.push_str(" #");
        line.push_str(&id);
    }
    if let Some(label) = label.as_ref() {
        line.push_str(": ");
        line.push_str(label);
    }
    if let Some(value) = value.filter(|value| Some(value) != label.as_ref()) {
        line.push_str(" = ");
        line.push_str(&value);
    }
    if let Some(frame) = object
        .and_then(|object| object.get("frame"))
        .and_then(Value::as_array)
        .filter(|frame| frame.len() == 4)
    {
        line.push_str(&format!(
            " @{},{} {}x{}",
            frame_value(&frame[0]),
            frame_value(&frame[1]),
            frame_value(&frame[2]),
            frame_value(&frame[3])
        ));
    }
    let mut flags = Vec::new();
    if object
        .and_then(|object| object.get("hidden"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        flags.push("hidden");
    }
    if object
        .and_then(|object| object.get("enabled"))
        .and_then(Value::as_bool)
        == Some(false)
    {
        flags.push("disabled");
    }
    if let Some(actions) = object
        .and_then(|object| object.get("actions"))
        .and_then(Value::as_array)
    {
        let actions = actions.iter().filter_map(Value::as_str).collect::<Vec<_>>();
        if !actions.is_empty() {
            line.push_str(" actions=");
            line.push_str(&actions.join(","));
        }
    }
    if !flags.is_empty() {
        line.push(' ');
        line.push_str(&flags.join(","));
    }
    lines.push(line);

    if let Some(children) = node.get("children").and_then(Value::as_array) {
        for child in children {
            render_agent_node(child, depth + 1, lines);
        }
    }
}

fn compact_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn frame_value(value: &Value) -> String {
    value
        .as_f64()
        .map(|value| {
            if value.fract() == 0.0 {
                format!("{value:.0}")
            } else {
                format!("{value:.1}")
            }
        })
        .unwrap_or_else(|| "?".to_owned())
}

fn resolve_tap_target(
    bridge: &NativeBridge,
    udid: &str,
    request: TapTargetRequest,
) -> Result<ResolvedTapTarget, crate::error::AppError> {
    if request.selector.id.is_none()
        && request.selector.label.is_none()
        && request.selector.value.is_none()
    {
        let x = request.x.ok_or_else(|| {
            crate::error::AppError::bad_request("Tap requires x and y or a selector.")
        })?;
        let y = request.y.ok_or_else(|| {
            crate::error::AppError::bad_request("Tap requires x and y or a selector.")
        })?;
        let (x, y) = resolve_touch_point(bridge, udid, x, y, request.normalized)?;
        return Ok(ResolvedTapTarget { x, y, input: None });
    }

    let deadline = std::time::Instant::now() + Duration::from_millis(request.wait_timeout_ms);
    loop {
        let snapshot = bridge.accessibility_snapshot(udid, None)?;
        if let Some(target) = find_element_tap_target(&snapshot, &request.selector) {
            let input = bridge.create_input_session(udid)?;
            let (x, y) = if let Some((display_width, display_height)) = input.display_size() {
                normalize_accessibility_point_for_display(
                    target.x,
                    target.y,
                    target.root_width,
                    target.root_height,
                    display_width,
                    display_height,
                )
            } else {
                (
                    (target.x / target.root_width).clamp(0.0, 1.0),
                    (target.y / target.root_height).clamp(0.0, 1.0),
                )
            };
            return Ok(ResolvedTapTarget {
                x,
                y,
                input: Some(input),
            });
        }
        if request.wait_timeout_ms == 0 || std::time::Instant::now() >= deadline {
            return Err(crate::error::AppError::not_found(
                "No accessibility element matched the tap selector.",
            ));
        }
        sleep_ms(request.poll_interval_ms.max(10));
    }
}

fn find_element_tap_target(
    snapshot: &Value,
    selector: &ElementSelector,
) -> Option<ElementTapTarget> {
    let roots = snapshot.get("roots")?.as_array()?;
    let mut matches = Vec::new();
    for root in roots {
        let (root_width, root_height) = element_size(root)?;
        collect_matching_elements(root, selector, root_width, root_height, &mut matches);
    }
    matches
        .into_iter()
        .max_by_key(|target| is_actionable_element(target.node) as u8)
        .and_then(|target| {
            element_center(target.node).map(|(x, y)| ElementTapTarget {
                x,
                y,
                root_width: target.root_width,
                root_height: target.root_height,
            })
        })
}

struct MatchedElement<'a> {
    node: &'a Value,
    root_width: f64,
    root_height: f64,
}

fn collect_matching_elements<'a>(
    node: &'a Value,
    selector: &ElementSelector,
    root_width: f64,
    root_height: f64,
    matches: &mut Vec<MatchedElement<'a>>,
) {
    if element_matches(node, selector) {
        matches.push(MatchedElement {
            node,
            root_width,
            root_height,
        });
    }
    if let Some(children) = node.get("children").and_then(Value::as_array) {
        for child in children {
            collect_matching_elements(child, selector, root_width, root_height, matches);
        }
    }
}

fn element_matches(node: &Value, selector: &ElementSelector) -> bool {
    if let Some(element_type) = &selector.element_type {
        let node_type = string_field(node, "type").or_else(|| string_field(node, "role"));
        if !node_type
            .as_deref()
            .map(|value| value.eq_ignore_ascii_case(element_type))
            .unwrap_or(false)
        {
            return false;
        }
    }
    if let Some(id) = &selector.id {
        return [
            "AXUniqueId",
            "AXIdentifier",
            "id",
            "identifier",
            "inspectorId",
        ]
        .iter()
        .filter_map(|key| string_field(node, key))
        .any(|value| value == *id);
    }
    if let Some(label) = &selector.label {
        return ["AXLabel", "label", "title", "name"]
            .iter()
            .filter_map(|key| string_field(node, key))
            .any(|value| value == *label);
    }
    if let Some(expected_value) = &selector.value {
        return ["AXValue", "value"]
            .iter()
            .filter_map(|key| string_field(node, key))
            .any(|value| value == *expected_value);
    }
    false
}

fn string_field(node: &Value, key: &str) -> Option<String> {
    node.get(key)?.as_str().map(str::to_owned)
}

fn element_center(node: &Value) -> Option<(f64, f64)> {
    let frame = node.get("frame")?;
    let x = frame.get("x")?.as_f64()?;
    let y = frame.get("y")?.as_f64()?;
    let width = frame.get("width")?.as_f64()?;
    let height = frame.get("height")?.as_f64()?;
    (width > 0.0 && height > 0.0).then_some((x + width / 2.0, y + height / 2.0))
}

fn element_size(node: &Value) -> Option<(f64, f64)> {
    let frame = node.get("frame")?;
    let width = frame.get("width")?.as_f64()?;
    let height = frame.get("height")?.as_f64()?;
    (width > 0.0 && height > 0.0).then_some((width, height))
}

fn normalize_accessibility_point_for_display(
    x: f64,
    y: f64,
    root_width: f64,
    root_height: f64,
    display_width: f64,
    display_height: f64,
) -> (f64, f64) {
    let normalized_x = (x / root_width).clamp(0.0, 1.0);
    let normalized_y = (y / root_height).clamp(0.0, 1.0);
    let root_is_landscape = root_width > root_height;
    let display_is_landscape = display_width > display_height;
    if root_is_landscape != display_is_landscape {
        return (normalized_y, normalized_x);
    }
    (normalized_x, normalized_y)
}

fn is_actionable_element(node: &Value) -> bool {
    let haystack = format!(
        "{} {}",
        string_field(node, "type").unwrap_or_default(),
        string_field(node, "role").unwrap_or_default()
    )
    .to_lowercase();
    ["button", "textfield", "switch", "link", "cell"]
        .iter()
        .any(|needle| haystack.contains(needle))
}

fn gesture_coordinates(
    bridge: &NativeBridge,
    udid: &str,
    preset: &str,
    screen_width: Option<f64>,
    screen_height: Option<f64>,
    normalized: bool,
    delta: Option<f64>,
) -> Result<GestureCoordinates, crate::error::AppError> {
    let (width, height) = if normalized {
        (1.0, 1.0)
    } else {
        match (screen_width, screen_height) {
            (Some(width), Some(height)) => (width, height),
            _ => accessibility_root_size(bridge, udid)
                .or_else(|| chrome_screen_size(bridge, udid))
                .unwrap_or((390.0, 844.0)),
        }
    };
    let center_x = width / 2.0;
    let center_y = height / 2.0;
    let edge = if normalized { 0.02 } else { 20.0 };
    let distance = delta.unwrap_or(if normalized { 0.25 } else { 200.0 });
    let (start_x, start_y, end_x, end_y, duration_ms) = match preset {
        "scroll-up" => (
            center_x,
            center_y + distance / 2.0,
            center_x,
            center_y - distance / 2.0,
            500,
        ),
        "scroll-down" => (
            center_x,
            center_y - distance / 2.0,
            center_x,
            center_y + distance / 2.0,
            500,
        ),
        "scroll-left" => (
            center_x + distance / 2.0,
            center_y,
            center_x - distance / 2.0,
            center_y,
            500,
        ),
        "scroll-right" => (
            center_x - distance / 2.0,
            center_y,
            center_x + distance / 2.0,
            center_y,
            500,
        ),
        "swipe-from-left-edge" => (edge, center_y, width - edge, center_y, 300),
        "swipe-from-right-edge" => (width - edge, center_y, edge, center_y, 300),
        "swipe-from-top-edge" => (center_x, edge, center_x, height - edge, 300),
        "swipe-from-bottom-edge" => (center_x, height - edge, center_x, edge, 300),
        _ => {
            return Err(crate::error::AppError::bad_request(format!(
                "Unsupported gesture preset `{preset}`."
            )))
        }
    };
    let (start_x, start_y) = resolve_touch_point(bridge, udid, start_x, start_y, normalized)?;
    let (end_x, end_y) = resolve_touch_point(bridge, udid, end_x, end_y, normalized)?;
    Ok(GestureCoordinates {
        start_x,
        start_y,
        end_x,
        end_y,
        duration_ms,
    })
}

#[allow(clippy::too_many_arguments)]
fn pinch_frames(
    bridge: &NativeBridge,
    udid: &str,
    center_x: Option<f64>,
    center_y: Option<f64>,
    start_distance: f64,
    end_distance: f64,
    angle_degrees: f64,
    normalized: bool,
    steps: u32,
) -> Result<Vec<MultiTouchFrame>, crate::error::AppError> {
    if start_distance < 0.0 || end_distance < 0.0 {
        return Err(crate::error::AppError::bad_request(
            "Pinch distances must be non-negative.",
        ));
    }
    let (width, height) = gesture_surface_size(bridge, udid, normalized);
    let center_x = center_x.unwrap_or(width / 2.0);
    let center_y = center_y.unwrap_or(height / 2.0);
    let angle = angle_degrees.to_radians();
    let unit_x = angle.cos();
    let unit_y = angle.sin();
    let count = steps.max(2);
    let mut frames = Vec::with_capacity(count as usize);
    for step in 0..count {
        let t = if count == 1 {
            1.0
        } else {
            f64::from(step) / f64::from(count - 1)
        };
        let distance = lerp(start_distance, end_distance, t) / 2.0;
        let p1x = center_x - unit_x * distance;
        let p1y = center_y - unit_y * distance;
        let p2x = center_x + unit_x * distance;
        let p2y = center_y + unit_y * distance;
        let (x1, y1) = resolve_touch_point(bridge, udid, p1x, p1y, normalized)?;
        let (x2, y2) = resolve_touch_point(bridge, udid, p2x, p2y, normalized)?;
        frames.push(MultiTouchFrame { x1, y1, x2, y2 });
    }
    Ok(frames)
}

fn rotate_gesture_frames(
    bridge: &NativeBridge,
    udid: &str,
    request: RotateGestureRequest,
) -> Result<Vec<MultiTouchFrame>, crate::error::AppError> {
    if request.radius < 0.0 {
        return Err(crate::error::AppError::bad_request(
            "Rotate gesture radius must be non-negative.",
        ));
    }
    let (width, height) = gesture_surface_size(bridge, udid, request.normalized);
    let center_x = request.center_x.unwrap_or(width / 2.0);
    let center_y = request.center_y.unwrap_or(height / 2.0);
    let count = request.steps.max(2);
    let mut frames = Vec::with_capacity(count as usize);
    for step in 0..count {
        let t = if count == 1 {
            1.0
        } else {
            f64::from(step) / f64::from(count - 1)
        };
        let angle = (request.degrees * t).to_radians();
        let unit_x = angle.cos();
        let unit_y = angle.sin();
        let p1x = center_x - unit_x * request.radius;
        let p1y = center_y - unit_y * request.radius;
        let p2x = center_x + unit_x * request.radius;
        let p2y = center_y + unit_y * request.radius;
        let (x1, y1) = resolve_touch_point(bridge, udid, p1x, p1y, request.normalized)?;
        let (x2, y2) = resolve_touch_point(bridge, udid, p2x, p2y, request.normalized)?;
        frames.push(MultiTouchFrame { x1, y1, x2, y2 });
    }
    Ok(frames)
}

fn gesture_surface_size(bridge: &NativeBridge, udid: &str, normalized: bool) -> (f64, f64) {
    if normalized {
        return (1.0, 1.0);
    }
    accessibility_root_size(bridge, udid)
        .or_else(|| chrome_screen_size(bridge, udid))
        .unwrap_or((390.0, 844.0))
}

fn parse_key_list(value: &str) -> Result<Vec<u16>, crate::error::AppError> {
    let mut keys = Vec::new();
    for token in value
        .split(',')
        .map(str::trim)
        .filter(|token| !token.is_empty())
    {
        keys.push(parse_hid_key(token)?);
    }
    if keys.is_empty() {
        return Err(crate::error::AppError::bad_request(
            "Key sequence must include at least one key.",
        ));
    }
    Ok(keys)
}

fn parse_hid_key(value: &str) -> Result<u16, crate::error::AppError> {
    if let Ok(code) = value.parse::<u16>() {
        return Ok(code);
    }
    let key = match value.to_lowercase().as_str() {
        "enter" | "return" => 40,
        "escape" | "esc" => 41,
        "backspace" | "delete" => 42,
        "tab" => 43,
        "space" => 44,
        "right" | "arrow-right" => 79,
        "left" | "arrow-left" => 80,
        "down" | "arrow-down" => 81,
        "up" | "arrow-up" => 82,
        "home" => 74,
        "end" => 77,
        other if other.len() == 1 => hid_for_character(other.chars().next().unwrap())
            .map(|(key, _)| key)
            .ok_or_else(|| {
                crate::error::AppError::bad_request(format!("Unsupported key `{value}`."))
            })?,
        _ => {
            return Err(crate::error::AppError::bad_request(format!(
                "Unsupported key `{value}`."
            )))
        }
    };
    Ok(key)
}

fn parse_modifier_mask(value: &str) -> Result<u32, crate::error::AppError> {
    let mut mask = 0;
    for token in value
        .split(',')
        .map(str::trim)
        .filter(|token| !token.is_empty())
    {
        mask |= match token.to_lowercase().as_str() {
            "shift" | "225" | "left-shift" => 1,
            "ctrl" | "control" | "224" | "left-control" => 1 << 1,
            "alt" | "option" | "226" | "left-option" => 1 << 2,
            "cmd" | "command" | "meta" | "227" | "left-command" => 1 << 3,
            "caps" | "caps-lock" | "57" => 1 << 4,
            other => {
                return Err(crate::error::AppError::bad_request(format!(
                    "Unsupported modifier `{other}`."
                )))
            }
        };
    }
    Ok(mask)
}

fn run_batch(
    bridge: &NativeBridge,
    udid: &str,
    steps: Vec<String>,
    file: Option<PathBuf>,
    use_stdin: bool,
    continue_on_error: bool,
) -> anyhow::Result<Value> {
    let step_lines = read_batch_steps(steps, file, use_stdin)?;
    let mut results = Vec::new();
    let mut failures = Vec::new();
    for (index, line) in step_lines.iter().enumerate() {
        let result = run_batch_step(bridge, udid, line);
        match result {
            Ok(action) => {
                results.push(serde_json::json!({ "index": index, "ok": true, "action": action }))
            }
            Err(error) => {
                let message = error.to_string();
                results.push(serde_json::json!({ "index": index, "ok": false, "error": message }));
                failures.push(message);
                if !continue_on_error {
                    return Err(crate::error::AppError::bad_request(format!(
                        "Batch step {} failed: {}",
                        index + 1,
                        failures.last().unwrap()
                    ))
                    .into());
                }
            }
        }
    }
    Ok(serde_json::json!({
        "ok": failures.is_empty(),
        "steps": results,
        "failureCount": failures.len()
    }))
}

fn read_batch_steps(
    steps: Vec<String>,
    file: Option<PathBuf>,
    use_stdin: bool,
) -> anyhow::Result<Vec<String>> {
    let source_count =
        usize::from(!steps.is_empty()) + usize::from(file.is_some()) + usize::from(use_stdin);
    if source_count != 1 {
        return Err(crate::error::AppError::bad_request(
            "Specify exactly one batch source: --step, --file, or --stdin.",
        )
        .into());
    }
    let raw = if use_stdin {
        let mut buffer = String::new();
        io::stdin().read_to_string(&mut buffer)?;
        buffer
    } else if let Some(file) = file {
        fs::read_to_string(file)?
    } else {
        return Ok(steps);
    };
    Ok(raw
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect())
}

fn batch_lines_to_json_steps(step_lines: &[String]) -> anyhow::Result<Vec<Value>> {
    step_lines
        .iter()
        .map(|line| batch_line_to_json_step(line))
        .collect()
}

fn batch_line_to_json_step(line: &str) -> anyhow::Result<Value> {
    let tokens = tokenize_step(line)?;
    let Some(command) = tokens.first().map(String::as_str) else {
        return Err(crate::error::AppError::bad_request("Empty batch step.").into());
    };
    let args = parse_step_options(&tokens[1..]);
    let value = match command {
        "sleep" => serde_json::json!({
            "action": "sleep",
            "seconds": tokens.get(1).and_then(|value| value.parse::<f64>().ok()).unwrap_or(0.0),
        }),
        "tap" => serde_json::json!({
            "action": "tap",
            "x": args.value("x").and_then(|value| value.parse::<f64>().ok()),
            "y": args.value("y").and_then(|value| value.parse::<f64>().ok()),
            "normalized": args.flag("normalized"),
            "selector": {
                "id": args.value("id"),
                "label": args.value("label"),
                "value": args.value("value"),
                "elementType": args.value("element-type"),
            },
            "durationMs": args.value("duration-ms").and_then(|value| value.parse::<u64>().ok()).unwrap_or(60),
            "waitTimeoutMs": args.value("wait-timeout-ms").and_then(|value| value.parse::<u64>().ok()).unwrap_or(0),
            "pollMs": args.value("poll-interval-ms").and_then(|value| value.parse::<u64>().ok()).unwrap_or(100),
        }),
        "key" => serde_json::json!({
            "action": "key",
            "keyCode": parse_hid_key(tokens.get(1).map(String::as_str).unwrap_or(""))?,
            "modifiers": args.value("modifiers").and_then(|value| value.parse::<u32>().ok()).unwrap_or(0),
        }),
        "key-sequence" => serde_json::json!({
            "action": "keySequence",
            "keyCodes": parse_key_list(args.value("keycodes").or_else(|| args.value("keys")).unwrap_or(""))?,
            "delayMs": args.value("delay-ms").and_then(|value| value.parse::<u64>().ok()).unwrap_or(0),
        }),
        "key-combo" => serde_json::json!({
            "action": "key",
            "keyCode": parse_hid_key(args.value("key").unwrap_or(""))?,
            "modifiers": parse_modifier_mask(args.value("modifiers").unwrap_or(""))?,
        }),
        "touch" => {
            let x = args
                .value("x")
                .or_else(|| tokens.get(1).map(String::as_str))
                .and_then(|value| value.parse::<f64>().ok());
            let y = args
                .value("y")
                .or_else(|| tokens.get(2).map(String::as_str))
                .and_then(|value| value.parse::<f64>().ok());
            serde_json::json!({
                "action": "touch",
                "x": x.unwrap_or(0.0),
                "y": y.unwrap_or(0.0),
                "phase": args.value("phase").unwrap_or("began"),
                "down": args.flag("down"),
                "up": args.flag("up"),
                "delayMs": args.value("delay-ms").and_then(|value| value.parse::<u64>().ok()).unwrap_or(100),
            })
        }
        "swipe" => {
            let value = |name: &str, index: usize| {
                args.value(name)
                    .or_else(|| tokens.get(index).map(String::as_str))
                    .and_then(|value| value.parse::<f64>().ok())
            };
            serde_json::json!({
                "action": "swipe",
                "startX": value("start-x", 1).unwrap_or(0.5),
                "startY": value("start-y", 2).unwrap_or(0.75),
                "endX": value("end-x", 3).unwrap_or(0.5),
                "endY": value("end-y", 4).unwrap_or(0.25),
                "durationMs": args.value("duration-ms").and_then(|value| value.parse::<u64>().ok()).unwrap_or(350),
                "steps": args.value("steps").and_then(|value| value.parse::<u32>().ok()).unwrap_or(12),
            })
        }
        "gesture" => serde_json::json!({
            "action": "gesture",
            "preset": tokens.get(1).map(String::as_str).unwrap_or("scroll-down"),
            "durationMs": args.value("duration-ms").and_then(|value| value.parse::<u64>().ok()),
            "delta": args.value("delta").and_then(|value| value.parse::<f64>().ok()),
            "steps": args.value("steps").and_then(|value| value.parse::<u32>().ok()).unwrap_or(12),
        }),
        "type" => serde_json::json!({
            "action": "type",
            "text": tokens.get(1).map(String::as_str).unwrap_or(""),
            "delayMs": args.value("delay-ms").and_then(|value| value.parse::<u64>().ok()).unwrap_or(12),
        }),
        "button" => serde_json::json!({
            "action": "button",
            "button": tokens.get(1).map(String::as_str).unwrap_or(""),
            "durationMs": args.value("duration-ms").and_then(|value| value.parse::<u32>().ok()).unwrap_or(0),
        }),
        "home" => serde_json::json!({ "action": "home" }),
        "dismiss-keyboard" => serde_json::json!({ "action": "dismissKeyboard" }),
        "app-switcher" => serde_json::json!({ "action": "appSwitcher" }),
        "rotate-left" => serde_json::json!({ "action": "rotateLeft" }),
        "rotate-right" => serde_json::json!({ "action": "rotateRight" }),
        "toggle-appearance" => serde_json::json!({ "action": "toggleAppearance" }),
        "launch" => serde_json::json!({
            "action": "launch",
            "bundleId": tokens.get(1).map(String::as_str).unwrap_or(""),
        }),
        "open-url" => serde_json::json!({
            "action": "openUrl",
            "url": tokens.get(1).map(String::as_str).unwrap_or(""),
        }),
        other => {
            return Err(crate::error::AppError::bad_request(format!(
                "Unsupported daemon batch step `{other}`."
            ))
            .into())
        }
    };
    Ok(value)
}

fn run_batch_step(
    bridge: &NativeBridge,
    udid: &str,
    line: &str,
) -> Result<&'static str, crate::error::AppError> {
    let tokens = tokenize_step(line)?;
    let Some(command) = tokens.first().map(String::as_str) else {
        return Err(crate::error::AppError::bad_request("Empty batch step."));
    };
    match command {
        "sleep" => {
            let seconds = tokens
                .get(1)
                .ok_or_else(|| crate::error::AppError::bad_request("sleep requires seconds."))?
                .parse::<f64>()
                .map_err(|_| {
                    crate::error::AppError::bad_request("sleep seconds must be numeric.")
                })?;
            sleep_ms((seconds * 1000.0).max(0.0) as u64);
            Ok("sleep")
        }
        "tap" => {
            let args = parse_step_options(&tokens[1..]);
            let x = args.value("x").and_then(|value| value.parse::<f64>().ok());
            let y = args.value("y").and_then(|value| value.parse::<f64>().ok());
            let normalized = args.flag("normalized");
            let duration_ms = args
                .value("duration-ms")
                .and_then(|value| value.parse().ok())
                .unwrap_or(60);
            let target = resolve_tap_target(
                bridge,
                udid,
                TapTargetRequest {
                    x,
                    y,
                    normalized,
                    selector: ElementSelector {
                        id: args.value("id").map(str::to_owned),
                        label: args.value("label").map(str::to_owned),
                        value: args.value("value").map(str::to_owned),
                        element_type: args.value("element-type").map(str::to_owned),
                    },
                    wait_timeout_ms: args
                        .value("wait-timeout-ms")
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(0),
                    poll_interval_ms: args
                        .value("poll-interval-ms")
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(100),
                },
            )?;
            if let Some(input) = target.input.as_ref() {
                perform_tap_with_input(input, target.x, target.y, duration_ms)?;
            } else {
                perform_tap(bridge, udid, target.x, target.y, duration_ms)?;
            }
            Ok("tap")
        }
        "swipe" => {
            let args = parse_step_options(&tokens[1..]);
            let start_x = required_f64(&args, "start-x")?;
            let start_y = required_f64(&args, "start-y")?;
            let end_x = required_f64(&args, "end-x")?;
            let end_y = required_f64(&args, "end-y")?;
            let normalized = args.flag("normalized");
            let (start_x, start_y) =
                resolve_touch_point(bridge, udid, start_x, start_y, normalized)?;
            let (end_x, end_y) = resolve_touch_point(bridge, udid, end_x, end_y, normalized)?;
            perform_swipe(
                bridge,
                udid,
                GestureCoordinates {
                    start_x,
                    start_y,
                    end_x,
                    end_y,
                    duration_ms: args
                        .value("duration-ms")
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(350),
                },
                args.value("steps")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(12),
            )?;
            Ok("swipe")
        }
        "gesture" => {
            let preset = tokens
                .get(1)
                .ok_or_else(|| crate::error::AppError::bad_request("gesture requires a preset."))?;
            let args = parse_step_options(&tokens[2..]);
            let gesture = gesture_coordinates(
                bridge,
                udid,
                preset,
                args.value("screen-width")
                    .and_then(|value| value.parse().ok()),
                args.value("screen-height")
                    .and_then(|value| value.parse().ok()),
                args.flag("normalized"),
                args.value("delta").and_then(|value| value.parse().ok()),
            )?;
            perform_swipe(
                bridge,
                udid,
                GestureCoordinates {
                    duration_ms: args
                        .value("duration-ms")
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(gesture.duration_ms),
                    ..gesture
                },
                12,
            )?;
            Ok("gesture")
        }
        "pinch" => {
            let args = parse_step_options(&tokens[1..]);
            let frames = pinch_frames(
                bridge,
                udid,
                args.value("center-x").and_then(|value| value.parse().ok()),
                args.value("center-y").and_then(|value| value.parse().ok()),
                args.value("start-distance")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(160.0),
                args.value("end-distance")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(80.0),
                args.value("angle-degrees")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(0.0),
                args.flag("normalized"),
                args.value("steps")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(12),
            )?;
            run_multitouch_frames(
                bridge,
                udid,
                frames,
                args.value("duration-ms")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(450),
            )?;
            Ok("pinch")
        }
        "rotate-gesture" => {
            let args = parse_step_options(&tokens[1..]);
            let frames = rotate_gesture_frames(
                bridge,
                udid,
                RotateGestureRequest {
                    center_x: args.value("center-x").and_then(|value| value.parse().ok()),
                    center_y: args.value("center-y").and_then(|value| value.parse().ok()),
                    radius: args
                        .value("radius")
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(100.0),
                    degrees: args
                        .value("degrees")
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(90.0),
                    normalized: args.flag("normalized"),
                    steps: args
                        .value("steps")
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(12),
                },
            )?;
            run_multitouch_frames(
                bridge,
                udid,
                frames,
                args.value("duration-ms")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(500),
            )?;
            Ok("rotate-gesture")
        }
        "touch" => {
            let args = parse_step_options(&tokens[1..]);
            let x = required_f64(&args, "x")?;
            let y = required_f64(&args, "y")?;
            let normalized = args.flag("normalized");
            let (x, y) = resolve_touch_point(bridge, udid, x, y, normalized)?;
            if args.flag("down") || args.flag("up") {
                let input = bridge.create_input_session(udid)?;
                if args.flag("down") {
                    input.send_touch(x, y, "began")?;
                }
                if args.flag("down") && args.flag("up") {
                    sleep_ms(
                        args.value("delay-ms")
                            .and_then(|value| value.parse().ok())
                            .unwrap_or(100),
                    );
                }
                if args.flag("up") {
                    input.send_touch(x, y, "ended")?;
                }
            } else {
                bridge.send_touch(udid, x, y, args.value("phase").unwrap_or("began"))?;
            }
            Ok("touch")
        }
        "type" => {
            let text = tokens.get(1).cloned().unwrap_or_default();
            type_text(bridge, udid, &text, 12)?;
            Ok("type")
        }
        "button" => {
            let button = tokens
                .get(1)
                .ok_or_else(|| crate::error::AppError::bad_request("button requires a name."))?;
            bridge.press_button(udid, button, 0)?;
            Ok("button")
        }
        "key" => {
            let key = tokens.get(1).ok_or_else(|| {
                crate::error::AppError::bad_request("key requires a keycode or key name.")
            })?;
            bridge.send_key(udid, parse_hid_key(key)?, 0)?;
            Ok("key")
        }
        "key-sequence" => {
            let args = parse_step_options(&tokens[1..]);
            let keys = parse_key_list(
                args.value("keycodes")
                    .or_else(|| args.value("keys"))
                    .ok_or_else(|| {
                        crate::error::AppError::bad_request("key-sequence requires --keycodes.")
                    })?,
            )?;
            let input = bridge.create_input_session(udid)?;
            for (index, key) in keys.iter().enumerate() {
                input.send_key(*key, 0)?;
                if index + 1 < keys.len() {
                    sleep_ms(
                        args.value("delay-ms")
                            .and_then(|value| value.parse().ok())
                            .unwrap_or(100),
                    );
                }
            }
            Ok("key-sequence")
        }
        "key-combo" => {
            let args = parse_step_options(&tokens[1..]);
            let modifiers = args.value("modifiers").ok_or_else(|| {
                crate::error::AppError::bad_request("key-combo requires --modifiers.")
            })?;
            let key = args
                .value("key")
                .ok_or_else(|| crate::error::AppError::bad_request("key-combo requires --key."))?;
            bridge.send_key(udid, parse_hid_key(key)?, parse_modifier_mask(modifiers)?)?;
            Ok("key-combo")
        }
        _ => Err(crate::error::AppError::bad_request(format!(
            "Unsupported batch step `{command}`."
        ))),
    }
}

#[derive(Default)]
struct StepOptions {
    values: Vec<(String, String)>,
    flags: Vec<String>,
}

impl StepOptions {
    fn value(&self, key: &str) -> Option<&str> {
        self.values
            .iter()
            .rev()
            .find(|(candidate, _)| candidate == key)
            .map(|(_, value)| value.as_str())
    }

    fn flag(&self, key: &str) -> bool {
        self.flags.iter().any(|candidate| candidate == key)
    }
}

fn parse_step_options(tokens: &[String]) -> StepOptions {
    let mut options = StepOptions::default();
    let mut index = 0;
    while index < tokens.len() {
        let token = &tokens[index];
        if let Some(stripped) = token.strip_prefix("--") {
            if let Some((key, value)) = stripped.split_once('=') {
                options.values.push((key.to_owned(), value.to_owned()));
            } else if index + 1 < tokens.len() && !tokens[index + 1].starts_with("--") {
                options
                    .values
                    .push((stripped.to_owned(), tokens[index + 1].clone()));
                index += 1;
            } else {
                options.flags.push(stripped.to_owned());
            }
        } else if let Some(stripped) = token.strip_prefix('-') {
            if index + 1 < tokens.len() && !tokens[index + 1].starts_with('-') {
                options
                    .values
                    .push((stripped.to_owned(), tokens[index + 1].clone()));
                index += 1;
            }
        }
        index += 1;
    }
    options
}

fn required_f64(args: &StepOptions, key: &str) -> Result<f64, crate::error::AppError> {
    args.value(key)
        .ok_or_else(|| crate::error::AppError::bad_request(format!("Missing --{key}.")))?
        .parse::<f64>()
        .map_err(|_| crate::error::AppError::bad_request(format!("--{key} must be numeric.")))
}

fn tokenize_step(line: &str) -> Result<Vec<String>, crate::error::AppError> {
    enum State {
        Normal,
        Single,
        Double,
    }
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut state = State::Normal;
    let mut escaping = false;
    let mut saw_boundary = false;
    for character in line.chars() {
        match state {
            State::Normal => {
                if escaping {
                    current.push(character);
                    escaping = false;
                    saw_boundary = true;
                } else if character == '\\' {
                    escaping = true;
                } else if character == '\'' {
                    state = State::Single;
                    saw_boundary = true;
                } else if character == '"' {
                    state = State::Double;
                    saw_boundary = true;
                } else if character.is_whitespace() {
                    if !current.is_empty() || saw_boundary {
                        tokens.push(std::mem::take(&mut current));
                        saw_boundary = false;
                    }
                } else {
                    current.push(character);
                    saw_boundary = true;
                }
            }
            State::Single => {
                if character == '\'' {
                    state = State::Normal;
                } else {
                    current.push(character);
                }
            }
            State::Double => {
                if escaping {
                    current.push(character);
                    escaping = false;
                } else if character == '\\' {
                    escaping = true;
                } else if character == '"' {
                    state = State::Normal;
                } else {
                    current.push(character);
                }
            }
        }
    }
    if escaping {
        return Err(crate::error::AppError::bad_request(
            "Dangling escape in batch step.",
        ));
    }
    if !matches!(state, State::Normal) {
        return Err(crate::error::AppError::bad_request(
            "Unterminated quote in batch step.",
        ));
    }
    if !current.is_empty() || saw_boundary {
        tokens.push(current);
    }
    Ok(tokens)
}

fn println_json(value: &Value) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn parse_point(value: &str) -> Result<(f64, f64), String> {
    let (x, y) = value
        .split_once(',')
        .ok_or_else(|| "point must be in the form x,y".to_owned())?;
    let x = x
        .trim()
        .parse::<f64>()
        .map_err(|_| "point x must be a number".to_owned())?;
    let y = y
        .trim()
        .parse::<f64>()
        .map_err(|_| "point y must be a number".to_owned())?;
    if !x.is_finite() || !y.is_finite() || x < 0.0 || y < 0.0 {
        return Err("point coordinates must be finite non-negative numbers".to_owned());
    }
    Ok((x, y))
}

fn resolve_touch_point(
    bridge: &NativeBridge,
    udid: &str,
    x: f64,
    y: f64,
    normalized: bool,
) -> Result<(f64, f64), crate::error::AppError> {
    if !x.is_finite() || !y.is_finite() || x < 0.0 || y < 0.0 {
        return Err(crate::error::AppError::bad_request(
            "Touch coordinates must be finite non-negative numbers.",
        ));
    }
    if normalized {
        return Ok((x.clamp(0.0, 1.0), y.clamp(0.0, 1.0)));
    }
    let (width, height) = accessibility_root_size(bridge, udid)
        .or_else(|| chrome_screen_size(bridge, udid))
        .unwrap_or((1.0, 1.0));
    Ok(((x / width).clamp(0.0, 1.0), (y / height).clamp(0.0, 1.0)))
}

fn accessibility_root_size(bridge: &NativeBridge, udid: &str) -> Option<(f64, f64)> {
    let snapshot = bridge.accessibility_snapshot(udid, None).ok()?;
    let frame = snapshot.get("roots")?.as_array()?.first()?.get("frame")?;
    let width = frame.get("width")?.as_f64()?;
    let height = frame.get("height")?.as_f64()?;
    (width > 0.0 && height > 0.0).then_some((width, height))
}

fn chrome_screen_size(bridge: &NativeBridge, udid: &str) -> Option<(f64, f64)> {
    let profile = bridge.chrome_profile(udid).ok()?;
    let width = profile.screen_width;
    let height = profile.screen_height;
    (width > 0.0 && height > 0.0).then_some((width, height))
}

fn lerp(start: f64, end: f64, t: f64) -> f64 {
    start + (end - start) * t
}

fn hid_for_character(character: char) -> Option<(u16, u32)> {
    let shift: u32 = 1;
    let mapping = match character {
        'a'..='z' => (character as u16 - b'a' as u16 + 4, 0),
        'A'..='Z' => (character as u16 - b'A' as u16 + 4, shift),
        '1' => (30, 0),
        '!' => (30, shift),
        '2' => (31, 0),
        '@' => (31, shift),
        '3' => (32, 0),
        '#' => (32, shift),
        '4' => (33, 0),
        '$' => (33, shift),
        '5' => (34, 0),
        '%' => (34, shift),
        '6' => (35, 0),
        '^' => (35, shift),
        '7' => (36, 0),
        '&' => (36, shift),
        '8' => (37, 0),
        '*' => (37, shift),
        '9' => (38, 0),
        '(' => (38, shift),
        '0' => (39, 0),
        ')' => (39, shift),
        '\n' | '\r' => (40, 0),
        '\t' => (43, 0),
        ' ' => (44, 0),
        '-' => (45, 0),
        '_' => (45, shift),
        '=' => (46, 0),
        '+' => (46, shift),
        '[' => (47, 0),
        '{' => (47, shift),
        ']' => (48, 0),
        '}' => (48, shift),
        '\\' => (49, 0),
        '|' => (49, shift),
        ';' => (51, 0),
        ':' => (51, shift),
        '\'' => (52, 0),
        '"' => (52, shift),
        '`' => (53, 0),
        '~' => (53, shift),
        ',' => (54, 0),
        '<' => (54, shift),
        '.' => (55, 0),
        '>' => (55, shift),
        '/' => (56, 0),
        '?' => (56, shift),
        _ => return None,
    };
    Some(mapping)
}

#[allow(clippy::too_many_arguments)]
async fn serve(
    port: u16,
    bind: IpAddr,
    advertise_host: Option<String>,
    client_root: Option<PathBuf>,
    video_codec: VideoCodecMode,
    low_latency: bool,
    access_token: Option<String>,
    pairing_code: Option<String>,
) -> anyhow::Result<()> {
    let root = match client_root {
        Some(root) => root,
        None => default_client_root()?,
    };
    let config = Config::new(
        port,
        root,
        bind,
        advertise_host,
        video_codec.as_env_value().to_owned(),
        low_latency,
        access_token,
        pairing_code,
    );
    let metrics = Arc::new(Metrics::default());
    let bridge = NativeBridge;
    let registry = SessionRegistry::new(bridge, metrics.clone());
    let logs = LogRegistry::default();
    let inspectors = InspectorHub::default();
    let (wt_runtime, wt_endpoint) = transport::webtransport::prepare(&config).await?;
    let state = AppState {
        config: config.clone(),
        registry,
        logs,
        inspectors,
        metrics,
        certificate_hash_hex: wt_runtime.certificate_hash_hex.clone(),
    };

    let http_router = app_router(
        state.clone(),
        config.client_root.clone(),
        config.access_token.clone(),
    );
    let http_listener = tokio::net::TcpListener::bind(config.http_addr())
        .await
        .with_context(|| format!("bind HTTP listener on {}", config.http_addr()))?;
    let health_heartbeat = Arc::new(AtomicU64::new(now_secs()));
    start_server_health_watchdog(config.http_addr(), health_heartbeat.clone());

    info!("HTTP listening on http://{}", config.http_addr());
    info!(
        "WebTransport listening on {}",
        wt_runtime.endpoint_url_template
    );
    info!("Serving client from {}", config.client_root.display());
    info!("API access token: {}", config.access_token);
    if config.bind_ip.is_unspecified() && config.advertise_host == Ipv4Addr::LOCALHOST.to_string() {
        warn!(
            "Server is listening on all interfaces, but WebTransport is still advertised as localhost. \
Use --advertise-host <LAN-IP-or-DNS-name> for remote browser access."
        );
    }

    let http_task = tokio::spawn(async move {
        axum::serve(
            http_listener,
            http_router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .context("serve HTTP")
    });
    let health_task = tokio::spawn(async move {
        loop {
            health_heartbeat.store(now_secs(), Ordering::Relaxed);
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
    let wt_task =
        tokio::spawn(async move { transport::webtransport::serve(wt_endpoint, state).await });

    let (_terminal_mode, quit_key) = start_quit_key_listener();
    let quit_key_signal = async move {
        match quit_key {
            Some(receiver) => {
                let _ = receiver.await;
            }
            None => std::future::pending::<()>().await,
        }
    };
    tokio::pin!(quit_key_signal);

    tokio::select! {
        result = http_task => result??,
        result = health_task => result.context("server health heartbeat task panicked")?,
        result = wt_task => result??,
        _ = tokio::signal::ctrl_c() => {}
        _ = &mut quit_key_signal => {}
    }

    Ok(())
}

fn app_router(state: AppState, client_root: PathBuf, access_token: String) -> Router {
    router(state).fallback(
        move |axum::extract::ConnectInfo(address): axum::extract::ConnectInfo<SocketAddr>,
              method,
              uri| {
            let access_token = address.ip().is_loopback().then(|| access_token.clone());
            static_files::serve_static(client_root.clone(), method, uri, access_token)
        },
    )
}

#[cfg(unix)]
struct TerminalInputMode {
    fd: libc::c_int,
    original: libc::termios,
}

#[cfg(unix)]
impl TerminalInputMode {
    fn enable_quit_key_mode() -> io::Result<Self> {
        let fd = libc::STDIN_FILENO;
        let mut original = unsafe { std::mem::zeroed::<libc::termios>() };
        if unsafe { libc::tcgetattr(fd, &mut original) } != 0 {
            return Err(io::Error::last_os_error());
        }

        let mut raw = original;
        raw.c_lflag &= !(libc::ICANON | libc::ECHO);
        raw.c_cc[libc::VMIN] = 1;
        raw.c_cc[libc::VTIME] = 0;
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } != 0 {
            return Err(io::Error::last_os_error());
        }

        Ok(Self { fd, original })
    }
}

#[cfg(unix)]
impl Drop for TerminalInputMode {
    fn drop(&mut self) {
        let _ = unsafe { libc::tcsetattr(self.fd, libc::TCSANOW, &self.original) };
    }
}

#[cfg(not(unix))]
struct TerminalInputMode;

fn start_quit_key_listener() -> (
    Option<TerminalInputMode>,
    Option<tokio::sync::oneshot::Receiver<()>>,
) {
    if !io::stdin().is_terminal() {
        return (None, None);
    }

    #[cfg(unix)]
    let terminal_mode = match TerminalInputMode::enable_quit_key_mode() {
        Ok(mode) => mode,
        Err(_) => return (None, None),
    };

    #[cfg(not(unix))]
    let terminal_mode = TerminalInputMode;

    let (sender, receiver) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let stdin = io::stdin();
        let mut stdin = stdin.lock();
        let mut byte = [0u8; 1];
        loop {
            match stdin.read(&mut byte) {
                Ok(0) => return,
                Ok(_) if byte[0] == b'q' || byte[0] == b'Q' => {
                    let _ = sender.send(());
                    return;
                }
                Ok(_) => {}
                Err(_) => return,
            }
        }
    });

    (Some(terminal_mode), Some(receiver))
}

fn default_client_root() -> anyhow::Result<PathBuf> {
    let current_exe = std::env::current_exe().context("resolve current executable path")?;

    if let Some(package_root) = current_exe.parent().and_then(|parent| parent.parent()) {
        let packaged_client = package_root.join("client").join("dist");
        if packaged_client.is_dir() {
            return Ok(packaged_client);
        }
    }

    Ok(std::env::current_dir()?.join("client").join("dist"))
}

#[cfg(test)]
mod tests {
    use super::normalize_accessibility_point_for_display;

    #[test]
    fn selector_tap_keeps_matching_orientation_coordinates() {
        assert_eq!(
            normalize_accessibility_point_for_display(240.0, 160.0, 480.0, 320.0, 1200.0, 800.0),
            (0.5, 0.5)
        );
    }

    #[test]
    fn selector_tap_transposes_swapped_orientation_coordinates() {
        assert_eq!(
            normalize_accessibility_point_for_display(240.0, 226.0, 480.0, 320.0, 800.0, 1200.0),
            (0.70625, 0.5)
        );
    }
}
