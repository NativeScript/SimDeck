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
