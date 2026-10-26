import { AlertCircle, Check, CheckCircle2, ScanText } from "lucide-react";
import type {
  Category,
  ExpenseDraft,
  ExtractionFieldName,
  ExtractionResult,
  ImportedTransaction,
} from "../domain/types";
import { money } from "../domain/ledger";
import {
  applicableFields,
  compareWithPayment,
  fieldReason,
  isLocked,
  suggestedValue,
  type ApplicableField,
  type FormValues,
} from "../domain/extraction";

const names: Record<ExtractionFieldName, string> = {
  vendor: "Vendor",
  date: "Date",
  total: "Total",
  currency: "Currency",
  reference: "Receipt number",
  subtotal: "Subtotal",
  tax: "Tax",
  tip: "Tip",
  category: "Category",
};

function display(
  result: ExtractionResult,
  field: ExtractionFieldName,
  categories: Category[],
  currency: string,
): string {
  const f = result.fields;
  switch (field) {
    case "total":
    case "subtotal":
    case "tax":
    case "tip": {
      const v = f[field].value;
      return v === null ? "" : money(v, currency as ExpenseDraft["currency"]);
    }
    case "category":
      return (
        categories.find((c) => c.id === f.category.value)?.name ??
        f.category.value ??
        ""
      );
    default:
      return String(f[field].value ?? "");
  }
}

const formValue = (values: FormValues, field: ApplicableField) =>
  field === "total"
    ? values.amount
    : field === "category"
      ? values.categoryId
      : values[field];

export function Suggestions({
  result,
  draft,
  values,
  payment,
  categories,
  edited,
  ruleCategoryId,
  onAccept,
  onAcceptAll,
}: {
  result: ExtractionResult;
  draft: ExpenseDraft;
  values: FormValues;
  payment?: ImportedTransaction;
  categories: Category[];
  edited: ReadonlySet<ApplicableField>;
  ruleCategoryId?: string;
  onAccept: (field: ApplicableField) => void;
  onAcceptAll: () => void;
}) {
  const currency =
    result.fields.currency.value ?? payment?.currency ?? draft.currency;
  const comparisons = payment
    ? compareWithPayment(result, payment, (n, c) =>
        money(n, c as ExpenseDraft["currency"]),
      )
    : [];
  const acceptable = applicableFields.filter(
    (f) =>
      !isLocked(draft, f) &&
      !edited.has(f) &&
      suggestedValue(result, f) !== undefined &&
      suggestedValue(result, f) !== formValue(values, f),
  );
  const readerCategory = result.fields.category.value;
  const categoryName = (id?: string | null) =>
    categories.find((c) => c.id === id)?.name ?? "";

  const row = (field: ExtractionFieldName) => {
    const f = result.fields[field];
    const applicable = (applicableFields as readonly string[]).includes(field);
    const comparison = comparisons.find((c) => c.field === field);
    let action: React.ReactNode = null;
    if (f.status === "found" && applicable) {
      const a = field as ApplicableField;
      if (comparison)
        action = (
          <p
            className={`suggestion-compare ${comparison.matches ? "match" : "mismatch"}`}
          >
            {comparison.matches ? (
              <CheckCircle2 size={14} />
            ) : (
              <AlertCircle size={14} />
            )}
            {comparison.message}
          </p>
        );
      else if (isLocked(draft, a)) action = null;
      else if (suggestedValue(result, a) === formValue(values, a))
        action = <span className="suggestion-applied">In the form</span>;
      else
        action = (
          <button
            type="button"
            className="button secondary small"
            aria-label={`Accept ${names[field].toLowerCase()} suggestion`}
            onClick={() => onAccept(a)}
          >
            <Check size={14} />
            Accept
          </button>
        );
    }
    return (
      <li key={field} className={`suggestion ${f.status}`}>
        <div className="suggestion-head">
          <span className="suggestion-name">{names[field]}</span>
          {f.status === "found" ? (
            <strong className="suggestion-value">
              {display(result, field, categories, currency)}
            </strong>
          ) : f.status === "unclear" ? (
            <span className="badge warning">Unclear</span>
          ) : (
            <span className="muted small">Not on the receipt</span>
          )}
        </div>
        {f.evidence && (
          <p className="suggestion-evidence">Seen on receipt: {f.evidence}</p>
        )}
        {f.status === "unclear" && (
          <p className="suggestion-reason">
            {fieldReason(result, field) ??
              "The model couldn't read this with certainty. Check the receipt."}
          </p>
        )}
        {f.status === "found" &&
          applicable &&
          edited.has(field as ApplicableField) &&
          !comparison && (
            <p className="muted small">
              You changed this field after reading the receipt. Accept all
              leaves it as you typed it.
            </p>
          )}
        {action}
      </li>
    );
  };

  return (
    <section className="suggestions" aria-label="Suggestions from the receipt">
      <div className="flex-between">
        <span className="mini-heading">
          <ScanText size={16} />
          Suggested from the receipt
        </span>
        <span className="badge neutral">{result.model}</span>
      </div>
      <p className="suggestions-note">
        Read by a model on this computer. Check each value before approving.
      </p>
      {result.warnings.length > 0 && (
        <ul className="suggestion-warnings">
          {result.warnings.map((w) => (
            <li key={w}>
              <AlertCircle size={14} />
              {w}
            </li>
          ))}
        </ul>
      )}
      <ul className="suggestion-list">
        {applicableFields.map(row)}
        {(["reference", "subtotal", "tax", "tip"] as const).map(row)}
      </ul>
      {ruleCategoryId &&
        readerCategory &&
        ruleCategoryId !== readerCategory && (
          <p className="suggestion-reason">
            Your saved vendor rule suggests {categoryName(ruleCategoryId)}; the
            receipt reader suggests {categoryName(readerCategory)}. Choose the
            one that fits.
          </p>
        )}
      {!draft.purpose.trim() && (
        <p className="suggestion-reason">
          Add the business purpose; the receipt can't tell us why you bought
          this.
        </p>
      )}
      <button
        type="button"
        className="button secondary"
        disabled={acceptable.length === 0}
        onClick={onAcceptAll}
      >
        <Check size={16} />
        Accept all found
      </button>
    </section>
  );
}
