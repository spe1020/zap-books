//! The service never logs a receipt's key, nonce or contents (receipt
//! storage brief, a merge gate). Every log line, at every level and from
//! every crate, is captured during a full round trip, including refusals,
//! and searched for them. Its own test binary, since it installs the
//! process's global log subscriber.

mod common;

use std::io::Write;
use std::sync::{Arc, Mutex};

use base64::Engine;
use common::*;
use ledger_core::StoredFile;
use ledger_service::blobs::{self, DEFAULT_QUOTA};
use nostr::nips::nip98::{HttpData, HttpMethod};
use nostr::prelude::Keys;
use nostr::types::url::Url;
use protocol::{Command, DocumentRef, History, MembershipChange, Role};
use serde_json::Value;

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

async fn token(base: &str, keys: &Keys) -> String {
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

async fn put(base: &str, token: &str, w: &World, hash: &str, bytes: &[u8]) -> u16 {
    let auth = format!("Bearer {token}");
    let path = format!("/v1/blobs/{hash}?business={}", w.business);
    let headers = [
        ("Authorization", auth.as_str()),
        ("Content-Type", "application/octet-stream"),
    ];
    http_bytes(base, "PUT", &path, &headers, bytes).await.0
}

#[tokio::test]
async fn no_key_nonce_or_contents_reaches_the_logs() {
    let captured = Captured::default();
    let writer = captured.clone();
    tracing_subscriber::fmt()
        .json()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(move || writer.clone())
        .init();

    let w = World::with_blobs(DEFAULT_QUOTA, true).await;
    w.create().await;
    let bookkeeper = key(10);
    let viewer = key(11);
    for (keys, role) in [(&bookkeeper, Role::Bookkeeper), (&viewer, Role::Viewer)] {
        let add = MembershipChange::Add {
            member: keys.public_key(),
            role,
            history: History::Full,
        };
        acceptance_event(&w.change(&owner(), add).await);
    }
    let base = w.serve().await;

    // Invented ciphertext, key and nonce, each distinctive enough to find.
    let bytes: Vec<u8> = (0..3000u32).map(|i| (i * 89 % 251) as u8).collect();
    let hash = blobs::hash(&bytes);
    let stored = StoredFile {
        hash: hash.clone(),
        size: bytes.len() as u64,
        alg: "AES-256-GCM".into(),
        key: "S2V5Rm9yQW5JbnZlbnRlZFJlY2VpcHRPbmx5MTIzNDU=".into(),
        nonce: "SW52ZW50ZWROb25j".into(),
    };
    let document = DocumentRef {
        id: "doc-1".into(),
        sha256: "5e".repeat(32),
        name: "receipt.pdf".into(),
        mime: "application/pdf".into(),
        size: bytes.len() as u64 - 16,
        stored: Some(stored.clone()),
    };
    let save = |id: &str, doc: DocumentRef| {
        let mut d = draft(w.business, id);
        d.document_ids = vec![doc.id.clone()];
        d.duplicate_reason = "A separate invented purchase.".into();
        Command::SaveDraft {
            draft: d,
            documents: vec![doc],
            extraction: None,
        }
    };

    let t = token(&base, &bookkeeper).await;
    // Refusals first: a mismatched upload and a proposal for a blob that
    // isn't there yet.
    assert_eq!(put(&base, &t, &w, &"ab".repeat(32), &bytes).await, 400);
    let head = w.head().await;
    rejection(
        &w.send(&bookkeeper, Some(head), save("draft-0", document.clone()))
            .await,
        &bookkeeper,
    );
    // Then the round trip: upload, propose, download.
    assert_eq!(put(&base, &t, &w, &hash, &bytes).await, 200);
    assert_eq!(put(&base, &t, &w, &hash, &bytes).await, 200);
    let head = w.head().await;
    acceptance_event(
        &w.send(&bookkeeper, Some(head), save("draft-1", document.clone()))
            .await,
    );
    let auth = format!("Bearer {}", token(&base, &viewer).await);
    let path = format!("/v1/blobs/{hash}?business={}", w.business);
    let (status, _, body) = http_bytes(&base, "GET", &path, &[("Authorization", &auth)], b"").await;
    assert_eq!(status, 200);
    assert_eq!(body, bytes);
    w.finish().await;

    let logs = String::from_utf8_lossy(&captured.0.lock().unwrap()).into_owned();
    // The capture works: the service logged the upload and the acceptance.
    assert!(logs.contains("blob stored"), "{logs}");
    assert!(logs.contains("\"accepted\""), "{logs}");
    let b64 = base64::engine::general_purpose::STANDARD;
    let secret_parts = [
        stored.key.clone(),
        stored.nonce.clone(),
        // The decoded key and nonce, as hex and as text.
        hex::encode(b64.decode(&stored.key).unwrap()),
        hex::encode(b64.decode(&stored.nonce).unwrap()),
        String::from_utf8(b64.decode(&stored.key).unwrap()).unwrap(),
        String::from_utf8(b64.decode(&stored.nonce).unwrap()).unwrap(),
        // Any 24 bytes of the contents, as hex or base64.
        hex::encode(&bytes[..24]),
        hex::encode(&bytes[1000..1024]),
        b64.encode(&bytes[..24]),
        b64.encode(&bytes[1200..1224]),
    ];
    for part in secret_parts {
        assert!(!logs.contains(&part), "the logs contain {part}");
    }
    // Nor the contents as an escaped byte list.
    let listed = format!("{:?}", &bytes[..12]);
    assert!(!logs.contains(listed.trim_matches(['[', ']'])));
}
