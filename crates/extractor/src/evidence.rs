//! Checks a value against the evidence the model says it read it from. The
//! evidence is untrusted receipt text; these helpers only look for things in
//! it and never take a value from it.

use ledger_core::{parse_money, valid_date, Currency, Minor};

/// Lowercase words of `text`: runs of letters and digits. Everything else
/// (spaces, punctuation, symbols) separates words.
fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// True if `evidence` contains one of `labels` as whole words, ignoring case.
/// A label may be several words (`"you saved"`), matched in order.
pub fn has_label(evidence: &str, labels: &[&str]) -> bool {
    let words = words(evidence);
    labels.iter().any(|label| {
        let want = self::words(label);
        !want.is_empty() && words.windows(want.len()).any(|w| w == want.as_slice())
    })
}

/// True if `word` appears in `evidence` as a whole word other than directly
/// before `next`, ignoring case. With `credit` and `card`: `STORE CREDIT`
/// and `Store credit to credit card` are true, `CREDIT CARD TOTAL` is false.
pub fn has_word_outside_phrase(evidence: &str, word: &str, next: &str) -> bool {
    let words = words(evidence);
    let (word, next) = (word.to_lowercase(), next.to_lowercase());
    words
        .iter()
        .enumerate()
        .any(|(i, w)| *w == word && words.get(i + 1).is_none_or(|n| *n != next))
}

/// The runs of digits in `evidence`. Spaces and punctuation separate runs
/// and are never part of one: `"Tran# 5103-27"` gives `["5103", "27"]`.
pub fn numbers_in_evidence(evidence: &str) -> Vec<String> {
    evidence
        .split(|c: char| !c.is_ascii_digit())
        .filter(|run| !run.is_empty())
        .map(str::to_string)
        .collect()
}

/// `1,234` or `12,345,678.90`: comma groups of exactly three digits.
pub(crate) fn is_thousands_grouped(s: &str) -> bool {
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

/// An amount printed in evidence, and whether it is printed as negative: a
/// minus sign before it (`-42.50`, `-$42.50`, `$-42.50`, past spaces and
/// currency symbols), a minus right after it (`42.50-`), or parentheses
/// around it (`(42.50)`). The hyphen in `T-SHIRT 42.50` or `SUB-TOTAL 42.50`
/// is not a sign, nor is the dash in `42.50 - thank you`.
///
/// `credit_marked` is a `CR` marker right after the amount (`42.50CR`,
/// `42.50 CR`), the way card statements print a credit without a sign.
struct PrintedAmount {
    amount: Minor,
    negative: bool,
    credit_marked: bool,
}

/// The amounts printed in `evidence`: each run of digits, commas and points,
/// read exactly, with the sign the receipt prints around it. Currency
/// symbols and spaces are ignored. A comma that isn't a thousands separator
/// splits runs.
fn printed_amounts(evidence: &str) -> Vec<PrintedAmount> {
    let chars: Vec<char> = evidence.chars().collect();
    let is_money_symbol = |c: &char| matches!(c, '$' | '€' | '£');
    let mut amounts = Vec::new();
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        if !c.is_ascii_digit() {
            i += 1;
            continue;
        }
        // `.99` reads as 0.99 unless the point ends another number.
        let point_before = i >= 1
            && chars.get(i - 1) == Some(&'.')
            && !(i >= 2 && chars.get(i - 2).is_some_and(char::is_ascii_digit));
        let start = i;
        while chars
            .get(i)
            .is_some_and(|c| c.is_ascii_digit() || *c == ',' || *c == '.')
        {
            i += 1;
        }
        let run: String = chars.get(start..i).unwrap_or_default().iter().collect();
        let trimmed = run.trim_end_matches([',', '.']);
        // The character before the number, past spaces and currency symbols
        // (and the point of `.99`), and the one after it, past spaces.
        let number_start = if point_before { start - 1 } else { start };
        let before = chars
            .get(..number_start)
            .unwrap_or_default()
            .iter()
            .rev()
            .find(|c| !c.is_whitespace() && !is_money_symbol(c));
        // A trailing minus is printed against the number (`42.50-`); with a
        // space it is a dash, not a sign. A closing parenthesis may follow
        // a space.
        let rest = chars
            .get(start + trimmed.chars().count()..)
            .unwrap_or_default();
        let after_adjacent = rest.first();
        let after = rest.iter().find(|c| !c.is_whitespace());
        let minus = |c: Option<&char>| matches!(c, Some('-' | '−'));
        let negative =
            minus(before) || minus(after_adjacent) || (before == Some(&'(') && after == Some(&')'));
        // `CR` as a whole word right after the number, past spaces only:
        // `42.50CR` and `42.50 CR`, not `42.50 CREDIT` (a word, handled by
        // the refund words) and not a `CR` before the number.
        let after_word: Vec<char> = rest
            .iter()
            .skip_while(|c| c.is_whitespace())
            .take(3)
            .copied()
            .collect();
        let credit_marked = matches!(
            (after_word.first(), after_word.get(1)),
            (Some(c), Some(r)) if c.eq_ignore_ascii_case(&'c') && r.eq_ignore_ascii_case(&'r')
        ) && after_word.get(2).is_none_or(|c| !c.is_alphanumeric());
        let run = if point_before {
            format!("0.{trimmed}")
        } else {
            trimmed.to_string()
        };
        let pieces: Vec<String> = if is_thousands_grouped(&run) {
            vec![run.replace(',', "")]
        } else {
            run.split(',').map(str::to_string).collect()
        };
        amounts.extend(
            pieces
                .iter()
                .filter_map(|p| parse_money(p).ok())
                .map(|amount| PrintedAmount {
                    amount,
                    negative,
                    credit_marked,
                }),
        );
    }
    amounts
}

