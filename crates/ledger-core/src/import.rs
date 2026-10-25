//! CSV import of payments and expense history (`src/domain/import.ts`).

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::date::{normalize_import_date, valid_date, DateFormat};
use crate::env::Env;
use crate::error::{LedgerError, Result};
use crate::js;
use crate::ledger::{blank_draft, push_event, save_draft_in, truthy};
use crate::money::{parse_money, Currency, Minor, MAX_MINOR};
use crate::types::{
    AccountKind, BusinessData, EventType, ImportProvider, ImportedExpenseSource,
    ImportedTransaction,
};

/// Which CSV header holds each field. Optional fields that are absent or
/// `""` are treated as unmapped.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvMapping {
    pub date: String,
    pub description: String,
    pub amount: String,
    pub currency: String,
    pub reference: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transaction_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_account: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImportKind {
    #[serde(rename = "payment")]
    Payment,
    #[serde(rename = "expense-history")]
    ExpenseHistory,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ImportKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<ImportProvider>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date_format: Option<DateFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
}

/// Whether outflows appear as negative or positive amounts in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SignConvention {
    Negative,
    Positive,
}

/// [`ImportedExpenseSource`] as it appears in a preview row. The currency is
/// the raw, upper-cased CSV text, which may not be a supported currency on a
/// row that failed validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewSource {
    pub id: String,
    pub business_id: String,
    pub provider: ImportProvider,
    pub reference: String,
    pub fingerprint: String,
    pub date: String,
    pub amount_minor: Minor,
    pub currency: String,
    pub vendor: String,
    pub category: String,
    pub filename: String,
}

impl PreviewSource {
    fn into_source(self) -> Result<ImportedExpenseSource> {
        let currency =
            Currency::from_code(&self.currency).ok_or(LedgerError::InvalidImportSource)?;
        Ok(ImportedExpenseSource {
            id: self.id,
            business_id: self.business_id,
            provider: self.provider,
            reference: self.reference,
            fingerprint: self.fingerprint,
            date: self.date,
            amount_minor: self.amount_minor,
            currency,
            vendor: self.vendor,
            category: self.category,
            filename: self.filename,
        })
    }
}

/// One previewed CSV row. `currency` is the raw text (see [`PreviewSource`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRow {
    pub row: u64,
    pub date: String,
    pub description: String,
    pub amount_minor: Minor,
    pub currency: String,
    pub reference: String,
    pub fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub duplicate: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warnings: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import_source: Option<PreviewSource>,
}

/// `text.length` limit, in UTF-16 code units.
const MAX_CSV_LENGTH: usize = 2_000_000;
/// One header row plus 2,000 transactions.
const MAX_CSV_ROWS: usize = 2001;

/// `parseCSV`: RFC 4180-style parsing with quoted fields, doubled quotes,
/// and CR, LF, or CRLF row breaks. Cells are trimmed and blank rows dropped.
pub fn parse_csv(text: &str) -> Result<Vec<Vec<String>>> {
    if js::len(text) > MAX_CSV_LENGTH {
        return Err(LedgerError::CsvTooLarge);
    }
    let source = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut cell = String::new();
    let (mut quoted, mut closed) = (false, false);
    let mut chars = source.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    cell.push('"');
                    chars.next();
                } else {
                    quoted = false;
                    closed = true;
                }
            } else {
                cell.push(c);
            }
        } else if c == '"' {
            if !cell.is_empty() || closed {
                return Err(LedgerError::MalformedQuote);
            }
            quoted = true;
        } else if c == ',' || c == '\n' || c == '\r' {
            row.push(js::trim(&cell).to_string());
            cell.clear();
            closed = false;
            if c != ',' {
                end_row(&mut rows, std::mem::take(&mut row));
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
            }
        } else {
            if closed && !js::is_space(c) {
                return Err(LedgerError::TextAfterQuote);
            }
            if !closed {
                cell.push(c);
            }
        }
    }
    if quoted {
        return Err(LedgerError::UnclosedQuote);
    }
    row.push(js::trim(&cell).to_string());
    end_row(&mut rows, row);
    let header = match rows.as_slice() {
        [header, _, ..] => header,
        _ => return Err(LedgerError::CsvTooShort),
    };
    let unique: HashSet<&String> = header.iter().collect();
    if header.iter().any(String::is_empty) || unique.len() != header.len() {
        return Err(LedgerError::InvalidHeaders);
    }
    if rows.len() > MAX_CSV_ROWS {
        return Err(LedgerError::TooManyRows);
    }
    Ok(rows)
}

fn end_row(rows: &mut Vec<Vec<String>>, row: Vec<String>) {
    if row.iter().any(|cell| !cell.is_empty()) {
        rows.push(row);
    }
}

