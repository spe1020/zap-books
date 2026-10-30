//! Read sessions (stage 2 PR 4 brief, "Sessions instead of a signature per
//! read"): a token from `POST /v1/session` stands in for NIP-98 on reads,
//! and every read still checks membership. Every one is a merge gate.

mod common;

use common::*;
use ledger_service::clock::Clock;
use nostr::nips::nip98::{HttpData, HttpMethod};
use nostr::prelude::{Event, Keys, Timestamp};
use nostr::types::url::Url;
use protocol::{History, MembershipChange, Role};
use serde_json::Value;

async fn auth(keys: &Keys, url: &str, method: HttpMethod) -> String {
    HttpData::new(Url::parse(url).unwrap(), method)
        .to_authorization(keys)
        .await
        .unwrap()
}

/// A NIP-98 header signed as at `secs`, for a service clock moved off the
/// real one.
fn auth_at(keys: &Keys, url: &str, method: HttpMethod, secs: u64) -> String {
    use base64::Engine;
    use nostr::event::{FinalizeEvent, IntoEventBuilder};
    let event = HttpData::new(Url::parse(url).unwrap(), method)
        .into_event_builder()
        .custom_created_at(Timestamp::from_secs(secs))
        .finalize(keys)
        .unwrap();
    let encoded = base64::engine::general_purpose::STANDARD.encode(event.as_json());
    format!("Nostr {encoded}")
}

/// `POST /v1/session` signed by `keys`: the status and the body.
async fn sign_in(base: &str, keys: &Keys) -> (u16, Value) {
    let header = auth(keys, &format!("{base}/v1/session"), HttpMethod::POST).await;
    let (status, body) = http(base, "POST", "/v1/session", Some(&header), "").await;
    (status, serde_json::from_str(&body).unwrap())
}

/// A session token for `keys`, which must be granted.
async fn token(base: &str, keys: &Keys) -> String {
    let (status, body) = sign_in(base, keys).await;
    assert_eq!(status, 200, "{body}");
    body["token"].as_str().unwrap().to_string()
}

/// A read of `path` with a session token.
async fn read(base: &str, token: &str, path: &str) -> (u16, String) {
    http(base, "GET", path, Some(&format!("Bearer {token}")), "").await
}

async fn sessions(w: &World) -> Vec<(String, String, i64)> {
    sqlx::query_as("select token_hash, pubkey, expires_at from sessions")
        .fetch_all(&w.pool)
        .await
        .unwrap()
}

fn add(keys: &Keys, role: Role) -> MembershipChange {
    MembershipChange::Add {
        member: keys.public_key(),
        role,
        history: History::Full,
    }
}

/// The members each delivery in a `GET /v1/epoch-keys` answer is for.
fn delivered_to(body: &str) -> Vec<String> {
    let value: Value = serde_json::from_str(body).unwrap();
    value["deliveries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            let event = Event::from_json(d.to_string()).unwrap();
            event
                .tags
                .iter()
                .find(|t| t.as_slice().first().map(String::as_str) == Some("p"))
                .map(|t| t.as_slice()[1].clone())
                .unwrap()
        })
        .collect()
}

