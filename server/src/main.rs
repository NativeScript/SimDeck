mod api;
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
use clap::{Parser, Subcommand, ValueEnum};
use config::Config;
use inspector::InspectorHub;
use logs::LogRegistry;
use metrics::counters::Metrics;
use native::bridge::NativeBridge;
use native::bridge::NativeSession;
use native::ffi;
use serde_json::Value;
use simulators::registry::SessionRegistry;
use std::fs;
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

#[derive(Parser)]
#[command(name = "xcode-canvas-web")]
#[command(bin_name = "xcode-canvas-web")]
#[command(about = "Local simulator control plane and browser transport server")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Serve {
        #[arg(long, default_value_t = 4310)]
        port: u16,
        #[arg(long, default_value_t = IpAddr::V4(Ipv4Addr::LOCALHOST))]
        bind: IpAddr,
        #[arg(long)]
        advertise_host: Option<String>,
        #[arg(long)]
        client_root: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = VideoCodecMode::Hevc)]
        video_codec: VideoCodecMode,
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
    DescribeUi {
        udid: String,
        #[arg(long, value_parser = parse_point)]
        point: Option<(f64, f64)>,
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
        #[arg(long, value_enum, default_value_t = VideoCodecMode::Hevc)]
        video_codec: VideoCodecMode,
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
        #[arg(long, value_enum, default_value_t = VideoCodecMode::Hevc)]
        video_codec: VideoCodecMode,
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

impl VideoCodecMode {
    fn as_env_value(self) -> &'static str {
        match self {
            Self::Hevc => "hevc",
            Self::H264 => "h264",
            Self::H264Software => "h264-software",
        }
    }
}

