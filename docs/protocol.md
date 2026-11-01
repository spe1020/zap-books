# Zap Books ledger protocol

**Version:** 1 (the `v` field of every encrypted payload)
**Implementation:** `crates/protocol`
**Background:** [Decision 0001](decisions/0001-nostr-security-model.md), [stage 2 brief](briefs/stage-2-accounting-service.md)

This is the wire format between members and the ledger service. A member
signs a **proposal**. The service checks it with `ledger-core` and answers
with a signed **acceptance**, which is one link of the books' hash chain, or
with a signed **rejection**. The service also sends **epoch key deliveries**:
the keys members use to read acceptances.

All four are ordinary Nostr events (NIP-01), so stage 4 can publish them to
relays without changing the format. In stage 2 they travel over HTTPS only:

- `POST /v1/proposals`, with a proposal event as the body;
- reads authenticated with NIP-98 (a kind-27235 event in the
  `Authorization` header), or with a session token obtained that way
  (`docs/service.md`, Sessions).

The HTTP API, NIP-98 checks and storage belong to `crates/ledger-service`
(PR 2). This document covers the events and the rules for them.

## Event kinds

| Kind   | Name               | Signed by | Encrypted to          | In the chain |
| ------ | ------------------ | --------- | --------------------- | ------------ |
| `4130` | Proposal           | A member  | The service           | No           |
| `4131` | Acceptance         | Service   | The current epoch key | Yes          |
| `4132` | Rejection          | Service   | The proposer          | No           |
| `4133` | Epoch key delivery | Service   | One member, one epoch | No           |

They are defined once, in `crates/protocol/src/kinds.rs`. All four are in
the regular range (1000–9999), so a relay stores every one of them; none is
replaceable or ephemeral.

**Collision check (2026-10-27).** None of 4130–4133 appears in:

- the Event Kinds table of `nostr-protocol/nips` `README.md`, at commit
  `400d157c53264b8e58c7b3cdbd21ef546fb7a157`, nor anywhere else in that
  repository's NIP texts;
- `nostr-protocol/registry-of-kinds` `schema.yaml`, at commit
  `836a60984e44a2269ec2636f2a32b2d04a14da69`, which the NIPs README now names
  as the fuller registry;
- the `Kind` enum of `nostr` (rust-nostr) 0.45.5.

The nearest registered kinds are 4454–4455 and 4550, and NIP-90 reserves
5000–7000 for data vending machines. Repeat this check before stage 4
publishes to public relays.

## Common rules

- **Tags.** `e` and `p` keep their NIP-01 meanings. `b` carries the business
  id (see below). The others are ours: `cmd`, `base_seq`, `service`,
  `payload`, `seq`, `prev`, `epoch`. Each tag this document lists appears at most once; a
  repeated tag is malformed, so two readers can never disagree about which
  value counts. Other tags are ignored.
- **Numbers** in tags are decimal with no sign and no leading zeros (`0`,
  `17`), so each has one spelling.
- **Business id:** 32 random bytes chosen by the founding owner, written as
  64 lowercase hex characters. No other spelling parses.
- **Keys and event ids** are 64-character lowercase hex, as in NIP-01. In
  the `service`, `prev`, `e` and `p` tags no other spelling parses, so a
  relay filter on the raw tag and a reader comparing parsed values always
  agree.
- **Record ids a proposer chooses** (a draft's id, a correction's
  replacement, a document reference, a rule given an id, and genesis's
  accounts and categories) are 1 to 128 ASCII letters, digits, hyphens or
  underscores. UUIDs and the ids the app derives from a business id fit;
  anything else, the empty id included, is refused (`BadId`, or
  `BadDocument` for a document). Ids the service generates are its own.
- **Times.** `created_at` is Unix seconds. Inside `ledger-core` records,
  times stay ISO-8601 strings, as today.

### The business tag

Every event carries `["b", <business id>]`. NIP-01 relays index
single-letter tags, so a client or the service's outbox can ask a relay for
one business's events with a `#b` filter, and a relay's policy plugin can
gate reads by business without decrypting anything. It is defined once, as
`kinds::tag::BUSINESS`.

**Collision check (2026-10-27).** No NIP assigns `b`. It appears as a tag
name in:

- no NIP text in `nostr-protocol/nips`, at commit
  `400d157c53264b8e58c7b3cdbd21ef546fb7a157`. The single-letter tags in use
  there are `a`, `c` to `i`, `k` to `n`, `p` to `u`, `x` to `z`, and the
  upper-case `A`, `D`, `E`, `F`, `I`, `K`, `L`, `N`, `P`, `R`, `S`, `T`;
- no tag definition in `nostr-protocol/registry-of-kinds` `schema.yaml`, at
  commit `836a60984e44a2269ec2636f2a32b2d04a14da69`;
- no standardized tag in `nostr` (rust-nostr) 0.45.5.

The other unassigned lower-case letters were `j`, `o`, `v` and `w`. Repeat
this check with the kinds check before stage 4.

### Encryption

All content is a single NIP-44 version 2 payload, through the `nostr` crate
(no hand-written cryptography). NIP-44 v2 encrypts 1 to 65,535 bytes of
plaintext. The `nostr` crate also accepts an extended length that other
implementations can't read, so longer plaintext is refused ("The content is
… bytes, over the 65535-byte limit NIP-44 encrypts."). It could not fit in
an event anyway (see Size limits).