#[tokio::test]
async fn a_session_reads_like_a_signature_for_an_hour_and_stores_only_a_hash() {
    let w = World::new().await;
    w.create().await;
    let base = w.serve().await;

    let (status, body) = sign_in(&base, &owner()).await;
    assert_eq!(status, 200, "{body}");
    let token = body["token"].as_str().unwrap().to_string();
    assert_eq!(token.len(), 64);
    assert!(token.bytes().all(|b| b.is_ascii_hexdigit()));
    let now = w.clock.now_millis() / 1000;
    assert_eq!(body["expiresAt"], now + 3600);

    let acceptances = format!("/v1/businesses/{}/acceptances", w.business);
    let (status, body) = read(&base, &token, &acceptances).await;
    assert_eq!(status, 200, "{body}");
    assert!(body.contains(r#""head":0"#));
    // Used again and again: a token is not a one-time signature.
    for path in [
        acceptances.clone(),
        format!("/v1/businesses/{}/pending", w.business),
        format!("/v1/epoch-keys?business={}", w.business),
        "/v1/rejections".to_string(),
    ] {
        assert_eq!(read(&base, &token, &path).await.0, 200, "{path}");
    }

    // Only the token's hash is stored.
    let stored = sessions(&w).await;
    assert_eq!(stored.len(), 1);
    let (hash, key, expires_at) = &stored[0];
    assert_eq!(*hash, ledger_service::session::hash(&token).unwrap());
    assert_ne!(*hash, token);
    assert_eq!(*key, owner().public_key().to_hex());
    assert_eq!(*expires_at as u64, now + 3600);

    // Still good just before the hour is up.
    w.clock.advance_secs(3599);
    assert_eq!(read(&base, &token, &acceptances).await.0, 200);
    w.finish().await;
}

#[tokio::test]
async fn an_expired_token_is_refused() {
    let w = World::new().await;
    w.create().await;
    let base = w.serve().await;
    let token = token(&base, &owner()).await;
    let path = format!("/v1/businesses/{}/acceptances", w.business);

    w.clock.advance_secs(3600);
    let (status, body) = read(&base, &token, &path).await;
    assert_eq!(status, 401, "{body}");
    assert!(body.contains("Sign in again"), "{body}");

    // Signing in again, an hour on, gives a new token and forgets the
    // expired one.
    let at = w.clock.now_millis() / 1000;
    let url = format!("{base}/v1/session");
    let header = auth_at(&owner(), &url, HttpMethod::POST, at);
    let (status, body) = http(&base, "POST", "/v1/session", Some(&header), "").await;
    assert_eq!(status, 200, "{body}");
    let fresh: Value = serde_json::from_str(&body).unwrap();
    let fresh = fresh["token"].as_str().unwrap().to_string();
    assert_ne!(fresh, token);
    assert_eq!(read(&base, &fresh, &path).await.0, 200);
    assert_eq!(sessions(&w).await.len(), 1);
    w.finish().await;
}

#[tokio::test]
async fn an_unknown_or_malformed_token_is_refused() {
    let w = World::new().await;
    w.create().await;
    let base = w.serve().await;
    let real = token(&base, &owner()).await;
    let path = format!("/v1/businesses/{}/acceptances", w.business);

    let unknown = "0f".repeat(32);
    let hash = ledger_service::session::hash(&real).unwrap();
    for bad in [
        unknown.as_str(),
        "",
        "abc",
        &real.to_uppercase(),
        &format!("{real}0"),
        &real[1..],
        // The stored hash is not a token.
        &hash,
    ] {
        assert_eq!(read(&base, bad, &path).await.0, 401, "{bad:?}");
    }
    // A session can't be had with a session.
    let (status, _) = http(
        &base,
        "POST",
        "/v1/session",
        Some(&format!("Bearer {real}")),
        "",
    )
    .await;
    assert_eq!(status, 401);
    // Nor with NIP-98 signed for a GET, or replayed.
    let url = format!("{base}/v1/session");
    let get = auth(&owner(), &url, HttpMethod::GET).await;
    assert_eq!(
        http(&base, "POST", "/v1/session", Some(&get), "").await.0,
        401
    );
    // A second on from the sign-in above, which signed the same event.
    let post = auth_at(
        &owner(),
        &url,
        HttpMethod::POST,
        w.clock.now_millis() / 1000 + 1,
    );
    assert_eq!(
        http(&base, "POST", "/v1/session", Some(&post), "").await.0,
        200
    );
    assert_eq!(
        http(&base, "POST", "/v1/session", Some(&post), "").await.0,
        401
    );
    w.finish().await;
}

#[tokio::test]
async fn a_token_used_after_its_member_was_removed_is_refused() {
    let w = World::new().await;
    w.create().await; // 0
    let bookkeeper = key(30);
    w.change(&owner(), add(&bookkeeper, Role::Bookkeeper)).await; // 1
                                                                  // A second business the bookkeeper founded, so they keep standing
                                                                  // somewhere after the removal.
    let theirs = protocol::BusinessId::generate();
    let genesis = World::proposal_for(
        theirs,
        &bookkeeper,
        None,
        common::genesis(theirs, backup_recovery()),
    );
    w.ledger.submit(&genesis).await.unwrap();

    let base = w.serve().await;
    let token = token(&base, &bookkeeper).await;
    let path = format!("/v1/businesses/{}/acceptances", w.business);
    let keys = format!("/v1/epoch-keys?business={}", w.business);
    assert_eq!(read(&base, &token, &path).await.0, 200);
    assert_eq!(delivered_to(&read(&base, &token, &keys).await.1).len(), 1);

    let removal = MembershipChange::Remove {
        member: bookkeeper.public_key(),
    };
    w.change(&owner(), removal).await; // 2
                                       // Refused at once, with the same token: no books, no keys.
    assert_eq!(read(&base, &token, &path).await.0, 404);
    let (status, body) = read(&base, &token, &keys).await;
    assert_eq!(status, 200);
    assert_eq!(body, r#"{"deliveries":[]}"#);
    // Their own business still reads.
    let own = format!("/v1/businesses/{theirs}/acceptances");
    assert_eq!(read(&base, &token, &own).await.0, 200);

    // A member removed from their only business keeps no session at all.
    let viewer = key(31);
    w.change(&owner(), add(&viewer, Role::Viewer)).await; // 3
    let viewer_token = self::token(&base, &viewer).await;
    assert_eq!(read(&base, &viewer_token, &path).await.0, 200);
    let removal = MembershipChange::Remove {
        member: viewer.public_key(),
    };
    w.change(&owner(), removal).await; // 4
    assert_eq!(read(&base, &viewer_token, &path).await.0, 401);
    assert_eq!(read(&base, &viewer_token, "/v1/epoch-keys").await.0, 401);
    w.finish().await;
}

#[tokio::test]
async fn a_token_reads_only_its_own_keys_deliveries_and_rejections() {
    let w = World::new().await;
    w.create().await; // 0
    let viewer = key(32);
    w.change(&owner(), add(&viewer, Role::Viewer)).await; // 1
                                                          // A rejection for the owner only.
    let stale = w.send(&owner(), Some(0), approve("draft-1")).await;
    rejection(&stale, &owner());

    let base = w.serve().await;
    let owner_token = token(&base, &owner()).await;
    let viewer_token = token(&base, &viewer).await;
    let owner_hex = owner().public_key().to_hex();
    let viewer_hex = viewer.public_key().to_hex();

    for path in [
        "/v1/epoch-keys".to_string(),
        format!("/v1/epoch-keys?business={}", w.business),
    ] {
        let (status, body) = read(&base, &viewer_token, &path).await;
        assert_eq!(status, 200);
        let to = delivered_to(&body);
        assert!(!to.is_empty());
        assert!(to.iter().all(|k| *k == viewer_hex), "{path}");

        let (_, body) = read(&base, &owner_token, &path).await;
        let to = delivered_to(&body);
        assert!(!to.is_empty());
        assert!(to.iter().all(|k| *k == owner_hex), "{path}");
    }

    let rejections = |body: String| {
        let value: Value = serde_json::from_str(&body).unwrap();
        value["rejections"].as_array().unwrap().len()
    };
    assert_eq!(
        rejections(read(&base, &owner_token, "/v1/rejections").await.1),
        1
    );
    assert_eq!(
        rejections(read(&base, &viewer_token, "/v1/rejections").await.1),
        0
    );
    // Pending changes are for owners and the recovery key only.
    let pending = format!("/v1/businesses/{}/pending", w.business);
    assert_eq!(read(&base, &viewer_token, &pending).await.0, 404);
    assert_eq!(read(&base, &owner_token, &pending).await.0, 200);
    w.finish().await;
}

#[tokio::test]
async fn the_service_reports_the_key_genesis_must_name() {
    let w = World::new().await;
    let base = w.serve().await;
    let (status, body) = http(&base, "GET", "/v1/service", None, "").await;
    assert_eq!(status, 200);
    assert_eq!(
        body,
        format!(r#"{{"pubkey":"{}"}}"#, service_keys().public_key().to_hex())
    );
    w.finish().await;
}

#[tokio::test]
async fn a_session_is_for_members_and_the_recovery_key_only() {
    let w = World::new().await;
    let base = w.serve().await;
    // Before the business exists, its founder has no standing either.
    assert_eq!(sign_in(&base, &owner()).await.0, 403);
    w.create().await;

    let (status, body) = sign_in(&base, &outsider()).await;
    assert_eq!(status, 403, "{body}");
    assert!(sessions(&w)
        .await
        .iter()
        .all(|(_, k, _)| *k != outsider().public_key().to_hex()));
    assert!(sessions(&w).await.is_empty());

    // The recovery key can sign in, to see and veto pending changes.
    let token = token(&base, &backup()).await;
    let pending = format!("/v1/businesses/{}/pending", w.business);
    assert_eq!(read(&base, &token, &pending).await.0, 200);
    // It reads no books.
    let path = format!("/v1/businesses/{}/acceptances", w.business);
    assert_eq!(read(&base, &token, &path).await.0, 404);
    w.finish().await;
}

#[tokio::test]
async fn the_recovery_key_sees_owners_and_pending_changes_but_no_books() {
    let w = World::new().await;
    let base = w.serve().await;
    w.create().await;
    let incoming = key(41);
    let ack = w.change(&owner(), add(&incoming, Role::Owner)).await;
    assert_eq!(ack.status, ledger_service::Status::Accepted);

    let backup_token = token(&base, &backup()).await;
    let (status, body) = read(&base, &backup_token, "/v1/recovery").await;
    assert_eq!(status, 200, "{body}");
    let value: Value = serde_json::from_str(&body).unwrap();
    let guarded = value["businesses"].as_array().unwrap();
    assert_eq!(guarded.len(), 1);
    let b = &guarded[0];
    assert_eq!(b["business"], w.business.to_string());
    assert_eq!(b["service"], service_keys().public_key().to_hex());
    assert_eq!(
        b["owners"],
        serde_json::json!([owner().public_key().to_hex()])
    );
    let pending = b["pending"].as_array().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0]["proposer"], owner().public_key().to_hex());
    assert_eq!(pending[0]["change"]["op"], "add");
    // Nothing from the books.
    let mut fields: Vec<&str> = b.as_object().unwrap().keys().map(|k| k.as_str()).collect();
    fields.sort();
    assert_eq!(fields, ["business", "owners", "pending", "service"]);

    // A member guards nothing.
    let owner_token = token(&base, &owner()).await;
    let (status, body) = read(&base, &owner_token, "/v1/recovery").await;
    assert_eq!(status, 200);
    assert_eq!(body, r#"{"businesses":[]}"#);
    w.finish().await;
}
