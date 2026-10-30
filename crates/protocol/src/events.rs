//! Builders and validators for the four event kinds.
//!
//! `parse_*` checks what anyone can check without a key: the signature, the
//! kind and the tags. `open_*` also decrypts, and checks that the encrypted
//! content agrees with the tags.

use nostr::event::FinalizeEvent;
use nostr::prelude::{Event, EventBuilder, EventId, Keys, PublicKey, SecretKey, Tag, Timestamp};
use serde::{Deserialize, Serialize};

use crate::business_id::BusinessId;
use crate::command::{
    is_lower_hex, Command, CommandType, CommittedPayload, MembershipChange, ProposalPayload,
    NOT_PROTOCOL_JSON, PROTOCOL_VERSION, UNSERIALIZABLE,
};
use crate::crypto::{decrypt_from, encrypt_to, encrypted_len, EpochKey, MAX_PLAINTEXT};
use crate::env::EnvRecord;
use crate::error::{ProtocolError, Result};
use crate::kinds::{self, tag};
use crate::limits::MAX_EVENT_BYTES;
use crate::records::RecordChange;

fn sign(keys: &Keys, kind: u16, content: String, tags: Vec<Tag>, at: Timestamp) -> Result<Event> {
    let event = EventBuilder::new(kinds::kind(kind), content)
        .tags(tags)
        .custom_created_at(at)
        .finalize(keys)
        .map_err(|_| ProtocolError::Encrypt)?;
    check_size(&event)?;
    Ok(event)
}

/// Encrypts `plaintext` to `recipient` and signs the event. Content too long
/// for one NIP-44 payload is reported as the event size it would have had,
/// so the caller always learns the size and the limit.
fn seal_and_sign(
    keys: &Keys,
    recipient: &PublicKey,
    plaintext: &str,
    kind: u16,
    tags: Vec<Tag>,
    at: Timestamp,
) -> Result<Event> {
    if plaintext.len() > MAX_PLAINTEXT {
        let placeholder = "A".repeat(encrypted_len(plaintext.len()));
        let would_be = EventBuilder::new(kinds::kind(kind), placeholder)
            .tags(tags)
            .custom_created_at(at)
            .finalize(keys)
            .map_err(|_| ProtocolError::Encrypt)?;
        return Err(ProtocolError::EventTooLarge {
            size: would_be.as_json().len(),
            max: MAX_EVENT_BYTES,
        });
    }
    let content = encrypt_to(keys.secret_key(), recipient, plaintext)?;
    sign(keys, kind, content, tags, at)
}

/// Every event this crate builds or accepts fits on a public relay.
fn check_size(event: &Event) -> Result<()> {
    let size = event.as_json().len();
    if size > MAX_EVENT_BYTES {
        return Err(ProtocolError::EventTooLarge {
            size,
            max: MAX_EVENT_BYTES,
        });
    }
    Ok(())
}

fn custom(name: &str, value: impl Into<String>) -> Tag {
    Tag::custom(name, [value.into()])
}

/// The value of the only tag named `name`. A repeated tag is malformed: two
/// readers must never disagree about which one counts.
fn single<'a>(event: &'a Event, name: &'static str) -> Result<Option<&'a str>> {
    let mut found = event
        .tags
        .iter()
        .filter(|t| t.as_slice().first().map(String::as_str) == Some(name));
    let first = found.next();
    if found.next().is_some() {
        return Err(ProtocolError::BadTag(name));
    }
    match first.map(|t| t.as_slice()) {
        None => Ok(None),
        Some([_, value]) => Ok(Some(value.as_str())),
        Some(_) => Err(ProtocolError::BadTag(name)),
    }
}

fn required<'a>(event: &'a Event, name: &'static str) -> Result<&'a str> {
    single(event, name)?.ok_or(ProtocolError::MissingTag(name))
}

/// Decimal digits only, no sign or leading zeros, so each number has one
/// spelling.
fn number(value: &str, name: &'static str) -> Result<u64> {
    if value.is_empty()
        || !value.bytes().all(|b| b.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return Err(ProtocolError::BadTag(name));
    }
    value.parse().map_err(|_| ProtocolError::BadTag(name))
}

