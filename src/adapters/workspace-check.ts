import type {
  BusinessData,
  EventType,
  ExtractionFields,
  QuarantinedRecord,
  Workspace,
} from "../domain/types";
import {
  currencies,
  fiatCurrencies,
  extractionFields,
  type ExtractionOutcome,
} from "../domain/types";
import { assertBalanced, now } from "../domain/ledger";
import { upgradeExtractionFields } from "../domain/extraction";
import { fiatSources } from "../domain/bitcoin";
import { isExtractionFields } from "./extractor";

/**
 * Checks a workspace read from storage before the app uses it, with the
 * rules the Rust loader applies (PARITY_NOTES D2, D3) where practical: every
 * field's type, enum values, safe-integer amounts, each record in its own
 * business, and every journal balanced against its business's accounts and
 * categories. A record that fails is moved to `quarantine`, with a fixed
 * reason, instead of crashing the app or being counted. It stays in storage
 * there, so the next save keeps it.
 *
 * Throws only when nothing usable is left: the workspace itself is
 * malformed, or every business failed. Storage is then left as it was.
 */
export function checkWorkspace(stored: unknown): Workspace {
  if (!isObject(stored)) throw new Error(DAMAGED);
  if (stored.schemaVersion !== 1)
    throw new Error("Unsupported workspace version.");
  if (
    !Number.isSafeInteger(stored.revision) ||
    !Array.isArray(stored.businesses)
  )
    throw new Error(DAMAGED);
  const at = now();
  const quarantine: QuarantinedRecord[] = Array.isArray(stored.quarantine)
    ? [...stored.quarantine]
    : [];
  const businesses: BusinessData[] = [];
  for (const raw of stored.businesses) {
    const b = checkBusiness(raw, (q) => quarantine.push({ ...q, at }));
    if (b) businesses.push(b);
  }
  if (!businesses.length) throw new Error(DAMAGED);
  return {
    schemaVersion: 1,
    revision: stored.revision as number,
    businesses,
    ...(quarantine.length ? { quarantine } : {}),
  };
}

const DAMAGED =
  "The saved workspace is damaged and can't be opened. It has been left as it was.";

type Fields = Record<string, unknown>;
type Check = (v: unknown) => boolean;

const isObject = (x: unknown): x is Fields =>
  !!x && typeof x === "object" && !Array.isArray(x);
const text: Check = (v) => typeof v === "string";
const optionalText: Check = (v) => v === undefined || typeof v === "string";
const flag: Check = (v) => typeof v === "boolean";
const minor: Check = (v) => Number.isSafeInteger(v);
const texts: Check = (v) => Array.isArray(v) && v.every(text);
const oneOf =
  (...values: readonly unknown[]): Check =>
  (v) =>
    values.includes(v);
const currency = oneOf(...currencies);
const optionalMinor: Check = (v) => v === undefined || minor(v);

// A record of every event type, so a new one can't be left out.
const eventTypes: Record<EventType, true> = {
  "expense.created": true,
  "expense.approved": true,
  "expense.posted": true,
  "expense.matched": true,
  "expense.reversed": true,
  "expense.updated": true,
  "rule.saved": true,
  "settings.updated": true,
  "transfer.recorded": true,
  "transfer.reversed": true,
};
const outcomes: Record<ExtractionOutcome, true> = {
  accepted: true,
  edited: true,
  ignored: true,
  unclear: true,
};

const shape =
  (spec: Record<string, Check>): Check =>
  (v) =>
    isObject(v) && Object.entries(spec).every(([k, ok]) => ok(v[k]));

const importSource = shape({
  id: text,
  businessId: text,
  provider: oneOf("csv", "quickbooks", "zoho"),
  reference: text,
  fingerprint: text,
  date: text,
  amountMinor: minor,
  currency,
  vendor: text,
  category: text,
  filename: text,
});

const fiatValue = shape({
  currency: oneOf(...fiatCurrencies),
  amountMinor: minor,
  rate: text,
  source: oneOf(...fiatSources),
  at: text,
});
const optionalFiatValue: Check = (v) => v === undefined || fiatValue(v);

const line = shape({
  accountId: text,
  categoryId: optionalText,
  debit: minor,
  credit: minor,
});

