//! Validates and normalizes the model's answer. Nothing the model returns is
//! trusted because the schema option was set: every value is re-checked here
//! with `ledger-core`'s exact money and date rules.

use ledger_core::{
    amount_input, parse_money, valid_date, Currency, ExtractedField, ExtractedLine, ExtractedLines,
    ExtractionFields, FieldStatus, Minor,
};
use serde::Deserialize;
use serde_json::Value;

use crate::error::ExtractError;
use crate::extract::Category;

/// The longest evidence string kept, in characters.
pub const MAX_EVIDENCE_CHARS: usize = 120;

/// The most lines kept in one list; a longer list is unclear.
pub const MAX_LINES: usize = 30;

/// One field exactly as the model sent it. Every part is optional so a
/// schema violation degrades the field instead of failing the receipt.
#[derive(Debug, Default, Deserialize)]
pub struct RawField {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub value: Option<Value>,
    #[serde(default)]
    pub evidence: Option<Value>,
}

/// The model's whole answer.
#[derive(Debug, Default, Deserialize)]
pub struct RawFields {
    #[serde(default)]
    pub vendor: RawField,
    #[serde(default)]
    pub date: RawField,
    #[serde(default)]
    pub total: RawField,
    #[serde(default)]
    pub currency: RawField,
    #[serde(default)]
    pub reference: RawField,
    #[serde(default)]
    pub subtotal: RawField,
    #[serde(default)]
    pub tax: RawLines,
    /// Optional: an answer without fees means no fee lines.
    #[serde(default = "absent_lines")]
    pub fees: RawLines,
    /// Optional: an answer without discounts means no discount lines.
    #[serde(default = "absent_lines")]
    pub discounts: RawLines,
    /// Optional: an answer without a tip means no tip line.
    #[serde(default = "absent_raw")]
    pub tip: RawField,
    #[serde(default)]
    pub category: RawField,
}

/// A list of printed lines exactly as the model sent it. `lines` stays raw
/// JSON so a malformed line degrades the list instead of failing the receipt.
#[derive(Debug, Default, Deserialize)]
pub struct RawLines {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub lines: Option<Value>,
}

fn absent_raw() -> RawField {
    RawField {
        status: Some("absent".into()),
        ..RawField::default()
    }
}

fn absent_lines() -> RawLines {
    RawLines {
        status: Some("absent".into()),
        lines: None,
    }
}

/// Parses the model's message content. Anything that is not a JSON object
/// is a malformed answer.
pub fn parse_answer(content: &str) -> Result<RawFields, ExtractError> {
    let value: Value = serde_json::from_str(content).map_err(|_| ExtractError::MalformedAnswer)?;
    if !value.is_object() {
        return Err(ExtractError::MalformedAnswer);
    }
    serde_json::from_value(value).map_err(|_| ExtractError::MalformedAnswer)
}

/// Strips control characters, trims, and keeps at most `max` characters.
pub fn clean_text(text: &str, max: usize) -> String {
    let kept: String = text.chars().filter(|c| !c.is_control()).collect();
    kept.trim()
        .chars()
        .take(max)
        .collect::<String>()
        .trim()
        .to_string()
}

/// Why an amount could not be accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AmountProblem {
    CommaDecimal,
    NotAnAmount,
    Negative,
}

/// Parses amount text exactly: an optional leading `$`, `€` or `£`, optional
/// comma thousands groups, at most two decimals, never through floating point.
/// A comma decimal (`42,50`) is never guessed. Zero is a valid amount here;
/// only the total must be positive.
pub fn parse_amount(text: &str) -> Result<Minor, AmountProblem> {
    let mut s = text.trim();
    let negative = s.starts_with('-');
    if negative {
        s = s.get(1..).unwrap_or_default().trim_start();
    }
    for symbol in ['$', '€', '£'] {
        if let Some(rest) = s.strip_prefix(symbol) {
            s = rest.trim_start();
            break;
        }
    }
    if s.contains(',') {
        if is_thousands_grouped(s) {
            let plain = s.replace(',', "");
            return finish(&plain, negative);
        }
        let (_, after) = s.rsplit_once(',').unwrap_or((s, ""));
        if !s.contains('.')
            && (1..=2).contains(&after.len())
            && after.chars().all(|c| c.is_ascii_digit())
        {
            return Err(AmountProblem::CommaDecimal);
        }
        return Err(AmountProblem::NotAnAmount);
    }
    finish(s, negative)
}

fn finish(s: &str, negative: bool) -> Result<Minor, AmountProblem> {
    if s.starts_with('-') || s.starts_with('+') {
        return Err(AmountProblem::NotAnAmount);
    }
    let value = parse_money(s).map_err(|_| AmountProblem::NotAnAmount)?;
    if negative && value.get() != 0 {
        return Err(AmountProblem::Negative);
    }
    Ok(if negative { Minor::ZERO } else { value })
}

/// Parses a discount, which receipts print as `0.25`, `-0.25`, `0.25-`,
/// `-$0.25`, `$-0.25` or `(0.25)`. The amount is always positive.
pub fn parse_discount(text: &str) -> Result<Minor, AmountProblem> {
    let mut s = text.trim();
    if let Some(inner) = s.strip_prefix('(').and_then(|r| r.strip_suffix(')')) {
        s = inner.trim();
    }
    s = s.strip_suffix('-').map_or(s, str::trim_end);
    s = s.strip_prefix('-').map_or(s, str::trim_start);
    for symbol in ['$', '€', '£'] {
        if let Some(rest) = s.strip_prefix(symbol) {
            if let Some(unsigned) = rest.trim_start().strip_prefix('-') {
                return parse_amount(&format!("{symbol}{unsigned}"));
            }
        }
    }
    parse_amount(s)
}

