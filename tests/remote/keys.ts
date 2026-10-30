/**
 * Invented keys for the remote e2e tests only. They sign nothing anywhere
 * but the local test service.
 */
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

export const SERVICE = key("e1");
export const OTHER_SERVICE = key("e4");
export const secretBytes = (k: TestKey) => bytes(k.secret);

export const SERVICE_PORT = 8797;
export const APP_PORT = 5199;
export const SERVICE_URL = `http://127.0.0.1:${SERVICE_PORT}`;
export const APP_URL = `http://127.0.0.1:${APP_PORT}`;