/// The amounts printed in `evidence`, signs ignored.
fn amounts_in(evidence: &str) -> Vec<Minor> {
    printed_amounts(evidence)
        .into_iter()
        .map(|p| p.amount)
        .collect()
}

/// True if `evidence` prints `value`, ignoring currency symbols, thousands
/// separators, spaces, and a leading or trailing minus or parentheses:
/// `TOTAL $1,042.50`, `COUPON 0.25-` and `(0.25)` all contain their amounts.
/// Discount, tax and fee lines rely on this; for the total, which must not
/// be a refund, see `amount_shown_negative`.
pub fn amount_in_evidence(value: Minor, evidence: &str) -> bool {
    let wanted = i64::from(value).abs();
    amounts_in(evidence)
        .iter()
        .any(|a| i64::from(*a).abs() == wanted)
}

/// True if `evidence` prints `value` as a negative amount: with a minus
/// sign before or after it, or in parentheses. `TOTAL -42.50`,
/// `Total (42.50)` and `Paid 42.50-` all do; `TOTAL $42.50`, `T-SHIRT 42.50`
/// and `(3%) $1.50` do not. The sign is compared ignoring the sign of
/// `value` itself, so a positive value the receipt prints negative is caught.
pub fn amount_shown_negative(value: Minor, evidence: &str) -> bool {
    let wanted = i64::from(value).abs();
    printed_amounts(evidence)
        .iter()
        .any(|p| p.negative && i64::from(p.amount).abs() == wanted)
}

/// True if `evidence` prints `value` with a `CR` credit marker right after
/// it: `TOTAL 42.50 CR` and `TOTAL 42.50CR` do; `CR 42.50` (a marker before
/// the amount), `42.50 CREDIT` (a word) and `42.50 CRN` do not. Scoped to
/// the amount, not the line, so initials and product codes elsewhere on the
/// line don't count. The sign of `value` is ignored.
pub fn amount_marked_credit(value: Minor, evidence: &str) -> bool {
    let wanted = i64::from(value).abs();
    printed_amounts(evidence)
        .iter()
        .any(|p| p.credit_marked && i64::from(p.amount).abs() == wanted)
}

/// One piece of date evidence: a run of digits, a run of letters
/// (lowercase), a separator that may sit inside a date, or anything else.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Num(String),
    Word(String),
    Sep(char),
    Other,
}

fn tokens(text: &str) -> Vec<Tok> {
    let mut out: Vec<Tok> = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let mut run = |first: char, keep: fn(&char) -> bool| {
            let mut s = String::from(first);
            while let Some(next) = chars.next_if(keep) {
                s.push(next);
            }
            s
        };
        out.push(if c.is_ascii_digit() {
            Tok::Num(run(c, char::is_ascii_digit))
        } else if c.is_alphabetic() {
            Tok::Word(run(c, |c| c.is_alphabetic()).to_lowercase())
        } else if c.is_whitespace() {
            run(c, |c| c.is_whitespace());
            Tok::Sep(' ')
        } else if matches!(c, '/' | '-' | '.' | ',') {
            Tok::Sep(c)
        } else {
            Tok::Other
        });
    }
    out
}

