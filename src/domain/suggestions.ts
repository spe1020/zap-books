import type { BusinessData, Suggestion } from "./types";
import { id, now } from "./ledger";

/** A suggestion to attach. The record's id and time are generated. */
export type SuggestionInput = Omit<
  Suggestion,
  "id" | "businessId" | "createdAt"
>;

/**
 * Attaches a suggestion to a draft of this business. It needs its author's
 * key, at least one of a category, a purpose or a note, and its evidence.
 * Text is trimmed and blank optional fields are left out. The draft itself
 * is never changed.
 */
export function suggest(
  business: BusinessData,
  input: SuggestionInput,
): BusinessData {
  if (!/^[0-9a-f]{64}$/.test(input.author))
    throw new Error("A suggestion needs its author's key.");
  const draft = business.drafts.find(
    (d) => d.id === input.draftId && d.businessId === business.profile.id,
  );
  if (!draft) throw new Error("Draft not found in this business.");
  if (draft.status !== "draft")
    throw new Error("Suggestions can only be attached to drafts.");
  const purpose = input.purpose?.trim();
  const note = input.note?.trim();
  if (!input.categoryId && !purpose && !note)
    throw new Error("A suggestion needs a category, a purpose or a note.");
  if (
    input.categoryId &&
    !business.categories.some(
      (c) => c.id === input.categoryId && c.businessId === business.profile.id,
    )
  )
    throw new Error("Choose a category in this business.");
  const evidence = input.evidence.map((e) => e.trim());
  if (!evidence.length || evidence.some((e) => !e))
    throw new Error("A suggestion needs its evidence.");
  const b = structuredClone(business);
  const suggestion: Suggestion = {
    id: id(),
    businessId: b.profile.id,
    draftId: draft.id,
    author: input.author,
    ...(input.categoryId ? { categoryId: input.categoryId } : {}),
    ...(purpose ? { purpose } : {}),
    ...(note ? { note } : {}),
    evidence,
    createdAt: now(),
  };
  b.suggestions = [...(b.suggestions ?? []), suggestion];
  return b;
}
