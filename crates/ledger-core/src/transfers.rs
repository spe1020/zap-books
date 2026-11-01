//! Transfers, opening balances and account balances
//! (`src/domain/transfers.ts`).

use serde::{Deserialize, Serialize};

use crate::bitcoin::{fiat_value_error, millisats_error};
use crate::date::valid_date;
use crate::env::Env;
use crate::error::{LedgerError, Result};
use crate::js;
use crate::ledger::push_event;
use crate::money::{Currency, Minor};
use crate::types::{
    Account, AccountKind, BusinessData, EventType, FiatSource, FiatValue, JournalEntry,
    JournalKind, JournalLine, TransferType, UncheckedJournal,
};

/// `TransferInput`: money moving between two of the business's own
/// non-expense accounts. `OpeningBalance` is refused by
/// [`record_transfer`]; only a correction of an opening balance passes it.
///
/// An `Exchange` buys or sells bitcoin: `currency` is BTC and `amount_minor`
/// the sats, one account a BTC wallet and the other a fiat bank or card
/// account, and `fiat_value` (source `exchange`) holds the fiat side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferInput {
    pub date: String,
    /// Positive minor units.
    pub amount_minor: Minor,
    pub currency: Currency,
    pub from_account_id: String,
    pub to_account_id: String,
    pub transfer_type: TransferType,
    pub purpose: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub document_ids: Vec<String>,
    /// Required in BTC, refused otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fiat_value: Option<FiatValue>,
    /// A Lightning amount as paid; `amount_minor` is it rounded to sats.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub millisats: Option<Minor>,
}

/// `OpeningBalanceInput`: an account's balance before the business's first
/// entry, as [`balances`] reports it: what's in a bank account, or what's
/// owed on a card or loan account. It may be negative, never zero.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpeningBalanceInput {
    pub account_id: String,
    pub date: String,
    pub amount_minor: Minor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    /// Required for a BTC account, refused otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fiat_value: Option<FiatValue>,
}

/// One row of [`balances`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Balance {
    pub account_id: String,
    pub name: String,
    pub kind: AccountKind,
    pub balance: Minor,
}

/// `transferTypeLabels`: what each transfer type is called on screens and
/// in exports.
pub fn transfer_type_label(t: TransferType) -> &'static str {
    match t {
        TransferType::OwnerLoan => "Owner loan",
        TransferType::OwnerContribution => "Owner contribution",
        TransferType::LoanRepayment => "Loan repayment",
        TransferType::OwnerDraw => "Owner draw",
        TransferType::BetweenAccounts => "Transfer between accounts",
        TransferType::OpeningBalance => "Opening balance",
        TransferType::Exchange => "Exchange",
    }
}

/// `accountKindLabels`
pub(crate) fn account_kind_label(k: AccountKind) -> &'static str {
    match k {
        AccountKind::Bank => "Bank account",
        AccountKind::Card => "Credit card",
        AccountKind::Liability => "Loan",
        AccountKind::Equity => "Owner equity",
        AccountKind::Expense => "Expense",
        AccountKind::Wallet => "Bitcoin wallet",
    }
}

/// `balanceMeanings`: money held in a bank account, owed on a card, owed to
/// the owner on a loan, or the owner's stake. `balances` never returns an
/// expense account.
pub(crate) fn balance_meaning(k: AccountKind) -> &'static str {
    match k {
        AccountKind::Bank | AccountKind::Wallet => "Held",
        AccountKind::Card => "Owed",
        AccountKind::Liability => "Owed to owner",
        AccountKind::Equity => "Owner's stake",
        AccountKind::Expense => "",
    }
}

fn paying(k: AccountKind) -> bool {
    matches!(
        k,
        AccountKind::Bank | AccountKind::Card | AccountKind::Wallet
    )
}

/// `holding`: accounts whose balance is debits minus credits.
fn holding(k: AccountKind) -> bool {
    matches!(k, AccountKind::Bank | AccountKind::Wallet)
}

