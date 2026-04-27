use crate::config::Config;
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Read;

pub const ACCESS_TOKEN_HEADER: &str = "x-simdeck-token";
pub const ACCESS_TOKEN_COOKIE: &str = "simdeck_token";
pub const ACCESS_TOKEN_QUERY: &str = "simdeckToken";

pub fn generate_access_token() -> String {
    let mut bytes = [0u8; 32];
    if File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .is_err()
    {
        let fallback = format!(
            "{}:{}:{:?}",
            std::process::id(),
            chrono_free_now_nanos(),
            std::thread::current().id()
        );
        return hex::encode(Sha256::digest(fallback.as_bytes()));
    }
    hex::encode(bytes)
}

pub fn access_cookie_value(token: &str) -> String {
    format!("{ACCESS_TOKEN_COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict")
}

pub fn tokenized_webtransport_template(config: &Config) -> String {
    format!(
        "{}?{}={}",
        config.wt_endpoint_template(),
        ACCESS_TOKEN_QUERY,
        config.access_token
    )
}

pub fn api_request_authorized(
    config: &Config,
    method: &Method,
    headers: &HeaderMap,
    peer_is_loopback: bool,
) -> bool {
    if bearer_or_header_token(headers).is_some_and(|token| token == config.access_token) {
        return true;
    }

    if cookie_token(headers).is_some_and(|token| token == config.access_token) {
        return if method == Method::GET || method == Method::HEAD {
            origin_allowed_or_absent(config, headers)
        } else {
            origin_allowed(config, headers)
        };
    }

    if peer_is_loopback {
        return if method == Method::GET || method == Method::HEAD {
            true
        } else {
            origin_allowed(config, headers)
        };
    }

    false
}

pub fn webtransport_request_authorized(config: &Config, path: &str, origin: Option<&str>) -> bool {
    query_token(path).is_some_and(|token| token == config.access_token)
        && origin.is_some_and(|origin| origin_is_allowed(config, origin))
}

pub fn strip_query(path: &str) -> &str {
    path.split_once('?').map(|(path, _)| path).unwrap_or(path)
}

pub fn preflight_response(config: &Config, headers: &HeaderMap) -> Response {
    let mut response = StatusCode::NO_CONTENT.into_response();
    append_cors_headers(config, headers, response.headers_mut());
    response
}

pub fn unauthorized_response(config: &Config, headers: &HeaderMap) -> Response {
    let mut response = (
        StatusCode::UNAUTHORIZED,
        Json(json!({
            "error": "SimDeck API access token is required.",
        })),
    )
        .into_response();
    append_cors_headers(config, headers, response.headers_mut());
    response
}

pub fn append_cors_headers(
    config: &Config,
    request_headers: &HeaderMap,
    response_headers: &mut HeaderMap,
) {
    let Some(origin) = request_headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    else {
        return;
    };

    if !origin_is_cors_allowed(config, origin) {
        return;
    }

    if let Ok(value) = HeaderValue::from_str(origin) {
        response_headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, value);
    }
    response_headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, OPTIONS"),
    );
    response_headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("content-type, authorization, x-simdeck-token"),
    );
    response_headers.insert(
        header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
        HeaderValue::from_static("true"),
    );
    response_headers.insert(header::VARY, HeaderValue::from_static("Origin"));
}

pub fn append_access_cookie(response_headers: &mut HeaderMap, token: &str) {
    if let Ok(value) = HeaderValue::from_str(&access_cookie_value(token)) {
        response_headers.insert(header::SET_COOKIE, value);
    }
}

fn bearer_or_header_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(ACCESS_TOKEN_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            headers
                .get(header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.trim().strip_prefix("Bearer "))
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
}

fn cookie_token(headers: &HeaderMap) -> Option<&str> {
    let cookie = headers.get(header::COOKIE)?.to_str().ok()?;
    cookie.split(';').find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        (name == ACCESS_TOKEN_COOKIE).then_some(value.trim())
    })
}

