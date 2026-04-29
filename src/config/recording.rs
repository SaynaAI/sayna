//! Recording storage configuration.
//!
//! Sayna writes call recordings via LiveKit Egress and reads them back over HTTP.
//! Both sides need to agree on the same backend (Amazon S3 or Google Cloud Storage),
//! the same bucket/prefix layout, and the same credentials. This module owns that
//! shared configuration so callers do not duplicate the probing logic.
//!
//! The runtime types ([`RecordingConfig`], [`RecordingBackend`]) are the single
//! source of truth. Construction helpers live alongside:
//!
//! - [`RecordingConfig::build_object_store`] returns the `Arc<dyn ObjectStore>` used
//!   by the `/recording/{stream_id}` download handler.
//! - [`RecordingConfig::build_egress_output`] returns the LiveKit
//!   `EncodedFileOutput` proto that tells Egress where to upload.
//! - [`RecordingConfig::build_recording_filepath`] composes `{prefix}/{stream_id}/audio.ogg`.
//!
//! The YAML / environment layer ([`RecordingYaml`]) is intentionally permissive so
//! values can come from either source; [`RecordingYaml::resolve`] performs the
//! validation that turns it into a [`RecordingConfig`].

use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use livekit_protocol as proto;
use object_store::ObjectStore;
use object_store::aws::AmazonS3Builder;
use object_store::gcp::GoogleCloudStorageBuilder;
use serde::Deserialize;
use thiserror::Error;

/// Errors that can occur while building or validating recording configuration.
#[derive(Debug, Error)]
pub enum RecordingConfigError {
    /// A backend variant was selected but a required field was missing.
    #[error("recording.{backend} backend is missing required field '{field}'")]
    MissingField {
        backend: &'static str,
        field: &'static str,
    },

    /// Both `credentials_path` and `credentials_json` were provided for the GCS backend.
    #[error("recording.gcs backend accepts at most one of credentials_path or credentials_json")]
    AmbiguousGcsCredentials,

    /// Neither `credentials_path` nor `credentials_json` was provided for the GCS backend.
    #[error(
        "recording.gcs backend requires either credentials_path or credentials_json (LiveKit Egress needs the service account JSON)"
    )]
    MissingGcsCredentials,

    /// The GCS service-account JSON could not be read from disk.
    #[error("failed to read GCS service account file '{}': {source}", path.display())]
    GcsCredentialsRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// The user picked an unrecognized backend type.
    #[error("recording backend '{0}' is not supported (expected 's3' or 'gcs')")]
    UnknownBackend(String),

    /// The configuration mixed YAML and environment variables for two different backends.
    #[error(
        "recording configuration is inconsistent: YAML selects '{yaml}' but environment selects '{env}'"
    )]
    BackendMismatch {
        yaml: &'static str,
        env: &'static str,
    },

    /// `object_store` failed to construct the underlying client.
    #[error("failed to build recording object store: {0}")]
    ObjectStore(#[from] object_store::Error),
}

/// Resolved, validated recording storage configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingConfig {
    /// Object key prefix shared by every backend, e.g. `"recordings/prod"`.
    /// Combined with `stream_id` to form `{prefix}/{stream_id}/audio.ogg`.
    pub prefix: String,
    pub backend: RecordingBackend,
}

/// Discriminated union over supported storage backends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordingBackend {
    S3(S3Backend),
    Gcs(GcsBackend),
}

/// Amazon S3 (or S3-compatible) backend configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S3Backend {
    pub bucket: String,
    pub region: String,
    pub access_key: String,
    pub secret_key: String,
    /// Custom endpoint URL for S3-compatible stores (MinIO, Cloudflare R2, etc.).
    /// When `None`, the AWS S3 default endpoint is used.
    pub endpoint: Option<String>,
    /// `true` ⇒ path-style addressing (`https://endpoint/bucket/key`); required
    /// for most S3-compatible servers. `false` ⇒ virtual-hosted style; the
    /// default and the AWS S3 recommendation.
    pub force_path_style: bool,
}

