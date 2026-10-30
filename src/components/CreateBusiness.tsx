import { useRef, useState } from "react";
import {
  Building2,
  KeyRound,
  Plus,
  Printer,
  ShieldCheck,
  Trash2,
} from "lucide-react";
import { decode, nsecEncode } from "nostr-tools/nip19";
import type { Currency } from "../domain/types";
import { currencies } from "../domain/types";
import type { RemoteLedgerClient } from "../remote";
import { protocol } from "../remote/protocol";
import { SERVICE_PUBKEY } from "../remote/config";
import { npub } from "../remote";

const ADVICE =
  "Your backup key can take over this business after 48 hours if it's ever misused. Keep it as safe as your own key, offline. If you can, add a second owner too.";

const bytes = (hex: string) =>
  Uint8Array.from(hex.match(/../g) ?? [], (h) => parseInt(h, 16));

const randomHex = (n: number) =>
  [...crypto.getRandomValues(new Uint8Array(n))]
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");

/** A key's hex, in groups of four, to compare by eye. */
export const fingerprint = (hex: string) => hex.match(/.{4}/g)!.join(" ");

type Recovery =
  | { type: "secondOwner"; npub: string }
  | { type: "backupKey"; pubkey?: string };

/**
 * Creating a business on the service: its name, currency and starting
 * accounts and categories, a required recovery setup, and a check of the
 * service key, then the genesis proposal.
 */
