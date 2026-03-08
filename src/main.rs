//! API Router - Multi-upstream proxy with failover.

mod admin;
mod config;
mod converters;
mod proxy;

use axum::{
    body::Body,
    extract::State,
    http::{header, HeaderMap, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use clap::Parser;
use std::{net::SocketAddr, path::PathBuf, sync::Arc};
use tower_http::{cors::CorsLayer, services::ServeDir};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::config::{AppState, LogStore};
use crate::proxy::proxy_request;

/// API Router - Multi-upstream proxy with failover
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Port to listen on
    #[arg(short, long, default_value = "1999")]
    port: u16,

    /// Host to bind to
    #[arg(long, default_value = "0.0.0.0")]
    host: String,

    /// Data directory for configuration and logs
    #[arg(long, default_value = "./data")]
    data_dir: PathBuf,
}

/// Root response
#[derive(serde::Serialize)]
struct RootResponse {
    status: &'static str,
    version: &'static str,
    features: Vec<&'static str>,
}

/// Application state wrapper
#[derive(Clone)]
struct AppServices {
    state: Arc<AppState>,
    log_store: Arc<LogStore>,
}

#[tokio::main]
async fn main() {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let args = Args::parse();
    let addr: SocketAddr = format!("{}:{}", args.host, args.port).parse().expect("Invalid address");

    // Initialize state
    let services = AppServices {
        state: AppState::new(&args.data_dir),
        log_store: LogStore::new(&args.data_dir),
    };

    tracing::info!("Starting API Router on {}", addr);
    tracing::info!("Admin UI: http://{}/admin-ui", addr);
    tracing::info!("API endpoint: http://{}/v1/messages", addr);

    // Build router
    let app = Router::new()
        // Root
        .route("/", get(root))
        // Admin routes
        .route("/admin/verify", post(admin_verify))
        .route("/admin/config", get(admin_config_get).post(admin_config_update))
        .route("/admin/client-keys", get(admin_client_keys_get).post(admin_client_keys_update))
        .route("/admin/generate-key", post(admin_generate_key))
        .route("/admin/logs", get(admin_logs_get).delete(admin_logs_clear))
        // Admin UI
        .route("/admin-ui", get(admin_ui))
        // Proxy routes - using catch-all path
        .fallback(proxy_handler)
        // Static files
        .nest_service("/static", ServeDir::new("./frontend"))
        // Middleware
        .layer(CorsLayer::permissive())
        .with_state(services);

    // Start server
    let listener = tokio::net::TcpListener::bind(addr).await.expect("Failed to bind");
    if let Err(e) = axum::serve(listener, app).await {
        tracing::error!("Server error: {}", e);
    }
}

/// Root endpoint
async fn root() -> Json<RootResponse> {
    Json(RootResponse {
        status: "ok",
        version: "v2.0-rust",
        features: vec![
            "multi-upstream",
            "failover",
            "circuit-breaker",
            "format-conversion",
            "local-storage",
        ],
    })
}

/// Admin UI endpoint
async fn admin_ui() -> Response {
    match tokio::fs::read_to_string("./frontend/index.html").await {
        Ok(content) => ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], content).into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "Admin UI not found").into_response(),
    }
}

// Admin handlers

async fn admin_verify(State(services): State<AppServices>, headers: HeaderMap) -> Response {
    admin::verify_password_handler(services.state, headers).await
}

async fn admin_config_get(State(services): State<AppServices>, headers: HeaderMap) -> Response {
    admin::get_config(services.state, headers).await
}

async fn admin_config_update(
    State(services): State<AppServices>,
    headers: HeaderMap,
    Json(payload): Json<admin::ConfigUpdate>,
) -> Response {
    admin::update_config(services.state, headers, Json(payload)).await
}

async fn admin_client_keys_get(State(services): State<AppServices>, headers: HeaderMap) -> Response {
    admin::get_client_keys(services.state, headers).await
}

async fn admin_client_keys_update(
    State(services): State<AppServices>,
    headers: HeaderMap,
    Json(payload): Json<admin::ClientKeysUpdate>,
) -> Response {
    admin::update_client_keys(services.state, headers, Json(payload)).await
}

async fn admin_generate_key(State(services): State<AppServices>, headers: HeaderMap) -> Response {
    admin::generate_key(services.state, headers).await
}

async fn admin_logs_get(State(services): State<AppServices>, headers: HeaderMap) -> Response {
    admin::get_logs(services.state, services.log_store, headers).await
}

async fn admin_logs_clear(State(services): State<AppServices>, headers: HeaderMap) -> Response {
    admin::clear_logs(services.state, services.log_store, headers).await
}

/// Proxy handler for /v1/* routes (fallback handler)
async fn proxy_handler(
    uri: Uri,
    method: Method,
    headers: HeaderMap,
    State(services): State<AppServices>,
    body: Body,
) -> Response {
    let path = uri.path();

    // Only handle /v1/* paths
    if !path.starts_with("/v1/") {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    }

    let proxy_path = path.strip_prefix("/v1/").unwrap_or("");

    // Check access control
    let access_enabled = {
        let config = services.state.config.read().await;
        config.access_control_enabled
    };

    if access_enabled {
        let api_key = headers
            .get("x-api-key")
            .and_then(|v| v.to_str().ok())
            .or_else(|| {
                headers
                    .get(header::AUTHORIZATION)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|auth| auth.strip_prefix("Bearer "))
            });

        let valid = {
            let config = services.state.config.read().await;
            api_key
                .map(|key| config.client_keys.iter().any(|k| k.key == key && k.enabled))
                .unwrap_or(false)
        };

        if !valid {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "error": { "message": "Invalid or missing API key", "type": "authentication_error" }
                })),
            )
                .into_response();
        }
    }

    // Extract query string
    let query = uri.query().unwrap_or("");

    // Get body bytes
    let body_bytes = match axum::body::to_bytes(body, 1024 * 1024 * 10).await {
        Ok(bytes) => Some(bytes.to_vec()),
        Err(_) => None,
    };

    proxy_request(
        proxy_path,
        method,
        headers,
        query,
        body_bytes,
        services.state,
        services.log_store,
    )
    .await
}