/// `openable`: the accounts opening balances are for.
fn openable(k: AccountKind) -> bool {
    matches!(
        k,
        AccountKind::Bank | AccountKind::Card | AccountKind::Liability | AccountKind::Wallet
    )
}

/// A user-chosen transfer type's label, the account kinds it moves money
/// from and to, and its message when they don't fit.
type Shape = (
    &'static str,
    fn(AccountKind) -> bool,
    fn(AccountKind) -> bool,
    &'static str,
);

/// Which accounts a transfer type moves money between. `None` for an
/// opening balance or an exchange.
fn shape(t: TransferType) -> Option<Shape> {
    let liability = |k| k == AccountKind::Liability;
    let equity = |k| k == AccountKind::Equity;
    Some(match t {
        TransferType::OwnerLoan => (
            transfer_type_label(t),
            liability,
            paying,
            "An owner loan moves money from a loan account into a bank or card account.",
        ),
        TransferType::OwnerContribution => (
            transfer_type_label(t),
            equity,
            paying,
            "An owner contribution moves money from an owner equity account into a bank or card account.",
        ),
        TransferType::LoanRepayment => (
            transfer_type_label(t),
            paying,
            liability,
            "A loan repayment moves money from a bank or card account into a loan account.",
        ),
        TransferType::OwnerDraw => (
            transfer_type_label(t),
            paying,
            equity,
            "An owner draw moves money from a bank or card account into an owner equity account.",
        ),
        TransferType::BetweenAccounts => (
            transfer_type_label(t),
            paying,
            paying,
            "A transfer between accounts moves money from one bank or card account to another.",
        ),
        TransferType::OpeningBalance | TransferType::Exchange => return None,
    })
}

fn is_opening(j: &UncheckedJournal) -> bool {
    j.transfer_type == Some(TransferType::OpeningBalance)
}

fn reversed(b: &BusinessData, journal_id: &str) -> bool {
    b.journals
        .iter()
        .any(|j| j.reverses_id.as_deref() == Some(journal_id))
}

/// `openingBalancesName`: the equity account opening balances in `currency`
/// post against. A business with accounts in several currencies gets one
/// per currency, since every account has one currency and names are unique.
pub fn opening_balances_name(b: &BusinessData, currency: Currency) -> String {
    if currency == b.profile.currency {
        "Opening balances".to_string()
    } else {
        format!("Opening balances ({currency})")
    }
}

/// The opening balance journal on `account_id` still in effect.
fn has_opening_balance(b: &BusinessData, account_id: &str) -> bool {
    b.journals.iter().any(|j| {
        j.business_id == b.profile.id
            && j.kind == JournalKind::Transfer
            && is_opening(j)
            && !reversed(b, &j.id)
            && j.lines.iter().any(|l| l.account_id == account_id)
    })
}

/// `post`: checks `input` against the business and appends its journal.
fn post(
    b: &mut BusinessData,
    input: &TransferInput,
    correction_of: Option<&str>,
    env: &mut impl Env,
) -> Result<String> {
    if !valid_date(&input.date) {
        return Err(LedgerError::InvalidTransferDate);
    }
    // `Number.isSafeInteger` holds by construction, and so does
    // `currencies.includes(input.currency)` (D2).
    if input.amount_minor <= Minor::ZERO {
        return Err(LedgerError::TransferAmount);
    }
    if !js::has_text(&input.purpose) {
        return Err(LedgerError::TransferPurpose);
    }
    let find = |id: &str| {
        b.accounts
            .iter()
            .find(|a| a.id == id && a.business_id == b.profile.id)
            .cloned()
    };
    let (Some(from), Some(to)) = (find(&input.from_account_id), find(&input.to_account_id)) else {
        return Err(LedgerError::TransferAccounts);
    };
    if from.id == to.id {
        return Err(LedgerError::TransferSameAccount);
    }
    if input.transfer_type == TransferType::Exchange {
        return post_exchange(b, input, &from, &to, correction_of, env);
    }
    if from.currency != input.currency || to.currency != input.currency {
        return Err(LedgerError::TransferCurrency);
    }
    let label = match shape(input.transfer_type) {
        None => {
            check_opening_balance(b, input, &from, &to)?;
            "Opening balance"
        }
        Some((label, from_ok, to_ok, message)) => {
            if !from_ok(from.kind) || !to_ok(to.kind) {
                return Err(LedgerError::TransferShape(message));
            }
            label
        }
    };
    check_values(input)?;
    let lines = vec![
        JournalLine {
            account_id: to.id.clone(),
            category_id: None,
            debit: input.amount_minor,
            credit: Minor::ZERO,
        },
        JournalLine {
            account_id: from.id.clone(),
            category_id: None,
            debit: Minor::ZERO,
            credit: input.amount_minor,
        },
    ];
    append(b, input, &from, &to, label, correction_of, lines, env)
}

