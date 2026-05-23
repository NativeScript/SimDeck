mod accessibility;
mod android;
mod api;
mod auth;
mod config;
mod core_simulator;
mod devtools;
mod error;
mod inspector;
mod logging;
mod logs;
mod metrics;
mod native;
mod performance;
mod service;
mod simulators;
mod static_files;
mod transport;
mod webkit;

use accessibility::{interactive_accessibility_snapshot, AccessibilitySource};
use anyhow::Context;
use api::routes::{router, AppState};
use axum::Router;
use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};
use config::{Config, ServerKind};
use inspector::{InspectorHub, InspectorRegistryAdvertisement};
use logs::LogRegistry;
use metrics::counters::Metrics;
use native::bridge::{
    tvos_remote_key_for_touch_motion, tvos_remote_key_for_touch_phase, NativeBridge,
    NativeInputSession, HID_KEY_ARROW_DOWN, HID_KEY_ARROW_LEFT, HID_KEY_ARROW_RIGHT,
    HID_KEY_ARROW_UP, HID_KEY_ENTER,
};
use native::ffi;
use performance::PerformanceRegistry;
use qrcode::{render::unicode, QrCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_yaml::Value as YamlValue;
use simulators::registry::SessionRegistry;
use std::collections::{hash_map::DefaultHasher, HashMap, HashSet};
use std::env;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::{self, IsTerminal, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, UdpSocket};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command as ProcessCommand, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use tracing::{info, warn};

const RECOVERABLE_RESTART_EXIT_CODE: i32 = 75;
const SUPERVISED_DAEMON_METADATA_PID_ENV: &str = "SIMDECK_DAEMON_METADATA_PID";
const RESTART_ON_CORE_SIMULATOR_MISMATCH_ENV: &str = "SIMDECK_RESTART_ON_CORE_SIMULATOR_MISMATCH";
const SERVER_FD_RESTART_THRESHOLD: usize = 4096;
const SERVER_HEALTH_WATCHDOG_INITIAL_DELAY: Duration = Duration::from_secs(15);
const SERVER_HEALTH_WATCHDOG_INTERVAL: Duration = Duration::from_secs(5);
const SERVER_HEALTH_WATCHDOG_PROBE_TIMEOUT: Duration = Duration::from_secs(3);
const SERVER_HEALTH_WATCHDOG_STALE_HEARTBEAT: Duration = Duration::from_secs(60);
const SERVER_HEALTH_WATCHDOG_FAILURE_THRESHOLD: usize = 12;
const SERVER_HEALTH_WATCHDOG_HTTP_FAILURE_THRESHOLD: usize = 3;
const SERVICE_PORT: u16 = 4310;
const DAEMON_PORT_START: u16 = 4311;

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
    #[arg(
        long,
        global = true,
        value_name = "SIMULATOR_NAME_OR_UDID",
        help = "Override the simulator target for this command"
    )]
    device: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Ui {
        #[arg(long, default_value_t = DAEMON_PORT_START)]
        port: u16,
        #[arg(long, default_value_t = IpAddr::V4(Ipv4Addr::LOCALHOST))]
        bind: IpAddr,
        #[arg(long)]
        advertise_host: Option<String>,
        #[arg(long)]
        client_root: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = VideoCodecMode::Auto)]
        video_codec: VideoCodecMode,
        #[arg(long)]
        low_latency: bool,
        #[arg(long, value_enum)]
        stream_quality: Option<StreamQualityProfileArg>,
        #[arg(long, value_parser = clap::value_parser!(u32).range(15..=240))]
        local_stream_fps: Option<u32>,
        #[arg(long)]
        open: bool,
    },
    Pair {
        #[arg(
            long,
            help = "Defaults to the existing service port, or 4310 when the service is not installed"
        )]
        port: Option<u16>,
        #[arg(long, default_value_t = IpAddr::V4(Ipv4Addr::UNSPECIFIED))]
        bind: IpAddr,
        #[arg(long)]
        advertise_host: Option<String>,
        #[arg(long)]
        client_root: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = VideoCodecMode::Auto)]
        video_codec: VideoCodecMode,
        #[arg(long)]
        low_latency: bool,
        #[arg(long, value_enum)]
        stream_quality: Option<StreamQualityProfileArg>,
        #[arg(long, value_parser = clap::value_parser!(u32).range(15..=240))]
        local_stream_fps: Option<u32>,
        #[arg(long)]
        json: bool,
    },
    Daemon {
        #[command(subcommand)]
        command: DaemonCommand,
    },
    Studio {
        #[command(subcommand)]
        command: StudioCommand,
    },
    Provider {
        #[command(subcommand)]
        command: ProviderCommand,
    },
    Maestro {
        #[command(subcommand)]
        command: MaestroCommand,
    },
    #[command(hide = true)]
    Serve {
        #[arg(long, default_value_t = SERVICE_PORT)]
        port: u16,
        #[arg(long, default_value_t = IpAddr::V4(Ipv4Addr::LOCALHOST))]
        bind: IpAddr,
        #[arg(long)]
        advertise_host: Option<String>,
        #[arg(long)]
        client_root: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = VideoCodecMode::Auto)]
        video_codec: VideoCodecMode,
        #[arg(long)]
        low_latency: bool,
        #[arg(long, value_enum)]
        stream_quality: Option<StreamQualityProfileArg>,
        #[arg(long, value_parser = clap::value_parser!(u32).range(15..=240))]
        local_stream_fps: Option<u32>,
        #[arg(long)]
        access_token: Option<String>,
        #[arg(long)]
        pairing_code: Option<String>,
        #[arg(long, hide = true, value_enum, default_value_t = ServerKindArg::Standalone)]
        server_kind: ServerKindArg,
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
    List {
        #[arg(long, value_enum, default_value_t = ListFormat::CompactJson)]
        format: ListFormat,
    },
    Use {
        #[arg(value_name = "UDID")]
        udid: String,
    },
    Boot {
        udid: Option<String>,
    },
    Shutdown {
        udid: Option<String>,
    },
    OpenUrl {
        #[arg(value_name = "UDID_OR_URL", num_args = 1..=2)]
        args: Vec<String>,
    },
    Launch {
        #[arg(value_name = "UDID_OR_BUNDLE_ID", num_args = 1..=2)]
        args: Vec<String>,
    },
    ToggleAppearance {
        udid: Option<String>,
    },
    Erase {
        udid: Option<String>,
    },
    Install {
        #[arg(value_name = "UDID_OR_APP_PATH", num_args = 1..=2)]
        args: Vec<String>,
    },
    Uninstall {
        #[arg(value_name = "UDID_OR_BUNDLE_ID", num_args = 1..=2)]
        args: Vec<String>,
    },
    Pasteboard {
        #[command(subcommand)]
        command: PasteboardCommand,
    },
    Logs {
        udid: Option<String>,
        #[arg(long, default_value_t = 30.0)]
        seconds: f64,
        #[arg(long, default_value_t = 200)]
        limit: usize,
    },
    Processes {
        udid: Option<String>,
    },
    Stats {
        udid: Option<String>,
        #[arg(long)]
        pid: Option<i32>,
        #[arg(long)]
        watch: bool,
        #[arg(long, default_value_t = 1.5)]
        interval: f64,
    },
    Sample {
        udid: Option<String>,
        #[arg(long)]
        pid: Option<i32>,
        #[arg(long, default_value_t = 3)]
        seconds: u64,
    },
    Screenshot {
        udid: Option<String>,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long)]
        stdout: bool,
        #[arg(long = "with-bezel", visible_alias = "bezel", action = ArgAction::SetTrue)]
        with_bezel: bool,
    },
    Record {
        udid: Option<String>,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long)]
        stdout: bool,
        #[arg(long, default_value_t = 5.0, value_parser = parse_positive_seconds_arg)]
        seconds: f64,
    },
    Stream {
        udid: Option<String>,
        #[arg(long, default_value_t = 0)]
        frames: u64,
    },
    #[command(name = "describe", visible_alias = "snapshot")]
    DescribeUi {
        udid: Option<String>,
        #[arg(long, value_parser = parse_point)]
        point: Option<(f64, f64)>,
        #[arg(long, value_enum, default_value_t = DescribeUiFormat::Json)]
        format: DescribeUiFormat,
        #[arg(long, value_enum, default_value_t = AccessibilitySource::NativeAX)]
        source: AccessibilitySource,
        #[arg(long)]
        max_depth: Option<usize>,
        #[arg(long)]
        include_hidden: bool,
        #[arg(short = 'i', long = "interactive", visible_alias = "interactive-only")]
        interactive_only: bool,
        #[arg(long)]
        direct: bool,
    },
    Touch {
        #[arg(value_name = "UDID_OR_POINT", num_args = 2..=3)]
        args: Vec<String>,
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
    #[command(visible_alias = "press")]
    Tap {
        #[arg(value_name = "UDID_OR_TARGET", num_args = 0..)]
        args: Vec<String>,
        #[arg(long)]
        id: Option<String>,
        #[arg(long)]
        label: Option<String>,
        #[arg(long)]
        value: Option<String>,
        #[arg(long)]
        element_type: Option<String>,
        #[arg(long)]
        expect_id: Option<String>,
        #[arg(long)]
        expect_label: Option<String>,
        #[arg(long)]
        expect_value: Option<String>,
        #[arg(long, alias = "expect-type")]
        expect_element_type: Option<String>,
        #[arg(long)]
        expect_index: Option<usize>,
        #[arg(long, default_value_t = 5_000)]
        expect_timeout_ms: u64,
        #[arg(long, default_value_t = 8)]
        expect_max_depth: usize,
        #[arg(long)]
        expect_include_hidden: bool,
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
    Back {
        udid: Option<String>,
        #[arg(long, default_value_t = 5_000)]
        timeout_ms: u64,
        #[arg(long, default_value_t = 100)]
        poll_interval_ms: u64,
        #[arg(long = "no-fallback-swipe", default_value_t = true, action = ArgAction::SetFalse)]
        fallback_swipe: bool,
    },
    #[command(visible_alias = "wait")]
    WaitFor {
        udid: Option<String>,
        #[command(flatten)]
        selector: SelectorArgs,
        #[arg(long, value_enum, default_value_t = AccessibilitySource::NativeAX)]
        source: AccessibilitySource,
        #[arg(long)]
        max_depth: Option<usize>,
        #[arg(long)]
        include_hidden: bool,
        #[arg(long, default_value_t = 5_000)]
        timeout_ms: u64,
        #[arg(long, default_value_t = 100)]
        poll_interval_ms: u64,
    },
    Assert {
        udid: Option<String>,
        #[command(flatten)]
        selector: SelectorArgs,
        #[arg(long, value_enum, default_value_t = AccessibilitySource::NativeAX)]
        source: AccessibilitySource,
        #[arg(long)]
        max_depth: Option<usize>,
        #[arg(long)]
        include_hidden: bool,
        #[arg(long, default_value_t = 5_000)]
        timeout_ms: u64,
        #[arg(long, default_value_t = 100)]
        poll_interval_ms: u64,
    },
    Swipe {
        #[arg(value_name = "UDID_OR_POINTS", num_args = 4..=5)]
        args: Vec<String>,
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
        #[arg(value_name = "UDID_OR_PRESET", num_args = 1..=2)]
        args: Vec<String>,
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
        #[arg(value_name = "UDID_OR_CENTER", num_args = 0..=3)]
        args: Vec<String>,
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
        #[arg(value_name = "UDID_OR_CENTER", num_args = 0..=3)]
        args: Vec<String>,
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
        #[arg(value_name = "UDID_OR_KEY", num_args = 1..=2)]
        args: Vec<String>,
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
        udid: Option<String>,
        #[arg(long = "keycodes", alias = "keys")]
        keycodes: String,
        #[arg(long, default_value_t = 100)]
        delay_ms: u64,
    },
    KeyCombo {
        udid: Option<String>,
        #[arg(long)]
        modifiers: String,
        #[arg(long)]
        key: String,
    },
    Type {
        #[arg(value_name = "UDID_OR_TEXT", num_args = 0..=2)]
        args: Vec<String>,
        #[arg(long)]
        stdin: bool,
        #[arg(long)]
        file: Option<PathBuf>,
        #[arg(long, default_value_t = 12)]
        delay_ms: u64,
    },
    Button {
        #[arg(value_name = "UDID_OR_BUTTON", num_args = 1..=2)]
        args: Vec<String>,
        #[arg(long, default_value_t = 0)]
        duration_ms: u32,
    },
    Crown {
        udid: Option<String>,
        #[arg(long, default_value_t = 50.0)]
        delta: f64,
    },
    Batch {
        udid: Option<String>,
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
        udid: Option<String>,
    },
    Home {
        udid: Option<String>,
    },
    AppSwitcher {
        udid: Option<String>,
    },
    RotateLeft {
        udid: Option<String>,
    },
    RotateRight {
        udid: Option<String>,
    },
    ChromeProfile {
        udid: Option<String>,
    },
}

#[derive(Subcommand)]
enum DaemonCommand {
    Start {
        #[arg(long, default_value_t = DAEMON_PORT_START)]
        port: u16,
        #[arg(long, default_value_t = IpAddr::V4(Ipv4Addr::LOCALHOST))]
        bind: IpAddr,
        #[arg(long)]
        advertise_host: Option<String>,
        #[arg(long)]
        client_root: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = VideoCodecMode::Auto)]
        video_codec: VideoCodecMode,
        #[arg(long)]
        low_latency: bool,
        #[arg(long, value_enum)]
        stream_quality: Option<StreamQualityProfileArg>,
        #[arg(long, value_parser = clap::value_parser!(u32).range(15..=240))]
        local_stream_fps: Option<u32>,
    },
    Restart {
        #[arg(long, default_value_t = DAEMON_PORT_START)]
        port: u16,
        #[arg(long, default_value_t = IpAddr::V4(Ipv4Addr::LOCALHOST))]
        bind: IpAddr,
        #[arg(long)]
        advertise_host: Option<String>,
        #[arg(long)]
        client_root: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = VideoCodecMode::Auto)]
        video_codec: VideoCodecMode,
        #[arg(long)]
        low_latency: bool,
        #[arg(long, value_enum)]
        stream_quality: Option<StreamQualityProfileArg>,
        #[arg(long, value_parser = clap::value_parser!(u32).range(15..=240))]
        local_stream_fps: Option<u32>,
    },
    Stop,
    Killall,
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
        #[arg(long, value_enum, default_value_t = VideoCodecMode::Auto)]
        video_codec: VideoCodecMode,
        #[arg(long)]
        low_latency: bool,
        #[arg(long, value_enum)]
        stream_quality: Option<StreamQualityProfileArg>,
        #[arg(long, value_parser = clap::value_parser!(u32).range(15..=240))]
        local_stream_fps: Option<u32>,
        #[arg(long)]
        access_token: String,
        #[arg(long)]
        pairing_code: Option<String>,
        #[arg(long, hide = true, value_enum, default_value_t = ServerKindArg::Workspace)]
        server_kind: ServerKindArg,
    },
}

#[derive(Subcommand)]
enum StudioCommand {
    Expose {
        simulator: Option<String>,
        #[arg(long, default_value = "https://simdeck.djdev.me")]
        studio_url: String,
        #[arg(long, default_value_t = DAEMON_PORT_START)]
        port: u16,
        #[arg(long, default_value_t = IpAddr::V4(Ipv4Addr::LOCALHOST))]
        bind: IpAddr,
        #[arg(long)]
        low_latency: bool,
        #[arg(long, value_enum, default_value_t = VideoCodecMode::Software)]
        video_codec: VideoCodecMode,
        #[arg(long, value_enum, conflicts_with = "low_latency")]
        stream_quality: Option<StreamQualityProfileArg>,
    },
}

#[derive(Subcommand)]
enum ProviderCommand {
    Connect {
        #[arg(long)]
        studio_url: String,
        #[arg(long)]
        host_id: String,
        #[arg(long)]
        host_token: String,
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long)]
        work_root: Option<PathBuf>,
    },
    Run {
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long)]
        studio_url: Option<String>,
        #[arg(long)]
        host_id: Option<String>,
        #[arg(long)]
        host_token: Option<String>,
        #[arg(long)]
        work_root: Option<PathBuf>,
        #[arg(long, default_value_t = 1)]
        max_capacity: u32,
        #[arg(long, default_value = "iPhone 17 Pro")]
        simulator_template: String,
        #[arg(long, default_value_t = DAEMON_PORT_START)]
        port: u16,
        #[arg(long, value_enum, default_value_t = VideoCodecMode::Software)]
        video_codec: VideoCodecMode,
        #[arg(long, value_enum, default_value_t = StreamQualityProfileArg::Smooth)]
        stream_quality: StreamQualityProfileArg,
    },
    Status {
        #[arg(long)]
        config: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum MaestroCommand {
    Test {
        #[arg(value_name = "UDID_OR_FLOW", num_args = 1..=2)]
        args: Vec<String>,
        #[arg(long)]
        artifacts_dir: Option<PathBuf>,
        #[arg(long)]
        continue_on_error: bool,
    },
}

#[derive(Subcommand)]
enum ServiceCommand {
    On {
        #[arg(long, default_value_t = SERVICE_PORT)]
        port: u16,
        #[arg(long, default_value_t = IpAddr::V4(Ipv4Addr::LOCALHOST))]
        bind: IpAddr,
        #[arg(long)]
        advertise_host: Option<String>,
        #[arg(long)]
        client_root: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = VideoCodecMode::Auto)]
        video_codec: VideoCodecMode,
        #[arg(long)]
        low_latency: bool,
        #[arg(long, value_enum)]
        stream_quality: Option<StreamQualityProfileArg>,
        #[arg(long, value_parser = clap::value_parser!(u32).range(15..=240))]
        local_stream_fps: Option<u32>,
        #[arg(long)]
        access_token: Option<String>,
    },
    Restart {
        #[arg(long, default_value_t = SERVICE_PORT)]
        port: u16,
        #[arg(long, default_value_t = IpAddr::V4(Ipv4Addr::LOCALHOST))]
        bind: IpAddr,
        #[arg(long)]
        advertise_host: Option<String>,
        #[arg(long)]
        client_root: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = VideoCodecMode::Auto)]
        video_codec: VideoCodecMode,
        #[arg(long)]
        low_latency: bool,
        #[arg(long, value_enum)]
        stream_quality: Option<StreamQualityProfileArg>,
        #[arg(long, value_parser = clap::value_parser!(u32).range(15..=240))]
        local_stream_fps: Option<u32>,
        #[arg(long)]
        access_token: Option<String>,
    },
    Reset {
        #[arg(long, default_value_t = SERVICE_PORT)]
        port: u16,
        #[arg(long, default_value_t = IpAddr::V4(Ipv4Addr::LOCALHOST))]
        bind: IpAddr,
        #[arg(long)]
        advertise_host: Option<String>,
        #[arg(long)]
        client_root: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = VideoCodecMode::Auto)]
        video_codec: VideoCodecMode,
        #[arg(long)]
        low_latency: bool,
        #[arg(long, value_enum)]
        stream_quality: Option<StreamQualityProfileArg>,
        #[arg(long, value_parser = clap::value_parser!(u32).range(15..=240))]
        local_stream_fps: Option<u32>,
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
        udid: Option<String>,
    },
    Set {
        #[arg(value_name = "UDID_OR_TEXT", num_args = 0..=2)]
        args: Vec<String>,
        #[arg(long)]
        stdin: bool,
        #[arg(long)]
        file: Option<PathBuf>,
    },
}

