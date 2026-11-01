//! Business-scoped CSV exports (`src/domain/export.ts`).

use std::cmp::Ordering;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::bitcoin::line_currency;
use crate::date::valid_date;
use crate::error::{LedgerError, Result};
use crate::js;
use crate::money::{amount_input, Currency, Minor};
use crate::transfers::{account_kind_label, balance_meaning, balances, transfer_type_label};
use crate::types::{
    AccountKind, BusinessData, ExpenseStatus, FiatValue, JournalKind, TransferType,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Expenses,
    Journal,
}

impl ExportFormat {
    fn as_str(self) -> &'static str {
        match self {
            ExportFormat::Expenses => "expenses",
            ExportFormat::Journal => "journal",
        }
    }
}

/// Inclusive date range and currency. Empty strings and `None` mean "no
/// limit"; on the wire, `currency` is `""` for all currencies.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportFilter {
    pub from: String,
    pub to: String,
    #[serde(
        serialize_with = "serialize_currency_filter",
        deserialize_with = "deserialize_currency_filter"
    )]
    pub currency: Option<Currency>,
}

fn serialize_currency_filter<S: Serializer>(
    currency: &Option<Currency>,
    s: S,
) -> std::result::Result<S::Ok, S::Error> {
    s.serialize_str(currency.map_or("", Currency::code))
}

fn deserialize_currency_filter<'de, D: Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<Currency>, D::Error> {
    let code = String::deserialize(d)?;
    if code.is_empty() {
        return Ok(None);
    }
    Currency::from_code(&code)
        .map(Some)
        .ok_or_else(|| serde::de::Error::custom(format!("unsupported currency {code}")))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvExport {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub filename: String,
    pub csv: String,
}

/// `csvCell`: quotes every cell and prefixes `'` to text a spreadsheet
/// would run as a formula (`[\s﻿]*[=+\-@|]` or a leading tab, CR, LF),
/// except a sign followed only by digits, `.` and `,`, which a spreadsheet
/// reads as a number (`^[+-]\d[\d.,]*$`).
fn csv_cell(value: &str) -> String {
    let formula = (value
        .trim_start_matches(js::is_space)
        .starts_with(['=', '+', '-', '@', '|'])
        && !signed_number(value))
        || value.starts_with(['\t', '\r', '\n']);
    let literal = if formula {
        format!("'{value}")
    } else {
        value.to_string()
    };
    format!("\"{}\"", literal.replace('"', "\"\""))
}

/// `/^[+-]\d[\d.,]*$/`, with `\d` ASCII as in JavaScript.
fn signed_number(value: &str) -> bool {
    let Some(rest) = value.strip_prefix(['+', '-']) else {
        return false;
    };
    rest.starts_with(|c: char| c.is_ascii_digit())
        && rest
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.' || c == ',')
}

/// `encodeCSV`: UTF-8 BOM, every cell quoted, CRLF line endings.
pub fn encode_csv(rows: &[Vec<String>]) -> String {
    let lines: Vec<String> = rows
        .iter()
        .map(|r| r.iter().map(|c| csv_cell(c)).collect::<Vec<_>>().join(","))
        .collect();
    format!("\u{FEFF}{}\r\n", lines.join("\r\n"))
}

/// `a.localeCompare(z)`. See PARITY_NOTES.md: this compares by UTF-16 code
/// unit, which agrees with ICU collation for the ISO dates, timestamps, and
/// lower-case ids the exports sort by.
fn locale_compare(a: &str, z: &str) -> Ordering {
    js::cmp(a, z)
}

