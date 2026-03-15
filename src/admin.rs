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
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "Unauthorized" })),
        )
            .into_response();
    }

    let config = state.config.read().await;
    Json(serde_json::json!({
        "upstreams": config.upstreams,
        "debug_mode": config.debug_mode
    }))
    .into_response()
}

/// POST /admin/config - Update configuration
pub async fn update_config(
    state: Arc<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ConfigUpdate>,
) -> Response {
    if !verify_admin(&headers, &state).await {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "Unauthorized" })),
        )
            .into_response();
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
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": "Failed to save config" })),
        )
            .into_response();
    }

    // Clear circuit breaker state to ensure new config takes effect immediately
    crate::proxy::clear_circuit_breaker();

    Json(GenericResponse { ok: true }).into_response()
}

/// GET /admin/client-keys - Get all client API keys
pub async fn get_client_keys(state: Arc<AppState>, headers: HeaderMap) -> Response {
    if !verify_admin(&headers, &state).await {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "Unauthorized" })),
        )
            .into_response();
    }

    let config = state.config.read().await;
    Json(serde_json::json!({
        "keys": config.client_keys,
        "access_control_enabled": config.access_control_enabled
    }))
    .into_response()
}

/// POST /admin/client-keys - Update client API keys
pub async fn update_client_keys(
    state: Arc<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ClientKeysUpdate>,
) -> Response {
    if !verify_admin(&headers, &state).await {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "Unauthorized" })),
        )
            .into_response();
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
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": "Failed to save config" })),
        )
            .into_response();
    }

    Json(GenericResponse { ok: true }).into_response()
}

/// POST /admin/generate-key - Generate a new client API key
pub async fn generate_key(state: Arc<AppState>, headers: HeaderMap) -> Response {
    if !verify_admin(&headers, &state).await {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "Unauthorized" })),
        )
            .into_response();
    }

    let new_key = generate_api_key();
    Json(GenerateKeyResponse { key: new_key }).into_response()
}

/// GET /admin/logs - Get request logs
pub async fn get_logs(
    state: Arc<AppState>,
    log_store: Arc<LogStore>,
    headers: HeaderMap,
) -> Response {
    if !verify_admin(&headers, &state).await {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "Unauthorized" })),
        )
            .into_response();
    }

    let logs = log_store.get_logs(100).await;
    Json(LogsResponse { logs }).into_response()
}

/// DELETE /admin/logs - Clear all request logs
pub async fn clear_logs(
    state: Arc<AppState>,
    log_store: Arc<LogStore>,
    headers: HeaderMap,
) -> Response {
    if !verify_admin(&headers, &state).await {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "Unauthorized" })),
        )
            .into_response();
    }

    log_store.clear_logs().await;
    Json(GenericResponse { ok: true }).into_response()
}

/// Model test request
#[derive(Debug, Deserialize)]
pub struct ModelTestRequest {
    pub upstream_id: Option<String>,
    pub model: String,
    pub prompt: String,
    #[serde(default)]
    pub stream: bool,
    pub source_format: Option<crate::config::ApiFormat>,
}

