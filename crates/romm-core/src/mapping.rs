use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
};

use romm_ipc::{
    ArchivePolicy, DetectionEvidence, MappingDetectionResult, MappingSource,
    MappingValidationIssue, MappingValidationResult, PlatformMappingDraft, PlatformSummary,
};

#[derive(Debug, Clone)]
struct CandidateRoot {
    label: String,
    path: PathBuf,
    source: MappingSource,
    confidence: u8,
}

pub fn detect_platform_mappings(platforms: &[PlatformSummary]) -> MappingDetectionResult {
    detect_platform_mappings_with_roots(platforms, candidate_roots())
}

fn detect_platform_mappings_with_roots(
    platforms: &[PlatformSummary],
    roots: Vec<CandidateRoot>,
) -> MappingDetectionResult {
    let existing_roots = roots
        .into_iter()
        .filter(|candidate| candidate.path.is_dir())
        .collect::<Vec<_>>();
    let evidence = existing_roots
        .iter()
        .enumerate()
        .map(|(index, candidate)| DetectionEvidence {
            id: format!("root-{index}"),
            label: candidate.label.clone(),
            path: candidate.path.to_string_lossy().into_owned(),
            source: candidate.source,
            confidence: candidate.confidence,
        })
        .collect::<Vec<_>>();

    let mut catalog = platforms.to_vec();
    catalog.sort_by_key(|platform| platform.name.to_lowercase());
    let drafts = catalog
        .iter()
        .map(|platform| {
            let matched = existing_roots.iter().find_map(|root| {
                find_platform_directory(&root.path, platform).map(|path| (root, path))
            });
            let (enabled, rom_root, source, preset_id, preset_version) = match matched {
                Some((root, path)) => (
                    true,
                    path.to_string_lossy().into_owned(),
                    root.source,
                    Some("emudeck".to_owned()),
                    Some(1),
                ),
                None => (
                    false,
                    existing_roots
                        .first()
                        .map(|root| {
                            root.path
                                .join(&platform.slug)
                                .to_string_lossy()
                                .into_owned()
                        })
                        .unwrap_or_default(),
                    MappingSource::Unconfigured,
                    None,
                    None,
                ),
            };
            PlatformMappingDraft {
                id: format!("platform-{}", platform.id),
                platform_id: platform.id,
                platform_name: platform.name.clone(),
                platform_slug: platform.slug.clone(),
                enabled,
                rom_root,
                save_roots: Vec::new(),
                state_roots: Vec::new(),
                archive_policy: ArchivePolicy::Keep,
                filename_strategy: "romm_filename".to_owned(),
                source,
                preset_id,
                preset_version,
                custom_fields: BTreeMap::new(),
            }
        })
        .collect::<Vec<_>>();
    let detected_count = drafts.iter().filter(|draft| draft.enabled).count();
    MappingDetectionResult {
        platforms: catalog,
        drafts,
        evidence,
        detected_count,
    }
}

