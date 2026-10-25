//! Ledger commands and queries (`src/domain/ledger.ts`).

use crate::date::valid_date;
use crate::error::{LedgerError, Result};
use crate::money::Minor;
use crate::types::{check_line_amounts, check_totals, AccountKind, BusinessData, UncheckedJournal};

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

/// JavaScript truthiness for an optional string: `undefined` and `""` are
/// both false.
pub(crate) fn truthy(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|s| !s.is_empty())
}
