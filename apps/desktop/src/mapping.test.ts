import { describe, expect, it } from "vitest";
import {
  addMappingPath,
  cycleArchivePolicy,
  issueForDraft,
  normalizeMappingPaths,
  removeMappingPath,
  updateMappingArchivePolicy,
  updateMappingEnabled,
  updateMappingPath,
  updateMappingPathAt,
} from "./mapping";
import type { PlatformMappingDraft } from "./types";

const draft: PlatformMappingDraft = {
  id: "platform-1",
  platformId: 1,
  platformName: "Game Boy Advance",
  platformSlug: "gba",
  enabled: true,
  romRoot: "C:\\Emulation\\roms\\gba",
  saveRoots: [],
  stateRoots: [],
  archivePolicy: "keep",
  filenameStrategy: "romm_filename",
  source: "emu_deck_internal",
  presetId: "emudeck",
  presetVersion: 1,
  customFields: {},
};

describe("mapping review helpers", () => {
  it("cycles all explicit archive policies", () => {
    expect(cycleArchivePolicy("keep")).toBe("extract_keep");
    expect(cycleArchivePolicy("extract_keep")).toBe("extract_delete");
    expect(cycleArchivePolicy("extract_delete")).toBe("keep");
  });

  it("marks edited paths custom without mutating the original draft", () => {
    const updated = updateMappingPath([draft], draft.id, "saveRoots", "D:\\Saves\\gba");
    expect(updated[0]).toMatchObject({
      saveRoots: ["D:\\Saves\\gba"],
      source: "custom",
      customFields: { saveRoots: true },
    });
    expect(draft.saveRoots).toEqual([]);
  });

  it("adds, edits, removes, and normalizes multiple optional roots", () => {
    const added = addMappingPath([draft], draft.id, "saveRoots");
    const first = updateMappingPathAt(added, draft.id, "saveRoots", 0, " D:\\Saves\\gba ");
    const secondSlot = addMappingPath(first, draft.id, "saveRoots");
    const second = updateMappingPathAt(secondSlot, draft.id, "saveRoots", 1, "D:\\Backup\\gba");
    expect(second[0]?.saveRoots).toEqual([" D:\\Saves\\gba ", "D:\\Backup\\gba"]);

    const removed = removeMappingPath(second, draft.id, "saveRoots", 0);
    expect(removed[0]?.saveRoots).toEqual(["D:\\Backup\\gba"]);
    expect(normalizeMappingPaths([
      { ...removed[0]!, stateRoots: [" ", " D:\\States\\gba "] },
    ])[0]).toMatchObject({
      saveRoots: ["D:\\Backup\\gba"],
      stateRoots: ["D:\\States\\gba"],
    });
  });

  it("records enabled and archive choices as preset-protected custom fields", () => {
    const disabled = updateMappingEnabled([draft], draft.id, false);
    expect(disabled[0]).toMatchObject({
      enabled: false,
      source: "custom",
      customFields: { enabled: true },
    });

    const archive = updateMappingArchivePolicy(disabled, draft.id);
    expect(archive[0]).toMatchObject({
      archivePolicy: "extract_keep",
      source: "custom",
      customFields: { enabled: true, archivePolicy: true },
    });
  });

  it("selects the first issue belonging to a platform", () => {
    expect(issueForDraft([
      { draftId: null, field: null, code: "global", message: "Global" },
      { draftId: draft.id, field: "romRoot", code: "missing", message: "Missing" },
    ], draft.id)?.message).toBe("Missing");
  });
});
