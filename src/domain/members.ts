/**
 * Membership as the ledger protocol defines it (`docs/protocol.md`,
 * Membership), for showing and proposing changes. The rules themselves run
 * in `crates/protocol`: the service applies them, and the browser re-checks
 * them while it syncs.
 */

export type Role =
  "owner" | "bookkeeper" | "viewer" | "submitter" | "assistant";

/** The roles an owner can invite. Submitters wait for their own screens. */
export const invitableRoles = [
  "owner",
  "bookkeeper",
  "viewer",
  "assistant",
] as const satisfies readonly Role[];

export const roleLabels: Record<Role, string> = {
  owner: "Owner",
  bookkeeper: "Bookkeeper",
  viewer: "Viewer",
  submitter: "Submitter",
  assistant: "AI assistant",
};

/** "an owner", "a bookkeeper", "an AI assistant". */
export const aRole = (role: Role) =>
  ({
    owner: "an owner",
    bookkeeper: "a bookkeeper",
    viewer: "a viewer",
    submitter: "a submitter",
    assistant: "an AI assistant",
  })[role];

export type History = "full" | "fromJoin";

/** A `ChangeMembership` command's change. Keys are lowercase hex. */
export type MembershipChange =
  | { op: "add"; member: string; role: Role; history: History }
  | { op: "remove"; member: string }
  | { op: "changeRole"; member: string; role: Role }
  | { op: "replaceRecoveryKey"; key: string | null }
  | { op: "replaceServiceKey"; key: string }
  | { op: "recover"; lost: string; replacement: string };

export interface Member {
  role: Role;
  history: History;
  joinedEpoch: number;
}

/** An owner-level change waiting out its 48 hours. */
export interface PendingChange {
  /** The proposal's event id: what a veto names. */
  proposal: string;
  proposer: string;
  change: MembershipChange;
  /** Unix seconds. */
  effectiveAt: number;
}

/** The protocol's `MembershipState`, as the browser verified it. */
export interface Membership {
  business: string;
  members: Record<string, Member>;
  recoveryKey: string | null;
  service: string;
  epoch: number;
  pending: PendingChange[];
  assistantDrafts?: Record<string, string>;
}

/** How long an owner-level change waits, as the protocol fixes it. */
export const OWNER_DELAY_SECS = 48 * 60 * 60;

/** Who may veto a pending change, as `MembershipState::veto` decides. */
export function canVeto(
  membership: Membership,
  key: string,
  p: PendingChange,
): boolean {
  const owner = membership.members[key]?.role === "owner";
  if (!owner && membership.recoveryKey !== key) return false;
  if (p.proposer === key) return false;
  return !(p.change.op === "recover" && p.change.lost === key);
}

/** "2 days 3 hours", "4 hours 10 minutes", "under a minute". */
export function timeLeft(seconds: number): string {
  if (seconds <= 0) return "due now";
  const days = Math.floor(seconds / 86_400);
  const hours = Math.floor((seconds % 86_400) / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const unit = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`;
  if (days) return `${unit(days, "day")} ${unit(hours, "hour")}`;
  if (hours) return `${unit(hours, "hour")} ${unit(minutes, "minute")}`;
  return minutes ? unit(minutes, "minute") : "under a minute";
}
