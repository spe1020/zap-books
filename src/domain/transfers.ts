import type {
  Account,
  AccountKind,
  BusinessData,
  Currency,
  JournalEntry,
  TransferType,
} from "./types";
import { currencies } from "./types";
import { assertBalanced, event, id, now, sum, validDate } from "./ledger";

/**
 * Money moving between two of the business's own non-expense accounts: a
 * loan from the owner, a contribution, a repayment, a draw, or a move
 * between bank and card accounts. `openingBalance` is refused here; it comes
 * from `recordOpeningBalance`, and only a correction of an opening balance
 * passes it back in.
 */
export interface TransferInput {
  date: string;
  /** Positive minor units. */
  amountMinor: number;
  currency: Currency;
  fromAccountId: string;
  toAccountId: string;
  transferType: TransferType;
  purpose: string;
  reference?: string;
  documentIds?: string[];
}

/**
 * An account's balance before the business's first entry. `amountMinor` is
 * the balance as `balances` reports it: what's in a bank account, or what's
 * owed on a card or loan account. It may be negative (an overdrawn bank
 * account, say), never zero.
 */
export interface OpeningBalanceInput {
  accountId: string;
  date: string;
  amountMinor: number;
  reference?: string;
}

/** One row of `balances`. */
export interface Balance {
  accountId: string;
  name: string;
  kind: AccountKind;
  balance: number;
}

const paying = (k: AccountKind) => k === "bank" || k === "card";

/** What each transfer type is called on screens and in exports. */
export const transferTypeLabels: Record<TransferType, string> = {
  ownerLoan: "Owner loan",
  ownerContribution: "Owner contribution",
  loanRepayment: "Loan repayment",
  ownerDraw: "Owner draw",
  betweenAccounts: "Transfer between accounts",
  openingBalance: "Opening balance",
};

/** What each account kind is called on screens and in exports. */
export const accountKindLabels: Record<AccountKind, string> = {
  bank: "Bank account",
  card: "Credit card",
  liability: "Loan",
  equity: "Owner equity",
  expense: "Expense",
};

/**
 * What a balance from `balances` means for each kind: money held in a bank
 * account, owed on a card, owed to the owner on a loan, or the owner's
 * stake.
 */
export const balanceMeanings: Record<
  Exclude<AccountKind, "expense">,
  string
> = {
  bank: "Held",
  card: "Owed",
  liability: "Owed to owner",
  equity: "Owner's stake",
};

/** Which accounts each transfer type moves money between, and its message. */
const shapes: Record<
  Exclude<TransferType, "openingBalance">,
  {
    label: string;
    from: (k: AccountKind) => boolean;
    to: (k: AccountKind) => boolean;
    message: string;
  }
> = {
  ownerLoan: {
    label: transferTypeLabels.ownerLoan,
    from: (k) => k === "liability",
    to: paying,
    message:
      "An owner loan moves money from a loan account into a bank or card account.",
  },
  ownerContribution: {
    label: transferTypeLabels.ownerContribution,
    from: (k) => k === "equity",
    to: paying,
    message:
      "An owner contribution moves money from an owner equity account into a bank or card account.",
  },
  loanRepayment: {
    label: transferTypeLabels.loanRepayment,
    from: paying,
    to: (k) => k === "liability",
    message:
      "A loan repayment moves money from a bank or card account into a loan account.",
  },
  ownerDraw: {
    label: transferTypeLabels.ownerDraw,
    from: paying,
    to: (k) => k === "equity",
    message:
      "An owner draw moves money from a bank or card account into an owner equity account.",
  },
  betweenAccounts: {
    label: transferTypeLabels.betweenAccounts,
    from: paying,
    to: paying,
    message:
      "A transfer between accounts moves money from one bank or card account to another.",
  },
};

const isOpening = (j: JournalEntry) => j.transferType === "openingBalance";
const reversed = (b: BusinessData, journalId: string) =>
  b.journals.some((j) => j.reversesId === journalId);

/**
 * The name of the equity account opening balances in `currency` post
 * against. Every account has one currency and names are unique, so a
 * business with accounts in several currencies gets one per currency.
 */
export const openingBalancesName = (b: BusinessData, currency: Currency) =>
  currency === b.profile.currency
    ? "Opening balances"
    : `Opening balances (${currency})`;

