import type {
  ArchivePolicy,
  MappingValidationIssue,
  PlatformMappingDraft,
} from "./types";

export type MappingPathField = "romRoot" | "saveRoots" | "stateRoots";
export type MappingListPathField = Exclude<MappingPathField, "romRoot">;

export const ARCHIVE_POLICIES: readonly ArchivePolicy[] = [
  "keep",
  "extract_keep",
  "extract_delete",
];

export const ARCHIVE_POLICY_LABELS: Record<ArchivePolicy, string> = {
  keep: "Keep archive",
  extract_keep: "Extract and keep archive",
  extract_delete: "Extract then delete archive",
};

export function cycleArchivePolicy(policy: ArchivePolicy): ArchivePolicy {
  const index = ARCHIVE_POLICIES.indexOf(policy);
  return ARCHIVE_POLICIES[(index + 1) % ARCHIVE_POLICIES.length] ?? "keep";
}

export function updateMappingDraft(
  drafts: PlatformMappingDraft[],
  id: string,
  update: (draft: PlatformMappingDraft) => PlatformMappingDraft,
): PlatformMappingDraft[] {
  return drafts.map((draft) => draft.id === id ? update(draft) : draft);
}

export function updateMappingEnabled(
  drafts: PlatformMappingDraft[],
  id: string,
  enabled: boolean,
): PlatformMappingDraft[] {
  return updateMappingDraft(drafts, id, (draft) => ({
    ...draft,
    enabled,
    source: "custom",
    customFields: { ...draft.customFields, enabled: true },
  }));
}

export function updateMappingArchivePolicy(
  drafts: PlatformMappingDraft[],
  id: string,
): PlatformMappingDraft[] {
  return updateMappingDraft(drafts, id, (draft) => ({
    ...draft,
    archivePolicy: cycleArchivePolicy(draft.archivePolicy),
    source: "custom",
    customFields: { ...draft.customFields, archivePolicy: true },
  }));
}

export function updateMappingPath(
  drafts: PlatformMappingDraft[],
  id: string,
  field: MappingPathField,
  value: string,
): PlatformMappingDraft[] {
  return updateMappingPathAt(drafts, id, field, 0, value);
}

export function updateMappingPathAt(
  drafts: PlatformMappingDraft[],
  id: string,
  field: MappingPathField,
  index: number,
  value: string,
): PlatformMappingDraft[] {
  return updateMappingDraft(drafts, id, (draft) => {
    if (field === "romRoot") {
      return {
        ...draft,
        romRoot: value,
        source: "custom",
        customFields: { ...draft.customFields, romRoot: true },
      };
    }
    const paths = [...draft[field]];
    while (paths.length <= index) paths.push("");
    paths[index] = value;
    return {
      ...draft,
      [field]: paths,
      source: "custom",
      customFields: { ...draft.customFields, [field]: true },
    };
  });
}

export function addMappingPath(
  drafts: PlatformMappingDraft[],
  id: string,
  field: MappingListPathField,
): PlatformMappingDraft[] {
  return updateMappingDraft(drafts, id, (draft) => ({
    ...draft,
    [field]: [...draft[field], ""],
    source: "custom",
    customFields: { ...draft.customFields, [field]: true },
  }));
}

export function removeMappingPath(
  drafts: PlatformMappingDraft[],
  id: string,
  field: MappingListPathField,
  index: number,
): PlatformMappingDraft[] {
  return updateMappingDraft(drafts, id, (draft) => ({
    ...draft,
    [field]: draft[field].filter((_, candidateIndex) => candidateIndex !== index),
    source: "custom",
    customFields: { ...draft.customFields, [field]: true },
  }));
}

export function normalizeMappingPaths(
  drafts: PlatformMappingDraft[],
): PlatformMappingDraft[] {
  return drafts.map((draft) => ({
    ...draft,
    romRoot: draft.romRoot.trim(),
    saveRoots: draft.saveRoots.map((path) => path.trim()).filter(Boolean),
    stateRoots: draft.stateRoots.map((path) => path.trim()).filter(Boolean),
  }));
}

export function issueForDraft(
  issues: MappingValidationIssue[],
  draftId: string,
): MappingValidationIssue | undefined {
  return issues.find((issue) => issue.draftId === draftId);
}