const MONTHS: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

/// 1 to 12 for a month name or an abbreviation of at least three letters.
fn month(word: &str) -> Option<u32> {
    if word.len() < 3 {
        return None;
    }
    let index = MONTHS.iter().position(|m| m.starts_with(word))?;
    u32::try_from(index + 1).ok()
}

/// A valid `YYYY-MM-DD`, or `None`.
fn ymd(year: u32, month: u32, day: u32) -> Option<String> {
    let date = format!("{year:04}-{month:02}-{day:02}");
    valid_date(&date).then_some(date)
}

/// A printed year: four digits, or two digits meaning 20YY.
fn year(digits: &str) -> Option<u32> {
    let n: u32 = digits.parse().ok()?;
    match digits.len() {
        4 => Some(n),
        2 => Some(2000 + n),
        _ => None,
    }
}

fn day_or_month(digits: &str) -> Option<u32> {
    (1..=2)
        .contains(&digits.len())
        .then(|| digits.parse().ok())
        .flatten()
}

/// What one date in the evidence reads as.
enum Reading {
    One(String),
    /// Day and month could be swapped and the currency doesn't settle it.
    Ambiguous,
}

/// Reads the dates printed in `evidence`: numeric (`M/D/YY`, `D/M/YYYY`,
/// `YYYY-MM-DD`, with `/`, `-` or `.`) and with English month names (`Oct 12,
/// 2026`, `12 October 2026`, `16-Feb-2026`). Two-digit years are 20YY. When
/// day and month could be swapped, USD means month first and EUR or GBP day
/// first; with no currency the date is ambiguous.
fn readings(evidence: &str, currency: Option<Currency>) -> Vec<Reading> {
    let toks = tokens(evidence);
    let num = |i: usize| match toks.get(i) {
        Some(Tok::Num(n)) => Some(n.as_str()),
        _ => None,
    };
    let date_sep = |i: usize| match toks.get(i) {
        Some(Tok::Sep(c @ ('/' | '-' | '.'))) => Some(*c),
        _ => None,
    };
    // A two-digit year followed by `:` or `.12` is the start of a time.
    let ends_cleanly = |i: usize| {
        !matches!(toks.get(i), Some(Tok::Other))
            && !(date_sep(i) == Some('.') && num(i + 1).is_some())
    };
    let mut out = Vec::new();
    for i in 0..toks.len() {
        // Numeric: a s b s c, not part of a longer chain like a phone number.
        if let (Some(a), Some(s), Some(b), Some(c)) =
            (num(i), date_sep(i + 1), num(i + 2), num(i + 4))
        {
            let chained_before = i >= 2 && date_sep(i - 1).is_some() && num(i - 2).is_some();
            let chained_after = date_sep(i + 5).is_some() && num(i + 6).is_some();
            if date_sep(i + 3) == Some(s) && !chained_before && !chained_after {
                if a.len() == 4 {
                    if let (Ok(y), Some(m), Some(d)) = (a.parse(), day_or_month(b), day_or_month(c))
                    {
                        out.extend(ymd(y, m, d).map(Reading::One));
                    }
                } else if let (Some(x), Some(z), Some(y)) =
                    (day_or_month(a), day_or_month(b), year(c))
                {
                    if c.len() == 4 || ends_cleanly(i + 5) {
                        let month_first = ymd(y, x, z);
                        let day_first = ymd(y, z, x);
                        out.extend(match (month_first, day_first) {
                            (Some(m), Some(d)) if m != d => match currency {
                                Some(Currency::Usd) => Some(Reading::One(m)),
                                Some(Currency::Eur | Currency::Gbp) => Some(Reading::One(d)),
                                None => Some(Reading::Ambiguous),
                            },
                            (Some(only), _) | (None, Some(only)) => Some(Reading::One(only)),
                            (None, None) => None,
                        });
                    }
                }
            }
        }
        // With a month name: "Oct 12, 2026", "12 October 2026", "16-Feb-2026".
        if let Some(Tok::Word(w)) = toks.get(i) {
            if let Some(m) = month(w) {
                // Month first: day after, then year.
                if let Some((d, j)) = day_after(&toks, i + 1) {
                    if let Some((y, k)) = year_after(&toks, j) {
                        if ends_cleanly(k) {
                            out.extend(ymd(y, m, d).map(Reading::One));
                        }
                    }
                }
                // Day first: day before, then year after the month.
                if let Some(d) = day_before(&toks, i) {
                    if let Some((y, k)) = year_after(&toks, i + 1) {
                        if ends_cleanly(k) {
                            out.extend(ymd(y, m, d).map(Reading::One));
                        }
                    }
                }
            }
        }
    }
    out
}