/// `guessMapping`: matches headers by lower-cased alphanumerics against the
/// names common exports use. Unmatched fields map to `""`.
pub fn guess_mapping(headers: &[String]) -> CsvMapping {
    let lookup = |names: &[&str]| {
        headers
            .iter()
            .find(|h| {
                let key: String = h
                    .to_lowercase()
                    .chars()
                    .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
                    .collect();
                names.contains(&key.as_str())
            })
            .cloned()
            .unwrap_or_default()
    };
    CsvMapping {
        date: lookup(&["date", "transactiondate", "expensedate"]),
        description: lookup(&[
            "description",
            "memodescription",
            "memo",
            "expensedescription",
            "merchant",
            "vendor",
        ]),
        amount: lookup(&["amount", "total", "totalamount", "expenseamount"]),
        currency: lookup(&["currency", "currencycode"]),
        reference: lookup(&[
            "expenseid",
            "transactionid",
            "reference",
            "referencenumber",
            "referenceno",
            "id",
            "num",
        ]),
        vendor: Some(lookup(&[
            "vendor",
            "vendorname",
            "merchant",
            "payee",
            "name",
        ])),
        category: Some(lookup(&["category", "expensecategory", "expenseaccount"])),
        purpose: Some(lookup(&["businesspurpose", "purpose"])),
        transaction_type: Some(lookup(&["transactiontype", "type"])),
        source_account: Some(lookup(&[
            "paymentaccount",
            "paymentaccountname",
            "paidthrough",
            "bankaccount",
        ])),
    }
}

/// `importAmount`: accepts explicit comma thousands groups
/// (`-?\d{1,3}(,\d{3})+(\.\d{1,2})?`) and never infers a decimal convention.
fn import_amount(value: &str) -> Result<Minor> {
    if has_thousands_groups(value) {
        parse_money(&value.replace(',', ""))
    } else {
        parse_money(value)
    }
}

fn has_thousands_groups(value: &str) -> bool {
    let unsigned = value.strip_prefix('-').unwrap_or(value);
    let (int, frac) = match unsigned.split_once('.') {
        Some((int, frac)) => (int, Some(frac)),
        None => (unsigned, None),
    };
    let frac_ok = frac.is_none_or(|f| (1..=2).contains(&f.len()) && js::all_digits(f));
    let mut groups = int.split(',');
    let first_ok = groups
        .next()
        .is_some_and(|g| (1..=3).contains(&g.len()) && js::all_digits(g));
    let mut rest = 0;
    let rest_ok = groups.all(|g| {
        rest += 1;
        g.len() == 3 && js::all_digits(g)
    });
    frac_ok && first_ok && rest_ok && rest > 0
}

/// Transaction types that represent a paid expense.
const PAID_EXPENSE_TYPES: [&str; 6] = [
    "expense",
    "check",
    "cheque",
    "credit card expense",
    "credit card charge",
    "cash expense",
];

/// What an existing record says about a fingerprint.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Seen {
    date: String,
    amount_minor: Minor,
    currency: String,
}

