import { describe, expect, test } from "vitest";
import {
  canVeto,
  OWNER_DELAY_SECS,
  timeLeft,
  type Membership,
  type PendingChange,
} from "../domain/members";
import { addedSinceRecovery, noPeople, observe, type People } from "./people";
import type { Opened } from "./protocol";

const key = (c: string) => c.repeat(64);
const [me, lost, backup, other, newcomer, bot] = [
  "a",
  "b",
  "c",
  "d",
  "e",
  "f",
].map(key);

function membership(
  members: Record<string, Membership["members"][string]["role"]>,
  pending: PendingChange[] = [],
): Membership {
  return {
    business: key("9"),
    members: Object.fromEntries(
      Object.entries(members).map(([k, role]) => [
        k,
        { role, history: "full", joinedEpoch: 0 },
      ]),
    ),
    recoveryKey: backup,
    service: key("8"),
    epoch: 0,
    pending,
  };
}

function opened(
  at: number,
  proposer: string,
  command: unknown,
  outcome = "applied",
  id = key("7"),
): Opened {
  return {
    header: {
      business: key("9"),
      seq: 1,
      prev: null,
      proposal: id,
      epoch: 0,
      nextService: null,
      createdAt: at,
    },
    content: {
      proposal: {
        id,
        pubkey: proposer,
        created_at: at,
        kind: 4130,
        tags: [],
        content: "",
        sig: "",
      },
      payload: JSON.stringify({ command }),
      outcome: { type: outcome },
    },
  };
}

describe("what sync records for the members screens", () => {
  test("join times: kept through role changes, gone on removal", () => {
    let people = observe(
      noPeople(),
      undefined,
      membership({ [me]: "owner" }),
      opened(100, me, { type: "CreateBusiness" }),
      me,
    );
    const two = membership({ [me]: "owner", [other]: "viewer" });
    people = observe(
      people,
      membership({ [me]: "owner" }),
      two,
      opened(200, me, { type: "ChangeMembership" }),
      me,
    );
    expect(people.joined).toEqual({ [me]: 100, [other]: 200 });
    const changed = membership({ [me]: "owner", [other]: "bookkeeper" });
    people = observe(
      people,
      two,
      changed,
      opened(300, me, { type: "ChangeMembership" }),
      me,
    );
    expect(people.joined[other]).toBe(200);
    people = observe(
      people,
      changed,
      membership({ [me]: "owner" }),
      opened(400, me, { type: "ChangeMembership" }),
      me,
    );
    expect(people.joined).toEqual({ [me]: 100 });
  });

  test("a draft is the assistant's only when an assistant submitted it", () => {
    const state = membership({ [me]: "owner", [bot]: "assistant" });
    const draft = { id: "d1" };
    let people: People = { ...noPeople(), joined: { [me]: 1, [bot]: 1 } };
    people = observe(
      people,
      state,
      state,
      opened(5, bot, { type: "SubmitReceiptDraft", draft }),
      me,
    );
    expect(people.byAssistant).toEqual({ d1: bot });
    people = observe(
      people,
      state,
      state,
      opened(6, me, { type: "SubmitReceiptDraft", draft: { id: "d2" } }),
      me,
    );
    expect(people.byAssistant).toEqual({ d1: bot });
  });

  test("a recovery naming this key lists the members added since it was proposed", () => {
    const proposedAt = 1_000;
    const recover = { op: "recover", lost, replacement: me };
    const pending: PendingChange = {
      proposal: key("7"),
      proposer: backup,
      change: { op: "recover", lost, replacement: me },
      effectiveAt: proposedAt + OWNER_DELAY_SECS,
    };
    const before = membership(
      { [lost]: "owner", [other]: "bookkeeper", [newcomer]: "viewer" },
      [pending],
    );
    const people: People = {
      ...noPeople(),
      joined: {
        [lost]: 10,
        [other]: proposedAt - 1,
        [newcomer]: proposedAt + 60,
      },
    };
    const after = membership({
      [me]: "owner",
      [other]: "bookkeeper",
      [newcomer]: "viewer",
    });
    const next = observe(
      people,
      before,
      after,
      opened(
        pending.effectiveAt,
        backup,
        { type: "ChangeMembership", change: recover },
        "activated",
      ),
      me,
    );
    expect(next.recovered).toMatchObject({
      lost,
      proposedAt,
      added: [newcomer],
    });
    expect(addedSinceRecovery(next, after)).toEqual([newcomer]);
    // Once removed, they leave the list.
    const without = membership({ [me]: "owner", [other]: "bookkeeper" });
    expect(addedSinceRecovery(next, without)).toEqual([]);
    // Another key's recovery shows nothing here.
    expect(
      observe(
        people,
        before,
        after,
        opened(
          pending.effectiveAt,
          backup,
          { type: "ChangeMembership", change: recover },
          "activated",
        ),
        other,
      ).recovered,
    ).toBeUndefined();
  });
});

describe("pending changes", () => {
  const change: PendingChange = {
    proposal: key("7"),
    proposer: me,
    change: { op: "add", member: newcomer, role: "owner", history: "full" },
    effectiveAt: 0,
  };
  const recovery: PendingChange = {
    proposal: key("6"),
    proposer: backup,
    change: { op: "recover", lost: me, replacement: newcomer },
    effectiveAt: 0,
  };
  const state = membership({
    [me]: "owner",
    [other]: "owner",
    [bot]: "assistant",
  });

  test("owners and the backup key veto, but not their own change or a recovery of themselves", () => {
    expect(canVeto(state, other, change)).toBe(true);
    expect(canVeto(state, backup, change)).toBe(true);
    expect(canVeto(state, me, change)).toBe(false);
    expect(canVeto(state, bot, change)).toBe(false);
    expect(canVeto(state, me, recovery)).toBe(false);
    expect(canVeto(state, other, recovery)).toBe(true);
  });

  test("the countdown", () => {
    expect(timeLeft(OWNER_DELAY_SECS)).toBe("2 days 0 hours");
    expect(timeLeft(OWNER_DELAY_SECS - 3600)).toBe("1 day 23 hours");
    expect(timeLeft(3 * 3600 + 60)).toBe("3 hours 1 minute");
    expect(timeLeft(30)).toBe("under a minute");
    expect(timeLeft(0)).toBe("due now");
  });
});
