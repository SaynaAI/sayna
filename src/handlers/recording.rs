use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use object_store::{Error as ObjectStoreError, ObjectStore, path::Path as ObjectPath};
use serde_json::json;
use std::sync::Arc;
use tracing::{debug, error, info};

use crate::state::AppState;

const CONTENT_TYPE: &str = "audio/ogg";

fn is_valid_stream_id(stream_id: &str) -> bool {
    !stream_id.is_empty() && !stream_id.contains("..") && !stream_id.contains('/')
}

fn build_recording_object_key(prefix: &str, stream_id: &str) -> String {
    let trimmed = prefix.trim_end_matches('/');
    if trimmed.is_empty() {
        format!("{stream_id}/audio.ogg")
    } else {
        format!("{trimmed}/{stream_id}/audio.ogg")
    }
}

/// Download recording by stream ID from configured object storage
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/recording/{stream_id}",
        params(
            ("stream_id" = String, Path, description = "Recording stream identifier", example = "550e8400-e29b-41d4-a716-446655440000")
        ),
        responses(
            (status = 200, description = "Recording retrieved successfully", content_type = "audio/ogg",
                headers(
                    ("Content-Disposition" = String, description = "Suggested filename for download"),
                    ("Content-Length" = u64, description = "Size of the recording in bytes")
                )
            ),
            (status = 400, description = "Invalid stream_id format"),
            (status = 404, description = "Recording not found"),
            (status = 503, description = "Recording storage not configured or unavailable")
        ),
        security(
            ("auth" = [])
        ),
        tag = "recordings"
    )
)]
pub async fn download_recording(
    State(state): State<Arc<AppState>>,
    Path(stream_id): Path<String>,
) -> Response {
    info!("Recording download requested - stream_id={}", stream_id);

    if !is_valid_stream_id(&stream_id) {
        error!(
            "Invalid stream_id format for recording download: {}",
            stream_id
        );
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Invalid stream_id format"})),
        )
            .into_response();
    }

    let store = match &state.object_store {
        Some(store) => store.clone(),
        None => {
            error!("Recording download attempted but storage is not configured");
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"error": "Recording storage not configured"})),
            )
                .into_response();
        }
    };

    let bucket = match &state.recording_bucket {
        Some(bucket) => bucket,
        None => {
            error!("Recording bucket not configured");
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"error": "Recording storage not configured"})),
            )
                .into_response();
        }
    };

    let prefix = state
        .config
        .recording
        .as_ref()
        .map(|r| r.prefix.as_str())
        .unwrap_or_default();
    let object_key = build_recording_object_key(prefix, &stream_id);

    let object_path = match ObjectPath::parse(object_key.clone()) {
        Ok(path) => path,
        Err(e) => {
            error!("Invalid recording path for stream_id={}: {}", stream_id, e);
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "Invalid recording path"})),
            )
                .into_response();
        }
    };

    debug!(
        "Fetching recording from bucket={} with key={}",
        bucket, object_key
    );

    let get_result = match store.get(&object_path).await {
        Ok(result) => result,
        Err(ObjectStoreError::NotFound { path, .. }) => {
            info!(
                "Recording not found for stream_id={} key={} (path={})",
                stream_id, object_key, path
            );
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": format!("Recording not found: {}", stream_id)})),
            )
                .into_response();
        }
        Err(e) => {
            error!(
                "Failed to retrieve recording from storage for stream_id={}: {:?}",
                stream_id, e
            );
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"error": "Failed to retrieve recording from storage"})),
            )
                .into_response();
        }
    };

    let size = get_result.meta.size;

    let body = match get_result.bytes().await {
        Ok(bytes) => bytes,
        Err(e) => {
            error!(
                "Failed to read recording from storage for stream_id={}: {:?}",
                stream_id, e
            );
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"error": "Failed to read recording from storage"})),
            )
                .into_response();
        }
    };

    info!(
        "Recording download successful - stream_id={}, size={} bytes",
        stream_id, size
    );

    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(CONTENT_TYPE));
    if let Ok(len) = HeaderValue::from_str(&size.to_string()) {
        headers.insert(header::CONTENT_LENGTH, len);
    }
    if let Ok(disposition) =
        HeaderValue::from_str(&format!("attachment; filename=\"{}.ogg\"", stream_id))
    {
        headers.insert(header::CONTENT_DISPOSITION, disposition);
    }

    (StatusCode::OK, headers, body).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_key_with_prefix() {
        let key = build_recording_object_key("recordings", "abc123");
        assert_eq!(key, "recordings/abc123/audio.ogg");
    }

    #[test]
    fn test_build_key_without_prefix() {
        assert_eq!(build_recording_object_key("", "abc123"), "abc123/audio.ogg");
    }

    #[test]
    fn test_build_key_with_trailing_slash() {
        assert_eq!(
            build_recording_object_key("recordings/", "abc123"),
            "recordings/abc123/audio.ogg"
        );
    }

    #[test]
    fn test_invalid_stream_id_empty() {
        assert!(!is_valid_stream_id(""));
    }

    #[test]
    fn test_invalid_stream_id_path_traversal() {
        assert!(!is_valid_stream_id("../etc/passwd"));
        assert!(!is_valid_stream_id(".."));
    }

    #[test]
    fn test_invalid_stream_id_contains_slash() {
        assert!(!is_valid_stream_id("abc/123"));
    }

    #[test]
    fn test_valid_stream_id_uuid() {
        assert!(is_valid_stream_id("550e8400-e29b-41d4-a716-446655440000"));
    }

    #[test]
    fn test_valid_stream_id_custom() {
        assert!(is_valid_stream_id("call-123"));
    }
}
