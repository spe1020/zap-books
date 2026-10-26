# Decision 0001: Nostr security model

**Status:** Agreed in principle (2026-10-25). Details to be settled in the stage 2 brief.
**Save as:** `docs/decisions/0001-nostr-security-model.md`

## Context

Zap Books uses Nostr as its integration layer: identity, membership, event exchange and receipt storage. Accounting still needs one authoritative ordering of postings, so two devices can never both decide what the books say. The concept plan already assigns that job to an accounting service, with the relay distributing accepted changes.

This record captures the security decisions agreed before stage 2 begins.

## Three kinds of keys

| Key | Purpose | Held by |
|---|---|---|
| Member identity key (nsec) | Signs proposals; logs in to relays | Each person |
| Business data key | Encrypts amounts, vendors, receipts. Rotates in numbered **epochs** | Members allowed to read the books, and the ledger service |
| Service key | Signs accepted postings: the signer of record | The ledger service only |

A Nostr identity key cannot be rotated: the npub is the identity. Compromise is handled by **revoking** an npub and adding a replacement, never by changing the key.

## Decision 1: Key management (option C)

- **Relays enforce cutoff.** NIP-42 login is required to read and write. Once a membership change is signed, every relay rejects the removed npub.
- **The ledger service manages key epochs.** It already decrypts proposals to validate them, so this adds no new party with access to plaintext. On a membership change it generates the next epoch key, encrypts a copy to each remaining member (NIP-44), and signs the announcement.
- **Relays never hold a business data key.**
- **Owner changes need a recovery key and a waiting period.** A second owner, or a backup key kept offline, is required. Owner-level changes take effect after a waiting period (proposed 48 hours), during which any other owner or recovery key can veto. In effect this is a time-locked multisig on membership.
- **Postings form a hash chain.** Each acceptance signed by the service includes the hash of the previous one, so forged, altered or withheld history is detectable.
- **The owner signs a service-key declaration,** naming the current service key, so a compromised service key can be replaced.

Known limit: a removed member keeps anything they already received. Rotation protects only the future. That is true of every scheme, including MLS.

### Compromise playbook

| What's compromised | Response |
|---|---|
| Member nsec | Owner revokes the npub and adds the new one. Relays cut off the old key; the service rotates the epoch. Postings signed by the old key after the reported compromise time are flagged for review. Encourage NIP-46 or hardware signers. |
| Owner nsec | Recovery key plus waiting period lets another owner veto the attacker's changes, then revoke the key. |
| Service key | Hash-chain breaks expose forged postings. The owner declares a new service key. Keep the service key on a separate, locked-down machine. |
| A relay | Attacker sees metadata only: no content, no forging. Withholding is detected by sequence gaps and repaired from another relay. |

## Decision 2: Relays (option B, plus redundancy)

- **A dedicated Zap Books relay setup,** separate from member-relay and recipe traffic. It's an existing relay implementation with a policy plugin.
- **The plugin must gate reads as well as writes:** members see only their own businesses' events. Verify read-side filtering when choosing the implementation; many plugins only gate writes.
- **Redundancy:** every event is published to **at least two relays, run on different providers or regions.** A business may add a relay of its own as a further copy. See "Multiple relays" below.
- The event format must not depend on any one relay, keeping "bring your own relay" possible later.

### Multiple relays

**Benefits**

- **Availability:** if one relay is down, clients and the service keep working from the others.
- **Recoverability:** every acceptance is signed and hash-chained, so any relay holding a full copy is a *verifiable backup*. If the service's database is lost, the books can be rebuilt by replaying the chain from any relay, and the chain proves nothing was altered or dropped. This directly supports stage 3 (backup and recovery).
- **Withholding detection:** comparing relays against the hash chain exposes a relay that drops events, and the gap is filled from another.
- **No single operator:** no one relay operator, including us, can make a business's history disappear.

**Costs**

- **Every relay sees metadata** (who posts, when, how often). More relays means more parties seeing it, so each must gate reads.
- **The weakest relay sets the privacy bar.** Access policy must be enforced the same way on every relay.
- **Revocation must reach every relay.** A slow relay could still serve a removed member new events, but those are encrypted under the new epoch they can't decrypt, so the exposure is metadata only.

**Rules**

- The service's database stays authoritative. Relays are replicas.
- The outbox publishes to every configured relay and retries each one independently. A posting counts as **published** once at least two relays confirm storing it.
- Clients read from all configured relays, deduplicate by event id, and use the hash chain and sequence numbers to detect gaps.

## Decision 3: Roles

Encryption is the real read control: anyone holding the business key can read everything. So each role has two parts, **key access** (reading) and **what the service accepts** (writing).

| Role | Gets business key | Service accepts | v1? |
|---|---|---|---|
| Owner | Yes | Everything, including membership, keys, settings | Yes |
| Bookkeeper | Yes | Drafts, imports, approvals, corrections | Yes |
| Viewer | Yes | Nothing (read only) | Yes |
| Submitter | **No** | New drafts and receipts only, encrypted to the service | Designed now, shipped later |

The submitter is write-only by cryptography: they can send receipts in but can never read the books. A later option: approvals above a set amount require someone other than the drafter (maker-checker).

## Open items for the stage 2 brief

- Length of the owner waiting period, and the form of the recovery key.
- Relay implementation choice, confirmed to support read-side filtering.
- Epoch key format, the announcement event, and how a new member receives past epochs (all of them, or only from their join date).
- Event kinds and the wire envelope for proposals, acceptances, membership and key announcements.
- Number and placement of relays, and the publish confirmation rule.
- How the service's key is stored and backed up.
- Whether membership uses NIP-29 group semantics or a custom owner-signed list.

## Options considered and set aside

- **Relay as key custodian:** simplest, but the relay could read every business's books.
- **Owner's device rotates keys:** the relay stays blind, but rotation waits for the owner to be online, and losing the owner key strands the business.
- **MLS group encryption:** strongest (adds forward secrecy) but complex, and Nostr implementations are young. The design should allow moving to it later.
- **Reusing member-relay:** shares failure modes and access rules with unrelated traffic.
