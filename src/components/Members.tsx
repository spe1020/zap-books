import { useEffect, useState } from "react";
import { Clock, KeyRound, ShieldAlert, UserPlus, Users } from "lucide-react";
import {
  aRole,
  canVeto,
  invitableRoles,
  roleLabels,
  timeLeft,
  type Membership,
  type MembershipChange,
  type PendingChange,
  type Role,
} from "../domain/members";
import type { People } from "../remote/people";
import { addedSinceRecovery } from "../remote/people";
import type { Mutation } from "./Review";
import { Modal } from "./common";

/** What the members screens need from the signed-in client. */
export interface PeopleTools {
  me: string;
  npub: (hex: string) => string;
  /** The hex key of an npub, or an error message. */
  parseNpub: (text: string) => { key: string } | { error: string };
  isOwnerLevel: (change: MembershipChange) => boolean;
}

export const ASSISTANT_WARNING =
  "An AI assistant can read the whole of this business's books, within the history you choose. If it runs on someone else's computer or service, that data goes there. It can create drafts and suggestions, but it can never approve, correct or import.";
export const OWNER_LEVEL_NOTE =
  "This change takes effect in 48 hours unless vetoed.";

/** A key in short: `npub1abcd…wxyz`. */
export function shortKey(npub: string) {
  return `${npub.slice(0, 10)}…${npub.slice(-6)}`;
}

export const longDate = (secs: number) =>
  new Date(secs * 1000).toLocaleDateString(undefined, { dateStyle: "long" });

const dateTime = (secs: number) =>
  new Date(secs * 1000).toLocaleString(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  });

/** Now in Unix seconds, refreshed every 30 seconds, for countdowns. */
export function useNow() {
  const [now, setNow] = useState(() => Math.floor(Date.now() / 1000));
  useEffect(() => {
    const t = setInterval(() => setNow(Math.floor(Date.now() / 1000)), 30_000);
    return () => clearInterval(t);
  }, []);
  return now;
}

/** A pending change in words. */
export function describePending(
  change: MembershipChange,
  name: (hex: string) => string,
) {
  switch (change.op) {
    case "add":
      return `Add ${name(change.member)} as ${aRole(change.role)}`;
    case "remove":
      return `Remove ${name(change.member)}`;
    case "changeRole":
      return `Make ${name(change.member)} ${aRole(change.role)}`;
    case "replaceRecoveryKey":
      return change.key
        ? `Replace the backup key with ${name(change.key)}`
        : "Drop the backup key";
    case "replaceServiceKey":
      return `Move the books to service key ${name(change.key)}`;
    case "recover":
      return name(change.lost) === "you"
        ? `Replace your owner key with ${name(change.replacement)}`
        : `Recover owner ${name(change.lost)}, replacing it with ${name(change.replacement)}`;
  }
}

/** Pending owner-level changes with their countdown and, where allowed, a veto. */
export function PendingChanges({
  pending,
  membership,
  me,
  name,
  onVeto,
  busy,
}: {
  pending: PendingChange[];
  /** For who may veto. The recovery key has none: it may veto all but its own. */
  membership?: Membership;
  me: string;
  name: (hex: string) => string;
  onVeto: (p: PendingChange) => void;
  busy: boolean;
}) {
  const now = useNow();
  if (!pending.length) return <p className="muted">No changes are waiting.</p>;
  return (
    <ul className="pending-list">
      {pending.map((p) => {
        const vetoable = membership
          ? canVeto(membership, me, p)
          : p.proposer !== me;
        const lostOwner = p.change.op === "recover" && p.change.lost === me;
        return (
          <li key={p.proposal} className="pending-item">
            <div>
              <strong>{describePending(p.change, name)}</strong>
              <small>
                Proposed by{" "}
                {p.proposer !== me
                  ? name(p.proposer)
                  : membership
                    ? "you"
                    : "this backup key"}
              </small>
              <small className="countdown">
                <Clock size={13} />
                Takes effect in {timeLeft(p.effectiveAt - now)} (
                {dateTime(p.effectiveAt)})
              </small>
              {lostOwner && (
                <small className="muted">
                  This recovery names your key as the lost owner, so you can't
                  veto it. Any other owner can.
                </small>
              )}
              {p.proposer === me && (
                <small className="muted">
                  {membership
                    ? "You proposed this, so another owner or the backup key would have to veto it."
                    : "Any owner but the one it names as lost can still veto it."}
                </small>
              )}
            </div>
            {vetoable && (
              <button
                className="button danger"
                disabled={busy}
                onClick={() => onVeto(p)}
                aria-label={`Veto: ${describePending(p.change, name)}`}
              >
                Veto
              </button>
            )}
          </li>
        );
      })}
    </ul>
  );
}