/// `previewImport`: validates and fingerprints every data row without
/// changing the business. Each row reports its own error, if any.
pub fn preview_import(
    b: &BusinessData,
    rows: &[Vec<String>],
    map: &CsvMapping,
    account_id: &str,
    convention: SignConvention,
    options: &ImportOptions,
    env: &mut impl Env,
) -> Result<Vec<ImportRow>> {
    // TS: `rows[0].indexOf(...)` throws a TypeError on an empty array.
    let (header, data) = rows
        .split_first()
        .ok_or(LedgerError::Internal("import has no header row"))?;
    let account = b
        .accounts
        .iter()
        .find(|x| x.id == account_id && x.kind != AccountKind::Expense);
    let history = options.kind == Some(ImportKind::ExpenseHistory);
    let provider = options.provider.unwrap_or_default();
    let mut seen: HashMap<String, Seen> = HashMap::new();
    if history {
        for s in b.drafts.iter().filter_map(|d| d.import_source.as_ref()) {
            seen.insert(
                s.fingerprint.clone(),
                Seen {
                    date: s.date.clone(),
                    amount_minor: s.amount_minor,
                    currency: s.currency.code().to_string(),
                },
            );
        }
    } else {
        for p in &b.payments {
            seen.insert(
                p.fingerprint.clone(),
                Seen {
                    date: p.date.clone(),
                    amount_minor: p.amount_minor,
                    currency: p.currency.code().to_string(),
                },
            );
        }
    }
    let column = |name: Option<&str>| header.iter().position(|h| h == name.unwrap_or(""));
    let account_column = column(map.source_account.as_deref());
    let source_accounts: HashSet<String> = data
        .iter()
        .map(|row| {
            let cell = account_column.and_then(|i| row.get(i));
            js::trim(cell.map_or("", String::as_str)).to_lowercase()
        })
        .filter(|s| !s.is_empty())
        .collect();
    // Every counted amount is positive, so once this leaves the safe range it
    // stays out, exactly as the TypeScript float accumulator does.
    let mut combined_amount: i128 = 0;
    let mut out = Vec::with_capacity(data.len());
    for (i, r) in data.iter().enumerate() {
        let value = |key: Option<&str>| {
            column(key)
                .and_then(|ix| r.get(ix))
                .cloned()
                .unwrap_or_default()
        };
        let date = normalize_import_date(
            &value(Some(&map.date)),
            options.date_format.unwrap_or_default(),
        );
        let description = value(Some(&map.description));
        let reference = value(Some(&map.reference));
        let vendor = if truthy(&map.vendor).is_some() {
            value(map.vendor.as_deref())
        } else {
            description.clone()
        };
        let category_name = value(map.category.as_deref());
        let purpose = value(map.purpose.as_deref());
        let category = b.categories.iter().find(|c| {
            c.business_id == b.profile.id && c.name.to_lowercase() == category_name.to_lowercase()
        });
        let currency = if map.currency.is_empty() {
            account.map_or("USD", |a| a.currency.code()).to_string()
        } else {
            value(Some(&map.currency)).to_uppercase()
        };
        let mut warnings = Vec::new();
        if history && vendor.is_empty() {
            warnings.push("Add a vendor during review.".to_string());
        }
        if history && category.is_none() {
            warnings.push(if category_name.is_empty() {
                "Choose a category during review.".to_string()
            } else {
                format!("Unmapped category: {category_name}. Choose it during review.")
            });
        }
        if history && purpose.is_empty() {
            warnings.push("Add the business purpose during review.".to_string());
        }
        let mut amount_minor = Minor::ZERO;
        let checked: Result<()> = (|| {
            if source_accounts.len() > 1 {
                return Err(LedgerError::MultipleSourceAccounts);
            }
            let account = account
                .filter(|a| a.business_id == b.profile.id)
                .ok_or(LedgerError::SelectPaymentAccount)?;
            if r.len() != header.len() {
                return Err(LedgerError::ColumnCount);
            }
            if map.date.is_empty() || map.description.is_empty() || map.amount.is_empty() {
                return Err(LedgerError::MappingIncomplete);
            }
            if !valid_date(&date) {
                return Err(LedgerError::InvalidImportDate);
            }
            if description.is_empty() {
                return Err(LedgerError::DescriptionRequired);
            }
            if Currency::from_code(&currency) != Some(account.currency) {
                return Err(LedgerError::ImportCurrency);
            }
            if truthy(&map.transaction_type).is_some() {
                let kind = value(map.transaction_type.as_deref()).to_lowercase();
                if !PAID_EXPENSE_TYPES.contains(&kind.as_str()) {
                    return Err(LedgerError::UnsupportedTransactionType);
                }
            }
            let parsed = import_amount(&value(Some(&map.amount)))?;
            amount_minor = match convention {
                SignConvention::Negative => -parsed,
                SignConvention::Positive => parsed,
            };
            if amount_minor <= Minor::ZERO {
                return Err(LedgerError::NotAnOutflow);
            }
            Ok(())
        })();
        let mut error = checked.err().map(|e| e.to_string());
        let scope = || {
            let mut parts = vec![Value::from(b.profile.id.as_str()), Value::from(account_id)];
            if history {
                parts.push(Value::from("expense-history"));
                parts.push(Value::from(provider.as_str()));
            }
            parts
        };
        let mut parts = scope();
        if reference.is_empty() {
            parts.push(Value::from(date.as_str()));
            parts.push(Value::from(js::normalize(&description)));
            if history {
                parts.push(Value::from(js::trim(&vendor).to_lowercase()));
            }
            parts.push(Value::from(amount_minor.get()));
            parts.push(Value::from(currency.as_str()));
        } else {
            parts.push(Value::from("ref"));
            parts.push(Value::from(reference.as_str()));
        }
        // `JSON.stringify` of an array of strings and safe integers.
        let fingerprint = Value::Array(parts).to_string();
        let previous = seen.get(&fingerprint);
        let duplicate = previous.is_some();
        if previous.is_some_and(|p| {
            p.date != date || p.amount_minor != amount_minor || p.currency != currency
        }) {
            error = Some(LedgerError::ConflictingReference.to_string());
        }
        if error.is_none() && !duplicate {
            combined_amount += i128::from(amount_minor.get());
            if combined_amount.abs() > i128::from(MAX_MINOR) {
                error = Some(LedgerError::CombinedAmountOutOfRange.to_string());
            }
        }
        if error.is_none() {
            seen.insert(
                fingerprint.clone(),
                Seen {
                    date: date.clone(),
                    amount_minor,
                    currency: currency.clone(),
                },
            );
        }
        let import_source = history.then(|| PreviewSource {
            id: env.new_id(),
            business_id: b.profile.id.clone(),
            provider,
            reference: reference.clone(),
            fingerprint: fingerprint.clone(),
            date: date.clone(),
            amount_minor,
            currency: currency.clone(),
            vendor: vendor.clone(),
            category: category_name.clone(),
            filename: options.filename.clone().unwrap_or_default(),
        });
        out.push(ImportRow {
            row: i as u64 + 2,
            date,
            description,
            amount_minor,
            currency,
            reference,
            fingerprint,
            error,
            duplicate,
            vendor: Some(vendor),
            category_id: category.map(|c| c.id.clone()),
            purpose: Some(purpose),
            warnings: Some(warnings),
            import_source,
        });
    }
    Ok(out)
}

