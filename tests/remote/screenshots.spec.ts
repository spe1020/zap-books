/**
 * Screenshots of the screens stage 2 PRs 4b and 4c add, for their
 * descriptions, with invented data. Run with SCREENSHOTS=1 npm run
 * test:e2e:remote; otherwise skipped. Dates are set in 2031, so no real
 * date appears.
 */
import { expect, test, type Page } from "@playwright/test";
import { npubEncode } from "nostr-tools/nip19";
import { freshKey, fromNsec, SERVICE_URL, type TestKey } from "./keys";
import { installSigner } from "./signer";
import {
  addMember,
  assistantDraft,
  chain,
  propose,
  tampered,
  type Event,
} from "./harness";
import { followClock, setClockOffset } from "./clock";
import {
  createBusiness,
  invite,
  openBusiness,
  openMembers,
  signIn,
} from "./flows";

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

/** The screens stage 2 PR 4c adds. The service and pages run in 2031. */
test("people screens", async ({ browser }) => {
  test.setTimeout(180_000);
  const dir4c = "docs/screenshots/stage-2-pr4c";
  const at = async (page: Page, name: string) => {
    for (const close of await page
      .getByRole("button", { name: "Dismiss notification" })
      .all())
      await close.click().catch(() => {});
    await page.screenshot({ path: `${dir4c}/${name}.png`, fullPage: true });
  };
  const viewport = { width: 1280, height: 860 };
  const start = Date.parse("2031-03-10T15:00:00Z") - Date.now();
  setClockOffset(start);
  try {
    const context = async (key: TestKey) => {
      const c = await browser.newContext({ viewport });
      await followClock(c);
      await installSigner(c, key);
      return c;
    };
    const owner = freshKey();
    const bookkeeper = freshKey();
    const assistant = freshKey();
    const incoming = freshKey();
    const later = freshKey();
    const replacement = freshKey();

    const ownerContext = await context(owner);
    const page = await ownerContext.newPage();
    await signIn(page);
    const backup = fromNsec(await createBusiness(page, "Fernhill Ceramics"));
    const business = await openBusiness(page);
    await invite(page, npubEncode(bookkeeper.pubkey), "Bookkeeper");
    await expect(page.getByRole("status")).toContainText("Bookkeeper added.");
    await page.getByLabel("Their npub").fill(npubEncode(assistant.pubkey));
    await page
      .getByRole("combobox", { name: "Role", exact: true })
      .selectOption({ label: "AI assistant" });
    await at(page, "invite-assistant");
    await page.getByRole("button", { name: "Invite", exact: true }).click();
    await expect(page.getByRole("status")).toContainText("AI assistant added.");

    // The assistant's draft, marked in the inbox.
    const draft = await assistantDraft(assistant, business, "Kiln Paper Co");
    await propose(assistant, business, {
      type: "SubmitReceiptDraft",
      draft,
      documents: [],
    });
    await page.evaluate(() => window.dispatchEvent(new Event("focus")));
    await page.getByRole("button", { name: /Expense inbox/ }).click();
    await expect(page.getByText("Created by an AI assistant")).toBeVisible();
    await at(page, "inbox-assistant-draft");

    // An owner-level invite says it waits, then shows its countdown.
    await openMembers(page);
    await page.getByLabel("Their npub").fill(npubEncode(incoming.pubkey));
    await page
      .getByRole("combobox", { name: "Role", exact: true })
      .selectOption({ label: "Owner" });
    await at(page, "invite-owner");
    await page.getByRole("button", { name: "Invite", exact: true }).click();
    await expect(page.locator(".pending-item")).toHaveCount(1);
    await at(page, "members");
    await page
      .getByRole("combobox", { name: /Role for/ })
      .first()
      .selectOption("owner");
    await at(page, "change-role-owner");
    await page
      .getByRole("dialog")
      .getByRole("button", { name: "Close dialog" })
      .click();

    // The assistant's own view: it saves its draft, and never approves.
    const assistantContext = await context(assistant);
    const bot = await assistantContext.newPage();
    await signIn(bot);
    await bot.getByRole("button", { name: /Expense inbox/ }).click();
    await bot
      .getByRole("button", { name: "Review Kiln Paper Co", exact: true })
      .click();
    await at(bot, "review-as-assistant");
    await assistantContext.close();

    // Recovery mode: veto the owner invite, then recover the owner.
    const backupContext = await context(backup);
    const recovery = await backupContext.newPage();
    await signIn(recovery);
    await expect(recovery.locator(".pending-item")).toHaveCount(1);
    await recovery
      .getByLabel("The replacement owner's npub")
      .fill(npubEncode(replacement.pubkey));
    await recovery
      .getByLabel(/I understand the replacement takes over/)
      .check();
    await at(recovery, "recovery-mode");
    await recovery.getByRole("button", { name: /^Veto/ }).click();
    await expect(recovery.getByRole("status")).toContainText("Change vetoed.");
    await recovery.getByRole("button", { name: "Start the recovery" }).click();
    await expect(recovery.getByRole("status")).toContainText(
      "Recovery proposed.",
    );
    await at(recovery, "recovery-mode-proposed");
    await backupContext.close();

    // The owner it names as lost sees it, and can't veto it.
    await page.evaluate(() => window.dispatchEvent(new Event("focus")));
    await expect(
      page.getByText("names your key as the lost owner"),
    ).toBeVisible();
    await at(page, "pending-recovery-lost-owner");
    await addMember(owner, business, later, "viewer");
    await ownerContext.close();

    // 48 hours on, the new owner reviews who was added meanwhile.
    setClockOffset(start + (48 * 60 * 60 + 120) * 1000);
    await chain(bookkeeper, business);
    const newContext = await context(replacement);
    const fresh = await newContext.newPage();
    await signIn(fresh);
    await expect(
      fresh.getByRole("region", { name: "Your business was recovered" }),
    ).toBeVisible();
    await at(fresh, "after-recovery");
    await newContext.close();

    // A from-join-date member, added outside the app, is told why the
    // books can't be opened.
    const joiner = freshKey();
    await propose(replacement, business, {
      type: "ChangeMembership",
      change: {
        op: "add",
        member: joiner.pubkey,
        role: "viewer",
        history: "fromJoin",
      },
    });
    const joinerContext = await context(joiner);
    const joined = await joinerContext.newPage();
    await signIn(joined);
    await expect(
      joined.getByRole("heading", { name: "Your books couldn't be opened" }),
    ).toBeVisible();
    await at(joined, "books-not-opened");
    await joinerContext.close();
  } finally {
    setClockOffset(0);
  }
});
