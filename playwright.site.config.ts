import { defineConfig } from "@playwright/test";
import { existsSync } from "node:fs";
const chromium =
  process.env.CHROMIUM_PATH ||
  (existsSync("/usr/bin/chromium") ? "/usr/bin/chromium" : undefined);
// zapbooks.io as GitHub Pages serves it, built with an invented form
// service (tests/site/form-service.json) that the tests stub.
export default defineConfig({
  testDir: "./tests/site",
  fullyParallel: false,
  workers: 1,
  timeout: 30000,
  use: {
    baseURL: "http://127.0.0.1:4180",
    headless: true,
    launchOptions: { executablePath: chromium },
    screenshot: "only-on-failure",
  },
  webServer: {
    command:
      "node scripts/build-pages.mjs --config tests/site/form-service.json && node scripts/serve-static.mjs dist-pages 4180",
    url: "http://127.0.0.1:4180",
    reuseExistingServer: false,
  },
});