/// The fiat value and millisat rules every transfer keeps.
fn check_values(input: &TransferInput) -> Result<()> {
    let error = fiat_value_error(
        input.currency,
        input.amount_minor,
        input.fiat_value.as_ref(),
    )
    .or_else(|| millisats_error(input.currency, input.amount_minor, input.millisats));
    match error {
        Some(error) => Err(LedgerError::Bitcoin(error)),
        None => Ok(()),
    }
}

/// `postExchange`: sats into or out of a BTC wallet, against a fiat bank or
/// card account for the fiat value's amount.
fn post_exchange(
    b: &mut BusinessData,
    input: &TransferInput,
    from: &Account,
    to: &Account,
    correction_of: Option<&str>,
    env: &mut impl Env,
) -> Result<String> {
    if input.currency != Currency::Btc {
        return Err(LedgerError::Bitcoin(
            "An exchange is recorded in sats, with its other side as the fiat value.",
        ));
    }
    let wallet = |a: &Account| a.kind == AccountKind::Wallet && a.currency == Currency::Btc;
    let fiat = |a: &Account| {
        matches!(a.kind, AccountKind::Bank | AccountKind::Card) && a.currency.is_fiat()
    };
    if !((wallet(from) && fiat(to)) || (fiat(from) && wallet(to))) {
        return Err(LedgerError::Bitcoin(
            "An exchange moves money between a bitcoin wallet and a dollar, euro or pound bank or card account.",
        ));
    }
    check_values(input)?;
    let value = input.fiat_value.as_ref().ok_or(LedgerError::Internal(
        "an exchange passed without a fiat value",
    ))?;
    let fiat_account = if wallet(from) { to } else { from };
    if value.source != FiatSource::Exchange {
        return Err(LedgerError::Bitcoin(
            "An exchange records its fiat value with the source exchange.",
        ));
    }
    if value.currency != fiat_account.currency {
        return Err(LedgerError::Bitcoin(
            "An exchange's fiat value is in the currency of its dollar, euro or pound account.",
        ));
    }
    if value.amount_minor <= Minor::ZERO {
        return Err(LedgerError::Bitcoin(
            "An exchange needs a fiat amount greater than zero.",
        ));
    }
    let amount = |a: &Account| {
        if a.id == fiat_account.id {
            value.amount_minor
        } else {
            input.amount_minor
        }
    };
    let lines = vec![
        JournalLine {
            account_id: to.id.clone(),
            category_id: None,
            debit: amount(to),
            credit: Minor::ZERO,
        },
        JournalLine {
            account_id: from.id.clone(),
            category_id: None,
            debit: Minor::ZERO,
            credit: amount(from),
        },
    ];
    append(b, input, from, to, "Exchange", correction_of, lines, env)
}

