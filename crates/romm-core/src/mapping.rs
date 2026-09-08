use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
};

use romm_ipc::{
    AppError, ArchivePolicy, DetectionEvidence, DirectoryCreationResult, DirectoryEntry,
    DirectoryListing, MappingDetectionResult, MappingPathStatus, MappingPathValidation,
    MappingPresetUpdate, MappingSource, MappingValidationIssue, MappingValidationResult,
    PlatformMappingDraft, PlatformSummary,
};
use uuid::Uuid;

const MAX_DIRECTORY_ENTRIES: usize = 500;

pub fn browse_mapping_directories(path: Option<&str>) -> Result<DirectoryListing, AppError> {
    let Some(requested) = path.map(str::trim).filter(|path| !path.is_empty()) else {
        return Ok(DirectoryListing {
            current_path: None,
            parent_path: None,
            entries: browse_locations(),
            locations: true,
            truncated: false,
        });
    };
    let requested = PathBuf::from(requested);
    if !requested.is_absolute() {
        return Err(AppError::new(
            "directory_path_not_absolute",
            "Choose an absolute directory path.",
            false,
        )
        .field("path"));
    }
    let current = nearest_existing_directory(requested).ok_or_else(|| {
        AppError::new(
            "directory_unavailable",
            "The selected directory and its parent storage are unavailable.",
            true,
        )
        .field("path")
    })?;
    let read_dir = fs::read_dir(&current).map_err(|error| {
        AppError::new(
            "directory_read_failed",
            format!("Cannot browse {}: {error}", current.display()),
            error.kind() == std::io::ErrorKind::Interrupted,
        )
        .field("path")
    })?;
    let mut entries = read_dir
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let entry_path = entry.path();
            entry_path.is_dir().then(|| DirectoryEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                path: entry_path.to_string_lossy().into_owned(),
                is_symlink: entry
                    .file_type()
                    .map(|file_type| file_type.is_symlink())
                    .unwrap_or(false),
            })
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.name.cmp(&right.name))
    });
    let truncated = entries.len() > MAX_DIRECTORY_ENTRIES;
    entries.truncate(MAX_DIRECTORY_ENTRIES);
    let parent_path = current
        .parent()
        .filter(|parent| *parent != current)
        .map(|parent| parent.to_string_lossy().into_owned());
    Ok(DirectoryListing {
        current_path: Some(current.to_string_lossy().into_owned()),
        parent_path,
        entries,
        locations: false,
        truncated,
    })
}

pub fn create_mapping_directory(
    parent_path: &str,
    name: &str,
    confirmed: bool,
) -> Result<DirectoryCreationResult, AppError> {
    if !confirmed {
        return Err(AppError::new(
            "directory_creation_confirmation_required",
            "Confirm directory creation before continuing.",
            false,
        ));
    }
    let parent = PathBuf::from(parent_path.trim());
    if !parent.is_absolute() || !parent.is_dir() {
        return Err(AppError::new(
            "directory_parent_unavailable",
            "The parent directory does not exist or its storage is unavailable.",
            true,
        )
        .field("parentPath"));
    }
    let name = name.trim();
    let mut components = Path::new(name).components();
    let is_single_component = matches!(components.next(), Some(std::path::Component::Normal(_)))
        && components.next().is_none();
    if name.is_empty()
        || name.chars().count() > 128
        || !is_single_component
        || name.contains(['/', '\\'])
    {
        return Err(AppError::new(
            "invalid_directory_name",
            "Use one directory name of 1 to 128 characters without path separators.",
            false,
        )
        .field("name"));
    }
    let target = parent.join(name);
    if target.exists() {
        if target.is_dir() {
            return Ok(DirectoryCreationResult {
                path: target.to_string_lossy().into_owned(),
                created: false,
            });
        }
        return Err(AppError::new(
            "directory_name_conflict",
            "A file with that name already exists in this location.",
            false,
        )
        .field("name"));
    }
    fs::create_dir(&target).map_err(|error| {
        AppError::new(
            "directory_create_failed",
            format!("Cannot create {}: {error}", target.display()),
            matches!(
                error.kind(),
                std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
            ),
        )
        .field("name")
    })?;
    Ok(DirectoryCreationResult {
        path: target.to_string_lossy().into_owned(),
        created: true,
    })
}

fn nearest_existing_directory(mut path: PathBuf) -> Option<PathBuf> {
    loop {
        if path.is_dir() {
            return Some(path);
        }
        if !path.pop() {
            return None;
        }
    }
}

