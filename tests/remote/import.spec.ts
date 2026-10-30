import { expect, test } from "@playwright/test";
import { freshKey, SERVICE_URL } from "./keys";
import { installSigner } from "./signer";
import { createBusiness, signIn } from "./flows";

const ROWS = 60;
const csv = [
  "date,merchant,amount,currency,reference",
  ...Array.from(
    { length: ROWS },
    (_, i) =>
      `2031-01-${String((i % 28) + 1).padStart(2, "0")},Studio Vendor ${i + 1},-${10 + i}.${String(i % 100).padStart(2, "0")},USD,TEST-IMPORT-${i + 1}`,
  ),
].join("\n");

test("a large import goes in batches, shows its progress, and resumes after an interruption", async ({
  browser,
}) => {
  const context = await browser.newContext();
  await installSigner(context, freshKey());
  const page = await context.newPage();
  await signIn(page);
  await createBusiness(page, "Birchfield Pottery");

  // The connection drops once, just after the first batch is accepted.
  let accepted = 0;
  let tooLarge = 0;
  let dropped = false;
  await page.route(`${SERVICE_URL}/v1/proposals`, async (route) => {
    if (accepted === 1 && !dropped) {
      dropped = true;
      return route.abort("internetdisconnected");
    }
    const response = await route.fetch();
    const body = (await response.json()) as { status: string };
    if (body.status === "accepted") accepted++;
    else tooLarge++;
    await route.fulfill({ response, json: body });
  });
  await page.getByRole("button", { name: "Import CSV", exact: true }).click();
  await page.getByLabel("Choose CSV file").setInputFiles({
    name: "payments.csv",
    mimeType: "text/csv",
    buffer: Buffer.from(csv),
  });
  await expect(
    page.getByText(`${ROWS} new · 0 repeated · 0 invalid`),
  ).toBeVisible();
  await page.getByRole("button", { name: `Import ${ROWS} drafts` }).click();
  await expect(page.getByRole("status")).toContainText(
    "when the service can be reached",
  );
  // The first batch is in the books; the rest wait, with the progress.
  const inbox = page.getByRole("button", { name: /Expense inbox/ });
  await expect(inbox).not.toContainText("inbox0");
  const first = Number((await inbox.innerText()).replace(/\D/g, ""));
  expect(first).toBeLessThan(ROWS);
  await page.getByRole("button", { name: /Sync status/ }).click();
  const panel = page.getByRole("dialog");
  const waiting = await panel.locator(".waiting-list li").count();
  expect(waiting).toBeGreaterThan(1);
  await expect(panel).toContainText(`Importing: batch 2 of ${waiting + 1}`);
  await panel.getByRole("button", { name: "Close dialog" }).click();

  // Back online (the route only ever dropped one request), the import
  // picks up where it stopped.
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await expect(
    page.getByRole("button", { name: /Sync status: Synced/ }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: /Expense inbox/ }),
  ).toContainText(String(ROWS));
  // 25 rows made too large an acceptance, so the batches shrank.
  expect(tooLarge).toBeGreaterThan(0);
  expect(accepted).toBeGreaterThan(2);
  // Nothing was imported twice.
  await page.getByRole("button", { name: /Expense inbox/ }).click();
  await page.getByLabel("Search inbox").fill("Studio Vendor 26");
  await expect(page.locator("tbody tr")).toHaveCount(1);
  await context.close();
});
