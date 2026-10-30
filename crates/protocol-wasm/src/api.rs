//! What the browser calls, as plain Rust: one JSON request in, one JSON
//! answer out, or an error message. The WebAssembly exports in `lib.rs`
//! wrap these one to one, so the native parity test runs exactly the code
//! the browser runs.
//!
//! Requests and answers cross as JSON text rather than as JavaScript
//! objects, so both sides parse them with `serde_json`, exactly as the
//! service does. Hex keys and ids are lowercase, as on the wire.

use ledger_core::BusinessData;
use nostr::prelude::{Event, EventId, Keys, PublicKey, SecretKey, Tag};
use protocol::{
    AcceptanceContent, AcceptanceHeader, BusinessId, ChainHead, Command, CommittedPayload,
    EpochKey, MembershipState, ProposalPayload, ProtocolError,
};
use serde::{Deserialize, Serialize};

/// An error message for a person or a log. Protocol messages never contain
/// decrypted business data, and request parsing drops serde's message,
/// which would quote the request.
pub type Answer = Result<String, String>;

fn request<T: for<'de> Deserialize<'de>>(json: &str) -> Result<T, String> {
    serde_json::from_str(json).map_err(|_| "The request is not valid for this function.".into())
}

fn answer<T: Serialize>(value: &T) -> Answer {
    serde_json::to_string(value).map_err(|_| "The answer could not be written as JSON.".into())
}

fn protocol(error: ProtocolError) -> String {
    error.to_string()
}

fn tags(tags: &[Tag]) -> Vec<Vec<String>> {
    tags.iter().map(|t| t.as_slice().to_vec()).collect()
}

/// [`ChainHead`] on the wire: what the browser stores after verifying.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Head {
    pub seq: u64,
    pub id: EventId,
    pub epoch: u64,
    /// The key that must sign the next acceptance.
    pub service: PublicKey,
}

impl From<ChainHead> for Head {
    fn from(h: ChainHead) -> Head {
        Head {
            seq: h.seq,
            id: h.id,
            epoch: h.epoch,
            service: h.service,
        }
    }
}

impl From<Head> for ChainHead {
    fn from(h: Head) -> ChainHead {
        ChainHead {
            seq: h.seq,
            id: h.id,
            epoch: h.epoch,
            service: h.service,
        }
    }
}

