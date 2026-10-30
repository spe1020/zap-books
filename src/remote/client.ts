/**
 * `LedgerClient` for books on the ledger service. Each command runs through
 * the TypeScript domain functions first, so validation messages appear at
 * once, then becomes a proposal: built and checked by `crates/protocol`,
 * encrypted and signed by the signer extension, and stored in the queue.
 * The queue sends proposals one at a time, in order. The books shown change
 * only through verified sync: the service decides.
 */
import type { BusinessData, Workspace } from "../domain/types";
import { CommandClient, type LedgerCommand } from "../adapters/ledger-client";
import { applyLocal } from "../adapters/local-ledger";
import { Offline, ServiceError, type Acknowledgment, ServiceApi } from "./api";
import {
  describe,
  protocolCommand,
  withHashes,
  withoutFiles,
} from "./commands";
import { protocol, type NostrEvent } from "./protocol";
import type { Signer } from "./signer";
import { store, type QueueItem, type StoredBooks } from "./store";
import { Sync, SyncStopped } from "./sync";

/** Saved and signed, but not yet accepted: it waits in the queue. */
export class Waiting extends Error {
  name = "Waiting";
}

export type SyncState =
  | { status: "synced" }
  | { status: "syncing" }
  | { status: "offline"; waiting: number }
  | { status: "error"; message: string };

export interface RemoteView {
  state: SyncState;
  queue: QueueItem[];
  /** The import being sent in batches, if any. */
  importing?: { batch: number; of: number };
}

const now = () => Math.floor(Date.now() / 1000);

export class RemoteLedgerClient extends CommandClient {
  readonly sync: Sync;
  private books = new Map<string, StoredBooks>();
  private receipts = new Map<string, Blob>();
  private queue: QueueItem[] = [];
  private state: SyncState = { status: "syncing" };
  private listeners = new Set<() => void>();
  private flushing?: Promise<void>;
  private syncing = new Map<string, Promise<StoredBooks>>();
  /** Actions whose caller is waiting for the answer. */
  private waiting = new Set<string>();
  private refused = new Map<string, string>();
  private batches = new Map<string, number>();

  constructor(
    readonly api: ServiceApi,
    readonly signer: Signer,
    readonly pinned: string,
  ) {
    super();
    this.sync = new Sync(api, signer, pinned);
  }

  // The view

  subscribe(listener: () => void) {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }
  private changed() {
    for (const l of this.listeners) l();
  }
  view(): RemoteView {
    const imports = this.queue.filter((q) => q.importId);
    const first = imports[0];
    return {
      state: this.state,
      queue: [...this.queue],
      importing: first
        ? {
            batch: (this.batches.get(first.importId!) ?? 0) + 1,
            of:
              (this.batches.get(first.importId!) ?? 0) +
              imports.filter((q) => q.importId === first.importId).length,
          }
        : undefined,
    };
  }
  private setState(state: SyncState) {
    this.state = state;
    this.changed();
  }
  /** Why sync stopped for a business, if it did. */
  stoppedFor(business: string) {
    return this.sync.stopped.get(business);
  }
  membership(business: string) {
    return this.books.get(business)?.membership;
  }

  workspace(): Workspace {
    return {
      schemaVersion: 1,
      revision: 0,
      businesses: [...this.books.values()].map((b) =>
        this.withReceipts(b.snapshot),
      ),
    };
  }

  /** The books with the receipt files this browser holds. */
  private withReceipts(b: BusinessData): BusinessData {
    return {
      ...b,
      documents: b.documents.map((d) => {
        const blob = this.receipts.get(`${b.profile.id}:${d.id}`);
        return blob ? { ...d, blob } : d;
      }),
    };
  }

  private async loadReceipts(b: BusinessData) {
    for (const d of b.documents) {
      const key = `${b.profile.id}:${d.id}`;
      if (this.receipts.has(key)) continue;
      const blob = await store.receipt(b.profile.id, d.id);
      if (blob) this.receipts.set(key, blob);
    }
  }

  // Sync

