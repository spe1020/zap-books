import { expect, test } from "@playwright/test";
import { freshKey } from "./keys";
import { installSigner } from "./signer";
import { addMember } from "./harness";
import { createHash } from "node:crypto";
import { createBusiness, openBusiness, recordExpense, signIn } from "./flows";

const png = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aOCcAAAAASUVORK5CYII=",
  "base64",
);

test("sign in, create a business with a backup key, post an expense, and see it as a second member", async ({
  browser,
}) => {
  const OWNER = freshKey();
  const MEMBER = freshKey();
  const owner = await browser.newContext();
  await installSigner(owner, OWNER);
  const page = await owner.newPage();
  await signIn(page);
  await createBusiness(page, "Fernhill Ceramics");
  await recordExpense(page, {
    vendor: "Kiln Supply Depot",
    amount: "61.37",
    approve: true,
    receipt: { name: "kiln-receipt.png", mimeType: "image/png", buffer: png },
  });
  await expect(page.locator(".principal .stat-value")).toHaveText("$61.37");
  const business = await openBusiness(page);

  // A second member, in a second browser, sees the same verified books.
  await addMember(OWNER, business, MEMBER);
  const member = await browser.newContext();
  await installSigner(member, MEMBER);
  const other = await member.newPage();
  await signIn(other);
  await expect(other.getByLabel("Switch business")).toHaveValue(business);
  await expect(other.locator(".principal .stat-value")).toHaveText("$61.37");
  await other
    .getByRole("button", { name: "Transactions", exact: true })
    .click();
  await other
    .getByRole("button", { name: "View Kiln Supply Depot", exact: true })
    .click();
  // The receipt stays in the browser that attached it; here, its hash.
  await expect(
    other.getByRole("heading", {
      name: "This receipt was attached on another device",
    }),
  ).toBeVisible();
  await expect(other.locator("code.hash")).toHaveText(
    createHash("sha256").update(png).digest("hex"),
  );
  // In the owner's browser, it is still the file.
  await page.getByRole("button", { name: "Transactions", exact: true }).click();
  await page
    .getByRole("button", { name: "View Kiln Supply Depot", exact: true })
    .click();
  await expect(
    page.getByRole("img", { name: "Receipt evidence: kiln-receipt.png" }),
  ).toBeVisible();
  await owner.close();
  await member.close();
});
