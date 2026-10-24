import type { AIAdapter } from "../domain/types";
import {
  categoryTotals,
  missingDetails,
  money,
  totals,
} from "../domain/ledger";
export const demoAI: AIAdapter = {
  propose(b, d) {
    if (d.businessId !== b.profile.id)
      throw new Error("Expense belongs to another business.");
    const rule = b.rules.find(
      (r) =>
        r.enabled &&
        d.vendor.toLowerCase().includes(r.vendorIncludes.toLowerCase()),
    );
    const payment = b.payments.find((p) => p.id === d.paymentId);
    return {
      simulated: true,
      categoryId: rule?.categoryId,
      purpose: rule?.purpose,
      ruleId: rule?.id,
      explanation: rule
        ? `The visible rule for “${rule.vendorIncludes}” suggests ${b.categories.find((c) => c.id === rule.categoryId)?.name}.`
        : "No saved vendor rule matches. Choose a category and describe the business purpose.",
      evidence: [
        ...(rule
          ? [
              `Vendor field: ${d.vendor}`,
              `Saved rule: vendor contains “${rule.vendorIncludes}”`,
            ]
          : ["Only the fields you entered are used."]),
        ...(payment
          ? [
              `Linked payment: ${payment.reference || payment.description}, ${money(payment.amountMinor, payment.currency)} on ${payment.date}`,
            ]
          : []),
      ],
      uncertainty:
        "Review the classification and business purpose. Document contents have not been read or extracted.",
    };
  },
  answer(b, question, period) {
    if (question === "What needs review?") {
      const drafts = b.drafts.filter((d) => d.status === "draft");
      const incomplete = drafts.filter(
        (d) => missingDetails(b, d).length,
      ).length;
      return `${b.profile.name} has ${drafts.length} expenses awaiting review; ${incomplete} have missing or invalid details. Approval records an entry and does not send money.`;
    }
    if (question === "What did we spend?") {
      const t = Object.entries(totals(b, period));
      return `Posted spending for ${period}: ${t.length ? t.map(([c, n]) => money(n, c as typeof b.profile.currency)).join("; ") : "no expenses yet"}. Transfers are excluded and corrections are netted. Currencies are kept separate.`;
    }
    if (question === "Which category is largest?") {
      const rows = categoryTotals(b, period, b.profile.currency);
      return rows.length
        ? `${rows[0].name} is the largest category in ${b.profile.currency} for ${period}: ${money(rows[0].amount, b.profile.currency)}, based on posted journal lines.`
        : "There are no posted expenses in the business reporting currency for this period.";
    }
    return "This demo supports the three questions shown. It uses saved records and rules, without a live model or document extraction.";
  },
};
