//! The ledger: accepts or rejects proposals, one business at a time, and
//! serves the signed history.
//!
//! Each proposal is handled in one database transaction that holds a lock
//! on its business, so two proposals for the same business never interleave
//! and different businesses proceed in parallel. Everything a proposal
//! changes (acceptance, snapshot, record versions, acknowledgment, outbox)
//! commits together or not at all.

use std::collections::HashMap;
use std::sync::Arc;

use ledger_core::BusinessData;
use nostr::prelude::{Event, Keys, PublicKey, Timestamp};
use protocol::{
    apply_command, build_acceptance, build_delivery, build_rejection, check_base_seq, decrypt_from,
    diff, encrypt_to, genesis_business, genesis_changes, open_proposal, parse_proposal, targets,
    AcceptanceContent, AcceptanceHeader, BusinessId, Command, CommandType, CommittedPayload,
    EnvRecord, EpochKey, MembershipState, Outcome, ProposalHeader, ProtocolError, Reason,
    RecordChange, RecordKey, RecordingEnv, PROTOCOL_VERSION,
};
use serde::Serialize;
use sqlx::{PgPool, Postgres, Row, Transaction};
use thiserror::Error;

use crate::clock::{iso_millis, Clock};

/// Shown for membership commands until the service supports them (PR 3).
pub const MEMBERSHIP_LATER: &str = "Membership changes are not available yet.";

#[derive(Debug, Error)]
pub enum LedgerError {
    /// The event isn't a well-formed, validly signed proposal. It gets no
    /// signed rejection, since it may not even be from who it claims.
    #[error("{0}")]
    Invalid(ProtocolError),
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    /// A bug or corrupt stored state. Nothing was stored.
    #[error("internal error: {0}")]
    Internal(String),
}

impl LedgerError {
    fn internal(e: impl std::fmt::Display) -> LedgerError {
        LedgerError::Internal(e.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Accepted,
    Rejected,
}

/// What the service answered. `text` is stored, and returned byte for byte
/// whenever the same proposal is sent again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Acknowledgment {
    pub status: Status,
    pub text: String,
}

impl Acknowledgment {
    fn new(status: Status, event: &Event) -> Acknowledgment {
        let status_text = match status {
            Status::Accepted => "accepted",
            Status::Rejected => "rejected",
        };
        Acknowledgment {
            status,
            text: format!(
                r#"{{"status":"{status_text}","event":{}}}"#,
                event.as_json()
            ),
        }
    }
}

/// A page of the chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    /// Acceptance events, as stored, in seq order.
    pub events: Vec<String>,
    /// The chain's head seq.
    pub head: u64,
}

pub struct Ledger {
    pool: PgPool,
    keys: Keys,
    clock: Arc<dyn Clock>,
    ids: Arc<dyn Fn() -> String + Send + Sync>,
}

/// What a proposal turned into, before anything is written.
enum Decision {
    Accept(Box<Accepted>),
    Reject(Reason),
}

struct Accepted {
    event: Event,
    seq: u64,
    business: BusinessData,
    changes: Vec<RecordChange>,
    genesis: Option<Genesis>,
}

struct Genesis {
    membership: MembershipState,
    epoch: EpochKey,
    deliveries: Vec<(PublicKey, Event)>,
}

