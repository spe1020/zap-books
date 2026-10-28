//! The typed command inside a proposal's encrypted content.

use std::ops::Deref;

use ledger_core::{Account, AccountKind, Business, Category, Currency, ExpenseDraft, ImportRow};
use nostr::prelude::{EventId, PublicKey};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};

use crate::business_id::BusinessId;
use crate::error::{ProtocolError, Result};

/// The `v` of every encrypted payload this crate writes.
pub const PROTOCOL_VERSION: u32 = 1;

/// A proposal's decrypted content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposalPayload {
    pub v: u32,
    pub business: BusinessId,
    /// The last acceptance seq the proposer had seen. Absent only on genesis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_seq: Option<u64>,
    /// 32 random bytes, hex. Without it, anyone could test guesses of a
    /// short command (an approval of a known expense, say) against the
    /// proposal's public payload hash.
    pub salt: String,
    pub command: Command,
}

impl ProposalPayload {
    /// A payload with a fresh salt from the operating system's random source.
    pub fn new(business: BusinessId, base_seq: Option<u64>, command: Command) -> ProposalPayload {
        ProposalPayload {
            v: PROTOCOL_VERSION,
            business,
            base_seq,
            salt: hex::encode(rand::random::<[u8; 32]>()),
            command,
        }
    }

    pub(crate) fn valid_salt(&self) -> bool {
        is_lower_hex(&self.salt, 64)
    }
}

pub(crate) fn is_lower_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// SHA-256 of `text`, lowercase hex.
pub(crate) fn sha256_hex(text: &str) -> String {
    hex::encode(Sha256::digest(text.as_bytes()))
}

/// A payload with the exact JSON text the proposer encrypted. The
/// proposal's signed `payload` tag is the SHA-256 of these bytes, so they
/// travel unchanged: re-serializing could reorder keys or change escaping.
/// It serializes as that text, a JSON string.
#[derive(Debug, Clone, PartialEq)]
pub struct CommittedPayload {
    text: String,
    payload: ProposalPayload,
}

impl CommittedPayload {
    pub fn new(payload: ProposalPayload) -> Result<CommittedPayload> {
        let text = serde_json::to_string(&payload)
            .map_err(|e| ProtocolError::BadContent(e.to_string()))?;
        Ok(CommittedPayload { text, payload })
    }

    pub fn from_text(text: String) -> Result<CommittedPayload> {
        let payload =
            serde_json::from_str(&text).map_err(|e| ProtocolError::BadContent(e.to_string()))?;
        Ok(CommittedPayload { text, payload })
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn payload(&self) -> &ProposalPayload {
        &self.payload
    }

    /// The value of the proposal's `payload` tag.
    pub fn hash(&self) -> String {
        sha256_hex(&self.text)
    }
}

impl Deref for CommittedPayload {
    type Target = ProposalPayload;

    fn deref(&self) -> &ProposalPayload {
        &self.payload
    }
}

impl Serialize for CommittedPayload {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.text)
    }
}

impl<'de> Deserialize<'de> for CommittedPayload {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        CommittedPayload::from_text(String::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Command {
    /// Genesis: always seq 0, signed by the founding owner.
    #[serde(rename_all = "camelCase")]
    CreateBusiness {
        /// The service-key declaration: the key whose acceptances count as
        /// this business's books.
        service: PublicKey,
        recovery: Recovery,
        profile: Business,
        accounts: Vec<Account>,
        categories: Vec<Category>,
    },
    #[serde(rename_all = "camelCase")]
    SaveDraft {
        draft: ExpenseDraft,
        /// References to documents first attached in this save.
        #[serde(default)]
        documents: Vec<DocumentRef>,
    },
    #[serde(rename_all = "camelCase")]
    Approve { expense_id: String },
    #[serde(rename_all = "camelCase")]
    Correct {
        original_id: String,
        replacement: ExpenseDraft,
        reason: String,
    },
    #[serde(rename_all = "camelCase")]
    CommitImport {
        account_id: String,
        rows: Vec<ImportRow>,
    },
    #[serde(rename_all = "camelCase")]
    UpdateSettings { change: SettingsChange },
    #[serde(rename_all = "camelCase")]
    SaveRule { rule: RuleInput },
    #[serde(rename_all = "camelCase")]
    ChangeMembership { change: MembershipChange },
    /// Cancels a pending owner-level change, named by its proposal id.
    #[serde(rename_all = "camelCase")]
    Veto { pending: EventId },
    /// A new draft from a submitter, who can't read the books.
    #[serde(rename_all = "camelCase")]
    SubmitReceiptDraft {
        draft: ExpenseDraft,
        #[serde(default)]
        documents: Vec<DocumentRef>,
    },
}

/// The unencrypted `cmd` tag's value, so the service can route a proposal
/// before decrypting it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommandType {
    CreateBusiness,
    SaveDraft,
    Approve,
    Correct,
    CommitImport,
    UpdateSettings,
    SaveRule,
    ChangeMembership,
    Veto,
    SubmitReceiptDraft,
}

impl CommandType {
    pub const ALL: [CommandType; 10] = [
        CommandType::CreateBusiness,
        CommandType::SaveDraft,
        CommandType::Approve,
        CommandType::Correct,
        CommandType::CommitImport,
        CommandType::UpdateSettings,
        CommandType::SaveRule,
        CommandType::ChangeMembership,
        CommandType::Veto,
        CommandType::SubmitReceiptDraft,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            CommandType::CreateBusiness => "CreateBusiness",
            CommandType::SaveDraft => "SaveDraft",
            CommandType::Approve => "Approve",
            CommandType::Correct => "Correct",
            CommandType::CommitImport => "CommitImport",
            CommandType::UpdateSettings => "UpdateSettings",
            CommandType::SaveRule => "SaveRule",
            CommandType::ChangeMembership => "ChangeMembership",
            CommandType::Veto => "Veto",
            CommandType::SubmitReceiptDraft => "SubmitReceiptDraft",
        }
    }

