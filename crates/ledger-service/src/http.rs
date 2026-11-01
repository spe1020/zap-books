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
//! - `GET /v1/recovery`: the businesses I am the recovery key of, with their
//!   owners and pending changes.
//! - `POST /v1/session`: a read session (see [`crate::session`]).
//! - `GET /v1/service`: the key new businesses must name.
//! - `PUT /v1/blobs/{hash}?business=`: upload an encrypted receipt.
//! - `GET /v1/blobs/{hash}?business=`: download one an accepted record
//!   refers to.
//! - `GET /health`.
//!
//! Reads need NIP-98 auth, or `Authorization: Bearer <token>` with a token
//! from `POST /v1/session`, which itself needs NIP-98. Either way, each
//! read checks the reader's membership. Blobs take a session token only.
//! Before anything is parsed or verified, every request passes three cheap
//! checks: its declared body size, a limit on all requests together, and a
//! limit per client address. After the signature, it is also rate limited
//! per public key.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::{ConnectInfo, DefaultBodyLimit, OriginalUri, Path, Query, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::Router;
use nostr::nips::nip98::HttpMethod;
use nostr::prelude::{Event, PublicKey, Timestamp};
use nostr::types::url::Url;
use protocol::{parse_proposal, BusinessId, MAX_EVENT_BYTES};
use serde::Deserialize;
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::blobs::{self, BlobError, MAX_BLOB_BYTES};
use crate::ledger::{Ledger, LedgerError};
use crate::nip98;
use crate::ratelimit::RateLimit;
use crate::session::{self, SESSION_SECS};

/// A proposal is one event of at most [`MAX_EVENT_BYTES`]; the body limit
/// leaves room for whitespace around it.
pub const MAX_BODY_BYTES: usize = MAX_EVENT_BYTES + 4096;
const MAX_PAGE: u32 = 500;

pub struct AppState {
    pub ledger: Arc<Ledger>,
    /// The service's public base URL, such as `https://ledger.example`. NIP-98
    /// events must name it exactly.
    pub public_url: String,
    /// Per public key, after the signature is checked.
    pub writes: RateLimit<PublicKey>,
    pub reads: RateLimit<PublicKey>,
    /// Every request together, before anything is parsed.
    pub global: RateLimit<()>,
    /// Per client address, before anything is parsed.
    pub per_ip: RateLimit<IpAddr>,
    /// Proxies whose `X-Forwarded-For` names the client. From any other
    /// peer the header is ignored, so a client can't choose its own bucket.
    pub trusted_proxies: Vec<IpAddr>,
    /// Browser origins allowed to call the API (CORS). Empty: none.
    pub allowed_origins: Vec<String>,
}

impl AppState {
    /// The state with the service's default limits.
    pub fn new(ledger: Arc<Ledger>, public_url: String) -> AppState {
        AppState {
            ledger,
            public_url,
            writes: RateLimit::new(60, 10),
            reads: RateLimit::new(120, 20),
            global: RateLimit::new(GLOBAL_BURST, GLOBAL_PER_SEC),
            per_ip: RateLimit::new(IP_BURST, IP_PER_SEC),
            trusted_proxies: Vec::new(),
            allowed_origins: Vec::new(),
        }
    }
}

/// All requests together: 400 at once, 200 a second after that.
pub const GLOBAL_BURST: u32 = 400;
pub const GLOBAL_PER_SEC: u32 = 200;
/// Per client address: 180 at once, 30 a second after that, enough for a
/// member's write and read buckets together.
pub const IP_BURST: u32 = 180;
pub const IP_PER_SEC: u32 = 30;

/// A plain browser origin, as a browser sends it in `Origin`: `http://` or
/// `https://`, a lowercase host name, IPv4 address or bracketed IPv6
/// address, and an optional port, with nothing after it. CORS compares
/// origins as exact strings, so anything else, `*` included, could never
/// match and only hides a typo.
pub fn is_plain_origin(origin: &str) -> bool {
    let Some(rest) = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
    else {
        return false;
    };
    let (host, port) = match rest.rsplit_once(':') {
        // A bracketed IPv6 address contains colons of its own.
        Some((host, port)) if !port.ends_with(']') => (host, Some(port)),
        _ => (rest, None),
    };
    let host_ok = if let Some(inner) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        !inner.is_empty()
            && inner
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c) || c == ':' || c == '.')
    } else {
        !host.is_empty()
            && !host.starts_with(['.', '-'])
            && !host.ends_with(['.', '-'])
            && host
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.')
    };
    let port_ok = port.is_none_or(|p| {
        !p.is_empty()
            && p.chars().all(|c| c.is_ascii_digit())
            && !(p.len() > 1 && p.starts_with('0'))
            && p.parse::<u16>().is_ok_and(|n| n > 0)
    });
    host_ok && port_ok
}

