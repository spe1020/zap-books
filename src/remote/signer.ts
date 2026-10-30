/**
 * The member's key, held by a NIP-07 signer extension. The app never sees a
 * secret key: the extension signs events and does the member's NIP-44
 * encryption and decryption. There is no NIP-04 fallback.
 */
import type { NostrEvent } from "./protocol";

/** An event for the extension to sign. */
export interface UnsignedEvent {
  kind: number;
  created_at: number;
  tags: string[][];
  content: string;
}

interface Nip07 {
  getPublicKey(): Promise<string>;
  signEvent(event: UnsignedEvent): Promise<NostrEvent>;
  nip44?: {
    encrypt(pubkey: string, plaintext: string): Promise<string>;
    decrypt(pubkey: string, ciphertext: string): Promise<string>;
  };
}

declare global {
  interface Window {
    nostr?: Nip07;
  }
}

export interface Signer {
  /** The member's public key, hex. */
  pubkey: string;
  /** The extension's name, as far as it can be told. */
  name: string;
  sign(event: UnsignedEvent): Promise<NostrEvent>;
  encrypt(to: string, plaintext: string): Promise<string>;
  decrypt(from: string, ciphertext: string): Promise<string>;
}

export type SignerCheck =
  | { status: "none" }
  | { status: "no-nip44"; name: string }
  | { status: "ready"; name: string };

/** Which extension this is, from what extensions are known to add. */
function extensionName(): string {
  const w = window as unknown as Record<string, unknown>;
  const own = (w.nostr as { name?: unknown } | undefined)?.name;
  if (typeof own === "string" && own.trim()) return own.trim();
  if (w.alby) return "Alby";
  if (w.nos2x) return "nos2x";
  return "A NIP-07 signer extension";
}

export function checkSigner(): SignerCheck {
  const nostr = window.nostr;
  if (!nostr || typeof nostr.getPublicKey !== "function")
    return { status: "none" };
  const name = extensionName();
  if (
    typeof nostr.nip44?.encrypt !== "function" ||
    typeof nostr.nip44?.decrypt !== "function"
  )
    return { status: "no-nip44", name };
  return { status: "ready", name };
}

/** Asks the extension for the member's key. */
export async function connectSigner(): Promise<Signer> {
  const check = checkSigner();
  const nostr = window.nostr;
  if (check.status === "none" || !nostr)
    throw new Error("No Nostr signer extension was found in this browser.");
  if (check.status === "no-nip44" || !nostr.nip44)
    throw new Error(
      `${check.name} can't encrypt with NIP-44, which this app needs. Update it, or use an extension that supports NIP-44.`,
    );
  const nip44 = nostr.nip44;
  const pubkey = (await nostr.getPublicKey()).toLowerCase();
  if (!/^[0-9a-f]{64}$/.test(pubkey))
    throw new Error("The signer extension gave an invalid public key.");
  return {
    pubkey,
    name: check.name,
    sign: async (event) => {
      const signed = await nostr.signEvent(event);
      if (signed.pubkey !== pubkey)
        throw new Error("The signer extension signed with a different key.");
      return signed;
    },
    encrypt: (to, plaintext) => nip44.encrypt(to, plaintext),
    decrypt: (from, ciphertext) => nip44.decrypt(from, ciphertext),
  };
}