    pub fn parse(s: &str) -> Option<CommandType> {
        CommandType::ALL.into_iter().find(|t| t.as_str() == s)
    }
}

impl Command {
    pub fn command_type(&self) -> CommandType {
        match self {
            Command::CreateBusiness { .. } => CommandType::CreateBusiness,
            Command::SaveDraft { .. } => CommandType::SaveDraft,
            Command::Approve { .. } => CommandType::Approve,
            Command::Correct { .. } => CommandType::Correct,
            Command::CommitImport { .. } => CommandType::CommitImport,
            Command::UpdateSettings { .. } => CommandType::UpdateSettings,
            Command::SaveRule { .. } => CommandType::SaveRule,
            Command::ChangeMembership { .. } => CommandType::ChangeMembership,
            Command::Veto { .. } => CommandType::Veto,
            Command::SubmitReceiptDraft { .. } => CommandType::SubmitReceiptDraft,
        }
    }
}

/// Every business keeps a recovery party, chosen at creation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Recovery {
    /// A second owner, added at genesis with full history.
    SecondOwner { pubkey: PublicKey },
    /// A key kept offline. It is not a member: it reads nothing and can
    /// sign only `Veto` and `Recover`.
    BackupKey { pubkey: PublicKey },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Owner,
    Bookkeeper,
    Viewer,
    /// Write-only: proposes `SubmitReceiptDraft`, receives no epoch keys.
    Submitter,
}

impl Role {
    /// Whether this role holds epoch keys.
    pub fn reads(self) -> bool {
        !matches!(self, Role::Submitter)
    }
}

/// Which epoch keys a new member receives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum History {
    /// Every past epoch key.
    #[default]
    Full,
    /// Only epochs from the one their joining starts.
    FromJoin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum MembershipChange {
    #[serde(rename_all = "camelCase")]
    Add {
        member: PublicKey,
        role: Role,
        #[serde(default)]
        history: History,
    },
    #[serde(rename_all = "camelCase")]
    Remove { member: PublicKey },
    #[serde(rename_all = "camelCase")]
    ChangeRole { member: PublicKey, role: Role },
    /// `None` removes the backup key, allowed only while two owners remain.
    #[serde(rename_all = "camelCase")]
    ReplaceRecoveryKey { key: Option<PublicKey> },
    #[serde(rename_all = "camelCase")]
    ReplaceServiceKey { key: PublicKey },
    /// Signed by the recovery key: replace a lost owner key.
    #[serde(rename_all = "camelCase")]
    Recover {
        lost: PublicKey,
        replacement: PublicKey,
    },
}

/// A receipt file referred to by id and content hash. The file itself stays
/// in the browser (stage 3 moves it to encrypted storage).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentRef {
    pub id: String,
    /// SHA-256 of the file bytes, 64 lowercase hex characters.
    pub sha256: String,
    pub name: String,
    pub mime: String,
    pub size: u64,
}

impl DocumentRef {
    pub fn valid_hash(&self) -> bool {
        self.sha256.len() == 64
            && self
                .sha256
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    }
}

/// The Settings screen's edits. New records get service-generated ids.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum SettingsChange {
    #[serde(rename_all = "camelCase")]
    Profile { name: String, industry: String },
    #[serde(rename_all = "camelCase")]
    AddAccount {
        name: String,
        kind: AccountKind,
        currency: Currency,
    },
    #[serde(rename_all = "camelCase")]
    RenameAccount { id: String, name: String },
    /// Creates the category and its expense account.
    #[serde(rename_all = "camelCase")]
    AddCategory { name: String, currency: Currency },
    #[serde(rename_all = "camelCase")]
    RenameCategory { id: String, name: String },
}

/// A classification rule to save. Without an `id` it is a new rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub vendor_includes: String,
    pub category_id: String,
    pub purpose: String,
    pub enabled: bool,
}
