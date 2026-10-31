//! Applying a command to the books: the one function the service runs and
//! every auditor re-runs. Genesis builds the business; every other command
//! goes through `ledger-core`, with document references and receipt
//! readings checked here first.

use ledger_core::{
    approve, commit_import, correct, correct_transfer, record_opening_balance, record_transfer,
    save_draft, save_rule, suggest, update_settings, AccountKind, BusinessData, DocumentKind, Env,
    ExpenseDraft, ExtractionRecord, LedgerError, SourceDocument,
};
use thiserror::Error;

use crate::business_id::BusinessId;
use crate::command::{Command, DocumentRef};
use crate::env::ReplayEnv;
use crate::error::ProtocolError;
use crate::events::AcceptanceContent;
use crate::records::{apply_changes, diff, Record, RecordChange};

/// Why a command could not be applied. The message goes into the signed
/// rejection unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Refusal {
    #[error(transparent)]
    Ledger(#[from] LedgerError),
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
}

/// The longest record id a proposer may choose.
pub const MAX_ID_LEN: usize = 128;

/// A record id a proposer chooses (a draft, document or rule id, or a
/// genesis account or category): 1 to [`MAX_ID_LEN`] ASCII letters, digits,
/// hyphens or underscores. UUIDs and the ids the app derives from a business
/// id fit. Ids the service generates are not checked here.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_LEN
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn check_id(id: &str) -> Result<(), Refusal> {
    if !valid_id(id) {
        return Err(ProtocolError::BadId.into());
    }
    Ok(())
}

/// Names unique within their list ignoring case, and not blank, as the
/// Settings screen requires of every later account or category.
fn unique_names<'a>(names: impl IntoIterator<Item = &'a str>) -> bool {
    let mut seen = std::collections::HashSet::new();
    names.into_iter().all(|name| {
        let name = name.trim();
        !name.is_empty() && seen.insert(name.to_lowercase())
    })
}

/// The books a genesis proposal creates: its profile, accounts and
/// categories, and nothing else.
pub fn genesis_business(business: BusinessId, command: &Command) -> Result<BusinessData, Refusal> {
    let Command::CreateBusiness {
        profile,
        accounts,
        categories,
        ..
    } = command
    else {
        return Err(ProtocolError::NotGenesis.into());
    };
    let bad = |why: &str| Refusal::Protocol(ProtocolError::InvalidGenesis(why.to_string()));
    let id = business.to_string();
    if profile.id != id || profile.business_id != id {
        return Err(bad("the profile must carry the business id"));
    }
    if accounts
        .iter()
        .map(|a| a.id.as_str())
        .chain(categories.iter().map(|c| c.id.as_str()))
        .any(|id| !valid_id(id))
    {
        return Err(ProtocolError::BadId.into());
    }
    let mut seen = std::collections::HashSet::new();
    for a in accounts {
        if a.business_id != id || !seen.insert(a.id.as_str()) {
            return Err(bad("each account needs a unique id in this business"));
        }
    }
    for c in categories {
        let expense = accounts
            .iter()
            .any(|a| a.id == c.account_id && a.kind == AccountKind::Expense);
        if c.business_id != id || !seen.insert(c.id.as_str()) || !expense {
            return Err(bad(
                "each category needs a unique id and its own expense account",
            ));
        }
    }
    if !unique_names(accounts.iter().map(|a| a.name.as_str()))
        || !unique_names(categories.iter().map(|c| c.name.as_str()))
    {
        return Err(bad(
            "account names and category names must each be unique, ignoring case",
        ));
    }
    Ok(BusinessData {
        profile: profile.clone(),
        accounts: accounts.clone(),
        categories: categories.clone(),
        vendors: Vec::new(),
        documents: Vec::new(),
        payments: Vec::new(),
        drafts: Vec::new(),
        journals: Vec::new(),
        approvals: Vec::new(),
        rules: Vec::new(),
        audits: Vec::new(),
        events: Vec::new(),
        extractions: Vec::new(),
        suggestions: Vec::new(),
    })
}

/// The genesis acceptance's record changes: the profile first, then the
/// accounts and categories. A rebuild starts from the profile.
pub fn genesis_changes(b: &BusinessData) -> Vec<RecordChange> {
    std::iter::once(Record::Profile(b.profile.clone()))
        .chain(b.accounts.iter().cloned().map(Record::Account))
        .chain(b.categories.iter().cloned().map(Record::Category))
        .map(RecordChange::Put)
        .collect()
}

/// Adds document records for newly attached references. Each needs a fresh
/// id and a valid hash; the service records its own time.
fn attach(
    b: &mut BusinessData,
    documents: &[DocumentRef],
    env: &mut impl Env,
) -> Result<(), Refusal> {
    for d in documents {
        let taken = b.documents.iter().any(|x| x.id == d.id);
        if !valid_id(&d.id) || taken || !d.valid_hash() || d.size == 0 {
            return Err(ProtocolError::BadDocument.into());
        }
        b.documents.push(SourceDocument {
            id: d.id.clone(),
            business_id: b.profile.id.clone(),
            name: d.name.clone(),
            mime: d.mime.clone(),
            size: d.size,
            kind: DocumentKind::Receipt,
            created_at: env.now(),
            demo: false,
            sha256: Some(d.sha256.clone()),
        });
    }
    Ok(())
}

