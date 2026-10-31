import { afterEach, describe, expect, test, vi } from "vitest";
import { LocalRepository } from "./storage";
import { checkWorkspace } from "./workspace-check";
import { seedWorkspace } from "../domain/seed";
import { updateSettings } from "../domain/settings";
import { correctTransfer, recordTransfer } from "../domain/transfers";

test("a workspace saved before extraction records loads with none", () => {
  const saved = seedWorkspace();
  for (const b of saved.businesses)
    delete (b as { extractions?: unknown }).extractions;
  expect(saved.businesses[0]).not.toHaveProperty("extractions");
  const loaded = checkWorkspace(saved);
  expect(loaded.businesses.map((b) => b.extractions)).toEqual([[], []]);
  // Nothing else changes.
  expect(loaded.businesses.map(({ extractions: _, ...rest }) => rest)).toEqual(
    saved.businesses,
  );
});

describe("the stored workspace is checked on load (WEB-3)", () => {
  test("an undamaged workspace loads unchanged, with nothing set aside", () => {
    const saved = seedWorkspace();
    const loaded = checkWorkspace(saved);
    expect(loaded).toEqual(saved);
    expect(loaded).not.toHaveProperty("quarantine");
  });

  test("damaged records are set aside, reported and kept; the rest loads", () => {
    const saved = seedWorkspace();
    const b = saved.businesses[0];
    const draft = b.drafts.find((d) => d.status === "draft")!;
    const journal = b.journals[0];
    const damaged = {
      // Would crash the money formatter on every render.
      draft: { ...draft, currency: "zz" },
      // Unbalanced: would be counted in totals.
      journal: {
        ...journal,
        lines: journal.lines.map((l, n) =>
          n === 0 ? { ...l, debit: l.debit + 1 } : l,
        ),
      },
      account: { ...b.accounts[0], id: "invented-account", kind: "savings" },
      event: { ...b.events[0], businessId: saved.businesses[1].profile.id },
      rule: { ...b.rules[0], enabled: "yes" },
    };
    b.drafts = b.drafts.map((d) =>
      d.id === draft.id ? damaged.draft : d,
    ) as never;
    b.journals = [damaged.journal, ...b.journals.slice(1)] as never;
    b.accounts.push(damaged.account as never);
    b.events.push(damaged.event as never);
    b.rules.push(damaged.rule as never);
    (b.extractions as unknown[]).push({
      id: "invented-reading",
      businessId: b.profile.id,
      fields: "none",
    });

    const loaded = checkWorkspace(saved);
    const kept = loaded.businesses[0];
    expect(kept.drafts.some((d) => d.id === draft.id)).toBe(false);
    expect(kept.journals.some((j) => j.id === journal.id)).toBe(false);
    expect(kept.accounts.some((a) => a.id === "invented-account")).toBe(false);
    expect(kept.events).toHaveLength(b.events.length - 1);
    expect(kept.rules).toHaveLength(b.rules.length - 1);
    expect(kept.extractions).toHaveLength(0);
    expect(kept.drafts).toHaveLength(b.drafts.length - 1);
    // The other business is untouched.
    expect(loaded.businesses[1]).toEqual(saved.businesses[1]);
    // Each is reported with its list, its id and a fixed reason, and kept
    // exactly as it was stored.
    const q = loaded.quarantine!;
    expect(q.map((x) => [x.collection, x.id])).toEqual([
      ["accounts", "invented-account"],
      ["drafts", draft.id],
      ["journals", journal.id],
      ["rules", b.rules.at(-1)!.id],
      ["events", damaged.event.id],
      ["extractions", "invented-reading"],
    ]);
    expect(q.find((x) => x.collection === "drafts")!.record).toBe(
      damaged.draft,
    );
    expect(q.find((x) => x.collection === "journals")!.reason).toBe(
      "Journal entry must balance.",
    );
    expect(q.every((x) => x.businessId === b.profile.id)).toBe(true);
    // Loading again keeps what was set aside before.
    const again = checkWorkspace(loaded);
    expect(again.quarantine).toEqual(q);
  });

  test("a journal on a set-aside account is set aside too", () => {
    const saved = seedWorkspace();
    const b = saved.businesses[0];
    const journal = b.journals.find((j) => j.kind === "expense")!;
    const paying = journal.lines.find((l) => !l.categoryId)!.accountId;
    b.accounts = b.accounts.map((a) =>
      a.id === paying ? ({ ...a, currency: 7 } as never) : a,
    );
    const loaded = checkWorkspace(saved);
    expect(loaded.businesses[0].journals.some((j) => j.id === journal.id)).toBe(
      false,
    );
    expect(loaded.quarantine!.find((x) => x.id === journal.id)!.reason).toBe(
      "Invalid journal account or currency.",
    );
  });

  test("a business that can't be read is set aside whole", () => {
    const saved = seedWorkspace();
    const [first, second] = saved.businesses;
    (first as { drafts: unknown }).drafts = "none";
    const loaded = checkWorkspace(saved);
    expect(loaded.businesses).toEqual([second]);
    expect(loaded.quarantine).toMatchObject([
      { collection: "business", businessId: first.profile.id, record: first },
    ]);
  });

  test("a workspace with nothing usable is refused, not loaded", () => {
    const saved = seedWorkspace();
    for (const b of saved.businesses)
      (b.profile as { currency: unknown }).currency = "zz";
    expect(() => checkWorkspace(saved)).toThrow(
      "The saved workspace is damaged",
    );
    expect(() => checkWorkspace({ ...saved, schemaVersion: 2 })).toThrow(
      "Unsupported workspace version.",
    );
    expect(() =>
      checkWorkspace({ ...seedWorkspace(), businesses: {} }),
    ).toThrow("The saved workspace is damaged");
    expect(() => checkWorkspace(null)).toThrow(
      "The saved workspace is damaged",
    );
  });
});

