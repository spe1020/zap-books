import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import type { FiatCurrency, FiatSource, FiatValue } from "../domain/types";
import type { BitcoinUnit } from "../domain/ledger";
import { amountInput, money, parseMoney, parseSats } from "../domain/ledger";
import { impliedRate } from "../domain/bitcoin";

// Bitcoin in the screens (decision 0005, phase 1). Amounts are typed and
// shown in sats unless the viewer picks BTC. A fiat value is always typed
// by a person: nothing here looks up a price.

const UNIT_KEY = "business-finance-btc-unit";

const BitcoinUnitContext = createContext<{
  unit: BitcoinUnit;
  setUnit: (unit: BitcoinUnit) => void;
}>({ unit: "sats", setUnit: () => {} });

/** The viewer's sats or BTC choice, kept in this browser when it can be. */
export function BitcoinUnitProvider({ children }: { children: ReactNode }) {
  const [unit, setUnitState] = useState<BitcoinUnit>(() => {
    try {
      return localStorage.getItem(UNIT_KEY) === "BTC" ? "BTC" : "sats";
    } catch {
      return "sats";
    }
  });
  const setUnit = useCallback((next: BitcoinUnit) => {
    setUnitState(next);
    try {
      localStorage.setItem(UNIT_KEY, next);
    } catch {
      // Private windows may refuse storage; the choice lasts this session.
    }
  }, []);
  const value = useMemo(() => ({ unit, setUnit }), [unit, setUnit]);
  return (
    <BitcoinUnitContext.Provider value={value}>
      {children}
    </BitcoinUnitContext.Provider>
  );
}

export const useBitcoinUnit = () => useContext(BitcoinUnitContext);

/** `money`, with bitcoin in the viewer's unit. */
export function useMoney() {
  const { unit } = useBitcoinUnit();
  return useCallback(
    (value: number, currency: Parameters<typeof money>[1]) =>
      money(value, currency, unit),
    [unit],
  );
}

/** Sats or BTC, as a pair of buttons. */
export function UnitToggle() {
  const { unit, setUnit } = useBitcoinUnit();
  return (
    <div
      className="filter-tabs unit-toggle"
      role="group"
      aria-label="Show bitcoin in"
    >
      {(["sats", "BTC"] as const).map((u) => (
        <button
          key={u}
          type="button"
          aria-pressed={u === unit}
          className={u === unit ? "active" : ""}
          onClick={() => setUnit(u)}
        >
          {u}
        </button>
      ))}
    </div>
  );
}

/** Whole sats from what was typed in `unit`; throws a message if invalid. */
export const parseBitcoin = (text: string, unit: BitcoinUnit) =>
  unit === "sats" ? parseSats(text) : parseMoney(text, "BTC");

/** Sats as they're typed in `unit`. */
export const bitcoinInput = (sats: number, unit: BitcoinUnit) =>
  unit === "sats" ? String(sats) : amountInput(sats, "BTC");

/**
 * Calls `rewrite` with each change of unit, so a form can show what was
 * already typed in the new one.
 */
export function useUnitChange(
  rewrite: (convert: (text: string) => string) => void,
) {
  const { unit } = useBitcoinUnit();
  const last = useRef(unit);
  const latest = useRef(rewrite);
  latest.current = rewrite;
  useEffect(() => {
    const from = last.current;
    if (from === unit) return;
    last.current = unit;
    latest.current((text) => {
      try {
        return text.trim()
          ? bitcoinInput(parseBitcoin(text, from), unit)
          : text;
      } catch {
        return text;
      }
    });
  }, [unit]);
}

/** A bitcoin amount field, labeled with the unit it's typed in. */
export function BitcoinAmount({
  label = "Amount",
  value,
  onChange,
  disabled,
}: {
  label?: string;
  value: string;
  onChange: (text: string) => void;
  disabled?: boolean;
}) {
  const { unit } = useBitcoinUnit();
  return (
    <div className="bitcoin-amount">
      <label>
        {label} ({unit})
        <input
          inputMode={unit === "sats" ? "numeric" : "decimal"}
          value={value}
          disabled={disabled}
          onChange={(e) => onChange(e.target.value)}
          placeholder={unit === "sats" ? "2100" : "0.00002100"}
        />
      </label>
      <UnitToggle />
    </div>
  );
}

const valueNames: Record<FiatCurrency, string> = {
  USD: "Dollar",
  EUR: "Euro",
  GBP: "Pound",
};

/** "Dollar value at time of payment", in the value's own currency. */
export const fiatValueLabel = (currency: FiatCurrency, at = "payment") =>
  `${valueNames[currency]} value at time of ${at}`;

/**
 * A rate, the price of one bitcoin, rounded to `currency`'s minor unit for
 * display. Worked out from the rate's digits, never as a float.
 */
export function rateText(rate: string, currency: FiatCurrency) {
  const [whole, fraction = ""] = rate.split(".");
  const scaled = BigInt(whole + fraction);
  const drop = 10n ** BigInt(Math.max(fraction.length - 2, 0));
  const minor =
    fraction.length >= 2
      ? (scaled * 2n + drop) / (2n * drop)
      : scaled * 10n ** BigInt(2 - fraction.length);
  return `${money(Number(minor), currency)} per BTC`;
}

/**
 * The fiat value for `sats` worth `text` in `currency`, at the rate they
 * imply. Throws a message if `text` isn't an amount above zero; undefined
 * while there are no sats to value.
 */
export function fiatValueFrom(
  sats: number | undefined,
  text: string,
  currency: FiatCurrency,
  source: FiatSource,
  at: string,
): FiatValue | undefined {
  if (!sats || sats <= 0) return undefined;
  const amountMinor = parseMoney(text, currency);
  if (amountMinor <= 0)
    throw new Error(`Enter the ${fiatValueLabel(currency).toLowerCase()}.`);
  return {
    currency,
    amountMinor,
    rate: impliedRate(sats, amountMinor, currency),
    source,
    at,
  };
}

/**
 * The required fiat value of a bitcoin entry, with the rate it implies.
 * The rate recorded is that implied rate, so the value always matches.
 */
export function FiatValueField({
  sats,
  currency,
  value,
  onChange,
  label,
  disabled,
  help = "What it was worth when it happened. Nothing is looked up: you enter it, and it's kept with the entry.",
}: {
  sats: number | undefined;
  currency: FiatCurrency;
  value: string;
  onChange: (text: string) => void;
  label?: string;
  disabled?: boolean;
  help?: string;
}) {
  let rate = "";
  try {
    const v = value.trim()
      ? fiatValueFrom(sats, value, currency, "manual", "")
      : undefined;
    if (v) rate = rateText(v.rate, currency);
  } catch {
    rate = "";
  }
  return (
    <div className="fiat-value">
      <label>
        {label ?? fiatValueLabel(currency)} ({currency})
        <input
          required
          inputMode="decimal"
          value={value}
          disabled={disabled}
          onChange={(e) => onChange(e.target.value)}
          placeholder="0.00"
        />
      </label>
      <small className="small muted field-help" role="status">
        {rate ? `Implied rate: ${rate}.` : help}
      </small>
    </div>
  );
}