/**
 * Shown to the new owner after a recovery, until dismissed: the members
 * added while the recovery waited, each with a remove action.
 */
export function RecoveryBanner({
  people,
  membership,
  tools,
  mutate,
  busy,
  onDismiss,
}: {
  people: People;
  membership: Membership;
  tools: PeopleTools;
  mutate: Mutation;
  busy: boolean;
  onDismiss: () => void;
}) {
  const r = people.recovered;
  if (!r || r.dismissed) return null;
  const added = addedSinceRecovery(people, membership);
  return (
    <section
      className="alert warning recovery-banner"
      aria-label="Your business was recovered"
    >
      <ShieldAlert size={18} />
      <div>
        <strong>
          Your business was recovered. Review the members added since{" "}
          {longDate(r.proposedAt)}.
        </strong>
        {added.length ? (
          <ul className="recovery-added">
            {added.map((k) => {
              const m = membership.members[k];
              const change: MembershipChange = { op: "remove", member: k };
              const later = tools.isOwnerLevel(change);
              return (
                <li key={k}>
                  <span title={tools.npub(k)}>{shortKey(tools.npub(k))}</span>
                  <span>{roleLabels[m.role]}</span>
                  <span>Added {longDate(people.joined[k])}</span>
                  <button
                    className="button secondary"
                    disabled={busy}
                    onClick={() =>
                      void mutate(
                        [{ type: "ChangeMembership", change }],
                        later
                          ? "Removal proposed. It takes effect in 48 hours unless vetoed."
                          : "Member removed.",
                      )
                    }
                  >
                    Remove {shortKey(tools.npub(k))}
                  </button>
                  {later && <small>{OWNER_LEVEL_NOTE}</small>}
                </li>
              );
            })}
          </ul>
        ) : (
          <p>No members were added in that time.</p>
        )}
        <button className="text-button" onClick={onDismiss}>
          Dismiss
        </button>
      </div>
    </section>
  );
}

type Confirm = { title: string; change: MembershipChange; done: string };