// Proposals

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BuildProposal {
    business: BusinessId,
    /// Absent only on genesis.
    #[serde(default)]
    base_seq: Option<u64>,
    command: Command,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProposalParts {
    kind: u16,
    /// The exact text to encrypt to the service with NIP-44: the payload
    /// with a fresh salt. Its SHA-256 is the `payload` tag.
    plaintext: String,
    tags: Vec<Vec<String>>,
}

/// Builds a proposal's plaintext and tags, for the signer extension to
/// encrypt to the service and sign.
pub fn build_proposal(json: &str) -> Answer {
    let req: BuildProposal = request(json)?;
    let payload = ProposalPayload::new(req.business, req.base_seq, req.command);
    let (committed, t) = protocol::proposal_parts(&payload).map_err(protocol)?;
    answer(&ProposalParts {
        kind: protocol::kinds::PROPOSAL,
        plaintext: committed.text().to_string(),
        tags: tags(&t),
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CheckProposal {
    event: Event,
    /// The plaintext [`build_proposal`] returned.
    plaintext: String,
    /// The signed-in member.
    author: PublicKey,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProposalSummary {
    id: EventId,
    business: BusinessId,
    command: &'static str,
    base_seq: Option<u64>,
}

/// Checks a proposal the extension signed before it is sent: signed by
/// `author`, the right kind and size, and tags that commit to exactly
/// `plaintext`. The content can't be checked here, since only the member
/// and the service can decrypt it; the service refuses one that doesn't
/// match the commitment.
pub fn check_signed_proposal(json: &str) -> Answer {
    let req: CheckProposal = request(json)?;
    let committed = CommittedPayload::from_text(req.plaintext).map_err(protocol)?;
    let header = protocol::check_proposal(&req.event, &committed).map_err(protocol)?;
    if header.author != req.author {
        return Err("The proposal is not signed by the signed-in key.".into());
    }
    answer(&ProposalSummary {
        id: header.id,
        business: header.business,
        command: header.command.as_str(),
        base_seq: header.base_seq,
    })
}

// The chain

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct VerifyChain {
    business: BusinessId,
    /// The service key genesis names: the pinned key, used only when
    /// `after` is absent.
    service: PublicKey,
    /// The stored head to continue from; absent to verify from genesis.
    #[serde(default)]
    after: Option<Head>,
    acceptances: Vec<Event>,
}

/// Verifies acceptances as the continuation of the chain after `after`
/// (or from genesis), without decrypting them. Answers the new head, or
/// `null` when there were none and no head before.
pub fn verify_chain(json: &str) -> Answer {
    let req: VerifyChain = request(json)?;
    let head = protocol::verify_chain(
        req.business,
        req.service,
        req.after.map(ChainHead::from),
        &req.acceptances,
    )
    .map_err(protocol)?;
    answer(&head.map(Head::from))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OpenAcceptance {
    event: Event,
    epoch: u64,
    /// The epoch's secret key, hex, from [`open_delivery`].
    epoch_secret: String,
    /// The key the chain says signs this acceptance.
    service: PublicKey,
    business: BusinessId,
}

/// An acceptance's header on the wire, with the event's `created_at`,
/// which the membership rules use as the acceptance's time.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Header {
    business: BusinessId,
    seq: u64,
    prev: Option<EventId>,
    proposal: EventId,
    epoch: u64,
    next_service: Option<PublicKey>,
    created_at: u64,
}

impl Header {
    fn new(h: AcceptanceHeader, created_at: u64) -> Header {
        Header {
            business: h.business,
            seq: h.seq,
            prev: h.prev,
            proposal: h.proposal,
            epoch: h.epoch,
            next_service: h.next_service,
            created_at,
        }
    }

    fn protocol(&self) -> AcceptanceHeader {
        AcceptanceHeader {
            business: self.business,
            seq: self.seq,
            prev: self.prev,
            proposal: self.proposal,
            epoch: self.epoch,
            next_service: self.next_service,
        }
    }
}

/// An opened acceptance: what [`follow_membership`] and
/// [`apply_record_changes`] take.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Opened {
    header: Header,
    content: AcceptanceContent,
}

fn epoch_key(epoch: u64, secret: &str) -> Result<EpochKey, String> {
    let secret =
        SecretKey::from_hex(secret).map_err(|_| "Invalid epoch secret key.".to_string())?;
    Ok(EpochKey::from_secret(epoch, secret))
}

/// Opens one acceptance: signed by `service`, for `business`, decrypted with
/// the epoch key, and its content checked against its tags and its
/// proposal's signed commitment.
pub fn open_acceptance(json: &str) -> Answer {
    let req: OpenAcceptance = request(json)?;
    let key = epoch_key(req.epoch, &req.epoch_secret)?;
    let (header, content) =
        protocol::open_acceptance(&req.event, &key, &req.service).map_err(protocol)?;
    if header.business != req.business {
        return Err(protocol(ProtocolError::WrongBusiness));
    }
    answer(&Opened {
        header: Header::new(header, req.event.created_at.as_secs()),
        content,
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FollowMembership {
    /// The membership before this acceptance; absent for genesis.
    #[serde(default)]
    state: Option<MembershipState>,
    acceptance: Opened,
}

/// The membership after one opened acceptance, re-checking the service's
/// decision ([`MembershipState::follow`]). Genesis (seq 0, no state) starts
/// it.
pub fn follow_membership(json: &str) -> Answer {
    let req: FollowMembership = request(json)?;
    let Opened { header, content } = &req.acceptance;
    let next = match &req.state {
        None if header.seq == 0 => MembershipState::from_genesis(content).map_err(protocol)?,
        None => return Err(protocol(ProtocolError::NotGenesis)),
        Some(_) if header.seq == 0 => return Err(protocol(ProtocolError::AlreadyCreated)),
        Some(state) => state
            .follow(&header.protocol(), content, header.created_at)
            .map_err(protocol)?,
    };
    answer(&next)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ApplyRecordChanges {
    /// The books before this acceptance; absent for genesis.
    #[serde(default)]
    snapshot: Option<BusinessData>,
    acceptance: Opened,
}

/// The books after one opened acceptance's record changes. Genesis (seq 0,
/// no snapshot) starts them from its profile.
pub fn apply_record_changes(json: &str) -> Answer {
    let req: ApplyRecordChanges = request(json)?;
    let Opened { header, content } = &req.acceptance;
    let books = match &req.snapshot {
        None if header.seq == 0 => protocol::rebuild([content]).map_err(protocol)?,
        None => return Err(protocol(ProtocolError::NotGenesis)),
        Some(_) if header.seq == 0 => return Err(protocol(ProtocolError::AlreadyCreated)),
        Some(b) => {
            if b.profile.id != header.business.to_string() {
                return Err(protocol(ProtocolError::WrongBusiness));
            }
            protocol::apply_changes(b, &content.changes).map_err(protocol)?
        }
    };
    answer(&books)
}

// Epoch key deliveries

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ParseDelivery {
    event: Event,
    /// The signed-in member.
    member: PublicKey,
    service: PublicKey,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DeliveryTags {
    business: BusinessId,
    epoch: u64,
}

/// Checks a delivery before the extension decrypts it: signed by `service`
/// and addressed to `member`.
pub fn parse_delivery(json: &str) -> Answer {
    let req: ParseDelivery = request(json)?;
    let (business, epoch) =
        protocol::parse_delivery(&req.event, &req.member, &req.service).map_err(protocol)?;
    answer(&DeliveryTags { business, epoch })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OpenDelivery {
    event: Event,
    member: PublicKey,
    service: PublicKey,
    /// The content as the extension decrypted it
    /// (`nip44.decrypt(service, event.content)`).
    plaintext: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EpochSecret {
    business: BusinessId,
    epoch: u64,
    /// Hex. Kept in memory for the session only.
    secret: String,
}

/// Reads a delivery the extension decrypted, checking the event again and
/// the content against its tags.
pub fn open_delivery(json: &str) -> Answer {
    let req: OpenDelivery = request(json)?;
    let (business, epoch) =
        protocol::parse_delivery(&req.event, &req.member, &req.service).map_err(protocol)?;
    let delivery = protocol::read_delivery(business, epoch, &req.plaintext).map_err(protocol)?;
    answer(&EpochSecret {
        business: delivery.business,
        epoch: delivery.key.epoch,
        secret: delivery.key.secret_key().to_secret_hex(),
    })
}

// Business creation

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Keypair {
    /// Hex. Shown once at setup; never stored, logged or sent.
    secret_key: String,
    public_key: PublicKey,
}

/// A fresh keypair for an offline backup key, from the browser's random
/// source. Only the public key goes into genesis.
pub fn generate_recovery_keypair() -> Answer {
    let keys = Keys::generate();
    answer(&Keypair {
        secret_key: keys.secret_key().to_secret_hex(),
        public_key: keys.public_key(),
    })
}
