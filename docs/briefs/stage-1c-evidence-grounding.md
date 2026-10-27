# Brief: Stage 1c, ground every value in its evidence

**For:** Claude Code
**Repo:** `spe1020/zap-books`
**Start from:** `business-finance-mvp` (PR #5 merged). Pull first.
**Branch name:** `fix/extraction-v4`
**Save this brief as:** `docs/briefs/stage-1c-evidence-grounding.md` (first commit, Prettier-formatted)

**Privacy rule for this PR:** no real receipt details anywhere in the repo, including tests, docs, commit messages and the PR description. Every example below and every test uses invented values. Before pushing, search all diffs and messages for real vendor names and amounts from the benchmark corpus.

## Why

The second benchmark showed the best model (`qwen3-vl:30b-a3b-instruct`) gave no confident-wrong totals, but it still gave confident-wrong dates, fees, discounts, subtotals and references. A smaller model gave one confident-wrong total by reading a large "savings this year" banner as the total. Analysis found one common thread: in almost every error, the value either isn't supported by its own evidence, or the evidence lacks the label that would identify the field.

## The principle

**A field is `found` only if Rust can confirm the value from its evidence.** The model copies a printed line, label included, and Rust checks that the value is really there and that the label fits the field. When the check fails, the field becomes `unclear` with a plain-language warning. This extends the existing "no evidence, no found" rule.

The prompt version becomes `receipt-v4`. The prompt tells the model that **every evidence string must include the line's printed label**, not just the number (for example `TOTAL $54.50`, not `$54.50`).

## Commit 1: shared evidence helpers

In `crates/extractor`, add tested helpers used by every rule below:

- `amount_in_evidence(value, evidence)`: true if the evidence contains the amount, ignoring currency symbols, thousands separators, spaces, and a trailing or leading minus or parentheses.
- `has_label(evidence, words)`: case-insensitive whole-word match against a list.
- `numbers_in_evidence(evidence)`: digit runs, ignoring spaces and punctuation.

**General rule:** every amount field (total, subtotal, each tax, fee, discount line, tip) must pass `amount_in_evidence`. Otherwise it's `unclear`, with the warning "The value doesn't match the text it was read from."

## Commit 2: dates

- Parse the date evidence in Rust. Accept numeric forms (`M/D/YY`, `M/D/YYYY`, `D/M/YY`, `D/M/YYYY`, `YYYY-MM-DD`, with `/`, `-` or `.`) and English month names (`Oct 25, 2026`, `25 October 2026`, `October 25 2026`).
- **Two-digit years become 20YY.**
- **Day/month order:** if both readings of a numeric date are valid and differ (such as `03/04/26`), use month-first when the receipt's currency was read as USD, and day-first for EUR or GBP. If the currency isn't found, the date is `unclear`.
- **The date is `found` only if the evidence yields exactly one valid date under these rules, and it equals the model's value.** If the evidence has no year, no parseable date, or a date different from the model's value, it becomes `unclear`: "Check the date; it couldn't be confirmed from the receipt."
- Keep the existing future-date rule.

Tests: two-digit years; ambiguous day/month under each currency; evidence with a time but no year; evidence containing an unrelated number that could pass for a year; month names; evidence and value disagreeing.

## Commit 3: totals

- The total's evidence must include a label from: `total`, `amount`, `paid`, `payment`, `charged`, `balance`, `due`, `grand`. Otherwise `unclear`: "The total was read without its label. Check the amount."
- If the evidence contains any of `saving`, `savings`, `saved`, `points`, `reward`, `rewards`, `ytd`, `year`, the total is `unclear`. This is a savings or loyalty figure, not a payment.
- **Negative totals:** if the amount is negative **and** the evidence contains `payment`, `paid` or `charged`, **and** does not contain `refund`, `credit`, `return` or `reversal`, use the absolute value as the total. Every other negative total stays `unclear`, so a refund can never become a positive expense.
- The prompt adds: "The total is the amount actually paid. Never a savings, points, rewards or year-to-date figure."

## Commit 4: subtotal, tax, fees, discounts, tip

- **Subtotal:** evidence must contain `sub` (as in Subtotal, Sub Total) or `merchandise`. Otherwise `absent`. The prompt adds: "A line whose label contains Total is never a subtotal."
- **Fees:**
  - **Percentage check.** If a fee line's evidence prints a percentage and a subtotal was found, the fee must equal subtotal × percentage, give or take one cent. Otherwise mark both fees and tax `unclear`, with the warning "The fee and tax may be swapped. Check both." Apply this to fees only, never to tax, because tax often applies to only some items.
  - **Fee already in the subtotal.** If the total check fails with fees included but passes without them, set fees to `absent` and record in the extraction that they were treated as part of the subtotal. The total stays `found`.
- **Discounts:**
  - Each line's evidence must show a deduction: a minus sign, parentheses, or one of `disc`, `discount`, `coupon`, `saving`, `savings`, `off`, `reward`, `markdown`, `promo`. Otherwise the whole list is `unclear`.
  - Summary lines don't count: a line whose evidence contains `total`, `summary`, `you saved` or `ytd` makes the list `unclear`, because summaries double-count the individual deductions.
  - The prompt adds: "A regular, original, clearance or sale price is not a deduction. Never use a savings summary."
- **Tip:** the prompt replaces any wording that implies tips are rare with: "A Tip or Gratuity line with an amount, printed between the subtotal and the total, is a tip, even when a table of suggested tips is also printed. A suggested-tip table alone is not a tip."

## Commit 5: reference and vendor

- **Reference,** prompt: change "prefer" to "only". Allowed: a number labelled as a receipt, transaction, invoice, order or check number. Name the exclusions: terminal ID, trace number, payment or card-slip reference ID, survey code, card, account and phone numbers, and product barcodes.
- **Reference,** Rust: the value must appear in its evidence (ignoring spaces and punctuation), and the evidence must contain one of `receipt`, `rcpt`, `transaction`, `trans`, `tran`, `invoice`, `inv`, `order`, `check`, `chk`. Otherwise `unclear`.
- **Vendor,** prompt: "The business name, usually in the logo or the largest text at the top. Never a tagline, slogan or list of products or services."

## Commit 6: default model

- Change the extractor's default model to `qwen3-vl:30b-a3b-instruct`. It's the only candidate with zero confident-wrong totals in both benchmarks, and the fastest.
- In `docs/receipt-extraction.md`, note it uses about 22 GB of memory, and give `qwen3-vl:8b-instruct` as the option for smaller machines, with the warning that it produced a confident-wrong total in testing.

## Commit 7: bench support for a holdout set

These rules were designed by looking at one corpus, so they need testing on receipts they haven't seen.

- The bench accepts `--corpus` more than once and reports each corpus separately, then combined.
- `docs/receipt-extraction.md` explains the holdout idea in two sentences: keep a second folder of receipts that are never used to design rules, and judge changes by it.

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

## Out of scope

- Layout problems where amounts print in a column offset from their labels. The total check already catches the resulting errors.
- Any change to ledger rules, Nostr, or the accounting service.

## Stop and ask if

- a new dependency is needed (date parsing must be hand-written; no `chrono`);
- any rule would turn a currently correct total into a confident-wrong one on the committed samples;
- a ledger fixture changes.

## PR description must include

1. Results of every check.
2. A table of the new rules, each with the invented example its test uses.
3. Before/after on the committed fictional samples with the default model.
4. Confirmation of the privacy search before pushing.
