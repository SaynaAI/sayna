//! # LiveKit Room Metadata Helpers
//!
//! This module provides utilities for managing room metadata in LiveKit rooms.
//! The primary use case is storing tenant identifiers (`auth_id`) in room metadata
//! as JSON, replacing the previous approach of prefixing room names.
//!
//! ## Metadata Schema
//!
//! Room metadata is stored as a JSON object with the following structure:
//!
//! ```json
//! {
//!     "auth_id": "tenant-identifier",
//!     // ... other keys preserved
//! }
//! ```
//!
//! ## Usage Example
//!
//! ```rust
//! use sayna::livekit::metadata::{get_auth_id, merge_auth_id};
//!
//! // Extract auth_id from existing metadata
//! let metadata = r#"{"auth_id": "tenant-123", "other": "data"}"#;
//! assert_eq!(get_auth_id(metadata), Some("tenant-123".to_string()));
//!
//! // Merge auth_id into empty metadata
//! let result = merge_auth_id("", "tenant-456").unwrap();
//! assert!(result.contains("tenant-456"));
//! ```

use serde_json::{Map, Value};

/// The key used to store the tenant identifier in room metadata.
pub const AUTH_ID_KEY: &str = "auth_id";

/// Errors that can occur during metadata operations.
#[derive(Debug, thiserror::Error)]
pub enum MetadataError {
    /// The metadata string contains invalid JSON.
    #[error("Invalid JSON in metadata: {0}")]
    InvalidJson(String),

    /// The metadata JSON is not an object (e.g., it's an array or primitive).
    #[error("Metadata must be a JSON object, got: {0}")]
    NotAnObject(String),

    /// Attempted to set auth_id when a different auth_id already exists.
    /// This prevents cross-tenant hijacking.
    #[error(
        "Metadata already contains different auth_id: existing={existing}, attempted={attempted}"
    )]
    AuthIdConflict { existing: String, attempted: String },
}

/// Parse raw metadata string into a JSON object map.
///
/// # Arguments
/// * `raw` - The raw metadata string from LiveKit room
///
/// # Returns
/// * `Ok(Map)` - Parsed JSON object (empty map for empty/whitespace input)
/// * `Err(MetadataError)` - If the JSON is invalid or not an object
///
/// # Examples
///
/// ```rust
/// use sayna::livekit::metadata::parse_metadata;
///
/// // Empty metadata returns empty map
/// let map = parse_metadata("").unwrap();
/// assert!(map.is_empty());
///
/// // Valid JSON object
/// let map = parse_metadata(r#"{"key": "value"}"#).unwrap();
/// assert_eq!(map.get("key").unwrap(), "value");
///
/// // Invalid JSON returns error
/// assert!(parse_metadata("not json").is_err());
/// ```
pub fn parse_metadata(raw: &str) -> Result<Map<String, Value>, MetadataError> {
    let trimmed = raw.trim();

    // Treat empty or whitespace-only metadata as an empty object
    if trimmed.is_empty() {
        return Ok(Map::new());
    }

    // Parse the JSON
    let value: Value =
        serde_json::from_str(trimmed).map_err(|e| MetadataError::InvalidJson(e.to_string()))?;

    // Ensure it's an object
    match value {
        Value::Object(map) => Ok(map),
        other => Err(MetadataError::NotAnObject(format!("{:?}", other))),
    }
}

