# Business Finance

A standalone, frontend-first expense workspace for small businesses. Built with Rust, TypeScript, and Vite. The temporary product name is centralized in `src/config.ts`.

## Run on your computer

Install **Node.js 24** (the version used for development), then open a terminal in this project folder:

```bash
npm ci
npm run dev
```

Open the address printed by Vite in your browser. Keep the terminal running; press **Ctrl+C** to stop. This starts a local development server, not an external deployment. Do not open `index.html` directly from your file manager. The `dev` and `preview` scripts listen on `127.0.0.1` only, so nothing else on your network can reach them. For cloud development, where the browser runs on another machine, use `npm run dev:cloud` (or `npm run preview:cloud`), which listen on every network interface.

If using nvm, run `nvm use` first. No API keys, database service, bank connection, or environment variables are needed. Node.js 22.12+ is supported by the toolchain; Node.js 24 is recommended and tested.

For a local production build (it builds `crates/protocol-wasm` first, so it needs Rust and `wasm-pack`; see Rust ledger core):

```bash
npm run build
npm run preview
```

### With the ledger service

Set the service's address and its public key, and the app starts at sign-in, with the local demo a click away:

```bash
npm run wasm:build   # once, and after protocol changes
VITE_SERVICE_URL=http://127.0.0.1:8787 VITE_SERVICE_PUBKEY=<hex public key> npm run dev
```

Start the service with `--allowed-origin` set to the app's address (`docs/service.md`). Signing in needs a NIP-07 signer extension that supports NIP-44: the app never asks for a secret key.

The built files are in `dist/`. Preview uses a different port, which means a separate browser storage origin and a fresh demonstration workspace.

## Try the complete workflow

1. Start in **Northstar Studio**, a fictional design business. The Overview uses posted journal entries for the selected month and currency.
2. Open **Expense inbox**, review **Figma**, inspect the fictional receipt, and apply the simulated classification suggestion. Click **Approve & post**. Approval records an accounting entry; it does not send money.
3. Check the updated Overview and locate the posted expense in **Transactions**.
4. Choose **New expense**. Enter a vendor, date, description, positive amount, currency, category, matching payment account, and business purpose. Attach a PNG, JPEG, WebP, or PDF up to 10 MB. Save a draft or approve it when valid.
5. Reload the page. Saved fields and receipt attachments remain available.
6. Open the posted transaction and choose **Correct expense**. Change the fields, provide a reason, and post the correction. Follow the links between the original and replacement; the original entry and its reversal remain inspectable.
7. Use the business switcher to open **Fieldwork Landscapes**, a fictional landscaping business. Its records, evidence, accounts, rules, reports, and assistant context are separate.

The seeded inbox also contains an incomplete expense, a receipt already matched to an imported payment, and a possible duplicate. Transactions include an account transfer and a corrected expense. Examples use the month when the workspace is first seeded; select that month in Overview if testing later.

### Upload a demo receipt

Use the fictional [Paper & Pine receipt (PDF)](docs/samples/demo-receipt.pdf) or [the same receipt as a PNG](docs/samples/demo-receipt.png). The printable [HTML source](docs/samples/demo-receipt.html) is also included. After pulling the repository, these files are in `docs/samples/`.

In **Northstar Studio → New expense**, attach either the PDF or PNG and enter:

| Field             | Value                                     |
| ----------------- | ----------------------------------------- |
| Vendor            | Paper & Pine                              |
| Date              | 2026-10-24                                |
| Description       | Notebooks, pens, and presentation folders |
| Amount / currency | 42.50 / USD                               |
| Category          | Office & supplies                         |
| Payment account   | Business card                             |
| Business purpose  | Stationery for client project planning    |

The receipt totals $40.00 of items plus $2.50 of sample sales tax. Enter the full **$42.50** as the expense amount; this prototype does not split tax into a separate ledger account. Save the draft, reopen it to check the attachment, then approve it. October 2026 USD spending should increase by $42.50. Use one format per test expense; the PDF and PNG represent the same fictional purchase. With the local extractor running, **Read receipt** suggests these fields instead; see [Receipt extraction](docs/receipt-extraction.md).

## Import payments from CSV

Choose **Import CSV** and download the sample, or use your own CSV:

```csv
date,description,amount,currency,reference
2026-10-23,Paper & Pine,-42.50,USD,DEMO-IMPORT-001
2026-10-22,Cloud Tools,-29.00,USD,DEMO-IMPORT-002
```

Select the payment account and amount-sign convention, map the columns, and inspect the preview before committing. Select the matching date format; `YYYY-MM-DD` is the default. Date, description, and amount are required; currency defaults to the account currency. This release imports expense outflows only. Remove income and transfers from the file. Limits: 2 MB and 2,000 transactions per file.

Each accepted payment creates one draft. Attach its receipt to that existing draft. Imported bank/card payment amount, date, currency, and account stay locked during review and corrections to keep the evidence consistent. Category and descriptive fields can be corrected. Manual expenses and **Expense history** imports allow amount corrections through a reversal and replacement. Adjusting or unmatching a bank payment is not supported in this prototype.

Repeats are skipped using business + account + provider reference. Without a reference, the key includes date, normalized description, amount, currency, and account. Amount alone is never sufficient. Conflicting details under an existing reference are rejected. Truly separate transactions with identical fields need distinct provider references; fallback matching cannot distinguish them. Imports create drafts, never posted entries.

