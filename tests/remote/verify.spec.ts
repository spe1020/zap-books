import { expect, test, type Page } from "@playwright/test";
import { freshKey, SERVICE_URL } from "./keys";
import { installSigner } from "./signer";
import { resigned, tampered, type Event } from "./harness";
import { createBusiness, recordExpense, signIn } from "./flows";

const seq = (e: Event) => Number(e.tags.find((t) => t[0] === "seq")![1]);

/** Serves the chain through `change`, as a dishonest service would. */
async function rewrite(page: Page, change: (e: Event) => Event) {
  await page.route(
    `${SERVICE_URL}/v1/businesses/*/acceptances*`,
    async (route) => {
      const response = await route.fetch();
      const body = (await response.json()) as { acceptances: Event[] };
      body.acceptances = body.acceptances.map(change);
      await route.fulfill({ response, json: body });
    },
  );
}

/** The head this browser has stored for its open business. */
async function storedHead(page: Page) {
  return page.evaluate(
    () =>
      new Promise<number[]>((resolve, reject) => {
        const r = indexedDB.open("zapbooks-remote-v1");
        r.onerror = () => reject(r.error);
        r.onsuccess = () => {
          const all = r.result
            .transaction("books")
            .objectStore("books")
            .getAll();
          all.onsuccess = () => {
            resolve(
              all.result.map((b: { head: { seq: number } }) => b.head.seq),
            );
            r.result.close();
          };
        };
      }),
  );
}

test("a tampered acceptance stops sync with an error and leaves the stored books unchanged", async ({
  browser,
}) => {
  const context = await browser.newContext();
  await installSigner(context, freshKey());
  const page = await context.newPage();
  await signIn(page);
  await createBusiness(page, "Tamarack Pottery");
  await recordExpense(page, {
    vendor: "Clay Barn",
    amount: "18.40",
    approve: true,
  });
  await expect(page.locator(".principal .stat-value")).toHaveText("$18.40");
  const [head] = await storedHead(page);

  // From now on the service's answers are altered after it signed them.
  await rewrite(page, (e) => (seq(e) > head ? tampered(e) : e));
  await page.getByRole("button", { name: "New expense", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("Vendor", { exact: true }).fill("Wheel Repair Co");
  await dialog.getByLabel("Amount", { exact: true }).fill("75.00");
  await dialog.getByRole("button", { name: "Save draft" }).click();
  await expect(page.getByRole("alert").first()).toContainText("didn't verify");
  await expect(
    page.getByRole("button", { name: /Sync status: Sync error/ }),
  ).toBeVisible();
  expect(await storedHead(page)).toEqual([head]);
  await expect(page.locator(".principal .stat-value")).toHaveText("$18.40");

  // Reloading changes nothing: the altered acceptance still fails.
  await page.reload();
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(page.getByRole("alert").first()).toContainText("didn't verify");
  expect(await storedHead(page)).toEqual([head]);
  await context.close();
});

test("a rewritten chain, with a different event at a seq already stored, is a fork", async ({
  browser,
}) => {
  const context = await browser.newContext();
  await installSigner(context, freshKey());
  const page = await context.newPage();
  await signIn(page);
  await createBusiness(page, "Juniper Kilnworks");
  await recordExpense(page, {
    vendor: "Clay Barn",
    amount: "22.10",
    approve: true,
  });
  const [head] = await storedHead(page);

  // The service now serves a validly signed but different acceptance at the
  // stored head.
  await rewrite(page, (e) => (seq(e) === head ? resigned(e) : e));
  await page.reload();
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(page.getByRole("alert").first()).toContainText(
    "forked or rewritten",
  );
  expect(await storedHead(page)).toEqual([head]);
  await expect(page.locator(".principal .stat-value")).toHaveText("$22.10");

  // The stored head was kept, so the honest chain still verifies.
  await page.unroute(`${SERVICE_URL}/v1/businesses/*/acceptances*`);
  await page.reload();
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(
    page.getByRole("button", { name: /Sync status: Synced/ }),
  ).toBeVisible();
  await context.close();
});