/// `1,234` or `12,345,678.90`: comma groups of exactly three digits.
fn is_thousands_grouped(s: &str) -> bool {
    let (whole, cents) = match s.split_once('.') {
        Some((w, c)) => (w, Some(c)),
        None => (s, None),
    };
    if cents.is_some_and(|c| c.is_empty() || c.len() > 2 || !c.chars().all(|d| d.is_ascii_digit()))
    {
        return false;
    }
    let mut groups = whole.split(',');
    let first_ok = groups
        .next()
        .is_some_and(|g| (1..=3).contains(&g.len()) && g.chars().all(|d| d.is_ascii_digit()));
    let mut rest = groups.peekable();
    first_ok
        && rest.peek().is_some()
        && rest.all(|g| g.len() == 3 && g.chars().all(|d| d.is_ascii_digit()))
}

/// Days since 1970-01-01 for a proleptic Gregorian date.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn day_number(date: &str) -> Option<i64> {
    let y = date.get(0..4)?.parse().ok()?;
    let m = date.get(5..7)?.parse().ok()?;
    let d = date.get(8..10)?.parse().ok()?;
    Some(days_from_civil(y, m, d))
}

/// The UTC calendar date `YYYY-MM-DD` for a Unix time in seconds.
pub fn utc_date(unix_seconds: i64) -> String {
    let z = unix_seconds.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Everything normalization needs besides the model's answer.
pub struct Context<'a> {
    pub categories: &'a [Category],
    pub currencies: &'a [Currency],
    /// Today's date, `YYYY-MM-DD`. A receipt dated more than one day later
    /// is unclear; one day of slack covers time zones.
    pub today: &'a str,
}

/// The result of normalization: fields and plain-language warnings.
pub struct Normalized {
    pub fields: ExtractionFields,
    pub warnings: Vec<String>,
}

fn status_of(raw: &RawField) -> FieldStatus {
    match raw.status.as_deref().map(str::trim) {
        Some("found") => FieldStatus::Found,
        Some("absent") => FieldStatus::Absent,
        _ => FieldStatus::Unclear,
    }
}

fn evidence_of(raw: &RawField) -> String {
    match &raw.evidence {
        Some(Value::String(s)) => clean_text(s, MAX_EVIDENCE_CHARS),
        _ => String::new(),
    }
}

fn unclear<T>(evidence: String) -> ExtractedField<T> {
    ExtractedField {
        status: FieldStatus::Unclear,
        value: None,
        evidence,
    }
}

fn absent<T>() -> ExtractedField<T> {
    ExtractedField {
        status: FieldStatus::Absent,
        value: None,
        evidence: String::new(),
    }
}

/// Applies the rules shared by every field, then `parse` for the value.
/// `found` requires a string value and evidence; anything else is unclear.
fn field<T>(
    raw: &RawField,
    label: &str,
    warnings: &mut Vec<String>,
    parse: impl FnOnce(&str, &mut Vec<String>) -> Option<T>,
) -> ExtractedField<T> {
    let evidence = evidence_of(raw);
    match status_of(raw) {
        FieldStatus::Absent => absent(),
        FieldStatus::Unclear => unclear(evidence),
        FieldStatus::Found => {
            let Some(Value::String(text)) = &raw.value else {
                warnings.push(format!(
                    "The {label} wasn't given as text the app can check. Check it on the receipt."
                ));
                return unclear(evidence);
            };
            if evidence.is_empty() {
                warnings.push(format!(
                    "The model didn't show where it saw the {label}. Check it on the receipt."
                ));
                return unclear(evidence);
            }
            match parse(text, warnings) {
                Some(value) => ExtractedField {
                    status: FieldStatus::Found,
                    value: Some(value),
                    evidence,
                },
                None => unclear(evidence),
            }
        }
    }
}

fn text_value(text: &str) -> Option<String> {
    let v = clean_text(text, MAX_EVIDENCE_CHARS);
    (!v.is_empty()).then_some(v)
}

/// An amount as the receipt shows it: `$54.50`, `−£2.00`, or `12.00` when the
/// currency wasn't read.
fn shown(amount: Minor, currency: Option<Currency>) -> String {
    let symbol = match currency {
        Some(Currency::Usd) => "$",
        Some(Currency::Eur) => "€",
        Some(Currency::Gbp) => "£",
        None => "",
    };
    let sign = if amount.get() < 0 { "−" } else { "" };
    let digits = amount_input(amount);
    format!("{sign}{symbol}{}", digits.trim_start_matches('-'))
}

/// An amount field. Only the total must be more than zero; a zero subtotal
/// or tip is valid. A negative total may be a refund, which must never
/// become an expense, so it stays unclear.
fn amount_field(
    raw: &RawField,
    label: &str,
    allow_zero: bool,
    currency: Option<Currency>,
    warnings: &mut Vec<String>,
) -> ExtractedField<Minor> {
    field(raw, label, warnings, |text, warnings| {
        match parse_amount(text) {
            Ok(value) if value == Minor::ZERO && !allow_zero => {
                warnings.push(format!(
                    "The {label} isn't a positive amount. Check it on the receipt."
                ));
                None
            }
            Ok(value) => Some(value),
            Err(problem) => {
                warnings.push(match problem {
                    AmountProblem::CommaDecimal => format!(
                    "The {label} uses a comma as the decimal separator. Enter the amount yourself."
                ),
                    AmountProblem::Negative if !allow_zero => match parse_discount(text) {
                        Ok(paid) => format!(
                            "The only amount is a payment or refund line ({}). Enter the {label} yourself.",
                            shown(-paid, currency)
                        ),
                        Err(_) => format!("The {label} is negative. Enter it yourself."),
                    },
                    AmountProblem::Negative => {
                        format!("The {label} is negative. Check it on the receipt.")
                    }
                    AmountProblem::NotAnAmount => format!(
                        "The {label} couldn't be read as an exact amount. Check it on the receipt."
                    ),
                });
                None
            }
        }
    })
}

