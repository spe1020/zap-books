# Architecture and backend handoff

## Responsibilities

| Layer           | Location                                        | Responsibility                                                                        |
| --------------- | ----------------------------------------------- | ------------------------------------------------------------------------------------- |
| UI              | `src/App.tsx`, `src/components/`                | Workspace shell, review, import preview, settings, reports, accessible dialogs        |
| Domain          | `src/domain/types.ts`, `ledger.ts`, `import.ts` | Business-scoped types, validation, journal commands, report queries, CSV identity     |
| Demo fixtures   | `src/domain/seed.ts`                            | Two independent fictional businesses and generated receipt evidence                   |
| Storage         | `src/adapters/storage.ts`, `attachments.ts`     | IndexedDB transactions, revision checks, scoped receipts, supported upload validation |
| AI              | `src/adapters/ai.ts`                            | `AIAdapter` deterministic proposals and supported questions                           |
| Receipt reading | `src/adapters/extractor.ts`, `crates/extractor` | `ExtractionAdapter` client; local Rust service calling a local Ollama vision model    |
| Event transport | `src/adapters/transport.ts`                     | `EventTransport` mock with event-ID deduplication and business-scoped synchronization |
| Protocol (WASM) | `crates/protocol-wasm`                          | `crates/protocol` for the browser: chain checks, acceptances, membership, deliveries  |

React owns presentation and in-progress forms. Ledger commands return a new business snapshot; they do not mutate their input. The application awaits an IndexedDB commit before displaying success or advancing posted reports. Receipt Blobs and their record references are persisted together. On load, `src/adapters/workspace-check.ts` checks every stored record with the Rust loader's rules (types, enums, amounts, business scope, balanced journals); a record that fails is set aside in the workspace's `quarantine` list and reported, and the rest loads. The page carries a Content-Security-Policy (`src/csp.ts`): scripts and styles from the app only, requests to the app and the extractor only, receipts from `blob:` URLs. A stored SVG is only ever shown as an image, never opened as a page. The single workspace snapshot makes this small prototype's atomic writes straightforward; large workspaces will need indexed tables, pagination, migrations, and storage-quota strategy.

Screens change the books only through a `LedgerClient` (`src/adapters/ledger-client.ts`): one method per ledger protocol command (`saveDraft`, `approve`, `correct`, `commitImport`, `updateSettings`, `saveRule`, `changeMembership`, `veto`), and `run` for an action made of several, such as saving a draft and approving it. `LocalLedgerClient` (`src/adapters/local-ledger.ts`) is the local demo: it applies an action's commands with the domain functions and makes them one revision-checked IndexedDB commit. `RemoteLedgerClient` (`src/remote/client.ts`) is a business on the ledger service: it runs the same domain functions first, for their messages, then makes each command a proposal, built and checked by `crates/protocol-wasm`, encrypted to the service and signed by the member's NIP-07 extension, and queued in IndexedDB. The queue sends proposals in order, one at a time; a refusal stops it there, with Discard and Redo. The books shown change only through sync (`src/remote/sync.ts`): every acceptance is chain-checked, opened with its epoch key, followed through the membership rules and applied, all in WebAssembly, and the verified head and books are stored only when everything fetched verifies. A stored head the service no longer extends stops sync for that business. Receipt files stay in the browser that attached them; proposals carry their id and SHA-256.

`BusinessRepository` defines scoped reads and revision-checked commits. `ReceiptStorage` defines scoped document listing and reads. `LedgerCommands` and `LedgerQueries` expose posting, correction, and totals. These are explicit boundaries for replacing browser-only adapters with services. Browser state and the local demo actor cannot establish identity or authority.

## Financial model

Every independently stored business-owned record has a stable ID and a business ID: profile, account, category, vendor, document, imported payment, expense draft, journal entry, approval, rule, audit event, and application event. Vendor names on expenses preserve the reviewed display text; vendor records are maintained when drafts are saved.

Receipt evidence, payment evidence, and the expense are separate records. A payment's draft ID and the expense's payment ID connect them. Adding a receipt to that draft does not create another expense. Payment evidence must agree with its expense before approval.

Amounts are safe integer minor units with an explicit currency. Decimal input is parsed as decimal digits, not multiplied floating-point input. Integer addition guards overflow. Every journal is balanced within a single currency; the selected payment account must match it. Category expense accounts may receive several currencies, but journal lines are only aggregated within the requested currency. The UI never computes an exchange rate.

