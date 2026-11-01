/**
 * Invented keys for the remote e2e tests only. They sign nothing anywhere
 * but the local test service.
 */
import { tmpdir } from "node:os";
import { join } from "node:path";
import { decode } from "nostr-tools/nip19";
import { getPublicKey } from "nostr-tools/pure";

const hex = (byte: string) => byte.repeat(32);
const bytes = (h: string) =>
  Uint8Array.from(h.match(/../g)!, (x) => parseInt(x, 16));

export interface TestKey {
  secret: string;
  pubkey: string;
}
const key = (byte: string): TestKey => ({
  secret: hex(byte),
  pubkey: getPublicKey(bytes(hex(byte))),
});

/** A fresh key, so each test starts with no businesses. */
export function freshKey(): TestKey {
  const secret = [...crypto.getRandomValues(new Uint8Array(32))]
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
  return { secret, pubkey: getPublicKey(bytes(secret)) };
}

/** A backup key as the owner wrote it down at creation, as a test key. */
export function fromNsec(nsec: string): TestKey {
  const decoded = decode(nsec);
  if (decoded.type !== "nsec") throw new Error("not an nsec");
  const secret = [...decoded.data]
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
  return { secret, pubkey: getPublicKey(decoded.data) };
}

export const SERVICE = key("e1");
export const OTHER_SERVICE = key("e4");
export const secretBytes = (k: TestKey) => bytes(k.secret);

export const SERVICE_PORT = 8797;
export const APP_PORT = 5199;
export const SERVICE_URL = `http://127.0.0.1:${SERVICE_PORT}`;
export const APP_URL = `http://127.0.0.1:${APP_PORT}`;
/** The test service's clock offset, in milliseconds (tests/remote/clock.ts). */
export const CLOCK_FILE = join(tmpdir(), `zapbooks-e2e-clock-${SERVICE_PORT}`);
/** The test service's blob store, emptied when it starts. */
export const BLOB_DIR = join(tmpdir(), `zapbooks-e2e-blobs-${SERVICE_PORT}`);
