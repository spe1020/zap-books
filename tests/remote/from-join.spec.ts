import { expect, test } from "@playwright/test";
import { freshKey } from "./keys";
import { installSigner } from "./signer";
import { chain, epochKeys, epochOf, openWith, propose } from "./harness";
import { createBusiness, openBusiness, recordExpense, signIn } from "./flows";

// The app invites with full history only (docs/protocol.md, Epochs). The
// protocol still supports from-join history, so its guarantee is checked
// here with a member added outside the app.
test("a from-join-date member sees nothing from before their join", async ({
  browser,
}) => {
  const owner = freshKey();
  const joiner = freshKey();
  const context = await browser.newContext();
  await installSigner(context, owner);
  const page = await context.newPage();
  await signIn(page);
  await createBusiness(page, "Fieldfare Glass");
  const business = await openBusiness(page);
  await recordExpense(page, {
    vendor: "Before Join Supply",
    amount: "22.40",
    approve: true,
  });

  const ack = await propose(owner, business, {
    type: "ChangeMembership",
    change: {
      op: "add",
      member: joiner.pubkey,
      role: "viewer",
      history: "fromJoin",
    },
  });
  expect(ack.status).toBe("accepted");
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await recordExpense(page, {
    vendor: "After Join Supply",
    amount: "31.05",
    approve: true,
  });

  // Their keys start at the epoch their joining started.
  const keys = await epochKeys(joiner, business);
  const first = Math.min(...keys.keys());
  expect(first).toBeGreaterThan(0);
  const { acceptances } = await chain(owner, business);
  const before = acceptances.filter((e) => epochOf(e) < first);
  const after = acceptances.filter((e) => epochOf(e) >= first);
  expect(before.length).toBeGreaterThan(0);
  expect(after.length).toBeGreaterThan(0);
  for (const event of before)
    for (const secret of keys.values())
      expect(() => openWith(event, business, secret)).toThrow();
  for (const event of after)
    expect(() =>
      openWith(event, business, keys.get(epochOf(event))!),
    ).not.toThrow();

  // The app can't rebuild books from a join date yet, and says so.
  const joinerContext = await browser.newContext();
  await installSigner(joinerContext, joiner);
  const second = await joinerContext.newPage();
  await signIn(second);
  await expect(
    second.getByRole("heading", { name: "Your books couldn't be opened" }),
  ).toBeVisible();
  await expect(second.getByRole("alert")).toContainText(
    "You were added with history from your join date",
  );
  await expect(second.getByText("Before Join Supply")).toHaveCount(0);
  await joinerContext.close();
  await context.close();
});
