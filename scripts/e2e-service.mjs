// Starts the ledger service for the remote e2e tests
// (playwright.remote.config.ts): an invented test key in a fresh temporary
// file (mode 0600), the database from DATABASE_URL, and CORS for the test
// app's origin. Never use this key anywhere else. It is built with the
// test-only `e2e-clock` feature: the service's clock is moved by the
// milliseconds written in the clock file (tests/remote/clock.ts). Its blob
// store is the directory given, emptied first, so the tests can reach the
// stored files.

import { spawn } from "node:child_process";
import { chmodSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const [secret, port, appOrigin, clockFile, blobDir] = process.argv.slice(2);
if (!process.env.DATABASE_URL) {
  console.error(
    "The remote e2e tests need Postgres: set DATABASE_URL, for example postgres://postgres:postgres@127.0.0.1:5432/postgres",
  );
  process.exit(1);
}
const dir = mkdtempSync(join(tmpdir(), "zb-e2e-"));
const keyFile = join(dir, "service.key");
writeFileSync(keyFile, secret, { mode: 0o600 });
chmodSync(keyFile, 0o600);
writeFileSync(clockFile, "0");
rmSync(blobDir, { recursive: true, force: true });

const url = `http://127.0.0.1:${port}`;
const child = spawn(
  "cargo",
  [
    "run",
    "--locked",
    "-q",
    "-p",
    "ledger-service",
    "--features",
    "e2e-clock",
    "--",
    "--key-file",
    keyFile,
    "--public-url",
    url,
    "--listen",
    `127.0.0.1:${port}`,
    "--allowed-origin",
    appOrigin,
    "--blob-dir",
    blobDir,
  ],
  {
    stdio: "inherit",
    env: {
      RUST_LOG: "warn",
      ...process.env,
      ZAPBOOKS_E2E_CLOCK_FILE: clockFile,
    },
  },
);
for (const signal of ["SIGINT", "SIGTERM"])
  process.on(signal, () => child.kill(signal));
child.on("exit", (code) => process.exit(code ?? 0));
