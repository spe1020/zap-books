//! Record changes: what an acceptance did to the books, by type and id.
//!
//! [`diff`] computes them from the state before and after a command, and
//! checks that [`apply_changes`] reproduces the after state exactly, so a
//! chain of record changes always rebuilds the snapshot it was taken from.

use std::collections::{HashMap, HashSet};

use ledger_core::{
    Account, Approval, AuditEvent, Business, BusinessData, Category, ClassificationRule,
    ExpenseDraft, ExtractionRecord, FinancialEvent, ImportedTransaction, JournalEntry,
    SourceDocument, Suggestion, UncheckedJournal, Vendor,
};
use serde::{Deserialize, Serialize};

use ledger_core::SettingsChange;

use crate::command::Command;
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
    Suggestion,
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
            RecordType::Suggestion => "suggestion",
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
    Suggestion(Suggestion),
}

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
            Record::Suggestion(r) => (RecordType::Suggestion, &r.id),
        };
        RecordKey::new(t, id.clone())
    }
}

/// `{"type": "remove", "of": "extraction", "id": …}`. Only extraction records
/// are ever removed: saving a new reading replaces an unapproved one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename = "remove")]
pub struct Removal {
    pub of: RecordType,
    pub id: String,
}

/// `{"type": "order", "of": "rule", "ids": […]}`: the collection's records,
/// in this order. Saving a rule moves it to the end.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename = "order")]
pub struct Reordering {
    pub of: RecordType,
    pub ids: Vec<String>,
}

/// One step of an acceptance's record changes.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RecordChange {
    /// `{"type": "draft", "record": {…}}`: created or replaced.
    Put(Record),
    Remove(Removal),
    Order(Reordering),
}

impl RecordChange {
    /// The record this change creates, replaces or removes. A reordering
    /// changes no record.
    pub fn key(&self) -> Option<RecordKey> {
        match self {
            RecordChange::Put(r) => Some(r.key()),
            RecordChange::Remove(r) => Some(RecordKey::new(r.of, r.id.clone())),
            RecordChange::Order(_) => None,
        }
    }
}

trait Keyed {
    fn key_id(&self) -> &str;
}

macro_rules! keyed {
    ($($t:ty),*) => {
        $(impl Keyed for $t {
            fn key_id(&self) -> &str {
                &self.id
            }
        })*
    };
}
keyed!(
    Account,
    Category,
    Vendor,
    SourceDocument,
    ImportedTransaction,
    ExpenseDraft,
    JournalEntry,
    UncheckedJournal,
    Approval,
    ClassificationRule,
    AuditEvent,
    FinancialEvent,
    ExtractionRecord,
    Suggestion
);

/// Appends the changes for one collection: removals, then every record in
/// `after` that is new or differs from its `before` copy (in `after`'s
/// order), then a reordering if those alone would leave a different order.
fn collection<T: Keyed + PartialEq + Clone>(
    before: &[T],
    after: &[T],
    wrap: impl Fn(T) -> Record,
    of: RecordType,
    out: &mut Vec<RecordChange>,
) -> Result<()> {
    // The first record with an id is the one `apply_changes` replaces.
    let mut old: HashMap<&str, &T> = HashMap::with_capacity(before.len());
    for b in before {
        old.entry(b.key_id()).or_insert(b);
    }
    let new: HashSet<&str> = after.iter().map(Keyed::key_id).collect();
    let mut order: Vec<&str> = Vec::with_capacity(after.len());
    for b in before {
        if new.contains(b.key_id()) {
            order.push(b.key_id());
        } else if of == RecordType::Extraction {
            out.push(RecordChange::Remove(Removal {
                of,
                id: b.key_id().to_string(),
            }));
        } else {
            return Err(ProtocolError::RecordRemoved(of.name()));
        }
    }
    for a in after {
        match old.get(a.key_id()) {
            Some(b) if *b == a => {}
            Some(_) => out.push(RecordChange::Put(wrap(a.clone()))),
            None => {
                out.push(RecordChange::Put(wrap(a.clone())));
                order.push(a.key_id());
            }
        }
    }
    let wanted: Vec<&str> = after.iter().map(Keyed::key_id).collect();
    if order != wanted {
        out.push(RecordChange::Order(Reordering {
            of,
            ids: wanted.into_iter().map(String::from).collect(),
        }));
    }
    Ok(())
}

