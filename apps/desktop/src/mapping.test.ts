import { describe, expect, it } from "vitest";
import {
  cycleArchivePolicy,
  issueForDraft,
  updateMappingPath,
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

  it("selects the first issue belonging to a platform", () => {
    expect(issueForDraft([
      { draftId: null, field: null, code: "global", message: "Global" },
      { draftId: draft.id, field: "romRoot", code: "missing", message: "Missing" },
    ], draft.id)?.message).toBe("Missing");
  });
});