/// `commitImport`: turns previewed rows into drafts (and, for payment files,
/// imported payments). Rows already imported are skipped; any conflict or
/// invalid row aborts the whole import.
pub fn commit_import(
    business: &BusinessData,
    rows: &[ImportRow],
    account_id: &str,
    env: &mut impl Env,
) -> Result<BusinessData> {
    let mut b = business.clone();
    if rows.iter().any(|r| truthy(&r.error).is_some()) {
        return Err(LedgerError::UnresolvedRows);
    }
    let account = b
        .accounts
        .iter()
        .find(|a| {
            a.id == account_id && a.kind != AccountKind::Expense && a.business_id == b.profile.id
        })
        .cloned()
        .ok_or(LedgerError::PaymentAccountNotFound)?;
    for row in rows {
        let previous = if row.import_source.is_some() {
            b.drafts
                .iter()
                .filter_map(|d| d.import_source.as_ref())
                .find(|p| p.fingerprint == row.fingerprint)
                .map(|p| (p.date.clone(), p.amount_minor, p.currency))
        } else {
            b.payments
                .iter()
                .find(|p| p.fingerprint == row.fingerprint)
                .map(|p| (p.date.clone(), p.amount_minor, p.currency))
        };
        if let Some((date, amount_minor, currency)) = previous {
            if date != row.date
                || amount_minor != row.amount_minor
                || currency.code() != row.currency
            {
                return Err(LedgerError::ConflictingReference);
            }
            continue;
        }
        // `Number.isSafeInteger(row.amountMinor)` is guaranteed by `Minor`.
        if !valid_date(&row.date)
            || row.description.is_empty()
            || row.amount_minor <= Minor::ZERO
            || row.currency != account.currency.code()
        {
            return Err(LedgerError::InvalidImportRow);
        }
        let import_source = match &row.import_source {
            Some(s) => {
                if s.business_id != b.profile.id
                    || s.fingerprint != row.fingerprint
                    || s.amount_minor != row.amount_minor
                    || s.date != row.date
                    || s.currency != row.currency
                {
                    return Err(LedgerError::InvalidImportSource);
                }
                Some(s.clone().into_source()?)
            }
            None => None,
        };
        let mut draft = blank_draft(&b, env);
        draft.vendor = row
            .vendor
            .clone()
            .unwrap_or_else(|| row.description.clone());
        draft.description = row.description.clone();
        draft.date = row.date.clone();
        draft.amount_minor = row.amount_minor;
        draft.currency = account.currency;
        draft.account_id = account_id.to_string();
        draft.category_id = row.category_id.clone().unwrap_or_default();
        draft.purpose = row.purpose.clone().unwrap_or_default();
        let (source_id, detail) = match import_source {
            Some(source) => {
                let detail = format!(
                    "Expense history imported from {}; source record retained for review. This is not bank payment evidence.",
                    source.provider.as_str()
                );
                let source_id = source.id.clone();
                draft.import_source = Some(source);
                (source_id, detail)
            }
            None => {
                let payment_id = env.new_id();
                draft.payment_id = Some(payment_id.clone());
                b.payments.push(ImportedTransaction {
                    id: payment_id.clone(),
                    business_id: b.profile.id.clone(),
                    date: row.date.clone(),
                    description: row.description.clone(),
                    amount_minor: row.amount_minor,
                    currency: account.currency,
                    account_id: account_id.to_string(),
                    reference: row.reference.clone(),
                    fingerprint: row.fingerprint.clone(),
                    draft_id: draft.id.clone(),
                });
                (
                    payment_id,
                    "Imported payment linked to one draft. Attach receipts to this draft."
                        .to_string(),
                )
            }
        };
        let draft_id = draft.id.clone();
        save_draft_in(&mut b, draft, env)?;
        push_event(
            &mut b,
            env,
            EventType::ExpenseMatched,
            &draft_id,
            &detail,
            vec![source_id],
        );
    }
    Ok(b)
}