/// The record changes that turn `before` into `after`. Fails if a record
/// other than an extraction record was removed, or if replaying the changes
/// would not give exactly `after`.
pub fn diff(before: &BusinessData, after: &BusinessData) -> Result<Vec<RecordChange>> {
    use RecordType as T;
    let mut out = Vec::new();
    if before.profile != after.profile {
        out.push(RecordChange::Put(Record::Profile(after.profile.clone())));
    }
    let o = &mut out;
    collection(
        &before.accounts,
        &after.accounts,
        Record::Account,
        T::Account,
        o,
    )?;
    collection(
        &before.categories,
        &after.categories,
        Record::Category,
        T::Category,
        o,
    )?;
    collection(
        &before.vendors,
        &after.vendors,
        Record::Vendor,
        T::Vendor,
        o,
    )?;
    collection(
        &before.documents,
        &after.documents,
        Record::Document,
        T::Document,
        o,
    )?;
    collection(
        &before.payments,
        &after.payments,
        Record::Payment,
        T::Payment,
        o,
    )?;
    collection(&before.drafts, &after.drafts, Record::Draft, T::Draft, o)?;
    let journals = |b: &BusinessData| {
        b.journals
            .iter()
            .map(JournalEntry::to_unchecked)
            .collect::<Vec<_>>()
    };
    collection(
        &journals(before),
        &journals(after),
        Record::Journal,
        T::Journal,
        o,
    )?;
    collection(
        &before.approvals,
        &after.approvals,
        Record::Approval,
        T::Approval,
        o,
    )?;
    collection(&before.rules, &after.rules, Record::Rule, T::Rule, o)?;
    collection(&before.audits, &after.audits, Record::Audit, T::Audit, o)?;
    collection(&before.events, &after.events, Record::Event, T::Event, o)?;
    collection(
        &before.extractions,
        &after.extractions,
        Record::Extraction,
        T::Extraction,
        o,
    )?;
    collection(
        &before.suggestions,
        &after.suggestions,
        Record::Suggestion,
        T::Suggestion,
        o,
    )?;
    if apply_changes(before, &out)? != *after {
        return Err(ProtocolError::ChangesIncomplete);
    }
    Ok(out)
}

fn upsert<T: Keyed>(list: &mut Vec<T>, record: T) {
    match list.iter_mut().find(|x| x.key_id() == record.key_id()) {
        Some(slot) => *slot = record,
        None => list.push(record),
    }
}

fn remove<T: Keyed>(list: &mut Vec<T>, id: &str) -> Result<()> {
    let index = list.iter().position(|x| x.key_id() == id).ok_or_else(|| {
        ProtocolError::BadChange("a removal names a record that doesn't exist".into())
    })?;
    list.remove(index);
    Ok(())
}

/// Puts `list` in the order of `ids`, which must name each record exactly once.
fn reorder<T: Keyed>(list: &mut Vec<T>, ids: &[String]) -> Result<()> {
    let mismatch = || ProtocolError::BadChange("a reordering must name every record once".into());
    if ids.len() != list.len() {
        return Err(mismatch());
    }
    let mut rest: Vec<Option<T>> = std::mem::take(list).into_iter().map(Some).collect();
    for id in ids {
        let slot = rest
            .iter_mut()
            .find(|x| x.as_ref().is_some_and(|r| r.key_id() == id.as_str()))
            .ok_or_else(mismatch)?;
        list.extend(slot.take());
    }
    Ok(())
}

