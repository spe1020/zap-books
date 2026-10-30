import type {
  Currency,
  ExtractionAdapter,
  ExtractionFieldName,
  ExtractionFields,
  ExtractionResult,
  ExtractorHealth,
} from "../domain/types";
import {
  currencies,
  extractionFields,
  extractionLists,
  type ExtractionListName,
} from "../domain/types";
import { validDate } from "../domain/ledger";
import { DEFAULT_EXTRACTOR_URL } from "../config";

/** The local extractor (crates/extractor). Override with VITE_EXTRACTOR_URL. */
export const extractorUrl = (
  (import.meta.env.VITE_EXTRACTOR_URL as string | undefined) ||
  DEFAULT_EXTRACTOR_URL
).replace(/\/$/, "");

/** Whether `url` is on this computer: a loopback host. */
export function isLoopback(url: string): boolean {
  const host = new URL(url).hostname;
  return (
    host === "localhost" || host === "[::1]" || /^127(\.\d{1,3}){3}$/.test(host)
  );
}

/**
 * Where receipts are read, for the sentences that say so: "on this
 * computer" only when the extractor really is on it.
 */
export function readingPlace(url = extractorUrl): string {
  return isLoopback(url) ? "on this computer" : `at ${new URL(url).host}`;
}

/** What the Review screen says once the extractor is ready. */
export function readyNote(model: string, url = extractorUrl): string {
  return isLoopback(url)
    ? `Reads the receipt with ${model} on this computer. Nothing is sent anywhere else.`
    : `Reads the receipt with ${model} at ${new URL(url).host}. The receipt is sent there to be read.`;
}

const UNAVAILABLE = `Receipt reading isn't running ${readingPlace()}. Enter the details manually.`;
const TOO_SLOW =
  "Receipt reading took too long. Try again, or enter the details manually.";
/**
 * How long the app waits for a reading: above the service's own 90-second
 * limit, so a working service always answers first.
 */
export const EXTRACT_TIMEOUT_MS = 120_000;

const statuses = ["found", "unclear", "absent"];

const isList = (name: string): name is ExtractionListName =>
  (extractionLists as readonly string[]).includes(name);

const isString = (v: unknown) => typeof v === "string";
const isMinor = (v: unknown) => Number.isSafeInteger(v);
/**
 * What a found value must be, field by field: the types the app stores.
 * Anything else answering on the extractor's port could otherwise put, say,
 * an unsupported currency into a draft.
 */
const valueChecks: Record<ExtractionFieldName, (v: unknown) => boolean> = {
  vendor: isString,
  date: (v) => isString(v) && validDate(v as string),
  total: isMinor,
  currency: (v) => currencies.includes(v as Currency),
  reference: isString,
  subtotal: isMinor,
  tax: isMinor,
  fees: isMinor,
  discounts: isMinor,
  tip: isMinor,
  category: isString,
};

/** A list's lines carry amounts only when it is found. */
function isLines(f: Record<string, unknown>): boolean {
  const lines = f.lines;
  if (!Array.isArray(lines)) return false;
  if (f.status === "absent" && lines.length > 0) return false;
  if (f.status === "found" && lines.length === 0) return false;
  return lines.every((line: unknown) => {
    if (!line || typeof line !== "object") return false;
    const l = line as Record<string, unknown>;
    return (
      typeof l.evidence === "string" &&
      (f.status === "found"
        ? Number.isSafeInteger(l.amount)
        : l.amount === null)
    );
  });
}

/** Every field's shape, and a found value's type. */
export function isExtractionFields(x: unknown): x is ExtractionFields {
  if (!x || typeof x !== "object") return false;
  const fields = x as Record<string, unknown>;
  return extractionFields.every((name) => {
    const f = fields[name] as Record<string, unknown> | undefined;
    return (
      !!f &&
      typeof f === "object" &&
      statuses.includes(f.status as string) &&
      (isList(name) ? isLines(f) : typeof f.evidence === "string") &&
      (f.status === "found" ? valueChecks[name](f.value) : f.value === null)
    );
  });
}

/**
 * The service validates everything, but its answer is still checked here:
 * every field's shape, and a found value's type.
 */
export function isExtractionResult(x: unknown): x is ExtractionResult {
  if (!x || typeof x !== "object") return false;
  const r = x as Record<string, unknown>;
  return (
    isExtractionFields(r.fields) &&
    Array.isArray(r.warnings) &&
    r.warnings.every((w) => typeof w === "string") &&
    typeof r.model === "string" &&
    typeof r.promptVersion === "string"
  );
}

export const extractor: ExtractionAdapter = {
  async health(): Promise<ExtractorHealth> {
    try {
      const response = await fetch(`${extractorUrl}/health`, {
        signal: AbortSignal.timeout(3000),
      });
      const body = (await response.json()) as {
        ready?: boolean;
        model?: string;
      };
      return body.ready && typeof body.model === "string"
        ? { ready: true, model: body.model }
        : { ready: false, reason: UNAVAILABLE };
    } catch {
      return { ready: false, reason: UNAVAILABLE };
    }
  },
  async extract(document, categories, signal) {
    const form = new FormData();
    form.append("file", document.blob, document.name);
    form.append(
      "categories",
      JSON.stringify(categories.map(({ id, name }) => ({ id, name }))),
    );
    form.append("currencies", JSON.stringify(currencies));
    // The caller's signal cancels; the timeout bounds a service that never
    // answers. A cancelled read rethrows the AbortError for the caller.
    const timeout = AbortSignal.timeout(EXTRACT_TIMEOUT_MS);
    const combined = signal ? AbortSignal.any([signal, timeout]) : timeout;
    const failed = (e: unknown): never => {
      if (signal?.aborted) throw e;
      throw new Error(timeout.aborted ? TOO_SLOW : UNAVAILABLE);
    };
    let response: Response;
    try {
      response = await fetch(`${extractorUrl}/extract`, {
        method: "POST",
        body: form,
        signal: combined,
      });
    } catch (e) {
      return failed(e);
    }
    let body: unknown = null;
    try {
      body = await response.json();
    } catch (e) {
      if (combined.aborted) return failed(e);
    }
    if (!response.ok) {
      const message = (body as { error?: unknown } | null)?.error;
      throw new Error(
        typeof message === "string"
          ? message
          : `Receipt reading failed ${readingPlace()}. Enter the details manually.`,
      );
    }
    if (!isExtractionResult(body))
      throw new Error(
        "Receipt reading returned an answer the app can't use. Enter the details manually.",
      );
    return body;
  },
};
