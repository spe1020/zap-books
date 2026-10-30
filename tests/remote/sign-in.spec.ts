import { expect, test } from "@playwright/test";
import { freshKey } from "./keys";
import { installSigner } from "./signer";

test("without a signer extension, the sign-in screen explains one and links to a few", async ({
  page,
}) => {
  await page.goto("/");
  await expect(
    page.getByText("No Nostr signer extension was found"),
  ).toBeVisible();
  await expect(page.locator(".extension-list li")).toHaveCount(3);
  await expect(
    page.getByRole("button", { name: "Sign in", exact: true }),
  ).toHaveCount(0);
  // The local demo needs no sign-in.
  await page.getByRole("button", { name: "Open the local demo" }).click();
  await expect(
    page.getByRole("heading", { name: "Your business, at a glance." }),
  ).toBeVisible();
  await expect(page.getByText("Demo data")).toBeVisible();
});

test("a signer without NIP-44 is refused, with no fallback", async ({
  browser,
}) => {
  const context = await browser.newContext();
  await installSigner(context, freshKey(), { nip44: false });
  const page = await context.newPage();
  await page.goto("/");
  await expect(page.getByRole("alert")).toContainText("doesn't support NIP-44");
  await expect(
    page.getByRole("button", { name: "Sign in", exact: true }),
  ).toHaveCount(0);
  await context.close();
});
