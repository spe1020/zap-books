// Merge gates for bitcoin in the ledger (decision 0005, phase 1). The Rust
// port has the same tests in crates/ledger-core/tests/bitcoin.rs.
import { describe, expect, test } from "vitest";
import type { BusinessData, Currency, FiatValue, JournalEntry } from "./types";
import { seedWorkspace } from "./seed";
import { updateSettings } from "./settings";
import {
  amountInput,
  approve,
  assertBalanced,
  blankDraft,
  categoryTotals,
  money,
  parseMoney,
  parseSats,
  recordedFiatSpending,
  saveDraft,
  totals,
} from "./ledger";
import {
  balances,
  correctTransfer,
  recordOpeningBalance,
  recordTransfer,
  type TransferInput,
} from "./transfers";
import { fiatValueMatches, impliedRate, satsFromMillisats } from "./bitcoin";

const MAX = Number.MAX_SAFE_INTEGER;

/** A small deterministic generator, so a failure repeats. */
function random(seed: number) {
  let s = seed >>> 0;
  return (n: number) => {
    s = (Math.imul(s ^ (s >>> 15), 0x2c1b3c6d) + 0x9e3779b9) >>> 0;
    s = (s ^ (s >>> 13)) >>> 0;
    return s % n;
  };
}

/** Northstar with no entries, and a Lightning and a cold-storage wallet. */
function setup(): BusinessData {
  let b: BusinessData = {
    ...seedWorkspace().businesses[0],
    drafts: [],
    journals: [],
    approvals: [],
    payments: [],
  };
  for (const name of ["Lightning wallet", "Cold storage"])
    b = updateSettings(b, {
      op: "addAccount",
      name,
      kind: "wallet",
      currency: "BTC",
    });
  return b;
}
const named = (b: BusinessData, name: string) =>
  b.accounts.find((a) => a.name === name)!.id;

/** `sats` at 60,000 a bitcoin, in cents. */
const valued = (sats: number, extra: Partial<FiatValue> = {}): FiatValue => ({
  currency: "USD",
  amountMinor: Math.round((sats * 60000 * 100) / 1e8),
  rate: "60000.00",
  source: "manual",
  at: "2027-01-15",
  ...extra,
});

function expense(
  b: BusinessData,
  currency: Currency,
  amountMinor: number,
  fiatValue?: FiatValue,
) {
  const draft = {
    ...blankDraft(b),
    vendor: "Invented Relay Co",
    description: "Relay subscription",
    purpose: "Client deliverable",
    categoryId: b.categories[0].id,
    date: "2027-01-15",
    currency,
    amountMinor,
    accountId:
      currency === "BTC"
        ? named(b, "Lightning wallet")
        : b.accounts.find((a) => a.kind === "bank" && a.currency === currency)!
            .id,
    ...(fiatValue && { fiatValue }),
  };
  return approve(saveDraft(b, draft), draft.id);
}

function exchange(
  b: BusinessData,
  sats: number,
  cents: number,
  buy = true,
): TransferInput {
  const bank = "northstar-checking";
  const wallet = named(b, "Lightning wallet");
  return {
    date: "2027-01-10",
    amountMinor: sats,
    currency: "BTC",
    fromAccountId: buy ? bank : wallet,
    toAccountId: buy ? wallet : bank,
    transferType: "exchange",
    purpose: "Invented purpose",
    fiatValue: {
      currency: "USD",
      amountMinor: cents,
      rate: impliedRate(sats, cents, "USD"),
      source: "exchange",
      at: "2027-01-10T09:00Z",
    },
  };
}

const balanceOf = (b: BusinessData, currency: Currency, accountId: string) =>
  balances(b, "2027-12-31", currency).find((r) => r.accountId === accountId)
    ?.balance;

describe("sats parse and display exactly", () => {
  test("at every precision, from one to eight decimal places", () => {
    const next = random(5);
    for (let places = 0; places <= 8; places++)
      for (let k = 0; k < 200; k++) {
        const whole = next(21_000_001);
        const fraction = String(next(10 ** places)).padStart(places, "0");
        const text = places ? `${whole}.${fraction}` : String(whole);
        const sats = whole * 1e8 + Number(fraction.padEnd(8, "0"));
        expect(parseMoney(text, "BTC")).toBe(sats);
        expect(parseMoney(amountInput(sats, "BTC"), "BTC")).toBe(sats);
        expect(parseSats(String(sats))).toBe(sats);
      }
    expect(parseMoney("0.00000001", "BTC")).toBe(1);
    expect(() => parseMoney("0.000000001", "BTC")).toThrow(
      "up to eight decimal places",
    );
    expect(amountInput(1, "BTC")).toBe("0.00000001");
    expect(amountInput(-123456789, "BTC")).toBe("-1.23456789");
    expect(money(2100, "BTC")).toBe("2,100 sats");
    expect(money(1, "BTC")).toBe("1 sat");
    expect(money(123456789, "BTC", "BTC")).toBe("1.23456789 BTC");
    expect(money(2_100_000_000_000_000, "BTC", "BTC")).toBe(
      "21,000,000.00000000 BTC",
    );
  });

  test("the largest valid BTC amount fits the range limit", () => {
    const all = 21_000_000 * 1e8;
    expect(Number.isSafeInteger(all)).toBe(true);
    expect(parseMoney("21000000", "BTC")).toBe(all);
    expect(parseMoney("90071992.54740991", "BTC")).toBe(MAX);
    expect(amountInput(MAX, "BTC")).toBe("90071992.54740991");
    expect(() => parseMoney("90071992.54740992", "BTC")).toThrow(
      "Amount is too large.",
    );
    const b = setup();
    const posted = recordTransfer(b, {
      date: "2027-01-10",
      amountMinor: all,
      currency: "BTC",
      fromAccountId: named(b, "Cold storage"),
      toAccountId: named(b, "Lightning wallet"),
      transferType: "betweenAccounts",
      purpose: "Invented purpose",
      fiatValue: valued(all),
    });
    expect(balanceOf(posted, "BTC", named(b, "Lightning wallet"))).toBe(all);
  });
});