NIP-44's conversation key is symmetric: content from key A to key B is read
with B's secret key and A's public key, or with A's secret key and B's
public key.

### Size limits

Every event a builder produces, and every event a parser accepts, is at
most **65,536 bytes** as compact NIP-01 JSON (`MAX_EVENT_BYTES`). A builder
asked for a larger event fails with:

> The event is 70210 bytes, over the 65536-byte limit relays accept. Send it
> as smaller proposals.

**Why 65,536.** It is the lowest default among the common relay
implementations (checked 2026-10-27):

| Relay          | Setting                                      | Default | Commit    |
| -------------- | -------------------------------------------- | ------- | --------- |
| strfry         | `events.maxEventSize`, normalised event JSON | 65,536  | `4cd3cf6` |
| nostr-rs-relay | `max_event_bytes`                            | 131,072 | `b5c1f64` |
| khatru         | `MaxMessageSize`, a whole websocket message  | 512,000 | `9f99b98` |

strfry runs on many public relays, so an event under its default is accepted
almost everywhere. Stage 4 publishes these same events to relays, so stage 2
enforces the limit from the start.

**The service checks the acceptance's size before storing anything.** It
builds and signs the acceptance inside the proposal's transaction, before
any write. If that acceptance would be over the limit, the service rejects
the proposal with the code `tooLarge` (see Rejection), and nothing is
stored: no acceptance, no seq, no record changes. A client whose own builder
refuses an oversized proposal treats that the same way.

## Proposal (kind 4130)

Signed by a member (or the recovery key). The content is NIP-44 encrypted
**to the service's public key**, so every role can propose without holding
the business key. That is what makes the write-only submitter role possible.

```json
{
  "kind": 4130,
  "pubkey": "<member>",
  "tags": [
    ["b", "abab…ab"],
    ["cmd", "Approve"],
    ["base_seq", "41"],
    ["payload", "<SHA-256 of the plaintext payload>"]
  ],
  "content": "<NIP-44 to the service>"
}
```

| Tag        | Value                                                        | Required                     |
| ---------- | ------------------------------------------------------------ | ---------------------------- |
| `b`        | The business id                                              | Always                       |
| `cmd`      | The command type, so the service can route before decrypting | Always                       |
| `base_seq` | The last acceptance seq the proposer had seen                | Every command except genesis |
| `service`  | The service's public key                                     | Genesis only, and only there |
| `payload`  | SHA-256 of the exact plaintext payload, lowercase hex        | Always                       |

The decrypted content is the **payload**:

```json
{
  "v": 1,
  "business": "abab…ab",
  "baseSeq": 41,
  "salt": "<32 random bytes, hex>",
  "command": { "type": "Approve", "expenseId": "draft-7" }
}
```

The service refuses a proposal unless its signature is valid, its tags are
well formed, its plaintext hashes to its `payload` tag, and the payload
agrees with the tags: same business, same command type, same `baseSeq`, and
for genesis, the same service key, which must be this service's key.

### Payload commitment

The `payload` tag is the SHA-256 of the exact plaintext the member
encrypted, and the member's signature covers it. **This lets members verify
the command without being able to decrypt the proposal.** The proposal is
encrypted to the service, so only the proposer and the service can read it.
Every acceptance carries the same plaintext, and a reader with the epoch key
checks that it hashes to the tag on the embedded signed proposal. So the
service can't record a different command under a member's proposal: a
substituted command, or the right command with a changed salt, fails.

- **The salt** is 32 random bytes, fresh for each proposal (`salt`, 64
  lowercase hex). The tag is public, so without it anyone could test guesses
  of a short command, such as approving a known expense, against the hash. A
  missing or malformed salt is refused.
- **The exact text is committed, not its meaning.** The acceptance carries
  the plaintext unchanged, as a JSON string (see Acceptance), because
  re-serializing could reorder keys or change escaping. The same payload
  with its keys in another order doesn't match.

**The proposal's event id is its idempotency key.** Sending the same event
again returns the original acknowledgment (its acceptance or rejection) and
changes nothing. A proposal re-signed with a new `created_at` is a new
proposal; for a command with targets (below), `baseSeq` makes the second
copy stale.

### Commands

Records use `ledger-core`'s JSON shapes (`src/domain/types.ts`).

