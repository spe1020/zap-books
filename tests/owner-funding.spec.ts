import { expect, test, type Page } from "@playwright/test";
import { readFile } from "node:fs/promises";
import AxeBuilder from "@axe-core/playwright";
import { parseCSV } from "../src/domain/import";

// The demo seed's "Owner's loan" starts at $4,714.70 owed: a $6,000.00 loan,
// a $214.70 expense paid personally, and a $1,500.00 repayment.

test.beforeEach(async ({ page }) => {
  await page.goto("/");
  await expect(
    page.getByRole("heading", { name: "Your business, at a glance." }),
  ).toBeVisible();
});

const toast = (page: Page, text: string) =>
  expect(page.getByRole("status").filter({ hasText: text })).toBeVisible();

function balanceRow(page: Page, account: string) {
  return page
    .getByRole("region", { name: "Balances" })
    .locator(".setting-row")
    .filter({ hasText: account });
}

async function recordTransfer(
  page: Page,
  type: string,
  amount: string,
  purpose: string,
  summary: string,
) {
  await page.getByRole("button", { name: "Transactions", exact: true }).click();
  await page.getByRole("button", { name: "Record transfer" }).click();
  const dialog = page.getByRole("dialog", { name: "Record transfer" });
  await dialog.getByLabel("Transfer type").selectOption({ label: type });
  await dialog.getByLabel("Amount (USD)").fill(amount);
  await dialog.getByLabel("Purpose").fill(purpose);
  await expect(dialog.getByRole("status")).toHaveText(summary);
  await dialog.getByRole("button", { name: "Record transfer" }).click();
  await toast(page, `${type} recorded.`);
  await expect(dialog).not.toBeVisible();
}

test("an owner loan, an expense paid personally and a repayment show in balances and the balances export", async ({
  page,
}) => {
  await expect(balanceRow(page, "Owner's loan")).toContainText("Owed to owner");
  await expect(balanceRow(page, "Owner's loan")).toContainText("$4,714.70");

  // The type preselects the loan and the operating account.
  await recordTransfer(
    page,
    "Owner loan",
    "250.00",
    "Invented bridge funding",
    "The business will owe Owner's loan $250.00 more, and Operating account will hold $250.00 more.",
  );
  await expect(page.locator(".transfer-row").first()).toBeVisible();
  await expect(
    page
      .locator(".transfer-row")
      .filter({ hasText: "Invented bridge funding" }),
  ).toContainText("Owner loan");

  // An expense the owner paid personally.
  await page.getByRole("button", { name: "New expense", exact: true }).click();
  const expense = page.getByRole("dialog");
  await expense.getByLabel("Vendor", { exact: true }).fill("Quill Lane Paper");
  await expense
    .getByLabel("Description", { exact: true })
    .fill("Invented notebooks");
  await expense.getByLabel("Amount", { exact: true }).fill("42.50");
  await expense
    .getByRole("combobox", { name: "Category", exact: true })
    .selectOption({ label: "Office & supplies" });
  await expense
    .getByRole("combobox", { name: "Paid from", exact: true })
    .selectOption({ label: "Paid personally: Owner's loan · USD" });
  // The help describes the field without becoming part of its name.
  await expect(
    expense.getByRole("combobox", { name: "Paid from", exact: true }),
  ).toHaveAccessibleDescription(
    "The owner paid this personally; the business will owe them the amount.",
  );
  await expense
    .getByLabel("Business purpose", { exact: true })
    .fill("Notebooks for client workshops");
  await expense.getByRole("button", { name: "Approve & post" }).click();
  await expect(expense).not.toBeVisible();

  await recordTransfer(
    page,
    "Loan repayment",
    "122.00",
    "Invented partial repayment",
    "The business will owe Owner's loan $122.00 less, paid from Operating account.",
  );

  // 4,714.70 + 250.00 + 42.50 − 122.00
  await page.getByRole("button", { name: "Overview", exact: true }).click();
  await expect(balanceRow(page, "Owner's loan")).toContainText("$4,885.20");
  await page.reload();
  await expect(balanceRow(page, "Owner's loan")).toContainText("$4,885.20");

  await page.getByRole("button", { name: "Export", exact: true }).click();
  await page
    .getByRole("combobox", { name: "Export format" })
    .selectOption("balances");
  const waiting = page.waitForEvent("download");
  await page.getByRole("button", { name: "Download CSV" }).click();
  const download = await waiting;
  expect(download.suggestedFilename()).toMatch(
    /^northstar-studio-balances-\d{4}-\d{2}-\d{2}-USD\.csv$/,
  );
  const rows = parseCSV(await readFile((await download.path())!, "utf8"));
  const loan = rows.find((r) => r[4] === "Owner's loan")!;
  expect(loan.slice(5)).toEqual(["Loan", "4885.20", "USD", "Owed to owner"]);
  expect(rows.slice(1).every((r) => r[0] === "northstar")).toBe(true);
});

