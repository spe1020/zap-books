/**
 * What the members screens show beyond the membership itself, read from
 * acceptances sync has already verified: when each member joined, which
 * drafts an assistant created, and the recovery that made this key an
 * owner. Display only: no decision here is a security rule.
 */
import { OWNER_DELAY_SECS, type Membership } from "../domain/members";
import type { Opened } from "./protocol";

export interface People {
  /** When each current member joined: their acceptance's time, Unix seconds. */
  joined: Record<string, number>;
  /** Drafts an assistant created, by draft id, with the assistant's key. */
  byAssistant: Record<string, string>;
  /** The recovery that made the signed-in key an owner, if one did. */
  recovered?: {
    lost: string;
    /** When it was proposed, and when it took effect: Unix seconds. */
    proposedAt: number;
    activatedAt: number;
    /** The members added between the proposal and the recovery. */
    added: string[];
    dismissed?: boolean;
  };
}

export const noPeople = (): People => ({ joined: {}, byAssistant: {} });

/** `people` after one verified acceptance, read as the key `me`. */
export function observe(
  people: People,
  before: Membership | undefined,
  after: Membership,
  opened: Opened,
  me: string,
): People {
  const at = opened.header.createdAt;
  const joined: Record<string, number> = {};
  for (const key of Object.keys(after.members))
    joined[key] =
      before?.members[key] !== undefined ? (people.joined[key] ?? at) : at;
  const next: People = { ...people, joined };
  const proposer = opened.content.proposal.pubkey;
  const { command } = JSON.parse(opened.content.payload) as {
    command: {
      type: string;
      draft?: { id: string };
      change?: { op: string; lost?: string; replacement?: string };
    };
  };
  if (
    command.type === "SubmitReceiptDraft" &&
    command.draft &&
    before?.members[proposer]?.role === "assistant"
  )
    next.byAssistant = { ...people.byAssistant, [command.draft.id]: proposer };
  const change = command.change;
  if (
    opened.content.outcome.type === "activated" &&
    change?.op === "recover" &&
    change.replacement === me &&
    change.lost
  ) {
    const pending = before?.pending.find(
      (p) => p.proposal === opened.content.proposal.id,
    );
    if (pending) {
      const proposedAt = pending.effectiveAt - OWNER_DELAY_SECS;
      next.recovered = {
        lost: change.lost,
        proposedAt,
        activatedAt: at,
        added: Object.keys(after.members).filter(
          (k) => k !== me && joined[k] >= proposedAt,
        ),
      };
    }
  }
  return next;
}

/** The members added since the recovery was proposed who are still members. */
export function addedSinceRecovery(
  people: People,
  membership: Membership,
): string[] {
  return (people.recovered?.added ?? []).filter(
    (k) => membership.members[k] !== undefined,
  );
}
