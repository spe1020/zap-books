import { useEffect, useState } from "react";
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
} from "lucide-react";
import type {
  BusinessData,
  ExpenseDraft,
  SourceDocument,
} from "../domain/types";
import { currencies } from "../domain/types";
import {
  amountInput,
  approve,
  correct,
  duplicates,
  event,
  id,
  missingDetails,
  money,
  now,
  parseMoney,
  saveDraft,
  validation,
} from "../domain/ledger";
import { attachment } from "../adapters/attachments";
import { demoAI } from "../adapters/ai";
import { Modal, Status } from "./common";
export type Mutation = (
  fn: (b: BusinessData) => BusinessData,
  message: string,
) => Promise<boolean>;
export function DocumentPreview({ document }: { document?: SourceDocument }) {
  const [url, setUrl] = useState("");
  const [failedUrl, setFailedUrl] = useState("");
  useEffect(() => {
    if (!document) {
      setUrl("");
      return;
    }
    // Create and revoke in the same effect, including React StrictMode remounts.
    const next = URL.createObjectURL(document.blob);
    setUrl(next);
    return () => URL.revokeObjectURL(next);
  }, [document]);
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
  if (!url) return <p className="document-caption">Loading attachment…</p>;
  return (
    <>
      <div className="document-toolbar">
        <span>
          <FileText size={16} />
          {document.name}
        </span>
        <a
          href={url}
          target="_blank"
          rel="noreferrer"
          aria-label="Open attachment in new tab"
        >
          Open <ChevronRight size={14} />
        </a>
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
          This image could not be displayed. Open the attachment to inspect it,
          or attach a valid image.
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
          : "Uploaded evidence · no OCR or document analysis"}
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
}: {
  b: BusinessData;
  initial: ExpenseDraft;
  mutate: Mutation;
  onClose: () => void;
  onOpen: (d: ExpenseDraft) => void;
  busy: boolean;
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
  const readOnly = initial.status !== "draft" && !correcting;
  const working = { ...b, documents: [...b.documents, ...docs] };
  const suggestion = demoAI.propose(working, draft);
  const details = missingDetails(working, draft);
  const duplicate = duplicates(working, draft);
  const payment = b.payments.find((p) => p.id === draft.paymentId);
  const document = working.documents.find((d) => d.id === activeDoc);
  function update<K extends keyof ExpenseDraft>(
    key: K,
    value: ExpenseDraft[K],
  ) {
    setDraft((d) => ({ ...d, [key]: value }));
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
    const ok = await mutate(
      (current) => {
        let updated = {
          ...current,
          documents: [...current.documents, ...docs],
        };
        if (saveRule) {
          const existing = updated.rules.find(
            (r) =>
              r.vendorIncludes.toLowerCase() ===
              next.vendor.trim().toLowerCase(),
          );
          const rule = {
            id: existing?.id ?? id(),
            businessId: b.profile.id,
            vendorIncludes: next.vendor.trim(),
            categoryId: next.categoryId,
            purpose: next.purpose,
            enabled: true,
          };
          updated = {
            ...updated,
            rules: [...updated.rules.filter((r) => r.id !== rule.id), rule],
          };
          event(
            updated,
            "rule.saved",
            rule.id,
            `Explicitly saved classification rule for ${rule.vendorIncludes}`,
          );
        }
        if (correcting) return correct(updated, initial.id, next, reason);
        updated = saveDraft(updated, next);
        return post ? approve(updated, next.id) : updated;
      },
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
                  This receipt belongs to the existing payment draft.
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
                  onClick={() =>
                    setDraft((d) => ({
                      ...d,
                      categoryId: suggestion.categoryId!,
                      purpose: suggestion.purpose || d.purpose,
                    }))
                  }
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
              Payment account
              <select
                value={draft.accountId}
                disabled={!!payment || readOnly}
                onChange={(e) => update("accountId", e.target.value)}
              >
                <option value="">Choose an account</option>
                {b.accounts
                  .filter((a) => a.kind !== "expense")
                  .map((a) => (
                    <option key={a.id} value={a.id}>
                      {a.name} · {a.currency}
                    </option>
                  ))}
              </select>
            </label>
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
            {!readOnly && (
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
              {initial.status === "posted" && (
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
            </>
          )}
        </div>
      </footer>
    </Modal>
  );
}