#[derive(Args, Clone, Debug, Default)]
struct SelectorArgs {
    #[arg(long)]
    id: Option<String>,
    #[arg(long)]
    label: Option<String>,
    #[arg(long)]
    value: Option<String>,
    #[arg(long, alias = "type")]
    element_type: Option<String>,
    #[arg(long)]
    index: Option<usize>,
}

impl SelectorArgs {
    fn is_empty(&self) -> bool {
        self.id.is_none()
            && self.label.is_none()
            && self.value.is_none()
            && self.element_type.is_none()
            && self.index.is_none()
    }

    fn to_json(&self) -> Value {
        serde_json::json!({
            "id": self.id.as_deref(),
            "label": self.label.as_deref(),
            "value": self.value.as_deref(),
            "elementType": self.element_type.as_deref(),
            "index": self.index,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum VideoCodecMode {
    Auto,
    Hardware,
    #[value(alias = "h264-software")]
    Software,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum ServerKindArg {
    LaunchAgent,
    Workspace,
    Foreground,
    Standalone,
}

impl From<ServerKindArg> for ServerKind {
    fn from(value: ServerKindArg) -> Self {
        match value {
            ServerKindArg::LaunchAgent => ServerKind::LaunchAgent,
            ServerKindArg::Workspace => ServerKind::Workspace,
            ServerKindArg::Foreground => ServerKind::Foreground,
            ServerKindArg::Standalone => ServerKind::Standalone,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum StreamQualityProfileArg {
    Quality,
    Full,
    Balanced,
    Fast,
    Smooth,
    Economy,
    Low,
    Tiny,
    CiSoftware,
}

impl StreamQualityProfileArg {
    fn as_profile_id(self) -> &'static str {
        match self {
            Self::Quality => "quality",
            Self::Full => "full",
            Self::Balanced => "balanced",
            Self::Fast => "fast",
            Self::Smooth => "smooth",
            Self::Economy => "economy",
            Self::Low => "low",
            Self::Tiny => "tiny",
            Self::CiSoftware => "ci-software",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum ListFormat {
    CompactJson,
    Json,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum DescribeUiFormat {
    Json,
    CompactJson,
    Agent,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DaemonMetadata {
    project_root: PathBuf,
    pid: u32,
    http_url: String,
    #[serde(default = "default_daemon_port")]
    port: u16,
    #[serde(default = "default_daemon_bind")]
    bind: IpAddr,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    advertise_host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    client_root: Option<PathBuf>,
    access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pairing_code: Option<String>,
    binary_path: PathBuf,
    started_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    log_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    video_codec: Option<String>,
    #[serde(default)]
    low_latency: bool,
    #[serde(default)]
    realtime_stream: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stream_quality_profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    local_stream_fps: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectDeviceSelection {
    project_root: PathBuf,
    udid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    runtime_name: Option<String>,
    selected_at: u64,
}

fn default_daemon_port() -> u16 {
    DAEMON_PORT_START
}

fn default_daemon_bind() -> IpAddr {
    IpAddr::V4(Ipv4Addr::LOCALHOST)
}

#[derive(Clone, Debug)]
struct DaemonLaunchOptions {
    port: u16,
    bind: IpAddr,
    advertise_host: Option<String>,
    client_root: Option<PathBuf>,
    video_codec: VideoCodecMode,
    low_latency: bool,
    realtime_stream: bool,
    stream_quality_profile: Option<String>,
    local_stream_fps: Option<u32>,
}

struct StudioExposeOptions {
    simulator: Option<String>,
    studio_url: String,
    port: u16,
    bind: IpAddr,
    video_codec: VideoCodecMode,
    low_latency: bool,
    stream_quality: Option<StreamQualityProfileArg>,
    local_stream_fps: Option<u32>,
}

impl VideoCodecMode {
    fn as_env_value(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Hardware => "hardware",
            Self::Software => "software",
        }
    }
}

struct StreamQualityEnvironment {
    profile: &'static str,
    max_edge: u32,
    fps: u32,
    min_bitrate: u32,
    bits_per_pixel: u32,
}

const DEFAULT_LOCAL_STREAM_QUALITY_PROFILE: &str = "full";

fn local_stream_quality_profile(
    low_latency: bool,
    requested: Option<StreamQualityProfileArg>,
) -> Option<String> {
    requested
        .map(|profile| profile.as_profile_id().to_owned())
        .or_else(|| (!low_latency).then_some(DEFAULT_LOCAL_STREAM_QUALITY_PROFILE.to_owned()))
}

fn stream_quality_env_for_profile(profile: &str) -> anyhow::Result<StreamQualityEnvironment> {
    match profile {
        "quality" => Ok(StreamQualityEnvironment {
            profile: "quality",
            max_edge: 4096,
            fps: 60,
            min_bitrate: 60_000_000,
            bits_per_pixel: 10,
        }),
        "full" => Ok(StreamQualityEnvironment {
            profile: "full",
            max_edge: 4096,
            fps: 60,
            min_bitrate: 12_000_000,
            bits_per_pixel: 4,
        }),
        "balanced" => Ok(StreamQualityEnvironment {
            profile: "balanced",
            max_edge: 1280,
            fps: 60,
            min_bitrate: 6_000_000,
            bits_per_pixel: 5,
        }),
        "fast" => Ok(StreamQualityEnvironment {
            profile: "fast",
            max_edge: 960,
            fps: 30,
            min_bitrate: 2_500_000,
            bits_per_pixel: 3,
        }),
        "smooth" => Ok(StreamQualityEnvironment {
            profile: "smooth",
            max_edge: 1170,
            fps: 60,
            min_bitrate: 4_000_000,
            bits_per_pixel: 5,
        }),
        "economy" => Ok(StreamQualityEnvironment {
            profile: "economy",
            max_edge: 1080,
            fps: 30,
            min_bitrate: 3_500_000,
            bits_per_pixel: 6,
        }),
        "low" => Ok(StreamQualityEnvironment {
            profile: "low",
            max_edge: 720,
            fps: 30,
            min_bitrate: 2_000_000,
            bits_per_pixel: 5,
        }),
        "tiny" => Ok(StreamQualityEnvironment {
            profile: "tiny",
            max_edge: 540,
            fps: 30,
            min_bitrate: 1_200_000,
            bits_per_pixel: 4,
        }),
        "ci-software" => Ok(StreamQualityEnvironment {
            profile: "ci-software",
            max_edge: 960,
            fps: 24,
            min_bitrate: 1_200_000,
            bits_per_pixel: 2,
        }),
        _ => anyhow::bail!("Unknown stream quality profile `{profile}`."),
    }
}

fn apply_stream_quality_environment(profile: &str) -> anyhow::Result<()> {
    let stream_quality_env = stream_quality_env_for_profile(profile)?;
    env::set_var("SIMDECK_STREAM_QUALITY_PROFILE", stream_quality_env.profile);
    env::set_var(
        "SIMDECK_REALTIME_MAX_EDGE",
        stream_quality_env.max_edge.to_string(),
    );
    env::set_var("SIMDECK_REALTIME_FPS", stream_quality_env.fps.to_string());
    env::set_var(
        "SIMDECK_REALTIME_MIN_BITRATE",
        stream_quality_env.min_bitrate.to_string(),
    );
    env::set_var(
        "SIMDECK_REALTIME_BITS_PER_PIXEL",
        stream_quality_env.bits_per_pixel.to_string(),
    );
    Ok(())
}

fn studio_stream_quality_profile(
    video_codec: VideoCodecMode,
    low_latency: bool,
    requested: Option<StreamQualityProfileArg>,
) -> Option<String> {
    requested
        .map(|profile| profile.as_profile_id().to_owned())
        .or_else(|| {
            (video_codec == VideoCodecMode::Software && !low_latency).then_some("smooth".to_owned())
        })
}

fn command_service_url(explicit: Option<&str>) -> anyhow::Result<String> {
    if let Some(url) = explicit
        .map(ToOwned::to_owned)
        .or_else(|| env::var("SIMDECK_SERVER_URL").ok())
        .filter(|value| !value.trim().is_empty())
    {
        return Ok(url);
    }
    Ok(ensure_project_daemon(DaemonLaunchOptions::default())?.http_url)
}

fn command_service_url_for_udid(
    udid: &str,
    explicit: &Option<String>,
    service_url: &Option<String>,
) -> anyhow::Result<Option<String>> {
    if android::is_android_id(udid) {
        Ok(Some(command_service_url(explicit.as_deref())?))
    } else {
        Ok(service_url.clone())
    }
}

impl Default for DaemonLaunchOptions {
    fn default() -> Self {
        Self {
            port: DAEMON_PORT_START,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            advertise_host: None,
            client_root: None,
            video_codec: VideoCodecMode::Auto,
            low_latency: false,
            realtime_stream: false,
            stream_quality_profile: Some(DEFAULT_LOCAL_STREAM_QUALITY_PROFILE.to_owned()),
            local_stream_fps: None,
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
        if daemon_is_healthy(&metadata) && daemon_matches_launch_options(&metadata, &options) {
            cleanup_orphaned_workspace_daemons_for_root(Some(&metadata.project_root));
            return Ok((metadata, false));
        }
        let _ = terminate_daemon_metadata(&metadata);
    }
    let project_root = project_root()?;
    cleanup_orphaned_workspace_daemons_for_root(Some(&project_root));
    Ok((start_project_daemon(options)?, true))
}

fn start_project_daemon(options: DaemonLaunchOptions) -> anyhow::Result<DaemonMetadata> {
    let project_root = project_root()?;
    let metadata_path = daemon_metadata_path_for_root(&project_root)?;
    let log_path = daemon_log_path_for_root(&project_root)?;
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create daemon log directory {}", parent.display()))?;
    }
    let port = choose_daemon_port_for_bind(options.port, options.bind)?;
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
        "--server-kind".to_owned(),
        "workspace".to_owned(),
    ];
    if options.low_latency {
        args.push("--low-latency".to_owned());
    }
    if let Some(local_stream_fps) = options.local_stream_fps {
        args.push("--local-stream-fps".to_owned());
        args.push(local_stream_fps.to_string());
    }
    if let Some(advertise_host) = &options.advertise_host {
        args.push("--advertise-host".to_owned());
        args.push(advertise_host.clone());
    }
    if let Some(client_root) = &options.client_root {
        args.push("--client-root".to_owned());
        args.push(client_root.to_string_lossy().into_owned());
    }
    let stream_quality_env = options
        .stream_quality_profile
        .as_deref()
        .map(stream_quality_env_for_profile)
        .transpose()?;

    let log_stdout = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("open daemon log {}", log_path.display()))?;
    let log_stderr = log_stdout
        .try_clone()
        .with_context(|| format!("clone daemon log {}", log_path.display()))?;
    let supervisor_script = format!(
        r#"terminating=0
trap 'terminating=1; if [ -n "$child" ]; then kill "$child" 2>/dev/null; wait "$child" 2>/dev/null; fi' TERM INT HUP
while :; do
  {metadata_pid_env}=$$ "$@" &
  child=$!
  wait "$child"
  status=$?
  child=
  if [ "$terminating" -eq 1 ]; then
    exit 0
  fi
  if [ "$status" -eq {recoverable_restart_exit_code} ] || [ "$status" -ge 128 ]; then
    printf '[simdeck-supervisor] daemon exited with status %s; restarting\n' "$status" >&2
    sleep 1
    continue
  fi
  exit "$status"
done
"#,
        metadata_pid_env = SUPERVISED_DAEMON_METADATA_PID_ENV,
        recoverable_restart_exit_code = RECOVERABLE_RESTART_EXIT_CODE
    );

    let mut command = ProcessCommand::new("/bin/sh");
    command
        .arg("-c")
        .arg(supervisor_script)
        .arg("simdeck-supervisor")
        .arg(&executable)
        .args(args)
        .env(
            "SIMDECK_REALTIME_STREAM",
            if options.realtime_stream || options.stream_quality_profile.is_some() {
                "1"
            } else {
                "0"
            },
        );
    if let Some(local_stream_fps) = options.local_stream_fps {
        command.env("SIMDECK_LOCAL_STREAM_FPS", local_stream_fps.to_string());
    }
    if let Some(stream_quality_env) = stream_quality_env.as_ref() {
        command.env("SIMDECK_STREAM_QUALITY_PROFILE", stream_quality_env.profile);
        command.env(
            "SIMDECK_REALTIME_MAX_EDGE",
            stream_quality_env.max_edge.to_string(),
        );
        command.env("SIMDECK_REALTIME_FPS", stream_quality_env.fps.to_string());
        command.env(
            "SIMDECK_REALTIME_MIN_BITRATE",
            stream_quality_env.min_bitrate.to_string(),
        );
        command.env(
            "SIMDECK_REALTIME_BITS_PER_PIXEL",
            stream_quality_env.bits_per_pixel.to_string(),
        );
    }
    if let Some(local_stream_fps) = options.local_stream_fps {
        command.env("SIMDECK_REALTIME_FPS", local_stream_fps.to_string());
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::from(log_stdout))
        .stderr(Stdio::from(log_stderr));
    #[cfg(unix)]
    {
        command.process_group(0);
    }
    let child = command.spawn().context("start project SimDeck daemon")?;

    let metadata = DaemonMetadata {
        project_root,
        pid: child.id(),
        http_url: format!("http://127.0.0.1:{port}"),
        port,
        bind: options.bind,
        advertise_host: options.advertise_host,
        client_root: options.client_root,
        access_token,
        pairing_code: Some(pairing_code),
        binary_path: executable,
        started_at: now_secs(),
        log_path: Some(log_path),
        video_codec: Some(options.video_codec.as_env_value().to_owned()),
        low_latency: options.low_latency,
        realtime_stream: options.realtime_stream || options.stream_quality_profile.is_some(),
        stream_quality_profile: options.stream_quality_profile,
        local_stream_fps: options.local_stream_fps,
    };
    write_daemon_metadata(&metadata)?;
    if let Err(error) = wait_for_daemon(&metadata, Duration::from_secs(15)) {
        let _ = terminate_daemon_metadata(&metadata);
        return Err(error);
    }
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
    terminate_process_group(metadata.pid, Duration::from_secs(5));
    let _ = fs::remove_file(daemon_metadata_path_for_root(&metadata.project_root)?);
    Ok(())
}

fn kill_all_project_daemons() -> anyhow::Result<()> {
    let mut killed = Vec::new();
    let mut stale = Vec::new();
    for metadata_path in daemon_metadata_paths()? {
        let Some(metadata) = fs::read_to_string(&metadata_path)
            .ok()
            .and_then(|data| serde_json::from_str::<DaemonMetadata>(&data).ok())
        else {
            let _ = fs::remove_file(&metadata_path);
            stale.push(metadata_path);
            continue;
        };
        if process_exists(metadata.pid) {
            terminate_daemon_metadata(&metadata)?;
            let _ = fs::remove_file(&metadata_path);
            killed.push(serde_json::json!({
                "pid": metadata.pid,
                "projectRoot": metadata.project_root,
                "url": metadata.http_url,
            }));
        } else {
            let _ = fs::remove_file(&metadata_path);
            stale.push(metadata_path);
        }
    }
    for process in cleanup_orphaned_workspace_daemons(None)? {
        killed.push(serde_json::json!({
            "pid": process.pgid,
            "projectRoot": process.project_root,
            "metadataPath": process.metadata_path,
            "orphaned": true,
        }));
    }
    let killed_count = killed.len();
    let stale_count = stale.len();
    println_json(&serde_json::json!({
        "ok": true,
        "killed": killed,
        "killedCount": killed_count,
        "staleCount": stale_count,
    }))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct WorkspaceDaemonProcess {
    pid: u32,
    ppid: u32,
    pgid: u32,
    project_root: PathBuf,
    metadata_path: PathBuf,
}

fn cleanup_orphaned_workspace_daemons_for_root(project_root: Option<&Path>) {
    match cleanup_orphaned_workspace_daemons(project_root) {
        Ok(killed) if !killed.is_empty() => {
            warn!(
                count = killed.len(),
                "Cleaned orphaned SimDeck workspace daemons"
            );
        }
        Ok(_) => {}
        Err(error) => {
            warn!(%error, "Failed to clean orphaned SimDeck workspace daemons");
        }
    }
}

fn cleanup_orphaned_workspace_daemons(
    project_root: Option<&Path>,
) -> anyhow::Result<Vec<WorkspaceDaemonProcess>> {
    let metadata_by_path = daemon_metadata_by_path()?;
    let mut killed = Vec::new();
    let mut killed_groups = HashSet::new();

    for process in workspace_daemon_processes()? {
        if project_root.is_some_and(|root| process.project_root != root) {
            continue;
        }
        if workspace_daemon_process_is_current(&process, &metadata_by_path) {
            continue;
        }
        if killed_groups.insert(process.pgid) {
            terminate_process_group(process.pgid, Duration::from_secs(3));
            killed.push(process);
        }
    }

    Ok(killed)
}

fn workspace_daemon_process_is_current(
    process: &WorkspaceDaemonProcess,
    metadata_by_path: &HashMap<PathBuf, DaemonMetadata>,
) -> bool {
    metadata_by_path
        .get(&process.metadata_path)
        .is_some_and(|metadata| {
            metadata.project_root == process.project_root && metadata.pid == process.pgid
        })
}

fn workspace_daemon_processes() -> anyhow::Result<Vec<WorkspaceDaemonProcess>> {
    let output = ProcessCommand::new("ps")
        .args(["-axo", "pid=,ppid=,pgid=,command="])
        .output()
        .context("list SimDeck daemon processes")?;
    if !output.status.success() {
        return Ok(Vec::new());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .lines()
        .filter_map(parse_workspace_daemon_process_line)
        .collect())
}

fn parse_workspace_daemon_process_line(line: &str) -> Option<WorkspaceDaemonProcess> {
    let (pid, rest) = take_ps_field(line)?;
    let (ppid, rest) = take_ps_field(rest)?;
    let (pgid, command) = take_ps_field(rest)?;
    if !command.contains(" daemon run ")
        || !command.contains(" --server-kind workspace")
        || !command.contains(" --metadata-path ")
    {
        return None;
    }

    let project_root = command_arg_after(command, "--project-root")?;
    let metadata_path = command_arg_after(command, "--metadata-path")?;
    Some(WorkspaceDaemonProcess {
        pid: pid.parse().ok()?,
        ppid: ppid.parse().ok()?,
        pgid: pgid.parse().ok()?,
        project_root: PathBuf::from(project_root),
        metadata_path: PathBuf::from(metadata_path),
    })
}

fn take_ps_field(line: &str) -> Option<(&str, &str)> {
    let trimmed = line.trim_start();
    let split_at = trimmed.find(char::is_whitespace)?;
    let field = &trimmed[..split_at];
    let rest = &trimmed[split_at..];
    Some((field, rest))
}

fn command_arg_after(command: &str, flag: &str) -> Option<String> {
    let marker = format!(" {flag} ");
    let start = command.find(&marker)? + marker.len();
    let value = &command[start..];
    let end = value.find(" --").unwrap_or(value.len());
    Some(value[..end].trim().to_owned()).filter(|value| !value.is_empty())
}

fn terminate_process_group(pid: u32, timeout: Duration) {
    signal_process_group(pid, "TERM");
    signal_process(pid, "TERM");
    if wait_for_process_exit(pid, timeout) {
        return;
    }
    signal_process_group(pid, "KILL");
    signal_process(pid, "KILL");
    let _ = wait_for_process_exit(pid, Duration::from_secs(2));
}

fn signal_process(pid: u32, signal: &str) {
    let _ = ProcessCommand::new("kill")
        .args([format!("-{signal}"), pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn signal_process_group(pgid: u32, signal: &str) {
    let _ = ProcessCommand::new("kill")
        .arg(format!("-{signal}"))
        .arg("--")
        .arg(format!("-{pgid}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn wait_for_process_exit(pid: u32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !process_exists(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    !process_exists(pid)
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
    let process_running = metadata
        .as_ref()
        .is_some_and(|metadata| process_exists(metadata.pid));
    let healthy = metadata.as_ref().is_some_and(daemon_is_healthy);
    let stale = metadata.is_some() && !process_running && !healthy;
    if stale {
        if let Some(metadata) = metadata.as_ref() {
            let _ = fs::remove_file(daemon_metadata_path_for_root(&metadata.project_root)?);
        }
    }
    println_json(&serde_json::json!({
        "running": healthy,
        "healthy": healthy,
        "processRunning": process_running,
        "stale": stale,
        "daemon": if stale { None } else { metadata.clone() },
        "staleDaemon": if stale { metadata } else { None },
    }))
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

fn print_existing_service_endpoints(
    result: service::ServiceInstallResult,
    selector: Option<&str>,
    open: bool,
    json: bool,
) -> anyhow::Result<()> {
    let target = PairingTarget::from_service(result)?;
    let local_url = ui_url("127.0.0.1", target.port, selector);
    let addresses: Vec<PairingAddress> = pairing_addresses(&target)
        .into_iter()
        .map(|address| PairingAddress {
            kind: address.kind,
            url: ui_url_from_base(address.url, selector),
        })
        .collect();

    if open {
        open_browser(&local_url)?;
    }

    if json {
        println_json(&serde_json::json!({
            "ok": true,
            "target": target.target,
            "service": target.service,
            "url": local_url,
            "started": false,
            "serverId": target.server_id,
            "pairingCode": target.pairing_code,
            "addresses": addresses,
        }))?;
        return Ok(());
    }

    println!("SimDeck service is already running");
    println!();
    for address in &addresses {
        let label = match address.kind {
            "local" => "Local:",
            "lan" => "LAN:",
            "tailscale" => "Tailscale:",
            _ => "URL:",
        };
        println!("{:>12}   {}", label, address.url);
    }
    println!(
        "{:>12}   {}",
        "Pair:",
        format_pairing_code(&target.pairing_code)
    );
    Ok(())
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PairingAddress {
    kind: &'static str,
    url: String,
}

#[derive(Clone, Debug)]
struct PairingTarget {
    target: &'static str,
    service: Option<String>,
    project_root: Option<PathBuf>,
    pid: Option<u32>,
    http_url: String,
    port: u16,
    advertise_host: Option<String>,
    server_id: Option<String>,
    pairing_code: String,
}

impl PairingTarget {
    fn from_service(result: service::ServiceInstallResult) -> anyhow::Result<Self> {
        Ok(Self {
            target: "service",
            service: Some(result.service),
            project_root: None,
            pid: None,
            http_url: http_url_for_host("127.0.0.1", result.port),
            port: result.port,
            advertise_host: result.advertise_host,
            server_id: result
                .access_token
                .as_deref()
                .map(auth::server_identity_for_token),
            pairing_code: result
                .pairing_code
                .context("SimDeck service did not publish a pairing code")?,
        })
    }
}

fn print_pairing_result(target: &PairingTarget, started: bool, json: bool) -> anyhow::Result<()> {
    let pairing_code = target.pairing_code.as_str();
    let addresses = pairing_addresses(target);
    let primary_url = addresses
        .iter()
        .find(|address| address.kind != "local")
        .or_else(|| addresses.first())
        .map(|address| address.url.as_str())
        .context("No SimDeck pairing address is available")?;
    let pair_url = simdeck_pair_url(
        primary_url,
        pairing_code,
        target.server_id.as_deref(),
        &addresses,
    );

    if json {
        println_json(&serde_json::json!({
            "ok": true,
            "target": target.target,
            "service": target.service,
            "projectRoot": target.project_root,
            "pid": target.pid,
            "url": target.http_url,
            "started": started,
            "serverId": target.server_id,
            "pairingCode": pairing_code,
            "pairUrl": pair_url,
            "addresses": addresses,
        }))?;
        return Ok(());
    }

    println!("🔐 SimDeck pairing");
    println!();
    for address in &addresses {
        let label = match address.kind {
            "local" => "Local:",
            "lan" => "LAN:",
            "tailscale" => "Tailscale:",
            _ => "URL:",
        };
        println!("{:>12}   {}", label, address.url);
    }
    println!("{:>12}   {}", "Pair:", format_pairing_code(pairing_code));
    println!();
    println!("Scan this with SimDeck for iOS:");
    println!();
    println!("{}", render_qr_code(&pair_url)?);
    println!("{:>12}   {}", "Deep Link:", pair_url);
    Ok(())
}

fn pairing_addresses(target: &PairingTarget) -> Vec<PairingAddress> {
    let mut addresses = Vec::new();
    push_pairing_address(
        &mut addresses,
        "local",
        http_url_for_host("127.0.0.1", target.port),
    );

    let advertise_host = target
        .advertise_host
        .as_deref()
        .filter(|host| !host.trim().is_empty());
    if let Some(host) = advertise_host {
        let kind = host
            .parse::<IpAddr>()
            .ok()
            .filter(|ip| is_tailscale_ip(*ip))
            .map(|_| "tailscale")
            .unwrap_or("lan");
        if host != "127.0.0.1" && host != "localhost" {
            push_pairing_address(&mut addresses, kind, http_url_for_host(host, target.port));
        }
    }

    if let Some(lan_ip) = detect_lan_ip() {
        push_pairing_address(
            &mut addresses,
            "lan",
            http_url_for_host(&lan_ip.to_string(), target.port),
        );
    }

    if let Some(tailscale_ip) = detect_tailscale_ip() {
        push_pairing_address(
            &mut addresses,
            "tailscale",
            http_url_for_host(&tailscale_ip.to_string(), target.port),
        );
    }

    addresses
}

fn push_pairing_address(addresses: &mut Vec<PairingAddress>, kind: &'static str, url: String) {
    if addresses.iter().any(|address| address.url == url) {
        return;
    }
    addresses.push(PairingAddress { kind, url });
}

fn simdeck_pair_url(
    primary_url: &str,
    pairing_code: &str,
    server_id: Option<&str>,
    addresses: &[PairingAddress],
) -> String {
    let mut url = format!(
        "simdeck://pair?u={}&c={}",
        percent_encode(&pairing_address_value(primary_url)),
        percent_encode(pairing_code)
    );
    if let Some(server_id) = server_id.filter(|value| !value.is_empty()) {
        url.push_str("&s=");
        url.push_str(&percent_encode(server_id));
    }
    for address in addresses
        .iter()
        .filter(|address| address.url != primary_url && address.kind != "local")
    {
        url.push_str("&a=");
        url.push_str(&percent_encode(&pairing_address_value(&address.url)));
    }
    url
}

fn pairing_address_value(url: &str) -> String {
    let Ok(parsed) = url.parse::<http::Uri>() else {
        return url.to_owned();
    };
    let Some(authority) = parsed.authority() else {
        return url.to_owned();
    };
    authority.as_str().to_owned()
}

fn render_qr_code(value: &str) -> anyhow::Result<String> {
    let code = QrCode::new(value.as_bytes()).context("generate pairing QR code")?;
    Ok(code
        .render::<unicode::Dense1x2>()
        .quiet_zone(true)
        .dark_color(unicode::Dense1x2::Light)
        .light_color(unicode::Dense1x2::Dark)
        .build())
}

fn print_pair_progress(message: impl AsRef<str>) {
    eprintln!("simdeck pair: {}", message.as_ref());
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

fn wait_for_pairing_target(target: &PairingTarget, timeout: Duration) -> anyhow::Result<()> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if http_get_json(&target.http_url, "/api/health").is_ok() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    anyhow::bail!(
        "Timed out waiting for SimDeck {} at {}",
        target.target,
        target.http_url
    )
}

fn daemon_is_healthy(metadata: &DaemonMetadata) -> bool {
    http_get_json(&metadata.http_url, "/api/health").is_ok()
}

fn daemon_matches_launch_options(metadata: &DaemonMetadata, options: &DaemonLaunchOptions) -> bool {
    daemon_port_matches_launch_options(metadata.port, options.port)
        && metadata.bind == options.bind
        && metadata.advertise_host == options.advertise_host
        && metadata.client_root == options.client_root
        && metadata
            .video_codec
            .as_deref()
            .is_some_and(|codec| codec == options.video_codec.as_env_value())
        && metadata.low_latency == options.low_latency
        && metadata.realtime_stream
            == (options.realtime_stream || options.stream_quality_profile.is_some())
        && metadata.stream_quality_profile == options.stream_quality_profile
        && metadata.local_stream_fps == options.local_stream_fps
}

fn daemon_port_matches_launch_options(actual: u16, preferred: u16) -> bool {
    let start = preferred.max(1024);
    actual >= start && actual < start.saturating_add(200)
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

fn daemon_log_path_for_root(root: &Path) -> anyhow::Result<PathBuf> {
    let mut hasher = DefaultHasher::new();
    root.to_string_lossy().hash(&mut hasher);
    Ok(env::temp_dir()
        .join("simdeck")
        .join(format!("{:016x}.log", hasher.finish())))
}

fn read_project_device_selection() -> anyhow::Result<Option<ProjectDeviceSelection>> {
    let root = project_root()?;
    let path = project_device_selection_path_for_root(&root)?;
    if !path.exists() {
        return Ok(None);
    }
    let data = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let selection = serde_json::from_str::<ProjectDeviceSelection>(&data)
        .with_context(|| format!("parse simulator selection {}", path.display()))?;
    if selection.project_root != root {
        return Ok(None);
    }
    Ok(Some(selection))
}

fn write_project_device_selection(selection: &ProjectDeviceSelection) -> anyhow::Result<PathBuf> {
    let path = project_device_selection_path_for_root(&selection.project_root)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, serde_json::to_vec_pretty(selection)?)
        .with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

fn project_device_selection_path_for_root(root: &Path) -> anyhow::Result<PathBuf> {
    let mut hasher = DefaultHasher::new();
    root.to_string_lossy().hash(&mut hasher);
    Ok(simdeck_user_state_dir()
        .join("default-devices")
        .join(format!("{:016x}.json", hasher.finish())))
}

fn simdeck_user_state_dir() -> PathBuf {
    env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(|home| home.join(".simdeck"))
        .unwrap_or_else(|| env::temp_dir().join("simdeck"))
}

fn daemon_metadata_paths() -> anyhow::Result<Vec<PathBuf>> {
    let dir = env::temp_dir().join("simdeck");
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    for entry in fs::read_dir(&dir).with_context(|| format!("read {}", dir.display()))? {
        let path = entry?.path();
        if path.extension().and_then(|value| value.to_str()) == Some("json") {
            paths.push(path);
        }
    }
    Ok(paths)
}

fn daemon_metadata_by_path() -> anyhow::Result<HashMap<PathBuf, DaemonMetadata>> {
    let mut metadata_by_path = HashMap::new();
    for path in daemon_metadata_paths()? {
        let Some(metadata) = fs::read_to_string(&path)
            .ok()
            .and_then(|data| serde_json::from_str::<DaemonMetadata>(&data).ok())
        else {
            continue;
        };
        metadata_by_path.insert(path, metadata);
    }
    Ok(metadata_by_path)
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

fn choose_daemon_port_for_bind(preferred: u16, bind: IpAddr) -> anyhow::Result<u16> {
    let start = preferred.max(1024);
    for port in start..start.saturating_add(200) {
        if port_available(bind, port) {
            return Ok(port);
        }
    }
    anyhow::bail!("No available SimDeck daemon port near {preferred}")
}

fn port_available(bind: IpAddr, port: u16) -> bool {
    if bind.is_unspecified() && TcpListener::bind((Ipv4Addr::LOCALHOST, port)).is_err() {
        return false;
    }
    TcpListener::bind((bind, port)).is_ok()
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
        "ui" | "pair"
            | "daemon"
            | "maestro"
            | "service"
            | "core-simulator"
            | "simctl-service"
            | "list"
            | "use"
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
            | "processes"
            | "stats"
            | "sample"
            | "screenshot"
            | "describe"
            | "touch"
            | "tap"
            | "back"
            | "swipe"
            | "gesture"
            | "pinch"
            | "rotate-gesture"
            | "key"
            | "key-sequence"
            | "key-combo"
            | "type"
            | "button"
            | "crown"
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
        NoCommandAction::Foreground(selector) => {
            let selector = selector.or_else(|| {
                read_project_device_selection()
                    .ok()
                    .flatten()
                    .map(|selection| selection.udid)
            });
            run_foreground_ui(selector)
        }
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

struct PairGlobalServiceOptions {
    port: Option<u16>,
    bind: IpAddr,
    advertise_host: Option<String>,
    client_root: Option<PathBuf>,
    video_codec: VideoCodecMode,
    low_latency: bool,
    stream_quality: Option<StreamQualityProfileArg>,
    local_stream_fps: Option<u32>,
    json: bool,
}

fn pair_global_service(options: PairGlobalServiceOptions) -> anyhow::Result<()> {
    let PairGlobalServiceOptions {
        port,
        bind,
        advertise_host,
        client_root,
        video_codec,
        low_latency,
        stream_quality,
        local_stream_fps,
        json,
    } = options;

    if port.is_none() {
        print_pair_progress("checking the installed service port");
    }
    let requested_port = match port {
        Some(port) => port,
        None => service::installed_port()?.unwrap_or(SERVICE_PORT),
    };
    print_pair_progress(format!("requesting port {requested_port}"));

    print_pair_progress("detecting LAN and Tailscale addresses");
    let advertise_host = advertise_host.or_else(|| {
        detect_lan_ip()
            .or_else(detect_tailscale_ip)
            .map(|ip| ip.to_string())
    });

    print_pair_progress("starting or reusing the global SimDeck service");
    cleanup_orphaned_workspace_daemons_for_root(None);
    let result = service::pair(ServiceOptions {
        port: requested_port,
        bind,
        advertise_host,
        client_root,
        video_codec,
        low_latency,
        stream_quality_profile: local_stream_quality_profile(low_latency, stream_quality),
        local_stream_fps,
        access_token: None,
        pairing_code: None,
    })?;
    if result.reused {
        print_pair_progress(format!(
            "using {} on port {}; logs: {}, {}",
            result.service,
            result.port,
            result.stdout_log.display(),
            result.stderr_log.display()
        ));
    } else {
        print_pair_progress(format!(
            "installed {} on port {}; logs: {}, {}",
            result.service,
            result.port,
            result.stdout_log.display(),
            result.stderr_log.display()
        ));
    }
    let reused = result.reused;
    let target = PairingTarget::from_service(result)?;
    if let Some(host) = target.advertise_host.as_deref() {
        print_pair_progress(format!("advertising {host}:{}", target.port));
    } else {
        print_pair_progress("no LAN or Tailscale address detected; local pairing only");
    }
    print_pair_progress(format!("waiting for service health at {}", target.http_url));
    wait_for_pairing_target(&target, Duration::from_secs(15))?;
    print_pair_progress("service is ready; rendering pairing QR");
    print_pairing_result(&target, !reused, json)
}

fn run_foreground_ui(selector: Option<String>) -> anyhow::Result<()> {
    if let Some(result) = service::active()? {
        return print_existing_service_endpoints(result, selector.as_deref(), false, false);
    }

    if let Some(metadata) = read_daemon_metadata().ok().flatten() {
        if daemon_is_healthy(&metadata) {
            terminate_daemon_metadata(&metadata)?;
        }
    }

    let project_root = project_root()?;
    let bind = IpAddr::V4(Ipv4Addr::UNSPECIFIED);
    let port = choose_daemon_port_for_bind(DAEMON_PORT_START, bind)?;
    let video_codec = VideoCodecMode::Auto;
    let low_latency = false;
    let stream_quality_profile = Some(DEFAULT_LOCAL_STREAM_QUALITY_PROFILE.to_owned());
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
        port,
        bind,
        advertise_host: Some(advertise_host.clone()),
        client_root: None,
        access_token: access_token.clone(),
        pairing_code: Some(pairing_code.clone()),
        binary_path: executable,
        started_at: now_secs(),
        log_path: None,
        video_codec: Some(video_codec.as_env_value().to_owned()),
        low_latency,
        realtime_stream: true,
        stream_quality_profile: stream_quality_profile.clone(),
        local_stream_fps: None,
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
        stream_quality_profile,
        None,
        ServerKind::Foreground,
        Some(access_token),
        Some(pairing_code),
    );
    let _ = remove_daemon_metadata_if_current(&project_root, std::process::id());
    result
}

fn supervised_daemon_metadata_pid() -> Option<u32> {
    env::var(SUPERVISED_DAEMON_METADATA_PID_ENV)
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|pid| *pid > 0)
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

fn detect_tailscale_ip() -> Option<IpAddr> {
    detect_tailscale_ip_from_cli().or_else(detect_tailscale_ip_from_ifconfig)
}

fn detect_tailscale_ip_from_cli() -> Option<IpAddr> {
    let output = ProcessCommand::new("tailscale")
        .args(["ip", "-4"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .filter_map(|line| line.trim().parse::<IpAddr>().ok())
        .find(|ip| is_tailscale_ip(*ip))
}

fn detect_tailscale_ip_from_ifconfig() -> Option<IpAddr> {
    let output = ProcessCommand::new("ifconfig")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.split_whitespace()
        .filter_map(|part| part.parse::<IpAddr>().ok())
        .find(|ip| is_tailscale_ip(*ip))
}

fn is_tailscale_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let octets = ip.octets();
            octets[0] == 100 && (octets[1] & 0b1100_0000) == 0b0100_0000
        }
        IpAddr::V6(_) => false,
    }
}

fn http_url_for_host(host: &str, port: u16) -> String {
    let host = host.trim();
    if host.contains(':') && !host.starts_with('[') && !host.ends_with(']') {
        format!("http://[{host}]:{port}")
    } else {
        format!("http://{host}:{port}")
    }
}

fn ui_url(host: &str, port: u16, selector: Option<&str>) -> String {
    ui_url_from_base(http_url_for_host(host, port), selector)
}

fn ui_url_from_base(mut url: String, selector: Option<&str>) -> String {
    if let Some(selector) = selector.filter(|value| !value.trim().is_empty()) {
        url.push_str(&format!("/?device={}", percent_encode(selector.trim())));
    }
    url
}

fn expose_to_studio(options: StudioExposeOptions) -> anyhow::Result<()> {
    let stream_quality_profile = studio_stream_quality_profile(
        options.video_codec,
        options.low_latency,
        options.stream_quality,
    );
    let metadata = ensure_project_daemon(DaemonLaunchOptions {
        port: options.port,
        bind: options.bind,
        advertise_host: None,
        client_root: None,
        video_codec: options.video_codec,
        low_latency: options.low_latency,
        realtime_stream: true,
        stream_quality_profile: stream_quality_profile.clone(),
        local_stream_fps: options.local_stream_fps,
    })?;
    let selected = if let Some(selector) = options.simulator.as_deref() {
        select_studio_simulator(&metadata.http_url, selector)
            .ok()
            .flatten()
    } else {
        select_default_studio_simulator(&metadata.http_url)
            .ok()
            .flatten()
    };
    if let Some(simulator) = selected.as_ref() {
        if !simulator.is_booted {
            service_post_ok(&metadata.http_url, &simulator.udid, "boot", &Value::Null)
                .with_context(|| format!("boot simulator {}", simulator.name))?;
        }
    }
    let health = service_get_json(&metadata.http_url, "/api/health").ok();
    let active_codec = health
        .as_ref()
        .and_then(|value| value.get("videoCodec"))
        .and_then(Value::as_str)
        .unwrap_or_else(|| options.video_codec.as_env_value());
    let active_stream_quality = health
        .as_ref()
        .and_then(|value| value.get("streamQuality"))
        .and_then(|value| value.get("profile"))
        .and_then(Value::as_str)
        .or(stream_quality_profile.as_deref());
    let realtime_stream = health
        .as_ref()
        .and_then(|value| value.get("realtimeStream"))
        .and_then(Value::as_bool)
        .unwrap_or(true);

    let bridge_script = studio_provider_bridge_script()?;
    let executable = env::current_exe().context("resolve simdeck executable")?;
    let restart_args = studio_daemon_restart_args(&options);
    let status_args = vec!["daemon".to_owned(), "status".to_owned()];
    println!(
        "Exposing {} through SimDeck Studio...",
        selected
            .as_ref()
            .map(|simulator| simulator.name.as_str())
            .unwrap_or("the selected simulator")
    );
    println!(
        "Stream: {}{}{}",
        active_codec,
        if realtime_stream { ", realtime" } else { "" },
        active_stream_quality
            .map(|profile| format!(", quality={profile}"))
            .unwrap_or_default()
    );
    println!("Press Ctrl-C to stop the Studio bridge.");
    let status = ProcessCommand::new("node")
        .arg(bridge_script)
        .env(
            "SIMDECK_CLOUD_URL",
            options.studio_url.trim_end_matches('/'),
        )
        .env("SIMDECK_LOCAL_URL", &metadata.http_url)
        .env("SIMDECK_LOCAL_TOKEN", &metadata.access_token)
        .env("SIMDECK_LOCAL_DAEMON_PID", metadata.pid.to_string())
        .env("SIMDECK_LOCAL_DAEMON_COMMAND", &executable)
        .env(
            "SIMDECK_LOCAL_DAEMON_RESTART_ARGS_JSON",
            serde_json::to_string(&restart_args)?,
        )
        .env(
            "SIMDECK_LOCAL_DAEMON_STATUS_ARGS_JSON",
            serde_json::to_string(&status_args)?,
        )
        .env(
            "SIMDECK_PROVIDER_PARENT_PID",
            std::process::id().to_string(),
        )
        .env(
            "SIMDECK_LOCAL_DAEMON_LOG",
            metadata
                .log_path
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default(),
        )
        .env(
            "SIMDECK_STUDIO_SIMULATOR_NAME",
            selected
                .as_ref()
                .map(|simulator| simulator.name.as_str())
                .unwrap_or("Local Mac simulator"),
        )
        .env(
            "SIMDECK_STUDIO_SIMULATOR_UDID",
            selected
                .as_ref()
                .map(|simulator| simulator.udid.as_str())
                .unwrap_or(""),
        )
        .env(
            "SIMDECK_STUDIO_RUNTIME_NAME",
            selected
                .as_ref()
                .and_then(|simulator| simulator.runtime_name.as_deref())
                .unwrap_or(""),
        )
        .stdin(Stdio::null())
        .status()
        .context("run Studio provider bridge")?;
    if !status.success() {
        anyhow::bail!("Studio provider bridge exited with status {status}");
    }
    Ok(())
}

fn studio_daemon_restart_args(options: &StudioExposeOptions) -> Vec<String> {
    let mut args = vec![
        "daemon".to_owned(),
        "restart".to_owned(),
        "--port".to_owned(),
        options.port.to_string(),
        "--bind".to_owned(),
        options.bind.to_string(),
        "--video-codec".to_owned(),
        options.video_codec.as_env_value().to_owned(),
    ];
    if options.low_latency {
        args.push("--low-latency".to_owned());
    } else if let Some(profile) = options.stream_quality {
        args.push("--stream-quality".to_owned());
        args.push(profile.as_profile_id().to_owned());
    } else if let Some(profile) = studio_stream_quality_profile(
        options.video_codec,
        options.low_latency,
        options.stream_quality,
    ) {
        args.push("--stream-quality".to_owned());
        args.push(profile);
    }
    if let Some(local_stream_fps) = options.local_stream_fps {
        args.push("--local-stream-fps".to_owned());
        args.push(local_stream_fps.to_string());
    }
    args
}

#[derive(Clone, Debug)]
struct StudioSimulatorSelection {
    udid: String,
    name: String,
    runtime_name: Option<String>,
    is_booted: bool,
}

fn select_studio_simulator(
    server_url: &str,
    selector: &str,
) -> anyhow::Result<Option<StudioSimulatorSelection>> {
    let normalized = selector.trim().to_lowercase();
    Ok(list_studio_simulators(server_url)?
        .into_iter()
        .find(|simulator| {
            simulator.udid.eq_ignore_ascii_case(selector)
                || simulator.name.eq_ignore_ascii_case(selector)
                || simulator.name.to_lowercase().contains(&normalized)
        }))
}

fn select_default_studio_simulator(
    server_url: &str,
) -> anyhow::Result<Option<StudioSimulatorSelection>> {
    let simulators = list_studio_simulators(server_url)?;
    Ok(simulators
        .iter()
        .find(|simulator| simulator.is_booted)
        .cloned()
        .or_else(|| simulators.into_iter().next()))
}

fn list_studio_simulators(server_url: &str) -> anyhow::Result<Vec<StudioSimulatorSelection>> {
    let response = service_get_json(server_url, "/api/simulators")?;
    let simulators = response
        .get("simulators")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(simulators
        .into_iter()
        .filter_map(|value| {
            Some(StudioSimulatorSelection {
                udid: value.get("udid")?.as_str()?.to_owned(),
                name: value.get("name")?.as_str()?.to_owned(),
                runtime_name: value
                    .get("runtimeName")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                is_booted: value
                    .get("isBooted")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
        })
        .collect())
}

fn resolve_cli_device_udid(
    positional: Option<&str>,
    global_selector: Option<&str>,
    explicit_server_url: Option<&str>,
) -> anyhow::Result<String> {
    if let Some(udid) = positional.map(str::trim).filter(|value| !value.is_empty()) {
        return Ok(udid.to_owned());
    }

    let selector = global_selector
        .map(str::to_owned)
        .or_else(|| env::var("SIMDECK_DEVICE").ok())
        .or_else(|| env::var("SIMDECK_UDID").ok())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());

    if let Some(selector) = selector {
        if android::is_android_id(&selector) || looks_like_device_selector(&selector) {
            return Ok(selector);
        }
        let server_url = command_service_url(explicit_server_url)?;
        if let Some(simulator) = select_studio_simulator(&server_url, &selector)? {
            return Ok(simulator.udid);
        }
        return Ok(selector);
    }

    if let Some(selection) = read_project_device_selection()? {
        let udid = selection.udid.trim();
        if !udid.is_empty() {
            return Ok(udid.to_owned());
        }
    }

    let server_url = command_service_url(explicit_server_url)?;
    if let Some(simulator) = infer_default_cli_simulator(&server_url)? {
        return Ok(simulator.udid);
    }

    let simulators = list_studio_simulators(&server_url)?;
    let booted = simulators
        .iter()
        .filter(|simulator| simulator.is_booted)
        .collect::<Vec<_>>();
    if booted.len() > 1 {
        anyhow::bail!(
            "Multiple booted simulators are available. Pass a UDID, run `simdeck use <UDID>`, use --device, or set SIMDECK_DEVICE."
        );
    }
    if simulators.is_empty() {
        anyhow::bail!("No simulators are available. Boot one or pass a UDID explicitly.");
    }
    anyhow::bail!(
        "No default simulator could be inferred. Pass a UDID, run `simdeck use <UDID>`, use --device, or set SIMDECK_DEVICE."
    )
}

fn infer_default_cli_simulator(
    server_url: &str,
) -> anyhow::Result<Option<StudioSimulatorSelection>> {
    let simulators = list_studio_simulators(server_url)?;
    let booted = simulators
        .iter()
        .filter(|simulator| simulator.is_booted)
        .cloned()
        .collect::<Vec<_>>();
    if booted.len() == 1 {
        return Ok(booted.into_iter().next());
    }
    if booted.is_empty() && simulators.len() == 1 {
        return Ok(simulators.into_iter().next());
    }
    Ok(None)
}

fn parse_tap_command_args(
    args: Vec<String>,
    id: Option<String>,
    label: Option<String>,
    value: Option<String>,
    element_type: Option<String>,
) -> anyhow::Result<TapCommandTarget> {
    let mut target = TapCommandTarget {
        selector: ElementSelector {
            id,
            label,
            value,
            element_type,
            index: None,
        },
        ..Default::default()
    };

    let args = args
        .into_iter()
        .map(|arg| arg.trim().to_owned())
        .filter(|arg| !arg.is_empty())
        .collect::<Vec<_>>();

    if !target.selector.is_empty() {
        match args.as_slice() {
            [] => return Ok(target),
            [udid] => {
                target.udid = Some(udid.clone());
                return Ok(target);
            }
            _ => anyhow::bail!(
                "tap accepts at most one positional UDID when selector flags are used."
            ),
        }
    }

    if args.is_empty() {
        return Ok(target);
    }

    let (udid, target_args) = if args.len() >= 2 && looks_like_device_selector(&args[0]) {
        (Some(args[0].clone()), &args[1..])
    } else {
        (None, args.as_slice())
    };
    target.udid = udid;

    if target_args.len() == 2 {
        if let (Some(x), Some(y)) = (
            parse_f64_arg(&target_args[0]),
            parse_f64_arg(&target_args[1]),
        ) {
            target.x = Some(x);
            target.y = Some(y);
            return Ok(target);
        }
    }

    if target_args.len() == 1 && parse_f64_arg(&target_args[0]).is_some() {
        anyhow::bail!("tap requires both x and y coordinates.");
    }
    if target_args.iter().any(|arg| parse_f64_arg(arg).is_some()) {
        anyhow::bail!("tap coordinates must be provided as exactly two numeric values.");
    }

    if target_args.len() == 1 {
        if let Some(index) = parse_agent_ref(&target_args[0]) {
            target.selector.index = Some(index);
            return Ok(target);
        }
    }

    target.selector.label = Some(target_args.join(" "));
    Ok(target)
}

fn parse_agent_ref(value: &str) -> Option<usize> {
    let digits = value.trim().strip_prefix("@e")?;
    let index = digits.parse::<usize>().ok()?;
    (index > 0).then_some(index - 1)
}

fn project_device_selection_for_selector(
    selector: &str,
    explicit_server_url: Option<&str>,
) -> anyhow::Result<ProjectDeviceSelection> {
    let selector = selector.trim();
    if selector.is_empty() {
        anyhow::bail!("simdeck use requires a simulator UDID or name.");
    }

    let project_root = project_root()?;
    if android::is_android_id(selector) || looks_like_device_selector(selector) {
        return Ok(ProjectDeviceSelection {
            project_root,
            udid: selector.to_owned(),
            name: None,
            runtime_name: None,
            selected_at: now_secs(),
        });
    }

    let server_url = command_service_url(explicit_server_url)?;
    let matched = select_studio_simulator(&server_url, selector)?;
    if let Some(simulator) = matched {
        return Ok(ProjectDeviceSelection {
            project_root,
            udid: simulator.udid,
            name: Some(simulator.name),
            runtime_name: simulator.runtime_name,
            selected_at: now_secs(),
        });
    }

    anyhow::bail!("No simulator matched {selector:?}. Run `simdeck list` to see available UDIDs.")
}

fn parse_optional_udid_value_args(
    command: &str,
    args: Vec<String>,
    value_name: &str,
) -> anyhow::Result<(Option<String>, String)> {
    let args = clean_cli_args(args);
    match args.as_slice() {
        [value] => Ok((None, value.clone())),
        [udid, value] => Ok((Some(udid.clone()), value.clone())),
        [] => anyhow::bail!("{command} requires {value_name}."),
        _ => anyhow::bail!("{command} accepts either {value_name} or UDID {value_name}."),
    }
}

fn parse_optional_udid_text_args(
    command: &str,
    args: Vec<String>,
    has_non_positional_input: bool,
) -> anyhow::Result<(Option<String>, Option<String>)> {
    let args = clean_cli_args(args);
    if has_non_positional_input {
        return match args.as_slice() {
            [] => Ok((None, None)),
            [udid] => Ok((Some(udid.clone()), None)),
            _ => anyhow::bail!(
                "{command} accepts at most one positional UDID with --stdin or --file."
            ),
        };
    }
    match args.as_slice() {
        [] => Ok((None, None)),
        [text] => Ok((None, Some(text.clone()))),
        [udid, text] => Ok((Some(udid.clone()), Some(text.clone()))),
        _ => anyhow::bail!("{command} accepts either TEXT or UDID TEXT. Quote multi-word text."),
    }
}

fn parse_optional_udid_f64_args(
    command: &str,
    args: Vec<String>,
    expected_values: usize,
) -> anyhow::Result<(Option<String>, Vec<f64>)> {
    let args = clean_cli_args(args);
    let (udid, values) = match args.len() {
        len if len == expected_values => (None, args.as_slice()),
        len if len == expected_values + 1 => (Some(args[0].clone()), &args[1..]),
        _ => anyhow::bail!(
            "{command} accepts either {expected_values} numeric values or UDID plus {expected_values} numeric values."
        ),
    };
    let mut parsed = Vec::with_capacity(values.len());
    for value in values {
        parsed.push(parse_f64_arg(value).ok_or_else(|| {
            anyhow::anyhow!("{command} expected a finite number, got {value:?}.")
        })?);
    }
    Ok((udid, parsed))
}

fn parse_optional_udid_point_args(
    command: &str,
    args: Vec<String>,
) -> anyhow::Result<(Option<String>, Option<f64>, Option<f64>)> {
    let args = clean_cli_args(args);
    match args.as_slice() {
        [] => Ok((None, None, None)),
        [udid] => Ok((Some(udid.clone()), None, None)),
        [x, y] => Ok((
            None,
            Some(parse_required_f64_arg(command, x)?),
            Some(parse_required_f64_arg(command, y)?),
        )),
        [udid, x, y] => Ok((
            Some(udid.clone()),
            Some(parse_required_f64_arg(command, x)?),
            Some(parse_required_f64_arg(command, y)?),
        )),
        _ => anyhow::bail!("{command} accepts [UDID] or [UDID] CENTER_X CENTER_Y."),
    }
}

fn parse_required_f64_arg(command: &str, value: &str) -> anyhow::Result<f64> {
    parse_f64_arg(value)
        .ok_or_else(|| anyhow::anyhow!("{command} expected a finite number, got {value:?}."))
}

fn clean_cli_args(args: Vec<String>) -> Vec<String> {
    args.into_iter()
        .map(|arg| arg.trim().to_owned())
        .filter(|arg| !arg.is_empty())
        .collect()
}

fn parse_f64_arg(value: &str) -> Option<f64> {
    value.parse::<f64>().ok().filter(|value| value.is_finite())
}

fn looks_like_device_selector(value: &str) -> bool {
    android::is_android_id(value)
        || (value.len() == 36 && value.chars().all(|ch| ch.is_ascii_hexdigit() || ch == '-'))
}

fn studio_provider_bridge_script() -> anyhow::Result<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(root) = project_root() {
        candidates.push(root.join("scripts/studio-provider-bridge.mjs"));
    }
    if let Ok(current_exe) = env::current_exe() {
        if let Some(package_root) = current_exe.parent().and_then(Path::parent) {
            candidates.push(package_root.join("scripts/studio-provider-bridge.mjs"));
        }
    }
    if let Ok(current_dir) = env::current_dir() {
        candidates.push(current_dir.join("scripts/studio-provider-bridge.mjs"));
    }
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| anyhow::anyhow!("Unable to find scripts/studio-provider-bridge.mjs."))
}

fn studio_host_provider_script() -> anyhow::Result<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(root) = project_root() {
        candidates.push(root.join("scripts/studio-host-provider.mjs"));
    }
    if let Ok(current_exe) = env::current_exe() {
        if let Some(package_root) = current_exe.parent().and_then(Path::parent) {
            candidates.push(package_root.join("scripts/studio-host-provider.mjs"));
        }
    }
    if let Ok(current_dir) = env::current_dir() {
        candidates.push(current_dir.join("scripts/studio-host-provider.mjs"));
    }
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| anyhow::anyhow!("Unable to find scripts/studio-host-provider.mjs."))
}

fn run_provider_command(command: ProviderCommand) -> anyhow::Result<()> {
    let script = studio_host_provider_script()?;
    let executable = env::current_exe().context("resolve simdeck executable")?;
    let mut args = Vec::new();
    match command {
        ProviderCommand::Connect {
            studio_url,
            host_id,
            host_token,
            config,
            work_root,
        } => {
            args.push("connect".to_owned());
            push_arg(&mut args, "--studio-url", studio_url);
            push_arg(&mut args, "--host-id", host_id);
            push_arg(&mut args, "--host-token", host_token);
            push_optional_path_arg(&mut args, "--config", config);
            push_optional_path_arg(&mut args, "--work-root", work_root);
        }
        ProviderCommand::Run {
            config,
            studio_url,
            host_id,
            host_token,
            work_root,
            max_capacity,
            simulator_template,
            port,
            video_codec,
            stream_quality,
        } => {
            args.push("run".to_owned());
            push_optional_path_arg(&mut args, "--config", config);
            push_optional_arg(&mut args, "--studio-url", studio_url);
            push_optional_arg(&mut args, "--host-id", host_id);
            push_optional_arg(&mut args, "--host-token", host_token);
            push_optional_path_arg(&mut args, "--work-root", work_root);
            push_arg(&mut args, "--max-capacity", max_capacity.to_string());
            push_arg(&mut args, "--simulator-template", simulator_template);
            push_arg(&mut args, "--local-url", format!("http://127.0.0.1:{port}"));
            push_arg(
                &mut args,
                "--video-codec",
                video_codec.as_env_value().to_owned(),
            );
            push_arg(
                &mut args,
                "--stream-quality",
                stream_quality.as_profile_id().to_owned(),
            );
        }
        ProviderCommand::Status { config } => {
            args.push("status".to_owned());
            push_optional_path_arg(&mut args, "--config", config);
        }
    }
    let status = ProcessCommand::new("node")
        .arg(script)
        .args(args)
        .env("SIMDECK_BINARY", executable)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .context("run Studio provider command")?;
    if !status.success() {
        anyhow::bail!("Studio provider command exited with status {status}");
    }
    Ok(())
}

fn push_arg(args: &mut Vec<String>, name: &str, value: String) {
    args.push(name.to_owned());
    args.push(value);
}

fn push_optional_arg(args: &mut Vec<String>, name: &str, value: Option<String>) {
    if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
        push_arg(args, name, value);
    }
}

fn push_optional_path_arg(args: &mut Vec<String>, name: &str, value: Option<PathBuf>) {
    if let Some(value) = value {
        push_arg(args, name, value.to_string_lossy().into_owned());
    }
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
    let device_selector = cli.device.clone();
    let service_url = explicit_server_url
        .clone()
        .or_else(|| env::var("SIMDECK_SERVER_URL").ok())
        .filter(|value| !value.trim().is_empty());
    let bridge = NativeBridge;
    let resolve_device_udid = |udid: Option<&str>| -> anyhow::Result<String> {
        resolve_cli_device_udid(
            udid,
            device_selector.as_deref(),
            explicit_server_url.as_deref(),
        )
    };

    match cli.command {
        Command::Ui {
            port,
            bind,
            advertise_host,
            client_root,
            video_codec,
            low_latency,
            stream_quality,
            local_stream_fps,
            open,
        } => {
            if let Some(result) = service::active()? {
                return print_existing_service_endpoints(result, None, open, true);
            }
            let (metadata, started) = ensure_project_daemon_with_status(DaemonLaunchOptions {
                port,
                bind,
                advertise_host,
                client_root,
                video_codec,
                low_latency,
                realtime_stream: false,
                stream_quality_profile: local_stream_quality_profile(low_latency, stream_quality),
                local_stream_fps,
            })?;
            if open {
                open_browser(&metadata.http_url)?;
            }
            print_daemon_start_result(&metadata, started)?;
            Ok(())
        }
        Command::Pair {
            port,
            bind,
            advertise_host,
            client_root,
            video_codec,
            low_latency,
            stream_quality,
            local_stream_fps,
            json,
        } => pair_global_service(PairGlobalServiceOptions {
            port,
            bind,
            advertise_host,
            client_root,
            video_codec,
            low_latency,
            stream_quality,
            local_stream_fps,
            json,
        }),
        Command::Daemon { command } => match command {
            DaemonCommand::Start {
                port,
                bind,
                advertise_host,
                client_root,
                video_codec,
                low_latency,
                stream_quality,
                local_stream_fps,
            } => {
                let (metadata, started) = ensure_project_daemon_with_status(DaemonLaunchOptions {
                    port,
                    bind,
                    advertise_host,
                    client_root,
                    video_codec,
                    low_latency,
                    realtime_stream: false,
                    stream_quality_profile: local_stream_quality_profile(
                        low_latency,
                        stream_quality,
                    ),
                    local_stream_fps,
                })?;
                print_daemon_start_result(&metadata, started)
            }
            DaemonCommand::Restart {
                port,
                bind,
                advertise_host,
                client_root,
                video_codec,
                low_latency,
                stream_quality,
                local_stream_fps,
            } => restart_detached_daemon(DaemonLaunchOptions {
                port,
                bind,
                advertise_host,
                client_root,
                video_codec,
                low_latency,
                realtime_stream: false,
                stream_quality_profile: local_stream_quality_profile(low_latency, stream_quality),
                local_stream_fps,
            }),
            DaemonCommand::Stop => stop_project_daemon(),
            DaemonCommand::Killall => kill_all_project_daemons(),
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
                stream_quality,
                local_stream_fps,
                access_token,
                pairing_code,
                server_kind,
            } => {
                if let Some(local_stream_fps) = local_stream_fps {
                    env::set_var("SIMDECK_LOCAL_STREAM_FPS", local_stream_fps.to_string());
                }
                if let Some(stream_quality) = stream_quality {
                    apply_stream_quality_environment(stream_quality.as_profile_id())?;
                }
                env::set_current_dir(&project_root).with_context(|| {
                    format!("set daemon project root to {}", project_root.display())
                })?;
                let log_path = daemon_log_path_for_root(&project_root).ok();
                write_daemon_metadata(&DaemonMetadata {
                    project_root,
                    pid: supervised_daemon_metadata_pid().unwrap_or_else(std::process::id),
                    http_url: format!("http://127.0.0.1:{port}"),
                    port,
                    bind,
                    advertise_host: advertise_host.clone(),
                    client_root: client_root.clone(),
                    access_token: access_token.clone(),
                    pairing_code: pairing_code.clone(),
                    binary_path: env::current_exe().context("resolve daemon executable")?,
                    started_at: now_secs(),
                    log_path,
                    video_codec: Some(video_codec.as_env_value().to_owned()),
                    low_latency,
                    realtime_stream: crate::transport::webrtc::realtime_stream_enabled()
                        || low_latency,
                    stream_quality_profile: env::var("SIMDECK_STREAM_QUALITY_PROFILE").ok(),
                    local_stream_fps,
                })?;
                let result = serve_with_appkit(
                    port,
                    bind,
                    advertise_host,
                    client_root,
                    video_codec,
                    low_latency,
                    env::var("SIMDECK_STREAM_QUALITY_PROFILE").ok(),
                    local_stream_fps,
                    server_kind.into(),
                    Some(access_token),
                    pairing_code,
                );
                let _ = fs::remove_file(metadata_path);
                result
            }
        },
        Command::Studio { command } => match command {
            StudioCommand::Expose {
                simulator,
                studio_url,
                port,
                bind,
                low_latency,
                video_codec,
                stream_quality,
            } => expose_to_studio(StudioExposeOptions {
                simulator,
                studio_url,
                port,
                bind,
                video_codec: if low_latency {
                    VideoCodecMode::Software
                } else {
                    video_codec
                },
                low_latency,
                stream_quality,
                local_stream_fps: None,
            }),
        },
        Command::Provider { command } => run_provider_command(command),
        Command::Maestro { command } => match command {
            MaestroCommand::Test {
                args,
                artifacts_dir,
                continue_on_error,
            } => {
                let (udid, flow) = parse_optional_udid_value_args("maestro test", args, "FLOW")?;
                let udid = resolve_device_udid(udid.as_deref())?;
                let flow = PathBuf::from(flow);
                let service_url = command_service_url(explicit_server_url.as_deref())?;
                let report =
                    run_maestro_flow(&service_url, &udid, &flow, artifacts_dir, continue_on_error)?;
                let ok = report.get("ok").and_then(Value::as_bool).unwrap_or(false);
                println_json(&report)?;
                if ok {
                    Ok(())
                } else {
                    anyhow::bail!("Maestro-compatible flow failed.")
                }
            }
        },
        Command::Serve {
            port,
            bind,
            advertise_host,
            client_root,
            video_codec,
            low_latency,
            stream_quality,
            local_stream_fps,
            access_token,
            pairing_code,
            server_kind,
        } => serve_with_appkit(
            port,
            bind,
            advertise_host,
            client_root,
            video_codec,
            low_latency,
            local_stream_quality_profile(low_latency, stream_quality),
            local_stream_fps,
            server_kind.into(),
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
                stream_quality,
                local_stream_fps,
                access_token,
            } => {
                cleanup_orphaned_workspace_daemons_for_root(None);
                service::enable(ServiceOptions {
                    port,
                    bind,
                    advertise_host,
                    client_root,
                    video_codec,
                    low_latency,
                    stream_quality_profile: local_stream_quality_profile(
                        low_latency,
                        stream_quality,
                    ),
                    local_stream_fps,
                    access_token,
                    pairing_code: None,
                })
            }
            ServiceCommand::Restart {
                port,
                bind,
                advertise_host,
                client_root,
                video_codec,
                low_latency,
                stream_quality,
                local_stream_fps,
                access_token,
            } => {
                cleanup_orphaned_workspace_daemons_for_root(None);
                service::restart(ServiceOptions {
                    port,
                    bind,
                    advertise_host,
                    client_root,
                    video_codec,
                    low_latency,
                    stream_quality_profile: local_stream_quality_profile(
                        low_latency,
                        stream_quality,
                    ),
                    local_stream_fps,
                    access_token,
                    pairing_code: None,
                })
            }
            ServiceCommand::Reset {
                port,
                bind,
                advertise_host,
                client_root,
                video_codec,
                low_latency,
                stream_quality,
                local_stream_fps,
                access_token,
            } => {
                cleanup_orphaned_workspace_daemons_for_root(None);
                service::reset(ServiceOptions {
                    port,
                    bind,
                    advertise_host,
                    client_root,
                    video_codec,
                    low_latency,
                    stream_quality_profile: local_stream_quality_profile(
                        low_latency,
                        stream_quality,
                    ),
                    local_stream_fps,
                    access_token,
                    pairing_code: None,
                })
            }
            ServiceCommand::Off => service::disable(),
        },
        Command::CoreSimulator { command } => match command {
            CoreSimulatorCommand::Start => core_simulator::start(),
            CoreSimulatorCommand::Shutdown => core_simulator::shutdown(),
            CoreSimulatorCommand::Restart => core_simulator::restart(),
        },
        Command::List { format } => {
            let service_url = command_service_url(explicit_server_url.as_deref())?;
            let simulators = service_get_json(&service_url, "/api/simulators")?
                .get("simulators")
                .cloned()
                .unwrap_or(Value::Array(Vec::new()));
            print_list_simulators(&simulators, format)?;
            Ok(())
        }
        Command::Use { udid } => {
            let selection =
                project_device_selection_for_selector(&udid, explicit_server_url.as_deref())?;
            let path = write_project_device_selection(&selection)?;
            println_json(&serde_json::json!({
                "ok": true,
                "action": "use",
                "udid": selection.udid,
                "name": selection.name,
                "runtimeName": selection.runtime_name,
                "projectRoot": selection.project_root,
                "path": path,
            }))?;
            Ok(())
        }
        Command::Boot { udid } => {
            let udid = resolve_device_udid(udid.as_deref())?;
            let service_url = command_service_url(explicit_server_url.as_deref())?;
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
            let udid = resolve_device_udid(udid.as_deref())?;
            let service_url = command_service_url(explicit_server_url.as_deref())?;
            service_post_ok(&service_url, &udid, "shutdown", &Value::Null)?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({ "ok": true, "udid": udid, "action": "shutdown" })
                )?
            );
            Ok(())
        }
        Command::OpenUrl { args } => {
            let (udid, url) = parse_optional_udid_value_args("open-url", args, "URL")?;
            let udid = resolve_device_udid(udid.as_deref())?;
            let service_url = command_service_url(explicit_server_url.as_deref())?;
            service_open_url(&service_url, &udid, &url)?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({ "ok": true, "udid": udid, "url": url })
                )?
            );
            Ok(())
        }
        Command::Launch { args } => {
            let (udid, bundle_id) = parse_optional_udid_value_args("launch", args, "BUNDLE_ID")?;
            let udid = resolve_device_udid(udid.as_deref())?;
            let service_url = command_service_url(explicit_server_url.as_deref())?;
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
            let udid = resolve_device_udid(udid.as_deref())?;
            let service_url = command_service_url(explicit_server_url.as_deref())?;
            service_action_ok(
                &service_url,
                &udid,
                &serde_json::json!({ "action": "toggleAppearance" }),
            )?;
            println_json(
                &serde_json::json!({ "ok": true, "udid": udid, "action": "toggle-appearance" }),
            )?;
            Ok(())
        }
        Command::Erase { udid } => {
            let udid = resolve_device_udid(udid.as_deref())?;
            let service_url = command_service_url(explicit_server_url.as_deref())?;
            service_post_ok(&service_url, &udid, "erase", &Value::Null)?;
            println_json(&serde_json::json!({ "ok": true, "udid": udid, "action": "erase" }))?;
            Ok(())
        }
        Command::Install { args } => {
            let (udid, app_path) = parse_optional_udid_value_args("install", args, "APP_PATH")?;
            let udid = resolve_device_udid(udid.as_deref())?;
            let service_url = command_service_url(explicit_server_url.as_deref())?;
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
        Command::Uninstall { args } => {
            let (udid, bundle_id) = parse_optional_udid_value_args("uninstall", args, "BUNDLE_ID")?;
            let udid = resolve_device_udid(udid.as_deref())?;
            let service_url = command_service_url(explicit_server_url.as_deref())?;
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
                let udid = resolve_device_udid(udid.as_deref())?;
                let service_url = command_service_url(explicit_server_url.as_deref())?;
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
            PasteboardCommand::Set { args, stdin, file } => {
                let has_non_positional_input = stdin || file.is_some();
                let (udid, text) = parse_optional_udid_text_args(
                    "pasteboard set",
                    args,
                    has_non_positional_input,
                )?;
                let udid = resolve_device_udid(udid.as_deref())?;
                let service_url = command_service_url(explicit_server_url.as_deref())?;
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
            let udid = resolve_device_udid(udid.as_deref())?;
            let service_url = command_service_url(explicit_server_url.as_deref())?;
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
        Command::Processes { udid } => {
            let udid = resolve_device_udid(udid.as_deref())?;
            let service_url = command_service_url(explicit_server_url.as_deref())?;
            let processes = service_get_json(
                &service_url,
                &format!("/api/simulators/{}/processes", url_path_component(&udid)),
            )?;
            println_json(&processes)?;
            Ok(())
        }
        Command::Stats {
            udid,
            pid,
            watch,
            interval,
        } => {
            let udid = resolve_device_udid(udid.as_deref())?;
            let service_url = command_service_url(explicit_server_url.as_deref())?;
            if watch {
                run_stats_watch(&service_url, &udid, pid, interval)?;
            } else {
                let stats = service_performance_json(&service_url, &udid, pid)?;
                println_json(&stats)?;
            }
            Ok(())
        }
        Command::Sample { udid, pid, seconds } => {
            let udid = resolve_device_udid(udid.as_deref())?;
            let service_url = command_service_url(explicit_server_url.as_deref())?;
            let pid = match pid {
                Some(pid) => pid,
                None => service_performance_json(&service_url, &udid, None)?
                    .get("selectedPid")
                    .and_then(Value::as_i64)
                    .ok_or_else(|| {
                        anyhow::anyhow!("No foreground simulator app process is available.")
                    })? as i32,
            };
            let report = service_post_sample(&service_url, &udid, pid, seconds)?;
            let sample = report.get("sample").unwrap_or(&Value::Null);
            if let Some(text) = sample.get("report").and_then(Value::as_str) {
                print!("{text}");
                if !text.ends_with('\n') {
                    println!();
                }
            } else {
                println_json(&report)?;
            }
            Ok(())
        }
        Command::Screenshot {
            udid,
            output,
            stdout,
            with_bezel,
        } => {
            let udid = resolve_device_udid(udid.as_deref())?;
            let service_url = command_service_url(explicit_server_url.as_deref())?;
            let query = if with_bezel { "?bezel=true" } else { "" };
            let png = service_get_bytes(
                &service_url,
                &format!(
                    "/api/simulators/{}/screenshot.png{}",
                    url_path_component(&udid),
                    query
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
        Command::Record {
            udid,
            output,
            stdout,
            seconds,
        } => {
            let udid = resolve_device_udid(udid.as_deref())?;
            let service_url = command_service_url(explicit_server_url.as_deref())?;
            let mp4 = service_post_bytes(
                &service_url,
                &format!(
                    "/api/simulators/{}/screen-recording",
                    url_path_component(&udid)
                ),
                &serde_json::json!({ "seconds": seconds }),
            )?;
            if stdout {
                io::stdout().write_all(&mp4)?;
            } else {
                let output = output.unwrap_or_else(|| default_recording_path(&udid));
                if let Some(parent) = output
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&output, &mp4)?;
                println_json(
                    &serde_json::json!({ "ok": true, "udid": udid, "action": "record", "output": output, "seconds": seconds }),
                )?;
            }
            Ok(())
        }
        Command::Stream { udid, frames } => {
            let udid = resolve_device_udid(udid.as_deref())?;
            run_stream_stdout(&bridge, udid, frames)
        }
        Command::DescribeUi {
            udid,
            point,
            format,
            source,
            max_depth,
            include_hidden,
            interactive_only,
            direct,
        } => {
            let udid = resolve_cli_device_udid(
                udid.as_deref(),
                device_selector.as_deref(),
                explicit_server_url.as_deref(),
            )?;
            let service_url = if direct {
                String::new()
            } else {
                command_service_url(explicit_server_url.as_deref())?
            };
            let snapshot = describe_ui_snapshot(
                &bridge,
                &udid,
                point,
                source,
                max_depth,
                include_hidden,
                interactive_only,
                direct,
                &service_url,
            )?;
            print_describe_ui(&snapshot, format)?;
            Ok(())
        }
        Command::Touch {
            args,
            phase,
            normalized,
            down,
            up,
            delay_ms,
        } => {
            let (udid, points) = parse_optional_udid_f64_args("touch", args, 2)?;
            let udid = resolve_device_udid(udid.as_deref())?;
            let x = points[0];
            let y = points[1];
            let android_device = android::is_android_id(&udid);
            if android_device && !normalized {
                anyhow::bail!("Android touch coordinates require --normalized.");
            }
            let command_server_url =
                command_service_url_for_udid(&udid, &explicit_server_url, &service_url)?;
            if let Some(server_url) = command_server_url.as_deref().filter(|_| normalized) {
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
                if bridge_simulator_is_tvos(&bridge, &udid) {
                    perform_tvos_touch_command(&bridge, &udid, &phase, down, up)?;
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
            }
            println_json(&serde_json::json!({ "ok": true, "udid": udid, "action": "touch" }))?;
            Ok(())
        }
        Command::Tap {
            args,
            id,
            label,
            value,
            element_type,
            expect_id,
            expect_label,
            expect_value,
            expect_element_type,
            expect_index,
            expect_timeout_ms,
            expect_max_depth,
            expect_include_hidden,
            wait_timeout_ms,
            poll_interval_ms,
            normalized,
            duration_ms,
            pre_delay_ms,
            post_delay_ms,
        } => {
            let target = parse_tap_command_args(args, id, label, value, element_type)?;
            let uses_inferred_device = target.udid.is_none();
            let uses_selector = !target.selector.is_empty();
            let expect_selector = SelectorArgs {
                id: expect_id,
                label: expect_label,
                value: expect_value,
                element_type: expect_element_type,
                index: expect_index,
            };
            let uses_expectation = !expect_selector.is_empty();
            let udid = resolve_cli_device_udid(
                target.udid.as_deref(),
                device_selector.as_deref(),
                explicit_server_url.as_deref(),
            )?;
            let x = target.x;
            let y = target.y;
            let ElementSelector {
                id,
                label,
                value,
                element_type,
                index,
            } = target.selector;
            let preferred_service_url = if uses_inferred_device || uses_selector || uses_expectation
            {
                Some(command_service_url(explicit_server_url.as_deref())?)
            } else {
                service_url.clone()
            };
            let command_server_url =
                command_service_url_for_udid(&udid, &explicit_server_url, &preferred_service_url)?;
            let mut expectation_match = None;
            if let (Some(server_url), Some(x), Some(y), true, None, None, None, None, false) = (
                command_server_url.as_deref(),
                x,
                y,
                normalized,
                id.as_ref(),
                label.as_ref(),
                value.as_ref(),
                element_type.as_ref(),
                uses_expectation,
            ) {
                sleep_ms(pre_delay_ms);
                service_tap(server_url, &udid, x, y, duration_ms)?;
                sleep_ms(post_delay_ms);
            } else if let Some(server_url) = command_server_url.as_deref() {
                sleep_ms(pre_delay_ms);
                let mut body = serde_json::json!({
                    "x": x,
                    "y": y,
                    "normalized": normalized,
                    "selector": {
                        "id": id,
                        "label": label,
                        "value": value,
                        "elementType": element_type,
                        "index": index,
                    },
                    "waitTimeoutMs": wait_timeout_ms,
                    "pollMs": poll_interval_ms,
                    "durationMs": duration_ms,
                });
                if uses_expectation {
                    if let Some(object) = body.as_object_mut() {
                        object.insert(
                            "expect".to_owned(),
                            serde_json::json!({
                                "selector": expect_selector.to_json(),
                                "source": AccessibilitySource::NativeAX.as_query_value(),
                                "maxDepth": expect_max_depth,
                                "includeHidden": expect_include_hidden,
                                "timeoutMs": expect_timeout_ms,
                                "pollMs": poll_interval_ms,
                            }),
                        );
                    }
                }
                let tap_result = service_tap_element(server_url, &udid, body)?;
                expectation_match = tap_result.get("expectation").cloned();
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
                            index,
                        },
                        wait_timeout_ms,
                        poll_interval_ms,
                    },
                )?;
                sleep_ms(pre_delay_ms);
                if bridge_simulator_is_tvos(&bridge, &udid) {
                    press_tvos_remote_key(&bridge, &udid, HID_KEY_ENTER)?;
                } else if let Some(input) = target.input.as_ref() {
                    perform_tap_with_input(input, target.x, target.y, duration_ms)?;
                } else {
                    perform_tap(&bridge, &udid, target.x, target.y, duration_ms)?;
                }
                sleep_ms(post_delay_ms);
            }
            let mut result = serde_json::json!({ "ok": true, "udid": udid, "action": "tap" });
            if let (Some(object), Some(expectation)) = (result.as_object_mut(), expectation_match) {
                object.insert("expectation".to_owned(), expectation);
            }
            println_json(&result)?;
            Ok(())
        }
        Command::Back {
            udid,
            timeout_ms,
            poll_interval_ms,
            fallback_swipe,
        } => {
            let udid = resolve_device_udid(udid.as_deref())?;
            let service_url = command_service_url(explicit_server_url.as_deref())?;
            let result = service_action(
                &service_url,
                &udid,
                &serde_json::json!({
                    "action": "back",
                    "timeoutMs": timeout_ms,
                    "pollMs": poll_interval_ms,
                    "fallbackSwipe": fallback_swipe,
                }),
            )?;
            println_json(&result)?;
            Ok(())
        }
        Command::WaitFor {
            udid,
            selector,
            source,
            max_depth,
            include_hidden,
            timeout_ms,
            poll_interval_ms,
        } => {
            let udid = resolve_device_udid(udid.as_deref())?;
            let service_url = command_service_url(explicit_server_url.as_deref())?;
            let result = service_wait_for_selector(
                &service_url,
                &udid,
                "wait-for",
                selector,
                source,
                max_depth,
                include_hidden,
                timeout_ms,
                poll_interval_ms,
            )?;
            println_json(&result)?;
            Ok(())
        }
        Command::Assert {
            udid,
            selector,
            source,
            max_depth,
            include_hidden,
            timeout_ms,
            poll_interval_ms,
        } => {
            let udid = resolve_device_udid(udid.as_deref())?;
            let service_url = command_service_url(explicit_server_url.as_deref())?;
            let result = service_wait_for_selector(
                &service_url,
                &udid,
                "assert",
                selector,
                source,
                max_depth,
                include_hidden,
                timeout_ms,
                poll_interval_ms,
            )?;
            println_json(&result)?;
            Ok(())
        }
        Command::Swipe {
            args,
            normalized,
            duration_ms,
            steps,
            pre_delay_ms,
            post_delay_ms,
        } => {
            let (udid, points) = parse_optional_udid_f64_args("swipe", args, 4)?;
            let udid = resolve_device_udid(udid.as_deref())?;
            let start_x = points[0];
            let start_y = points[1];
            let end_x = points[2];
            let end_y = points[3];
            let android_device = android::is_android_id(&udid);
            if android_device && !normalized {
                anyhow::bail!("Android swipe coordinates require --normalized.");
            }
            let command_server_url =
                command_service_url_for_udid(&udid, &explicit_server_url, &service_url)?;
            if let Some(server_url) = command_server_url.as_deref().filter(|_| normalized) {
                sleep_ms(pre_delay_ms);
                if android_device {
                    service_action_ok(
                        server_url,
                        &udid,
                        &serde_json::json!({
                            "action": "swipe",
                            "startX": start_x,
                            "startY": start_y,
                            "endX": end_x,
                            "endY": end_y,
                            "durationMs": duration_ms,
                            "steps": steps,
                        }),
                    )?;
                } else {
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
                }
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
            args,
            screen_width,
            screen_height,
            normalized,
            duration_ms,
            delta,
            pre_delay_ms,
            post_delay_ms,
        } => {
            let (udid, preset) = parse_optional_udid_value_args("gesture", args, "PRESET")?;
            let udid = resolve_device_udid(udid.as_deref())?;
            let android_device = android::is_android_id(&udid);
            let command_server_url =
                command_service_url_for_udid(&udid, &explicit_server_url, &service_url)?;
            if android_device {
                let server_url = command_server_url
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("Android command requires SimDeck daemon."))?;
                sleep_ms(pre_delay_ms);
                service_action_ok(
                    server_url,
                    &udid,
                    &serde_json::json!({
                        "action": "gesture",
                        "preset": preset,
                        "durationMs": duration_ms,
                        "delta": delta,
                        "steps": 4,
                    }),
                )?;
                sleep_ms(post_delay_ms);
                println_json(
                    &serde_json::json!({ "ok": true, "udid": udid, "action": "gesture", "preset": preset }),
                )?;
                return Ok(());
            }
            if let Some(server_url) = command_server_url.as_deref().filter(|_| normalized) {
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
            args,
            start_distance,
            end_distance,
            angle_degrees,
            normalized,
            duration_ms,
            steps,
        } => {
            let (udid, center_x, center_y) = parse_optional_udid_point_args("pinch", args)?;
            let udid = resolve_device_udid(udid.as_deref())?;
            if android::is_android_id(&udid) {
                anyhow::bail!("Android pinch gestures are not supported by the ADB input bridge.");
            }
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
            args,
            radius,
            degrees,
            normalized,
            duration_ms,
            steps,
        } => {
            let (udid, center_x, center_y) =
                parse_optional_udid_point_args("rotate-gesture", args)?;
            let udid = resolve_device_udid(udid.as_deref())?;
            if android::is_android_id(&udid) {
                anyhow::bail!("Android rotate gestures are not supported by the ADB input bridge.");
            }
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
            args,
            modifiers,
            duration_ms,
            pre_delay_ms,
            post_delay_ms,
        } => {
            let (udid, key) = parse_optional_udid_value_args("key", args, "KEY")?;
            let udid = resolve_device_udid(udid.as_deref())?;
            let key_code = parse_hid_key(&key)?;
            sleep_ms(pre_delay_ms);
            let command_server_url =
                command_service_url_for_udid(&udid, &explicit_server_url, &service_url)?;
            if let Some(server_url) = command_server_url.as_deref().filter(|_| duration_ms == 0) {
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
            let udid = resolve_device_udid(udid.as_deref())?;
            let keys = parse_key_list(&keycodes)?;
            let command_server_url =
                command_service_url_for_udid(&udid, &explicit_server_url, &service_url)?;
            if let Some(server_url) = command_server_url.as_deref() {
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
            let udid = resolve_device_udid(udid.as_deref())?;
            let modifier_mask = parse_modifier_mask(&modifiers)?;
            let key_code = parse_hid_key(&key)?;
            let command_server_url =
                command_service_url_for_udid(&udid, &explicit_server_url, &service_url)?;
            if let Some(server_url) = command_server_url.as_deref() {
                service_key(server_url, &udid, key_code, modifier_mask)?;
            } else {
                bridge.send_key(&udid, key_code, modifier_mask)?;
            }
            println_json(&serde_json::json!({ "ok": true, "udid": udid, "action": "key-combo" }))?;
            Ok(())
        }
        Command::Type {
            args,
            stdin,
            file,
            delay_ms,
        } => {
            let has_non_positional_input = stdin || file.is_some();
            let (udid, text) =
                parse_optional_udid_text_args("type", args, has_non_positional_input)?;
            let udid = resolve_device_udid(udid.as_deref())?;
            let text = read_text_input(text, stdin, file)?;
            if android::is_android_id(&udid) {
                let server_url = command_service_url(explicit_server_url.as_deref())?;
                service_action_ok(
                    &server_url,
                    &udid,
                    &serde_json::json!({
                        "action": "type",
                        "text": text,
                        "delayMs": delay_ms,
                    }),
                )?;
            } else {
                type_text(&bridge, &udid, &text, delay_ms)?;
            }
            println_json(&serde_json::json!({ "ok": true, "udid": udid, "action": "type" }))?;
            Ok(())
        }
        Command::Button { args, duration_ms } => {
            let (udid, button) = parse_optional_udid_value_args("button", args, "BUTTON")?;
            let udid = resolve_device_udid(udid.as_deref())?;
            let command_server_url =
                command_service_url_for_udid(&udid, &explicit_server_url, &service_url)?;
            if let Some(server_url) = command_server_url.as_deref() {
                service_button(server_url, &udid, &button, duration_ms)?;
            } else {
                bridge.press_button(&udid, &button, duration_ms)?;
            }
            println_json(
                &serde_json::json!({ "ok": true, "udid": udid, "action": "button", "button": button }),
            )?;
            Ok(())
        }
        Command::Crown { udid, delta } => {
            let udid = resolve_device_udid(udid.as_deref())?;
            if let Some(server_url) = service_url.as_deref() {
                service_crown(server_url, &udid, delta)?;
            } else {
                bridge.rotate_crown(&udid, delta)?;
            }
            println_json(
                &serde_json::json!({ "ok": true, "udid": udid, "action": "crown", "delta": delta }),
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
            let udid = resolve_device_udid(udid.as_deref())?;
            let step_lines = read_batch_steps(steps, file, stdin)?;
            let report = match command_service_url(explicit_server_url.as_deref()) {
                Ok(server_url) => service_batch(
                    &server_url,
                    &udid,
                    batch_lines_to_json_steps(&step_lines)?,
                    continue_on_error,
                )?,
                Err(_error) if !android::is_android_id(&udid) => {
                    run_batch(&bridge, &udid, step_lines, None, false, continue_on_error)?
                }
                Err(error) => return Err(error),
            };
            println_json(&report)?;
            Ok(())
        }
        Command::DismissKeyboard { udid } => {
            let udid = resolve_device_udid(udid.as_deref())?;
            let command_server_url =
                command_service_url_for_udid(&udid, &explicit_server_url, &service_url)?;
            if let Some(server_url) = command_server_url.as_deref() {
                service_action_ok(
                    server_url,
                    &udid,
                    &serde_json::json!({ "action": "dismissKeyboard" }),
                )?;
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
            let udid = resolve_device_udid(udid.as_deref())?;
            let command_server_url =
                command_service_url_for_udid(&udid, &explicit_server_url, &service_url)?;
            if let Some(server_url) = command_server_url.as_deref() {
                service_action_ok(server_url, &udid, &serde_json::json!({ "action": "home" }))?;
            } else {
                bridge.press_home(&udid)?;
            }
            println_json(&serde_json::json!({ "ok": true, "udid": udid, "action": "home" }))?;
            Ok(())
        }
        Command::AppSwitcher { udid } => {
            let udid = resolve_device_udid(udid.as_deref())?;
            let command_server_url =
                command_service_url_for_udid(&udid, &explicit_server_url, &service_url)?;
            if let Some(server_url) = command_server_url.as_deref() {
                service_action_ok(
                    server_url,
                    &udid,
                    &serde_json::json!({ "action": "appSwitcher" }),
                )?;
            } else {
                bridge.open_app_switcher(&udid)?;
            }
            println_json(
                &serde_json::json!({ "ok": true, "udid": udid, "action": "app-switcher" }),
            )?;
            Ok(())
        }
        Command::RotateLeft { udid } => {
            let udid = resolve_device_udid(udid.as_deref())?;
            let command_server_url =
                command_service_url_for_udid(&udid, &explicit_server_url, &service_url)?;
            if let Some(server_url) = command_server_url.as_deref() {
                service_action_ok(
                    server_url,
                    &udid,
                    &serde_json::json!({ "action": "rotateLeft" }),
                )?;
            } else {
                bridge.rotate_left(&udid)?;
            }
            println_json(
                &serde_json::json!({ "ok": true, "udid": udid, "action": "rotate-left" }),
            )?;
            Ok(())
        }
        Command::RotateRight { udid } => {
            let udid = resolve_device_udid(udid.as_deref())?;
            let command_server_url =
                command_service_url_for_udid(&udid, &explicit_server_url, &service_url)?;
            if let Some(server_url) = command_server_url.as_deref() {
                service_action_ok(
                    server_url,
                    &udid,
                    &serde_json::json!({ "action": "rotateRight" }),
                )?;
            } else {
                bridge.rotate_right(&udid)?;
            }
            println_json(
                &serde_json::json!({ "ok": true, "udid": udid, "action": "rotate-right" }),
            )?;
            Ok(())
        }
        Command::ChromeProfile { udid } => {
            let udid = resolve_device_udid(udid.as_deref())?;
            let service_url = command_service_url(explicit_server_url.as_deref())?;
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
    stream_quality_profile: Option<String>,
    local_stream_fps: Option<u32>,
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
    stream_quality_profile: Option<String>,
    local_stream_fps: Option<u32>,
    server_kind: ServerKind,
    access_token: Option<String>,
    pairing_code: Option<String>,
) -> anyhow::Result<()> {
    std::env::set_var("SIMDECK_VIDEO_CODEC", video_codec.as_env_value());
    std::env::set_var("SIMDECK_LOW_LATENCY", if low_latency { "1" } else { "0" });
    if let Some(local_stream_fps) = local_stream_fps {
        std::env::set_var("SIMDECK_LOCAL_STREAM_FPS", local_stream_fps.to_string());
    } else {
        std::env::remove_var("SIMDECK_LOCAL_STREAM_FPS");
    }
    if let Some(profile) = stream_quality_profile.as_deref() {
        apply_stream_quality_environment(profile)?;
    }
    if let Some(local_stream_fps) = local_stream_fps {
        std::env::set_var("SIMDECK_REALTIME_FPS", local_stream_fps.to_string());
    }
    let stream_quality_realtime = stream_quality_profile.is_some();
    let inherited_realtime_stream = crate::transport::webrtc::realtime_stream_enabled();
    std::env::set_var(
        "SIMDECK_REALTIME_STREAM",
        if inherited_realtime_stream || low_latency || stream_quality_realtime {
            "1"
        } else {
            "0"
        },
    );
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
                server_kind,
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
        let mut consecutive_http_probe_failures = 0usize;

        loop {
            std::thread::sleep(SERVER_HEALTH_WATCHDOG_INTERVAL);

            let heartbeat_age = now_secs().saturating_sub(heartbeat.load(Ordering::Relaxed));
            let heartbeat_stale = heartbeat_age > SERVER_HEALTH_WATCHDOG_STALE_HEARTBEAT.as_secs();
            let health_ok = http_health_probe(http_addr, SERVER_HEALTH_WATCHDOG_PROBE_TIMEOUT);

            if heartbeat_stale {
                consecutive_failures += 1;
            } else {
                consecutive_failures = 0;
            }
            if health_ok {
                consecutive_http_probe_failures = 0;
            } else {
                consecutive_http_probe_failures += 1;
            }

            if server_health_watchdog_should_restart(
                consecutive_failures,
                consecutive_http_probe_failures,
            ) {
                eprintln!(
                    "SimDeck server health watchdog failed \
(heartbeat_failures={consecutive_failures}, http_probe_failures={consecutive_http_probe_failures}, \
heartbeat_age={heartbeat_age}s, http_health_ok={health_ok}); restarting server process."
                );
                std::process::exit(RECOVERABLE_RESTART_EXIT_CODE);
            }
        }
    });
}

fn server_health_watchdog_should_restart(
    consecutive_heartbeat_failures: usize,
    consecutive_http_probe_failures: usize,
) -> bool {
    consecutive_heartbeat_failures >= SERVER_HEALTH_WATCHDOG_FAILURE_THRESHOLD
        || consecutive_http_probe_failures >= SERVER_HEALTH_WATCHDOG_HTTP_FAILURE_THRESHOLD
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

#[derive(Clone, Debug, Default, PartialEq)]
struct ElementSelector {
    id: Option<String>,
    label: Option<String>,
    value: Option<String>,
    element_type: Option<String>,
    index: Option<usize>,
}

impl ElementSelector {
    fn is_empty(&self) -> bool {
        self.id.is_none()
            && self.label.is_none()
            && self.value.is_none()
            && self.element_type.is_none()
            && self.index.is_none()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct TapCommandTarget {
    udid: Option<String>,
    x: Option<f64>,
    y: Option<f64>,
    selector: ElementSelector,
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

fn bridge_simulator_is_tvos(bridge: &NativeBridge, udid: &str) -> bool {
    bridge.simulator_is_tvos(udid).unwrap_or(false)
}

fn press_tvos_remote_key(
    bridge: &NativeBridge,
    udid: &str,
    key_code: u16,
) -> Result<(), crate::error::AppError> {
    bridge.send_key(udid, key_code, 0)
}

fn perform_tvos_touch_phase(
    bridge: &NativeBridge,
    udid: &str,
    phase: &str,
) -> Result<(), crate::error::AppError> {
    if let Some(key_code) = tvos_remote_key_for_touch_phase(phase)? {
        press_tvos_remote_key(bridge, udid, key_code)?;
    }
    Ok(())
}

fn perform_tvos_touch_command(
    bridge: &NativeBridge,
    udid: &str,
    phase: &str,
    down: bool,
    up: bool,
) -> Result<(), crate::error::AppError> {
    if down || up {
        if up {
            return press_tvos_remote_key(bridge, udid, HID_KEY_ENTER);
        }
        return Ok(());
    }
    perform_tvos_touch_phase(bridge, udid, phase)
}

fn perform_tap(
    bridge: &NativeBridge,
    udid: &str,
    x: f64,
    y: f64,
    duration_ms: u64,
) -> Result<(), crate::error::AppError> {
    if bridge_simulator_is_tvos(bridge, udid) {
        return press_tvos_remote_key(bridge, udid, HID_KEY_ENTER);
    }
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
    if bridge_simulator_is_tvos(bridge, udid) {
        let key_code = tvos_remote_key_for_touch_motion(
            gesture.start_x.clamp(0.0, 1.0),
            gesture.start_y.clamp(0.0, 1.0),
            gesture.end_x.clamp(0.0, 1.0),
            gesture.end_y.clamp(0.0, 1.0),
        );
        return press_tvos_remote_key(bridge, udid, key_code);
    }
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

fn default_recording_path(udid: &str) -> PathBuf {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    PathBuf::from(format!("Simulator Recording - {udid} - {timestamp}.mp4"))
}

fn run_stream_stdout(bridge: &NativeBridge, udid: String, frames: u64) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .context("create stream runtime")?;
    let _runtime_guard = runtime.enter();
    let metrics = Arc::new(Metrics::default());
    let session = simulators::session::SimulatorSession::new(bridge, udid, metrics)
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    session
        .ensure_started()
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    session.request_keyframe();

    let mut receiver = session.subscribe();
    let mut stdout = io::stdout().lock();
    let mut written = 0u64;
    runtime.block_on(async {
        loop {
            if frames > 0 && written >= frames {
                break;
            }
            let frame = receiver.recv().await?;
            let sample = crate::transport::webrtc::h264_annex_b_sample(&frame)
                .map_err(|error| anyhow::anyhow!("encode Annex B frame: {error}"))?;
            stdout.write_all(&sample)?;
            stdout.flush()?;
            written += 1;
        }
        anyhow::Ok(())
    })
}

#[allow(clippy::too_many_arguments)]
fn describe_ui_snapshot(
    bridge: &NativeBridge,
    udid: &str,
    point: Option<(f64, f64)>,
    source: AccessibilitySource,
    max_depth: Option<usize>,
    include_hidden: bool,
    interactive_only: bool,
    direct: bool,
    server_url: &str,
) -> anyhow::Result<Value> {
    if !direct {
        if let Some((x, y)) = point {
            if matches!(
                source,
                AccessibilitySource::Auto
                    | AccessibilitySource::NativeAX
                    | AccessibilitySource::AndroidUiautomator
            ) {
                match fetch_service_accessibility_point(udid, x, y, server_url) {
                    Ok(snapshot) => return Ok(snapshot),
                    Err(error) if source != AccessibilitySource::Auto => return Err(error),
                    Err(_) => {}
                }
            }
        } else {
            match fetch_service_accessibility_tree(
                udid,
                source,
                max_depth,
                include_hidden,
                interactive_only,
                server_url,
            ) {
                Ok(snapshot) => return Ok(snapshot),
                Err(error) if source != AccessibilitySource::Auto => return Err(error),
                Err(_) => {}
            }
        }
    }

    if source != AccessibilitySource::Auto && source != AccessibilitySource::NativeAX {
        anyhow::bail!(
            "The `{}` hierarchy source requires a running SimDeck daemon. Start it with `simdeck daemon start --port 4311`, or use --source native-ax.",
            source.as_query_value()
        );
    }

    let snapshot =
        bridge.accessibility_snapshot_with_options(udid, point, max_depth, interactive_only)?;
    Ok(if interactive_only && point.is_none() {
        interactive_accessibility_snapshot(&snapshot)
    } else {
        snapshot
    })
}

fn fetch_service_accessibility_tree(
    udid: &str,
    source: AccessibilitySource,
    max_depth: Option<usize>,
    include_hidden: bool,
    interactive_only: bool,
    server_url: &str,
) -> anyhow::Result<Value> {
    let mut query = vec![format!("source={}", source.as_query_value())];
    if let Some(max_depth) = max_depth {
        query.push(format!("maxDepth={}", max_depth.min(80)));
    }
    if include_hidden {
        query.push("includeHidden=true".to_owned());
    }
    if interactive_only {
        query.push("interactiveOnly=true".to_owned());
    }
    let path = format!(
        "/api/simulators/{}/accessibility-tree?{}",
        url_path_component(udid),
        query.join("&")
    );
    http_get_json(server_url, &path)
}

fn fetch_service_accessibility_point(
    udid: &str,
    x: f64,
    y: f64,
    server_url: &str,
) -> anyhow::Result<Value> {
    let path = format!(
        "/api/simulators/{}/accessibility-point?x={x}&y={y}",
        url_path_component(udid)
    );
    http_get_json(server_url, &path)
}

fn print_list_simulators(simulators: &Value, format: ListFormat) -> anyhow::Result<()> {
    match format {
        ListFormat::Json => {
            println_json(&serde_json::json!({ "simulators": simulators }))?;
        }
        ListFormat::CompactJson => {
            let compact = simulators
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .map(compact_simulator_list_entry)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            println!(
                "{}",
                serde_json::to_string(&serde_json::json!({ "simulators": compact }))?
            );
        }
    }
    Ok(())
}

fn compact_simulator_list_entry(simulator: &Value) -> Value {
    let mut entry = serde_json::Map::new();
    copy_json_field(simulator, &mut entry, "udid");
    copy_json_field(simulator, &mut entry, "name");
    copy_json_field(simulator, &mut entry, "state");
    copy_json_field(simulator, &mut entry, "isBooted");
    copy_json_field(simulator, &mut entry, "isAvailable");
    copy_json_field(simulator, &mut entry, "platform");
    copy_json_field(simulator, &mut entry, "deviceTypeName");
    copy_json_field(simulator, &mut entry, "runtimeName");
    if let Some(display) = simulator.get("privateDisplay") {
        copy_json_field_as(display, &mut entry, "displayStatus", "displayStatus");
        copy_json_field_as(display, &mut entry, "displayReady", "displayReady");
    }
    Value::Object(entry)
}

fn copy_json_field(source: &Value, target: &mut serde_json::Map<String, Value>, key: &str) {
    copy_json_field_as(source, target, key, key);
}

fn copy_json_field_as(
    source: &Value,
    target: &mut serde_json::Map<String, Value>,
    source_key: &str,
    target_key: &str,
) {
    if let Some(value) = source.get(source_key).filter(|value| !value.is_null()) {
        target.insert(target_key.to_owned(), value.clone());
    }
}

fn http_get_json(server_url: &str, path: &str) -> anyhow::Result<Value> {
    http_request_json(server_url, "GET", path, None)
}

include!("main/service_client.rs");

include!("main/http.rs");

include!("main/accessibility_format.rs");

include!("main/tap_target.rs");

include!("main/input_helpers.rs");

include!("main/maestro.rs");

include!("main/batch.rs");

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

fn parse_positive_seconds_arg(value: &str) -> Result<f64, String> {
    let seconds = value
        .trim()
        .parse::<f64>()
        .map_err(|_| "seconds must be a number".to_owned())?;
    if !seconds.is_finite() || seconds <= 0.0 {
        return Err("seconds must be finite and greater than zero".to_owned());
    }
    Ok(seconds)
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
    server_kind: ServerKind,
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
        server_kind,
        video_codec.as_env_value().to_owned(),
        low_latency,
        access_token,
        pairing_code,
    );
    let metrics = Arc::new(Metrics::default());
    let bridge = NativeBridge;
    let registry = SessionRegistry::new(bridge, metrics.clone());
    let logs = LogRegistry::default();
    let inspectors = InspectorHub::with_registry(InspectorRegistryAdvertisement::new(&config));
    let state = AppState {
        config: config.clone(),
        registry,
        logs,
        inspectors,
        metrics,
        performance: PerformanceRegistry::default(),
        stream_clients: Default::default(),
        simulator_inventory: Default::default(),
        accessibility_cache: Default::default(),
        android: Default::default(),
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
    let _bonjour_advertisement = BonjourAdvertisement::start(&config);

    info!("HTTP listening on http://{}", config.http_addr());
    info!("Serving client from {}", config.client_root.display());
    info!("API access token: {}", config.access_token);

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
    let termination_signal = shutdown_signal();
    tokio::pin!(termination_signal);

    tokio::select! {
        result = http_task => result??,
        result = health_task => result.context("server health heartbeat task panicked")?,
        _ = tokio::signal::ctrl_c() => {}
        _ = &mut quit_key_signal => {}
        _ = &mut termination_signal => {}
    }

    Ok(())
}

#[cfg(unix)]
async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};

    let mut terminate = match signal(SignalKind::terminate()) {
        Ok(signal) => signal,
        Err(error) => {
            warn!("Unable to install SIGTERM handler: {error}");
            std::future::pending::<()>().await;
            return;
        }
    };
    let mut hangup = match signal(SignalKind::hangup()) {
        Ok(signal) => signal,
        Err(error) => {
            warn!("Unable to install SIGHUP handler: {error}");
            std::future::pending::<()>().await;
            return;
        }
    };

    tokio::select! {
        _ = terminate.recv() => {}
        _ = hangup.recv() => {}
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() {
    std::future::pending::<()>().await
}

struct BonjourAdvertisement {
    child: Child,
}

impl BonjourAdvertisement {
    fn start(config: &Config) -> Option<Self> {
        if config.bind_ip.is_loopback() {
            return None;
        }
        let service_name = bonjour_service_name(&config.advertise_host);
        let server_id = auth::server_identity(config);
        let server_kind = config.server_kind.as_str();
        match ProcessCommand::new("dns-sd")
            .args([
                "-R",
                &service_name,
                "_simdeck._tcp.",
                "local.",
                &config.http_port.to_string(),
                &format!("sid={server_id}"),
                &format!("host={}", config.advertise_host),
                &format!("hid={}", config.host_id),
                &format!("hname={}", config.host_name),
                &format!("kind={server_kind}"),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => {
                info!(
                    "Advertising Bonjour service '{}' on _simdeck._tcp. port {}",
                    service_name, config.http_port
                );
                Some(Self { child })
            }
            Err(error) => {
                warn!("Unable to advertise Bonjour service with dns-sd: {error}");
                None
            }
        }
    }
}

impl Drop for BonjourAdvertisement {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn bonjour_service_name(advertise_host: &str) -> String {
    let host = advertise_host.trim();
    if host.is_empty() || host == "127.0.0.1" || host == "localhost" {
        "SimDeck".to_owned()
    } else {
        format!("SimDeck {host}")
    }
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
    use super::{
        batch_line_to_json_step, daemon_matches_launch_options, http_url_for_host,
        interactive_accessibility_snapshot, is_tailscale_ip, maestro_commands_from_flow,
        maestro_selector, normalize_accessibility_point_for_display, parse_maestro_flow_yaml,
        parse_maestro_point, parse_optional_udid_f64_args, parse_optional_udid_text_args,
        parse_optional_udid_value_args, parse_tap_command_args,
        parse_workspace_daemon_process_line, render_agent_accessibility_tree, render_qr_code,
        run_maestro_command, server_health_watchdog_should_restart,
        service_post_error_is_retryable, simdeck_pair_url, studio_daemon_restart_args,
        workspace_daemon_process_is_current, Cli, Command, DaemonCommand, DaemonLaunchOptions,
        DaemonMetadata, ElementSelector, PairingAddress, ServiceCommand, StreamQualityProfileArg,
        StudioExposeOptions, TapCommandTarget, VideoCodecMode, WorkspaceDaemonProcess, YamlValue,
        DEFAULT_LOCAL_STREAM_QUALITY_PROFILE, SERVER_HEALTH_WATCHDOG_FAILURE_THRESHOLD,
        SERVER_HEALTH_WATCHDOG_HTTP_FAILURE_THRESHOLD,
    };
    use clap::Parser;
    use std::collections::HashMap;
    use std::net::{IpAddr, Ipv4Addr};
    use std::path::{Path, PathBuf};

    fn daemon_metadata_for_test(
        port: u16,
        bind: &str,
        advertise_host: Option<&str>,
        client_root: Option<&str>,
    ) -> DaemonMetadata {
        DaemonMetadata {
            project_root: PathBuf::from("/tmp/project"),
            pid: 42,
            http_url: format!("http://127.0.0.1:{port}"),
            port,
            bind: bind.parse().unwrap(),
            advertise_host: advertise_host.map(str::to_owned),
            client_root: client_root.map(PathBuf::from),
            access_token: "token".to_owned(),
            pairing_code: Some("123456".to_owned()),
            binary_path: PathBuf::from("/tmp/simdeck-bin"),
            started_at: 1,
            log_path: None,
            video_codec: Some(VideoCodecMode::Auto.as_env_value().to_owned()),
            low_latency: false,
            realtime_stream: true,
            stream_quality_profile: Some(DEFAULT_LOCAL_STREAM_QUALITY_PROFILE.to_owned()),
            local_stream_fps: None,
        }
    }

    fn daemon_launch_options_for_test(
        port: u16,
        bind: &str,
        advertise_host: Option<&str>,
        client_root: Option<&str>,
    ) -> DaemonLaunchOptions {
        DaemonLaunchOptions {
            port,
            bind: bind.parse().unwrap(),
            advertise_host: advertise_host.map(str::to_owned),
            client_root: client_root.map(PathBuf::from),
            video_codec: VideoCodecMode::Auto,
            low_latency: false,
            realtime_stream: false,
            stream_quality_profile: Some(DEFAULT_LOCAL_STREAM_QUALITY_PROFILE.to_owned()),
            local_stream_fps: None,
        }
    }

    #[test]
    fn local_daemon_start_defaults_to_auto_video_codec() {
        let cli = Cli::parse_from(["simdeck", "daemon", "start"]);

        let Command::Daemon {
            command: DaemonCommand::Start { video_codec, .. },
        } = cli.command
        else {
            panic!("expected daemon start command");
        };
        assert_eq!(video_codec, VideoCodecMode::Auto);
    }

    #[test]
    fn local_daemon_start_accepts_named_video_codec_modes() {
        for (mode, expected) in [
            ("auto", VideoCodecMode::Auto),
            ("hardware", VideoCodecMode::Hardware),
            ("software", VideoCodecMode::Software),
            ("h264-software", VideoCodecMode::Software),
        ] {
            let cli = Cli::parse_from(["simdeck", "daemon", "start", "--video-codec", mode]);
            let Command::Daemon {
                command: DaemonCommand::Start { video_codec, .. },
            } = cli.command
            else {
                panic!("expected daemon start command");
            };
            assert_eq!(video_codec, expected);
        }
    }

    #[test]
    fn local_daemon_start_accepts_local_stream_fps_range() {
        let cli = Cli::parse_from(["simdeck", "daemon", "start", "--local-stream-fps", "240"]);
        let Command::Daemon {
            command: DaemonCommand::Start {
                local_stream_fps, ..
            },
        } = cli.command
        else {
            panic!("expected daemon start command");
        };
        assert_eq!(local_stream_fps, Some(240));
        assert!(
            Cli::try_parse_from(["simdeck", "daemon", "start", "--local-stream-fps", "241"])
                .is_err()
        );
    }

    #[test]
    fn screenshot_accepts_bezel_capture_flag() {
        let cli = Cli::parse_from(["simdeck", "screenshot", "SIM-1", "--with-bezel"]);

        let Command::Screenshot { with_bezel, .. } = cli.command else {
            panic!("expected screenshot command");
        };
        assert!(with_bezel);

        let cli = Cli::parse_from(["simdeck", "screenshot", "SIM-1", "--bezel"]);
        let Command::Screenshot { with_bezel, .. } = cli.command else {
            panic!("expected screenshot command");
        };
        assert!(with_bezel);
    }

    #[test]
    fn record_accepts_duration_output_and_stdout() {
        let cli = Cli::parse_from([
            "simdeck",
            "record",
            "SIM-1",
            "--seconds",
            "2.5",
            "--output",
            "capture.mp4",
        ]);

        let Command::Record {
            seconds,
            output,
            stdout,
            ..
        } = cli.command
        else {
            panic!("expected record command");
        };
        assert_eq!(seconds, 2.5);
        assert_eq!(output, Some(PathBuf::from("capture.mp4")));
        assert!(!stdout);

        let cli = Cli::parse_from(["simdeck", "record", "SIM-1", "--stdout"]);
        let Command::Record {
            seconds, stdout, ..
        } = cli.command
        else {
            panic!("expected record command");
        };
        assert_eq!(seconds, 5.0);
        assert!(stdout);
    }

    #[test]
    fn pair_command_defaults_to_lan_bind() {
        let cli = Cli::parse_from(["simdeck", "pair"]);

        let Command::Pair { bind, port, .. } = cli.command else {
            panic!("expected pair command");
        };
        assert_eq!(port, None);
        assert_eq!(bind, IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    }

    #[test]
    fn service_reset_command_accepts_service_options() {
        let cli = Cli::parse_from([
            "simdeck",
            "service",
            "reset",
            "--port",
            "4315",
            "--access-token",
            "explicit-token",
        ]);

        let Command::Service {
            command: ServiceCommand::Reset {
                port, access_token, ..
            },
        } = cli.command
        else {
            panic!("expected service reset command");
        };
        assert_eq!(port, 4315);
        assert_eq!(access_token.as_deref(), Some("explicit-token"));
    }

    #[test]
    fn workspace_daemon_process_parser_reads_supervised_command_paths() {
        let process = parse_workspace_daemon_process_line(
            " 8327 1 8327 /bin/sh -c script simdeck-supervisor /tmp/simdeck-bin daemon run --project-root /Users/dj/Developer/Flutter App Design --metadata-path /tmp/simdeck/flutter.json --port 4318 --bind 127.0.0.1 --access-token token --pairing-code 123456 --video-codec auto --server-kind workspace",
        )
        .expect("parse supervised daemon");

        assert_eq!(process.pid, 8327);
        assert_eq!(process.ppid, 1);
        assert_eq!(process.pgid, 8327);
        assert_eq!(
            process.project_root,
            PathBuf::from("/Users/dj/Developer/Flutter App Design")
        );
        assert_eq!(
            process.metadata_path,
            PathBuf::from("/tmp/simdeck/flutter.json")
        );
    }

    #[test]
    fn workspace_daemon_current_metadata_keeps_only_current_process_group() {
        let metadata_path = PathBuf::from("/tmp/simdeck/project.json");
        let metadata = DaemonMetadata {
            project_root: PathBuf::from("/tmp/project"),
            pid: 200,
            http_url: "http://127.0.0.1:4310".to_owned(),
            port: 4310,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            advertise_host: None,
            client_root: None,
            access_token: "token".to_owned(),
            pairing_code: Some("123456".to_owned()),
            binary_path: PathBuf::from("/tmp/simdeck-bin"),
            started_at: 1,
            log_path: None,
            video_codec: Some(VideoCodecMode::Auto.as_env_value().to_owned()),
            low_latency: false,
            realtime_stream: true,
            stream_quality_profile: Some(DEFAULT_LOCAL_STREAM_QUALITY_PROFILE.to_owned()),
            local_stream_fps: None,
        };
        let mut metadata_by_path = HashMap::new();
        metadata_by_path.insert(metadata_path.clone(), metadata);

        let current = WorkspaceDaemonProcess {
            pid: 201,
            ppid: 200,
            pgid: 200,
            project_root: PathBuf::from("/tmp/project"),
            metadata_path: metadata_path.clone(),
        };
        let orphaned = WorkspaceDaemonProcess {
            pgid: 199,
            ..current.clone()
        };

        assert!(workspace_daemon_process_is_current(
            &current,
            &metadata_by_path
        ));
        assert!(!workspace_daemon_process_is_current(
            &orphaned,
            &metadata_by_path
        ));
    }

    #[test]
    fn simdeck_pair_url_encodes_alternate_addresses() {
        let addresses = vec![
            PairingAddress {
                kind: "local",
                url: "http://127.0.0.1:4310".to_owned(),
            },
            PairingAddress {
                kind: "lan",
                url: "http://10.0.0.55:4310".to_owned(),
            },
            PairingAddress {
                kind: "tailscale",
                url: "http://100.112.42.69:4310".to_owned(),
            },
        ];

        let url = simdeck_pair_url(
            "http://10.0.0.55:4310",
            "123456",
            Some("server-1"),
            &addresses,
        );

        assert!(url.starts_with("simdeck://pair?u=10.0.0.55%3A4310&c=123456&s=server-1"));
        assert!(url.contains("a=100.112.42.69%3A4310"));
        assert!(!url.contains("127.0.0.1"));
    }

    #[test]
    fn qr_renderer_uses_compact_metro_style_blocks() {
        let qr = render_qr_code("simdeck://pair?url=http%3A%2F%2F10.0.0.55%3A4310&code=123456")
            .expect("render QR");

        assert!(qr.contains('█'));
        assert!(qr.contains(' '));
        assert!(!qr.contains("\x1b["));
        assert!(qr.lines().count() < 40);
    }

    #[test]
    fn tailscale_ip_detection_matches_100_64_10() {
        assert!(is_tailscale_ip("100.64.0.1".parse().unwrap()));
        assert!(is_tailscale_ip("100.127.255.254".parse().unwrap()));
        assert!(!is_tailscale_ip("100.128.0.1".parse().unwrap()));
        assert!(!is_tailscale_ip("10.0.0.55".parse().unwrap()));
    }

    #[test]
    fn http_url_for_host_brackets_ipv6() {
        assert_eq!(http_url_for_host("fe80::1", 4310), "http://[fe80::1]:4310");
        assert_eq!(
            http_url_for_host("10.0.0.55", 4310),
            "http://10.0.0.55:4310"
        );
    }

    #[test]
    fn legacy_h264_video_codec_modes_are_rejected() {
        assert!(
            Cli::try_parse_from(["simdeck", "daemon", "start", "--video-codec", "h264"]).is_err()
        );
    }

    #[test]
    fn daemon_launch_options_match_listener_metadata() {
        let metadata = daemon_metadata_for_test(4310, "127.0.0.1", None, None);
        let options = daemon_launch_options_for_test(4310, "127.0.0.1", None, None);

        assert!(daemon_matches_launch_options(&metadata, &options));
    }

    #[test]
    fn daemon_launch_options_reject_different_port() {
        let metadata = daemon_metadata_for_test(4310, "127.0.0.1", None, None);
        let options = daemon_launch_options_for_test(4320, "127.0.0.1", None, None);

        assert!(!daemon_matches_launch_options(&metadata, &options));
    }

    #[test]
    fn daemon_launch_options_accept_probed_port() {
        let metadata = daemon_metadata_for_test(4313, "127.0.0.1", None, None);
        let options = daemon_launch_options_for_test(4311, "127.0.0.1", None, None);

        assert!(daemon_matches_launch_options(&metadata, &options));
    }

    #[test]
    fn daemon_launch_options_reject_different_bind_or_client() {
        let metadata =
            daemon_metadata_for_test(4310, "127.0.0.1", Some("127.0.0.1"), Some("/tmp/client-a"));

        assert!(!daemon_matches_launch_options(
            &metadata,
            &daemon_launch_options_for_test(
                4310,
                "0.0.0.0",
                Some("127.0.0.1"),
                Some("/tmp/client-a"),
            ),
        ));
        assert!(!daemon_matches_launch_options(
            &metadata,
            &daemon_launch_options_for_test(
                4310,
                "127.0.0.1",
                Some("localhost"),
                Some("/tmp/client-a"),
            ),
        ));
        assert!(!daemon_matches_launch_options(
            &metadata,
            &daemon_launch_options_for_test(
                4310,
                "127.0.0.1",
                Some("127.0.0.1"),
                Some("/tmp/client-b"),
            ),
        ));
    }

    #[test]
    fn lifecycle_service_posts_retry_connection_resets() {
        assert!(service_post_error_is_retryable(
            "shutdown",
            "Connection reset by peer (os error 54)"
        ));
        assert!(service_post_error_is_retryable(
            "boot",
            "Resource temporarily unavailable"
        ));
        assert!(service_post_error_is_retryable(
            "launch",
            "SimDeck service returned HTTP 500: xcrun simctl launch timed out after 120s."
        ));
        assert!(service_post_error_is_retryable(
            "open-url",
            "Resource temporarily unavailable (os error 35)"
        ));
        assert!(!service_post_error_is_retryable(
            "touch",
            "Connection reset by peer (os error 54)"
        ));
    }

    #[test]
    fn describe_interactive_flag_prunes_agent_tree_but_keeps_context() {
        let parsed =
            Cli::try_parse_from(["simdeck", "describe", "sim-1", "--format", "agent", "-i"])
                .unwrap();
        let Command::DescribeUi {
            interactive_only, ..
        } = parsed.command
        else {
            panic!("expected describe command");
        };
        assert!(interactive_only);

        let snapshot = serde_json::json!({
            "source": "native-ax",
            "roots": [{
                "type": "Window",
                "children": [{
                    "type": "View",
                    "AXLabel": "Static wrapper",
                    "children": [{
                        "type": "Button",
                        "AXLabel": "Continue",
                        "enabled": true,
                        "children": []
                    }, {
                        "type": "Label",
                        "AXLabel": "Read only",
                        "children": []
                    }]
                }]
            }]
        });

        let pruned = interactive_accessibility_snapshot(&snapshot);
        let output = render_agent_accessibility_tree(&pruned);

        assert!(output.contains("- @e1 Window"));
        assert!(output.contains("- @e2 View: Static wrapper"));
        assert!(output.contains("- @e3 Button: Continue"));
        assert!(!output.contains("Read only"));
    }

    #[test]
    fn describe_agent_format_emits_stable_element_refs() {
        let snapshot = serde_json::json!({
            "source": "native-ax",
            "roots": [{
                "type": "Application",
                "AXLabel": "Fixture",
                "children": [{
                    "type": "Button",
                    "AXLabel": "Continue",
                    "children": []
                }]
            }]
        });

        let output = render_agent_accessibility_tree(&snapshot);

        assert!(output.contains("- @e1 Application: Fixture"));
        assert!(output.contains("  - @e2 Button: Continue"));
    }

    #[test]
    fn tap_single_positional_arg_is_label_shorthand() {
        let parsed = Cli::try_parse_from(["simdeck", "tap", "Continue"]).unwrap();
        let Command::Tap { args, .. } = parsed.command else {
            panic!("expected tap command");
        };
        let target = parse_tap_command_args(args, None, None, None, None).unwrap();

        assert_eq!(
            target,
            TapCommandTarget {
                udid: None,
                x: None,
                y: None,
                selector: ElementSelector {
                    label: Some("Continue".to_owned()),
                    ..Default::default()
                }
            }
        );
    }

    #[test]
    fn tap_agent_ref_maps_to_element_index() {
        let parsed = Cli::try_parse_from(["simdeck", "tap", "@e3"]).unwrap();
        let Command::Tap { args, .. } = parsed.command else {
            panic!("expected tap command");
        };
        let target = parse_tap_command_args(args, None, None, None, None).unwrap();

        assert_eq!(
            target.selector,
            ElementSelector {
                index: Some(2),
                ..Default::default()
            }
        );
        assert_eq!(target.udid, None);
    }

    #[test]
    fn tap_accepts_post_action_expectation_flags() {
        let parsed = Cli::try_parse_from([
            "simdeck",
            "tap",
            "--id",
            "com.apple.settings.screenTime",
            "--expect-id",
            "BackButton",
            "--expect-timeout-ms",
            "2500",
        ])
        .unwrap();
        let Command::Tap {
            expect_id,
            expect_timeout_ms,
            ..
        } = parsed.command
        else {
            panic!("expected tap command");
        };

        assert_eq!(expect_id.as_deref(), Some("BackButton"));
        assert_eq!(expect_timeout_ms, 2500);
    }

    #[test]
    fn agent_command_aliases_parse_like_primary_commands() {
        let parsed = Cli::try_parse_from(["simdeck", "snapshot", "sim-1"]).unwrap();
        assert!(matches!(parsed.command, Command::DescribeUi { .. }));

        let parsed = Cli::try_parse_from(["simdeck", "press", "Continue"]).unwrap();
        assert!(matches!(parsed.command, Command::Tap { .. }));

        let parsed = Cli::try_parse_from(["simdeck", "wait", "--label", "Continue"]).unwrap();
        assert!(matches!(parsed.command, Command::WaitFor { .. }));
    }

    #[test]
    fn back_command_accepts_default_device_and_timeout() {
        let parsed = Cli::try_parse_from([
            "simdeck",
            "back",
            "--timeout-ms",
            "3000",
            "--no-fallback-swipe",
        ])
        .unwrap();
        let Command::Back {
            udid,
            timeout_ms,
            fallback_swipe,
            ..
        } = parsed.command
        else {
            panic!("expected back command");
        };

        assert_eq!(udid, None);
        assert_eq!(timeout_ms, 3000);
        assert!(!fallback_swipe);
    }

    #[test]
    fn tap_legacy_udid_coordinates_still_parse() {
        let udid = "00000000-0000-0000-0000-000000000001";
        let target = parse_tap_command_args(
            vec![udid.to_owned(), "120".to_owned(), "240".to_owned()],
            None,
            None,
            None,
            None,
        )
        .unwrap();

        assert_eq!(target.udid.as_deref(), Some(udid));
        assert_eq!(target.x, Some(120.0));
        assert_eq!(target.y, Some(240.0));
        assert!(target.selector.is_empty());
    }

    #[test]
    fn tap_legacy_udid_label_shorthand_still_parse() {
        let udid = "00000000-0000-0000-0000-000000000001";
        let target = parse_tap_command_args(
            vec![udid.to_owned(), "Continue".to_owned()],
            None,
            None,
            None,
            None,
        )
        .unwrap();

        assert_eq!(target.udid.as_deref(), Some(udid));
        assert_eq!(target.selector.label.as_deref(), Some("Continue"));
    }

    #[test]
    fn global_device_flag_is_available_for_agent_shortcuts() {
        let parsed =
            Cli::try_parse_from(["simdeck", "--device", "iPhone 16", "tap", "Continue"]).unwrap();

        assert_eq!(parsed.device.as_deref(), Some("iPhone 16"));
    }

    #[test]
    fn use_command_accepts_udid_selector() {
        let parsed =
            Cli::try_parse_from(["simdeck", "use", "00000000-0000-0000-0000-000000000001"])
                .unwrap();

        let Command::Use { udid } = parsed.command else {
            panic!("expected use command");
        };
        assert_eq!(udid, "00000000-0000-0000-0000-000000000001");
    }

    #[test]
    fn device_commands_accept_omitted_udid() {
        let parsed = Cli::try_parse_from(["simdeck", "boot"]).unwrap();
        let Command::Boot { udid } = parsed.command else {
            panic!("expected boot command");
        };
        assert_eq!(udid, None);

        let parsed = Cli::try_parse_from(["simdeck", "home"]).unwrap();
        let Command::Home { udid } = parsed.command else {
            panic!("expected home command");
        };
        assert_eq!(udid, None);

        let parsed = Cli::try_parse_from(["simdeck", "screenshot", "--stdout"]).unwrap();
        let Command::Screenshot { udid, stdout, .. } = parsed.command else {
            panic!("expected screenshot command");
        };
        assert_eq!(udid, None);
        assert!(stdout);
    }

    #[test]
    fn payload_commands_keep_legacy_udid_but_allow_default_device() {
        let parsed = Cli::try_parse_from(["simdeck", "launch", "com.example.App"]).unwrap();
        let Command::Launch { args } = parsed.command else {
            panic!("expected launch command");
        };
        let (udid, bundle_id) =
            parse_optional_udid_value_args("launch", args, "BUNDLE_ID").unwrap();
        assert_eq!(udid, None);
        assert_eq!(bundle_id, "com.example.App");

        let parsed =
            Cli::try_parse_from(["simdeck", "launch", "SIM-1", "com.example.App"]).unwrap();
        let Command::Launch { args } = parsed.command else {
            panic!("expected launch command");
        };
        let (udid, bundle_id) =
            parse_optional_udid_value_args("launch", args, "BUNDLE_ID").unwrap();
        assert_eq!(udid.as_deref(), Some("SIM-1"));
        assert_eq!(bundle_id, "com.example.App");
    }

    #[test]
    fn coordinate_commands_keep_legacy_udid_but_allow_default_device() {
        let parsed = Cli::try_parse_from(["simdeck", "touch", "120", "240"]).unwrap();
        let Command::Touch { args, .. } = parsed.command else {
            panic!("expected touch command");
        };
        let (udid, points) = parse_optional_udid_f64_args("touch", args, 2).unwrap();
        assert_eq!(udid, None);
        assert_eq!(points, vec![120.0, 240.0]);

        let parsed =
            Cli::try_parse_from(["simdeck", "swipe", "SIM-1", "10", "20", "30", "40"]).unwrap();
        let Command::Swipe { args, .. } = parsed.command else {
            panic!("expected swipe command");
        };
        let (udid, points) = parse_optional_udid_f64_args("swipe", args, 4).unwrap();
        assert_eq!(udid.as_deref(), Some("SIM-1"));
        assert_eq!(points, vec![10.0, 20.0, 30.0, 40.0]);
    }

    #[test]
    fn text_commands_use_positional_text_or_legacy_udid_with_input_flags() {
        let parsed = Cli::try_parse_from(["simdeck", "type", "hello"]).unwrap();
        let Command::Type {
            args, stdin, file, ..
        } = parsed.command
        else {
            panic!("expected type command");
        };
        let (udid, text) =
            parse_optional_udid_text_args("type", args, stdin || file.is_some()).unwrap();
        assert_eq!(udid, None);
        assert_eq!(text.as_deref(), Some("hello"));

        let parsed = Cli::try_parse_from(["simdeck", "type", "SIM-1", "--stdin"]).unwrap();
        let Command::Type {
            args, stdin, file, ..
        } = parsed.command
        else {
            panic!("expected type command");
        };
        let (udid, text) =
            parse_optional_udid_text_args("type", args, stdin || file.is_some()).unwrap();
        assert_eq!(udid.as_deref(), Some("SIM-1"));
        assert_eq!(text, None);
    }

    #[test]
    fn batch_sleep_positional_duration_defaults_to_milliseconds() {
        let step = batch_line_to_json_step("sleep 500").unwrap();

        assert_eq!(step["action"], "sleep");
        assert_eq!(step["ms"], 500);
        assert!(step.get("seconds").is_none());
    }

    #[test]
    fn batch_sleep_accepts_explicit_seconds_and_milliseconds() {
        assert_eq!(batch_line_to_json_step("sleep 0.5s").unwrap()["ms"], 500);
        assert_eq!(
            batch_line_to_json_step("sleep --seconds 0.25").unwrap()["ms"],
            250
        );
        assert_eq!(
            batch_line_to_json_step("sleep --ms 125").unwrap()["ms"],
            125
        );
        assert_eq!(
            batch_line_to_json_step("sleep --duration-ms 75").unwrap()["ms"],
            75
        );
    }

    #[test]
    fn batch_wait_for_maps_selector_and_timeout_options() {
        let step = batch_line_to_json_step(
            "wait-for --id todo-title-1 --label Done --timeout-ms 750 --poll-interval-ms 25 --source native-ax --max-depth 4",
        )
        .unwrap();

        assert_eq!(step["action"], "waitFor");
        assert_eq!(step["selector"]["id"], "todo-title-1");
        assert_eq!(step["selector"]["label"], "Done");
        assert_eq!(step["timeoutMs"], 750);
        assert_eq!(step["pollMs"], 25);
        assert_eq!(step["source"], "native-ax");
        assert_eq!(step["maxDepth"], 4);
    }

    #[test]
    fn batch_tap_maps_post_action_expectation() {
        let step = batch_line_to_json_step(
            "tap --id com.apple.settings.screenTime --expect-id BackButton --expect-timeout-ms 2500",
        )
        .unwrap();

        assert_eq!(step["action"], "tap");
        assert_eq!(step["selector"]["id"], "com.apple.settings.screenTime");
        assert_eq!(step["expect"]["selector"]["id"], "BackButton");
        assert_eq!(step["expect"]["timeoutMs"], 2500);

        let back = batch_line_to_json_step("back --timeout-ms 3000 --no-fallback-swipe").unwrap();
        assert_eq!(back["action"], "back");
        assert_eq!(back["timeoutMs"], 3000);
        assert_eq!(back["fallbackSwipe"], false);
    }

    #[test]
    fn batch_assert_maps_to_assert_action() {
        let step = batch_line_to_json_step("assert --value Ready").unwrap();

        assert_eq!(step["action"], "assert");
        assert_eq!(step["selector"]["value"], "Ready");
        assert_eq!(step["timeoutMs"], 5000);
    }

    #[test]
    fn batch_assert_not_and_scroll_map_to_daemon_actions() {
        let assert_not = batch_line_to_json_step("assert-not --text Loading --regex").unwrap();
        assert_eq!(assert_not["action"], "assertNot");
        assert_eq!(assert_not["selector"]["text"], "Loading");
        assert_eq!(assert_not["selector"]["regex"], true);

        let scroll =
            batch_line_to_json_step("scroll-until-visible --text Settings --direction down")
                .unwrap();
        assert_eq!(scroll["action"], "scrollUntilVisible");
        assert_eq!(scroll["selector"]["text"], "Settings");
        assert_eq!(scroll["direction"], "down");
    }

    #[test]
    fn maestro_flow_accepts_config_with_commands() {
        let yaml = parse_maestro_flow_yaml(
            r#"
appId: com.example.App
---
- launchApp
- tapOn: Continue
"#,
        )
        .unwrap();
        let commands = maestro_commands_from_flow(&yaml).unwrap();
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0]["launchApp"].as_str(), Some("com.example.App"));
    }

    #[test]
    fn maestro_selector_maps_text_and_state() {
        let yaml: YamlValue = serde_yaml::from_str(
            r#"
text: Continue.*
enabled: true
index: 1
"#,
        )
        .unwrap();
        let selector = maestro_selector(&yaml).unwrap();
        assert_eq!(selector["text"], "Continue.*");
        assert_eq!(selector["enabled"], true);
        assert_eq!(selector["index"], 1);
        assert_eq!(selector["regex"], true);
    }

    #[test]
    fn maestro_selector_keeps_id_literals_exact_by_default() {
        let yaml: YamlValue = serde_yaml::from_str("id: login.button").unwrap();
        let selector = maestro_selector(&yaml).unwrap();

        assert_eq!(selector["id"], "login.button");
        assert_eq!(selector["regex"], false);
    }

    #[test]
    fn maestro_selector_escapes_literal_ids_when_text_requires_regex() {
        let yaml: YamlValue = serde_yaml::from_str(
            r#"
text: Continue.*
id: login.button
"#,
        )
        .unwrap();
        let selector = maestro_selector(&yaml).unwrap();

        assert_eq!(selector["text"], "Continue.*");
        assert_eq!(selector["id"], "^login\\.button$");
        assert_eq!(selector["regex"], true);
    }

    #[test]
    fn maestro_swipe_rejects_unknown_directions() {
        let command: YamlValue = serde_yaml::from_str(
            r#"
swipe:
  direction: rigth
"#,
        )
        .unwrap();
        let error =
            run_maestro_command("http://127.0.0.1:9", "test-udid", &command, Path::new("."))
                .unwrap_err();

        assert!(error
            .to_string()
            .contains("Unsupported Maestro swipe direction `rigth`"));
    }

    #[test]
    fn maestro_percent_points_become_normalized_coordinates() {
        assert_eq!(parse_maestro_point("50%,75%").unwrap(), (0.5, 0.75));
    }

    #[test]
    fn maestro_tap_on_string_maps_to_text_selector() {
        let yaml: YamlValue = serde_yaml::from_str("Continue").unwrap();
        let body = super::maestro_tap_body(&yaml).unwrap();
        assert_eq!(body["selector"]["text"], "Continue");
    }

    #[test]
    fn server_health_watchdog_restarts_when_http_listener_is_unhealthy() {
        assert!(server_health_watchdog_should_restart(
            0,
            SERVER_HEALTH_WATCHDOG_HTTP_FAILURE_THRESHOLD
        ));
    }

    #[test]
    fn server_health_watchdog_waits_for_transient_http_probe_failures() {
        assert!(!server_health_watchdog_should_restart(
            0,
            SERVER_HEALTH_WATCHDOG_HTTP_FAILURE_THRESHOLD - 1
        ));
    }

    #[test]
    fn server_health_watchdog_restarts_when_runtime_heartbeat_is_stale() {
        assert!(server_health_watchdog_should_restart(
            SERVER_HEALTH_WATCHDOG_FAILURE_THRESHOLD,
            0
        ));
    }

    #[test]
    fn studio_daemon_restart_args_preserve_remote_stream_defaults() {
        let args = studio_daemon_restart_args(&StudioExposeOptions {
            simulator: None,
            studio_url: "https://simdeck.djdev.me".to_owned(),
            port: 4310,
            bind: "127.0.0.1".parse().unwrap(),
            video_codec: VideoCodecMode::Software,
            low_latency: false,
            stream_quality: None,
            local_stream_fps: None,
        });
        assert_eq!(
            args,
            [
                "daemon",
                "restart",
                "--port",
                "4310",
                "--bind",
                "127.0.0.1",
                "--video-codec",
                "software",
                "--stream-quality",
                "smooth",
            ]
        );
    }

    #[test]
    fn studio_daemon_restart_args_preserve_explicit_quality() {
        let args = studio_daemon_restart_args(&StudioExposeOptions {
            simulator: None,
            studio_url: "https://simdeck.djdev.me".to_owned(),
            port: 4310,
            bind: "127.0.0.1".parse().unwrap(),
            video_codec: VideoCodecMode::Hardware,
            low_latency: false,
            stream_quality: Some(StreamQualityProfileArg::Balanced),
            local_stream_fps: None,
        });
        assert!(args.ends_with(&[
            "--video-codec".to_owned(),
            "hardware".to_owned(),
            "--stream-quality".to_owned(),
            "balanced".to_owned(),
        ]));
    }

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
