//! Ledger commands and queries (`src/domain/ledger.ts`).
//!
//! Commands take `&BusinessData` and return a new `BusinessData`; the input
//! is never changed, so on any error the caller still holds the original.
//! This matches the TypeScript's `structuredClone` and keeps `correct`
//! atomic.

use std::collections::{BTreeMap, HashMap};

use serde::Serialize;

use crate::date::valid_date;
use crate::env::Env;
use crate::error::{LedgerError, Result};
use crate::js;
use crate::money::{Currency, Minor};
use crate::types::{
    check_line_amounts, check_totals, AccountKind, Approval, AuditEvent, BusinessData,
    DraftAuthority, ExpenseDraft, ExpenseStatus, FinancialEvent, JournalEntry, JournalKind,
    JournalLine, PayloadValue, SchemaVersion, UncheckedJournal, Vendor,
};
use crate::EventType;

/// The actor recorded on every approval, event, and audit entry.
pub const ACTOR: &str = "local-demo-user";

/// `duplicates`: other live drafts in the same business with the same vendor,
/// date, amount, and currency. A draft with a blank vendor has no duplicates.
pub fn duplicates(b: &BusinessData, d: &ExpenseDraft) -> Vec<ExpenseDraft> {
    b.drafts
        .iter()
        .filter(|x| {
            x.id != d.id
                && x.business_id == d.business_id
                && Some(&x.id) != d.correction_of.as_ref()
                && x.status() != ExpenseStatus::Corrected
                && x.amount_minor == d.amount_minor
                && x.currency == d.currency
                && x.date == d.date
                && js::normalize(&x.vendor) == js::normalize(&d.vendor)
                && js::has_text(&d.vendor)
        })
        .cloned()
        .collect()
}

/// `missingDetails`: every reason `d` cannot be posted, apart from the
/// duplicate check, in the order the TypeScript reports them.
pub fn missing_details(b: &BusinessData, d: &ExpenseDraft) -> Vec<String> {
    let mut errors = Vec::new();
    let mut push = |message: &str| errors.push(message.to_string());
    if d.business_id != b.profile.id {
        push("Expense belongs to another business.");
    }
    if d.import_source
        .as_ref()
        .is_some_and(|s| s.business_id != b.profile.id)
    {
        push("Import source belongs to another business.");
    }
    if !js::has_text(&d.vendor) {
        push("Vendor is required.");
    }
    if !valid_date(&d.date) {
        push("A valid expense date is required.");
    }
    if !js::has_text(&d.description) {
        push("Description is required.");
    }
    // `Number.isSafeInteger(d.amountMinor)` is guaranteed by `Minor`, and
    // "Choose a supported currency." by `Currency`; see PARITY_NOTES.md.
    if d.amount_minor <= Minor::ZERO {
        push("Amount must be greater than zero in whole minor units.");
    }
    let category = b
        .categories
        .iter()
        .find(|x| x.id == d.category_id && x.business_id == d.business_id);
    if category.is_none() {
        push("Category is required.");
    }
    let account = b.accounts.iter().find(|x| {
        x.id == d.account_id && x.business_id == d.business_id && x.kind != AccountKind::Expense
    });
    match account {
        None => push("Payment account is required."),
        Some(account) if account.currency != d.currency => {
            push("Payment account currency must match the expense.")
        }
        Some(_) => {}
    }
    if !js::has_text(&d.purpose) {
        push("Business purpose is required.");
    }
    if d.document_ids.iter().any(|doc_id| {
        !b.documents
            .iter()
            .any(|x| &x.id == doc_id && x.business_id == d.business_id)
    }) {
        push("Receipt does not belong to this business.");
    }
    if let Some(payment_id) = truthy(&d.payment_id) {
        let payment = b
            .payments
            .iter()
            .find(|x| x.id == payment_id && x.business_id == d.business_id);
        let matches = payment.is_some_and(|p| {
            p.amount_minor == d.amount_minor
                && p.currency == d.currency
                && p.account_id == d.account_id
                && p.date == d.date
        });
        if !matches {
            push("Expense must match its imported payment amount, date, currency, and account.");
        }
    }
    errors
}

