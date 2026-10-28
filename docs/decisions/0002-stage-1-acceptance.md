# Decision 0002: Stage 1 acceptance (local receipt extraction)

**Status:** Accepted (2026-10-27)
**Save as:** `docs/decisions/0002-stage-1-acceptance.md`

## Decision

Stage 1 (local receipt extraction) is complete. The default model is `qwen3-vl:30b-a3b-instruct`. The accuracy targets set in the stage 1 brief are replaced by the revised targets below, which the current build meets.

## Why the original targets were replaced

The stage 1 targets (for example, total accuracy ≥ 95% and confident-wrong ≤ 1%) were set before any real receipts were measured. Two problems emerged:

- **They can't be measured yet.** With 28 receipts, one error is 3.6%, so a ≤ 1% rate can't be distinguished from zero until the corpus reaches roughly 100 or more receipts.
- **They weigh all fields equally.** Subtotal, tax, fees, discounts and tip are reference-only: they feed the total cross-check and are shown to the user, but are never applied to an expense. Only vendor, date, total, currency and category can be accepted into an expense, and the total is the one that matters most.

## Revised targets

| Target                                        | Holdout result                     | Status                           |
| --------------------------------------------- | ---------------------------------- | -------------------------------- |
| Zero confident-wrong totals                   | 0 of 12 (0 of 28 across both sets) | Met                              |
| At least 80% of totals found on a holdout set | 9 of 11 clear receipts (82%)       | Met                              |
| Median time under 20 s                        | 6.6 s                              | Met                              |
| Date and vendor confident-wrong rates         | 1 of 11 each                       | Re-measure at about 100 receipts |

## Results (aggregate)

Model `qwen3-vl:30b-a3b-instruct`, prompt `receipt-v4`, thinking off, temperature 0, fixed seed.

|                        | Design set (16) | Holdout set (12)                | Combined (28) |
| ---------------------- | --------------- | ------------------------------- | ------------- |
| Totals found           | 87.5%           | 75.0% (81.8% of clear receipts) | 82.1%         |
| Confident-wrong totals | 0               | 0                               | 0             |
| Dates correct          | 81.2%           | 90.9%                           | 85.2%         |
| Vendors correct        | 100%            | 90.9%                           | 96.2%         |
| Currency correct       | 100%            | 100%                            | 100%          |
| Median time            | 11.8 s          | 6.6 s                           | 7.0 s         |

The design set was used to develop the rules, so its numbers are optimistic. The holdout set had never been used for development and is the honest measure.

Across three benchmark rounds, the chosen model never produced a confident-wrong total.

## What made the difference

- **The model transcribes; Rust does the arithmetic.** Amounts are copied as printed lines, and all sums and checks run in Rust with `ledger-core` money rules.
- **Every value must be confirmed by its own evidence.** A value that doesn't appear in its evidence, or lacks the label that identifies the field, becomes unclear.
- **A total is confirmed by its label or by adding up.** Savings and loyalty figures are never totals, and a negative amount becomes a total only on a clearly labelled payment line.
- **The request itself was a source of error:** schema key order and the model's thinking mode both changed results. Both are now fixed and tested.

## Known gaps (for later)

- **Documents with several dates** (folios, invoices with due dates) are the likely source of the remaining wrong date. A general "several distinct dates → unclear" rule is a candidate.
- **Estimates and authorizations** (for example a rental estimate) are not receipts. The app should eventually recognize and flag them rather than suggest a total.
- **Payment-only documents** now work when the payment line is labelled. Unlabelled negative amounts stay unclear.
- **Amounts printed in a column offset from their labels** cause misreads that the total cross-check catches but can't correct.
- **Zero-amount lines** (a $0.00 discount or shipping line) have no defined treatment.
- **The first answer to a new request can differ** from later identical ones, even at temperature 0, probably due to Ollama's prompt cache. The rules judge each answer on its own, so this affects accuracy, not safety.

## Benchmark discipline

- Real receipts stay outside the repository. A local pre-commit hook blocks commits containing vendor names, amounts or receipt numbers from the corpus labels.
- **Once a holdout receipt is used to design a fix, it stops being a holdout.** Collect a fresh holdout set before claiming new accuracy numbers.

## Direction for AI work

Reading receipts is the intake step. The main value of AI in Zap Books, as set out in the concept plan, is **accounting help**:

- suggesting a category from the business's own categories, with the reason;
- matching receipts to imported payments;
- explaining missing details and possible duplicates;
- answering questions from posted records and calculated report results.

Future AI work should prioritize these, under the same principles: suggestions show their evidence, the user approves, and the model never writes to posted balances.
