//! The HTTPS API: signed proposals in, signed history out.
//!
//! - `POST /v1/proposals`: a proposal event as the body. Always answers 200
//!   with the acknowledgment (accepted or rejected); 400 for anything that
//!   isn't a validly signed proposal.
//! - `GET /v1/businesses/{id}/acceptances?after=&limit=`: the chain.
//! - `GET /v1/businesses/{id}/pending`: owner-level changes waiting out
//!   their 48 hours, for owners and the recovery key.
//! - `GET /v1/epoch-keys?business=`: my epoch key deliveries.
//! - `GET /v1/rejections?after=`: my rejections.
//! - `GET /health`.
//!
//! Reads need NIP-98 auth. Every request is rate limited per public key.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, OriginalUri, Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use nostr::nips::nip98::HttpMethod;
use nostr::prelude::{Event, PublicKey, Timestamp};
use nostr::types::url::Url;
use protocol::{parse_proposal, BusinessId, MAX_EVENT_BYTES};
use serde::Deserialize;

use crate::ledger::{Ledger, LedgerError};
use crate::nip98;
use crate::ratelimit::RateLimit;

/// A proposal is one event of at most [`MAX_EVENT_BYTES`]; the body limit
/// leaves room for whitespace around it.
pub const MAX_BODY_BYTES: usize = MAX_EVENT_BYTES + 4096;
const MAX_PAGE: u32 = 500;

pub struct AppState {
    pub ledger: Arc<Ledger>,
    /// The service's public base URL, such as `https://ledger.example`. NIP-98
    /// events must name it exactly.
    pub public_url: String,
    pub writes: RateLimit,
    pub reads: RateLimit,
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/v1/proposals", post(submit))
        .route("/v1/businesses/{id}/acceptances", get(acceptances))
        .route("/v1/businesses/{id}/pending", get(pending))
        .route("/v1/epoch-keys", get(epoch_keys))
        .route("/v1/rejections", get(rejections))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(state)
}

fn error(status: StatusCode, message: &str) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::json!({ "error": message }).to_string(),
    )
        .into_response()
}

fn json(body: String) -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response()
}

fn failure(e: LedgerError) -> Response {
    match e {
        LedgerError::Invalid(e) => error(StatusCode::BAD_REQUEST, &e.to_string()),
        other => {
            tracing::error!(error = %other, "request failed");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "The service failed; nothing was stored.",
            )
        }
    }
}

async fn submit(State(state): State<Arc<AppState>>, body: Bytes) -> Response {
    let Ok(event) = Event::from_json(&body[..]) else {
        return error(StatusCode::BAD_REQUEST, "The body must be a Nostr event.");
    };
    // Check the signature before charging the author's rate limit, so no
    // one can spend another key's allowance.
    let header = match parse_proposal(&event) {
        Ok(h) => h,
        Err(e) => return error(StatusCode::BAD_REQUEST, &e.to_string()),
    };
    if !state
        .writes
        .allow(&header.author, state.ledger.clock().now_millis())
    {
        return error(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many requests. Wait and try again.",
        );
    }
    match state.ledger.submit(&event).await {
        Ok(ack) => json(ack.text),
        Err(e) => failure(e),
    }
}