/// `validation`: [`missing_details`] plus the duplicate-reason requirement.
pub fn validation(b: &BusinessData, d: &ExpenseDraft) -> Vec<String> {
    let mut errors = missing_details(b, d);
    if !duplicates(b, d).is_empty() && !js::has_text(&d.duplicate_reason) {
        errors.push("Explain why this possible duplicate is a separate expense.".to_string());
    }
    errors
}

/// `assertBalanced`: the entry belongs to this business, uses this
/// business's accounts in one currency (expense accounts excepted), has
/// valid one-sided amounts and categories, and balances.
pub fn assert_balanced(b: &BusinessData, entry: &UncheckedJournal) -> Result<()> {
    // `currencies.includes(entry.currency)` is guaranteed by `Currency`.
    if entry.business_id != b.profile.id || !valid_date(&entry.date) || entry.lines.len() < 2 {
        return Err(LedgerError::InvalidJournalContext);
    }
    let (mut debit, mut credit) = (Minor::ZERO, Minor::ZERO);
    for line in &entry.lines {
        let account = b
            .accounts
            .iter()
            .find(|x| x.id == line.account_id && x.business_id == entry.business_id)
            .filter(|a| a.kind == AccountKind::Expense || a.currency == entry.currency)
            .ok_or(LedgerError::InvalidJournalAccount)?;
        check_line_amounts(line)?;
        if let Some(category_id) = truthy(&line.category_id) {
            let known = b.categories.iter().any(|c| {
                c.id == category_id
                    && c.business_id == entry.business_id
                    && c.account_id == account.id
            });
            if !known {
                return Err(LedgerError::InvalidJournalCategory);
            }
        }
        debit = debit.checked_add(line.debit)?;
        credit = credit.checked_add(line.credit)?;
    }
    check_totals(debit, credit)
}

/// `event`: appends a financial event and its matching audit entry.
pub fn event(
    b: &BusinessData,
    env: &mut impl Env,
    event_type: EventType,
    entity_id: &str,
    detail: &str,
    source_ids: Vec<String>,
) -> BusinessData {
    let mut next = b.clone();
    push_event(&mut next, env, event_type, entity_id, detail, source_ids);
    next
}

pub(crate) fn push_event(
    b: &mut BusinessData,
    env: &mut impl Env,
    event_type: EventType,
    entity_id: &str,
    detail: &str,
    source_ids: Vec<String>,
) {
    let at = env.now();
    let e = FinancialEvent {
        id: env.new_id(),
        business_id: b.profile.id.clone(),
        schema_version: SchemaVersion,
        event_type,
        actor: ACTOR.to_string(),
        occurred_at: at.clone(),
        source_ids,
        entity_id: entity_id.to_string(),
        payload: BTreeMap::from([(
            "detail".to_string(),
            PayloadValue::String(detail.to_string()),
        )]),
    };
    let audit = AuditEvent {
        id: env.new_id(),
        business_id: b.profile.id.clone(),
        action: event_type.as_str().to_string(),
        entity_id: entity_id.to_string(),
        at,
        actor: e.actor.clone(),
        detail: detail.to_string(),
    };
    b.events.push(e);
    b.audits.push(audit);
}

/// `currentPeriod`: the `YYYY-MM` of [`Env::today`].
pub fn current_period(env: &impl Env) -> String {
    let today = env.today();
    today.get(..7).unwrap_or(&today).to_string()
}

