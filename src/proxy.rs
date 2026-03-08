//! Proxy with multi-upstream failover and circuit breaker.

use crate::config::{ApiFormat, AppState, LogStore, RequestLog, Upstream, UpstreamAttempt};
use crate::converters::{convert_stream_chunk, from_upstream, to_upstream};
use axum::{
    http::{HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response, sse::Event},
};
use dashmap::DashMap;
use futures::stream::StreamExt;
use reqwest::Client;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Circuit breaker state
struct CircuitBreaker {
    failures: u32,
    open_until: Instant,
}

lazy_static::lazy_static! {
    static ref CIRCUIT_BREAKER: DashMap<String, CircuitBreaker> = DashMap::new();
    static ref KEY_INDEX: DashMap<String, usize> = DashMap::new();
    static ref HTTP_CLIENT: Client = {
        let mut builder = Client::builder()
            .timeout(Duration::from_secs(120))
            .no_proxy();
        
        // 支持从环境变量读取代理配置
        if let Ok(proxy_url) = std::env::var("HTTPS_PROXY")
            .or_else(|_| std::env::var("https_proxy"))
        {
            if let Ok(proxy) = reqwest::Proxy::all(&proxy_url) {
                // 处理 NO_PROXY
                let mut proxy = proxy;
                if let Ok(no_proxy) = std::env::var("NO_PROXY")
                    .or_else(|_| std::env::var("no_proxy"))
                {
                    for host in no_proxy.split(',') {
                        let host = host.trim();
                        if !host.is_empty() {
                            proxy = proxy.no_proxy(reqwest::NoProxy::from_string(host));
                        }
                    }
                }
                builder = builder.proxy(proxy);
            }
        } else if let Ok(proxy_url) = std::env::var("HTTP_PROXY")
            .or_else(|_| std::env::var("http_proxy"))
        {
            if let Ok(proxy) = reqwest::Proxy::all(&proxy_url) {
                let mut proxy = proxy;
                if let Ok(no_proxy) = std::env::var("NO_PROXY")
                    .or_else(|_| std::env::var("no_proxy"))
                {
                    for host in no_proxy.split(',') {
                        let host = host.trim();
                        if !host.is_empty() {
                            proxy = proxy.no_proxy(reqwest::NoProxy::from_string(host));
                        }
                    }
                }
                builder = builder.proxy(proxy);
            }
        }
        
        builder.build().expect("Failed to create HTTP client")
    };
}

/// Status codes that trigger failover
const FAILOVER_STATUS_CODES: [u16; 10] = [401, 403, 429, 500, 502, 503, 504, 520, 522, 524];

/// Get available upstreams sorted by priority, excluding circuit-broken ones
pub fn get_available_upstreams(upstreams: &[Upstream]) -> Vec<Upstream> {
    let now = Instant::now();
    let mut available: Vec<Upstream> = upstreams
        .iter()
        .filter(|u| {
            if !u.enabled {
                return false;
            }
            if let Some(cb) = CIRCUIT_BREAKER.get(&u.id) {
                if cb.open_until > now {
                    return false;
                }
            }
            true
        })
        .cloned()
        .collect();

    available.sort_by_key(|u| u.priority);
    available
}

/// Record a failure and open circuit if threshold reached
fn record_failure(upstream_id: &str) {
    let mut entry = CIRCUIT_BREAKER.entry(upstream_id.to_string()).or_insert(CircuitBreaker {
        failures: 0,
        open_until: Instant::now(),
    });

    entry.failures += 1;
    if entry.failures >= 3 {
        entry.open_until = Instant::now() + Duration::from_secs(60);
        entry.failures = 0;
    }
}

/// Clear circuit breaker state on success
fn record_success(upstream_id: &str) {
    CIRCUIT_BREAKER.remove(upstream_id);
}

/// Clear all circuit breaker state (used when config is updated)
pub fn clear_circuit_breaker() {
    CIRCUIT_BREAKER.clear();
}

/// Apply model mapping from upstream config
fn apply_model_map(body: &mut serde_json::Value, upstream: &Upstream) {
    if let Some(model) = body.get("model").and_then(|m| m.as_str()) {
        if let Some(mapped) = upstream.model_map.get(model) {
            body["model"] = serde_json::Value::String(mapped.clone());
        }
    }
}

/// Build upstream URL
fn build_upstream_url(base_url: &str, path: &str, endpoint: Option<&str>, query: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let default_path = format!("/{}", path.trim_start_matches('/'));
    let target_path = endpoint.unwrap_or(&default_path);
    let mut url = format!("{}{}", base, target_path);
    if !query.is_empty() {
        url.push('?');
        url.push_str(query);
    }
    url
}

