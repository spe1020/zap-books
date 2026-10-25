// Runs only the parity fixture generator; `npm test` keeps its own config.
import { defineConfig } from "vitest/config";

export default defineConfig({
  test: { include: ["scripts/gen-parity-fixtures.test.ts"] },
});
