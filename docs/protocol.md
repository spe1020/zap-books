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
  `Authorization` header).

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
- **Keys and event ids** are 64-character lowercase hex, as in NIP-01.
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

| `type`               | Fields                                                                                           | Applied with                  |
| -------------------- | ------------------------------------------------------------------------------------------------ | ----------------------------- |
| `CreateBusiness`     | `service`, `recovery`, `profile`, `accounts`, `categories`                                       | Genesis (see below)           |
| `SaveDraft`          | `draft`, `documents` (document references, see below), `extraction` (optional receipt reading)   | `save_draft`                  |
| `Approve`            | `expenseId`                                                                                      | `approve`                     |
| `Correct`            | `originalId`, `replacement`, `reason`                                                            | `correct`                     |
| `CommitImport`       | `accountId`, `rows` (previewed rows)                                                             | `commit_import`               |
| `UpdateSettings`     | `change`: `profile`, `addAccount`, `renameAccount`, `addCategory`, `renameCategory` (`op` field) | `update_settings`             |
| `SaveRule`           | `rule`: `id` (absent for a new rule), `vendorIncludes`, `categoryId`, `purpose`, `enabled`       | `save_rule`                   |
| `ChangeMembership`   | `change` (see Membership)                                                                        | The membership rules          |
| `Veto`               | `pending`: the proposal id of a pending change                                                   | The membership rules          |
| `SubmitReceiptDraft` | `draft`, `documents`                                                                             | `save_draft`, for a new draft |

`update_settings` and `save_rule` port `src/domain/settings.ts`, which holds
the Settings screen's rules: names unique within their list ignoring case,
a new category with its own expense account, and a saved rule moving to the
end of the list.

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
- `SubmitReceiptDraft` does the same as `SaveDraft` without a reading, and
  only for a draft id the business doesn't have yet.

New records get ids from the service's environment, never from the
proposal, except where `ledger-core` already takes the id from its input
(a draft's own id, a document's id from its reference, a rule being
replaced).

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

Receipt files stay in the browser in stage 2 (stage 3 moves them to
encrypted storage). A proposal refers to a file by id and content hash:

```json
{
  "id": "doc-3",
  "sha256": "<64 lowercase hex>",
  "name": "receipt.pdf",
  "mime": "application/pdf",
  "size": 48213
}
```

The service stores and checks the reference, never the file, and the
ledger's existing document rules apply to it.

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

The service accepts genesis as **seq 0**. Because the acceptance embeds the
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
    `extraction`;
  - `{"type": "order", "of": <record type>, "ids": [...]}`: the collection's
    records in this order. Saving a rule moves it to the end, which matters
    because the first matching rule wins;
  - `{"type": "remove", "of": "extraction", "id": ...}`: an unapproved
    receipt reading replaced by a new one. No other record is ever removed,
    and a removal of any other type is refused.
- `outcome`: `applied`; `pending` with `effectiveAt` (an owner-level change
  waiting 48 hours); `activated` (the scheduler activating a pending
  change); or `lapsed` (a due change that could no longer apply).

An `activated` or `lapsed` acceptance repeats the original pending
proposal and payload. One proposal id can therefore appear on two
acceptances: `pending`, then `activated` or `lapsed`.

**Which epoch.** An acceptance that starts a new epoch (a removal, say) is
itself encrypted under the new epoch, so a removed member can't read the
acceptance that removed them, nor anything after it.

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
6. never goes back to an earlier epoch.

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

Only a proposal whose signature and tags are valid gets a signed rejection.
A malformed request gets an HTTP error.

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

**Who receives which keys.** Owners, bookkeepers and viewers receive the
current epoch's key. A member added with **full history** (the default)
receives every past epoch key too; a from-join-date member receives keys
from the epoch their joining started. Submitters, the recovery key and
removed members receive nothing.

A removed member keeps whatever they already received. Rotation protects
only the future, as decision 0001 says.

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

Roles: `owner`, `bookkeeper`, `viewer`, `submitter`.

- **Owner-level changes wait 48 hours.** They are accepted with outcome
  `pending` and `effectiveAt` = acceptance time + 48 hours. When one is due,
  the service's scheduler appends an `activated` acceptance, unless a `Veto`
  was accepted first. A due change is activated before anything else the
  service accepts for the business, so a veto must arrive before
  `effectiveAt`. Pending changes are visible to all owners and the recovery
  key.
- **Veto:** any owner or the recovery key, except whoever proposed the
  change.
- **Checked twice.** A pending change is checked against the state when it
  is proposed and again when it falls due. If it no longer applies (its
  member has meanwhile joined in another role, say), it **lapses**: the
  scheduler appends a `lapsed` acceptance, and nothing changes.
- **Pending changes lose their proposer's authority.** When an owner is
  removed, or the recovery key replaced, their pending changes are
  cancelled.
- **Other changes take effect immediately:** adding or removing a
  bookkeeper, viewer or submitter, and changing a role between them.
- **The recovery key** can sign only `Veto` and `recover`. Recovery replaces
  a lost owner key with a new owner key (full history), after the same
  48-hour wait. The owner whose key was not really lost can veto it.
- **A business always keeps at least one owner and a recovery party**: a
  backup key, or a second owner. Commands that would break this are
  rejected. The recovery key, the service key and member keys are always
  distinct.

## Permissions

Enforced by the service, and re-checked by readers.

| Command                                   | Owner    | Bookkeeper | Viewer | Submitter | Recovery key |
| ----------------------------------------- | -------- | ---------- | ------ | --------- | ------------ |
| SaveDraft, Approve, Correct, CommitImport | ✓        | ✓          |        |           |              |
| UpdateSettings, SaveRule                  | ✓        | ✓          |        |           |              |
| ChangeMembership (non-owner roles)        | ✓        |            |        |           |              |
| ChangeMembership (owner-level)            | ✓ (48 h) |            |        |           |              |
| ChangeMembership `recover`                |          |            |        |           | ✓ (48 h)     |
| Veto                                      | ✓        |            |        |           | ✓            |
| SubmitReceiptDraft                        | ✓        | ✓          |        | ✓         |              |
| Read acceptances and receive epoch keys   | ✓        | ✓          | ✓      |           |              |

A removed member is an outsider and is refused everything at once, before
any key rotation completes.

## Stale proposals (`baseSeq`)

Each command carries `baseSeq`, the last seq its proposer had seen. The
service rejects a command whose **target records** changed in an acceptance
after `baseSeq`, with the message _"This record changed since you opened
it. Reload it and try again."_ This replaces the browser's revision check.

| Command                       | Targets                                                           |
| ----------------------------- | ----------------------------------------------------------------- |
| SaveDraft, SubmitReceiptDraft | The draft                                                         |
| Approve                       | The expense                                                       |
| Correct                       | The original expense                                              |
| UpdateSettings                | The profile, or the account or category renamed; none when adding |
| SaveRule                      | The rule being replaced; none for a new rule                      |
| CommitImport                  | None: rows are deduplicated by fingerprint                        |
| ChangeMembership, Veto        | None: checked against the current membership                      |

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
