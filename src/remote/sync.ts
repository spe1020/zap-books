/**
 * Verified sync of one business's books. The first time, the whole chain is
 * verified from genesis; after that, only what follows the stored head. Each
 * acceptance is chain-checked, opened with its epoch key against the key
 * the chain says signs it, followed through the membership rules, and its
 * record changes applied, all in `crates/protocol` (WebAssembly). Nothing is
 * stored unless every acceptance fetched verifies.
 *
 * A stored head the service no longer extends (a fork or a rewritten chain)
 * stops sync for that business. The stored head is never overwritten.
 */
import type { BusinessData } from "../domain/types";
import { ServiceError, type ServiceApi } from "./api";
import {
  protocol,
  type Head,
  type Membership,
  type NostrEvent,
} from "./protocol";
import type { Signer } from "./signer";
import { store, type StoredBooks } from "./store";

/** Sync stopped for a business; its stored books are unchanged. */
export class SyncStopped extends Error {}

const PAGE = 500;

/** Epoch keys, per business and epoch: memory only, fetched each session. */
type Keys = Map<string, Map<number, string>>;

export class Sync {
  private keys: Keys = new Map();
  /** Businesses whose sync has stopped, with why. */
  readonly stopped = new Map<string, string>();
  constructor(
    private readonly api: ServiceApi,
    private readonly signer: Signer,
    /** The configured service key, which genesis must name. */
    private readonly pinned: string,
  ) {}

  /** Keys a delivery may be signed by: the pinned key, or the business's own. */
  private trusted(event: NostrEvent, books?: StoredBooks) {
    return (
      event.pubkey === this.pinned ||
      event.pubkey === books?.head.service ||
      event.pubkey === books?.membership.service
    );
  }

  /**
   * Fetches and opens the epoch keys delivered to this member, for one
   * business or all of them. The extension decrypts each one. Returns the
   * businesses they are for.
   */
  async fetchKeys(business?: string): Promise<string[]> {
    const query = business ? `?business=${business}` : "";
    let deliveries: NostrEvent[];
    try {
      ({ deliveries } = await this.api.read<{ deliveries: NostrEvent[] }>(
        `/v1/epoch-keys${query}`,
      ));
    } catch (e) {
      // A key with no standing anywhere yet has no session and no keys.
      if (e instanceof ServiceError && e.status === 403) return [];
      throw e;
    }
    const found = new Set<string>();
    for (const event of deliveries) {
      const tagged = event.tags.find((t) => t[0] === "b")?.[1] ?? "";
      const books = /^[0-9a-f]{64}$/.test(tagged)
        ? await store.books(this.signer.pubkey, tagged)
        : undefined;
      if (!this.trusted(event, books)) continue;
      const service = event.pubkey;
      const member = this.signer.pubkey;
      const { business: b, epoch } = protocol.parseDelivery({
        event,
        member,
        service,
      });
      const known = this.keys.get(b)?.get(epoch);
      if (!known) {
        const plaintext = await this.signer.decrypt(service, event.content);
        const opened = protocol.openDelivery({
          event,
          member,
          service,
          plaintext,
        });
        if (!this.keys.has(b)) this.keys.set(b, new Map());
        this.keys.get(b)!.set(opened.epoch, opened.secret);
      }
      found.add(b);
    }
    return [...found];
  }

  private async secret(business: string, epoch: number): Promise<string> {
    let secret = this.keys.get(business)?.get(epoch);
    if (!secret) {
      await this.fetchKeys(business);
      secret = this.keys.get(business)?.get(epoch);
    }
    if (!secret)
      throw new SyncStopped(
        `There's no key here for epoch ${epoch} of this business's books, so they can't be opened.`,
      );
    return secret;
  }

  private async page(business: string, after?: number) {
    const query = new URLSearchParams({ limit: String(PAGE) });
    if (after !== undefined) query.set("after", String(after));
    try {
      return await this.api.read<{ head: number; acceptances: NostrEvent[] }>(
        `/v1/businesses/${business}/acceptances?${query}`,
      );
    } catch (e) {
      if (e instanceof ServiceError && e.status === 404)
        throw new SyncStopped(
          "You can't read this business's books: it doesn't exist, or you're no longer a member.",
        );
      throw e;
    }
  }

  /** Everything after `after` (or from genesis), page by page. */
  private async fetchAfter(business: string, after?: number) {
    const events: NostrEvent[] = [];
    let cursor = after;
    for (;;) {
      const page = await this.page(business, cursor);
      events.push(...page.acceptances);
      const last = events.length ? seqOf(events[events.length - 1]) : cursor;
      if (!page.acceptances.length || last === undefined || last >= page.head)
        return events;
      cursor = last;
    }
  }

  /**
   * Brings one business's stored books up to the service's head, verifying
   * everything new. Throws `SyncStopped` if anything fails to verify, and
   * remembers it: sync stays stopped for the business until reload.
   */
  async sync(business: string): Promise<StoredBooks> {
    const stopped = this.stopped.get(business);
    if (stopped) throw new SyncStopped(stopped);
    try {
      return await this.syncOnce(business);
    } catch (e) {
      if (e instanceof SyncStopped) this.stopped.set(business, e.message);
      throw e;
    }
  }

  private async syncOnce(business: string): Promise<StoredBooks> {
    const owner = this.signer.pubkey;
    const stored = await store.books(owner, business);
    let events: NostrEvent[];
    if (stored) {
      // Fetch from the stored head itself, which must still be there.
      const from = stored.head.seq === 0 ? undefined : stored.head.seq - 1;
      const fetched = await this.fetchAfter(business, from);
      if (fetched[0]?.id !== stored.head.id)
        throw new SyncStopped(
          `The service's history no longer matches what this browser verified at seq ${stored.head.seq}: the chain was forked or rewritten. Sync has stopped for this business.`,
        );
      events = fetched.slice(1);
    } else events = await this.fetchAfter(business);
    if (!events.length) {
      if (stored) return stored;
      throw new SyncStopped("This business has no books yet.");
    }
    let head: Head | undefined = stored?.head;
    let snapshot: BusinessData | undefined = stored?.snapshot;
    let membership: Membership | undefined = stored?.membership;
    for (const event of events) {
      try {
        const signer = head?.service ?? this.pinned;
        const next = protocol.verifyChain({
          business,
          service: this.pinned,
          after: head,
          acceptances: [event],
        })!;
        const epochSecret = await this.secret(business, next.epoch);
        const opened = protocol.openAcceptance({
          event,
          epoch: next.epoch,
          epochSecret,
          service: signer,
          business,
        });
        membership = protocol.followMembership({
          state: membership,
          acceptance: opened,
        });
        snapshot = protocol.applyRecordChanges({
          snapshot,
          acceptance: opened,
        });
        head = next;
      } catch (e) {
        if (e instanceof SyncStopped) throw e;
        throw new SyncStopped(
          `An acceptance from the service didn't verify (${(e as Error).message}) Sync has stopped for this business, and its books here are unchanged.`,
        );
      }
    }
    const books: StoredBooks = {
      owner,
      business,
      head: head!,
      snapshot: snapshot!,
      membership: membership!,
    };
    await store.saveBooks(books);
    return books;
  }
}

/** An acceptance's `seq` tag, for paging only; verification checks it. */
function seqOf(event: NostrEvent): number | undefined {
  const value = event.tags.find((t) => t[0] === "seq")?.[1];
  return value === undefined ? undefined : Number(value);
}