/// Extract the `auth_id` value from room metadata.
///
/// # Arguments
/// * `raw` - The raw metadata string from LiveKit room
///
/// # Returns
/// * `Some(String)` - The auth_id value if present and valid
/// * `None` - If metadata is empty, invalid, or auth_id is missing/not a string
///
/// # Examples
///
/// ```rust
/// use sayna::livekit::metadata::get_auth_id;
///
/// // Extract auth_id from valid metadata
/// assert_eq!(
///     get_auth_id(r#"{"auth_id": "tenant-123"}"#),
///     Some("tenant-123".to_string())
/// );
///
/// // Returns None for empty metadata
/// assert_eq!(get_auth_id(""), None);
///
/// // Returns None if auth_id is missing
/// assert_eq!(get_auth_id(r#"{"other": "data"}"#), None);
///
/// // Returns None for invalid JSON (fails closed)
/// assert_eq!(get_auth_id("not json"), None);
/// ```
pub fn get_auth_id(raw: &str) -> Option<String> {
    let map = parse_metadata(raw).ok()?;
    map.get(AUTH_ID_KEY)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Merge an `auth_id` into existing room metadata.
///
/// This function safely merges an auth_id into existing metadata while:
/// - Preserving all other keys in the metadata
/// - Preventing cross-tenant hijacking by rejecting conflicting auth_ids
///
/// # Arguments
/// * `raw` - The raw metadata string from LiveKit room
/// * `auth_id` - The tenant identifier to merge
///
/// # Returns
/// * `Ok(String)` - The updated metadata JSON string
/// * `Err(MetadataError)` - If the metadata is invalid or contains a different auth_id
///
/// # Behavior
/// - If `auth_id` is already present and **equal**, returns original metadata unchanged
/// - If `auth_id` is already present and **different**, returns `AuthIdConflict` error
/// - If `auth_id` is missing, merges it into the existing object
/// - Empty metadata is treated as `{}`
///
/// # Examples
///
/// ```rust
/// use sayna::livekit::metadata::merge_auth_id;
///
/// // Merge into empty metadata
/// let result = merge_auth_id("", "tenant-123").unwrap();
/// assert!(result.contains("tenant-123"));
///
/// // Same auth_id returns original unchanged
/// let original = r#"{"auth_id":"tenant-123","other":"data"}"#;
/// let result = merge_auth_id(original, "tenant-123").unwrap();
/// // Note: JSON may be reformatted but semantically equivalent
///
/// // Different auth_id returns error
/// let result = merge_auth_id(r#"{"auth_id":"tenant-A"}"#, "tenant-B");
/// assert!(result.is_err());
/// ```
pub fn merge_auth_id(raw: &str, auth_id: &str) -> Result<String, MetadataError> {
    let mut map = parse_metadata(raw)?;

    // Check for existing auth_id
    if let Some(existing_value) = map.get(AUTH_ID_KEY)
        && let Some(existing_id) = existing_value.as_str()
    {
        if existing_id == auth_id {
            // Same auth_id - return original metadata unchanged
            // Re-serialize to normalize formatting
            return Ok(serde_json::to_string(&Value::Object(map))
                .expect("serializing valid map cannot fail"));
        } else {
            // Different auth_id - this is a conflict
            return Err(MetadataError::AuthIdConflict {
                existing: existing_id.to_string(),
                attempted: auth_id.to_string(),
            });
        }
    }
    // If existing value is not a string - treat as missing and overwrite

    // Insert or update auth_id
    map.insert(AUTH_ID_KEY.to_string(), Value::String(auth_id.to_string()));

    // Serialize back to JSON
    Ok(serde_json::to_string(&Value::Object(map)).expect("serializing valid map cannot fail"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ==================== parse_metadata tests ====================

    #[test]
    fn test_parse_metadata_empty_string() {
        let result = parse_metadata("").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_metadata_whitespace_only() {
        let result = parse_metadata("   \n\t  ").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_metadata_valid_object() {
        let result = parse_metadata(r#"{"key": "value", "num": 42}"#).unwrap();
        assert_eq!(result.get("key").unwrap().as_str().unwrap(), "value");
        assert_eq!(result.get("num").unwrap().as_i64().unwrap(), 42);
    }

    #[test]
    fn test_parse_metadata_nested_object() {
        let result = parse_metadata(r#"{"outer": {"inner": "value"}}"#).unwrap();
        let outer = result.get("outer").unwrap().as_object().unwrap();
        assert_eq!(outer.get("inner").unwrap().as_str().unwrap(), "value");
    }

    #[test]
    fn test_parse_metadata_invalid_json() {
        let result = parse_metadata("not valid json");
        assert!(matches!(result, Err(MetadataError::InvalidJson(_))));
    }

    #[test]
    fn test_parse_metadata_json_array() {
        let result = parse_metadata("[1, 2, 3]");
        assert!(matches!(result, Err(MetadataError::NotAnObject(_))));
    }

    #[test]
    fn test_parse_metadata_json_string() {
        let result = parse_metadata(r#""just a string""#);
        assert!(matches!(result, Err(MetadataError::NotAnObject(_))));
    }

    #[test]
    fn test_parse_metadata_json_number() {
        let result = parse_metadata("42");
        assert!(matches!(result, Err(MetadataError::NotAnObject(_))));
    }

    #[test]
    fn test_parse_metadata_json_null() {
        let result = parse_metadata("null");
        assert!(matches!(result, Err(MetadataError::NotAnObject(_))));
    }

    // ==================== get_auth_id tests ====================

    #[test]
    fn test_get_auth_id_present() {
        let result = get_auth_id(r#"{"auth_id": "tenant-123"}"#);
        assert_eq!(result, Some("tenant-123".to_string()));
    }

    #[test]
    fn test_get_auth_id_with_other_keys() {
        let result = get_auth_id(r#"{"auth_id": "tenant-456", "other": "data", "count": 5}"#);
        assert_eq!(result, Some("tenant-456".to_string()));
    }

    #[test]
    fn test_get_auth_id_empty_string() {
        let result = get_auth_id("");
        assert_eq!(result, None);
    }

    #[test]
    fn test_get_auth_id_empty_object() {
        let result = get_auth_id("{}");
        assert_eq!(result, None);
    }

    #[test]
    fn test_get_auth_id_missing_key() {
        let result = get_auth_id(r#"{"other": "value"}"#);
        assert_eq!(result, None);
    }

    #[test]
    fn test_get_auth_id_not_string() {
        // auth_id is a number, not a string
        let result = get_auth_id(r#"{"auth_id": 12345}"#);
        assert_eq!(result, None);
    }

    #[test]
    fn test_get_auth_id_null_value() {
        let result = get_auth_id(r#"{"auth_id": null}"#);
        assert_eq!(result, None);
    }

    #[test]
    fn test_get_auth_id_invalid_json() {
        // Should fail closed (return None) for invalid JSON
        let result = get_auth_id("not json");
        assert_eq!(result, None);
    }

    #[test]
    fn test_get_auth_id_empty_auth_id_value() {
        // Empty string is a valid auth_id
        let result = get_auth_id(r#"{"auth_id": ""}"#);
        assert_eq!(result, Some("".to_string()));
    }

    // ==================== merge_auth_id tests ====================

    #[test]
    fn test_merge_auth_id_into_empty() {
        let result = merge_auth_id("", "tenant-new").unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["auth_id"], "tenant-new");
    }

    #[test]
    fn test_merge_auth_id_into_empty_object() {
        let result = merge_auth_id("{}", "tenant-new").unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["auth_id"], "tenant-new");
    }

    #[test]
    fn test_merge_auth_id_preserves_other_keys() {
        let result = merge_auth_id(r#"{"other": "data", "count": 42}"#, "tenant-new").unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["auth_id"], "tenant-new");
        assert_eq!(parsed["other"], "data");
        assert_eq!(parsed["count"], 42);
    }

    #[test]
    fn test_merge_auth_id_same_value_unchanged() {
        let result = merge_auth_id(r#"{"auth_id": "tenant-same"}"#, "tenant-same").unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["auth_id"], "tenant-same");
    }

    #[test]
    fn test_merge_auth_id_same_value_preserves_other_keys() {
        let result = merge_auth_id(
            r#"{"auth_id": "tenant-same", "extra": "info"}"#,
            "tenant-same",
        )
        .unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["auth_id"], "tenant-same");
        assert_eq!(parsed["extra"], "info");
    }

    #[test]
    fn test_merge_auth_id_conflict_error() {
        let result = merge_auth_id(r#"{"auth_id": "tenant-A"}"#, "tenant-B");
        match result {
            Err(MetadataError::AuthIdConflict {
                existing,
                attempted,
            }) => {
                assert_eq!(existing, "tenant-A");
                assert_eq!(attempted, "tenant-B");
            }
            _ => panic!("Expected AuthIdConflict error"),
        }
    }

    #[test]
    fn test_merge_auth_id_invalid_json_error() {
        let result = merge_auth_id("not json", "tenant-new");
        assert!(matches!(result, Err(MetadataError::InvalidJson(_))));
    }

    #[test]
    fn test_merge_auth_id_overwrites_non_string() {
        // If auth_id exists but is not a string, overwrite it
        let result = merge_auth_id(r#"{"auth_id": 12345}"#, "tenant-new").unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["auth_id"], "tenant-new");
    }

    #[test]
    fn test_merge_auth_id_overwrites_null() {
        let result = merge_auth_id(r#"{"auth_id": null}"#, "tenant-new").unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["auth_id"], "tenant-new");
    }

    #[test]
    fn test_merge_auth_id_complex_metadata() {
        let complex = r#"{
            "version": 2,
            "settings": {"a": 1, "b": [1, 2, 3]},
            "tags": ["tag1", "tag2"]
        }"#;
        let result = merge_auth_id(complex, "tenant-complex").unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["auth_id"], "tenant-complex");
        assert_eq!(parsed["version"], 2);
        assert_eq!(parsed["settings"]["a"], 1);
        assert_eq!(parsed["tags"][0], "tag1");
    }
}
