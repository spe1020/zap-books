//! Calendar dates as `YYYY-MM-DD` strings.

use serde::{Deserialize, Serialize};

use crate::js;

/// `validDate`: `^\d{4}-\d{2}-\d{2}$` that names a real proleptic Gregorian
/// day.
///
/// The TypeScript also requires `new Date(date).toISOString().slice(0, 10)`
/// to give back `date`. For a four-digit year that holds exactly when the
/// month is 01-12 and the day exists in that month, which is what this checks.
pub fn valid_date(date: &str) -> bool {
    let Some((year, month, day)) = split_ymd(date) else {
        return false;
    };
    (1..=12).contains(&month) && day >= 1 && day <= days_in_month(year, month)
}

fn split_ymd(date: &str) -> Option<(u32, u32, u32)> {
    let mut parts = date.split('-');
    let year = parts.next().filter(|p| p.len() == 4 && js::all_digits(p))?;
    let month = parts.next().filter(|p| p.len() == 2 && js::all_digits(p))?;
    let day = parts.next().filter(|p| p.len() == 2 && js::all_digits(p))?;
    if parts.next().is_some() {
        return None;
    }
    Some((year.parse().ok()?, month.parse().ok()?, day.parse().ok()?))
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => {
            29
        }
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// `type DateFormat = "iso" | "mdy" | "dmy"`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DateFormat {
    #[default]
    Iso,
    Mdy,
    Dmy,
}

/// `normalizeImportDate`: ISO passes through unchanged; slash dates matching
/// `^(\d{1,2})\/(\d{1,2})\/(\d{4})$` are reordered and zero padded; anything
/// else becomes `""`. No validity check happens here.
pub fn normalize_import_date(value: &str, format: DateFormat) -> String {
    if format == DateFormat::Iso {
        return value.to_string();
    }
    let mut parts = value.split('/');
    let (Some(first), Some(second), Some(year), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return String::new();
    };
    let short = |p: &str| (1..=2).contains(&p.len()) && js::all_digits(p);
    if !short(first) || !short(second) || year.len() != 4 || !js::all_digits(year) {
        return String::new();
    }
    let (month, day) = match format {
        DateFormat::Mdy => (first, second),
        _ => (second, first),
    };
    format!("{year}-{month:0>2}-{day:0>2}")
}
