/**
 * Screenshots of the bitcoin screens (decision 0005, phase 1) for PR
 * descriptions, from the fictional demo. Run with SCREENSHOTS=1 npx
 * playwright test tests/screenshots-bitcoin.spec.ts; otherwise skipped.
 * The clock is fixed in 2031 before the demo is made, so no real date
 * appears.
 */
import { expect, test, type Page } from "@playwright/test";

test.skip(!process.env.SCREENSHOTS, "Set SCREENSHOTS=1 to take them.");

const dir = "docs/screenshots/bitcoin";
const widths = {
  desktop: { width: 1280, height: 900 },
  phone: { width: 390, height: 844 },
} as const;

for (const [name, viewport] of Object.entries(widths))
  test(`bitcoin screens, ${name}`, async ({ page }) => {
    await page.clock.setFixedTime(new Date("2031-03-20T15:00:00"));
    await page.setViewportSize(viewport);
    await page.goto("/");
    const card = page.getByRole("region", { name: "Bitcoin" });
    await expect(card).toBeVisible();
    const shot = (what: string) =>
      page.screenshot({ path: `${dir}/${what}-${name}.png` });

    await card.screenshot({ path: `${dir}/overview-card-${name}.png` });

    // A sats expense, with its dollar value and the rate it implies.
    await page
      .getByRole("button", { name: "New expense", exact: true })
      .click();
    const expense = page.getByRole("dialog");
    await expense.getByLabel("Vendor", { exact: true }).fill("Tidewater Relay");
    await expense
      .getByLabel("Description", { exact: true })
      .fill("Relay add-on, paid over Lightning");
    await expense.getByLabel("Currency").selectOption("BTC");
    await expense.getByLabel("Amount (sats)").fill("2100");
    await expense
      .getByLabel("Dollar value at time of payment (USD)")
      .fill("1.26");
    await expense
      .getByRole("combobox", { name: "Category", exact: true })
      .selectOption({ label: "Software & subscriptions" });
    await expect(
      expense.getByText("Implied rate: $60,000.00 per BTC."),
    ).toBeVisible();
    // Both fields in view, at either width.
    await expense
      .getByLabel("Dollar value at time of payment (USD)")
      .evaluate((el) => el.scrollIntoView({ block: "center" }));
    await shot("expense-form");
    await close(page);

    // Buying bitcoin.
    await openNav(page, "Transactions");
    await page.getByRole("button", { name: "Buy or sell bitcoin" }).click();
    const exchange = page.getByRole("dialog", { name: "Buy or sell bitcoin" });
    await exchange.getByLabel("Bitcoin bought (sats)").fill("100000");
    await exchange.getByLabel("Amount paid (USD)").fill("61.00");
    await exchange.getByLabel("Time, UTC (optional)").fill("14:30");
    await exchange.getByLabel("Purpose").fill("Sats for relay subscriptions");
    await exchange.getByLabel("Purpose").blur();
    await shot("exchange-form");
    await close(page);

    // Wallet setup: the account, then its opening balance.
    await openNav(page, "Settings");
    await page.getByRole("button", { name: "Add account" }).click();
    const add = page.getByRole("dialog", { name: "Add account" });
    await add.getByLabel("Name").fill("Cold storage");
    await add.getByLabel("Type").selectOption({ label: "Bitcoin wallet" });
    await add.getByLabel("Name").blur();
    await shot("settings-add-wallet");
    await close(page);
    await page
      .getByRole("button", { name: "Set opening balance for Lightning wallet" })
      .click();
    const opening = page.getByRole("dialog");
    await opening.getByLabel("As of").fill("2031-01-31");
    await opening.getByLabel("Bitcoin in the wallet (sats)").fill("50000");
    await opening
      .getByLabel("Dollar value at time of opening (USD)")
      .fill("30.00");
    await opening.getByLabel("As of").blur();
    await shot("settings-wallet-opening");
  });

async function close(page: Page) {
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toHaveCount(0);
}

/** A main view, through the menu on a phone. */
async function openNav(page: Page, view: string) {
  const menu = page.getByRole("button", { name: "Open navigation" });
  if (await menu.isVisible()) await menu.click();
  await page.getByRole("button", { name: view, exact: true }).click();
}
