//! The service's required tests (stage 2 brief, PR 2), against Postgres.
//! Membership and keys (PR 3) are in `membership.rs`.
//! Every one is a merge gate; CI runs them with a Postgres service.

mod common;

use common::*;
use ledger_service::Status;
use nostr::nips::nip98::{HttpData, HttpMethod};
use nostr::prelude::Event;
use nostr::types::url::Url;
use protocol::{
    audit, rebuild, verify_chain, Command, RejectionCode, CHANGED_SINCE_OPENED, MAX_EVENT_BYTES,
};

#[tokio::test]
async fn genesis_creates_the_books_and_delivers_epoch_zero() {
    let w = World::new().await;
    let ack = w.create().await;
    assert_eq!(ack.status, Status::Accepted);
    assert_eq!(w.head().await, 0);
    let (events, contents) = w.read(&owner()).await;
    assert_eq!(events.len(), 1);
    assert!(matches!(
        contents[0].payload.command,
        Command::CreateBusiness { .. }
    ));
    // Creating it again is rejected; so is a command for a business that
    // doesn't exist.
    let again = w.create().await;
    assert_eq!(
        rejection(&again, &owner()).reason.message,
        "This business already exists."
    );
    let other = World {
        business: protocol::BusinessId::generate(),
        ..w
    };
    let missing = other.send(&owner(), Some(0), approve("draft-1")).await;
    assert_eq!(
        rejection(&missing, &owner()).reason.code,
        RejectionCode::Rejected
    );
    other.finish().await;
}

#[tokio::test]
async fn two_concurrent_approvals_accept_exactly_one() {
    let w = World::new().await;
    // Two owners, so two different people approve at the same moment.
    w.send(
        &owner(),
        None,
        common::genesis(
            w.business,
            protocol::Recovery::SecondOwner {
                pubkey: pk(&second_owner()),
            },
        ),
    )
    .await;
    for round in 0..5i64 {
        // A fresh draft each round, with its own amount so the ledger's
        // duplicate check stays out of it. Both approvals start from the
        // same seq.
        let head = w.head().await;
        let id = format!("draft-{round}");
        let mut d = draft(w.business, &id);
        d.amount_minor = ledger_core::Minor::new(1_000 + round).unwrap();
        let saved = w
            .send(
                &owner(),
                Some(head),
                Command::SaveDraft {
                    draft: d,
                    documents: vec![],
                    extraction: None,
                },
            )
            .await;
        assert_eq!(saved.status, Status::Accepted, "{}", saved.text);
        let base = w.head().await;
        let a = w.proposal(&owner(), Some(base), approve(&id));
        let b = w.proposal(&second_owner(), Some(base), approve(&id));
        let (ra, rb) = tokio::join!(w.ledger.submit(&a), w.ledger.submit(&b));
        let (ra, rb) = (ra.unwrap(), rb.unwrap());
        let accepted = [&ra, &rb]
            .iter()
            .filter(|r| r.status == Status::Accepted)
            .count();
        let why: Vec<String> = [(&ra, owner()), (&rb, second_owner())]
            .into_iter()
            .filter(|(r, _)| r.status == Status::Rejected)
            .map(|(r, k)| rejection(r, &k).reason.message)
            .collect();
        assert_eq!(accepted, 1, "rejections: {why:?}");
        // The loser is told the expense changed since it was opened.
        assert_eq!(why, vec![CHANGED_SINCE_OPENED.to_string()]);
        assert_eq!(w.head().await, base + 1);
    }
    let posted = w.snapshot().await.journals.len();
    assert_eq!(posted, 5);
    w.finish().await;
}

#[tokio::test]
async fn the_same_proposal_twice_gives_one_acceptance_and_the_same_answer() {
    let w = World::new().await;
    w.create().await;
    let event = w.proposal(&owner(), Some(0), save(w.business, "draft-1"));
    let first = w.ledger.submit(&event).await.unwrap();
    let second = w.ledger.submit(&event).await.unwrap();
    assert_eq!(first, second);
    // And sent concurrently.
    let next = w.proposal(&owner(), Some(1), approve("draft-1"));
    let (x, y) = tokio::join!(w.ledger.submit(&next), w.ledger.submit(&next));
    assert_eq!(x.unwrap(), y.unwrap());
    assert_eq!(w.head().await, 2);
    let rows: i64 = sqlx::query_scalar("select count(*) from acceptances")
        .fetch_one(&w.pool)
        .await
        .unwrap();
    assert_eq!(rows, 3);
    // A rejection is idempotent too.
    let stale = w.proposal(&owner(), Some(0), approve("draft-1"));
    let r1 = w.ledger.submit(&stale).await.unwrap();
    assert_eq!(r1, w.ledger.submit(&stale).await.unwrap());
    w.finish().await;
}