/// Which way a list's amounts are printed.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Sign {
    /// Tax and fees: a negative line is a misreading.
    Positive,
    /// Discounts: any printed sign, stored as a positive amount.
    Either,
}

/// A list of printed lines. One line that can't be checked makes the whole
/// list unclear; an unclear list keeps the evidence of the lines it saw.
fn line_list(
    raw: &RawLines,
    label: &str,
    sign: Sign,
    warnings: &mut Vec<String>,
) -> ExtractedLines {
    let items: &[Value] = match &raw.lines {
        Some(Value::Array(items)) => items,
        _ => &[],
    };
    let evidence = |item: &Value| match item.get("evidence") {
        Some(Value::String(s)) => clean_text(s, MAX_EVIDENCE_CHARS),
        _ => String::new(),
    };
    let unclear_list = || ExtractedLines {
        status: FieldStatus::Unclear,
        value: None,
        lines: items
            .iter()
            .take(MAX_LINES)
            .map(evidence)
            .filter(|e| !e.is_empty())
            .map(|evidence| ExtractedLine {
                amount: None,
                evidence,
            })
            .collect(),
    };
    match raw.status.as_deref().map(str::trim) {
        Some("absent") => return ExtractedLines::default(),
        Some("found") => {}
        _ => return unclear_list(),
    }
    if items.is_empty() || items.len() > MAX_LINES || !matches!(raw.lines, Some(Value::Array(_))) {
        warnings.push(format!(
            "The {label} lines couldn't be read as a list. Check them on the receipt."
        ));
        return unclear_list();
    }
    let mut lines = Vec::with_capacity(items.len());
    let mut sum = Minor::ZERO;
    for item in items {
        let evidence = evidence(item);
        let Some(Value::String(text)) = item.get("amount") else {
            warnings.push(format!(
                "The {label} lines weren't given as text the app can check. Check them on the receipt."
            ));
            return unclear_list();
        };
        if evidence.is_empty() {
            warnings.push(format!(
                "The model didn't show where it saw the {label} lines. Check them on the receipt."
            ));
            return unclear_list();
        }
        let parsed = match sign {
            Sign::Positive => parse_amount(text),
            Sign::Either => parse_discount(text),
        };
        let amount = match parsed {
            Ok(amount) => amount,
            Err(problem) => {
                warnings.push(match problem {
                    AmountProblem::CommaDecimal => format!(
                        "The {label} lines use a comma as the decimal separator. Enter the amounts yourself."
                    ),
                    AmountProblem::Negative => format!(
                        "The {label} lines include a negative amount. Check them on the receipt."
                    ),
                    AmountProblem::NotAnAmount => format!(
                        "The {label} lines couldn't all be read as exact amounts. Check them on the receipt."
                    ),
                });
                return unclear_list();
            }
        };
        let Ok(next) = sum.checked_add(amount) else {
            warnings.push(format!(
                "The {label} lines add up to more than the app can store. Check them on the receipt."
            ));
            return unclear_list();
        };
        sum = next;
        lines.push(ExtractedLine {
            amount: Some(amount),
            evidence,
        });
    }
    ExtractedLines {
        status: FieldStatus::Found,
        value: Some(sum),
        lines,
    }
}

