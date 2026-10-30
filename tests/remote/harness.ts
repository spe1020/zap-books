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

const now = () => Math.floor(Date.now() / 1000);

const signedAt = new Map<string, number>();

/** As the app does: never two identical auth events in one second. */
async function nip98(key: TestKey, url: string, method: string) {
  const id = `${key.pubkey} ${method} ${url}`;
  let at = now();
  const last = signedAt.get(id);
  if (last !== undefined && at <= last) {
    await new Promise((r) => setTimeout(r, (last + 1) * 1000 - Date.now()));
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
