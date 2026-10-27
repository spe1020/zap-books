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

3. Download the vision model (about 20 GB):

   ```bash
   ollama pull qwen3-vl:30b-a3b-instruct
   ```

   This is the default. It uses about 22 GB of memory while loaded. On a
   machine with less, use `qwen3-vl:8b-instruct` (about 6 GB) and pass
   `--model qwen3-vl:8b-instruct`; in testing it once reported a wrong total
   as found, so check its totals with extra care.

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
                          [--model qwen3-vl:30b-a3b-instruct] [--allowed-origin <origin>]...
                          [--timeout-seconds 90] [--max-tokens 2048]
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
one of the app's. It listens on `127.0.0.1` only.

Every request to Ollama asks for thinking off (`think: false`) and caps the
answer at `--max-tokens` tokens (2048 by default), so a runaway generation
stops. Each result records whether the model reports thinking support
(`modelSupportsThinking`, from Ollama's `/api/show`); a thinking-only model
may ignore the request and spend its tokens thinking. Requests time out after
90 seconds. A request that times out is cancelled: the connection to Ollama
is closed, which makes Ollama stop generating. Before the next receipt, the
extractor sends Ollama a one-token request to confirm the model is idle; if
that doesn't finish within 30 seconds (or the timeout, if shorter), it
answers "The model is still finishing an earlier receipt. Try again in a
minute, or enter the details manually."

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

The prompt (`receipt-v4`) defines each field literally, and the benchmark
labels follow the same definitions.

**A field is found only if the extractor can confirm the value from its
evidence.** The prompt asks for every evidence string to include the line's
printed label (`TOTAL $54.50`, not `$54.50`). Every amount (total, subtotal,
tip, and each tax, fee and discount line) must be printed in its own evidence,
ignoring currency symbols, thousands separators, spaces and a printed sign or
parentheses. Otherwise it is unclear: "The value doesn't match the text it was
read from."

- **date:** the extractor reads the date in the evidence itself: numeric
  (`M/D/YY`, `D/M/YYYY`, `YYYY-MM-DD`, with `/`, `-` or `.`) or with an
  English month name (`Oct 12, 2026`, `12 October 2026`). Two-digit years are
  20YY. When day and month could be swapped (`03/04/26`), a USD receipt is
  read month first and a EUR or GBP receipt day first; without a currency the
  date is unclear. The date is found only if the evidence shows exactly one
  date and it equals the model's value. Otherwise: "Check the date; it
  couldn't be confirmed from the receipt."

- **subtotal:** only a line labelled Subtotal or an equivalent ("Sub Total",
  "Merchandise"). Never computed or inferred, and a line whose label contains
  Total is never a subtotal. The evidence must contain `sub` or `merchandise`,
  or the subtotal is absent.
- **tax:** every printed tax line. None printed means absent, not `0.00`. A
  line printed as `0.00` is a line. The tax is the sum of its lines.
- **fees:** every surcharge or convenience, service or delivery fee (and a
  service charge) added after the subtotal. A fee listed among the items is
  already in the subtotal and is left out. A fee line that prints a
  percentage must be that percentage of the subtotal, or of the subtotal and
  tax (as with service charges), give or take one cent. If not, fees and tax
  are both unclear: "The fee and tax may be swapped. Check both." Tax is never
  checked this way, because it often applies to only some items.
- **discounts:** every coupon, savings, reward or markdown line printed as a
  deduction. Stored as positive amounts whatever sign is printed (`0.25-`,
  `-0.25`, `(0.25)`). Each line's evidence must show a deduction (a minus
  sign, parentheses, or a word such as disc, coupon, savings, off or promo);
  a regular, original, clearance or sale price is not one. A savings summary
  ("Total saved", "You saved", YTD) is never a discount line. Either problem
  makes the whole list unclear.
- **tip:** only a tip the customer actually added. A Tip or Gratuity line with
  an amount, printed between the subtotal and the total, is a tip even when a
  table of suggested tips is also printed; the table alone is not a tip.
