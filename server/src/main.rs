mod api;
mod config;
mod error;
mod logging;
mod metrics;
mod native;
mod simulators;
mod static_files;
mod transport;

use anyhow::Context;
use api::routes::{router, AppState};
use clap::{Parser, Subcommand};
use config::Config;
use metrics::counters::Metrics;
use native::bridge::NativeBridge;
use native::ffi;
use simulators::registry::SessionRegistry;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::Arc;
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
}

fn main() -> anyhow::Result<()> {
    logging::init();
    let cli = Cli::parse();
    let bridge = NativeBridge::default();

    match cli.command {
        Command::Serve {
            port,
            bind,
            advertise_host,
            client_root,
        } => serve_with_appkit(port, bind, advertise_host, client_root),
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
    }
}

fn serve_with_appkit(
    port: u16,
    bind: IpAddr,
    advertise_host: Option<String>,
    client_root: Option<PathBuf>,
) -> anyhow::Result<()> {
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
            Ok(runtime) => runtime.block_on(serve(port, bind, advertise_host, client_root)),
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

async fn serve(
    port: u16,
    bind: IpAddr,
    advertise_host: Option<String>,
    client_root: Option<PathBuf>,
) -> anyhow::Result<()> {
    let root = std::env::current_dir()?.join("client").join("dist");
    let config = Config::new(port, client_root.unwrap_or(root), bind, advertise_host);
    let metrics = Arc::new(Metrics::default());
    let bridge = NativeBridge::default();
    let registry = SessionRegistry::new(bridge, metrics.clone());
    let (wt_runtime, wt_endpoint) = transport::webtransport::prepare(&config).await?;
    let state = AppState {
        config: config.clone(),
        registry,
        metrics,
        wt_endpoint_template: wt_runtime.endpoint_url_template.clone(),
        certificate_hash_hex: wt_runtime.certificate_hash_hex.clone(),
    };

    let http_router = router(state.clone()).fallback_service(static_files::service(&config));
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
