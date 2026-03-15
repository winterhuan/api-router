//! Configuration management with local JSON storage.

use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Default admin password: "admin" (SHA-256 hash)
const DEFAULT_PASSWORD_HASH: &str =
    "8c6976e5b5410415bde908bd4dee15dfb167a9c873fc4bb8a81f6f2ab448a918";

/// Upstream configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Upstream {
    pub id: String,
    pub base_url: String,
    pub api_format: ApiFormat,
    pub keys: Vec<String>,
    #[serde(default)]
    pub model_map: std::collections::HashMap<String, String>,
    #[serde(default = "default_priority")]
    pub priority: u32,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_priority() -> u32 {
    999
}

fn default_enabled() -> bool {
    true
}

/// API format types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ApiFormat {
    Anthropic,
    Openai,
    OpenaiResponse,
    Gemini,
}

impl Default for ApiFormat {
    fn default() -> Self {
        Self::Anthropic
    }
}

/// Client API key info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientKey {
    pub key: String,
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}

/// Upstream attempt detail for logging
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpstreamAttempt {
    pub upstream_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub status_code: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_headers: Option<std::collections::HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_body: Option<String>,
}

/// Request log entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestLog {
    pub timestamp: String,
    pub method: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream_id: Option<String>,
    pub status_code: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Details of each upstream attempt
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attempts: Vec<UpstreamAttempt>,
}

/// Application configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub upstreams: Vec<Upstream>,
    #[serde(default)]
    pub debug_mode: bool,
    #[serde(default = "default_password_hash")]
    pub admin_password_hash: String,
    #[serde(default)]
    pub client_keys: Vec<ClientKey>,
    #[serde(default)]
    pub access_control_enabled: bool,
}

fn default_password_hash() -> String {
    DEFAULT_PASSWORD_HASH.to_string()
}

/// Global application state
#[derive(Debug)]
pub struct AppState {
    pub config: RwLock<AppConfig>,
    pub config_path: PathBuf,
}

impl AppState {
    pub fn new(data_dir: &Path) -> Arc<Self> {
        std::fs::create_dir_all(data_dir).ok();
        let config_path = data_dir.join("config.json");

        let config = if config_path.exists() {
            match std::fs::read_to_string(&config_path) {
                Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
                Err(_) => AppConfig::default(),
            }
        } else {
            AppConfig::default()
        };

        Arc::new(Self {
            config: RwLock::new(config),
            config_path,
        })
    }

    pub async fn save_config(&self) -> std::io::Result<()> {
        let config = self.config.read().await;
        let content = serde_json::to_string_pretty(&*config)?;
        tokio::fs::write(&self.config_path, content).await
    }
}

/// Generate a secure random API key
pub fn generate_api_key() -> String {
    let mut rng = rand::thread_rng();
    let random_bytes: [u8; 32] = rng.gen();
    let encoded = base64_encode(&random_bytes);
    format!("sk-{}", encoded.replace(['/', '+', '='], ""))
}

fn base64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();

    for chunk in data.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = chunk.get(1).copied().unwrap_or(0) as usize;
        let b2 = chunk.get(2).copied().unwrap_or(0) as usize;

        result.push(ALPHABET[b0 >> 2] as char);
        result.push(ALPHABET[((b0 & 0x03) << 4) | (b1 >> 4)] as char);
        if chunk.len() > 1 {
            result.push(ALPHABET[((b1 & 0x0f) << 2) | (b2 >> 6)] as char);
        }
        if chunk.len() > 2 {
            result.push(ALPHABET[b2 & 0x3f] as char);
        }
    }

    result
}

/// Hash password using SHA-256
pub fn hash_password(password: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(password.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Verify password against hash
pub fn verify_password(password: &str, hash: &str) -> bool {
    hash_password(password) == hash
}

/// Request logs storage
pub struct LogStore {
    logs: RwLock<Vec<RequestLog>>,
    log_path: PathBuf,
}

impl LogStore {
    pub fn new(data_dir: &Path) -> Arc<Self> {
        let log_path = data_dir.join("logs.json");
        let logs: Vec<RequestLog> = if log_path.exists() {
            match std::fs::read_to_string(&log_path) {
                Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
                Err(_) => Vec::new(),
            }
        } else {
            Vec::new()
        };

        Arc::new(Self {
            logs: RwLock::new(logs),
            log_path,
        })
    }

    pub async fn add_log(&self, log: RequestLog) {
        let mut logs = self.logs.write().await;
        logs.insert(0, log);
        if logs.len() > 100 {
            logs.truncate(100);
        }

        if let Ok(content) = serde_json::to_string_pretty(&*logs) {
            let _ = tokio::fs::write(&self.log_path, content).await;
        }
    }

    pub async fn get_logs(&self, limit: usize) -> Vec<RequestLog> {
        let logs = self.logs.read().await;
        logs.iter().take(limit).cloned().collect()
    }

    pub async fn clear_logs(&self) {
        let mut logs = self.logs.write().await;
        logs.clear();
        let _ = tokio::fs::write(&self.log_path, "[]").await;
    }
}
