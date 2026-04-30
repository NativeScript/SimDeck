use crate::auth;
use axum::body::Body;
use axum::http::{header, HeaderName, HeaderValue, Method, Response, StatusCode, Uri};
use std::path::{Component, Path, PathBuf};

pub async fn serve_static(
    root: PathBuf,
    method: Method,
    uri: Uri,
    access_token: Option<String>,
) -> Result<Response<Body>, StatusCode> {
    if method != Method::GET && method != Method::HEAD {
        return Err(StatusCode::METHOD_NOT_ALLOWED);
    }

    let path = uri.path();
    let Some(relative_path) = safe_relative_path(path) else {
        return Err(StatusCode::BAD_REQUEST);
    };

    let requested_path = root.join(&relative_path);
    let file_path = match tokio::fs::metadata(&requested_path).await {
        Ok(metadata) if metadata.is_file() => requested_path,
        _ => root.join("index.html"),
    };

    let bytes = tokio::fs::read(&file_path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let content_type = content_type_for_path(&file_path);
    let body = if method == Method::HEAD {
        Body::empty()
    } else {
        Body::from(bytes)
    };

    let mut response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(
            header::CACHE_CONTROL,
            "no-store, no-cache, must-revalidate, max-age=0",
        )
        .header(header::PRAGMA, "no-cache")
        .header(header::EXPIRES, "0")
        .body(body)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    response.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        "*".parse().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    );
    if content_type.starts_with("text/html") {
        response.headers_mut().insert(
            HeaderName::from_static("clear-site-data"),
            "\"cache\""
                .parse()
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
        );
    }
    if let Some(access_token) = access_token {
        let cookie = HeaderValue::from_str(&auth::access_cookie_value(&access_token))
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        response.headers_mut().insert(header::SET_COOKIE, cookie);
    }
    Ok(response)
}

fn safe_relative_path(path: &str) -> Option<PathBuf> {
    let trimmed = path.trim_start_matches('/');
    if trimmed.is_empty() {
        return Some(PathBuf::from("index.html"));
    }

    let mut relative = PathBuf::new();
    for component in Path::new(trimmed).components() {
        match component {
            Component::Normal(part) => relative.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(relative)
}

fn content_type_for_path(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("css") => "text/css; charset=utf-8",
        Some("html") => "text/html; charset=utf-8",
        Some("ico") => "image/x-icon",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("map") => "application/json; charset=utf-8",
        Some("png") => "image/png",
        Some("svg") => "image/svg+xml",
        Some("wasm") => "application/wasm",
        Some("webp") => "image/webp",
        _ => "application/octet-stream",
    }
}
