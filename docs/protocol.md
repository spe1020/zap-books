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

- **Tags.** `e` and `p` keep their NIP-01 meanings. The others are ours:
  `business`, `cmd`, `base_seq`, `service`, `seq`, `prev`, `epoch`. Each tag
  this document lists appears at most once; a repeated tag is malformed, so
  two readers can never disagree about which value counts. Other tags are
  ignored.
- **Numbers** in tags are decimal with no sign and no leading zeros (`0`,
  `17`), so each has one spelling.
- **Business id:** 32 random bytes chosen by the founding owner, written as
  64 lowercase hex characters. No other spelling parses.
- **Keys and event ids** are 64-character lowercase hex, as in NIP-01.
- **Times.** `created_at` is Unix seconds. Inside `ledger-core` records,
  times stay ISO-8601 strings, as today.

### Encryption

All content is NIP-44 version 2, through the `nostr` crate (no
hand-written cryptography). NIP-44 v2 encrypts 1 to 65,535 bytes. Longer
content (a large import, or the acceptance that records it) is split at
character boundaries into chunks of at most 65,535 bytes. Each chunk is
encrypted on its own, and the content is the JSON array of the chunk
payloads, in order:

```json
["Ak9u…", "AuRf…", "AqYx…"]
```

A base64 payload never starts with `[`, so the two forms can't be confused.
A one-element array is refused, so a plaintext has exactly one encoding.

NIP-44's conversation key is symmetric: content from key A to key B is read
with B's secret key and A's public key, or with A's secret key and B's
public key.

## Proposal (kind 4130)

Signed by a member (or the recovery key). The content is NIP-44 encrypted
**to the service's public key**, so every role can propose without holding
the business key. That is what makes the write-only submitter role possible.

```json
{
  "kind": 4130,
  "pubkey": "<member>",
  "tags": [
    ["business", "abab…ab"],
    ["cmd", "Approve"],
    ["base_seq", "41"]
  ],
  "content": "<NIP-44 to the service>"
}
```

| Tag        | Value                                                        | Required                     |
| ---------- | ------------------------------------------------------------ | ---------------------------- |
| `business` | The business id                                              | Always                       |
| `cmd`      | The command type, so the service can route before decrypting | Always                       |
| `base_seq` | The last acceptance seq the proposer had seen                | Every command except genesis |
| `service`  | The service's public key                                     | Genesis only, and only there |

The decrypted content is the **payload**:

```json
{
  "v": 1,
  "business": "abab…ab",
  "baseSeq": 41,
  "command": { "type": "Approve", "expenseId": "draft-7" }
}
```

The service refuses a proposal unless its signature is valid, its tags are
well formed, and the payload agrees with the tags: same business, same
command type, same `baseSeq`, and for genesis, the same service key, which
must be this service's key.

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
| `SaveDraft`          | `draft`, `documents` (document references, see below)                                            | `save_draft`                  |
| `Approve`            | `expenseId`                                                                                      | `approve`                     |
| `Correct`            | `originalId`, `replacement`, `reason`                                                            | `correct`                     |
| `CommitImport`       | `accountId`, `rows` (previewed rows)                                                             | `commit_import`               |
| `UpdateSettings`     | `change`: `profile`, `addAccount`, `renameAccount`, `addCategory`, `renameCategory` (`op` field) | Not in `ledger-core` yet      |
| `SaveRule`           | `rule`: `id` (absent for a new rule), `vendorIncludes`, `categoryId`, `purpose`, `enabled`       | Not in `ledger-core` yet      |
| `ChangeMembership`   | `change` (see Membership)                                                                        | The membership rules          |
| `Veto`               | `pending`: the proposal id of a pending change                                                   | The membership rules          |
| `SubmitReceiptDraft` | `draft`, `documents`                                                                             | `save_draft`, for a new draft |

`UpdateSettings` and `SaveRule` are applied in the browser today
(`src/components/Settings.tsx`, `Review.tsx`), not by a domain function. The
service needs them in `ledger-core`, which is a parity change (TypeScript
first). That is raised with PR 2.

