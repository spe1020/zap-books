import { afterEach, describe, expect, test, vi } from "vitest";

// The flag is read when the modules load, so each case loads them afresh.
async function load(env: Record<string, string>) {
  vi.resetModules();
  for (const [name, value] of Object.entries(env)) vi.stubEnv(name, value);
  return {
    remote: await import("./remote/config"),
    extractor: await import("./adapters/extractor"),
  };
}

const service = {
  VITE_SERVICE_URL: "https://ledger.invalid",
  VITE_SERVICE_PUBKEY: "a".repeat(64),
};

afterEach(() => {
  vi.unstubAllEnvs();
  vi.unstubAllGlobals();
});

describe("a demo-only build (VITE_DEMO_ONLY)", () => {
  test("without the flag, a configured service starts at sign-in", async () => {
    const { remote, extractor } = await load(service);
    expect(remote.serviceConfigured).toBe(true);
    expect(extractor.receiptReading).toBe(true);
  });

  test("with it, there's no sign-in, even with a service configured", async () => {
    const { remote } = await load({ ...service, VITE_DEMO_ONLY: "true" });
    expect(remote.serviceConfigured).toBe(false);
  });

  test("with it, receipt reading is off and never asks the extractor", async () => {
    const fetch = vi.fn();
    vi.stubGlobal("fetch", fetch);
    const { extractor } = await load({ VITE_DEMO_ONLY: "true" });
    expect(extractor.receiptReading).toBe(false);
    expect((await extractor.extractor.health()).ready).toBe(false);
    expect(fetch).not.toHaveBeenCalled();
  });
});
