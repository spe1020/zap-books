import { expect, test } from "@playwright/test";
import { readFile } from "node:fs/promises";
import AxeBuilder from "@axe-core/playwright";
import { parseCSV } from "../src/domain/import";
import { parseMoney } from "../src/domain/ledger";
test.beforeEach(async ({ page }) => {
  await page.goto("/");
  await expect(
    page.getByRole("heading", { name: "Your business, at a glance." }),
  ).toBeVisible();
});
test("expense and journal CSV downloads contain accurate, scoped records", async ({
  page,
}) => {
  await page.getByRole("button", { name: "Export", exact: true }).click();
  await expect(page.getByText("6 rows · first 5 shown")).toBeVisible();
  const waiting = page.waitForEvent("download");
  await page.getByRole("button", { name: "Download CSV" }).click();
  const download = await waiting;
  expect(download.suggestedFilename()).toContain("northstar-studio-expenses");
  const rows = parseCSV(await readFile((await download.path())!, "utf8"));
  expect(rows.slice(1).every((r) => r[0] === "northstar")).toBe(true);
  expect(rows.slice(1).reduce((n, r) => n + parseMoney(r[6]), 0)).toBe(461499);
  expect(rows.slice(1).some((r) => r[4] === "Figma")).toBe(false);
  await page
    .getByRole("combobox", { name: "Export format" })
    .selectOption("journal");
  const waitingJournal = page.waitForEvent("download");
  await page.getByRole("button", { name: "Download CSV" }).click();
  const journal = parseCSV(
    await readFile((await (await waitingJournal).path())!, "utf8"),
  );
  expect(journal.some((r) => r[4] === "reversal" && !!r[6])).toBe(true);
  expect(journal.some((r) => r[4] === "transfer")).toBe(true);
  await page.getByRole("button", { name: "Close", exact: true }).click();
  await page.getByLabel("Switch business").selectOption("fieldwork");
  await page.getByRole("button", { name: "Export", exact: true }).click();
  const fieldworkDownload = page.waitForEvent("download");
  await page.getByRole("button", { name: "Download CSV" }).click();
  const otherRows = parseCSV(
    await readFile((await (await fieldworkDownload).path())!, "utf8"),
  );
  expect(otherRows.slice(1).every((r) => r[0] === "fieldwork")).toBe(true);
});
test("mapped expense history becomes a reviewable draft and re-import skips the posted record", async ({
  page,
}) => {
  const [year, month, day] = new Date().toISOString().slice(0, 10).split("-");
  const csv = `Date,Name,Memo/Description,Amount,Transaction Type,Num,Expense Category,Business Purpose\n${month}/${day}/${year},Maple Supply,Workshop materials,42.50,Expense,QB-TEST-100,Office & supplies,Client workshop\n`;
  const openImport = async () => {
    await page.getByRole("button", { name: "Import CSV", exact: true }).click();
    await page
      .getByRole("combobox", { name: "Type of import" })
      .selectOption("expense-history");
    await page
      .getByRole("combobox", { name: "Source system" })
      .selectOption("quickbooks");
    await page
      .getByRole("combobox", { name: "Date format", exact: true })
      .selectOption("mdy");
    await page.getByLabel("Choose CSV file").setInputFiles({
      name: "quickbooks-expenses.csv",
      mimeType: "text/csv",
      buffer: Buffer.from(csv),
    });
  };
  await openImport();
  await expect(page.getByText("1 new · 0 repeated · 0 invalid")).toBeVisible();
  await expect(page.getByText(/New expenses total: \$42.50/)).toBeVisible();
  await page
    .getByRole("button", { name: "Import 1 draft", exact: true })
    .click();
  await page.reload();
  await page.getByRole("button", { name: /Expense inbox/ }).click();
  await page.getByRole("button", { name: "Review Maple Supply" }).click();
  await expect(
    page.getByText("Imported expense history", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("Payment evidence linked", { exact: true }),
  ).toHaveCount(0);
  await expect(page.getByLabel("Amount", { exact: true })).toBeEnabled();
  await expect(
    page.getByRole("combobox", { name: "Category", exact: true }),
  ).toHaveValue("northstar-category-1");
  await page.getByRole("button", { name: "Approve & post" }).click();
  await page.getByRole("button", { name: "Overview", exact: true }).click();
  await expect(page.locator(".principal .stat-value")).toHaveText("$4,657.49");
  await openImport();
  await expect(page.getByText("0 new · 1 repeated · 0 invalid")).toBeVisible();
  await expect(
    page.getByRole("button", { name: /Import.*drafts$/ }),
  ).toBeDisabled();
});
test("export filters, mobile layout and accessibility work", async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.getByRole("button", { name: "Export", exact: true }).click();
  expect(
    await page
      .locator("dialog")
      .evaluate((el) => el.scrollWidth <= el.clientWidth),
  ).toBe(true);
  const scan = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21aa"])
    .analyze();
  expect(
    scan.violations.map((v) => ({
      id: v.id,
      nodes: v.nodes.map((n) => n.failureSummary),
    })),
  ).toEqual([]);
  await page.getByLabel("From date", { exact: true }).fill("1900-01-01");
  await page.getByLabel("To date", { exact: true }).fill("1900-02-01");
  await expect(
    page.getByRole("heading", { name: "No records in this selection" }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Download CSV" }),
  ).toBeDisabled();
  await page.getByLabel("From date", { exact: true }).fill("1900-03-01");
  await expect(page.getByRole("dialog").getByRole("alert")).toContainText(
    "start date",
  );
});