/// `exportRecords`: posted expenses or journal lines for this business only,
/// filtered by date range and currency, in a stable order.
pub fn export_records(
    b: &BusinessData,
    format: ExportFormat,
    filter: &ExportFilter,
) -> Result<CsvExport> {
    if (!filter.from.is_empty() && !valid_date(&filter.from))
        || (!filter.to.is_empty() && !valid_date(&filter.to))
    {
        return Err(LedgerError::InvalidExportDates);
    }
    if !filter.from.is_empty()
        && !filter.to.is_empty()
        && js::cmp(&filter.from, &filter.to) == Ordering::Greater
    {
        return Err(LedgerError::ExportRangeReversed);
    }
    let in_range = |date: &str, currency: Currency| {
        (filter.from.is_empty() || js::cmp(date, &filter.from) != Ordering::Less)
            && (filter.to.is_empty() || js::cmp(date, &filter.to) != Ordering::Greater)
            && filter.currency.is_none_or(|c| c == currency)
    };
    let account_of = |id: &str| {
        b.accounts
            .iter()
            .find(|a| a.id == id && a.business_id == b.profile.id)
    };
    let account = |id: &str| account_of(id).map(|a| a.name.clone()).unwrap_or_default();
    let category = |id: Option<&str>| {
        b.categories
            .iter()
            .find(|c| Some(c.id.as_str()) == id && c.business_id == b.profile.id)
            .map(|c| c.name.clone())
            .unwrap_or_default()
    };
    let (headers, rows): (&[&str], Vec<Vec<String>>) = match format {
        ExportFormat::Expenses => {
            let mut drafts: Vec<_> = b
                .drafts
                .iter()
                .filter(|d| {
                    d.business_id == b.profile.id
                        && d.status() == ExpenseStatus::Posted
                        && in_range(&d.date, d.currency)
                })
                .collect();
            drafts.sort_by(|a, z| {
                locale_compare(&a.date, &z.date).then_with(|| locale_compare(&a.id, &z.id))
            });
            let rows = drafts
                .into_iter()
                .map(|d| {
                    let payment = b.payments.iter().find(|p| {
                        Some(&p.id) == d.payment_id.as_ref() && p.business_id == b.profile.id
                    });
                    let source_system = match (&d.import_source, payment) {
                        (Some(s), _) => s.provider.as_str().to_string(),
                        (None, Some(_)) => "Payment CSV".to_string(),
                        (None, None) => String::new(),
                    };
                    let source_reference = match (&d.import_source, payment) {
                        (Some(s), _) => s.reference.clone(),
                        (None, Some(p)) => p.reference.clone(),
                        (None, None) => String::new(),
                    };
                    vec![
                        b.profile.id.clone(),
                        b.profile.name.clone(),
                        d.id.clone(),
                        d.date.clone(),
                        d.vendor.clone(),
                        d.description.clone(),
                        amount_input(d.amount_minor, d.currency),
                        d.currency.code().to_string(),
                        category(Some(&d.category_id)),
                        account(&d.account_id),
                        d.purpose.clone(),
                        d.correction_of.clone().unwrap_or_default(),
                        source_system,
                        source_reference,
                        d.document_ids.len().to_string(),
                        // Paid from a loan account, it's owed to the owner;
                        // from an equity account, it's their contribution.
                        match account_of(&d.account_id).map(|a| a.kind) {
                            Some(AccountKind::Liability) => "Loan",
                            Some(AccountKind::Equity) => "Contribution",
                            _ => "",
                        }
                        .to_string(),
                        sats(d.amount_minor, d.currency),
                    ]
                    .into_iter()
                    .chain(fiat_cells(d.fiat_value.as_ref(), d.millisats))
                    .collect::<Vec<_>>()
                })
                .collect();
            (&EXPENSE_HEADERS, rows)
        }
        ExportFormat::Journal => {
            let mut journals: Vec<_> = b
                .journals
                .iter()
                // An exchange is in both of its currencies.
                .filter(|j| {
                    j.business_id == b.profile.id
                        && (in_range(&j.date, j.currency)
                            || (j.transfer_type == Some(TransferType::Exchange)
                                && j.fiat_value
                                    .as_ref()
                                    .is_some_and(|v| in_range(&j.date, v.currency))))
                })
                .collect();
            journals.sort_by(|a, z| {
                locale_compare(&a.date, &z.date)
                    .then_with(|| locale_compare(&a.created_at, &z.created_at))
            });
            let rows = journals
                .into_iter()
                .flat_map(|j| {
                    j.lines.iter().map(|l| {
                        let currency = line_currency(j, account_of(&l.account_id).map(|a| a.kind));
                        vec![
                            b.profile.id.clone(),
                            b.profile.name.clone(),
                            j.id.clone(),
                            j.date.clone(),
                            journal_kind(j.kind).to_string(),
                            j.expense_id.clone().unwrap_or_default(),
                            j.reverses_id.clone().unwrap_or_default(),
                            l.account_id.clone(),
                            account(&l.account_id),
                            category(l.category_id.as_deref()),
                            amount_input(l.debit, currency),
                            amount_input(l.credit, currency),
                            currency.code().to_string(),
                            j.memo.clone(),
                            j.created_at.clone(),
                            j.transfer_type
                                .map(transfer_type_label)
                                .unwrap_or_default()
                                .to_string(),
                            j.purpose.clone().unwrap_or_default(),
                            j.reference.clone().unwrap_or_default(),
                            j.correction_of.clone().unwrap_or_default(),
                            sats(l.debit, currency),
                            sats(l.credit, currency),
                        ]
                        .into_iter()
                        // The journal's recorded value, the same on each line.
                        .chain(fiat_cells(j.fiat_value.as_ref(), j.millisats))
                        .collect::<Vec<_>>()
                    })
                })
                .collect();
            (&JOURNAL_HEADERS, rows)
        }
    };
    let headers: Vec<String> = headers.iter().map(|h| h.to_string()).collect();
    let filename = format!(
        "{}-{}-{}-to-{}-{}.csv",
        file_stem(&b.profile.name),
        format.as_str(),
        if filter.from.is_empty() {
            "start"
        } else {
            &filter.from
        },
        if filter.to.is_empty() {
            "latest"
        } else {
            &filter.to
        },
        filter.currency.map_or("all-currencies", Currency::code),
    );
    let mut all = Vec::with_capacity(rows.len() + 1);
    all.push(headers.clone());
    all.extend(rows.iter().cloned());
    Ok(CsvExport {
        headers,
        rows,
        filename,
        csv: encode_csv(&all),
    })
}

