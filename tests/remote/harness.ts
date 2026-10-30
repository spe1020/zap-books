/**
 * What the tests do outside the browser: send proposals as a test key
 * (building them with the same protocol WebAssembly the app uses), read the
 * chain with NIP-98, and forge acceptances for the tamper and fork tests.
 */
import { readFileSync } from "node:fs";
import { finalizeEvent } from "nostr-tools/pure";
import * as nip44 from "nostr-tools/nip44";
import * as wasm from "../../crates/protocol-wasm/pkg/protocol_wasm.js";
import { SERVICE, SERVICE_URL, secretBytes, type TestKey } from "./keys";
import { conversation } from "./signer";
import { serviceNow } from "./clock";

let loaded = false;
function protocol() {
  if (!loaded) {
    wasm.initSync({
      module: readFileSync(
        new URL(
          "../../crates/protocol-wasm/pkg/protocol_wasm_bg.wasm",
          import.meta.url,
        ),
      ),
    });
    loaded = true;
  }
  return wasm;
}

export interface Event {
  id: string;
  pubkey: string;
  created_at: number;
  kind: number;
  tags: string[][];
  content: string;
  sig: string;
}

const now = () => Math.floor(serviceNow() / 1000);

const signedAt = new Map<string, number>();

/** As the app does: never two identical auth events in one second. */
async function nip98(key: TestKey, url: string, method: string) {
  const id = `${key.pubkey} ${method} ${url}`;
  let at = now();
  const last = signedAt.get(id);
  if (last !== undefined && at <= last) {
    await new Promise((r) => setTimeout(r, (last + 1) * 1000 - serviceNow()));
    at = last + 1;
  }
  signedAt.set(id, at);
  const event = finalizeEvent(
    {
      kind: 27235,
      created_at: at,
      tags: [
        ["u", url],
        ["method", method],
      ],
      content: "",
    },
    secretBytes(key),
  );
  return `Nostr ${Buffer.from(JSON.stringify(event)).toString("base64")}`;
}

/** The whole chain of `business`, read as `key`. */
export async function chain(key: TestKey, business: string) {
  const url = `${SERVICE_URL}/v1/businesses/${business}/acceptances?limit=500`;
  const r = await fetch(url, {
    headers: { Authorization: await nip98(key, url, "GET") },
  });
  if (!r.ok) throw new Error(`chain: ${r.status} ${await r.text()}`);
  return (await r.json()) as { head: number; acceptances: Event[] };
}

/** Sends `command` as `key`, against the current head. */
export async function propose(
  key: TestKey,
  business: string,
  command: unknown,
): Promise<{ status: string }> {
  const { head } = await chain(key, business);
  const parts = JSON.parse(
    protocol().buildProposal(
      JSON.stringify({ business, baseSeq: head, command }),
    ),
  ) as { kind: number; plaintext: string; tags: string[][] };
  const event = finalizeEvent(
    {
      kind: parts.kind,
      created_at: now(),
      tags: parts.tags,
      content: nip44.encrypt(
        parts.plaintext,
        conversation(key, SERVICE.pubkey),
      ),
    },
    secretBytes(key),
  );
  const r = await fetch(`${SERVICE_URL}/v1/proposals`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(event),
  });
  if (!r.ok) throw new Error(`propose: ${r.status} ${await r.text()}`);
  return (await r.json()) as { status: string };
}

/** Adds `member` with full history, as the owner `owner`. */
export async function addMember(
  owner: TestKey,
  business: string,
  member: TestKey,
  role = "bookkeeper",
) {
  const ack = await propose(owner, business, {
    type: "ChangeMembership",
    change: { op: "add", member: member.pubkey, role, history: "full" },
  });
  if (ack.status !== "accepted") throw new Error("The member wasn't added.");
}

/**
 * The same acceptance, signed again by the service key a second later: a
 * valid event with a different id, as a service rewriting its history
 * would make.
 */
export function resigned(event: Event): Event {
  return finalizeEvent(
    {
      kind: event.kind,
      created_at: event.created_at + 1,
      tags: event.tags,
      content: event.content,
    },
    secretBytes(SERVICE),
  ) as Event;
}

