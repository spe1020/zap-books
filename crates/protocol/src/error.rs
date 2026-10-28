//! Every way a protocol check can fail.
//!
//! Messages that reach a person (in a signed rejection) are written for them;
//! the rest name the broken rule for logs and tests. None of them contains
//! decrypted business data.

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProtocolError {
    // Event shape
    #[error(
        "The event is {size} bytes, over the {max}-byte limit relays accept. Send it as smaller proposals."
    )]
    EventTooLarge { size: usize, max: usize },
    #[error(
        "The content is {size} bytes, over the {max}-byte limit NIP-44 encrypts. Send it as smaller proposals."
    )]
    ContentTooLarge { size: usize, max: usize },
    #[error("The event signature or id is invalid.")]
    BadSignature,
    #[error("Unexpected event kind {0}.")]
    WrongKind(u16),
    #[error("The event is missing the {0} tag.")]
    MissingTag(&'static str),
    #[error("The {0} tag is malformed.")]
    BadTag(&'static str),
    #[error("The command does not match the hash its proposal signed.")]
    PayloadMismatch,
    #[error("The event's tags don't match its encrypted content ({0}).")]
    TagMismatch(&'static str),
    #[error("Invalid business id: use 64 lowercase hex characters.")]
    BadBusinessId,
    #[error("Unsupported protocol version {0}.")]
    UnsupportedVersion(u32),
    #[error("The content could not be read: {0}")]
    BadContent(String),

    // Encryption
    #[error("The content could not be encrypted.")]
    Encrypt,
    #[error("The content could not be decrypted with this key.")]
    Decrypt,

    // Chain
    #[error("The acceptance at seq {0} is not signed by the service.")]
    ChainSigner(u64),
    #[error("Expected seq {expected}, found seq {found}.")]
    ChainSeq { expected: u64, found: u64 },
    #[error("The acceptance at seq {0} does not link to the previous acceptance.")]
    ChainPrev(u64),
    #[error("The acceptance at seq {0} belongs to another business.")]
    ChainBusiness(u64),
    #[error("The acceptance at seq {0} has an earlier epoch than its predecessor.")]
    ChainEpoch(u64),

    // Membership and permissions
    #[error("You are not a member of this business.")]
    NotMember,
    #[error("Your role can't make this change.")]
    Forbidden,
    #[error("A business must keep at least one owner.")]
    NoOwner,
    #[error("A business must keep a recovery key or a second owner.")]
    NoRecovery,
    #[error("This person is already a member.")]
    AlreadyMember,
    #[error("This person is not a member.")]
    UnknownMember,
    #[error("There is no pending change with this id.")]
    NoSuchPending,
    #[error("You can't veto your own change.")]
    OwnVeto,
    #[error("The owner key being recovered can't veto its recovery.")]
    RecoveredVeto,
    #[error("This change is not due yet.")]
    NotDue,
    #[error("This business already exists.")]
    AlreadyCreated,
    #[error("The first proposal for a business must create it.")]
    NotGenesis,
    #[error("The service key in this proposal is not this service's key.")]
    WrongService,

    // Applying commands
    #[error("This business can't be created: {0}.")]
    InvalidGenesis(String),
    #[error("A document reference needs a new id, a size and a SHA-256 hash.")]
    BadDocument,
    #[error("The receipt reading must belong to this business, this draft and one of its documents, and not be approved yet.")]
    ExtractionScope,
    #[error("Submitters can only add new drafts.")]
    SubmitterNewDraftsOnly,
    #[error("Re-running the command did not reproduce the recorded changes.")]
    AuditMismatch,

    // Records
    /// [`crate::CHANGED_SINCE_OPENED`].
    #[error("This record changed since you opened it. Reload it and try again.")]
    Changed,
    #[error("A command removed a {0} record; acceptances can only create or replace records.")]
    RecordRemoved(&'static str),
    #[error("The record changes do not reproduce the new state.")]
    ChangesIncomplete,
    #[error("A recorded change is invalid: {0}")]
    BadChange(String),
}

pub type Result<T> = std::result::Result<T, ProtocolError>;
