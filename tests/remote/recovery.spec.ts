import { expect, test } from "@playwright/test";
import { npubEncode } from "nostr-tools/nip19";
import { freshKey, fromNsec } from "./keys";
import { installSigner } from "./signer";
import { addMember, chain, propose, readAs } from "./harness";
import { followClock, setClockOffset } from "./clock";
import {
  createBusiness,
  invite,
  openBusiness,
  openMembers,
  signIn,
} from "./flows";
import type { PendingChange } from "../../src/domain/members";

/** A key as the app shows it: `npub1abcde…uvwxyz`. */
const short = (hex: string) => {
  const npub = npubEncode(hex);
  return `${npub.slice(0, 10)}…${npub.slice(-6)}`;
};

test("the backup key recovers an owner, the lost owner can't veto, and the new owner reviews exactly the members added meanwhile", async ({
  browser,
}) => {
  test.setTimeout(120_000);
  const owner = freshKey();
  const bookkeeper = freshKey();
  const later = freshKey();
  const replacement = freshKey();
  const ownerContext = await browser.newContext();
  await installSigner(ownerContext, owner);
  const page = await ownerContext.newPage();
  await signIn(page);
  const backup = fromNsec(await createBusiness(page, "Heronsgate Kilns"));
  const business = await openBusiness(page);
  // Added before the recovery is proposed: not in the banner.
  await invite(page, npubEncode(bookkeeper.pubkey), "Bookkeeper");
  await expect(page.getByRole("status")).toContainText("Bookkeeper added.");

  // Recovery mode, signed in with the backup key in its own browser.
  const backupContext = await browser.newContext();
  await installSigner(backupContext, backup);
  const recovery = await backupContext.newPage();
  await signIn(recovery);
  await expect(
    recovery.getByText(
      "You're signed in with this business's backup key. It can only veto changes and recover a lost owner. It can't read the books.",
    ),
  ).toBeVisible();
  await expect(
    recovery.getByRole("combobox", { name: "The lost owner" }),
  ).toContainText(short(owner.pubkey));
  await recovery
    .getByLabel("The replacement owner's npub")
    .fill(npubEncode(replacement.pubkey));
  const start = recovery.getByRole("button", { name: "Start the recovery" });
  await expect(start).toBeDisabled();
  await recovery.getByLabel(/I understand the replacement takes over/).check();
  await start.click();
  await expect(recovery.getByRole("status")).toContainText(
    "Recovery proposed.",
  );
  const item = recovery.locator(".pending-item");
  await expect(item).toContainText(
    `Recover owner ${short(owner.pubkey)}, replacing it with ${short(replacement.pubkey)}`,
  );
  await expect(item).toContainText("Proposed by this backup key");
  await expect(item).toContainText(/Takes effect in/);

  // The owner it names as lost sees it, with its countdown, and no veto.
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await openMembers(page);
  const theirs = page.locator(".pending-item");
  await expect(theirs).toContainText("names your key as the lost owner");
  await expect(theirs).toContainText(/Takes effect in/);
  await expect(theirs.getByRole("button", { name: /Veto/ })).toHaveCount(0);
  // A veto crafted with that key is rejected.
  const { body } = await readAs<{ pending: PendingChange[] }>(
    owner,
    `/v1/businesses/${business}/pending`,
  );
  expect(body.pending).toHaveLength(1);
  const veto = await propose(owner, business, {
    type: "Veto",
    pending: body.pending[0].proposal,
  });
  expect(veto.status).toBe("rejected");

  // Added while the recovery waits: the new owner should review this one.
  await addMember(owner, business, later, "viewer");

  try {
    // 48 hours pass.
    setClockOffset((48 * 60 * 60 + 120) * 1000);
    // A read lets the service activate what's due.
    await chain(bookkeeper, business);

    const newContext = await browser.newContext();
    await followClock(newContext);
    await installSigner(newContext, replacement);
    const fresh = await newContext.newPage();
    await signIn(fresh);
    const banner = fresh.getByRole("region", {
      name: "Your business was recovered",
    });
    await expect(banner).toContainText(
      /Your business was recovered\. Review the members added since .+\./,
    );
    const listed = banner.locator(".recovery-added li");
    await expect(listed).toHaveCount(1);
    await expect(listed).toContainText(short(later.pubkey));
    await expect(banner).not.toContainText(short(bookkeeper.pubkey));
    await expect(banner).not.toContainText(short(owner.pubkey));

    await listed
      .getByRole("button", { name: `Remove ${short(later.pubkey)}` })
      .click();
    await expect(fresh.getByRole("status")).toContainText("Member removed.");
    await expect(banner).toContainText("No members were added in that time.");
    await banner.getByRole("button", { name: "Dismiss" }).click();
    await expect(banner).toHaveCount(0);
    // Dismissed for good in this browser.
    await signIn(fresh);
    await expect(
      fresh.getByRole("button", { name: /Sync status: Synced/ }),
    ).toBeVisible();
    await expect(banner).toHaveCount(0);
    await newContext.close();
  } finally {
    setClockOffset(0);
  }
  await backupContext.close();
  await ownerContext.close();
});