/// Build request headers for upstream
fn build_upstream_headers(
    headers: &HeaderMap,
    api_key: Option<&str>,
    api_format: &ApiFormat,
) -> reqwest::header::HeaderMap {
    let mut req_headers = reqwest::header::HeaderMap::new();

    for (key, value) in headers.iter() {
        let key_lower = key.as_str().to_lowercase();
        if [
            "host",
            "content-length",
            "authorization",
            "x-api-key",
            "cf-connecting-ip",
            "cf-ray",
            "cf-ipcountry",
            "x-forwarded-proto",
            "x-forwarded-host",
            "x-real-ip",
        ]
        .contains(&key_lower.as_str())
        {
            continue;
        }
        req_headers.insert(key.clone(), value.clone());
    }

    if let Some(key) = api_key {
        req_headers.insert(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {}", key).parse().unwrap(),
        );
        req_headers.insert("x-api-key", key.parse().unwrap());
    }

    if *api_format == ApiFormat::Anthropic {
        req_headers.insert(
            "anthropic-version",
            headers
                .get("anthropic-version")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("2023-06-01")
                .parse()
                .unwrap(),
        );
    }

    req_headers
}

/// Result of a single upstream attempt
enum AttemptResult {
    Success(Response),
    RetryableError { status: u16, body: Vec<u8>, content_type: String },
    FatalError,
}