  /** Syncs one business; one sync at a time per business. */
  async syncBusiness(business: string): Promise<StoredBooks> {
    const running = this.syncing.get(business);
    if (running) return running;
    const work = this.sync
      .sync(business)
      .then(async (books) => {
        await this.loadReceipts(books.snapshot);
        this.books.set(business, books);
        return books;
      })
      .finally(() => this.syncing.delete(business));
    this.syncing.set(business, work);
    return work;
  }

  /** Every business this key reads or created here, verified. */
  async load(): Promise<Workspace> {
    await this.refresh(true);
    return this.workspace();
  }

  /**
   * Fetches new deliveries (with `discover`), syncs every known business,
   * and sends what's queued. Errors land in the sync state.
   */
  async refresh(discover = false) {
    this.setState({ status: "syncing" });
    try {
      const owner = this.signer.pubkey;
      this.queue = await store.queue(owner);
      // What this browser verified before is shown whatever happens next.
      for (const b of await store.known(owner)) {
        const cached = await store.books(owner, b);
        if (cached && !this.books.has(b)) {
          await this.loadReceipts(cached.snapshot);
          this.books.set(b, cached);
        }
      }
      if (discover) await store.addKnown(owner, await this.sync.fetchKeys());
      for (const b of await store.known(owner)) {
        try {
          await this.syncBusiness(b);
        } catch (e) {
          if (!(e instanceof SyncStopped)) throw e;
        }
      }
      await this.flush();
      this.settle();
    } catch (e) {
      this.fail(e);
    }
  }

  private settle() {
    const stopped = [...this.sync.stopped.values()][0];
    const rejected = this.queue.find((q) => q.rejected);
    if (stopped) this.setState({ status: "error", message: stopped });
    else if (rejected)
      this.setState({ status: "error", message: rejected.rejected! });
    else if (this.state.status === "offline" && this.queue.length)
      this.changed();
    else this.setState({ status: "synced" });
  }

  private fail(e: unknown) {
    if (e instanceof Offline)
      this.setState({ status: "offline", waiting: this.queue.length });
    else this.setState({ status: "error", message: (e as Error).message });
  }

  // Commands

  async run(businessId: string, commands: LedgerCommand[]): Promise<Workspace> {
    const books = this.books.get(businessId);
    if (!books) throw new Error("Business not found.");
    const stopped = this.sync.stopped.get(businessId);
    if (stopped) throw new Error(stopped);
    // The domain rules first: the same messages as the local demo.
    commands.reduce(applyLocal, this.withReceipts(books.snapshot));
    const action = crypto.randomUUID();
    const items: QueueItem[] = [];
    const ahead = this.queue.filter((q) => q.business === businessId).length;
    let order = (this.queue.at(-1)?.order ?? 0) + 1;
    let state = this.withReceipts(books.snapshot);
    for (const [i, raw] of commands.entries()) {
      const command =
        raw.type === "SaveDraft" || raw.type === "Correct"
          ? { ...raw, documents: await withHashes(raw.documents) }
          : raw;
      if (command.type === "SaveDraft" || command.type === "Correct")
        await store.saveReceipts(
          businessId,
          command.documents.flatMap((d) =>
            d.blob ? [{ id: d.id, blob: d.blob }] : [],
          ),
        );
      for (const part of this.batched(command)) {
        // Each proposal names the books it was made against: the head, plus
        // the proposals ahead of it, which should be accepted first.
        const baseSeq =
          part.type === "CommitImport"
            ? books.head.seq
            : books.head.seq + ahead + i;
        items.push(
          await this.build(businessId, books, state, part, baseSeq, {
            order: order++,
            action,
            importId: command.type === "CommitImport" ? action : undefined,
          }),
        );
      }
      state = applyLocal(state, command);
    }
    await store.enqueue(items);
    this.queue = await store.queue(this.signer.pubkey);
    this.waiting.add(action);
    this.changed();
    try {
      await this.flushAndSettle();
      const refused = this.refused.get(action);
      if (refused) throw new Error(refused);
      if (this.queue.some((q) => q.action === action)) {
        const blocked = this.queue.find((q) => q.rejected);
        throw new Waiting(
          blocked
            ? "Saved here. It will be sent after the change the service refused is discarded or redone (see Sync status)."
            : "Saved here. It will be sent when the service can be reached.",
        );
      }
      return this.workspace();
    } finally {
      this.waiting.delete(action);
      this.refused.delete(action);
    }
  }