fn to_i64(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

fn to_u64(n: i64) -> u64 {
    u64::try_from(n).unwrap_or(0)
}

fn type_name(key: &RecordKey) -> String {
    serde_json::to_value(key.record_type)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_default()
}

impl Ledger {
    pub fn new(
        pool: PgPool,
        keys: Keys,
        clock: Arc<dyn Clock>,
        ids: Arc<dyn Fn() -> String + Send + Sync>,
    ) -> Ledger {
        Ledger {
            pool,
            keys,
            clock,
            ids,
        }
    }

    pub fn public_key(&self) -> PublicKey {
        self.keys.public_key()
    }

    pub fn clock(&self) -> &dyn Clock {
        self.clock.as_ref()
    }

    pub async fn migrate(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
        sqlx::migrate!("./migrations").run(pool).await
    }

    /// Accepts or rejects one proposal. Sending the same event again returns
    /// the original acknowledgment and changes nothing.
    pub async fn submit(&self, proposal: &Event) -> Result<Acknowledgment, LedgerError> {
        let header = parse_proposal(proposal).map_err(LedgerError::Invalid)?;
        let id = header.id.to_hex();
        if let Some(ack) = stored_ack(&self.pool, &id).await? {
            return Ok(ack);
        }
        let business = header.business.to_string();
        let mut tx = self.pool.begin().await?;
        // One business at a time. An advisory lock also covers genesis,
        // whose business row doesn't exist yet.
        sqlx::query("select pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(&business)
            .execute(&mut *tx)
            .await?;
        if let Some(ack) = stored_ack(&mut *tx, &id).await? {
            tx.commit().await?;
            return Ok(ack);
        }
        let ack = match self.decide(&mut tx, &header, proposal).await? {
            Decision::Accept(accepted) => {
                self.store_acceptance(&mut tx, &header, *accepted).await?
            }
            Decision::Reject(reason) => self.store_rejection(&mut tx, &header, &reason).await?,
        };
        tx.commit().await?;
        Ok(ack)
    }

    async fn decide(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        header: &ProposalHeader,
        proposal: &Event,
    ) -> Result<Decision, LedgerError> {
        let reject = |e: &ProtocolError| Ok(Decision::Reject(Reason::from_error(e)));
        let payload = match open_proposal(proposal, &self.keys) {
            Ok((_, payload)) => payload,
            Err(e) => return reject(&e),
        };
        let business_id = header.business.to_string();
        let row = sqlx::query(
            "select seq, head, epoch, snapshot::text as snapshot, membership::text as membership
             from businesses where id = $1 for update",
        )
        .bind(&business_id)
        .fetch_optional(&mut **tx)
        .await?;
        if header.command == CommandType::CreateBusiness {
            if row.is_some() {
                return reject(&ProtocolError::AlreadyCreated);
            }
            return self.decide_genesis(header, proposal, payload);
        }
        let Some(row) = row else {
            return reject(&ProtocolError::NotGenesis);
        };
        let head_seq = to_u64(row.try_get("seq")?);
        let head: String = row.try_get("head")?;
        let epoch = to_u64(row.try_get("epoch")?);
        let snapshot: BusinessData =
            serde_json::from_str(row.try_get("snapshot")?).map_err(LedgerError::internal)?;
        let membership: MembershipState =
            serde_json::from_str(row.try_get("membership")?).map_err(LedgerError::internal)?;

        let command = &payload.command;
        if let Err(e) = membership.authorize(&header.author, command) {
            return reject(&e);
        }
        if matches!(
            command,
            Command::ChangeMembership { .. } | Command::Veto { .. }
        ) {
            return Ok(Decision::Reject(Reason::rejected(MEMBERSHIP_LATER)));
        }
        let base_seq = header.base_seq.unwrap_or(0);
        let keys = targets(command, &snapshot);
        let versions = record_versions(tx, &business_id, &keys).await?;
        if let Err(e) = check_base_seq(base_seq, head_seq, &keys, |k| versions.get(k).copied()) {
            return reject(&e);
        }

        let now = self.clock.now_millis();
        let ids = self.ids.clone();
        let mut env = RecordingEnv::new(move || ids(), iso_millis(now));
        let after = match apply_command(&snapshot, command, &mut env) {
            Ok(after) => after,
            Err(refusal) => return Ok(Decision::Reject(Reason::rejected(refusal.to_string()))),
        };
        let changes = diff(&snapshot, &after).map_err(LedgerError::internal)?;
        let epoch_key = self.epoch_key(tx, &business_id, epoch).await?;
        let seq = head_seq + 1;
        let acceptance = AcceptanceHeader {
            business: header.business,
            seq,
            prev: Some(nostr::prelude::EventId::from_hex(&head).map_err(LedgerError::internal)?),
            proposal: header.id,
            epoch,
            next_service: None,
        };
        let content = AcceptanceContent {
            v: PROTOCOL_VERSION,
            proposal: proposal.clone(),
            payload,
            env: env.into_record(),
            changes: changes.clone(),
            outcome: Outcome::Applied,
        };
        let at = Timestamp::from_secs(now / 1000);
        match build_acceptance(
            &self.keys,
            &epoch_key.public_key(),
            &acceptance,
            &content,
            at,
        ) {
            Ok(event) => Ok(Decision::Accept(Box::new(Accepted {
                event,
                seq,
                business: after,
                changes,
                genesis: None,
            }))),
            // Too large, most likely: rejected before anything is stored.
            Err(e) => reject(&e),
        }
    }

    fn decide_genesis(
        &self,
        header: &ProposalHeader,
        proposal: &Event,
        payload: CommittedPayload,
    ) -> Result<Decision, LedgerError> {
        let Command::CreateBusiness {
            service, recovery, ..
        } = &payload.command
        else {
            return Ok(Decision::Reject(Reason::from_error(
                &ProtocolError::NotGenesis,
            )));
        };
        let membership = match MembershipState::genesis(header.author, *service, *recovery) {
            Ok(m) => m,
            Err(e) => return Ok(Decision::Reject(Reason::from_error(&e))),
        };
        let business = match genesis_business(header.business, &payload.command) {
            Ok(b) => b,
            Err(refusal) => return Ok(Decision::Reject(Reason::rejected(refusal.to_string()))),
        };
        let changes = genesis_changes(&business);
        let now = self.clock.now_millis();
        let at = Timestamp::from_secs(now / 1000);
        let epoch = EpochKey::generate(0);
        let acceptance = AcceptanceHeader {
            business: header.business,
            seq: 0,
            prev: None,
            proposal: header.id,
            epoch: 0,
            next_service: None,
        };
        let content = AcceptanceContent {
            v: PROTOCOL_VERSION,
            proposal: proposal.clone(),
            payload,
            env: EnvRecord {
                ids: Vec::new(),
                now: iso_millis(now),
            },
            changes: changes.clone(),
            outcome: Outcome::Applied,
        };
        let event =
            match build_acceptance(&self.keys, &epoch.public_key(), &acceptance, &content, at) {
                Ok(event) => event,
                Err(e) => return Ok(Decision::Reject(Reason::from_error(&e))),
            };
        let mut deliveries = Vec::new();
        for reader in membership.readers() {
            let delivery = build_delivery(&self.keys, reader, header.business, &epoch, at)
                .map_err(LedgerError::internal)?;
            deliveries.push((*reader, delivery));
        }
        Ok(Decision::Accept(Box::new(Accepted {
            event,
            seq: 0,
            business,
            changes,
            genesis: Some(Genesis {
                membership,
                epoch,
                deliveries,
            }),
        })))
    }

    async fn store_acceptance(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        header: &ProposalHeader,
        accepted: Accepted,
    ) -> Result<Acknowledgment, LedgerError> {
        let business_id = header.business.to_string();
        let event_json = accepted.event.as_json();
        let event_id = accepted.event.id.to_hex();
        let snapshot = serde_json::to_string(&accepted.business).map_err(LedgerError::internal)?;
        let seq = to_i64(accepted.seq);
        match &accepted.genesis {
            Some(g) => {
                let membership =
                    serde_json::to_string(&g.membership).map_err(LedgerError::internal)?;
                sqlx::query(
                    "insert into businesses (id, seq, head, epoch, snapshot, membership)
                     values ($1, 0, $2, 0, $3::jsonb, $4::jsonb)",
                )
                .bind(&business_id)
                .bind(&event_id)
                .bind(&snapshot)
                .bind(&membership)
                .execute(&mut **tx)
                .await?;
                let sealed = encrypt_to(
                    self.keys.secret_key(),
                    &self.keys.public_key(),
                    &g.epoch.secret_key().to_secret_hex(),
                )
                .map_err(LedgerError::internal)?;
                sqlx::query(
                    "insert into epochs (business_id, epoch, public_key, secret_sealed)
                     values ($1, 0, $2, $3)",
                )
                .bind(&business_id)
                .bind(g.epoch.public_key().to_hex())
                .bind(&sealed)
                .execute(&mut **tx)
                .await?;
                for (key, member) in &g.membership.members {
                    sqlx::query(
                        "insert into members (business_id, pubkey, role, history, joined_epoch)
                         values ($1, $2, $3, $4, $5)",
                    )
                    .bind(&business_id)
                    .bind(key.to_hex())
                    .bind(json_name(&member.role))
                    .bind(json_name(&member.history))
                    .bind(to_i64(member.joined_epoch))
                    .execute(&mut **tx)
                    .await?;
                }
            }
            None => {
                sqlx::query(
                    "update businesses set seq = $2, head = $3, snapshot = $4::jsonb where id = $1",
                )
                .bind(&business_id)
                .bind(seq)
                .bind(&event_id)
                .bind(&snapshot)
                .execute(&mut **tx)
                .await?;
            }
        }
        sqlx::query(
            "insert into acceptances (business_id, seq, event_id, proposal_id, event)
             values ($1, $2, $3, $4, $5)",
        )
        .bind(&business_id)
        .bind(seq)
        .bind(&event_id)
        .bind(header.id.to_hex())
        .bind(&event_json)
        .execute(&mut **tx)
        .await?;
        for key in accepted.changes.iter().filter_map(RecordChange::key) {
            sqlx::query(
                "insert into record_versions (business_id, record_type, record_id, seq)
                 values ($1, $2, $3, $4)
                 on conflict (business_id, record_type, record_id) do update set seq = excluded.seq",
            )
            .bind(&business_id)
            .bind(type_name(&key))
            .bind(&key.id)
            .bind(seq)
            .execute(&mut **tx)
            .await?;
        }
        outbox(tx, &event_id, &event_json).await?;
        if let Some(g) = &accepted.genesis {
            for (member, delivery) in &g.deliveries {
                let json = delivery.as_json();
                sqlx::query(
                    "insert into epoch_deliveries (business_id, epoch, member, event)
                     values ($1, 0, $2, $3)",
                )
                .bind(&business_id)
                .bind(member.to_hex())
                .bind(&json)
                .execute(&mut **tx)
                .await?;
                outbox(tx, &delivery.id.to_hex(), &json).await?;
            }
        }
        let ack = Acknowledgment::new(Status::Accepted, &accepted.event);
        record_ack(tx, header, &ack).await?;
        tracing::info!(
            business = %business_id,
            seq = accepted.seq,
            command = header.command.as_str(),
            changes = accepted.changes.len(),
            "accepted"
        );
        Ok(ack)
    }

    async fn store_rejection(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        header: &ProposalHeader,
        reason: &Reason,
    ) -> Result<Acknowledgment, LedgerError> {
        let at = Timestamp::from_secs(self.clock.now_secs());
        let event =
            build_rejection(&self.keys, header, reason, at).map_err(LedgerError::internal)?;
        let ack = Acknowledgment::new(Status::Rejected, &event);
        record_ack(tx, header, &ack).await?;
        sqlx::query(
            "insert into audit (business_id, proposal_id, author, code, message, rejection)
             values ($1, $2, $3, $4, $5, $6)",
        )
        .bind(header.business.to_string())
        .bind(header.id.to_hex())
        .bind(header.author.to_hex())
        .bind(json_name(&reason.code))
        .bind(&reason.message)
        .bind(event.as_json())
        .execute(&mut **tx)
        .await?;
        // The message can quote business data, so it isn't logged.
        tracing::info!(
            business = %header.business,
            command = header.command.as_str(),
            code = json_name(&reason.code),
            "rejected"
        );
        Ok(ack)
    }

    async fn epoch_key(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        business: &str,
        epoch: u64,
    ) -> Result<EpochKey, LedgerError> {
        let sealed: String = sqlx::query_scalar(
            "select secret_sealed from epochs where business_id = $1 and epoch = $2",
        )
        .bind(business)
        .bind(to_i64(epoch))
        .fetch_one(&mut **tx)
        .await?;
        let hex = decrypt_from(self.keys.secret_key(), &self.keys.public_key(), &sealed)
            .map_err(LedgerError::internal)?;
        let secret = nostr::prelude::SecretKey::from_hex(&hex).map_err(LedgerError::internal)?;
        Ok(EpochKey::from_secret(epoch, secret))
    }

    async fn membership(
        &self,
        business: &BusinessId,
    ) -> Result<Option<MembershipState>, LedgerError> {
        let text: Option<String> =
            sqlx::query_scalar("select membership::text from businesses where id = $1")
                .bind(business.to_string())
                .fetch_optional(&self.pool)
                .await?;
        text.map(|t| serde_json::from_str(&t).map_err(LedgerError::internal))
            .transpose()
    }

    /// Acceptances after `after` (all of them if `None`), at most `limit`,
    /// for a member who may read the books. `None` if the business doesn't
    /// exist or `reader` may not read it.
    pub async fn acceptances(
        &self,
        business: &BusinessId,
        reader: &PublicKey,
        after: Option<u64>,
        limit: u32,
    ) -> Result<Option<Page>, LedgerError> {
        let Some(membership) = self.membership(business).await? else {
            return Ok(None);
        };
        if membership.readable_epochs(reader).is_none() {
            return Ok(None);
        }
        let id = business.to_string();
        let events: Vec<String> = sqlx::query_scalar(
            "select event from acceptances where business_id = $1 and seq > $2
             order by seq limit $3",
        )
        .bind(&id)
        .bind(after.map_or(-1, to_i64))
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?;
        let head: i64 = sqlx::query_scalar("select seq from businesses where id = $1")
            .bind(&id)
            .fetch_one(&self.pool)
            .await?;
        Ok(Some(Page {
            events,
            head: to_u64(head),
        }))
    }

    /// Every epoch key delivered to `member`, optionally for one business.
    pub async fn deliveries(
        &self,
        member: &PublicKey,
        business: Option<&BusinessId>,
    ) -> Result<Vec<String>, LedgerError> {
        Ok(sqlx::query_scalar(
            "select event from epoch_deliveries
             where member = $1 and ($2::text is null or business_id = $2)
             order by business_id, epoch",
        )
        .bind(member.to_hex())
        .bind(business.map(|b| b.to_string()))
        .fetch_all(&self.pool)
        .await?)
    }

    /// `author`'s rejections, oldest first, after the cursor `after`.
    /// Returns the events and the cursor of the last one.
    pub async fn rejections(
        &self,
        author: &PublicKey,
        after: Option<i64>,
        limit: u32,
    ) -> Result<(Vec<String>, Option<i64>), LedgerError> {
        let rows = sqlx::query(
            "select id, rejection from audit where author = $1 and id > $2 order by id limit $3",
        )
        .bind(author.to_hex())
        .bind(after.unwrap_or(0))
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?;
        let mut cursor = None;
        let mut events = Vec::with_capacity(rows.len());
        for row in rows {
            cursor = Some(row.try_get::<i64, _>("id")?);
            events.push(row.try_get("rejection")?);
        }
        Ok((events, cursor))
    }

    /// Records a NIP-98 auth event as used; false if it already was.
    pub async fn remember_auth(&self, event_id: &str, now_secs: u64) -> Result<bool, LedgerError> {
        sqlx::query("delete from nip98_seen where seen_at < $1")
            .bind(to_i64(now_secs.saturating_sub(crate::nip98::REMEMBER_SECS)))
            .execute(&self.pool)
            .await?;
        let inserted = sqlx::query(
            "insert into nip98_seen (event_id, seen_at) values ($1, $2) on conflict do nothing",
        )
        .bind(event_id)
        .bind(to_i64(now_secs))
        .execute(&self.pool)
        .await?
        .rows_affected();
        Ok(inserted == 1)
    }
}

/// A serde enum's JSON name: `Role::Owner` → `owner`.
fn json_name<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_default()
}

async fn stored_ack<'e, E>(executor: E, id: &str) -> Result<Option<Acknowledgment>, sqlx::Error>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query("select status, ack from proposals where event_id = $1")
        .bind(id)
        .fetch_optional(executor)
        .await?;
    row.map(|r| {
        let status = match r.try_get::<String, _>("status")?.as_str() {
            "accepted" => Status::Accepted,
            _ => Status::Rejected,
        };
        Ok(Acknowledgment {
            status,
            text: r.try_get("ack")?,
        })
    })
    .transpose()
}