/** Each list's record shape, past `id` and `businessId`. */
const records: Record<Exclude<keyof BusinessData, "profile">, Check> = {
  accounts: shape({
    name: text,
    kind: oneOf("bank", "card", "expense", "liability", "equity", "wallet"),
    currency,
  }),
  categories: shape({ name: text, accountId: text }),
  vendors: shape({ name: text }),
  documents: shape({
    name: text,
    mime: text,
    size: (v) => minor(v) && (v as number) >= 0,
    blob: (v) => v instanceof Blob,
    kind: oneOf("receipt"),
    createdAt: text,
    demo: flag,
    sha256: optionalText,
  }),
  payments: shape({
    date: text,
    description: text,
    amountMinor: minor,
    currency,
    accountId: text,
    reference: text,
    fingerprint: text,
    draftId: text,
  }),
  drafts: shape({
    vendor: text,
    date: text,
    description: text,
    amountMinor: minor,
    currency,
    fiatValue: optionalFiatValue,
    millisats: optionalMinor,
    categoryId: text,
    accountId: text,
    purpose: text,
    documentIds: texts,
    paymentId: optionalText,
    importSource: (v) => v === undefined || importSource(v),
    status: oneOf("draft", "posted", "corrected"),
    journalId: optionalText,
    correctionOf: optionalText,
    duplicateReason: text,
    createdAt: text,
  }),
  journals: shape({
    date: text,
    currency,
    lines: (v) => Array.isArray(v) && v.every(line),
    kind: oneOf("expense", "transfer", "reversal"),
    expenseId: optionalText,
    reversesId: optionalText,
    memo: text,
    createdAt: text,
    transferType: (v) =>
      v === undefined ||
      oneOf(
        "ownerLoan",
        "ownerContribution",
        "loanRepayment",
        "ownerDraw",
        "betweenAccounts",
        "openingBalance",
        "exchange",
      )(v),
    purpose: optionalText,
    reference: optionalText,
    documentIds: (v) => v === undefined || texts(v),
    correctionOf: optionalText,
    fiatValue: optionalFiatValue,
    millisats: optionalMinor,
  }),
  approvals: shape({
    expenseId: text,
    actor: text,
    at: text,
    duplicateReason: text,
  }),
  rules: shape({
    vendorIncludes: text,
    categoryId: text,
    purpose: text,
    enabled: flag,
  }),
  audits: shape({
    action: text,
    entityId: text,
    at: text,
    actor: text,
    detail: text,
  }),
  events: shape({
    schemaVersion: oneOf(1),
    type: (v) => typeof v === "string" && Object.hasOwn(eventTypes, v),
    actor: text,
    occurredAt: text,
    sourceIds: texts,
    entityId: text,
    payload: (v) =>
      isObject(v) &&
      Object.values(v).every((x) =>
        ["string", "number", "boolean"].includes(typeof x),
      ),
  }),
  extractions: shape({
    documentId: text,
    draftId: text,
    model: text,
    promptVersion: text,
    createdAt: text,
    fields: isExtractionFields,
    accepted: (v) =>
      Array.isArray(v) &&
      v.every((x) => (extractionFields as readonly unknown[]).includes(x)),
    outcomes: (v) =>
      v === null ||
      (isObject(v) &&
        Object.entries(v).every(
          ([k, x]) =>
            (extractionFields as readonly string[]).includes(k) &&
            typeof x === "string" &&
            Object.hasOwn(outcomes, x),
        )),
  }),
  suggestions: shape({
    draftId: text,
    author: (v) => typeof v === "string" && /^[0-9a-f]{64}$/.test(v),
    categoryId: optionalText,
    purpose: optionalText,
    note: optionalText,
    evidence: texts,
    createdAt: text,
  }),
};

// Accounts and categories first: journals are checked against those kept.
const order = [
  "accounts",
  "categories",
  "vendors",
  "documents",
  "payments",
  "drafts",
  "journals",
  "approvals",
  "rules",
  "audits",
  "events",
  "extractions",
  "suggestions",
] as const satisfies readonly (keyof typeof records)[];

const profile = shape({
  id: text,
  name: text,
  industry: text,
  currency: oneOf(...fiatCurrencies),
  initials: text,
  color: text,
});

function checkBusiness(
  raw: unknown,
  set: (q: Omit<QuarantinedRecord, "at">) => void,
): BusinessData | undefined {
  // Workspaces saved before extraction records have none (storage.ts), and
  // a business has no suggestion list until its first suggestion.
  const optional = (k: string) => k === "extractions" || k === "suggestions";
  const listed = (r: Fields) =>
    order.every(
      (k) => Array.isArray(r[k]) || (optional(k) && r[k] === undefined),
    );
  if (
    !isObject(raw) ||
    !profile(raw.profile) ||
    (raw.profile as Fields).businessId !== (raw.profile as Fields).id ||
    !listed(raw)
  ) {
    const id =
      isObject(raw) && isObject(raw.profile) && text(raw.profile.id)
        ? (raw.profile.id as string)
        : null;
    set({
      businessId: id,
      collection: "business",
      id,
      reason: "The business's profile or record lists are malformed.",
      record: raw,
    });
    return undefined;
  }
  const businessId = (raw.profile as Fields).id as string;
  const b = { profile: raw.profile } as BusinessData;
  for (const key of order) {
    if (key === "suggestions" && raw[key] === undefined) continue;
    const kept: unknown[] = [];
    (b as unknown as Record<string, unknown[]>)[key] = kept;
    for (const record of (raw[key] ?? []) as unknown[]) {
      const checked = checkRecord(b, key, record, businessId);
      if ("record" in checked) kept.push(checked.record);
      else
        set({
          businessId,
          collection: key,
          id:
            isObject(record) && text(record.id) ? (record.id as string) : null,
          reason: checked.reason,
          record,
        });
    }
  }
  return b;
}

/** The record to keep (extractions upgraded), or why it fails. */
function checkRecord(
  b: BusinessData,
  key: keyof typeof records,
  record: unknown,
  businessId: string,
): { record: unknown } | { reason: string } {
  if (!isObject(record) || !text(record.id) || record.businessId !== businessId)
    return { reason: "The record has no id or belongs to another business." };
  let r: Fields = record;
  // Fills fields added later, as storage.ts `upgradeWorkspace` does.
  if (key === "extractions")
    try {
      r = {
        ...record,
        fields: upgradeExtractionFields(record.fields as ExtractionFields),
      };
    } catch {
      return { reason: "The record's fields are malformed." };
    }
  if (!records[key](r)) return { reason: "The record's fields are malformed." };
  if (key === "journals")
    try {
      assertBalanced(b, r as never);
    } catch (e) {
      return { reason: (e as Error).message };
    }
  return { record: r };
}