/// Stores a receipt reading for `draft`, replacing an earlier unapproved
/// reading of the same draft and document, as the browser does. It must
/// belong to this business, this draft and one of its documents, and must
/// not be approved yet; the service gives it its own id and time.
fn record_extraction(
    b: &mut BusinessData,
    draft: &ExpenseDraft,
    reading: &ExtractionRecord,
    env: &mut impl Env,
) -> Result<(), Refusal> {
    let scoped = reading.business_id == b.profile.id
        && reading.draft_id == draft.id
        && draft.document_ids.contains(&reading.document_id)
        && b.documents
            .iter()
            .any(|d| d.id == reading.document_id && d.business_id == b.profile.id);
    if !scoped || reading.outcomes.is_some() {
        return Err(ProtocolError::ExtractionScope.into());
    }
    let mut record = reading.clone();
    record.id = env.new_id();
    record.created_at = env.now();
    b.extractions.retain(|x| {
        !(x.draft_id == record.draft_id
            && x.document_id == record.document_id
            && x.outcomes.is_none())
    });
    b.extractions.push(record);
    Ok(())
}

/// Applies `command` to `business`. Membership commands change no records;
/// the membership rules handle them.
pub fn apply_command(
    business: &BusinessData,
    command: &Command,
    env: &mut impl Env,
) -> Result<BusinessData, Refusal> {
    match command {
        Command::CreateBusiness { .. } => Err(ProtocolError::AlreadyCreated.into()),
        Command::SaveDraft {
            draft,
            documents,
            extraction,
        } => {
            check_id(&draft.id)?;
            let mut b = business.clone();
            attach(&mut b, documents, env)?;
            let mut b = save_draft(&b, draft, env)?;
            if let Some(reading) = extraction {
                record_extraction(&mut b, draft, reading, env)?;
            }
            Ok(b)
        }
        Command::SubmitReceiptDraft { draft, documents } => {
            check_id(&draft.id)?;
            if business.drafts.iter().any(|d| d.id == draft.id) {
                return Err(ProtocolError::SubmitterNewDraftsOnly.into());
            }
            let mut b = business.clone();
            attach(&mut b, documents, env)?;
            Ok(save_draft(&b, draft, env)?)
        }
        Command::Approve { expense_id } => Ok(approve(business, expense_id, env)?),
        Command::Correct {
            original_id,
            replacement,
            reason,
            documents,
            extraction,
        } => {
            check_id(&replacement.id)?;
            // As for SaveDraft: documents, then the reading, which `correct`
            // finalizes when it posts the replacement.
            let mut b = business.clone();
            attach(&mut b, documents, env)?;
            if let Some(reading) = extraction {
                record_extraction(&mut b, replacement, reading, env)?;
            }
            Ok(correct(&b, original_id, replacement, reason, env)?)
        }
        Command::CommitImport { account_id, rows } => {
            Ok(commit_import(business, rows, account_id, env)?)
        }
        Command::UpdateSettings { change } => Ok(update_settings(business, change, env)?),
        Command::SaveRule { rule } => {
            if let Some(id) = &rule.id {
                check_id(id)?;
            }
            Ok(save_rule(business, rule, env)?)
        }
        Command::Suggest { suggestion } => Ok(suggest(business, suggestion, env)?),
        Command::RecordTransfer { transfer } => Ok(record_transfer(business, transfer, env)?),
        Command::CorrectTransfer {
            original_id,
            replacement,
            reason,
        } => Ok(correct_transfer(
            business,
            original_id,
            replacement,
            reason,
            env,
        )?),
        Command::RecordOpeningBalance { opening_balance } => {
            Ok(record_opening_balance(business, opening_balance, env)?)
        }
        Command::ChangeMembership { .. } | Command::Veto { .. } => Ok(business.clone()),
    }
}

/// Rebuilds the books from acceptance contents, in seq order, using their
/// record changes alone.
pub fn rebuild<'a>(
    contents: impl IntoIterator<Item = &'a AcceptanceContent>,
) -> Result<BusinessData, ProtocolError> {
    let mut contents = contents.into_iter();
    let genesis = contents.next().ok_or(ProtocolError::NotGenesis)?;
    let Some((RecordChange::Put(Record::Profile(profile)), rest)) = genesis.changes.split_first()
    else {
        return Err(ProtocolError::NotGenesis);
    };
    let empty = BusinessData {
        profile: profile.clone(),
        accounts: Vec::new(),
        categories: Vec::new(),
        vendors: Vec::new(),
        documents: Vec::new(),
        payments: Vec::new(),
        drafts: Vec::new(),
        journals: Vec::new(),
        approvals: Vec::new(),
        rules: Vec::new(),
        audits: Vec::new(),
        events: Vec::new(),
        extractions: Vec::new(),
        suggestions: Vec::new(),
    };
    let mut b = apply_changes(&empty, rest)?;
    for content in contents {
        b = apply_changes(&b, &content.changes)?;
    }
    Ok(b)
}

/// Audits one acceptance: re-runs its command on `before` with the recorded
/// ids and time, and checks it gives exactly the recorded changes, using
/// exactly the recorded ids. Returns the books after it. For genesis, pass
/// `before: None`. An acceptance for another business than `before` is
/// refused before anything is re-run.
pub fn audit(
    before: Option<&BusinessData>,
    content: &AcceptanceContent,
) -> Result<BusinessData, Refusal> {
    let command = &content.payload.command;
    let (after, changes) = match before {
        None => {
            // Genesis generates nothing, so it records no ids.
            if !content.env.ids.is_empty() {
                return Err(ProtocolError::AuditMismatch.into());
            }
            let b = genesis_business(content.payload.business, command)?;
            let changes = genesis_changes(&b);
            (b, changes)
        }
        Some(before) => {
            if content.payload.business.to_string() != before.profile.id {
                return Err(ProtocolError::WrongBusiness.into());
            }
            let mut env = ReplayEnv::new(&content.env);
            let after = apply_command(before, command, &mut env)?;
            env.finish()?;
            let changes = diff(before, &after)?;
            (after, changes)
        }
    };
    if changes != content.changes {
        return Err(ProtocolError::AuditMismatch.into());
    }
    Ok(after)
}
