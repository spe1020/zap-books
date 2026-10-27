# Brief: Stage 1b, extraction fixes from the first real benchmark

**For:** Claude Code
**Repo:** `spe1020/zap-books`
**Start from:** `business-finance-mvp` (PR #4 merged). Pull first.
**Branch name:** `fix/extraction-v3`
**Save this brief as:** `docs/briefs/stage-1b-extraction-fixes.md` (first commit, Prettier-formatted)

## Why

The first benchmark on 13 real receipts gave **zero confident-wrong totals across all four models**, so the safety design works. But no model met the accuracy targets. Analysis showed most failures come from our own request and rules, not from model ability:

- The JSON schema reaches Ollama with its keys sorted alphabetically, so models fill in `tip` before `total`. Sending fields in the declared order removed all of qwen2.5vl's invented tips.
- The total cross-check knows only subtotal, tax and tip. A surcharge makes a correctly read total "unclear".
- The prompt defines subtotal as "the amount before tax, as printed", so models report a pre-tax amount on receipts with no Subtotal line, while the labels say null.
- Several tax lines are handled inconsistently.
- The installed `qwen3-vl:8b` is a thinking variant that can't be switched off, and is slow.
- A timed-out request keeps running in Ollama and holds the GPU, distorting later timings.

## Guiding principle for this PR

**The model transcribes; Rust does the arithmetic.** The model copies printed amounts with their evidence. Every sum, comparison and decision about the total happens in Rust with `ledger-core` money rules.

## Commit 1: send the schema in declared order

- Enable `serde_json`'s `preserve_order` feature in `crates/extractor` (a feature of an allowed dependency; no new crate).
- The schema sent to Ollama and the prompt list fields in this order: `vendor`, `date`, `currency`, `total`, `subtotal`, `tax`, `fees`, `discounts`, `tip`, `reference`, `category`. Within each field, keep the current declared key order.
- Add a test asserting the exact key order of the schema as serialized, so a future dependency change can't silently re-sort it.

## Commit 2: amounts become lists of printed lines

Replace the single `subtotal`, `tax` and `tip` amount fields with this shape, and add `fees` and `discounts`:

```text
subtotal:  Field<amount>              // only a line labelled Subtotal (or an equivalent label)
tax:       LineList                   // every printed tax line
fees:      LineList                   // surcharges, convenience or service fees, delivery fees
discounts: LineList                   // coupons, savings, rewards, markdowns shown as deductions
tip:       Field<amount>              // only a tip actually added, never a suggested-tip table

LineList {
  status: "found" | "unclear" | "absent"
  lines:  [{ amount: "text as printed", evidence: "max 120 chars" }]
}
```

Rules, all in Rust and all tested:

- Each line amount is parsed with `ledger-core`. One unparseable line makes the whole list `unclear`.
- Discounts are stored as positive amounts, whatever sign the receipt prints.
- `tax` total = sum of its lines. A receipt with `TAX 0.00` and `TAX 0.45` gives tax 0.45.
- **The total is consistent** when it equals either
  `subtotal + tax + fees + tip`, or
  `subtotal − discounts + tax + fees + tip`,
  with any absent list or field counted as 0. Accept both because receipts differ on whether the printed subtotal is before or after savings.
- If subtotal is absent, no cross-check runs, as today.
- When the check fails, the total stays `unclear`. The warning names what was read, for example: "Subtotal $50.00, tax $3.00 and fees $1.50 add up to $54.50, but the total reads $55.50. Check the amount."
- **Negative totals stay unclear.** A negative amount may be a refund, and a refund must never become a positive expense. Improve the warning: "The only amount is a payment or refund line (−$24.00). Enter the total yourself."

Show subtotal, tax, fees, discounts and tip as reference-only in the suggestions panel, with their evidence. Store them in the `ExtractionRecord` the same way.

**Stop and ask** if the Rust `ExtractionRecord` shape change alters any ledger fixture other than state fixtures holding extraction records.

## Commit 3: tighten the prompt to match the labels

Prompt version becomes `receipt-v3`. Make the field definitions literal:

- **subtotal:** only if a line is labelled Subtotal or an equivalent ("Sub Total", "Merchandise"). Otherwise `absent`. Never compute or infer one.
- **tax:** only printed tax lines. If none are printed, `absent`, not `0.00`.
- **tip:** only a tip the customer actually added (a filled-in tip line). A table of suggested tips is not a tip.
- **reference:** prefer a number labelled as a receipt, transaction, invoice or order number. Never a card number, account number or phone number.
- **total:** the amount actually paid, including tax, fees and tip.

Update `docs/receipt-extraction.md` and the sample labels to match.

## Commit 4: thinking control, output cap, and real cancellation

- Send `think: false` on every request. Record in each result whether the model reports thinking support, from `/api/show`.
- Set `num_predict` (default 2048, flag `--max-tokens`) so a runaway generation stops.
- **On timeout, the request is actually cancelled** (drop the HTTP connection so Ollama stops generating), and the service confirms the model is idle before the next request. Add a test with the fake model server that the connection is closed on timeout.
- The bench unloads each model when it finishes with it (`keep_alive: 0`), so models don't share memory and timings don't overlap.
- The bench records, per model: thinking on/off, max tokens, timeout, and prompt version.

## Commit 5: bench scoring

- `reference` in `expected.json` may be a list of accepted values, like `vendor`.
- Score `fees` and `discounts` as the sum of their lines. Score `tax` as the sum of its lines.
- A field labelled `null` (not on the receipt) and found by the model counts as confident-wrong, as today. A key left out of the label is still not scored.
- Report a short "causes" section in the per-receipt local file only, never in `bench-results.md`.

## Commit 6: candidate models

- Replace the model list in `docs/receipt-extraction.md` with **non-thinking (instruct) variants**: `qwen3-vl:8b-instruct` and the instruct variant of the 30B model. Find the exact current Ollama tags and confirm each is a non-thinking template with `ollama show`. Keep `qwen3-vl:8b` (thinking) in the list as a reference point.
- Drop `gemma4:12b` (timed out on 8 of 13) and `qwen2.5vl:7b` (invented tips and subtotals) from the recommended run. They can still be passed with `--model`.

## Checks (all must pass, locally and in CI)

```
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
npm test
npm run build
npm run format:check
npm run test:e2e
```

Plus the fixture-drift check. CI runs no real model.

## After the PR is open: benchmark run (Seth's machine)

1. Seth fixes any labels missing printed tax, fees or references, and turns a `reference` into a list where a receipt prints more than one candidate number.
2. Labels gain `fees` and `discounts` where printed, or `null` where not. Claude Code may draft these for Seth to check, reading the photos itself, without running any model.
3. Seth adds PDF e-receipts to the corpus if he has them.
4. Run the bench with the new model list and paste `bench-results.md` into the PR.

## Out of scope

- Turning negative payment lines into totals.
- Layout-specific fixes (for example, receipts that print amounts one line above their labels).
- Any change to ledger rules.
- Nostr, the accounting service, or anything from `docs/decisions/0001-nostr-security-model.md`.

## Stop and ask if

- a dependency or crate feature beyond `serde_json/preserve_order` is needed;
- a ledger fixture changes beyond extraction-record state;
- Ollama can't cancel a request when the connection drops;
- no non-thinking variant of either qwen3-vl model exists in Ollama.

## PR description must include

1. Results of every check.
2. A test showing the schema key order as sent.
3. Before/after on the four committed samples with the model available locally.
4. The exact model tags chosen and the `ollama show` evidence that they're non-thinking.
5. The updated labeling instructions for `fees`, `discounts` and list references.