/// `append`: checks the receipts, builds the transfer's journal and
/// appends it.
#[allow(clippy::too_many_arguments)]
fn append(
    b: &mut BusinessData,
    input: &TransferInput,
    from: &Account,
    to: &Account,
    label: &str,
    correction_of: Option<&str>,
    lines: Vec<JournalLine>,
    env: &mut impl Env,
) -> Result<String> {
    if input.document_ids.iter().any(|doc_id| {
        !b.documents
            .iter()
            .any(|x| &x.id == doc_id && x.business_id == b.profile.id)
    }) {
        return Err(LedgerError::TransferReceipt);
    }
    let journal = UncheckedJournal {
        id: env.new_id(),
        business_id: b.profile.id.clone(),
        date: input.date.clone(),
        currency: input.currency,
        lines,
        kind: JournalKind::Transfer,
        expense_id: None,
        reverses_id: None,
        memo: format!("{} → {}", from.name, to.name),
        created_at: env.now(),
        transfer_type: Some(input.transfer_type),
        purpose: Some(js::trim(&input.purpose).to_string()),
        reference: input
            .reference
            .as_deref()
            .map(js::trim)
            .filter(|r| !r.is_empty())
            .map(str::to_string),
        document_ids: Some(input.document_ids.clone()).filter(|d| !d.is_empty()),
        correction_of: correction_of.map(str::to_string),
        fiat_value: input.fiat_value.clone(),
        millisats: input.millisats,
    };
    let journal = JournalEntry::new(b, journal)?;
    let id = journal.id.clone();
    b.journals.push(journal);
    push_event(
        b,
        env,
        EventType::TransferRecorded,
        &id,
        &format!("{label} recorded"),
        input.document_ids.clone(),
    );
    Ok(id)
}

/// `checkOpeningBalance`: between this currency's opening balances account
/// and one bank, card or loan account, at most one per account, dated on or
/// before every other entry.
fn check_opening_balance(
    b: &BusinessData,
    input: &TransferInput,
    from: &Account,
    to: &Account,
) -> Result<()> {
    let name = opening_balances_name(b, input.currency).to_lowercase();
    let equity = |a: &Account| a.kind == AccountKind::Equity && a.name.to_lowercase() == name;
    let account = if equity(from) {
        Some(to)
    } else if equity(to) {
        Some(from)
    } else {
        None
    };
    // The message predates wallets, which may also take one.
    let Some(account) = account.filter(|a| openable(a.kind)) else {
        return Err(LedgerError::OpeningBalanceAccountKind);
    };
    // A wallet's opening balance is a debit; one credited to it would open
    // below zero, and a wallet can't hold less than no bitcoin.
    if account.kind == AccountKind::Wallet && account.id == from.id {
        return Err(LedgerError::Bitcoin(
            "A wallet can't open below zero. Enter its opening balance as sats greater than zero.",
        ));
    }
    if has_opening_balance(b, &account.id) {
        return Err(LedgerError::OpeningBalanceExists);
    }
    let first = b
        .journals
        .iter()
        .filter(|j| j.business_id == b.profile.id && !is_opening(j))
        .map(|j| j.date.as_str())
        .min();
    if let Some(first) = first {
        if input.date.as_str() > first {
            return Err(LedgerError::OpeningBalanceTooLate(first.to_string()));
        }
    }
    Ok(())
}

/// `recordTransfer`: posts a transfer at once; there are no transfer drafts.
pub fn record_transfer(
    business: &BusinessData,
    input: &TransferInput,
    env: &mut impl Env,
) -> Result<BusinessData> {
    if input.transfer_type == TransferType::OpeningBalance {
        return Err(LedgerError::OpeningBalanceNotTransfer);
    }
    let mut b = business.clone();
    post(&mut b, input, None, env)?;
    Ok(b)
}

