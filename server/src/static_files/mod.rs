use crate::config::Config;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_status::SetStatus;

pub fn service(config: &Config) -> ServeDir<SetStatus<ServeFile>> {
    let index = config.client_root.join("index.html");
    ServeDir::new(&config.client_root).not_found_service(ServeFile::new(index))
}
