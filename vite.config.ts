import { loadEnv, type Plugin } from "vite";
import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import { contentSecurityPolicy } from "./src/csp";
import { DEFAULT_EXTRACTOR_URL } from "./src/config";

/** Puts the Content-Security-Policy first in `<head>`, before any script. */
export function csp(policy: string): Plugin {
  return {
    name: "content-security-policy",
    transformIndexHtml: () => [
      {
        tag: "meta",
        attrs: { "http-equiv": "Content-Security-Policy", content: policy },
        injectTo: "head-prepend",
      },
    ],
  };
}

export default defineConfig(({ command, mode }) => {
  const env = loadEnv(mode, process.cwd(), "VITE_");
  // A demo-only build (src/demo.ts) reads no receipts and has no service.
  const demoOnly = env.VITE_DEMO_ONLY === "true";
  const policy = contentSecurityPolicy({
    dev: command === "serve",
    extractorUrl: demoOnly
      ? undefined
      : env.VITE_EXTRACTOR_URL || DEFAULT_EXTRACTOR_URL,
    serviceUrl: demoOnly ? undefined : env.VITE_SERVICE_URL || undefined,
  });
  return {
    plugins: [react(), csp(policy)],
    test: {
      include: [
        "src/**/*.test.ts",
        "scripts/*.test.mjs",
        "scripts/hooks/**/*.test.mjs",
      ],
    },
  };
});