| `type`                 | Fields                                                                                                  | Applied with                  |
| ---------------------- | ------------------------------------------------------------------------------------------------------- | ----------------------------- |
| `CreateBusiness`       | `service`, `recovery`, `profile`, `accounts`, `categories`                                              | Genesis (see below)           |
| `SaveDraft`            | `draft`, `documents` (document references, see below), `extraction` (optional receipt reading)          | `save_draft`                  |
| `Approve`              | `expenseId`                                                                                             | `approve`                     |
| `Correct`              | `originalId`, `replacement`, `reason`, `documents` and `extraction` (each optional, as for `SaveDraft`) | `correct`                     |
| `CommitImport`         | `accountId`, `rows` (previewed rows)                                                                    | `commit_import`               |
| `UpdateSettings`       | `change`: `profile`, `addAccount`, `renameAccount`, `addCategory`, `renameCategory` (`op` field)        | `update_settings`             |
| `SaveRule`             | `rule`: `id` (absent for a new rule), `vendorIncludes`, `categoryId`, `purpose`, `enabled`              | `save_rule`                   |
| `ChangeMembership`     | `change` (see Membership)                                                                               | The membership rules          |
| `Veto`                 | `pending`: the proposal id of a pending change                                                          | The membership rules          |
| `SubmitReceiptDraft`   | `draft`, `documents`                                                                                    | `save_draft`, for a new draft |
| `Suggest`              | `suggestion`: `draftId`, `author`, `categoryId`, `purpose`, `note` (each optional), `evidence`          | `suggest`                     |
| `RecordTransfer`       | `transfer` (see Transfers)                                                                              | `record_transfer`             |
| `CorrectTransfer`      | `originalId`, `replacement` (a `transfer`), `reason`                                                    | `correct_transfer`            |
| `RecordOpeningBalance` | `openingBalance`: `accountId`, `date`, `amountMinor`, `reference` (optional)                            | `record_opening_balance`      |

