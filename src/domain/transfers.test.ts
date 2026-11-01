import { describe, expect, test } from "vitest";
import type { BusinessData, TransferType } from "./types";
import { seedWorkspace } from "./seed";
import { updateSettings } from "./settings";
import {
  approve,
  blankDraft,
  categoryTotals,
  saveDraft,
  totals,
} from "./ledger";
import {
  balances,
  correctTransfer,
  openingBalance,
  openingBalanceReplacement,
  paymentAccounts,
  transferAccounts,
  recordOpeningBalance,
  recordTransfer,
  type TransferInput,
} from "./transfers";

/** Northstar's accounts and categories with no entries, plus a loan and an equity account. */
function setup(): BusinessData {
  let b: BusinessData = {
    ...seedWorkspace().businesses[0],
    drafts: [],
    journals: [],
    approvals: [],
    payments: [],
  };
  b = updateSettings(b, {
    op: "addAccount",
    name: "Loan from owner",
    kind: "liability",
    currency: "USD",
  });
  b = updateSettings(b, {
    op: "addAccount",
    name: "Owner contributions",
    kind: "equity",
    currency: "USD",
  });
  return b;
}

const account = (b: BusinessData, name: string) =>
  b.accounts.find((a) => a.name === name)!.id;
const ids = (b: BusinessData) => ({
  bank: account(b, "Operating account"),
  card: account(b, "Business card"),
  reserve: account(b, "Reserve account"),
  euro: account(b, "Euro account"),
  loan: account(b, "Loan from owner"),
  equity: account(b, "Owner contributions"),
  expense: b.categories[0].accountId,
});
const balanceOf = (b: BusinessData, accountId: string, asOf = "2026-12-31") =>
  balances(b, asOf, "USD").find((r) => r.accountId === accountId)!.balance;

const transfer = (
  transferType: TransferType,
  from: string,
  to: string,
  extra: Partial<TransferInput> = {},
): TransferInput => ({
  date: "2026-03-10",
  amountMinor: 50000,
  currency: "USD",
  fromAccountId: from,
  toAccountId: to,
  transferType,
  purpose: "Invented purpose",
  ...extra,
});

function expense(b: BusinessData, accountId: string, amountMinor = 1234) {
  const draft = {
    ...blankDraft(b),
    vendor: "Invented Supplies",
    description: "Invented paper",
    purpose: "Office use",
    categoryId: b.categories[0].id,
    accountId,
    amountMinor,
    date: "2026-03-12",
  };
  return approve(saveDraft(b, draft), draft.id);
}