/// Google Cloud Storage backend configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcsBackend {
    pub bucket: String,
    /// Service-account JSON content, resolved at config load time.
    ///
    /// LiveKit Egress requires the credentials inline in its protobuf, so the
    /// JSON file (if the operator pointed at one) is read eagerly here. That
    /// keeps Sayna's local downloader and LiveKit Egress pinned to the exact
    /// same credential bytes.
    ///
    /// To target a GCS emulator (e.g. fake-gcs-server) include `gcs_base_url`
    /// inside this JSON; `object_store` reads it directly. There is no
    /// separate endpoint override exposed by the GCS builder.
    pub credentials_json: String,
}

impl RecordingConfig {
    /// Bucket name used by the active backend. Convenience for logging callers.
    pub fn bucket(&self) -> &str {
        match &self.backend {
            RecordingBackend::S3(s3) => &s3.bucket,
            RecordingBackend::Gcs(gcs) => &gcs.bucket,
        }
    }

    /// Short identifier for the backend variant (`"s3"` or `"gcs"`); useful in logs.
    pub fn backend_kind(&self) -> &'static str {
        match &self.backend {
            RecordingBackend::S3(_) => "s3",
            RecordingBackend::Gcs(_) => "gcs",
        }
    }

    /// Compose the full object key for a given stream id.
    ///
    /// Trailing slashes on `prefix` are normalized away so the result never
    /// contains `//`. An empty prefix yields `"{stream_id}/audio.ogg"`.
    pub fn build_recording_filepath(&self, stream_id: &str) -> String {
        build_recording_filepath(&self.prefix, stream_id)
    }

    /// Build the `Arc<dyn ObjectStore>` used by the recording download handler.
    pub fn build_object_store(&self) -> Result<Arc<dyn ObjectStore>, RecordingConfigError> {
        match &self.backend {
            RecordingBackend::S3(s3) => {
                let mut builder = AmazonS3Builder::new()
                    .with_bucket_name(&s3.bucket)
                    .with_region(&s3.region)
                    .with_access_key_id(&s3.access_key)
                    .with_secret_access_key(&s3.secret_key)
                    .with_virtual_hosted_style_request(!s3.force_path_style);

                if let Some(endpoint) = &s3.endpoint {
                    if endpoint.starts_with("http://") {
                        builder = builder.with_allow_http(true);
                    }
                    builder = builder.with_endpoint(endpoint);
                }

                Ok(Arc::new(builder.build()?))
            }
            RecordingBackend::Gcs(gcs) => {
                let builder = GoogleCloudStorageBuilder::new()
                    .with_bucket_name(&gcs.bucket)
                    .with_service_account_key(&gcs.credentials_json);

                Ok(Arc::new(builder.build()?))
            }
        }
    }

    /// Build the LiveKit Egress `EncodedFileOutput` proto for a given recording filepath.
    pub fn build_egress_output(&self, filepath: String) -> proto::EncodedFileOutput {
        let output = match &self.backend {
            RecordingBackend::S3(s3) => {
                let s3_upload = proto::S3Upload {
                    bucket: s3.bucket.clone(),
                    region: s3.region.clone(),
                    endpoint: s3.endpoint.clone().unwrap_or_default(),
                    access_key: s3.access_key.clone(),
                    secret: s3.secret_key.clone(),
                    force_path_style: s3.force_path_style,
                    ..Default::default()
                };
                proto::encoded_file_output::Output::S3(s3_upload)
            }
            RecordingBackend::Gcs(gcs) => {
                let gcp_upload = proto::GcpUpload {
                    credentials: gcs.credentials_json.clone(),
                    bucket: gcs.bucket.clone(),
                    ..Default::default()
                };
                proto::encoded_file_output::Output::Gcp(gcp_upload)
            }
        };

        proto::EncodedFileOutput {
            file_type: proto::EncodedFileType::Ogg as i32,
            filepath,
            disable_manifest: false,
            output: Some(output),
        }
    }
}

/// Compose `{prefix}/{stream_id}/audio.ogg`, normalizing trailing slashes on `prefix`.
fn build_recording_filepath(prefix: &str, stream_id: &str) -> String {
    let trimmed = prefix.trim_end_matches('/');
    if trimmed.is_empty() {
        format!("{stream_id}/audio.ogg")
    } else {
        format!("{trimmed}/{stream_id}/audio.ogg")
    }
}

// -----------------------------------------------------------------------------
// YAML / environment merge layer
// -----------------------------------------------------------------------------

