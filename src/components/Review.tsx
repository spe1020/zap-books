import { createContext, useContext, useEffect, useRef, useState } from "react";
import {
  Check,
  CheckCircle2,
  ChevronRight,
  FileText,
  Paperclip,
  Sparkles,
  Upload,
  AlertCircle,
  Link2,
  RotateCcw,
  ScanText,
} from "lucide-react";
import type {
  BusinessData,
  ExpenseDraft,
  ExtractionFieldName,
  ExtractionResult,
  ExtractorHealth,
  SourceDocument,
} from "../domain/types";
import { currencies } from "../domain/types";
import {
  amountInput,
  duplicates,
  id,
  missingDetails,
  money,
  now,
  parseMoney,
  validation,
} from "../domain/ledger";
import { attachment } from "../adapters/attachments";
import { demoAI } from "../adapters/ai";
import { extractor, readingPlace, readyNote } from "../adapters/extractor";
import type { LedgerCommand } from "../adapters/ledger-client";
import {
  acceptAllFound,
  applySuggestion,
  type ApplicableField,
  type FormValues,
} from "../domain/extraction";
import { Modal, Status } from "./common";
import { paymentAccounts } from "../domain/transfers";
import { Suggestions } from "./Suggestions";
/** Runs one action's commands through the app's `LedgerClient`. */
export type Mutation = (
  commands: LedgerCommand[],
  message: string,
) => Promise<boolean>;
/**
 * Gets a receipt's file when this browser doesn't hold it: on the ledger
 * service, the stored copy, downloaded, decrypted and checked. Absent in
 * the local demo.
 */
export const ReceiptFiles = createContext<
  ((document: SourceDocument) => Promise<Blob>) | undefined