/**
 * Just enough of IndexedDB for `load`: one database whose read either
 * succeeds or fails, recording each connection's `close`.
 */
function fakeIndexedDB(read: { result?: unknown; error?: Error }) {
  const connections: {
    closed: boolean;
    onversionchange: (() => void) | null;
  }[] = [];
  const open = () => {
    const request: Record<string, unknown> = {};
    const db = {
      closed: false,
      onversionchange: null as (() => void) | null,
      close() {
        db.closed = true;
      },
      transaction() {
        const tx: Record<string, unknown> = {};
        const get: Record<string, unknown> = {};
        tx.objectStore = () => ({
          get: () => {
            queueMicrotask(() => {
              if (read.error) {
                get.error = read.error;
                (get.onerror as () => void)();
                (tx.onabort as (() => void) | undefined)?.();
              } else {
                get.result = read.result;
                (get.onsuccess as () => void)();
                (tx.oncomplete as (() => void) | undefined)?.();
              }
            });
            return get;
          },
        });
        return tx;
      },
    };
    connections.push(db);
    request.result = db;
    queueMicrotask(() => (request.onsuccess as () => void)());
    return request;
  };
  vi.stubGlobal("indexedDB", { open });
  return connections;
}

describe("the database connection (WEB-7)", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  test("a failed read closes its connection", async () => {
    const connections = fakeIndexedDB({ error: new Error("Invented failure") });
    const repository = new LocalRepository();
    await expect(repository.load()).rejects.toThrow("Invented failure");
    await expect(repository.load()).rejects.toThrow("Invented failure");
    expect(connections).toHaveLength(2);
    expect(connections.every((c) => c.closed)).toBe(true);
  });

  test("a successful read closes its connection too", async () => {
    const connections = fakeIndexedDB({ result: seedWorkspace() });
    await new LocalRepository().load();
    expect(connections.map((c) => c.closed)).toEqual([true]);
  });

  test("a newer version opened elsewhere closes this connection", async () => {
    const connections = fakeIndexedDB({ error: new Error("Invented failure") });
    await new LocalRepository().load().catch(() => {});
    const [db] = connections;
    db.closed = false;
    db.onversionchange!();
    expect(db.closed).toBe(true);
  });
});

describe("suggestions on load (decision 0004)", () => {
  test("a business without suggestions keeps none", () => {
    const loaded = checkWorkspace(seedWorkspace());
    expect(loaded.businesses[0]).not.toHaveProperty("suggestions");
  });

  test("suggestions load, and a malformed one is set aside", () => {
    const saved = seedWorkspace();
    const b = saved.businesses[0];
    const good = {
      id: "invented-suggestion",
      businessId: b.profile.id,
      draftId: b.drafts[0].id,
      author: "ef".repeat(32),
      note: "Invented note",
      evidence: ["Invented line"],
      createdAt: "2026-01-02T03:04:05.000Z",
    };
    b.suggestions = [good, { ...good, id: "bad", author: "not a key" }];
    const loaded = checkWorkspace(saved);
    expect(loaded.businesses[0].suggestions).toEqual([good]);
    expect(loaded.quarantine?.map((q) => [q.collection, q.id])).toEqual([
      ["suggestions", "bad"],
    ]);
  });
});

describe("owner funding on load", () => {
  test("loan and equity accounts and typed transfers load unchanged", () => {
    const saved = seedWorkspace();
    let b = updateSettings(saved.businesses[0], {
      op: "addAccount",
      name: "Loan from owner",
      kind: "liability",
      currency: "USD",
    });
    const loan = b.accounts.at(-1)!.id;
    b = recordTransfer(b, {
      date: "2026-10-20",
      amountMinor: 50000,
      currency: "USD",
      fromAccountId: loan,
      toAccountId: b.accounts[0].id,
      transferType: "ownerLoan",
      purpose: "Invented working capital",
      reference: "INV-1",
    });
    b = correctTransfer(
      b,
      b.journals.at(-1)!.id,
      {
        date: "2026-10-20",
        amountMinor: 40000,
        currency: "USD",
        fromAccountId: loan,
        toAccountId: b.accounts[0].id,
        transferType: "ownerLoan",
        purpose: "Invented working capital",
      },
      "Invented correction",
    );
    saved.businesses[0] = b;
    const loaded = checkWorkspace(saved);
    expect(loaded).toEqual(saved);
    expect(loaded).not.toHaveProperty("quarantine");
  });

  test("an unknown transfer type is set aside", () => {
    const saved = seedWorkspace();
    const transfer = saved.businesses[0].journals.find(
      (j) => j.kind === "transfer",
    )!;
    (transfer as { transferType?: string }).transferType = "ownerGift";
    const loaded = checkWorkspace(saved);
    expect(loaded.quarantine!.map((x) => x.id)).toEqual([transfer.id]);
  });
});
