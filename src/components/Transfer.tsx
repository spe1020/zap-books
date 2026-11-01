import { useState } from "react";
import type {
  Account,
  BusinessData,
  Currency,
  JournalEntry,
} from "../domain/types";
import { currencies } from "../domain/types";
import { amountInput, money, parseMoney, today } from "../domain/ledger";
import {
  transferAccounts,
  transferTypeLabels,
  transferTypes,
} from "../domain/transfers";
import type { Mutation } from "./Review";
import { Modal } from "./common";

type Choice = (typeof transferTypes)[number];

/** What each type is for, shown under the type. */
const typeHelp: Record<Choice, string> = {
  ownerLoan:
    "The owner lends the business money, from a loan account into a bank or card account. The business owes it back.",
  ownerContribution:
    "The owner puts money in as equity, from an owner equity account. It isn't paid back.",
  loanRepayment:
    "The business pays the owner back, from a bank or card account into a loan account.",
  ownerDraw:
    "The owner takes money out, from a bank or card account into an owner equity account.",
  betweenAccounts:
    "Money moves between the business's own bank and card accounts, such as paying a card.",
};

/**
 * What the transfer will do, in a sentence, before it's posted. A card's
 * and a loan's balance is what's owed on it.
 */
function summary(
  type: Choice,
  from: Account,
  to: Account,
  amount: string,
): string {
  if (type === "ownerLoan")
    return `The business will owe ${from.name} ${amount} more, and ${to.name} will ${to.kind === "card" ? "owe" : "hold"} ${amount} ${to.kind === "card" ? "less" : "more"}.`;
  if (type === "ownerContribution")
    return `The owner puts ${amount} into ${to.name} as equity (${from.name}). Nothing is owed back.`;
  if (type === "loanRepayment")
    return `The business will owe ${to.name} ${amount} less, paid from ${from.name}.`;
  if (type === "ownerDraw")
    return `The owner takes ${amount} out of ${from.name}; their stake in ${to.name} falls by ${amount}.`;
  return to.kind === "card"
    ? `${from.name} pays ${amount} off ${to.name}. This isn't spending.`
    : `${amount} moves from ${from.name} to ${to.name}. This isn't spending.`;
}

/** The first account in `list`, unless `current` is already one of them. */
const keep = (list: Account[], current: string) =>
  list.some((a) => a.id === current) ? current : (list[0]?.id ?? "");

/**
 * Records a transfer, or with `correcting`, corrects one: the form starts
 * from the original, and posting reverses it exactly and records this in
 * its place.
 */