/** The opening balance journal on `accountId` that is still in effect. */
const openingBalanceOf = (b: BusinessData, accountId: string) =>
  b.journals.find(
    (j) =>
      j.businessId === b.profile.id &&
      j.kind === "transfer" &&
      isOpening(j) &&
      !reversed(b, j.id) &&
      j.lines.some((l) => l.accountId === accountId),
  );

/**
 * Checks `input` against the business and appends its journal. Shared by
 * every command here; `b` is already a copy.
 */
function post(
  b: BusinessData,
  input: TransferInput,
  correctionOf?: string,
): JournalEntry {
  if (!validDate(input.date))
    throw new Error("A valid transfer date is required.");
  if (!Number.isSafeInteger(input.amountMinor) || input.amountMinor <= 0)
    throw new Error("Amount must be greater than zero in whole minor units.");
  if (!currencies.includes(input.currency))
    throw new Error("Choose a supported currency.");
  if (!input.purpose.trim()) throw new Error("Purpose is required.");
  const find = (accountId: string) =>
    b.accounts.find((a) => a.id === accountId && a.businessId === b.profile.id);
  const from = find(input.fromAccountId);
  const to = find(input.toAccountId);
  if (!from || !to) throw new Error("Choose accounts in this business.");
  if (from.id === to.id) throw new Error("Choose two different accounts.");
  if (from.currency !== input.currency || to.currency !== input.currency)
    throw new Error("Both accounts must be in the transfer's currency.");
  let label: string;
  if (input.transferType === "openingBalance") {
    label = "Opening balance";
    checkOpeningBalance(b, input, from, to);
  } else {
    const shape = shapes[input.transferType];
    if (!shape) throw new Error("Choose a transfer type.");
    if (!shape.from(from.kind) || !shape.to(to.kind))
      throw new Error(shape.message);
    label = shape.label;
  }
  const documentIds = input.documentIds ?? [];
  if (
    documentIds.some(
      (docId) =>
        !b.documents.some(
          (x) => x.id === docId && x.businessId === b.profile.id,
        ),
    )
  )
    throw new Error("Receipt does not belong to this business.");
  const journal: JournalEntry = {
    id: id(),
    businessId: b.profile.id,
    date: input.date,
    currency: input.currency,
    lines: [
      { accountId: to.id, debit: input.amountMinor, credit: 0 },
      { accountId: from.id, debit: 0, credit: input.amountMinor },
    ],
    kind: "transfer",
    memo: `${from.name} → ${to.name}`,
    createdAt: now(),
    transferType: input.transferType,
    purpose: input.purpose.trim(),
  };
  const reference = input.reference?.trim();
  if (reference) journal.reference = reference;
  if (documentIds.length) journal.documentIds = [...documentIds];
  if (correctionOf !== undefined) journal.correctionOf = correctionOf;
  assertBalanced(b, journal);
  b.journals.push(journal);
  event(b, "transfer.recorded", journal.id, `${label} recorded`, [
    ...documentIds,
  ]);
  return journal;
}

/**
 * An opening balance is between this currency's opening balances account
 * and one bank, card or loan account, at most one per account, dated on or
 * before every other entry.
 */
function checkOpeningBalance(
  b: BusinessData,
  input: TransferInput,
  from: Account,
  to: Account,
) {
  const name = openingBalancesName(b, input.currency).toLowerCase();
  const equity = (a: Account) =>
    a.kind === "equity" && a.name.toLowerCase() === name;
  const account = equity(from) ? to : equity(to) ? from : undefined;
  if (!account || !["bank", "card", "liability"].includes(account.kind))
    throw new Error("Opening balances are for bank, card and loan accounts.");
  if (openingBalanceOf(b, account.id))
    throw new Error(
      "This account already has an opening balance. Correct it instead.",
    );
  const first = b.journals
    .filter((j) => j.businessId === b.profile.id && !isOpening(j))
    .map((j) => j.date)
    .sort()[0];
  if (first !== undefined && input.date > first)
    throw new Error(
      `An opening balance must be dated on or before the first entry, ${first}.`,
    );
}

/** Records a transfer. It posts at once; there are no transfer drafts. */
export function recordTransfer(
  business: BusinessData,
  input: TransferInput,
): BusinessData {
  if (input.transferType === "openingBalance")
    throw new Error("Record an opening balance from its own step.");
  const b = structuredClone(business);
  post(b, input);
  return b;
}

