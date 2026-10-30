import { readFileSync } from "node:fs";
import { describe, expect, test } from "vitest";
import { contentSecurityPolicy } from "./csp";
import { csp } from "../vite.config";

const directives = (policy: string) =>
  Object.fromEntries(
    policy.split("; ").map((d) => {
      const [name, ...values] = d.split(" ");
      return [name, values];
    }),
  );

describe("Content-Security-Policy (WEB-4)", () => {
  test("a build allows only the app's own scripts and the extractor", () => {
    const p = directives(
      contentSecurityPolicy({
        dev: false,
        extractorUrl: "http://127.0.0.1:8787/",
      }),
    );
    expect(p["default-src"]).toEqual(["'self'"]);
    expect(p["script-src"]).toEqual(["'self'"]);
    expect(p["style-src"]).toEqual(["'self'"]);
    expect(p["connect-src"]).toEqual(["'self'", "http://127.0.0.1:8787"]);
    expect(p["img-src"]).toEqual(["'self'", "blob:", "data:"]);
    expect(p["object-src"]).toEqual(["blob:"]);
    expect(p["frame-src"]).toEqual(["blob:"]);
    expect(p["base-uri"]).toEqual(["'none'"]);
    expect(p["form-action"]).toEqual(["'none'"]);
  });

  test("the extractor's own origin is allowed, wherever it is", () => {
    const p = directives(
      contentSecurityPolicy({
        dev: false,
        extractorUrl: "https://reader.invalid:9000/path",
      }),
    );
    expect(p["connect-src"]).toEqual(["'self'", "https://reader.invalid:9000"]);
  });

  test("with a service: exactly its origin, and the app's own WebAssembly", () => {
    const p = directives(
      contentSecurityPolicy({
        dev: false,
        extractorUrl: "http://127.0.0.1:8787",
        serviceUrl: "https://ledger.invalid:8443/base/",
      }),
    );
    expect(p["connect-src"]).toEqual([
      "'self'",
      "http://127.0.0.1:8787",
      "https://ledger.invalid:8443",
    ]);
    expect(p["script-src"]).toEqual(["'self'", "'wasm-unsafe-eval'"]);
  });

  test("only the dev server gets inline scripts and its WebSocket", () => {
    const p = directives(
      contentSecurityPolicy({
        dev: true,
        extractorUrl: "http://127.0.0.1:8787",
      }),
    );
    expect(p["script-src"]).toEqual(["'self'", "'unsafe-inline'"]);
    expect(p["connect-src"]).toContain("ws:");
  });

  test("the policy is the first thing in <head>", () => {
    const plugin = csp("default-src 'self'");
    const transform = plugin.transformIndexHtml as () => unknown;
    expect(transform()).toEqual([
      {
        tag: "meta",
        attrs: {
          "http-equiv": "Content-Security-Policy",
          content: "default-src 'self'",
        },
        injectTo: "head-prepend",
      },
    ]);
  });
});

test("dev and preview listen on this computer only unless asked (WEB-5)", () => {
  const { scripts } = JSON.parse(
    readFileSync(new URL("../package.json", import.meta.url), "utf8"),
  ) as { scripts: Record<string, string> };
  expect(scripts.dev).toBe("vite --host 127.0.0.1");
  expect(scripts.preview).toBe("vite preview --host 127.0.0.1");
  // Cloud development opts in by name.
  for (const [name, command] of Object.entries(scripts))
    if (command.includes("0.0.0.0")) expect(name).toMatch(/:cloud$/);
});