/// 64 lowercase hex characters, so each id has one spelling: `from_hex`
/// alone also accepts uppercase.
fn event_id(value: &str, name: &'static str) -> Result<EventId> {
    if !is_lower_hex(value, 64) {
        return Err(ProtocolError::BadTag(name));
    }
    EventId::from_hex(value).map_err(|_| ProtocolError::BadTag(name))
}

/// 64 lowercase hex characters, as for [`event_id`].
fn public_key(value: &str, name: &'static str) -> Result<PublicKey> {
    if !is_lower_hex(value, 64) {
        return Err(ProtocolError::BadTag(name));
    }
    PublicKey::from_hex(value).map_err(|_| ProtocolError::BadTag(name))
}

fn verified(event: &Event, kind: u16) -> Result<()> {
    check_size(event)?;
    event.verify().map_err(|_| ProtocolError::BadSignature)?;
    let found = u16::from(event.kind);
    if found != kind {
        return Err(ProtocolError::WrongKind(found));
    }
    Ok(())
}

fn to_json<T: Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(|_| ProtocolError::BadContent(UNSERIALIZABLE))
}

/// serde's message is dropped: it quotes the offending text, which is
/// decrypted content.
fn from_json<T: for<'de> Deserialize<'de>>(text: &str) -> Result<T> {
    serde_json::from_str(text).map_err(|_| ProtocolError::BadContent(NOT_PROTOCOL_JSON))
}

fn check_version(v: u32) -> Result<()> {
    if v != PROTOCOL_VERSION {
        return Err(ProtocolError::UnsupportedVersion(v));
    }
    Ok(())
}

// Proposals

/// What a proposal's tags say, checked against its signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposalHeader {
    /// Also the idempotency key.
    pub id: EventId,
    pub author: PublicKey,
    pub business: BusinessId,
    pub command: CommandType,
    pub base_seq: Option<u64>,
    /// The service the proposal is encrypted to; required on genesis.
    pub service: Option<PublicKey>,
    /// SHA-256 of the exact plaintext payload, lowercase hex: the signed
    /// commitment that lets readers check the command without decrypting
    /// the proposal.
    pub payload_hash: String,
    pub created_at: Timestamp,
}

/// Signs a proposal, encrypting `payload` to `service`.
pub fn build_proposal(
    member: &Keys,
    service: &PublicKey,
    payload: &ProposalPayload,
    at: Timestamp,
) -> Result<Event> {
    let (committed, tags) = proposal_parts(payload)?;
    seal_and_sign(member, service, committed.text(), kinds::PROPOSAL, tags, at)
}

/// A proposal before encryption and signing: the exact plaintext to encrypt
/// to the service, and the tags, the payload commitment among them. For a
/// signer that holds the member's key elsewhere (a NIP-07 extension):
/// it encrypts the text and signs, then [`check_proposal`] checks the result.
pub fn proposal_parts(payload: &ProposalPayload) -> Result<(CommittedPayload, Vec<Tag>)> {
    if !payload.valid_salt() {
        return Err(ProtocolError::BadContent("invalid salt"));
    }
    let committed = CommittedPayload::new(payload.clone())?;
    let mut tags = vec![
        custom(tag::BUSINESS, payload.business.to_string()),
        custom(tag::COMMAND, payload.command.command_type().as_str()),
    ];
    if let Some(seq) = payload.base_seq {
        tags.push(custom(tag::BASE_SEQ, seq.to_string()));
    }
    if let Command::CreateBusiness { service, .. } = &payload.command {
        tags.push(custom(tag::SERVICE, service.to_hex()));
    }
    tags.push(custom(tag::PAYLOAD, committed.hash()));
    Ok((committed, tags))
}