/// `blankDraft`: an empty draft dated today, paid from the first bank account
/// in the business currency.
pub fn blank_draft(b: &BusinessData, env: &mut impl Env) -> ExpenseDraft {
    let id = env.new_id();
    let date = env.today();
    let account_id = b
        .accounts
        .iter()
        .find(|x| x.kind == AccountKind::Bank && x.currency == b.profile.currency)
        .map(|x| x.id.clone())
        .unwrap_or_default();
    ExpenseDraft::blank(
        id,
        b.profile.id.clone(),
        date,
        b.profile.currency,
        account_id,
        env.now(),
    )
}

/// `saveDraft`: inserts or replaces a draft, records a new vendor name, and
/// logs the change. Import evidence and payment links cannot be added,
/// changed, or removed here.
pub fn save_draft(
    business: &BusinessData,
    draft: &ExpenseDraft,
    env: &mut impl Env,
) -> Result<BusinessData> {
    let mut b = business.clone();
    let options = WriteOptions {
        allow_import_source: false,
        evidence_of: None,
    };
    write_draft(&mut b, draft.clone(), options, env)?;
    Ok(b)
}

/// `writeDraft`'s options.
pub(crate) struct WriteOptions<'a> {
    /// Only `commit_import` passes `true`.
    pub(crate) allow_import_source: bool,
    /// The draft whose evidence a correction carries onto a new draft;
    /// otherwise the stored draft's evidence is the reference.
    pub(crate) evidence_of: Option<&'a ExpenseDraft>,
}

/// `writeDraft`: the body of `saveDraft`, shared with `commitImport` (which
/// may attach import evidence) and `correct`.
pub(crate) fn write_draft(
    b: &mut BusinessData,
    draft: ExpenseDraft,
    options: WriteOptions<'_>,
    env: &mut impl Env,
) -> Result<()> {
    let prior = b.drafts.iter().find(|x| x.id == draft.id);
    if draft
        .import_source
        .as_ref()
        .is_some_and(|s| s.business_id != b.profile.id)
    {
        return Err(LedgerError::ForeignImportSource);
    }
    // The TypeScript compares `JSON.stringify` output; see PARITY_NOTES.md.
    if prior.is_some_and(|p| p.import_source.is_some() && p.import_source != draft.import_source) {
        return Err(LedgerError::ImportSourceChanged);
    }
    let evidence = options
        .evidence_of
        .or(prior)
        .and_then(|d| d.import_source.as_ref());
    if !options.allow_import_source && evidence != draft.import_source.as_ref() {
        return Err(if evidence.is_some() {
            LedgerError::ImportSourceChanged
        } else {
            LedgerError::ImportSourceNotFromImport
        });
    }
    let payment = options
        .evidence_of
        .or(prior)
        .and_then(|d| d.payment_id.as_ref());
    if !options.allow_import_source && payment != draft.payment_id.as_ref() {
        return Err(LedgerError::PaymentNotFromImport);
    }
    if draft.business_id != b.profile.id
        || draft.status() != ExpenseStatus::Draft
        || b.drafts
            .iter()
            .any(|x| x.id == draft.id && x.status() != ExpenseStatus::Draft)
    {
        return Err(LedgerError::NotEditable);
    }
    if draft.document_ids.iter().any(|doc_id| {
        !b.documents
            .iter()
            .any(|x| &x.id == doc_id && x.business_id == draft.business_id)
    }) {
        return Err(LedgerError::InvalidReceipt);
    }
    // The stored draft: the caller's fields, and the authority-bearing ones
    // from the record's own history (PARITY_NOTES W13).
    let authority = DraftAuthority {
        payment_id: if options.allow_import_source {
            draft.payment_id.clone()
        } else {
            payment.cloned()
        },
        import_source: if options.allow_import_source {
            draft.import_source.clone()
        } else {
            evidence.cloned()
        },
        journal_id: prior.and_then(|p| p.journal_id.clone()),
        correction_of: options
            .evidence_of
            .map(|o| o.id.clone())
            .or_else(|| prior.and_then(|p| p.correction_of.clone())),
    };
    let draft = draft.written(authority);
    let sources = evidence_ids(&draft);
    let index = b.drafts.iter().position(|x| x.id == draft.id);
    let vendor = draft.vendor.clone();
    let draft_id = draft.id.clone();
    match index.and_then(|i| b.drafts.get_mut(i)) {
        Some(slot) => *slot = draft,
        None => b.drafts.push(draft),
    }
    if js::has_text(&vendor)
        && !b
            .vendors
            .iter()
            .any(|v| js::normalize(&v.name) == js::normalize(&vendor))
    {
        b.vendors.push(Vendor {
            id: env.new_id(),
            business_id: b.profile.id.clone(),
            name: js::trim(&vendor).to_string(),
        });
    }
    let (event_type, detail) = match index {
        None => (EventType::ExpenseCreated, "Expense draft created"),
        Some(_) => (EventType::ExpenseUpdated, "Expense draft updated"),
    };
    push_event(b, env, event_type, &draft_id, detail, sources);
    Ok(())
}

