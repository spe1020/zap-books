//! Business-scoped CSV exports (`src/domain/export.ts`).

use std::cmp::Ordering;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::date::valid_date;
use crate::error::{LedgerError, Result};
use crate::js;
use crate::money::{amount_input, Currency};
use crate::types::{BusinessData, ExpenseStatus, JournalKind};

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
/// would run as a formula (`[\s﻿]*[=+\-@]` or a leading tab, CR, LF).
fn csv_cell(value: &str) -> String {
    let formula = value
        .trim_start_matches(js::is_space)
        .starts_with(['=', '+', '-', '@'])
        || value.starts_with(['\t', '\r', '\n']);
    let literal = if formula {
        format!("'{value}")
    } else {
        value.to_string()
    };
    format!("\"{}\"", literal.replace('"', "\"\""))
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
    let account = |id: &str| {
        b.accounts
            .iter()
            .find(|a| a.id == id && a.business_id == b.profile.id)
            .map(|a| a.name.clone())
            .unwrap_or_default()
    };
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
                        amount_input(d.amount_minor),
                        d.currency.code().to_string(),
                        category(Some(&d.category_id)),
                        account(&d.account_id),
                        d.purpose.clone(),
                        d.correction_of.clone().unwrap_or_default(),
                        source_system,
                        source_reference,
                        d.document_ids.len().to_string(),
                    ]
                })
                .collect();
            (&EXPENSE_HEADERS, rows)
        }
        ExportFormat::Journal => {
            let mut journals: Vec<_> = b
                .journals
                .iter()
                .filter(|j| j.business_id == b.profile.id && in_range(&j.date, j.currency))
                .collect();
            journals.sort_by(|a, z| {
                locale_compare(&a.date, &z.date)
                    .then_with(|| locale_compare(&a.created_at, &z.created_at))
            });
            let rows = journals
                .into_iter()
                .flat_map(|j| {
                    j.lines.iter().map(|l| {
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
                            amount_input(l.debit),
                            amount_input(l.credit),
                            j.currency.code().to_string(),
                            j.memo.clone(),
                            j.created_at.clone(),
                        ]
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

const EXPENSE_HEADERS: [&str; 15] = [
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
];

const JOURNAL_HEADERS: [&str; 15] = [
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