/// Runs `$f` on the collection `$of` names in `$b`.
macro_rules! on_list {
    ($b:expr, $of:expr, $f:ident $(, $arg:expr)*) => {
        match $of {
            RecordType::Profile => Err(ProtocolError::BadChange("the profile is not a list".into())),
            RecordType::Account => $f(&mut $b.accounts $(, $arg)*),
            RecordType::Category => $f(&mut $b.categories $(, $arg)*),
            RecordType::Vendor => $f(&mut $b.vendors $(, $arg)*),
            RecordType::Document => $f(&mut $b.documents $(, $arg)*),
            RecordType::Payment => $f(&mut $b.payments $(, $arg)*),
            RecordType::Draft => $f(&mut $b.drafts $(, $arg)*),
            RecordType::Journal => $f(&mut $b.journals $(, $arg)*),
            RecordType::Approval => $f(&mut $b.approvals $(, $arg)*),
            RecordType::Rule => $f(&mut $b.rules $(, $arg)*),
            RecordType::Audit => $f(&mut $b.audits $(, $arg)*),
            RecordType::Event => $f(&mut $b.events $(, $arg)*),
            RecordType::Extraction => $f(&mut $b.extractions $(, $arg)*),
            RecordType::Suggestion => $f(&mut $b.suggestions $(, $arg)*),
        }
    };
}

/// Applies record changes in order: a record replaces the one with its id,
/// or is appended; a removal drops an extraction record; a reordering puts a
/// collection in the given order. Journals are checked against the business
/// as it stands when they are applied.
pub fn apply_changes(business: &BusinessData, changes: &[RecordChange]) -> Result<BusinessData> {
    let mut b = business.clone();
    for change in changes {
        match change.clone() {
            RecordChange::Put(Record::Profile(r)) => {
                if r.id != b.profile.id {
                    return Err(ProtocolError::BadChange(
                        "the profile id cannot change".into(),
                    ));
                }
                b.profile = r;
            }
            RecordChange::Put(Record::Account(r)) => upsert(&mut b.accounts, r),
            RecordChange::Put(Record::Category(r)) => upsert(&mut b.categories, r),
            RecordChange::Put(Record::Vendor(r)) => upsert(&mut b.vendors, r),
            RecordChange::Put(Record::Document(r)) => upsert(&mut b.documents, r),
            RecordChange::Put(Record::Payment(r)) => upsert(&mut b.payments, r),
            RecordChange::Put(Record::Draft(r)) => upsert(&mut b.drafts, r),
            RecordChange::Put(Record::Journal(r)) => {
                let entry = JournalEntry::new(&b, r)
                    .map_err(|e| ProtocolError::BadChange(e.to_string()))?;
                upsert(&mut b.journals, entry);
            }
            RecordChange::Put(Record::Approval(r)) => upsert(&mut b.approvals, r),
            RecordChange::Put(Record::Rule(r)) => upsert(&mut b.rules, r),
            RecordChange::Put(Record::Audit(r)) => upsert(&mut b.audits, r),
            RecordChange::Put(Record::Event(r)) => upsert(&mut b.events, r),
            RecordChange::Put(Record::Extraction(r)) => upsert(&mut b.extractions, r),
            RecordChange::Put(Record::Suggestion(r)) => upsert(&mut b.suggestions, r),
            RecordChange::Remove(r) => {
                if r.of != RecordType::Extraction {
                    return Err(ProtocolError::RecordRemoved(r.of.name()));
                }
                remove(&mut b.extractions, &r.id)?;
            }
            RecordChange::Order(r) => on_list!(b, r.of, reorder, &r.ids)?,
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
        // checked against the current membership, not a snapshot; a
        // suggestion only adds a record, and never changes its draft.
        Command::CreateBusiness { .. }
        | Command::CommitImport { .. }
        | Command::Suggest { .. }
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