/// The receipts, payment, and import source a draft rests on.
fn evidence_ids(d: &ExpenseDraft) -> Vec<String> {
    let mut ids = d.document_ids.clone();
    ids.extend(truthy(&d.payment_id).map(str::to_string));
    ids.extend(d.import_source.as_ref().map(|s| s.id.clone()));
    ids
}

/// `approve`: posts a valid draft as a balanced expense journal and records
/// the approval. No payment is initiated.
pub fn approve(
    business: &BusinessData,
    expense_id: &str,
    env: &mut impl Env,
) -> Result<BusinessData> {
    let mut b = business.clone();
    approve_in(&mut b, expense_id, env)?;
    Ok(b)
}

fn approve_in(b: &mut BusinessData, expense_id: &str, env: &mut impl Env) -> Result<()> {
    let index = b
        .drafts
        .iter()
        .position(|x| x.id == expense_id && x.business_id == b.profile.id)
        .ok_or(LedgerError::ExpenseNotFound)?;
    let d = b
        .drafts
        .get(index)
        .cloned()
        .ok_or(LedgerError::ExpenseNotFound)?;
    if d.status() != ExpenseStatus::Draft {
        return Err(LedgerError::AlreadyPosted);
    }
    if b.approvals.iter().any(|x| x.expense_id == d.id)
        || b.journals
            .iter()
            .any(|x| x.expense_id.as_ref() == Some(&d.id))
    {
        return Err(LedgerError::DuplicatePosting);
    }
    let errors = validation(b, &d);
    if !errors.is_empty() {
        return Err(LedgerError::Validation(errors));
    }
    let other_posted = |x: &&ExpenseDraft| {
        // A real correction has already reversed and marked its original
        // corrected. A caller-supplied correction_of cannot exempt a posting.
        x.id != d.id && x.status() == ExpenseStatus::Posted
    };
    if let Some(source) = &d.import_source {
        if b.drafts.iter().filter(other_posted).any(|x| {
            x.import_source
                .as_ref()
                .is_some_and(|s| s.fingerprint == source.fingerprint)
        }) {
            return Err(LedgerError::ImportSourceAlreadyPosted);
        }
    }
    if let Some(payment_id) = truthy(&d.payment_id) {
        if b.drafts
            .iter()
            .filter(other_posted)
            .any(|x| x.payment_id.as_deref() == Some(payment_id))
        {
            return Err(LedgerError::PaymentAlreadyLinked);
        }
        let owner = b
            .payments
            .iter()
            .find(|p| p.id == payment_id && p.business_id == b.profile.id)
            .map(|p| p.draft_id.as_str());
        if owner != Some(d.id.as_str()) {
            return Err(LedgerError::PaymentOfOtherExpense);
        }
    }
    // TS: `b.categories.find((x) => x.id === d.categoryId && x.businessId === b.profile.id)!`
    let category = b
        .categories
        .iter()
        .find(|x| x.id == d.category_id && x.business_id == b.profile.id)
        .ok_or(LedgerError::Internal("approved expense has no category"))?;
    let lines = vec![
        JournalLine {
            account_id: category.account_id.clone(),
            category_id: Some(category.id.clone()),
            debit: d.amount_minor,
            credit: Minor::ZERO,
        },
        JournalLine {
            account_id: d.account_id.clone(),
            category_id: None,
            debit: Minor::ZERO,
            credit: d.amount_minor,
        },
    ];
    let journal = UncheckedJournal {
        id: env.new_id(),
        business_id: b.profile.id.clone(),
        date: d.date.clone(),
        currency: d.currency,
        lines,
        kind: JournalKind::Expense,
        expense_id: Some(d.id.clone()),
        reverses_id: None,
        memo: format!("{} · {}", d.vendor, d.description),
        created_at: env.now(),
    };
    let journal = JournalEntry::new(b, journal)?;
    let journal_id = journal.id.clone();
    b.journals.push(journal);
    if let Some(draft) = b.drafts.get_mut(index) {
        draft.mark_posted(journal_id.clone());
    }
    b.approvals.push(Approval {
        id: env.new_id(),
        business_id: b.profile.id.clone(),
        expense_id: d.id.clone(),
        actor: ACTOR.to_string(),
        at: env.now(),
        duplicate_reason: d.duplicate_reason.clone(),
    });
    push_event(
        b,
        env,
        EventType::ExpenseApproved,
        &d.id,
        "Reviewed and approved. No payment initiated.",
        evidence_ids(&d),
    );
    push_event(
        b,
        env,
        EventType::ExpensePosted,
        &d.id,
        "Balanced journal entry recorded",
        vec![journal_id],
    );
    Ok(())
}

