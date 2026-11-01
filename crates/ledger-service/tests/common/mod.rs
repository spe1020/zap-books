//! Test setup: a fresh Postgres database per test (from `DATABASE_URL`), a
//! ledger with a hand-set clock, fixed keys, and a fictional business.
//!
//! Time-based tests move the hand-set clock and call the scheduler's work
//! (`Ledger::activate_due`) directly: nothing waits.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use ledger_core::{blank_draft, BusinessData, ExpenseDraft, Minor, SequentialEnv};
use ledger_service::blobs::{BlobStore, DEFAULT_QUOTA};
use ledger_service::clock::{uuid, Clock, FixedClock};
use ledger_service::http::{router, AppState};
use ledger_service::ratelimit::RateLimit;
use ledger_service::{Acknowledgment, Ledger, LedgerError};
use nostr::prelude::{Event, Keys, PublicKey, SecretKey, Timestamp};
use protocol::{
    build_proposal, open_acceptance, open_delivery, open_rejection, AcceptanceContent, BusinessId,
    Command, EpochKey, MembershipChange, MembershipState, ProposalPayload, Recovery, Rejection,
};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{ConnectOptions, PgPool};

/// Seconds since the Unix epoch, now: NIP-98 events are signed with the
/// real clock, so the service's clock starts there too.
pub fn real_now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

pub fn key(n: u8) -> Keys {
    Keys::new(SecretKey::from_slice(&[n; 32]).unwrap())
}
pub fn service_keys() -> Keys {
    key(1)
}
/// A second key the service holds, for businesses moving to a new key.
pub fn next_service_keys() -> Keys {
    key(5)
}
/// The keys a reader accepts deliveries and rejections from.
pub fn is_service(key: &PublicKey) -> bool {
    *key == service_keys().public_key() || *key == next_service_keys().public_key()
}
pub fn owner() -> Keys {
    key(2)
}
pub fn backup() -> Keys {
    key(3)
}
pub fn second_owner() -> Keys {
    key(4)
}
pub fn outsider() -> Keys {
    key(7)
}

pub struct World {
    pub ledger: Arc<Ledger>,
    pub clock: Arc<FixedClock>,
    pub pool: PgPool,
    pub admin: PgPool,
    pub name: String,
    pub business: BusinessId,
    /// The blob store's directory, removed by [`World::finish`].
    pub blob_dir: PathBuf,
}

impl World {
    pub async fn new() -> World {
        World::with_blobs(DEFAULT_QUOTA, false).await
    }

    /// A world whose blob store has `quota` bytes per business, and whose
    /// database connections log their statements if `statement_logging`.
    pub async fn with_blobs(quota: u64, statement_logging: bool) -> World {
        let url = std::env::var("DATABASE_URL").expect(
            "The ledger-service tests need Postgres: set DATABASE_URL, for example \
             postgres://postgres@127.0.0.1:5432/postgres",
        );
        let admin_options = PgConnectOptions::from_str(&url).unwrap();
        let admin = PgPoolOptions::new()
            .max_connections(2)
            .connect_with(admin_options.clone())
            .await
            .unwrap();
        let name = format!("zb_test_{}", &uuid().replace('-', "")[..16]);
        sqlx::query(sqlx::AssertSqlSafe(format!("create database {name}")))
            .execute(&admin)
            .await
            .unwrap();
        let options = admin_options.database(&name);
        let options = if statement_logging {
            options
        } else {
            options.disable_statement_logging()
        };
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect_with(options)
            .await
            .unwrap();
        Ledger::migrate(&pool).await.unwrap();
        let clock = Arc::new(FixedClock::new(real_now_millis()));
        let blob_dir = std::env::temp_dir().join(&name);
        let ledger = Arc::new(
            Ledger::new(pool.clone(), service_keys(), clock.clone(), Arc::new(uuid))
                .with_key(next_service_keys())
                .with_blobs(BlobStore::open(&blob_dir, quota).unwrap()),
        );
        World {
            ledger,
            clock,
            pool,
            admin,
            name,
            business: BusinessId::generate(),
            blob_dir,
        }
    }

    /// Drops the test database and the blob store.
    pub async fn finish(self) {
        let _ = std::fs::remove_dir_all(&self.blob_dir);
        self.pool.close().await;
        let _ = sqlx::query(sqlx::AssertSqlSafe(format!(
            "drop database if exists {} with (force)",
            self.name
        )))
        .execute(&self.admin)
        .await;
    }

