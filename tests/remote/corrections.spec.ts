import { expect, test } from "@playwright/test";
import { freshKey } from "./keys";
import { installSigner } from "./signer";
import { createBusiness, recordExpense, signIn } from "./flows";

const png = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aOCcAAAAASUVORK5CYII=",
  "base64",
);

test("a correction with a new receipt, and a rule saved from Review, go to the service", async ({
  browser,
}) => {
  const context = await browser.newContext();
  await installSigner(context, freshKey());
  const page = await context.newPage();
  await signIn(page);
  await createBusiness(page, "Cedar Hollow Studio");

  // "Save as a rule" sends its own SaveRule proposal before the draft.
  await page.getByRole("button", { name: "New expense", exact: true }).click();
  let dialog = page.getByRole("dialog");
  await dialog.getByLabel("Vendor", { exact: true }).fill("Bisque Supply");
  await dialog.getByLabel("Description", { exact: true }).fill("Cones");
  await dialog.getByLabel("Amount", { exact: true }).fill("40.00");
  await dialog
    .getByRole("combobox", { name: "Category", exact: true })
    .selectOption({ label: "Supplies" });
  await dialog.getByLabel("Business purpose", { exact: true }).fill("Firing");
  await dialog
    .getByLabel("Save this vendor classification as a visible rule")
    .check();
  await dialog.getByRole("button", { name: "Approve & post" }).click();
  await expect(dialog).not.toBeVisible();
  await expect(page.locator(".principal .stat-value")).toHaveText("$40.00");
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await expect(
    page.getByText("Vendor contains “Bisque Supply”", { exact: true }),
  ).toBeVisible();

  // Correct it, attaching the receipt in the correction.
  await page.getByRole("button", { name: "Transactions", exact: true }).click();
  await page
    .getByRole("button", { name: "View Bisque Supply", exact: true })
    .click();
  await page.getByRole("button", { name: "Correct expense" }).click();
  dialog = page.getByRole("dialog");
  await dialog.getByLabel("Amount", { exact: true }).fill("44.00");
  await dialog
    .getByLabel("Attach receipt", { exact: true })
    .setInputFiles({ name: "cones.png", mimeType: "image/png", buffer: png });
  await dialog
    .getByLabel("Reason for correction")
    .fill("The receipt says 44.00");
  await dialog.getByRole("button", { name: "Post correction" }).click();
  await expect(dialog).not.toBeVisible();
  await page.getByRole("button", { name: "Overview", exact: true }).click();
  await expect(page.locator(".principal .stat-value")).toHaveText("$44.00");
  await page.getByRole("button", { name: "Transactions", exact: true }).click();
  await page.getByRole("button", { name: "Corrections", exact: true }).click();
  await expect(page.locator("tbody tr")).toHaveCount(2);
  await context.close();
});

test("a draft that fails validation is refused before anything is sent", async ({
  browser,
}) => {
  const context = await browser.newContext();
  await installSigner(context, freshKey());
  const page = await context.newPage();
  await signIn(page);
  await createBusiness(page, "Rowan Field Pottery");
  let proposals = 0;
  page.on("request", (r) => {
    if (r.url().endsWith("/v1/proposals")) proposals++;
  });
  await page.getByRole("button", { name: "New expense", exact: true }).click();
  await page.getByRole("button", { name: "Approve & post" }).click();
  await expect(page.getByRole("dialog").getByRole("alert")).toContainText(
    "Vendor is required",
  );
  expect(proposals).toBe(0);
  await page.getByRole("button", { name: "Close dialog" }).click();
  await recordExpense(page, {
    vendor: "Glaze House",
    amount: "5.00",
    approve: false,
  });
  expect(proposals).toBe(1);
  await context.close();
});