  /** An import goes as size-limited batches; anything else as itself. */
  private batched(command: LedgerCommand): LedgerCommand[] {
    if (command.type !== "CommitImport") return [command];
    const size = protocol.limits().importBatchStart;
    const parts: LedgerCommand[] = [];
    for (let i = 0; i < command.rows.length; i += size)
      parts.push({ ...command, rows: command.rows.slice(i, i + size) });
    return parts.length ? parts : [command];
  }

  /** Builds, signs and checks one proposal. */
  private async build(
    business: string,
    books: StoredBooks,
    state: BusinessData,
    command: LedgerCommand,
    baseSeq: number,
    meta: Pick<QueueItem, "order" | "action" | "importId">,
  ): Promise<QueueItem> {
    const service = books.head.service;
    const parts = protocol.buildProposal({
      business,
      baseSeq,
      command: protocolCommand(state, command),
    });
    const event = await this.sign(service, parts);
    return {
      ...meta,
      owner: this.signer.pubkey,
      business,
      label: describe(command),
      command: withoutFiles(command),
      event,
      plaintext: parts.plaintext,
      baseSeq,
    };
  }

  /** The extension encrypts the payload to the service and signs. */
  private async sign(
    service: string,
    parts: { kind: number; plaintext: string; tags: string[][] },
  ): Promise<NostrEvent> {
    const content = await this.signer.encrypt(service, parts.plaintext);
    const event = await this.signer.sign({
      kind: parts.kind,
      created_at: now(),
      tags: parts.tags,
      content,
    });
    protocol.checkSignedProposal({
      event,
      plaintext: parts.plaintext,
      author: this.signer.pubkey,
    });
    return event;
  }

  // The queue

  private async flushAndSettle() {
    try {
      await this.flush();
      this.settle();
    } catch (e) {
      this.fail(e);
      if (!(e instanceof Offline)) throw e;
    }
  }

  /** Sends queued proposals in order, one at a time; one flush at a time. */
  flush(): Promise<void> {
    this.flushing ??= this.flushOnce().finally(
      () => (this.flushing = undefined),
    );
    return this.flushing;
  }

  private async flushOnce() {
    for (;;) {
      const item = this.queue[0];
      if (!item || item.rejected) return;
      const books = this.books.get(item.business);
      if (!books) return;
      if (this.sync.stopped.has(item.business)) return;
      if (item.baseSeq > books.head.seq) {
        await this.refuse(
          item,
          "The books changed before this could be sent. Redo it from the current books, or discard it.",
        );
        continue;
      }
      this.setState({ status: "syncing" });
      let ack: Acknowledgment;
      try {
        ack = await this.api.propose(item.event);
      } catch (e) {
        if (e instanceof ServiceError) {
          await this.refuse(item, e.message);
          continue;
        }
        throw e;
      }
      if (ack.status === "accepted") {
        await store.removeItem(item.id!);
        this.queue.shift();
        if (item.importId)
          this.batches.set(
            item.importId,
            (this.batches.get(item.importId) ?? 0) + 1,
          );
        this.changed();
        await this.syncBusiness(item.business);
        continue;
      }
      const reason = await this.readRejection(item, ack.event, books);
      if (reason.code === "tooLarge" && item.command.type === "CommitImport") {
        await this.split(item, books, reason.size ?? 0, reason.limit ?? 0);
        continue;
      }
      await this.refuse(item, reason.message);
    }
  }

  private async readRejection(
    item: QueueItem,
    event: NostrEvent,
    books: StoredBooks,
  ) {
    const member = this.signer.pubkey;
    const service = event.pubkey;
    if (service !== books.head.service && service !== this.pinned)
      throw new Error(
        "A rejection came from a key that isn't this business's service.",
      );
    const { proposal } = protocol.parseRejection({ event, member, service });
    if (proposal !== item.event.id)
      throw new Error(
        "The service rejected a different proposal than the one sent.",
      );
    const plaintext = await this.signer.decrypt(service, event.content);
    return protocol.openRejection({ event, member, service, plaintext });
  }

