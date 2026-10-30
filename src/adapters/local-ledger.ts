import type { BusinessData, Workspace } from "../domain/types";
import { approve, correct, event, id, saveDraft } from "../domain/ledger";
import { commitImport } from "../domain/import";
import { saveRule, updateSettings } from "../domain/settings";
import { finalizeExtractions, recordExtraction } from "../domain/extraction";
import { CommandClient, type LedgerCommand } from "./ledger-client";
import { repository, type LocalRepository } from "./storage";

/**
 * The browser-only books: the local demo. Commands run through the domain
 * functions and commit to IndexedDB, one action (`run`) as one commit, so a
 * failed step saves nothing. Commits are revision-checked against the
 * workspace this client last loaded or saved, so a change made in another
 * tab is refused rather than overwritten.
 */
export class LocalLedgerClient extends CommandClient {
  private workspace?: Workspace;
  constructor(private readonly repo: LocalRepository = repository) {
    super();
  }
  async load() {
    this.workspace = await this.repo.load();
    return this.workspace;
  }
  async run(businessId: string, commands: LedgerCommand[]) {
    const workspace = this.workspace ?? (await this.load());
    const current = workspace.businesses.find(
      (b) => b.profile.id === businessId,
    );
    if (!current) throw new Error("Business not found.");
    const next = commands.reduce(applyLocal, current);
    this.workspace = await this.repo.commit(
      businessId,
      next,
      workspace.revision,
    );
    return this.workspace;
  }
  /** Restores the fictional demo businesses, discarding every local edit. */
  async reset() {
    const workspace = this.workspace ?? (await this.load());
    this.workspace = await this.repo.reset(workspace.revision);
    return this.workspace;
  }
}

/** One command applied to the local books. */
export function applyLocal(
  current: BusinessData,
  command: LedgerCommand,
): BusinessData {
  switch (command.type) {
    case "SaveDraft":
    case "Correct": {
      const next =
        command.type === "SaveDraft" ? command.draft : command.replacement;
      let updated = {
        ...current,
        documents: [...current.documents, ...command.documents],
      };
      if (command.rememberRule) {
        const existing = updated.rules.find(
          (r) =>
            r.vendorIncludes.toLowerCase() === next.vendor.trim().toLowerCase(),
        );
        const rule = {
          id: existing?.id ?? id(),
          businessId: current.profile.id,
          vendorIncludes: next.vendor.trim(),
          categoryId: next.categoryId,
          purpose: next.purpose,
          enabled: true,
        };
        updated = {
          ...updated,
          rules: [...updated.rules.filter((r) => r.id !== rule.id), rule],
        };
        event(
          updated,
          "rule.saved",
          rule.id,
          `Explicitly saved classification rule for ${rule.vendorIncludes}`,
        );
      }
      const reading = command.extraction;
      const withReading = (state: BusinessData) =>
        reading
          ? recordExtraction(state, { draftId: next.id, ...reading })
          : state;
      if (command.type === "Correct")
        return finalizeExtractions(
          withReading(
            correct(updated, command.originalId, next, command.reason),
          ),
          next.id,
        );
      return withReading(saveDraft(updated, next));
    }
    case "Approve":
      return finalizeExtractions(
        approve(current, command.expenseId),
        command.expenseId,
      );
    case "CommitImport":
      return commitImport(current, command.rows, command.accountId);
    case "UpdateSettings":
      return updateSettings(current, command.change);
    case "SaveRule":
      return saveRule(current, command.rule);
    case "ChangeMembership":
    case "Veto":
      throw new Error(
        "The local demo has no members. Membership needs a business on the service.",
      );
  }
}

export const localLedger = new LocalLedgerClient();