/// Checks a proposal's signature and tags without decrypting it.
pub fn parse_proposal(event: &Event) -> Result<ProposalHeader> {
    verified(event, kinds::PROPOSAL)?;
    let business = required(event, tag::BUSINESS)?
        .parse()
        .map_err(|_| ProtocolError::BadTag(tag::BUSINESS))?;
    let command = CommandType::parse(required(event, tag::COMMAND)?)
        .ok_or(ProtocolError::BadTag(tag::COMMAND))?;
    let base_seq = single(event, tag::BASE_SEQ)?
        .map(|v| number(v, tag::BASE_SEQ))
        .transpose()?;
    let service = single(event, tag::SERVICE)?
        .map(|v| public_key(v, tag::SERVICE))
        .transpose()?;
    let payload_hash = required(event, tag::PAYLOAD)?;
    if !is_lower_hex(payload_hash, 64) {
        return Err(ProtocolError::BadTag(tag::PAYLOAD));
    }
    let genesis = command == CommandType::CreateBusiness;
    if genesis && base_seq.is_some() {
        return Err(ProtocolError::BadTag(tag::BASE_SEQ));
    }
    if !genesis && base_seq.is_none() {
        return Err(ProtocolError::MissingTag(tag::BASE_SEQ));
    }
    if genesis && service.is_none() {
        return Err(ProtocolError::MissingTag(tag::SERVICE));
    }
    if !genesis && service.is_some() {
        return Err(ProtocolError::BadTag(tag::SERVICE));
    }
    Ok(ProposalHeader {
        id: event.id,
        author: event.pubkey,
        business,
        command,
        base_seq,
        service,
        payload_hash: payload_hash.to_string(),
        created_at: event.created_at,
    })
}

/// Checks a decrypted payload against its proposal's signed tags: the
/// payload hash first, then what the tags say about the command.
fn check_payload(header: &ProposalHeader, committed: &CommittedPayload) -> Result<()> {
    if committed.hash() != header.payload_hash {
        return Err(ProtocolError::PayloadMismatch);
    }
    let payload = committed.payload();
    check_version(payload.v)?;
    if !payload.valid_salt() {
        return Err(ProtocolError::BadContent("invalid salt"));
    }
    if payload.business != header.business {
        return Err(ProtocolError::TagMismatch(tag::BUSINESS));
    }
    if payload.command.command_type() != header.command {
        return Err(ProtocolError::TagMismatch(tag::COMMAND));
    }
    if payload.base_seq != header.base_seq {
        return Err(ProtocolError::TagMismatch(tag::BASE_SEQ));
    }
    if let Command::CreateBusiness { service, .. } = &payload.command {
        if Some(*service) != header.service {
            return Err(ProtocolError::TagMismatch(tag::SERVICE));
        }
    }
    Ok(())
}

/// Checks a signed proposal against the payload it was built from, without
/// decrypting it: its signature, kind, size and tags, then that the tags
/// commit to exactly `committed`.
pub fn check_proposal(event: &Event, committed: &CommittedPayload) -> Result<ProposalHeader> {
    let header = parse_proposal(event)?;
    check_payload(&header, committed)?;
    Ok(header)
}

/// The service's view: checks and decrypts a proposal. A genesis proposal
/// must name this service's key.
pub fn open_proposal(event: &Event, service: &Keys) -> Result<(ProposalHeader, CommittedPayload)> {
    let header = parse_proposal(event)?;
    if header.service.is_some_and(|s| s != service.public_key()) {
        return Err(ProtocolError::WrongService);
    }
    let text = decrypt_from(service.secret_key(), &header.author, &event.content)?;
    let payload = CommittedPayload::from_text(text)?;
    check_payload(&header, &payload)?;
    Ok((header, payload))
}

// Acceptances

/// What an acceptance's tags say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptanceHeader {
    pub business: BusinessId,
    pub seq: u64,
    /// The previous acceptance's event id; `None` only at seq 0.
    pub prev: Option<EventId>,
    /// The proposal this acceptance is for.
    pub proposal: EventId,
    pub epoch: u64,
    /// Set on the acceptance that activates a service-key replacement: every
    /// later acceptance is signed by this key.
    pub next_service: Option<PublicKey>,
}

/// How the proposal was accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Outcome {
    /// Took effect now.
    Applied,
    /// An owner-level change, waiting until `effective_at` (Unix seconds).
    #[serde(rename_all = "camelCase")]
    Pending { effective_at: u64 },
    /// A pending change took effect, appended by the service's scheduler.
    Activated,
    /// A pending change that could no longer apply when it fell due (for
    /// example, its member had already left). Nothing changed.
    Lapsed,
}

