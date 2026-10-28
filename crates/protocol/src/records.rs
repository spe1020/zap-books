//! Record changes: what an acceptance did to the books, by type and id.
//!
//! [`diff`] computes them from the state before and after a command, and
//! checks that [`apply_changes`] reproduces the after state exactly, so a
//! chain of record changes always rebuilds the snapshot it was taken from.

use std::collections::{HashMap, HashSet};

use ledger_core::{
    Account, Approval, AuditEvent, Business, BusinessData, Category, ClassificationRule,
    ExpenseDraft, ExtractionRecord, FinancialEvent, ImportedTransaction, JournalEntry,
    SourceDocument, UncheckedJournal, Vendor,
};
use serde::{Deserialize, Serialize};

use crate::command::{Command, SettingsChange};
use crate::error::{ProtocolError, Result};

/// The rejection message for a `baseSeq` conflict.
pub const CHANGED_SINCE_OPENED: &str =
    "This record changed since you opened it. Reload it and try again.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecordType {
    Profile,
    Account,
    Category,
    Vendor,
    Document,
    Payment,
    Draft,
    Journal,
    Approval,
    Rule,
    Audit,
    Event,
    Extraction,
}

impl RecordType {
    fn name(self) -> &'static str {
        match self {
            RecordType::Profile => "profile",
            RecordType::Account => "account",
            RecordType::Category => "category",
            RecordType::Vendor => "vendor",
            RecordType::Document => "document",
            RecordType::Payment => "payment",
            RecordType::Draft => "draft",
            RecordType::Journal => "journal",
            RecordType::Approval => "approval",
            RecordType::Rule => "rule",
            RecordType::Audit => "audit",
            RecordType::Event => "event",
            RecordType::Extraction => "extraction",
        }
    }
}

/// A record's identity within one business.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordKey {
    #[serde(rename = "type")]
    pub record_type: RecordType,
    pub id: String,
}

impl RecordKey {
    pub fn new(record_type: RecordType, id: impl Into<String>) -> RecordKey {
        RecordKey {
            record_type,
            id: id.into(),
        }
    }
}

/// A whole record, created or replaced. Journals travel unchecked and are
/// validated against the business when applied.
/// Variants are records of very different sizes; they are short-lived wire
/// values, so boxing them would only add noise.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "record", rename_all = "lowercase")]
pub enum Record {
    Profile(Business),
    Account(Account),
    Category(Category),
    Vendor(Vendor),
    Document(SourceDocument),
    Payment(ImportedTransaction),
    Draft(ExpenseDraft),
    Journal(UncheckedJournal),
    Approval(Approval),
    Rule(ClassificationRule),
    Audit(AuditEvent),
    Event(FinancialEvent),
    Extraction(ExtractionRecord),
}

/// `{ "type": "draft", "record": { … } }`. There are no deletions: the
/// ledger never removes a record.
pub type RecordChange = Record;

impl Record {
    pub fn key(&self) -> RecordKey {
        let (t, id) = match self {
            Record::Profile(r) => (RecordType::Profile, &r.id),
            Record::Account(r) => (RecordType::Account, &r.id),
            Record::Category(r) => (RecordType::Category, &r.id),
            Record::Vendor(r) => (RecordType::Vendor, &r.id),
            Record::Document(r) => (RecordType::Document, &r.id),
            Record::Payment(r) => (RecordType::Payment, &r.id),
            Record::Draft(r) => (RecordType::Draft, &r.id),
            Record::Journal(r) => (RecordType::Journal, &r.id),
            Record::Approval(r) => (RecordType::Approval, &r.id),
            Record::Rule(r) => (RecordType::Rule, &r.id),
            Record::Audit(r) => (RecordType::Audit, &r.id),
            Record::Event(r) => (RecordType::Event, &r.id),
            Record::Extraction(r) => (RecordType::Extraction, &r.id),
        };
        RecordKey::new(t, id.clone())
    }
}

