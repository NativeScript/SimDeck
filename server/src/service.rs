use crate::{auth, default_client_root, ServiceOptions};
use anyhow::{anyhow, bail, Context};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

const SERVICE_LABEL: &str = "dev.nativescript.simdeck";

#[derive(Clone, Debug)]
pub struct ServiceInstallResult {
    pub service: String,
    pub plist_path: PathBuf,
    pub stdout_log: PathBuf,
    pub stderr_log: PathBuf,
    pub port: u16,
    pub advertise_host: Option<String>,
    pub access_token: Option<String>,
    pub pairing_code: Option<String>,
}

pub fn enable(options: ServiceOptions) -> anyhow::Result<()> {
    let result = install(options)?;
    print_install_result(&result)
}

pub fn restart(options: ServiceOptions) -> anyhow::Result<()> {
    let result = install(options)?;
    print_install_result(&result)
}

pub fn pair(mut options: ServiceOptions) -> anyhow::Result<ServiceInstallResult> {
    let existing_credentials = installed_credentials().unwrap_or(None);
    if options.access_token.is_none() {
        options.access_token = existing_credentials
            .as_ref()
            .and_then(|credentials| credentials.access_token.clone())
            .or_else(|| Some(auth::generate_access_token()));
    }
    if options.pairing_code.is_none() {
        options.pairing_code = existing_credentials
            .as_ref()
            .and_then(|credentials| credentials.pairing_code.clone())
            .or_else(|| Some(auth::generate_pairing_code()));
    }
    install(options)
}

pub fn installed_port() -> anyhow::Result<Option<u16>> {
    Ok(installed_argument_value("--port")?.and_then(|value| value.parse::<u16>().ok()))
}

fn install(options: ServiceOptions) -> anyhow::Result<ServiceInstallResult> {
    let plist_path = plist_path()?;
    let log_dir = log_dir()?;
    fs::create_dir_all(
        plist_path
            .parent()
            .ok_or_else(|| anyhow!("resolve LaunchAgents directory"))?,
    )?;
    fs::create_dir_all(&log_dir)?;

    let client_root = match options.client_root.as_ref() {
        Some(path) => path.clone(),
        None => default_client_root()?,
    };
    let executable = std::env::current_exe().context("resolve current executable path")?;
    let stdout_log = log_dir.join("simdeck.log");
    let stderr_log = log_dir.join("simdeck.err.log");
    let plist = plist_contents(
        &executable,
        &client_root,
        &stdout_log,
        &stderr_log,
        &options,
    );

    fs::write(&plist_path, plist).with_context(|| format!("write {}", plist_path.display()))?;

    let domain = launchctl_domain()?;
    let _ = Command::new("launchctl")
        .args(["bootout", &domain, plist_path.to_string_lossy().as_ref()])
        .output();
    wait_for_port_release(options.port, Duration::from_secs(5))?;

    run_launchctl(["bootstrap", &domain, plist_path.to_string_lossy().as_ref()])?;
    run_launchctl(["kickstart", "-k", &format!("{domain}/{SERVICE_LABEL}")])?;

    let advertise_host = options.advertise_host.clone();
    let access_token = options.access_token.clone();
    let pairing_code = options.pairing_code.clone();
    Ok(ServiceInstallResult {
        service: SERVICE_LABEL.to_owned(),
        plist_path,
        stdout_log,
        stderr_log,
        port: options.port,
        advertise_host,
        access_token,
        pairing_code,
    })
}

fn print_install_result(result: &ServiceInstallResult) -> anyhow::Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "ok": true,
            "service": result.service,
            "plist": result.plist_path,
            "stdoutLog": result.stdout_log,
            "stderrLog": result.stderr_log,
        }))?
    );
    Ok(())
}

pub fn disable() -> anyhow::Result<()> {
    let plist_path = plist_path()?;
    let domain = launchctl_domain()?;

    if plist_path.exists() {
        let _ = Command::new("launchctl")
            .args(["bootout", &domain, plist_path.to_string_lossy().as_ref()])
            .output();
        fs::remove_file(&plist_path).with_context(|| format!("remove {}", plist_path.display()))?;
    }

    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "ok": true,
            "service": SERVICE_LABEL,
            "plist": plist_path,
        }))?
    );
    Ok(())
}

fn plist_path() -> anyhow::Result<PathBuf> {
    Ok(home_dir()?
        .join("Library/LaunchAgents")
        .join(format!("{SERVICE_LABEL}.plist")))
}

fn log_dir() -> anyhow::Result<PathBuf> {
    Ok(home_dir()?.join("Library/Logs"))
}

#[derive(Clone, Debug)]
struct ServiceCredentials {
    access_token: Option<String>,
    pairing_code: Option<String>,
}

