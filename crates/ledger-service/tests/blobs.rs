//! The blob store (receipt storage brief, PR 1): uploads, the proposal
//! checks, downloads, cleanup and the quota. Every one is a merge gate.
//! All bytes, keys and nonces are invented; the service never decrypts a
//! blob, so the ciphertext here is arbitrary bytes.

mod common;

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

use common::*;
use ledger_core::StoredFile;
use ledger_service::blobs::{self, MAX_BLOB_BYTES, PENDING_SECS};
use ledger_service::ledger::BLOB_NOT_UPLOADED;
use nostr::nips::nip98::{HttpData, HttpMethod};
use nostr::prelude::Keys;
use nostr::types::url::Url;
use protocol::{BusinessId, Command, DocumentRef, History, MembershipChange, Role};
use serde_json::Value;
use tokio::io::AsyncWriteExt;

fn bookkeeper() -> Keys {
    key(10)
}
fn viewer() -> Keys {
    key(11)
}
fn submitter() -> Keys {
    key(12)
}
fn assistant() -> Keys {
    key(13)
}
/// The owner of another business on the same service.
fn elsewhere() -> Keys {
    key(20)
}

/// Invented ciphertext of `len` bytes; `seed` makes each blob different.
fn ciphertext(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect()
}

fn stored_file(bytes: &[u8]) -> StoredFile {
    StoredFile {
        hash: blobs::hash(bytes),
        size: bytes.len() as u64,
        alg: "AES-256-GCM".into(),
        key: format!("{}w=", "q".repeat(42)),
        nonce: "Zm9yZXhhbXBsZTEy".into(),
    }
}

/// A document whose stored copy is `bytes`.
fn document(id: &str, bytes: &[u8]) -> DocumentRef {
    DocumentRef {
        id: id.into(),
        sha256: "5e".repeat(32),
        name: "receipt.pdf".into(),
        mime: "application/pdf".into(),
        size: bytes.len() as u64 - 16,
        stored: Some(stored_file(bytes)),
    }
}

/// A draft for `doc`, marked as a separate purchase so that drafts with the
/// same invented details never count as duplicates of each other.
fn draft_with(w: &World, id: &str, doc: &DocumentRef) -> ledger_core::ExpenseDraft {
    let mut d = draft(w.business, id);
    d.document_ids = vec![doc.id.clone()];
    d.duplicate_reason = "A separate invented purchase.".into();
    d
}

fn save_with(w: &World, id: &str, doc: DocumentRef) -> Command {
    Command::SaveDraft {
        draft: draft_with(w, id, &doc),
        documents: vec![doc],
        extraction: None,
    }
}

fn add(keys: &Keys, role: Role) -> MembershipChange {
    MembershipChange::Add {
        member: keys.public_key(),
        role,
        history: History::Full,
    }
}

/// A business with a bookkeeper, a viewer, a submitter and an assistant,
/// served over HTTP.
async fn staffed(w: &World) -> String {
    w.create().await;
    for (keys, role) in [
        (bookkeeper(), Role::Bookkeeper),
        (viewer(), Role::Viewer),
        (submitter(), Role::Submitter),
        (assistant(), Role::Assistant),
    ] {
        acceptance_event(&w.change(&owner(), add(&keys, role)).await);
    }
    w.serve().await
}

/// Creates another business, owned by [`elsewhere`].
async fn other_business(w: &World) -> BusinessId {
    let other = BusinessId::generate();
    let genesis = World::proposal_for(other, &elsewhere(), None, genesis(other, backup_recovery()));
    acceptance_event(&w.ledger.submit(&genesis).await.unwrap());
    other
}