/// Appends the changes for one collection: every record in `after` that is
/// new or differs from its `before` copy, in `after`'s order.
fn collection<T: PartialEq + Clone>(
    before: &[T],
    after: &[T],
    id: impl Fn(&T) -> &str,
    wrap: impl Fn(T) -> Record,
    record_type: RecordType,
    out: &mut Vec<Record>,
) -> Result<()> {
    // The first record with an id is the one `apply_changes` replaces.
    let mut old: HashMap<&str, &T> = HashMap::with_capacity(before.len());
    for b in before {
        old.entry(id(b)).or_insert(b);
    }
    let new: HashSet<&str> = after.iter().map(&id).collect();
    if old.keys().any(|k| !new.contains(k)) {
        return Err(ProtocolError::RecordRemoved(record_type.name()));
    }
    for a in after {
        if old.get(id(a)) != Some(&a) {
            out.push(wrap(a.clone()));
        }
    }
    Ok(())
}

/// The record changes that turn `before` into `after`. Fails if a record was
/// removed, or if replaying the changes would not give exactly `after` (for
/// example, if a command reordered records).
pub fn diff(before: &BusinessData, after: &BusinessData) -> Result<Vec<RecordChange>> {
    let mut out = Vec::new();
    if before.profile != after.profile {
        out.push(Record::Profile(after.profile.clone()));
    }
    collection(
        &before.accounts,
        &after.accounts,
        |r| &r.id,
        Record::Account,
        RecordType::Account,
        &mut out,
    )?;
    collection(
        &before.categories,
        &after.categories,
        |r| &r.id,
        Record::Category,
        RecordType::Category,
        &mut out,
    )?;
    collection(
        &before.vendors,
        &after.vendors,
        |r| &r.id,
        Record::Vendor,
        RecordType::Vendor,
        &mut out,
    )?;
    collection(
        &before.documents,
        &after.documents,
        |r| &r.id,
        Record::Document,
        RecordType::Document,
        &mut out,
    )?;
    collection(
        &before.payments,
        &after.payments,
        |r| &r.id,
        Record::Payment,
        RecordType::Payment,
        &mut out,
    )?;
    collection(
        &before.drafts,
        &after.drafts,
        |r| &r.id,
        Record::Draft,
        RecordType::Draft,
        &mut out,
    )?;
    let journals = |b: &BusinessData| {
        b.journals
            .iter()
            .map(JournalEntry::to_unchecked)
            .collect::<Vec<_>>()
    };
    collection(
        &journals(before),
        &journals(after),
        |r| &r.id,
        Record::Journal,
        RecordType::Journal,
        &mut out,
    )?;
    collection(
        &before.approvals,
        &after.approvals,
        |r| &r.id,
        Record::Approval,
        RecordType::Approval,
        &mut out,
    )?;
    collection(
        &before.rules,
        &after.rules,
        |r| &r.id,
        Record::Rule,
        RecordType::Rule,
        &mut out,
    )?;
    collection(
        &before.audits,
        &after.audits,
        |r| &r.id,
        Record::Audit,
        RecordType::Audit,
        &mut out,
    )?;
    collection(
        &before.events,
        &after.events,
        |r| &r.id,
        Record::Event,
        RecordType::Event,
        &mut out,
    )?;
    collection(
        &before.extractions,
        &after.extractions,
        |r| &r.id,
        Record::Extraction,
        RecordType::Extraction,
        &mut out,
    )?;
    if apply_changes(before, &out)? != *after {
        return Err(ProtocolError::ChangesIncomplete);
    }
    Ok(out)
}

fn upsert<T>(list: &mut Vec<T>, record: T, id: impl Fn(&T) -> &str) {
    let key = id(&record).to_string();
    match list.iter_mut().find(|x| id(x) == key) {
        Some(slot) => *slot = record,
        None => list.push(record),
    }
}

