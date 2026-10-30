import { expect, test } from "@playwright/test";
import { npubEncode } from "nostr-tools/nip19";
import { freshKey } from "./keys";
import { installSigner } from "./signer";
import { assistantDraft, books, propose } from "./harness";
import { createBusiness, invite, openBusiness, signIn } from "./flows";

test("an assistant's drafts are marked, and it can't approve: the UI hides the action and the service refuses a crafted approval", async ({
  browser,
}) => {
  const owner = freshKey();
  const assistant = freshKey();
  const ownerContext = await browser.newContext();
  await installSigner(ownerContext, owner);
  const page = await ownerContext.newPage();
  await signIn(page);
  await createBusiness(page, "Copperleaf Ceramics");
  const business = await openBusiness(page);
  await invite(page, npubEncode(assistant.pubkey), "AI assistant");
  await expect(page.getByRole("status")).toContainText("AI assistant added.");

  // The assistant's harness creates a draft with its own key.
  const draft = await assistantDraft(assistant, business, "Kiln Paper Co");
  const created = await propose(assistant, business, {
    type: "SubmitReceiptDraft",
    draft,
    documents: [],
  });
  expect(created.status).toBe("accepted");

  // The owner sees it marked, in the inbox and on the Review screen.
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await page.getByRole("button", { name: /Expense inbox/ }).click();
  const row = page.getByRole("row", { name: /Kiln Paper Co/ });
  await expect(row).toContainText("Created by an AI assistant");
  await row.getByRole("button", { name: "Review Kiln Paper Co" }).click();
  await expect(page.getByRole("dialog")).toContainText(
    "Created by an AI assistant",
  );
  await page.getByRole("button", { name: "Close dialog" }).click();

  // In the assistant's own browser, nothing approves, corrects or imports.
  const assistantContext = await browser.newContext();
  await installSigner(assistantContext, assistant);
  const second = await assistantContext.newPage();
  await signIn(second);
  await expect(
    second.getByRole("button", { name: /Sync status: Synced/ }),
  ).toBeVisible();
  await expect(
    second.getByRole("button", { name: "Import CSV", exact: true }),
  ).toHaveCount(0);
  await expect(
    second.getByRole("button", { name: "New expense", exact: true }),
  ).toHaveCount(0);
  await second.getByRole("button", { name: /Expense inbox/ }).click();
  await second
    .getByRole("button", { name: "Review Kiln Paper Co", exact: true })
    .click();
  const dialog = second.getByRole("dialog");
  await expect(
    dialog.getByRole("button", { name: "Save draft" }),
  ).toBeVisible();
  await expect(
    dialog.getByRole("button", { name: "Approve & post" }),
  ).toHaveCount(0);
  await expect(
    dialog.getByText("Save this vendor classification as a visible rule"),
  ).toHaveCount(0);

  // A crafted approval, signed by the assistant, is rejected.
  const approval = await propose(assistant, business, {
    type: "Approve",
    expenseId: draft.id,
  });
  expect(approval.status).toBe("rejected");
  const after = await books(owner, business);
  expect(after.drafts.find((d) => d.id === draft.id)?.status).toBe("draft");
  await assistantContext.close();
  await ownerContext.close();
});
