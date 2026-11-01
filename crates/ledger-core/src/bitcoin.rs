//! Bitcoin in the ledger (`src/domain/bitcoin.ts`, decision 0005 phase 1).
//!
//! BTC amounts are whole sats. Every BTC journal records its fiat value at
//! the time, as data a person entered or confirmed: nothing here looks up a
//! price.

use crate::date::valid_date;
use crate::money::{Currency, Minor};
use crate::types::{AccountKind, FiatValue, TransferType, UncheckedJournal};

/// Decimal places of a sat in a bitcoin.
const SATS_PER_BTC_PLACES: u32 = 8;
/// At most this many digits on each side of a rate's point.
const RATE_DIGITS: usize = 12;

/// `RATE`: `\d{1,12}(\.\d{1,12})?`, returning the two parts.
fn rate_parts(rate: &str) -> Option<(&str, &str)> {
    let (whole, fraction) = match rate.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (rate, None),
    };
    let digits =
        |s: &str| (1..=RATE_DIGITS).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(whole) || !fraction.is_none_or(digits) {
        return None;
    }
    Some((whole, fraction.unwrap_or("")))
}

/// Two ASCII digits as a number.
fn two_digits(s: &str) -> Option<u32> {
    match s.as_bytes() {
        [a, b] if a.is_ascii_digit() && b.is_ascii_digit() => {
            Some(u32::from(a - b'0') * 10 + u32::from(b - b'0'))
        }
        _ => None,
    }
}

/// `validAt`: a day as `YYYY-MM-DD`, or a UTC time on it (`THH:MMZ`, with
/// optional `:SS` and `.sss`).
fn valid_at(at: &str) -> bool {
    let (day, time) = match at.split_once('T') {
        Some((day, time)) => (day, Some(time)),
        None => (at, None),
    };
    // `valid_date` requires the `\d{4}-\d{2}-\d{2}` shape itself.
    if !valid_date(day) {
        return false;
    }
    let Some(time) = time else { return true };
    let Some(time) = time.strip_suffix('Z') else {
        return false;
    };
    let (clock, millis) = match time.split_once('.') {
        Some((clock, millis)) => (clock, Some(millis)),
        None => (time, None),
    };
    let parts: Vec<&str> = clock.split(':').collect();
    let millis_ok = millis.is_none_or(|m| m.len() == 3 && m.bytes().all(|b| b.is_ascii_digit()));
    match parts.as_slice() {
        [h, m] if millis.is_none() => {
            two_digits(h).is_some_and(|h| h <= 23) && two_digits(m).is_some_and(|m| m <= 59)
        }
        [h, m, sec] => {
            millis_ok
                && two_digits(h).is_some_and(|h| h <= 23)
                && two_digits(m).is_some_and(|m| m <= 59)
                && two_digits(sec).is_some_and(|s| s <= 59)
        }
        _ => false,
    }
}

/// `fiatValueShape`: the first thing wrong with a fiat value's own fields.
/// An unknown source, or an amount that isn't a safe integer, doesn't
/// deserialize (PARITY_NOTES D2).
pub fn fiat_value_shape(v: &FiatValue) -> Option<&'static str> {
    if !v.currency.is_fiat() {
        return Some("A fiat value is in US dollars, euros or pounds.");
    }
    if v.amount_minor < Minor::ZERO {
        return Some("A fiat value must be zero or more, in whole minor units.");
    }
    if rate_parts(&v.rate).is_none() || !v.rate.bytes().any(|b| (b'1'..=b'9').contains(&b)) {
        return Some("Enter the rate as the price of one bitcoin, such as 65000.00.");
    }
    if !valid_at(&v.at) {
        return Some("A fiat value needs the date or UTC time it applies to.");
    }
    None
}

/// `fiatValueMatches`: whether `v.amount_minor` is `sats` bitcoin at
/// `v.rate`, within one minor unit, worked out exactly in integers. A
/// product too large for a `u128` is far beyond any safe amount, so it
/// doesn't match, as in the TypeScript's `BigInt`.
pub fn fiat_value_matches(sats: Minor, v: &FiatValue) -> bool {
    let Some((whole, fraction)) = rate_parts(&v.rate) else {
        return false;
    };
    let digits = format!("{whole}{fraction}");
    let (Ok(rate), Ok(amount)) = (digits.parse::<u128>(), u128::try_from(v.amount_minor.get()))
    else {
        return false;
    };
    let scale = 10u128.pow(SATS_PER_BTC_PLACES + fraction.len() as u32);
    let sats = u128::from(sats.get().unsigned_abs());
    let lhs = amount.checked_mul(scale);
    let rhs = sats
        .checked_mul(rate)
        .and_then(|x| x.checked_mul(10u128.pow(v.currency.precision())));
    match (lhs, rhs) {
        (Some(lhs), Some(rhs)) => lhs.abs_diff(rhs) <= scale,
        _ => false,
    }
}

/// `fiatValueError`: why an entry of `sats` in `currency` can't carry `v`.
/// A BTC entry needs one, valid and matching its rate; no other has one.
pub fn fiat_value_error(
    currency: Currency,
    sats: Minor,
    v: Option<&FiatValue>,
) -> Option<&'static str> {
    if currency != Currency::Btc {
        return v.map(|_| "Only a bitcoin entry records a fiat value.");
    }
    let Some(v) = v else {
        return Some("A bitcoin entry needs its fiat value at the time of the transaction.");
    };
    fiat_value_shape(v).or_else(|| {
        (!fiat_value_matches(sats, v))
            .then_some("The fiat value must be the sats times the rate, within one minor unit.")
    })
}

/// `satsFromMillisats`: millisats rounded to the nearest sat, half a sat
/// rounding up. Negative millisats are refused before this is used.
pub fn sats_from_millisats(millisats: Minor) -> Minor {
    let m = millisats.get();
    let rest = m % 1000;
    // Within the safe range: the result is a thousandth of a safe value.
    Minor::new(m / 1000 + i64::from(rest >= 500)).unwrap_or(Minor::ZERO)
}

/// `millisatsError`: why `sats` in `currency` can't come from `millisats`.
pub fn millisats_error(
    currency: Currency,
    sats: Minor,
    millisats: Option<Minor>,
) -> Option<&'static str> {
    let millisats = millisats?;
    if currency != Currency::Btc {
        return Some("Only a bitcoin amount can be in millisats.");
    }
    if millisats <= Minor::ZERO {
        return Some("Millisats must be a whole number greater than zero.");
    }
    (sats_from_millisats(millisats) != sats)
        .then_some("The amount in sats must be the millisats rounded to the nearest sat.")
}

/// `accountCurrencyError`: bitcoin is held in wallets, and a wallet holds
/// only bitcoin.
pub fn account_currency_error(kind: AccountKind, currency: Currency) -> Option<&'static str> {
    match kind {
        AccountKind::Wallet if currency != Currency::Btc => {
            Some("A wallet holds bitcoin. Choose BTC.")
        }
        AccountKind::Bank | AccountKind::Card if currency == Currency::Btc => {
            Some("Hold bitcoin in a wallet account.")
        }
        _ => None,
    }
}

/// `lineCurrency`: a line's currency, the journal's own except for the fiat
/// side of an exchange.
pub(crate) fn line_currency(j: &UncheckedJournal, kind: Option<AccountKind>) -> Currency {
    match &j.fiat_value {
        Some(v)
            if j.transfer_type == Some(TransferType::Exchange)
                && kind != Some(AccountKind::Wallet) =>
        {
            v.currency
        }
        _ => j.currency,
    }
}