/// Try a single key for an upstream
async fn try_upstream_key(
    upstream: &Upstream,
    api_key: Option<&str>,
    path: &str,
    method: &Method,
    headers: &HeaderMap,
    query: &str,
    body_json: &Option<serde_json::Value>,
    body_bytes: &Option<Vec<u8>>,
    should_convert: bool,
    is_stream: bool,
    debug_mode: bool,
) -> (AttemptResult, Option<UpstreamAttempt>) {
    let upstream_id = upstream.id.clone();
    let api_format = upstream.api_format.clone();

    let (request_body, endpoint): (Option<serde_json::Value>, Option<String>) = if should_convert {
        let mut body = body_json.clone().unwrap();
        apply_model_map(&mut body, upstream);
        let (converted, endpoint) = to_upstream(&body, &api_format);
        (Some(converted), Some(endpoint.to_string()))
    } else if let Some(ref body) = body_json {
        let mut body = body.clone();
        apply_model_map(&mut body, upstream);
        (Some(body), None)
    } else {
        (None, None)
    };

    let endpoint_str = endpoint.as_deref();
    let url = build_upstream_url(&upstream.base_url, path, endpoint_str, query);
    let req_headers = build_upstream_headers(headers, api_key, &api_format);

    if debug_mode {
        tracing::info!(
            "[PROXY] upstream_request: upstream_id={}, key_index={}, url={}",
            upstream_id,
            api_key.map(|k| if k.len() > 10 { &k[..10] } else { k }).unwrap_or("none"),
            url
        );
    }

    // Build request
    let mut request_builder = HTTP_CLIENT.request(method.clone(), &url).headers(req_headers);

    if let Some(ref body) = request_body {
        request_builder = request_builder.json(body);
    } else if let Some(ref bytes) = body_bytes {
        request_builder = request_builder.body(bytes.clone());
    }

    let timeout = if is_stream { 60 } else { 20 };
    request_builder = request_builder.timeout(Duration::from_secs(timeout));

    match request_builder.send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();

            if FAILOVER_STATUS_CODES.contains(&status) {
                let content_type = resp
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("application/json")
                    .to_string();

                let response_body = match resp.bytes().await {
                    Ok(bytes) => {
                        let body_str = String::from_utf8_lossy(&bytes);
                        let truncated = if body_str.len() > 500 {
                            format!("{}... (truncated)", &body_str[..500])
                        } else {
                            body_str.to_string()
                        };
                        let attempt = UpstreamAttempt {
                            upstream_id: upstream_id.clone(),
                            url: Some(url.clone()),
                            status_code: status,
                            error: Some(format!("HTTP error {}", status)),
                            response_body: Some(truncated),
                        };
                        (AttemptResult::RetryableError { status, body: bytes.to_vec(), content_type }, Some(attempt))
                    }
                    Err(e) => {
                        let attempt = UpstreamAttempt {
                            upstream_id: upstream_id.clone(),
                            url: Some(url.clone()),
                            status_code: status,
                            error: Some(format!("HTTP error {}, failed to read body: {}", status, e)),
                            response_body: None,
                        };
                        (AttemptResult::RetryableError { status, body: vec![], content_type }, Some(attempt))
                    }
                };
                response_body
            } else {
                // Success or non-retryable error
                record_success(&upstream_id);

                // Handle streaming response
                if is_stream
                    || resp
                        .headers()
                        .get(reqwest::header::CONTENT_TYPE)
                        .and_then(|v| v.to_str().ok())
                        .map(|ct| ct.contains("text/event-stream"))
                        .unwrap_or(false)
                {
                    let attempt = UpstreamAttempt {
                        upstream_id: upstream_id.clone(),
                        url: Some(url.clone()),
                        status_code: status,
                        error: None,
                        response_body: None,
                    };
                    let convert_format = if should_convert { api_format } else { ApiFormat::Anthropic };
                    let stream = resp.bytes_stream().filter_map(move |chunk_result| {
                        let format = convert_format.clone();
                        async move {
                            match chunk_result {
                                Ok(chunk) => {
                                    let chunk_str = String::from_utf8_lossy(&chunk);
                                    let converted = convert_stream_chunk(&chunk_str, &format);
                                    converted.map(|s| Ok::<_, std::convert::Infallible>(Event::default().data(s)))
                                }
                                Err(e) => Some(Ok(Event::default().data(format!("Error: {}", e)))),
                            }
                        }
                    });
                    (AttemptResult::Success(axum::response::sse::Sse::new(stream).into_response()), Some(attempt))
                } else {
                    let content_type = resp
                        .headers()
                        .get(reqwest::header::CONTENT_TYPE)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("application/json")
                        .to_string();

                    match resp.bytes().await {
                        Ok(response_bytes) => {
                            let (response_content, media_type) =
                                if should_convert && content_type.contains("application/json") {
                                    if let Ok(parsed) = serde_json::from_slice::<serde_json::Value>(&response_bytes) {
                                        let converted = from_upstream(&parsed, &api_format);
                                        (
                                            serde_json::to_vec(&converted).unwrap_or_else(|_| response_bytes.to_vec()),
                                            "application/json",
                                        )
                                    } else {
                                        (
                                            response_bytes.to_vec(),
                                            content_type.split(';').next().unwrap_or("application/json"),
                                        )
                                    }
                                } else {
                                    (
                                        response_bytes.to_vec(),
                                        content_type.split(';').next().unwrap_or("application/json"),
                                    )
                                };

                            let attempt = UpstreamAttempt {
                                upstream_id: upstream_id.clone(),
                                url: Some(url.clone()),
                                status_code: status,
                                error: None,
                                response_body: None,
                            };

                            let response = (
                                StatusCode::from_u16(status).unwrap_or(StatusCode::OK),
                                [("content-type", media_type)],
                                response_content,
                            )
                                .into_response();

                            (AttemptResult::Success(response), Some(attempt))
                        }
                        Err(e) => {
                            let attempt = UpstreamAttempt {
                                upstream_id: upstream_id.clone(),
                                url: Some(url),
                                status_code: 502,
                                error: Some(format!("Failed to read response: {}", e)),
                                response_body: None,
                            };
                            (AttemptResult::FatalError, Some(attempt))
                        }
                    }
                }
            }
        }
        Err(e) => {
            let error_msg = if e.is_timeout() {
                format!("Request timeout: {}", e)
            } else if e.is_connect() {
                format!("Connection failed: {}", e)
            } else {
                format!("Request error: {}", e)
            };
            let attempt = UpstreamAttempt {
                upstream_id: upstream_id.clone(),
                url: Some(url),
                status_code: 502,
                error: Some(error_msg),
                response_body: None,
            };
            (AttemptResult::FatalError, Some(attempt))
        }
    }
}

