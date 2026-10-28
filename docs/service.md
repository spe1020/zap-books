# Ledger service

`crates/ledger-service` is the signer of record for each business's books
([decision 0001](decisions/0001-nostr-security-model.md), the
[stage 2 brief](briefs/stage-2-accounting-service.md)). Members send it signed
proposals; it checks each one with `ledger-core`, and answers with a signed
acceptance (one link of the hash chain) or a signed rejection. The wire
format is [docs/protocol.md](protocol.md).

Stage 2 builds and tests it locally. Deployment is the last PR of the stage.

## Running it

```sh
# Once: a service key, readable by its owner only. Prints the public key.
cargo run -p ledger-service -- generate-key /secure/path/service.key

DATABASE_URL=postgres://… cargo run -p ledger-service -- \
  --key-file /secure/path/service.key \
  --public-url https://ledger.example \
  --listen 127.0.0.1:8787
```

- `--public-url` is the address clients use. NIP-98 auth events must name
  it exactly, so set it to what the TLS-terminating proxy in front of the
  service serves.
- The service creates and migrates its schema at start
  (`crates/ledger-service/migrations`).
- Logs are JSON lines on stdout. They carry business ids, seqs, command
  types and rejection codes, never decrypted content, rejection messages
  (which can quote business data) or keys. `RUST_LOG` sets the level.

## The service key

- **Loading.** From the systemd credential `service-key` when the service
  runs with `LoadCredential=service-key:/path/to/key`, or else from
  `--key-file`. A key file must have permissions exactly `0600`; with any
  other mode the service refuses to start and says how to fix it. The file
  holds a 64-character hex secret or an `nsec`.
- **Never logged.** No log line or error message contains it.
- **`generate-key <path>`** writes a new key to a file that must not exist
  yet, with mode `0600`, and prints only the public key.
- **`public-key <path>`** loads a key file with the same checks and prints
  its public key.

### Offline backup procedure

Do this once, when the key is created, and again whenever it is replaced.

1. Generate the key on the service machine with `generate-key`, or on an
   offline machine and copy it over. Record the printed public key: it is the
   key owners' genesis proposals declare.
2. Make **two** offline copies of the key file, each encrypted with a
   passphrase (for example `age -p service.key > service.key.age`), on two
   USB drives. Keep the passphrase separately from the drives (a password
   manager the owner controls, or on paper in a safe).
3. Store the drives in two different places, neither of them with the
   service's server or its database backups.
4. Test a restore: on an offline machine, decrypt one copy to a file with
   mode `0600`, run `ledger-service public-key <file>`, check that it prints
   the recorded public key, and delete the decrypted file.
5. Never put the key in the repository, a ticket, chat or an unencrypted
   backup.

If the key is lost, the service can't sign further acceptances, and the
books are frozen until owners declare a new service key. If it leaks,
whoever holds it can sign a different history. The hash chain and the
members' signed proposals expose that, and owners replace the key. Replacing
the service key is an owner-level change (48 hours, vetoable). It arrives
with membership in the service, PR 3.

## API

| Request                                             | Auth                | Answer                                                                                                                |
| --------------------------------------------------- | ------------------- | --------------------------------------------------------------------------------------------------------------------- |
| `POST /v1/proposals`, a proposal event as the body  | The proposal itself | 200 `{"status": "accepted" \| "rejected", "event": …}`; 400 if not a validly signed proposal; 413 over the body limit |
| `GET /v1/businesses/{id}/acceptances?after=&limit=` | NIP-98              | `{"head": seq, "acceptances": [...]}`, in seq order; 404 unless the caller reads this business                        |
| `GET /v1/epoch-keys?business=`                      | NIP-98              | `{"deliveries": [...]}`: the caller's epoch key deliveries                                                            |
| `GET /v1/rejections?after=`                         | NIP-98              | `{"cursor": n, "rejections": [...]}`: the caller's rejections, oldest first                                           |
| `GET /health`                                       | None                | `ok`                                                                                                                  |

- **Acknowledgments are idempotent.** The proposal's event id is the key:
  sending the same event again returns the stored answer byte for byte.
