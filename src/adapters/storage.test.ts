import { expect, test } from "vitest";
import { upgradeWorkspace } from "./storage";
import { seedWorkspace } from "../domain/seed";

test("a workspace saved before extraction records loads with none", () => {
  const saved = JSON.parse(JSON.stringify(seedWorkspace()));
  for (const b of saved.businesses) delete b.extractions;
  expect(saved.businesses[0]).not.toHaveProperty("extractions");
  const loaded = upgradeWorkspace(saved);
  expect(loaded.businesses.map((b) => b.extractions)).toEqual([[], []]);
  // Nothing else changes.
  expect(loaded.businesses.map(({ extractions: _, ...rest }) => rest)).toEqual(
    saved.businesses,
  );
});
