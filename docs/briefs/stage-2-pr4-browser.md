# Brief: Stage 2 PR 4, the browser connected to the service

**For:** Claude Code
**Repo:** `spe1020/zap-books`
**Save this brief as:** `docs/briefs/stage-2-pr4-browser.md` (Prettier only)
**Read first:** decisions `0001`–`0004`, `docs/protocol.md`, `docs/service.md`, the stage 2 brief, and `PARITY_NOTES.md`.

**Privacy rule:** invented values only. The guard must pass on every commit and PR description.

## Goal

Seth can sign in with his Nostr key in a desktop browser, create a real business on the service with a recovery key, record expenses on it, and see the same verified books from a second browser. The local-only demo mode keeps working unchanged.

## Principles

1. **One implementation of the security rules.** The browser verifies the chain, opens acceptances, checks payload commitments and follows membership by running `crates/protocol` compiled to WebAssembly. It is never reimplemented in TypeScript.
2. **No secret key is ever entered into or stored by the app.** Member keys stay in a NIP-07 signer extension. Epoch keys live only in memory for the session.
3. **The browser trusts nothing it hasn't verified.** Every acceptance it applies has been chain-checked, signature-checked, opened against its signer and business, and followed through the membership rules. Anything that fails stops sync for that business and shows an error. It is never skipped.
4. **The service stays the only authority.** The browser shows what's been accepted. Proposals that haven't been accepted yet are visibly marked as waiting.

## Architecture

### `crates/protocol-wasm` (new)

