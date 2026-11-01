import { expect, test, type Page } from "@playwright/test";
import { readFile } from "node:fs/promises";
import AxeBuilder from "@axe-core/playwright";
import { parseCSV } from "../src/domain/import";

// Bitcoin in the screens (decision 0005, phase 1). The demo's Northstar has
// a Lightning wallet holding 229,000 sats: 250,000 bought last month for
// $150.00, less a 21,000-sat relay subscription this month, valued at
// $12.60 when it was paid. Every amount here is invented.

test.beforeEach(async ({ page }) => {
  await page.goto("/");
  await expect(
    page.getByRole("heading", { name: "Your business, at a glance." }),
  ).toBeVisible();
});

const toast = (page: Page, text: string) =>
  expect(page.getByRole("status").filter({ hasText: text })).toBeVisible();

const bitcoin = (page: Page) => page.getByRole("region", { name: "Bitcoin" });

async function download(page: Page) {
  const waiting = page.waitForEvent("download");
  await page.getByRole("button", { name: "Download CSV" }).click();
  return parseCSV(await readFile((await (await waiting).path())!, "utf8"));
}

test("the Overview shows bitcoin in sats, apart, with a labeled value at the time", async ({
  page,
}) => {
  const panel = bitcoin(page);
  await expect(
    panel.locator(".setting-row").filter({ hasText: "Lightning wallet" }),
  ).toContainText("229,000 sats");
  await expect(panel.getByLabel("Bitcoin spending this period")).toHaveText(
    "21,000 sats",
  );
  await expect(
    panel.getByRole("heading", { name: "Dollar value at time of transaction" }),
  ).toBeVisible();
  await expect(
    panel.locator(".setting-row").filter({ hasText: "USD" }),
  ).toContainText("$12.60");
  // Dollar spending is unchanged: sats are never added to it.
  await expect(page.locator(".principal .stat-value")).toHaveText("$4,585.59");
  await expect(page.locator(".principal")).toContainText(
    "Also 21,000 sats · reported separately",
  );

  // BTC is a display choice, kept across a reload.
  await panel.getByRole("button", { name: "BTC", exact: true }).click();
  await expect(panel.getByLabel("Bitcoin spending this period")).toHaveText(
    "0.00021000 BTC",
  );
  await page.reload();
  await expect(
    bitcoin(page).getByRole("button", { name: "BTC", exact: true }),
  ).toHaveAttribute("aria-pressed", "true");
  await expect(
    bitcoin(page).locator(".setting-row").filter({ hasText: "Lightning" }),
  ).toContainText("0.00229000 BTC");
  const scan = await new AxeBuilder({ page })
    .include('[aria-labelledby="bitcoin-heading"]')
    .analyze();
  expect(scan.violations).toEqual([]);
});

test("a wallet is set up in Settings, with an opening balance and its dollar value", async ({
  page,
}) => {
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Add account" }).click();
  const add = page.getByRole("dialog", { name: "Add account" });
  await add.getByLabel("Name").fill("Cold storage");
  await add.getByLabel("Type").selectOption({ label: "Bitcoin wallet" });
  await expect(add.getByLabel("Currency")).toHaveValue("BTC");
  await expect(add.getByLabel("Currency")).toBeDisabled();
  await add.getByRole("button", { name: "Save account" }).click();
  await toast(page, "Account saved.");

  await page
    .getByRole("button", { name: "Set opening balance for Cold storage" })
    .click();
  const opening = page.getByRole("dialog", {
    name: "Opening balance · Cold storage",
  });
  await opening.getByLabel("Bitcoin in the wallet (sats)").fill("100000");
  const value = opening.getByLabel("Dollar value at time of opening (USD)");
  await expect(value).toBeVisible();
  await value.fill("61.00");
  await expect(
    opening.getByText("Implied rate: $61,000.00 per BTC."),
  ).toBeVisible();
  const scan = await new AxeBuilder({ page }).include("dialog").analyze();
  expect(scan.violations).toEqual([]);
  await opening.getByRole("button", { name: "Save opening balance" }).click();
  await toast(page, "Opening balance saved.");
  await expect(
    page.locator(".setting-row").filter({ hasText: "Cold storage" }).last(),
  ).toContainText("Bitcoin in the wallet: 100,000 sats");

  await page.getByRole("button", { name: "Overview", exact: true }).click();
  await expect(
    bitcoin(page).locator(".setting-row").filter({ hasText: "Cold storage" }),
  ).toContainText("100,000 sats");
});

test("a sats expense needs its dollar value at time of payment, and shows the implied rate", async ({
  page,
}) => {
  await page.getByRole("button", { name: "New expense", exact: true }).click();
  const expense = page.getByRole("dialog");
  await expense.getByLabel("Vendor", { exact: true }).fill("Tidewater Relay");
  await expense
    .getByLabel("Description", { exact: true })
    .fill("Invented relay add-on");
  await expense.getByLabel("Currency").selectOption("BTC");
  const amount = expense.getByLabel("Amount (sats)");
  await amount.fill("2100");
  // The same amount in BTC, and back.
  await expense.getByRole("button", { name: "BTC", exact: true }).click();
  await expect(expense.getByLabel("Amount (BTC)")).toHaveValue("0.00002100");
  await expense.getByRole("button", { name: "sats", exact: true }).click();
  await expect(amount).toHaveValue("2100");
  await expense
    .getByRole("combobox", { name: "Category", exact: true })
    .selectOption({ label: "Software & subscriptions" });
  await expense
    .getByRole("combobox", { name: "Paid from", exact: true })
    .selectOption({ label: "Lightning wallet · BTC" });
  await expense
    .getByLabel("Business purpose", { exact: true })
    .fill("Relay storage for client notes");

  // Required: the expense isn't posted without it.
  await expense.getByRole("button", { name: "Approve & post" }).click();
  await expect(expense.getByRole("alert")).toContainText(
    "A bitcoin entry needs its fiat value at the time of the transaction.",
  );
  await expense
    .getByLabel("Dollar value at time of payment (USD)")
    .fill("1.26");
  await expect(
    expense.getByText("Implied rate: $60,000.00 per BTC."),
  ).toBeVisible();
  await expense.getByRole("button", { name: "Approve & post" }).click();
  await toast(page, "Expense posted.");
  await expect(expense).not.toBeVisible();

  await expect(
    bitcoin(page).getByLabel("Bitcoin spending this period"),
  ).toHaveText("23,100 sats");
  await expect(
    bitcoin(page).locator(".setting-row").filter({ hasText: "USD" }),
  ).toContainText("$13.86");
  await expect(page.locator(".principal .stat-value")).toHaveText("$4,585.59");
});

