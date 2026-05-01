use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Config {
    pub access_token: String,
    pub advertise_host: String,
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
        Self {
            access_token: access_token.unwrap_or_else(crate::auth::generate_access_token),
            advertise_host,
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

#[cfg(test)]
mod tests {
    use super::Config;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    use std::path::PathBuf;

    fn config(bind_ip: IpAddr, advertise_host: Option<String>) -> Config {
        Config::new(
            4310,
            PathBuf::from("client/dist"),
            bind_ip,
            advertise_host,
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
}