describe("owner funding merge gates", () => {
  test("a personally paid expense leaves the bank unchanged and raises the loan", () => {
    const b = setup();
    const { bank, loan, equity } = ids(b);
    const paid = expense(b, loan, 1234);
    expect(balanceOf(paid, bank)).toBe(0);
    expect(balanceOf(paid, loan)).toBe(1234);
    expect(totals(paid, "2026-03")).toEqual({ USD: 1234 });
    const contributed = expense(b, equity, 999);
    expect(balanceOf(contributed, bank)).toBe(0);
    expect(balanceOf(contributed, equity)).toBe(999);
  });

  test("a loan deposit raises the bank and the loan; a repayment lowers both", () => {
    const b = setup();
    const { bank, loan } = ids(b);
    const lent = recordTransfer(b, transfer("ownerLoan", loan, bank));
    expect(balanceOf(lent, bank)).toBe(50000);
    expect(balanceOf(lent, loan)).toBe(50000);
    const repaid = recordTransfer(
      lent,
      transfer("loanRepayment", bank, loan, { amountMinor: 20000 }),
    );
    expect(balanceOf(repaid, bank)).toBe(30000);
    expect(balanceOf(repaid, loan)).toBe(30000);
  });

  test("transfers never count as spending", () => {
    let b = setup();
    const { bank, card, reserve, loan, equity } = ids(b);
    b = recordTransfer(b, transfer("ownerLoan", loan, bank));
    b = recordTransfer(b, transfer("ownerContribution", equity, card));
    b = recordTransfer(b, transfer("loanRepayment", bank, loan));
    b = recordTransfer(b, transfer("ownerDraw", card, equity));
    b = recordTransfer(b, transfer("betweenAccounts", bank, reserve));
    b = recordOpeningBalance(b, {
      accountId: bank,
      date: "2026-01-01",
      amountMinor: 70000,
    });
    expect(totals(b, "2026")).toEqual({});
    expect(categoryTotals(b, "2026", "USD")).toEqual([]);
  });

  test("every wrong account for each transfer type is refused with its message", () => {
    const b = setup();
    const a = ids(b);
    const kinds = {
      bank: a.bank,
      card: a.card,
      loan: a.loan,
      equity: a.equity,
    };
    const valid: Record<string, [string[], string[]]> = {
      ownerLoan: [["loan"], ["bank", "card"]],
      ownerContribution: [["equity"], ["bank", "card"]],
      loanRepayment: [["bank", "card"], ["loan"]],
      ownerDraw: [["bank", "card"], ["equity"]],
      betweenAccounts: [
        ["bank", "card"],
        ["bank", "card"],
      ],
    };
    const messages: Record<string, string> = {
      ownerLoan:
        "An owner loan moves money from a loan account into a bank or card account.",
      ownerContribution:
        "An owner contribution moves money from an owner equity account into a bank or card account.",
      loanRepayment:
        "A loan repayment moves money from a bank or card account into a loan account.",
      ownerDraw:
        "An owner draw moves money from a bank or card account into an owner equity account.",
      betweenAccounts:
        "A transfer between accounts moves money from one bank or card account to another.",
    };
    let checked = 0;
    for (const [type, [froms, tos]] of Object.entries(valid))
      for (const [fromKind, from] of Object.entries(kinds))
        for (const [toKind, to] of Object.entries(kinds)) {
          if (from === to) continue;
          const input = transfer(type as TransferType, from, to);
          if (froms.includes(fromKind) && tos.includes(toKind))
            expect(() => recordTransfer(b, input)).not.toThrow();
          else {
            expect(() => recordTransfer(b, input)).toThrow(messages[type]);
            checked++;
          }
        }
    expect(checked).toBeGreaterThan(30);
    // An expense account is never a transfer's account.
    expect(() =>
      recordTransfer(b, transfer("ownerLoan", a.loan, a.expense)),
    ).toThrow(messages.ownerLoan);
  });

  test("correcting a transfer reverses it exactly and links the replacement", () => {
    const b = setup();
    const { bank, card, loan } = ids(b);
    const lent = recordTransfer(b, transfer("ownerLoan", loan, bank));
    const original = lent.journals.at(-1)!;
    const fixed = correctTransfer(
      lent,
      original.id,
      transfer("ownerLoan", loan, card, { amountMinor: 45000 }),
      "Deposited to the card account",
    );
    const reversal = fixed.journals.find((j) => j.reversesId === original.id)!;
    const replacement = fixed.journals.find(
      (j) => j.correctionOf === original.id,
    )!;
    expect(reversal.kind).toBe("reversal");
    expect(reversal.lines).toEqual(
      original.lines.map((l) => ({ ...l, debit: l.credit, credit: l.debit })),
    );
    expect(replacement.kind).toBe("transfer");
    // The original and its reversal net to zero on both accounts.
    expect(balanceOf(fixed, bank)).toBe(0);
    // Money paid into a card leaves it in credit: less than nothing owed.
    expect(balanceOf(fixed, card)).toBe(-45000);
    expect(balanceOf(fixed, loan)).toBe(45000);
    expect(fixed.events.at(-1)).toMatchObject({
      type: "transfer.reversed",
      entityId: original.id,
      sourceIds: [original.id, reversal.id, replacement.id],
    });
    // A second correction of the same transfer is refused, and so is
    // correcting a reversal.
    expect(() =>
      correctTransfer(
        fixed,
        original.id,
        transfer("ownerLoan", loan, bank),
        "Again",
      ),
    ).toThrow("This transfer has already been corrected.");
    expect(() =>
      correctTransfer(
        fixed,
        reversal.id,
        transfer("ownerLoan", loan, bank),
        "Again",
      ),
    ).toThrow("A posted transfer and correction reason are required.");
    // An invalid replacement leaves everything as it was.
    expect(() =>
      correctTransfer(
        lent,
        original.id,
        transfer("ownerLoan", bank, loan),
        "Wrong way round",
      ),
    ).toThrow("An owner loan moves money");
    expect(lent.journals).toHaveLength(1);
  });

  test("a transfer between currencies, or to the same account, is refused", () => {
    const b = setup();
    const { bank, reserve, euro } = ids(b);
    expect(() =>
      recordTransfer(b, transfer("betweenAccounts", bank, euro)),
    ).toThrow("Both accounts must be in the transfer's currency.");
    expect(() =>
      recordTransfer(
        b,
        transfer("betweenAccounts", reserve, bank, { currency: "EUR" }),
      ),
    ).toThrow("Both accounts must be in the transfer's currency.");
    expect(() =>
      recordTransfer(b, transfer("betweenAccounts", bank, bank)),
    ).toThrow("Choose two different accounts.");
  });

  test("an opening balance dated after the first entry is refused", () => {
    const b = setup();
    const { bank, card, loan } = ids(b);
    const started = expense(b, card); // dated 2026-03-12
    expect(() =>
      recordOpeningBalance(started, {
        accountId: bank,
        date: "2026-03-13",
        amountMinor: 11700,
      }),
    ).toThrow(
      "An opening balance must be dated on or before the first entry, 2026-03-12.",
    );
    const opened = recordOpeningBalance(started, {
      accountId: bank,
      date: "2026-03-12",
      amountMinor: 11700,
    });
    expect(balanceOf(opened, bank)).toBe(11700);
    // One per account; other accounts can still have one.
    expect(() =>
      recordOpeningBalance(opened, {
        accountId: bank,
        date: "2026-01-01",
        amountMinor: 1,
      }),
    ).toThrow(
      "This account already has an opening balance. Correct it instead.",
    );
    const owed = recordOpeningBalance(opened, {
      accountId: loan,
      date: "2026-01-01",
      amountMinor: 25000,
    });
    expect(balanceOf(owed, loan)).toBe(25000);
    expect(balanceOf(owed, account(owed, "Opening balances"))).toBe(-13300);
  });

  test("balances are right before, between and after entries, with reversals", () => {
    let b = setup();
    const { bank, card, loan } = ids(b);
    b = recordOpeningBalance(b, {
      accountId: bank,
      date: "2026-01-01",
      amountMinor: 100000,
    });
    b = recordTransfer(
      b,
      transfer("ownerLoan", loan, bank, { date: "2026-02-01" }),
    );
    const loanJournal = b.journals.at(-1)!;
    b = expense(b, card, 2000); // 2026-03-12
    b = correctTransfer(
      b,
      loanJournal.id,
      transfer("ownerLoan", loan, bank, {
        date: "2026-02-01",
        amountMinor: 30000,
      }),
      "Smaller loan",
    );
    expect(balanceOf(b, bank, "2025-12-31")).toBe(0);
    expect(balanceOf(b, bank, "2026-01-01")).toBe(100000);
    expect(balanceOf(b, loan, "2026-01-31")).toBe(0);
    expect(balanceOf(b, bank, "2026-02-01")).toBe(130000);
    expect(balanceOf(b, loan, "2026-02-01")).toBe(30000);
    expect(balanceOf(b, card, "2026-03-11")).toBe(0);
    expect(balanceOf(b, card, "2026-03-12")).toBe(2000);
    expect(balanceOf(b, bank, "2027-01-01")).toBe(130000);
    expect(balances(b, "2026-12-31", "EUR").map((r) => r.name)).toEqual([
      "Euro account",
    ]);
    expect(() => balances(b, "2026-02-30", "USD")).toThrow(
      "Choose a valid date.",
    );
  });
  test("a card's balance is what's owed: purchases raise it, payments lower it", () => {
    let b = setup();
    const { bank, card } = ids(b);
    b = expense(b, card, 2500);
    expect(balanceOf(b, card)).toBe(2500);
    b = expense(b, card, 1500);
    expect(balanceOf(b, card)).toBe(4000);
    b = recordTransfer(
      b,
      transfer("betweenAccounts", bank, card, { amountMinor: 3000 }),
    );
    expect(balanceOf(b, card)).toBe(1000);
    expect(balanceOf(b, bank)).toBe(-3000);
  });

  test("a card's opening balance is entered as the amount owed", () => {
    const b = setup();
    const { card } = ids(b);
    const owed = recordOpeningBalance(b, {
      accountId: card,
      date: "2026-01-01",
      amountMinor: 8000,
    });
    expect(balanceOf(owed, card)).toBe(8000);
    // The card is credited, as a purchase would credit it.
    const line = owed.journals.at(-1)!.lines.find((l) => l.accountId === card)!;
    expect(line).toMatchObject({ debit: 0, credit: 8000 });
    expect(balanceOf(expense(owed, card, 500), card)).toBe(8500);
  });

  describe("correcting an opening balance", () => {
    function opened() {
      const b = setup();
      const { bank } = ids(b);
      const next = recordOpeningBalance(b, {
        accountId: bank,
        date: "2026-01-01",
        amountMinor: 12300,
      });
      const journal = next.journals.at(-1)!;
      const opening = account(next, "Opening balances");
      return { b: next, journal, opening, ...ids(next) };
    }
    const replacement = (
      from: string,
      to: string,
      extra: Partial<TransferInput> = {},
    ) =>
      transfer("openingBalance", from, to, {
        date: "2026-01-01",
        purpose: "Opening balance",
        ...extra,
      });

    test("may change the amount, its direction and the date", () => {
      const { b, journal, opening, bank } = opened();
      const smaller = correctTransfer(
        b,
        journal.id,
        replacement(opening, bank, { amountMinor: 9000, date: "2025-12-31" }),
        "Statement misread",
      );
      expect(balanceOf(smaller, bank)).toBe(9000);
      const overdrawn = correctTransfer(
        b,
        journal.id,
        replacement(bank, opening, { amountMinor: 700 }),
        "Overdrawn",
      );
      expect(balanceOf(overdrawn, bank)).toBe(-700);
    });

    test("refuses a replacement that isn't an opening balance", () => {
      const { b, journal, opening, bank } = opened();
      expect(() =>
        correctTransfer(
          b,
          journal.id,
          { ...replacement(opening, bank), transferType: "betweenAccounts" },
          "Invented reason",
        ),
      ).toThrow(
        "Correct an opening balance with an opening balance, and a transfer with a transfer.",
      );
    });

    test("refuses a replacement on another account", () => {
      const { b, journal, opening, reserve, card, loan } = opened();
      for (const [from, to] of [
        [opening, reserve],
        [card, opening],
        [loan, opening],
      ])
        expect(() =>
          correctTransfer(
            b,
            journal.id,
            replacement(from, to),
            "Invented reason",
          ),
        ).toThrow("An opening balance correction keeps its account.");
      // Nor against another equity account.
      const { equity, bank } = ids(b);
      expect(() =>
        correctTransfer(
          b,
          journal.id,
          replacement(equity, bank),
          "Invented reason",
        ),
      ).toThrow("An opening balance correction keeps its account.");
    });

    test("refuses a replacement dated after the first entry", () => {
      const { b, opening, bank, card } = opened();
      const started = expense(b, card); // dated 2026-03-12
      const journal = started.journals.find(
        (j) => j.transferType === "openingBalance",
      )!;
      expect(() =>
        correctTransfer(
          started,
          journal.id,
          replacement(opening, bank, { date: "2026-03-13" }),
          "Invented reason",
        ),
      ).toThrow(
        "An opening balance must be dated on or before the first entry, 2026-03-12.",
      );
      expect(() =>
        correctTransfer(
          started,
          journal.id,
          replacement(opening, bank, { date: "2026-03-12" }),
          "Invented reason",
        ),
      ).not.toThrow();
    });
  });
});

