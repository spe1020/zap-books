import { describe, expect, test } from "vitest";
import { seedWorkspace } from "./seed";
import { saveRule, updateSettings } from "./settings";

const fresh = () => seedWorkspace().businesses[0];

describe("settings", () => {
  test("updates the profile with trimmed text and logs it", () => {
    const b = fresh();
    const next = updateSettings(b, {
      op: "profile",
      name: "  Invented Studio ",
      industry: " Design ",
    });
    expect(next.profile.name).toBe("Invented Studio");
    expect(next.profile.industry).toBe("Design");
    expect(next.events.at(-1)?.type).toBe("settings.updated");
    expect(b.profile.name).not.toBe("Invented Studio");
    expect(() =>
      updateSettings(b, { op: "profile", name: " ", industry: "Design" }),
    ).toThrow("Enter a business name and industry.");
  });

  test("adds a category with its expense account", () => {
    const b = fresh();
    const next = updateSettings(b, {
      op: "addCategory",
      name: "Invented travel",
      currency: "USD",
    });
    const category = next.categories.at(-1)!;
    const account = next.accounts.find((a) => a.id === category.accountId)!;
    expect(category.name).toBe("Invented travel");
    expect(account.kind).toBe("expense");
    expect(next.audits.at(-1)?.detail).toBe("category saved: Invented travel");
  });

  test("names are unique within their list, ignoring case", () => {
    const b = fresh();
    const taken = b.accounts[0].name.toUpperCase();
    expect(() =>
      updateSettings(b, {
        op: "addAccount",
        name: taken,
        kind: "bank",
        currency: "USD",
      }),
    ).toThrow("That name is already in use.");
    // Renaming a record to its own name is allowed.
    const same = updateSettings(b, {
      op: "renameAccount",
      id: b.accounts[0].id,
      name: taken,
    });
    expect(same.accounts[0].name).toBe(taken);
    expect(() =>
      updateSettings(b, { op: "renameCategory", id: "missing", name: "X" }),
    ).toThrow("Category not found in this business.");
  });

  test("a category and its expense account keep one name (ACC-3)", () => {
    const b = fresh();
    const bank = b.accounts.find((a) => a.kind === "bank")!;
    // A category's name must be free among the accounts too.
    expect(() =>
      updateSettings(b, {
        op: "addCategory",
        name: bank.name.toLowerCase(),
        currency: "USD",
      }),
    ).toThrow("That name is already in use.");
    // An expense account is renamed through its category...
    const category = b.categories[0];
    expect(() =>
      updateSettings(b, {
        op: "renameAccount",
        id: category.accountId,
        name: "Invented spend",
      }),
    ).toThrow("Rename an expense account through its category.");
    // ...which renames both.
    const renamed = updateSettings(b, {
      op: "renameCategory",
      id: category.id,
      name: "Invented spend",
    });
    expect(renamed.categories[0].name).toBe("Invented spend");
    expect(
      renamed.accounts.find((a) => a.id === category.accountId)!.name,
    ).toBe("Invented spend");
    // Its own account's name doesn't count as taken; another account's does.
    expect(() =>
      updateSettings(b, {
        op: "renameCategory",
        id: category.id,
        name: category.name.toUpperCase(),
      }),
    ).not.toThrow();
    expect(() =>
      updateSettings(b, {
        op: "renameCategory",
        id: category.id,
        name: bank.name,
      }),
    ).toThrow("That name is already in use.");
    // Bank and card accounts are still renamed directly.
    expect(
      updateSettings(b, {
        op: "renameAccount",
        id: bank.id,
        name: "Invented bank",
      }).accounts.find((a) => a.id === bank.id)!.name,
    ).toBe("Invented bank");
  });

  test("a saved rule moves to the end and vendors stay unique", () => {
    const b = updateSettings(fresh(), {
      op: "profile",
      name: "Invented Studio",
      industry: "Design",
    });
    const first = saveRule(b, {
      vendorIncludes: " Invented Vendor ",
      categoryId: b.categories[0].id,
      purpose: " Office use ",
      enabled: true,
    });
    const rule = first.rules.at(-1)!;
    expect(rule.vendorIncludes).toBe("Invented Vendor");
    expect(rule.purpose).toBe("Office use");
    const second = saveRule(first, {
      vendorIncludes: "Other Vendor",
      categoryId: b.categories[0].id,
      purpose: "Supplies",
      enabled: true,
    });
    const edited = saveRule(second, { ...rule, enabled: false });
    expect(edited.rules.at(-1)).toMatchObject({ id: rule.id, enabled: false });
    expect(edited.rules).toHaveLength(second.rules.length);
    expect(() =>
      saveRule(edited, {
        vendorIncludes: "invented vendor",
        categoryId: b.categories[0].id,
        purpose: "Again",
        enabled: true,
      }),
    ).toThrow("A rule for this vendor already exists.");
    expect(() =>
      saveRule(edited, {
        vendorIncludes: "New",
        categoryId: "missing",
        purpose: "Again",
        enabled: true,
      }),
    ).toThrow("Choose a category in this business.");
  });
});
