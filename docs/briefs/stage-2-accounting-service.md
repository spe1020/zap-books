# Brief: Stage 2, the accounting service on Nostr

**For:** Claude Code
**Repo:** `spe1020/zap-books`
**Save this brief as:** `docs/briefs/stage-2-accounting-service.md`
**Background, read first:** `docs/decisions/0001-nostr-security-model.md`, `docs/decisions/0002-stage-1-acceptance.md`, `docs/architecture.md`, `PARITY_NOTES.md`.

**Privacy rule:** invented values only, everywhere. The pre-commit guard must stay installed and passing.

## Goal

Make one authoritative ledger service the signer of record for each business's books, using Nostr identities, signed events and encryption, as decided in 0001. After stage 2:

- a person signs in with their Nostr key and belongs to businesses with a role;
- every change to the books is a signed proposal, validated by `ledger-core` on the service, and either accepted into a signed, hash-chained history or rejected;
- the books can be rebuilt and checked from that history alone;
- removing a member cuts them off immediately and rotates the business key.

## Decisions already made (do not reopen)

From 0001 and Seth's answers:

- **Signer of record:** a ledger service with its own Nostr key. Only events it signs count as the books. Its database is authoritative; relays (stage 4) carry replicas.
- **Keys:** member identity keys (never rotated, only revoked), business **epoch keys** (rotated by the service on membership changes), and the service key.
- **Roles:** owner, bookkeeper, viewer in v1. The submitter role is designed in now, so the key and permission model supports it, but its UI ships later.
- **Owner-level changes wait 48 hours.** During that time any other owner or the recovery key can veto. Owner-level means: adding or removing an owner, changing the recovery key, replacing the service key.
- **Recovery key:** every business must have at least one, either a second owner or an offline backup key. The owner chooses.
- **History access:** each invite chooses full history or from join date. Default: full history.
- **Service host for the pilot:** a small cloud server, separate from relays and personal machines. Stage 2 builds and tests locally. Deployment is the last PR.

## Key design choice for this stage: relays come later

The concept plan puts event synchronization (stage 4) after the authoritative service. So in stage 2, **clients send signed Nostr events to the service directly over HTTPS**, and read the signed history back the same way. The events are standard Nostr events from day one, so stage 4 only adds publishing those same events to relays. No format changes.

- **Writes:** `POST /v1/proposals` with a signed Nostr event as the body.
- **Reads and authentication:** NIP-98 HTTP Auth (a signed kind-27235 event in the `Authorization` header), with timestamp, URL and method checks and replay protection.

## Protocol

Write `docs/protocol.md` as the specification, and implement it in a new crate `crates/protocol`.

### Event kinds

Pick application-specific kinds that don't collide with registered NIP kinds. Check the current kind list in the nostr-protocol/nips repository, record the choice and the check in `docs/protocol.md`, and define them once as constants. You need:

- **Proposal:** signed by a member.
- **Acceptance:** signed by the service; this is the books.
- **Rejection:** signed by the service, addressed to the proposer only, not part of the chain.
- **Epoch key delivery:** signed by the service, one per member per epoch.

### Business identity and the genesis event

- A business id is 32 random bytes, hex-encoded, chosen by the founding owner.
- The first proposal for a business is **genesis**, signed by the founding owner. It names the service public key, the recovery configuration, the business profile, and the initial accounts and categories.
- The service accepts genesis as sequence 0. Its acceptance is the service-key declaration from 0001.

### Proposals

- `content` is NIP-44 v2 encrypted **to the service's public key**. Every role can therefore propose without holding the business key, which is what makes the submitter role possible.
- The encrypted payload is a typed command: `CreateBusiness`, `SaveDraft`, `Approve`, `Correct`, `CommitImport`, `UpdateSettings`, `SaveRule`, `ChangeMembership`, `Veto`, and `SubmitReceiptDraft` (submitter only).
- Each command carries `baseSeq`, the last sequence the client had seen. The service rejects a command whose target records were changed by a later acceptance, with a "this record changed since you opened it" message. That replaces today's browser revision check.
- **Tags:** business id, command type (unencrypted, so the service can route before decrypting), and the client's `baseSeq`.
- **The proposal's event id is its idempotency key.** Sending the same proposal again returns the original acknowledgment and changes nothing.

### Acceptances: the hash chain

Each acceptance is signed by the service and tagged with:

- business id;
- `seq`, contiguous from 0;
- `prev`, the event id of the previous acceptance (absent only for seq 0);
- the proposal's event id;
- the epoch number its content is encrypted under.