describe("owner funding properties", () => {
  /** A small deterministic generator, so a failure always reproduces. */
  function random(seed: number) {
    let s = seed >>> 0;
    return () => {
      s = (Math.imul(s, 1664525) + 1013904223) >>> 0;
      return s / 2 ** 32;
    };
  }

  test("debits equal credits, and account balances plus spending reconcile", () => {
    for (let seed = 1; seed <= 40; seed++) {
      const next = random(seed);
      const pick = <T>(xs: T[]) => xs[Math.floor(next() * xs.length)];
      let b = setup();
      const a = ids(b);
      const types: TransferType[] = [
        "ownerLoan",
        "ownerContribution",
        "loanRepayment",
        "ownerDraw",
        "betweenAccounts",
      ];
      const usd = [a.bank, a.card, a.reserve, a.loan, a.equity];
      for (let step = 0; step < 25; step++) {
        const amountMinor = 1 + Math.floor(next() * 100000);
        const date = `2026-0${1 + Math.floor(next() * 9)}-1${Math.floor(next() * 10)}`;
        const roll = next();
        try {
          if (roll < 0.35)
            b = recordTransfer(
              b,
              transfer(pick(types), pick(usd), pick(usd), {
                amountMinor,
                date,
              }),
            );
          else if (roll < 0.65) b = expense(b, pick(usd), amountMinor);
          else if (roll < 0.8) {
            const live = b.journals.filter(
              (j) =>
                j.kind === "transfer" &&
                !b.journals.some((r) => r.reversesId === j.id),
            );
            if (live.length) {
              const original = pick(live);
              const lines = original.lines;
              b = correctTransfer(
                b,
                original.id,
                {
                  ...transfer(
                    original.transferType!,
                    lines[1].accountId,
                    lines[0].accountId,
                  ),
                  amountMinor,
                  date: original.date,
                },
                "Invented correction",
              );
            }
          } else
            b = recordOpeningBalance(b, {
              accountId: pick([a.bank, a.card, a.reserve, a.loan]),
              date: "2026-01-01",
              amountMinor: amountMinor * (next() < 0.5 ? -1 : 1),
            });
        } catch (e) {
          // An invalid random command is refused and changes nothing.
          expect(e).toBeInstanceOf(Error);
        }
      }
      let debit = 0,
        credit = 0;
      for (const j of b.journals)
        for (const l of j.lines) {
          debit += l.debit;
          credit += l.credit;
        }
      expect(debit).toBe(credit);
      // The bank less what is owed on cards and loans and the owner's
      // stake, plus spending,
      // nets to zero: every journal balances.
      const rows = balances(b, "2026-12-31", "USD");
      const net = rows.reduce(
        (sum, r) => sum + (r.kind === "bank" ? r.balance : -r.balance),
        0,
      );
      expect(net + (totals(b, "2026").USD ?? 0)).toBe(0);
    }
  });
});

