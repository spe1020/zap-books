// The screens' bitcoin helpers: typing in sats or BTC, the implied rate as
// shown, and the fiat value a form records.
import { describe, expect, test } from "vitest";
import { fiatValueMatches } from "../domain/bitcoin";
import {
  bitcoinInput,
  fiatValueFrom,
  fiatValueLabel,
  parseBitcoin,
  rateText,
} from "./bitcoin";

describe("typing bitcoin", () => {
  test("in sats or BTC reads the same sats, and writes them back", () => {
    for (const sats of [1, 2100, 100_000_000, 2_100_000_000_000_000]) {
      expect(parseBitcoin(bitcoinInput(sats, "sats"), "sats")).toBe(sats);
      expect(parseBitcoin(bitcoinInput(sats, "BTC"), "BTC")).toBe(sats);
    }
    expect(bitcoinInput(2100, "BTC")).toBe("0.00002100");
    expect(() => parseBitcoin("0.5", "sats")).toThrow("whole sats");
    expect(() => parseBitcoin("0.000000001", "BTC")).toThrow(
      "eight decimal places",
    );
  });
});

describe("a fiat value from a form", () => {
  test("records the rate its amounts imply, which always matches", () => {
    const v = fiatValueFrom(2100, "1.26", "USD", "manual", "2027-01-15")!;
    expect(v).toEqual({
      currency: "USD",
      amountMinor: 126,
      rate: "60000.000000000000",
      source: "manual",
      at: "2027-01-15",
    });
    for (const [sats, text] of [
      [1, "0.01"],
      [333, "0.07"],
      [123_456_789, "98765.43"],
    ] as const)
      expect(
        fiatValueMatches(
          sats,
          fiatValueFrom(sats, text, "EUR", "exchange", "2027-01-15")!,
        ),
      ).toBe(true);
  });

  test("needs an amount above zero, and waits for sats", () => {
    expect(() =>
      fiatValueFrom(2100, "0", "USD", "manual", "2027-01-15"),
    ).toThrow("Enter the dollar value at time of payment.");
    expect(() =>
      fiatValueFrom(2100, "abc", "GBP", "manual", "2027-01-15"),
    ).toThrow();
    expect(
      fiatValueFrom(undefined, "1.26", "USD", "manual", "2027-01-15"),
    ).toBe(undefined);
  });

  test("is labeled in its own currency", () => {
    expect(fiatValueLabel("USD")).toBe("Dollar value at time of payment");
    expect(fiatValueLabel("EUR", "transfer")).toBe(
      "Euro value at time of transfer",
    );
  });
});

test("a rate is shown to the cent, rounded from its digits", () => {
  expect(rateText("60000.000000000000", "USD")).toBe("$60,000.00 per BTC");
  expect(rateText("60000", "USD")).toBe("$60,000.00 per BTC");
  expect(rateText("60000.5", "USD")).toBe("$60,000.50 per BTC");
  expect(rateText("61234.564999999999", "EUR")).toBe("€61,234.56 per BTC");
  expect(rateText("61234.565000000000", "GBP")).toBe("£61,234.57 per BTC");
  expect(rateText("999999999999.999999999999", "USD")).toBe(
    "$1,000,000,000,000.00 per BTC",
  );
});