/// A short history with every kind of record change.
async fn history(w: &World) {
    w.create().await;
    let doc = protocol::DocumentRef {
        id: "doc-1".into(),
        sha256: "4c".repeat(32),
        name: "receipt.pdf".into(),
        mime: "application/pdf".into(),
        size: 4096,
    };
    let mut d = draft(w.business, "draft-1");
    d.document_ids = vec!["doc-1".into()];
    let steps = vec![
        Command::SaveDraft {
            draft: d,
            documents: vec![doc],
            extraction: None,
        },
        approve("draft-1"),
        Command::UpdateSettings {
            change: ledger_core::SettingsChange::AddCategory {
                name: "Invented travel".into(),
                currency: ledger_core::Currency::Usd,
            },
        },
        Command::SaveRule {
            rule: ledger_core::RuleInput {
                id: None,
                vendor_includes: "Invented Vendor".into(),
                category_id: setup(w.business).categories[0].id.clone(),
                purpose: "Office use".into(),
                enabled: true,
            },
        },
        save(w.business, "draft-2"),
    ];
    for command in steps {
        let head = w.head().await;
        let ack = w.send(&owner(), Some(head), command).await;
        assert_eq!(ack.status, Status::Accepted, "{}", ack.text);
    }
}

#[tokio::test]
async fn rebuilding_from_the_chain_reproduces_the_stored_snapshot() {
    let w = World::new().await;
    history(&w).await;
    let (_, contents) = w.read(&owner()).await;
    assert_eq!(contents.len(), 6);
    assert_eq!(rebuild(&contents).unwrap(), w.snapshot().await);
    w.finish().await;
}

#[tokio::test]
async fn re_running_every_command_reproduces_its_record_changes() {
    let w = World::new().await;
    history(&w).await;
    let (_, contents) = w.read(&owner()).await;
    let mut books = audit(None, &contents[0]).unwrap();
    for content in &contents[1..] {
        books = audit(Some(&books), content).unwrap();
    }
    assert_eq!(books, w.snapshot().await);
    w.finish().await;
}

#[tokio::test]
async fn tampering_with_the_stored_chain_is_detected() {
    let w = World::new().await;
    history(&w).await;
    let (events, _) = w.read(&owner()).await;
    let service = pk(&service_keys());
    assert!(verify_chain(w.business, service, None, &events).is_ok());

    let mut deleted = events.clone();
    deleted.remove(2);
    assert!(verify_chain(w.business, service, None, &deleted).is_err());
    let mut reordered = events.clone();
    reordered.swap(1, 3);
    assert!(verify_chain(w.business, service, None, &reordered).is_err());

    // Altering a stored row: what the service then serves fails.
    let mut json: serde_json::Value = serde_json::from_str(&events[3].as_json()).unwrap();
    json["created_at"] = (json["created_at"].as_u64().unwrap() + 1).into();
    sqlx::query("update acceptances set event = $1 where business_id = $2 and seq = 3")
        .bind(json.to_string())
        .bind(w.business.to_string())
        .execute(&w.pool)
        .await
        .unwrap();
    let page = w
        .ledger
        .acceptances(&w.business, &pk(&owner()), None, 500)
        .await
        .unwrap()
        .unwrap();
    let served: Vec<Event> = page
        .events
        .iter()
        .map(|e| Event::from_json(e).unwrap())
        .collect();
    assert!(verify_chain(w.business, service, None, &served).is_err());
    w.finish().await;
}

#[tokio::test]
async fn a_stale_base_seq_is_rejected_as_changed_since_opened() {
    let w = World::new().await;
    w.create().await;
    w.send(&owner(), Some(0), save(w.business, "draft-1")).await;
    // Opened at seq 1, then someone else edits the draft (seq 2).
    let mut edited = draft(w.business, "draft-1");
    edited.purpose = "Edited purpose".into();
    let edit = Command::SaveDraft {
        draft: edited,
        documents: vec![],
        extraction: None,
    };
    assert_eq!(
        w.send(&owner(), Some(1), edit).await.status,
        Status::Accepted
    );
    let stale = w.send(&owner(), Some(1), approve("draft-1")).await;
    assert_eq!(
        rejection(&stale, &owner()).reason.message,
        CHANGED_SINCE_OPENED
    );
    assert_eq!(w.head().await, 2);
    // From the new head it goes through.
    assert_eq!(
        w.send(&owner(), Some(2), approve("draft-1")).await.status,
        Status::Accepted
    );
    w.finish().await;
}

#[tokio::test]
async fn too_large_is_rejected_with_its_size_and_nothing_is_stored() {
    let w = World::new().await;
    w.create().await;
    // A draft whose purpose fits in a proposal but not in its acceptance,
    // which carries it twice and adds the records it creates.
    let mut d = draft(w.business, "draft-1");
    d.purpose = "x".repeat(30_000);
    let command = Command::SaveDraft {
        draft: d,
        documents: vec![],
        extraction: None,
    };
    let ack = w.send(&owner(), Some(0), command).await;
    let reason = rejection(&ack, &owner()).reason;
    assert_eq!(reason.code, RejectionCode::TooLarge);
    assert_eq!(reason.limit, Some(MAX_EVENT_BYTES as u64));
    assert!(reason.size.unwrap() > MAX_EVENT_BYTES as u64);
    assert_eq!(w.head().await, 0);
    assert!(w.snapshot().await.drafts.is_empty());
    let versions: i64 =
        sqlx::query_scalar("select count(*) from record_versions where record_type = 'draft'")
            .fetch_one(&w.pool)
            .await
            .unwrap();
    assert_eq!(versions, 0);
    w.finish().await;
}