/// `correct`: reverses a posted expense's journal and posts `replacement` in
/// its place. Either every step succeeds or the caller's data is unchanged.
pub fn correct(
    business: &BusinessData,
    original_id: &str,
    replacement: &ExpenseDraft,
    reason: &str,
    env: &mut impl Env,
) -> Result<BusinessData> {
    let mut b = business.clone();
    let index = b
        .drafts
        .iter()
        .position(|x| x.id == original_id && x.business_id == b.profile.id);
    let original = index
        .and_then(|i| b.drafts.get(i))
        .filter(|o| o.status() == ExpenseStatus::Posted && js::has_text(reason))
        .cloned()
        .ok_or(LedgerError::CorrectionRequiresPosted)?;
    if replacement.id == original.id
        || b.drafts.iter().any(|x| x.id == replacement.id)
        || replacement.business_id != b.profile.id
        || replacement.status() != ExpenseStatus::Draft
    {
        return Err(LedgerError::InvalidReplacement);
    }
    if original.payment_id != replacement.payment_id {
        return Err(LedgerError::PaymentLinkChanged);
    }
    // The TypeScript compares `JSON.stringify` output; see PARITY_NOTES.md.
    if original.import_source != replacement.import_source {
        return Err(LedgerError::ImportSourceNotPreserved);
    }
    // TS: `b.journals.find((x) => x.id === original.journalId)!`
    let prior = b
        .journals
        .iter()
        .find(|x| Some(&x.id) == original.journal_id.as_ref())
        .ok_or(LedgerError::Internal("posted expense has no journal"))?;
    let mut reversal = prior.to_unchecked();
    reversal.id = env.new_id();
    reversal.kind = JournalKind::Reversal;
    reversal.reverses_id = Some(prior.id.clone());
    reversal.memo = format!("Correction: {reason}");
    for line in &mut reversal.lines {
        std::mem::swap(&mut line.debit, &mut line.credit);
    }
    reversal.created_at = env.now();
    let reversal = JournalEntry::new(&b, reversal)?;
    let sources = vec![
        prior.id.clone(),
        reversal.id.clone(),
        replacement.id.clone(),
    ];
    if let Some(draft) = index.and_then(|i| b.drafts.get_mut(i)) {
        draft.mark_corrected();
    }
    b.journals.push(reversal);
    push_event(
        &mut b,
        env,
        EventType::ExpenseReversed,
        &original.id,
        reason,
        sources,
    );
    // write_draft records the replacement as a correction of `evidence_of`.
    let replacement = replacement.clone();
    let replacement_id = replacement.id.clone();
    let options = WriteOptions {
        allow_import_source: false,
        evidence_of: Some(&original),
    };
    write_draft(&mut b, replacement, options, env)?;
    // The replacement takes over the original's payment.
    if let Some(payment_id) = truthy(&original.payment_id) {
        let business_id = &b.profile.id;
        if let Some(p) = b
            .payments
            .iter_mut()
            .find(|p| p.id == payment_id && p.business_id == *business_id)
        {
            p.draft_id = replacement_id.clone();
        }
    }
    approve_in(&mut b, &replacement_id, env)?;
    Ok(b)
}