- **NIP-98.** `nostr`'s `verify_auth_header` checks the kind (27235), the
  URL (`--public-url` plus the request's path and query), the method, the
  signature, and the time (at most 60 seconds old, 30 seconds ahead). Each
  auth event is accepted once; its id is remembered for 120 seconds, longer
  than any event stays valid.
- **Limits.** The request body is at most 65,536 + 4,096 bytes (one event).
  Each public key gets a token bucket: 60 proposals at once and 10 a second
  after that, and 120 reads at once and 20 a second. The buckets are in
  memory, so they are per instance.

## One proposal, one transaction

Each proposal is handled in one database transaction:

1. Check the signature and tags (no transaction yet). A proposal that fails
   here gets a 400 and no signed answer, since it may not be from who it
   claims.
2. Return the stored acknowledgment if this event was seen before.
3. Take the business's lock (`pg_advisory_xact_lock` on its id, which also
   covers genesis, plus `select … for update` on its row). Proposals for one
   business are handled one at a time; different businesses in parallel.
4. Check again for a stored acknowledgment (a concurrent copy may have
   just finished).
5. Decrypt and check the payload commitment; check permissions; check
   `baseSeq` against the record versions; apply the command with
   `protocol::apply_command` and the service's environment; compute the
   record changes; build and sign the acceptance under the current epoch
   key.
6. Store the acceptance, the new snapshot, the record versions, the
   acknowledgment and an outbox row, and commit. All or nothing.

Any rejection in step 5 is signed, stored as the acknowledgment and in the
`audit` table, and committed. A rejection stores no acceptance, seq, snapshot
or record version. That includes `tooLarge`, which carries the size the
acceptance would have been.

Genesis does the same, creating the business row, epoch 0 (its secret NIP-44
encrypted from the service key to itself), the members and one epoch key
delivery per member who reads.

## Storage

Minimal, to normalize later. Event JSON is stored as text, exactly as
signed, so it is served byte for byte.

| Table                        | Holds                                                                                                                    |
| ---------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| `businesses`                 | Per business: head seq and event id, current epoch, snapshot (`BusinessData`) and membership (`MembershipState`) as JSON |
| `acceptances`                | The chain, append-only, one signed event per seq                                                                         |
| `proposals`                  | Each proposal's acknowledgment, by event id                                                                              |
| `record_versions`            | The seq that last changed each record, for `baseSeq`                                                                     |
| `members`                    | Members and their roles                                                                                                  |
| `pending_changes`            | Owner-level changes waiting out 48 hours (PR 3)                                                                          |
| `epochs`, `epoch_deliveries` | Epoch keys (secrets sealed to the service key) and their deliveries                                                      |
| `outbox`                     | Every acceptance and delivery, to publish to relays in stage 4                                                           |
| `audit`                      | Signed rejections                                                                                                        |
| `nip98_seen`                 | Used NIP-98 auth events                                                                                                  |

## `today()` on the service

`ledger-core`'s `Env::today()` is the user's local date in the browser. On
the service it is the UTC date of the acceptance's `now`. The brief asks
whether it ever reaches a stored value. It doesn't:

- **The only readers of `today()`** in `ledger-core` are `blank_draft`
  (the new draft's `date`) and `current_period` (the `YYYY-MM` reports
  default to).
- **`current_period`** serves queries only. No command calls it, and the
  service runs no queries.
- **`blank_draft`** is called by one command the service runs:
  `commit_import`, for each new row. It then sets the draft's `date` to the
  row's date before the draft is written, on every path that writes one.
  The blank draft's `createdAt` comes from `now()`, not `today()`.
- **None of the others** (`save_draft`, `approve`, `correct`,
  `update_settings`, `save_rule`, nor the protocol's document and receipt
  reading steps) read `today()`.

`today_never_reaches_the_books` (`crates/protocol/tests/apply.rs`) runs every
command type under two environments that differ only in `today()`, ten years
apart, and requires identical books.

## Not yet

- **Membership changes, epochs after 0, the 48-hour scheduler, vetoes,
  recovery and revocation** come in PR 3. Until then `ChangeMembership` and
  `Veto` are rejected ("Membership changes are not available yet."), so the
  members are the genesis members.
- **Pending owner-level changes** have a table but no endpoint yet (PR 3).
- **The outbox** is written but not published (stage 4).
