import { useState } from "react";
import {
  Building2,
  CreditCard,
  Tags,
  Plus,
  Pencil,
  Sparkles,
  RotateCcw,
  Landmark,
} from "lucide-react";
import type {
  Account,
  AccountKind,
  BusinessData,
  ClassificationRule,
  Currency,
} from "../domain/types";
import { currencies, fiatCurrencies } from "../domain/types";
import { amountInput, id, parseMoney, today } from "../domain/ledger";
import {
  accountKindLabels,
  firstEntryDate,
  openingBalance,
  openingBalanceReplacement,
} from "../domain/transfers";
import { PRODUCT_NAME } from "../config";
import type { Mutation } from "./Review";
import { Modal } from "./common";
import {
  BitcoinAmount,
  FiatValueField,
  bitcoinInput,
  fiatValueFrom,
  fiatValueLabel,
  parseBitcoin,
  useBitcoinUnit,
  useMoney,
  useUnitChange,
} from "./bitcoin";

type AccountChoice = Exclude<AccountKind, "expense">;

/** One line on what each kind of account is for. */
const kindHelp: Record<AccountChoice, string> = {
  bank: "Money the business holds.",
  card: "A business credit card. Its balance is what's owed on it.",
  liability:
    "Money the business owes, such as a loan from the owner. An expense the owner pays personally can be owed back here.",
  equity:
    "The owner's stake: money put in that isn't paid back. An expense the owner pays personally can count as a contribution here.",
  wallet: "Bitcoin the business holds, on-chain or on Lightning, in sats.",
};
const ACCOUNTANT_NOTE =
  "Ask your accountant whether owner money is a loan or a contribution.";

