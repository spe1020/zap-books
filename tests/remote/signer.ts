/**
 * A test-only NIP-07 signer. It exists only in these tests: the page gets a
 * `window.nostr` whose calls are answered here, in the Playwright process,
 * so the invented secret key never reaches the page or the app's build.
 */
import type { BrowserContext } from "@playwright/test";
import { finalizeEvent, type EventTemplate } from "nostr-tools/pure";
import * as nip44 from "nostr-tools/nip44";
import { secretBytes, type TestKey } from "./keys";

export const conversation = (key: TestKey, other: string) =>
  nip44.getConversationKey(secretBytes(key), other);

export async function installSigner(
  context: BrowserContext,
  key: TestKey,
  options: { nip44?: boolean } = {},
) {
  await context.exposeBinding(
    "__zbTestSigner",
    (_source, op: string, a?: unknown, b?: unknown) => {
      switch (op) {
        case "pubkey":
          return key.pubkey;
        case "sign":
          return finalizeEvent(a as EventTemplate, secretBytes(key));
        case "encrypt":
          return nip44.encrypt(b as string, conversation(key, a as string));
        case "decrypt":
          return nip44.decrypt(b as string, conversation(key, a as string));
      }
      throw new Error(`Unknown signer call ${op}`);
    },
  );
  await context.addInitScript(
    ({ withNip44 }) => {
      const call = (
        window as unknown as {
          __zbTestSigner: (...args: unknown[]) => Promise<unknown>;
        }
      ).__zbTestSigner;
      (window as unknown as { nostr: unknown }).nostr = {
        name: "Test signer",
        getPublicKey: () => call("pubkey"),
        signEvent: (e: unknown) => call("sign", e),
        ...(withNip44 && {
          nip44: {
            encrypt: (pk: string, text: string) => call("encrypt", pk, text),
            decrypt: (pk: string, text: string) => call("decrypt", pk, text),
          },
        }),
      };
    },
    { withNip44: options.nip44 ?? true },
  );
}
