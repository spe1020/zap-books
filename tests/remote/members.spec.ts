import { expect, test } from "@playwright/test";
import { npubEncode } from "nostr-tools/nip19";
import { freshKey } from "./keys";
import { installSigner } from "./signer";
import {
  chain,
  epochKeys,
  epochOf,
  openWith,
  readAs,
  sessionStatus,
} from "./harness";
import {
  createBusiness,
  invite,
  openBusiness,
  openMembers,
  recordExpense,
  signIn,
} from "./flows";

test("an owner invites, changes a role and removes; the removed member is cut off at once and can't open what follows", async ({
  browser,
}) => {
  const owner = freshKey();
  const member = freshKey();
  const ownerContext = await browser.newContext();
  await installSigner(ownerContext, owner);
  const page = await ownerContext.newPage();
  await signIn(page);
  await createBusiness(page, "Larkspur Studio");
  const business = await openBusiness(page);

  await invite(page, npubEncode(member.pubkey), "Bookkeeper");
  await expect(page.getByRole("status")).toContainText("Bookkeeper added.");
  const short = `${npubEncode(member.pubkey).slice(0, 10)}…`;
  const row = page.getByRole("row", { name: new RegExp(short) });
  await expect(row).toContainText("Full history");
  await expect(row.getByRole("combobox", { name: /Role for/ })).toHaveValue(
    "bookkeeper",
  );

  // The member opens the books in their own browser.
  const memberContext = await browser.newContext();
  await installSigner(memberContext, member);
  const second = await memberContext.newPage();
  await signIn(second);
  await expect(
    second.getByRole("button", { name: /Sync status: Synced/ }),
  ).toBeVisible();
  await expect(second.getByLabel("Switch business")).toHaveValue(business);

  // A role change between non-owner roles takes effect at once.
  await row.getByRole("combobox", { name: /Role for/ }).selectOption("viewer");
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText("This takes effect at once.");
  await dialog.getByRole("button", { name: "Change role" }).click();
  await expect(row.getByRole("combobox", { name: /Role for/ })).toHaveValue(
    "viewer",
  );

  // What the member could open before the removal.
  const before = await epochKeys(member, business);

  await row.getByRole("button", { name: "Remove" }).click();
  await expect(dialog).toContainText("They lose access at once.");
  await dialog.getByRole("button", { name: "Remove" }).click();
  await expect(page.getByRole("row", { name: new RegExp(short) })).toHaveCount(
    0,
  );

  // Refused at once: no session, no chain, no keys.
  expect(await sessionStatus(member)).toBe(403);
  expect(
    (await readAs(member, `/v1/businesses/${business}/acceptances`)).status,
  ).toBe(404);
  expect(
    (
      await readAs<{ deliveries: unknown[] }>(
        member,
        `/v1/epoch-keys?business=${business}`,
      )
    ).body.deliveries,
  ).toEqual([]);
  // Their browser stops syncing and says why; what it verified stays.
  await second.evaluate(() => window.dispatchEvent(new Event("focus")));
  await expect(second.getByRole("alert").first()).toContainText(
    "you're no longer a member",
  );

  // After the rotation, the keys the member held open nothing new.
  await page.getByRole("button", { name: "Overview", exact: true }).click();
  await recordExpense(page, {
    vendor: "Clay Barn",
    amount: "48.10",
    approve: true,
  });
  const { acceptances } = await chain(owner, business);
  const removal = acceptances.findIndex(
    (e) => epochOf(e) > Math.max(...before.keys()),
  );
  expect(removal).toBeGreaterThan(0);
  for (const event of acceptances.slice(removal)) {
    expect(before.has(epochOf(event))).toBe(false);
    for (const secret of before.values())
      expect(() => openWith(event, business, secret)).toThrow();
  }
  await memberContext.close();
  await ownerContext.close();
});

test("the invite form offers full history only, warns about assistants, and says when a change waits", async ({
  browser,
}) => {
  const context = await browser.newContext();
  await installSigner(context, freshKey());
  const page = await context.newPage();
  await signIn(page);
  await createBusiness(page, "Thistle Works");
  await openMembers(page);
  // No history choice: invites send full history.
  await expect(page.getByLabel("History", { exact: true })).toHaveCount(0);
  await expect(page.getByText("From join date")).toHaveCount(0);
  await expect(
    page.getByText("New members can read the books' full history."),
  ).toBeVisible();
  // Submitters wait for their own screens.
  const roles = page.getByRole("combobox", { name: "Role", exact: true });
  await expect(roles.locator("option")).toHaveText([
    "Owner",
    "Bookkeeper",
    "Viewer",
    "AI assistant",
  ]);

  await roles.selectOption({ label: "AI assistant" });
  await expect(
    page.getByText(
      "An AI assistant can read the whole of this business's books, within the history you choose. If it runs on someone else's computer or service, that data goes there. It can create drafts and suggestions, but it can never approve, correct or import.",
    ),
  ).toBeVisible();
  await expect(page.getByText("takes effect in 48 hours")).toHaveCount(0);

  await roles.selectOption({ label: "Owner" });
  await expect(
    page.getByText("This change takes effect in 48 hours unless vetoed."),
  ).toBeVisible();

  // A secret key pasted by mistake is refused and cleared.
  const npub = page.getByLabel("Their npub");
  await npub.fill(`nsec1${"q".repeat(58)}`);
  await page.getByRole("button", { name: "Invite", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("That's a secret key.");
  await expect(npub).toHaveValue("");
  await context.close();
});