/** The members of a business on the service: list, invite, roles, removal. */
export function Members({
  membership,
  people,
  tools,
  mutate,
  busy,
}: {
  membership: Membership;
  people: People;
  tools: PeopleTools;
  mutate: Mutation;
  busy: boolean;
}) {
  const { me, npub } = tools;
  const owner = membership.members[me]?.role === "owner";
  const name = (k: string) =>
    k === me
      ? "you"
      : k === membership.recoveryKey
        ? "the backup key"
        : shortKey(npub(k));
  const [invitee, setInvitee] = useState(""),
    [role, setRole] = useState<Role>("bookkeeper"),
    [error, setError] = useState(""),
    [confirm, setConfirm] = useState<Confirm>();
  const members = Object.entries(membership.members).sort(
    ([a], [b]) =>
      (people.joined[a] ?? 0) - (people.joined[b] ?? 0) || a.localeCompare(b),
  );
  // From-join history can't be rebuilt in the browser yet
  // (docs/protocol.md, Epochs), so every invite is for full history.
  const invite: MembershipChange | undefined = (() => {
    const parsed = tools.parseNpub(invitee.trim());
    return "key" in parsed
      ? { op: "add", member: parsed.key, role, history: "full" }
      : undefined;
  })();
  const inviteLater = invite ? tools.isOwnerLevel(invite) : role === "owner";

  async function send(change: MembershipChange, done: string) {
    return mutate([{ type: "ChangeMembership", change }], done);
  }

  return (
    <div className="settings members">
      <section className="card setting-card">
        <h2>
          <Users size={19} />
          Members
        </h2>
        <p>
          Everyone who can read or change these books. Each member signs with
          their own key.
        </p>
        <table className="members-table">
          <thead>
            <tr>
              <th>Member</th>
              <th>Role</th>
              <th>History</th>
              <th>Joined</th>
              {owner && <th aria-label="Actions" />}
            </tr>
          </thead>
          <tbody>
            {members.map(([k, m]) => (
              <tr key={k}>
                <td title={npub(k)}>
                  {shortKey(npub(k))}
                  {k === me && <span className="badge neutral">You</span>}
                </td>
                <td>
                  {owner && k !== me ? (
                    <select
                      aria-label={`Role for ${shortKey(npub(k))}`}
                      value={m.role}
                      disabled={busy}
                      onChange={(e) => {
                        const next = e.target.value as Role;
                        setConfirm({
                          title: `Make ${shortKey(npub(k))} ${aRole(next)}?`,
                          change: { op: "changeRole", member: k, role: next },
                          done: "Role changed.",
                        });
                      }}
                    >
                      {(invitableRoles as readonly Role[])
                        .concat(m.role === "submitter" ? ["submitter"] : [])
                        .map((r) => (
                          <option key={r} value={r}>
                            {roleLabels[r]}
                          </option>
                        ))}
                    </select>
                  ) : (
                    roleLabels[m.role]
                  )}
                </td>
                <td>
                  {m.history === "full" ? "Full history" : "From join date"}
                </td>
                <td>{people.joined[k] ? longDate(people.joined[k]) : "—"}</td>
                {owner && (
                  <td>
                    {k !== me && (
                      <button
                        className="button secondary"
                        disabled={busy}
                        onClick={() =>
                          setConfirm({
                            title: `Remove ${shortKey(npub(k))}?`,
                            change: { op: "remove", member: k },
                            done: "Member removed.",
                          })
                        }
                      >
                        Remove
                      </button>
                    )}
                  </td>
                )}
              </tr>
            ))}
          </tbody>
        </table>
        <div className="static-field">
          <span>Recovery</span>
          <strong>
            {membership.recoveryKey
              ? `Backup key ${shortKey(npub(membership.recoveryKey))}`
              : "A second owner"}
          </strong>
        </div>
      </section>
      {owner && (
        <section className="card setting-card">
          <h2>
            <UserPlus size={19} />
            Invite a member
          </h2>
          <p>Add someone by their npub. They sign in with their own key.</p>
          <form
            className="invite-form"
            onSubmit={async (e) => {
              e.preventDefault();
              setError("");
              const parsed = tools.parseNpub(invitee.trim());
              if ("error" in parsed) {
                // A secret key pasted by mistake leaves the page at once.
                if (invitee.trim().startsWith("nsec")) setInvitee("");
                setError(parsed.error);
                return;
              }
              const later = tools.isOwnerLevel(invite!);
              if (
                await send(
                  invite!,
                  later
                    ? "Invitation proposed. It takes effect in 48 hours unless vetoed."
                    : `${roleLabels[role]} added.`,
                )
              )
                setInvitee("");
            }}
          >
            <label>
              Their npub
              <input
                value={invitee}
                onChange={(e) => setInvitee(e.target.value)}
                placeholder="npub1…"
                spellCheck={false}
                autoComplete="off"
              />
            </label>
            <label>
              Role
              <select
                value={role}
                onChange={(e) => setRole(e.target.value as Role)}
              >
                {invitableRoles.map((r) => (
                  <option key={r} value={r}>
                    {roleLabels[r]}
                  </option>
                ))}
              </select>
            </label>
            <p className="small muted">
              New members can read the books' full history.
            </p>
            {role === "assistant" && (
              <p className="alert warning" role="note">
                {ASSISTANT_WARNING}
              </p>
            )}
            {inviteLater && (
              <p className="alert warning" role="note">
                {OWNER_LEVEL_NOTE}
              </p>
            )}
            {error && (
              <p role="alert" className="alert error">
                {error}
              </p>
            )}
            <button className="button primary" disabled={busy}>
              Invite
            </button>
          </form>
        </section>
      )}
      {owner && (
        <section className="card setting-card">
          <h2>
            <KeyRound size={19} />
            Pending changes
          </h2>
          <p>
            Owner-level changes wait 48 hours. Any other owner, or the backup
            key, can veto one before then.
          </p>
          <PendingChanges
            pending={membership.pending}
            membership={membership}
            me={me}
            name={name}
            busy={busy}
            onVeto={(p) =>
              void mutate(
                [{ type: "Veto", pending: p.proposal }],
                "Change vetoed.",
              )
            }
          />
        </section>
      )}
      {confirm && (
        <Modal title={confirm.title} onClose={() => setConfirm(undefined)}>
          <div className="modal-form">
            {confirm.change.op === "changeRole" &&
              confirm.change.role === "assistant" && (
                <p className="alert warning" role="note">
                  {ASSISTANT_WARNING}
                </p>
              )}
            <p
              className={
                tools.isOwnerLevel(confirm.change) ? "alert warning" : ""
              }
              role="note"
            >
              {tools.isOwnerLevel(confirm.change)
                ? OWNER_LEVEL_NOTE
                : confirm.change.op === "remove"
                  ? "They lose access at once. What they already read stays with them; new entries use a new key."
                  : "This takes effect at once."}
            </p>
            <button
              className="button primary"
              disabled={busy}
              onClick={async () => {
                const later = tools.isOwnerLevel(confirm.change);
                if (
                  await send(
                    confirm.change,
                    later
                      ? "Change proposed. It takes effect in 48 hours unless vetoed."
                      : confirm.done,
                  )
                )
                  setConfirm(undefined);
              }}
            >
              {confirm.change.op === "remove" ? "Remove" : "Change role"}
            </button>
          </div>
        </Modal>
      )}
    </div>
  );
}