export function RecordTransfer({
  b,
  mutate,
  onClose,
  busy,
  correcting,
}: {
  b: BusinessData;
  mutate: Mutation;
  onClose: () => void;
  busy: boolean;
  correcting?: JournalEntry;
}) {
  // A transfer stored before transfer types moved money between accounts.
  const original =
    correcting &&
    ({
      type:
        correcting.transferType && correcting.transferType !== "openingBalance"
          ? correcting.transferType
          : "betweenAccounts",
      fromId: correcting.lines.find((l) => l.credit > 0)?.accountId ?? "",
      toId: correcting.lines.find((l) => l.debit > 0)?.accountId ?? "",
    } satisfies { type: Choice; fromId: string; toId: string });
  const start = (type: Choice, currency: Currency) => {
    const { from, to } = transferAccounts(b, type, currency);
    const fromId = from[0]?.id ?? "";
    return {
      fromId,
      toId: to.find((a) => a.id !== fromId)?.id ?? "",
    };
  };
  const [type, setType] = useState<Choice>(original?.type ?? "ownerLoan");
  const [currency, setCurrency] = useState<Currency>(
    correcting?.currency ?? b.profile.currency,
  );
  const [{ fromId, toId }, setAccounts] = useState(() =>
    original
      ? { fromId: original.fromId, toId: original.toId }
      : start("ownerLoan", b.profile.currency),
  );
  const [date, setDate] = useState(correcting?.date ?? today());
  const [amount, setAmount] = useState(
    correcting
      ? amountInput(correcting.lines[0].debit || correcting.lines[0].credit)
      : "",
  );
  const [purpose, setPurpose] = useState(correcting?.purpose ?? "");
  const [reference, setReference] = useState(correcting?.reference ?? "");
  const [reason, setReason] = useState("");
  const [error, setError] = useState("");
  const { from, to } = transferAccounts(b, type, currency);
  const fromAccount = from.find((a) => a.id === fromId);
  const toAccount = to.find((a) => a.id === toId);
  let amountMinor: number | undefined;
  try {
    amountMinor = amount.trim() ? parseMoney(amount) : undefined;
  } catch {
    amountMinor = undefined;
  }
  const ready =
    fromAccount && toAccount && fromAccount.id !== toAccount.id && amountMinor;
  function choose(nextType: Choice, nextCurrency: Currency) {
    const next = transferAccounts(b, nextType, nextCurrency);
    const nextFrom = keep(next.from, fromId);
    setType(nextType);
    setCurrency(nextCurrency);
    setAccounts({
      fromId: nextFrom,
      toId: keep(
        next.to.filter((a) => a.id !== nextFrom),
        toId,
      ),
    });
  }
  async function save() {
    setError("");
    let minor: number;
    try {
      minor = parseMoney(amount);
    } catch (e) {
      setError((e as Error).message);
      return;
    }
    const transfer = {
      date,
      amountMinor: minor,
      currency,
      fromAccountId: fromId,
      toAccountId: toId,
      transferType: type,
      purpose,
      ...(reference.trim() && { reference }),
    };
    const ok = await mutate(
      [
        correcting
          ? {
              type: "CorrectTransfer",
              originalId: correcting.id,
              replacement: transfer,
              reason,
            }
          : { type: "RecordTransfer", transfer },
      ],
      correcting
        ? "Transfer corrected."
        : `${transferTypeLabels[type]} recorded.`,
    );
    if (ok) onClose();
  }
  const missing =
    !from.length || !to.length
      ? `Add ${
          type === "ownerLoan" || type === "loanRepayment"
            ? "a loan account"
            : type === "betweenAccounts"
              ? "a second bank or card account"
              : "an owner equity account"
        } in ${currency} in Settings first.`
      : "";
  return (
    <Modal
      title={correcting ? "Correct transfer" : "Record transfer"}
      onClose={onClose}
    >
      <form
        className="modal-form"
        onSubmit={(e) => {
          e.preventDefault();
          void save();
        }}
      >
        {error && (
          <p role="alert" className="error-text">
            {error}
          </p>
        )}
        <label>
          Transfer type
          <select
            value={type}
            onChange={(e) => choose(e.target.value as Choice, currency)}
          >
            {transferTypes.map((t) => (
              <option key={t} value={t}>
                {transferTypeLabels[t]}
              </option>
            ))}
          </select>
        </label>
        <p className="small muted">{typeHelp[type]}</p>
        <label>
          Currency
          <select
            value={currency}
            onChange={(e) => choose(type, e.target.value as Currency)}
          >
            {currencies.map((c) => (
              <option key={c}>{c}</option>
            ))}
          </select>
        </label>
        {missing ? (
          <p className="alert error" role="alert">
            {missing}
          </p>
        ) : (
          <div className="field-row">
            <label>
              From
              <select
                value={fromId}
                onChange={(e) =>
                  setAccounts({
                    fromId: e.target.value,
                    toId: keep(
                      to.filter((a) => a.id !== e.target.value),
                      toId,
                    ),
                  })
                }
              >
                {from.map((a) => (
                  <option key={a.id} value={a.id}>
                    {a.name}
                  </option>
                ))}
              </select>
            </label>
            <label>
              To
              <select
                value={toId}
                onChange={(e) => setAccounts({ fromId, toId: e.target.value })}
              >
                {to
                  .filter((a) => a.id !== fromId)
                  .map((a) => (
                    <option key={a.id} value={a.id}>
                      {a.name}
                    </option>
                  ))}
              </select>
            </label>
          </div>
        )}
        <div className="field-row">
          <label>
            Date
            <input
              type="date"
              required
              value={date}
              onChange={(e) => setDate(e.target.value)}
            />
          </label>
          <label>
            Amount ({currency})
            <input
              required
              inputMode="decimal"
              value={amount}
              onChange={(e) => setAmount(e.target.value)}
            />
          </label>
        </div>
        <label>
          Purpose
          <input
            required
            value={purpose}
            onChange={(e) => setPurpose(e.target.value)}
            maxLength={200}
          />
        </label>
        <label>
          Reference (optional)
          <input
            value={reference}
            onChange={(e) => setReference(e.target.value)}
            maxLength={80}
          />
        </label>
        {correcting && (
          <label>
            Reason for the correction
            <input
              required
              value={reason}
              onChange={(e) => setReason(e.target.value)}
              maxLength={200}
            />
          </label>
        )}
        {ready && (
          <p className="validation-note" role="status">
            {correcting && "Once the original is reversed: "}
            {summary(
              type,
              fromAccount,
              toAccount,
              money(amountMinor!, currency),
            )}
          </p>
        )}
        <p className="small muted">
          {correcting
            ? "The original stays in the books: it's reversed exactly, and this transfer is recorded in its place, linked to it."
            : "Transfers post at once and never count as spending. A loan or a contribution is for your accountant to decide."}
        </p>
        <button className="button primary" disabled={busy || !ready}>
          {correcting ? "Post correction" : "Record transfer"}
        </button>
      </form>
    </Modal>
  );
}
