//! The ledger: accepts or rejects proposals, one business at a time,
//! activates owner-level changes when they fall due, and serves the signed
//! history and epoch keys.
//!
//! Each proposal is handled in one database transaction that holds a lock
//! on its business, so two proposals for the same business never interleave
//! and different businesses proceed in parallel. Everything a proposal
//! changes (acceptance, snapshot, record versions, membership, epoch keys and
//! deliveries, acknowledgment, outbox) commits together or not at all.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use ledger_core::BusinessData;
use nostr::prelude::{Event, EventId, Keys, PublicKey, SecretKey, Timestamp};
use protocol::{
    apply_command, build_acceptance, build_delivery, build_rejection, check_base_seq, decrypt_from,
    diff, encrypt_to, genesis_business, genesis_changes, open_proposal, parse_proposal, targets,
    AcceptanceContent, AcceptanceHeader, Actor, BusinessId, Command, CommandType, CommittedPayload,
    EnvRecord, EpochKey, MembershipChange, MembershipState, Outcome, PendingChange, ProposalHeader,
    ProtocolError, Reason, RecordChange, RecordKey, RecordingEnv, Role, PROTOCOL_VERSION,
};
use serde::Serialize;
use sqlx::{PgPool, Postgres, Row, Transaction};
use thiserror::Error;

use crate::clock::{iso_millis, Clock};

/// Rejects a `ReplaceServiceKey` naming a key this service doesn't hold: it
/// could sign nothing for the business after the change.
pub const KEY_NOT_HELD: &str =
    "This service doesn't hold that key yet. Ask its operator to load it, then propose the change again.";

#[derive(Debug, Error)]
pub enum LedgerError {
    /// The event isn't a well-formed, validly signed proposal. It gets no
    /// signed rejection, since it may not even be from who it claims.
    #[error("{0}")]
    Invalid(ProtocolError),
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    /// A bug, corrupt stored state, or a service key that isn't loaded.
    /// Nothing was stored.
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
    /// The key the service is started with: it signs rejections for
    /// businesses it doesn't know.
    primary: Keys,
    /// Further keys, so businesses can move to one (`ReplaceServiceKey`).
    others: Vec<Keys>,
    clock: Arc<dyn Clock>,
    ids: Arc<dyn Fn() -> String + Send + Sync>,
}

/// A business as of its head acceptance.
struct Head {
    seq: u64,
    event: EventId,
    books: BusinessData,
    membership: MembershipState,
}

/// What a proposal turned into, before anything is written.
enum Decision {
    Accept(Box<Accepted>),
    Reject(Reason),
}

/// The next acceptance, before it is signed.
struct Next {
    proposal: Event,
    payload: CommittedPayload,
    env: EnvRecord,
    books: BusinessData,
    changes: Vec<RecordChange>,
    membership: MembershipState,
    outcome: Outcome,
    /// The acceptance's time, Unix milliseconds.
    now: u64,
}

/// A signed acceptance and everything storing it writes.
struct Accepted {
    event: Event,
    seq: u64,
    proposal: Event,
    /// The proposal's plaintext payload, kept for a pending change.
    payload: String,
    books: BusinessData,
    changes: Vec<RecordChange>,
    /// The membership before this acceptance; `None` at genesis.
    before: Option<MembershipState>,
    membership: MembershipState,
    /// The key of the epoch this acceptance starts, if it starts one.
    new_epoch: Option<EpochKey>,
    command: CommandType,
    outcome: Outcome,
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

fn outcome_name(outcome: &Outcome) -> &'static str {
    match outcome {
        Outcome::Applied => "applied",
        Outcome::Pending { .. } => "pending",
        Outcome::Activated => "activated",
        Outcome::Lapsed => "lapsed",
    }
}

fn from_json<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, LedgerError> {
    serde_json::from_str(text).map_err(LedgerError::internal)
}

