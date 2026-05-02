use tracing_subscriber::EnvFilter;

pub fn init() {
    let filter = EnvFilter::try_from_env("SIMDECK_LOG")
        .unwrap_or_else(|_| EnvFilter::new("error,webrtc=error,webrtc_ice=error,webrtc_mdns=off"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}