/// CORS for exactly `origins` (each must pass [`is_plain_origin`]): `GET`,
/// `POST` and `PUT`, with the `Authorization` and `Content-Type` headers,
/// and no credentials, since auth is a header, not a cookie.
pub fn cors(origins: &[String]) -> Result<CorsLayer, String> {
    let values = origins
        .iter()
        .map(|o| {
            if !is_plain_origin(o) {
                return Err(format!(
                    "--allowed-origin must be an origin like https://app.example, with no path or trailing slash; got {o}."
                ));
            }
            HeaderValue::from_str(o).map_err(|_| format!("--allowed-origin {o} is not valid."))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(CorsLayer::new()
        .allow_origin(AllowOrigin::list(values))
        .allow_methods([Method::GET, Method::POST, Method::PUT])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]))
}

/// The address a request comes from: its peer, or, when the peer is a
/// trusted proxy, the last address in `X-Forwarded-For` that isn't one.
pub fn client_ip(peer: IpAddr, headers: &HeaderMap, trusted: &[IpAddr]) -> IpAddr {
    if !trusted.contains(&peer) {
        return peer;
    }
    headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(str::trim)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|a| a.parse::<IpAddr>())
        .take_while(Result::is_ok)
        .filter_map(Result::ok)
        .find(|ip| !trusted.contains(ip))
        .unwrap_or(peer)
}

/// The cheap checks that run before any JSON is parsed or signature
/// verified: the declared body size, then the global and per-address
/// limits.
async fn limits(
    State(state): State<Arc<AppState>>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let declared = request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    // A blob upload may be one receipt; anything else, one event.
    let blob = request.method() == Method::PUT && request.uri().path().starts_with("/v1/blobs/");
    if blob && declared.is_some_and(|n| n > MAX_BLOB_BYTES) {
        return error(
            StatusCode::PAYLOAD_TOO_LARGE,
            &BlobError::TooLarge.to_string(),
        );
    }
    if !blob && declared.is_some_and(|n| n > MAX_BODY_BYTES as u64) {
        return error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "The request is larger than one event.",
        );
    }
    let now = state.ledger.clock().now_millis();
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED), |c| c.0.ip());
    let ip = client_ip(peer, request.headers(), &state.trusted_proxies);
    if !state.global.allow(&(), now) || !state.per_ip.allow(&ip, now) {
        return error(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many requests. Wait and try again.",
        );
    }
    next.run(request).await
}

/// The API. Serve it with `into_make_service_with_connect_info::<SocketAddr>`
/// so the per-address limit sees each client; without it, every request
/// shares one address bucket.
pub fn router(state: Arc<AppState>) -> Result<Router, String> {
    let cors = (!state.allowed_origins.is_empty())
        .then(|| cors(&state.allowed_origins))
        .transpose()?;
    let api = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/v1/service", get(service_key))
        .route("/v1/proposals", post(submit))
        .route("/v1/businesses/{id}/acceptances", get(acceptances))
        .route("/v1/businesses/{id}/pending", get(pending))
        .route("/v1/recovery", get(recovery))
        .route("/v1/epoch-keys", get(epoch_keys))
        .route("/v1/rejections", get(rejections))
        .route("/v1/session", post(create_session))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        // The body is read with its own limit (see `upload`).
        .route("/v1/blobs/{hash}", put(upload).get(download));
    let api = match cors {
        Some(cors) => api.layer(cors),
        None => api,
    };
    Ok(api
        .layer(middleware::from_fn_with_state(state.clone(), limits))
        .with_state(state))
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
        LedgerError::NoSuchBusiness => error(StatusCode::NOT_FOUND, "No such business."),
        // Unsigned: the code and message a signed rejection would carry.
        LedgerError::Refused(reason) => {
            let mut body = serde_json::Map::new();
            body.insert("error".into(), reason.message.into());
            body.insert("code".into(), serde_json::json!(reason.code));
            if let (Some(size), Some(limit)) = (reason.size, reason.limit) {
                body.insert("size".into(), size.into());
                body.insert("limit".into(), limit.into());
            }
            let body = serde_json::Value::Object(body);
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                [(header::CONTENT_TYPE, "application/json")],
                body.to_string(),
            )
                .into_response()
        }
        other => {
            tracing::error!(error = %other, "request failed");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "The service failed; nothing was stored.",
            )
        }
    }
}

