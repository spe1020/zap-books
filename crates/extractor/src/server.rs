//! The HTTP service: `GET /health` and `POST /extract`, on 127.0.0.1 only.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Multipart, State};
use axum::http::{header, HeaderValue, Method, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use ledger_core::Currency;
use serde_json::json;
use tokio::net::TcpListener;
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::config::Config;
use crate::error::ExtractError;
use crate::extract::{Category, Extractor};
use crate::files::{detect, MAX_FILE_BYTES};
use crate::ollama::Ollama;
use crate::pdf;

/// The most categories accepted with one request.
const MAX_CATEGORIES: usize = 500;

struct AppState {
    extractor: Extractor,
    port: u16,
    origins: Vec<String>,
    timeout: Duration,
}

impl IntoResponse for ExtractError {
    fn into_response(self) -> Response {
        let status =
            StatusCode::from_u16(self.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        (status, Json(json!({ "error": self.to_string() }))).into_response()
    }
}

fn reject(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

/// Blocks DNS rebinding and other sites: the `Host` must be this service on
/// 127.0.0.1 or localhost, and any `Origin` must be one of the app's.
async fn guard(State(state): State<Arc<AppState>>, request: Request<Body>, next: Next) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default();
    let port = state.port;
    if host != format!("127.0.0.1:{port}") && host != format!("localhost:{port}") {
        tracing::warn!("rejected request with an unexpected Host header");
        return reject(
            StatusCode::FORBIDDEN,
            "This service only answers on this computer.",
        );
    }
    if let Some(origin) = request.headers().get(header::ORIGIN) {
        let allowed = origin
            .to_str()
            .is_ok_and(|o| state.origins.iter().any(|a| a == o));
        if !allowed {
            tracing::warn!("rejected request from a disallowed origin");
            return reject(
                StatusCode::FORBIDDEN,
                "This origin may not use receipt reading.",
            );
        }
    }
    next.run(request).await
}

async fn health(State(state): State<Arc<AppState>>) -> Response {
    let ready = state
        .extractor
        .ollama
        .has_model(&state.extractor.model)
        .await
        .unwrap_or(false);
    Json(json!({
        "ready": ready,
        "model": state.extractor.model,
        "poppler": pdf::poppler_available().await,
    }))
    .into_response()
}

fn multipart_error(e: &axum::extract::multipart::MultipartError) -> ExtractError {
    if e.status() == StatusCode::PAYLOAD_TOO_LARGE {
        ExtractError::TooLarge
    } else {
        ExtractError::BadRequest("the upload could not be read")
    }
}

async fn extract(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<Response, ExtractError> {
    let started = Instant::now();
    let mut file: Option<Vec<u8>> = None;
    let mut categories: Vec<Category> = Vec::new();
    let mut currencies: Vec<Currency> = Currency::ALL.to_vec();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| multipart_error(&e))?
    {
        let name = field.name().unwrap_or_default().to_string();
        let bytes = field.bytes().await.map_err(|e| multipart_error(&e))?;
        match name.as_str() {
            "file" => {
                if bytes.len() > MAX_FILE_BYTES {
                    return Err(ExtractError::TooLarge);
                }
                file = Some(bytes.to_vec());
            }
            "categories" => {
                categories = serde_json::from_slice(&bytes)
                    .map_err(|_| ExtractError::BadRequest("categories must be a JSON list"))?;
                if categories.len() > MAX_CATEGORIES {
                    return Err(ExtractError::BadRequest("too many categories"));
                }
            }
            "currencies" => {
                let codes: Vec<String> = serde_json::from_slice(&bytes)
                    .map_err(|_| ExtractError::BadRequest("currencies must be a JSON list"))?;
                currencies = codes
                    .iter()
                    .filter_map(|c| Currency::from_code(c))
                    .collect();
            }
            _ => {}
        }
    }
    let file = file.ok_or(ExtractError::MissingFile)?;
    detect(&file)?;
    // A receipt that timed out earlier may still hold the model.
    state
        .extractor
        .ollama
        .wait_until_idle(&state.extractor.model)
        .await?;
    let result = tokio::time::timeout(
        state.timeout,
        state.extractor.extract(&file, &categories, &currencies),
    )
    .await
    .unwrap_or(Err(ExtractError::Timeout));
    if let Err(e) = &result {
        tracing::info!(
            status = e.status(),
            duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            "extraction failed"
        );
    }
    Ok(Json(result?).into_response())
}

/// The router, with its guard, CORS and body limit.
fn router(state: Arc<AppState>) -> Router {
    let origins: Vec<HeaderValue> = state
        .origins
        .iter()
        .filter_map(|o| HeaderValue::from_str(o).ok())
        .collect();
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods([Method::GET, Method::POST]);
    Router::new()
        .route("/health", get(health))
        .route("/extract", post(extract))
        // Room for the file plus the small JSON fields.
        .layer(DefaultBodyLimit::max(MAX_FILE_BYTES + 1024 * 1024))
        .layer(cors)
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .with_state(state)
}

/// Only 127.0.0.1 is allowed: the service must not be reachable from other
/// computers.
pub fn check_bind_address(addr: &SocketAddr) -> Result<(), String> {
    if addr.ip() == std::net::IpAddr::V4(Ipv4Addr::LOCALHOST) {
        Ok(())
    } else {
        Err(format!(
            "Refusing to listen on {addr}: the extractor only runs on 127.0.0.1."
        ))
    }
}

/// A bound, not yet running service.
pub struct Service {
    listener: TcpListener,
    router: Router,
    pub addr: SocketAddr,
}

impl Service {
    /// Binds `addr` (which must be 127.0.0.1). Port 0 picks a free port.
    pub async fn bind(config: &Config, addr: SocketAddr) -> Result<Service, String> {
        check_bind_address(&addr)?;
        let listener = TcpListener::bind(addr)
            .await
            .map_err(|e| format!("Could not listen on {addr}: {e}"))?;
        let addr = listener
            .local_addr()
            .map_err(|e| format!("Could not read the listening address: {e}"))?;
        let ollama = Ollama::new(&config.ollama, config.timeout, config.max_tokens)
            .map_err(|e| e.to_string())?;
        let state = Arc::new(AppState {
            extractor: Extractor {
                ollama,
                model: config.model.clone(),
            },
            port: addr.port(),
            origins: config.allowed_origins.clone(),
            timeout: config.timeout,
        });
        Ok(Service {
            listener,
            router: router(state),
            addr,
        })
    }

    /// Serves until the process ends.
    pub async fn run(self) -> std::io::Result<()> {
        axum::serve(self.listener, self.router).await
    }
}
