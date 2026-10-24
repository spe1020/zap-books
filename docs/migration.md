# Expense history import and CSV export

This release migrates **paid expense history into reviewable drafts**. It does not migrate complete books, opening balances, unpaid invoices/bills, tax ledgers, reconciliation state, or attachments from another accounting product. There is no QuickBooks or Zoho API connection. The source selector records provenance; it does not log into either service.

## Import from an existing accounting system

1. Export the paid expenses you want to bring over from QuickBooks, Zoho Books, or another system. Export layouts vary by product, edition, region, and report. Convert XLSX to CSV locally if necessary.
2. Prepare a UTF-8 comma-delimited CSV with a header and one row per expense. Remove report titles, subtotal/total rows, split ledger lines, unpaid bills, invoices, income, transfers, refunds, and tax breakdown rows. Import one payment account and currency per file. Keep the original export for comparison.
3. In Business Finance, select the correct business, then **Import CSV → Type of import → Expense history**. Select the source system. This is a source label, not an automatic migration guarantee.
4. Choose the date format explicitly: `YYYY-MM-DD`, `MM/DD/YYYY`, or `DD/MM/YYYY`. Choose the target payment account and positive/negative amount convention. The selected account applies to every row. If a source payment-account column is mapped, a file containing multiple accounts is rejected.
5. Review the suggested column mapping. Common labels such as `Expense Date`, `Vendor Name`, `Memo/Description`, `Expense Amount`, and `Reference Number` are recognized. Other names can be mapped manually. Date, description, and amount are required. Vendor, category, purpose, currency, source account, reference, and transaction type are optional mappings.
6. Check the preview counts, dates, amounts, categories, warnings, and total against the same rows in your original export. Resolve invalid rows before importing. A mapped transaction-type column only accepts expense/check/credit-card-charge types; it rejects bills, transfers, income, and totals. If no type column exists, you must ensure the file contains only supported expenses.
7. Import drafts, then review and approve them in the inbox. History imports retain their original vendor, amount, date, category, provider, reference, and filename. Those source details remain visible through edits and corrections. They are accounting history evidence, not independent bank-payment evidence.

**Correction behavior:** expense-history amounts can be edited through **Correct expense**, while the original imported values remain preserved as source evidence. Expenses imported using **Bank or card payments** retain locked amounts, dates, currencies, and accounts, including during corrections. This prototype does not yet provide bank-payment adjustment or unmatching.

Only exact category-name matches in the current business are prefilled. Create your categories in Settings before importing, or classify unmatched expenses during review. No category or business purpose is invented. Imported dates become accounting dates on approval; select the corresponding historical month in Overview to inspect the report.

Amounts support a dot decimal separator with up to two places: `1234.56` or quoted `"1,234.56"`. Currency symbols, decimal-comma formats, and accounting parentheses must be normalized before import. Supported currencies are USD, EUR, and GBP; no exchange rates are inferred. Limits remain 2 MB and 2,000 rows per file.

Use a stable transaction ID as the reference whenever possible. Deduplication uses business, selected account, source system, and source reference. Without a reference it uses the normalized date, vendor, description, amount, and currency within that scope. Changing the file name does not defeat deduplication. Conflicting dates/amounts/currencies under the same reference are blocked. Edits or corrections to the imported draft do not change the preserved source identity. Importing the same records under a different source system/account can create new drafts; possible-duplicate checks still apply on approval. A receipt attachment should be added to the existing draft.

Synthetic CSV tests exercise the mapping contract. Native exports from every QuickBooks/Zoho version have not been certified. Validate a small representative file and reconcile its row count and amounts before a larger migration. CSV history import is not a replacement for an accountant-reviewed full ledger conversion.

## Export your records

Choose **Export** from Overview, Inbox, or Transactions. Exports include only the selected business. Leave the date fields blank for all history, or use inclusive from/to dates. Choose a currency or include all currencies in separate rows.

- **Posted expenses · CSV:** one row per current posted expense, with vendor, date, description, decimal amount, explicit currency, category, payment account, purpose, IDs, source references, and receipt count. Drafts, transfers, and replaced originals are excluded. This represents current expense records, not the complete audit history.
- **Journal entries and corrections · CSV:** one row per journal line, including original postings, reversals, replacements, and transfers. Journal IDs, expense IDs, reversal links, debit/credit values, currencies, and timestamps preserve the accounting trail.

Preview the first five rows and total row count before downloading. Export does not alter records. CSVs contain unencrypted financial information; store them appropriately. Receipt files are not bundled—open them from their transactions. These files are spreadsheet/accountant exports, not a complete application backup or a guaranteed direct-import format for QuickBooks or Zoho. A receiving system may need its own column mapping.

Exports use UTF-8 with a BOM, CRLF lines, and quoted cells. User-entered text that could be interpreted as a spreadsheet formula is prefixed with an apostrophe. Monetary values are generated from integer minor units. Currencies are never silently combined.