/// Proxy request with multi-upstream failover
pub async fn proxy_request(
    path: &str,
    method: Method,
    headers: HeaderMap,
    query: &str,
    body_bytes: Option<Vec<u8>>,
    state: Arc<AppState>,
    log_store: Arc<LogStore>,
) -> Response {
    let start_time = Instant::now();
    let config = state.config.read().await;
    let available = get_available_upstreams(&config.upstreams);
    let debug_mode = config.debug_mode;

    if available.is_empty() {
        let log = RequestLog {
            timestamp: chrono::Utc::now().to_rfc3339(),
            method: method.to_string(),
            path: format!("/v1/{}", path),
            model: None,
            upstream_id: None,
            status_code: 503,
            duration_ms: Some(start_time.elapsed().as_millis() as u64),
            error: Some("No available upstreams configured".to_string()),
            attempts: vec![],
        };
        log_store.add_log(log).await;

        return (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({
                "error": { "message": "No available upstreams" }
            })),
        )
            .into_response();
    }

    // Parse request body
    let body_json: Option<serde_json::Value> = body_bytes
        .as_ref()
        .and_then(|b| serde_json::from_slice(b).ok());

    let model: Option<String> = body_json.as_ref().and_then(|b| b.get("model")?.as_str().map(|s| s.to_string()));
    let should_convert = method == Method::POST && path.trim_matches('/') == "messages" && body_json.is_some();
    let is_stream = body_json
        .as_ref()
        .and_then(|b| b.get("stream")?.as_bool())
        .unwrap_or(false);

    if debug_mode {
        tracing::info!(
            "[PROXY] request: method={}, path=/v1/{}, available={:?}, convert={}, stream={}",
            method,
            path,
            available.iter().map(|u| u.id.as_str()).collect::<Vec<_>>(),
            should_convert,
            is_stream
        );
    }

    let mut attempts: Vec<UpstreamAttempt> = Vec::new();
    let mut last_upstream_failure: Option<(String, u16, Vec<u8>, String)> = None;
    let available_count = available.len();

    // Iterate through upstreams
    for upstream in &available {
        let keys = if upstream.keys.is_empty() {
            vec![None]
        } else {
            upstream.keys.iter().map(Some).collect::<Vec<_>>()
        };

        // Try each key for this upstream
        for api_key in keys {
            let (result, attempt) = try_upstream_key(
                &upstream,
                api_key.map(|s| s.as_str()),
                path,
                &method,
                &headers,
                query,
                &body_json,
                &body_bytes,
                should_convert,
                is_stream,
                debug_mode,
            )
            .await;

            if let Some(a) = attempt {
                attempts.push(a);
            }

            match result {
                AttemptResult::Success(response) => {
                    // Log success
                    let log = RequestLog {
                        timestamp: chrono::Utc::now().to_rfc3339(),
                        method: method.to_string(),
                        path: format!("/v1/{}", path),
                        model: model.clone(),
                        upstream_id: Some(upstream.id.clone()),
                        status_code: response.status().as_u16(),
                        duration_ms: Some(start_time.elapsed().as_millis() as u64),
                        error: None,
                        attempts,
                    };
                    log_store.add_log(log).await;
                    return response;
                }
                AttemptResult::RetryableError { status, body, content_type } => {
                    record_failure(&upstream.id);
                    last_upstream_failure = Some((upstream.id.clone(), status, body, content_type));
                    // Continue to next key
                    if debug_mode {
                        tracing::warn!(
                            "[PROXY] key failed, trying next key: upstream_id={}, status={}",
                            upstream.id,
                            status
                        );
                    }
                    continue;
                }
                AttemptResult::FatalError => {
                    record_failure(&upstream.id);
                    // Continue to next key
                    continue;
                }
            }
        }

        // All keys for this upstream failed, move to next upstream
        if debug_mode {
            tracing::warn!(
                "[PROXY] all keys failed for upstream, trying next upstream: upstream_id={}",
                upstream.id
            );
        }
    }

    // All upstreams and keys failed
    let error_summary = format!(
        "All {} attempt(s) failed across {} upstream(s)",
        attempts.len(),
        available_count
    );

    let log = RequestLog {
        timestamp: chrono::Utc::now().to_rfc3339(),
        method: method.to_string(),
        path: format!("/v1/{}", path),
        model,
        upstream_id: None,
        status_code: last_upstream_failure.as_ref().map(|(_, s, _, _)| *s).unwrap_or(502),
        duration_ms: Some(start_time.elapsed().as_millis() as u64),
        error: Some(error_summary),
        attempts,
    };
    log_store.add_log(log).await;

    if let Some((upstream_id, status, body, content_type)) = last_upstream_failure {
        return (
            StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY),
            [
                ("content-type", content_type.as_str()),
                ("X-APIRouter-All-Upstreams-Failed", "1"),
                ("X-APIRouter-Upstream-Id", Box::leak(upstream_id.into_boxed_str()) as &'static str),
            ],
            body,
        )
            .into_response();
    }

    (
        StatusCode::BAD_GATEWAY,
        axum::Json(serde_json::json!({
            "error": { "message": "All upstreams and keys failed" }
        })),
    )
        .into_response()
}