pub fn validate_mapping_drafts(
    drafts: &[PlatformMappingDraft],
    no_platforms: bool,
) -> MappingValidationResult {
    let mut issues = Vec::new();
    let enabled = drafts
        .iter()
        .filter(|draft| draft.enabled)
        .collect::<Vec<_>>();
    if no_platforms && !enabled.is_empty() {
        issues.push(issue(
            None,
            None,
            "no_platforms_conflict",
            "Disable every platform before choosing no-platform setup.",
        ));
    } else if !no_platforms && enabled.is_empty() {
        issues.push(issue(
            None,
            None,
            "platform_required",
            "Enable at least one platform or explicitly choose no-platform setup.",
        ));
    }

    let mut draft_ids = BTreeSet::new();
    let mut platform_ids = BTreeSet::new();
    let mut rom_destinations = BTreeMap::<String, String>::new();
    for draft in drafts {
        if draft.id.trim().is_empty() || !draft_ids.insert(draft.id.clone()) {
            issues.push(issue(
                Some(&draft.id),
                None,
                "duplicate_draft",
                "Every mapping draft requires a unique ID.",
            ));
        }
        if draft.platform_id <= 0 || !platform_ids.insert(draft.platform_id) {
            issues.push(issue(
                Some(&draft.id),
                Some("platformId"),
                "duplicate_platform",
                "Every mapping must belong to one unique RomM platform.",
            ));
        }
        if !draft.enabled {
            continue;
        }
        validate_directory(&draft.id, "romRoot", &draft.rom_root, true, &mut issues);
        for (index, path) in draft.save_roots.iter().enumerate() {
            validate_directory(
                &draft.id,
                &format!("saveRoots.{index}"),
                path,
                false,
                &mut issues,
            );
        }
        for (index, path) in draft.state_roots.iter().enumerate() {
            validate_directory(
                &draft.id,
                &format!("stateRoots.{index}"),
                path,
                false,
                &mut issues,
            );
        }
        if let Ok(canonical) = fs::canonicalize(draft.rom_root.trim()) {
            let key = normalized_path_key(&canonical);
            if let Some(existing) = rom_destinations.insert(key, draft.id.clone()) {
                issues.push(issue(
                    Some(&draft.id),
                    Some("romRoot"),
                    "duplicate_destination",
                    &format!("This ROM destination is already used by mapping {existing}."),
                ));
            }
        }
    }

    MappingValidationResult {
        valid: issues.is_empty(),
        issues,
    }
}

pub fn validate_mapping_drafts_for_review(
    drafts: &[PlatformMappingDraft],
) -> MappingValidationResult {
    let mut issues = Vec::new();
    let mut draft_ids = BTreeSet::new();
    let mut platform_ids = BTreeSet::new();
    for draft in drafts {
        if draft.id.trim().is_empty() || !draft_ids.insert(draft.id.clone()) {
            issues.push(issue(
                Some(&draft.id),
                None,
                "duplicate_draft",
                "Every mapping draft requires a unique ID.",
            ));
        }
        if draft.platform_id <= 0 || !platform_ids.insert(draft.platform_id) {
            issues.push(issue(
                Some(&draft.id),
                Some("platformId"),
                "duplicate_platform",
                "Every mapping must belong to one unique RomM platform.",
            ));
        }
        if draft.platform_name.trim().is_empty() || draft.platform_slug.trim().is_empty() {
            issues.push(issue(
                Some(&draft.id),
                Some("platformName"),
                "platform_identity_missing",
                "Every draft requires the RomM platform name and slug.",
            ));
        }
    }
    MappingValidationResult {
        valid: issues.is_empty(),
        issues,
    }
}

fn validate_directory(
    draft_id: &str,
    field: &str,
    value: &str,
    required: bool,
    issues: &mut Vec<MappingValidationIssue>,
) {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        if required {
            issues.push(issue(
                Some(draft_id),
                Some(field),
                "path_required",
                "Choose an existing absolute directory.",
            ));
        }
        return;
    }
    let path = Path::new(trimmed);
    if !path.is_absolute() {
        issues.push(issue(
            Some(draft_id),
            Some(field),
            "path_not_absolute",
            "The path must be absolute.",
        ));
    } else if !path.is_dir() {
        issues.push(issue(
            Some(draft_id),
            Some(field),
            "path_unavailable",
            "The directory does not exist or its storage is not mounted.",
        ));
    }
}

fn issue(
    draft_id: Option<&str>,
    field: Option<&str>,
    code: &str,
    message: &str,
) -> MappingValidationIssue {
    MappingValidationIssue {
        draft_id: draft_id.map(ToOwned::to_owned),
        field: field.map(ToOwned::to_owned),
        code: code.to_owned(),
        message: message.to_owned(),
    }
}