/** The acceptance with one character of its content changed. */
export function tampered(event: Event): Event {
  const c = event.content;
  return { ...event, content: (c[0] === "A" ? "B" : "A") + c.slice(1) };
}

/** A read as `key`, with NIP-98: the status and the parsed body. */
export async function readAs<T>(key: TestKey, path: string) {
  const url = `${SERVICE_URL}${path}`;
  const r = await fetch(url, {
    headers: { Authorization: await nip98(key, url, "GET") },
  });
  return { status: r.status, body: (await r.json()) as T };
}

/** A session sign-in as `key`: the status the service answers. */
export async function sessionStatus(key: TestKey) {
  const url = `${SERVICE_URL}/v1/session`;
  const r = await fetch(url, {
    method: "POST",
    headers: { Authorization: await nip98(key, url, "POST") },
  });
  return r.status;
}

/** The epoch keys delivered to `key` for `business`, opened: epoch to secret. */
export async function epochKeys(key: TestKey, business: string) {
  const { status, body } = await readAs<{ deliveries: Event[] }>(
    key,
    `/v1/epoch-keys?business=${business}`,
  );
  if (status !== 200) throw new Error(`epoch keys: ${status}`);
  const keys = new Map<number, string>();
  for (const event of body.deliveries) {
    const plaintext = nip44.decrypt(
      event.content,
      conversation(key, event.pubkey),
    );
    const opened = JSON.parse(
      protocol().openDelivery(
        JSON.stringify({
          event,
          member: key.pubkey,
          service: event.pubkey,
          plaintext,
        }),
      ),
    ) as { epoch: number; secret: string };
    keys.set(opened.epoch, opened.secret);
  }
  return keys;
}

const tag = (event: Event, name: string) =>
  event.tags.find((t) => t[0] === name)?.[1];

/** The epoch an acceptance is sealed under, from its tag. */
export const epochOf = (event: Event) => Number(tag(event, "epoch"));

/**
 * Opens an acceptance with an epoch secret, as the app does. Throws if the
 * secret doesn't open it.
 */
export function openWith(event: Event, business: string, secret: string) {
  return JSON.parse(
    protocol().openAcceptance(
      JSON.stringify({
        event,
        epoch: epochOf(event),
        epochSecret: secret,
        service: SERVICE.pubkey,
        business,
      }),
    ),
  ) as { content: { payload: string } };
}

interface Snapshot {
  accounts: { id: string; kind: string; currency: string }[];
  categories: { id: string; accountId: string }[];
  drafts: { id: string; status: string }[];
}

/** The books as `key` reads them: every acceptance opened and applied. */
export async function books(key: TestKey, business: string) {
  const keys = await epochKeys(key, business);
  const { acceptances } = await chain(key, business);
  let snapshot: Snapshot | undefined;
  for (const event of acceptances) {
    const secret = keys.get(epochOf(event));
    if (!secret) throw new Error(`no key for epoch ${epochOf(event)}`);
    const opened = protocol().openAcceptance(
      JSON.stringify({
        event,
        epoch: epochOf(event),
        epochSecret: secret,
        service: SERVICE.pubkey,
        business,
      }),
    );
    snapshot = JSON.parse(
      protocol().applyRecordChanges(
        JSON.stringify({ snapshot, acceptance: JSON.parse(opened) }),
      ),
    ) as Snapshot;
  }
  return snapshot!;
}

/** A new, complete draft for `business`, as an assistant's harness makes one. */
export async function assistantDraft(
  reader: TestKey,
  business: string,
  vendor: string,
) {
  const b = await books(reader, business);
  const category = b.categories[0];
  const account = b.accounts.find((a) => a.kind !== "expense")!;
  return {
    id: crypto.randomUUID(),
    businessId: business,
    vendor,
    date: "2031-04-02",
    description: "Kiln shelf paper",
    amountMinor: 1840,
    currency: account.currency,
    categoryId: category.id,
    accountId: account.id,
    purpose: "Firing supplies",
    documentIds: [],
    status: "draft",
    duplicateReason: "",
    createdAt: "2031-04-02T10:00:00.000Z",
  };
}