`update_settings` and `save_rule` port `src/domain/settings.ts`, which holds
the Settings screen's rules: names unique within their list ignoring case,
a new category with its own expense account under the same name (renamed
with it, and free among the accounts too), and a saved rule moving to the
end of the list. `addAccount` takes a `kind` of `bank`, `card`,
`liability` (money the business owes, such as a loan from the owner) or
`equity` (the owner's stake). Any of these four can pay an expense.

### Transfers

`record_transfer`, `correct_transfer`, `record_opening_balance` and the
`balances` query port `src/domain/transfers.ts`
([owner funding brief](briefs/owner-funding.md)). A `transfer` is:

```json
{
  "date": "2027-01-15",
  "amountMinor": 50000,
  "currency": "USD",
  "fromAccountId": "…",
  "toAccountId": "…",
  "transferType": "ownerLoan",
  "purpose": "Working capital",
  "reference": "optional",
  "documentIds": []
}
```

- The journal (kind `transfer`) debits the to-account and credits the
  from-account, and stores the transfer type, the purpose and any
  reference and document ids. It posts at once (there are no transfer
  drafts) and never counts as spending.
- The type must fit the accounts: `ownerLoan` from a liability to a bank or
  card account; `ownerContribution` from equity to a bank or card account;
  `loanRepayment` from a bank or card account to a liability; `ownerDraw`
  from a bank or card account to equity; `betweenAccounts` between bank or
  card accounts. Both accounts are this business's, differ, and are in the
  transfer's currency.
- `CorrectTransfer` posts an exact reversal of the original (its
  `reversesId` names the original) and the replacement (its
  `correctionOf` names the original), all or nothing. A transfer already
  reversed can't be corrected again.
- `RecordOpeningBalance` posts a transfer of type `openingBalance` between
  the account (bank, card or liability) and an equity account named
  "Opening balances", added the first time it's needed ("Opening balances
  (EUR)" for a currency other than the business's). `amountMinor` is the
  balance as `balances` reports it (what's in a bank account, or what's
  owed on a card or loan account), and may be negative. Each account has
  at most one, dated on or before the business's first other entry. It is
  corrected with `CorrectTransfer`, by a replacement of type
  `openingBalance` on the same account and opening balances account,
  still dated on or before the first other entry ("An opening balance
  correction keeps its account.").
- `RecordTransfer` refuses the type `openingBalance`, and
  `CorrectTransfer` replaces an opening balance only with another.
- `balances` gives a bank account's balance as debits minus credits, and a
  card, liability or equity account's as credits minus debits: a card's
  balance is what's owed on it.

Document ids must name documents the business already has; these commands
attach no new files.

Every command is applied by one function, `protocol::apply_command`, which
the service runs and auditors re-run:

- `SaveDraft` first adds a document record for each new reference (its
  `sha256`, the service's time), then calls `save_draft`, which refuses a
  draft that points at a document the business doesn't have. Then it stores
  the receipt reading, if there is one.
- **A receipt reading** (`extraction`, an `ExtractionRecord`) must belong to
  this business, this draft and one of the draft's documents, and must not
  be approved yet. The service gives it its own id and time. Like the
  browser, it replaces an earlier unapproved reading of the same draft and
  document.
- `Correct` checks and adds its document references and stores its reading
  (for the replacement) the same way, before calling `correct`. Posting the
  replacement finalizes the reading's outcomes, as approving a draft does.
- `SubmitReceiptDraft` does the same as `SaveDraft` without a reading, and
  only for a draft id the business doesn't have yet.
- `Suggest` adds a **suggestion** record ([decision 0004](decisions/0004-bring-your-own-ai.md)):
  a proposed category, purpose or note for a draft, with its evidence and
  the key of the assistant that made it. `suggest` ports
  `src/domain/suggestions.ts`. The draft must be an open draft of this
  business, the category one of its categories, and there must be at least
  one of a category, a purpose or a note, and at least one line of
  evidence. The service gives the record its own id and time. **A
  suggestion never changes the draft**: a person reads it and accepts or
  ignores it. Its `author` must be the key that signed the proposal (see
  Permissions), so every suggestion is attributable.

New records get ids from the service's environment, never from the
proposal, except where `ledger-core` already takes the id from its input
(a draft's own id, a document's id from its reference, a rule being
replaced). An import source is not an exception: `commit_import` replaces
the id each row's preview gave it.

**A `CommitImport` is trusted as previewed.** The rows are the member's own
preview of their own file, and the service never sees the file. Commit
checks each row's business, account and scope (its fingerprint), that its
values are valid, and that a reference already imported keeps its date,
amount and currency. It can't check that a row matches the file, so a
member can commit rows with any valid date, description and amount: the
same thing they could do by editing the file before previewing it. A
referenced row's fingerprint is its scope and reference only, on purpose,
so that a changed row under a reference already imported is caught as a
conflict rather than imported twice.

### Large imports

A CSV import can hold 2,000 rows, far more than fits in one event. The
client sends it as consecutive `CommitImport` proposals, each batch sized so
that its acceptance fits:

- **Start at 25 rows** (`IMPORT_BATCH_START`). The client previews the whole
  file once, then sends its rows in file order. It sends each batch after
  the previous one is acknowledged, with the `baseSeq` it has seen.
  `CommitImport` has no target records, so one batch never makes the next
  one stale.
- **Resize on `tooLarge`.** A `tooLarge` rejection carries the `size` the
  acceptance would have been and the `limit`; so does the builder's own
  error when the proposal itself is too large. The client sends the same rows
  again in a batch of

  > current rows × limit ÷ size × 0.9, rounded down, at least 1

  (`protocol::next_batch`), and keeps that size for the rest of the file.
  The 10% margin covers rows longer than the batch's average. If a single
  row is still too large, it can't be imported, and the client stops and
  says so.

- **Each batch is atomic.** `commit_import` applies all of a batch's rows or
  none of them. The file as a whole is no longer all-or-nothing: if a batch
  is rejected for any other reason, the client stops and shows the message
  and how many rows were imported. Earlier batches stay imported, and later
  ones are not sent.
- **Retries are safe.** Resending the same event returns its original
  acknowledgment (it is the idempotency key). A batch re-signed after a lost
  acknowledgment, or the whole file sent again, imports nothing twice:
  `commit_import` skips any row whose fingerprint is already imported with
  the same date, amount and currency. It rejects one whose details differ
  ("Existing reference has conflicting transaction details."). A `tooLarge`
  rejection stored nothing, so its rows are simply sent again.

**How many proposals.** `crates/protocol/tests/limits.rs` sends a 2,000-row
import of invented rows this way, with a new vendor on every row. _Typical_
text means descriptions of about 20 characters and vendor names of about 20.
_Long_ text means about 110 and 60, plus a long category and purpose.

| Text    | Form            | Accepted proposals | `tooLarge` rejections | Batch size | Largest acceptance |
| ------- | --------------- | ------------------ | --------------------- | ---------- | ------------------ |
| Typical | Payments        | 182                | 1                     | 11         | 55,291 bytes (84%) |
| Typical | Expense history | 250                | 1                     | 8          | 55,291 bytes (84%) |
| Long    | Payments        | 223                | 1                     | 9          | 55,291 bytes (84%) |
| Long    | Expense history | 334                | 1                     | 6          | 55,291 bytes (84%) |

In every case the first 25-row batch is too large, and the next batch, sized
from the reported size, fits for the rest of the file. (Halving instead took
334 proposals and 2 rejections in all four cases.) The largest acceptance is
the same in every case because NIP-44 pads plaintext to fixed steps, and the
batches land on the same step.

Most of an acceptance is fixed overhead per row, so text length matters less
than one might expect. It carries each row three times: inside the member's
signed proposal (encrypted), in the plaintext payload, and as the records the
row created (draft, payment or import source, vendor, event and audit entry,
each with UUIDs, fingerprints and timestamps). Then the whole acceptance is
encrypted again.

### Documents

A proposal refers to a receipt file by id and content hash. In `SaveDraft`,
`Correct` and `SubmitReceiptDraft`, a reference may also carry `stored`:
where the file's encrypted copy is kept in the service's blob store, and
the key that opens it ([receipt storage brief](briefs/receipt-storage.md)):

```json
{
  "id": "doc-3",
  "sha256": "<64 lowercase hex>",
  "name": "receipt.pdf",
  "mime": "application/pdf",
  "size": 48213,
  "stored": {
    "hash": "<64 lowercase hex: SHA-256 of the ciphertext>",
    "size": 48229,
    "alg": "AES-256-GCM",
    "key": "<base64 of a fresh 32-byte key>",
    "nonce": "<base64 of a fresh 12-byte nonce>"
  }
}
```

- **The browser encrypts the file** with WebCrypto's AES-256-GCM, under a
  key and nonce of its own for that file, uploads the ciphertext
  (`PUT /v1/blobs/{hash}`, see `docs/service.md`), and only then sends the
  proposal.
- **The key lives only in the books.** The document record keeps `stored`
  as it is, so the key is encrypted under the epoch keys like every other
  record, and only members who read the books can open the file. It
  travels with the record through rebuild, audit and backups, which treat
  it as ordinary record data and never need the file.
- **`ledger-core` checks its shape** (`check_document`, `checkDocument` in
  TypeScript): a lowercase 64-character hex `hash`, a `size` from 1 byte to
  10 MB plus the 16-byte tag (`MAX_STORED_BYTES`, 10,485,776), a known
  `alg`, canonical base64 of exactly 32 bytes for `key` and 12 for `nonce`,
  and a `sha256` on the document. The refusal is `ledger-core`'s message.
- **The service also checks the blob:** a document whose `stored` names a
  blob not uploaded for this business, or whose `size` isn't the uploaded
  blob's, is rejected ("A receipt's encrypted file wasn't uploaded to this
  business, or its size doesn't match. Upload it, then send the proposal
  again."). When the proposal is accepted, the blob is kept for good.
- **After decrypting,** the browser checks that the plaintext's SHA-256 is
  the document's `sha256` and that its file type matches `mime`, and shows
  nothing if either fails.

A reference without `stored` is as before: the file stays in the browser
that attached it, and the service stores and checks the reference, never
the file. Documents recorded before receipt storage have no `stored`, and
load unchanged. The id follows the record id rule above.

`limits()` in the WASM bindings gives `maxReceiptBytes` (10 MB) and
`maxStoredBytes` beside the event limits.

## Genesis

The first proposal for a business is `CreateBusiness`, signed by the
founding owner:

```json
{
  "type": "CreateBusiness",
  "service": "<service public key>",
  "recovery": { "type": "backupKey", "pubkey": "<offline key>" },
  "profile": { "id": "abab…ab", "businessId": "abab…ab", "name": "…" },
  "accounts": [],
  "categories": []
}
```

`recovery` is either `{"type": "secondOwner", "pubkey": …}`, a second owner
added at genesis with full history, or `{"type": "backupKey", "pubkey": …}`,
a key kept offline. The owner chooses.

Genesis's accounts and categories meet the rules every later one does: ids
under the record id rule and unique, every category on its own expense
account, and account names and category names each unique within their list
ignoring case and not blank. Genesis generates no ids, so its acceptance
records none, and an audit refuses one that does.

The service accepts genesis as **seq 0**, in epoch 0. Because the acceptance embeds the
owner's signed proposal (see below), and that proposal names the service's
key in a signed tag, the genesis acceptance is the **service-key
declaration** of decision 0001. A reader takes the expected service key from
it and verifies every later acceptance against that key.

## Acceptance (kind 4131)

Signed by the service. This is the books.

| Tag       | Value                                                                        |
| --------- | ---------------------------------------------------------------------------- |
| `b`       | The business id                                                              |
| `seq`     | Contiguous from 0                                                            |
| `prev`    | The previous acceptance's event id; absent only at seq 0                     |
| `e`       | The proposal's event id                                                      |
| `epoch`   | The epoch whose key the content is encrypted to                              |
| `service` | Only on the acceptance that activates a service-key replacement: the new key |

The content is NIP-44 encrypted from the service key to **the current epoch
key's public key**:

```json
{
  "v": 1,
  "proposal": { "id": "…", "kind": 4130, "sig": "…" },
  "payload": "{\"v\":1,\"business\":\"abab…ab\",\"baseSeq\":41,\"salt\":\"…\",\"command\":{…}}",
  "env": { "ids": ["…", "…"], "now": "2027-01-15T08:00:00.000Z" },
  "changes": [{ "type": "draft", "record": {} }],
  "outcome": { "type": "applied" }
}
```

- `proposal`: the member's signed proposal event, exactly as received.
  Anyone can check that the member signed a proposal with this id, command
  type, `baseSeq` and payload hash. Only the member and the service can
  decrypt its content.
- `payload`: the proposal's plaintext payload, as the exact text the member
  encrypted (a JSON string). A reader checks that it hashes to the
  proposal's signed `payload` tag (see Payload commitment), and that its
  business, type and `baseSeq` agree with the other signed tags.
- `env`: the **environment values the service used**: every generated id,
  in the order `ledger-core` asked for them, and the timestamp. The
  service's `today()` is the UTC date of `now`, so it is not recorded
  separately.
- `changes`: the **resulting record changes**, in order. Each is one of:
  - `{"type": <record type>, "record": <record>}`: a record created or
    replaced. Types: `profile`, `account`, `category`, `vendor`, `document`,
    `payment`, `draft`, `journal`, `approval`, `rule`, `audit`, `event`,
    `extraction`, `suggestion`;
  - `{"type": "order", "of": <record type>, "ids": [...]}`: the collection's
    records in this order. Saving a rule moves it to the end, which matters
    because the first matching rule wins;
  - `{"type": "remove", "of": "extraction", "id": ...}`: an unapproved
    receipt reading replaced by a new one. No other record is ever removed,
    and a removal of any other type is refused.
- `outcome`: `applied`; `pending` with `effectiveAt` (an owner-level change
  waiting 48 hours); `activated` (the scheduler activating a pending
  change); or `lapsed` (a due change that could no longer apply).

**The `service` tag must match the content.** It is allowed only on an
`activated` acceptance whose command is `replaceServiceKey`, and must name
that command's key; that acceptance must carry it. Opening an acceptance
(`open_acceptance`, which every audit starts from) refuses any other
combination, and the service can't build one. Without this, whoever holds
the service key could hand the chain to another key with an ordinary
acceptance, and `verify_chain`, which reads only tags, would follow it.

**An acceptance is opened against its signer and its business.**
`open_acceptance` takes the key the chain says signs that seq (from genesis,
or the last hand-over) and refuses any other signer, as `open_rejection` and
`open_delivery` do; anyone holding an epoch key could otherwise sign an
acceptance of their own that readers would decrypt. It also refuses an
acceptance whose embedded proposal names another business, so a member's
genuine, signed proposal for one business can't be recorded in another's
chain under their name. `MembershipState::follow` and `audit` check the
business again, against the state and the books they are given.

An `activated` or `lapsed` acceptance repeats the original pending
proposal and payload. One proposal id can therefore appear on two
acceptances: `pending`, then `activated` or `lapsed`.

**Which epoch.** An acceptance that starts a new epoch (a removal, say) is
itself encrypted under the new epoch, so a removed member can't read the
acceptance that removed them, nor anything after it. Every other acceptance
stays in the current epoch. Readers check this: `MembershipState::follow`
takes the acceptance's header and refuses an `epoch` tag other than the
epoch its membership rules give after that acceptance's own change
(`WrongEpoch`), so a service that stops rotating keys, or rotates when the
rules don't, is caught by anyone re-deriving the membership.

### Two kinds of verification

- **Rebuild.** Apply each acceptance's `changes` in order (`rebuild`). A
  record replaces the record with the same id in its collection, or is
  appended; an `order` change must name every record of its collection
  exactly once. Journals are validated against the business as it stands
  (`JournalEntry::new`). The genesis acceptance's first change is the
  profile, which a rebuild starts from. The
  service computes `changes` with `diff`, which fails unless applying them
  reproduces the new state exactly, so rebuilding always gives the stored
  snapshot.
- **Audit.** Re-run each command through `apply_command` with an
  environment that hands back the recorded ids and time (`ReplayEnv`), and
  check that it produces the same `changes` and uses exactly the recorded
  number of ids (`audit`).
  Anyone holding the business key can confirm that the service applied the
  rules correctly.

### The hash chain

Event ids are SHA-256 hashes of the serialized event (NIP-01), so the
`prev` links make a tamper-evident chain. `verify_chain` checks, without
decrypting anything, that each acceptance:

1. has a valid id and signature;
2. is signed by the current service key (from genesis, or from the last
   `service` hand-over tag);
3. names this business;
4. has the next `seq`;
5. links by `prev` to the one before;
6. starts at epoch 0, and then has its predecessor's epoch or the next one:
   one acceptance can start at most one epoch.

An altered acceptance no longer matches its id or signature. A missing,
inserted or reordered one breaks `seq` or `prev`. A truncated chain is
valid but short, so a reader compares its head with the head the service
reports (and, in stage 4, with each relay).

The limit is the one decision 0001 names: whoever holds the service key can
sign a different history. Embedding members' signed proposals, with their
payload commitments, means it can't invent a proposal from a member or
change what one said, and comparing heads across copies detects a fork.

## Rejection (kind 4132)

Signed by the service, addressed to the proposer only, and **not** part of
the chain. Tags: `b`, `e` (the proposal), `p` (the proposer). The
content is encrypted to the proposer and holds a code and the message, which
is `ledger-core`'s error message or a protocol message:

```json
{
  "v": 1,
  "code": "rejected",
  "message": "This record changed since you opened it. Reload it and try again."
}
```

A `tooLarge` rejection also says how large the event would have been and the
limit, so the client can size its next batch:

```json
{
  "v": 1,
  "code": "tooLarge",
  "message": "The event is 70210 bytes, over the 65536-byte limit relays accept. Send it as smaller proposals.",
  "size": 70210,
  "limit": 65536
}
```

When the content is longer than NIP-44 can encrypt at all, `size` is what
the event would have been, worked out from NIP-44's padding rule.

| `code`     | Meaning                                                                                                                                                         |
| ---------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `tooLarge` | The proposal, or the acceptance it would produce, is over the size limit. Nothing was stored. Split it into smaller batches and send again (see Large imports). |
| `rejected` | Any other reason. The message says which.                                                                                                                       |

Only a proposal whose signature and tags are valid gets a signed rejection,
and only from a member or the recovery key of a business that exists; it is
signed by that business's own service key. A malformed request gets an HTTP
error, and so does a proposal from anyone else, or a refused genesis, with
nothing stored (see `docs/service.md`, API).

## Epochs and key delivery (kind 4133)

**An epoch key is a secp256k1 keypair** generated by the service. Content
for epoch _n_ is encrypted from the service key to epoch _n_'s public key.
A member holding epoch _n_'s secret key decrypts it with the service's
public key.

A **delivery** gives one epoch secret key to one member. Tags: `b`,
`p` (the member), `epoch`. The content is encrypted to the member:

```json
{ "v": 1, "business": "abab…ab", "epoch": 3, "secret": "<hex>" }
```

The reader checks that the content's business and epoch match the tags.
A delivery is signed by the business's service key when it was made, so
after a service-key replacement a reader accepts deliveries from any key the
chain has handed over to.

**A new epoch starts** when:

- a member is removed (including the lost key in a recovery);
- a member who reads loses read access (demoted to submitter);
- someone gains read access with **from-join-date** history: a new member,
  or a submitter promoted to a reading role. They then can't read the
  current epoch's past data;
- the service key is replaced.

**Who receives which keys.** Owners, bookkeepers, viewers and assistants receive the
current epoch's key. A member added with **full history** (the default)
receives every past epoch key too; a from-join-date member receives keys
from the epoch their joining started. Submitters, the recovery key and
removed members receive nothing.

A removed member keeps whatever they already received. Rotation protects
only the future, as decision 0001 says.

**From-join members can't rebuild the books yet.** A reader rebuilds the
books and the membership by replaying the chain from genesis, and a
from-join-date member holds no keys for the epochs before their join, so
they can't. The protocol supports from-join history, and its tests stay,
but the browser invites members with full history only until this is
solved. The planned design: such a member starts from a settings-only
snapshot at their join (the profile, accounts, categories and rules, and
the membership, with no records from before), plus the activity from their
join on, and verifies the earlier part of the chain against its headers
only: links, seqs and the service's signatures, without opening content
they can't decrypt.

Relays never see an epoch key. In stage 2 there are no relays; in stage 4
deliveries are still encrypted to each member.

## Membership

Membership is derived from accepted `CreateBusiness`, `ChangeMembership` and
`Veto` commands. There is no separate membership document. Anyone holding
the business key can rebuild it from the chain and check each decision
(`MembershipState::follow`).

`ChangeMembership` changes (`op` field):

| `op`                 | Fields                                                             | Owner-level                         |
| -------------------- | ------------------------------------------------------------------ | ----------------------------------- |
| `add`                | `member`, `role`, `history` (`full` or `fromJoin`, default `full`) | When `role` is `owner`              |
| `remove`             | `member`                                                           | When the member is an owner         |
| `changeRole`         | `member`, `role`                                                   | When the old or new role is `owner` |
| `replaceRecoveryKey` | `key` (or `null` to drop the backup key)                           | Always                              |
| `replaceServiceKey`  | `key`                                                              | Always                              |
| `recover`            | `lost`, `replacement`                                              | Always; recovery key only           |

Roles: `owner`, `bookkeeper`, `viewer`, `submitter`, `assistant`.

**An assistant is an AI agent with its own key**
([decision 0004](decisions/0004-bring-your-own-ai.md)). The user's harness
holds the key, never Zap Books, so everything the agent proposes is signed
by it. An owner adds it like anyone else, with a history choice, and
removing it works like any removal: it is refused at once and the epoch
rotates.

- **Owner-level changes wait 48 hours.** They are accepted with outcome
  `pending` and `effectiveAt` = acceptance time + 48 hours, exactly: a
  reader following the chain refuses any other time, earlier or later, so
  the service can't hold a change pending forever. When one is due,
  the service's scheduler appends an `activated` acceptance, unless a `Veto`
  was accepted first. A due change is activated before anything else the
  service accepts for the business, so a veto must arrive before
  `effectiveAt`. Pending changes are visible to all owners and the recovery
  key.
- **Veto:** any owner or the recovery key, except whoever proposed the
  change, and except the owner a `recover` names as lost (see below).
- **Checked twice.** A pending change is checked against the state when it
  is proposed and again when it falls due. If it no longer applies (its
  member has meanwhile joined in another role, say), it **lapses**: the
  scheduler appends a `lapsed` acceptance, and nothing changes.
- **Pending changes lose their proposer's authority.** When an owner is
  removed, or the recovery key replaced, their pending changes are
  cancelled.
- **Other changes take effect immediately:** adding or removing a
  bookkeeper, viewer, submitter or assistant, and changing a role between
  them.
- **The recovery key** can sign only `Veto` and `recover`. Recovery replaces
  a lost owner key with a new owner key (full history), after the same
  48-hour wait.
- **Recovery outranks the owner being recovered.** The owner key a
  `recover` names as `lost` can't veto it; every other owner still can.
  Recovery exists for a key that is lost _or stolen_, and a thief holding a
  stolen owner key would otherwise veto every recovery and keep the
  business. The cost: with a single owner, a recovery key used against
  the owner's will takes over after 48 hours, with nothing to stop it. So
  the backup key must be kept as carefully as an owner key (offline, by
  someone the owner trusts), or a second owner added, whose veto still
  cancels a malicious recovery.
- **A business always keeps at least one owner and a recovery party**: a
  backup key, or a second owner. Commands that would break this are
  rejected. The recovery key, the service key and member keys are always
  distinct.

## Permissions

Enforced by the service, and re-checked by readers.

| Command                                 | Owner    | Bookkeeper | Viewer | Submitter | Assistant              | Recovery key |
| --------------------------------------- | -------- | ---------- | ------ | --------- | ---------------------- | ------------ |
| SaveDraft                               | ✓        | ✓          |        |           | Its own drafts (below) |              |
| Approve, Correct, CommitImport          | ✓        | ✓          |        |           |                        |              |
| UpdateSettings, SaveRule                | ✓        | ✓          |        |           |                        |              |
| RecordTransfer, CorrectTransfer         | ✓        | ✓          |        |           |                        |              |
| RecordOpeningBalance                    | ✓        | ✓          |        |           |                        |              |
| ChangeMembership (non-owner roles)      | ✓        |            |        |           |                        |              |
| ChangeMembership (owner-level)          | ✓ (48 h) |            |        |           |                        |              |
| ChangeMembership `recover`              |          |            |        |           |                        | ✓ (48 h)     |
| Veto                                    | ✓        |            |        |           |                        | ✓            |
| SubmitReceiptDraft                      | ✓        | ✓          |        | ✓         | ✓                      |              |
| Suggest                                 |          |            |        |           | ✓ (any open draft)     |              |
| Read acceptances and receive epoch keys | ✓        | ✓          | ✓      |           | ✓                      |              |
| See pending owner-level changes         | ✓        |            |        |           |                        | ✓            |

No one can veto their own change, and the owner a `recover` names as lost
can't veto that recovery.

A removed member is an outsider and is refused everything at once, before
any key rotation completes.

**Receipt files** follow the same lines. A blob can be uploaded by anyone
who may attach a receipt (an owner, bookkeeper, assistant or submitter),
and downloaded only by a member who reads the books (an owner, bookkeeper,
viewer or assistant), once an accepted record refers to it. A submitter
sends receipts in but can't fetch them back; the recovery key can do
neither. The blob holds only ciphertext, so the read control is, as
everywhere, the key, which is in the books.

**An assistant never approves.** Approval stays with a person, and that is
enforced here and by the signature on every proposal, not by an interface.
An assistant can't approve, correct or import, record or correct
transfers or opening balances, change settings, rules or membership, veto,
recover, or see pending owner-level changes
(`MembershipState::sees_pending`).

**An assistant edits only the drafts it created, until someone else
saves them.** It creates a draft with `SubmitReceiptDraft`, which accepts
only a new draft id, and may then `SaveDraft` that draft; a `SaveDraft` of
any other draft, including a new id, is refused ("Assistants can only edit
drafts they created."). People can edit an assistant's drafts as usual.

Which drafts each assistant may still edit is part of the membership state
(`assistantDrafts`, draft id to assistant key), derived from the chain like
the rest of it:

- each accepted `SubmitReceiptDraft` signed by an assistant adds its draft
  (`MembershipState::accepted`);
- an accepted `SaveDraft` signed by anyone but that assistant removes the
  draft: a person who saves it takes it over, and from then on the
  assistant can only `Suggest` on it;
- removing an assistant, or changing its role to anything but `assistant`,
  removes all of its drafts. They don't come back if it is added again or
  changed back. A change to `owner` waits 48 hours like any owner-level
  change, and the drafts go when it activates.

So a reader re-deriving the membership re-checks this rule too, and catches
a service that let an assistant edit a draft that isn't, or is no longer,
its own.

**A suggestion names its signer.** A `Suggest` whose `author` isn't the
hex public key that signed the proposal is refused ("A suggestion must name
the key that signed it."). Only assistants suggest; people edit the draft
instead.

## Stale proposals (`baseSeq`)

Each command carries `baseSeq`, the last seq its proposer had seen. The
service rejects a command whose **target records** changed in an acceptance
after `baseSeq`, with the message _"This record changed since you opened
it. Reload it and try again."_ This replaces the browser's revision check.

| Command                       | Targets                                                                                               |
| ----------------------------- | ----------------------------------------------------------------------------------------------------- |
| SaveDraft, SubmitReceiptDraft | The draft                                                                                             |
| Approve                       | The expense                                                                                           |
| Correct                       | The original expense                                                                                  |
| UpdateSettings                | The profile, or the account or category renamed; none when adding                                     |
| SaveRule                      | The rule being replaced; none for a new rule                                                          |
| CommitImport                  | None: rows are deduplicated by fingerprint                                                            |
| Suggest                       | None: it adds a record and never changes the draft                                                    |
| RecordTransfer                | None: it adds records                                                                                 |
| CorrectTransfer               | The original journal, which never changes once posted; a transfer already reversed is refused instead |
| RecordOpeningBalance          | None: a second opening balance for an account is refused                                              |
| ChangeMembership, Veto        | None: checked against the current membership                                                          |

A `baseSeq` later than the chain head is malformed. So when two members
approve the same expense from the same `baseSeq`, the first acceptance
changes the expense and the second is rejected as stale.

## Future options

Not planned. They would cut event sizes further if imports or relays need it.

- **Compress before encrypting.** The acceptance's JSON is highly
  repetitive (field names, ids, timestamps) and would shrink several-fold,
  so far fewer import batches would be needed. The costs: NIP-44 has no
  compression, so it would be our own format inside the payload, and every
  client and auditor would need the same decompressor. Compressed length
  also leaks information about the plaintext (the CRIME and BREACH class of
  attacks) when attacker-influenced text, such as an imported description,
  sits next to secret data. Padding hides some of this, but not all.
- **Drop record changes and rebuild by re-execution.** An acceptance could
  carry only the command and its environment values, and a reader would
  rebuild by re-running every command, as an audit does today. Acceptances
  would be roughly half the size. The costs: every reader must run exactly
  the rules each acceptance was made under. Any later change to
  `ledger-core` (a bug fix, say) would change how old history replays, so
  every version of the rules would have to be kept and selected per
  acceptance. Rebuild and audit would also become the same check, losing
  the independent comparison of recorded changes against re-execution.