## Migration and export

Use **Import CSV → Expense history** for paid-expense CSVs exported from QuickBooks, Zoho Books, or another system. The importer offers explicit date formats, vendor/category/purpose mapping, source evidence, preview totals, and repeat-import checks. Records remain drafts until reviewed. This does not migrate complete books or connect to those services.

Use **Export** to download current posted expenses or the full journal history, filtered by business, date, and currency. See [Expense history import and CSV export](docs/migration.md) for preparation requirements and limitations.

## Data and demo boundaries

- IndexedDB stores records and receipt Blobs in your current browser, scoped to this application's origin. Same-browser reloads persist data; another device, browser profile, hostname, or port has a separate store. Clearing site data or using **Settings → Reset demo** removes local work. There is no cloud backup or cross-device synchronization.
- Business IDs enforce local workspace separation. **This is not production authentication or authorization.** Anyone with access to the browser profile can access or alter its local data. Attachments are not encrypted at rest by this app.
- Saving is atomic across records and evidence. Revision checks reject stale-tab writes rather than overwriting newer records; reload the stale tab before retrying. Reset requires typing `RESET` and replaces both businesses.
- Amounts use integer minor units. USD, EUR, and GBP are supported with two decimal places. Reports never combine currencies or assume exchange rates. Transfers do not count as expenses. Corrections post an exact reversal on the original accounting date and a linked replacement; closed accounting periods are outside this prototype.
- **Read receipt** uses a vision model running on your own computer (Ollama, through the local extractor in `crates/extractor`) to suggest receipt fields with the text each was read from. Receipts never leave the computer. Every suggestion needs a click, values are re-checked with the ledger's money and date rules, unclear values are shown as unclear, and nothing is approved automatically. Any instructions inside documents are untrusted content. Without the extractor running, the button is disabled and manual entry works as before. See [Receipt extraction](docs/receipt-extraction.md).
- The assistant and the category rule suggestion are deterministic demo logic using saved records and explicit rules; they don't read documents. Rule suggestions are labeled simulated and require review. Rules are saved only when explicitly requested and can be edited or paused in Settings.
- Transport is an in-memory mock. No public relays, key collection, real Nostr publishing, payment execution, live banking, payroll, tax filing, or full accounting-platform coverage is included. The local ledger is prototype domain logic, not a production accounting backend.
- PDF preview depends on the browser's PDF support; an Open link is available when inline display is unavailable. User uploads support PNG, JPEG, WebP, and PDF; SVG receipts are trusted fictional seed assets only.

## Verification

```bash
npm test                 # Financial domain tests
npm run build            # TypeScript checks and production bundle
npm run test:e2e         # Chromium browser workflows and accessibility
npm run test:e2e:remote  # The app against a local ledger service (needs DATABASE_URL)
npm run format:check     # Source formatting
```

Browser tests use `/usr/bin/chromium` if present, or a Playwright-managed Chromium elsewhere. To install the managed browser on your computer:

```bash
npx playwright install chromium
npm run test:e2e
```

`test:e2e:remote` starts the ledger service (with an invented test key) on Postgres from `DATABASE_URL`, and a second Vite server configured for it. Each test injects a test-only NIP-07 signer whose keys stay in the Playwright process; the app has no signer of its own.

Set `CHROMIUM_PATH` if you want to use a different Chromium executable. Tests start Vite automatically when needed and use isolated browser contexts. They cover posting and totals, attachment persistence, import/re-import, corrections, business switching, explicit rules, errors and empty reports, deliberate reset, stale-tab conflicts, keyboard focus, mobile layout, and automated accessibility checks. These checks are not a complete manual accessibility certification or cross-browser support matrix.

## Rust ledger core

`crates/ledger-core` ports the TypeScript accounting rules into a library; the browser application still uses TypeScript. Install Rust through rustup, which reads the pinned toolchain in `rust-toolchain.toml`, then run:

```bash
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
npm run parity:fixtures
```

`crates/protocol-wasm` builds the protocol for the browser with [wasm-pack](https://rustwasm.github.io/wasm-pack/) (`cargo install wasm-pack --version 0.15.0 --locked`). The build goes to `crates/protocol-wasm/pkg`, which is never committed. Its parity test checks that the WebAssembly build gives exactly the native build's answers:

```bash
npm run test:wasm    # builds the package, then runs fixtures/protocol through it
```

`cargo test` also runs the receipt extractor's tests (`crates/extractor`) against a fake model server; its PDF tests need Poppler (`brew install poppler`). No real model runs in tests.

The parity fixtures compare both implementations, including rejected imports and duplicate postings. Commit regenerated fixtures with any shared behavior change. See [Parity notes](PARITY_NOTES.md) for deliberate differences.

Rust journal construction requires `JournalEntry::new(&business, unchecked)`. Load saved JSON through `BusinessData` or `Workspace` so journals are checked against their business, accounts, categories, and currency. Context-free journal deserialization and conversion are intentionally unavailable. Valid stored JSON keeps the same shape; invalid journals fail loading instead of entering reports.

## Code boundaries and next milestone

See [Architecture and backend handoff](docs/architecture.md) for the domain model, storage/AI/transport interfaces, event contract, and recommended next milestone: **one authoritative accounting service, encrypted document storage, and an existing private Nostr relay with enforced authentication and business authorization**.