fn browse_locations() -> Vec<DirectoryEntry> {
    let mut locations = Vec::new();
    #[cfg(windows)]
    for letter in b'A'..=b'Z' {
        let path = PathBuf::from(format!("{}:\\", char::from(letter)));
        if path.is_dir() {
            locations.push(DirectoryEntry {
                name: format!("{}: drive", char::from(letter)),
                path: path.to_string_lossy().into_owned(),
                is_symlink: false,
            });
        }
    }
    #[cfg(not(windows))]
    {
        if let Some(home) = env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_dir())
        {
            locations.push(DirectoryEntry {
                name: "Home".to_owned(),
                path: home.to_string_lossy().into_owned(),
                is_symlink: false,
            });
        }
        for (name, path) in [
            ("Filesystem", PathBuf::from("/")),
            ("Mounted media", PathBuf::from("/run/media")),
            ("Media", PathBuf::from("/media")),
        ] {
            if path.is_dir()
                && !locations
                    .iter()
                    .any(|entry| entry.path == path.to_string_lossy())
            {
                locations.push(DirectoryEntry {
                    name: name.to_owned(),
                    path: path.to_string_lossy().into_owned(),
                    is_symlink: false,
                });
            }
        }
    }
    locations
}

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
                id: Uuid::new_v4().to_string(),
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
        preset_updates: Vec::new(),
    }
}

pub fn merge_mapping_detection(
    mut detected: MappingDetectionResult,
    existing: &[PlatformMappingDraft],
) -> MappingDetectionResult {
    let existing_by_platform = existing
        .iter()
        .map(|draft| (draft.platform_id, draft))
        .collect::<BTreeMap<_, _>>();
    let mut updates = Vec::new();
    detected.drafts = detected
        .drafts
        .into_iter()
        .map(|proposal| {
            let Some(current) = existing_by_platform.get(&proposal.platform_id) else {
                return proposal;
            };
            merge_mapping_draft(&proposal, current, &mut updates)
        })
        .collect();
    detected.preset_updates = updates;
    detected
}

fn merge_mapping_draft(
    proposal: &PlatformMappingDraft,
    current: &PlatformMappingDraft,
    updates: &mut Vec<MappingPresetUpdate>,
) -> PlatformMappingDraft {
    let newly_detected = current.source == MappingSource::Unconfigured
        && current.preset_id.is_none()
        && current.custom_fields.values().all(|custom| !custom)
        && proposal.enabled;
    if newly_detected {
        return PlatformMappingDraft {
            id: current.id.clone(),
            ..proposal.clone()
        };
    }

    let mut merged = current.clone();
    merged.platform_name.clone_from(&proposal.platform_name);
    merged.platform_slug.clone_from(&proposal.platform_slug);

    let same_preset = current.preset_id.is_some() && current.preset_id == proposal.preset_id;
    let newer_version = same_preset
        && proposal.preset_version.unwrap_or_default() > current.preset_version.unwrap_or_default();
    if !newer_version {
        return merged;
    }

    let mut updated_fields = Vec::new();
    let preserve = |field: &str| current.custom_fields.get(field).copied().unwrap_or(false);
    if !preserve("romRoot") && merged.rom_root != proposal.rom_root {
        merged.rom_root.clone_from(&proposal.rom_root);
        updated_fields.push("romRoot".to_owned());
    }
    if !preserve("saveRoots") && merged.save_roots != proposal.save_roots {
        merged.save_roots.clone_from(&proposal.save_roots);
        updated_fields.push("saveRoots".to_owned());
    }
    if !preserve("stateRoots") && merged.state_roots != proposal.state_roots {
        merged.state_roots.clone_from(&proposal.state_roots);
        updated_fields.push("stateRoots".to_owned());
    }
    if !preserve("archivePolicy") && merged.archive_policy != proposal.archive_policy {
        merged.archive_policy = proposal.archive_policy;
        updated_fields.push("archivePolicy".to_owned());
    }
    if !preserve("filenameStrategy") && merged.filename_strategy != proposal.filename_strategy {
        merged
            .filename_strategy
            .clone_from(&proposal.filename_strategy);
        updated_fields.push("filenameStrategy".to_owned());
    }
    merged.preset_version = proposal.preset_version;
    if current.custom_fields.values().all(|custom| !custom) {
        merged.source = proposal.source;
    }
    let preserved_custom_fields = current
        .custom_fields
        .iter()
        .filter(|(_, custom)| **custom)
        .map(|(field, _)| field.clone())
        .collect();
    updates.push(MappingPresetUpdate {
        draft_id: merged.id.clone(),
        platform_id: merged.platform_id,
        preset_id: merged.preset_id.clone().unwrap_or_default(),
        from_version: current.preset_version,
        to_version: merged.preset_version.unwrap_or_default(),
        updated_fields,
        preserved_custom_fields,
    });
    merged
}

