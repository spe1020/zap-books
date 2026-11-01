import { useState } from "react";
import { KeyRound, MonitorSmartphone } from "lucide-react";
import { BRAND, PRODUCT_NAME } from "../config";
import { checkSigner } from "../remote/signer";
import type { RemoteLedgerClient } from "../remote";

/** Common NIP-07 signer extensions, in no particular order of preference. */
const extensions = [
  { name: "Alby", url: "https://getalby.com/" },
  { name: "Keys.band", url: "https://keys.band/" },
  { name: "nos2x", url: "https://github.com/fiatjaf/nos2x" },
];

/**
 * The first screen when the app is set up for a ledger service: sign in
 * with a NIP-07 signer extension, or open the local demo. The app never
 * asks for a secret key.
 */
export function SignIn({
  onSignedIn,
  onLocal,
}: {
  onSignedIn: (client: RemoteLedgerClient) => void;
  onLocal: () => void;
}) {
  const [check] = useState(checkSigner);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  async function signIn() {
    setBusy(true);
    setError("");
    try {
      const remote = await import("../remote");
      onSignedIn(await remote.signIn());
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <main className="loading sign-in">
      <h1 className="brand-heading">
        <img className="brand-logo" src={BRAND.logo} alt={PRODUCT_NAME} />
      </h1>
      <section className="card sign-in-card">
        <h2>
          <KeyRound size={19} />
          Sign in with your Nostr key
        </h2>
        {check.status === "none" ? (
          <>
            <p>
              No Nostr signer extension was found in this browser. A signer is a
              browser extension that holds your Nostr key and signs for you, so
              this app never sees the key itself.
            </p>
            <p>Some common ones (choose any that supports NIP-44):</p>
            <ul className="extension-list">
              {extensions.map((x) => (
                <li key={x.name}>
                  <a href={x.url} target="_blank" rel="noreferrer noopener">
                    {x.name}
                  </a>
                </li>
              ))}
            </ul>
            <p className="small muted">
              After installing one, reload this page.
            </p>
          </>
        ) : check.status === "no-nip44" ? (
          <p role="alert" className="alert error">
            {check.name} doesn't support NIP-44 encryption, which this app needs
            to keep your books private. Update it, or use a signer extension
            that supports NIP-44.
          </p>
        ) : (
          <>
            <p>
              Using <strong>{check.name}</strong>. It will ask you to share your
              public key and to sign. Your secret key stays in the extension.
            </p>
            <button className="button primary" disabled={busy} onClick={signIn}>
              {busy ? "Signing in…" : "Sign in"}
            </button>
          </>
        )}
        {error && (
          <p role="alert" className="alert error">
            {error}
          </p>
        )}
      </section>
      <section className="card sign-in-card">
        <h2>
          <MonitorSmartphone size={19} />
          Local demo
        </h2>
        <p>
          Fictional businesses kept in this browser only. No sign-in, and
          nothing is sent anywhere.
        </p>
        <button className="button secondary" onClick={onLocal}>
          Open the local demo
        </button>
      </section>
    </main>
  );
}
