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

/// `type Currency = FiatCurrency | "BTC"`. BTC amounts are in sats; a
/// business reports, and records fiat values, in the other three.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Currency {
    Usd,
    Eur,
    Gbp,
    Btc,
}

impl Currency {
    /// `currencies`, in the same order as the TypeScript array.
    pub const ALL: [Currency; 4] = [Currency::Usd, Currency::Eur, Currency::Gbp, Currency::Btc];

    /// `fiatCurrencies`: the reporting currencies.
    pub const FIAT: [Currency; 3] = [Currency::Usd, Currency::Eur, Currency::Gbp];

    pub fn code(self) -> &'static str {
        match self {
            Currency::Usd => "USD",
            Currency::Eur => "EUR",
            Currency::Gbp => "GBP",
            Currency::Btc => "BTC",
        }
    }

    /// `precision`: the decimal places of the minor unit, 2 for cents and 8
    /// for sats.
    pub fn precision(self) -> u32 {
        match self {
            Currency::Btc => 8,
            Currency::Usd | Currency::Eur | Currency::Gbp => 2,
        }
    }

    /// `fiatCurrencies.includes(this)`
    pub fn is_fiat(self) -> bool {
        self != Currency::Btc
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

/// `parseMoney`: parses `-?\d+(\.\d{1,p})?` after a JS `trim()` into exact
/// minor units, with p the currency's precision.
pub fn parse_money(input: &str, currency: Currency) -> Result<Minor> {
    let places = currency.precision() as usize;
    let s = js::trim(input);
    let (negative, unsigned) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    let (whole, fraction) = match unsigned.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (unsigned, None),
    };
    let fraction_ok = fraction.is_none_or(|c| (1..=places).contains(&c.len()) && js::all_digits(c));
    if !js::all_digits(whole) || !fraction_ok {
        return Err(LedgerError::InvalidAmount(currency));
    }
    // Number(whole) * 10^p + Number(fraction.padEnd(p, "0")). Accumulate
    // exactly and stop as soon as the value leaves the safe range, so long
    // runs of digits cannot overflow.
    let limit = u128::from(MAX_MINOR.unsigned_abs());
    let mut value: u128 = 0;
    for digit in whole.bytes() {
        value = value * 10 + u128::from(digit - b'0');
        if value > limit {
            return Err(LedgerError::AmountTooLarge);
        }
    }
    let digits = fraction.unwrap_or("").as_bytes();
    for place in 0..places {
        let digit = digits.get(place).map_or(0, |d| u128::from(d - b'0'));
        value = value * 10 + digit;
    }
    let value = i64::try_from(value)
        .ok()
        .filter(|v| *v <= MAX_MINOR)
        .ok_or(LedgerError::AmountTooLarge)?;
    Minor::new(if negative { -value } else { value })
}

/// `parseSats`: a whole number of sats, after a JS `trim()`.
pub fn parse_sats(input: &str) -> Result<Minor> {
    let s = js::trim(input);
    let unsigned = s.strip_prefix('-').unwrap_or(s);
    if !js::all_digits(unsigned) {
        return Err(LedgerError::InvalidSats);
    }
    let limit = u128::from(MAX_MINOR.unsigned_abs());
    let mut value: u128 = 0;
    for digit in unsigned.bytes() {
        value = value * 10 + u128::from(digit - b'0');
        if value > limit {
            return Err(LedgerError::AmountTooLarge);
        }
    }
    let value = i64::try_from(value).map_err(|_| LedgerError::AmountTooLarge)?;
    Minor::new(if s.starts_with('-') { -value } else { value })
}

/// `amountInput`: formats minor units in the major unit, to the currency's
/// places (`-?\d+\.\d{p}`).
pub fn amount_input(n: Minor, currency: Currency) -> String {
    let places = currency.precision();
    let unit = 10u64.pow(places);
    let value = n.get();
    let sign = if value < 0 { "-" } else { "" };
    let abs = value.unsigned_abs();
    format!(
        "{sign}{}.{:0width$}",
        abs / unit,
        abs % unit,
        width = places as usize
    )
}