/// The primary key, which every genesis must name. A client compares it
/// with the key it was configured with before creating a business; after
/// that it follows the chain's own hand-overs.
async fn service_key(State(state): State<Arc<AppState>>) -> Response {
    json(serde_json::json!({ "pubkey": state.ledger.public_key().to_hex() }).to_string())
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

fn unauthorized() -> Box<Response> {
    Box::new(error(
        StatusCode::UNAUTHORIZED,
        "Sign this request with NIP-98, or send a session token.",
    ))
}

fn too_many() -> Box<Response> {
    Box::new(error(
        StatusCode::TOO_MANY_REQUESTS,
        "Too many requests. Wait and try again.",
    ))
}

/// Who a read is from: a NIP-98 signature for this GET, or a session.
async fn authorize(
    state: &AppState,
    headers: &HeaderMap,
    uri: &axum::http::Uri,
) -> Result<PublicKey, Box<Response>> {
    let header = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(unauthorized)?;
    match header.strip_prefix("Bearer ") {
        Some(token) => session_key(state, token).await,
        None => signed(state, header, uri, HttpMethod::GET).await,
    }
}

/// Checks a NIP-98 `header` for a request to `uri` with `method`, refusing
/// replays.
async fn signed(
    state: &AppState,
    header: &str,
    uri: &axum::http::Uri,
    method: HttpMethod,
) -> Result<PublicKey, Box<Response>> {
    let path = uri.path_and_query().map_or("/", |p| p.as_str());
    let url = Url::parse(&format!("{}{path}", state.public_url.trim_end_matches('/')))
        .map_err(|_| unauthorized())?;
    let now = state.ledger.clock().now_secs();
    let auth =
        nip98::check(header, &url, method, Timestamp::from_secs(now)).ok_or_else(unauthorized)?;
    if !state
        .reads
        .allow(&auth.pubkey, state.ledger.clock().now_millis())
    {
        return Err(too_many());
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

/// The key a session token is for. An unknown or expired token is a 401,
/// and so is one whose key is no longer a member or recovery key anywhere;
/// the client signs in again.
async fn session_key(state: &AppState, token: &str) -> Result<PublicKey, Box<Response>> {
    let expired = || {
        Box::new(error(
            StatusCode::UNAUTHORIZED,
            "Your session has expired. Sign in again.",
        ))
    };
    let hash = session::hash(token).ok_or_else(expired)?;
    let now = state.ledger.clock().now_secs();
    let key = match state.ledger.session(&hash, now).await {
        Ok(Some(key)) => key,
        Ok(None) => return Err(expired()),
        Err(e) => return Err(Box::new(failure(e))),
    };
    match state.ledger.has_standing(&key).await {
        Ok(true) => {}
        Ok(false) => return Err(expired()),
        Err(e) => return Err(Box::new(failure(e))),
    }
    if !state.reads.allow(&key, state.ledger.clock().now_millis()) {
        return Err(too_many());
    }
    Ok(key)
}

/// `POST /v1/session`, signed with NIP-98: a token for reads, valid for an
/// hour, for a key that is a member or recovery key of some business.
/// Anyone else is refused unsigned, and nothing is stored for them.
async fn create_session(
    State(state): State<Arc<AppState>>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    let Some(header) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    else {
        return *unauthorized();
    };
    // A session can't be had with a session.
    if header.starts_with("Bearer ") {
        return *unauthorized();
    }
    let key = match signed(&state, header, &uri, HttpMethod::POST).await {
        Ok(k) => k,
        Err(r) => return *r,
    };
    match state.ledger.has_standing(&key).await {
        Ok(true) => {}
        Ok(false) => {
            return error(
                StatusCode::FORBIDDEN,
                "This key isn't a member of any business yet.",
            )
        }
        Err(e) => return failure(e),
    }
    let (token, hash) = session::generate();
    let now = state.ledger.clock().now_secs();
    let expires_at = now.saturating_add(SESSION_SECS);
    match state
        .ledger
        .create_session(&hash, &key, expires_at, now)
        .await
    {
        Ok(()) => json(serde_json::json!({ "token": token, "expiresAt": expires_at }).to_string()),
        Err(e) => failure(e),
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

async fn recovery(
    State(state): State<Arc<AppState>>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    let key = match authorize(&state, &headers, &uri).await {
        Ok(k) => k,
        Err(r) => return *r,
    };
    match state.ledger.guarded(&key).await {
        Ok(businesses) => {
            match serde_json::to_string(&serde_json::json!({ "businesses": businesses })) {
                Ok(body) => json(body),
                Err(e) => failure(LedgerError::Internal(e.to_string())),
            }
        }
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

#[derive(Deserialize)]
struct BlobQuery {
    business: Option<String>,
}

fn blob_failure(e: BlobError) -> Response {
    let (status, code) = match &e {
        BlobError::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, "unavailable"),
        BlobError::NotFound => (StatusCode::NOT_FOUND, "notFound"),
        BlobError::Mismatch => (StatusCode::BAD_REQUEST, "hashMismatch"),
        BlobError::TooLarge => (StatusCode::PAYLOAD_TOO_LARGE, "tooLarge"),
        BlobError::Empty => (StatusCode::BAD_REQUEST, "empty"),
        BlobError::Quota { .. } => (StatusCode::INSUFFICIENT_STORAGE, "quota"),
        BlobError::DiskFull => (StatusCode::INSUFFICIENT_STORAGE, "diskFull"),
        BlobError::Ledger(_) | BlobError::Io(_) => {
            tracing::error!(error = %e, "blob request failed");
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "The service failed; nothing was stored.",
            );
        }
    };
    (
        status,
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::json!({ "error": e.to_string(), "code": code }).to_string(),
    )
        .into_response()
}

/// The session's key, the business and the hash of a blob request, or the
/// answer refusing it. Only a session token will do: a browser uploads and
/// downloads in one session, without a signer prompt.
async fn blob_request(
    state: &AppState,
    headers: &HeaderMap,
    hash: &str,
    query: &BlobQuery,
) -> Result<(PublicKey, BusinessId), Box<Response>> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| {
            Box::new(error(
                StatusCode::UNAUTHORIZED,
                "Send a session token from POST /v1/session.",
            ))
        })?;
    let key = session_key(state, token).await?;
    if !blobs::valid_hash(hash) {
        return Err(Box::new(error(
            StatusCode::BAD_REQUEST,
            "A blob's address is its SHA-256: 64 lowercase hex characters.",
        )));
    }
    let business = query
        .business
        .as_deref()
        .and_then(|b| b.parse::<BusinessId>().ok())
        .ok_or_else(|| Box::new(error(StatusCode::BAD_REQUEST, "Invalid business id.")))?;
    Ok((key, business))
}

/// `PUT /v1/blobs/{hash}?business=`: an encrypted receipt as
/// `application/octet-stream`, from an owner, bookkeeper, assistant or
/// submitter. The body is refused once it passes the limit, never read
/// whole first, and its SHA-256 must be `hash`. Uploading a blob the
/// business already has succeeds and stores nothing new.
async fn upload(
    State(state): State<Arc<AppState>>,
    Path(hash): Path<String>,
    Query(q): Query<BlobQuery>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let (key, business) = match blob_request(&state, &headers, &hash, &q).await {
        Ok(r) => r,
        Err(r) => return *r,
    };
    let octets = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == "application/octet-stream");
    if !octets {
        return error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Send the encrypted receipt as application/octet-stream.",
        );
    }
    // Who may upload is checked before the body is read, so no one else can
    // make the service hold 10 MB.
    match state.ledger.may_upload(&business, &key).await {
        Ok(true) => {}
        Ok(false) => return blob_failure(BlobError::NotFound),
        Err(e) => return failure(e),
    }
    let limit = usize::try_from(MAX_BLOB_BYTES).unwrap_or(usize::MAX);
    let bytes = match axum::body::to_bytes(body, limit).await {
        Ok(b) => b,
        // Over the limit, or the connection broke: either way, too much or
        // too little arrived to store.
        Err(_) => return blob_failure(BlobError::TooLarge),
    };
    let size = bytes.len();
    match state
        .ledger
        .put_blob(&business, &key, &hash, bytes.to_vec())
        .await
    {
        Ok(created) => {
            json(serde_json::json!({ "hash": hash, "size": size, "created": created }).to_string())
        }
        Err(e) => blob_failure(e),
    }
}

/// `GET /v1/blobs/{hash}?business=`: the blob, for a member who reads the
/// books, once an accepted record refers to it. Anyone else, and a blob
/// that isn't there, get the same 404.
async fn download(
    State(state): State<Arc<AppState>>,
    Path(hash): Path<String>,
    Query(q): Query<BlobQuery>,
    headers: HeaderMap,
) -> Response {
    let (key, business) = match blob_request(&state, &headers, &hash, &q).await {
        Ok(r) => r,
        Err(r) => return *r,
    };
    match state.ledger.blob(&business, &key, &hash).await {
        Ok(Some(bytes)) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "application/octet-stream"),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
                (header::CONTENT_DISPOSITION, "attachment"),
            ],
            bytes,
        )
            .into_response(),
        Ok(None) => error(StatusCode::NOT_FOUND, "No such receipt."),
        Err(e) => blob_failure(e),
    }
}
