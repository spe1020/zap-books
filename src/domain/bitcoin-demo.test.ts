// The demo's bitcoin (decision 0005, phase 1 screens) and the export
// columns that carry sats and recorded fiat values.
import { describe, expect, test } from "vitest";
import { demoWorkspace, seedWorkspace } from "./seed";
import { assertBalanced, recordedFiatSpending, totals } from "./ledger";
import { balances } from "./transfers";
import { exportBalances, exportRecords } from "./export";
import { checkWorkspace } from "../adapters/workspace-check";

const all = { from: "", to: "", currency: "" as const };

describe("the demo", () => {
  test("adds a wallet, an exchange and a sats expense to Northstar only", () => {
    const demo = demoWorkspace();
    const [northstar, fieldwork] = demo.businesses;
    const checked = checkWorkspace(structuredClone(demo));
    expect(checked.businesses).toHaveLength(2);
    expect(checked.quarantine ?? []).toEqual([]);
    for (const j of northstar.journals) assertBalanced(northstar, j);
    expect(
      northstar.accounts.filter((a) => a.kind === "wallet").map((a) => a.name),
    ).toEqual(["Lightning wallet"]);
    expect(
      northstar.journals.filter((j) => j.transferType === "exchange"),
    ).toHaveLength(1);
    expect(
      northstar.drafts.filter((d) => d.currency === "BTC").map((d) => d.status),
    ).toEqual(["posted"]);
    expect(fieldwork.accounts.some((a) => a.kind === "wallet")).toBe(false);
    // The parity fixtures' seed stays without bitcoin.
    expect(
      seedWorkspace().businesses.some((b) =>
        b.accounts.some((a) => a.kind === "wallet"),
      ),
    ).toBe(false);
  });

  test("leaves dollar spending as it was, and holds 229,000 sats", () => {
    const demo = demoWorkspace().businesses[0];
    const seed = seedWorkspace().businesses[0];
    for (const period of ["", demo.drafts[0].date.slice(0, 7)]) {
      const withBitcoin = totals(demo, period);
      expect(withBitcoin.USD).toBe(totals(seed, period).USD);
      expect(withBitcoin.BTC).toBe(21000);
      expect(recordedFiatSpending(demo, period)).toEqual({ USD: 1260 });
    }
    const wallet = balances(demo, "9999-12-31", "BTC").find(
      (r) => r.kind === "wallet",
    );
    expect(wallet?.balance).toBe(229000);
  });
});

describe("exports", () => {
  const b = demoWorkspace().businesses[0];

  test("give a bitcoin expense in sats and its fiat value, after the existing columns", () => {
    const out = exportRecords(b, "expenses", all);
    expect(out.headers.slice(0, 16).at(-1)).toBe("Paid personally");
    const sats = out.rows.find((r) => r[7] === "BTC")!;
    expect(sats[6]).toBe("0.00021000");
    expect(sats.slice(16)).toEqual([
      "21000",
      "12.60",
      "USD",
      "60000.000000000000",
      "manual",
      sats[3],
      "",
    ]);
    // Seven blank cells on every other row.
    for (const r of out.rows.filter((r) => r[7] !== "BTC"))
      expect(r.slice(16)).toEqual(Array(7).fill(""));
  });

  test("give each journal line in sats, and its journal's fiat value", () => {
    const out = exportRecords(b, "journal", all);
    expect(out.headers.slice(0, 19).at(-1)).toBe("Corrects journal ID");
    const col = (name: string) => out.headers.indexOf(name);
    const exchange = out.rows.filter((r) => r[15] === "Exchange");
    expect(
      exchange.map((r) => [
        r[col("Currency")],
        r[col("Debit in sats")],
        r[col("Credit in sats")],
        r[col("Fiat value")],
      ]),
    ).toEqual([
      ["BTC", "250000", "0", "150.00"],
      ["USD", "", "", "150.00"],
    ]);
    for (const r of out.rows.filter(
      (r) => r[col("Currency")] !== "BTC" && r[15] !== "Exchange",
    ))
      expect(r.slice(19)).toEqual(Array(8).fill(""));
  });

  test("give a wallet's balance in sats", () => {
    const out = exportBalances(b, "9999-12-31", "BTC");
    expect(out.headers.at(-1)).toBe("Balance in sats");
    expect(out.rows.find((r) => r[4] === "Lightning wallet")!.slice(6)).toEqual(
      ["0.00229000", "BTC", "Held", "229000"],
    );
    expect(
      exportBalances(b, "9999-12-31", "USD").rows.every((r) => r.at(-1) === ""),
    ).toBe(true);
  });
});