- WebAssembly bindings over `crates/protocol` and the parts of `ledger-core` the client needs. Export only what the browser uses:
  - **build a proposal:** returns the plaintext payload (with its salt) and the tags, including the payload commitment, ready to be encrypted and signed by the extension;
  - **check a signed proposal** before sending it;
  - **verify a chain segment** from a stored head;
  - **open an acceptance**, given the epoch secret key, the expected signer and the business;
  - **follow membership;**
  - **apply record changes** to a business snapshot;
  - **open epoch key deliveries** (with the member's NIP-44 decrypt done by the extension; see below);
  - **generate a recovery keypair** (setup only; see Business creation).
- Build it with `wasm-pack`, and check the output into neither git nor the npm package lockfile. CI builds it.
- Add a parity-style test: the same chain fixtures verified by Rust natively and through the WASM build give identical results.

### Encryption and signing split

- **Proposals:** WASM builds the plaintext and tags, the extension encrypts the plaintext to the service key (`window.nostr.nip44.encrypt`) and signs the event (`signEvent`), then WASM checks the signed result before it's sent.
- **Epoch key deliveries:** the extension decrypts each delivery (`window.nostr.nip44.decrypt`), giving the epoch secret key, which WASM then uses to open acceptances. Keep epoch keys in memory only, and fetch and decrypt deliveries again at the start of each session.
- **If the extension lacks NIP-44 support,** say so clearly and stop. Don't fall back to NIP-04.

### Sessions instead of a signature per read (small service change)

NIP-98 needs a signature for every request, which would mean constant extension prompts. Add to the service:

- `POST /v1/session` authenticated with NIP-98, returning a random 32-byte token and its expiry (1 hour). Store only a hash of the token.
- Reads accept `Authorization: Bearer <token>` as an alternative to NIP-98. **Membership is still checked on every request,** so removal still cuts a reader off immediately.
- Tokens are held in memory only, never in browser storage. Expired or unknown tokens get 401, and the client signs in again.
- Proposals need no session: they're signed events already.
- Tests: an expired token, an unknown token, a token used after its member was removed, and a token for one key reading another member's deliveries are all refused.

### `LedgerClient` (refactor)

- Today the app applies domain functions and commits a snapshot. Introduce a `LedgerClient` interface with one method per command (`saveDraft`, `approve`, `correct`, `commitImport`, `updateSettings`, `saveRule`, `changeMembership`, `veto`) plus reads.
- **`LocalLedgerClient`** wraps today's behavior exactly. All existing e2e tests must pass unchanged against it.
- **`RemoteLedgerClient`** turns each command into a proposal, and updates the view only from verified acceptances.
- The TypeScript domain functions still run in the browser **before** sending, to show validation messages early. The service decides.

### Sync

- On opening a business: verify the full chain from genesis the first time, then store the verified head (seq, event id, epoch, service key) and the snapshot in IndexedDB. After that, fetch and verify only what comes after the stored head.
- Poll every 15 seconds while the window is focused, and immediately on focus and after each send. No websockets.
- **A stored head the service no longer extends** (a fork, or a rewritten chain) is a hard error: stop sync for that business and show it. Never overwrite the stored head silently.
- Large imports use the protocol's size-based batching, with a progress display ("batch 3 of 8") and resume after a failure.

### Offline queue

- Commands made while offline are built, signed and stored in an IndexedDB queue, in order, and shown as "waiting to sync".
- When the service is reachable, send them in order, one at a time.
- A rejection (including a `baseSeq` conflict) stops the queue at that item and shows its message with options to discard it or redo it from the current books. Later items stay queued.
- Retries are safe: a resent proposal returns its original answer.

### Service key pinning

- The service's public key is part of the app's configuration (`VITE_SERVICE_PUBKEY`, with the service URL in `VITE_SERVICE_URL`).
- Genesis names that key, and the browser refuses to create a business if the service reports a different one.
- After that, the browser follows the chain's own service-key hand-overs.
- Update the Content-Security-Policy's `connect-src` to allow exactly the configured service origin.

### Receipts in this stage

- Files stay in the browser where they were attached. Compute `sha256` when attaching, and send only the document id and hash in proposals.
- On a browser that doesn't have the file, the Review screen says "This receipt was attached on another device" and shows its hash. Stage 3 adds encrypted storage.

## Screens

### Sign-in

- "Sign in with your Nostr key" using NIP-07. Show the npub, and which extension is in use.
- If no extension is found, explain what one is and link to a few common ones. Don't recommend a specific one.
- "Local demo" stays available without signing in.

### Business creation

1. Business name, reporting currency, and the starting accounts and categories. No demo data.
2. **Recovery setup is required before creating:**
   - **a second owner:** enter their npub;
   - **or an offline backup key:** the app generates a keypair, shows the private key **once**, in a large printable format, and requires the user to confirm the last six characters. It's never stored, logged or sent anywhere. Only the public key goes into genesis.
3. Show this advice, not hidden behind a link: "Your backup key can take over this business after 48 hours if it's ever misused. Keep it as safe as your own key, offline. If you can, add a second owner too."
4. Confirm the service key fingerprint, then create.

### Members (owners)

- A list of members with their role, history choice and join date.
- **Invite:** npub, a role (owner, bookkeeper, viewer, assistant; **don't offer submitter yet**), and a history choice, defaulting to full history.
- **Adding an assistant** shows: "An AI assistant can read the whole of this business's books, within the history you choose. If it runs on someone else's computer or service, that data goes there. It can create drafts and suggestions, but it can never approve, correct or import."
- **Remove** a member, and change a member's role.
- Drafts created by an assistant are marked as such in the inbox and on the Review screen.
- Owner-level changes say "takes effect in 48 hours unless vetoed" before they're sent.

### Pending changes

- Visible to owners and the recovery key, with a countdown to each change's effective time and who proposed it.
- **Veto** for each change, except the viewer's own proposals, and a recovery that names the viewer as the lost owner.

### After a recovery

- When a recovery activates, show the new owner a banner that stays until dismissed: "Your business was recovered. Review the members added since [date the recovery was proposed]." Below it, list exactly those members, with a remove action for each.

### Recovery mode

- A screen for someone signed in **with the recovery key** (loaded into a signer extension, ideally in a separate browser profile). The app never asks for the key itself.
- It shows the pending changes with veto, and **Recover an owner**: choose the lost owner, enter the replacement npub, and confirm the 48-hour wait.
- Explain once at the top: "You're signed in with this business's backup key. It can only veto changes and recover a lost owner. It can't read the books."

### Sync status

- A small persistent indicator: synced, syncing, offline (with the number waiting), or an error (with its message).

## Required tests

**Every one of these is a merge gate.** E2e tests run against a locally started service and Postgres, with a **test-only** NIP-07 signer injected into the page using invented keys. The test signer must not be reachable outside the test build.

- Sign in, create a business with an offline backup key, record and approve an expense, and see it from a second browser context signed in as a second member.
- A tampered acceptance from the service (served by a test proxy) stops sync with an error and leaves the stored books unchanged.
- A rewritten chain (a different event at a seq already stored) is detected as a fork.
- Offline: queue two commands, reconnect, and both are accepted in order. A conflicting queued command stops the queue with its message.
- A removed member's session is refused immediately, and after rotation the member can't open new acceptances.
- A from-join-date member sees nothing from before their join.
- An owner-level change shows its countdown, a veto by another owner cancels it, and the lost owner can't veto a recovery.
- After a recovery activates, the banner lists exactly the members added since the recovery was proposed.
- An assistant member can't approve: the UI hides the action, and a crafted approval is rejected by the service.
- A large import completes in batches, and resumes after an interruption.
- The service-key pin: creation is refused if the service reports a different key.
- The backup key shown at creation never appears in storage, logs, network requests or the DOM after confirmation.
- **All existing e2e tests pass unchanged against `LocalLedgerClient`.**

## PR sequence

Do these as separate PRs, in order. **Stop after each one is open and wait for Seth's go-ahead.**

1. **4a. Foundations:** `crates/protocol-wasm` and its parity test, the `LedgerClient` refactor with `LocalLedgerClient` (no behavior change), and the service's session endpoint with its tests.
2. **4b. Remote books:** sign-in, business creation with recovery setup and the service-key pin, `RemoteLedgerClient`, verified sync, the offline queue, batched imports, and sync status.
3. **4c. People:** members, invites (including the assistant warning), removal and role changes, pending changes with vetoes, recovery mode, and the after-recovery review.

## Allowed dependencies

- **Rust:** `wasm-bindgen`, `serde-wasm-bindgen`, `getrandom` with its browser feature (needed for randomness in WASM), and `js-sys` if required. Plus everything already allowed.
- **Tooling:** `wasm-pack` in CI and locally.
- **Frontend:** `nostr-tools`, pinned, for NIP-07 types, NIP-19 (npub) encoding and NIP-98.
- **Anything else:** stop and ask. No hand-written cryptography; NIP-44, Schnorr and SHA-256 come from the `nostr` crate (through WASM) or the signer extension.

## Stop and ask if

- the `nostr` crate or `crates/protocol` can't be built for WASM without changes to the protocol's behavior;
- a common NIP-07 extension can't do NIP-44, in a way that blocks the design;
- any screen would need a secret key typed into the app;
- an existing e2e test needs to change to pass against `LocalLedgerClient`;
- a protocol or `ledger-core` behavior change seems needed. Parity rules apply.

## Out of scope

- Phone sign-in (NIP-46), relays, encrypted receipt storage and backups (stage 3), deployment (PR 5).
- The submitter UI, showing suggestions, the MCP server, and remote models.
- Moving existing local demo data onto the service.

## Each PR description must include

1. Results of every check, including the WASM parity test and the e2e gates the PR adds.
2. Screenshots of every new screen, using invented data.
3. Any deviation from this brief, and why.
4. Confirmation that the privacy guard passed on the commits and the description.