describe("a BTC journal records its fiat value", () => {
  test("an expense without one, or with one its rate doesn't give, is refused", () => {
    const b = setup();
    expect(() => expense(b, "BTC", 21000)).toThrow(
      "A bitcoin entry needs its fiat value at the time of the transaction.",
    );
    expect(() =>
      expense(b, "BTC", 21000, valued(21000, { amountMinor: 1262 })),
    ).toThrow(
      "The fiat value must be the sats times the rate, within one minor unit.",
    );
    const posted = expense(b, "BTC", 21000, valued(21000));
    const journal = posted.journals.at(-1)!;
    expect(journal.fiatValue).toEqual(valued(21000));
    const { fiatValue: _, ...bare } = journal;
    expect(() => assertBalanced(posted, bare as JournalEntry)).toThrow(
      "Invalid journal fiat value.",
    );
    expect(() =>
      assertBalanced(posted, {
        ...journal,
        fiatValue: valued(21000, { amountMinor: 9 }),
      }),
    ).toThrow("Invalid journal fiat value.");
  });

  test("transfers and opening balances need one too, and dollars refuse one", () => {
    const b = setup();
    const cold = named(b, "Cold storage");
    expect(() =>
      recordOpeningBalance(b, {
        accountId: cold,
        date: "2027-01-01",
        amountMinor: 100000,
      }),
    ).toThrow("needs its fiat value");
    const opened = recordOpeningBalance(b, {
      accountId: cold,
      date: "2027-01-01",
      amountMinor: 100000,
      fiatValue: valued(100000),
    });
    expect(balanceOf(opened, "BTC", cold)).toBe(100000);
    expect(() => expense(b, "USD", 4200, valued(4200))).toThrow(
      "Only a bitcoin entry records a fiat value.",
    );
  });

  test("within one minor unit means at most one cent either way", () => {
    for (const [cents, ok] of [
      [6000, true],
      [6001, true],
      [5999, true],
      [6002, false],
      [5998, false],
    ] as const)
      expect(
        fiatValueMatches(100000, valued(100000, { amountMinor: cents })),
      ).toBe(ok);
  });
});

describe("exchanges", () => {
  test("balance on both sides and never count as spending", () => {
    const b = setup();
    const bank = "northstar-checking";
    const wallet = named(b, "Lightning wallet");
    const bought = recordTransfer(b, exchange(b, 250000, 15000));
    expect(balanceOf(bought, "USD", bank)).toBe(-15000);
    expect(balanceOf(bought, "BTC", wallet)).toBe(250000);
    const sold = recordTransfer(bought, exchange(b, 50000, 3100, false));
    expect(balanceOf(sold, "USD", bank)).toBe(-11900);
    expect(balanceOf(sold, "BTC", wallet)).toBe(200000);
    for (const period of ["", "2027-01"]) {
      expect(totals(sold, period)).toEqual({});
      expect(recordedFiatSpending(sold, period)).toEqual({});
      expect(categoryTotals(sold, period, "BTC")).toEqual([]);
      expect(categoryTotals(sold, period, "USD")).toEqual([]);
    }
    // Corrected like any transfer: reversed exactly, both sides.
    const original = sold.journals.find((j) => j.transferType === "exchange")!;
    const fixed = correctTransfer(
      sold,
      original.id,
      exchange(b, 200000, 12000),
      "Invented reason",
    );
    expect(balanceOf(fixed, "USD", bank)).toBe(-8900);
    expect(balanceOf(fixed, "BTC", wallet)).toBe(150000);
  });

  test("the implied rate always gives back its amount", () => {
    const next = random(9);
    for (let k = 0; k < 500; k++) {
      // Any price up to 10^11 a bitcoin: the rate keeps 12 integer digits.
      const sats = 1000 + next(2_000_000_000);
      const cents = 1 + next(100_000_000);
      const value = exchange(setup(), sats, cents).fiatValue!;
      expect(fiatValueMatches(sats, value)).toBe(true);
    }
  });
});

