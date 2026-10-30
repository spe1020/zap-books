import { defineConfig } from "@playwright/test";
import { existsSync } from "node:fs";
import {
  APP_PORT,
  APP_URL,
  CLOCK_FILE,
  SERVICE,
  SERVICE_PORT,
  SERVICE_URL,
} from "./tests/remote/keys";

// The app against a locally started ledger service and Postgres
// (`npm run test:e2e:remote`, DATABASE_URL set). Each test injects a
// test-only NIP-07 signer (tests/remote/signer.ts); the app itself has none.
const chromium =
  process.env.CHROMIUM_PATH ||
  (existsSync("/usr/bin/chromium") ? "/usr/bin/chromium" : undefined);
export default defineConfig({
  testDir: "./tests/remote",
  fullyParallel: false,
  workers: 1,
  timeout: 60000,
  use: {
    baseURL: APP_URL,
    headless: true,
    launchOptions: { executablePath: chromium },
    screenshot: "only-on-failure",
  },
  webServer: [
    {
      command: `node scripts/e2e-service.mjs ${SERVICE.secret} ${SERVICE_PORT} ${APP_URL} ${CLOCK_FILE}`,
      url: `${SERVICE_URL}/health`,
      timeout: 300_000,
      reuseExistingServer: false,
    },
    {
      command: `npx vite --host 127.0.0.1 --port ${APP_PORT} --strictPort`,
      url: APP_URL,
      reuseExistingServer: false,
      env: {
        VITE_SERVICE_URL: SERVICE_URL,
        VITE_SERVICE_PUBKEY: SERVICE.pubkey,
      },
    },
  ],
});