async fn record_ack(
    tx: &mut Transaction<'_, Postgres>,
    header: &ProposalHeader,
    ack: &Acknowledgment,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "insert into proposals (event_id, business_id, author, status, ack)
         values ($1, $2, $3, $4, $5)",
    )
    .bind(header.id.to_hex())
    .bind(header.business.to_string())
    .bind(header.author.to_hex())
    .bind(json_name(&ack.status))
    .bind(&ack.text)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn outbox(
    tx: &mut Transaction<'_, Postgres>,
    event_id: &str,
    event: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("insert into outbox (event_id, event) values ($1, $2)")
        .bind(event_id)
        .bind(event)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn record_versions(
    tx: &mut Transaction<'_, Postgres>,
    business: &str,
    keys: &[RecordKey],
) -> Result<HashMap<RecordKey, u64>, sqlx::Error> {
    let mut out = HashMap::new();
    for key in keys {
        let seq: Option<i64> = sqlx::query_scalar(
            "select seq from record_versions
             where business_id = $1 and record_type = $2 and record_id = $3",
        )
        .bind(business)
        .bind(type_name(key))
        .bind(&key.id)
        .fetch_optional(&mut **tx)
        .await?;
        if let Some(seq) = seq {
            out.insert(key.clone(), to_u64(seq));
        }
    }
    Ok(out)
}