fn main() -> anyhow::Result<()> {
    logging::init();
    let cli = Cli::parse();
    let bridge = NativeBridge;

    match cli.command {
        Command::Serve {
            port,
            bind,
            advertise_host,
            client_root,
            video_codec,
        } => serve_with_appkit(port, bind, advertise_host, client_root, video_codec),
        Command::Service { command } => match command {
            ServiceCommand::On {
                port,
                bind,
                advertise_host,
                client_root,
                video_codec,
            } => service::enable(ServiceOptions {
                port,
                bind,
                advertise_host,
                client_root,
                video_codec,
            }),
            ServiceCommand::Restart {
                port,
                bind,
                advertise_host,
                client_root,
                video_codec,
            } => service::restart(ServiceOptions {
                port,
                bind,
                advertise_host,
                client_root,
                video_codec,
            }),
            ServiceCommand::Off => service::disable(),
        },
        Command::CoreSimulator { command } => match command {
            CoreSimulatorCommand::Start => core_simulator::start(),
            CoreSimulatorCommand::Shutdown => core_simulator::shutdown(),
            CoreSimulatorCommand::Restart => core_simulator::restart(),
        },
        Command::List => {
            let simulators = bridge.list_simulators()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({ "simulators": simulators }))?
            );
            Ok(())
        }
        Command::Boot { udid } => {
            bridge.boot_simulator(&udid)?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({ "ok": true, "udid": udid, "action": "boot" })
                )?
            );
            Ok(())
        }
        Command::Shutdown { udid } => {
            bridge.shutdown_simulator(&udid)?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({ "ok": true, "udid": udid, "action": "shutdown" })
                )?
            );
            Ok(())
        }
        Command::OpenUrl { udid, url } => {
            bridge.open_url(&udid, &url)?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({ "ok": true, "udid": udid, "url": url })
                )?
            );
            Ok(())
        }
        Command::Launch { udid, bundle_id } => {
            bridge.launch_bundle(&udid, &bundle_id)?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({ "ok": true, "udid": udid, "bundleId": bundle_id })
                )?
            );
            Ok(())
        }
        Command::ToggleAppearance { udid } => {
            bridge.toggle_appearance(&udid)?;
            println_json(
                &serde_json::json!({ "ok": true, "udid": udid, "action": "toggle-appearance" }),
            )?;
            Ok(())
        }
        Command::Erase { udid } => {
            bridge.erase_simulator(&udid)?;
            println_json(&serde_json::json!({ "ok": true, "udid": udid, "action": "erase" }))?;
            Ok(())
        }
        Command::Install { udid, app_path } => {
            bridge.install_app(&udid, &app_path)?;
            println_json(
                &serde_json::json!({ "ok": true, "udid": udid, "action": "install", "appPath": app_path }),
            )?;
            Ok(())
        }
        Command::Uninstall { udid, bundle_id } => {
            bridge.uninstall_app(&udid, &bundle_id)?;
            println_json(
                &serde_json::json!({ "ok": true, "udid": udid, "action": "uninstall", "bundleId": bundle_id }),
            )?;
            Ok(())
        }
        Command::Pasteboard { command } => match command {
            PasteboardCommand::Get { udid } => {
                let text = bridge.pasteboard_text(&udid)?;
                println_json(&serde_json::json!({ "udid": udid, "text": text }))?;
                Ok(())
            }
            PasteboardCommand::Set {
                udid,
                text,
                stdin,
                file,
            } => {
                let text = read_text_input(text, stdin, file)?;
                bridge.set_pasteboard_text(&udid, &text)?;
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
            let filters = native::bridge::LogFilters::new(Vec::new(), Vec::new(), String::new());
            let entries = bridge.recent_logs(&udid, seconds, limit, &filters)?;
            println_json(&serde_json::json!({ "entries": entries }))?;
            Ok(())
        }
        Command::Screenshot {
            udid,
            output,
            stdout,
        } => {
            let png = bridge.screenshot_png(&udid)?;
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
        Command::DescribeUi { udid, point } => {
            let snapshot = bridge.accessibility_snapshot(&udid, point)?;
            println_json(&snapshot)?;
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
            let (x, y) = resolve_touch_point(&bridge, &udid, x, y, normalized)?;
            if down || up {
                if down {
                    bridge.send_touch(&udid, x, y, "began")?;
                }
                if down && up {
                    std::thread::sleep(Duration::from_millis(delay_ms));
                }
                if up {
                    bridge.send_touch(&udid, x, y, "ended")?;
                }
            } else {
                bridge.send_touch(&udid, x, y, &phase)?;
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
            let (x, y) = resolve_tap_target(
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
            perform_tap(&bridge, &udid, x, y, duration_ms)?;
            sleep_ms(post_delay_ms);
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
                12,
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
            if duration_ms > 0 && modifiers == 0 {
                bridge.send_key_event(&udid, key_code, true)?;
                sleep_ms(duration_ms);
                bridge.send_key_event(&udid, key_code, false)?;
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
            for (index, key) in keys.iter().enumerate() {
                bridge.send_key(&udid, *key, 0)?;
                if index + 1 < keys.len() {
                    sleep_ms(delay_ms);
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
            bridge.send_key(&udid, key_code, modifier_mask)?;
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
            bridge.press_button(&udid, &button, duration_ms)?;
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
            let report = run_batch(&bridge, &udid, steps, file, stdin, continue_on_error)?;
            println_json(&report)?;
            Ok(())
        }
        Command::DismissKeyboard { udid } => {
            bridge.send_key(&udid, 41, 0)?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({ "ok": true, "udid": udid, "action": "dismiss-keyboard" })
                )?
            );
            Ok(())
        }
        Command::Home { udid } => {
            bridge.press_home(&udid)?;
            println_json(&serde_json::json!({ "ok": true, "udid": udid, "action": "home" }))?;
            Ok(())
        }
        Command::AppSwitcher { udid } => {
            bridge.press_home(&udid)?;
            std::thread::sleep(Duration::from_millis(140));
            bridge.press_home(&udid)?;
            println_json(
                &serde_json::json!({ "ok": true, "udid": udid, "action": "app-switcher" }),
            )?;
            Ok(())
        }
        Command::RotateLeft { udid } => {
            run_session_action_with_appkit("rotate left", udid.clone(), |session| {
                session.rotate_left()
            })?;
            println_json(
                &serde_json::json!({ "ok": true, "udid": udid, "action": "rotate-left" }),
            )?;
            Ok(())
        }
        Command::RotateRight { udid } => {
            run_session_action_with_appkit("rotate right", udid.clone(), |session| {
                session.rotate_right()
            })?;
            println_json(
                &serde_json::json!({ "ok": true, "udid": udid, "action": "rotate-right" }),
            )?;
            Ok(())
        }
        Command::ChromeProfile { udid } => {
            let profile = bridge.chrome_profile(&udid)?;
            println_json(&serde_json::json!(profile))?;
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
}

fn serve_with_appkit(
    port: u16,
    bind: IpAddr,
    advertise_host: Option<String>,
    client_root: Option<PathBuf>,
    video_codec: VideoCodecMode,
) -> anyhow::Result<()> {
    std::env::set_var("XCW_VIDEO_CODEC", video_codec.as_env_value());
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
            Ok(runtime) => {
                runtime.block_on(serve(port, bind, advertise_host, client_root, video_codec))
            }
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

fn run_session_action_with_appkit<F>(
    label: &'static str,
    udid: String,
    action: F,
) -> anyhow::Result<()>
where
    F: FnOnce(&NativeSession) -> Result<(), crate::error::AppError> + Send + 'static,
{
    unsafe {
        ffi::xcw_native_initialize_app();
    }

    let (result_tx, result_rx) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let result: anyhow::Result<()> = (|| {
            let bridge = NativeBridge;
            let session = bridge.create_session(&udid)?;
            session.start()?;
            action(&session)?;
            Ok(())
        })();
        let _ = result_tx.send(result);
    });

    loop {
        match result_rx.try_recv() {
            Ok(result) => return result,
            Err(mpsc::TryRecvError::Disconnected) => {
                anyhow::bail!("{label} worker exited unexpectedly");
            }
            Err(mpsc::TryRecvError::Empty) => unsafe {
                ffi::xcw_native_run_main_loop_slice(0.05);
            },
        }
    }
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
    bridge.send_touch(udid, x, y, "began")?;
    sleep_ms(duration_ms);
    bridge.send_touch(udid, x, y, "ended")
}

fn perform_swipe(
    bridge: &NativeBridge,
    udid: &str,
    gesture: GestureCoordinates,
    steps: u32,
) -> Result<(), crate::error::AppError> {
    let step_count = steps.max(1);
    let delay = Duration::from_millis(gesture.duration_ms / u64::from(step_count));
    bridge.send_touch(udid, gesture.start_x, gesture.start_y, "began")?;
    for step in 1..step_count {
        let t = f64::from(step) / f64::from(step_count);
        bridge.send_touch(
            udid,
            lerp(gesture.start_x, gesture.end_x, t),
            lerp(gesture.start_y, gesture.end_y, t),
            "moved",
        )?;
        std::thread::sleep(delay);
    }
    bridge.send_touch(udid, gesture.end_x, gesture.end_y, "ended")
}

fn type_text(
    bridge: &NativeBridge,
    udid: &str,
    text: &str,
    delay_ms: u64,
) -> Result<(), crate::error::AppError> {
    for character in text.chars() {
        let Some((key_code, modifiers)) = hid_for_character(character) else {
            return Err(crate::error::AppError::bad_request(format!(
                "Unsupported character for HID typing: {character:?}"
            )));
        };
        bridge.send_key(udid, key_code, modifiers)?;
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

fn resolve_tap_target(
    bridge: &NativeBridge,
    udid: &str,
    request: TapTargetRequest,
) -> Result<(f64, f64), crate::error::AppError> {
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
        return resolve_touch_point(bridge, udid, x, y, request.normalized);
    }

    let deadline = std::time::Instant::now() + Duration::from_millis(request.wait_timeout_ms);
    loop {
        let snapshot = bridge.accessibility_snapshot(udid, None)?;
        if let Some((point_x, point_y)) = find_element_center(&snapshot, &request.selector) {
            return resolve_touch_point(bridge, udid, point_x, point_y, false);
        }
        if request.wait_timeout_ms == 0 || std::time::Instant::now() >= deadline {
            return Err(crate::error::AppError::not_found(
                "No accessibility element matched the tap selector.",
            ));
        }
        sleep_ms(request.poll_interval_ms.max(10));
    }
}

fn find_element_center(snapshot: &Value, selector: &ElementSelector) -> Option<(f64, f64)> {
    let roots = snapshot.get("roots")?.as_array()?;
    let mut matches = Vec::new();
    for root in roots {
        collect_matching_elements(root, selector, &mut matches);
    }
    matches
        .into_iter()
        .max_by_key(|node| is_actionable_element(node) as u8)
        .and_then(element_center)
}

fn collect_matching_elements<'a>(
    node: &'a Value,
    selector: &ElementSelector,
    matches: &mut Vec<&'a Value>,
) {
    if element_matches(node, selector) {
        matches.push(node);
    }
    if let Some(children) = node.get("children").and_then(Value::as_array) {
        for child in children {
            collect_matching_elements(child, selector, matches);
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
            perform_tap(bridge, udid, target.0, target.1, duration_ms)?;
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
                if args.flag("down") {
                    bridge.send_touch(udid, x, y, "began")?;
                }
                if args.flag("down") && args.flag("up") {
                    sleep_ms(
                        args.value("delay-ms")
                            .and_then(|value| value.parse().ok())
                            .unwrap_or(100),
                    );
                }
                if args.flag("up") {
                    bridge.send_touch(udid, x, y, "ended")?;
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
            for (index, key) in keys.iter().enumerate() {
                bridge.send_key(udid, *key, 0)?;
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

async fn serve(
    port: u16,
    bind: IpAddr,
    advertise_host: Option<String>,
    client_root: Option<PathBuf>,
    video_codec: VideoCodecMode,
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
        wt_endpoint_template: wt_runtime.endpoint_url_template.clone(),
        certificate_hash_hex: wt_runtime.certificate_hash_hex.clone(),
    };

    let client_root = config.client_root.clone();
    let http_router = router(state.clone())
        .fallback(move |method, uri| static_files::serve_static(client_root.clone(), method, uri));
    let http_listener = tokio::net::TcpListener::bind(config.http_addr())
        .await
        .with_context(|| format!("bind HTTP listener on {}", config.http_addr()))?;

    info!("HTTP listening on http://{}", config.http_addr());
    info!(
        "WebTransport listening on {}",
        wt_runtime.endpoint_url_template
    );
    info!("Serving client from {}", config.client_root.display());
    if config.bind_ip.is_unspecified() && config.advertise_host == Ipv4Addr::LOCALHOST.to_string() {
        warn!(
            "Server is listening on all interfaces, but WebTransport is still advertised as localhost. \
Use --advertise-host <LAN-IP-or-DNS-name> for remote browser access."
        );
    }

    let http_task = tokio::spawn(async move {
        axum::serve(http_listener, http_router)
            .await
            .context("serve HTTP")
    });
    let wt_task =
        tokio::spawn(async move { transport::webtransport::serve(wt_endpoint, state).await });

    tokio::select! {
        result = http_task => result??,
        result = wt_task => result??,
        _ = tokio::signal::ctrl_c() => {}
    }

    Ok(())
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
