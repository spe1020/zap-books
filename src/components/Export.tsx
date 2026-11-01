import { useMemo, useState } from "react";
import { Download, FileSpreadsheet } from "lucide-react";
import type { BusinessData, Currency } from "../domain/types";
import { fiatCurrencies } from "../domain/types";
import { exportBalances, exportRecords } from "../domain/export";
import { today } from "../domain/ledger";
import type { ExportFilter, ExportFormat } from "../domain/export";
import { Modal, Empty } from "./common";
export function ExportDialog({
  b,
  onClose,
}: {
  b: BusinessData;
  onClose: () => void;
}) {
  const [format, setFormat] = useState<ExportFormat | "balances">("expenses");
  const [asOf, setAsOf] = useState(today());
  const [balanceCurrency, setBalanceCurrency] = useState<Currency>(
    b.profile.currency,
  );
  const [filter, setFilter] = useState<ExportFilter>({
    from: "",
    to: "",
    currency: "",
  });
  const [downloaded, setDownloaded] = useState(false);
  const previewColumns =
    format === "expenses"
      ? [3, 4, 6, 7, 8, 9]
      : format === "balances"
        ? [4, 5, 6, 8]
        : [3, 4, 8, 10, 11, 12, 15];
  const result = useMemo(() => {
    try {
      return {
        data:
          format === "balances"
            ? exportBalances(b, asOf, balanceCurrency)
            : exportRecords(b, format, filter),
        error: "",
      };
    } catch (e) {
      return { data: undefined, error: (e as Error).message };
    }
  }, [b, format, filter, asOf, balanceCurrency]);
  function download() {
    if (!result.data?.rows.length) return;
    const url = URL.createObjectURL(
      new Blob([result.data.csv], { type: "text/csv;charset=utf-8" }),
    );
    const link = document.createElement("a");
    link.href = url;
    link.download = result.data.filename;
    document.body.append(link);
    link.click();
    link.remove();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
    setDownloaded(true);
  }
  return (
    <Modal title="Export records" onClose={onClose} wide>
      <div className="import-body">
        <div className="import-intro">
          <div className="section-icon">
            <FileSpreadsheet size={24} />
          </div>
          <div>
            <h3>Your records, ready to take with you.</h3>
            <p>
              Export {b.profile.name}’s records for a spreadsheet or accountant.
              Other businesses are excluded.
            </p>
          </div>
        </div>
        <label>
          Export format
          <select
            value={format}
            onChange={(e) => {
              setFormat(e.target.value as ExportFormat | "balances");
              setDownloaded(false);
            }}
          >
            <option value="expenses">Posted expenses · CSV</option>
            <option value="journal">
              Journal entries and corrections · CSV
            </option>
            <option value="balances">Account balances · CSV</option>
          </select>
        </label>
        <p className="small muted export-description">
          {format === "expenses"
            ? "One row per current posted expense, with the account that paid it. Paid personally shows Loan or Contribution. Replaced originals, drafts, and transfers are excluded."
            : format === "balances"
              ? "One row per bank, card, loan and owner equity account in one currency, as of the date you choose. Cards and loans show what's owed."
              : "One row per journal line, including original entries, reversals, replacements, and transfers with their type and purpose. Debit and credit amounts remain separate by currency."}
        </p>
        {format === "balances" ? (
          <div className="field-row">
            <label>
              As of
              <input
                type="date"
                value={asOf}
                onChange={(e) => {
                  setAsOf(e.target.value);
                  setDownloaded(false);
                }}
              />
            </label>
            <label>
              Balance currency
              <select
                value={balanceCurrency}
                onChange={(e) => {
                  setBalanceCurrency(e.target.value as Currency);
                  setDownloaded(false);
                }}
              >
                {fiatCurrencies.map((c) => (
                  <option key={c}>{c}</option>
                ))}
              </select>
            </label>
          </div>
        ) : (
          <div className="field-row">
            <label>
              From date
              <input
                type="date"
                value={filter.from}
                onChange={(e) => {
                  setFilter({ ...filter, from: e.target.value });
                  setDownloaded(false);
                }}
              />
            </label>
            <label>
              To date
              <input
                type="date"
                value={filter.to}
                onChange={(e) => {
                  setFilter({ ...filter, to: e.target.value });
                  setDownloaded(false);
                }}
              />
            </label>
            <label>
              Export currency
              <select
                value={filter.currency}
                onChange={(e) => {
                  setFilter({
                    ...filter,
                    currency: e.target.value as Currency | "",
                  });
                  setDownloaded(false);
                }}
              >
                <option value="">All currencies · separate amounts</option>
                {fiatCurrencies.map((c) => (
                  <option key={c}>{c}</option>
                ))}
              </select>
            </label>
          </div>
        )}
        <p className="small muted export-description">
          {format === "balances"
            ? "CSV files contain financial information and are not encrypted."
            : "Leave dates blank for all history. CSV files contain financial information and are not encrypted. Receipt files are not included; open them from the corresponding transaction."}
        </p>
        {result.error && (
          <p className="alert error" role="alert">
            {result.error}
          </p>
        )}
        {result.data && (
          <>
            <div className="flex-between spaced-heading">
              <h3>Preview</h3>
              <span className="small">
                {result.data.rows.length} rows · first 5 shown
              </span>
            </div>
            {result.data.rows.length ? (
              <div
                className="table-scroll import-table"
                tabIndex={0}
                role="region"
                aria-label="Export preview"
              >
                <table>
                  <thead>
                    <tr>
                      {previewColumns.map((column) => (
                        <th key={column}>{result.data!.headers[column]}</th>
                      ))}
                    </tr>
                  </thead>
                  <tbody>
                    {result.data.rows.slice(0, 5).map((row, i) => (
                      <tr key={i}>
                        {previewColumns.map((column) => (
                          <td key={column}>{row[column] || "—"}</td>
                        ))}
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            ) : (
              <Empty title="No records in this selection">
                Choose a different period or currency, or post an expense first.
              </Empty>
            )}
          </>
        )}
        {downloaded && (
          <p role="status" className="validation-note">
            CSV download requested. Check your browser’s downloads.
          </p>
        )}
      </div>
      <footer className="review-footer">
        <span>Amounts use explicit currencies. No conversion is applied.</span>
        <div>
          <button className="button secondary" onClick={onClose}>
            Close
          </button>
          <button
            className="button primary"
            disabled={!result.data?.rows.length}
            onClick={download}
          >
            <Download size={16} />
            Download CSV
          </button>
        </div>
      </footer>
    </Modal>
  );
}
