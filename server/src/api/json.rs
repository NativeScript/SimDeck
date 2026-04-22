use axum::Json;
use serde::Serialize;

pub fn json<T: Serialize>(value: T) -> Json<T> {
    Json(value)
}
