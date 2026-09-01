import type {
  ArchivePolicy,
  MappingValidationIssue,
  PlatformMappingDraft,
} from "./types";

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

export function updateMappingPath(
  drafts: PlatformMappingDraft[],
  id: string,
  field: "romRoot" | "saveRoots" | "stateRoots",
  value: string,
): PlatformMappingDraft[] {
  return updateMappingDraft(drafts, id, (draft) => ({
    ...draft,
    [field]: field === "romRoot" ? value : value.trim() ? [value] : [],
    source: "custom",
    customFields: { ...draft.customFields, [field]: true },
  }));
}

export function issueForDraft(
  issues: MappingValidationIssue[],
  draftId: string,
): MappingValidationIssue | undefined {
  return issues.find((issue) => issue.draftId === draftId);
}