/// Applies record changes in order: a record replaces the one with its id,
/// or is appended. Journals are checked against the business as it stands
/// when they are applied.
pub fn apply_changes(business: &BusinessData, changes: &[RecordChange]) -> Result<BusinessData> {
    let mut b = business.clone();
    for change in changes {
        match change.clone() {
            Record::Profile(r) => {
                if r.id != b.profile.id {
                    return Err(ProtocolError::BadChange(
                        "the profile id cannot change".into(),
                    ));
                }
                b.profile = r;
            }
            Record::Account(r) => upsert(&mut b.accounts, r, |x| &x.id),
            Record::Category(r) => upsert(&mut b.categories, r, |x| &x.id),
            Record::Vendor(r) => upsert(&mut b.vendors, r, |x| &x.id),
            Record::Document(r) => upsert(&mut b.documents, r, |x| &x.id),
            Record::Payment(r) => upsert(&mut b.payments, r, |x| &x.id),
            Record::Draft(r) => upsert(&mut b.drafts, r, |x| &x.id),
            Record::Journal(r) => {
                let entry = JournalEntry::new(&b, r)
                    .map_err(|e| ProtocolError::BadChange(e.to_string()))?;
                upsert(&mut b.journals, entry, |x| &x.id);
            }
            Record::Approval(r) => upsert(&mut b.approvals, r, |x| &x.id),
            Record::Rule(r) => upsert(&mut b.rules, r, |x| &x.id),
            Record::Audit(r) => upsert(&mut b.audits, r, |x| &x.id),
            Record::Event(r) => upsert(&mut b.events, r, |x| &x.id),
            Record::Extraction(r) => upsert(&mut b.extractions, r, |x| &x.id),
        }
    }
    Ok(b)
}

/// The existing records a command acts on. If any changed after the
/// proposer's `baseSeq`, the proposal is stale.
pub fn targets(command: &Command, business: &BusinessData) -> Vec<RecordKey> {
    match command {
        Command::SaveDraft { draft, .. } | Command::SubmitReceiptDraft { draft, .. } => {
            vec![RecordKey::new(RecordType::Draft, draft.id.clone())]
        }
        Command::Approve { expense_id } => {
            vec![RecordKey::new(RecordType::Draft, expense_id.clone())]
        }
        Command::Correct { original_id, .. } => {
            vec![RecordKey::new(RecordType::Draft, original_id.clone())]
        }
        Command::UpdateSettings { change } => match change {
            SettingsChange::Profile { .. } => {
                vec![RecordKey::new(
                    RecordType::Profile,
                    business.profile.id.clone(),
                )]
            }
            SettingsChange::RenameAccount { id, .. } => {
                vec![RecordKey::new(RecordType::Account, id.clone())]
            }
            SettingsChange::RenameCategory { id, .. } => {
                vec![RecordKey::new(RecordType::Category, id.clone())]
            }
            SettingsChange::AddAccount { .. } | SettingsChange::AddCategory { .. } => Vec::new(),
        },
        Command::SaveRule { rule } => rule
            .id
            .iter()
            .map(|id| RecordKey::new(RecordType::Rule, id.clone()))
            .collect(),
        // Imports are deduplicated by fingerprint; membership commands are
        // checked against the current membership, not a snapshot.
        Command::CreateBusiness { .. }
        | Command::CommitImport { .. }
        | Command::ChangeMembership { .. }
        | Command::Veto { .. } => Vec::new(),
    }
}

/// Rejects a proposal whose targets changed after `base_seq`.
/// `last_changed` gives the seq of the latest acceptance that changed a
/// record, or `None` if none has. `head` is the current chain head.
pub fn check_base_seq(
    base_seq: u64,
    head: u64,
    targets: &[RecordKey],
    last_changed: impl Fn(&RecordKey) -> Option<u64>,
) -> Result<()> {
    if base_seq > head {
        return Err(ProtocolError::BadTag("base_seq"));
    }
    if targets
        .iter()
        .any(|t| last_changed(t).is_some_and(|seq| seq > base_seq))
    {
        return Err(ProtocolError::Changed);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_changed_message_is_the_constant() {
        assert_eq!(ProtocolError::Changed.to_string(), CHANGED_SINCE_OPENED);
    }

    #[test]
    fn base_seq_conflicts_only_for_later_changes() {
        let t = [RecordKey::new(RecordType::Draft, "d1")];
        let changed_at = |seq| move |_: &RecordKey| Some(seq);
        assert_eq!(check_base_seq(5, 7, &t, changed_at(5)), Ok(()));
        assert_eq!(
            check_base_seq(5, 7, &t, changed_at(6)),
            Err(ProtocolError::Changed)
        );
        assert_eq!(check_base_seq(5, 7, &t, |_| None), Ok(()));
        assert_eq!(
            check_base_seq(8, 7, &t, |_| None),
            Err(ProtocolError::BadTag("base_seq"))
        );
    }
}