/// YAML shape for the `recording:` block.
///
/// All fields are optional so callers can mix-and-match with environment
/// variables; [`RecordingYaml::resolve`] performs the actual validation.
#[derive(Debug, Clone, Deserialize, Default, PartialEq, Eq)]
#[serde(default)]
pub struct RecordingYaml {
    pub prefix: Option<String>,
    pub backend: Option<RecordingBackendYaml>,
}

/// Tagged-union YAML form for the active backend. Forces operators to be
/// explicit about which backend they intend to use rather than letting
/// half-configured combinations slip through.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RecordingBackendYaml {
    S3(S3BackendYaml),
    Gcs(GcsBackendYaml),
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq, Eq)]
#[serde(default)]
pub struct S3BackendYaml {
    pub bucket: Option<String>,
    pub region: Option<String>,
    pub access_key: Option<String>,
    pub secret_key: Option<String>,
    pub endpoint: Option<String>,
    pub force_path_style: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq, Eq)]
#[serde(default)]
pub struct GcsBackendYaml {
    pub bucket: Option<String>,
    /// Path on disk to a service-account JSON file.
    pub credentials_path: Option<String>,
    /// Inline service-account JSON, suitable for secret-management injection.
    pub credentials_json: Option<String>,
}

impl RecordingYaml {
    /// Merge `self` (YAML) with environment variables and produce a validated
    /// [`RecordingConfig`], or `None` if neither source configures recording.
    ///
    /// Resolution order:
    /// 1. The backend variant is taken from YAML if present, otherwise from
    ///    the `RECORDING_BACKEND` environment variable, otherwise inferred
    ///    from the presence of `RECORDING_S3_*` / `RECORDING_GCS_*` variables.
    /// 2. Within the chosen variant, YAML values win over environment values
    ///    (matching the existing project-wide priority order).
    pub fn resolve(self) -> Result<Option<RecordingConfig>, RecordingConfigError> {
        let env_backend = env::var("RECORDING_BACKEND").ok();
        let env_has_s3 = env_has_any_s3();
        let env_has_gcs = env_has_any_gcs();

        let yaml_kind = self.backend.as_ref().map(BackendKind::from_yaml);
        let env_kind = match env_backend.as_deref().map(str::trim) {
            Some("") | None => match (env_has_s3, env_has_gcs) {
                (true, false) => Some(BackendKind::S3),
                (false, true) => Some(BackendKind::Gcs),
                (true, true) => {
                    // Both prefixes present in env; require an explicit selector
                    // rather than guessing.
                    return Err(RecordingConfigError::UnknownBackend(
                        "ambiguous (set RECORDING_BACKEND to 's3' or 'gcs')".to_string(),
                    ));
                }
                (false, false) => None,
            },
            Some(other) => Some(BackendKind::parse(other)?),
        };

        let kind = match (yaml_kind, env_kind) {
            (Some(y), Some(e)) if y != e => {
                return Err(RecordingConfigError::BackendMismatch {
                    yaml: y.as_str(),
                    env: e.as_str(),
                });
            }
            (Some(k), _) | (_, Some(k)) => k,
            (None, None) => {
                // No YAML backend and no env signal at all — recording disabled.
                // A bare `prefix:` in YAML without a backend is a no-op rather
                // than an error; recording stays off.
                return Ok(None);
            }
        };

        let prefix = self
            .prefix
            .clone()
            .or_else(|| env::var("RECORDING_PREFIX").ok())
            .unwrap_or_default();

        let backend = match kind {
            BackendKind::S3 => RecordingBackend::S3(resolve_s3(self.backend)?),
            BackendKind::Gcs => RecordingBackend::Gcs(resolve_gcs(self.backend)?),
        };

        Ok(Some(RecordingConfig { prefix, backend }))
    }
}

fn resolve_s3(
    yaml_backend: Option<RecordingBackendYaml>,
) -> Result<S3Backend, RecordingConfigError> {
    let yaml = match yaml_backend {
        Some(RecordingBackendYaml::S3(s3)) => s3,
        // YAML picked a different backend or none — fall back to env-only.
        _ => S3BackendYaml::default(),
    };

    let bucket = required("s3", "bucket", yaml.bucket, "RECORDING_S3_BUCKET")?;
    let region = required("s3", "region", yaml.region, "RECORDING_S3_REGION")?;
    let access_key = required(
        "s3",
        "access_key",
        yaml.access_key,
        "RECORDING_S3_ACCESS_KEY",
    )?;
    let secret_key = required(
        "s3",
        "secret_key",
        yaml.secret_key,
        "RECORDING_S3_SECRET_KEY",
    )?;
    let endpoint = optional(yaml.endpoint, "RECORDING_S3_ENDPOINT");
    let force_path_style = match yaml.force_path_style {
        Some(v) => v,
        None => env::var("RECORDING_S3_FORCE_PATH_STYLE")
            .ok()
            .and_then(|raw| parse_bool(&raw))
            .unwrap_or(false),
    };

    Ok(S3Backend {
        bucket,
        region,
        access_key,
        secret_key,
        endpoint,
        force_path_style,
    })
}

