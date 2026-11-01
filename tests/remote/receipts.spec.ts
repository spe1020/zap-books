/**
 * Encrypted receipt storage in the browser (receipt storage brief, PR 2).
 * Each test here is a merge gate. Invented files and business data only.
 */
import { expect, test, type Page, type Request } from "@playwright/test";
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { BLOB_DIR, freshKey, SERVICE_URL, type TestKey } from "./keys";
import { installSigner } from "./signer";
import { addMember, assistantDraft, books, propose } from "./harness";
import { createBusiness, openBusiness, recordExpense, signIn } from "./flows";

// An invented one-pixel PNG.
const png = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aOCcAAAAASUVORK5CYII=",
  "base64",
);
const receipt = {
  name: "kiln-receipt.png",
  mimeType: "image/png",
  buffer: png,
};

const offline = (page: Page) =>
  page.route(`${SERVICE_URL}/**`, (route) =>
    route.abort("internetdisconnected"),
  );
async function online(page: Page) {
  await page.unroute(`${SERVICE_URL}/**`);
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
}
const status = (page: Page, text: string) =>
  expect(
    page.getByRole("button", { name: `Sync status: ${text}` }),
  ).toBeVisible();

/** The owner's business with one posted expense whose receipt is stored. */
async function postedWithReceipt(page: Page, vendor: string) {
  await signIn(page);
  await createBusiness(page, "Fernhill Ceramics");
  await recordExpense(page, {
    vendor,
    amount: "61.37",
    approve: true,
    receipt,
  });
  return openBusiness(page);
}

/** A second member's browser, signed in to the business. */
async function memberBrowser(
  browser: import("@playwright/test").Browser,
  owner: TestKey,
  business: string,
) {
  const member = freshKey();
  await addMember(owner, business, member);
  const context = await browser.newContext();
  await installSigner(context, member);
  const page = await context.newPage();
  await signIn(page);
  await expect(page.getByLabel("Switch business")).toHaveValue(business);
  return { member, context, page };
}

async function viewPosted(page: Page, vendor: string) {
  await page.getByRole("button", { name: "Transactions", exact: true }).click();
  await page
    .getByRole("button", { name: `View ${vendor}`, exact: true })
    .click();
}

/** The one stored file of `business`, on the test service's disk. */
function storedFile(business: string) {
  const dir = join(BLOB_DIR, business);
  const files = readdirSync(dir).filter((f) => !f.startsWith("."));
  expect(files).toHaveLength(1);
  return join(dir, files[0]);
}

/** The receipt file this page's browser keeps, as base64. */
function heldFile(page: Page, business: string, document: string) {
  return page.evaluate(
    ({ key }) =>
      new Promise<string | null>((resolve, reject) => {
        const open = indexedDB.open("zapbooks-remote-v1");
        open.onerror = () => reject(open.error);
        open.onsuccess = () => {
          const get = open.result
            .transaction("receipts")
            .objectStore("receipts")
            .get(key);
          get.onsuccess = async () => {
            const blob = get.result as Blob | undefined;
            open.result.close();
            if (!blob) return resolve(null);
            const bytes = new Uint8Array(await blob.arrayBuffer());
            resolve(btoa(String.fromCharCode(...bytes)));
          };
        };
      }),
    { key: `${business}:${document}` },
  );
}

test("a receipt goes up encrypted, and a second member's browser downloads, decrypts and shows the same bytes", async ({
  browser,
}) => {
  const OWNER = freshKey();
  const owner = await browser.newContext();
  await installSigner(owner, OWNER);
  const page = await owner.newPage();
  const business = await postedWithReceipt(page, "Kiln Supply Depot");

  // The service holds ciphertext only: not the file, nor any part of it.
  const onDisk = readFileSync(storedFile(business));
  expect(onDisk.length).toBe(png.length + 16);
  expect(onDisk.includes(png.subarray(0, 16))).toBe(false);
  const document = (await books(OWNER, business)).documents[0];
  expect(document.stored?.hash).toMatch(/^[0-9a-f]{64}$/);

  const { context, page: other } = await memberBrowser(
    browser,
    OWNER,
    business,
  );
  await viewPosted(other, "Kiln Supply Depot");
  await expect(
    other.getByRole("img", { name: "Receipt evidence: kiln-receipt.png" }),
  ).toBeVisible();
  await expect(
    other.getByText("This receipt was attached on another device"),
  ).toHaveCount(0);
  // Decrypted in that browser to the very bytes attached, and kept there.
  expect(await heldFile(other, business, document.id)).toBe(
    png.toString("base64"),
  );
  await owner.close();
  await context.close();
});

test("a receipt from before storage still says it was attached on another device", async ({
  browser,
}) => {
  const OWNER = freshKey();
  const owner = await browser.newContext();
  await installSigner(owner, OWNER);
  const page = await owner.newPage();
  await signIn(page);
  await createBusiness(page, "Fernhill Ceramics");
  const business = await openBusiness(page);
  // As an older browser sent it: a reference with no stored copy.
  const draft = await assistantDraft(OWNER, business, "Clay Barn");
  const ack = await propose(OWNER, business, {
    type: "SaveDraft",
    draft: { ...draft, documentIds: ["doc-old"] },
    documents: [
      {
        id: "doc-old",
        sha256: "5e".repeat(32),
        name: "old.png",
        mime: "image/png",
        size: 68,
      },
    ],
  });
  expect(ack.status).toBe("accepted");
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await page.getByRole("button", { name: /Expense inbox/ }).click();
  await page
    .getByRole("button", { name: "Review Clay Barn", exact: true })
    .click();
  await expect(
    page.getByRole("heading", {
      name: "This receipt was attached on another device",
    }),
  ).toBeVisible();
  await owner.close();
});