pub fn validate_mapping_drafts(
    drafts: &[PlatformMappingDraft],
    no_platforms: bool,
) -> MappingValidationResult {
    validate_mapping_drafts_with_access(drafts, no_platforms, true)
}

pub fn recheck_mapping_drafts(
    drafts: &[PlatformMappingDraft],
    no_platforms: bool,
) -> MappingValidationResult {
    validate_mapping_drafts_with_access(drafts, no_platforms, false)
}

fn validate_mapping_drafts_with_access(
    drafts: &[PlatformMappingDraft],
    no_platforms: bool,
    probe_write: bool,
) -> MappingValidationResult {
    let mut issues = Vec::new();
    let mut paths = Vec::new();
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
        validate_directory(
            &draft.id,
            "romRoot",
            &draft.rom_root,
            true,
            probe_write,
            &mut issues,
            &mut paths,
        );
        for (index, path) in draft.save_roots.iter().enumerate() {
            validate_directory(
                &draft.id,
                &format!("saveRoots.{index}"),
                path,
                false,
                probe_write,
                &mut issues,
                &mut paths,
            );
        }
        for (index, path) in draft.state_roots.iter().enumerate() {
            validate_directory(
                &draft.id,
                &format!("stateRoots.{index}"),
                path,
                false,
                probe_write,
                &mut issues,
                &mut paths,
            );
        }
    }

    validate_path_relationships(&mut paths, &mut issues);

    MappingValidationResult {
        valid: issues.is_empty(),
        issues,
        paths,
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
        paths: Vec::new(),
    }
}

fn validate_directory(
    draft_id: &str,
    field: &str,
    value: &str,
    required: bool,
    probe_write: bool,
    issues: &mut Vec<MappingValidationIssue>,
    paths: &mut Vec<MappingPathValidation>,
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
        paths.push(path_validation(
            draft_id,
            field,
            trimmed,
            MappingPathStatus::Unsafe,
        ));
        return;
    }

    let removable = is_removable_path(path);
    if !path.is_dir() {
        issues.push(issue(
            Some(draft_id),
            Some(field),
            "path_unavailable",
            &format!(
                "Cannot access {trimmed}: the directory does not exist or its storage is not mounted."
            ),
        ));
        let mut validation = path_validation(
            draft_id,
            field,
            trimmed,
            MappingPathStatus::TemporarilyUnavailable,
        );
        validation.removable = removable;
        paths.push(validation);
        return;
    }

    let canonical = match fs::canonicalize(path) {
        Ok(canonical) => canonical,
        Err(error) => {
            issues.push(issue(
                Some(draft_id),
                Some(field),
                "path_canonicalize_failed",
                &format!("Cannot resolve {trimmed}: {error}"),
            ));
            let mut validation = path_validation(
                draft_id,
                field,
                trimmed,
                MappingPathStatus::PermissionDenied,
            );
            validation.removable = removable;
            validation.mounted = true;
            paths.push(validation);
            return;
        }
    };
    let contains_symlink = contains_symlink(path);
    let readable = match fs::read_dir(&canonical) {
        Ok(_) => true,
        Err(error) => {
            issues.push(issue(
                Some(draft_id),
                Some(field),
                "path_read_denied",
                &format!("Cannot list {trimmed}: {error}"),
            ));
            false
        }
    };
    let writable = if probe_write {
        match verify_directory_write_access(&canonical) {
            Ok(()) => true,
            Err(error) => {
                issues.push(issue(
                    Some(draft_id),
                    Some(field),
                    "path_write_denied",
                    &format!("Cannot create, rename, or remove files in {trimmed}: {error}"),
                ));
                false
            }
        }
    } else {
        !fs::metadata(&canonical)
            .map(|metadata| metadata.permissions().readonly())
            .unwrap_or(true)
    };
    let available_bytes = match fs2::available_space(&canonical) {
        Ok(bytes) => Some(bytes),
        Err(error) => {
            issues.push(issue(
                Some(draft_id),
                Some(field),
                "path_space_unavailable",
                &format!("Cannot measure free space for {trimmed}: {error}"),
            ));
            None
        }
    };
    let status = if readable && writable && available_bytes.is_some() {
        MappingPathStatus::Ready
    } else {
        MappingPathStatus::PermissionDenied
    };
    paths.push(MappingPathValidation {
        draft_id: draft_id.to_owned(),
        field: field.to_owned(),
        path: trimmed.to_owned(),
        canonical_path: Some(display_path(&canonical)),
        status,
        readable,
        writable,
        available_bytes,
        removable,
        mounted: true,
        contains_symlink,
    });
}

