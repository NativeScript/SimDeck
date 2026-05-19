use crate::{auth, default_client_root, ServiceOptions};
use anyhow::{anyhow, bail, Context};
use std::fs;
use std::net::{IpAddr, Ipv4Addr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

const SERVICE_LABEL: &str = "org.nativescript.simdeck";
const LEGACY_SERVICE_LABELS: &[&str] = &["dev.nativescript.simdeck"];
const DEFAULT_SERVICE_DISCOVERY_MAX_PORT: u16 = 4320;

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
    pub reused: bool,
}

pub fn enable(mut options: ServiceOptions) -> anyhow::Result<()> {
    preserve_or_create_credentials(&mut options);
    let result = install(options)?;
    print_install_result(&result)
}

pub fn restart(mut options: ServiceOptions) -> anyhow::Result<()> {
    preserve_or_create_credentials(&mut options);
    let result = install(options)?;
    print_install_result(&result)
}

pub fn reset(mut options: ServiceOptions) -> anyhow::Result<()> {
    reset_credentials(&mut options);
    let result = install(options)?;
    print_install_result(&result)
}

pub fn pair(mut options: ServiceOptions) -> anyhow::Result<ServiceInstallResult> {
    preserve_or_create_credentials(&mut options);
    if let Some(result) = reuse_running_service_if_matching(&options)? {
        return Ok(result);
    }
    install(options)
}

fn preserve_or_create_credentials(options: &mut ServiceOptions) {
    let existing_credentials = installed_credentials().unwrap_or(None);
    apply_credentials(
        options,
        existing_credentials.as_ref(),
        CredentialPolicy::Preserve,
    );
}

fn reset_credentials(options: &mut ServiceOptions) {
    apply_credentials(options, None, CredentialPolicy::Reset);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CredentialPolicy {
    Preserve,
    Reset,
}

fn apply_credentials(
    options: &mut ServiceOptions,
    existing_credentials: Option<&ServiceCredentials>,
    policy: CredentialPolicy,
) {
    if options.access_token.is_none() {
        options.access_token = (policy == CredentialPolicy::Preserve)
            .then(|| existing_credentials.and_then(|credentials| credentials.access_token.clone()))
            .flatten()
            .or_else(|| Some(auth::generate_access_token()));
    }
    if options.pairing_code.is_none() {
        options.pairing_code = (policy == CredentialPolicy::Preserve)
            .then(|| existing_credentials.and_then(|credentials| credentials.pairing_code.clone()))
            .flatten()
            .or_else(|| Some(auth::generate_pairing_code()));
    }
}

pub fn installed_port() -> anyhow::Result<Option<u16>> {
    Ok(installed_argument_value("--port")?.and_then(|value| value.parse::<u16>().ok()))
}

fn install(mut options: ServiceOptions) -> anyhow::Result<ServiceInstallResult> {
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

    let domain = launchctl_domain()?;
    unload_existing_services(&domain)?;
    remove_legacy_service_plists()?;

    options.port = choose_service_port_for_bind(options.port, options.bind)?;
    let plist = plist_contents(
        &executable,
        &client_root,
        &stdout_log,
        &stderr_log,
        &options,
    );

    fs::write(&plist_path, plist).with_context(|| format!("write {}", plist_path.display()))?;

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
        reused: false,
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
            "port": result.port,
        }))?
    );
    Ok(())
}

pub fn disable() -> anyhow::Result<()> {
    let plist_path = plist_path()?;
    let domain = launchctl_domain()?;

    unload_existing_services(&domain)?;
    for path in service_plist_paths()? {
        if path.exists() {
            fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
        }
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
    plist_path_for_label(SERVICE_LABEL)
}

fn plist_path_for_label(label: &str) -> anyhow::Result<PathBuf> {
    Ok(home_dir()?
        .join("Library/LaunchAgents")
        .join(format!("{label}.plist")))
}

fn service_labels() -> Vec<&'static str> {
    std::iter::once(SERVICE_LABEL)
        .chain(LEGACY_SERVICE_LABELS.iter().copied())
        .collect()
}

fn service_plist_paths() -> anyhow::Result<Vec<PathBuf>> {
    service_labels()
        .into_iter()
        .map(plist_path_for_label)
        .collect()
}