fn to_json<T: Serialize>(value: &T) -> Result<String, LedgerError> {
    serde_json::to_string(value).map_err(LedgerError::internal)
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
            primary: keys,
            others: Vec::new(),
            clock,
            ids,
        }
    }

    /// Also holds `keys`: a business can then name it in `CreateBusiness`
    /// or move to it with `ReplaceServiceKey`.
    pub fn with_key(mut self, keys: Keys) -> Ledger {
        if self.key(&keys.public_key()).is_none() {
            self.others.push(keys);
        }
        self
    }

    pub fn public_key(&self) -> PublicKey {
        self.primary.public_key()
    }

    /// Every key the service holds, the primary first.
    pub fn public_keys(&self) -> Vec<PublicKey> {
        std::iter::once(&self.primary)
            .chain(&self.others)
            .map(Keys::public_key)
            .collect()
    }

    fn key(&self, public: &PublicKey) -> Option<&Keys> {
        std::iter::once(&self.primary)
            .chain(&self.others)
            .find(|k| k.public_key() == *public)
    }

    fn business_key(&self, public: &PublicKey) -> Result<&Keys, LedgerError> {
        self.key(public)
            .ok_or_else(|| LedgerError::Internal(format!("the service key {public} is not loaded")))
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
        let mut tx = self.pool.begin().await?;
        lock(&mut tx, &header.business).await?;
        if let Some(ack) = stored_ack(&mut *tx, &id).await? {
            tx.commit().await?;
            return Ok(ack);
        }
        // Changes that fell due come first, so a late veto can't overtake
        // its change, and a proposal sees the membership as it now stands.
        self.activate_due_locked(&mut tx, &header.business).await?;
        let ack = match self.decide(&mut tx, &header, proposal).await? {
            Decision::Accept(accepted) => {
                self.store(&mut tx, &header.business, &accepted).await?;
                let ack = Acknowledgment::new(Status::Accepted, &accepted.event);
                record_ack(&mut tx, &header, &ack).await?;
                ack
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
        let head = load_head(tx, &header.business).await?;
        if header.command == CommandType::CreateBusiness {
            if head.is_some() {
                return reject(&ProtocolError::AlreadyCreated);
            }
            return self.decide_genesis(tx, header, proposal).await;
        }
        let Some(head) = head else {
            return reject(&ProtocolError::NotGenesis);
        };
        // Proposals are encrypted to the business's current service key.
        let service = self.business_key(&head.membership.service)?;
        let payload = match open_proposal(proposal, service) {
            Ok((_, payload)) => payload,
            Err(e) => return reject(&e),
        };
        let command = &payload.command;
        if let Err(e) = head.membership.authorize(&header.author, command) {
            return reject(&e);
        }
        let business_id = header.business.to_string();
        let base_seq = header.base_seq.unwrap_or(0);
        let keys = targets(command, &head.books);
        let versions = record_versions(tx, &business_id, &keys).await?;
        if let Err(e) = check_base_seq(base_seq, head.seq, &keys, |k| versions.get(k).copied()) {
            return reject(&e);
        }

        let now = self.clock.now_millis();
        let ids = self.ids.clone();
        let mut env = RecordingEnv::new(move || ids(), iso_millis(now));
        let books = match apply_command(&head.books, command, &mut env) {
            Ok(after) => after,
            Err(refusal) => return Ok(Decision::Reject(Reason::rejected(refusal.to_string()))),
        };
        let changes = diff(&head.books, &books).map_err(LedgerError::internal)?;
        let (membership, outcome) = match command {
            Command::ChangeMembership { change } => {
                if let MembershipChange::ReplaceServiceKey { key } = change {
                    if self.key(key).is_none() {
                        return Ok(Decision::Reject(Reason::rejected(KEY_NOT_HELD)));
                    }
                }
                match head
                    .membership
                    .propose(header.id, &header.author, change, now / 1000)
                {
                    Ok(update) => (update.state, update.outcome),
                    Err(e) => return reject(&e),
                }
            }
            Command::Veto { pending } => match head.membership.veto(&header.author, pending) {
                Ok(state) => (state, Outcome::Applied),
                Err(e) => return reject(&e),
            },
            _ => (head.membership.clone(), Outcome::Applied),
        };
        let next = Next {
            proposal: proposal.clone(),
            payload,
            env: env.into_record(),
            books,
            changes,
            membership,
            outcome,
            now,
        };
        match self.seal(tx, header.business, Some(&head), next).await? {
            Ok(accepted) => Ok(Decision::Accept(Box::new(accepted))),
            // Too large, most likely: rejected before anything is stored.
            Err(e) => reject(&e),
        }
    }

    async fn decide_genesis(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        header: &ProposalHeader,
        proposal: &Event,
    ) -> Result<Decision, LedgerError> {
        let reject = |e: &ProtocolError| Ok(Decision::Reject(Reason::from_error(e)));
        // Genesis names its service key in a signed tag: any key this
        // service holds will do.
        let Some(service) = header.service.as_ref().and_then(|s| self.key(s)) else {
            return reject(&ProtocolError::WrongService);
        };
        let payload = match open_proposal(proposal, service) {
            Ok((_, payload)) => payload,
            Err(e) => return reject(&e),
        };
        let Command::CreateBusiness {
            service, recovery, ..
        } = &payload.command
        else {
            return reject(&ProtocolError::NotGenesis);
        };
        let membership =
            match MembershipState::genesis(header.business, header.author, *service, *recovery) {
                Ok(m) => m,
                Err(e) => return reject(&e),
            };
        let books = match genesis_business(header.business, &payload.command) {
            Ok(b) => b,
            Err(refusal) => return Ok(Decision::Reject(Reason::rejected(refusal.to_string()))),
        };
        let now = self.clock.now_millis();
        let next = Next {
            proposal: proposal.clone(),
            payload,
            env: EnvRecord {
                ids: Vec::new(),
                now: iso_millis(now),
            },
            changes: genesis_changes(&books),
            books,
            membership,
            outcome: Outcome::Applied,
            now,
        };
        match self.seal(tx, header.business, None, next).await? {
            Ok(accepted) => Ok(Decision::Accept(Box::new(accepted))),
            Err(e) => reject(&e),
        }
    }

    /// Signs the acceptance after `head` (genesis if `None`). It is signed by
    /// the business's service key as it stood before (the old key, on the
    /// acceptance that hands over to a new one), and encrypted under the
    /// epoch it leaves current: an acceptance that starts an epoch is the
    /// first under it.
    async fn seal(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        business: BusinessId,
        head: Option<&Head>,
        next: Next,
    ) -> Result<Result<Accepted, ProtocolError>, LedgerError> {
        let before = head.map(|h| &h.membership);
        let signer = self.business_key(&before.map_or(next.membership.service, |m| m.service))?;
        let epoch = next.membership.epoch;
        let new_epoch = before
            .is_none_or(|m| epoch > m.epoch)
            .then(|| EpochKey::generate(epoch));
        let epoch_key = match &new_epoch {
            Some(key) => key.clone(),
            None => {
                self.epoch_key(tx, &business.to_string(), epoch, signer)
                    .await?
            }
        };
        let header = AcceptanceHeader {
            business,
            seq: head.map_or(0, |h| h.seq + 1),
            prev: head.map(|h| h.event),
            proposal: next.proposal.id,
            epoch,
            next_service: before
                .filter(|m| m.service != next.membership.service)
                .map(|_| next.membership.service),
        };
        let payload = next.payload.text().to_string();
        let command = next.payload.payload().command.command_type();
        let content = AcceptanceContent {
            v: PROTOCOL_VERSION,
            proposal: next.proposal.clone(),
            payload: next.payload,
            env: next.env,
            changes: next.changes,
            outcome: next.outcome,
        };
        let at = Timestamp::from_secs(next.now / 1000);
        let event = match build_acceptance(signer, &epoch_key.public_key(), &header, &content, at) {
            Ok(event) => event,
            Err(e) => return Ok(Err(e)),
        };
        Ok(Ok(Accepted {
            event,
            seq: header.seq,
            proposal: next.proposal,
            payload,
            books: next.books,
            changes: content.changes,
            before: before.cloned(),
            membership: next.membership,
            new_epoch,
            command,
            outcome: content.outcome,
        }))
    }

    async fn store(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        business: &BusinessId,
        accepted: &Accepted,
    ) -> Result<(), LedgerError> {
        let business_id = business.to_string();
        let event_json = accepted.event.as_json();
        let event_id = accepted.event.id.to_hex();
        let snapshot = to_json(&accepted.books)?;
        let membership = to_json(&accepted.membership)?;
        let seq = to_i64(accepted.seq);
        let insert = if accepted.before.is_none() {
            "insert into businesses (id, seq, head, epoch, snapshot, membership)
             values ($1, $2, $3, $4, $5::jsonb, $6::jsonb)"
        } else {
            "update businesses
             set seq = $2, head = $3, epoch = $4, snapshot = $5::jsonb, membership = $6::jsonb
             where id = $1"
        };
        sqlx::query(insert)
            .bind(&business_id)
            .bind(seq)
            .bind(&event_id)
            .bind(to_i64(accepted.membership.epoch))
            .bind(&snapshot)
            .bind(&membership)
            .execute(&mut **tx)
            .await?;
        sqlx::query(
            "insert into acceptances (business_id, seq, event_id, proposal_id, event)
             values ($1, $2, $3, $4, $5)",
        )
        .bind(&business_id)
        .bind(seq)
        .bind(&event_id)
        .bind(accepted.proposal.id.to_hex())
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
        if accepted.before.as_ref() != Some(&accepted.membership) {
            self.store_membership(tx, business, accepted).await?;
        }
        tracing::info!(
            business = %business_id,
            seq = accepted.seq,
            command = accepted.command.as_str(),
            outcome = outcome_name(&accepted.outcome),
            epoch = accepted.membership.epoch,
            changes = accepted.changes.len(),
            "accepted"
        );
        Ok(())
    }

    /// Brings the membership tables in line with an acceptance that changed
    /// the membership: epoch keys (resealed if the service key changed),
    /// members, pending changes, and the epoch key deliveries everyone who
    /// reads is now entitled to.
    async fn store_membership(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        business: &BusinessId,
        accepted: &Accepted,
    ) -> Result<(), LedgerError> {
        let business_id = business.to_string();
        let state = &accepted.membership;
        let signer = self.business_key(&state.service)?;
        if let Some(old) = accepted
            .before
            .as_ref()
            .filter(|m| m.service != state.service)
        {
            // The new key seals every epoch secret from now on, so the old
            // key can leave the service once no business uses it.
            let old = self.business_key(&old.service)?;
            let rows = sqlx::query(
                "select epoch, secret_sealed from epochs where business_id = $1 order by epoch",
            )
            .bind(&business_id)
            .fetch_all(&mut **tx)
            .await?;
            for row in rows {
                let secret = decrypt_from(
                    old.secret_key(),
                    &old.public_key(),
                    row.try_get("secret_sealed")?,
                )
                .map_err(LedgerError::internal)?;
                let sealed = encrypt_to(signer.secret_key(), &signer.public_key(), &secret)
                    .map_err(LedgerError::internal)?;
                sqlx::query(
                    "update epochs set secret_sealed = $3 where business_id = $1 and epoch = $2",
                )
                .bind(&business_id)
                .bind(row.try_get::<i64, _>("epoch")?)
                .bind(&sealed)
                .execute(&mut **tx)
                .await?;
            }
        }
        let mut keys: HashMap<u64, EpochKey> = HashMap::new();
        if let Some(key) = &accepted.new_epoch {
            let sealed = encrypt_to(
                signer.secret_key(),
                &signer.public_key(),
                &key.secret_key().to_secret_hex(),
            )
            .map_err(LedgerError::internal)?;
            sqlx::query(
                "insert into epochs (business_id, epoch, public_key, secret_sealed)
                 values ($1, $2, $3, $4)",
            )
            .bind(&business_id)
            .bind(to_i64(key.epoch))
            .bind(key.public_key().to_hex())
            .bind(&sealed)
            .execute(&mut **tx)
            .await?;
            keys.insert(key.epoch, key.clone());
        }

        sqlx::query("delete from members where business_id = $1")
            .bind(&business_id)
            .execute(&mut **tx)
            .await?;
        for (key, member) in &state.members {
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

        // Pending changes: drop the ones gone (activated, lapsed, vetoed or
        // cancelled), add the one this acceptance made pending.
        let still: Vec<String> = state.pending.iter().map(|p| p.proposal.to_hex()).collect();
        sqlx::query(
            "delete from pending_changes where business_id = $1 and not (proposal_id = any($2))",
        )
        .bind(&business_id)
        .bind(&still)
        .execute(&mut **tx)
        .await?;
        let was: HashSet<EventId> = accepted
            .before
            .iter()
            .flat_map(|m| m.pending.iter().map(|p| p.proposal))
            .collect();
        for p in state.pending.iter().filter(|p| !was.contains(&p.proposal)) {
            if p.proposal != accepted.proposal.id {
                return Err(LedgerError::Internal(
                    "a pending change appeared from another proposal".into(),
                ));
            }
            sqlx::query(
                "insert into pending_changes
                 (business_id, proposal_id, proposer, change, effective_at, proposal, payload)
                 values ($1, $2, $3, $4::jsonb, $5, $6, $7)",
            )
            .bind(&business_id)
            .bind(p.proposal.to_hex())
            .bind(p.proposer.to_hex())
            .bind(to_json(&p.change)?)
            .bind(to_i64(p.effective_at))
            .bind(accepted.proposal.as_json())
            .bind(&accepted.payload)
            .execute(&mut **tx)
            .await?;
        }

        // Deliveries: every epoch each reader is entitled to and doesn't
        // have yet. A new epoch reaches every reader; a new full-history
        // reader receives every past epoch too.
        let delivered: HashSet<(u64, String)> =
            sqlx::query("select epoch, member from epoch_deliveries where business_id = $1")
                .bind(&business_id)
                .fetch_all(&mut **tx)
                .await?
                .iter()
                .map(|r| Ok((to_u64(r.try_get("epoch")?), r.try_get("member")?)))
                .collect::<Result<_, sqlx::Error>>()?;
        let at = accepted.event.created_at;
        for reader in state.readers() {
            let Some(range) = state.readable_epochs(reader) else {
                continue;
            };
            for epoch in range {
                if delivered.contains(&(epoch, reader.to_hex())) {
                    continue;
                }
                let key = match keys.get(&epoch) {
                    Some(key) => key.clone(),
                    None => {
                        let key = self.epoch_key(tx, &business_id, epoch, signer).await?;
                        keys.insert(epoch, key.clone());
                        key
                    }
                };
                let delivery = build_delivery(signer, reader, *business, &key, at)
                    .map_err(LedgerError::internal)?;
                let json = delivery.as_json();
                sqlx::query(
                    "insert into epoch_deliveries (business_id, epoch, member, event)
                     values ($1, $2, $3, $4)
                     on conflict (business_id, epoch, member) do update set event = excluded.event",
                )
                .bind(&business_id)
                .bind(to_i64(epoch))
                .bind(reader.to_hex())
                .bind(&json)
                .execute(&mut **tx)
                .await?;
                outbox(tx, &delivery.id.to_hex(), &json).await?;
            }
        }
        Ok(())
    }

    async fn store_rejection(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        header: &ProposalHeader,
        reason: &Reason,
    ) -> Result<Acknowledgment, LedgerError> {
        let at = Timestamp::from_secs(self.clock.now_secs());
        let signer = self.rejection_key(tx, &header.business).await?;
        let event = build_rejection(signer, header, reason, at).map_err(LedgerError::internal)?;
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

    /// The key that signs a rejection: the business's service key, or the
    /// primary key for a business this service doesn't hold.
    async fn rejection_key(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        business: &BusinessId,
    ) -> Result<&Keys, LedgerError> {
        let service: Option<String> =
            sqlx::query_scalar("select membership->>'service' from businesses where id = $1")
                .bind(business.to_string())
                .fetch_optional(&mut **tx)
                .await?
                .flatten();
        Ok(service
            .and_then(|s| PublicKey::from_hex(&s).ok())
            .and_then(|k| self.key(&k))
            .unwrap_or(&self.primary))
    }

    /// An epoch key, unsealed with the business's service key.
    async fn epoch_key(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        business: &str,
        epoch: u64,
        service: &Keys,
    ) -> Result<EpochKey, LedgerError> {
        let sealed: String = sqlx::query_scalar(
            "select secret_sealed from epochs where business_id = $1 and epoch = $2",
        )
        .bind(business)
        .bind(to_i64(epoch))
        .fetch_one(&mut **tx)
        .await?;
        let hex = decrypt_from(service.secret_key(), &service.public_key(), &sealed)
            .map_err(LedgerError::internal)?;
        let secret = SecretKey::from_hex(&hex).map_err(LedgerError::internal)?;
        Ok(EpochKey::from_secret(epoch, secret))
    }

    /// Appends an acceptance for each pending change of `business` that is
    /// due: `activated`, or `lapsed` if it no longer applies. The caller
    /// holds the business's lock.
    async fn activate_due_locked(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        business: &BusinessId,
    ) -> Result<usize, LedgerError> {
        let mut done = 0;
        loop {
            let Some(head) = load_head(tx, business).await? else {
                return Ok(done);
            };
            let now = self.clock.now_millis();
            let secs = now / 1000;
            let Some(id) = head.membership.due(secs).first().copied() else {
                return Ok(done);
            };
            let row = sqlx::query(
                "select proposal, payload from pending_changes
                 where business_id = $1 and proposal_id = $2",
            )
            .bind(business.to_string())
            .bind(id.to_hex())
            .fetch_one(&mut **tx)
            .await?;
            let proposal = Event::from_json(row.try_get::<String, _>("proposal")?)
                .map_err(LedgerError::internal)?;
            let payload = CommittedPayload::from_text(row.try_get("payload")?)
                .map_err(LedgerError::internal)?;
            let (membership, outcome) = match head.membership.activate(&id, secs) {
                Ok((state, _)) => (state, Outcome::Activated),
                Err(_) => (
                    head.membership
                        .lapse(&id, secs)
                        .map_err(LedgerError::internal)?,
                    Outcome::Lapsed,
                ),
            };
            let next = Next {
                proposal,
                payload,
                env: EnvRecord {
                    ids: Vec::new(),
                    now: iso_millis(now),
                },
                books: head.books.clone(),
                changes: Vec::new(),
                membership,
                outcome,
                now,
            };
            let accepted = self
                .seal(tx, *business, Some(&head), next)
                .await?
                .map_err(LedgerError::internal)?;
            self.store(tx, business, &accepted).await?;
            done += 1;
        }
    }

    /// Activates the due changes of one business, if it has any.
    async fn catch_up(&self, business: &BusinessId) -> Result<(), LedgerError> {
        let due: bool = sqlx::query_scalar(
            "select exists (select 1 from pending_changes
                            where business_id = $1 and effective_at <= $2)",
        )
        .bind(business.to_string())
        .bind(to_i64(self.clock.now_secs()))
        .fetch_one(&self.pool)
        .await?;
        if due {
            let mut tx = self.pool.begin().await?;
            lock(&mut tx, business).await?;
            self.activate_due_locked(&mut tx, business).await?;
            tx.commit().await?;
        }
        Ok(())
    }

    /// The scheduler's work: activates every pending change that is due, in
    /// every business, each business in its own transaction. Returns how
    /// many acceptances it appended. A business that fails is logged and
    /// left for the next run; the others go ahead.
    pub async fn activate_due(&self) -> Result<usize, LedgerError> {
        let businesses: Vec<String> = sqlx::query_scalar(
            "select distinct business_id from pending_changes where effective_at <= $1",
        )
        .bind(to_i64(self.clock.now_secs()))
        .fetch_all(&self.pool)
        .await?;
        let mut done = 0;
        for id in businesses {
            let business: BusinessId = id.parse().map_err(LedgerError::internal)?;
            let result = async {
                let mut tx = self.pool.begin().await?;
                lock(&mut tx, &business).await?;
                let n = self.activate_due_locked(&mut tx, &business).await?;
                tx.commit().await?;
                Ok::<usize, LedgerError>(n)
            }
            .await;
            match result {
                Ok(n) => done += n,
                Err(e) => tracing::error!(business = %business, error = %e, "activation failed"),
            }
        }
        Ok(done)
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
        text.map(|t| from_json(&t)).transpose()
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
        self.catch_up(business).await?;
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

    /// The epoch key deliveries `member` is entitled to now, optionally for
    /// one business. A removed member, or one who no longer reads, gets
    /// none, even of the keys they once received.
    pub async fn deliveries(
        &self,
        member: &PublicKey,
        business: Option<&BusinessId>,
    ) -> Result<Vec<String>, LedgerError> {
        if let Some(b) = business {
            self.catch_up(b).await?;
        }
        let rows = sqlx::query(
            "select business_id, epoch, event from epoch_deliveries
             where member = $1 and ($2::text is null or business_id = $2)
             order by business_id, epoch",
        )
        .bind(member.to_hex())
        .bind(business.map(|b| b.to_string()))
        .fetch_all(&self.pool)
        .await?;
        let ids: Vec<String> = rows
            .iter()
            .map(|r| r.try_get("business_id"))
            .collect::<Result<HashSet<String>, _>>()?
            .into_iter()
            .collect();
        let mut readable = HashMap::new();
        for row in sqlx::query(
            "select id, membership::text as membership from businesses where id = any($1)",
        )
        .bind(&ids)
        .fetch_all(&self.pool)
        .await?
        {
            let state: MembershipState = from_json(row.try_get("membership")?)?;
            if let Some(range) = state.readable_epochs(member) {
                readable.insert(row.try_get::<String, _>("id")?, range);
            }
        }
        let mut events = Vec::new();
        for row in rows {
            let business: String = row.try_get("business_id")?;
            let epoch = to_u64(row.try_get("epoch")?);
            if readable.get(&business).is_some_and(|r| r.contains(&epoch)) {
                events.push(row.try_get("event")?);
            }
        }
        Ok(events)
    }

    /// The owner-level changes waiting out their 48 hours, in the order they
    /// were accepted, for an owner or the recovery key. `None` if the
    /// business doesn't exist or `reader` is neither.
    pub async fn pending(
        &self,
        business: &BusinessId,
        reader: &PublicKey,
    ) -> Result<Option<Vec<PendingChange>>, LedgerError> {
        self.catch_up(business).await?;
        let Some(membership) = self.membership(business).await? else {
            return Ok(None);
        };
        Ok(match membership.actor(reader) {
            Actor::Member(Role::Owner) | Actor::RecoveryKey => Some(membership.pending),
            _ => None,
        })
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

/// Takes a business's lock for the rest of the transaction. An advisory
/// lock also covers genesis, whose business row doesn't exist yet;
/// [`load_head`] adds a row lock.
async fn lock(
    tx: &mut Transaction<'_, Postgres>,
    business: &BusinessId,
) -> Result<(), sqlx::Error> {
    sqlx::query("select pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(business.to_string())
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn load_head(
    tx: &mut Transaction<'_, Postgres>,
    business: &BusinessId,
) -> Result<Option<Head>, LedgerError> {
    let row = sqlx::query(
        "select seq, head, snapshot::text as snapshot, membership::text as membership
         from businesses where id = $1 for update",
    )
    .bind(business.to_string())
    .fetch_optional(&mut **tx)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    Ok(Some(Head {
        seq: to_u64(row.try_get("seq")?),
        event: EventId::from_hex(row.try_get::<&str, _>("head")?).map_err(LedgerError::internal)?,
        books: from_json(row.try_get("snapshot")?)?,
        membership: from_json(row.try_get("membership")?)?,
    }))
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