    pub fn proposal(&self, signer: &Keys, base_seq: Option<u64>, command: Command) -> Event {
        self.proposal_to(&service_keys().public_key(), signer, base_seq, command)
    }

    /// A proposal for any business id, to the primary key, signed now.
    pub fn proposal_for(
        business: BusinessId,
        signer: &Keys,
        base_seq: Option<u64>,
        command: Command,
    ) -> Event {
        let payload = ProposalPayload::new(business, base_seq, command);
        let at = Timestamp::from_secs(real_now_millis() / 1000);
        build_proposal(signer, &service_keys().public_key(), &payload, at).unwrap()
    }

    /// A proposal encrypted to `service`.
    pub fn proposal_to(
        &self,
        service: &PublicKey,
        signer: &Keys,
        base_seq: Option<u64>,
        command: Command,
    ) -> Event {
        let payload = ProposalPayload::new(self.business, base_seq, command);
        let at = Timestamp::from_secs(self.clock.now_millis() / 1000);
        build_proposal(signer, service, &payload, at).unwrap()
    }

    /// Proposes a membership change from the current head.
    pub async fn change(&self, signer: &Keys, change: MembershipChange) -> Acknowledgment {
        let head = self.head().await;
        self.send(signer, Some(head), Command::ChangeMembership { change })
            .await
    }

    /// Runs the scheduler's work once at the clock's current time.
    pub async fn run_due(&self) -> usize {
        self.ledger.activate_due().await.unwrap()
    }

    /// The membership as stored after the head acceptance.
    pub async fn membership(&self) -> MembershipState {
        let text: String =
            sqlx::query_scalar("select membership::text from businesses where id = $1")
                .bind(self.business.to_string())
                .fetch_one(&self.pool)
                .await
                .unwrap();
        serde_json::from_str(&text).unwrap()
    }

    /// The epoch keys the service delivers to `reader` now, opened.
    pub async fn keys_for(&self, reader: &Keys) -> BTreeMap<u64, EpochKey> {
        self.ledger
            .deliveries(&reader.public_key(), Some(&self.business))
            .await
            .unwrap()
            .iter()
            .map(|d| {
                let event = Event::from_json(d).unwrap();
                assert!(is_service(&event.pubkey));
                let key = open_delivery(&event, reader, &event.pubkey).unwrap().key;
                (key.epoch, key)
            })
            .collect()
    }

    /// The whole chain as stored, for checks that don't depend on who may
    /// read it.
    pub async fn chain(&self) -> Vec<Event> {
        let rows: Vec<String> =
            sqlx::query_scalar("select event from acceptances where business_id = $1 order by seq")
                .bind(self.business.to_string())
                .fetch_all(&self.pool)
                .await
                .unwrap();
        rows.iter().map(|e| Event::from_json(e).unwrap()).collect()
    }

    pub async fn send(
        &self,
        signer: &Keys,
        base_seq: Option<u64>,
        command: Command,
    ) -> Acknowledgment {
        self.try_send(signer, base_seq, command).await.unwrap()
    }

    /// As [`World::send`], for a proposal that may be refused unsigned.
    pub async fn try_send(
        &self,
        signer: &Keys,
        base_seq: Option<u64>,
        command: Command,
    ) -> Result<Acknowledgment, LedgerError> {
        let event = self.proposal(signer, base_seq, command);
        self.ledger.submit(&event).await
    }

    /// Rows stored for `author`'s proposals and rejections.
    pub async fn stored_for(&self, author: &Keys) -> (i64, i64) {
        let author = author.public_key().to_hex();
        let proposals: i64 = sqlx::query_scalar("select count(*) from proposals where author = $1")
            .bind(&author)
            .fetch_one(&self.pool)
            .await
            .unwrap();
        let audit: i64 = sqlx::query_scalar("select count(*) from audit where author = $1")
            .bind(&author)
            .fetch_one(&self.pool)
            .await
            .unwrap();
        (proposals, audit)
    }

    /// Creates the business with a backup recovery key.
    pub async fn create(&self) -> Acknowledgment {
        self.send(&owner(), None, genesis(self.business, backup_recovery()))
            .await
    }

