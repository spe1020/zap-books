import { useEffect, useState } from "react";
import { KeyRound, LifeBuoy, LogOut } from "lucide-react";
import { PRODUCT_NAME } from "../config";
import type { MembershipChange, PendingChange } from "../domain/members";
import type { Guarded, RemoteLedgerClient } from "../remote";
import { PendingChanges, shortKey, type PeopleTools } from "./Members";

/** How often to fetch the pending changes again. */
const POLL_MS = 15_000;

/**
 * For someone signed in with a business's backup key. The key reads no
 * books, so this shows only what the service reports for it: the owners,
 * and the pending changes it can veto. The service checks every veto and
 * recovery.
 */
export function RecoveryMode({
  client,
  tools,
  onSignOut,
  onBack,
}: {
  client: RemoteLedgerClient;
  tools: PeopleTools;
  onSignOut: () => void;
  /** Back to the books, when this key is also a member somewhere. */
  onBack?: () => void;
}) {
  const [guarded, setGuarded] = useState<Guarded[]>(client.guarded);
  const [busy, setBusy] = useState(false),
    [error, setError] = useState(""),
    [notice, setNotice] = useState("");
  useEffect(() => {
    let live = true;
    const fetch = () =>
      client
        .fetchGuarded()
        .then((g) => live && setGuarded(g))
        .catch((e) => live && setError((e as Error).message));
    const timer = setInterval(fetch, POLL_MS);
    window.addEventListener("focus", fetch);
    return () => {
      live = false;
      clearInterval(timer);
      window.removeEventListener("focus", fetch);
    };
  }, [client]);

  async function send(
    business: string,
    command:
      | { type: "Veto"; pending: string }
      | { type: "ChangeMembership"; change: MembershipChange },
    done: string,
  ) {
    setBusy(true);
    setError("");
    setNotice("");
    try {
      await client.proposeAsRecoveryKey(business, command);
      setGuarded(client.guarded);
      setNotice(done);
      return true;
    } catch (e) {
      setError((e as Error).message);
      return false;
    } finally {
      setBusy(false);
    }
  }

  const name = (k: string) =>
    k === tools.me ? "this backup key" : shortKey(tools.npub(k));
  return (
    <main className="recovery-mode">
      <header className="recovery-header">
        <div className="brand-mark">
          b<span>f</span>
        </div>
        <div>
          <h1>{PRODUCT_NAME}: backup key</h1>
          <small title={tools.npub(tools.me)}>
            Signed in as {shortKey(tools.npub(tools.me))} with{" "}
            {client.signer.name}
          </small>
        </div>
        {onBack && (
          <button className="button secondary" onClick={onBack}>
            Back to the books
          </button>
        )}
        <button
          className="icon-button"
          aria-label="Sign out"
          onClick={onSignOut}
        >
          <LogOut size={15} />
        </button>
      </header>
      <p className="alert warning" role="note">
        You're signed in with this business's backup key. It can only veto
        changes and recover a lost owner. It can't read the books.
      </p>
      {error && (
        <p role="alert" className="alert error">
          {error}
        </p>
      )}
      {notice && (
        <p role="status" className="alert">
          {notice}
        </p>
      )}
      {!guarded.length && (
        <p className="muted">This key isn't the backup key of any business.</p>
      )}
      {guarded.map((g) => (
        <GuardedBusiness
          key={g.business}
          g={g}
          tools={tools}
          name={name}
          busy={busy}
          onVeto={(p) =>
            void send(
              g.business,
              { type: "Veto", pending: p.proposal },
              "Change vetoed.",
            )
          }
          onRecover={(change) =>
            send(
              g.business,
              { type: "ChangeMembership", change },
              "Recovery proposed. The new owner key takes over in 48 hours unless an owner vetoes it.",
            )
          }
        />
      ))}
    </main>
  );
}

function GuardedBusiness({
  g,
  tools,
  name,
  busy,
  onVeto,
  onRecover,
}: {
  g: Guarded;
  tools: PeopleTools;
  name: (hex: string) => string;
  busy: boolean;
  onVeto: (p: PendingChange) => void;
  onRecover: (change: MembershipChange) => Promise<boolean>;
}) {
  const [lost, setLost] = useState(g.owners[0] ?? ""),
    [replacement, setReplacement] = useState(""),
    [understood, setUnderstood] = useState(false),
    [error, setError] = useState("");
  return (
    <section
      className="card setting-card"
      aria-label={`Business ${g.business.slice(0, 8)}`}
    >
      <h2>
        <KeyRound size={19} />
        Business {g.business.slice(0, 8)}…{g.business.slice(-4)}
      </h2>
      <h3>Pending changes</h3>
      <PendingChanges
        pending={g.pending}
        me={tools.me}
        name={name}
        busy={busy}
        onVeto={onVeto}
      />
      <h3>
        <LifeBuoy size={16} /> Recover an owner
      </h3>
      <p>
        Replace an owner key that was lost or stolen with a new one. The new key
        becomes an owner with full history after 48 hours, unless another owner
        vetoes it. The lost key can't.
      </p>
      <form
        className="invite-form"
        onSubmit={async (e) => {
          e.preventDefault();
          setError("");
          const parsed = tools.parseNpub(replacement.trim());
          if ("error" in parsed) {
            // A secret key pasted by mistake leaves the page at once.
            if (replacement.trim().startsWith("nsec")) setReplacement("");
            setError(parsed.error);
            return;
          }
          if (
            await onRecover({ op: "recover", lost, replacement: parsed.key })
          ) {
            setReplacement("");
            setUnderstood(false);
          }
        }}
      >
        <label>
          The lost owner
          <select value={lost} onChange={(e) => setLost(e.target.value)}>
            {g.owners.map((o) => (
              <option key={o} value={o}>
                {shortKey(tools.npub(o))}
              </option>
            ))}
          </select>
        </label>
        <label>
          The replacement owner's npub
          <input
            value={replacement}
            onChange={(e) => setReplacement(e.target.value)}
            placeholder="npub1…"
            spellCheck={false}
            autoComplete="off"
          />
        </label>
        <label className="check-row">
          <input
            type="checkbox"
            checked={understood}
            onChange={(e) => setUnderstood(e.target.checked)}
          />
          I understand the replacement takes over only after 48 hours, and only
          if no owner vetoes it first.
        </label>
        {error && (
          <p role="alert" className="alert error">
            {error}
          </p>
        )}
        <button
          className="button primary"
          disabled={busy || !understood || !lost}
        >
          Start the recovery
        </button>
      </form>
    </section>
  );
}
