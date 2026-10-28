//! The Zap Books ledger protocol (`docs/protocol.md`).
//!
//! Members send signed **proposals** to the ledger service; the service
//! answers with signed **acceptances** (the books, a hash chain), signed
//! **rejections**, and **epoch key deliveries**. Everything here is a pure
//! function over Nostr events and `ledger-core` records: no database, no
//! network, no clock. Callers pass the time in.
#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod apply;
mod business_id;
mod chain;
mod command;
mod crypto;
mod env;
mod error;
mod events;
pub mod kinds;
mod limits;
mod membership;
mod records;

pub use apply::{apply_command, audit, genesis_business, genesis_changes, rebuild, Refusal};
pub use business_id::BusinessId;
pub use chain::{verify_chain, ChainHead};
pub use command::{
    Command, CommandType, CommittedPayload, DocumentRef, History, MembershipChange,
    ProposalPayload, Recovery, Role, RuleInput, SettingsChange, PROTOCOL_VERSION,
};
pub use crypto::{decrypt_from, encrypt_to, encrypted_len, EpochKey, MAX_PLAINTEXT};
pub use env::{EnvRecord, RecordingEnv, ReplayEnv};
pub use error::{ProtocolError, Result};
pub use events::{
    build_acceptance, build_delivery, build_proposal, build_rejection, open_acceptance,
    open_delivery, open_proposal, open_rejection, parse_acceptance, parse_proposal,
    AcceptanceContent, AcceptanceHeader, Delivery, Outcome, ProposalHeader, Reason, Rejection,
    RejectionCode, RejectionContent,
};
pub use limits::{next_batch, IMPORT_BATCH_START, MAX_EVENT_BYTES};
pub use membership::{
    Actor, Member, MembershipState, MembershipUpdate, PendingChange, OWNER_DELAY_SECS,
};
pub use records::{
    apply_changes, check_base_seq, diff, targets, Record, RecordChange, RecordKey, RecordType,
    Removal, Reordering, CHANGED_SINCE_OPENED,
};