/**
 * Reverses a posted transfer exactly and posts `replacement` in its place,
 * all or nothing. The reversal names the original (`reversesId`), and so
 * does the replacement (`correctionOf`). A transfer already reversed can't
 * be corrected again. An opening balance is corrected by another, on the
 * same account.
 */
export function correctTransfer(
  business: BusinessData,
  originalId: string,
  replacement: TransferInput,
  reason: string,
): BusinessData {
  const b = structuredClone(business);
  const original = b.journals.find(
    (j) => j.id === originalId && j.businessId === b.profile.id,
  );
  if (!original || original.kind !== "transfer" || !reason.trim())
    throw new Error("A posted transfer and correction reason are required.");
  if (reversed(b, original.id))
    throw new Error("This transfer has already been corrected.");
  if (isOpening(original) !== (replacement.transferType === "openingBalance"))
    throw new Error(
      "Correct an opening balance with an opening balance, and a transfer with a transfer.",
    );
  // The same account and the same opening balances account; the amount,
  // its direction and the date may change.
  if (
    isOpening(original) &&
    ![replacement.fromAccountId, replacement.toAccountId].every((id) =>
      original.lines.some((l) => l.accountId === id),
    )
  )
    throw new Error("An opening balance correction keeps its account.");
  const { correctionOf: _, ...prior } = original;
  const reversal: JournalEntry = {
    ...prior,
    id: id(),
    kind: "reversal",
    reversesId: original.id,
    memo: `Correction: ${reason}`,
    lines: original.lines.map((x) => ({
      ...x,
      debit: x.credit,
      credit: x.debit,
    })),
    createdAt: now(),
  };
  assertBalanced(b, reversal);
  b.journals.push(reversal);
  const posted = post(b, replacement, original.id);
  event(b, "transfer.reversed", original.id, reason, [
    original.id,
    reversal.id,
    posted.id,
  ]);
  return b;
}

/**
 * Sets an account's balance before the business's first entry, against
 * this currency's opening balances equity account, which is added the first
 * time it's needed.
 */
export function recordOpeningBalance(
  business: BusinessData,
  input: OpeningBalanceInput,
): BusinessData {
  const b = structuredClone(business);
  const account = b.accounts.find(
    (a) => a.id === input.accountId && a.businessId === b.profile.id,
  );
  if (!account) throw new Error("Choose an account in this business.");
  if (!["bank", "card", "liability"].includes(account.kind))
    throw new Error("Opening balances are for bank, card and loan accounts.");
  if (!Number.isSafeInteger(input.amountMinor) || input.amountMinor === 0)
    throw new Error("Enter an opening balance other than zero.");
  const equity = openingBalancesAccount(b, account.currency);
  // A bank account's balance is a debit; what's owed on a card or loan is
  // a credit.
  const debit = (account.kind === "bank") === input.amountMinor > 0;
  post(b, {
    date: input.date,
    amountMinor: Math.abs(input.amountMinor),
    currency: account.currency,
    fromAccountId: debit ? equity.id : account.id,
    toAccountId: debit ? account.id : equity.id,
    transferType: "openingBalance",
    purpose: "Opening balance",
    ...(input.reference !== undefined && { reference: input.reference }),
  });
  return b;
}

/** This currency's opening balances account, added if there is none. */
function openingBalancesAccount(b: BusinessData, currency: Currency) {
  const name = openingBalancesName(b, currency);
  const existing = b.accounts.find(
    (a) =>
      a.businessId === b.profile.id &&
      a.name.toLowerCase() === name.toLowerCase(),
  );
  if (existing) {
    if (existing.kind !== "equity" || existing.currency !== currency)
      throw new Error(
        `Another account is named ${name}. Rename it to record opening balances.`,
      );
    return existing;
  }
  const account: Account = {
    id: id(),
    businessId: b.profile.id,
    name,
    kind: "equity",
    currency,
  };
  b.accounts.push(account);
  event(b, "settings.updated", b.profile.id, `account saved: ${name}`);
  return account;
}

/**
 * Every non-expense account in `currency` with its balance on `asOf`, from
 * posted journals dated on or before it: debits minus credits for bank
 * accounts, and credits minus debits for card, loan and equity accounts, so
 * a card's balance is what's owed on it.
 * Reversals offset their originals. Currencies are never combined.
 */
