import { expect, test, type Page } from "@playwright/test";
import { freshKey, SERVICE_URL } from "./keys";
import { installSigner } from "./signer";
import { addMember } from "./harness";
import { createBusiness, openBusiness, recordExpense, signIn } from "./flows";

const offline = (page: Page) =>
  page.route(`${SERVICE_URL}/**`, (route) =>
    route.abort("internetdisconnected"),
  );
async function online(page: Page) {
  await page.unroute(`${SERVICE_URL}/**`);
  // Coming back to the window syncs and sends what waits.
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
}
const status = (page: Page, text: string | RegExp) =>
  expect(
    page.getByRole("button", {
      name: new RegExp(
        `Sync status: ${typeof text === "string" ? text : text.source}`,
      ),
    }),
  ).toBeVisible();

async function editDraft(page: Page, vendor: string, description: string) {
  await page.getByRole("button", { name: /Expense inbox/ }).click();
  await page
    .getByRole("button", { name: `Review ${vendor}`, exact: true })
    .click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("Description", { exact: true }).fill(description);
  await dialog.getByRole("button", { name: "Save draft" }).click();
  await expect(dialog).not.toBeVisible();
}

test("commands made offline wait in order, and are accepted when the service is back", async ({
  browser,
}) => {
  const context = await browser.newContext();
  await installSigner(context, freshKey());
  const page = await context.newPage();
  await signIn(page);
  await createBusiness(page, "Willowbank Studio");

  await offline(page);
  await recordExpense(page, {
    vendor: "Slip Casting Co",
    amount: "12.30",
    approve: false,
  });
  await expect(page.getByRole("status")).toContainText(
    "when the service can be reached",
  );
  await recordExpense(page, {
    vendor: "Glaze House",
    amount: "9.80",
    approve: false,
  });
  await status(page, "Offline · 2 waiting");
  await page.getByRole("button", { name: /Sync status/ }).click();
  const panel = page.getByRole("dialog");
  await expect(
    panel.getByText("Waiting to sync", { exact: true }).first(),
  ).toBeVisible();
  await expect(panel.locator(".waiting-list li")).toHaveCount(2);
  await expect(panel.locator(".waiting-list li").first()).toContainText(
    "Slip Casting Co",
  );
  await panel.getByRole("button", { name: "Close dialog" }).click();
  // Nothing is in the books until the service accepts it.
  await page.getByRole("button", { name: /Expense inbox/ }).click();
  await expect(page.locator("tbody tr")).toHaveCount(0);

  await online(page);
  await status(page, "Synced");
  await expect(page.locator("tbody tr")).toHaveCount(2);
  await expect(page.locator("tbody tr").nth(0)).toContainText(
    "Slip Casting Co",
  );
  await expect(page.locator("tbody tr").nth(1)).toContainText("Glaze House");
  await context.close();
});

test("a queued command that conflicts stops the queue with its message; later ones stay queued", async ({
  browser,
}) => {
  const owner = freshKey();
  const member = freshKey();
  const context = await browser.newContext();
  await installSigner(context, owner);
  const page = await context.newPage();
  await signIn(page);
  await createBusiness(page, "Alder Row Ceramics");
  await recordExpense(page, {
    vendor: "Slip Casting Co",
    amount: "12.30",
    approve: false,
  });
  const business = await openBusiness(page);
  await addMember(owner, business, member);

  // Offline, the owner edits the draft and records another.
  await offline(page);
  await editDraft(page, "Slip Casting Co", "Edited offline");
  await recordExpense(page, {
    vendor: "Glaze House",
    amount: "9.80",
    approve: false,
  });
  await status(page, "Offline · 2 waiting");

  // Meanwhile a bookkeeper changes the same draft.
  const other = await browser.newContext();
  await installSigner(other, member);
  const second = await other.newPage();
  await signIn(second);
  await editDraft(second, "Slip Casting Co", "Edited by the bookkeeper");
  await status(second, "Synced");

  await online(page);
  await status(page, "Sync error");
  await page.getByRole("button", { name: /Sync status/ }).click();
  const panel = page.getByRole("dialog");
  const items = panel.locator(".waiting-list li");
  await expect(items).toHaveCount(2);
  await expect(items.nth(0)).toContainText(
    "This record changed since you opened it",
  );
  await expect(items.nth(1)).toContainText("Waiting to sync");

  // Redo it from the current books: both go through, in order.
  await items
    .nth(0)
    .getByRole("button", { name: "Redo from current books" })
    .click();
  await expect(items).toHaveCount(0);
  await panel.getByRole("button", { name: "Close dialog" }).click();
  await status(page, "Synced");
  await page.getByRole("button", { name: /Expense inbox/ }).click();
  await expect(page.locator("tbody")).toContainText("Edited offline");
  await expect(page.locator("tbody")).toContainText("Glaze House");
  await context.close();
  await other.close();
});