describe("screen helpers", () => {
  test("Paid from lists business accounts, then the accounts the owner pays from", () => {
    let b = setup();
    b = recordOpeningBalance(b, {
      accountId: ids(b).bank,
      date: "2026-01-01",
      amountMinor: 1000,
    });
    const { own, personal } = paymentAccounts(b);
    expect(own.map((a) => a.kind)).toEqual(["bank", "card", "bank", "bank"]);
    // Never the opening balances account.
    expect(personal.map((a) => a.name)).toEqual([
      "Owner's loan",
      "Loan from owner",
      "Owner contributions",
    ]);
  });

  test("each transfer type offers only the accounts it fits, in its currency", () => {
    const b = setup();
    const names = (list: { name: string }[]) => list.map((a) => a.name);
    const loan = transferAccounts(b, "ownerLoan", "USD");
    expect(names(loan.from)).toEqual(["Owner's loan", "Loan from owner"]);
    expect(names(loan.to)).toEqual([
      "Operating account",
      "Business card",
      "Reserve account",
    ]);
    expect(names(transferAccounts(b, "ownerDraw", "USD").to)).toEqual([
      "Owner contributions",
    ]);
    expect(names(transferAccounts(b, "betweenAccounts", "EUR").from)).toEqual([
      "Euro account",
    ]);
  });

  test("an opening balance reads back, and its replacement corrects it", () => {
    let b = setup();
    const { card } = ids(b);
    b = recordOpeningBalance(b, {
      accountId: card,
      date: "2026-01-01",
      amountMinor: 8000,
    });
    const current = openingBalance(b, card)!;
    expect(current.amountMinor).toBe(8000);
    const replacement = openingBalanceReplacement(b, {
      accountId: card,
      date: "2025-12-31",
      amountMinor: 6500,
    });
    const fixed = correctTransfer(
      b,
      current.journal.id,
      replacement,
      "Statement misread",
    );
    expect(openingBalance(fixed, card)).toMatchObject({ amountMinor: 6500 });
    expect(balanceOf(fixed, card)).toBe(6500);
  });
});
