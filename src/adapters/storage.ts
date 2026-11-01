import type {
  BusinessData,
  BusinessRepository,
  ReceiptStorage,
  Workspace,
} from "../domain/types";
import { demoWorkspace } from "../domain/seed";
import { checkWorkspace } from "./workspace-check";
const DB = "business-finance-v1";
async function database(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const r = indexedDB.open(DB, 1);
    r.onupgradeneeded = () => r.result.createObjectStore("workspace");
    r.onsuccess = () => {
      // Another tab opening a newer version waits until this one lets go.
      r.result.onversionchange = () => r.result.close();
      resolve(r.result);
    };
    r.onerror = () => reject(r.error);
  });
}
export class LocalRepository implements BusinessRepository {
  private loading?: Promise<Workspace>;
  load(): Promise<Workspace> {
    if (!this.loading)
      this.loading = this.loadOnce().finally(() => {
        this.loading = undefined;
      });
    return this.loading;
  }
  private async loadOnce(): Promise<Workspace> {
    const db = await database();
    let state: unknown;
    try {
      state = await new Promise<unknown>((resolve, reject) => {
        const tx = db.transaction("workspace");
        const r = tx.objectStore("workspace").get("state");
        r.onsuccess = () => resolve(r.result);
        r.onerror = () => reject(r.error);
      });
    } finally {
      // Whether the read worked or not; a pending transaction still ends.
      db.close();
    }
    if (state !== undefined) return checkWorkspace(state);
    return this.write(demoWorkspace(), -1);
  }
  private async write(
    state: Workspace,
    expectedRevision: number,
  ): Promise<Workspace> {
    const db = await database();
    return new Promise((resolve, reject) => {
      // Strict durability: `complete` fires only once the write is flushed,
      // and callers report success only after `complete`.
      const tx = db.transaction("workspace", "readwrite", {
        durability: "strict",
      });
      const store = tx.objectStore("workspace");
      const r = store.get("state");
      let conflict = false;
      r.onsuccess = () => {
        if ((r.result?.revision ?? -1) !== expectedRevision) {
          conflict = true;
          tx.abort();
          return;
        }
        store.put(state, "state");
      };
      tx.oncomplete = () => {
        db.close();
        resolve(state);
      };
      tx.onabort = tx.onerror = () => {
        db.close();
        reject(
          new Error(
            conflict
              ? "This workspace changed in another tab. Reload before saving."
              : "Could not save locally. Check available storage and browser permissions.",
          ),
        );
      };
    });
  }
  async read(businessId: string) {
    const b = (await this.load()).businesses.find(
      (b) => b.profile.id === businessId,
    );
    if (!b) throw new Error("Business not found.");
    return b;
  }
  async commit(
    businessId: string,
    business: BusinessData,
    expectedRevision: number,
  ) {
    const state = await this.load();
    if (
      state.revision !== expectedRevision ||
      business.profile.id !== businessId ||
      !state.businesses.some((b) => b.profile.id === businessId)
    )
      throw new Error(
        "Workspace changed or business scope is invalid. Reload and try again.",
      );
    for (const records of [
      business.accounts,
      business.categories,
      business.vendors,
      business.documents,
      business.payments,
      business.drafts,
      business.journals,
      business.approvals,
      business.rules,
      business.audits,
      business.events,
      business.extractions,
      business.suggestions ?? [],
    ])
      if (records.some((r) => r.businessId !== businessId))
        throw new Error("Cross-business record rejected.");
    return this.write(
      {
        ...state,
        revision: state.revision + 1,
        businesses: state.businesses.map((b) =>
          b.profile.id === businessId ? business : b,
        ),
      },
      expectedRevision,
    );
  }
  async reset(expectedRevision: number) {
    return this.write(
      { ...demoWorkspace(), revision: expectedRevision + 1 },
      expectedRevision,
    );
  }
  async list(businessId: string) {
    return (await this.read(businessId)).documents;
  }
  async readReceipt(businessId: string, documentId: string) {
    const doc = (await this.list(businessId)).find((x) => x.id === documentId);
    if (!doc) throw new Error("Receipt not found in this business.");
    return doc.blob;
  }
}
// Separate receipt adapter avoids overloading repository.read with a document lookup.
export const repository = new LocalRepository();
export const receiptStorage: ReceiptStorage = {
  list: (id) => repository.list(id),
  read: (id, doc) => repository.readReceipt(id, doc),
};