/// POST /admin/test-model - Test a model or specific upstream
pub async fn test_model(
    state: Arc<AppState>,
    log_store: Arc<LogStore>,
    headers: HeaderMap,
    Json(payload): Json<ModelTestRequest>,
) -> Response {
    if !verify_admin(&headers, &state).await {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "Unauthorized" })),
        )
            .into_response();
    }

    let source_fmt = payload
        .source_format
        .unwrap_or(crate::config::ApiFormat::Anthropic);

    let body = match source_fmt {
        crate::config::ApiFormat::Anthropic => {
            serde_json::json!({
                "model": payload.model,
                "messages": [
                    { "role": "user", "content": payload.prompt }
                ],
                "stream": payload.stream,
                "max_tokens": 1024
            })
        }
        _ => {
            // Default to OpenAI style for other formats in test
            serde_json::json!({
                "model": payload.model,
                "messages": [
                    { "role": "user", "content": payload.prompt }
                ],
                "stream": payload.stream,
                "max_tokens": 1024
            })
        }
    };

    let body_bytes = serde_json::to_vec(&body).unwrap();
    let path = match source_fmt {
        crate::config::ApiFormat::Openai => "chat/completions",
        _ => "messages",
    };

    // If upstream_id is provided, we only use that upstream
    // Otherwise, we use the normal proxy logic
    if let Some(upstream_id) = payload.upstream_id {
        let config = state.config.read().await;
        let upstream = config.upstreams.iter().find(|u| u.id == upstream_id);

        if let Some(upstream) = upstream {
            let upstream = upstream.clone();
            drop(config); // Release lock

            let mut last_result = None;
            let mut all_attempts = Vec::new();
            let start_time = std::time::Instant::now();

            // Try all keys for the specified upstream
            let keys = if upstream.keys.is_empty() {
                vec![None]
            } else {
                upstream
                    .keys
                    .iter()
                    .map(|s| Some(s.as_str()))
                    .collect::<Vec<_>>()
            };

            for api_key in keys {
                let (result, attempt) = crate::proxy::try_upstream_key(
                    &upstream,
                    api_key,
                    path, // Use the path determined by source_fmt
                    &axum::http::Method::POST,
                    &headers,
                    "",
                    &Some(body.clone()),
                    &Some(body_bytes.clone()),
                    &source_fmt, // Use the selected source format
                    true,
                    payload.stream,
                    true,
                )
                .await;

                if let Some(a) = attempt {
                    all_attempts.push(a);
                }

                match result {
                    crate::proxy::AttemptResult::Success(resp) => {
                        // Record log for successful test request
                        let log = crate::config::RequestLog {
                            timestamp: chrono::Utc::now().to_rfc3339(),
                            method: "POST".to_string(),
                            path: format!("/admin/test-model ({})", upstream.id),
                            model: Some(payload.model.clone()),
                            upstream_id: Some(upstream.id.clone()),
                            status_code: resp.status().as_u16(),
                            duration_ms: Some(start_time.elapsed().as_millis() as u64),
                            error: None,
                            attempts: all_attempts,
                        };
                        log_store.add_log(log).await;
                        return resp;
                    }
                    crate::proxy::AttemptResult::RetryableError {
                        status,
                        body,
                        content_type,
                    } => {
                        last_result = Some((status, body, content_type));
                        // Continue to next key
                    }
                    crate::proxy::AttemptResult::FatalError => {
                        last_result = None;
                        break;
                    }
                }
            }

            // If we reached here, all keys failed or fatal error occurred
            let status_code = last_result.as_ref().map(|r| r.0).unwrap_or(502);
            let log = crate::config::RequestLog {
                timestamp: chrono::Utc::now().to_rfc3339(),
                method: "POST".to_string(),
                path: format!("/admin/test-model ({})", upstream.id),
                model: Some(payload.model.clone()),
                upstream_id: Some(upstream.id.clone()),
                status_code,
                duration_ms: Some(start_time.elapsed().as_millis() as u64),
                error: Some(format!("All keys failed for test request")),
                attempts: all_attempts,
            };
            log_store.add_log(log).await;

            if let Some((status, body, content_type)) = last_result {
                (
                    StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY),
                    [("content-type", content_type)],
                    body,
                )
                    .into_response()
            } else {
                (
                    StatusCode::BAD_GATEWAY,
                    "Fatal error during upstream attempt",
                )
                    .into_response()
            }
        } else {
            (StatusCode::NOT_FOUND, "Upstream not found").into_response()
        }
    } else {
        // Normal proxy logic via determined endpoint
        crate::proxy::proxy_request(
            path,
            axum::http::Method::POST,
            headers, // Pass original headers to preserve UA etc.
            "",
            Some(body_bytes),
            state,
            log_store,
        )
        .await
    }
}