/// Skips one or two separators from `i`; the index after them.
fn skip_seps(toks: &[Tok], i: usize) -> Option<usize> {
    let is_sep = |j: usize| matches!(toks.get(j), Some(Tok::Sep(_)));
    if !is_sep(i) {
        return None;
    }
    Some(if is_sep(i + 1) { i + 2 } else { i + 1 })
}

/// A day after separators at `i`, with an optional `st`/`nd`/`rd`/`th`;
/// returns it and the index after it.
fn day_after(toks: &[Tok], i: usize) -> Option<(u32, usize)> {
    let j = skip_seps(toks, i)?;
    let Some(Tok::Num(n)) = toks.get(j) else {
        return None;
    };
    let d = day_or_month(n)?;
    let ordinal = matches!(toks.get(j + 1), Some(Tok::Word(w)) if ["st", "nd", "rd", "th"].contains(&w.as_str()));
    Some((d, if ordinal { j + 2 } else { j + 1 }))
}

/// A day just before the month name at `i`, separated by one or two
/// separators, with an optional ordinal suffix.
fn day_before(toks: &[Tok], i: usize) -> Option<u32> {
    let mut j = i.checked_sub(1)?;
    if !matches!(toks.get(j), Some(Tok::Sep(_))) {
        return None;
    }
    j = j.checked_sub(1)?;
    if matches!(toks.get(j), Some(Tok::Sep(_))) {
        j = j.checked_sub(1)?;
    }
    if matches!(toks.get(j), Some(Tok::Word(w)) if ["st", "nd", "rd", "th"].contains(&w.as_str())) {
        j = j.checked_sub(1)?;
    }
    match toks.get(j) {
        // Not the tail of a longer number such as 12.25.
        Some(Tok::Num(n))
            if !(j >= 2
                && matches!(toks.get(j - 1), Some(Tok::Sep('.' | '/' | '-')))
                && matches!(toks.get(j - 2), Some(Tok::Num(_)))) =>
        {
            day_or_month(n)
        }
        _ => None,
    }
}

/// A year after separators at `i`; returns it and the index after it.
fn year_after(toks: &[Tok], i: usize) -> Option<(u32, usize)> {
    let j = skip_seps(toks, i)?;
    let Some(Tok::Num(n)) = toks.get(j) else {
        return None;
    };
    Some((year(n)?, j + 1))
}

/// The one date `evidence` shows, read by the rules of `readings`. `None` if
/// it shows no date, a date without a year, a day and month that the
/// currency can't settle, or two different dates.
pub fn date_in_evidence(evidence: &str, currency: Option<Currency>) -> Option<String> {
    let mut found: Option<String> = None;
    for reading in readings(evidence, currency) {
        match reading {
            Reading::Ambiguous => return None,
            Reading::One(date) => match &found {
                Some(other) if *other != date => return None,
                _ => found = Some(date),
            },
        }
    }
    found
}

/// The percentages printed in `evidence` (`12.5%`, `3 %`), each as
/// `(digits, decimals)`: `12.5%` is `(125, 1)`.
fn percentages(evidence: &str) -> Vec<(i128, u32)> {
    let chars: Vec<char> = evidence.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if !chars.get(i).is_some_and(char::is_ascii_digit) {
            i += 1;
            continue;
        }
        let start = i;
        while chars
            .get(i)
            .is_some_and(|c| c.is_ascii_digit() || *c == '.')
        {
            i += 1;
        }
        let number: String = chars.get(start..i).unwrap_or_default().iter().collect();
        let mut j = i;
        while chars.get(j) == Some(&' ') {
            j += 1;
        }
        if chars.get(j) != Some(&'%') {
            continue;
        }
        let (whole, frac) = number.split_once('.').unwrap_or((&number, ""));
        if frac.contains('.') {
            continue;
        }
        if let (Ok(n), Ok(decimals)) = (
            format!("{whole}{frac}").parse::<i128>(),
            u32::try_from(frac.len()),
        ) {
            out.push((n, decimals));
        }
    }
    out
}