New records get ids from the service's environment, never from the
proposal, except where `ledger-core` already takes the id from its input
(a draft's own id, a rule being replaced).

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

| Tag        | Value                                                                        |
| ---------- | ---------------------------------------------------------------------------- |
| `business` | The business id                                                              |
| `seq`      | Contiguous from 0                                                            |
| `prev`     | The previous acceptance's event id; absent only at seq 0                     |
| `e`        | The proposal's event id                                                      |
| `epoch`    | The epoch whose key the content is encrypted to                              |
| `service`  | Only on the acceptance that activates a service-key replacement: the new key |

The content is NIP-44 encrypted from the service key to **the current epoch
key's public key**:

```json
{
  "v": 1,
  "proposal": { "id": "…", "kind": 4130, "sig": "…" },
  "payload": { "v": 1, "business": "abab…ab", "baseSeq": 41, "command": {} },
  "env": { "ids": ["…", "…"], "now": "2027-01-15T08:00:00.000Z" },
  "changes": [{ "type": "draft", "record": {} }],
  "outcome": { "type": "applied" }
}
```

- `proposal`: the member's signed proposal event, exactly as received.
  Anyone can check that the member signed a proposal with this id, command
  type and `baseSeq`. Only the member and the service can decrypt its
  content, so `payload` carries the decrypted command; a reader checks that
  its business, type and `baseSeq` agree with the signed tags.
- `env`: the **environment values the service used**: every generated id,
  in the order `ledger-core` asked for them, and the timestamp. The
  service's `today()` is the UTC date of `now`, so it is not recorded
  separately.
- `changes`: the **resulting record changes**, records created or replaced,
  each as `{"type": <record type>, "record": <record>}`. Types: `profile`,
  `account`, `category`, `vendor`, `document`, `payment`, `draft`,
  `journal`, `approval`, `rule`, `audit`, `event`, `extraction`. There are no
  deletions, because the ledger never removes a record.
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

- **Rebuild.** Apply each acceptance's `changes` in order. A record replaces
  the record with the same id in its collection, or is appended. Journals are
  validated against the business as it stands (`JournalEntry::new`). The
  service computes `changes` with `diff`, which fails unless applying them
  reproduces the new state exactly, so rebuilding always gives the stored
  snapshot.
- **Audit.** Re-run each command through `ledger-core` with an environment
  that hands back the recorded ids and time (`ReplayEnv`), and check that it
  produces the same `changes` and uses exactly the recorded number of ids.
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
sign a different history. Embedding members' signed proposals means it
can't invent a proposal from a member, and comparing heads across copies
detects a fork.

## Rejection (kind 4132)

Signed by the service, addressed to the proposer only, and **not** part of
the chain. Tags: `business`, `e` (the proposal), `p` (the proposer). The
content is encrypted to the proposer and holds the message, which is
`ledger-core`'s error message or a protocol message:

```json
{
  "v": 1,
  "message": "This record changed since you opened it. Reload it and try again."
}
```

Only a proposal whose signature and tags are valid gets a signed rejection.
A malformed request gets an HTTP error.

## Epochs and key delivery (kind 4133)

**An epoch key is a secp256k1 keypair** generated by the service. Content
for epoch _n_ is encrypted from the service key to epoch _n_'s public key.
A member holding epoch _n_'s secret key decrypts it with the service's
public key.

A **delivery** gives one epoch secret key to one member. Tags: `business`,
`p` (the member), `epoch`. The content is encrypted to the member:

```json
{ "v": 1, "business": "abab…ab", "epoch": 3, "secret": "<hex>" }
```

The reader checks that the content's business and epoch match the tags.

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
  was accepted first. Pending changes are visible to all owners.
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

## Limits and open questions

- **Event size.** A 2,000-row import makes a proposal and an acceptance of
  roughly a megabyte. NIP-44 chunking handles encryption, and HTTPS handles
  the size in stage 2, but many relays refuse events that large. Stage 4
  needs either relays configured for it or imports split into several
  proposals (which would change `commit_import`'s all-or-nothing rule).
- **Relay filtering.** Our tags are multi-letter, which relays don't index.
  Stage 4's read-side policy plugin can still read them, but if clients need
  to filter by business on relays, a single-letter tag would have to be
  added then.
- **Document hashes.** `SourceDocument` has no hash field. PR 2 decides
  whether the service keeps references in its own table or `ledger-core`
  gains the field (a parity change).
- **Receipt readings.** The browser stores an `ExtractionRecord` when a
  draft is saved after reading a receipt. `SaveDraft` doesn't carry one yet,
  and `recordExtraction` isn't in `ledger-core`.
