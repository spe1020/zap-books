// WASM parity, the WebAssembly half: every case in
// fixtures/protocol/wasm-parity.json must give exactly its recorded answer
// through the build in ../pkg. tests/parity.rs checks the same cases
// natively, so the two builds agree byte for byte. Run with
// `npm run test:wasm`, which builds the package first.

import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { beforeAll, describe, expect, test } from "vitest";
import * as wasm from "../pkg/protocol_wasm.js";

const fixture = JSON.parse(
  readFileSync(
    new URL("../../../fixtures/protocol/wasm-parity.json", import.meta.url),
    "utf8",
  ),
);

beforeAll(() => {
  wasm.initSync({
    module: readFileSync(
      new URL("../pkg/protocol_wasm_bg.wasm", import.meta.url),
    ),
  });
});

function call(name, request) {
  try {
    return { answer: wasm[name](JSON.stringify(request)) };
  } catch (error) {
    return { error: error.message };
  }
}

describe("every case gives its recorded answer", () => {
  test("the fixture has the cases", () => {
    expect(fixture.cases.length).toBeGreaterThan(30);
  });
  for (const c of fixture.cases) {
    test(c.name, () => {
      const want = "answer" in c ? { answer: c.answer } : { error: c.error };
      expect(call(c.function, c.request)).toEqual(want);
    });
  }
});

describe("what a fixed answer can't cover", () => {
  const business = "cd".repeat(32);
  const request = JSON.stringify({
    business,
    baseSeq: 3,
    command: { type: "Approve", expenseId: "jl-draft-1" },
  });

  test("a built proposal commits to its plaintext, with a fresh salt", () => {
    const parts = JSON.parse(wasm.buildProposal(request));
    expect(parts.kind).toBe(4130);
    const payload = JSON.parse(parts.plaintext);
    expect(payload).toMatchObject({ v: 1, business, baseSeq: 3 });
    expect(payload.salt).toMatch(/^[0-9a-f]{64}$/);
    const hash = createHash("sha256").update(parts.plaintext).digest("hex");
    expect(parts.tags).toEqual([
      ["b", business],
      ["cmd", "Approve"],
      ["base_seq", "3"],
      ["payload", hash],
    ]);
    expect(JSON.parse(wasm.buildProposal(request)).plaintext).not.toBe(
      parts.plaintext,
    );
  });

  test("a recovery keypair is fresh each time", () => {
    const a = JSON.parse(wasm.generateRecoveryKeypair());
    const b = JSON.parse(wasm.generateRecoveryKeypair());
    expect(a.secretKey).toMatch(/^[0-9a-f]{64}$/);
    expect(a.publicKey).toMatch(/^[0-9a-f]{64}$/);
    expect(a.secretKey).not.toBe(b.secretKey);
  });
});
