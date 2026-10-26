# Receipt extraction (stage 1)

**Read receipt** on the Review screen asks a vision model running on your own
computer to suggest an expense's vendor, date, total, currency, receipt number,
subtotal, tax and category, each with the text it was read from. You accept,
edit or ignore each suggestion, then approve through the usual validation.
Nothing is applied without a click and nothing is ever approved automatically.

**Privacy:** receipts never leave your computer. The browser sends the receipt
to the extractor service on `127.0.0.1`, which sends it to Ollama on
`127.0.0.1`. There is no cloud model, and the service refuses to use an Ollama
address that isn't on this computer.

## Set up on a Mac

1. Install the tools with [Homebrew](https://brew.sh):

   ```bash
   brew install ollama poppler
   ```

2. Start Ollama and leave it running:

   ```bash
   brew services start ollama
   ```

   (Or open the Ollama app, or run `ollama serve` in a terminal.)

3. Download the vision model (about 6 GB):

   ```bash
   ollama pull qwen2.5vl:7b
   ```

   This is the default until the benchmark picks a model; pass another with
   `--model`.

4. Install Rust if you haven't: follow [rustup.rs](https://rustup.rs). The
   repository pins the toolchain, so rustup installs the right version on
   first use.

5. In the repository, install the app's packages once:

   ```bash
   npm ci
   ```

6. Start the extractor in one terminal:

   ```bash
   cargo run -p extractor
   ```

   It prints `Receipt reading is running at http://127.0.0.1:8787 …`. If
   Poppler, Ollama or the model is missing, it says what to install.

7. Start the app in another terminal and open http://localhost:5173:

   ```bash
   npm run dev
   ```

8. Open an expense, attach a receipt, and click **Read receipt**. Try the
   fictional receipts in `docs/samples/`.

If the extractor isn't running, **Read receipt** is disabled with "Receipt
reading isn't running on this computer. Enter the details manually." Everything
else works as before.

### Options

```text
cargo run -p extractor -- [--port 8787] [--ollama http://127.0.0.1:11434]
                          [--model qwen2.5vl:7b] [--allowed-origin <origin>]...
                          [--timeout-seconds 90]
```

`--allowed-origin` replaces the default origins (`http://127.0.0.1:5173`,
`http://localhost:5173`, and the same on port 4173 for `npm run preview`) the
first time it is given. Set `VITE_EXTRACTOR_URL` if the extractor runs on
another port; the app needs no environment variables otherwise.

## What runs where

```text
Browser (React)            Extractor (Rust, 127.0.0.1)              Ollama (127.0.0.1)
Review ── POST /extract ─▶ check file ─ render PDF pages ─────────▶ vision model
       ◀── suggestions ─── validate and normalize with ledger-core ◀── JSON
```

- **Browser:** sends the attached receipt and the business's category ids and
  names. It shows suggestions as plain text and records what you did with
  them (`ExtractionRecord`).
- **Extractor** (`crates/extractor`): accepts PNG, JPEG, WebP or PDF up to
  10 MB, identified by content. For PDFs, Poppler renders the first three
  pages and extracts the text layer when there is one. It asks the model for
  a fixed JSON schema, then re-checks every value: amounts with
  `ledger-core`'s exact money rules (comma decimals are never guessed), a
  total that disagrees with the subtotal and tax becomes unclear, dates must
  be real and not in the future, currencies must be USD, EUR or GBP, and the
  category must be one sent in. A value without evidence is unclear.
- **Model:** sees only the receipt, the category list, and the fixed prompt in
  `crates/extractor/src/prompt.rs`. It has no tools; its only output is the
  schema. The prompt tells it that text on the receipt addressed to an AI is
  receipt content, never instructions.

The service only answers requests whose `Host` is `127.0.0.1:<port>` or
`localhost:<port>` (this blocks DNS rebinding) and whose `Origin`, if any, is
one of the app's. It listens on `127.0.0.1` only. Requests time out after
90 seconds.

## What is logged

The extractor logs to its terminal: timings, the model name, page counts,
whether a PDF text layer was used, and each field's status (`found`,
`unclear`, `absent`). It never logs receipt contents, evidence, or extracted
values. PDF pages are rendered into a private temporary folder that is deleted
as soon as the receipt is read.

In the app, each reading is saved with the expense as an `ExtractionRecord`:
the model, prompt version, proposed fields with their evidence, and, once the
expense is approved, whether you accepted, edited, ignored, or couldn't use
each suggestion. It stays in the browser's local storage like every other
record.

## Known limits

- Restaurant receipts with a tip: the total includes the tip but subtotal plus
  tax doesn't, so the total is marked unclear by design. Enter it yourself.
- Receipts in currencies other than USD, EUR and GBP are marked unclear.
- The business purpose is never suggested; only you know why you bought it.
- Receipt numbers, subtotals and taxes are shown for checking; the expense
  form has no fields for them.

## Benchmark

```bash
cargo run -p extractor --release --bin bench -- --corpus <dir> --model <name> [--model <name>]...
```

It runs the same pipeline over a folder of labelled receipts and writes
`bench-results.md` (aggregate numbers only) and
`bench-results.details.local.md` (per-receipt verdicts, which name files; keep
it local). Both are gitignored. Label each receipt with `<name>.expected.json`
next to it:

```json
{
  "vendor": "Paper & Pine",
  "date": "2026-10-24",
  "total": "42.50",
  "currency": "USD",
  "reference": "DEMO-RECEIPT-001",
  "subtotal": "40.00",
  "tax": "2.50",
  "category": "northstar-category-1",
  "clear": true,
  "kind": "pdf"
}
```

A missing key isn't scored; `null` means the receipt doesn't show that field.
`vendor` may be a list of accepted spellings. Amounts are text as printed.
`clear: false` leaves a receipt out of "Total, clear receipts". `kind` is a
free tag counted in the report. An optional `categories.json` at the corpus
root lists `{ "id", "name" }` categories to send. `docs/samples/` is a small
labelled corpus of fictional receipts. Real receipts never go in the
repository.