- **vendor:** the business name, usually in the logo or the largest text at
  the top; never a tagline, slogan or list of products or services.
- **reference:** only a number labelled as a receipt, transaction, invoice,
  order, check or bill number; never a terminal ID, trace number, payment or
  card-slip reference ID, survey code, card, account, approval or phone
  number, or product barcode. The value must be in its evidence (ignoring
  spaces and punctuation; a number of digits only must be whole printed
  numbers), and the evidence must contain one of `receipt`, `rcpt`,
  `transaction`, `trans`, `tran`, `invoice`, `inv`, `order`, `check`, `chk`
  or `bill`. Otherwise it is unclear.
- **total:** the amount actually paid, including tax, fees and tip. Never a
  savings, points, rewards or year-to-date figure. Its evidence must include
  one of the labels `total`, `amount`, `paid`, `payment`, `charged`,
  `balance`, `due` or `grand` ("The total was read without its label. Check
  the amount."), and a total whose evidence mentions savings, points, rewards,
  YTD or a year is unclear.

Tax, fees and discounts are lists: one amount and its evidence per printed
line. One line that can't be read exactly makes the whole list unclear.

The total stays found only if it equals either `subtotal + tax + fees + tip`
or `subtotal − discounts + tax + fees + tip` (receipts differ on whether the
printed subtotal is before or after savings). Anything not read counts as 0.
Without a subtotal line there is nothing to check against, so no check runs.
If the check fails with the fees but passes without them, the fees were
printed among the items and are already in the subtotal: they are left out,
the total stays found, and a warning says so.
When the check fails, the total is unclear and the warning names what was
read, for example "Subtotal $50.00, tax $3.00 and fees $1.50 add up to $54.50,
but the total reads $55.50. Check the amount." When only a subtotal and tax
were read and the total is higher, it ends "If you added a tip, check the
amount matches what you paid." instead, since a tip written in by hand is the
usual cause.

A negative total is taken as the amount paid only when its evidence says
payment, paid or charged and doesn't say refund, credit, return or reversal
(`Payment (card): -$24.00`). Any other negative total stays unclear, because it
may be a refund and a refund must never become an expense: "The only amount is
a payment or refund line (−$24.00). Enter the total yourself."

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
cargo run -p extractor --release --bin bench -- --corpus <dir> [--corpus <dir>]... --model <name> [--model <name>]...
```

`--corpus` can be given more than once. Each corpus is reported on its own
(numbered, so the report names no folders) and then all of them combined; each
uses its own `categories.json`.

### Holdout set

Keep a second folder of receipts that is never looked at when designing
extraction rules, and judge every change by its numbers (`--corpus <design>
--corpus <holdout>`). Rules tuned on one corpus can look better there than
they are, and the holdout shows whether they carry over.

### Models

The recommended run compares the non-thinking (instruct) Qwen3-VL models, with
the thinking variant as a reference point:

```bash
ollama pull qwen3-vl:8b-instruct
ollama pull qwen3-vl:30b-a3b-instruct
cargo run -p extractor --release --bin bench -- --corpus <dir> \
  --model qwen3-vl:8b-instruct --model qwen3-vl:30b-a3b-instruct --model qwen3-vl:8b
```

| Tag                         | Download | Thinking                            | Role                                    |
| --------------------------- | -------- | ----------------------------------- | --------------------------------------- |
| `qwen3-vl:8b-instruct`      | 6.2 GB   | none (`qwen3-vl-instruct` template) | candidate                               |
| `qwen3-vl:30b-a3b-instruct` | 20 GB    | none (`qwen3-vl-instruct` template) | default; needs about 22 GB while loaded |
| `qwen3-vl:8b`               | 6.2 GB   | always on (`qwen3-vl-thinking`)     | reference point only                    |

`ollama show <tag>` lists `thinking` under Capabilities only for the thinking
variant; `ollama show <tag> --modelfile` names the template (`RENDERER`). The
thinking variant ignores `think: false`, so with the 2048-token cap it may run
out of tokens while thinking; that is part of what the reference shows.

`gemma4:12b` (timed out on 8 of 13 receipts in the first benchmark) and
`qwen2.5vl:7b` (invented tips and subtotals) are no longer in the recommended
run. Either can still be passed with `--model`. The service's default model is
`qwen3-vl:30b-a3b-instruct`: it is the only candidate with no confident-wrong
totals in both benchmarks, and the fastest.

It runs the same pipeline over a folder of labelled receipts and writes
`bench-results.md` (aggregate numbers only) and
`bench-results.details.local.md` (per-receipt verdicts, which name files; keep
it local). Both are gitignored. It takes the same `--timeout-seconds` and
`--max-tokens` flags as the service. Each model's section records its run
settings (thinking requested off and whether the model supports it, max
tokens, timeout, prompt version). When the bench finishes with a model it
unloads it (`keep_alive: 0`), so models don't share memory and one model's
timings don't overlap the next. Label each receipt with
`<name>.expected.json` next to it:

```json
{
  "vendor": ["Lantern & Loaf Bakery", "Lantern & Loaf"],
  "date": "2026-09-12",
  "total": "54.50",
  "currency": "USD",
  "reference": "LL-4410",
  "subtotal": "50.00",
  "tax": "3.00",
  "fees": "1.50",
  "discounts": null,
  "tip": null,
  "category": "northstar-category-2",
  "clear": true,
  "kind": "photo"
}
```

(A made-up receipt: $50.00 subtotal, $3.00 tax and a $1.50 card surcharge.)

How to label:

- **A missing key isn't scored; `null` means the receipt doesn't show that
  field.** A field labelled `null` that the model reports as found counts as
  confident-wrong. Leave a key out only when you can't tell what's right.
- **Amounts** are text as printed (`"3.00"`, `"$3.00"` and `"1,042.50"` all
  work); never JSON numbers.
- **`subtotal`:** only a line labelled Subtotal or an equivalent ("Sub
  Total", "Merchandise"). No such line means `null`, even if an item price or
  the total would be the same amount.
- **`tax`:** the sum of every printed tax line (`TAX 0.00` and `TAX 0.45`
  give `"0.45"`). A printed `0.00` tax line gives `"0.00"`; no tax line gives
  `null`.
- **`fees`:** the sum of surcharge, convenience, service, service-charge and
  delivery-fee lines added after the subtotal. A fee listed among the items
  (already in the subtotal) isn't counted. None gives `null`.
- **`discounts`:** the sum of coupon, savings, reward and markdown lines
  printed as deductions, as a **positive** amount (lines `1.50-` and `0.50-`
  give `"2.00"`). A "Total saved" or "Your savings" summary isn't counted. None
  gives `null`.
- **`tip`:** only a tip the customer actually added. A suggested-tip table is
  `null`.
- **`reference`:** a number labelled as a receipt, transaction, invoice,
  order, check or bill number, never a card, account, approval or phone
  number, terminal ID, trace number, card-slip reference ID or survey code. When the
  receipt prints more than one such number, give a list of all of them; any
  one is accepted, like `vendor`'s spellings (`"reference": ["R-1001", "77"]`).
  None gives `null`.
- **`vendor`** may be a list of accepted spellings.

`clear: false` leaves a receipt out of "Total, clear receipts". `kind` is a
free tag counted in the report (for example `photo`, `screenshot`, `pdf` for a
PDF with a text layer, `scanned pdf` for one without). An optional
`categories.json` at the corpus root lists `{ "id", "name" }` categories to
send. `docs/samples/` is a small labelled corpus of fictional receipts. Real
receipts never go in the repository.

Tax, fees and discounts are scored by the sum of their lines. The local
per-receipt file ends with a short **Causes** section per model (for example
"total unclear: the parts read don't add up to it: 2", "tip:
confident-wrong: 1", "receipt not read: timed out: 1"). It names categories
and fields only, never values, and it never appears in `bench-results.md`.
