import { useState } from "react";
import type {
  Account,
  BusinessData,
  Currency,
  FiatCurrency,
  JournalEntry,
} from "../domain/types";
import { currencies, fiatCurrencies } from "../domain/types";
import { amountInput, money, parseMoney, today } from "../domain/ledger";
import {
  transferAccounts,
  transferTypeLabels,
  transferTypes,
  type TransferInput,
} from "../domain/transfers";
import type { Mutation } from "./Review";
import { Modal } from "./common";
import {
  BitcoinAmount,
  FiatValueField,
  bitcoinInput,
  fiatValueFrom,
  fiatValueLabel,
  parseBitcoin,
  rateText,
  useBitcoinUnit,
  useMoney,
  useUnitChange,
} from "./bitcoin";

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
        correcting.transferType &&
        correcting.transferType !== "openingBalance" &&
        correcting.transferType !== "exchange"
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
  const { unit } = useBitcoinUnit();
  const format = useMoney();
  const originalAmount =
    correcting && (correcting.lines[0].debit || correcting.lines[0].credit);
  const [amount, setAmount] = useState(
    correcting
      ? correcting.currency === "BTC"
        ? bitcoinInput(originalAmount!, unit)
        : amountInput(originalAmount!, correcting.currency)
      : "",
  );
  // A bitcoin transfer's value at the time, typed by a person.
  const fiatCurrency: FiatCurrency =
    correcting?.fiatValue?.currency ?? b.profile.currency;
  const [fiat, setFiat] = useState(
    correcting?.fiatValue
      ? amountInput(correcting.fiatValue.amountMinor, fiatCurrency)
      : "",
  );
  const bitcoin = currency === "BTC";
  const parseAmount = (text: string) =>
    bitcoin ? parseBitcoin(text, unit) : parseMoney(text, currency);
  useUnitChange((convert) => {
    if (bitcoin) setAmount(convert);
  });
  const [purpose, setPurpose] = useState(correcting?.purpose ?? "");
  const [reference, setReference] = useState(correcting?.reference ?? "");
  const [reason, setReason] = useState("");
  const [error, setError] = useState("");
  const { from, to } = transferAccounts(b, type, currency);
  const fromAccount = from.find((a) => a.id === fromId);
  const toAccount = to.find((a) => a.id === toId);
  let amountMinor: number | undefined;
  try {
    amountMinor = amount.trim() ? parseAmount(amount) : undefined;
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
    let fiatValue;
    try {
      minor = parseAmount(amount);
      fiatValue = bitcoin
        ? fiatValueFrom(minor, fiat, fiatCurrency, "manual", date)
        : undefined;
    } catch (e) {
      setError((e as Error).message);
      return;
    }
    const transfer: TransferInput = {
      date,
      amountMinor: minor,
      currency,
      fromAccountId: fromId,
      toAccountId: toId,
      transferType: type,
      purpose,
      ...(reference.trim() && { reference }),
      ...(fiatValue && { fiatValue }),
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
              ? bitcoin
                ? "a second wallet"
                : "a second bank or card account"
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
              <option key={c} value={c}>
                {c === "BTC" ? "BTC (bitcoin)" : c}
              </option>
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
          {!bitcoin && (
            <label>
              Amount ({currency})
              <input
                required
                inputMode="decimal"
                value={amount}
                onChange={(e) => setAmount(e.target.value)}
              />
            </label>
          )}
        </div>
        {bitcoin && (
          <>
            <BitcoinAmount value={amount} onChange={setAmount} />
            <FiatValueField
              sats={amountMinor}
              currency={fiatCurrency}
              label={fiatValueLabel(fiatCurrency, "transfer")}
              value={fiat}
              onChange={setFiat}
            />
          </>
        )}
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
              format(amountMinor!, currency),
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

/**
 * Buys or sells bitcoin, or with `correcting`, corrects an exchange: sats
 * into or out of a wallet, against a dollar, euro or pound bank or card
 * account, both amounts as they happened. A correction keeps both
 * accounts; the amounts, direction, date and time may change.
 */
export function RecordExchange({
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
  const { unit } = useBitcoinUnit();
  const mine = (a: Account) => a.businessId === b.profile.id;
  const wallets = b.accounts.filter((a) => mine(a) && a.kind === "wallet");
  const fiatAccounts = b.accounts.filter(
    (a) =>
      mine(a) &&
      (a.kind === "bank" || a.kind === "card") &&
      (fiatCurrencies as Currency[]).includes(a.currency),
  );
  const walletLine = correcting?.lines.find((l) =>
    wallets.some((w) => w.id === l.accountId),
  );
  const fiatLine = correcting?.lines.find((l) => l !== walletLine);
  const [buy, setBuy] = useState(walletLine ? walletLine.debit > 0 : true);
  const [walletId, setWalletId] = useState(
    walletLine?.accountId ?? wallets[0]?.id ?? "",
  );
  const [fiatId, setFiatId] = useState(
    fiatLine?.accountId ??
      fiatAccounts.find((a) => a.currency === b.profile.currency)?.id ??
      fiatAccounts[0]?.id ??
      "",
  );
  const [date, setDate] = useState(correcting?.date ?? today());
  const [time, setTime] = useState(
    /T(\d{2}:\d{2})/.exec(correcting?.fiatValue?.at ?? "")?.[1] ?? "",
  );
  const [amount, setAmount] = useState(
    walletLine ? bitcoinInput(walletLine.debit || walletLine.credit, unit) : "",
  );
  const [fiat, setFiat] = useState(
    correcting?.fiatValue
      ? amountInput(
          correcting.fiatValue.amountMinor,
          correcting.fiatValue.currency,
        )
      : "",
  );
  const [purpose, setPurpose] = useState(correcting?.purpose ?? "");
  const [reference, setReference] = useState(correcting?.reference ?? "");
  const [reason, setReason] = useState("");
  const [error, setError] = useState("");
  useUnitChange((convert) => setAmount(convert));
  const wallet = wallets.find((a) => a.id === walletId);
  const fiatAccount = fiatAccounts.find((a) => a.id === fiatId);
  const fiatCurrency = (fiatAccount?.currency ??
    b.profile.currency) as FiatCurrency;
  let sats: number | undefined;
  try {
    sats = amount.trim() ? parseBitcoin(amount, unit) : undefined;
  } catch {
    sats = undefined;
  }
  /** The exchange as typed; throws a message if an amount isn't one. */
  function exchange(): TransferInput {
    const amountMinor = parseBitcoin(amount, unit);
    const fiatValue = fiatValueFrom(
      amountMinor,
      fiat,
      fiatCurrency,
      "exchange",
      time ? `${date}T${time}Z` : date,
    );
    return {
      date,
      amountMinor,
      currency: "BTC",
      fromAccountId: buy ? fiatId : walletId,
      toAccountId: buy ? walletId : fiatId,
      transferType: "exchange",
      purpose,
      ...(reference.trim() && { reference }),
      ...(fiatValue && { fiatValue }),
    };
  }
  let preview: TransferInput | undefined;
  try {
    preview = wallet && fiatAccount && fiat.trim() ? exchange() : undefined;
  } catch {
    preview = undefined;
  }
  async function save() {
    setError("");
    let transfer: TransferInput;
    try {
      transfer = exchange();
    } catch (e) {
      setError((e as Error).message);
      return;
    }
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
        ? "Exchange corrected."
        : buy
          ? "Bitcoin purchase recorded."
          : "Bitcoin sale recorded.",
    );
    if (ok) onClose();
  }
  const missing = !wallets.length
    ? "Add a bitcoin wallet in Settings first."
    : !fiatAccounts.length
      ? "Add a dollar, euro or pound bank or card account in Settings first."
      : "";
  const paid = buy ? "paid" : "received";
  return (
    <Modal
      title={correcting ? "Correct exchange" : "Buy or sell bitcoin"}
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
        <fieldset className="choice-row">
          <legend>Exchange</legend>
          {(
            [
              [true, "Buy bitcoin"],
              [false, "Sell bitcoin"],
            ] as const
          ).map(([value, label]) => (
            <label key={label} className="checkbox-label">
              <input
                type="radio"
                name="direction"
                checked={buy === value}
                onChange={() => setBuy(value)}
              />
              {label}
            </label>
          ))}
        </fieldset>
        {missing ? (
          <p className="alert error" role="alert">
            {missing}
          </p>
        ) : (
          <div className="field-row">
            <label>
              Wallet
              <select
                value={walletId}
                disabled={!!correcting}
                onChange={(e) => setWalletId(e.target.value)}
              >
                {wallets.map((a) => (
                  <option key={a.id} value={a.id}>
                    {a.name}
                  </option>
                ))}
              </select>
            </label>
            <label>
              {buy ? "Paid from" : "Paid into"}
              <select
                value={fiatId}
                disabled={!!correcting}
                onChange={(e) => setFiatId(e.target.value)}
              >
                {fiatAccounts.map((a) => (
                  <option key={a.id} value={a.id}>
                    {a.name} · {a.currency}
                  </option>
                ))}
              </select>
            </label>
          </div>
        )}
        {correcting && (
          <p className="small muted">
            A correction keeps the wallet and the account. To change either,
            correct this exchange to the right amounts and record another.
          </p>
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
            Time, UTC (optional)
            <input
              type="time"
              value={time}
              onChange={(e) => setTime(e.target.value)}
            />
          </label>
        </div>
        <BitcoinAmount
          label={buy ? "Bitcoin bought" : "Bitcoin sold"}
          value={amount}
          onChange={setAmount}
        />
        <FiatValueField
          sats={sats}
          currency={fiatCurrency}
          label={`Amount ${paid}`}
          value={fiat}
          onChange={setFiat}
          help={`What the exchange ${buy ? "charged" : "paid"}, in ${fiatCurrency}, fees included.`}
        />
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
        {preview?.fiatValue && wallet && fiatAccount && (
          <p className="validation-note" role="status">
            {correcting && "Once the original is reversed: "}
            {buy
              ? `${fiatAccount.name} pays ${money(preview.fiatValue.amountMinor, fiatCurrency)} for ${money(preview.amountMinor, "BTC", unit)} into ${wallet.name}`
              : `${wallet.name} sells ${money(preview.amountMinor, "BTC", unit)} for ${money(preview.fiatValue.amountMinor, fiatCurrency)} into ${fiatAccount.name}`}
            , at {rateText(preview.fiatValue.rate, fiatCurrency)}. This isn't
            spending.
          </p>
        )}
        <p className="small muted">
          {correcting
            ? "The original stays in the books: it's reversed exactly, and this exchange is recorded in its place, linked to it."
            : "An exchange posts at once. Both amounts are kept as they happened; nothing is looked up."}
        </p>
        <button
          className="button primary"
          disabled={busy || !!missing || !preview}
        >
          {correcting
            ? "Post correction"
            : buy
              ? "Record purchase"
              : "Record sale"}
        </button>
      </form>
    </Modal>
  );
}
