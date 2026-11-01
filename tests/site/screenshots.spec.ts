/**
 * Screenshots of zapbooks.io and its demo for PR descriptions. Run with
 * SCREENSHOTS=1 npm run test:e2e:site -- screenshots; otherwise skipped.
 * The clock is fixed in 2031 before the demo is made, so no real date
 * appears.
 */
import { expect, test } from "@playwright/test";

test.skip(!process.env.SCREENSHOTS, "Set SCREENSHOTS=1 to take them.");

const dir = "docs/screenshots/site";
const widths = {
  desktop: { width: 1280, height: 900 },
  phone: { width: 360, height: 800 },
} as const;

for (const [name, viewport] of Object.entries(widths))
  test(`the site and the demo, ${name}`, async ({ page }) => {
    await page.clock.setFixedTime(new Date("2031-03-20T15:00:00"));
    await page.setViewportSize(viewport);

    await page.goto("/");
    await expect(page.getByRole("heading", { level: 1 })).toBeVisible();
    await page.evaluate(() => document.fonts.ready);
    await page.screenshot({ path: `${dir}/site-${name}.png`, fullPage: true });

    await page.goto("/demo/");
    await expect(
      page.getByRole("heading", { name: "Your business, at a glance." }),
    ).toBeVisible();
    await page.evaluate(() => document.fonts.ready);
    await page.screenshot({ path: `${dir}/demo-${name}.png` });
  });