fn remove_legacy_service_plists() -> anyhow::Result<()> {
    for label in LEGACY_SERVICE_LABELS {
        let path = plist_path_for_label(label)?;
        if path.exists() {
            fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
        }
    }
    Ok(())
}

fn log_dir() -> anyhow::Result<PathBuf> {
    Ok(home_dir()?.join("Library/Logs"))
}

fn reuse_running_service_if_matching(
    options: &ServiceOptions,
) -> anyhow::Result<Option<ServiceInstallResult>> {
    let domain = launchctl_domain()?;
    if launchagent_pid(&domain, SERVICE_LABEL).is_none() {
        return Ok(None);
    }
    let Some(arguments) = installed_arguments_for_label(SERVICE_LABEL)? else {
        return Ok(None);
    };
    let client_root = match options.client_root.as_ref() {
        Some(path) => path.clone(),
        None => default_client_root()?,
    };
    if !service_options_match_arguments(&arguments, options, &client_root) {
        return Ok(None);
    }
    let plist_path = plist_path()?;
    let log_dir = log_dir()?;
    Ok(Some(ServiceInstallResult {
        service: SERVICE_LABEL.to_owned(),
        plist_path,
        stdout_log: log_dir.join("simdeck.log"),
        stderr_log: log_dir.join("simdeck.err.log"),
        port: options.port,
        advertise_host: options.advertise_host.clone(),
        access_token: options.access_token.clone(),
        pairing_code: options.pairing_code.clone(),
        reused: true,
    }))
}

fn service_options_match_arguments(
    arguments: &[String],
    options: &ServiceOptions,
    client_root: &Path,
) -> bool {
    let port = options.port.to_string();
    let bind = options.bind.to_string();
    let client_root = client_root.to_string_lossy();
    let local_stream_fps = options.local_stream_fps.map(|value| value.to_string());
    argument_value(arguments, "--port").as_deref() == Some(port.as_str())
        && argument_value(arguments, "--bind").as_deref() == Some(bind.as_str())
        && argument_value(arguments, "--client-root").as_deref() == Some(client_root.as_ref())
        && argument_value(arguments, "--video-codec").as_deref()
            == Some(options.video_codec.as_env_value())
        && argument_value(arguments, "--server-kind").as_deref() == Some("launch-agent")
        && optional_argument_matches(
            arguments,
            "--advertise-host",
            options.advertise_host.as_deref(),
        )
        && optional_argument_matches(
            arguments,
            "--stream-quality",
            options.stream_quality_profile.as_deref(),
        )
        && optional_argument_matches(arguments, "--local-stream-fps", local_stream_fps.as_deref())
        && optional_argument_matches(arguments, "--access-token", options.access_token.as_deref())
        && optional_argument_matches(arguments, "--pairing-code", options.pairing_code.as_deref())
        && flag_matches(arguments, "--low-latency", options.low_latency)
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
    for plist_path in service_plist_paths()? {
        if let Some(arguments) = installed_arguments_from_plist(&plist_path)? {
            return Ok(Some(arguments));
        }
    }
    Ok(None)
}

fn installed_arguments_for_label(label: &str) -> anyhow::Result<Option<Vec<String>>> {
    installed_arguments_from_plist(&plist_path_for_label(label)?)
}

