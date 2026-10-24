import { useState } from "react";
import {
  Building2,
  CreditCard,
  Tags,
  Plus,
  Pencil,
  Sparkles,
  RotateCcw,
} from "lucide-react";
import type {
  BusinessData,
  ClassificationRule,
  Currency,
} from "../domain/types";
import { currencies } from "../domain/types";
import { event, id } from "../domain/ledger";
import type { Mutation } from "./Review";
import { Modal } from "./common";
export function Settings({
  b,
  mutate,
  onReset,
  busy,
}: {
  b: BusinessData;
  mutate: Mutation;
  onReset: () => void;
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
    kind: "bank" | "card";
  }>();
  const [error, setError] = useState("");
  async function saveEditor() {
    if (!editor?.name.trim()) {
      setError("Enter a name.");
      return;
    }
    const { type, name, currency, kind } = editor;
    const ok = await mutate(
      (current) => {
        const next = structuredClone(current);
        const list = type === "account" ? next.accounts : next.categories;
        if (
          list.some(
            (x) =>
              x.id !== editor.id &&
              x.name.toLowerCase() === name.trim().toLowerCase(),
          )
        )
          throw new Error("That name is already in use.");
        if (editor.id) {
          const record = list.find((x) => x.id === editor.id)!;
          record.name = name.trim();
        } else if (type === "account")
          next.accounts.push({
            id: id(),
            businessId: b.profile.id,
            name: name.trim(),
            kind,
            currency,
          });
        else {
          const accountId = id();
          next.accounts.push({
            id: accountId,
            businessId: b.profile.id,
            name: name.trim(),
            kind: "expense",
            currency,
          });
          next.categories.push({
            id: id(),
            businessId: b.profile.id,
            name: name.trim(),
            accountId,
          });
        }
        event(
          next,
          "settings.updated",
          b.profile.id,
          `${type} saved: ${name.trim()}`,
        );
        return next;
      },
      `${type === "account" ? "Account" : "Category"} saved.`,
    );
    if (ok) setEditor(undefined);
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
              await mutate((current) => {
                const next = structuredClone(current);
                next.profile.name = name.trim();
                next.profile.industry = industry.trim();
                event(
                  next,
                  "settings.updated",
                  next.profile.id,
                  "Business profile updated",
                );
                return next;
              }, "Business profile saved.");
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
              Payment accounts
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
          <p>Accounts are local demo records, with no bank connection.</p>
          {b.accounts
            .filter((a) => a.kind !== "expense")
            .map((a) => (
              <div key={a.id} className="setting-row">
                <div>
                  <strong>{a.name}</strong>
                  <small>
                    {a.kind === "card" ? "Credit card" : "Bank account"} ·{" "}
                    {a.currency}
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
                      kind: a.kind as "bank" | "card",
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
              const ok = await mutate((current) => {
                if (
                  current.rules.some(
                    (r) =>
                      r.id !== rule.id &&
                      r.vendorIncludes.toLowerCase() ===
                        rule.vendorIncludes.trim().toLowerCase(),
                  )
                )
                  throw new Error("A rule for this vendor already exists.");
                const next = structuredClone(current);
                next.rules = [
                  ...next.rules.filter((r) => r.id !== rule.id),
                  {
                    ...rule,
                    vendorIncludes: rule.vendorIncludes.trim(),
                    purpose: rule.purpose.trim(),
                  },
                ];
                event(
                  next,
                  "rule.saved",
                  rule.id,
                  `Classification rule saved: ${rule.vendorIncludes}`,
                );
                return next;
              }, "Classification rule saved.");
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
                    onChange={(e) =>
                      setEditor({
                        ...editor,
                        kind: e.target.value as "bank" | "card",
                      })
                    }
                  >
                    <option value="bank">Bank account</option>
                    <option value="card">Credit card</option>
                  </select>
                </label>
                <label>
                  Currency
                  <select
                    value={editor.currency}
                    onChange={(e) =>
                      setEditor({
                        ...editor,
                        currency: e.target.value as Currency,
                      })
                    }
                  >
                    {currencies.map((c) => (
                      <option key={c}>{c}</option>
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
