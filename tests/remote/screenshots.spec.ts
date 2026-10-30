/**
 * Screenshots of the screens stage 2 PR 4b adds, for its description, with
 * invented data. Run with SCREENSHOTS=1 npm run test:e2e:remote; otherwise
 * skipped. Dates are set in 2031, so no real date appears.
 */
import { expect, test, type Page } from "@playwright/test";
import { freshKey, SERVICE_URL } from "./keys";
import { installSigner } from "./signer";
import { addMember, tampered, type Event } from "./harness";
import { openBusiness, signIn } from "./flows";

test.skip(!process.env.SCREENSHOTS, "Set SCREENSHOTS=1 to take them.");

const dir = "docs/screenshots/stage-2-pr4b";
async function shot(page: Page, name: string) {
  // Toasts from earlier steps would cover the screen.
  for (const close of await page
    .getByRole("button", { name: "Dismiss notification" })
    .all())
    await close.click().catch(() => {});
  await page.screenshot({ path: `${dir}/${name}.png`, fullPage: true });
}
const png = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aOCcAAAAASUVORK5CYII=",
  "base64",
);

async function expense(page: Page, vendor: string, amount: string) {
  await page.getByRole("button", { name: "New expense", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("Vendor", { exact: true }).fill(vendor);
  await dialog.getByLabel("Description", { exact: true }).fill("Glaze samples");
  await dialog.getByLabel("Date", { exact: true }).fill("2031-03-14");
  await dialog.getByLabel("Amount", { exact: true }).fill(amount);
  await dialog
    .getByRole("combobox", { name: "Category", exact: true })
    .selectOption({ label: "Supplies" });
  await dialog
    .getByLabel("Business purpose", { exact: true })
    .fill("Testing new glazes");
  await dialog
    .getByLabel("Attach receipt", { exact: true })
    .setInputFiles({ name: "receipt.png", mimeType: "image/png", buffer: png });
  await dialog.getByRole("button", { name: "Approve & post" }).click();
  await expect(dialog).not.toBeVisible();
}

test("screens", async ({ browser }) => {
  test.setTimeout(120_000);
  const viewport = { width: 1280, height: 860 };

  const bare = await browser.newContext({ viewport });
  const none = await bare.newPage();
  await none.goto("/");
  await expect(none.getByText("No Nostr signer extension")).toBeVisible();
  await shot(none, "sign-in-no-extension");
  await bare.close();

  const owner = freshKey();
  const context = await browser.newContext({ viewport });
  await installSigner(context, owner);
  const page = await context.newPage();
  await page.goto("/");
  await expect(
    page.getByRole("button", { name: "Sign in", exact: true }),
  ).toBeVisible();
  await shot(page, "sign-in");
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await page.getByLabel("Business name").fill("Fernhill Ceramics");
  await page.getByLabel("Industry").fill("Pottery");
  await shot(page, "create-business");
  await page.getByRole("button", { name: "Generate a backup key" }).click();
  await shot(page, "create-business-backup-key");
  const backup = (
    await page.getByLabel("Backup key", { exact: true }).innerText()
  ).replace(/\s+/g, "");
  await page
    .getByLabel("Type the last six characters of the backup key")
    .fill(backup.slice(-6));
  await page.getByRole("button", { name: "I've written it down" }).click();
  await page.getByLabel("This matches the service key I was given").check();
  await shot(page, "create-business-confirmed");
  await page.getByRole("button", { name: "Create business" }).click();
  await expect(
    page.getByRole("button", { name: /Sync status: Synced/ }),
  ).toBeVisible();
  await expense(page, "Kiln Supply Depot", "61.37");
  await page.getByLabel("Reporting period").fill("2031-03");
  await shot(page, "workspace-synced");

  // Offline: the change waits, and the panel lists it.
  await page.route(`${SERVICE_URL}/**`, (r) => r.abort("internetdisconnected"));
  await page.getByRole("button", { name: "New expense", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("Vendor", { exact: true }).fill("Glaze House");
  await dialog.getByLabel("Date", { exact: true }).fill("2031-03-15");
  await dialog.getByLabel("Amount", { exact: true }).fill("9.80");
  await dialog.getByRole("button", { name: "Save draft" }).click();
  await expect(page.getByRole("button", { name: /Offline/ })).toBeVisible();
  await shot(page, "sync-offline");
  await page.getByRole("button", { name: /Sync status/ }).click();
  await shot(page, "sync-waiting-list");
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Close dialog" })
    .click();
  await page.unroute(`${SERVICE_URL}/**`);
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await expect(
    page.getByRole("button", { name: /Sync status: Synced/ }),
  ).toBeVisible();

  // A second member sees the receipt as attached on another device.
  const member = freshKey();
  await addMember(owner, await openBusiness(page), member);
  const other = await browser.newContext({ viewport });
  await installSigner(other, member);
  const second = await other.newPage();
  await signIn(second);
  await second
    .getByRole("button", { name: "Transactions", exact: true })
    .click();
  await second
    .getByRole("button", { name: "View Kiln Supply Depot", exact: true })
    .click();
  await expect(
    second.getByText("This receipt was attached on another device"),
  ).toBeVisible();
  await shot(second, "receipt-on-another-device");
  await other.close();

  // A tampered acceptance: sync stops with the error.
  await page.route(
    `${SERVICE_URL}/v1/businesses/*/acceptances*`,
    async (route) => {
      const response = await route.fetch();
      const body = (await response.json()) as { acceptances: Event[] };
      body.acceptances = body.acceptances.map(tampered);
      await route.fulfill({ response, json: body });
    },
  );
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByLabel("Industry").fill("Pottery and glazes");
  await page
    .getByRole("button", { name: "Save profile" })
    .click()
    .catch(() => {});
  await expect(page.getByRole("button", { name: /Sync error/ })).toBeVisible();
  await page.getByRole("button", { name: "Overview", exact: true }).click();
  await page.getByLabel("Reporting period").fill("2031-03");
  await shot(page, "sync-error");
  await context.close();
});