fn installed_arguments_from_plist(plist_path: &Path) -> anyhow::Result<Option<Vec<String>>> {
    if !plist_path.exists() {
        return Ok(None);
    }
    let plist = plist::Value::from_file(plist_path)
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

fn optional_argument_matches(arguments: &[String], name: &str, expected: Option<&str>) -> bool {
    argument_value(arguments, name).as_deref() == expected
}

fn flag_matches(arguments: &[String], name: &str, expected: bool) -> bool {
    arguments.iter().any(|argument| argument == name) == expected
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

fn unload_existing_services(domain: &str) -> anyhow::Result<()> {
    for label in service_labels() {
        let plist_path = plist_path_for_label(label)?;
        let old_pid = launchagent_pid(domain, label);
        let service_target = format!("{domain}/{label}");
        let _ = Command::new("launchctl")
            .args(["bootout", &service_target])
            .output();
        if plist_path.exists() {
            let _ = Command::new("launchctl")
                .args(["bootout", domain, plist_path.to_string_lossy().as_ref()])
                .output();
        }
        if let Some(pid) = old_pid {
            terminate_process_group(pid, Duration::from_secs(5));
        }
    }
    Ok(())
}

fn launchagent_pid(domain: &str, label: &str) -> Option<u32> {
    let target = format!("{domain}/{label}");
    let output = Command::new("launchctl")
        .args(["print", &target])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.lines().find_map(parse_launchctl_pid)
}

fn parse_launchctl_pid(line: &str) -> Option<u32> {
    line.trim().strip_prefix("pid = ")?.trim().parse().ok()
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
    let _ = Command::new("kill")
        .args([format!("-{signal}"), pid.to_string()])
        .output();
}

fn signal_process_group(pgid: u32, signal: &str) {
    let _ = Command::new("kill")
        .arg(format!("-{signal}"))
        .arg("--")
        .arg(format!("-{pgid}"))
        .output();
}

fn wait_for_process_exit(pid: u32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !process_exists(pid) {
            return true;
        }
        thread::sleep(Duration::from_millis(50));
    }
    !process_exists(pid)
}

fn process_exists(pid: u32) -> bool {
    Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn choose_service_port_for_bind(preferred: u16, bind: IpAddr) -> anyhow::Result<u16> {
    let start = preferred.max(1024);
    let end = if start <= DEFAULT_SERVICE_DISCOVERY_MAX_PORT {
        DEFAULT_SERVICE_DISCOVERY_MAX_PORT
    } else {
        start.saturating_add(10)
    };
    for port in start..=end {
        if port_available(bind, port) {
            return Ok(port);
        }
    }
    bail!("No available SimDeck LaunchAgent port between {start} and {end}");
}

fn port_available(bind: IpAddr, port: u16) -> bool {
    if bind.is_unspecified() && TcpListener::bind((Ipv4Addr::LOCALHOST, port)).is_err() {
        return false;
    }
    TcpListener::bind((bind, port)).is_ok()
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
        "--server-kind".to_string(),
        "launch-agent".to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn service_options_for_test() -> ServiceOptions {
        ServiceOptions {
            port: 4310,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            advertise_host: None,
            client_root: None,
            video_codec: crate::VideoCodecMode::Auto,
            low_latency: false,
            stream_quality_profile: None,
            local_stream_fps: None,
            access_token: None,
            pairing_code: None,
        }
    }

    fn service_arguments_for_test(options: &ServiceOptions) -> Vec<String> {
        let mut arguments = vec![
            "/tmp/simdeck".to_owned(),
            "serve".to_owned(),
            "--port".to_owned(),
            options.port.to_string(),
            "--bind".to_owned(),
            options.bind.to_string(),
            "--client-root".to_owned(),
            "/tmp/client".to_owned(),
            "--video-codec".to_owned(),
            options.video_codec.as_env_value().to_owned(),
            "--server-kind".to_owned(),
            "launch-agent".to_owned(),
        ];
        if options.low_latency {
            arguments.push("--low-latency".to_owned());
        }
        if let Some(stream_quality_profile) = options.stream_quality_profile.as_ref() {
            arguments.push("--stream-quality".to_owned());
            arguments.push(stream_quality_profile.clone());
        }
        if let Some(local_stream_fps) = options.local_stream_fps {
            arguments.push("--local-stream-fps".to_owned());
            arguments.push(local_stream_fps.to_string());
        }
        if let Some(advertise_host) = options.advertise_host.as_ref() {
            arguments.push("--advertise-host".to_owned());
            arguments.push(advertise_host.clone());
        }
        if let Some(access_token) = options.access_token.as_ref() {
            arguments.push("--access-token".to_owned());
            arguments.push(access_token.clone());
        }
        if let Some(pairing_code) = options.pairing_code.as_ref() {
            arguments.push("--pairing-code".to_owned());
            arguments.push(pairing_code.clone());
        }
        arguments
    }

    #[test]
    fn service_label_matches_bundle_identifier() {
        assert_eq!(SERVICE_LABEL, "org.nativescript.simdeck");
        assert!(LEGACY_SERVICE_LABELS.contains(&"dev.nativescript.simdeck"));
    }

    #[test]
    fn parses_launchctl_pid_line() {
        assert_eq!(parse_launchctl_pid("\tpid = 24969"), Some(24969));
        assert_eq!(
            parse_launchctl_pid("\tlast terminating signal = Terminated: 15"),
            None
        );
    }

    #[test]
    fn service_port_selection_skips_occupied_port() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind test port");
        let occupied = listener.local_addr().expect("local addr").port();
        let selected = choose_service_port_for_bind(occupied, IpAddr::V4(Ipv4Addr::LOCALHOST))
            .expect("choose service port");

        assert!(selected > occupied);
        assert!(selected <= occupied.saturating_add(10));
    }

    #[test]
    fn default_service_port_search_matches_ios_discovery_window() {
        assert_eq!(DEFAULT_SERVICE_DISCOVERY_MAX_PORT, 4320);
    }

    #[test]
    fn preserve_credentials_reuses_installed_values() {
        let mut options = service_options_for_test();
        let installed = ServiceCredentials {
            access_token: Some("installed-token".to_owned()),
            pairing_code: Some("installed-code".to_owned()),
        };

        apply_credentials(&mut options, Some(&installed), CredentialPolicy::Preserve);

        assert_eq!(options.access_token.as_deref(), Some("installed-token"));
        assert_eq!(options.pairing_code.as_deref(), Some("installed-code"));
    }

    #[test]
    fn preserve_credentials_generates_missing_values() {
        let mut options = service_options_for_test();

        apply_credentials(&mut options, None, CredentialPolicy::Preserve);

        assert!(options
            .access_token
            .as_ref()
            .is_some_and(|token| token.len() == 64));
        assert!(options
            .pairing_code
            .as_ref()
            .is_some_and(|code| code.len() == 6 && code.chars().all(|ch| ch.is_ascii_digit())));
    }

    #[test]
    fn reset_credentials_ignores_installed_values() {
        let mut options = service_options_for_test();
        let installed = ServiceCredentials {
            access_token: Some("installed-token".to_owned()),
            pairing_code: Some("installed-code".to_owned()),
        };

        apply_credentials(&mut options, Some(&installed), CredentialPolicy::Reset);

        assert_ne!(options.access_token.as_deref(), Some("installed-token"));
        assert_ne!(options.pairing_code.as_deref(), Some("installed-code"));
        assert!(options
            .access_token
            .as_ref()
            .is_some_and(|token| token.len() == 64));
        assert!(options
            .pairing_code
            .as_ref()
            .is_some_and(|code| code.len() == 6 && code.chars().all(|ch| ch.is_ascii_digit())));
    }

    #[test]
    fn explicit_credentials_win_over_preserve_or_reset() {
        for policy in [CredentialPolicy::Preserve, CredentialPolicy::Reset] {
            let mut options = service_options_for_test();
            options.access_token = Some("explicit-token".to_owned());
            options.pairing_code = Some("654321".to_owned());
            let installed = ServiceCredentials {
                access_token: Some("installed-token".to_owned()),
                pairing_code: Some("installed-code".to_owned()),
            };

            apply_credentials(&mut options, Some(&installed), policy);

            assert_eq!(options.access_token.as_deref(), Some("explicit-token"));
            assert_eq!(options.pairing_code.as_deref(), Some("654321"));
        }
    }

    #[test]
    fn service_options_match_installed_arguments() {
        let mut options = service_options_for_test();
        options.advertise_host = Some("192.168.1.10".to_owned());
        options.stream_quality_profile = Some("low".to_owned());
        options.local_stream_fps = Some(30);
        options.access_token = Some("token".to_owned());
        options.pairing_code = Some("123456".to_owned());
        options.low_latency = true;
        let arguments = service_arguments_for_test(&options);

        assert!(service_options_match_arguments(
            &arguments,
            &options,
            Path::new("/tmp/client")
        ));
    }

    #[test]
    fn service_options_reject_changed_arguments() {
        let mut options = service_options_for_test();
        options.access_token = Some("token".to_owned());
        options.pairing_code = Some("123456".to_owned());
        let arguments = service_arguments_for_test(&options);

        options.port = 4311;

        assert!(!service_options_match_arguments(
            &arguments,
            &options,
            Path::new("/tmp/client")
        ));
    }
}