/// A session token for `keys`, made once per service: a key signing in
/// twice within a second would sign the same NIP-98 event, a replay.
async fn token(base: &str, keys: &Keys) -> String {
    static TOKENS: Mutex<BTreeMap<(String, String), String>> = Mutex::new(BTreeMap::new());
    let id = (base.to_string(), keys.public_key().to_hex());
    if let Some(t) = TOKENS.lock().unwrap().get(&id) {
        return t.clone();
    }
    let t = sign_in(base, keys).await;
    TOKENS.lock().unwrap().insert(id, t.clone());
    t
}

async fn sign_in(base: &str, keys: &Keys) -> String {
    let url = format!("{base}/v1/session");
    let header = HttpData::new(Url::parse(&url).unwrap(), HttpMethod::POST)
        .to_authorization(keys)
        .await
        .unwrap();
    let (status, body) = http(base, "POST", "/v1/session", Some(&header), "").await;
    assert_eq!(status, 200, "{body}");
    let body: Value = serde_json::from_str(&body).unwrap();
    body["token"].as_str().unwrap().to_string()
}

fn path(business: &BusinessId, hash: &str) -> String {
    format!("/v1/blobs/{hash}?business={business}")
}

/// `PUT` of `bytes` to `hash`: the status and the JSON answer.
async fn put_at(
    base: &str,
    token: &str,
    business: &BusinessId,
    hash: &str,
    bytes: &[u8],
) -> (u16, Value) {
    let auth = format!("Bearer {token}");
    let (status, _, body) = http_bytes(
        base,
        "PUT",
        &path(business, hash),
        &[
            ("Authorization", &auth),
            ("Content-Type", "application/octet-stream"),
        ],
        bytes,
    )
    .await;
    (status, serde_json::from_slice(&body).unwrap())
}

async fn put(base: &str, token: &str, business: &BusinessId, bytes: &[u8]) -> (u16, Value) {
    put_at(base, token, business, &blobs::hash(bytes), bytes).await
}

/// `GET` of `hash`: the status, the response head and the body.
async fn get(base: &str, token: &str, business: &BusinessId, hash: &str) -> (u16, String, Vec<u8>) {
    let auth = format!("Bearer {token}");
    http_bytes(
        base,
        "GET",
        &path(business, hash),
        &[("Authorization", &auth)],
        b"",
    )
    .await
}

async fn rows(w: &World) -> Vec<(String, String, i64, String)> {
    sqlx::query_as("select business_id, hash, size, state from blobs order by created_at, hash")
        .fetch_all(&w.pool)
        .await
        .unwrap()
}

async fn state(w: &World, hash: &str) -> Option<String> {
    sqlx::query_scalar("select state from blobs where hash = $1")
        .bind(hash)
        .fetch_optional(&w.pool)
        .await
        .unwrap()
}