Its content is encrypted to the **current epoch key** (see Encryption) and contains:

- the original command;
- the **environment values the service used**: every generated id in order, and the timestamp;
- the **resulting record changes**: records created or replaced, by type and id.

This gives two kinds of verification:

- **Rebuild:** apply the record changes in order to reproduce the books.
- **Audit:** re-run each command through `ledger-core` with the recorded environment values, and check it produces the same changes. Anyone holding the business key can confirm the service applied the rules correctly.

Event ids are SHA-256 hashes of the serialized event, so `prev` links make a tamper-evident chain. A missing, altered or reordered acceptance breaks it.

### Encryption

- **An epoch key is a secp256k1 keypair** generated by the service. Content for epoch _n_ is NIP-44 v2 encrypted using the service key and epoch _n_'s public key. Members holding epoch _n_'s private key decrypt it using the service's public key.
- **Epoch key delivery:** the service NIP-44 encrypts the epoch private key to each entitled member's public key.
- **A new epoch starts:**
  - when a member is removed;
  - when a member is added with **from-join-date** history, so they can't read the current epoch's past data;
  - and when the service key is replaced.
- **A member added with full history** receives every past epoch key.
- **Relays never see an epoch key.** In stage 2 there are no relays; this constraint is for stage 4.

### Membership

- Membership state is derived from accepted `CreateBusiness`, `ChangeMembership` and `Veto` commands. There is no separate membership document.
- **Owner-level changes** are accepted as **pending** with an `effectiveAt` 48 hours later. A scheduler in the service activates each one when it's due by appending an acceptance, unless a `Veto` from another owner or the recovery key was accepted first. Pending changes are visible to all owners.
- **Other changes** take effect immediately: adding or removing a bookkeeper or viewer, and changing a role between them.
- **The recovery key** can sign only `Veto` and recovery proposals. Recovery is replacing a lost owner key, and it is subject to the same 48-hour wait.
- **A business must always keep at least one owner and a recovery key.** Commands that would break this are rejected.

### Permissions (enforced by the service)

| Command                                   | Owner    | Bookkeeper | Viewer | Submitter | Recovery key |
| ----------------------------------------- | -------- | ---------- | ------ | --------- | ------------ |
| SaveDraft, Approve, Correct, CommitImport | ✓        | ✓          |        |           |              |
| UpdateSettings, SaveRule                  | ✓        | ✓          |        |           |              |
| ChangeMembership (non-owner roles)        | ✓        |            |        |           |              |
| ChangeMembership (owner-level)            | ✓ (48 h) |            |        |           |              |
| Veto                                      | ✓        |            |        |           | ✓            |
| SubmitReceiptDraft                        | ✓        | ✓          |        | ✓         |              |
| Read acceptances and receive epoch keys   | ✓        | ✓          | ✓      |           |              |

**A removed member is refused at the API immediately**, before any key rotation completes.

### Receipts in stage 2

Receipt files stay in the browser, as today. Encrypted document storage on Blossom is stage 3. Proposals refer to documents by id and SHA-256 hash only, and the service stores and checks those references but never the file. The ledger's existing document checks apply to the references.

## The service

New crate `crates/ledger-service`: axum, tokio, sqlx with Postgres, `ledger-core`, and `crates/protocol`.

- **Per-business ordering:** each proposal is processed inside one database transaction that locks the business row, so two proposals for the same business never interleave. Different businesses proceed in parallel.
- **Within that transaction:** verify the signature, decrypt, check permissions, check `baseSeq` conflicts, load the business state, apply the command with `ledger-core`, assign the next `seq`, sign the acceptance, store it, update the state snapshot, record the proposal's acknowledgment, and write outbox rows. Everything commits together or not at all.
- **Rejections** return a signed rejection with the `ledger-core` error message. Store them in an audit table, not in the chain.
- **`ledger-core` `Env` on the service:** ids and timestamps come from the service and are recorded in the acceptance. The service's `today()` is the UTC date. Check every place `today()` reaches a stored value, and list them in the PR. If any does, stop and ask.
- **Storage (minimal, normalize later):**
  - `businesses`: id, current seq, head event id, current epoch, state snapshot as JSON (`BusinessData`);
  - `acceptances`: append-only, holding the full signed event JSON;
  - `proposals`: event id, status, acknowledgment;
  - `members`; `pending_changes`; `epochs` and `epoch_deliveries`;
  - `outbox`: rows written now, published in stage 4;
  - `audit`.