/// The columns every bitcoin export adds after its existing ones
/// (`fiatHeaders`): the recorded fiat value, as entered, and the Lightning
/// amount if there was one. Blank for an entry in dollars, euros or pounds.
fn fiat_cells(v: Option<&FiatValue>, millisats: Option<Minor>) -> [String; 6] {
    [
        v.map(|v| amount_input(v.amount_minor, v.currency))
            .unwrap_or_default(),
        v.map(|v| v.currency.code().to_string()).unwrap_or_default(),
        v.map(|v| v.rate.clone()).unwrap_or_default(),
        v.map(|v| v.source.as_str().to_string()).unwrap_or_default(),
        v.map(|v| v.at.clone()).unwrap_or_default(),
        millisats.map(|m| m.get().to_string()).unwrap_or_default(),
    ]
}

/// `sats`: whole sats for a BTC amount, and blank for any other currency.
fn sats(n: Minor, currency: Currency) -> String {
    if currency == Currency::Btc {
        n.get().to_string()
    } else {
        String::new()
    }
}

const EXPENSE_HEADERS: [&str; 23] = [
    "Business ID",
    "Business",
    "Expense ID",
    "Date",
    "Vendor",
    "Description",
    "Amount",
    "Currency",
    "Category",
    "Payment account",
    "Business purpose",
    "Corrects expense ID",
    "Source system",
    "Source reference",
    "Receipt count",
    "Paid personally",
    "Amount in sats",
    "Fiat value",
    "Fiat currency",
    "Fiat rate",
    "Fiat source",
    "Fiat value at",
    "Millisats",
];

const JOURNAL_HEADERS: [&str; 27] = [
    "Business ID",
    "Business",
    "Journal ID",
    "Date",
    "Type",
    "Expense ID",
    "Reverses journal ID",
    "Account ID",
    "Account",
    "Category",
    "Debit",
    "Credit",
    "Currency",
    "Memo",
    "Recorded at",
    "Transfer type",
    "Purpose",
    "Reference",
    "Corrects journal ID",
    "Debit in sats",
    "Credit in sats",
    "Fiat value",
    "Fiat currency",
    "Fiat rate",
    "Fiat source",
    "Fiat value at",
    "Millisats",
];

fn journal_kind(kind: JournalKind) -> &'static str {
    match kind {
        JournalKind::Expense => "expense",
        JournalKind::Transfer => "transfer",
        JournalKind::Reversal => "reversal",
    }
}

/// `name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "")`,
/// falling back to `business`.
fn file_stem(name: &str) -> String {
    let mut stem = String::new();
    let mut in_run = false;
    for c in name.to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            stem.push(c);
            in_run = false;
        } else if !in_run {
            stem.push('-');
            in_run = true;
        }
    }
    let stem = stem.strip_prefix('-').unwrap_or(&stem);
    let stem = stem.strip_suffix('-').unwrap_or(stem);
    if stem.is_empty() {
        "business".to_string()
    } else {
        stem.to_string()
    }
}

const BALANCE_HEADERS: [&str; 10] = [
    "Business ID",
    "Business",
    "As of",
    "Account ID",
    "Account",
    "Account type",
    "Balance",
    "Currency",
    "Balance is",
    "Balance in sats",
];

/// `exportBalances`: every bank, card, wallet, loan and equity account's
/// balance in `currency` on `as_of`, one row each, from [`balances`]. A
/// card's balance is what's owed on it, and a loan's what's owed to the
/// owner. A wallet's is also given in whole sats.
pub fn export_balances(b: &BusinessData, as_of: &str, currency: Currency) -> Result<CsvExport> {
    let headers: Vec<String> = BALANCE_HEADERS.iter().map(|h| h.to_string()).collect();
    let rows: Vec<Vec<String>> = balances(b, as_of, currency)?
        .into_iter()
        .map(|r| {
            vec![
                b.profile.id.clone(),
                b.profile.name.clone(),
                as_of.to_string(),
                r.account_id,
                r.name,
                account_kind_label(r.kind).to_string(),
                amount_input(r.balance, currency),
                currency.code().to_string(),
                balance_meaning(r.kind).to_string(),
                sats(r.balance, currency),
            ]
        })
        .collect();
    let mut all = Vec::with_capacity(rows.len() + 1);
    all.push(headers.clone());
    all.extend(rows.iter().cloned());
    Ok(CsvExport {
        headers,
        rows,
        filename: format!(
            "{}-balances-{}-{}.csv",
            file_stem(&b.profile.name),
            as_of,
            currency.code()
        ),
        csv: encode_csv(&all),
    })
}