/// One row of [`category_totals`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryTotal {
    pub category_id: String,
    pub name: String,
    pub amount: Minor,
}

/// `categoryTotals`: net spend per category for this business's journals in
/// one currency whose date starts with `period`, largest first, zero totals
/// dropped.
pub fn category_totals(
    b: &BusinessData,
    period: &str,
    currency: Currency,
) -> Result<Vec<CategoryTotal>> {
    // A `Map` in the TypeScript: keep first-seen order for stable sorting.
    let mut order: Vec<String> = Vec::new();
    let mut sums: HashMap<String, Minor> = HashMap::new();
    for j in &b.journals {
        if j.business_id != b.profile.id || j.currency != currency || !j.date.starts_with(period) {
            continue;
        }
        for line in &j.lines {
            let Some(category_id) = truthy(&line.category_id) else {
                continue;
            };
            let delta = line.debit.checked_sub(line.credit)?;
            let total = match sums.get(category_id) {
                Some(total) => total.checked_add(delta)?,
                None => {
                    order.push(category_id.to_string());
                    Minor::ZERO.checked_add(delta)?
                }
            };
            sums.insert(category_id.to_string(), total);
        }
    }
    let mut rows: Vec<CategoryTotal> = order
        .into_iter()
        .map(|category_id| {
            let amount = sums.get(&category_id).copied().unwrap_or_default();
            let name = b
                .categories
                .iter()
                .find(|c| c.id == category_id && c.business_id == b.profile.id)
                .map_or_else(|| "Unknown".to_string(), |c| c.name.clone());
            CategoryTotal {
                category_id,
                name,
                amount,
            }
        })
        .filter(|x| x.amount != Minor::ZERO)
        .collect();
    // Stable, like `Array.prototype.sort`: ties keep first-seen order.
    rows.sort_by_key(|x| std::cmp::Reverse(x.amount));
    Ok(rows)
}

/// `totals`: net categorized spend per currency for this business's journals
/// whose date starts with `period`. Currencies are never combined.
pub fn totals(b: &BusinessData, period: &str) -> Result<BTreeMap<Currency, Minor>> {
    let mut result = BTreeMap::new();
    for j in &b.journals {
        if j.business_id != b.profile.id || !j.date.starts_with(period) {
            continue;
        }
        for line in &j.lines {
            if truthy(&line.category_id).is_none() {
                continue;
            }
            let delta = line.debit.checked_sub(line.credit)?;
            let total = result.get(&j.currency).copied().unwrap_or_default();
            result.insert(j.currency, Minor::checked_add(total, delta)?);
        }
    }
    Ok(result)
}

/// JavaScript truthiness for an optional string: `undefined` and `""` are
/// both false.
pub(crate) fn truthy(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|s| !s.is_empty())
}