test("millisats round to the nearest sat, and are kept", () => {
  expect(
    [1, 499, 500, 999, 1000, 1499, 1500, 2500].map(satsFromMillisats),
  ).toEqual([0, 0, 1, 1, 1, 1, 2, 3]);
  expect(satsFromMillisats(MAX)).toBe(9007199254741);
  const b = setup();
  const input = {
    date: "2027-01-10",
    amountMinor: 100000,
    currency: "BTC" as const,
    fromAccountId: named(b, "Cold storage"),
    toAccountId: named(b, "Lightning wallet"),
    transferType: "betweenAccounts" as const,
    purpose: "Invented purpose",
    fiatValue: valued(100000),
  };
  const posted = recordTransfer(b, { ...input, millisats: 100000499 });
  expect(posted.journals.at(-1)!.millisats).toBe(100000499);
  expect(() => recordTransfer(b, { ...input, millisats: 100000500 })).toThrow(
    "rounded to the nearest sat",
  );
});

describe("existing currencies are unchanged", () => {
  // The two-decimal functions as they were before per-currency precision.
  const legacyParse = (input: string) => {
    const s = input.trim();
    if (!/^-?\d+(\.\d{1,2})?$/.test(s))
      throw new Error(
        "Use an amount with up to two decimal places, such as 42.50.",
      );
    const negative = s.startsWith("-");
    const [whole, cents = ""] = s.replace("-", "").split(".");
    const value = Number(whole) * 100 + Number(cents.padEnd(2, "0"));
    if (!Number.isSafeInteger(value)) throw new Error("Amount is too large.");
    return negative ? -value : value;
  };
  const legacyInput = (n: number) =>
    `${n < 0 ? "-" : ""}${Math.floor(Math.abs(n) / 100)}.${String(Math.abs(n) % 100).padStart(2, "0")}`;
  const outcome = (f: () => unknown) => {
    try {
      return { value: f() };
    } catch (e) {
      return { error: (e as Error).message };
    }
  };

  test("parsing, amount input and display", () => {
    const next = random(3);
    const inputs = ["", "1.001", "1e4", "-0", "00012.30", "90071992547409.92"];
    for (let k = 0; k < 2000; k++) {
      const n = next(2) ? next(1e9) : next(MAX % 1e9) * 1e6 + next(1e6);
      inputs.push(legacyInput(n), `-${n}`, `${n}.${next(10)}`);
    }
    for (const currency of ["USD", "EUR", "GBP"] as const) {
      for (const input of inputs)
        expect(outcome(() => parseMoney(input, currency))).toEqual(
          outcome(() => legacyParse(input)),
        );
      for (const n of [0, 1, -1, 99, 100, -405, 123456, MAX, -MAX])
        expect(amountInput(n, currency)).toBe(legacyInput(n));
      expect(money(123456, currency)).toBe(
        new Intl.NumberFormat("en-US", { style: "currency", currency }).format(
          1234.56,
        ),
      );
    }
  });
});

test("no report ever adds sats to dollars", () => {
  const next = random(17);
  for (let run = 0; run < 40; run++) {
    let b = setup();
    const wallet = named(b, "Lightning wallet");
    const bank = "northstar-checking";
    let usdSpent = 0,
      satsSpent = 0,
      valueSpent = 0,
      usdHeld = 0,
      satsHeld = 0;
    for (let step = 0; step < 8; step++) {
      const kind = next(3);
      if (kind === 0) {
        const cents = 1 + next(500000);
        b = expense(b, "USD", cents);
        usdSpent += cents;
        usdHeld -= cents;
      } else if (kind === 1) {
        const sats = 1 + next(5_000_000);
        const value = valued(sats);
        b = expense(b, "BTC", sats, value);
        satsSpent += sats;
        valueSpent += value.amountMinor;
        satsHeld -= sats;
      } else {
        const sats = 1 + next(5_000_000);
        const cents = 1 + next(500000);
        b = recordTransfer(b, exchange(b, sats, cents));
        usdHeld -= cents;
        satsHeld += sats;
      }
    }
    const spent = totals(b, "");
    expect(spent.USD ?? 0).toBe(usdSpent);
    expect(spent.BTC ?? 0).toBe(satsSpent);
    expect(recordedFiatSpending(b, "")).toEqual(
      valueSpent ? { USD: valueSpent } : {},
    );
    expect(balanceOf(b, "USD", bank)).toBe(usdHeld);
    expect(balanceOf(b, "BTC", wallet)).toBe(satsHeld);
    // A dollar report lists only dollar accounts, and a sats one only BTC.
    expect(
      balances(b, "2027-12-31", "USD").every(
        (r) => b.accounts.find((a) => a.id === r.accountId)!.currency === "USD",
      ),
    ).toBe(true);
    expect(
      balances(b, "2027-12-31", "BTC").every(
        (r) => b.accounts.find((a) => a.id === r.accountId)!.currency === "BTC",
      ),
    ).toBe(true);
    const sum = (rows: { amount: number }[]) =>
      rows.reduce((n, r) => n + r.amount, 0);
    expect(sum(categoryTotals(b, "", "USD"))).toBe(usdSpent);
    expect(sum(categoryTotals(b, "", "BTC"))).toBe(satsSpent);
  }
});