export function balances(
  b: BusinessData,
  asOf: string,
  currency: Currency,
): Balance[] {
  if (!validDate(asOf)) throw new Error("Choose a valid date.");
  const rows = b.accounts
    .filter(
      (a) =>
        a.businessId === b.profile.id &&
        a.kind !== "expense" &&
        a.currency === currency,
    )
    .map((a) => ({ accountId: a.id, name: a.name, kind: a.kind, balance: 0 }));
  for (const j of b.journals)
    if (
      j.businessId === b.profile.id &&
      j.currency === currency &&
      j.date <= asOf
    )
      for (const line of j.lines) {
        const row = rows.find((r) => r.accountId === line.accountId);
        if (!row) continue;
        const delta =
          row.kind === "bank"
            ? line.debit - line.credit
            : line.credit - line.debit;
        row.balance = sum(row.balance, delta);
      }
  return rows;
}

// Helpers for the screens. They choose what to offer; the commands above
// still decide what's allowed. Not ported to Rust (PARITY_NOTES, Not
// ported).

/** Whether `a` is an opening balances account (see `openingBalancesName`). */
export const isOpeningBalancesAccount = (b: BusinessData, a: Account) =>
  a.kind === "equity" &&
  a.name.toLowerCase() === openingBalancesName(b, a.currency).toLowerCase();

/**
 * The accounts an expense can be paid from: the business's own bank and
 * card accounts, then the loan and equity accounts that mean the owner
 * paid personally. Opening balances accounts are left out.
 */
export function paymentAccounts(b: BusinessData) {
  const own = b.accounts.filter(
    (a) => a.businessId === b.profile.id && paying(a.kind),
  );
  const personal = b.accounts.filter(
    (a) =>
      a.businessId === b.profile.id &&
      (a.kind === "liability" ||
        (a.kind === "equity" && !isOpeningBalancesAccount(b, a))),
  );
  return { own, personal };
}

/** The transfer types a user chooses, in the order the form offers them. */
export const transferTypes = [
  "ownerLoan",
  "ownerContribution",
  "loanRepayment",
  "ownerDraw",
  "betweenAccounts",
] as const;

/**
 * The accounts in `currency` a transfer of `type` can move money from and
 * to. Opening balances accounts are never offered.
 */
export function transferAccounts(
  b: BusinessData,
  type: (typeof transferTypes)[number],
  currency: Currency,
) {
  const shape = shapes[type];
  const usable = b.accounts.filter(
    (a) =>
      a.businessId === b.profile.id &&
      a.currency === currency &&
      !isOpeningBalancesAccount(b, a),
  );
  return {
    from: usable.filter((a) => shape.from(a.kind)),
    to: usable.filter((a) => shape.to(a.kind)),
  };
}

/**
 * The opening balance in effect on `accountId`: its journal and its amount
 * as `recordOpeningBalance` takes it (what's in a bank account, or what's
 * owed on a card or loan account).
 */
export function openingBalance(b: BusinessData, accountId: string) {
  const journal = openingBalanceOf(b, accountId);
  const account = b.accounts.find((a) => a.id === accountId);
  if (!journal || !account) return undefined;
  const line = journal.lines.find((l) => l.accountId === accountId)!;
  const debit = line.debit - line.credit;
  return { journal, amountMinor: account.kind === "bank" ? debit : -debit };
}

/**
 * The `correctTransfer` replacement that changes the opening balance on
 * `input.accountId` to `input`: the same two accounts, in the direction
 * the new amount's sign gives.
 */
export function openingBalanceReplacement(
  b: BusinessData,
  input: OpeningBalanceInput,
): TransferInput {
  const current = openingBalance(b, input.accountId);
  const account = b.accounts.find((a) => a.id === input.accountId);
  if (!current || !account) throw new Error("There is no opening balance.");
  const equity = current.journal.lines.find(
    (l) => l.accountId !== input.accountId,
  )!.accountId;
  const debit = (account.kind === "bank") === input.amountMinor > 0;
  return {
    date: input.date,
    amountMinor: Math.abs(input.amountMinor),
    currency: account.currency,
    fromAccountId: debit ? equity : account.id,
    toAccountId: debit ? account.id : equity,
    transferType: "openingBalance",
    purpose: "Opening balance",
    ...(input.reference !== undefined && { reference: input.reference }),
  };
}

/** The date of the business's first entry other than opening balances. */
export const firstEntryDate = (b: BusinessData) =>
  b.journals
    .filter((j) => j.businessId === b.profile.id && !isOpening(j))
    .map((j) => j.date)
    .sort()[0] as string | undefined;
