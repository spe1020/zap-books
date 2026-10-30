import { expect, test } from "@playwright/test";
import { decode } from "nostr-tools/nip19";
import { freshKey, OTHER_SERVICE, SERVICE_URL } from "./keys";
import { installSigner } from "./signer";
import { createBusiness, signIn } from "./flows";

test("the service-key pin: creation is refused if the service reports another key", async ({
  browser,
}) => {
  const context = await browser.newContext();
  await installSigner(context, freshKey());
  const page = await context.newPage();
  await page.route(`${SERVICE_URL}/v1/service`, (route) =>
    route.fulfill({
      json: { pubkey: OTHER_SERVICE.pubkey },
      headers: { "access-control-allow-origin": "*" },
    }),
  );
  let proposals = 0;
  page.on("request", (r) => {
    if (r.url().endsWith("/v1/proposals")) proposals++;
  });
  await signIn(page);
  await page.getByLabel("Business name").fill("Larkspur Glassworks");
  await page.getByRole("button", { name: "Generate a backup key" }).click();
  const backup = (
    await page.getByLabel("Backup key", { exact: true }).innerText()
  ).replace(/\s+/g, "");
  await page
    .getByLabel("Type the last six characters of the backup key")
    .fill(backup.slice(-6));
  await page.getByRole("button", { name: "I've written it down" }).click();
  await page.getByLabel("This matches the service key I was given").check();
  await page.getByRole("button", { name: "Create business" }).click();
  await expect(page.getByRole("alert")).toContainText(
    "reports a different key",
  );
  await expect(
    page.getByRole("heading", { name: "Create a business" }),
  ).toBeVisible();
  expect(proposals).toBe(0);
  await context.close();
});

test("the backup key shown at creation never reaches storage, logs, requests or the DOM", async ({
  browser,
}) => {
  const context = await browser.newContext();
  await installSigner(context, freshKey());
  const page = await context.newPage();
  const seen: string[] = [];
  page.on("console", (m) => seen.push(m.text()));
  page.on("request", (r) =>
    seen.push(r.url(), JSON.stringify(r.headers()), r.postData() ?? ""),
  );
  await signIn(page);
  const nsec = await createBusiness(page, "Sorrel Lane Pottery");
  const hex = Buffer.from(decode(nsec).data as Uint8Array).toString("hex");
  expect(hex).toMatch(/^[0-9a-f]{64}$/);

  const forms = [nsec, hex, hex.toUpperCase()];
  const dom = await page.content();
  const storage = await page.evaluate(async () => {
    const out: string[] = [
      JSON.stringify(localStorage),
      JSON.stringify(sessionStorage),
    ];
    for (const info of await indexedDB.databases()) {
      const db = await new Promise<IDBDatabase>((resolve, reject) => {
        const r = indexedDB.open(info.name!);
        r.onsuccess = () => resolve(r.result);
        r.onerror = () => reject(r.error);
      });
      for (const name of db.objectStoreNames) {
        const all = await new Promise<unknown[]>((resolve) => {
          const r = db.transaction(name).objectStore(name).getAll();
          r.onsuccess = () => resolve(r.result);
        });
        out.push(JSON.stringify(all));
      }
      db.close();
    }
    return out.join("\n");
  });
  for (const form of forms) {
    expect(dom).not.toContain(form);
    expect(storage).not.toContain(form);
    for (const s of seen) expect(s).not.toContain(form);
  }
  // The last six characters were typed, but aren't left in the page.
  expect(dom).not.toContain(nsec.slice(-12));
  await context.close();
});
