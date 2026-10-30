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
  const policy = contentSecurityPolicy({
    dev: command === "serve",
    extractorUrl: env.VITE_EXTRACTOR_URL || DEFAULT_EXTRACTOR_URL,
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
