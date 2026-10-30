/**
 * The ledger service, when this build is configured for one
 * (`VITE_SERVICE_URL`, `VITE_SERVICE_PUBKEY`): its URL and its pinned public
 * key, hex. Without both, the app is the local demo only.
 */
export const SERVICE_URL: string = (import.meta.env.VITE_SERVICE_URL ?? "")
  .trim()
  .replace(/\/+$/, "");
export const SERVICE_PUBKEY: string = (
  import.meta.env.VITE_SERVICE_PUBKEY ?? ""
).trim();
export const serviceConfigured =
  !!SERVICE_URL && /^[0-9a-f]{64}$/.test(SERVICE_PUBKEY);
