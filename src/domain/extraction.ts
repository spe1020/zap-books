import type {
  BusinessData,
  ExpenseDraft,
  ExtractionFieldName,
  ExtractionOutcome,
  ExtractionRecord,
  ExtractionResult,
  ImportedTransaction,
} from "./types";
import { amountInput, id, now } from "./ledger";

/** Suggestions a user can apply to a draft. The rest are shown for reference. */
export const applicableFields = [
  "vendor",
  "date",
  "total",
  "currency",
  "category",
] as const;
export type ApplicableField = (typeof applicableFields)[number];

/** Fields that come from a linked imported payment and cannot be edited. */
export const paymentLockedFields: ApplicableField[] = [
  "total",
  "date",
  "currency",
];

/** The form values a suggestion can fill. `amount` is the amount text box. */
export interface FormValues {
  vendor: string;
  date: string;
  amount: string;
  currency: string;
  categoryId: string;
}

export function isLocked(draft: ExpenseDraft, field: ApplicableField) {
  return !!draft.paymentId && paymentLockedFields.includes(field);
}

/** The form value a found suggestion would produce, or undefined. */
export function suggestedValue(
  result: ExtractionResult,
  field: ApplicableField,
): string | undefined {
  if (field === "total") {
    const f = result.fields.total;
    return f.status === "found" && f.value !== null
      ? amountInput(f.value)
      : undefined;
  }
  const f = result.fields[field];
  return f.status === "found" && f.value !== null ? String(f.value) : undefined;
}

const formKey = (field: ApplicableField): keyof FormValues =>
  field === "total" ? "amount" : field === "category" ? "categoryId" : field;

/**
 * Applies one found suggestion. Locked fields and fields without a found
 * value are left alone.
 */
export function applySuggestion(
  values: FormValues,
  draft: ExpenseDraft,
  result: ExtractionResult,
  field: ApplicableField,
): FormValues {
  const value = suggestedValue(result, field);
  if (value === undefined || isLocked(draft, field)) return values;
  return { ...values, [formKey(field)]: value };
}

/**
 * Applies every found, unlocked suggestion the user has not edited since
 * reading the receipt.
 */
export function acceptAllFound(
  values: FormValues,
  draft: ExpenseDraft,
  result: ExtractionResult,
  edited: ReadonlySet<ApplicableField>,
): { values: FormValues; applied: ApplicableField[] } {
  const applied: ApplicableField[] = [];
  for (const field of applicableFields) {
    if (edited.has(field) || isLocked(draft, field)) continue;
    if (suggestedValue(result, field) === undefined) continue;
    values = applySuggestion(values, draft, result, field);
    applied.push(field);
  }
  return { values, applied };
}

export interface PaymentComparison {
  field: ApplicableField;
  matches: boolean;
  message: string;
}

/** For payment-linked drafts: how each found locked field compares. */
export function compareWithPayment(
  result: ExtractionResult,
  payment: ImportedTransaction,
  format: (minor: number, currency: string) => string,
): PaymentComparison[] {
  const out: PaymentComparison[] = [];
  const { total, date, currency } = result.fields;
  if (total.status === "found" && total.value !== null) {
    const seen = format(total.value, payment.currency);
    const paid = format(payment.amountMinor, payment.currency);
    out.push(
      total.value === payment.amountMinor
        ? {
            field: "total",
            matches: true,
            message: `Receipt total ${seen} matches the payment`,
          }
        : {
            field: "total",
            matches: false,
            message: `Receipt total ${seen} doesn't match the payment (${paid}). Check that this is the right receipt.`,
          },
    );
  }
  if (date.status === "found" && date.value !== null)
    out.push(
      date.value === payment.date
        ? {
            field: "date",
            matches: true,
            message: `Receipt date ${date.value} matches the payment`,
          }
        : {
            field: "date",
            matches: false,
            message: `Receipt date ${date.value} doesn't match the payment (${payment.date}). Check that this is the right receipt.`,
          },
    );
  if (currency.status === "found" && currency.value !== null)
    out.push(
      currency.value === payment.currency
        ? {
            field: "currency",
            matches: true,
            message: `Receipt currency ${currency.value} matches the payment`,
          }
        : {
            field: "currency",
            matches: false,
            message: `Receipt currency ${currency.value} doesn't match the payment (${payment.currency}). Check that this is the right receipt.`,
          },
    );
  return out;
}

/**
 * Saves what the reader proposed for a draft and which suggestions the user
 * accepted, replacing an earlier unapproved record for the same draft and
 * document.
 */
export function recordExtraction(
  business: BusinessData,
  input: {
    draftId: string;
    documentId: string;
    result: ExtractionResult;
    accepted: ExtractionFieldName[];
  },
): BusinessData {
  const record: ExtractionRecord = {
    id: id(),
    businessId: business.profile.id,
    documentId: input.documentId,
    draftId: input.draftId,
    model: input.result.model,
    promptVersion: input.result.promptVersion,
    createdAt: now(),
    fields: structuredClone(input.result.fields),
    accepted: [...new Set(input.accepted)],
    outcomes: null,
  };
  return {
    ...business,
    extractions: [
      ...business.extractions.filter(
        (x) =>
          !(
            x.draftId === input.draftId &&
            x.documentId === input.documentId &&
            x.outcomes === null
          ),
      ),
      record,
    ],
  };
}

const draftFieldValue = (
  d: ExpenseDraft,
  field: ApplicableField,
): string | number =>
  field === "total"
    ? d.amountMinor
    : field === "category"
      ? d.categoryId
      : d[field];

/** The outcome of each applicable field, judged against the final draft. */
export function outcomesFor(
  record: ExtractionRecord,
  draft: ExpenseDraft,
): Partial<Record<ExtractionFieldName, ExtractionOutcome>> {
  const out: Partial<Record<ExtractionFieldName, ExtractionOutcome>> = {};
  for (const field of applicableFields) {
    const f = record.fields[field];
    if (f.status === "unclear") out[field] = "unclear";
    else if (f.status === "found")
      out[field] = !record.accepted.includes(field)
        ? "ignored"
        : draftFieldValue(draft, field) === f.value
          ? "accepted"
          : "edited";
  }
  return out;
}

/** Finalizes the outcomes of a draft's pending records once it is approved. */
export function finalizeExtractions(
  business: BusinessData,
  draftId: string,
): BusinessData {
  const draft = business.drafts.find(
    (d) => d.id === draftId && d.businessId === business.profile.id,
  );
  if (!draft || draft.status !== "posted") return business;
  return {
    ...business,
    extractions: business.extractions.map((x) =>
      x.draftId === draftId && x.outcomes === null
        ? { ...x, outcomes: outcomesFor(x, draft) }
        : x,
    ),
  };
}

const labels: Record<ExtractionFieldName, string> = {
  vendor: "vendor",
  date: "date",
  total: "total",
  currency: "currency",
  reference: "receipt number",
  subtotal: "subtotal",
  tax: "tax",
  category: "category",
};

/**
 * The service's warning that explains why a field is unclear, if any.
 * Warnings are plain sentences such as "The total couldn't be read…".
 */
export function fieldReason(
  result: ExtractionResult,
  field: ExtractionFieldName,
): string | undefined {
  const label = labels[field];
  return result.warnings.find(
    (w) =>
      w.startsWith(`The ${label} `) ||
      (field === "total" && w.includes("add up to the total")) ||
      (field === "date" &&
        (w.startsWith("The date ") || w.startsWith("The receipt date "))) ||
      (field === "currency" && w.startsWith("The receipt's currency ")),
  );
}
