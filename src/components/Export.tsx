import { useMemo, useState } from "react";
import { Download, FileSpreadsheet } from "lucide-react";
import type { BusinessData, Currency } from "../domain/types";
import { currencies } from "../domain/types";
import { exportRecords } from "../domain/export";
import type { ExportFilter, ExportFormat } from "../domain/export";
import { Modal, Empty } from "./common";
export function ExportDialog({
  b,
  onClose,
}: {
  b: BusinessData;
  onClose: () => void;
}) {
  const [format, setFormat] = useState<ExportFormat>("expenses");
  const [filter, setFilter] = useState<ExportFilter>({
    from: "",
    to: "",
    currency: "",
  });
  const [downloaded, setDownloaded] = useState(false);
  const previewColumns =
    format === "expenses" ? [3, 4, 6, 7, 8] : [3, 4, 8, 10, 11, 12];
  const result = useMemo(() => {
    try {
      return { data: exportRecords(b, format, filter), error: "" };
    } catch (e) {
      return { data: undefined, error: (e as Error).message };
    }
  }, [b, format, filter]);
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
              setFormat(e.target.value as ExportFormat);
              setDownloaded(false);
            }}
          >
            <option value="expenses">Posted expenses · CSV</option>
            <option value="journal">
              Journal entries and corrections · CSV
            </option>
          </select>
        </label>
        <p className="small muted export-description">
          {format === "expenses"
            ? "One row per current posted expense. Replaced originals, drafts, and transfers are excluded. Use the journal export for complete posting history."
            : "One row per journal line, including original entries, reversals, replacements, and transfers. Debit and credit amounts remain separate by currency."}
        </p>
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
              {currencies.map((c) => (
                <option key={c}>{c}</option>
              ))}
            </select>
          </label>
        </div>
        <p className="small muted export-description">
          Leave dates blank for all history. CSV files contain financial
          information and are not encrypted. Receipt files are not included;
          open them from the corresponding transaction.
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
