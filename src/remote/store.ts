/**
 * What the browser keeps for businesses on the service, in its own IndexedDB
 * database (the local demo's stays separate):
 *
 * - `books`: per signed-in key and business, the verified head, the books
 *   and the membership after it, and what the members screens show. Written
 *   only after a whole sync verified.
 * - `queue`: signed proposals waiting to be sent, in order.
 * - `receipts`: receipt files this browser attached or downloaded.
 * - `uploads`: encrypted receipts waiting to be uploaded ahead of their
 *   proposals, kept until the proposal is accepted, so a retry sends the
 *   same ciphertext under the same key.
 * - `known`: the businesses each key has opened or created here.
 *
 * Secret keys are never stored: member keys stay in the signer extension
 * and epoch keys in memory.
 */
import type { BusinessData } from "../domain/types";
import type { LedgerCommand } from "../adapters/ledger-client";
import type { Head, Membership, NostrEvent } from "./protocol";
import type { People } from "./people";

const DB = "zapbooks-remote-v1";

export interface StoredBooks {
  owner: string;
  business: string;
  head: Head;
  snapshot: BusinessData;
  membership: Membership;
  /** Join times and the like, for the members screens (`people.ts`). */
  people: People;
}

/** One proposal waiting, or stopped by a rejection. */
export interface QueueItem {
  id?: number;
  owner: string;
  business: string;
  /** What it does, for the "waiting to sync" list. */
  label: string;
  /** The command, without receipt files, so it can be redone. */
  command: LedgerCommand;
  /** The signed proposal and its plaintext; resending gives the same answer. */
  event: NostrEvent;
  plaintext: string;
  /** The `baseSeq` it was built with. */
  baseSeq: number;
  /** Its place in the queue. */
  order: number;
  /** The action it belongs to: one user action can be several proposals. */
  action: string;
  /** Set on each batch of one import. */
  importId?: string;
  /** Set when the service rejected it: the queue stops here. */
  rejected?: string;
  /** The `stored` hashes of encrypted receipts to upload before it. */
  uploads?: string[];
}

function open(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const r = indexedDB.open(DB, 2);
    r.onupgradeneeded = (e) => {
      const db = r.result;
      if (e.oldVersion < 1) {
        db.createObjectStore("books");
        db.createObjectStore("queue", { keyPath: "id", autoIncrement: true });
        db.createObjectStore("receipts");
        db.createObjectStore("known");
      }
      if (e.oldVersion < 2) db.createObjectStore("uploads");
    };
    r.onsuccess = () => {
      r.result.onversionchange = () => r.result.close();
      resolve(r.result);
    };
    r.onerror = () => reject(r.error);
  });
}

async function run<T>(
  stores: string[],
  mode: IDBTransactionMode,
  work: (tx: IDBTransaction) => IDBRequest<T> | void,
): Promise<T> {
  const db = await open();
  return new Promise<T>((resolve, reject) => {
    const tx = db.transaction(stores, mode, { durability: "strict" });
    let result: T;
    const request = work(tx);
    if (request) request.onsuccess = () => (result = request.result);
    tx.oncomplete = () => {
      db.close();
      resolve(result);
    };
    tx.onabort = tx.onerror = () => {
      db.close();
      reject(
        new Error(
          "Could not use this browser's storage. Check available storage and browser permissions.",
        ),
      );
    };
  });
}

const booksKey = (owner: string, business: string) => `${owner}:${business}`;

export const store = {
  books: (owner: string, business: string) =>
    run<StoredBooks | undefined>(["books"], "readonly", (tx) =>
      tx.objectStore("books").get(booksKey(owner, business)),
    ),
  /** The books after a verified sync; the caller has checked the head extends. */
  saveBooks: (books: StoredBooks) =>
    run<IDBValidKey>(["books"], "readwrite", (tx) =>
      tx.objectStore("books").put(books, booksKey(books.owner, books.business)),
    ),
  known: async (owner: string) =>
    (await run<string[] | undefined>(["known"], "readonly", (tx) =>
      tx.objectStore("known").get(owner),
    )) ?? [],
  addKnown: async (owner: string, businesses: string[]) => {
    const now = await store.known(owner);
    const next = [...new Set([...now, ...businesses])];
    if (next.length !== now.length)
      await run(["known"], "readwrite", (tx) =>
        tx.objectStore("known").put(next, owner),
      );
  },
  queue: async (owner: string) =>
    (
      await run<QueueItem[]>(["queue"], "readonly", (tx) =>
        tx.objectStore("queue").getAll(),
      )
    )
      .filter((q) => q.owner === owner)
      .sort((a, z) => a.order - z.order),
  enqueue: (items: QueueItem[]) =>
    run(["queue"], "readwrite", (tx) => {
      for (const item of items) tx.objectStore("queue").add(item);
    }),
  updateItem: (item: QueueItem) =>
    run(["queue"], "readwrite", (tx) => {
      tx.objectStore("queue").put(item);
    }),
  removeItem: (id: number) =>
    run(["queue"], "readwrite", (tx) => {
      tx.objectStore("queue").delete(id);
    }),
  receipt: (business: string, document: string) =>
    run<Blob | undefined>(["receipts"], "readonly", (tx) =>
      tx.objectStore("receipts").get(`${business}:${document}`),
    ),
  saveReceipts: (business: string, files: { id: string; blob: Blob }[]) =>
    run(["receipts"], "readwrite", (tx) => {
      for (const f of files)
        tx.objectStore("receipts").put(f.blob, `${business}:${f.id}`);
    }),
  upload: (business: string, hash: string) =>
    run<Uint8Array<ArrayBuffer> | undefined>(["uploads"], "readonly", (tx) =>
      tx.objectStore("uploads").get(`${business}:${hash}`),
    ),
  saveUploads: (
    business: string,
    files: { hash: string; ciphertext: Uint8Array<ArrayBuffer> }[],
  ) =>
    run(["uploads"], "readwrite", (tx) => {
      for (const f of files)
        tx.objectStore("uploads").put(f.ciphertext, `${business}:${f.hash}`);
    }),
  removeUploads: (business: string, hashes: string[]) =>
    run(["uploads"], "readwrite", (tx) => {
      for (const h of hashes)
        tx.objectStore("uploads").delete(`${business}:${h}`);
    }),
};