fn installed_credentials() -> anyhow::Result<Option<ServiceCredentials>> {
    let Some(arguments) = installed_arguments()? else {
        return Ok(None);
    };
    Ok(Some(ServiceCredentials {
        access_token: argument_value(&arguments, "--access-token"),
        pairing_code: argument_value(&arguments, "--pairing-code"),
    }))
}

fn installed_argument_value(name: &str) -> anyhow::Result<Option<String>> {
    Ok(installed_arguments()?
        .as_deref()
        .and_then(|arguments| argument_value(arguments, name)))
}

fn installed_arguments() -> anyhow::Result<Option<Vec<String>>> {
    let plist_path = plist_path()?;
    if !plist_path.exists() {
        return Ok(None);
    }
    let plist = plist::Value::from_file(&plist_path)
        .with_context(|| format!("read {}", plist_path.display()))?;
    let Some(arguments) = plist
        .as_dictionary()
        .and_then(|dict| dict.get("ProgramArguments"))
        .and_then(|value| value.as_array())
    else {
        return Ok(None);
    };
    let arguments = arguments
        .iter()
        .filter_map(|value| value.as_string())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    Ok(Some(arguments))
}

fn argument_value(arguments: &[String], name: &str) -> Option<String> {
    arguments
        .windows(2)
        .find(|window| window.first().is_some_and(|value| value == name))
        .and_then(|window| window.get(1))
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn home_dir() -> anyhow::Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("HOME is not set"))
}

fn launchctl_domain() -> anyhow::Result<String> {
    let output = Command::new("id")
        .arg("-u")
        .output()
        .context("run `id -u`")?;
    if !output.status.success() {
        bail!("`id -u` failed");
    }
    let uid = String::from_utf8(output.stdout)
        .context("parse uid as utf-8")?
        .trim()
        .to_string();
    if uid.is_empty() {
        bail!("`id -u` returned an empty uid");
    }
    Ok(format!("gui/{uid}"))
}

fn run_launchctl<const N: usize>(args: [&str; N]) -> anyhow::Result<()> {
    let output = Command::new("launchctl")
        .args(args)
        .output()
        .context("run launchctl")?;
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    bail!(
        "launchctl {} failed: {}",
        args.join(" "),
        if stderr.is_empty() {
            "unknown error"
        } else {
            &stderr
        }
    );
}

fn wait_for_port_release(port: u16, timeout: Duration) -> anyhow::Result<()> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !port_has_listener(port)? {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
    bail!("port {port} is still in use after waiting for previous service shutdown");
}

fn port_has_listener(port: u16) -> anyhow::Result<bool> {
    let output = Command::new("lsof")
        .args(["-nP", &format!("-iTCP:{port}"), "-sTCP:LISTEN", "-t"])
        .output()
        .context("run lsof")?;
    Ok(!output.stdout.is_empty())
}

fn plist_contents(
    executable: &Path,
    client_root: &Path,
    stdout_log: &Path,
    stderr_log: &Path,
    options: &ServiceOptions,
) -> String {
    let mut program_arguments = vec![
        executable.to_string_lossy().into_owned(),
        "serve".to_string(),
        "--port".to_string(),
        options.port.to_string(),
        "--bind".to_string(),
        options.bind.to_string(),
        "--client-root".to_string(),
        client_root.to_string_lossy().into_owned(),
        "--video-codec".to_string(),
        options.video_codec.as_env_value().to_string(),
    ];
    if options.low_latency {
        program_arguments.push("--low-latency".to_string());
    }
    if let Some(stream_quality_profile) = options.stream_quality_profile.as_ref() {
        program_arguments.push("--stream-quality".to_string());
        program_arguments.push(stream_quality_profile.clone());
    }
    if let Some(local_stream_fps) = options.local_stream_fps {
        program_arguments.push("--local-stream-fps".to_string());
        program_arguments.push(local_stream_fps.to_string());
    }

    if let Some(advertise_host) = options.advertise_host.as_ref() {
        program_arguments.push("--advertise-host".to_string());
        program_arguments.push(advertise_host.clone());
    }
    if let Some(access_token) = options.access_token.as_ref() {
        program_arguments.push("--access-token".to_string());
        program_arguments.push(access_token.clone());
    }
    if let Some(pairing_code) = options.pairing_code.as_ref() {
        program_arguments.push("--pairing-code".to_string());
        program_arguments.push(pairing_code.clone());
    }

    let program_arguments_xml = program_arguments
        .into_iter()
        .map(|argument| format!("    <string>{}</string>", xml_escape(&argument)))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{label}</string>
  <key>ProgramArguments</key>
  <array>
{program_arguments}
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>StandardOutPath</key>
  <string>{stdout_log}</string>
  <key>StandardErrorPath</key>
  <string>{stderr_log}</string>
</dict>
</plist>
"#,
        label = SERVICE_LABEL,
        program_arguments = program_arguments_xml,
        stdout_log = xml_escape(&stdout_log.to_string_lossy()),
        stderr_log = xml_escape(&stderr_log.to_string_lossy()),
    )
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
