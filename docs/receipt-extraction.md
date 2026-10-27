# Receipt extraction (stage 1)

**Read receipt** on the Review screen asks a vision model running on your own
computer to suggest an expense's vendor, date, total, currency, receipt number,
subtotal, tax, fees, discounts, tip and category, each with the text it was
read from. You accept, edit or ignore each suggestion, then approve through the
usual validation. Nothing is applied without a click and nothing is ever
approved automatically.

The model transcribes; Rust does the arithmetic. The model copies printed
amounts with their evidence, and every sum, comparison and decision about the
total happens in the extractor with `ledger-core`'s money rules.

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
  a fixed JSON schema, with fields in a fixed order (vendor, date, currency,
  total, subtotal, tax, fees, discounts, tip, reference, category; the
  model fills them in that order). It then re-checks every value: amounts
  with `ledger-core`'s exact money rules (comma decimals are never guessed;
  zero is allowed for every amount but the total), dates must be real and not
  in the future, currencies must be USD, EUR or GBP, and the category must be
  one sent in. A value without evidence is unclear. See
  [Fields and the total check](#fields-and-the-total-check).
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

## Fields and the total check

The prompt (`receipt-v3`) defines each field literally, and the benchmark
labels follow the same definitions:

- **subtotal:** only a line labelled Subtotal or an equivalent ("Sub Total",
  "Merchandise"). Otherwise absent; never computed or inferred.
- **tax:** every printed tax line. None printed means absent, not `0.00`. A
  line printed as `0.00` is a line. The tax is the sum of its lines.
- **fees:** every surcharge or convenience, service or delivery fee (and a
  service charge) added after the subtotal. A fee listed among the items is
  already in the subtotal and is left out.
- **discounts:** every coupon, savings, reward or markdown line printed as a
  deduction. Stored as positive amounts whatever sign is printed (`0.25-`,
  `-0.25`, `(0.25)`). A "Total saved" summary is not a discount line.
- **tip:** only a tip the customer actually added (a filled-in tip line). A
  table of suggested tips is not a tip.
- **reference:** preferably a number labelled as a receipt, transaction,
  invoice or order number; never a card, account, approval or phone number.
- **total:** the amount actually paid, including tax, fees and tip.

Tax, fees and discounts are lists: one amount and its evidence per printed
line. One line that can't be read exactly makes the whole list unclear.

The total stays found only if it equals either `subtotal + tax + fees + tip`
or `subtotal − discounts + tax + fees + tip` (receipts differ on whether the
printed subtotal is before or after savings). Anything not read counts as 0.
Without a subtotal line there is nothing to check against, so no check runs.
When the check fails, the total is unclear and the warning names what was
read, for example "Subtotal $50.00, tax $3.00 and fees $1.50 add up to $54.50,
but the total reads $55.50. Check the amount." When only a subtotal and tax
were read and the total is higher, it ends "If you added a tip, check the
amount matches what you paid." instead, since a tip written in by hand is the
usual cause.

A negative total stays unclear, because it may be a refund and a refund must
never become an expense: "The only amount is a payment or refund line
(−$24.00). Enter the total yourself."

## Known limits

- Receipts in currencies other than USD, EUR and GBP are marked unclear.
- The business purpose is never suggested; only you know why you bought it.
- Receipt numbers, subtotals, taxes, fees, discounts and tips are shown for
  checking; the expense form has no fields for them.
- Amounts printed on a different line from their labels (as on some
  fast-food receipts) are often read against the wrong label. The total check
  usually catches this and marks the total unclear.

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
  "tip": null,
  "category": "northstar-category-1",
  "clear": true,
  "kind": "pdf"
}
```

A missing key isn't scored; `null` means the receipt doesn't show that field
(use `"tip": null` when there is no tip line).
`vendor` may be a list of accepted spellings. Amounts are text as printed.
`clear: false` leaves a receipt out of "Total, clear receipts". `kind` is a
free tag counted in the report. An optional `categories.json` at the corpus
root lists `{ "id", "name" }` categories to send. `docs/samples/` is a small
labelled corpus of fictional receipts. Real receipts never go in the
repository.