fn resolve_gcs(
    yaml_backend: Option<RecordingBackendYaml>,
) -> Result<GcsBackend, RecordingConfigError> {
    let yaml = match yaml_backend {
        Some(RecordingBackendYaml::Gcs(gcs)) => gcs,
        _ => GcsBackendYaml::default(),
    };

    let bucket = required("gcs", "bucket", yaml.bucket, "RECORDING_GCS_BUCKET")?;
    let credentials_path = optional(yaml.credentials_path, "RECORDING_GCS_CREDENTIALS_PATH");
    let credentials_json = optional(yaml.credentials_json, "RECORDING_GCS_CREDENTIALS_JSON");

    let credentials_json = match (credentials_path, credentials_json) {
        (Some(_), Some(_)) => return Err(RecordingConfigError::AmbiguousGcsCredentials),
        (Some(path), None) => {
            let path_buf = PathBuf::from(&path);
            fs::read_to_string(&path_buf).map_err(|source| {
                RecordingConfigError::GcsCredentialsRead {
                    path: path_buf,
                    source,
                }
            })?
        }
        (None, Some(json)) => json,
        (None, None) => return Err(RecordingConfigError::MissingGcsCredentials),
    };

    Ok(GcsBackend {
        bucket,
        credentials_json,
    })
}

fn required(
    backend: &'static str,
    field: &'static str,
    yaml_value: Option<String>,
    env_var: &str,
) -> Result<String, RecordingConfigError> {
    yaml_value
        .or_else(|| env::var(env_var).ok())
        .filter(|v| !v.is_empty())
        .ok_or(RecordingConfigError::MissingField { backend, field })
}

fn optional(yaml_value: Option<String>, env_var: &str) -> Option<String> {
    yaml_value
        .or_else(|| env::var(env_var).ok())
        .filter(|v| !v.is_empty())
}

fn env_has_any_s3() -> bool {
    [
        "RECORDING_S3_BUCKET",
        "RECORDING_S3_REGION",
        "RECORDING_S3_ENDPOINT",
        "RECORDING_S3_ACCESS_KEY",
        "RECORDING_S3_SECRET_KEY",
        "RECORDING_S3_FORCE_PATH_STYLE",
    ]
    .iter()
    .any(|key| env::var(key).is_ok())
}

fn env_has_any_gcs() -> bool {
    [
        "RECORDING_GCS_BUCKET",
        "RECORDING_GCS_CREDENTIALS_PATH",
        "RECORDING_GCS_CREDENTIALS_JSON",
    ]
    .iter()
    .any(|key| env::var(key).is_ok())
}

