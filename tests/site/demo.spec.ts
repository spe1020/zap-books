import { test, expect } from "@playwright/test";

const widths = [
  { width: 360, height: 800 },
  { width: 1280, height: 900 },
];

for (const viewport of widths) {
  test.describe(`at ${viewport.width} px`, () => {
    test.use({ viewport });

    test("/demo/ opens the demo, with no sign-in and no failed requests", async ({
      page,
    }) => {
      const failed: string[] = [];
      const requests: string[] = [];
      const errors: string[] = [];
      page.on("request", (r) => requests.push(r.url()));
      page.on("requestfailed", (r) => failed.push(r.url()));
      page.on("response", (r) => r.status() >= 400 && failed.push(r.url()));
      page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
      page.on("pageerror", (e) => errors.push(e.message));

      await page.goto("/demo/");
      await expect(
        page.getByRole("heading", { name: "Your business, at a glance." }),
      ).toBeVisible();
      const banner = page.getByRole("note");
      await expect(banner).toContainText(
        "Demo with fictional data. Everything stays in your browser.",
      );
      await expect(
        banner.getByRole("link", { name: "About Zap Books" }),
      ).toHaveAttribute("href", "/");
      await expect(page.getByRole("button", { name: /sign in/i })).toHaveCount(
        0,
      );
      await expect(page.getByText(/ledger service/i)).toHaveCount(0);
      await page.evaluate(() => document.fonts.ready);
      await page.waitForLoadState("networkidle");

      expect(failed).toEqual([]);
      expect(errors).toEqual([]);
      for (const url of requests)
        expect(new URL(url).origin, url).toBe("http://127.0.0.1:4180");
      // No WebAssembly: that's the ledger service's protocol, unused here.
      expect(requests.filter((u) => u.endsWith(".wasm"))).toEqual([]);
    });
  });
}

test("the demo offers no receipt reading, and asks for none", async ({
  page,
}) => {
  const requests: string[] = [];
  page.on("request", (r) => requests.push(r.url()));
  await page.goto("/demo/");
  await page.getByRole("button", { name: "New expense", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("Attach receipt", { exact: true }).setInputFiles({
    name: "receipt.png",
    mimeType: "image/png",
    buffer: Buffer.from(
      "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aOCcAAAAASUVORK5CYII=",
      "base64",
    ),
  });
  await expect(
    dialog.getByRole("img", { name: "Receipt evidence: receipt.png" }),
  ).toBeVisible();
  await expect(
    dialog.getByRole("button", { name: "Read receipt" }),
  ).toHaveCount(0);
  await expect(dialog.getByText(/receipt reading/i)).toHaveCount(0);
  for (const url of requests)
    expect(new URL(url).origin, url).toBe("http://127.0.0.1:4180");
});

test("the demo build keeps its sample assistant, labeled as no live AI", async ({
  page,
}) => {
  await page.goto("/demo/");
  await page.getByLabel("Switch business").selectOption("fieldwork");
  await page.getByRole("button", { name: "Ask assistant" }).click();
  const panel = page.getByRole("dialog", { name: "Your workspace assistant" });
  await expect(panel).toContainText("Deterministic demo · no live AI");
  await panel.getByRole("button", { name: "What needs review?" }).click();
  await expect(panel.locator(".assistant-answer")).toContainText(
    "Fieldwork Landscapes",
  );
});
