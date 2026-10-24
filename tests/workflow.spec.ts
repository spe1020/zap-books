import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
const png = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aOCcAAAAASUVORK5CYII=",
  "base64",
);
test.beforeEach(async ({ page }) => {
  await page.goto("/");
  await expect(
    page.getByRole("heading", { name: "Your business, at a glance." }),
  ).toBeVisible();
});
test("create, preview, post once, reload, and keep businesses isolated", async ({
  page,
}) => {
  await page.getByRole("button", { name: "New expense", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("Vendor", { exact: true }).fill("Paper Crane");
  await dialog
    .getByLabel("Description", { exact: true })
    .fill("Client workshop materials");
  await dialog.getByLabel("Amount", { exact: true }).fill("42.50");
  await dialog
    .getByRole("combobox", { name: "Category", exact: true })
    .selectOption({ label: "Office & supplies" });
  await dialog
    .getByLabel("Business purpose", { exact: true })
    .fill("Materials for the team client workshop");
  await dialog
    .getByLabel("Attach receipt", { exact: true })
    .setInputFiles({ name: "receipt.png", mimeType: "image/png", buffer: png });
  await expect(
    dialog.getByRole("img", { name: "Receipt evidence: receipt.png" }),
  ).toBeVisible();
  await expect
    .poll(() =>
      dialog
        .getByRole("img", { name: "Receipt evidence: receipt.png" })
        .evaluate((img: HTMLImageElement) => img.naturalWidth),
    )
    .toBeGreaterThan(0);
  await dialog.getByRole("button", { name: "Approve & post" }).click();
  await expect(dialog).not.toBeVisible();
  await expect(page.locator(".principal .stat-value")).toHaveText("$4,657.49");
  await page.reload();
  await expect(page.locator(".principal .stat-value")).toHaveText("$4,657.49");
  await page.getByRole("button", { name: "Transactions", exact: true }).click();
  await page.getByLabel("Search transactions").fill("Paper Crane");
  await expect(page.locator("tbody tr")).toHaveCount(1);
  await page
    .getByRole("button", { name: "View Paper Crane", exact: true })
    .click();
  await expect(
    page.getByRole("img", { name: "Receipt evidence: receipt.png" }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Approve & post" }),
  ).toHaveCount(0);
  await page.getByRole("button", { name: "Done", exact: true }).click();
  await page.getByLabel("Switch business").selectOption("fieldwork");
  await page.getByLabel("Search transactions").fill("Paper Crane");
  await expect(
    page.getByRole("heading", { name: "No matching records" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Ask assistant" }).click();
  await page.getByRole("button", { name: "What needs review?" }).click();
  await expect(page.locator(".assistant-answer")).toContainText(
    "Fieldwork Landscapes",
  );
});
test("CSV mapping, invalid rows, preview, repeated imports, and matching receipt", async ({
  page,
}) => {
  const csv =
    "date,merchant,amount,currency,reference\n2026-10-23,Workshop Tools,-55.25,USD,TEST-REF-1\n2026-10-23,Supply Shop,-55.25,USD,TEST-REF-2";
  const importCSV = async () => {
    await page.getByRole("button", { name: "Import CSV", exact: true }).click();
    await page.getByLabel("Choose CSV file").setInputFiles({
      name: "payments.csv",
      mimeType: "text/csv",
      buffer: Buffer.from(csv),
    });
  };
  await importCSV();
  await expect(page.getByText("2 new · 0 repeated · 0 invalid")).toBeVisible();
  await page.getByRole("button", { name: "Import 2 drafts" }).click();
  await importCSV();
  await expect(page.getByText("0 new · 2 repeated · 0 invalid")).toBeVisible();
  await expect(
    page.getByRole("button", { name: /Import.*drafts$/ }),
  ).toBeDisabled();
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  await page.getByRole("button", { name: /Expense inbox/ }).click();
  await page.getByLabel("Search inbox").fill("Workshop Tools");
  await page.getByRole("button", { name: "Review Workshop Tools" }).click();
  await expect(page.getByText("Payment evidence linked")).toBeVisible();
  await page
    .getByLabel("Attach receipt", { exact: true })
    .setInputFiles("tests/fixtures/receipt.pdf");
  await expect(page.locator('object[type="application/pdf"]')).toBeVisible();
  await expect(page.getByLabel("Amount", { exact: true })).toBeDisabled();
  await page.getByRole("button", { name: "Save draft" }).click();
  await expect(page.locator("tbody tr")).toHaveCount(1);
  await page.reload();
  await page.getByRole("button", { name: /Expense inbox/ }).click();
  await page.getByLabel("Search inbox").fill("Workshop Tools");
  await page.getByRole("button", { name: "Review Workshop Tools" }).click();
  await expect(page.locator("object")).toBeVisible();
});
test("correction preserves original and changes report by the difference", async ({
  page,
}) => {
  await page.getByRole("button", { name: "Transactions", exact: true }).click();
  await page.getByRole("button", { name: "View Adobe", exact: true }).click();
  await page.getByRole("button", { name: "Correct expense" }).click();
  await page.getByLabel("Amount", { exact: true }).fill("70.00");
  await page
    .getByLabel("Reason for correction")
    .fill("Invoice correction, amount reduced");
  await page.getByRole("button", { name: "Post correction" }).click();
  await page.getByRole("button", { name: "Corrections", exact: true }).click();
  await page.getByLabel("Search transactions").fill("Adobe");
  await expect(page.locator("tbody tr")).toHaveCount(2);
  await page
    .locator("tbody tr")
    .filter({ hasText: "Corrected" })
    .getByRole("button", { name: "View Adobe", exact: true })
    .click();
  await expect(
    page.getByText("Invoice correction, amount reduced", { exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "View replacement expense" }).click();
  await expect(
    page.getByRole("button", { name: "View original expense" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Done", exact: true }).click();
  await page.getByRole("button", { name: "Overview", exact: true }).click();
  await expect(page.locator(".principal .stat-value")).toHaveText("$4,595.00");
});
test("validation, attachment errors, explicit rules, and empty report", async ({
  page,
}) => {
  await page.getByRole("button", { name: "New expense", exact: true }).click();
  await page.getByRole("button", { name: "Approve & post" }).click();
  await expect(page.getByRole("dialog").getByRole("alert")).toContainText(
    "Vendor is required",
  );
  await page.getByLabel("Attach receipt", { exact: true }).setInputFiles({
    name: "bad.png",
    mimeType: "image/png",
    buffer: Buffer.from("not an image"),
  });
  await expect(page.getByRole("dialog").getByRole("alert")).toContainText(
    "does not match",
  );
  await page.getByRole("button", { name: "Close dialog" }).click();
  await page.getByRole("button", { name: /Expense inbox/ }).click();
  await page.getByRole("button", { name: "Review Figma", exact: true }).click();
  await page.getByRole("button", { name: "Apply suggestion" }).click();
  await page
    .getByLabel("Save this vendor classification as a visible rule")
    .check();
  await page.getByRole("button", { name: "Approve & post" }).click();
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await expect(
    page.getByText("Vendor contains “Figma”", { exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Edit", exact: true }).click();
  await page.getByLabel("Rule active").uncheck();
  await page.getByRole("button", { name: "Save rule", exact: true }).click();
  await expect(page.getByText("Paused", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Overview", exact: true }).click();
  await page.getByLabel("Reporting period").fill("1900-01");
  await expect(
    page.getByRole("heading", { name: "A fresh start" }),
  ).toBeVisible();
  await expect(page.locator(".principal .stat-value")).toHaveText("$0.00");
});
test("desktop screenshot, keyboard dialog, and accessibility", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 1050 });
  await page.screenshot({
    path: "test-results/overview-desktop.png",
    fullPage: true,
    animations: "disabled",
  });
  const scan = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21aa"])
    .analyze();
  expect(
    scan.violations.map((v) => ({
      id: v.id,
      nodes: v.nodes.map((n) => ({
        target: n.target,
        summary: n.failureSummary,
      })),
    })),
  ).toEqual([]);
  await page.getByRole("button", { name: "New expense", exact: true }).focus();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("dialog")).toBeVisible();
  for (let i = 0; i < 25; i++) {
    await page.keyboard.press("Tab");
    expect(
      await page.evaluate(() => !!document.activeElement?.closest("dialog")),
    ).toBe(true);
  }
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "New expense", exact: true }),
  ).toBeFocused();
});
test("mobile shell and review have no page overflow and preserve navigation", async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({
    path: "test-results/overview-mobile.png",
    fullPage: true,
    animations: "disabled",
  });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.getByRole("button", { name: "Open navigation" }).click();
  await page.getByLabel("Switch business").selectOption("fieldwork");
  await page.getByRole("button", { name: "New expense", exact: true }).click();
  await expect(page.getByRole("dialog")).toBeVisible();
  expect(
    await page
      .locator("dialog")
      .evaluate((el) => el.scrollWidth <= el.clientWidth),
  ).toBe(true);
  await page.screenshot({
    path: "test-results/review-mobile.png",
    fullPage: true,
    animations: "disabled",
  });
  await page.getByRole("button", { name: "Close dialog" }).click();
});
test("reset is deliberate and restores both businesses", async ({ page }) => {
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByLabel("Business name").fill("Edited studio");
  await page.getByRole("button", { name: "Save profile" }).click();
  await page.getByRole("button", { name: "Reset demo", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Reset both businesses" }),
  ).toBeDisabled();
  await page.getByLabel("Type RESET to continue").fill("RESET");
  await page.getByRole("button", { name: "Reset both businesses" }).click();
  await expect(page.getByLabel("Switch business")).toHaveValue("northstar");
  await expect(
    page.getByLabel("Switch business").locator("option:checked"),
  ).toHaveText("Northstar Studio");
});

test("stale tabs cannot overwrite newer records or post an expense twice", async ({
  page,
  context,
}) => {
  const second = await context.newPage();
  await second.goto("/");
  await second.getByRole("button", { name: /Expense inbox/ }).click();
  await second
    .getByRole("button", { name: "Review Figma", exact: true })
    .click();
  await page.getByRole("button", { name: /Expense inbox/ }).click();
  await page.getByRole("button", { name: "Review Figma", exact: true }).click();
  await page.getByRole("button", { name: "Approve & post" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await second.getByRole("button", { name: "Approve & post" }).click();
  await expect(second.getByRole("dialog").getByRole("alert")).toContainText(
    "Reload",
  );
  await second.reload();
  await expect(second.locator(".principal .stat-value")).toHaveText(
    "$4,710.99",
  );
  await second
    .getByRole("button", { name: "Transactions", exact: true })
    .click();
  await second.getByLabel("Search transactions").fill("Figma");
  await expect(second.locator("tbody tr")).toHaveCount(1);
});

test("inbox, review, transactions, settings, and import pass accessibility checks", async ({
  page,
}) => {
  const audit = async () => {
    const result = await new AxeBuilder({ page })
      .withTags(["wcag2a", "wcag2aa", "wcag21aa"])
      .analyze();
    expect(
      result.violations.map((v) => ({
        id: v.id,
        nodes: v.nodes.map((n) => ({
          target: n.target,
          summary: n.failureSummary,
        })),
      })),
    ).toEqual([]);
  };
  await page.getByRole("button", { name: /Expense inbox/ }).click();
  await audit();
  await page.getByRole("button", { name: "Review Figma", exact: true }).click();
  await audit();
  await expect
    .poll(() =>
      page
        .getByRole("img", { name: "Receipt evidence: figma-receipt.svg" })
        .evaluate((img: HTMLImageElement) => img.naturalWidth),
    )
    .toBe(600);
  await page.screenshot({
    path: "test-results/review-desktop.png",
    fullPage: true,
    animations: "disabled",
  });
  await page.getByRole("button", { name: "Close dialog" }).click();
  await page.getByRole("button", { name: "Transactions", exact: true }).click();
  await audit();
  await page.getByRole("button", { name: "Import CSV", exact: true }).click();
  await audit();
  await page.getByRole("button", { name: "Close dialog" }).click();
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await audit();
});

test("bank-linked corrections explain locked amounts and preserve payment evidence", async ({
  page,
}) => {
  await page.getByRole("button", { name: /Expense inbox/ }).click();
  await page
    .getByRole("button", { name: "Review Notion", exact: true })
    .click();
  await page.getByRole("button", { name: "Approve & post" }).click();
  await page.getByRole("button", { name: "Transactions", exact: true }).click();
  await page.getByRole("button", { name: "View Notion", exact: true }).click();
  await page.getByRole("button", { name: "Correct expense" }).click();
  await expect(page.getByLabel("Amount", { exact: true })).toBeDisabled();
  await expect(
    page.getByLabel("Amount", { exact: true }),
  ).toHaveAccessibleDescription(
    /Amount comes from the linked bank\/card payment/,
  );
  await expect(page.getByLabel("Date", { exact: true })).toBeDisabled();
  await expect(
    page.getByRole("combobox", { name: "Currency", exact: true }),
  ).toBeDisabled();
  await expect(
    page.getByRole("combobox", { name: "Payment account", exact: true }),
  ).toBeDisabled();
  await page
    .getByLabel("Description", { exact: true })
    .fill("Corrected project description");
  await page
    .getByLabel("Reason for correction")
    .fill("Clarify the project this expense supported");
  const scan = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21aa"])
    .analyze();
  expect(
    scan.violations.map((v) => ({
      id: v.id,
      targets: v.nodes.map((n) => n.target),
    })),
  ).toEqual([]);
  await page.getByRole("button", { name: "Post correction" }).click();
  await page.getByLabel("Search transactions").fill("Notion");
  await expect(page.locator("tbody tr")).toHaveCount(2);
  await page
    .locator("tbody tr")
    .filter({ hasText: "Posted" })
    .getByRole("button", { name: "View Notion", exact: true })
    .click();
  await expect(page.locator(".payment-evidence")).toContainText("$180.00");
  await expect(page.getByLabel("Description", { exact: true })).toHaveValue(
    "Corrected project description",
  );
});
