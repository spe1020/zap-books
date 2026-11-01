import type {
  AccountKind,
  Currency,
  FiatSource,
  FiatValue,
  JournalEntry,
} from "./types";
import { fiatCurrencies, precision } from "./types";
import { validDate } from "./ledger";

// Bitcoin in the ledger (decision 0005, phase 1). BTC amounts are whole
// sats. Every BTC journal records its fiat value at the time, as data a
// person entered or confirmed: nothing here looks up a price.

export const fiatSources: FiatSource[] = ["manual", "price-series", "exchange"];

/** At most 12 digits on each side of the point keeps every product exact. */
const RATE = /^\d{1,12}(\.\d{1,12})?$/;
const AT = /^(\d{4}-\d{2}-\d{2})(T(\d{2}):(\d{2})(:(\d{2})(\.\d{3})?)?Z)?$/;
const SATS_PER_BTC_PLACES = 8;

/**
 * A day as `YYYY-MM-DD`, or a UTC time on it (`2027-01-15T14:30Z`, with
 * optional seconds and milliseconds).
 */
function validAt(at: string) {
  const m = AT.exec(at);
  if (!m || !validDate(m[1])) return false;
  if (m[2] === undefined) return true;
  return (
    Number(m[3]) <= 23 &&
    Number(m[4]) <= 59 &&
    (m[6] === undefined || Number(m[6]) <= 59)
  );
}

/**
 * The first thing wrong with a fiat value's own fields, or `undefined`.
 * Whether it fits its entry is `fiatValueError`'s question.
 */
export function fiatValueShape(v: FiatValue): string | undefined {
  if (!fiatCurrencies.includes(v.currency))
    return "A fiat value is in US dollars, euros or pounds.";
  if (!Number.isSafeInteger(v.amountMinor) || v.amountMinor < 0)
    return "A fiat value must be zero or more, in whole minor units.";
  if (typeof v.rate !== "string" || !RATE.test(v.rate) || !/[1-9]/.test(v.rate))
    return "Enter the rate as the price of one bitcoin, such as 65000.00.";
  if (!fiatSources.includes(v.source))
    return "Choose where the fiat value came from.";
  if (typeof v.at !== "string" || !validAt(v.at))
    return "A fiat value needs the date or UTC time it applies to.";
  return undefined;
}

/**
 * Whether `v.amountMinor` is `sats` bitcoin at `v.rate` per bitcoin, within
 * one minor unit of the reporting currency. Worked out exactly in integers:
 * |amount × 10^8 × 10^k − sats × rate × 10^p| ≤ 10^8 × 10^k, with the rate
 * scaled by 10^k to a whole number and p the currency's places.
 */
export function fiatValueMatches(sats: number, v: FiatValue): boolean {
  const [whole, fraction = ""] = v.rate.split(".");
  const scale = 10n ** BigInt(SATS_PER_BTC_PLACES + fraction.length);
  const lhs = BigInt(v.amountMinor) * scale;
  const rhs =
    BigInt(Math.abs(sats)) *
    BigInt(whole + fraction) *
    10n ** BigInt(precision[v.currency]);
  return (lhs > rhs ? lhs - rhs : rhs - lhs) <= scale;
}

/**
 * Why an entry of `sats` in `currency` can't carry `v`, or `undefined`: a
 * BTC entry needs one, valid and matching its rate; no other entry has one.
 */
export function fiatValueError(
  currency: Currency,
  sats: number,
  v: FiatValue | undefined,
): string | undefined {
  if (currency !== "BTC")
    return v === undefined
      ? undefined
      : "Only a bitcoin entry records a fiat value.";
  if (v === undefined)
    return "A bitcoin entry needs its fiat value at the time of the transaction.";
  return (
    fiatValueShape(v) ??
    (fiatValueMatches(sats, v)
      ? undefined
      : "The fiat value must be the sats times the rate, within one minor unit.")
  );
}

/** A fiat value's own fields, and nothing else a caller put beside them. */
export const fiatValueOf = (v: FiatValue): FiatValue => ({
  currency: v.currency,
  amountMinor: v.amountMinor,
  rate: v.rate,
  source: v.source,
  at: v.at,
});

/** Millisats rounded to the nearest sat, half a sat rounding up. */
export function satsFromMillisats(millisats: number): number {
  const rest = millisats % 1000;
  return (millisats - rest) / 1000 + (rest >= 500 ? 1 : 0);
}

/**
 * Why `sats` in `currency` can't come from `millisats`, or `undefined`. A
 * Lightning amount is stored as whole sats, its millisats kept beside it.
 */
export function millisatsError(
  currency: Currency,
  sats: number,
  millisats: number | undefined,
): string | undefined {
  if (millisats === undefined) return undefined;
  if (currency !== "BTC") return "Only a bitcoin amount can be in millisats.";
  if (!Number.isSafeInteger(millisats) || millisats <= 0)
    return "Millisats must be a whole number greater than zero.";
  if (satsFromMillisats(millisats) !== sats)
    return "The amount in sats must be the millisats rounded to the nearest sat.";
  return undefined;
}

/**
 * Why an account of `kind` can't be in `currency`, or `undefined`. Bitcoin
 * is held in wallets, and a wallet holds only bitcoin.
 */
export function accountCurrencyError(
  kind: AccountKind,
  currency: Currency,
): string | undefined {
  if (kind === "wallet" && currency !== "BTC")
    return "A wallet holds bitcoin. Choose BTC.";
  if ((kind === "bank" || kind === "card") && currency === "BTC")
    return "Hold bitcoin in a wallet account.";
  return undefined;
}

/**
 * The currency of one line of `j`: the journal's own, except for the fiat
 * side of an exchange, which is in its fiat value's currency.
 */
export function lineCurrency(
  j: JournalEntry,
  accountKind: AccountKind | undefined,
): Currency {
  return j.transferType === "exchange" &&
    j.fiatValue &&
    accountKind !== "wallet"
    ? j.fiatValue.currency
    : j.currency;
}

/**
 * The price of one bitcoin that `amountMinor` of `currency` for `sats`
 * implies, to 12 decimal places, rounded half up: an exchange's rate. It
 * always passes `fiatValueMatches`.
 */
export function impliedRate(
  sats: number,
  amountMinor: number,
  currency: Currency,
): string {
  const places = 12;
  const numerator =
    BigInt(amountMinor) *
    10n ** BigInt(SATS_PER_BTC_PLACES + places - precision[currency]);
  const denominator = BigInt(sats);
  const scaled = (numerator * 2n + denominator) / (2n * denominator);
  const digits = scaled.toString().padStart(places + 1, "0");
  return `${digits.slice(0, -places)}.${digits.slice(-places)}`;
}