fn path_validation(
    draft_id: &str,
    field: &str,
    path: &str,
    status: MappingPathStatus,
) -> MappingPathValidation {
    MappingPathValidation {
        draft_id: draft_id.to_owned(),
        field: field.to_owned(),
        path: path.to_owned(),
        canonical_path: None,
        status,
        readable: false,
        writable: false,
        available_bytes: None,
        removable: false,
        mounted: false,
        contains_symlink: false,
    }
}

#[cfg(windows)]
fn verify_directory_write_access(directory: &Path) -> std::io::Result<()> {
    use std::{os::windows::ffi::OsStrExt, ptr};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        Storage::FileSystem::{
            CreateFileW, FILE_ADD_FILE, FILE_ADD_SUBDIRECTORY, FILE_DELETE_CHILD,
            FILE_FLAG_BACKUP_SEMANTICS, FILE_LIST_DIRECTORY, FILE_SHARE_DELETE, FILE_SHARE_READ,
            FILE_SHARE_WRITE, OPEN_EXISTING,
        },
    };

    let mut wide = directory.as_os_str().encode_wide().collect::<Vec<_>>();
    wide.push(0);
    // SAFETY: `wide` is a null-terminated UTF-16 path. Null security/template handles are valid,
    // and the returned handle is closed below without escaping this function.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            FILE_LIST_DIRECTORY | FILE_ADD_FILE | FILE_ADD_SUBDIRECTORY | FILE_DELETE_CHILD,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `handle` is a valid handle returned by `CreateFileW` and is closed exactly once.
    let closed = unsafe { CloseHandle(handle) };
    if closed == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(unix)]
fn verify_directory_write_access(directory: &Path) -> std::io::Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};

    let path = CString::new(directory.as_os_str().as_bytes()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "directory path contains a null byte",
        )
    })?;
    // SAFETY: `path` is a valid null-terminated filesystem path for the duration of the call.
    if unsafe { libc::access(path.as_ptr(), libc::W_OK) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(any(unix, windows)))]
fn verify_directory_write_access(directory: &Path) -> std::io::Result<()> {
    if fs::metadata(directory)?.permissions().readonly() {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "directory is read-only",
        ))
    } else {
        Ok(())
    }
}

fn contains_symlink(path: &Path) -> bool {
    let mut current = PathBuf::new();
    path.components().any(|component| {
        current.push(component.as_os_str());
        fs::symlink_metadata(&current)
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false)
    })
}

fn validate_path_relationships(
    paths: &mut [MappingPathValidation],
    issues: &mut Vec<MappingValidationIssue>,
) {
    for left_index in 0..paths.len() {
        let Some(left_key) = paths[left_index]
            .canonical_path
            .as_deref()
            .map(normalized_path_key_from_str)
        else {
            continue;
        };
        for right_index in (left_index + 1)..paths.len() {
            let Some(right_key) = paths[right_index]
                .canonical_path
                .as_deref()
                .map(normalized_path_key_from_str)
            else {
                continue;
            };
            if !path_keys_overlap(&left_key, &right_key) {
                continue;
            }
            let duplicate = left_key == right_key;
            let both_roms =
                paths[left_index].field == "romRoot" && paths[right_index].field == "romRoot";
            let code = if duplicate && both_roms {
                "duplicate_destination"
            } else {
                "overlapping_roots"
            };
            let message = if duplicate && both_roms {
                format!(
                    "{} and {} use the same canonical ROM destination.",
                    paths[left_index].draft_id, paths[right_index].draft_id
                )
            } else {
                format!(
                    "{} overlaps {}; choose independent ROM, save, and state roots.",
                    paths[left_index].path, paths[right_index].path
                )
            };
            for index in [left_index, right_index] {
                paths[index].status = MappingPathStatus::Unsafe;
                issues.push(issue(
                    Some(&paths[index].draft_id),
                    Some(&paths[index].field),
                    code,
                    &message,
                ));
            }
        }
    }
}

