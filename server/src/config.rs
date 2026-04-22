use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Config {
    pub advertise_host: String,
    pub bind_ip: IpAddr,
    pub http_port: u16,
    pub wt_port: u16,
    pub client_root: PathBuf,
}

impl Config {
    pub fn new(
        http_port: u16,
        client_root: PathBuf,
        bind_ip: IpAddr,
        advertise_host: Option<String>,
    ) -> Self {
        let wt_port = http_port.saturating_add(1);
        let advertise_host = advertise_host.unwrap_or_else(|| match bind_ip {
            IpAddr::V4(ip) if ip.is_unspecified() => Ipv4Addr::LOCALHOST.to_string(),
            IpAddr::V6(ip) if ip.is_unspecified() => Ipv4Addr::LOCALHOST.to_string(),
            _ => bind_ip.to_string(),
        });
        Self {
            advertise_host,
            bind_ip,
            http_port,
            wt_port,
            client_root,
        }
    }

    pub fn http_addr(&self) -> SocketAddr {
        SocketAddr::new(self.bind_ip, self.http_port)
    }

    pub fn wt_addr(&self) -> SocketAddr {
        SocketAddr::new(self.bind_ip, self.wt_port)
    }

    pub fn wt_endpoint_template(&self) -> String {
        format!(
            "https://{}:{}/wt/simulators/{{udid}}",
            self.advertise_host, self.wt_port
        )
    }

    pub fn certificate_subject_alt_names(&self) -> Vec<String> {
        let mut names = vec![
            "localhost".to_string(),
            "127.0.0.1".to_string(),
            "::1".to_string(),
        ];

        let bind_ip = self.bind_ip.to_string();
        if !self.bind_ip.is_unspecified() && !names.contains(&bind_ip) {
            names.push(bind_ip);
        }
        if !names.contains(&self.advertise_host) {
            names.push(self.advertise_host.clone());
        }

        names
    }
}
