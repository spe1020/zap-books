import { expect, test } from "@playwright/test";
import { npubEncode } from "nostr-tools/nip19";
import { freshKey } from "./keys";
import { installSigner } from "./signer";
import { createWithSecondOwner, invite, openMembers, signIn } from "./flows";

test("an owner-level change shows its countdown, and a veto by another owner cancels it", async ({
  browser,
}) => {
  const first = freshKey();
  const second = freshKey();
  const incoming = freshKey();
  const firstContext = await browser.newContext();
  await installSigner(firstContext, first);
  const page = await firstContext.newPage();
  await signIn(page);
  await createWithSecondOwner(
    page,
    "Saltmarsh Pottery",
    npubEncode(second.pubkey),
  );

  // Before sending, the form says it waits; afterwards, it's pending.
  await openMembers(page);
  await page
    .getByRole("combobox", { name: "Role", exact: true })
    .selectOption({ label: "Owner" });
  await expect(
    page.getByText("This change takes effect in 48 hours unless vetoed."),
  ).toBeVisible();
  await invite(page, npubEncode(incoming.pubkey), "Owner");
  await expect(page.getByRole("status")).toContainText(
    "It takes effect in 48 hours unless vetoed.",
  );
  const pending = page.locator(".pending-item");
  await expect(pending).toHaveCount(1);
  await expect(pending).toContainText("as an owner");
  await expect(pending).toContainText("Proposed by you");
  await expect(pending).toContainText(
    /Takes effect in (1 day 23 hours|2 days 0 hours)/,
  );
  // No one vetoes their own change.
  await expect(pending.getByRole("button", { name: /Veto/ })).toHaveCount(0);
  // The new owner isn't a member yet.
  await expect(page.locator(".members-table tbody tr")).toHaveCount(2);

  // The other owner sees it, with who proposed it, and vetoes it.
  const secondContext = await browser.newContext();
  await installSigner(secondContext, second);
  const other = await secondContext.newPage();
  await signIn(other);
  await openMembers(other);
  const theirs = other.locator(".pending-item");
  await expect(theirs).toContainText(
    `Proposed by ${npubEncode(first.pubkey).slice(0, 10)}…`,
  );
  await expect(theirs).toContainText(/Takes effect in/);
  await theirs.getByRole("button", { name: /Veto/ }).click();
  await expect(other.getByRole("status")).toContainText("Change vetoed.");
  await expect(other.getByText("No changes are waiting.")).toBeVisible();

  // Cancelled for everyone.
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await expect(page.getByText("No changes are waiting.")).toBeVisible();
  await expect(page.locator(".members-table tbody tr")).toHaveCount(2);
  await secondContext.close();
  await firstContext.close();
});