fn parse_bool(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BackendKind {
    S3,
    Gcs,
}

impl BackendKind {
    fn as_str(self) -> &'static str {
        match self {
            BackendKind::S3 => "s3",
            BackendKind::Gcs => "gcs",
        }
    }

    fn parse(raw: &str) -> Result<Self, RecordingConfigError> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "s3" => Ok(BackendKind::S3),
            "gcs" | "gcp" | "google" => Ok(BackendKind::Gcs),
            other => Err(RecordingConfigError::UnknownBackend(other.to_string())),
        }
    }

    fn from_yaml(yaml: &RecordingBackendYaml) -> Self {
        match yaml {
            RecordingBackendYaml::S3(_) => BackendKind::S3,
            RecordingBackendYaml::Gcs(_) => BackendKind::Gcs,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::io::Write;
    use tempfile::NamedTempFile;

    /// All recording-related env vars touched by the merge layer. Tests scrub
    /// these before and after to avoid cross-contamination from the runtime env.
    const ALL_RECORDING_ENV_VARS: &[&str] = &[
        "RECORDING_BACKEND",
        "RECORDING_PREFIX",
        "RECORDING_S3_BUCKET",
        "RECORDING_S3_REGION",
        "RECORDING_S3_ENDPOINT",
        "RECORDING_S3_ACCESS_KEY",
        "RECORDING_S3_SECRET_KEY",
        "RECORDING_S3_FORCE_PATH_STYLE",
        "RECORDING_GCS_BUCKET",
        "RECORDING_GCS_CREDENTIALS_PATH",
        "RECORDING_GCS_CREDENTIALS_JSON",
    ];

    fn scrub_env() {
        // SAFETY: tests guarded by #[serial] so no concurrent env access can race.
        unsafe {
            for key in ALL_RECORDING_ENV_VARS {
                env::remove_var(key);
            }
        }
    }

    fn s3_yaml(bucket: &str) -> RecordingYaml {
        RecordingYaml {
            prefix: Some("recordings/test".into()),
            backend: Some(RecordingBackendYaml::S3(S3BackendYaml {
                bucket: Some(bucket.into()),
                region: Some("us-east-1".into()),
                access_key: Some("ak".into()),
                secret_key: Some("sk".into()),
                endpoint: Some("https://s3.amazonaws.com".into()),
                force_path_style: Some(false),
            })),
        }
    }

    #[test]
    fn build_filepath_with_prefix() {
        let cfg = RecordingConfig {
            prefix: "recordings/prod".into(),
            backend: RecordingBackend::S3(S3Backend {
                bucket: "b".into(),
                region: "r".into(),
                access_key: "ak".into(),
                secret_key: "sk".into(),
                endpoint: None,
                force_path_style: false,
            }),
        };
        assert_eq!(
            cfg.build_recording_filepath("abc"),
            "recordings/prod/abc/audio.ogg"
        );
    }

    #[test]
    fn build_filepath_strips_trailing_slashes() {
        assert_eq!(
            build_recording_filepath("data///", "xyz"),
            "data/xyz/audio.ogg"
        );
    }

    #[test]
    fn build_filepath_empty_prefix() {
        assert_eq!(build_recording_filepath("", "abc"), "abc/audio.ogg");
        assert_eq!(build_recording_filepath("/", "abc"), "abc/audio.ogg");
    }

    #[test]
    #[serial]
    fn resolve_yaml_only_s3() {
        scrub_env();
        let cfg = s3_yaml("my-bucket").resolve().unwrap().unwrap();
        match cfg.backend {
            RecordingBackend::S3(s3) => {
                assert_eq!(s3.bucket, "my-bucket");
                assert_eq!(s3.region, "us-east-1");
                assert!(!s3.force_path_style);
            }
            _ => panic!("expected S3 backend"),
        }
        assert_eq!(cfg.prefix, "recordings/test");
        scrub_env();
    }

    #[test]
    #[serial]
    fn resolve_env_only_s3() {
        scrub_env();
        // SAFETY: serialized via #[serial]
        unsafe {
            env::set_var("RECORDING_S3_BUCKET", "env-bucket");
            env::set_var("RECORDING_S3_REGION", "us-west-2");
            env::set_var("RECORDING_S3_ACCESS_KEY", "ak");
            env::set_var("RECORDING_S3_SECRET_KEY", "sk");
            env::set_var("RECORDING_S3_ENDPOINT", "http://minio.local:9000");
            env::set_var("RECORDING_PREFIX", "calls");
        }
        let cfg = RecordingYaml::default().resolve().unwrap().unwrap();
        assert_eq!(cfg.prefix, "calls");
        assert_eq!(cfg.bucket(), "env-bucket");
        assert_eq!(cfg.backend_kind(), "s3");
        scrub_env();
    }

    #[test]
    #[serial]
    fn resolve_yaml_overrides_env_within_same_backend() {
        scrub_env();
        // SAFETY: serialized via #[serial]
        unsafe {
            env::set_var("RECORDING_S3_BUCKET", "env-bucket");
            env::set_var("RECORDING_S3_REGION", "us-west-2");
            env::set_var("RECORDING_S3_ACCESS_KEY", "env-ak");
            env::set_var("RECORDING_S3_SECRET_KEY", "env-sk");
        }
        let cfg = s3_yaml("yaml-bucket").resolve().unwrap().unwrap();
        match cfg.backend {
            RecordingBackend::S3(s3) => {
                assert_eq!(s3.bucket, "yaml-bucket");
                // YAML region wins
                assert_eq!(s3.region, "us-east-1");
                // YAML credentials win
                assert_eq!(s3.access_key, "ak");
            }
            _ => panic!("expected S3"),
        }
        scrub_env();
    }

    #[test]
    #[serial]
    fn resolve_yaml_gcs_with_inline_json() {
        scrub_env();
        let yaml = RecordingYaml {
            prefix: Some("recordings".into()),
            backend: Some(RecordingBackendYaml::Gcs(GcsBackendYaml {
                bucket: Some("g-bucket".into()),
                credentials_json: Some(r#"{"type":"service_account"}"#.into()),
                ..Default::default()
            })),
        };
        let cfg = yaml.resolve().unwrap().unwrap();
        match cfg.backend {
            RecordingBackend::Gcs(gcs) => {
                assert_eq!(gcs.bucket, "g-bucket");
                assert_eq!(gcs.credentials_json, r#"{"type":"service_account"}"#);
            }
            _ => panic!("expected GCS"),
        }
    }

    #[test]
    #[serial]
    fn resolve_gcs_reads_credentials_path() {
        scrub_env();
        let mut tmp = NamedTempFile::new().unwrap();
        writeln!(tmp, "{{\"type\":\"service_account\",\"project_id\":\"x\"}}").unwrap();
        let yaml = RecordingYaml {
            prefix: None,
            backend: Some(RecordingBackendYaml::Gcs(GcsBackendYaml {
                bucket: Some("g-bucket".into()),
                credentials_path: Some(tmp.path().to_string_lossy().into_owned()),
                ..Default::default()
            })),
        };
        let cfg = yaml.resolve().unwrap().unwrap();
        match cfg.backend {
            RecordingBackend::Gcs(gcs) => {
                assert!(gcs.credentials_json.contains("service_account"));
            }
            _ => panic!("expected GCS"),
        }
    }

    #[test]
    #[serial]
    fn resolve_gcs_rejects_both_credentials() {
        scrub_env();
        let yaml = RecordingYaml {
            prefix: None,
            backend: Some(RecordingBackendYaml::Gcs(GcsBackendYaml {
                bucket: Some("g".into()),
                credentials_path: Some("/tmp/x.json".into()),
                credentials_json: Some(r#"{"a":1}"#.into()),
            })),
        };
        assert!(matches!(
            yaml.resolve(),
            Err(RecordingConfigError::AmbiguousGcsCredentials)
        ));
    }

    #[test]
    #[serial]
    fn resolve_gcs_requires_credentials() {
        scrub_env();
        let yaml = RecordingYaml {
            prefix: None,
            backend: Some(RecordingBackendYaml::Gcs(GcsBackendYaml {
                bucket: Some("g".into()),
                ..Default::default()
            })),
        };
        assert!(matches!(
            yaml.resolve(),
            Err(RecordingConfigError::MissingGcsCredentials)
        ));
    }

    #[test]
    #[serial]
    fn resolve_s3_missing_required_field() {
        scrub_env();
        let yaml = RecordingYaml {
            prefix: None,
            backend: Some(RecordingBackendYaml::S3(S3BackendYaml {
                bucket: Some("b".into()),
                region: None, // missing
                access_key: Some("ak".into()),
                secret_key: Some("sk".into()),
                ..Default::default()
            })),
        };
        match yaml.resolve() {
            Err(RecordingConfigError::MissingField { backend, field }) => {
                assert_eq!(backend, "s3");
                assert_eq!(field, "region");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    #[serial]
    fn resolve_returns_none_when_unconfigured() {
        scrub_env();
        let result = RecordingYaml::default().resolve().unwrap();
        assert!(result.is_none());
    }

    #[test]
    #[serial]
    fn resolve_explicit_backend_yaml_env_mismatch() {
        scrub_env();
        // SAFETY: serialized via #[serial]
        unsafe {
            env::set_var("RECORDING_BACKEND", "gcs");
        }
        let yaml = s3_yaml("b");
        match yaml.resolve() {
            Err(RecordingConfigError::BackendMismatch { yaml, env }) => {
                assert_eq!(yaml, "s3");
                assert_eq!(env, "gcs");
            }
            other => panic!("unexpected: {other:?}"),
        }
        scrub_env();
    }

    #[test]
    #[serial]
    fn resolve_unknown_backend() {
        scrub_env();
        // SAFETY: serialized via #[serial]
        unsafe {
            env::set_var("RECORDING_BACKEND", "azure");
        }
        match RecordingYaml::default().resolve() {
            Err(RecordingConfigError::UnknownBackend(name)) => assert_eq!(name, "azure"),
            other => panic!("unexpected: {other:?}"),
        }
        scrub_env();
    }

    #[test]
    #[serial]
    fn resolve_ambiguous_env_requires_explicit_backend() {
        scrub_env();
        // SAFETY: serialized via #[serial]
        unsafe {
            env::set_var("RECORDING_S3_BUCKET", "s3b");
            env::set_var("RECORDING_GCS_BUCKET", "gcsb");
        }
        match RecordingYaml::default().resolve() {
            Err(RecordingConfigError::UnknownBackend(_)) => {}
            other => panic!("unexpected: {other:?}"),
        }
        scrub_env();
    }

    #[test]
    fn build_egress_output_s3() {
        let cfg = RecordingConfig {
            prefix: "p".into(),
            backend: RecordingBackend::S3(S3Backend {
                bucket: "b".into(),
                region: "r".into(),
                access_key: "ak".into(),
                secret_key: "sk".into(),
                endpoint: Some("https://example.com".into()),
                force_path_style: true,
            }),
        };
        let out = cfg.build_egress_output("p/x/audio.ogg".to_string());
        assert_eq!(out.filepath, "p/x/audio.ogg");
        match out.output {
            Some(proto::encoded_file_output::Output::S3(s3)) => {
                assert_eq!(s3.bucket, "b");
                assert_eq!(s3.endpoint, "https://example.com");
                assert!(s3.force_path_style);
            }
            other => panic!("unexpected output variant: {other:?}"),
        }
    }

    #[test]
    fn build_egress_output_gcs() {
        let cfg = RecordingConfig {
            prefix: "p".into(),
            backend: RecordingBackend::Gcs(GcsBackend {
                bucket: "g".into(),
                credentials_json: r#"{"a":1}"#.into(),
            }),
        };
        let out = cfg.build_egress_output("p/x/audio.ogg".to_string());
        match out.output {
            Some(proto::encoded_file_output::Output::Gcp(gcp)) => {
                assert_eq!(gcp.bucket, "g");
                assert_eq!(gcp.credentials, r#"{"a":1}"#);
            }
            other => panic!("unexpected output variant: {other:?}"),
        }
    }

    #[test]
    fn build_object_store_s3_succeeds() {
        let cfg = RecordingConfig {
            prefix: String::new(),
            backend: RecordingBackend::S3(S3Backend {
                bucket: "b".into(),
                region: "us-east-1".into(),
                access_key: "ak".into(),
                secret_key: "sk".into(),
                endpoint: Some("http://localhost:9000".into()),
                force_path_style: true,
            }),
        };
        // Should construct without error; we don't make any network calls.
        let _ = cfg.build_object_store().expect("S3 store should build");
    }

    #[test]
    fn build_object_store_gcs_returns_object_store_error_on_invalid_key() {
        // The GCS builder validates the RSA private key at build time. A real
        // service-account JSON requires an actual key, so we simply assert that
        // an invalid key surfaces as the typed `ObjectStore` error variant
        // rather than panicking. The happy path is exercised in production
        // and through the egress proto test below (which does not validate keys).
        let cfg = RecordingConfig {
            prefix: String::new(),
            backend: RecordingBackend::Gcs(GcsBackend {
                bucket: "g".into(),
                credentials_json: r#"{
                    "type": "service_account",
                    "project_id": "p",
                    "private_key_id": "k",
                    "private_key": "-----BEGIN PRIVATE KEY-----\nINVALID\n-----END PRIVATE KEY-----\n",
                    "client_email": "x@y.iam.gserviceaccount.com",
                    "client_id": "1",
                    "auth_uri": "https://accounts.google.com/o/oauth2/auth",
                    "token_uri": "https://oauth2.googleapis.com/token"
                }"#
                .into(),
            }),
        };
        match cfg.build_object_store() {
            Err(RecordingConfigError::ObjectStore(_)) => {}
            other => panic!("expected ObjectStore error, got {other:?}"),
        }
    }
}
