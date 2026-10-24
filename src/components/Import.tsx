import { useMemo, useState } from "react";
import { Upload, FileSpreadsheet, Check, Download } from "lucide-react";
import type { BusinessData } from "../domain/types";
import { commitImport, parseCSV, previewImport } from "../domain/import";
import type { CSVMapping } from "../domain/import";
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
  });
  const [account, setAccount] = useState(
    b.accounts.find((a) => a.kind === "bank")!.id,
  );
  const [convention, setConvention] = useState<"negative" | "positive">(
    "negative",
  );
  const [error, setError] = useState("");
  const preview = useMemo(
    () =>
      rows.length ? previewImport(b, rows, mapping, account, convention) : [],
    [b, rows, mapping, account, convention],
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
      const lookup = (names: string[]) =>
        parsed[0].find((h) => names.includes(h.toLowerCase())) ?? "";
      setMapping({
        date: lookup(["date", "transaction date"]),
        description: lookup(["description", "merchant", "vendor"]),
        amount: lookup(["amount", "total"]),
        currency: lookup(["currency"]),
        reference: lookup(["reference", "id", "transaction id"]),
      });
    } catch (e) {
      setError((e as Error).message);
      setRows([]);
    }
  }
  function sample() {
    const csv = `date,description,amount,currency,reference\n${currentPeriod()}-23,Paper & Pine,-42.50,USD,DEMO-IMPORT-001\n${currentPeriod()}-22,Cloud Tools,-29.00,USD,DEMO-IMPORT-002\n`;
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
              Upload a CSV, map its columns, and review before adding drafts to
              the inbox.
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
        <label className="upload-zone csv-upload">
          <Upload size={22} />
          <span>
            {filename || "Choose a CSV file"}
            <small>Up to 2 MB · 2,000 rows · YYYY-MM-DD dates</small>
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
          Expense outflows only. Remove income and transfers before import. A
          provider reference is recommended; without one, identical date,
          description, amount, currency, and account rows are treated as
          repeats.
        </p>
        {!!rows.length && (
          <>
            <h3 className="spaced-heading">1. Map columns</h3>
            <div className="mapping-grid">
              {(Object.keys(mapping) as (keyof CSVMapping)[]).map((key) => (
                <label key={key}>
                  {key}
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
            <div className="table-scroll import-table">
              <table>
                <thead>
                  <tr>
                    <th>Row</th>
                    <th>Date</th>
                    <th>Description</th>
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
                (current) => commitImport(current, preview, account),
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
