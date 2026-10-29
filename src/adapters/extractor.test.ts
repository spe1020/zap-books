import { afterEach, describe, expect, test, vi } from "vitest";
import { extractor, isExtractionResult } from "./extractor";
import type { SourceDocument } from "../domain/types";

const found = (value: unknown, evidence = "seen") => ({
  status: "found",
  value,
  evidence,
});
const absent = () => ({ status: "absent", value: null, evidence: "" });
const noLines = () => ({ status: "absent", value: null, lines: [] });

/** A well-formed answer with invented values. */
function answer(fields: Record<string, unknown> = {}) {
  return {
    fields: {
      vendor: found("Invented Stationer"),
      date: found("2026-10-02"),
      total: found(1875),
      currency: found("USD"),
      reference: found("INV-0001"),
      subtotal: found(1700),
      tax: {
        status: "found",
        value: 175,
        lines: [{ amount: 175, evidence: "TAX" }],
      },
      fees: noLines(),
      discounts: noLines(),
      tip: absent(),
      category: found("cat-1"),
      ...fields,
    },
    warnings: [],
    model: "stub",
    promptVersion: "v1",
  };
}

describe("isExtractionResult (WEB-1, EXT-8)", () => {
  test("a well-formed answer passes", () => {
    expect(isExtractionResult(answer())).toBe(true);
  });
  test("a found value must have the type the app stores", () => {
    const bad: [string, unknown][] = [
      ["currency", "zz"],
      ["currency", "CAD"],
      ["currency", "usd"],
      ["currency", 1],
      ["total", 12.5],
      ["total", "1875"],
      ["total", 2 ** 53],
      ["subtotal", NaN],
      ["tip", 1.25],
      ["date", "2026-02-30"],
      ["date", "10/02/2026"],
      ["date", 20261002],
      ["vendor", 42],
      ["vendor", { name: "x" }],
      ["reference", ["INV-0001"]],
      ["category", 7],
    ];
    for (const [field, value] of bad)
      expect(
        isExtractionResult(answer({ [field]: found(value) })),
        `${field}: ${JSON.stringify(value)}`,
      ).toBe(false);
    const list = { status: "found", value: 1.5, lines: [] };
    expect(isExtractionResult(answer({ fees: list }))).toBe(false);
  });
  test("each supported currency passes", () => {
    for (const code of ["USD", "EUR", "GBP"])
      expect(isExtractionResult(answer({ currency: found(code) }))).toBe(true);
  });
});

describe("extract (WEB-2)", () => {
  const document = {
    id: "doc-1",
    name: "receipt.png",
    blob: new Blob(["x"], { type: "image/png" }),
  } as unknown as SourceDocument;
  /** A fetch that never answers, but rejects as fetch does when aborted. */
  const hanging = () =>
    vi.fn(
      (_url: string, init?: RequestInit) =>
        new Promise<Response>((_, reject) => {
          const signal = init?.signal;
          if (signal?.aborted) reject(signal.reason);
          signal?.addEventListener("abort", () => reject(signal.reason));
        }),
    );
  afterEach(() => vi.unstubAllGlobals());

  test("the caller can cancel a reading", async () => {
    const fetch = hanging();
    vi.stubGlobal("fetch", fetch);
    const controller = new AbortController();
    const reading = extractor.extract(document, [], controller.signal);
    controller.abort();
    await expect(reading).rejects.toMatchObject({ name: "AbortError" });
    expect(fetch.mock.calls[0][1]?.signal?.aborted).toBe(true);
  });

  test("a service that never answers times out with a plain message", async () => {
    vi.stubGlobal("fetch", hanging());
    vi.spyOn(AbortSignal, "timeout").mockReturnValue(
      AbortSignal.abort(new DOMException("timed out", "TimeoutError")),
    );
    await expect(extractor.extract(document, [])).rejects.toThrow(
      "Receipt reading took too long. Try again, or enter the details manually.",
    );
    vi.restoreAllMocks();
  });

  test("an answer of the wrong type is refused, not returned", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () =>
        Response.json(answer({ currency: found("zz") }), { status: 200 }),
      ),
    );
    await expect(extractor.extract(document, [])).rejects.toThrow(
      "Receipt reading returned an answer the app can't use. Enter the details manually.",
    );
  });
});