test("Record transfer offers only the accounts each type fits", async ({
  page,
}) => {
  await page.getByRole("button", { name: "Transactions", exact: true }).click();
  await page.getByRole("button", { name: "Record transfer" }).click();
  const dialog = page.getByRole("dialog", { name: "Record transfer" });
  const options = (label: string) =>
    expect(
      dialog
        .getByRole("combobox", { name: label, exact: true })
        .getByRole("option"),
    );
  await options("From").toHaveText(["Owner's loan"]);
  await options("To").toHaveText([
    "Operating account",
    "Business card",
    "Reserve account",
  ]);
  await dialog
    .getByLabel("Transfer type")
    .selectOption({ label: "Loan repayment" });
  await options("To").toHaveText(["Owner's loan"]);
  // No owner equity account yet: the form says what to add.
  await dialog
    .getByLabel("Transfer type")
    .selectOption({ label: "Owner contribution" });
  await expect(dialog.getByRole("alert")).toHaveText(
    "Add an owner equity account in USD in Settings first.",
  );
  await expect(
    dialog.getByRole("button", { name: "Record transfer" }),
  ).toBeDisabled();
  const scan = await new AxeBuilder({ page }).include("dialog").analyze();
  expect(scan.violations).toEqual([]);
});

test("Settings adds owner equity with its note, and sets and corrects an opening balance", async ({
  page,
}) => {
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Add account" }).click();
  const add = page.getByRole("dialog");
  await add.getByLabel("Name", { exact: true }).fill("Owner contributions");
  await add.getByLabel("Type").selectOption({ label: "Owner equity" });
  await expect(add.getByText("The owner's stake")).toBeVisible();
  await expect(
    add.getByText(
      "Ask your accountant whether owner money is a loan or a contribution.",
    ),
  ).toBeVisible();
  await add.getByRole("button", { name: "Save account" }).click();
  await toast(page, "Account saved.");
  await expect(page.getByText("Owner equity · USD")).toBeVisible();

  // The reserve account's opening balance, before the first entry.
  await page
    .getByRole("button", { name: "Set opening balance for Reserve account" })
    .click();
  const opening = page.getByRole("dialog");
  await opening.getByLabel("Balance in the account (USD)").fill("900.00");
  await opening.getByRole("button", { name: "Save opening balance" }).click();
  await toast(page, "Opening balance saved.");
  await page
    .getByRole("button", {
      name: "Correct opening balance for Reserve account",
    })
    .click();
  await opening.getByLabel("Balance in the account (USD)").fill("950.00");
  await opening
    .getByLabel("Reason for the correction")
    .fill("Invented statement correction");
  await opening.getByRole("button", { name: "Save opening balance" }).click();
  await toast(page, "Opening balance saved.");
  await expect(
    page.locator(".setting-row").filter({ hasText: "Reserve account" }).last(),
  ).toContainText("Balance in the account: $950.00");
  const scan = await new AxeBuilder({ page }).analyze();
  expect(scan.violations).toEqual([]);
});

test("a transfer is corrected from its detail: reversed, replaced and linked", async ({
  page,
}) => {
  await page.getByRole("button", { name: "Transactions", exact: true }).click();
  await page.getByRole("button", { name: "Transfers", exact: true }).click();
  const loan = page
    .locator(".transfer-row")
    .filter({ hasText: "Working capital from the owner" });
  await loan.click();
  await page.getByRole("button", { name: "Correct transfer" }).click();
  const dialog = page.getByRole("dialog", { name: "Correct transfer" });
  // The form starts from the original.
  await expect(dialog.getByLabel("Transfer type")).toHaveValue("ownerLoan");
  await expect(dialog.getByLabel("Amount (USD)")).toHaveValue("6000.00");
  await expect(dialog.getByLabel("Purpose")).toHaveValue(
    "Working capital from the owner",
  );
  await dialog.getByLabel("Amount (USD)").fill("5520.00");
  await dialog
    .getByLabel("Reason for the correction")
    .fill("Invented: the loan was smaller");
  await dialog.getByRole("button", { name: "Post correction" }).click();
  await toast(page, "Transfer corrected.");
  await expect(dialog).not.toBeVisible();

  // The original stays, marked corrected; the replacement is a new row.
  await expect(loan).toHaveCount(2);
  await expect(loan.filter({ hasText: "Corrected" })).toContainText(
    "$6,000.00",
  );
  await expect(loan.filter({ hasText: "Transfer" })).toContainText("$5,520.00");
  // A corrected transfer can't be corrected again.
  await loan.filter({ hasText: "Corrected" }).click();
  await expect(
    page.getByText("Corrected: a reversal and a replacement were posted."),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Correct transfer" }),
  ).toHaveCount(0);
  await page.keyboard.press("Escape");

  // The seed's $4,714.70 owed, less the difference.
  await page.getByRole("button", { name: "Overview", exact: true }).click();
  await expect(balanceRow(page, "Owner's loan")).toContainText("$4,234.70");
});