fn query_token(path: &str) -> Option<&str> {
    let query = path.split_once('?')?.1;
    query.split('&').find_map(|part| {
        let (name, value) = part.split_once('=')?;
        (name == ACCESS_TOKEN_QUERY).then_some(value)
    })
}

fn origin_allowed_or_absent(config: &Config, headers: &HeaderMap) -> bool {
    headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .map(|origin| origin_is_allowed(config, origin))
        .unwrap_or(true)
}

fn origin_allowed(config: &Config, headers: &HeaderMap) -> bool {
    headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|origin| origin_is_allowed(config, origin))
}

fn origin_is_allowed(config: &Config, origin: &str) -> bool {
    let allowed = [
        format!("http://{}:{}", config.advertise_host, config.http_port),
        format!("http://{}:{}", config.bind_ip, config.http_port),
        format!("http://localhost:{}", config.http_port),
        format!("http://127.0.0.1:{}", config.http_port),
        format!("http://[::1]:{}", config.http_port),
    ];
    allowed.iter().any(|value| value == origin)
}

fn origin_is_cors_allowed(config: &Config, origin: &str) -> bool {
    origin == "null" || origin_is_allowed(config, origin)
}

fn chrono_free_now_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

#[cfg(test)]
mod tests {
    use super::{api_request_authorized, tokenized_webtransport_template, ACCESS_TOKEN_HEADER};
    use crate::config::Config;
    use axum::http::{header, HeaderMap, HeaderValue, Method};
    use std::net::{IpAddr, Ipv4Addr};
    use std::path::PathBuf;

    fn config() -> Config {
        Config::new(
            4310,
            PathBuf::from("client/dist"),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            None,
            "hevc".to_owned(),
            Some("secret-token".to_owned()),
        )
    }

    #[test]
    fn accepts_explicit_token_header_without_cookie() {
        let config = config();
        let mut headers = HeaderMap::new();
        headers.insert(
            ACCESS_TOKEN_HEADER,
            HeaderValue::from_static("secret-token"),
        );

        assert!(api_request_authorized(
            &config,
            &Method::POST,
            &headers,
            false
        ));
    }

    #[test]
    fn accepts_same_origin_cookie_for_browser_ui() {
        let config = config();
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("simdeck_token=secret-token"),
        );
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("http://127.0.0.1:4310"),
        );

        assert!(api_request_authorized(
            &config,
            &Method::POST,
            &headers,
            false
        ));
    }

    #[test]
    fn rejects_cross_origin_cookie_without_header_token() {
        let config = config();
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("simdeck_token=secret-token"),
        );
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://example.invalid"),
        );

        assert!(!api_request_authorized(
            &config,
            &Method::POST,
            &headers,
            false
        ));
    }

    #[test]
    fn health_template_carries_webtransport_token() {
        let config = config();
        assert_eq!(
            tokenized_webtransport_template(&config),
            "https://127.0.0.1:4311/wt/simulators/{udid}?simdeckToken=secret-token"
        );
    }

    #[test]
    fn accepts_loopback_same_origin_without_existing_token() {
        let config = config();
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("http://127.0.0.1:4310"),
        );

        assert!(api_request_authorized(
            &config,
            &Method::POST,
            &headers,
            true
        ));
    }

    #[test]
    fn rejects_loopback_cross_origin_without_existing_token() {
        let config = config();
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://example.invalid"),
        );

        assert!(!api_request_authorized(
            &config,
            &Method::POST,
            &headers,
            true
        ));
    }

    #[test]
    fn accepts_loopback_null_origin_read_without_existing_token() {
        let config = config();
        let mut headers = HeaderMap::new();
        headers.insert(header::ORIGIN, HeaderValue::from_static("null"));

        assert!(api_request_authorized(
            &config,
            &Method::GET,
            &headers,
            true
        ));
    }

    #[test]
    fn rejects_loopback_null_origin_write_without_existing_token() {
        let config = config();
        let mut headers = HeaderMap::new();
        headers.insert(header::ORIGIN, HeaderValue::from_static("null"));

        assert!(!api_request_authorized(
            &config,
            &Method::POST,
            &headers,
            true
        ));
    }
}
