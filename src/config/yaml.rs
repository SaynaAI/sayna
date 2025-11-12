use serde::Deserialize;
use std::path::PathBuf;

/// Complete YAML configuration structure
///
/// This structure represents the full configuration that can be loaded from a YAML file.
/// All fields are optional to allow partial configuration. Environment variables can
/// override any values specified here.
///
/// # Example YAML structure
/// ```yaml
/// server:
///   host: "0.0.0.0"
///   port: 3001
///
/// livekit:
///   url: "ws://localhost:7880"
///   public_url: "http://localhost:7880"
///   api_key: "your-api-key"
///   api_secret: "your-api-secret"
///
/// providers:
///   deepgram_api_key: "your-deepgram-key"
///   elevenlabs_api_key: "your-elevenlabs-key"
///
/// recording:
///   s3_bucket: "my-bucket"
///   s3_region: "us-west-2"
///   s3_endpoint: "https://s3.amazonaws.com"
///   s3_access_key: "access-key"
///   s3_secret_key: "secret-key"
///
/// cache:
///   path: "/var/cache/sayna"
///   ttl_seconds: 2592000
///
/// auth:
///   required: true
///   service_url: "https://auth.example.com"
///   signing_key_path: "/path/to/key.pem"
///   api_secret: "your-api-secret"
///   timeout_seconds: 5
/// ```
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct YamlConfig {
    pub server: Option<ServerYaml>,
    pub livekit: Option<LiveKitYaml>,
    pub providers: Option<ProvidersYaml>,
    pub recording: Option<RecordingYaml>,
    pub cache: Option<CacheYaml>,
    pub auth: Option<AuthYaml>,
}

/// Server configuration from YAML
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct ServerYaml {
    pub host: Option<String>,
    pub port: Option<u16>,
}

/// LiveKit configuration from YAML
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct LiveKitYaml {
    pub url: Option<String>,
    pub public_url: Option<String>,
    pub api_key: Option<String>,
    pub api_secret: Option<String>,
}

/// Provider API keys from YAML
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct ProvidersYaml {
    pub deepgram_api_key: Option<String>,
    pub elevenlabs_api_key: Option<String>,
}

/// Recording S3 configuration from YAML
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct RecordingYaml {
    pub s3_bucket: Option<String>,
    pub s3_region: Option<String>,
    pub s3_endpoint: Option<String>,
    pub s3_access_key: Option<String>,
    pub s3_secret_key: Option<String>,
}

/// Cache configuration from YAML
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct CacheYaml {
    pub path: Option<String>,
    pub ttl_seconds: Option<u64>,
}

/// Authentication configuration from YAML
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct AuthYaml {
    pub required: Option<bool>,
    pub service_url: Option<String>,
    pub signing_key_path: Option<String>,
    pub api_secret: Option<String>,
    pub timeout_seconds: Option<u64>,
}

impl YamlConfig {
    /// Load configuration from a YAML file
    ///
    /// # Arguments
    /// * `path` - Path to the YAML configuration file
    ///
    /// # Returns
    /// * `Result<YamlConfig, Box<dyn std::error::Error>>` - The loaded configuration or an error
    ///
    /// # Errors
    /// Returns an error if:
    /// - The file cannot be read
    /// - The YAML is malformed
    /// - Required fields have invalid types
    pub fn from_file(path: &PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        let contents = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read config file {}: {e}", path.display()))?;

        let config: YamlConfig = serde_yaml::from_str(&contents)
            .map_err(|e| format!("Failed to parse YAML config: {e}"))?;

        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_yaml_config_full() {
        let yaml = r#"
server:
  host: "127.0.0.1"
  port: 8080

livekit:
  url: "ws://livekit.example.com"
  public_url: "https://livekit.example.com"
  api_key: "test-key"
  api_secret: "test-secret"

providers:
  deepgram_api_key: "dg-key"
  elevenlabs_api_key: "el-key"

recording:
  s3_bucket: "my-recordings"
  s3_region: "us-east-1"
  s3_endpoint: "https://s3.amazonaws.com"
  s3_access_key: "access"
  s3_secret_key: "secret"

cache:
  path: "/tmp/cache"
  ttl_seconds: 3600

auth:
  required: true
  service_url: "https://auth.example.com"
  signing_key_path: "/path/to/key.pem"
  api_secret: "auth-secret"
  timeout_seconds: 10
"#;

        let config: YamlConfig = serde_yaml::from_str(yaml).unwrap();

        assert_eq!(config.server.as_ref().unwrap().host, Some("127.0.0.1".to_string()));
        assert_eq!(config.server.as_ref().unwrap().port, Some(8080));
        assert_eq!(config.livekit.as_ref().unwrap().url, Some("ws://livekit.example.com".to_string()));
        assert_eq!(config.providers.as_ref().unwrap().deepgram_api_key, Some("dg-key".to_string()));
        assert_eq!(config.recording.as_ref().unwrap().s3_bucket, Some("my-recordings".to_string()));
        assert_eq!(config.cache.as_ref().unwrap().path, Some("/tmp/cache".to_string()));
        assert_eq!(config.auth.as_ref().unwrap().required, Some(true));
    }

    #[test]
    fn test_yaml_config_partial() {
        let yaml = r#"
server:
  port: 9000

cache:
  ttl_seconds: 7200
"#;

        let config: YamlConfig = serde_yaml::from_str(yaml).unwrap();

        assert!(config.server.as_ref().unwrap().host.is_none());
        assert_eq!(config.server.as_ref().unwrap().port, Some(9000));
        assert!(config.livekit.is_none());
        assert_eq!(config.cache.as_ref().unwrap().ttl_seconds, Some(7200));
        assert!(config.cache.as_ref().unwrap().path.is_none());
    }

    #[test]
    fn test_yaml_config_empty() {
        let yaml = "";

        let config: YamlConfig = serde_yaml::from_str(yaml).unwrap();

        assert!(config.server.is_none());
        assert!(config.livekit.is_none());
        assert!(config.providers.is_none());
        assert!(config.recording.is_none());
        assert!(config.cache.is_none());
        assert!(config.auth.is_none());
    }

    #[test]
    fn test_from_file() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.yaml");

        let yaml_content = r#"
server:
  host: "localhost"
  port: 3000
"#;

        fs::write(&config_path, yaml_content).unwrap();

        let config = YamlConfig::from_file(&config_path).unwrap();

        assert_eq!(config.server.as_ref().unwrap().host, Some("localhost".to_string()));
        assert_eq!(config.server.as_ref().unwrap().port, Some(3000));
    }

    #[test]
    fn test_from_file_not_found() {
        let path = PathBuf::from("/nonexistent/config.yaml");
        let result = YamlConfig::from_file(&path);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Failed to read config file"));
    }

    #[test]
    fn test_from_file_invalid_yaml() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("invalid.yaml");

        fs::write(&config_path, "invalid: yaml: content:").unwrap();

        let result = YamlConfig::from_file(&config_path);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Failed to parse YAML"));
    }
}
