/**
 * Signing in to the ledger service. Loaded only when someone signs in, so
 * the local demo never loads the protocol's WebAssembly.
 */
import { decode, npubEncode } from "nostr-tools/nip19";
import type { PeopleTools } from "../components/Members";
import { ServiceApi } from "./api";
import { RemoteLedgerClient } from "./client";
import { SERVICE_PUBKEY, SERVICE_URL } from "./config";
import { loadProtocol } from "./protocol";
import { connectSigner } from "./signer";

export { RemoteLedgerClient, Waiting } from "./client";
export type { Guarded, RemoteView, SyncState } from "./client";

/** Connects the signer extension and opens this key's books. */
export async function signIn(): Promise<RemoteLedgerClient> {
  await loadProtocol();
  const signer = await connectSigner();
  const client = new RemoteLedgerClient(
    new ServiceApi(SERVICE_URL, signer),
    signer,
    SERVICE_PUBKEY,
  );
  await client.load();
  return client;
}

export const npub = (hex: string) => npubEncode(hex);

/** The hex key an npub names, or why it doesn't. */
export function parseNpub(text: string): { key: string } | { error: string } {
  if (!text) return { error: "Enter their npub." };
  try {
    const decoded = decode(text);
    if (decoded.type === "npub") return { key: decoded.data };
  } catch {
    /* Not NIP-19. */
  }
  return {
    error: text.startsWith("nsec")
      ? "That's a secret key. Never share it: ask for their npub, which starts with npub1."
      : "That isn't an npub. It starts with npub1.",
  };
}

/** What the members screens need, for the signed-in key. */
export function peopleTools(
  client: RemoteLedgerClient,
  business: string,
): PeopleTools {
  return {
    me: client.signer.pubkey,
    npub,
    parseNpub,
    isOwnerLevel: (change) => client.isOwnerLevel(business, change),
  };
}