The permitted expense progression is `draft → posted → corrected`. Reapproval is rejected. Approval emits one balanced expense journal, with a debit to the expense category and a credit to the bank/card account. A correction validates an entire replacement, reverses the original journal, marks the original expense corrected, and posts the linked replacement atomically. Failure leaves the original untouched. Original and reversal entries remain in the journal. A replacement may itself be corrected later. Closed periods, reconciliation, opening balances, asset denomination policies, and jurisdiction-specific accounting are future work.

Transfers have their own journal type and no expense category lines. Spending queries derive totals from category-bearing journal lines, so transfers are excluded and reversals naturally offset original postings. Dashboard figures are never seeded aggregates.

## Application events

`FinancialEvent` in `src/domain/types.ts` is a **versioned application schema, not an established Nostr accounting standard**. Version 1 contains:

- `id`: stable event UUID for transport deduplication.
- `businessId`: application business scope.
- `schemaVersion: 1` and a typed `type` such as `expense.created`, `expense.matched`, `expense.approved`, `expense.posted`, or `expense.reversed`.
- `entityId`, `sourceIds`: links to expenses, receipts, payments, journals, and related corrections.
- `actor`, `occurredAt`: local demo actor and timestamp; neither is trusted production identity or ordering.
- `payload`: application metadata describing the accepted action. Full journals remain authoritative in local storage; these metadata events alone do not rebuild a complete ledger.

Events and audit records are persisted with the business. The mock publisher stores copies in memory, makes no network calls, and can synchronize by business ID. It is an adapter exercise, not a durable outbox, conflict resolver, cryptographic implementation, or synchronization protocol. A future backend must specify signed/encrypted wire envelopes, schema migration, acknowledgements, cursor semantics, rejection handling, and a transactional outbox.

## AI and document boundaries

The demo adapter only consults explicit vendor rules and stored fields. It reports the rule and linked-payment evidence, marks every proposal simulated, and states uncertainty. Accepted proposals only update the editable draft. Rules are explicitly saved and visible; there is no implicit training. Posting always requires the same deterministic ledger validation as manual entry.

Documents are evidence, never instructions. Receipt reading (stage 1) runs entirely on the user's computer: the browser sends an attached receipt to the extractor service (`crates/extractor`, bound to `127.0.0.1`), which renders PDF pages with Poppler and asks a local Ollama vision model for a fixed JSON schema. The model has no tools, sees only the receipt, the business's category ids and names, and a fixed prompt that treats text addressed to an AI as receipt content. The service re-validates every value with `ledger-core`'s exact money and date rules, downgrades anything doubtful to "unclear", and only accepts category ids it was given. It rejects foreign `Host` headers and origins, accepts only the attachment rules' file types and size, and logs timings and field statuses, never contents. See [Receipt extraction](receipt-extraction.md).

The Review screen shows each suggestion with its evidence; nothing is applied without a click, a suggestion never overwrites a field edited since reading, and fields locked to an imported payment are compared rather than applied. Every reading is stored as a business-scoped `ExtractionRecord` (model, prompt version, proposed fields, and per-field outcome finalized at approval) for provenance. Posting still requires the same deterministic ledger validation as manual entry. A model must never bypass the authoritative ledger service or directly alter posted balances. Sending a document to any model off the computer requires an explicit user choice that does not exist yet.

## Recommended next backend milestone

1. Establish authenticated users, explicit business memberships, and server-enforced roles. Browser workspace IDs are not authorization.
2. Build **one authoritative accounting service**. It validates, orders, and accepts postings using database transactions and business-scoped idempotency keys. Persist immutable journals, approvals, corrections, and a durable outbox together. Revalidate currency, account, period, duplicate, and balance invariants server-side. Clients submit proposals and reconcile against acknowledged server versions.
3. Introduce encrypted object storage for receipt documents. Bind object references and keys to authorized business members, verify integrity, and design key rotation, retention, recovery, and access auditing. Do not include plaintext document contents in relay events.
4. Use an **existing private Nostr relay implementation**, configured with enforced authentication and business authorization. Select supported authentication and audited encryption/signing libraries; design key custody before handling real financial information. No custom relay is needed. A relay transports encrypted application events; it does not decide accounting truth or ordering.
5. Replace the demo assistant with a scoped service that returns typed proposals and evidence, building on the local extractor. Treat extracted receipt content as untrusted data and keep approvals explicit.
6. Validate concurrent submissions, offline retries, replay protection, member revocation, recovery, schema upgrades, and encrypted document access before any real-business pilot.

External deployment, payments, live banking, payroll, taxes, and broad QuickBooks parity are outside this frontend milestone.
