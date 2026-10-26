//! Domain records and their JSON wire shape (`src/domain/types.ts`).
//!
//! Every record serializes with camelCase keys, and optional TypeScript fields
//! are omitted when absent, so JSON written by the TypeScript deserializes
//! here and re-serializes to the same JSON (key order aside).

use std::collections::BTreeMap;
use std::ops::Deref;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::{LedgerError, Result};
use crate::money::{Currency, Minor};

/// The literal `schemaVersion: 1`. Any other value fails to deserialize.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SchemaVersion;

impl Serialize for SchemaVersion {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_u8(1)
    }
}

impl<'de> Deserialize<'de> for SchemaVersion {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        match u8::deserialize(d)? {
            1 => Ok(SchemaVersion),
            other => Err(serde::de::Error::custom(format!(
                "unsupported schemaVersion {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Business {
    pub id: String,
    pub business_id: String,
    pub name: String,
    pub industry: String,
    pub currency: Currency,
    pub initials: String,
    pub color: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AccountKind {
    Bank,
    Card,
    Expense,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    pub business_id: String,
    pub name: String,
    pub kind: AccountKind,
    pub currency: Currency,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Category {
    pub id: String,
    pub business_id: String,
    pub name: String,
    pub account_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Vendor {
    pub id: String,
    pub business_id: String,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DocumentKind {
    Receipt,
}

/// Receipt metadata. The TypeScript record also carries the file `blob`;
/// bytes belong to a storage adapter, so they are not part of this type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceDocument {
    pub id: String,
    pub business_id: String,
    pub name: String,
    pub mime: String,
    pub size: u64,
    pub kind: DocumentKind,
    pub created_at: String,
    pub demo: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedTransaction {
    pub id: String,
    pub business_id: String,
    pub date: String,
    pub description: String,
    pub amount_minor: Minor,
    pub currency: Currency,
    pub account_id: String,
    pub reference: String,
    pub fingerprint: String,
    pub draft_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImportProvider {
    #[default]
    Csv,
    Quickbooks,
    Zoho,
}

impl ImportProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            ImportProvider::Csv => "csv",
            ImportProvider::Quickbooks => "quickbooks",
            ImportProvider::Zoho => "zoho",
        }
    }
}

/// Embedded source evidence, preserved when the draft is edited or corrected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedExpenseSource {
    pub id: String,
    pub business_id: String,
    pub provider: ImportProvider,
    pub reference: String,
    pub fingerprint: String,
    pub date: String,
    pub amount_minor: Minor,
    pub currency: Currency,
    pub vendor: String,
    pub category: String,
    pub filename: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExpenseStatus {
    Draft,
    Posted,
    Corrected,
}

/// An expense awaiting review, or the record of one that was posted.
///
/// `status` is private: only `approve` and `correct` move an expense out of
/// `draft`. New drafts come from [`crate::blank_draft`],
/// [`ExpenseDraft::replacement`], or deserialization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpenseDraft {
    pub id: String,
    pub business_id: String,
    pub vendor: String,
    pub date: String,
    pub description: String,
    pub amount_minor: Minor,
    pub currency: Currency,
    pub category_id: String,
    pub account_id: String,
    pub purpose: String,
    pub document_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import_source: Option<ImportedExpenseSource>,
    status: ExpenseStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub journal_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correction_of: Option<String>,
    pub duplicate_reason: String,
    pub created_at: String,
}

impl ExpenseDraft {
    pub(crate) fn blank(
        id: String,
        business_id: String,
        date: String,
        currency: Currency,
        account_id: String,
        created_at: String,
    ) -> ExpenseDraft {
        ExpenseDraft {
            id,
            business_id,
            vendor: String::new(),
            date,
            description: String::new(),
            amount_minor: Minor::ZERO,
            currency,
            category_id: String::new(),
            account_id,
            purpose: String::new(),
            document_ids: Vec::new(),
            payment_id: None,
            import_source: None,
            status: ExpenseStatus::Draft,
            journal_id: None,
            correction_of: None,
            duplicate_reason: String::new(),
            created_at,
        }
    }

    pub fn status(&self) -> ExpenseStatus {
        self.status
    }

    /// `{ ...this, id, status: "draft" }`: a copy under a new id, in draft
    /// status, as the TypeScript builds a correction's replacement. Every
    /// other field, including `journalId`, is copied unchanged.
    pub fn replacement(&self, id: impl Into<String>) -> ExpenseDraft {
        ExpenseDraft {
            id: id.into(),
            status: ExpenseStatus::Draft,
            ..self.clone()
        }
    }

    pub(crate) fn mark_posted(&mut self, journal_id: String) {
        self.status = ExpenseStatus::Posted;
        self.journal_id = Some(journal_id);
    }

    pub(crate) fn mark_corrected(&mut self) {
        self.status = ExpenseStatus::Corrected;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalLine {
    pub account_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category_id: Option<String>,
    pub debit: Minor,
    pub credit: Minor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JournalKind {
    Expense,
    Transfer,
    Reversal,
}

/// The fields of a journal entry before any checks. Build one, then turn it
/// into a [`JournalEntry`] with [`JournalEntry::new`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UncheckedJournal {
    pub id: String,
    pub business_id: String,
    pub date: String,
    pub currency: Currency,
    pub lines: Vec<JournalLine>,
    pub kind: JournalKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expense_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reverses_id: Option<String>,
    pub memo: String,
    pub created_at: String,
}

pub(crate) fn check_line_amounts(line: &JournalLine) -> Result<()> {
    let negative = line.debit < Minor::ZERO || line.credit < Minor::ZERO;
    let two_sided = line.debit > Minor::ZERO && line.credit > Minor::ZERO;
    if negative || two_sided {
        Err(LedgerError::InvalidJournalAmounts)
    } else {
        Ok(())
    }
}

pub(crate) fn check_totals(debit: Minor, credit: Minor) -> Result<()> {
    if debit != credit || debit <= Minor::ZERO {
        Err(LedgerError::Unbalanced)
    } else {
        Ok(())
    }
}

/// A balanced, single-currency journal entry.
///
/// The only constructor is [`JournalEntry::new`], which runs every
/// `assertBalanced` check against the business. Load persisted entries through
/// [`BusinessData`] or [`Workspace`], whose deserializers provide that context.
/// Fields are readable through `Deref` and cannot be changed.
///
/// A context-free conversion cannot create a validated entry:
/// ```compile_fail
/// use ledger_core::{JournalEntry, UncheckedJournal};
/// fn bypass(entry: UncheckedJournal) -> JournalEntry {
///     entry.try_into().unwrap()
/// }
/// ```
/// Nor can a context-free JSON load:
/// ```compile_fail
/// use ledger_core::JournalEntry;
/// fn bypass(json: &str) -> JournalEntry {
///     serde_json::from_str(json).unwrap()
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct JournalEntry(UncheckedJournal);

impl JournalEntry {
    /// Checks `entry` with `assertBalanced` and wraps it.
    pub fn new(business: &BusinessData, entry: UncheckedJournal) -> Result<JournalEntry> {
        crate::ledger::assert_balanced(business, &entry)?;
        Ok(JournalEntry(entry))
    }

    /// A copy of the fields, for building a related entry.
    pub fn to_unchecked(&self) -> UncheckedJournal {
        self.0.clone()
    }
}

impl Deref for JournalEntry {
    type Target = UncheckedJournal;
    fn deref(&self) -> &UncheckedJournal {
        &self.0
    }
}

impl From<JournalEntry> for UncheckedJournal {
    fn from(entry: JournalEntry) -> UncheckedJournal {
        entry.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Approval {
    pub id: String,
    pub business_id: String,
    pub expense_id: String,
    pub actor: String,
    pub at: String,
    pub duplicate_reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassificationRule {
    pub id: String,
    pub business_id: String,
    pub vendor_includes: String,
    pub category_id: String,
    pub purpose: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEvent {
    pub id: String,
    pub business_id: String,
    pub action: String,
    pub entity_id: String,
    pub at: String,
    pub actor: String,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventType {
    #[serde(rename = "expense.created")]
    ExpenseCreated,
    #[serde(rename = "expense.approved")]
    ExpenseApproved,
    #[serde(rename = "expense.posted")]
    ExpensePosted,
    #[serde(rename = "expense.matched")]
    ExpenseMatched,
    #[serde(rename = "expense.reversed")]
    ExpenseReversed,
    #[serde(rename = "expense.updated")]
    ExpenseUpdated,
    #[serde(rename = "rule.saved")]
    RuleSaved,
    #[serde(rename = "settings.updated")]
    SettingsUpdated,
}

impl EventType {
    pub fn as_str(self) -> &'static str {
        match self {
            EventType::ExpenseCreated => "expense.created",
            EventType::ExpenseApproved => "expense.approved",
            EventType::ExpensePosted => "expense.posted",
            EventType::ExpenseMatched => "expense.matched",
            EventType::ExpenseReversed => "expense.reversed",
            EventType::ExpenseUpdated => "expense.updated",
            EventType::RuleSaved => "rule.saved",
            EventType::SettingsUpdated => "settings.updated",
        }
    }
}

/// `string | number | boolean`. Numbers keep their JSON form, so an integer
/// re-serializes without a trailing `.0`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PayloadValue {
    Bool(bool),
    Number(serde_json::Number),
    String(String),
}

/// Application-defined schema. This is not a Nostr accounting standard.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinancialEvent {
    pub id: String,
    pub business_id: String,
    pub schema_version: SchemaVersion,
    #[serde(rename = "type")]
    pub event_type: EventType,
    pub actor: String,
    pub occurred_at: String,
    pub source_ids: Vec<String>,
    pub entity_id: String,
    pub payload: BTreeMap<String, PayloadValue>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BusinessData {
    pub profile: Business,
    pub accounts: Vec<Account>,
    pub categories: Vec<Category>,
    pub vendors: Vec<Vendor>,
    pub documents: Vec<SourceDocument>,
    pub payments: Vec<ImportedTransaction>,
    pub drafts: Vec<ExpenseDraft>,
    pub journals: Vec<JournalEntry>,
    pub approvals: Vec<Approval>,
    pub rules: Vec<ClassificationRule>,
    pub audits: Vec<AuditEvent>,
    pub events: Vec<FinancialEvent>,
    pub extractions: Vec<ExtractionRecord>,
}

/// Wire records stay unchecked until their owning business's accounts and
/// categories are available. This type is private so it cannot be mistaken
/// for validated domain state.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredBusinessData {
    profile: Business,
    accounts: Vec<Account>,
    categories: Vec<Category>,
    vendors: Vec<Vendor>,
    documents: Vec<SourceDocument>,
    payments: Vec<ImportedTransaction>,
    drafts: Vec<ExpenseDraft>,
    journals: Vec<UncheckedJournal>,
    approvals: Vec<Approval>,
    rules: Vec<ClassificationRule>,
    audits: Vec<AuditEvent>,
    events: Vec<FinancialEvent>,
    /// Absent in workspaces saved before receipt extraction existed.
    #[serde(default)]
    extractions: Vec<ExtractionRecord>,
}

impl<'de> Deserialize<'de> for BusinessData {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let raw = StoredBusinessData::deserialize(d)?;
        let mut business = BusinessData {
            profile: raw.profile,
            accounts: raw.accounts,
            categories: raw.categories,
            vendors: raw.vendors,
            documents: raw.documents,
            payments: raw.payments,
            drafts: raw.drafts,
            journals: Vec::with_capacity(raw.journals.len()),
            approvals: raw.approvals,
            rules: raw.rules,
            audits: raw.audits,
            events: raw.events,
            extractions: raw.extractions,
        };
        for entry in raw.journals {
            let checked = JournalEntry::new(&business, entry).map_err(serde::de::Error::custom)?;
            business.journals.push(checked);
        }
        Ok(business)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    pub schema_version: SchemaVersion,
    pub revision: u64,
    pub businesses: Vec<BusinessData>,
}

/// `ExtractedField<T>["status"]`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldStatus {
    Found,
    Unclear,
    Absent,
}

/// `ExtractedField<T>`: one value a receipt reader proposed. `value` is
/// `null` unless `status` is `found`; `evidence` is untrusted receipt text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractedField<T> {
    pub status: FieldStatus,
    pub value: Option<T>,
    pub evidence: String,
}

/// `ExtractionFields`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractionFields {
    pub vendor: ExtractedField<String>,
    /// `YYYY-MM-DD`
    pub date: ExtractedField<String>,
    pub total: ExtractedField<Minor>,
    pub currency: ExtractedField<Currency>,
    pub reference: ExtractedField<String>,
    pub subtotal: ExtractedField<Minor>,
    pub tax: ExtractedField<Minor>,
    /// A tip or gratuity line. Absent in records made before tips were read.
    #[serde(default = "absent_amount")]
    pub tip: ExtractedField<Minor>,
    /// A category id of the business.
    pub category: ExtractedField<String>,
}

fn absent_amount() -> ExtractedField<Minor> {
    ExtractedField {
        status: FieldStatus::Absent,
        value: None,
        evidence: String::new(),
    }
}

/// `ExtractionFieldName`
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExtractionFieldName {
    Vendor,
    Date,
    Total,
    Currency,
    Reference,
    Subtotal,
    Tax,
    Tip,
    Category,
}

/// `ExtractionOutcome`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExtractionOutcome {
    Accepted,
    Edited,
    Ignored,
    Unclear,
}

/// `ExtractionRecord`: what a receipt reader proposed for a draft and what
/// the user did with it. Stored only; no ledger rule reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractionRecord {
    pub id: String,
    pub business_id: String,
    pub document_id: String,
    pub draft_id: String,
    pub model: String,
    pub prompt_version: String,
    pub created_at: String,
    pub fields: ExtractionFields,
    pub accepted: Vec<ExtractionFieldName>,
    /// Set when the draft is approved; `null` before.
    pub outcomes: Option<BTreeMap<ExtractionFieldName, ExtractionOutcome>>,
}