/// Checks NIP-98 auth for a GET of `uri`, refusing replays.
async fn authorize(
    state: &AppState,
    headers: &HeaderMap,
    uri: &axum::http::Uri,
) -> Result<PublicKey, Box<Response>> {
    let unauthorized = || {
        Box::new(error(
            StatusCode::UNAUTHORIZED,
            "Sign this request with NIP-98.",
        ))
    };
    let header = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(unauthorized)?;
    let path = uri.path_and_query().map_or("/", |p| p.as_str());
    let url = Url::parse(&format!("{}{path}", state.public_url.trim_end_matches('/')))
        .map_err(|_| unauthorized())?;
    let now = state.ledger.clock().now_secs();
    let auth = nip98::check(header, &url, HttpMethod::GET, Timestamp::from_secs(now))
        .ok_or_else(unauthorized)?;
    if !state
        .reads
        .allow(&auth.pubkey, state.ledger.clock().now_millis())
    {
        return Err(Box::new(error(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many requests. Wait and try again.",
        )));
    }
    match state
        .ledger
        .remember_auth(&auth.event_id.to_hex(), now)
        .await
    {
        Ok(true) => Ok(auth.pubkey),
        Ok(false) => Err(Box::new(error(
            StatusCode::UNAUTHORIZED,
            "This authorization was already used. Sign a new one.",
        ))),
        Err(e) => Err(Box::new(failure(e))),
    }
}

#[derive(Deserialize)]
struct PageQuery {
    after: Option<u64>,
    limit: Option<u32>,
}

async fn acceptances(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<PageQuery>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    let reader = match authorize(&state, &headers, &uri).await {
        Ok(k) => k,
        Err(r) => return *r,
    };
    let Ok(business) = id.parse::<BusinessId>() else {
        return error(StatusCode::NOT_FOUND, "No such business.");
    };
    let limit = q.limit.unwrap_or(100).clamp(1, MAX_PAGE);
    match state
        .ledger
        .acceptances(&business, &reader, q.after, limit)
        .await
    {
        Ok(Some(page)) => json(format!(
            r#"{{"head":{},"acceptances":[{}]}}"#,
            page.head,
            page.events.join(",")
        )),
        // The same answer whether the business is missing or closed to
        // this reader, so the response reveals nothing about it.
        Ok(None) => error(StatusCode::NOT_FOUND, "No such business."),
        Err(e) => failure(e),
    }
}

async fn pending(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    let reader = match authorize(&state, &headers, &uri).await {
        Ok(k) => k,
        Err(r) => return *r,
    };
    let Ok(business) = id.parse::<BusinessId>() else {
        return error(StatusCode::NOT_FOUND, "No such business.");
    };
    match state.ledger.pending(&business, &reader).await {
        Ok(Some(pending)) => {
            match serde_json::to_string(&serde_json::json!({ "pending": pending })) {
                Ok(body) => json(body),
                Err(e) => failure(LedgerError::Internal(e.to_string())),
            }
        }
        Ok(None) => error(StatusCode::NOT_FOUND, "No such business."),
        Err(e) => failure(e),
    }
}

#[derive(Deserialize)]
struct KeysQuery {
    business: Option<String>,
}

async fn epoch_keys(
    State(state): State<Arc<AppState>>,
    Query(q): Query<KeysQuery>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    let member = match authorize(&state, &headers, &uri).await {
        Ok(k) => k,
        Err(r) => return *r,
    };
    let business = match q
        .business
        .as_deref()
        .map(str::parse::<BusinessId>)
        .transpose()
    {
        Ok(b) => b,
        Err(_) => return error(StatusCode::BAD_REQUEST, "Invalid business id."),
    };
    match state.ledger.deliveries(&member, business.as_ref()).await {
        Ok(events) => json(format!(r#"{{"deliveries":[{}]}}"#, events.join(","))),
        Err(e) => failure(e),
    }
}

#[derive(Deserialize)]
struct RejectionsQuery {
    after: Option<i64>,
}

async fn rejections(
    State(state): State<Arc<AppState>>,
    Query(q): Query<RejectionsQuery>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    let author = match authorize(&state, &headers, &uri).await {
        Ok(k) => k,
        Err(r) => return *r,
    };
    match state.ledger.rejections(&author, q.after, MAX_PAGE).await {
        Ok((events, cursor)) => json(format!(
            r#"{{"cursor":{},"rejections":[{}]}}"#,
            cursor.map_or("null".to_string(), |c| c.to_string()),
            events.join(",")
        )),
        Err(e) => failure(e),
    }
}
