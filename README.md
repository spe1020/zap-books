# Business Finance

A standalone, frontend-first expense workspace for small businesses. Built with React, TypeScript, and Vite. The temporary product name is centralized in `src/config.ts`.

## Run on your computer

Install **Node.js 24** (the version used for development), then open a terminal in this project folder:

```bash
npm ci
npm run dev
```

Open the address printed by Vite in your browser. Keep the terminal running; press **Ctrl+C** to stop. This starts a local development server, not an external deployment. Do not open `index.html` directly from your file manager. The `dev` script listens on available network interfaces for cloud development; for use only on your own computer, run `npm run dev -- --host 127.0.0.1`.

If using nvm, run `nvm use` first. No API keys, database service, bank connection, or environment variables are needed. Node.js 22.12+ is supported by the toolchain; Node.js 24 is recommended and tested.

For a local production build:

```bash
npm run build
npm run preview
```

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

## Import payments from CSV

Choose **Import CSV** and download the sample, or use your own CSV:

```csv
date,description,amount,currency,reference
2026-10-23,Paper & Pine,-42.50,USD,DEMO-IMPORT-001
2026-10-22,Cloud Tools,-29.00,USD,DEMO-IMPORT-002
```

Select the payment account and amount-sign convention, map the columns, and inspect the preview before committing. Dates must be `YYYY-MM-DD`. Date, description, and amount are required; currency defaults to the account currency. This release imports expense outflows only. Remove income and transfers from the file. Limits: 2 MB and 2,000 transactions per file.

Each accepted payment creates one draft. Attach its receipt to that existing draft. Imported payment amount, date, currency, and account are locked during review to keep the evidence consistent.

Repeats are skipped using business + account + provider reference. Without a reference, the key includes date, normalized description, amount, currency, and account. Amount alone is never sufficient. Conflicting details under an existing reference are rejected. Truly separate transactions with identical fields need distinct provider references; fallback matching cannot distinguish them. Imports create drafts, never posted entries.

## Data and demo boundaries

- IndexedDB stores records and receipt Blobs in your current browser, scoped to this application's origin. Same-browser reloads persist data; another device, browser profile, hostname, or port has a separate store. Clearing site data or using **Settings → Reset demo** removes local work. There is no cloud backup or cross-device synchronization.
- Business IDs enforce local workspace separation. **This is not production authentication or authorization.** Anyone with access to the browser profile can access or alter its local data. Attachments are not encrypted at rest by this app.
- Saving is atomic across records and evidence. Revision checks reject stale-tab writes rather than overwriting newer records; reload the stale tab before retrying. Reset requires typing `RESET` and replaces both businesses.
- Amounts use integer minor units. USD, EUR, and GBP are supported with two decimal places. Reports never combine currencies or assume exchange rates. Transfers do not count as expenses. Corrections post an exact reversal on the original accounting date and a linked replacement; closed accounting periods are outside this prototype.
- The assistant is deterministic demo logic using saved records and explicit rules. There is no live model or OCR. Uploaded document text is not extracted or interpreted; any instructions inside documents are untrusted content. Suggestions are labeled simulated and require review. Rules are saved only when explicitly requested and can be edited or paused in Settings.
- Transport is an in-memory mock. No public relays, key collection, real Nostr publishing, payment execution, live banking, payroll, tax filing, or full accounting-platform coverage is included. The local ledger is prototype domain logic, not a production accounting backend.
- PDF preview depends on the browser's PDF support; an Open link is available when inline display is unavailable. User uploads support PNG, JPEG, WebP, and PDF; SVG receipts are trusted fictional seed assets only.

## Verification

```bash
npm test                 # Financial domain tests
npm run build            # TypeScript checks and production bundle
npm run test:e2e         # Chromium browser workflows and accessibility
npm run format:check     # Source formatting
```

Browser tests use `/usr/bin/chromium` if present, or a Playwright-managed Chromium elsewhere. To install the managed browser on your computer:

```bash
npx playwright install chromium
npm run test:e2e
```

Set `CHROMIUM_PATH` if you want to use a different Chromium executable. Tests start Vite automatically when needed and use isolated browser contexts. They cover posting and totals, attachment persistence, import/re-import, corrections, business switching, explicit rules, errors and empty reports, deliberate reset, stale-tab conflicts, keyboard focus, mobile layout, and automated accessibility checks. These checks are not a complete manual accessibility certification or cross-browser support matrix.

## Code boundaries and next milestone

See [Architecture and backend handoff](docs/architecture.md) for the domain model, storage/AI/transport interfaces, event contract, and recommended next milestone: **one authoritative accounting service, encrypted document storage, and an existing private Nostr relay with enforced authentication and business authorization**.
