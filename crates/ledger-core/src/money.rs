//! Currencies, minor-unit amounts, and amount text input.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::{LedgerError, Result};
use crate::js;

/// `Number.MAX_SAFE_INTEGER`. Amounts are stored as JSON numbers and must
/// round-trip through JavaScript without loss, so this, not `i64::MAX`, is
/// the ceiling. Every `Number.isSafeInteger` check in the TypeScript is a
/// range check against this constant.
pub const MAX_MINOR: i64 = 9_007_199_254_740_991;

/// `type Currency = "USD" | "EUR" | "GBP"`
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Currency {
    Usd,
    Eur,
    Gbp,
}

impl Currency {
    /// `currencies`, in the same order as the TypeScript array.
    pub const ALL: [Currency; 3] = [Currency::Usd, Currency::Eur, Currency::Gbp];

    pub fn code(self) -> &'static str {
        match self {
            Currency::Usd => "USD",
            Currency::Eur => "EUR",
            Currency::Gbp => "GBP",
        }
    }

    /// `currencies.includes(code)`, returning the matching currency.
    pub fn from_code(code: &str) -> Option<Currency> {
        Currency::ALL.into_iter().find(|c| c.code() == code)
    }
}

impl fmt::Display for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

/// An amount in minor units (cents). Always within `±MAX_MINOR`, which is the
/// range JavaScript's `Number.isSafeInteger` accepts.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(try_from = "i64", into = "i64")]
pub struct Minor(i64);

impl Minor {
    pub const ZERO: Minor = Minor(0);

    /// Accepts `value` only if `Number.isSafeInteger(value)` would.
    pub fn new(value: i64) -> Result<Minor> {
        if (-MAX_MINOR..=MAX_MINOR).contains(&value) {
            Ok(Minor(value))
        } else {
            Err(LedgerError::AmountTooLarge)
        }
    }

    pub fn get(self) -> i64 {
        self.0
    }

    /// The TypeScript `sum(a, b)`: addition that rejects results outside the
    /// safe-integer range.
    pub fn checked_add(self, rhs: Minor) -> Result<Minor> {
        self.0
            .checked_add(rhs.0)
            .and_then(|v| Minor::new(v).ok())
            .ok_or(LedgerError::TotalOutOfRange)
    }

    /// `a - b`, rejecting results outside the safe-integer range.
    pub fn checked_sub(self, rhs: Minor) -> Result<Minor> {
        self.0
            .checked_sub(rhs.0)
            .and_then(|v| Minor::new(v).ok())
            .ok_or(LedgerError::TotalOutOfRange)
    }
}

/// `-a`. The safe range is symmetric, so this cannot leave it.
impl std::ops::Neg for Minor {
    type Output = Minor;
    fn neg(self) -> Minor {
        Minor(-self.0)
    }
}

impl TryFrom<i64> for Minor {
    type Error = LedgerError;
    fn try_from(value: i64) -> Result<Minor> {
        Minor::new(value)
    }
}

impl From<Minor> for i64 {
    fn from(value: Minor) -> i64 {
        value.0
    }
}

impl fmt::Display for Minor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// `parseMoney`: parses `-?\d+(\.\d{1,2})?` after a JS `trim()` into exact
/// minor units.
pub fn parse_money(input: &str) -> Result<Minor> {
    let s = js::trim(input);
    let (negative, unsigned) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    let (whole, cents) = match unsigned.split_once('.') {
        Some((whole, cents)) => (whole, Some(cents)),
        None => (unsigned, None),
    };
    let cents_ok = cents.is_none_or(|c| (1..=2).contains(&c.len()) && js::all_digits(c));
    if !js::all_digits(whole) || !cents_ok {
        return Err(LedgerError::InvalidAmount);
    }
    // Number(whole) * 100 + Number(cents.padEnd(2, "0")). Accumulate exactly
    // and stop as soon as the value leaves the safe range, so long runs of
    // digits cannot overflow.
    let limit = u128::from(MAX_MINOR.unsigned_abs());
    let mut value: u128 = 0;
    for digit in whole.bytes() {
        value = value * 10 + u128::from(digit - b'0');
        if value > limit {
            return Err(LedgerError::AmountTooLarge);
        }
    }
    let cents_value = match cents.map(str::as_bytes) {
        Some([tens]) => u128::from(tens - b'0') * 10,
        Some([tens, ones]) => u128::from(tens - b'0') * 10 + u128::from(ones - b'0'),
        _ => 0,
    };
    value = value * 100 + cents_value;
    let value = i64::try_from(value)
        .ok()
        .filter(|v| *v <= MAX_MINOR)
        .ok_or(LedgerError::AmountTooLarge)?;
    Minor::new(if negative { -value } else { value })
}

/// `amountInput`: formats minor units as `-?\d+\.\d\d`.
pub fn amount_input(n: Minor) -> String {
    let value = n.get();
    let sign = if value < 0 { "-" } else { "" };
    let abs = value.unsigned_abs();
    format!("{sign}{}.{:02}", abs / 100, abs % 100)
}
