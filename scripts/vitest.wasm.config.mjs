// Runs only the WASM parity test, which needs crates/protocol-wasm/pkg built
// first (`npm run test:wasm` does both); `npm test` keeps its own config.
import { defineConfig } from "vitest/config";

export default defineConfig({
  test: { include: ["crates/protocol-wasm/tests/*.test.mjs"] },
});