/// An acceptance's decrypted content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceptanceContent {
    pub v: u32,
    /// The member's signed proposal, as received. Anyone can check its
    /// signature and tags; only the proposer and the service can decrypt it.
    pub proposal: Event,
    /// The decrypted payload, as the exact text the proposer encrypted.
    /// Its SHA-256 matches the proposal's signed `payload` tag.
    pub payload: CommittedPayload,
    /// The environment values the service used.
    pub env: EnvRecord,
    /// The records created or replaced, in order.
    pub changes: Vec<RecordChange>,
    pub outcome: Outcome,
}

/// The `service` tag hands the chain to a new key, so it must be exactly
/// what the content says: present on the acceptance that activates a
/// `ReplaceServiceKey` and naming that key, and on no other acceptance.
/// Otherwise whoever holds the service key could hand the chain to any key
/// with an ordinary acceptance.
fn check_handover(header: &AcceptanceHeader, content: &AcceptanceContent) -> Result<()> {
    let activated = match (&content.payload.command, content.outcome) {
        (
            Command::ChangeMembership {
                change: MembershipChange::ReplaceServiceKey { key },
            },
            Outcome::Activated,
        ) => Some(*key),
        _ => None,
    };
    if activated != header.next_service {
        return Err(ProtocolError::TagMismatch(tag::SERVICE));
    }
    Ok(())
}

/// Signs an acceptance, encrypting `content` to the epoch key `epoch`.
pub fn build_acceptance(
    service: &Keys,
    epoch: &PublicKey,
    header: &AcceptanceHeader,
    content: &AcceptanceContent,
    at: Timestamp,
) -> Result<Event> {
    if (header.seq == 0) != header.prev.is_none() {
        return Err(ProtocolError::BadTag(tag::PREV));
    }
    if content.proposal.id != header.proposal || content.payload.business != header.business {
        return Err(ProtocolError::TagMismatch(tag::EVENT));
    }
    check_handover(header, content)?;
    let mut tags = vec![
        custom(tag::BUSINESS, header.business.to_string()),
        custom(tag::SEQ, header.seq.to_string()),
    ];
    if let Some(prev) = header.prev {
        tags.push(custom(tag::PREV, prev.to_hex()));
    }
    tags.push(Tag::event(header.proposal));
    tags.push(custom(tag::EPOCH, header.epoch.to_string()));
    if let Some(next) = header.next_service {
        tags.push(custom(tag::SERVICE, next.to_hex()));
    }
    seal_and_sign(
        service,
        epoch,
        &to_json(content)?,
        kinds::ACCEPTANCE,
        tags,
        at,
    )
}

/// Checks an acceptance's signature and tags without decrypting it. The
/// caller checks the signer (see [`crate::verify_chain`]).
pub fn parse_acceptance(event: &Event) -> Result<AcceptanceHeader> {
    verified(event, kinds::ACCEPTANCE)?;
    let business = required(event, tag::BUSINESS)?
        .parse()
        .map_err(|_| ProtocolError::BadTag(tag::BUSINESS))?;
    let seq = number(required(event, tag::SEQ)?, tag::SEQ)?;
    let prev = single(event, tag::PREV)?
        .map(|v| event_id(v, tag::PREV))
        .transpose()?;
    if (seq == 0) != prev.is_none() {
        return Err(ProtocolError::BadTag(tag::PREV));
    }
    let proposal = event_id(required(event, tag::EVENT)?, tag::EVENT)?;
    let epoch = number(required(event, tag::EPOCH)?, tag::EPOCH)?;
    let next_service = single(event, tag::SERVICE)?
        .map(|v| public_key(v, tag::SERVICE))
        .transpose()?;
    Ok(AcceptanceHeader {
        business,
        seq,
        prev,
        proposal,
        epoch,
        next_service,
    })
}

