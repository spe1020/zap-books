import type { BusinessData, ClassificationRule, Currency } from "./types";
import { currencies } from "./types";
import { event, id } from "./ledger";

/** One edit from the Settings screen. New records get generated ids. */
export type SettingsChange =
  | { op: "profile"; name: string; industry: string }
  | {
      op: "addAccount";
      name: string;
      kind: "bank" | "card";
      currency: Currency;
    }
  | { op: "renameAccount"; id: string; name: string }
  | { op: "addCategory"; name: string; currency: Currency }
  | { op: "renameCategory"; id: string; name: string };

/** A classification rule to save. Without an `id` it is a new rule. */
export type RuleInput = Omit<ClassificationRule, "id" | "businessId"> & {
  id?: string;
};

/**
 * Applies one Settings edit and logs it. Account and category names are
 * unique within their list, ignoring case. Adding a category also adds its
 * expense account, under the same name: a category's name must be free
 * among the accounts too, renaming the category renames its account, and an
 * expense account can't be renamed on its own.
 */
export function updateSettings(
  business: BusinessData,
  change: SettingsChange,
): BusinessData {
  const b = structuredClone(business);
  if (change.op === "profile") {
    if (!change.name.trim() || !change.industry.trim())
      throw new Error("Enter a business name and industry.");
    b.profile.name = change.name.trim();
    b.profile.industry = change.industry.trim();
    event(b, "settings.updated", b.profile.id, "Business profile updated");
    return b;
  }
  const type =
    change.op === "addAccount" || change.op === "renameAccount"
      ? "account"
      : "category";
  const name = change.name.trim();
  if (!name) throw new Error("Enter a name.");
  const list = type === "account" ? b.accounts : b.categories;
  const editing = "id" in change ? change.id : undefined;
  const taken = (x: { id: string; name: string }, except?: string) =>
    x.id !== except && x.name.toLowerCase() === name.toLowerCase();
  // The account a renamed category shares its name with.
  const paired =
    change.op === "renameCategory"
      ? b.categories.find(
          (c) => c.id === change.id && c.businessId === b.profile.id,
        )?.accountId
      : undefined;
  if (
    list.some((x) => taken(x, editing)) ||
    (type === "category" && b.accounts.some((x) => taken(x, paired)))
  )
    throw new Error("That name is already in use.");
  if (change.op === "renameAccount" || change.op === "renameCategory") {
    const record = list.find(
      (x) => x.id === change.id && x.businessId === b.profile.id,
    );
    if (!record)
      throw new Error(
        type === "account"
          ? "Account not found in this business."
          : "Category not found in this business.",
      );
    if ("kind" in record && record.kind === "expense")
      throw new Error("Rename an expense account through its category.");
    record.name = name;
    if (change.op === "renameCategory")
      for (const a of b.accounts)
        if (
          a.id === paired &&
          a.businessId === b.profile.id &&
          a.kind === "expense"
        )
          a.name = name;
  } else {
    if (!currencies.includes(change.currency))
      throw new Error("Choose a supported currency.");
    if (change.op === "addAccount") {
      if (change.kind !== "bank" && change.kind !== "card")
        throw new Error("Choose a bank or card account.");
      b.accounts.push({
        id: id(),
        businessId: b.profile.id,
        name,
        kind: change.kind,
        currency: change.currency,
      });
    } else {
      const accountId = id();
      b.accounts.push({
        id: accountId,
        businessId: b.profile.id,
        name,
        kind: "expense",
        currency: change.currency,
      });
      b.categories.push({
        id: id(),
        businessId: b.profile.id,
        name,
        accountId,
      });
    }
  }
  event(b, "settings.updated", b.profile.id, `${type} saved: ${name}`);
  return b;
}

/**
 * Saves a classification rule and logs it. Vendor text is unique within the
 * business, ignoring case. A saved rule moves to the end of the list, so an
 * edited rule is consulted after the others.
 */
export function saveRule(
  business: BusinessData,
  input: RuleInput,
): BusinessData {
  if (!input.vendorIncludes.trim() || !input.purpose.trim())
    throw new Error("Vendor and purpose are required.");
  if (
    !business.categories.some(
      (c) => c.id === input.categoryId && c.businessId === business.profile.id,
    )
  )
    throw new Error("Choose a category in this business.");
  if (
    business.rules.some(
      (r) =>
        r.id !== input.id &&
        r.vendorIncludes.toLowerCase() ===
          input.vendorIncludes.trim().toLowerCase(),
    )
  )
    throw new Error("A rule for this vendor already exists.");
  const b = structuredClone(business);
  const rule: ClassificationRule = {
    id: input.id ?? id(),
    businessId: b.profile.id,
    vendorIncludes: input.vendorIncludes.trim(),
    categoryId: input.categoryId,
    purpose: input.purpose.trim(),
    enabled: input.enabled,
  };
  b.rules = [...b.rules.filter((r) => r.id !== rule.id), rule];
  event(
    b,
    "rule.saved",
    rule.id,
    `Classification rule saved: ${input.vendorIncludes}`,
  );
  return b;
}