>(undefined);
export function DocumentPreview({ document }: { document?: SourceDocument }) {
  const [url, setUrl] = useState("");
  const [failedUrl, setFailedUrl] = useState("");
  const fetchFile = useContext(ReceiptFiles);
  const [fetched, setFetched] = useState<{ id: string; blob: Blob }>();
  const [fetchError, setFetchError] = useState<{
    id: string;
    message: string;
  }>();
  const stored = !document?.blob && !!document?.stored && !!fetchFile;
  useEffect(() => {
    if (!stored || !document || !fetchFile || fetched?.id === document.id)
      return;
    let current = true;
    fetchFile(document).then(
      (blob) => current && setFetched({ id: document.id, blob }),
      (e: Error) =>
        current && setFetchError({ id: document.id, message: e.message }),
    );
    return () => {
      current = false;
    };
  }, [stored, document, fetchFile, fetched]);
  const file =
    document?.blob ??
    (fetched && fetched.id === document?.id ? fetched.blob : undefined);
  useEffect(() => {
    if (!file) {
      setUrl("");
      return;
    }
    // Create and revoke in the same effect, including React StrictMode remounts.
    const next = URL.createObjectURL(file);
    setUrl(next);
    return () => URL.revokeObjectURL(next);
  }, [file]);
  if (!document)
    return (
      <div className="document-empty">
        <div className="paper-icon">
          <FileText size={36} strokeWidth={1.3} />
        </div>
        <h3>A little evidence goes a long way.</h3>
        <p>
          Attach a receipt to keep the full story
          <br />
          alongside this expense.
        </p>
      </div>
    );
  if (stored && !file) {
    const failed = fetchError?.id === document.id ? fetchError.message : "";
    return failed ? (
      <div className="document-empty elsewhere">
        <div className="paper-icon">
          <FileText size={36} strokeWidth={1.3} />
        </div>
        <h3>This receipt can't be shown</h3>
        <p role="alert" className="alert error">
          {failed}
        </p>
      </div>
    ) : (
      <p className="document-caption" role="status">
        Downloading receipt…
      </p>
    );
  }
  // Attached before receipt storage, on the ledger service: the file is
  // only in the browser that attached it.
  if (!file)
    return (
      <div className="document-empty elsewhere">
        <div className="paper-icon">
          <FileText size={36} strokeWidth={1.3} />
        </div>
        <h3>This receipt was attached on another device</h3>
        <p>
          {document.name}
          <br />
          SHA-256 <code className="hash">{document.sha256 ?? "unknown"}</code>
        </p>
      </div>
    );
  if (!url) return <p className="document-caption">Loading attachment…</p>;
  // A blob: URL opened in a tab runs in the app's origin, and an SVG
  // document can carry script, so an SVG is only ever shown as an <img>.
  const openable = document.mime !== "image/svg+xml";
  return (
    <>
      <div className="document-toolbar">
        <span>
          <FileText size={16} />
          {document.name}
        </span>
        {openable && (
          <a
            href={url}
            target="_blank"
            rel="noopener noreferrer"
            aria-label="Open attachment in new tab"
          >
            Open <ChevronRight size={14} />
          </a>
        )}
      </div>
      {document.mime === "application/pdf" ? (
        <object
          className="pdf-preview"
          data={url}
          type="application/pdf"
          aria-label={`PDF preview: ${document.name}`}
        >
          <p>
            Your browser cannot display this PDF.{" "}
            <a href={url} target="_blank" rel="noreferrer">
              Open the PDF
            </a>
            .
          </p>
        </object>
      ) : failedUrl === url ? (
        <p className="alert error" role="alert">
          This image could not be displayed.{" "}
          {openable ? "Open the attachment to inspect it, or attach" : "Attach"}{" "}
          a valid image.
        </p>
      ) : (
        <img
          className="receipt-image"
          src={url}
          alt={`Receipt evidence: ${document.name}`}
          onError={() => setFailedUrl(url)}
        />
      )}
      <p className="document-caption">
        {document.demo
          ? "Fictional receipt · demo evidence"
          : "Uploaded evidence · read only if you choose Read receipt"}
      </p>
    </>
  );
}
export function Review({
  b,
  initial,
  mutate,
  onClose,
  onOpen,
  busy,
  canEdit = true,
  canApprove = true,
  byAssistant = false,
}: {
  b: BusinessData;
  initial: ExpenseDraft;
  mutate: Mutation;
  onClose: () => void;
  onOpen: (d: ExpenseDraft) => void;
  busy: boolean;
  /** Whether this key may save this draft (books on the service). */
  canEdit?: boolean;
  /** Whether this key may approve, correct and import. */
  canApprove?: boolean;
  /** Whether an AI assistant created this draft. */
  byAssistant?: boolean;
}) {
  const [draft, setDraft] = useState<ExpenseDraft>(structuredClone(initial));
  const [amount, setAmount] = useState(
    initial.amountMinor ? amountInput(initial.amountMinor) : "",
  );
  const [docs, setDocs] = useState<SourceDocument[]>([]);
  const [activeDoc, setActiveDoc] = useState(initial.documentIds[0]);
  const [error, setError] = useState("");
  const [correcting, setCorrecting] = useState(false);
  const [reason, setReason] = useState("");
  const [saveRule, setSaveRule] = useState(false);
  const [loadingFile, setLoadingFile] = useState(false);
  const [health, setHealth] = useState<ExtractorHealth | null>(null);
  const [reading, setReading] = useState(false);
  /** Cancels the reading in progress; closing the dialog cancels it too. */
  const readController = useRef<AbortController | null>(null);
  useEffect(() => () => readController.current?.abort(), []);
  const [readError, setReadError] = useState("");
  const [extraction, setExtraction] = useState<{
    documentId: string;
    result: ExtractionResult;
  } | null>(null);
  // Fields the user changed since clicking Read receipt.
  const [edited, setEdited] = useState<Set<ApplicableField>>(new Set());
  const [accepted, setAccepted] = useState<Set<ExtractionFieldName>>(new Set());
  const readOnly = (initial.status !== "draft" || !canEdit) && !correcting;
  const working = { ...b, documents: [...b.documents, ...docs] };
  const suggestion = demoAI.propose(working, draft);
  const details = missingDetails(working, draft);
  const duplicate = duplicates(working, draft);
  const payment = b.payments.find((p) => p.id === draft.paymentId);
  const paid = paymentAccounts(b);
  const document = working.documents.find((d) => d.id === activeDoc);
  useEffect(() => {
    if (readOnly) return;
    let live = true;
    void extractor.health().then((h) => live && setHealth(h));
    return () => {
      live = false;
    };
  }, [readOnly]);
  const formField: Partial<Record<keyof ExpenseDraft, ApplicableField>> = {
    vendor: "vendor",
    date: "date",
    amountMinor: "total",
    currency: "currency",
    categoryId: "category",
  };
  function update<K extends keyof ExpenseDraft>(
    key: K,
    value: ExpenseDraft[K],
  ) {
    const field = formField[key];
    if (field) setEdited((e) => new Set(e).add(field));
    setDraft((d) => ({ ...d, [key]: value }));
  }
  const values: FormValues = {
    vendor: draft.vendor,
    date: draft.date,
    amount,
    currency: draft.currency,
    categoryId: draft.categoryId,
  };
  function applyValues(next: FormValues, fields: ApplicableField[]) {
    setAmount(next.amount);
    setDraft((d) => {
      let amountMinor = d.amountMinor;
      try {
        amountMinor = next.amount ? parseMoney(next.amount) : 0;
      } catch {
        amountMinor = 0;
      }
      return {
        ...d,
        vendor: next.vendor,
        date: next.date,
        currency: next.currency as ExpenseDraft["currency"],
        categoryId: next.categoryId,
        amountMinor,
      };
    });
    setAccepted((a) => new Set([...a, ...fields]));
  }
  async function readReceipt() {
    if (!document) return;
    const controller = new AbortController();
    readController.current = controller;
    setReading(true);
    setReadError("");
    setEdited(new Set());
    try {
      const result = await extractor.extract(
        document,
        b.categories.map(({ id, name }) => ({ id, name })),
        controller.signal,
      );
      if (controller.signal.aborted) return;
      setExtraction({ documentId: document.id, result });
      setAccepted(new Set());
    } catch (e) {
      if (controller.signal.aborted) return;
      setReadError((e as Error).message);
    } finally {
      if (readController.current === controller) {
        readController.current = null;
        if (!controller.signal.aborted) setReading(false);
      }
    }
  }
  function cancelReading() {
    readController.current?.abort();
    readController.current = null;
    setReading(false);
  }
  async function upload(file?: File) {
    if (!file) return;
    setLoadingFile(true);
    setError("");
    try {
      const doc = await attachment(file, b.profile.id);
      setDocs((old) => [...old, doc]);
      setDraft((d) => ({ ...d, documentIds: [...d.documentIds, doc.id] }));
      setActiveDoc(doc.id);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setLoadingFile(false);
    }
  }
  async function submit(post: boolean) {
    setError("");
    let amountMinor: number;
    try {
      amountMinor = amount ? parseMoney(amount) : 0;
    } catch (e) {
      setError((e as Error).message);
      return;
    }
    const next = { ...draft, amountMinor };
    const errors = validation(working, next);
    if (post && errors.length) {
      setError(errors.join(" "));
      return;
    }
    if (
      saveRule &&
      (!next.vendor.trim() || !next.categoryId || !next.purpose.trim())
    ) {
      setError("A rule requires a vendor, category, and business purpose.");
      return;
    }
    if (correcting && !reason.trim()) {
      setError("Explain the correction to preserve its history.");
      return;
    }
    const reading = extraction
      ? {
          documentId: extraction.documentId,
          result: extraction.result,
          accepted: [...accepted],
        }
      : undefined;
    // "Save as a rule" is its own command, with the Settings screen's
    // checks: an existing rule for this vendor is replaced.
    const existing = b.rules.find(
      (r) =>
        r.vendorIncludes.toLowerCase() === next.vendor.trim().toLowerCase(),
    );
    const rule: LedgerCommand[] = saveRule
      ? [
          {
            type: "SaveRule",
            rule: {
              id: existing?.id,
              vendorIncludes: next.vendor.trim(),
              categoryId: next.categoryId,
              purpose: next.purpose,
              enabled: true,
            },
          },
        ]
      : [];
    const ok = await mutate(
      correcting
        ? [
            ...rule,
            {
              type: "Correct",
              originalId: initial.id,
              replacement: next,
              reason,
              documents: docs,
              extraction: reading,
            },
          ]
        : [
            ...rule,
            {
              type: "SaveDraft",
              draft: next,
              documents: docs,
              extraction: reading,
            },
            ...(post ? [{ type: "Approve" as const, expenseId: next.id }] : []),
          ],
      correcting
        ? "Correction posted. Original history preserved."
        : post
          ? "Expense posted. Your reports are up to date."
          : "Draft saved.",
    );
    if (ok) onClose();
  }
  const history = b.audits.filter((a) => a.entityId === initial.id);
  const related = b.drafts.filter(
    (d) => d.id === initial.correctionOf || d.correctionOf === initial.id,
  );
  return (
    <Modal
      title={
        readOnly
          ? "Transaction details"
          : correcting
            ? "Correct this expense"
            : "Review expense"
      }
      onClose={onClose}
      wide
    >
      <div className="review-subhead">
        <span className="eyebrow">{b.profile.name}</span>
        <Status b={b} d={initial} />
        {byAssistant && (
          <span className="badge blue assistant-mark">
            Created by an AI assistant
          </span>
        )}
      </div>
      <div className="review-grid">
        <section className="evidence-pane" aria-label="Receipt evidence">
          <div className="pane-label">
            <span>RECEIPT EVIDENCE</span>
            <span>{draft.documentIds.length} attached</span>
          </div>
          <div className="document-stage">
            <DocumentPreview document={document} />
          </div>
          {!readOnly && document?.blob && (
            <div className="read-receipt">
              <button
                type="button"
                className="button secondary"
                disabled={!health?.ready || reading}
                onClick={() => void readReceipt()}
                aria-describedby="read-receipt-note"
              >
                <ScanText size={16} />
                {reading ? "Reading receipt…" : "Read receipt"}
              </button>
              {reading && (
                <button
                  type="button"
                  className="button secondary"
                  onClick={cancelReading}
                >
                  Cancel reading
                </button>
              )}
              <p id="read-receipt-note" className="small muted" role="status">
                {reading
                  ? `Reading ${readingPlace()}. You can keep editing.`
                  : health === null
                    ? `Checking for receipt reading ${readingPlace()}…`
                    : health.ready
                      ? readyNote(health.model)
                      : health.reason}
              </p>
              {readError && (
                <p className="alert error" role="alert">
                  {readError}
                </p>
              )}
            </div>
          )}
          {draft.documentIds.length > 1 && (
            <div className="doc-tabs">
              {draft.documentIds.map((docId, i) => (
                <button
                  key={docId}
                  className={activeDoc === docId ? "active" : ""}
                  onClick={() => setActiveDoc(docId)}
                >
                  Receipt {i + 1}
                </button>
              ))}
            </div>
          )}
          {!readOnly && (
            <label className="upload-zone">
              <Upload size={18} />
              <span>
                {loadingFile ? "Reading attachment…" : "Attach receipt"}
                <small>PNG, JPG, WebP or PDF · up to 10 MB</small>
              </span>
              <input
                aria-label="Attach receipt"
                type="file"
                accept="image/png,image/jpeg,image/webp,application/pdf"
                disabled={loadingFile || busy}
                onChange={(e) => {
                  void upload(e.target.files?.[0]);
                  e.target.value = "";
                }}
              />
            </label>
          )}
          {draft.importSource && (
            <div className="payment-evidence">
              <FileText size={18} />
              <div>
                <strong>Imported expense history</strong>
                <p>
                  {draft.importSource.provider === "quickbooks"
                    ? "QuickBooks"
                    : draft.importSource.provider === "zoho"
                      ? "Zoho Books"
                      : "CSV"}{" "}
                  · {draft.importSource.reference || "No source reference"}
                </p>
                <p>
                  {draft.importSource.vendor} ·{" "}
                  {money(
                    draft.importSource.amountMinor,
                    draft.importSource.currency,
                  )}{" "}
                  · {draft.importSource.date}
                </p>
                {draft.importSource.category && (
                  <small>
                    Original category: {draft.importSource.category}
                  </small>
                )}
                <p className="small">
                  Source: {draft.importSource.filename || "CSV file"}. This
                  preserves the original accounting record; it is not bank
                  payment evidence.
                </p>
              </div>
            </div>
          )}
          {payment && (
            <div className="payment-evidence">
              <Link2 size={18} />
              <div>
                <strong>Payment evidence linked</strong>
                <p>
                  {payment.description} ·{" "}
                  {money(payment.amountMinor, payment.currency)}
                </p>
                <small>
                  {payment.date} · {payment.reference || "Imported transaction"}
                </small>
                <p className="small">
                  This expense is linked to the original bank/card payment. Its
                  amount, date, currency, and account stay aligned with that
                  payment when you correct other details.
                </p>
              </div>
            </div>
          )}
        </section>
        <section className="review-form" aria-label="Expense details">
          {error && (
            <div className="alert error" role="alert">
              {error}
            </div>
          )}
          {!readOnly && extraction && (
            <Suggestions
              result={extraction.result}
              draft={draft}
              values={values}
              payment={payment}
              categories={b.categories}
              edited={edited}
              ruleCategoryId={suggestion.categoryId}
              onAccept={(field) =>
                applyValues(
                  applySuggestion(values, draft, extraction.result, field),
                  [field],
                )
              }
              onAcceptAll={() => {
                const { values: next, applied } = acceptAllFound(
                  values,
                  draft,
                  extraction.result,
                  edited,
                );
                applyValues(next, applied);
              }}
            />
          )}
          {!readOnly && (
            <div className="ai-proposal">
              <div className="flex-between">
                <span className="mini-heading">
                  <Sparkles size={16} />
                  Suggested classification
                </span>
                <span className="badge neutral">Simulated</span>
              </div>
              <p>{suggestion.explanation}</p>
              <details>
                <summary>Evidence & limitations</summary>
                <ul>
                  {suggestion.evidence.map((x) => (
                    <li key={x}>{x}</li>
                  ))}
                </ul>
                <p>{suggestion.uncertainty}</p>
              </details>
              {suggestion.categoryId && (
                <button
                  className="text-button"
                  onClick={() => {
                    setEdited((e) => new Set(e).add("category"));
                    setDraft((d) => ({
                      ...d,
                      categoryId: suggestion.categoryId!,
                      purpose: suggestion.purpose || d.purpose,
                    }));
                  }}
                >
                  <Check size={14} />
                  Apply suggestion
                </button>
              )}
            </div>
          )}
          <fieldset disabled={readOnly || busy} className="fields">
            <label>
              Vendor
              <input
                value={draft.vendor}
                onChange={(e) => update("vendor", e.target.value)}
                placeholder="Who was this paid to?"
                maxLength={160}
              />
            </label>
            <div className="field-row">
              <label>
                Date
                <input
                  type="date"
                  value={draft.date}
                  onChange={(e) => update("date", e.target.value)}
                  disabled={!!payment || readOnly}
                />
              </label>
              <label>
                Currency
                <select
                  value={draft.currency}
                  onChange={(e) =>
                    update(
                      "currency",
                      e.target.value as ExpenseDraft["currency"],
                    )
                  }
                  disabled={!!payment || readOnly}
                >
                  {currencies.map((c) => (
                    <option key={c}>{c}</option>
                  ))}
                </select>
              </label>
            </div>
            <label>
              Description
              <input
                value={draft.description}
                onChange={(e) => update("description", e.target.value)}
                placeholder="What was purchased?"
                maxLength={300}
              />
            </label>
            <label>
              Amount
              <input
                inputMode="decimal"
                value={amount}
                disabled={!!payment || readOnly}
                aria-describedby={payment ? "payment-amount-help" : undefined}
                onChange={(e) => {
                  setAmount(e.target.value);
                  try {
                    update("amountMinor", parseMoney(e.target.value));
                  } catch {
                    update("amountMinor", 0);
                  }
                }}
                placeholder="0.00"
              />
            </label>
            {payment && (
              <p
                id="payment-amount-help"
                className="small muted payment-lock-note"
              >
                Amount comes from the linked bank/card payment and cannot be
                changed here. You can correct the category and descriptive
                details while preserving that payment.
              </p>
            )}
            <label>
              Category
              <select
                value={draft.categoryId}
                onChange={(e) => update("categoryId", e.target.value)}
              >
                <option value="">Choose a category</option>
                {b.categories.map((c) => (
                  <option key={c.id} value={c.id}>
                    {c.name}
                  </option>
                ))}
              </select>
            </label>
            <label>
              Paid from
              <select
                value={draft.accountId}
                aria-describedby={
                  paid.personal.some((a) => a.id === draft.accountId)
                    ? "paid-from-help"
                    : undefined
                }
                disabled={!!payment || readOnly}
                onChange={(e) => update("accountId", e.target.value)}
              >
                <option value="">Choose an account</option>
                {paid.own.map((a) => (
                  <option key={a.id} value={a.id}>
                    {a.name} · {a.currency}
                  </option>
                ))}
                {paid.personal.map((a) => (
                  <option key={a.id} value={a.id}>
                    Paid personally: {a.name} · {a.currency}
                  </option>
                ))}
              </select>
            </label>
            {paid.personal.some((a) => a.id === draft.accountId) && (
              <small id="paid-from-help" className="small muted field-help">
                {b.accounts.find((a) => a.id === draft.accountId)?.kind ===
                "liability"
                  ? "The owner paid this personally; the business will owe them the amount."
                  : "The owner paid this personally, as a contribution to the business."}
              </small>
            )}
            <label>
              Business purpose
              <textarea
                value={draft.purpose}
                onChange={(e) => update("purpose", e.target.value)}
                rows={2}
                placeholder="How does this support your business?"
                maxLength={1000}
              />
            </label>
            {!!duplicate.length && !readOnly && (
              <div className="alert warning">
                <strong>
                  <AlertCircle size={16} />
                  Possible duplicate
                </strong>
                <p>
                  {duplicate.map((d) => d.vendor).join(", ")} has the same
                  vendor, date, amount, and currency. Review the existing record
                  before posting.
                </p>
                <label>
                  Why is this a separate expense?
                  <textarea
                    rows={2}
                    value={draft.duplicateReason}
                    onChange={(e) => update("duplicateReason", e.target.value)}
                    placeholder="For example, a separate license with its own invoice."
                  />
                </label>
              </div>
            )}
            {!readOnly && canApprove && (
              <label className="checkbox-label">
                <input
                  type="checkbox"
                  checked={saveRule}
                  onChange={(e) => setSaveRule(e.target.checked)}
                />
                <span>
                  Save this vendor classification as a visible rule
                  <small>Review and edit it in Settings.</small>
                </span>
              </label>
            )}
            {correcting && (
              <label>
                Reason for correction
                <textarea
                  value={reason}
                  onChange={(e) => setReason(e.target.value)}
                  placeholder="Explain what changed and why."
                  rows={2}
                />
              </label>
            )}
          </fieldset>
          {!readOnly && (
            <div className={`validation-note ${details.length ? "needs" : ""}`}>
              <span>
                {details.length ? (
                  <AlertCircle size={17} />
                ) : (
                  <CheckCircle2 size={17} />
                )}
              </span>
              <div>
                {details.length ? (
                  <>
                    <strong>
                      {details.length} detail{details.length > 1 ? "s" : ""} to
                      resolve
                    </strong>
                    <ul>
                      {details.map((e) => (
                        <li key={e}>{e}</li>
                      ))}
                    </ul>
                  </>
                ) : (
                  "Required details are complete."
                )}
              </div>
            </div>
          )}
          {readOnly && (
            <div className="history">
              <h3>Record history</h3>
              {history.map((a) => (
                <div className="history-item" key={a.id}>
                  <span className="history-dot" />
                  <div>
                    <strong>
                      {a.action.replace("expense.", "").replaceAll(".", " ")}
                    </strong>
                    <p>{a.detail}</p>
                    <small>
                      {new Date(a.at).toLocaleString()} · {a.actor}
                    </small>
                  </div>
                </div>
              ))}
              {related.map((d) => (
                <button
                  key={d.id}
                  className="text-button"
                  onClick={() => onOpen(d)}
                >
                  <Link2 size={15} />
                  {d.id === initial.correctionOf
                    ? "View original expense"
                    : "View replacement expense"}
                  <ChevronRight size={14} />
                </button>
              ))}
              <details>
                <summary>Balanced accounting entries</summary>
                {b.journals
                  .filter((j) => j.expenseId === initial.id)
                  .map((j) => (
                    <div key={j.id} className="journal">
                      <strong>
                        {j.kind} · {j.id.slice(0, 8)}
                      </strong>
                      {j.lines.map((l, i) => (
                        <p key={i}>
                          {b.accounts.find((a) => a.id === l.accountId)?.name}
                          <span>
                            {l.debit ? "Debit" : "Credit"}{" "}
                            {money(l.debit || l.credit, j.currency)}
                          </span>
                        </p>
                      ))}
                    </div>
                  ))}
              </details>
            </div>
          )}
        </section>
      </div>
      <footer className="review-footer">
        <span>
          <Paperclip size={14} />
          {readOnly
            ? "Posted records preserve their history."
            : "Approval records an entry. It does not send a payment."}
        </span>
        <div>
          {readOnly ? (
            <>
              {initial.status === "posted" && canApprove && (
                <button
                  className="button secondary"
                  onClick={() => {
                    setCorrecting(true);
                    setDraft((d) => ({
                      ...d,
                      id: id(),
                      status: "draft",
                      journalId: undefined,
                      correctionOf: initial.id,
                      createdAt: now(),
                    }));
                  }}
                >
                  <RotateCcw size={16} />
                  Correct expense
                </button>
              )}
              <button className="button primary" onClick={onClose}>
                Done
              </button>
            </>
          ) : (
            <>
              {!correcting && (
                <button
                  className="button secondary"
                  onClick={() => void submit(false)}
                  disabled={busy || loadingFile}
                >
                  Save draft
                </button>
              )}
              {canApprove && (
                <button
                  className="button primary"
                  onClick={() => void submit(true)}
                  disabled={busy || loadingFile}
                >
                  <Check size={16} />
                  {busy
                    ? "Saving…"
                    : correcting
                      ? "Post correction"
                      : "Approve & post"}
                </button>
              )}
            </>
          )}
        </div>
      </footer>
    </Modal>
  );
}
