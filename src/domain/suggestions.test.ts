import { describe, expect, test } from "vitest";
import { seedWorkspace } from "./seed";
import { suggest } from "./suggestions";

const author = "cd".repeat(32);
const fresh = () => {
  const [b, other] = seedWorkspace().businesses;
  const draft = b.drafts.find((d) => d.status === "draft")!;
  return { b, other, draft };
};

describe("suggestions (decision 0004)", () => {
  test("a suggestion is attached with its author, evidence and time", () => {
    const { b, draft } = fresh();
    const next = suggest(b, {
      draftId: draft.id,
      author,
      categoryId: b.categories[0].id,
      purpose: " Invented purpose ",
      evidence: [" Invented line "],
    });
    expect(next.suggestions).toEqual([
      {
        id: expect.any(String),
        businessId: b.profile.id,
        draftId: draft.id,
        author,
        categoryId: b.categories[0].id,
        purpose: "Invented purpose",
        evidence: ["Invented line"],
        createdAt: expect.any(String),
      },
    ]);
    expect(b.suggestions).toBeUndefined();
  });

  test("a suggestion never changes the draft or anything else", () => {
    const { b, draft } = fresh();
    const next = suggest(b, {
      draftId: draft.id,
      author,
      categoryId: b.categories[1].id,
      purpose: "Invented purpose",
      note: "Invented note",
      evidence: ["Invented line"],
    });
    const { suggestions: _, ...rest } = next;
    expect(rest).toEqual(b);
  });

  test("a suggestion is refused unless it is complete and in scope", () => {
    const { b, other, draft } = fresh();
    const posted = b.drafts.find((d) => d.status === "posted")!;
    const base = { draftId: draft.id, author, note: "x", evidence: ["y"] };
    const refused: [object, string][] = [
      [{ author: "CD".repeat(32) }, "A suggestion needs its author's key."],
      [{ draftId: "missing" }, "Draft not found in this business."],
      [{ draftId: other.drafts[0].id }, "Draft not found in this business."],
      [{ draftId: posted.id }, "Suggestions can only be attached to drafts."],
      [
        { note: " ", purpose: "" },
        "A suggestion needs a category, a purpose or a note.",
      ],
      [
        { categoryId: other.categories[0].id },
        "Choose a category in this business.",
      ],
      [{ evidence: [] }, "A suggestion needs its evidence."],
      [{ evidence: ["y", " "] }, "A suggestion needs its evidence."],
    ];
    for (const [change, message] of refused)
      expect(() => suggest(b, { ...base, ...change })).toThrow(message);
  });
});
