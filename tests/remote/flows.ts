/** Steps the remote tests share. Invented business data only. */
import { expect, type Page } from "@playwright/test";

/** Signs in with the injected test signer. */
export async function signIn(page: Page) {
  await page.goto("/");
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
}

/**
 * Creates a business with an offline backup key, and returns the backup
 * key as it was shown.
 */
export async function createBusiness(page: Page, name: string) {
  await expect(
    page.getByRole("heading", { name: "Create a business" }),
  ).toBeVisible();
  await page.getByLabel("Business name").fill(name);
  await page.getByLabel("Industry").fill("Pottery");
  await page.getByRole("button", { name: "Generate a backup key" }).click();
  const backup = (
    await page.getByLabel("Backup key", { exact: true }).innerText()
  ).replace(/\s+/g, "");
  expect(backup).toMatch(/^nsec1/);
  await page
    .getByLabel("Type the last six characters of the backup key")
    .fill(backup.slice(-6));
  await page.getByRole("button", { name: "I've written it down" }).click();
  await page.getByLabel("This matches the service key I was given").check();
  await page.getByRole("button", { name: "Create business" }).click();
  await expect(
    page.getByRole("heading", { name: "Your business, at a glance." }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: /Sync status: Synced/ }),
  ).toBeVisible();
  return backup;
}

/** Records and, with `approve`, posts an expense. */
export async function recordExpense(
  page: Page,
  e: {
    vendor: string;
    amount: string;
    approve: boolean;
    receipt?: { name: string; mimeType: string; buffer: Buffer };
  },
) {
  await page.getByRole("button", { name: "New expense", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("Vendor", { exact: true }).fill(e.vendor);
  await dialog.getByLabel("Description", { exact: true }).fill("Glaze samples");
  await dialog.getByLabel("Amount", { exact: true }).fill(e.amount);
  await dialog
    .getByRole("combobox", { name: "Category", exact: true })
    .selectOption({ label: "Supplies" });
  await dialog
    .getByLabel("Business purpose", { exact: true })
    .fill("Testing new glazes");
  if (e.receipt)
    await dialog
      .getByLabel("Attach receipt", { exact: true })
      .setInputFiles(e.receipt);
  await dialog
    .getByRole("button", { name: e.approve ? "Approve & post" : "Save draft" })
    .click();
  await expect(dialog).not.toBeVisible();
}

/** The business id the app has open. */
export async function openBusiness(page: Page) {
  return page.getByLabel("Switch business").inputValue();
}
