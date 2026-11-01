import { defineConfig } from "@playwright/test";
import { existsSync } from "node:fs";
const chromium =
  process.env.CHROMIUM_PATH ||
  (existsSync("/usr/bin/chromium") ? "/usr/bin/chromium" : undefined);
export default defineConfig({
  testDir: "./tests",
  // Against the ledger service: playwright.remote.config.ts. The landing
  // site: playwright.site.config.ts.
  testIgnore: ["remote/**", "site/**"],
  fullyParallel: false,
  workers: 1,
  timeout: 30000,
  use: {
    baseURL: "http://127.0.0.1:5173",
    headless: true,
    // No "--no-sandbox" here: Playwright already launches Chromium without
    // its sandbox unless `chromiumSandbox: true` is set (its default is
    // false), so the flag only repeated that, and CI passes without it.
    launchOptions: { executablePath: chromium },
    screenshot: "only-on-failure",
  },
  webServer: {
    command: "npm run dev -- --port 5173 --strictPort",
    url: "http://127.0.0.1:5173",
    reuseExistingServer: !process.env.CI,
  },
});
