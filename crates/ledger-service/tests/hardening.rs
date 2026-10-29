//! Service hardening from the October 2026 audit (PR B): keys, unsigned
//! refusals, the read lock, stored rejections, request limits, CORS and
//! shutdown. Each test names the finding it pins.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use ledger_service::clock::uuid;
use ledger_service::http::{client_ip, cors};
use ledger_service::ratelimit::RateLimit;
use ledger_service::{Ledger, LedgerError, Status};
use protocol::{Command, History, MembershipChange, ProtocolError, Role, OWNER_DELAY_SECS};

/// The same database and clock, with only the primary key loaded.
fn without_next_key(w: &World) -> Arc<Ledger> {
    Arc::new(Ledger::new(
        w.pool.clone(),
        service_keys(),
        w.clock.clone(),
        Arc::new(uuid),
    ))
}

fn add_owner(keys: &nostr::prelude::Keys) -> MembershipChange {
    MembershipChange::Add {
        member: pk(keys),
        role: Role::Owner,
        history: History::Full,
    }
}

async fn pending_rows(w: &World) -> i64 {
    sqlx::query_scalar("select count(*) from pending_changes")
        .fetch_one(&w.pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_due_key_replacement_whose_key_is_gone_freezes_nothing_but_proposals() {
    // SVC-1: the replacement was accepted with the key loaded; the service
    // then restarts without it.
    let w = World::new().await;
    w.create().await;
    let next = pk(&next_service_keys());
    let ack = w
        .change(&owner(), MembershipChange::ReplaceServiceKey { key: next })
        .await;
    assert_eq!(ack.status, Status::Accepted);
    w.clock.advance_secs(OWNER_DELAY_SECS);
    let ledger = without_next_key(&w);

    // The startup check names it before it falls due.
    assert_eq!(
        ledger.missing_pending_keys().await.unwrap(),
        vec![(w.business.to_string(), next.to_hex())]
    );
    // The scheduler logs and goes on; nothing is appended.
    assert_eq!(ledger.activate_due().await.unwrap(), 0);
    assert_eq!(w.head().await, 1);
    // Reads keep working on the committed state, and the change is still
    // pending.
    let page = ledger
        .acceptances(&w.business, &pk(&owner()), None, 10)
        .await
        .unwrap()
        .expect("the owner still reads");
    assert_eq!((page.head, page.events.len()), (1, 2));
    let pending = ledger
        .pending(&w.business, &pk(&owner()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pending.len(), 1);
    assert!(!ledger
        .deliveries(&pk(&owner()), Some(&w.business))
        .await
        .unwrap()
        .is_empty());
    assert!(!ledger
        .deliveries(&pk(&owner()), None)
        .await
        .unwrap()
        .is_empty());
    // A proposal still fails, since the due change must come first, and
    // stores nothing.
    let proposal = w.proposal(&owner(), Some(1), save(w.business, "draft-1"));
    assert!(matches!(
        ledger.submit(&proposal).await,
        Err(LedgerError::Internal(_))
    ));
    assert_eq!(w.head().await, 1);
    assert_eq!(pending_rows(&w).await, 1);

    // With the key back, it activates.
    assert_eq!(w.run_due().await, 1);
    assert_eq!(w.membership().await.service, next);
    w.finish().await;
}

#[tokio::test]
async fn the_startup_check_names_a_business_whose_key_is_not_loaded() {
    // SVC-1: main refuses to start on any of these.
    let w = World::new().await;
    w.create().await;
    assert!(w.ledger.missing_service_keys().await.unwrap().is_empty());
    let next = pk(&next_service_keys());
    w.change(&owner(), MembershipChange::ReplaceServiceKey { key: next })
        .await;
    w.clock.advance_secs(OWNER_DELAY_SECS);
    assert_eq!(w.run_due().await, 1);
    assert!(w.ledger.missing_service_keys().await.unwrap().is_empty());
    assert_eq!(
        without_next_key(&w).missing_service_keys().await.unwrap(),
        vec![(w.business.to_string(), next.to_hex())]
    );
    w.finish().await;
}

#[tokio::test]
async fn a_rejection_is_never_signed_by_another_businesss_key() {
    // SVC-3: the business moved to the next key, which is then unloaded.
    // Its owner's repeated genesis used to be rejected by the primary key,
    // and stored.
    let w = World::new().await;
    w.create().await;
    w.change(
        &owner(),
        MembershipChange::ReplaceServiceKey {
            key: pk(&next_service_keys()),
        },
    )
    .await;
    w.clock.advance_secs(OWNER_DELAY_SECS);
    assert_eq!(w.run_due().await, 1);
    let ledger = without_next_key(&w);
    let stored = w.stored_for(&owner()).await;
    let again = w.proposal(&owner(), None, genesis(w.business, backup_recovery()));
    assert!(matches!(
        ledger.submit(&again).await,
        Err(LedgerError::Internal(_))
    ));
    assert_eq!(w.stored_for(&owner()).await, stored);
    // With the key loaded, it is rejected by the business's own key.
    let ack = w.ledger.submit(&again).await.unwrap();
    assert_eq!(
        acceptance_or_rejection_signer(&ack),
        pk(&next_service_keys())
    );
    w.finish().await;
}

fn acceptance_or_rejection_signer(
    ack: &ledger_service::Acknowledgment,
) -> nostr::prelude::PublicKey {
    let value: serde_json::Value = serde_json::from_str(&ack.text).unwrap();
    nostr::prelude::Event::from_json(value["event"].to_string())
        .unwrap()
        .pubkey
}

#[tokio::test]
async fn genesis_names_the_primary_key_only() {
    // SVC-7: the other keys are for businesses moving to them.
    let w = World::new().await;
    let mut command = genesis(w.business, backup_recovery());
    if let Command::CreateBusiness { service, .. } = &mut command {
        *service = pk(&next_service_keys());
    }
    let on_next = w.proposal_to(&pk(&next_service_keys()), &owner(), None, command);
    match w.ledger.submit(&on_next).await {
        Err(LedgerError::Refused(reason)) => {
            assert_eq!(
                reason.message,
                ProtocolError::WrongService.to_string(),
                "{reason:?}"
            )
        }
        other => panic!("expected an unsigned refusal, got {other:?}"),
    }
    assert_eq!(w.stored_for(&owner()).await, (0, 0));
    assert_eq!(w.create().await.status, Status::Accepted);
    w.finish().await;
}

#[tokio::test]
async fn strangers_get_unsigned_answers_and_store_nothing() {
    // SVC-6, over HTTP: a fresh key, for a business that exists or not,
    // gets the same unsigned 404, and no row anywhere.
    let w = World::new().await;
    w.create().await;
    let base = w.serve().await;
    let stranger = key(40);
    for business in [w.business, protocol::BusinessId::generate()] {
        let proposal = World::proposal_for(business, &stranger, Some(0), save(business, "d-1"));
        let (status, body) = http(&base, "POST", "/v1/proposals", None, &proposal.as_json()).await;
        assert_eq!(
            (status, body.as_str()),
            (404, r#"{"error":"No such business."}"#)
        );
    }
    assert_eq!(w.stored_for(&stranger).await, (0, 0));
    // A refused genesis is unsigned too, with the reason a rejection would
    // give, and stores nothing.
    let other = protocol::BusinessId::generate();
    let mut bad = genesis(other, backup_recovery());
    if let Command::CreateBusiness { profile, .. } = &mut bad {
        profile.id = "not-the-business-id".into();
    }
    let proposal = World::proposal_for(other, &stranger, None, bad);
    let (status, body) = http(&base, "POST", "/v1/proposals", None, &proposal.as_json()).await;
    assert_eq!(status, 422, "{body}");
    assert!(body.contains(r#""code":"rejected""#), "{body}");
    assert_eq!(w.stored_for(&stranger).await, (0, 0));
    w.finish().await;
}

#[tokio::test]
async fn reading_every_business_activates_their_due_changes_first() {
    // SVC-2 / HTTP-4: a second owner's removal is due but the scheduler
    // hasn't run. With ?business= they got nothing; without it, they got
    // their old deliveries.
    let w = World::new().await;
    w.create().await;
    w.change(&owner(), add_owner(&second_owner())).await;
    w.clock.advance_secs(OWNER_DELAY_SECS);
    assert_eq!(w.run_due().await, 1);
    assert!(!w
        .ledger
        .deliveries(&pk(&second_owner()), None)
        .await
        .unwrap()
        .is_empty());
    w.change(
        &owner(),
        MembershipChange::Remove {
            member: pk(&second_owner()),
        },
    )
    .await;
    w.clock.advance_secs(OWNER_DELAY_SECS);
    assert_eq!(
        w.ledger
            .deliveries(&pk(&second_owner()), None)
            .await
            .unwrap(),
        Vec::<String>::new()
    );
    // The read activated it.
    assert_eq!(w.run_due().await, 0);
    w.finish().await;
}

#[tokio::test]
async fn a_read_waits_for_a_proposal_holding_the_business() {
    // HTTP-5: the permission check and the page come from one state. Here
    // a "proposal" holds the business row, removes every reader, and
    // commits; the read that started meanwhile must see the removal.
    let w = World::new().await;
    w.create().await;
    let mut tx = w.pool.begin().await.unwrap();
    sqlx::query("select 1 from businesses where id = $1 for update")
        .bind(w.business.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let ledger = w.ledger.clone();
    let business = w.business;
    let read =
        tokio::spawn(async move { ledger.acceptances(&business, &pk(&owner()), None, 10).await });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!read.is_finished(), "the read didn't wait for the lock");
    sqlx::query(
        "update businesses set membership = jsonb_set(membership, '{members}', '{}') where id = $1",
    )
    .bind(w.business.to_string())
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert!(read.await.unwrap().unwrap().is_none());
    w.finish().await;
}

#[tokio::test]
async fn the_audit_table_keeps_codes_not_messages() {
    // HTTP-6 / PROTO-4: the message can quote the payload; it now travels
    // only inside the rejection, encrypted to the proposer.
    let w = World::new().await;
    w.create().await;
    let ack = w
        .send(&owner(), Some(0), approve("invented-missing-draft"))
        .await;
    assert_eq!(ack.status, Status::Rejected);
    let columns: Vec<String> = sqlx::query_scalar(
        "select column_name::text from information_schema.columns where table_name = 'audit'",
    )
    .fetch_all(&w.pool)
    .await
    .unwrap();
    assert!(!columns.iter().any(|c| c == "message"), "{columns:?}");
    let row: String = sqlx::query_scalar("select row_to_json(audit)::text from audit")
        .fetch_one(&w.pool)
        .await
        .unwrap();
    assert!(!row.contains("invented-missing-draft"), "{row}");
    assert!(row.contains(r#""code":"rejected""#), "{row}");
    w.finish().await;
}

/// Sends `n` bodies that aren't events, from `forwarded` if given.
async fn garbage(base: &str, n: usize, forwarded: Option<&str>) -> Vec<u16> {
    let mut out = Vec::new();
    for _ in 0..n {
        let headers: Vec<(&str, &str)> = forwarded
            .map(|f| ("X-Forwarded-For", f))
            .into_iter()
            .collect();
        out.push(
            http_with(base, "POST", "/v1/proposals", &headers, "not json")
                .await
                .0,
        );
    }
    out
}

#[tokio::test]
async fn every_request_is_limited_before_it_is_parsed() {
    // HTTP-1: the limits run before JSON parsing or a signature check, so
    // a flood of unsigned junk is refused by them, not by the parser.
    let w = World::new().await;
    let base = w
        .serve_with(|s| s.global = RateLimit::with_capacity(2, 0, 10))
        .await;
    assert_eq!(garbage(&base, 3, None).await, vec![400, 400, 429]);
    let base = w
        .serve_with(|s| s.per_ip = RateLimit::with_capacity(2, 0, 10))
        .await;
    assert_eq!(garbage(&base, 3, None).await, vec![400, 400, 429]);
    w.finish().await;
}

#[tokio::test]
async fn the_client_address_comes_from_a_trusted_proxy_only() {
    // HTTP-1: behind a trusted proxy each client has its own bucket; from
    // anyone else, X-Forwarded-For is ignored.
    let w = World::new().await;
    let local = "127.0.0.1".parse().unwrap();
    let base = w
        .serve_with(|s| {
            s.per_ip = RateLimit::with_capacity(2, 0, 10);
            s.trusted_proxies = vec![local];
        })
        .await;
    let a = Some("192.0.2.10");
    assert_eq!(garbage(&base, 3, a).await, vec![400, 400, 429]);
    assert_eq!(garbage(&base, 1, Some("192.0.2.11")).await, vec![400]);
    let base = w
        .serve_with(|s| s.per_ip = RateLimit::with_capacity(2, 0, 10))
        .await;
    assert_eq!(garbage(&base, 2, Some("192.0.2.20")).await, vec![400, 400]);
    assert_eq!(garbage(&base, 1, Some("192.0.2.21")).await, vec![429]);

    let headers = |v: &str| {
        let mut h = axum::http::HeaderMap::new();
        h.insert("x-forwarded-for", v.parse().unwrap());
        h
    };
    let proxy: std::net::IpAddr = "10.0.0.1".parse().unwrap();
    let ip = |s: &str| s.parse::<std::net::IpAddr>().unwrap();
    // The last hop the proxy added is the client; earlier ones are the
    // client's own claims.
    assert_eq!(
        client_ip(proxy, &headers("203.0.113.9, 192.0.2.30"), &[proxy]),
        ip("192.0.2.30")
    );
    assert_eq!(
        client_ip(proxy, &headers("192.0.2.30, 10.0.0.1"), &[proxy]),
        ip("192.0.2.30")
    );
    assert_eq!(client_ip(proxy, &headers("garbage"), &[proxy]), proxy);
    assert_eq!(
        client_ip(ip("192.0.2.40"), &headers("192.0.2.30"), &[proxy]),
        ip("192.0.2.40")
    );
    w.finish().await;
}

#[tokio::test]
async fn a_body_declared_too_large_is_refused_before_anything_else() {
    // HTTP-1: on a route that never reads a body, the declared size used
    // to be ignored (this was a 401).
    let w = World::new().await;
    let base = w.serve().await;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let host = base.trim_start_matches("http://");
    let mut stream = tokio::net::TcpStream::connect(host).await.unwrap();
    let request = format!(
        "GET /v1/epoch-keys HTTP/1.1\r\nHost: {host}\r\nContent-Length: 50000000\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut head = [0u8; 12];
    stream.read_exact(&mut head).await.unwrap();
    assert_eq!(&head[9..12], b"413");
    w.finish().await;
}

#[tokio::test]
async fn cors_allows_exactly_the_configured_origins() {
    // HTTP-7.
    let w = World::new().await;
    let base = w
        .serve_with(|s| s.allowed_origins = vec!["https://app.example".into()])
        .await;
    let preflight = |origin: &'static str| {
        let base = base.clone();
        async move {
            http_with(
                &base,
                "OPTIONS",
                "/v1/proposals",
                &[
                    ("Origin", origin),
                    ("Access-Control-Request-Method", "POST"),
                    (
                        "Access-Control-Request-Headers",
                        "authorization, content-type",
                    ),
                ],
                "",
            )
            .await
            .1
            .to_ascii_lowercase()
        }
    };
    let allowed = preflight("https://app.example").await;
    assert!(
        allowed.contains("access-control-allow-origin: https://app.example"),
        "{allowed}"
    );
    assert!(allowed.contains("authorization"), "{allowed}");
    assert!(!allowed.contains("access-control-allow-credentials"));
    let other = preflight("https://elsewhere.example").await;
    assert!(!other.contains("access-control-allow-origin"), "{other}");
    // Without configuration, no origin is allowed.
    let base = w.serve().await;
    let (_, head, _) = http_with(
        &base,
        "GET",
        "/health",
        &[("Origin", "https://app.example")],
        "",
    )
    .await;
    assert!(!head
        .to_ascii_lowercase()
        .contains("access-control-allow-origin"));
    // A wildcard or a malformed origin is refused, never allowed.
    for bad in [
        "*",
        "https://app.example/",
        "app.example",
        "HTTPS://APP.EXAMPLE",
    ] {
        assert!(cors(&[bad.to_string()]).is_err(), "{bad}");
    }
    w.finish().await;
}

#[tokio::test]
async fn sigterm_shuts_the_service_down_cleanly() {
    // HTTP-3: SIGTERM used to kill the process at once, without draining.
    let w = World::new().await;
    let dir = std::env::temp_dir().join(format!("zb-sigterm-{}", uuid()));
    std::fs::create_dir(&dir).unwrap();
    let key_file = dir.join("service.key");
    let bin = env!("CARGO_BIN_EXE_ledger-service");
    let made = std::process::Command::new(bin)
        .arg("generate-key")
        .arg(&key_file)
        .output()
        .unwrap();
    assert!(made.status.success());
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let url = std::env::var("DATABASE_URL").unwrap();
    let database = format!("{}/{}", url.rsplit_once('/').unwrap().0, w.name);
    let mut child = std::process::Command::new(bin)
        .args(["--public-url", "https://ledger.example", "--listen"])
        .arg(format!("127.0.0.1:{port}"))
        .arg("--key-file")
        .arg(&key_file)
        .env("DATABASE_URL", database)
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let base = format!("http://127.0.0.1:{port}");
    let mut up = false;
    for _ in 0..100 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            up = http(&base, "GET", "/health", None, "").await.0 == 200;
            if up {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(up, "the service didn't start");
    let pid = child.id().to_string();
    assert!(std::process::Command::new("kill")
        .args(["-TERM", &pid])
        .status()
        .unwrap()
        .success());
    let mut status = None;
    for _ in 0..200 {
        status = child.try_wait().unwrap();
        if status.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let Some(status) = status else {
        child.kill().unwrap();
        panic!("it didn't stop");
    };
    assert!(status.success(), "{status:?}");
    std::fs::remove_dir_all(&dir).unwrap();
    w.finish().await;
}
