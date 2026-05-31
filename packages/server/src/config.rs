use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::ffi::CStr;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Config {
    pub access_token: String,
    pub advertise_host: String,
    pub host_id: String,
    pub host_name: String,
    pub server_kind: ServerKind,
    pub bind_ip: IpAddr,
    pub http_port: u16,
    pub pairing_code: Option<String>,
    pub client_root: PathBuf,
    pub video_codec: String,
    pub low_latency: bool,
}

impl Config {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        http_port: u16,
        client_root: PathBuf,
        bind_ip: IpAddr,
        advertise_host: Option<String>,
        server_kind: ServerKind,
        video_codec: String,
        low_latency: bool,
        access_token: Option<String>,
        pairing_code: Option<String>,
    ) -> Self {
        let advertise_host = advertise_host.unwrap_or_else(|| match bind_ip {
            IpAddr::V4(ip) if ip.is_unspecified() => Ipv4Addr::LOCALHOST.to_string(),
            IpAddr::V6(ip) if ip.is_unspecified() => Ipv4Addr::LOCALHOST.to_string(),
            _ => bind_ip.to_string(),
        });
        let host_name = local_host_name();
        let host_id = host_identity(&host_name);
        Self {
            access_token: access_token.unwrap_or_else(crate::auth::generate_access_token),
            advertise_host,
            host_id,
            host_name,
            server_kind,
            bind_ip,
            http_port,
            pairing_code,
            client_root,
            video_codec,
            low_latency,
        }
    }

    pub fn http_addr(&self) -> SocketAddr {
        SocketAddr::new(self.bind_ip, self.http_port)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServerKind {
    LaunchAgent,
    Workspace,
    Foreground,
    Standalone,
}

impl ServerKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LaunchAgent => "launchAgent",
            Self::Workspace => "workspace",
            Self::Foreground => "foreground",
            Self::Standalone => "standalone",
        }
    }
}

fn local_host_name() -> String {
    platform_host_name()
        .and_then(|value| {
            value
                .trim()
                .trim_end_matches(".local")
                .trim_end_matches('.')
                .split('.')
                .next()
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| "localhost".to_owned())
}

#[cfg(unix)]
fn platform_host_name() -> Option<String> {
    let mut buffer = [0 as libc::c_char; 256];
    unsafe {
        if libc::gethostname(buffer.as_mut_ptr(), buffer.len()) != 0 {
            None
        } else {
            buffer[buffer.len() - 1] = 0;
            CStr::from_ptr(buffer.as_ptr()).to_str().ok()
        }
    }
    .map(ToOwned::to_owned)
}

#[cfg(not(unix))]
fn platform_host_name() -> Option<String> {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
}

fn host_identity(host_name: &str) -> String {
    let source = machine_identity_source()
        .unwrap_or_else(|| format!("hostname:{}", normalized_host_name(host_name)));
    host_identity_from_source(&source)
}

fn normalized_host_name(host_name: &str) -> String {
    host_name.trim().to_ascii_lowercase()
}

fn host_identity_from_source(source: &str) -> String {
    let digest = hex::encode(Sha256::digest(source.as_bytes()));
    digest[..16].to_owned()
}

#[cfg(target_os = "macos")]
fn machine_identity_source() -> Option<String> {
    io_platform_uuid().map(|uuid| format!("ioplatformuuid:{}", uuid.to_ascii_lowercase()))
}

#[cfg(not(target_os = "macos"))]
fn machine_identity_source() -> Option<String> {
    None
}

#[cfg(target_os = "macos")]
fn io_platform_uuid() -> Option<String> {
    let output = std::process::Command::new("/usr/sbin/ioreg")
        .args(["-rd1", "-c", "IOPlatformExpertDevice"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_io_platform_uuid(&String::from_utf8_lossy(&output.stdout))
}

fn parse_io_platform_uuid(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let (_, value) = line.split_once("\"IOPlatformUUID\"")?;
        let (_, value) = value.split_once('=')?;
        value
            .trim()
            .trim_matches('"')
            .split_whitespace()
            .next()
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    })
}

#[cfg(test)]
mod tests {
    use super::{host_identity_from_source, parse_io_platform_uuid, Config, ServerKind};
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    use std::path::PathBuf;

    fn config(bind_ip: IpAddr, advertise_host: Option<String>) -> Config {
        Config::new(
            4310,
            PathBuf::from("packages/client/dist"),
            bind_ip,
            advertise_host,
            ServerKind::Standalone,
            "auto".to_owned(),
            false,
            Some("token".to_owned()),
            None,
        )
    }

    #[test]
    fn unspecified_bind_defaults_advertise_host_to_loopback() {
        assert_eq!(
            config(IpAddr::V4(Ipv4Addr::UNSPECIFIED), None).advertise_host,
            "127.0.0.1"
        );
        assert_eq!(
            config(IpAddr::V6(Ipv6Addr::UNSPECIFIED), None).advertise_host,
            "127.0.0.1"
        );
    }

    #[test]
    fn explicit_advertise_host_overrides_bind_address() {
        assert_eq!(
            config(
                IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)),
                Some("192.168.1.50".to_owned())
            )
            .advertise_host,
            "192.168.1.50"
        );
    }

    #[test]
    fn concrete_bind_address_is_used_as_advertise_host() {
        assert_eq!(
            config(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 23)), None).advertise_host,
            "192.168.1.23"
        );
    }

    #[test]
    fn http_addr_uses_configured_bind_ip_and_port() {
        let config = config(IpAddr::V4(Ipv4Addr::LOCALHOST), None);

        assert_eq!(config.http_addr().to_string(), "127.0.0.1:4310");
    }

    #[test]
    fn server_kind_serializes_for_health_contract() {
        assert_eq!(ServerKind::LaunchAgent.as_str(), "launchAgent");
        assert_eq!(ServerKind::Workspace.as_str(), "workspace");
        assert_eq!(ServerKind::Foreground.as_str(), "foreground");
        assert_eq!(ServerKind::Standalone.as_str(), "standalone");
    }

    #[test]
    fn host_identity_changes_with_machine_source() {
        assert_ne!(
            host_identity_from_source("ioplatformuuid:machine-a"),
            host_identity_from_source("ioplatformuuid:machine-b")
        );
    }

    #[test]
    fn parses_io_platform_uuid_from_ioreg_output() {
        let output = r#"
        +-o IOPlatformExpertDevice  <class IOPlatformExpertDevice, id 0x100000100, registered, matched, active, busy 0 (0 ms), retain 41>
            "IOPlatformUUID" = "01234567-89AB-CDEF-0123-456789ABCDEF"
        "#;

        assert_eq!(
            parse_io_platform_uuid(output).as_deref(),
            Some("01234567-89AB-CDEF-0123-456789ABCDEF")
        );
    }
}
