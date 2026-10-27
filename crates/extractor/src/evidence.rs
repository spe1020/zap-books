//! Checks a value against the evidence the model says it read it from. The
//! evidence is untrusted receipt text; these helpers only look for things in
//! it and never take a value from it.

use ledger_core::{parse_money, Minor};

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

/// The amounts printed in `evidence`: each run of digits, commas and points,
/// read exactly. Currency symbols, spaces, signs and parentheses around a
/// number are ignored. A comma that isn't a thousands separator splits runs.
fn amounts_in(evidence: &str) -> Vec<Minor> {
    let chars: Vec<char> = evidence.chars().collect();
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
        let run = run.trim_end_matches([',', '.']);
        let run = if point_before {
            format!("0.{run}")
        } else {
            run.to_string()
        };
        let pieces: Vec<String> = if is_thousands_grouped(&run) {
            vec![run.replace(',', "")]
        } else {
            run.split(',').map(str::to_string).collect()
        };
        amounts.extend(pieces.iter().filter_map(|p| parse_money(p).ok()));
    }
    amounts
}

/// True if `evidence` prints `value`, ignoring currency symbols, thousands
/// separators, spaces, and a leading or trailing minus or parentheses:
/// `TOTAL $1,042.50`, `COUPON 0.25-` and `(0.25)` all contain their amounts.
pub fn amount_in_evidence(value: Minor, evidence: &str) -> bool {
    let wanted = i64::from(value).abs();
    amounts_in(evidence)
        .iter()
        .any(|a| i64::from(*a).abs() == wanted)
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
    fn labels_match_whole_words_ignoring_case() {
        assert!(has_label("TOTAL PAID · USD $42.50", &["total"]));
        assert!(has_label("Grand-Total: $9.00", &["grand"]));
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
}