test("ciphertext tampered on the service's disk is refused with a clear error, and nothing is shown", async ({
  browser,
}) => {
  const OWNER = freshKey();
  const owner = await browser.newContext();
  await installSigner(owner, OWNER);
  const page = await owner.newPage();
  const business = await postedWithReceipt(page, "Kiln Supply Depot");

  const file = storedFile(business);
  const bytes = readFileSync(file);
  bytes[bytes.length - 20] ^= 0x01;
  writeFileSync(file, bytes);

  const { context, page: other } = await memberBrowser(
    browser,
    OWNER,
    business,
  );
  await viewPosted(other, "Kiln Supply Depot");
  await expect(
    other.getByRole("heading", { name: "This receipt can't be shown" }),
  ).toBeVisible();
  await expect(other.getByRole("alert").first()).toContainText(
    "altered or damaged",
  );
  await expect(other.locator("img.receipt-image, object")).toHaveCount(0);
  const document = (await books(OWNER, business)).documents[0];
  expect(await heldFile(other, business, document.id)).toBeNull();
  await owner.close();
  await context.close();
});

test("a receipt attached offline is uploaded before its proposal when back online", async ({
  browser,
}) => {
  const OWNER = freshKey();
  const owner = await browser.newContext();
  await installSigner(owner, OWNER);
  const page = await owner.newPage();
  await signIn(page);
  await createBusiness(page, "Willowbank Studio");
  const business = await openBusiness(page);

  await offline(page);
  await recordExpense(page, {
    vendor: "Slip Casting Co",
    amount: "12.30",
    approve: false,
    receipt,
  });
  await status(page, "Offline · 1 waiting");

  const sent: string[] = [];
  const log = (r: Request) => {
    const path = new URL(r.url()).pathname;
    if (r.method() === "PUT" && path.startsWith("/v1/blobs/"))
      sent.push("blob");
    if (r.method() === "POST" && path === "/v1/proposals")
      sent.push("proposal");
  };
  page.on("request", log);
  await online(page);
  await status(page, "Synced");
  expect(sent).toEqual(["blob", "proposal"]);

  const { context, page: other } = await memberBrowser(
    browser,
    OWNER,
    business,
  );
  await other.getByRole("button", { name: /Expense inbox/ }).click();
  await other
    .getByRole("button", { name: "Review Slip Casting Co", exact: true })
    .click();
  await expect(
    other.getByRole("img", { name: "Receipt evidence: kiln-receipt.png" }),
  ).toBeVisible();
  await owner.close();
  await context.close();
});

test("a refused upload stops its proposal with why, and a retry sends the same ciphertext", async ({
  browser,
}) => {
  const OWNER = freshKey();
  const owner = await browser.newContext();
  await installSigner(owner, OWNER);
  const page = await owner.newPage();
  await signIn(page);
  await createBusiness(page, "Willowbank Studio");
  const business = await openBusiness(page);

  await offline(page);
  await recordExpense(page, {
    vendor: "Slip Casting Co",
    amount: "12.30",
    approve: false,
    receipt,
  });
  await status(page, "Offline · 1 waiting");

  // Back online, but the service refuses the upload.
  const hashes: string[] = [];
  let proposals = 0;
  page.on("request", (r) => {
    const path = new URL(r.url()).pathname;
    if (r.method() === "PUT" && path.startsWith("/v1/blobs/"))
      hashes.push(path.split("/").pop()!);
    if (r.method() === "POST" && path === "/v1/proposals") proposals++;
  });
  await page.unroute(`${SERVICE_URL}/**`);
  await page.route(`${SERVICE_URL}/v1/blobs/**`, (route) =>
    route.fulfill({
      status: 507,
      contentType: "application/json",
      headers: { "Access-Control-Allow-Origin": "*" },
      body: JSON.stringify({
        error: "This business's receipt storage is full.",
        code: "quota",
      }),
    }),
  );
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await status(page, "Sync error");
  expect(proposals).toBe(0);
  await page.getByRole("button", { name: /Sync status/ }).click();
  const panel = page.getByRole("dialog");
  const item = panel.locator(".waiting-list li");
  await expect(item).toContainText(
    "The receipt couldn't be uploaded: This business's receipt storage is full.",
  );

  // Once the service takes it, the retry goes through, with the same file.
  await page.unroute(`${SERVICE_URL}/v1/blobs/**`);
  await item.getByRole("button", { name: "Redo from current books" }).click();
  await expect(item).toHaveCount(0);
  await panel.getByRole("button", { name: "Close dialog" }).click();
  await status(page, "Synced");
  expect(proposals).toBe(1);
  expect(new Set(hashes).size).toBe(1);
  const stored = (await books(OWNER, business)).documents[0].stored;
  expect(stored?.hash).toBe(hashes[0]);
  await owner.close();
});
