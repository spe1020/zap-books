// Renders a fictional sample receipt (docs/samples/*.html) to PNG and/or PDF
// with the Playwright Chromium the e2e tests already use.
// Usage: node scripts/render-sample.mjs <name> [png] [pdf]
import { chromium } from "@playwright/test";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

const [name, ...formats] = process.argv.slice(2);
if (!name)
  throw new Error("Usage: node scripts/render-sample.mjs <name> [png] [pdf]");
const html = resolve("docs/samples", `${name}.html`);
const browser = await chromium.launch();
try {
  const page = await browser.newPage({
    viewport: { width: 640, height: 900 },
    deviceScaleFactor: 2,
  });
  await page.goto(pathToFileURL(html).href);
  if (formats.includes("png"))
    await page.locator(".receipt").screenshot({
      path: resolve("docs/samples", `${name}.png`),
    });
  if (formats.includes("pdf"))
    await page.pdf({
      path: resolve("docs/samples", `${name}.pdf`),
      width: "640px",
      printBackground: true,
    });
} finally {
  await browser.close();
}