    /// The books as stored after the head acceptance.
    pub async fn snapshot(&self) -> BusinessData {
        let text: String =
            sqlx::query_scalar("select snapshot::text from businesses where id = $1")
                .bind(self.business.to_string())
                .fetch_one(&self.pool)
                .await
                .unwrap();
        serde_json::from_str(&text).unwrap()
    }

    pub async fn head(&self) -> u64 {
        let seq: i64 = sqlx::query_scalar("select seq from businesses where id = $1")
            .bind(self.business.to_string())
            .fetch_one(&self.pool)
            .await
            .unwrap();
        seq as u64
    }

    /// Everything a reader fetches: the chain, opened with the epoch keys
    /// delivered to `reader`.
    pub async fn read(&self, reader: &Keys) -> (Vec<Event>, Vec<AcceptanceContent>) {
        let page = self
            .ledger
            .acceptances(&self.business, &reader.public_key(), None, 500)
            .await
            .unwrap()
            .expect("reader may read the books");
        let events: Vec<Event> = page
            .events
            .iter()
            .map(|e| Event::from_json(e).unwrap())
            .collect();
        let keys = self.keys_for(reader).await;
        // As a reader does: each acceptance must be signed by the key the
        // chain says signs it, from genesis or the last hand-over.
        let mut signer = service_keys().public_key();
        let mut contents = Vec::new();
        for e in &events {
            let epoch = protocol::parse_acceptance(e).unwrap().epoch;
            let (header, content) = open_acceptance(e, &keys[&epoch], &signer).unwrap();
            if let Some(next) = header.next_service {
                signer = next;
            }
            contents.push(content);
        }
        (events, contents)
    }

    /// Starts the HTTP API on a free local port, with limits no test
    /// reaches; returns its base URL.
    pub async fn serve(&self) -> String {
        self.serve_with(|_| {}).await
    }

    /// As [`World::serve`], with the state adjusted by `adjust`.
    pub async fn serve_with(&self, adjust: impl FnOnce(&mut AppState)) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let mut state = AppState::new(self.ledger.clone(), base.clone());
        state.writes = RateLimit::new(1000, 1000);
        state.reads = RateLimit::new(1000, 1000);
        state.global = RateLimit::new(10_000, 10_000);
        state.per_ip = RateLimit::new(10_000, 10_000);
        adjust(&mut state);
        let app = router(Arc::new(state)).unwrap();
        tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .await
            .unwrap();
        });
        base
    }
}

pub fn backup_recovery() -> Recovery {
    Recovery::BackupKey {
        pubkey: backup().public_key(),
    }
}

/// A fictional business's setup (the parity fixtures' history-free
/// Northstar), carrying `business`'s id.
pub fn setup(business: BusinessId) -> BusinessData {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/parity/states/northstar-setup.json");
    let mut b: BusinessData =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let id = business.to_string();
    b.profile.id = id.clone();
    b.profile.business_id = id.clone();
    for a in &mut b.accounts {
        a.business_id = id.clone();
    }
    for c in &mut b.categories {
        c.business_id = id.clone();
    }
    b
}

pub fn genesis(business: BusinessId, recovery: Recovery) -> Command {
    let b = setup(business);
    Command::CreateBusiness {
        service: service_keys().public_key(),
        recovery,
        profile: b.profile,
        accounts: b.accounts,
        categories: b.categories,
    }
}

/// A ready-to-approve invented draft.
pub fn draft(business: BusinessId, id: &str) -> ExpenseDraft {
    let b = setup(business);
    let mut env = SequentialEnv::new(1, "2023-03-01T09:00:00.000Z", "2023-03-01");
    let mut d = blank_draft(&b, &mut env);
    d.id = id.into();
    d.vendor = "Invented Vendor".into();
    d.description = "Invented supplies".into();
    d.purpose = "Office use".into();
    d.date = "2023-02-14".into();
    d.amount_minor = Minor::new(2_500).unwrap();
    d.category_id = b.categories[0].id.clone();
    d
}

pub fn save(business: BusinessId, id: &str) -> Command {
    Command::SaveDraft {
        draft: draft(business, id),
        documents: vec![],
        extraction: None,
    }
}

pub fn approve(id: &str) -> Command {
    Command::Approve {
        expense_id: id.into(),
    }
}