/// Whether `amount` is a percentage printed in `evidence` of one of `bases`,
/// give or take one cent. `None` when the evidence prints no percentage.
pub fn fits_percentage(amount: Minor, bases: &[Minor], evidence: &str) -> Option<bool> {
    let printed = percentages(evidence);
    if printed.is_empty() {
        return None;
    }
    let amount = i128::from(i64::from(amount));
    Some(printed.iter().any(|&(n, decimals)| {
        // amount ≈ base × n / (100 × 10^decimals), in cents.
        let Some(scale) = 10_i128
            .checked_pow(decimals)
            .and_then(|p| p.checked_mul(100))
        else {
            return false;
        };
        bases.iter().any(|base| {
            let exact = i128::from(i64::from(*base)).saturating_mul(n);
            amount.saturating_mul(scale).saturating_sub(exact).abs() <= scale
        })
    }))
}

/// Words that mark a line as a deduction.
pub const DEDUCTION_WORDS: [&str; 9] = [
    "disc", "discount", "coupon", "saving", "savings", "off", "reward", "markdown", "promo",
];

/// Whether `evidence` shows a deduction: a minus sign or an opening
/// parenthesis next to an amount, or a deduction word. A hyphen between
/// letters, as in `T-SHIRT`, is not a minus sign.
pub fn shows_deduction(evidence: &str) -> bool {
    let chars: Vec<char> = evidence.chars().collect();
    let money = |c: Option<&char>| {
        c.is_some_and(|c| c.is_ascii_digit() || matches!(c, '$' | '€' | '£' | '.'))
    };
    let signed = chars.iter().enumerate().any(|(i, c)| match c {
        '-' | '−' => money(chars.get(i + 1)) || (i > 0 && money(chars.get(i - 1))),
        '(' => money(chars.get(i + 1)),
        _ => false,
    });
    signed || has_label(evidence, &DEDUCTION_WORDS)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn minor(n: i64) -> Minor {
        Minor::new(n).unwrap()
    }

    #[test]
    fn amounts_are_found_whatever_surrounds_them() {
        for (value, evidence) in [
            (4250, "TOTAL $42.50"),
            (4250, "Total = $ 42.50"),
            (104_250, "TOTAL £1,042.50"),
            (25, "COUPON 0.25-"),
            (25, "MEMBER SAVINGS -$0.25"),
            (25, "Promo (0.25)"),
            (99, "SAVINGS .99"),
            (4000, "SUBTOTAL 40"),
            (419, "Sales tax 6% 4.19T"),
            (150, "Card surcharge (3%) $1.50"),
            (0, "TAX 0.00"),
        ] {
            assert!(amount_in_evidence(minor(value), evidence), "{evidence}");
        }
    }

    #[test]
    fn a_different_or_missing_amount_is_not_found() {
        for (value, evidence) in [
            (4250, "$42.05"),
            (4250, "TOTAL"),
            (4250, "TOTAL $425.00"),
            (4250, "TOTAL 4250"),
            (615, "Surcharge (2.50%) $1.65"),
            (1050, "Order 1050.123"),
        ] {
            assert!(!amount_in_evidence(minor(value), evidence), "{evidence}");
        }
        // A part of a longer number never counts.
        assert!(!amount_in_evidence(minor(4250), "REF 1242.50"));
        // A comma that isn't a thousands separator splits two amounts.
        assert!(amount_in_evidence(minor(150), "1.50,2.00"));
    }

    #[test]
    fn a_printed_sign_is_seen_for_the_amount_it_belongs_to() {
        for evidence in [
            "TOTAL -42.50",
            "TOTAL -$42.50",
            "TOTAL $-42.50",
            "Total −42.50",
            "TOTAL (42.50)",
            "Total ($42.50)",
            "REFUND TOTAL ( 42.50 )",
            "Paid 42.50-",
            "Payment (card): -$42.50",
            "TOTAL-42.50",
        ] {
            assert!(amount_shown_negative(minor(4250), evidence), "{evidence}");
            assert!(amount_in_evidence(minor(4250), evidence), "{evidence}");
        }
        for evidence in [
            "TOTAL $42.50",
            "TOTAL 42.50",
            "T-SHIRT 42.50",
            "SUB-TOTAL 42.50",
            "(2 items) TOTAL 42.50",
            "TOTAL 42.50 (incl. tax)",
            "Card surcharge (3%) $42.50",
            "10-28 TOTAL 42.50",
            "TOTAL 42.50 - thank you",
            "TOTAL 42.50)",
            "(TOTAL 42.50",
        ] {
            assert!(!amount_shown_negative(minor(4250), evidence), "{evidence}");
            assert!(amount_in_evidence(minor(4250), evidence), "{evidence}");
        }
        // The sign belongs to the amount it is printed on, not to others.
        assert!(amount_shown_negative(
            minor(25),
            "Subtotal 42.50 Coupon -0.25"
        ));
        assert!(!amount_shown_negative(
            minor(4250),
            "Subtotal 42.50 Coupon -0.25"
        ));
        assert!(amount_shown_negative(minor(99), "SAVINGS -.99"));
        // A value's own sign doesn't matter; only the receipt's does.
        assert!(amount_shown_negative(minor(-4250), "TOTAL (42.50)"));
        assert!(!amount_shown_negative(minor(-4250), "TOTAL 42.50"));
        // A different amount printed negative doesn't count.
        assert!(!amount_shown_negative(minor(4250), "TOTAL -42.05"));
    }

    #[test]
    fn a_cr_marker_right_after_the_amount_is_a_credit() {
        for evidence in [
            "TOTAL 42.50 CR",
            "TOTAL 42.50CR",
            "Total $42.50 cr",
            "TOTAL 42.50 CR.",
            "TOTAL 42.50  Cr  ",
            "TOTAL 42.50 CR THANK YOU",
        ] {
            assert!(amount_marked_credit(minor(4250), evidence), "{evidence}");
            assert!(amount_in_evidence(minor(4250), evidence), "{evidence}");
            // A marker is not a sign.
            assert!(!amount_shown_negative(minor(4250), evidence), "{evidence}");
        }
        for evidence in [
            "TOTAL 42.50",
            "CR 42.50",
            "CR TOTAL 42.50",
            "TOTAL 42.50 CREDIT",
            "TOTAL 42.50 CRN",
            "TOTAL 42.50 C R",
            "TOTAL 42.50 - CR",
            "SUBTOTAL 40.00 CR TOTAL 42.50",
            "C.R. SMITH 42.50",
        ] {
            assert!(!amount_marked_credit(minor(4250), evidence), "{evidence}");
        }
        // The marker belongs to the amount it follows.
        assert!(amount_marked_credit(
            minor(4000),
            "SUBTOTAL 40.00 CR TOTAL 42.50"
        ));
        assert!(amount_marked_credit(minor(-4250), "TOTAL 42.50 CR"));
    }

    #[test]
    fn labels_match_whole_words_ignoring_case() {
        assert!(has_label("TOTAL PAID · USD $42.50", &["total"]));
        assert!(has_label("Grand-Total: $63.18", &["grand"]));
        assert!(has_label("Tran# 4471", &["tran"]));
        assert!(!has_label("Subtotal $40.00", &["total"]));
        assert!(!has_label("Transaction 4471", &["tran"]));
        assert!(has_label("You Saved $3.00 today", &["you saved"]));
        assert!(!has_label("You have saved $3.00", &["you saved"]));
        assert!(!has_label("anything", &[]));
        assert!(!has_label("anything", &[""]));
    }

    #[test]
    fn numbers_are_digit_runs() {
        assert_eq!(numbers_in_evidence("Tran# 5103-27"), vec!["5103", "27"]);
        assert_eq!(numbers_in_evidence("Order 61, kiosk 3"), vec!["61", "3"]);
        assert!(numbers_in_evidence("no digits").is_empty());
    }
    #[test]
    fn dates_are_read_from_their_evidence() {
        let usd = Some(Currency::Usd);
        for (evidence, date) in [
            ("2026-09-14", "2026-09-14"),
            ("PURCHASE DATE 2026/09/14", "2026-09-14"),
            ("11/14/25 3:05 PM", "2025-11-14"),
            ("Date: 8/19/26", "2026-08-19"),
            ("Sale 7/22/2025 10:14:09 AM", "2025-07-22"),
            ("24.09.2026", "2026-09-24"),
            ("Oct 12, 2026", "2026-10-12"),
            ("October 12 2026", "2026-10-12"),
            ("12 October 2026", "2026-10-12"),
            ("16-Feb-2026 09:41:12A", "2026-02-16"),
            ("October 2nd, 2026", "2026-10-02"),
            ("Sept 3 26", "2026-09-03"),
            // The same date printed twice is one date.
            ("09/24/2026 · Sep 24, 2026", "2026-09-24"),
        ] {
            assert_eq!(
                date_in_evidence(evidence, usd).as_deref(),
                Some(date),
                "{evidence}"
            );
        }
    }

    #[test]
    fn day_and_month_order_follows_the_currency() {
        assert_eq!(
            date_in_evidence("03/04/26", Some(Currency::Usd)).as_deref(),
            Some("2026-03-04")
        );
        assert_eq!(
            date_in_evidence("03/04/26", Some(Currency::Gbp)).as_deref(),
            Some("2026-04-03")
        );
        assert_eq!(
            date_in_evidence("03/04/26", Some(Currency::Eur)).as_deref(),
            Some("2026-04-03")
        );
        assert_eq!(date_in_evidence("03/04/26", None), None);
        // Only one reading is a real date, so the currency doesn't matter.
        assert_eq!(
            date_in_evidence("21/10/2026", None).as_deref(),
            Some("2026-10-21")
        );
        assert_eq!(
            date_in_evidence("10/21/2026", Some(Currency::Gbp)).as_deref(),
            Some("2026-10-21")
        );
        // Both readings are the same date.
        assert_eq!(
            date_in_evidence("05/05/26", None).as_deref(),
            Some("2026-05-05")
        );
    }

    #[test]
    fn evidence_without_one_clear_date_gives_none() {
        let usd = Some(Currency::Usd);
        for evidence in [
            "12/19 10:42 AM",
            "DATE",
            "Table 12/19 Guests 2025",
            "Oct 12, Store 2026",
            "TEL 312-555-0142",
            "Card 1234/5678/90",
            "02/30/2026",
            "09/24/2026 and 09/23/2026",
            "Oct 12 10:42",
        ] {
            assert_eq!(date_in_evidence(evidence, usd), None, "{evidence}");
        }
        // An unrelated number that looks like a year is never one.
        assert_eq!(
            date_in_evidence("Date: 8/19/26 Cashier: 2019457", usd).as_deref(),
            Some("2026-08-19")
        );
    }
    #[test]
    fn percentages_are_checked_against_each_base() {
        let m = |n| Minor::new(n).unwrap();
        // 12.5% of 36.00 is 4.50; of 30.00 it is 3.75.
        let evidence = "Service (12.5%, optional) £4.50";
        assert_eq!(fits_percentage(m(450), &[m(3000)], evidence), Some(false));
        assert_eq!(
            fits_percentage(m(450), &[m(3000), m(3600)], evidence),
            Some(true)
        );
        // One cent either way is rounding.
        assert_eq!(
            fits_percentage(m(165), &[m(6580)], "Surcharge (2.50%) $1.65"),
            Some(true)
        );
        assert_eq!(
            fits_percentage(m(164), &[m(6580)], "Surcharge (2.50%) $1.64"),
            Some(true)
        );
        assert_eq!(
            fits_percentage(m(615), &[m(6580)], "Surcharge (2.50%) $6.15"),
            Some(false)
        );
        assert_eq!(
            fits_percentage(m(150), &[m(5000)], "Card fee 3 % $1.50"),
            Some(true)
        );
        assert_eq!(fits_percentage(m(150), &[m(5000)], "Card fee $1.50"), None);
    }

    #[test]
    fn deductions_show_a_sign_or_a_word() {
        for evidence in [
            "SALE ITEM 2.40-F",
            "ITEM -$0.25",
            "ITEM −0.25",
            "ITEM ($1.50)",
            "BAKERY DISC 0.40",
            "Member savings 2.00",
            "Coupon 1.00",
            "20% OFF 3.00",
            "Promo 0.50",
        ] {
            assert!(shows_deduction(evidence), "{evidence}");
        }
        for evidence in [
            "CLEARANCE 13.00",
            "T-SHIRT 9.99",
            "SALE PRICE 4.00",
            "Offer 2.00",
        ] {
            assert!(!shows_deduction(evidence), "{evidence}");
        }
    }
}