- **Service key:** loaded from a file whose permissions must be `0600`, or from a systemd credential. The service refuses to start otherwise. Document an offline backup procedure. Never log it.
- **Read endpoints (NIP-98 authenticated):**
  - acceptances for a business after a given seq, with paging;
  - my epoch key deliveries;
  - my pending rejections;
  - pending owner-level changes.
- **Health and limits:** request body size limits, rate limiting per public key, and structured logs with no decrypted content.

### Required tests

Use a Postgres service container in CI. **Every one of these is a merge gate:**

- Two concurrent approvals of the same expense: exactly one is accepted.
- The same proposal sent twice: one acceptance, identical acknowledgment.
- **Rebuild:** replaying the chain's record changes reproduces the stored snapshot exactly.
- **Audit:** re-running every command with the recorded environment values reproduces the record changes.
- Tampering with, deleting or reordering any acceptance is detected by chain verification.
- A removed member is refused immediately; after rotation they can't decrypt new acceptances.
- An owner-level change activates after 48 hours (use an injected clock), and is cancelled by a veto from another owner or the recovery key.
- Commands that would leave no owner or no recovery key are rejected.
- From-join-date members can't decrypt anything before their join; full-history members can decrypt everything.
- A submitter can propose `SubmitReceiptDraft` only, and receives no epoch keys.
- NIP-98: an expired timestamp, the wrong URL or method, and a replayed auth event are all refused.
- A `baseSeq` conflict produces the "changed since you opened it" rejection.

## The browser

- **Sign-in with NIP-07** (browser extensions such as nos2x or Alby). NIP-46 remote signing follows in a later PR. Never ask for or store an nsec.
- **New `RemoteRepository`** implementing the existing `BusinessRepository` interface. It sends signed proposals, fetches acceptances, decrypts them with epoch keys, verifies the chain and the service signature, and applies record changes to the IndexedDB cache.
- **Offline:** proposals made offline wait in a local queue, are sent in order when the service is reachable, and show as "waiting to sync". Rejections are shown with their messages.
- **Business creation and member management UI** for owners: invite by npub with a role and history choice, remove, the pending owner-level changes with veto, and the recovery key setup at creation.
- **Local-only mode stays** for demos and the existing tests.
- **New dependency:** `nostr-tools`, pinned, for NIP-07, NIP-44 and NIP-98 in the browser.

## PR sequence

Do these as separate PRs, in order. **Stop after each one is open and wait for Seth's go-ahead** before starting the next.

1. **Protocol:** `docs/protocol.md` and `crates/protocol`. Event types, builders, validators, NIP-44 epoch encryption, chain verification, and the membership and permission rules as pure functions. No database, no network.
2. **Service core:** `crates/ledger-service` with Postgres, proposals, acceptances, rejections, idempotency, ordering, and the rebuild and audit tests.
3. **Membership and keys in the service:** epochs, deliveries, the 48-hour scheduler, vetoes, recovery, revocation, and their tests.
4. **Browser:** NIP-07 sign-in, `RemoteRepository`, the offline queue, and the member management UI, with e2e tests against a locally running service.
5. **Deployment:** a container image, a `docker-compose` file for local runs, and a deployment guide for a small cloud server: TLS, Postgres backups, key file handling and the offline key backup. No deployment is performed; Seth does that.

## Allowed dependencies

- **Rust:** `nostr` (rust-nostr, with the NIP-44 feature), `axum`, `tokio`, `tower-http`, `sqlx` (Postgres, runtime tokio, rustls), `serde`, `serde_json`, `thiserror`, `tracing`, `tracing-subscriber`, `rand`, `sha2`, `hex`.
- **Frontend:** `nostr-tools`.
- **Anything else:** stop and ask. **No hand-written cryptography:** signing, hashing and encryption come from these libraries only.

## Stop and ask if

- a `ledger-core` behavior change seems necessary. Parity rules still apply: TypeScript first, regenerate fixtures, then Rust;
- `today()` reaches a stored value on the service;
- a design choice would let the service, a relay or anyone else read business data without an epoch key, beyond what's described here;
- a chosen event kind turns out to be registered or in common use;
- any required test can't be made deterministic.

## Out of scope

- Relays, the outbox publisher, and multi-relay redundancy (stage 4).
- Blossom document storage, backups and restore (stage 3).
- NIP-46, the submitter UI, MLS, and maker-checker approval limits.
- Any AI work.

## Each PR's description must include

1. Results of every check, including the new service tests.
2. Which tests from "Required tests" it adds.
3. Any deviation from this brief, and why.
4. Confirmation that the privacy guard passed.
