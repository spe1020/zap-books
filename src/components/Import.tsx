import { useMemo, useState } from "react";
import { Upload, FileSpreadsheet, Check, Download } from "lucide-react";
import type { BusinessData, ImportProvider } from "../domain/types";
import { parseCSV, previewImport, guessMapping } from "../domain/import";
import type { CSVMapping, DateFormat } from "../domain/import";
import { currentPeriod, money } from "../domain/ledger";
import { Modal } from "./common";
import type { Mutation } from "./Review";
export function ImportDialog({
  b,
  mutate,
  onClose,
  busy,
}: {
  b: BusinessData;
  mutate: Mutation;
  onClose: () => void;
  busy: boolean;
}) {
  const [rows, setRows] = useState<string[][]>([]),
    [filename, setFilename] = useState("");
  const [mapping, setMapping] = useState<CSVMapping>({
    date: "",
    description: "",
    amount: "",
    currency: "",
    reference: "",
    vendor: "",
    category: "",
    purpose: "",
    transactionType: "",
    sourceAccount: "",
  });
  const [account, setAccount] = useState(
    b.accounts.find((a) => a.kind === "bank")!.id,
  );
  const [convention, setConvention] = useState<"negative" | "positive">(
    "negative",
  );
  const [error, setError] = useState("");
  const [kind, setKind] = useState<"payment" | "expense-history">("payment");
  const [provider, setProvider] = useState<ImportProvider>("csv");
  const [dateFormat, setDateFormat] = useState<DateFormat>("iso");
  const preview = useMemo(
    () =>
      rows.length
        ? previewImport(b, rows, mapping, account, convention, {
            kind,
            provider,
            dateFormat,
            filename,
          })
        : [],
    [
      b,
      rows,
      mapping,
      account,
      convention,
      kind,
      provider,
      dateFormat,
      filename,
    ],
  );
  const errors = preview.filter((r) => r.error).length,
    repeated = preview.filter((r) => !r.error && r.duplicate).length,
    valid = preview.filter((r) => !r.error && !r.duplicate).length;
  async function load(file?: File) {
    if (!file) return;
    setError("");
    try {
      if (file.size > 2_000_000)
        throw new Error("CSV must be smaller than 2 MB.");
      const parsed = parseCSV(await file.text());
      setRows(parsed);
      setFilename(file.name);
      setMapping(guessMapping(parsed[0]));
    } catch (e) {
      setError((e as Error).message);
      setRows([]);
    }
  }
  function sample() {
    const [year, month] = currentPeriod().split("-");
    const date = (day: string) =>
      dateFormat === "iso"
        ? `${year}-${month}-${day}`
        : dateFormat === "mdy"
          ? `${month}/${day}/${year}`
          : `${day}/${month}/${year}`;
    const sign = convention === "negative" ? "-" : "";
    const csv =
      kind === "expense-history"
        ? `date,vendor,description,amount,currency,reference,category,business purpose\n${date("23")},Paper & Pine,Workshop materials,${sign}42.50,USD,DEMO-HISTORY-001,Office & supplies,Client workshop\n`
        : `date,description,amount,currency,reference\n${date("23")},Paper & Pine,${sign}42.50,USD,DEMO-IMPORT-001\n${date("22")},Cloud Tools,${sign}29.00,USD,DEMO-IMPORT-002\n`;
    const url = URL.createObjectURL(new Blob([csv], { type: "text/csv" }));
    const a = document.createElement("a");
    a.href = url;
    a.download = "sample-expenses.csv";
    a.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  }
  return (
    <Modal title="Import transactions" onClose={onClose} wide>
      <div className="import-body">
        <div className="import-intro">
          <div className="section-icon">
            <FileSpreadsheet size={24} />
          </div>
          <div>
            <h3>Bring your expenses together.</h3>
            <p>
              Bring in bank payments or expense history exported from
              QuickBooks, Zoho Books, or another accounting system. Map columns
              and review before importing.
            </p>
          </div>
          <button className="button secondary" onClick={sample}>
            <Download size={16} />
            Sample CSV
          </button>
        </div>
        {error && (
          <div role="alert" className="alert error">
            {error}
          </div>
        )}
        <div className="field-row import-options">
          <label>
            Type of import
            <select
              value={kind}
              onChange={(e) => {
                const next = e.target.value as typeof kind;
                setKind(next);
                setConvention(
                  next === "expense-history" ? "positive" : "negative",
                );
              }}
            >
              <option value="payment">Bank or card payments</option>
              <option value="expense-history">Expense history</option>
            </select>
          </label>
          {kind === "expense-history" && (
            <label>
              Source system
              <select
                value={provider}
                onChange={(e) => setProvider(e.target.value as ImportProvider)}
              >
                <option value="csv">Other CSV</option>
                <option value="quickbooks">QuickBooks</option>
                <option value="zoho">Zoho Books</option>
              </select>
            </label>
          )}
          <label>
            Date format
            <select
              value={dateFormat}
              onChange={(e) => setDateFormat(e.target.value as DateFormat)}
            >
              <option value="iso">YYYY-MM-DD</option>
              <option value="mdy">MM/DD/YYYY</option>
              <option value="dmy">DD/MM/YYYY</option>
            </select>
          </label>
        </div>
        {kind === "expense-history" && (
          <p className="small muted export-description">
            Export paid expenses with one row per expense, one payment account
            and one currency per file. Exclude unpaid bills, invoices,
            transfers, tax breakdowns, split rows, and report totals. Source
            layouts vary; verify every mapping. These records become drafts, not
            opening balances or bank payment evidence.
          </p>
        )}
        <label className="upload-zone csv-upload">
          <Upload size={22} />
          <span>
            {filename || "Choose a CSV file"}
            <small>
              Up to 2 MB · 2,000 rows · choose the matching date format
            </small>
          </span>
          <input
            type="file"
            aria-label="Choose CSV file"
            accept=".csv,text/csv"
            onChange={(e) => {
              void load(e.target.files?.[0]);
              e.target.value = "";
            }}
          />
        </label>
        <div className="field-row">
          <label>
            Payment account
            <select
              value={account}
              onChange={(e) => setAccount(e.target.value)}
            >
              {b.accounts
                .filter((a) => a.kind !== "expense")
                .map((a) => (
                  <option value={a.id} key={a.id}>
                    {a.name} · {a.currency}
                  </option>
                ))}
            </select>
          </label>
          <label>
            Expense amounts
            <select
              value={convention}
              onChange={(e) =>
                setConvention(e.target.value as typeof convention)
              }
            >
              <option value="negative">Negative amounts are expenses</option>
              <option value="positive">Positive amounts are expenses</option>
            </select>
          </label>
        </div>
        <p className="small muted">
          The selected payment account is applied to every row. Import one
          source account at a time. Expense outflows only; remove income and
          transfers before import. A provider reference is recommended; without
          one, identical date, description, amount, currency, and account rows
          are treated as repeats. Use plain decimal amounts (1234.56) or comma
          thousands groups (1,234.56), without currency symbols. Exact category
          names are matched; unknown categories stay unresolved for review.
        </p>
        {!!rows.length && (
          <>
            <h3 className="spaced-heading">1. Map columns</h3>
            <div className="mapping-grid">
              {(Object.keys(mapping) as (keyof CSVMapping)[]).map((key) => (
                <label key={key}>
                  {key === "sourceAccount"
                    ? "Source payment account"
                    : key === "transactionType"
                      ? "Transaction type"
                      : key === "purpose"
                        ? "Business purpose"
                        : key}
                  {["date", "description", "amount"].includes(key)
                    ? " *"
                    : " (optional)"}
                  <select
                    aria-label={`Map ${key}`}
                    value={mapping[key]}
                    onChange={(e) =>
                      setMapping((m) => ({ ...m, [key]: e.target.value }))
                    }
                  >
                    <option value="">
                      {key === "currency"
                        ? "Use account currency"
                        : "Select column"}
                    </option>
                    {rows[0].map((h) => (
                      <option key={h}>{h}</option>
                    ))}
                  </select>
                </label>
              ))}
            </div>
            <div className="flex-between spaced-heading">
              <h3>2. Review import</h3>
              <span className="small">
                {valid} new · {repeated} repeated · {errors} invalid
              </span>
            </div>
            {valid > 0 && errors === 0 && (
              <p className="small muted export-description">
                New expenses total:{" "}
                {money(
                  preview
                    .filter((r) => !r.error && !r.duplicate)
                    .reduce((sum, r) => sum + r.amountMinor, 0),
                  b.accounts.find((a) => a.id === account)!.currency,
                )}
                . Compare this with the same rows in your source export before
                importing.
              </p>
            )}
            <div
              className="table-scroll import-table"
              tabIndex={0}
              role="region"
              aria-label="Import preview"
            >
              <table>
                <thead>
                  <tr>
                    <th>Row</th>
                    <th>Date</th>
                    <th>Description</th>
                    <th>Vendor / category</th>
                    <th>Amount</th>
                    <th>Result</th>
                  </tr>
                </thead>
                <tbody>
                  {preview.map((r) => (
                    <tr key={r.row}>
                      <td>{r.row}</td>
                      <td>{r.date}</td>
                      <td>{r.description}</td>
                      <td>
                        {r.vendor || "—"}
                        <small className="import-warning">
                          {b.categories.find((c) => c.id === r.categoryId)
                            ?.name || "Needs classification"}
                        </small>
                      </td>
                      <td>
                        {["USD", "EUR", "GBP"].includes(r.currency)
                          ? money(r.amountMinor, r.currency)
                          : r.currency}
                      </td>
                      <td>
                        {r.error ? (
                          <span className="error-text">{r.error}</span>
                        ) : (
                          <span
                            className={`badge ${r.duplicate ? "neutral" : "success"}`}
                          >
                            {r.duplicate
                              ? "Already imported · skip"
                              : "New draft"}
                          </span>
                        )}
                        {!r.error &&
                          !r.duplicate &&
                          r.warnings?.map((w) => (
                            <small key={w} className="import-warning">
                              {w}
                            </small>
                          ))}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            {errors > 0 && (
              <p className="error-text" role="alert">
                Fix the mapping or CSV to resolve all invalid rows before
                importing.
              </p>
            )}
          </>
        )}
      </div>
      <footer className="review-footer">
        <span>Imports create drafts. Nothing is posted automatically.</span>
        <div>
          <button className="button secondary" onClick={onClose}>
            Cancel
          </button>
          <button
            className="button primary"
            disabled={!valid || errors > 0 || busy}
            onClick={async () => {
              const ok = await mutate(
                [{ type: "CommitImport", accountId: account, rows: preview }],
                `${valid} expense drafts imported.`,
              );
              if (ok) onClose();
            }}
          >
            <Check size={16} />
            Import {valid || ""} draft{valid !== 1 ? "s" : ""}
          </button>
        </div>
      </footer>
    </Modal>
  );
}