  /** A rejected import batch, again as smaller batches in its place. */
  private async split(
    item: QueueItem,
    books: StoredBooks,
    size: number,
    limit: number,
  ) {
    if (item.command.type !== "CommitImport") return;
    const rows = item.command.rows;
    const next = protocol.nextBatch({ rows: rows.length, size, limit });
    if (rows.length <= 1 || next >= rows.length) {
      await this.refuse(item, "One imported row is too large to send.");
      return;
    }
    const parts: QueueItem[] = [];
    const following = this.queue[1]?.order ?? item.order + 1;
    let n = 0;
    const count = Math.ceil(rows.length / next);
    for (let i = 0; i < rows.length; i += next)
      parts.push(
        await this.build(
          item.business,
          books,
          books.snapshot,
          { ...item.command, rows: rows.slice(i, i + next) },
          books.head.seq,
          {
            order: item.order + ((following - item.order) * ++n) / (count + 1),
            action: item.action,
            importId: item.importId,
          },
        ),
      );
    await store.removeItem(item.id!);
    await store.enqueue(parts);
    this.queue = await store.queue(this.signer.pubkey);
    this.changed();
  }

  /**
   * The service refused `item`. The caller waiting for it hears why, and it
   * and the rest of its action leave the queue; otherwise the queue stops
   * here until it is discarded or redone.
   */
  private async refuse(item: QueueItem, message: string) {
    if (this.waiting.has(item.action)) {
      this.refused.set(item.action, message);
      for (const q of this.queue.filter((q) => q.action === item.action))
        await store.removeItem(q.id!);
      this.queue = this.queue.filter((q) => q.action !== item.action);
    } else {
      item.rejected = message;
      await store.updateItem(item);
    }
    this.changed();
  }

  /** Drops a refused item; the queue goes on. */
  async discard(id: number) {
    await store.removeItem(id);
    this.queue = this.queue.filter((q) => q.id !== id);
    this.changed();
    await this.flushAndSettle();
  }

  /** Builds a refused item again against the current books, and sends it. */
  async redo(id: number) {
    const item = this.queue.find((q) => q.id === id);
    if (!item) return;
    const books = await this.syncBusiness(item.business);
    const state = this.withReceipts(books.snapshot);
    try {
      applyLocal(state, item.command);
    } catch (e) {
      item.rejected = (e as Error).message;
      await store.updateItem(item);
      this.changed();
      return;
    }
    const again = await this.build(
      item.business,
      books,
      state,
      item.command,
      books.head.seq,
      { order: item.order, action: item.action, importId: item.importId },
    );
    again.id = item.id;
    await store.updateItem(again);
    this.queue = await store.queue(this.signer.pubkey);
    this.changed();
    await this.flushAndSettle();
  }

  // Business creation

  /**
   * Sends a genesis proposal and opens the new business. Refuses when the
   * service reports a key other than the pinned one.
   */
  async createBusiness(input: {
    business: string;
    command: Record<string, unknown>;
  }): Promise<Workspace> {
    const reported = await this.api.serviceKey();
    if (reported !== this.pinned)
      throw new Error(
        "The service reports a different key from the one this app was set up with, so the business wasn't created. Don't continue until you know why.",
      );
    const parts = protocol.buildProposal({
      business: input.business,
      command: input.command,
    });
    const event = await this.sign(this.pinned, parts);
    const ack = await this.api.propose(event);
    if (ack.status !== "accepted") {
      const plaintext = await this.signer.decrypt(
        ack.event.pubkey,
        ack.event.content,
      );
      const r = protocol.openRejection({
        event: ack.event,
        member: this.signer.pubkey,
        service: this.pinned,
        plaintext,
      });
      throw new Error(r.message);
    }
    await store.addKnown(this.signer.pubkey, [input.business]);
    await this.sync.fetchKeys(input.business);
    await this.syncBusiness(input.business);
    this.settle();
    return this.workspace();
  }
}