/// Every file in the business's blob directory, temporary ones included.
fn files(w: &World, business: &BusinessId) -> Vec<String> {
    let dir = w.blob_dir.join(business.to_string());
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|d| {
            d.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

async fn send(w: &World, signer: &Keys, command: Command) -> protocol::Rejection {
    let head = w.head().await;
    rejection(&w.send(signer, Some(head), command).await, signer)
}

async fn accept(w: &World, signer: &Keys, command: Command) {
    let head = w.head().await;
    acceptance_event(&w.send(signer, Some(head), command).await);
}

#[tokio::test]
async fn a_receipt_goes_up_is_referenced_and_comes_down_to_another_member() {
    let w = World::new().await;
    let base = staffed(&w).await;
    let bytes = ciphertext(4112, 1);
    let hash = blobs::hash(&bytes);
    let (status, body) = put(
        &base,
        &token(&base, &bookkeeper()).await,
        &w.business,
        &bytes,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["created"], true);
    assert_eq!(body["size"], 4112);
    assert_eq!(state(&w, &hash).await.as_deref(), Some("pending"));

    // Pending: no accepted record refers to it yet, so no one gets it.
    let reader = token(&base, &viewer()).await;
    assert_eq!(get(&base, &reader, &w.business, &hash).await.0, 404);

    accept(
        &w,
        &bookkeeper(),
        save_with(&w, "draft-1", document("doc-1", &bytes)),
    )
    .await;
    assert_eq!(state(&w, &hash).await.as_deref(), Some("referenced"));
    // The books carry the key; the service stored only ciphertext.
    let books = w.snapshot().await;
    assert_eq!(books.documents[0].stored, Some(stored_file(&bytes)));
    let on_disk = std::fs::read(w.blob_dir.join(w.business.to_string()).join(&hash)).unwrap();
    assert_eq!(on_disk, bytes);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let file = w.blob_dir.join(w.business.to_string()).join(&hash);
        let mode = std::fs::metadata(file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    // Every member who reads the books gets the same bytes back.
    for keys in [viewer(), owner(), assistant(), bookkeeper()] {
        let (status, head, body) = get(&base, &token(&base, &keys).await, &w.business, &hash).await;
        assert_eq!(status, 200);
        assert_eq!(body, bytes);
        let head = head.to_ascii_lowercase();
        assert!(
            head.contains("content-type: application/octet-stream"),
            "{head}"
        );
        assert!(head.contains("x-content-type-options: nosniff"), "{head}");
        assert!(head.contains("content-disposition: attachment"), "{head}");
    }
    w.finish().await;
}

#[tokio::test]
async fn an_upload_whose_body_doesnt_match_its_address_is_refused() {
    let w = World::new().await;
    let base = staffed(&w).await;
    let bytes = ciphertext(2064, 2);
    let other = blobs::hash(&ciphertext(2064, 3));
    let t = token(&base, &bookkeeper()).await;
    let (status, body) = put_at(&base, &t, &w.business, &other, &bytes).await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["code"], "hashMismatch");
    // An empty body has its own refusal.
    let (status, body) = put(&base, &t, &w.business, b"").await;
    assert_eq!((status, &body["code"]), (400, &Value::from("empty")));
    // A malformed address is refused before anything else.
    let (status, _) = put_at(&base, &t, &w.business, &"AB".repeat(32), &bytes).await;
    assert_eq!(status, 400);
    assert!(rows(&w).await.is_empty());
    assert!(files(&w, &w.business).is_empty());
    w.finish().await;
}

#[tokio::test]
async fn an_oversized_upload_is_refused_before_its_body_is_read() {
    let w = World::new().await;
    let base = staffed(&w).await;
    let t = token(&base, &bookkeeper()).await;
    let auth = format!("Bearer {t}");
    let hash = "ab".repeat(32);
    let host = base.trim_start_matches("http://").to_string();

    // A declared length over the limit: answered at once, with only a few
    // bytes of the body ever sent.
    let too_long = (MAX_BLOB_BYTES + 1).to_string();
    let answer = tokio::time::timeout(
        Duration::from_secs(10),
        http_bytes(
            &base,
            "PUT",
            &path(&w.business, &hash),
            &[
                ("Authorization", &auth),
                ("Content-Type", "application/octet-stream"),
                ("Content-Length", &too_long),
            ],
            b"only a few bytes",
        ),
    )
    .await
    .expect("answered without waiting for the body");
    assert_eq!(answer.0, 413);

    // No declared length: refused once the limit is passed, although the
    // body never ends.
    let mut stream = tokio::net::TcpStream::connect(&host).await.unwrap();
    let head = format!(
        "PUT {} HTTP/1.1\r\nHost: {host}\r\nAuthorization: {auth}\r\nContent-Type: application/octet-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
        path(&w.business, &hash)
    );
    stream.write_all(head.as_bytes()).await.unwrap();
    let piece = vec![7u8; 64 * 1024];
    let mut sent = 0u64;
    while sent <= MAX_BLOB_BYTES {
        let chunk = format!("{:x}\r\n", piece.len());
        if stream.write_all(chunk.as_bytes()).await.is_err()
            || stream.write_all(&piece).await.is_err()
            || stream.write_all(b"\r\n").await.is_err()
        {
            break;
        }
        sent += piece.len() as u64;
    }
    let (status, _, _) = tokio::time::timeout(Duration::from_secs(10), read_response(&mut stream))
        .await
        .expect("answered without the body's end");
    assert_eq!(status, 413);
    assert!(rows(&w).await.is_empty());
    assert!(files(&w, &w.business).is_empty());
    w.finish().await;
}

#[tokio::test]
async fn owners_bookkeepers_assistants_and_submitters_upload_and_no_one_else() {
    let w = World::new().await;
    let base = staffed(&w).await;
    let other = other_business(&w).await;
    for (n, keys) in [owner(), bookkeeper(), assistant(), submitter()]
        .iter()
        .enumerate()
    {
        let bytes = ciphertext(100, 10 + n as u8);
        let (status, body) = put(&base, &token(&base, keys).await, &w.business, &bytes).await;
        assert_eq!(status, 200, "{body}");
    }
    let bytes = ciphertext(100, 30);
    // A viewer, the recovery key, and a member of another business.
    for keys in [viewer(), backup(), elsewhere()] {
        let (status, body) = put(&base, &token(&base, &keys).await, &w.business, &bytes).await;
        assert_eq!(status, 404, "{body}");
    }
    // Not with NIP-98, nor without a session, nor as anything but bytes.
    let (status, _, _) = http_bytes(
        &base,
        "PUT",
        &path(&w.business, &blobs::hash(&bytes)),
        &[("Content-Type", "application/octet-stream")],
        &bytes,
    )
    .await;
    assert_eq!(status, 401);
    let t = token(&base, &owner()).await;
    let auth = format!("Bearer {t}");
    let (status, _, _) = http_bytes(
        &base,
        "PUT",
        &path(&w.business, &blobs::hash(&bytes)),
        &[("Authorization", &auth), ("Content-Type", "image/png")],
        &bytes,
    )
    .await;
    assert_eq!(status, 415);
    // A removed member is cut off at once.
    let t = token(&base, &bookkeeper()).await;
    acceptance_event(
        &w.change(
            &owner(),
            MembershipChange::Remove {
                member: bookkeeper().public_key(),
            },
        )
        .await,
    );
    assert_eq!(put(&base, &t, &w.business, &bytes).await.0, 401);
    // The other business's owner uploads to their own.
    let (status, _) = put(&base, &token(&base, &elsewhere()).await, &other, &bytes).await;
    assert_eq!(status, 200);
    assert_eq!(rows(&w).await.len(), 5);
    w.finish().await;
}

#[tokio::test]
async fn downloads_are_only_for_members_who_read_the_books() {
    let w = World::new().await;
    let base = staffed(&w).await;
    let other = other_business(&w).await;
    let bytes = ciphertext(512, 4);
    let hash = blobs::hash(&bytes);
    put(
        &base,
        &token(&base, &bookkeeper()).await,
        &w.business,
        &bytes,
    )
    .await;
    accept(
        &w,
        &bookkeeper(),
        save_with(&w, "draft-1", document("doc-1", &bytes)),
    )
    .await;
    let viewers_token = token(&base, &viewer()).await;
    assert_eq!(get(&base, &viewers_token, &w.business, &hash).await.0, 200);

    // A submitter, the recovery key, and a member of another business.
    for keys in [submitter(), backup(), elsewhere()] {
        let t = token(&base, &keys).await;
        assert_eq!(get(&base, &t, &w.business, &hash).await.0, 404);
    }
    // Nor under another business's id, even by its own owner.
    let t = token(&base, &elsewhere()).await;
    assert_eq!(get(&base, &t, &other, &hash).await.0, 404);
    // A non-member has no session to send, and NIP-98 isn't accepted here.
    assert_eq!(
        get(&base, &"00".repeat(32), &w.business, &hash).await.0,
        401
    );
    let url = format!("{base}{}", path(&w.business, &hash));
    let signed = HttpData::new(Url::parse(&url).unwrap(), HttpMethod::GET)
        .to_authorization(&viewer())
        .await
        .unwrap();
    let (status, _, _) = http_bytes(
        &base,
        "GET",
        &path(&w.business, &hash),
        &[("Authorization", &signed)],
        b"",
    )
    .await;
    assert_eq!(status, 401);

    // A removed member, at once, with the token they already had.
    acceptance_event(
        &w.change(
            &owner(),
            MembershipChange::Remove {
                member: viewer().public_key(),
            },
        )
        .await,
    );
    assert_eq!(get(&base, &viewers_token, &w.business, &hash).await.0, 401);
    // Demoted to submitter: still a member, but no longer a reader.
    let t = token(&base, &assistant()).await;
    acceptance_event(
        &w.change(
            &owner(),
            MembershipChange::ChangeRole {
                member: assistant().public_key(),
                role: Role::Submitter,
            },
        )
        .await,
    );
    assert_eq!(get(&base, &t, &w.business, &hash).await.0, 404);
    w.finish().await;
}

#[tokio::test]
async fn a_proposal_needs_its_blob_uploaded_to_this_business_at_its_size() {
    let w = World::new().await;
    let base = staffed(&w).await;
    let other = other_business(&w).await;
    let here = ciphertext(300, 5);
    let there = ciphertext(300, 6);
    put(
        &base,
        &token(&base, &bookkeeper()).await,
        &w.business,
        &here,
    )
    .await;
    put(&base, &token(&base, &elsewhere()).await, &other, &there).await;
    let never = ciphertext(300, 7);
    let mut wrong_size = document("doc-x", &here);
    wrong_size.stored.as_mut().unwrap().size += 1;
    let refused = [
        ("not uploaded", document("doc-x", &never)),
        ("uploaded to another business", document("doc-x", &there)),
        ("the wrong size", wrong_size),
    ];

    // A save, a submitted receipt and a correction are all checked.
    accept(&w, &bookkeeper(), save(w.business, "posted")).await;
    accept(&w, &owner(), approve("posted")).await;
    let correct = |doc: DocumentRef| Command::Correct {
        original_id: "posted".into(),
        replacement: draft_with(&w, "replacement", &doc),
        reason: "The invented receipt was replaced.".into(),
        documents: vec![doc],
        extraction: None,
    };
    let submit = |doc: DocumentRef| Command::SubmitReceiptDraft {
        draft: draft_with(&w, "submitted", &doc),
        documents: vec![doc],
    };
    for (case, doc) in &refused {
        let save = save_with(&w, "draft-x", doc.clone());
        let cases = [
            (bookkeeper(), save),
            (submitter(), submit(doc.clone())),
            (bookkeeper(), correct(doc.clone())),
        ];
        for (signer, command) in cases {
            let kind = command.command_type().as_str();
            let r = send(&w, &signer, command).await;
            assert_eq!(r.reason.message, BLOB_NOT_UPLOADED, "{kind}: {case}");
        }
    }
    assert!(w.snapshot().await.documents.is_empty());
    let hash = blobs::hash(&here);
    assert_eq!(state(&w, &hash).await.as_deref(), Some("pending"));
    // A blob uploaded here at its size is accepted in each.
    accept(&w, &submitter(), submit(document("doc-1", &here))).await;
    assert_eq!(state(&w, &hash).await.as_deref(), Some("referenced"));
    accept(
        &w,
        &bookkeeper(),
        save_with(&w, "draft-2", document("doc-2", &here)),
    )
    .await;
    accept(&w, &bookkeeper(), correct(document("doc-3", &here))).await;
    let documents = w.snapshot().await.documents;
    assert_eq!(documents.len(), 3);
    assert!(documents
        .iter()
        .all(|d| d.stored == Some(stored_file(&here))));
    // The other business's blob is untouched.
    let there_state: String = sqlx::query_scalar("select state from blobs where business_id = $1")
        .bind(other.to_string())
        .fetch_one(&w.pool)
        .await
        .unwrap();
    assert_eq!(there_state, "pending");
    w.finish().await;
}

#[tokio::test]
async fn an_unreferenced_blob_goes_after_24_hours_and_a_referenced_one_never() {
    let w = World::new().await;
    let base = staffed(&w).await;
    let t = token(&base, &bookkeeper()).await;
    let left = ciphertext(200, 8);
    let kept = ciphertext(200, 9);
    put(&base, &t, &w.business, &left).await;
    put(&base, &t, &w.business, &kept).await;
    accept(
        &w,
        &bookkeeper(),
        save_with(&w, "draft-1", document("doc-1", &kept)),
    )
    .await;
    assert_eq!(files(&w, &w.business).len(), 2);

    w.clock.advance_secs(PENDING_SECS - 1);
    assert_eq!(w.ledger.clean_blobs().await.unwrap(), 0);
    w.clock.advance_secs(1);
    assert_eq!(w.ledger.clean_blobs().await.unwrap(), 1);
    assert_eq!(state(&w, &blobs::hash(&left)).await, None);
    assert_eq!(files(&w, &w.business), vec![blobs::hash(&kept)]);

    // Ten years on, the referenced blob is still there.
    w.clock.advance_secs(10 * 365 * 24 * 60 * 60);
    assert_eq!(w.ledger.clean_blobs().await.unwrap(), 0);
    assert_eq!(
        state(&w, &blobs::hash(&kept)).await.as_deref(),
        Some("referenced")
    );
    assert_eq!(files(&w, &w.business), vec![blobs::hash(&kept)]);
    // A proposal can no longer refer to the deleted one.
    let r = send(
        &w,
        &bookkeeper(),
        save_with(&w, "draft-2", document("doc-2", &left)),
    )
    .await;
    assert_eq!(r.reason.message, BLOB_NOT_UPLOADED);
    w.finish().await;
}

#[tokio::test]
async fn the_same_blob_uploaded_twice_is_stored_once() {
    let w = World::new().await;
    let base = staffed(&w).await;
    let bytes = ciphertext(1000, 11);
    let (status, first) = put(
        &base,
        &token(&base, &bookkeeper()).await,
        &w.business,
        &bytes,
    )
    .await;
    assert_eq!((status, &first["created"]), (200, &Value::Bool(true)));
    let (status, again) = put(
        &base,
        &token(&base, &submitter()).await,
        &w.business,
        &bytes,
    )
    .await;
    assert_eq!((status, &again["created"]), (200, &Value::Bool(false)));
    assert_eq!(rows(&w).await.len(), 1);
    // One file, and no temporary one left behind.
    assert_eq!(files(&w, &w.business), vec![blobs::hash(&bytes)]);
    w.finish().await;
}

#[tokio::test]
async fn the_quota_is_enforced() {
    let w = World::with_blobs(3000, false).await;
    let base = staffed(&w).await;
    let t = token(&base, &bookkeeper()).await;
    let first = ciphertext(2000, 12);
    assert_eq!(put(&base, &t, &w.business, &first).await.0, 200);
    let (status, body) = put(&base, &t, &w.business, &ciphertext(1001, 13)).await;
    assert_eq!(status, 507, "{body}");
    assert_eq!(body["code"], "quota");
    assert_eq!(
        body["error"],
        "This business's receipt storage is full: its limit is 3000 bytes."
    );
    // Up to the limit exactly is fine, and a blob already stored costs
    // nothing more.
    assert_eq!(
        put(&base, &t, &w.business, &ciphertext(1000, 14)).await.0,
        200
    );
    assert_eq!(put(&base, &t, &w.business, &first).await.0, 200);
    assert_eq!(rows(&w).await.len(), 2);
    assert_eq!(files(&w, &w.business).len(), 2);
    w.finish().await;
}