/** What an opening balance means for each kind it's offered for. */
const openingLabel: Record<"bank" | "card" | "liability" | "wallet", string> = {
  bank: "Balance in the account",
  wallet: "Bitcoin in the wallet",
  card: "Amount owed on the card",
  liability: "Amount owed to the owner",
};
export function Settings({
  b,
  mutate,
  onReset,
  busy,
}: {
  b: BusinessData;
  mutate: Mutation;
  /** The local demo only. */
  onReset?: () => void;
  busy: boolean;
}) {
  const [name, setName] = useState(b.profile.name),
    [industry, setIndustry] = useState(b.profile.industry);
  const [rule, setRule] = useState<ClassificationRule | null>(null);
  const [editor, setEditor] = useState<{
    type: "account" | "category";
    id?: string;
    name: string;
    currency: Currency;
    kind: AccountChoice;
  }>();
  const [opening, setOpening] = useState<{
    account: Account;
    date: string;
    amount: string;
    /** A wallet's value on the opening date, typed by a person. */
    fiat: string;
    reason: string;
  }>();
  const { unit } = useBitcoinUnit();
  const format = useMoney();
  useUnitChange((convert) => {
    if (opening?.account.kind === "wallet")
      setOpening({ ...opening, amount: convert(opening.amount) });
  });
  const [error, setError] = useState("");
  async function saveEditor() {
    if (!editor?.name.trim()) {
      setError("Enter a name.");
      return;
    }
    const { type, name, currency, kind } = editor;
    const ok = await mutate(
      [
        {
          type: "UpdateSettings",
          change: editor.id
            ? {
                op: type === "account" ? "renameAccount" : "renameCategory",
                id: editor.id,
                name,
              }
            : type === "account"
              ? { op: "addAccount", name, kind, currency }
              : { op: "addCategory", name, currency },
        },
      ],
      `${type === "account" ? "Account" : "Category"} saved.`,
    );
    if (ok) setEditor(undefined);
  }
  const first = firstEntryDate(b);
  const openable = b.accounts.filter(
    (a) =>
      a.businessId === b.profile.id &&
      (a.kind === "bank" ||
        a.kind === "card" ||
        a.kind === "liability" ||
        a.kind === "wallet"),
  );
  async function saveOpening() {
    if (!opening) return;
    const wallet = opening.account.kind === "wallet";
    let amountMinor: number;
    let fiatValue;
    try {
      amountMinor = wallet
        ? parseBitcoin(opening.amount, unit)
        : parseMoney(opening.amount, opening.account.currency);
      fiatValue = wallet
        ? fiatValueFrom(
            amountMinor,
            opening.fiat,
            b.profile.currency,
            "manual",
            opening.date,
          )
        : undefined;
    } catch (e) {
      setError((e as Error).message);
      return;
    }
    const input = {
      accountId: opening.account.id,
      date: opening.date,
      amountMinor,
      ...(fiatValue && { fiatValue }),
    };
    let command;
    try {
      command = openingBalance(b, opening.account.id)
        ? {
            type: "CorrectTransfer" as const,
            originalId: openingBalance(b, opening.account.id)!.journal.id,
            replacement: openingBalanceReplacement(b, input),
            reason: opening.reason,
          }
        : { type: "RecordOpeningBalance" as const, openingBalance: input };
    } catch (e) {
      setError((e as Error).message);
      return;
    }
    const ok = await mutate([command], "Opening balance saved.");
    if (ok) setOpening(undefined);
  }
  return (
    <div className="settings">
      <div className="settings-grid">
        <section className="card setting-card">
          <h2>
            <Building2 size={19} />
            Business profile
          </h2>
          <p>Your workspace and default reporting currency.</p>
          <form
            onSubmit={async (e) => {
              e.preventDefault();
              if (!name.trim() || !industry.trim()) return;
              await mutate(
                [
                  {
                    type: "UpdateSettings",
                    change: { op: "profile", name, industry },
                  },
                ],
                "Business profile saved.",
              );
            }}
          >
            <label>
              Business name
              <input
                required
                value={name}
                onChange={(e) => setName(e.target.value)}
                maxLength={80}
              />
            </label>
            <label>
              Industry
              <input
                required
                value={industry}
                onChange={(e) => setIndustry(e.target.value)}
                maxLength={120}
              />
            </label>
            <div className="static-field">
              <span>Reporting currency</span>
              <strong>{b.profile.currency}</strong>
            </div>
            <button className="button primary" disabled={busy}>
              Save profile
            </button>
          </form>
        </section>
        <section className="card setting-card">
          <div className="flex-between">
            <h2>
              <CreditCard size={19} />
              Accounts
            </h2>
            <button
              className="icon-button"
              aria-label="Add account"
              onClick={() => {
                setError("");
                setEditor({
                  type: "account",
                  name: "",
                  currency: b.profile.currency,
                  kind: "bank",
                });
              }}
            >
              <Plus size={18} />
            </button>
          </div>
          <p>
            Bank and card accounts, bitcoin wallets, loans from the owner, and
            owner equity. Accounts are local demo records, with no bank or
            wallet connection.
          </p>
          <p className="small muted">{ACCOUNTANT_NOTE}</p>
          {b.accounts
            .filter((a) => a.kind !== "expense")
            .map((a) => (
              <div key={a.id} className="setting-row">
                <div>
                  <strong>{a.name}</strong>
                  <small>
                    {accountKindLabels[a.kind]} · {a.currency}
                  </small>
                </div>
                <button
                  className="icon-button"
                  aria-label={`Edit ${a.name}`}
                  onClick={() => {
                    setError("");
                    setEditor({
                      type: "account",
                      id: a.id,
                      name: a.name,
                      currency: a.currency,
                      kind: a.kind as AccountChoice,
                    });
                  }}
                >
                  <Pencil size={15} />
                </button>
              </div>
            ))}
        </section>
      </div>
      <section className="card setting-card">
        <h2>
          <Landmark size={19} />
          Opening balances
        </h2>
        <p>
          For accounts that existed before {b.profile.name} started using{" "}
          {PRODUCT_NAME}: what each held or owed before the first entry
          {first ? `, on ${first}` : ""}. Each account has one, and it can be
          corrected.
        </p>
        {openable.length === 0 ? (
          <p className="small muted">
            Add a bank, card, wallet or loan account to record its opening
            balance.
          </p>
        ) : (
          openable.map((a) => {
            const current = openingBalance(b, a.id);
            return (
              <div key={a.id} className="setting-row">
                <div>
                  <strong>{a.name}</strong>
                  <small>
                    {current
                      ? `${openingLabel[a.kind as keyof typeof openingLabel]}: ${format(current.amountMinor, a.currency)} · ${current.journal.date}`
                      : `${accountKindLabels[a.kind]} · no opening balance`}
                  </small>
                </div>
                <button
                  className="button secondary"
                  aria-label={`${current ? "Correct" : "Set"} opening balance for ${a.name}`}
                  onClick={() => {
                    setError("");
                    setOpening({
                      account: a,
                      date: current?.journal.date ?? first ?? today(),
                      amount: current
                        ? a.kind === "wallet"
                          ? bitcoinInput(current.amountMinor, unit)
                          : amountInput(current.amountMinor, a.currency)
                        : "",
                      fiat: current?.journal.fiatValue
                        ? amountInput(
                            current.journal.fiatValue.amountMinor,
                            current.journal.fiatValue.currency,
                          )
                        : "",
                      reason: "",
                    });
                  }}
                >
                  {current ? "Correct" : "Set"}
                </button>
              </div>
            );
          })
        )}
      </section>
      <section className="card setting-card">
        <div className="flex-between">
          <h2>
            <Sparkles size={19} />
            Classification rules
          </h2>
          <button
            className="button secondary"
            onClick={() => {
              setError("");
              setRule({
                id: id(),
                businessId: b.profile.id,
                vendorIncludes: "",
                categoryId: b.categories[0].id,
                purpose: "",
                enabled: true,
              });
            }}
          >
            <Plus size={15} />
            Add rule
          </button>
        </div>
        <p>
          Rules suggest classifications for this business. Every rule is
          explicitly saved and editable.
        </p>
        {b.rules.length === 0 ? (
          <p>No rules yet. Add a vendor rule or save one during review.</p>
        ) : (
          b.rules.map((r) => (
            <div className="rule-row" key={r.id}>
              <span className="rule-symbol">
                <Sparkles size={17} />
              </span>
              <div>
                <strong>Vendor contains “{r.vendorIncludes}”</strong>
                <p>
                  {b.categories.find((c) => c.id === r.categoryId)?.name}{" "}
                  <span>· {r.purpose}</span>
                </p>
              </div>
              <span className={`badge ${r.enabled ? "success" : "neutral"}`}>
                {r.enabled ? "Active" : "Paused"}
              </span>
              <button
                className="button secondary"
                onClick={() => {
                  setError("");
                  setRule({ ...r });
                }}
              >
                Edit
              </button>
            </div>
          ))
        )}
      </section>
      <section className="card setting-card">
        <div className="flex-between">
          <h2>
            <Tags size={19} />
            Expense categories
          </h2>
          <button
            className="button secondary"
            onClick={() => {
              setError("");
              setEditor({
                type: "category",
                name: "",
                currency: b.profile.currency,
                kind: "bank",
              });
            }}
          >
            <Plus size={15} />
            Add category
          </button>
        </div>
        <div className="category-chips">
          {b.categories.map((c) => (
            <button
              key={c.id}
              onClick={() => {
                setError("");
                setEditor({
                  type: "category",
                  id: c.id,
                  name: c.name,
                  currency: b.profile.currency,
                  kind: "bank",
                });
              }}
            >
              {c.name}
              <Pencil size={13} />
            </button>
          ))}
        </div>
      </section>
      {onReset && (
        <section className="reset-panel">
          <div>
            <h3>Your demo workspace</h3>
            <p>
              Records and attachments stay in this browser. Reset replaces both
              businesses with fresh demonstration data.
            </p>
          </div>
          <button className="button danger" onClick={onReset}>
            <RotateCcw size={15} />
            Reset demo
          </button>
        </section>
      )}
      {rule && (
        <Modal title="Classification rule" onClose={() => setRule(null)}>
          <form
            className="modal-form"
            onSubmit={async (e) => {
              e.preventDefault();
              if (!rule.vendorIncludes.trim() || !rule.purpose.trim()) {
                setError("Vendor and purpose are required.");
                return;
              }
              const ok = await mutate(
                [{ type: "SaveRule", rule }],
                "Classification rule saved.",
              );
              if (ok) setRule(null);
            }}
          >
            {error && (
              <p className="error-text" role="alert">
                {error}
              </p>
            )}
            <label>
              Vendor contains
              <input
                required
                value={rule.vendorIncludes}
                onChange={(e) =>
                  setRule({ ...rule, vendorIncludes: e.target.value })
                }
              />
            </label>
            <label>
              Suggest category
              <select
                value={rule.categoryId}
                onChange={(e) =>
                  setRule({ ...rule, categoryId: e.target.value })
                }
              >
                {b.categories.map((c) => (
                  <option value={c.id} key={c.id}>
                    {c.name}
                  </option>
                ))}
              </select>
            </label>
            <label>
              Suggest business purpose
              <textarea
                required
                value={rule.purpose}
                onChange={(e) => setRule({ ...rule, purpose: e.target.value })}
              />
            </label>
            <label className="checkbox-label">
              <input
                type="checkbox"
                checked={rule.enabled}
                onChange={(e) =>
                  setRule({ ...rule, enabled: e.target.checked })
                }
              />
              Rule active
            </label>
            <p className="small muted">
              Suggestions still require review. This rule never posts an
              expense.
            </p>
            <button className="button primary" disabled={busy}>
              Save rule
            </button>
          </form>
        </Modal>
      )}
      {opening && (
        <Modal
          title={`Opening balance · ${opening.account.name}`}
          onClose={() => setOpening(undefined)}
        >
          <form
            className="modal-form"
            onSubmit={(e) => {
              e.preventDefault();
              void saveOpening();
            }}
          >
            {error && (
              <p role="alert" className="error-text">
                {error}
              </p>
            )}
            <p>
              Enter what this account showed before your first entry in{" "}
              {PRODUCT_NAME}, as of the day you start from.
            </p>
            <label>
              As of
              <input
                type="date"
                required
                value={opening.date}
                onChange={(e) =>
                  setOpening({ ...opening, date: e.target.value })
                }
              />
            </label>
            {opening.account.kind === "wallet" ? (
              <>
                <BitcoinAmount
                  label={openingLabel.wallet}
                  value={opening.amount}
                  onChange={(amount) => setOpening({ ...opening, amount })}
                />
                <FiatValueField
                  sats={(() => {
                    try {
                      return parseBitcoin(opening.amount, unit);
                    } catch {
                      return undefined;
                    }
                  })()}
                  currency={b.profile.currency}
                  label={fiatValueLabel(b.profile.currency, "opening")}
                  value={opening.fiat}
                  onChange={(fiat) => setOpening({ ...opening, fiat })}
                />
              </>
            ) : (
              <label>
                {
                  openingLabel[
                    opening.account.kind as keyof typeof openingLabel
                  ]
                }{" "}
                ({opening.account.currency})
                <input
                  required
                  inputMode="decimal"
                  value={opening.amount}
                  onChange={(e) =>
                    setOpening({ ...opening, amount: e.target.value })
                  }
                />
              </label>
            )}
            <p className="small muted">
              {opening.account.kind === "wallet"
                ? "What the wallet held, as a positive amount. A wallet can't hold less than none."
                : opening.account.kind === "bank"
                  ? "For an overdrawn account, enter a negative amount."
                  : opening.account.kind === "card"
                    ? "What the business owed on the card, as a positive amount."
                    : "What the business owed the owner, as a positive amount."}
            </p>
            {openingBalance(b, opening.account.id) && (
              <label>
                Reason for the correction
                <input
                  required
                  value={opening.reason}
                  onChange={(e) =>
                    setOpening({ ...opening, reason: e.target.value })
                  }
                />
              </label>
            )}
            <button className="button primary" disabled={busy}>
              Save opening balance
            </button>
          </form>
        </Modal>
      )}
      {editor && (
        <Modal
          title={`${editor.id ? "Edit" : "Add"} ${editor.type}`}
          onClose={() => setEditor(undefined)}
        >
          <form
            className="modal-form"
            onSubmit={(e) => {
              e.preventDefault();
              void saveEditor();
            }}
          >
            {error && (
              <p role="alert" className="error-text">
                {error}
              </p>
            )}
            <label>
              Name
              <input
                required
                value={editor.name}
                onChange={(e) => setEditor({ ...editor, name: e.target.value })}
                maxLength={80}
              />
            </label>
            {editor.type === "account" && !editor.id && (
              <>
                <label>
                  Type
                  <select
                    value={editor.kind}
                    onChange={(e) => {
                      const kind = e.target.value as AccountChoice;
                      // A wallet holds only bitcoin, and only a wallet,
                      // loan or equity account can.
                      const currency =
                        kind === "wallet"
                          ? "BTC"
                          : editor.currency === "BTC" &&
                              (kind === "bank" || kind === "card")
                            ? b.profile.currency
                            : editor.currency;
                      setEditor({ ...editor, kind, currency });
                    }}
                  >
                    <option value="bank">Bank account</option>
                    <option value="card">Credit card</option>
                    <option value="wallet">Bitcoin wallet</option>
                    <option value="liability">Loan (owed to the owner)</option>
                    <option value="equity">Owner equity</option>
                  </select>
                </label>
                <p className="small muted" id="kind-help">
                  {kindHelp[editor.kind]}
                </p>
                {(editor.kind === "liability" || editor.kind === "equity") && (
                  <p className="small muted">{ACCOUNTANT_NOTE}</p>
                )}
                <label>
                  Currency
                  <select
                    value={editor.currency}
                    disabled={editor.kind === "wallet"}
                    onChange={(e) =>
                      setEditor({
                        ...editor,
                        currency: e.target.value as Currency,
                      })
                    }
                  >
                    {(editor.kind === "wallet"
                      ? (["BTC"] as const)
                      : editor.kind === "bank" || editor.kind === "card"
                        ? fiatCurrencies
                        : currencies
                    ).map((c) => (
                      <option key={c} value={c}>
                        {c === "BTC" ? "BTC (in sats)" : c}
                      </option>
                    ))}
                  </select>
                </label>
              </>
            )}
            <button className="button primary" disabled={busy}>
              Save {editor.type}
            </button>
          </form>
        </Modal>
      )}
    </div>
  );
}
