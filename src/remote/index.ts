/**
 * Signing in to the ledger service. Loaded only when someone signs in, so
 * the local demo never loads the protocol's WebAssembly.
 */
import { npubEncode } from "nostr-tools/nip19";
import { ServiceApi } from "./api";
import { RemoteLedgerClient } from "./client";
import { SERVICE_PUBKEY, SERVICE_URL } from "./config";
import { loadProtocol } from "./protocol";
import { connectSigner } from "./signer";

export { RemoteLedgerClient, Waiting } from "./client";
export type { RemoteView, SyncState } from "./client";

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