export function CreateBusiness({
  client,
  onCreated,
  onCancel,
}: {
  client: RemoteLedgerClient;
  onCreated: (businessId: string) => void;
  onCancel?: () => void;
}) {
  const [name, setName] = useState("");
  const [industry, setIndustry] = useState("");
  const [currency, setCurrency] = useState<Currency>("USD");
  const [accounts, setAccounts] = useState([
    { name: "Business checking", kind: "bank" as "bank" | "card" },
  ]);
  const [categories, setCategories] = useState([
    "Supplies",
    "Software and subscriptions",
    "Travel",
  ]);
  const [recovery, setRecovery] = useState<Recovery>({ type: "backupKey" });
  // The backup key's secret, only while it is shown. Never stored or sent.
  const [secret, setSecret] = useState<string | null>(null);
  const [lastSix, setLastSix] = useState("");
  // Its public half, between generating and confirming it.
  const pendingPublic = useRef<string | undefined>(undefined);
  const [serviceOk, setServiceOk] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  const nsec = secret ? nsecEncode(bytes(secret)) : "";

  function generateBackup() {
    const keys = protocol.generateRecoveryKeypair();
    setSecret(keys.secretKey);
    setRecovery({ type: "backupKey" });
    // The public key is kept only once the last six characters match.
    pendingPublic.current = keys.publicKey;
    setLastSix("");
  }

  function confirmBackup() {
    if (!nsec || lastSix.trim() !== nsec.slice(-6)) {
      setError(
        "The last six characters don't match. Check what you wrote down.",
      );
      return;
    }
    setError("");
    setRecovery({ type: "backupKey", pubkey: pendingPublic.current });
    pendingPublic.current = undefined;
    setSecret(null);
    setLastSix("");
  }

  function recoveryCommand() {
    if (recovery.type === "backupKey") {
      if (!recovery.pubkey)
        throw new Error(
          "Set up the backup key, and confirm you've written it down.",
        );
      return { type: "backupKey", pubkey: recovery.pubkey };
    }
    let pubkey: string;
    try {
      const d = decode(recovery.npub.trim());
      if (d.type !== "npub") throw new Error();
      pubkey = d.data;
    } catch {
      throw new Error("Enter the second owner's npub (it starts with npub1).");
    }
    if (pubkey === client.signer.pubkey)
      throw new Error("The second owner must be someone else's key.");
    return { type: "secondOwner", pubkey };
  }

  async function create() {
    setError("");
    try {
      if (!name.trim()) throw new Error("Enter the business name.");
      const accountNames = accounts.map((a) => a.name.trim());
      const categoryNames = categories.map((c) => c.trim());
      if (!accountNames.length || accountNames.some((n) => !n))
        throw new Error("Name each bank or card account.");
      if (!categoryNames.length || categoryNames.some((n) => !n))
        throw new Error("Name each category.");
      // Each category gets an expense account of the same name.
      const all = [...accountNames, ...categoryNames].map((n) =>
        n.toLowerCase(),
      );
      if (new Set(all).size !== all.length)
        throw new Error(
          "Account and category names must all be different, ignoring case.",
        );
      const recoveryKey = recoveryCommand();
      if (!serviceOk)
        throw new Error(
          "Confirm the service key before creating the business.",
        );
      const business = randomHex(32);
      const expense = categoryNames.map((n) => ({
        id: crypto.randomUUID(),
        businessId: business,
        name: n,
        kind: "expense",
        currency,
      }));
      const command = {
        type: "CreateBusiness",
        service: SERVICE_PUBKEY,
        recovery: recoveryKey,
        profile: {
          id: business,
          businessId: business,
          name: name.trim(),
          industry: industry.trim(),
          currency,
          initials:
            name
              .trim()
              .split(/\s+/)
              .slice(0, 2)
              .map((w) => w[0]?.toUpperCase() ?? "")
              .join("") || "B",
          color: "#c8da78",
        },
        accounts: [
          ...accounts.map((a) => ({
            id: crypto.randomUUID(),
            businessId: business,
            name: a.name.trim(),
            kind: a.kind,
            currency,
          })),
          ...expense,
        ],
        categories: expense.map((a) => ({
          id: crypto.randomUUID(),
          businessId: business,
          name: a.name,
          accountId: a.id,
        })),
      };
      setBusy(true);
      await client.createBusiness({ business, command });
      onCreated(business);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <main className="create-business">
      <header>
        <p className="eyebrow">A NEW BUSINESS ON THE LEDGER SERVICE</p>
        <h1>Create a business</h1>
        <p className="muted">
          Signed in as <code>{npub(client.signer.pubkey)}</code> with{" "}
          {client.signer.name}.
        </p>
      </header>

      <section className="card setting-card">
        <h2>
          <Building2 size={19} />
          1. The business
        </h2>
        <div className="create-grid">
          <label>
            Business name
            <input
              value={name}
              onChange={(e) => setName(e.target.value)}
              maxLength={80}
            />
          </label>
          <label>
            Industry
            <input
              value={industry}
              onChange={(e) => setIndustry(e.target.value)}
              maxLength={80}
            />
          </label>
          <label>
            Reporting currency
            <select
              value={currency}
              onChange={(e) => setCurrency(e.target.value as Currency)}
            >
              {currencies.map((c) => (
                <option key={c}>{c}</option>
              ))}
            </select>
          </label>
        </div>
        <h3>Bank and card accounts</h3>
        {accounts.map((a, i) => (
          <div className="create-row" key={i}>
            <input
              aria-label={`Account ${i + 1} name`}
              value={a.name}
              onChange={(e) =>
                setAccounts(
                  accounts.map((x, j) =>
                    j === i ? { ...x, name: e.target.value } : x,
                  ),
                )
              }
            />
            <select
              aria-label={`Account ${i + 1} kind`}
              value={a.kind}
              onChange={(e) =>
                setAccounts(
                  accounts.map((x, j) =>
                    j === i
                      ? { ...x, kind: e.target.value as "bank" | "card" }
                      : x,
                  ),
                )
              }
            >
              <option value="bank">Bank</option>
              <option value="card">Card</option>
            </select>
            <button
              className="icon-button"
              aria-label={`Remove account ${i + 1}`}
              disabled={accounts.length === 1}
              onClick={() => setAccounts(accounts.filter((_, j) => j !== i))}
            >
              <Trash2 size={15} />
            </button>
          </div>
        ))}
        <button
          className="button secondary small"
          onClick={() => setAccounts([...accounts, { name: "", kind: "card" }])}
        >
          <Plus size={14} />
          Add account
        </button>
        <h3>Categories</h3>
        {categories.map((c, i) => (
          <div className="create-row" key={i}>
            <input
              aria-label={`Category ${i + 1} name`}
              value={c}
              onChange={(e) =>
                setCategories(
                  categories.map((x, j) => (j === i ? e.target.value : x)),
                )
              }
            />
            <button
              className="icon-button"
              aria-label={`Remove category ${i + 1}`}
              disabled={categories.length === 1}
              onClick={() =>
                setCategories(categories.filter((_, j) => j !== i))
              }
            >
              <Trash2 size={15} />
            </button>
          </div>
        ))}
        <button
          className="button secondary small"
          onClick={() => setCategories([...categories, ""])}
        >
          <Plus size={14} />
          Add category
        </button>
      </section>

      <section className="card setting-card">
        <h2>
          <KeyRound size={19} />
          2. Recovery
        </h2>
        <p>
          Every business needs a way back in if an owner's key is lost or
          stolen: a second owner, or a backup key kept offline.
        </p>
        <p className="alert warning recovery-advice">{ADVICE}</p>
        <fieldset className="recovery-choice">
          <legend className="sr-only">Recovery</legend>
          <label>
            <input
              type="radio"
              name="recovery"
              checked={recovery.type === "backupKey"}
              onChange={() => setRecovery({ type: "backupKey" })}
            />
            An offline backup key
          </label>
          <label>
            <input
              type="radio"
              name="recovery"
              checked={recovery.type === "secondOwner"}
              onChange={() => {
                setSecret(null);
                setRecovery({ type: "secondOwner", npub: "" });
              }}
            />
            A second owner
          </label>
        </fieldset>
        {recovery.type === "secondOwner" ? (
          <label>
            Second owner's npub
            <input
              value={recovery.npub}
              placeholder="npub1…"
              onChange={(e) =>
                setRecovery({ type: "secondOwner", npub: e.target.value })
              }
            />
          </label>
        ) : recovery.pubkey ? (
          <p className="badge success" role="status">
            Backup key set up. Only its public key goes into the business.
          </p>
        ) : secret ? (
          <div className="backup-key">
            <p>
              <strong>This is shown once.</strong> Write it down or print it,
              and keep it offline. It is never stored, logged or sent.
            </p>
            <pre className="backup-secret" aria-label="Backup key">
              {nsec.match(/.{1,21}/g)!.join("\n")}
            </pre>
            <button
              className="button secondary small"
              onClick={() => window.print()}
            >
              <Printer size={14} />
              Print
            </button>
            <label>
              Type the last six characters of the backup key
              <input
                value={lastSix}
                maxLength={6}
                autoComplete="off"
                spellCheck={false}
                onChange={(e) => setLastSix(e.target.value)}
              />
            </label>
            <button className="button primary small" onClick={confirmBackup}>
              I've written it down
            </button>
          </div>
        ) : (
          <button className="button secondary" onClick={generateBackup}>
            Generate a backup key
          </button>
        )}
      </section>

      <section className="card setting-card">
        <h2>
          <ShieldCheck size={19} />
          3. The service key
        </h2>
        <p>
          This business's books will be signed by this key. Check it matches the
          one you were given for the service.
        </p>
        <pre className="fingerprint" aria-label="Service key fingerprint">
          {fingerprint(SERVICE_PUBKEY)}
        </pre>
        <p className="small muted">
          <code>{npub(SERVICE_PUBKEY)}</code>
        </p>
        <label className="check-row">
          <input
            type="checkbox"
            checked={serviceOk}
            onChange={(e) => setServiceOk(e.target.checked)}
          />
          This matches the service key I was given
        </label>
      </section>

      {error && (
        <p role="alert" className="alert error">
          {error}
        </p>
      )}
      <div className="create-actions">
        {onCancel && (
          <button className="button secondary" onClick={onCancel}>
            Cancel
          </button>
        )}
        <button
          className="button primary"
          disabled={busy || !!secret}
          onClick={create}
        >
          {busy ? "Creating…" : "Create business"}
        </button>
      </div>
    </main>
  );
}