/// `a`, `a and b`, `a, b and c`.
fn join(parts: &[String]) -> String {
    match parts {
        [] => String::new(),
        [only] => only.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

/// The amounts the total is checked against. Lists and fields that weren't
/// read count as 0.
struct Parts {
    subtotal: Minor,
    tax: Option<Minor>,
    fees: Option<Minor>,
    discounts: Option<Minor>,
    tip: Option<Minor>,
}

impl Parts {
    /// `subtotal + tax + fees + tip`, and the same less discounts for
    /// receipts whose subtotal is before savings. `None` on overflow.
    fn sums(&self) -> (Option<Minor>, Option<Minor>) {
        let zero = Minor::ZERO;
        let extras = [self.tax, self.fees, self.tip]
            .iter()
            .try_fold(zero, |acc, x| acc.checked_add(x.unwrap_or(zero)).ok());
        let after = extras.and_then(|e| self.subtotal.checked_add(e).ok());
        let before = self
            .subtotal
            .checked_sub(self.discounts.unwrap_or(zero))
            .ok()
            .and_then(|s| extras.and_then(|e| s.checked_add(e).ok()));
        (after, before)
    }

    /// Why `total` is inconsistent, naming every amount that was read.
    fn warning(&self, total: Minor, currency: Option<Currency>) -> String {
        let money = |m: Minor| shown(m, currency);
        let mut read = vec![format!("Subtotal {}", money(self.subtotal))];
        for (name, amount) in [
            ("discounts", self.discounts),
            ("tax", self.tax),
            ("fees", self.fees),
            ("tip", self.tip),
        ] {
            if let Some(a) = amount {
                read.push(format!("{name} {}", money(a)));
            }
        }
        let (after, before) = self.sums();
        let sum = match (after, before) {
            (Some(a), Some(b)) if a != b => format!(
                "{} (or {} if the subtotal is already after discounts)",
                money(b),
                money(a)
            ),
            (Some(a), _) => money(a),
            (None, _) => "more than the app can store".into(),
        };
        let above = [after, before]
            .iter()
            .flatten()
            .all(|s| total.get() > s.get());
        // A gap above subtotal + tax alone is most often an unread tip.
        let only_tax = self.tip.is_none() && self.fees.is_none() && self.discounts.is_none();
        let advice = if only_tax && above {
            "If you added a tip, check the amount matches what you paid."
        } else {
            "Check the amount."
        };
        if read.len() == 1 {
            return format!(
                "The subtotal reads {}, but the total reads {}. {advice}",
                money(self.subtotal),
                money(total)
            );
        }
        format!(
            "{} add up to {sum}, but the total reads {}. {advice}",
            join(&read),
            money(total)
        )
    }
}

/// Normalizes the model's answer into the fields the app shows.
pub fn normalize(raw: &RawFields, ctx: &Context<'_>) -> Normalized {
    let mut warnings = Vec::new();
    let w = &mut warnings;

    let vendor = field(&raw.vendor, "vendor", w, |t, _| text_value(t));
    let reference = field(&raw.reference, "receipt number", w, |t, _| text_value(t));

    let date = field(&raw.date, "date", w, |text, warnings| {
        let text = text.trim();
        if !valid_date(text) {
            warnings.push(
                "The date on the receipt couldn't be read as a calendar date. Check it.".into(),
            );
            return None;
        }
        let too_late = match (day_number(text), day_number(ctx.today)) {
            (Some(date), Some(today)) => date > today + 1,
            _ => true,
        };
        if too_late {
            warnings.push("The receipt date is in the future. Check it.".into());
            return None;
        }
        Some(text.to_string())
    });

    let currency = field(&raw.currency, "currency", w, |text, warnings| {
        let code = clean_text(text, 12).to_uppercase();
        match Currency::from_code(&code).filter(|c| ctx.currencies.contains(c)) {
            Some(c) => Some(c),
            None => {
                let supported: Vec<&str> = ctx.currencies.iter().map(|c| c.code()).collect();
                warnings.push(format!(
                    "The receipt's currency ({code}) isn't supported. Supported: {}.",
                    supported.join(", ")
                ));
                None
            }
        }
    });

    let money = currency.value;
    let mut total = amount_field(&raw.total, "total", false, money, w);
    let subtotal = amount_field(&raw.subtotal, "subtotal", true, money, w);
    let tax = line_list(&raw.tax, "tax", Sign::Positive, w);
    let fees = line_list(&raw.fees, "fees", Sign::Positive, w);
    let discounts = line_list(&raw.discounts, "discounts", Sign::Either, w);
    let tip = amount_field(&raw.tip, "tip", true, money, w);
    // The total must equal subtotal + tax + fees + tip, either with or
    // without the discounts taken off the subtotal. Anything not read counts
    // as 0. Without a subtotal there is nothing to check against.
    if let (Some(t), Some(s)) = (total.value, subtotal.value) {
        let parts = Parts {
            subtotal: s,
            tax: tax.value,
            fees: fees.value,
            discounts: discounts.value,
            tip: tip.value,
        };
        let (after, before) = parts.sums();
        if after != Some(t) && before != Some(t) {
            w.push(parts.warning(t, money));
            total = unclear(total.evidence);
        }
    }

    // The category must be one of the ids sent in; anything else is absent.
    let category = match field(&raw.category, "category", w, |t, _| {
        Some(t.trim().to_string())
    }) {
        ExtractedField {
            status: FieldStatus::Found,
            value: Some(id),
            evidence,
        } if ctx.categories.iter().any(|c| c.id == id) => ExtractedField {
            status: FieldStatus::Found,
            value: Some(id),
            evidence,
        },
        _ => absent(),
    };

    Normalized {
        fields: ExtractionFields {
            vendor,
            date,
            total,
            currency,
            reference,
            subtotal,
            tax,
            fees,
            discounts,
            tip,
            category,
        },
        warnings,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use serde_json::json;

    fn minor(n: i64) -> Minor {
        Minor::new(n).unwrap()
    }

    fn categories() -> Vec<Category> {
        vec![
            Category {
                id: "cat-office".into(),
                name: "Office & supplies".into(),
            },
            Category {
                id: "cat-software".into(),
                name: "Software".into(),
            },
        ]
    }

    fn run(answer: Value) -> Normalized {
        let raw: RawFields = serde_json::from_value(answer).unwrap();
        let cats = categories();
        normalize(
            &raw,
            &Context {
                categories: &cats,
                currencies: &Currency::ALL,
                today: "2026-10-25",
            },
        )
    }

    fn found(value: &str, evidence: &str) -> Value {
        json!({ "status": "found", "value": value, "evidence": evidence })
    }

    fn absent() -> Value {
        json!({ "status": "absent", "value": null, "evidence": "" })
    }

    /// A found list of `(amount, evidence)` lines.
    fn lines(lines: &[(&str, &str)]) -> Value {
        let lines: Vec<Value> = lines
            .iter()
            .map(|(amount, evidence)| json!({ "amount": amount, "evidence": evidence }))
            .collect();
        json!({ "status": "found", "lines": lines })
    }

    fn no_lines() -> Value {
        json!({ "status": "absent", "lines": [] })
    }

    fn good() -> Value {
        json!({
            "vendor": found("Paper & Pine", "Paper & Pine"),
            "date": found("2026-10-24", "2026-10-24"),
            "currency": found("USD", "USD · US dollars"),
            "total": found("42.50", "TOTAL PAID $42.50"),
            "subtotal": found("40.00", "Subtotal $40.00"),
            "tax": lines(&[("2.50", "Sales tax $2.50")]),
            "reference": found("DEMO-RECEIPT-001", "DEMO-RECEIPT-001"),
            "category": found("cat-office", "Office supplies"),
        })
    }

    fn with(field: &str, value: Value) -> Normalized {
        let mut answer = good();
        answer[field] = value;
        run(answer)
    }

    #[test]
    fn a_clean_answer_is_kept() {
        let n = run(good());
        assert!(n.warnings.is_empty(), "{:?}", n.warnings);
        assert_eq!(n.fields.total.value, Some(minor(4250)));
        assert_eq!(n.fields.total.status, FieldStatus::Found);
        assert_eq!(n.fields.currency.value, Some(Currency::Usd));
        assert_eq!(n.fields.category.value.as_deref(), Some("cat-office"));
        assert_eq!(n.fields.tax.value, Some(minor(250)));
        assert_eq!(
            n.fields.tax.lines,
            vec![ExtractedLine {
                amount: Some(minor(250)),
                evidence: "Sales tax $2.50".into()
            }]
        );
        // An answer without fees or discounts means there are none.
        assert_eq!(n.fields.fees, ExtractedLines::default());
        assert_eq!(n.fields.discounts, ExtractedLines::default());
    }

    #[test]
    fn amounts_strip_symbols_and_thousands_separators() {
        assert_eq!(parse_amount("$42.50"), Ok(minor(4250)));
        assert_eq!(parse_amount("€ 7"), Ok(minor(700)));
        assert_eq!(parse_amount("£1,234.56"), Ok(minor(123_456)));
        assert_eq!(parse_amount("1,234"), Ok(minor(123_400)));
        assert_eq!(parse_amount("12,345,678.9"), Ok(minor(1_234_567_890)));
        assert_eq!(parse_amount(" 42.5 "), Ok(minor(4250)));
    }

    #[test]
    fn comma_decimals_are_never_guessed() {
        assert_eq!(parse_amount("42,50"), Err(AmountProblem::CommaDecimal));
        assert_eq!(parse_amount("€42,5"), Err(AmountProblem::CommaDecimal));
        assert_eq!(parse_amount("1.234,56"), Err(AmountProblem::NotAnAmount));
        assert_eq!(parse_amount("12,34,567"), Err(AmountProblem::NotAnAmount));
        let n = with("total", found("42,50", "TOTAL 42,50"));
        assert_eq!(n.fields.total.status, FieldStatus::Unclear);
        assert_eq!(n.fields.total.value, None);
        assert!(n.warnings[0].contains("comma as the decimal separator"));
    }

    #[test]
    fn negative_and_garbage_amounts_are_unclear() {
        assert_eq!(parse_amount("-5.00"), Err(AmountProblem::Negative));
        assert_eq!(parse_amount("-$5.00"), Err(AmountProblem::Negative));
        assert_eq!(parse_amount("$-5.00"), Err(AmountProblem::NotAnAmount));
        assert_eq!(parse_amount("42.505"), Err(AmountProblem::NotAnAmount));
        assert_eq!(parse_amount("forty"), Err(AmountProblem::NotAnAmount));
        assert_eq!(parse_amount("1e3"), Err(AmountProblem::NotAnAmount));
        assert_eq!(
            parse_amount("9007199254740992"),
            Err(AmountProblem::NotAnAmount)
        );
        let n = with("subtotal", found("-2.50", "SUB -2.50"));
        assert_eq!(n.fields.subtotal.status, FieldStatus::Unclear);
        assert_eq!(
            n.warnings,
            vec!["The subtotal is negative. Check it on the receipt."]
        );
    }

    #[test]
    fn a_negative_total_stays_unclear_and_says_it_may_be_a_refund() {
        let mut answer = good();
        answer["total"] = found("-$24.00", "Payment (card): -$24.00");
        answer["subtotal"] = absent();
        answer["tax"] = no_lines();
        let n = run(answer);
        assert_eq!(n.fields.total.status, FieldStatus::Unclear);
        assert_eq!(n.fields.total.value, None);
        assert_eq!(
            n.warnings,
            vec![
                "The only amount is a payment or refund line (−$24.00). Enter the total yourself."
            ]
        );
        // Without a currency the amount is shown bare.
        let mut answer = good();
        answer["currency"] = absent();
        answer["total"] = found("-12.00", "REFUND -12.00");
        assert_eq!(
            run(answer).warnings,
            vec!["The only amount is a payment or refund line (−12.00). Enter the total yourself."]
        );
    }

    #[test]
    fn zero_is_rejected_only_for_the_total() {
        assert_eq!(parse_amount("0.00"), Ok(Minor::ZERO));
        assert_eq!(parse_amount("-0.00"), Ok(Minor::ZERO));
        let n = with("total", found("0.00", "TOTAL $0.00"));
        assert_eq!(n.fields.total.status, FieldStatus::Unclear);
        assert!(n.warnings[0].contains("The total isn't a positive amount"));
        // A zero tax line with a matching total stays found, as does a zero tip.
        let mut answer = good();
        answer["total"] = found("40.00", "TOTAL $40.00");
        answer["tax"] = lines(&[("0.00", "TAX $0.00")]);
        answer["tip"] = found("0.00", "TIP $0.00");
        let n = run(answer);
        assert!(n.warnings.is_empty(), "{:?}", n.warnings);
        assert_eq!(n.fields.tax.value, Some(Minor::ZERO));
        assert_eq!(n.fields.tip.value, Some(Minor::ZERO));
        assert_eq!(n.fields.total.value, Some(minor(4000)));
        let n = with("subtotal", found("0", "SUBTOTAL 0"));
        assert_eq!(n.fields.subtotal.value, Some(Minor::ZERO));
    }

    #[test]
    fn amounts_given_as_json_numbers_are_unclear() {
        let n = with(
            "total",
            json!({ "status": "found", "value": 42.5, "evidence": "TOTAL $42.50" }),
        );
        assert_eq!(n.fields.total.status, FieldStatus::Unclear);
        let n = with(
            "tax",
            json!({ "status": "found", "lines": [{ "amount": 2.5, "evidence": "TAX" }] }),
        );
        assert_eq!(n.fields.tax.status, FieldStatus::Unclear);
        assert_eq!(n.fields.tax.value, None);
    }

    #[test]
    fn the_total_is_checked_and_the_warning_names_what_was_read() {
        let n = with("total", found("40.00", "TOTAL $40.00"));
        assert_eq!(n.fields.total.status, FieldStatus::Unclear);
        assert_eq!(n.fields.total.evidence, "TOTAL $40.00");
        assert_eq!(
            n.warnings,
            vec!["Subtotal $40.00 and tax $2.50 add up to $42.50, but the total reads $40.00. Check the amount."]
        );
        // The brief's example: a surcharge read as a fee.
        let mut answer = good();
        answer["total"] = found("$55.50", "Total $55.50");
        answer["subtotal"] = found("$50.00", "Subtotal $50.00");
        answer["tax"] = lines(&[("$3.00", "Sales tax $3.00")]);
        answer["fees"] = lines(&[("$1.50", "Card surcharge (3%) $1.50")]);
        assert_eq!(
            run(answer).warnings,
            vec!["Subtotal $50.00, tax $3.00 and fees $1.50 add up to $54.50, but the total reads $55.50. Check the amount."]
        );
    }

    #[test]
    fn an_absent_tax_counts_as_zero() {
        let mut answer = good();
        answer["tax"] = no_lines();
        answer["total"] = found("40.00", "TOTAL $40.00");
        let n = run(answer.clone());
        assert_eq!(n.fields.total.value, Some(minor(4000)));
        assert!(n.warnings.is_empty(), "{:?}", n.warnings);
        answer["total"] = found("45.00", "TOTAL $45.00");
        let n = run(answer);
        assert_eq!(n.fields.total.status, FieldStatus::Unclear);
        assert_eq!(
            n.warnings,
            vec!["The subtotal reads $40.00, but the total reads $45.00. If you added a tip, check the amount matches what you paid."]
        );
    }

    #[test]
    fn without_a_subtotal_nothing_is_checked() {
        let mut answer = good();
        answer["subtotal"] = absent();
        answer["total"] = found("99.00", "TOTAL $99.00");
        let n = run(answer);
        assert_eq!(n.fields.total.value, Some(minor(9900)));
        assert!(n.warnings.is_empty(), "{:?}", n.warnings);
    }

    #[test]
    fn several_tax_lines_are_summed() {
        let mut answer = good();
        answer["total"] = found("30.45", "TOTAL $30.45");
        answer["subtotal"] = found("30.00", "SUBTOTAL 30.00");
        answer["tax"] = lines(&[("0.00", "TAX 0.00"), ("0.45", "TAX 0.45")]);
        let n = run(answer);
        assert!(n.warnings.is_empty(), "{:?}", n.warnings);
        assert_eq!(n.fields.tax.value, Some(minor(45)));
        assert_eq!(n.fields.tax.lines.len(), 2);
        assert_eq!(n.fields.total.value, Some(minor(3045)));
    }

    #[test]
    fn fees_reconcile_a_surcharged_total() {
        let mut answer = good();
        answer["total"] = found("$54.50", "Total $54.50");
        answer["subtotal"] = found("$50.00", "Subtotal $50.00");
        answer["tax"] = lines(&[("$3.00", "Sales tax $3.00")]);
        answer["fees"] = lines(&[("$1.50", "Card surcharge (3%) $1.50")]);
        let n = run(answer);
        assert!(n.warnings.is_empty(), "{:?}", n.warnings);
        assert_eq!(n.fields.total.value, Some(minor(5450)));
        assert_eq!(n.fields.fees.value, Some(minor(150)));
    }

    #[test]
    fn discounts_are_positive_whatever_the_printed_sign() {
        for text in [
            "0.25", "-0.25", "0.25-", "-$0.25", "$-0.25", "(0.25)", "($0.25)",
        ] {
            assert_eq!(parse_discount(text), Ok(minor(25)), "{text}");
        }
        assert_eq!(parse_discount("0.25-F"), Err(AmountProblem::NotAnAmount));
        assert!(parse_discount("--0.25").is_err());
        let n = with(
            "discounts",
            lines(&[("1.50-", "COUPON 1.50-"), ("-0.50", "MEMBER SAVINGS -0.50")]),
        );
        assert_eq!(n.fields.discounts.value, Some(minor(200)));
        assert_eq!(n.fields.discounts.lines[0].amount, Some(minor(150)));
        // Tax and fee lines are never negative.
        let n = with("fees", lines(&[("-1.00", "FEE -1.00")]));
        assert_eq!(n.fields.fees.status, FieldStatus::Unclear);
        assert_eq!(
            n.warnings,
            vec!["The fees lines include a negative amount. Check them on the receipt."]
        );
    }

    #[test]
    fn discounts_reconcile_with_a_subtotal_before_or_after_savings() {
        // Subtotal printed before savings: 40.00 − 5.00 + 2.50 = 37.50.
        let mut answer = good();
        answer["total"] = found("37.50", "TOTAL $37.50");
        answer["discounts"] = lines(&[("5.00", "COUPON -5.00")]);
        let n = run(answer.clone());
        assert!(n.warnings.is_empty(), "{:?}", n.warnings);
        assert_eq!(n.fields.total.value, Some(minor(3750)));
        // Subtotal printed after savings: 40.00 + 2.50 = 42.50.
        answer["total"] = found("42.50", "TOTAL $42.50");
        let n = run(answer.clone());
        assert!(n.warnings.is_empty(), "{:?}", n.warnings);
        // Neither: both sums are named.
        answer["total"] = found("41.00", "TOTAL $41.00");
        let n = run(answer);
        assert_eq!(n.fields.total.status, FieldStatus::Unclear);
        assert_eq!(
            n.warnings,
            vec!["Subtotal $40.00, discounts $5.00 and tax $2.50 add up to $37.50 (or $42.50 if the subtotal is already after discounts), but the total reads $41.00. Check the amount."]
        );
    }

    #[test]
    fn one_bad_line_makes_the_whole_list_unclear() {
        let n = with(
            "tax",
            lines(&[("1.00", "STATE TAX 1.00"), ("1,50", "CITY TAX 1,50")]),
        );
        assert_eq!(n.fields.tax.status, FieldStatus::Unclear);
        assert_eq!(n.fields.tax.value, None);
        // The evidence of every line is kept, without amounts.
        assert_eq!(
            n.fields.tax.lines,
            vec![
                ExtractedLine {
                    amount: None,
                    evidence: "STATE TAX 1.00".into()
                },
                ExtractedLine {
                    amount: None,
                    evidence: "CITY TAX 1,50".into()
                }
            ]
        );
        assert_eq!(
            n.warnings[0],
            "The tax lines use a comma as the decimal separator. Enter the amounts yourself."
        );
        // An unclear tax counts as 0 for the total, which then can't be
        // confirmed.
        assert_eq!(n.fields.total.status, FieldStatus::Unclear);
        let n = with("tax", lines(&[("2.50", "")]));
        assert_eq!(n.fields.tax.status, FieldStatus::Unclear);
        assert!(n.warnings[0].contains("didn't show where it saw the tax lines"));
    }

    #[test]
    fn malformed_lists_are_unclear() {
        for tax in [
            json!({ "status": "found", "lines": [] }),
            json!({ "status": "found" }),
            json!({ "status": "found", "lines": "2.50" }),
            json!({ "status": "found", "lines": [{ "evidence": "TAX" }] }),
            json!({ "status": "certain", "lines": [] }),
            json!({ "status": "found", "value": "2.50", "evidence": "TAX $2.50" }),
        ] {
            let n = with("tax", tax.clone());
            assert_eq!(n.fields.tax.status, FieldStatus::Unclear, "{tax}");
            assert_eq!(n.fields.tax.value, None, "{tax}");
        }
        let many: Vec<(&str, &str)> = vec![("0.01", "X"); MAX_LINES + 1];
        let n = with("fees", lines(&many));
        assert_eq!(n.fields.fees.status, FieldStatus::Unclear);
        assert_eq!(n.fields.fees.lines.len(), MAX_LINES);
        // An unclear list keeps what the model saw; an absent one keeps nothing.
        let n = with(
            "fees",
            json!({ "status": "unclear", "lines": [{ "amount": "?", "evidence": "SERVICE FEE ?" }] }),
        );
        assert_eq!(n.fields.fees.lines[0].evidence, "SERVICE FEE ?");
        assert_eq!(n.fields.fees.lines[0].amount, None);
        let n = with(
            "fees",
            json!({ "status": "absent", "lines": [{ "amount": "1.00", "evidence": "FEE" }] }),
        );
        assert_eq!(n.fields.fees, ExtractedLines::default());
        // A tax the model left out is unclear, not absent.
        let mut answer = good();
        answer.as_object_mut().unwrap().remove("tax");
        assert_eq!(run(answer).fields.tax.status, FieldStatus::Unclear);
    }

    #[test]
    fn dates_must_be_real_and_not_in_the_future() {
        for bad in ["2026-02-30", "1899-12-31", "10/24/2026", "2026-10-27"] {
            let n = with("date", found(bad, "DATE"));
            assert_eq!(n.fields.date.status, FieldStatus::Unclear, "{bad}");
            assert_eq!(n.fields.date.value, None);
        }
        assert!(with("date", found("2026-10-27", "DATE")).warnings[0].contains("future"));
        // Up to one day ahead is allowed for time zones.
        let n = with("date", found("2026-10-26", "DATE"));
        assert_eq!(n.fields.date.value.as_deref(), Some("2026-10-26"));
    }

    #[test]
    fn unsupported_currencies_are_unclear_and_named() {
        let n = with("currency", found("CAD", "CAD"));
        assert_eq!(n.fields.currency.status, FieldStatus::Unclear);
        assert_eq!(
            n.warnings,
            vec!["The receipt's currency (CAD) isn't supported. Supported: USD, EUR, GBP."]
        );
        let n = with("currency", found("eur", "EUR"));
        assert_eq!(n.fields.currency.value, Some(Currency::Eur));
        // Only the currencies sent in are allowed.
        let raw: RawFields = serde_json::from_value(good()).unwrap();
        let cats = categories();
        let n = normalize(
            &raw,
            &Context {
                categories: &cats,
                currencies: &[Currency::Eur],
                today: "2026-10-25",
            },
        );
        assert_eq!(n.fields.currency.status, FieldStatus::Unclear);
    }

    #[test]
    fn unknown_category_ids_are_absent() {
        for value in [
            found("cat-made-up", "Office"),
            found("Office & supplies", "Office"),
        ] {
            let n = with("category", value);
            assert_eq!(n.fields.category.status, FieldStatus::Absent);
            assert_eq!(n.fields.category.value, None);
            assert_eq!(n.fields.category.evidence, "");
        }
    }

    #[test]
    fn found_without_evidence_is_unclear() {
        let n = with("vendor", found("Paper & Pine", "  "));
        assert_eq!(n.fields.vendor.status, FieldStatus::Unclear);
        assert!(n.warnings[0].contains("didn't show where it saw the vendor"));
        let n = with(
            "vendor",
            json!({ "status": "found", "value": "Paper & Pine" }),
        );
        assert_eq!(n.fields.vendor.status, FieldStatus::Unclear);
        let n = with("vendor", found("   ", "Paper & Pine"));
        assert_eq!(n.fields.vendor.status, FieldStatus::Unclear);
    }

    #[test]
    fn evidence_is_trimmed_to_120_characters_without_control_characters() {
        let long = "x".repeat(300);
        let n = with("vendor", found("Paper & Pine", &long));
        assert_eq!(n.fields.vendor.evidence.chars().count(), MAX_EVIDENCE_CHARS);
        let n = with(
            "vendor",
            found("Paper\u{0} & Pine", " \u{1b}[31mPaper & Pine\u{7}\n "),
        );
        assert_eq!(n.fields.vendor.evidence, "[31mPaper & Pine");
        assert_eq!(n.fields.vendor.value.as_deref(), Some("Paper & Pine"));
        assert_eq!(clean_text("ééé", 2), "éé");
        let n = with("tax", lines(&[("2.50", &long)]));
        assert_eq!(
            n.fields.tax.lines[0].evidence.chars().count(),
            MAX_EVIDENCE_CHARS
        );
    }

    #[test]
    fn non_found_statuses_never_carry_values() {
        let n = with(
            "total",
            json!({ "status": "unclear", "value": "42.50", "evidence": "TOTAL ?" }),
        );
        assert_eq!(n.fields.total.value, None);
        assert_eq!(n.fields.total.evidence, "TOTAL ?");
        let n = with(
            "reference",
            json!({ "status": "absent", "value": "R-1", "evidence": "R-1" }),
        );
        assert_eq!(n.fields.reference.value, None);
        assert_eq!(n.fields.reference.evidence, "");
        let n = with(
            "vendor",
            json!({ "status": "certain", "value": "X", "evidence": "X" }),
        );
        assert_eq!(n.fields.vendor.status, FieldStatus::Unclear);
        // A field the model left out is unclear, not absent.
        let mut answer = good();
        answer.as_object_mut().unwrap().remove("reference");
        assert_eq!(run(answer).fields.reference.status, FieldStatus::Unclear);
    }

    #[test]
    fn answers_that_are_not_objects_are_malformed() {
        assert!(parse_answer("not json").is_err());
        assert!(parse_answer("[1,2]").is_err());
        assert!(parse_answer("{\"vendor\": 5}").is_err());
        assert!(parse_answer("{\"tax\": 5}").is_err());
        assert!(parse_answer("{}").is_ok());
    }

    #[test]
    fn utc_dates_round_trip() {
        assert_eq!(utc_date(0), "1970-01-01");
        assert_eq!(utc_date(1_792_886_400), "2026-10-25");
        assert_eq!(day_number("2026-10-25"), Some(1_792_886_400 / 86_400));
        assert_eq!(utc_date(951_782_400), "2000-02-29");
    }

    #[test]
    fn a_tip_makes_the_parts_add_up() {
        let mut answer = good();
        answer["total"] = found("51.00", "TOTAL $51.00");
        answer["tip"] = found("8.50", "TIP $8.50");
        let n = run(answer);
        assert!(n.warnings.is_empty(), "{:?}", n.warnings);
        assert_eq!(n.fields.total.value, Some(minor(5100)));
        assert_eq!(n.fields.tip.value, Some(minor(850)));
    }

    #[test]
    fn a_total_above_the_parts_without_a_tip_suggests_one() {
        for tip in [
            absent(),
            json!({ "status": "unclear", "value": null, "evidence": "TIP ___" }),
        ] {
            let mut answer = good();
            answer["total"] = found("51.00", "TOTAL $51.00");
            answer["tip"] = tip;
            let n = run(answer);
            assert_eq!(n.fields.total.status, FieldStatus::Unclear);
            assert_eq!(
                n.warnings,
                vec!["Subtotal $40.00 and tax $2.50 add up to $42.50, but the total reads $51.00. If you added a tip, check the amount matches what you paid."]
            );
        }
        // A tip that doesn't reconcile is named with the other parts.
        let mut answer = good();
        answer["total"] = found("51.00", "TOTAL $51.00");
        answer["tip"] = found("5.00", "TIP $5.00");
        let n = run(answer);
        assert_eq!(n.fields.total.status, FieldStatus::Unclear);
        assert_eq!(
            n.warnings,
            vec!["Subtotal $40.00, tax $2.50 and tip $5.00 add up to $47.50, but the total reads $51.00. Check the amount."]
        );
    }

    #[test]
    fn an_unclear_tip_counts_as_no_tip() {
        let mut answer = good();
        answer["tip"] = json!({ "status": "unclear", "value": null, "evidence": "TIP ?" });
        let n = run(answer);
        assert_eq!(n.fields.total.status, FieldStatus::Found);
        assert!(n.warnings.is_empty(), "{:?}", n.warnings);
    }

    #[test]
    fn an_answer_without_a_tip_means_no_tip_line() {
        let mut answer = good();
        answer.as_object_mut().unwrap().remove("tip");
        let n = run(answer);
        assert_eq!(n.fields.tip.status, FieldStatus::Absent);
        assert_eq!(n.fields.total.status, FieldStatus::Found);
    }

    #[test]
    fn amounts_are_shown_with_the_receipt_currency() {
        assert_eq!(shown(minor(5450), Some(Currency::Usd)), "$54.50");
        assert_eq!(shown(minor(-2400), Some(Currency::Usd)), "−$24.00");
        assert_eq!(shown(minor(200), Some(Currency::Gbp)), "£2.00");
        assert_eq!(shown(minor(5), Some(Currency::Eur)), "€0.05");
        assert_eq!(shown(minor(1200), None), "12.00");
        assert_eq!(join(&["a".into()]), "a");
        assert_eq!(join(&["a".into(), "b".into(), "c".into()]), "a, b and c");
    }
}
