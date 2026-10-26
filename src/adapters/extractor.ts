import type {
  ExtractionAdapter,
  ExtractionResult,
  ExtractorHealth,
} from "../domain/types";
import { currencies, extractionFields } from "../domain/types";

/** The local extractor (crates/extractor). Override with VITE_EXTRACTOR_URL. */
export const extractorUrl = (
  (import.meta.env.VITE_EXTRACTOR_URL as string | undefined) ||
  "http://127.0.0.1:8787"
).replace(/\/$/, "");

const UNAVAILABLE =
  "Receipt reading isn't running on this computer. Enter the details manually.";

const statuses = ["found", "unclear", "absent"];

/** The service validates everything, but its answer is still checked here. */
export function isExtractionResult(x: unknown): x is ExtractionResult {
  if (!x || typeof x !== "object") return false;
  const r = x as Record<string, unknown>;
  const fields = r.fields as Record<string, unknown> | undefined;
  if (!fields || typeof fields !== "object") return false;
  return (
    extractionFields.every((name) => {
      const f = fields[name] as Record<string, unknown> | undefined;
      return (
        !!f &&
        statuses.includes(f.status as string) &&
        typeof f.evidence === "string" &&
        (f.status === "found" ? f.value !== null : f.value === null)
      );
    }) &&
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
  async extract(document, categories) {
    const form = new FormData();
    form.append("file", document.blob, document.name);
    form.append(
      "categories",
      JSON.stringify(categories.map(({ id, name }) => ({ id, name }))),
    );
    form.append("currencies", JSON.stringify(currencies));
    let response: Response;
    try {
      response = await fetch(`${extractorUrl}/extract`, {
        method: "POST",
        body: form,
      });
    } catch {
      throw new Error(UNAVAILABLE);
    }
    const body: unknown = await response.json().catch(() => null);
    if (!response.ok) {
      const message = (body as { error?: unknown } | null)?.error;
      throw new Error(
        typeof message === "string"
          ? message
          : "Receipt reading failed on this computer. Enter the details manually.",
      );
    }
    if (!isExtractionResult(body))
      throw new Error(
        "Receipt reading returned an answer the app can't use. Enter the details manually.",
      );
    return body;
  },
};