test("an exchange buys and sells bitcoin, isn't spending, and is corrected as an exchange", async ({
  page,
}) => {
  await page.getByRole("button", { name: "Transactions", exact: true }).click();
  await page.getByRole("button", { name: "Buy or sell bitcoin" }).click();
  const buy = page.getByRole("dialog", { name: "Buy or sell bitcoin" });
  await buy
    .getByLabel("Paid from")
    .selectOption({ label: "Operating account · USD" });
  await buy.getByLabel("Bitcoin bought (sats)").fill("100000");
  await buy.getByLabel("Amount paid (USD)").fill("61.00");
  await buy.getByLabel("Purpose").fill("Invented top-up for subscriptions");
  await expect(buy.getByRole("status").last()).toHaveText(
    "Operating account pays $61.00 for 100,000 sats into Lightning wallet, at $61,000.00 per BTC. This isn't spending.",
  );
  const scan = await new AxeBuilder({ page }).include("dialog").analyze();
  expect(scan.violations).toEqual([]);
  await buy.getByRole("button", { name: "Record purchase" }).click();
  await toast(page, "Bitcoin purchase recorded.");
  const row = page
    .locator(".transfer-row")
    .filter({ hasText: "Invented top-up for subscriptions" });
  await expect(row).toContainText("100,000 sats · $61.00");

  // Its correction keeps both accounts: here, a sale instead.
  await row.click();
  await page.getByRole("button", { name: "Correct transfer" }).click();
  const fix = page.getByRole("dialog", { name: "Correct exchange" });
  await expect(fix.getByLabel("Wallet")).toBeDisabled();
  await expect(fix.getByLabel("Paid from")).toBeDisabled();
  await expect(fix.getByLabel("Bitcoin bought (sats)")).toHaveValue("100000");
  await fix.getByLabel("Sell bitcoin").check();
  await fix.getByLabel("Bitcoin sold (sats)").fill("40000");
  await fix.getByLabel("Amount received (USD)").fill("25.00");
  await fix
    .getByLabel("Reason for the correction")
    .fill("Invented: it was a sale");
  await fix.getByRole("button", { name: "Post correction" }).click();
  await toast(page, "Exchange corrected.");

  // 229,000 sats less the 40,000 sold; dollar spending unchanged.
  await page.getByRole("button", { name: "Overview", exact: true }).click();
  await expect(
    bitcoin(page).locator(".setting-row").filter({ hasText: "Lightning" }),
  ).toContainText("189,000 sats");
  await expect(page.locator(".principal .stat-value")).toHaveText("$4,585.59");
});

test("exports carry sats and the recorded fiat values after the existing columns", async ({
  page,
}) => {
  await page.getByRole("button", { name: "Export", exact: true }).click();
  await page
    .getByRole("combobox", { name: "Export format" })
    .selectOption("journal");
  await page
    .getByRole("combobox", { name: "Export currency" })
    .selectOption("BTC");
  const journal = await download(page);
  const column = (name: string) => journal[0].indexOf(name);
  expect(journal[0].slice(19)).toEqual([
    "Debit in sats",
    "Credit in sats",
    "Fiat value",
    "Fiat currency",
    "Fiat rate",
    "Fiat source",
    "Fiat value at",
    "Millisats",
  ]);
  // The exchange, one line in each currency, and the relay subscription.
  const exchange = journal.filter(
    (r) => r[column("Transfer type")] === "Exchange",
  );
  const wallet = exchange.find((r) => r[8] === "Lightning wallet")!;
  const bank = exchange.find((r) => r[8] === "Operating account")!;
  expect(wallet[column("Currency")]).toBe("BTC");
  expect(wallet[column("Debit")]).toBe("0.00250000");
  expect(wallet[column("Debit in sats")]).toBe("250000");
  expect(bank[column("Currency")]).toBe("USD");
  expect(bank[column("Credit")]).toBe("150.00");
  expect(bank[column("Credit in sats")]).toBe("");
  expect(wallet.slice(column("Fiat value"), column("Fiat source") + 1)).toEqual(
    ["150.00", "USD", "60000.000000000000", "exchange"],
  );
  const relay = journal.filter((r) => r[4] === "expense");
  expect(relay.map((r) => r[column("Fiat value")])).toEqual(["12.60", "12.60"]);

  await page
    .getByRole("combobox", { name: "Export format" })
    .selectOption("balances");
  await page
    .getByRole("combobox", { name: "Balance currency" })
    .selectOption("BTC");
  const balances = await download(page);
  const held = balances.find((r) => r[4] === "Lightning wallet")!;
  expect(held.slice(6)).toEqual(["0.00229000", "BTC", "Held", "229000"]);
});