/// `correctTransfer`: reverses a posted transfer exactly and posts
/// `replacement` in its place, all or nothing. The reversal names the
/// original (`reverses_id`), and so does the replacement (`correction_of`).
/// A transfer already reversed can't be corrected again. An opening balance
/// is corrected by another, on the same account, and an exchange by another
/// between the same two accounts.
pub fn correct_transfer(
    business: &BusinessData,
    original_id: &str,
    replacement: &TransferInput,
    reason: &str,
    env: &mut impl Env,
) -> Result<BusinessData> {
    let mut b = business.clone();
    let original = b
        .journals
        .iter()
        .find(|j| j.id == original_id && j.business_id == b.profile.id)
        .filter(|j| j.kind == JournalKind::Transfer && js::has_text(reason))
        .map(JournalEntry::to_unchecked)
        .ok_or(LedgerError::TransferCorrectionRequiresPosted)?;
    if reversed(&b, &original.id) {
        return Err(LedgerError::TransferAlreadyCorrected);
    }
    if is_opening(&original) != (replacement.transfer_type == TransferType::OpeningBalance) {
        return Err(LedgerError::TransferCorrectionKind);
    }
    // The same account and the same opening balances account; the amount,
    // its direction and the date may change.
    let on_original = |id: &str| original.lines.iter().any(|l| l.account_id == id);
    if is_opening(&original)
        && !(on_original(&replacement.from_account_id) && on_original(&replacement.to_account_id))
    {
        return Err(LedgerError::OpeningBalanceAccountChanged);
    }
    // An exchange is corrected by another between the same wallet and the
    // same fiat account; the amounts, direction, date and rate may change.
    if original.transfer_type == Some(TransferType::Exchange) {
        if replacement.transfer_type != TransferType::Exchange {
            return Err(LedgerError::Bitcoin(
                "Correct an exchange with another exchange.",
            ));
        }
        let sides = [&replacement.from_account_id, &replacement.to_account_id];
        let is_wallet = |id: &str| {
            b.accounts
                .iter()
                .any(|a| a.id == id && a.kind == AccountKind::Wallet)
        };
        let kept = |wallet: bool| {
            original
                .lines
                .iter()
                .all(|l| is_wallet(&l.account_id) != wallet || sides.contains(&&l.account_id))
        };
        if !kept(true) {
            return Err(LedgerError::Bitcoin(
                "An exchange correction keeps its wallet.",
            ));
        }
        if !kept(false) {
            return Err(LedgerError::Bitcoin(
                "An exchange correction keeps its dollar, euro or pound account.",
            ));
        }
    }
    let mut reversal = original.clone();
    reversal.correction_of = None;
    reversal.id = env.new_id();
    reversal.kind = JournalKind::Reversal;
    reversal.reverses_id = Some(original.id.clone());
    reversal.memo = format!("Correction: {reason}");
    for line in &mut reversal.lines {
        std::mem::swap(&mut line.debit, &mut line.credit);
    }
    reversal.created_at = env.now();
    let reversal = JournalEntry::new(&b, reversal)?;
    let reversal_id = reversal.id.clone();
    b.journals.push(reversal);
    let posted = post(&mut b, replacement, Some(&original.id), env)?;
    push_event(
        &mut b,
        env,
        EventType::TransferReversed,
        &original.id,
        reason,
        vec![original.id.clone(), reversal_id, posted],
    );
    Ok(b)
}