#[tokio::test]
async fn permissions_are_enforced() {
    let w = World::new().await;
    w.create().await;
    let stranger = w
        .send(&outsider(), Some(0), save(w.business, "draft-1"))
        .await;
    assert_eq!(
        rejection(&stranger, &outsider()).reason.message,
        "You are not a member of this business."
    );
    let recovery = w
        .send(&backup(), Some(0), save(w.business, "draft-1"))
        .await;
    assert_eq!(
        rejection(&recovery, &backup()).reason.message,
        "Your role can't make this change."
    );
    assert_eq!(w.head().await, 0);
    // Only members who read get the chain.
    assert!(w
        .ledger
        .acceptances(&w.business, &pk(&outsider()), None, 10)
        .await
        .unwrap()
        .is_none());
    assert!(w
        .ledger
        .acceptances(&w.business, &pk(&backup()), None, 10)
        .await
        .unwrap()
        .is_none());
    w.finish().await;
}

async fn auth(keys: &nostr::prelude::Keys, url: &str, method: HttpMethod) -> String {
    HttpData::new(Url::parse(url).unwrap(), method)
        .to_authorization(keys)
        .await
        .unwrap()
}

#[tokio::test]
async fn nip98_refuses_expired_misdirected_and_replayed_auth() {
    let w = World::new().await;
    w.create().await;
    let base = w.serve().await;
    let path = format!("/v1/businesses/{}/acceptances?after=0", w.business);
    let url = format!("{base}{path}");

    let good = auth(&owner(), &url, HttpMethod::GET).await;
    let (status, body) = http(&base, "GET", &path, Some(&good), "").await;
    assert_eq!(status, 200, "{body}");
    assert!(body.contains(r#""head":0"#));

    // The same header again is a replay.
    let (status, _) = http(&base, "GET", &path, Some(&good), "").await;
    assert_eq!(status, 401);

    // Signed for another URL, or for another method.
    let other = auth(&owner(), &format!("{base}/v1/epoch-keys"), HttpMethod::GET).await;
    assert_eq!(http(&base, "GET", &path, Some(&other), "").await.0, 401);
    let post = auth(&owner(), &url, HttpMethod::POST).await;
    assert_eq!(http(&base, "GET", &path, Some(&post), "").await.0, 401);

    // Expired: signed now, checked 61 seconds later.
    let late = auth(&owner(), &url, HttpMethod::GET).await;
    w.clock.advance_secs(61);
    assert_eq!(http(&base, "GET", &path, Some(&late), "").await.0, 401);

    // No header at all.
    assert_eq!(http(&base, "GET", &path, None, "").await.0, 401);
    w.finish().await;
}

#[tokio::test]
async fn the_http_api_accepts_proposals_and_serves_keys_and_rejections() {
    let w = World::new().await;
    let base = w.serve().await;
    let genesis = w.proposal(
        &owner(),
        None,
        common::genesis(w.business, backup_recovery()),
    );
    let (status, body) = http(&base, "POST", "/v1/proposals", None, &genesis.as_json()).await;
    assert_eq!(status, 200);
    assert!(body.starts_with(r#"{"status":"accepted""#), "{body}");
    // Not an event, or a broken signature: 400, no signed answer.
    assert_eq!(
        http(&base, "POST", "/v1/proposals", None, "{}").await.0,
        400
    );
    let mut forged: serde_json::Value = serde_json::from_str(&genesis.as_json()).unwrap();
    forged["created_at"] = 1.into();
    assert_eq!(
        http(&base, "POST", "/v1/proposals", None, &forged.to_string())
            .await
            .0,
        400
    );
    // Too big a body.
    let huge = "x".repeat(ledger_service::http::MAX_BODY_BYTES + 1);
    assert_eq!(
        http(&base, "POST", "/v1/proposals", None, &huge).await.0,
        413
    );

    let keys_url = format!("{base}/v1/epoch-keys?business={}", w.business);
    let a = auth(&owner(), &keys_url, HttpMethod::GET).await;
    let (status, body) = http(
        &base,
        "GET",
        &format!("/v1/epoch-keys?business={}", w.business),
        Some(&a),
        "",
    )
    .await;
    assert_eq!(status, 200);
    let value: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(value["deliveries"].as_array().unwrap().len(), 1);

    let stale = w.proposal(&owner(), Some(5), approve("draft-1"));
    let (_, body) = http(&base, "POST", "/v1/proposals", None, &stale.as_json()).await;
    assert!(body.starts_with(r#"{"status":"rejected""#), "{body}");
    let r = auth(&owner(), &format!("{base}/v1/rejections"), HttpMethod::GET).await;
    let (status, body) = http(&base, "GET", "/v1/rejections", Some(&r), "").await;
    assert_eq!(status, 200);
    let value: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(value["rejections"].as_array().unwrap().len(), 1);
    w.finish().await;
}