/// The acknowledgment's event, opened as a rejection for `author`.
pub fn rejection(ack: &Acknowledgment, author: &Keys) -> Rejection {
    let value: serde_json::Value = serde_json::from_str(&ack.text).unwrap();
    assert_eq!(value["status"], "rejected", "{}", ack.text);
    let event = Event::from_json(value["event"].to_string()).unwrap();
    assert!(is_service(&event.pubkey));
    open_rejection(&event, author, &event.pubkey).unwrap()
}

pub fn acceptance_event(ack: &Acknowledgment) -> Event {
    let value: serde_json::Value = serde_json::from_str(&ack.text).unwrap();
    assert_eq!(value["status"], "accepted", "{}", ack.text);
    Event::from_json(value["event"].to_string()).unwrap()
}

pub fn pk(keys: &Keys) -> PublicKey {
    keys.public_key()
}

/// A bare HTTP/1.1 request, so the tests need no HTTP client crate.
pub async fn http(
    base: &str,
    method: &str,
    path: &str,
    auth: Option<&str>,
    body: &str,
) -> (u16, String) {
    let auth = auth.map(|a| ("Authorization", a));
    let (status, _, body) = http_with(base, method, path, auth.as_slice(), body).await;
    (status, body)
}

/// As [`http`], with extra request headers; also returns the response's
/// head (status line and headers).
pub async fn http_with(
    base: &str,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> (u16, String, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let host = base.trim_start_matches("http://");
    let mut stream = tokio::net::TcpStream::connect(host).await.unwrap();
    let extra: String = headers
        .iter()
        .map(|(name, value)| format!("{name}: {value}\r\n"))
        .collect();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\n{extra}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    // The service may answer (a 413, say) before the body is all sent and
    // close the connection: keep whatever answer arrived, as clients do.
    let _ = stream.write_all(request.as_bytes()).await;
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 8192];
    while let Ok(n) = stream.read(&mut chunk).await {
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..n]);
    }
    let response = String::from_utf8(bytes).unwrap();
    let status = response[9..12].parse().unwrap();
    let (head, body) = response.split_once("\r\n\r\n").unwrap();
    // axum may answer with chunked encoding; small bodies come in one chunk.
    let body = if head
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        let (_, rest) = body.split_once("\r\n").unwrap();
        rest.rsplit_once("\r\n0\r\n")
            .map_or(rest, |(b, _)| b)
            .to_string()
    } else {
        body.to_string()
    };
    (status, head.to_string(), body)
}

/// As [`http_with`], for a body of any bytes with its own content type;
/// returns the response's head and its body's bytes. `body` is sent with
/// its length unless `headers` declares one.
pub async fn http_bytes(
    base: &str,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> (u16, String, Vec<u8>) {
    use tokio::io::AsyncWriteExt;
    let host = base.trim_start_matches("http://");
    let mut stream = tokio::net::TcpStream::connect(host).await.unwrap();
    let extra: String = headers
        .iter()
        .map(|(name, value)| format!("{name}: {value}\r\n"))
        .collect();
    let has_length = headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("content-length"));
    let length = if has_length {
        String::new()
    } else {
        format!("Content-Length: {}\r\n", body.len())
    };
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\n{extra}{length}Connection: close\r\n\r\n"
    );
    let _ = stream.write_all(head.as_bytes()).await;
    let _ = stream.write_all(body).await;
    read_response(&mut stream).await
}

/// Reads one response to the end of the connection: its status, head, and
/// body (de-chunked).
pub async fn read_response(stream: &mut tokio::net::TcpStream) -> (u16, String, Vec<u8>) {
    use tokio::io::AsyncReadExt;
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 65536];
    while let Ok(n) = stream.read(&mut chunk).await {
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..n]);
    }
    let split = bytes
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("a response head");
    let head = String::from_utf8(bytes[..split].to_vec()).unwrap();
    let mut body = bytes[split + 4..].to_vec();
    let status = head[9..12].parse().unwrap();
    if head
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        let mut out = Vec::new();
        let mut rest = &body[..];
        loop {
            let line = rest.windows(2).position(|w| w == b"\r\n").unwrap();
            let size =
                usize::from_str_radix(std::str::from_utf8(&rest[..line]).unwrap().trim(), 16)
                    .unwrap();
            if size == 0 {
                break;
            }
            out.extend_from_slice(&rest[line + 2..line + 2 + size]);
            rest = &rest[line + 2 + size + 2..];
        }
        body = out;
    }
    (status, head, body)
}