/// `recordOpeningBalance`: sets an account's balance before the business's
/// first entry, against this currency's opening balances equity account,
/// which is added the first time it's needed.
pub fn record_opening_balance(
    business: &BusinessData,
    input: &OpeningBalanceInput,
    env: &mut impl Env,
) -> Result<BusinessData> {
    let mut b = business.clone();
    let account = b
        .accounts
        .iter()
        .find(|a| a.id == input.account_id && a.business_id == b.profile.id)
        .cloned()
        .ok_or(LedgerError::OpeningBalanceAccountNotFound)?;
    if !openable(account.kind) {
        return Err(LedgerError::OpeningBalanceAccountKind);
    }
    if input.amount_minor == Minor::ZERO {
        return Err(LedgerError::OpeningBalanceZero);
    }
    let equity = opening_balances_account(&mut b, account.currency, env)?;
    // A bank account's or wallet's balance is a debit; what's owed on a
    // card or loan is a credit.
    let debit = holding(account.kind) == (input.amount_minor > Minor::ZERO);
    let amount = if input.amount_minor < Minor::ZERO {
        -input.amount_minor
    } else {
        input.amount_minor
    };
    let (from, to) = if debit {
        (equity, account.id)
    } else {
        (account.id, equity)
    };
    let transfer = TransferInput {
        date: input.date.clone(),
        amount_minor: amount,
        currency: account.currency,
        from_account_id: from,
        to_account_id: to,
        transfer_type: TransferType::OpeningBalance,
        purpose: "Opening balance".to_string(),
        reference: input.reference.clone(),
        document_ids: Vec::new(),
        fiat_value: input.fiat_value.clone(),
        millisats: None,
    };
    post(&mut b, &transfer, None, env)?;
    Ok(b)
}

/// This currency's opening balances account's id, added if there is none.
fn opening_balances_account(
    b: &mut BusinessData,
    currency: Currency,
    env: &mut impl Env,
) -> Result<String> {
    let name = opening_balances_name(b, currency);
    let wanted = name.to_lowercase();
    let existing = b
        .accounts
        .iter()
        .find(|a| a.business_id == b.profile.id && a.name.to_lowercase() == wanted);
    if let Some(existing) = existing {
        if existing.kind != AccountKind::Equity || existing.currency != currency {
            return Err(LedgerError::OpeningBalancesNameTaken(name));
        }
        return Ok(existing.id.clone());
    }
    let id = env.new_id();
    b.accounts.push(Account {
        id: id.clone(),
        business_id: b.profile.id.clone(),
        name: name.clone(),
        kind: AccountKind::Equity,
        currency,
    });
    let profile_id = b.profile.id.clone();
    push_event(
        b,
        env,
        EventType::SettingsUpdated,
        &profile_id,
        &format!("account saved: {name}"),
        Vec::new(),
    );
    Ok(id)
}

/// `balances`: every non-expense account in `currency` with its balance on
/// `as_of`, from journals dated on or before it: debits minus credits for
/// bank accounts and wallets, and credits minus debits for card, loan and
/// equity accounts, so a card's balance is what's owed on it. BTC balances
/// are in sats. An exchange moves both of its accounts, each in its own
/// currency. Reversals offset their originals. Currencies are never
/// combined.
pub fn balances(b: &BusinessData, as_of: &str, currency: Currency) -> Result<Vec<Balance>> {
    if !valid_date(as_of) {
        return Err(LedgerError::InvalidBalanceDate);
    }
    let mut rows: Vec<Balance> = b
        .accounts
        .iter()
        .filter(|a| {
            a.business_id == b.profile.id
                && a.kind != AccountKind::Expense
                && a.currency == currency
        })
        .map(|a| Balance {
            account_id: a.id.clone(),
            name: a.name.clone(),
            kind: a.kind,
            balance: Minor::ZERO,
        })
        .collect();
    for j in &b.journals {
        let exchange_in = j.transfer_type == Some(TransferType::Exchange)
            && j.fiat_value
                .as_ref()
                .is_some_and(|v| v.currency == currency);
        if j.business_id != b.profile.id
            || !(j.currency == currency || exchange_in)
            || j.date.as_str() > as_of
        {
            continue;
        }
        // Only lines on accounts in `currency` find a row.
        for line in &j.lines {
            let Some(row) = rows.iter_mut().find(|r| r.account_id == line.account_id) else {
                continue;
            };
            let delta = if holding(row.kind) {
                line.debit.checked_sub(line.credit)?
            } else {
                line.credit.checked_sub(line.debit)?
            };
            row.balance = row.balance.checked_add(delta)?;
        }
    }
    Ok(rows)
}