fn candidate_roots() -> Vec<CandidateRoot> {
    let mut candidates = Vec::new();
    let home = env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .map(PathBuf::from);
    if let Some(home) = home {
        candidates.push(CandidateRoot {
            label: "EmuDeck internal storage".to_owned(),
            path: home.join("Emulation").join("roms"),
            source: MappingSource::EmuDeckInternal,
            confidence: 95,
        });
        candidates.push(CandidateRoot {
            label: "EmuDeck user storage".to_owned(),
            path: home.join("EmuDeck").join("Emulation").join("roms"),
            source: MappingSource::EmuDeckInternal,
            confidence: 80,
        });
    }
    #[cfg(windows)]
    candidates.push(CandidateRoot {
        label: "EmuDeck Windows storage".to_owned(),
        path: PathBuf::from(r"C:\Emulation\roms"),
        source: MappingSource::EmuDeckInternal,
        confidence: 90,
    });
    #[cfg(target_os = "linux")]
    {
        for mount_parent in [Path::new("/run/media/deck"), Path::new("/run/media")] {
            if let Ok(entries) = fs::read_dir(mount_parent) {
                for entry in entries.flatten() {
                    candidates.push(CandidateRoot {
                        label: "EmuDeck removable storage".to_owned(),
                        path: entry.path().join("Emulation").join("roms"),
                        source: MappingSource::EmuDeckRemovable,
                        confidence: 90,
                    });
                }
            }
        }
    }
    candidates
}

fn find_platform_directory(root: &Path, platform: &PlatformSummary) -> Option<PathBuf> {
    let aliases = platform_aliases(platform);
    let entries = fs::read_dir(root).ok()?;
    entries.flatten().find_map(|entry| {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        (entry.path().is_dir() && aliases.contains(&name)).then(|| entry.path())
    })
}

fn platform_aliases(platform: &PlatformSummary) -> BTreeSet<String> {
    let mut aliases = BTreeSet::from([
        platform.slug.to_lowercase(),
        platform.name.to_lowercase().replace([' ', '-'], "_"),
    ]);
    match platform.slug.as_str() {
        "ps" => aliases.extend(["psx".to_owned(), "ps1".to_owned()]),
        "ngc" => {
            aliases.insert("gc".to_owned());
        }
        "genesis" => {
            aliases.insert("megadrive".to_owned());
        }
        _ => {}
    }
    aliases
}

fn normalized_path_key(path: &Path) -> String {
    let value = path.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        value.to_lowercase()
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn platform(id: i64, name: &str, slug: &str) -> PlatformSummary {
        PlatformSummary {
            id,
            name: name.to_owned(),
            slug: slug.to_owned(),
        }
    }

    #[test]
    fn detects_only_existing_platform_directories_without_writing() {
        let temp = tempfile::tempdir().expect("temporary root");
        let root = temp.path().join("Emulation").join("roms");
        fs::create_dir_all(root.join("gba")).expect("fixture directory");
        let result = detect_platform_mappings_with_roots(
            &[
                platform(1, "Game Boy Advance", "gba"),
                platform(2, "PlayStation", "ps"),
            ],
            vec![CandidateRoot {
                label: "Test root".to_owned(),
                path: root.clone(),
                source: MappingSource::EmuDeckInternal,
                confidence: 95,
            }],
        );
        assert_eq!(result.detected_count, 1);
        assert!(result.drafts[0].enabled);
        assert!(!result.drafts[1].enabled);
        assert!(!root.join("ps").exists());
    }

    #[test]
    fn validates_explicit_empty_setup_and_rejects_missing_or_duplicate_destinations() {
        let temp = tempfile::tempdir().expect("temporary root");
        let path = temp.path().to_string_lossy().into_owned();
        let draft = PlatformMappingDraft {
            id: "platform-1".to_owned(),
            platform_id: 1,
            platform_name: "GBA".to_owned(),
            platform_slug: "gba".to_owned(),
            enabled: true,
            rom_root: path,
            save_roots: Vec::new(),
            state_roots: Vec::new(),
            archive_policy: ArchivePolicy::Keep,
            filename_strategy: "romm_filename".to_owned(),
            source: MappingSource::Custom,
            preset_id: None,
            preset_version: None,
            custom_fields: BTreeMap::new(),
        };
        assert!(validate_mapping_drafts(&[], true).valid);
        assert!(!validate_mapping_drafts(&[], false).valid);
        assert!(!validate_mapping_drafts(std::slice::from_ref(&draft), true).valid);
        assert!(
            !validate_mapping_drafts(
                &[
                    draft.clone(),
                    PlatformMappingDraft {
                        id: "platform-2".to_owned(),
                        platform_id: 2,
                        ..draft
                    }
                ],
                false
            )
            .valid
        );
    }
}
