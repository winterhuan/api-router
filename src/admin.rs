//! Admin API routes for configuration management.

use crate::config::{generate_api_key, hash_password, verify_password, AppState, LogStore};
use axum::{
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Config update request
#[derive(Debug, Deserialize)]
pub struct ConfigUpdate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstreams: Option<Vec<crate::config::Upstream>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debug_mode: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_password: Option<String>,
}

/// Client keys update request
#[derive(Debug, Deserialize)]
pub struct ClientKeysUpdate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keys: Option<Vec<crate::config::ClientKey>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_control_enabled: Option<bool>,
}

/// Generic response
#[derive(Debug, Serialize)]
pub struct GenericResponse {
    pub ok: bool,
}

/// Logs response
#[derive(Debug, Serialize)]
pub struct LogsResponse {
    pub logs: Vec<crate::config::RequestLog>,
}

/// Generate key response
#[derive(Debug, Serialize)]
pub struct GenerateKeyResponse {
    pub key: String,
}

/// Verify admin password from header
async fn verify_admin(headers: &HeaderMap, state: &Arc<AppState>) -> bool {
    let password = headers
        .get("x-admin-password")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let config = state.config.read().await;
    verify_password(password, &config.admin_password_hash)
}

/// POST /admin/verify - Verify admin password
pub async fn verify_password_handler(state: Arc<AppState>, headers: HeaderMap) -> Response {
    let is_valid = verify_admin(&headers, &state).await;
    Json(serde_json::json!({ "ok": is_valid })).into_response()
}

/// GET /admin/config - Get full configuration
pub async fn get_config(state: Arc<AppState>, headers: HeaderMap) -> Response {
    if !verify_admin(&headers, &state).await {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "Unauthorized" }))).into_response();
    }

    let config = state.config.read().await;
    Json(serde_json::json!({
        "upstreams": config.upstreams,
        "debug_mode": config.debug_mode
    }))
    .into_response()
}

/// POST /admin/config - Update configuration
pub async fn update_config(state: Arc<AppState>, headers: HeaderMap, Json(payload): Json<ConfigUpdate>) -> Response {
    if !verify_admin(&headers, &state).await {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "Unauthorized" }))).into_response();
    }

    {
        let mut config = state.config.write().await;

        if let Some(upstreams) = payload.upstreams {
            config.upstreams = upstreams;
        }

        if let Some(debug_mode) = payload.debug_mode {
            config.debug_mode = debug_mode;
        }

        if let Some(new_password) = payload.new_password {
            config.admin_password_hash = hash_password(&new_password);
        }
    }

    if let Err(e) = state.save_config().await {
        tracing::error!("Failed to save config: {}", e);
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": "Failed to save config" }))).into_response();
    }

    // Clear circuit breaker state to ensure new config takes effect immediately
    crate::proxy::clear_circuit_breaker();

    Json(GenericResponse { ok: true }).into_response()
}

/// GET /admin/client-keys - Get all client API keys
pub async fn get_client_keys(state: Arc<AppState>, headers: HeaderMap) -> Response {
    if !verify_admin(&headers, &state).await {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "Unauthorized" }))).into_response();
    }

    let config = state.config.read().await;
    Json(serde_json::json!({
        "keys": config.client_keys,
        "access_control_enabled": config.access_control_enabled
    }))
    .into_response()
}

/// POST /admin/client-keys - Update client API keys
pub async fn update_client_keys(state: Arc<AppState>, headers: HeaderMap, Json(payload): Json<ClientKeysUpdate>) -> Response {
    if !verify_admin(&headers, &state).await {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "Unauthorized" }))).into_response();
    }

    {
        let mut config = state.config.write().await;

        if let Some(keys) = payload.keys {
            config.client_keys = keys;
        }

        if let Some(enabled) = payload.access_control_enabled {
            config.access_control_enabled = enabled;
        }
    }

    if let Err(e) = state.save_config().await {
        tracing::error!("Failed to save config: {}", e);
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": "Failed to save config" }))).into_response();
    }

    Json(GenericResponse { ok: true }).into_response()
}

/// POST /admin/generate-key - Generate a new client API key
pub async fn generate_key(state: Arc<AppState>, headers: HeaderMap) -> Response {
    if !verify_admin(&headers, &state).await {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "Unauthorized" }))).into_response();
    }

    let new_key = generate_api_key();
    Json(GenerateKeyResponse { key: new_key }).into_response()
}

/// GET /admin/logs - Get request logs
pub async fn get_logs(state: Arc<AppState>, log_store: Arc<LogStore>, headers: HeaderMap) -> Response {
    if !verify_admin(&headers, &state).await {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "Unauthorized" }))).into_response();
    }

    let logs = log_store.get_logs(100).await;
    Json(LogsResponse { logs }).into_response()
}

/// DELETE /admin/logs - Clear all request logs
pub async fn clear_logs(state: Arc<AppState>, log_store: Arc<LogStore>, headers: HeaderMap) -> Response {
    if !verify_admin(&headers, &state).await {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "Unauthorized" }))).into_response();
    }

    log_store.clear_logs().await;
    Json(GenericResponse { ok: true }).into_response()
}