/// A reader's view of an acceptance, like [`open_rejection`] and
/// [`open_delivery`]: fails unless `service` signed it (the key the chain
/// says signs this seq, from genesis or the last hand-over), then decrypts
/// it with the epoch key its `epoch` tag names and checks the content
/// against the tags: the embedded proposal must be the one the `e` tag
/// names and must be for this business, its payload must match its signed
/// commitment, and a `service` tag must match an activated key replacement.
/// Following the membership and auditing start from what this returns, so
/// a forged signer, a proposal borrowed from another business or a forged
/// hand-over all fail before either. [`crate::verify_chain`] still checks
/// the links between acceptances; this checks one acceptance on its own.
pub fn open_acceptance(
    event: &Event,
    epoch: &EpochKey,
    service: &PublicKey,
) -> Result<(AcceptanceHeader, AcceptanceContent)> {
    let header = parse_acceptance(event)?;
    if event.pubkey != *service {
        return Err(ProtocolError::WrongService);
    }
    if header.epoch != epoch.epoch {
        return Err(ProtocolError::Decrypt);
    }
    let text = decrypt_from(epoch.secret_key(), service, &event.content)?;
    let content: AcceptanceContent = from_json(&text)?;
    check_version(content.v)?;
    if content.proposal.id != header.proposal {
        return Err(ProtocolError::TagMismatch(tag::EVENT));
    }
    let proposal = parse_proposal(&content.proposal)?;
    if proposal.business != header.business {
        return Err(ProtocolError::TagMismatch(tag::BUSINESS));
    }
    check_payload(&proposal, &content.payload)?;
    check_handover(&header, &content)?;
    Ok((header, content))
}

// Rejections

/// Why a proposal was rejected, so a client can react without parsing the
/// message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RejectionCode {
    /// The proposal, or the acceptance it would produce, is over
    /// [`MAX_EVENT_BYTES`]. Nothing was stored; split it into smaller
    /// batches and send again.
    TooLarge,
    /// Any other reason; the message says which.
    Rejected,
}

impl RejectionCode {
    /// The code for a protocol error.
    pub fn of(error: &ProtocolError) -> RejectionCode {
        match error {
            ProtocolError::EventTooLarge { .. } | ProtocolError::ContentTooLarge { .. } => {
                RejectionCode::TooLarge
            }
            _ => RejectionCode::Rejected,
        }
    }
}

/// Why a proposal was rejected: a code, the message, and for `tooLarge`
/// the size the acceptance (or proposal) would have been and the limit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reason {
    pub code: RejectionCode,
    pub message: String,
    pub size: Option<u64>,
    pub limit: Option<u64>,
}

impl Reason {
    /// A `rejected` reason with a `ledger-core` or other message.
    pub fn rejected(message: impl Into<String>) -> Reason {
        Reason {
            code: RejectionCode::Rejected,
            message: message.into(),
            size: None,
            limit: None,
        }
    }

    /// The reason for a protocol error, with the sizes for `tooLarge`.
    pub fn from_error(error: &ProtocolError) -> Reason {
        let (size, limit) = match error {
            ProtocolError::EventTooLarge { size, max }
            | ProtocolError::ContentTooLarge { size, max } => {
                (Some(*size as u64), Some(*max as u64))
            }
            _ => (None, None),
        };
        Reason {
            code: RejectionCode::of(error),
            message: error.to_string(),
            size,
            limit,
        }
    }
}

/// A rejection's decrypted content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RejectionContent {
    pub v: u32,
    pub code: RejectionCode,
    pub message: String,
    /// `tooLarge` only: the size in bytes the event would have been.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// `tooLarge` only: the limit it exceeded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
}

/// A rejection as its proposer reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejection {
    pub proposal: EventId,
    pub reason: Reason,
}

/// Signs a rejection of `proposal`, encrypted to its author only. Rejections
/// are not part of the chain.
pub fn build_rejection(
    service: &Keys,
    proposal: &ProposalHeader,
    reason: &Reason,
    at: Timestamp,
) -> Result<Event> {
    let tags = vec![
        custom(tag::BUSINESS, proposal.business.to_string()),
        Tag::event(proposal.id),
        Tag::public_key(proposal.author),
    ];
    let content = RejectionContent {
        v: PROTOCOL_VERSION,
        code: reason.code,
        message: reason.message.clone(),
        size: reason.size,
        limit: reason.limit,
    };
    seal_and_sign(
        service,
        &proposal.author,
        &to_json(&content)?,
        kinds::REJECTION,
        tags,
        at,
    )
}