fn normalized_path_key_from_str(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let normalized = normalized.trim_end_matches('/');
    if cfg!(windows) {
        normalized.to_lowercase()
    } else {
        normalized.to_owned()
    }
}

fn path_keys_overlap(left: &str, right: &str) -> bool {
    left == right
        || left
            .strip_prefix(right)
            .is_some_and(|suffix| suffix.starts_with('/'))
        || right
            .strip_prefix(left)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn display_path(path: &Path) -> String {
    let path = path.to_string_lossy();
    if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else {
        path.strip_prefix(r"\\?\").unwrap_or(&path).to_owned()
    }
}

#[cfg(windows)]
fn is_removable_path(path: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
    use windows_sys::Win32::System::WindowsProgramming::{
        DRIVE_CDROM, DRIVE_REMOTE, DRIVE_REMOVABLE,
    };

    let Some(prefix) = path.components().next() else {
        return false;
    };
    let root = PathBuf::from(prefix.as_os_str());
    let mut wide = root.as_os_str().encode_wide().collect::<Vec<_>>();
    if !wide.last().is_some_and(|value| *value == u16::from(b'\\')) {
        wide.push(u16::from(b'\\'));
    }
    wide.push(0);
    // SAFETY: `wide` is a null-terminated UTF-16 buffer that remains alive for the call.
    matches!(
        unsafe { GetDriveTypeW(wide.as_ptr()) },
        DRIVE_REMOVABLE | DRIVE_REMOTE | DRIVE_CDROM
    )
}

#[cfg(not(windows))]
fn is_removable_path(path: &Path) -> bool {
    ["/run/media", "/media", "/mnt"]
        .iter()
        .any(|root| path.starts_with(root))
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

    fn draft(id: &str, platform_id: i64, rom_root: &str) -> PlatformMappingDraft {
        PlatformMappingDraft {
            id: id.to_owned(),
            platform_id,
            platform_name: "Game Boy Advance".to_owned(),
            platform_slug: "gba".to_owned(),
            enabled: true,
            rom_root: rom_root.to_owned(),
            save_roots: vec!["/preset/saves-v1".to_owned()],
            state_roots: vec!["/preset/states-v1".to_owned()],
            archive_policy: ArchivePolicy::Keep,
            filename_strategy: "romm_filename".to_owned(),
            source: MappingSource::EmuDeckInternal,
            preset_id: Some("emudeck".to_owned()),
            preset_version: Some(1),
            custom_fields: BTreeMap::new(),
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
        assert!(Uuid::parse_str(&result.drafts[0].id).is_ok());
        assert!(Uuid::parse_str(&result.drafts[1].id).is_ok());
        assert_ne!(result.drafts[0].id, result.drafts[1].id);
    }

    #[test]
    fn emudeck_removable_detection_uses_platform_aliases_and_versioned_presets() {
        let temp = tempfile::tempdir().expect("temporary root");
        let root = temp.path().join("Emulation").join("roms");
        fs::create_dir_all(root.join("psx")).expect("PlayStation alias fixture");
        let result = detect_platform_mappings_with_roots(
            &[platform(7, "PlayStation", "ps")],
            vec![CandidateRoot {
                label: "Steam Deck microSD".to_owned(),
                path: root,
                source: MappingSource::EmuDeckRemovable,
                confidence: 90,
            }],
        );

        assert_eq!(result.detected_count, 1);
        assert_eq!(result.drafts[0].source, MappingSource::EmuDeckRemovable);
        assert_eq!(result.drafts[0].preset_id.as_deref(), Some("emudeck"));
        assert_eq!(result.drafts[0].preset_version, Some(1));
        assert!(result.drafts[0].rom_root.ends_with("psx"));
    }

    #[test]
    fn preset_upgrade_updates_owned_fields_and_preserves_custom_fields() {
        let mut current = draft("saved-id", 1, "/custom/roms");
        current.archive_policy = ArchivePolicy::ExtractKeep;
        current.custom_fields = BTreeMap::from([
            ("romRoot".to_owned(), true),
            ("archivePolicy".to_owned(), true),
        ]);
        current.source = MappingSource::Custom;

        let mut proposal = draft("new-random-id", 1, "/preset/roms-v2");
        proposal.save_roots = vec!["/preset/saves-v2".to_owned()];
        proposal.state_roots = vec!["/preset/states-v2".to_owned()];
        proposal.archive_policy = ArchivePolicy::ExtractDelete;
        proposal.filename_strategy = "preset_filename_v2".to_owned();
        proposal.preset_version = Some(2);
        let merged = merge_mapping_detection(
            MappingDetectionResult {
                platforms: vec![platform(1, "Game Boy Advance", "gba")],
                drafts: vec![proposal],
                evidence: Vec::new(),
                detected_count: 1,
                preset_updates: Vec::new(),
            },
            &[current],
        );

        let updated = &merged.drafts[0];
        assert_eq!(updated.id, "saved-id");
        assert_eq!(updated.rom_root, "/custom/roms");
        assert_eq!(updated.archive_policy, ArchivePolicy::ExtractKeep);
        assert_eq!(updated.save_roots, ["/preset/saves-v2"]);
        assert_eq!(updated.state_roots, ["/preset/states-v2"]);
        assert_eq!(updated.filename_strategy, "preset_filename_v2");
        assert_eq!(updated.preset_version, Some(2));
        assert_eq!(updated.source, MappingSource::Custom);
        assert_eq!(merged.preset_updates.len(), 1);
        assert_eq!(
            merged.preset_updates[0].updated_fields,
            ["saveRoots", "stateRoots", "filenameStrategy"]
        );
        assert!(
            merged.preset_updates[0]
                .preserved_custom_fields
                .contains(&"romRoot".to_owned())
        );
        assert!(
            merged.preset_updates[0]
                .preserved_custom_fields
                .contains(&"archivePolicy".to_owned())
        );
    }

    #[test]
    fn rescan_preserves_an_existing_mapping_when_its_mount_is_missing() {
        let current = draft("saved-id", 1, "/run/media/deck/SD/Emulation/roms/gba");
        let mut missing_proposal = draft("new-random-id", 1, "");
        missing_proposal.enabled = false;
        missing_proposal.source = MappingSource::Unconfigured;
        missing_proposal.preset_id = None;
        missing_proposal.preset_version = None;
        let merged = merge_mapping_detection(
            MappingDetectionResult {
                platforms: vec![platform(1, "GBA", "gba")],
                drafts: vec![missing_proposal],
                evidence: Vec::new(),
                detected_count: 0,
                preset_updates: Vec::new(),
            },
            std::slice::from_ref(&current),
        );

        assert_eq!(merged.drafts[0].id, current.id);
        assert!(merged.drafts[0].enabled);
        assert_eq!(merged.drafts[0].rom_root, current.rom_root);
        assert_eq!(merged.drafts[0].preset_version, current.preset_version);
        assert!(merged.preset_updates.is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn windows_overlap_keys_are_case_insensitive_and_component_aware() {
        let root = normalized_path_key_from_str(r"D:\Emulation\ROMS\GBA\");
        let same = normalized_path_key_from_str(r"d:/emulation/roms/gba");
        let nested = normalized_path_key_from_str(r"D:\Emulation\roms\gba\saves");
        let sibling = normalized_path_key_from_str(r"D:\Emulation\roms\gba-backup");
        assert_eq!(root, same);
        assert!(path_keys_overlap(&root, &nested));
        assert!(!path_keys_overlap(&root, &sibling));
    }

    #[cfg(unix)]
    #[test]
    fn linux_mounts_and_symlinked_roots_are_reported_without_rewriting_them() {
        use std::os::unix::fs::symlink;

        assert!(is_removable_path(Path::new(
            "/run/media/deck/SD/Emulation/roms/gba"
        )));
        let temp = tempfile::tempdir().expect("temporary root");
        let target = temp.path().join("target");
        let link = temp.path().join("linked-roms");
        fs::create_dir(&target).expect("target root");
        symlink(&target, &link).expect("symlink fixture");
        let mut mapping = draft("saved-id", 1, &link.to_string_lossy());
        mapping.save_roots.clear();
        mapping.state_roots.clear();
        let checked = validate_mapping_drafts(&[mapping], false);
        assert!(checked.valid, "unexpected issues: {:?}", checked.issues);
        assert!(checked.paths[0].contains_symlink);
        assert_eq!(
            checked.paths[0].canonical_path.as_deref(),
            Some(target.to_string_lossy().as_ref())
        );
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

    #[test]
    fn reports_canonical_access_and_space_without_writing() {
        let temp = tempfile::tempdir().expect("temporary root");
        let root = temp.path().join("roms");
        fs::create_dir(&root).expect("ROM root");
        let draft = PlatformMappingDraft {
            id: "platform-1".to_owned(),
            platform_id: 1,
            platform_name: "GBA".to_owned(),
            platform_slug: "gba".to_owned(),
            enabled: true,
            rom_root: root.to_string_lossy().into_owned(),
            save_roots: Vec::new(),
            state_roots: Vec::new(),
            archive_policy: ArchivePolicy::Keep,
            filename_strategy: "romm_filename".to_owned(),
            source: MappingSource::Custom,
            preset_id: None,
            preset_version: None,
            custom_fields: BTreeMap::new(),
        };

        let result = validate_mapping_drafts(&[draft], false);
        assert!(result.valid, "unexpected issues: {:?}", result.issues);
        assert_eq!(result.paths.len(), 1);
        let checked = &result.paths[0];
        assert_eq!(checked.status, MappingPathStatus::Ready);
        assert!(checked.readable);
        assert!(checked.writable);
        assert!(checked.mounted);
        assert!(checked.available_bytes.is_some());
        assert!(checked.canonical_path.is_some());
        assert_eq!(
            fs::read_dir(&root).expect("validated directory").count(),
            0,
            "access validation must not write to the selected root"
        );
    }

    #[test]
    fn blocks_overlapping_roots_and_marks_missing_storage_unavailable() {
        let temp = tempfile::tempdir().expect("temporary root");
        let rom_root = temp.path().join("roms");
        let nested_save = rom_root.join("saves");
        fs::create_dir_all(&nested_save).expect("nested fixture");
        let missing_state = temp.path().join("removed-drive");
        let draft = PlatformMappingDraft {
            id: "platform-1".to_owned(),
            platform_id: 1,
            platform_name: "GBA".to_owned(),
            platform_slug: "gba".to_owned(),
            enabled: true,
            rom_root: rom_root.to_string_lossy().into_owned(),
            save_roots: vec![nested_save.to_string_lossy().into_owned()],
            state_roots: vec![missing_state.to_string_lossy().into_owned()],
            archive_policy: ArchivePolicy::Keep,
            filename_strategy: "romm_filename".to_owned(),
            source: MappingSource::Custom,
            preset_id: None,
            preset_version: None,
            custom_fields: BTreeMap::new(),
        };

        let result = recheck_mapping_drafts(&[draft], false);
        assert!(!result.valid);
        assert!(
            result
                .issues
                .iter()
                .any(|issue| issue.code == "overlapping_roots")
        );
        assert_eq!(
            result
                .paths
                .iter()
                .find(|path| path.field == "stateRoots.0")
                .map(|path| path.status),
            Some(MappingPathStatus::TemporarilyUnavailable)
        );
    }

    #[test]
    fn directory_browser_uses_existing_ancestors_and_creation_requires_confirmation() {
        let temp = tempfile::tempdir().expect("temporary root");
        let parent = temp.path().join("roms");
        fs::create_dir(&parent).expect("parent directory");
        let missing = parent.join("gba").join("nested");
        let listing = browse_mapping_directories(Some(&missing.to_string_lossy()))
            .expect("nearest existing directory should be browsable");
        assert_eq!(
            listing.current_path.as_deref(),
            Some(parent.to_string_lossy().as_ref())
        );
        assert!(!listing.locations);

        let error = create_mapping_directory(&parent.to_string_lossy(), "gba", false)
            .expect_err("creation must require confirmation");
        assert_eq!(error.code, "directory_creation_confirmation_required");
        assert!(!parent.join("gba").exists());
        let error = create_mapping_directory(&parent.to_string_lossy(), "../escape", true)
            .expect_err("creation must accept one component only");
        assert_eq!(error.code, "invalid_directory_name");

        let created = create_mapping_directory(&parent.to_string_lossy(), "gba", true)
            .expect("confirmed final directory should be created");
        assert!(created.created);
        assert_eq!(created.path, parent.join("gba").to_string_lossy());
        assert!(parent.join("gba").is_dir());
        let existing = create_mapping_directory(&parent.to_string_lossy(), "gba", true)
            .expect("an existing directory should be selected idempotently");
        assert!(!existing.created);
    }
}
