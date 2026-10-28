//! Test setup: a fresh Postgres database per test (from `DATABASE_URL`), a
//! ledger with a hand-set clock, fixed keys, and a fictional business.
#![allow(dead_code)]

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use ledger_core::{blank_draft, BusinessData, ExpenseDraft, Minor, SequentialEnv};
use ledger_service::clock::{uuid, Clock, FixedClock};
use ledger_service::http::{router, AppState};
use ledger_service::ratelimit::RateLimit;
use ledger_service::{Acknowledgment, Ledger};
use nostr::prelude::{Event, Keys, PublicKey, SecretKey, Timestamp};
use protocol::{
    build_proposal, open_acceptance, open_delivery, open_rejection, AcceptanceContent, BusinessId,
    Command, ProposalPayload, Recovery, Rejection,
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
}

impl World {
    pub async fn new() -> World {
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
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect_with(admin_options.database(&name).disable_statement_logging())
            .await
            .unwrap();
        Ledger::migrate(&pool).await.unwrap();
        let clock = Arc::new(FixedClock::new(real_now_millis()));
        let ledger = Arc::new(Ledger::new(
            pool.clone(),
            service_keys(),
            clock.clone(),
            Arc::new(uuid),
        ));
        World {
            ledger,
            clock,
            pool,
            admin,
            name,
            business: BusinessId::generate(),
        }
    }

    /// Drops the test database.
    pub async fn finish(self) {
        self.pool.close().await;
        let _ = sqlx::query(sqlx::AssertSqlSafe(format!(
            "drop database if exists {} with (force)",
            self.name
        )))
        .execute(&self.admin)
        .await;
    }

    pub fn proposal(&self, signer: &Keys, base_seq: Option<u64>, command: Command) -> Event {
        let payload = ProposalPayload::new(self.business, base_seq, command);
        let at = Timestamp::from_secs(self.clock.now_millis() / 1000);
        build_proposal(signer, &service_keys().public_key(), &payload, at).unwrap()
    }

    pub async fn send(
        &self,
        signer: &Keys,
        base_seq: Option<u64>,
        command: Command,
    ) -> Acknowledgment {
        let event = self.proposal(signer, base_seq, command);
        self.ledger.submit(&event).await.unwrap()
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
        let keys: Vec<protocol::EpochKey> = self
            .ledger
            .deliveries(&reader.public_key(), Some(&self.business))
            .await
            .unwrap()
            .iter()
            .map(|d| {
                open_delivery(
                    &Event::from_json(d).unwrap(),
                    reader,
                    &service_keys().public_key(),
                )
                .unwrap()
                .key
            })
            .collect();
        let contents = events
            .iter()
            .map(|e| {
                let epoch = protocol::parse_acceptance(e).unwrap().epoch;
                let key = keys.iter().find(|k| k.epoch == epoch).unwrap();
                open_acceptance(e, key).unwrap().1
            })
            .collect();
        (events, contents)
    }

    /// Starts the HTTP API on a free local port; returns its base URL.
    pub async fn serve(&self) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let state = Arc::new(AppState {
            ledger: self.ledger.clone(),
            public_url: base.clone(),
            writes: RateLimit::new(1000, 1000),
            reads: RateLimit::new(1000, 1000),
        });
        tokio::spawn(async move {
            axum::serve(listener, router(state)).await.unwrap();
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
    open_rejection(&event, author, &service_keys().public_key()).unwrap()
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
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let host = base.trim_start_matches("http://");
    let mut stream = tokio::net::TcpStream::connect(host).await.unwrap();
    let auth = auth.map_or(String::new(), |a| format!("Authorization: {a}\r\n"));
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
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
    (status, body)
}