/// The proposer's view of a rejection. Fails unless `service` signed it and
/// it is addressed to `member`.
pub fn open_rejection(event: &Event, member: &Keys, service: &PublicKey) -> Result<Rejection> {
    verified(event, kinds::REJECTION)?;
    if event.pubkey != *service {
        return Err(ProtocolError::WrongService);
    }
    if public_key(required(event, tag::PUBKEY)?, tag::PUBKEY)? != member.public_key() {
        return Err(ProtocolError::TagMismatch(tag::PUBKEY));
    }
    let proposal = event_id(required(event, tag::EVENT)?, tag::EVENT)?;
    let text = decrypt_from(member.secret_key(), service, &event.content)?;
    let content: RejectionContent = from_json(&text)?;
    check_version(content.v)?;
    Ok(Rejection {
        proposal,
        reason: Reason {
            code: content.code,
            message: content.message,
            size: content.size,
            limit: content.limit,
        },
    })
}

// Epoch key deliveries

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeliveryContent {
    v: u32,
    business: BusinessId,
    epoch: u64,
    /// The epoch's secret key, hex.
    secret: String,
}

/// A decrypted delivery.
#[derive(Debug, Clone)]
pub struct Delivery {
    pub business: BusinessId,
    pub key: EpochKey,
}

/// Signs a delivery of one epoch key to one member.
pub fn build_delivery(
    service: &Keys,
    member: &PublicKey,
    business: BusinessId,
    key: &EpochKey,
    at: Timestamp,
) -> Result<Event> {
    let tags = vec![
        custom(tag::BUSINESS, business.to_string()),
        Tag::public_key(*member),
        custom(tag::EPOCH, key.epoch.to_string()),
    ];
    let content = DeliveryContent {
        v: PROTOCOL_VERSION,
        business,
        epoch: key.epoch,
        secret: key.secret_key().to_secret_hex(),
    };
    seal_and_sign(
        service,
        member,
        &to_json(&content)?,
        kinds::EPOCH_KEY_DELIVERY,
        tags,
        at,
    )
}

/// The member's view: decrypts a delivery from `service`.
pub fn open_delivery(event: &Event, member: &Keys, service: &PublicKey) -> Result<Delivery> {
    let (business, epoch) = parse_delivery(event, &member.public_key(), service)?;
    let text = decrypt_from(member.secret_key(), service, &event.content)?;
    read_delivery(business, epoch, &text)
}

/// Checks a delivery without decrypting it: signed by `service`, addressed
/// to `member`. Returns the business and epoch its tags name.
pub fn parse_delivery(
    event: &Event,
    member: &PublicKey,
    service: &PublicKey,
) -> Result<(BusinessId, u64)> {
    verified(event, kinds::EPOCH_KEY_DELIVERY)?;
    if event.pubkey != *service {
        return Err(ProtocolError::WrongService);
    }
    if public_key(required(event, tag::PUBKEY)?, tag::PUBKEY)? != *member {
        return Err(ProtocolError::TagMismatch(tag::PUBKEY));
    }
    let business: BusinessId = required(event, tag::BUSINESS)?
        .parse()
        .map_err(|_| ProtocolError::BadTag(tag::BUSINESS))?;
    let epoch = number(required(event, tag::EPOCH)?, tag::EPOCH)?;
    Ok((business, epoch))
}

/// Reads a delivery's decrypted content, `text`, against the business and
/// epoch [`parse_delivery`] returned. For a member whose key is held
/// elsewhere (a NIP-07 extension), which decrypts the content.
pub fn read_delivery(business: BusinessId, epoch: u64, text: &str) -> Result<Delivery> {
    let content: DeliveryContent = from_json(text)?;
    check_version(content.v)?;
    if content.business != business {
        return Err(ProtocolError::TagMismatch(tag::BUSINESS));
    }
    if content.epoch != epoch {
        return Err(ProtocolError::TagMismatch(tag::EPOCH));
    }
    let secret = SecretKey::from_hex(&content.secret)
        .map_err(|_| ProtocolError::BadContent("invalid epoch secret"))?;
    Ok(Delivery {
        business,
        key: EpochKey::from_secret(epoch, secret),
    })
}
