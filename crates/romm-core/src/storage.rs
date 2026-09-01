use std::{
    env, fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use fs2::FileExt;
use romm_ipc::{
    ArchivePolicy, ArtworkKind, CollectionKind, DeviceIdentity, DevicePlatform,
    DeviceRegistrationState, DeviceSyncMode, GameDetails, LibraryMetadata, LibraryQuery,
    LibrarySort, LibrarySource, LibraryViewKind, LocalGameStatus, MappingSource, OnboardingState,
    PlatformMappingDraft, RomPage, RomSummary,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
#[cfg(unix)]
use std::fs::{File, OpenOptions};
use thiserror::Error;

pub const LIBRARY_STALE_AFTER_MS: i64 = 24 * 60 * 60 * 1_000;

const INITIAL_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY,
    applied_at_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS app_state (
    key TEXT PRIMARY KEY,
    value_json TEXT NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS server_profile (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    base_url TEXT NOT NULL,
    server_version TEXT,
    credential_locator TEXT,
    updated_at_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS device (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    romm_device_id TEXT NOT NULL,
    display_name TEXT NOT NULL,
    platform TEXT NOT NULL,
    hostname TEXT NOT NULL,
    sync_mode TEXT NOT NULL,
    registration_fingerprint TEXT NOT NULL,
    registered_at_ms INTEGER NOT NULL,
    verified_at_ms INTEGER
);
CREATE TABLE IF NOT EXISTS platform_mapping (
    id TEXT PRIMARY KEY,
    romm_platform_id INTEGER NOT NULL,
    preset_id TEXT,
    preset_version INTEGER,
    rom_root TEXT NOT NULL,
    save_roots_json TEXT NOT NULL,
    state_roots_json TEXT NOT NULL,
    archive_policy TEXT NOT NULL,
    filename_strategy TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    validation_status TEXT NOT NULL,
    custom_fields_json TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS rom_cache (
    romm_id INTEGER PRIMARY KEY,
    platform_id INTEGER,
    platform_name TEXT NOT NULL,
    title TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS collection_cache (
    romm_id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS downloaded_rom (
    romm_id INTEGER PRIMARY KEY,
    local_path TEXT NOT NULL,
    size_bytes INTEGER,
    checksum TEXT,
    updated_at_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS download_job (
    id TEXT PRIMARY KEY,
    romm_id INTEGER NOT NULL,
    state TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS local_asset (
    id TEXT PRIMARY KEY,
    romm_id INTEGER,
    asset_type TEXT NOT NULL,
    local_path TEXT NOT NULL,
    sha1 TEXT,
    updated_at_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS sync_journal (
    id TEXT PRIMARY KEY,
    operation TEXT NOT NULL,
    state TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS sync_conflict (
    id TEXT PRIMARY KEY,
    romm_id INTEGER,
    asset_type TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    resolved_at_ms INTEGER,
    created_at_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS cache_entry (
    key TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    local_path TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    etag TEXT,
    last_accessed_at_ms INTEGER NOT NULL,
    pinned INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS trusted_certificate (
    id TEXT PRIMARY KEY,
    server_origin TEXT NOT NULL,
    imported_at_ms INTEGER NOT NULL
);
"#;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("required environment variable {0} is unavailable")]
    MissingEnvironment(&'static str),
    #[error("unable to prepare application storage at {path}: {source}")]
    PrepareDirectory {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("another RomM Companion agent already owns {0}")]
    AlreadyRunning(PathBuf),
    #[error("unable to lock agent state at {path}: {source}")]
    Lock {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("database operation failed: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("stored application state {key} is invalid: {message}")]
    InvalidAppState { key: String, message: String },
    #[error("unable to preserve the corrupt database at {path}: {source}")]
    RecoveryCopy {
        path: PathBuf,
        source: std::io::Error,
    },
}

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub log_dir: PathBuf,
}

impl AppPaths {
    pub fn resolve() -> Result<Self, StorageError> {
        #[cfg(windows)]
        {
            let config_root = env::var_os("APPDATA")
                .map(PathBuf::from)
                .ok_or(StorageError::MissingEnvironment("APPDATA"))?;
            let local_root = env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .ok_or(StorageError::MissingEnvironment("LOCALAPPDATA"))?;
            Ok(Self::from_roots(
                config_root.join("RommCompanion"),
                local_root.join("RommCompanion"),
                local_root.join("RommCompanion").join("cache"),
            ))
        }
        #[cfg(target_os = "linux")]
        {
            let home = env::var_os("HOME")
                .map(PathBuf::from)
                .ok_or(StorageError::MissingEnvironment("HOME"))?;
            let config = env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".config"))
                .join("romm-companion");
            let data = env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".local/share"))
                .join("romm-companion");
            let cache = env::var_os("XDG_CACHE_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".cache"))
                .join("romm-companion");
            Ok(Self::from_roots(config, data, cache))
        }
        #[cfg(not(any(windows, target_os = "linux")))]
        {
            Err(StorageError::MissingEnvironment(
                "supported Windows or Linux environment",
            ))
        }
    }

    pub fn from_roots(config_dir: PathBuf, data_dir: PathBuf, cache_dir: PathBuf) -> Self {
        let log_dir = data_dir.join("logs");
        Self {
            config_dir,
            data_dir,
            cache_dir,
            log_dir,
        }
    }

    pub fn prepare(&self) -> Result<(), StorageError> {
        for path in [
            &self.config_dir,
            &self.data_dir,
            &self.cache_dir,
            &self.log_dir,
        ] {
            fs::create_dir_all(path).map_err(|source| StorageError::PrepareDirectory {
                path: path.clone(),
                source,
            })?;
        }
        Ok(())
    }

    pub fn database_path(&self) -> PathBuf {
        self.data_dir.join("romm-companion.sqlite3")
    }

    pub fn lock_path(&self) -> PathBuf {
        self.data_dir.join("agent.lock")
    }

    pub fn certificates_dir(&self) -> PathBuf {
        self.config_dir.join("certificates")
    }

    pub fn save_ca_certificate(&self, bytes: &[u8]) -> Result<String, StorageError> {
        let id = super::certificate_fingerprint(bytes);
        let directory = self.certificates_dir();
        fs::create_dir_all(&directory).map_err(|source| StorageError::PrepareDirectory {
            path: directory.clone(),
            source,
        })?;
        let path = directory.join(format!("{id}.cert"));
        fs::write(&path, bytes)
            .map_err(|source| StorageError::PrepareDirectory { path, source })?;
        Ok(id)
    }

    pub fn load_ca_certificate(&self, id: &str) -> Result<Vec<u8>, StorageError> {
        let path = self.certificates_dir().join(format!("{id}.cert"));
        fs::read(&path).map_err(|source| StorageError::PrepareDirectory { path, source })
    }
}

pub struct AgentLock {
    #[cfg(unix)]
    file: File,
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
}

impl AgentLock {
    pub fn acquire(paths: &AppPaths) -> Result<Self, StorageError> {
        paths.prepare()?;
        let path = paths.lock_path();
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            use windows_sys::Win32::{
                Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError},
                System::Threading::CreateMutexW,
            };

            let hash = path
                .to_string_lossy()
                .bytes()
                .fold(0xcbf29ce484222325_u64, |hash, byte| {
                    hash.wrapping_mul(0x100000001b3) ^ u64::from(byte.to_ascii_lowercase())
                });
            let name = format!("Local\\RommCompanionAgent-{hash:016x}");
            let encoded: Vec<u16> = std::ffi::OsStr::new(&name)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            let handle = unsafe { CreateMutexW(std::ptr::null(), 1, encoded.as_ptr()) };
            if handle.is_null() {
                return Err(StorageError::Lock {
                    path,
                    source: std::io::Error::last_os_error(),
                });
            }
            if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
                unsafe {
                    CloseHandle(handle);
                }
                return Err(StorageError::AlreadyRunning(path));
            }
            Ok(Self { handle })
        }

        #[cfg(unix)]
        {
            let file = OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .open(&path)
                .map_err(|source| StorageError::Lock {
                    path: path.clone(),
                    source,
                })?;
            file.try_lock_exclusive().map_err(|source| {
                if matches!(
                    source.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::PermissionDenied
                ) {
                    StorageError::AlreadyRunning(path.clone())
                } else {
                    StorageError::Lock {
                        path: path.clone(),
                        source,
                    }
                }
            })?;
            Ok(Self { file })
        }
    }
}

impl Drop for AgentLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        let _ = FileExt::unlock(&self.file);
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::{Foundation::CloseHandle, System::Threading::ReleaseMutex};
            ReleaseMutex(self.handle);
            CloseHandle(self.handle);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredServerProfile {
    pub base_url: String,
    pub server_version: Option<String>,
    pub credential_locator: Option<String>,
    pub http_approved: bool,
    pub ca_id: Option<String>,
    pub token_id: Option<i64>,
    pub account_id: Option<i64>,
    pub account_name: Option<String>,
    pub granted_scopes: Vec<String>,
    pub last_contact_at_ms: Option<i64>,
}

pub struct Database {
    connection: Connection,
    path: PathBuf,
}

impl Database {
    pub fn open(paths: &AppPaths) -> Result<Self, StorageError> {
        paths.prepare()?;
        let path = paths.database_path();
        let mut connection = Connection::open(&path)?;
        connection.busy_timeout(Duration::from_secs(5))?;

        let integrity: String = connection
            .query_row("PRAGMA quick_check(1)", [], |row| row.get(0))
            .unwrap_or_else(|_| "corrupt".to_owned());
        if integrity != "ok" {
            drop(connection);
            preserve_corrupt_database(&path)?;
            connection = Connection::open(&path)?;
            connection.busy_timeout(Duration::from_secs(5))?;
        }

        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "journal_mode", "WAL")?;

        run_migrations(&mut connection)?;
        Ok(Self { connection, path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load_server_profile(&self) -> Result<Option<StoredServerProfile>, StorageError> {
        self.connection
            .query_row(
                r#"SELECT base_url, server_version, credential_locator, http_approved, ca_id,
                          token_id, account_id, account_name, granted_scopes_json, last_contact_at_ms
                   FROM server_profile WHERE singleton = 1"#,
                [],
                |row| {
                    Ok(StoredServerProfile {
                        base_url: row.get(0)?,
                        server_version: row.get(1)?,
                        credential_locator: row.get(2)?,
                        http_approved: row.get::<_, i64>(3)? != 0,
                        ca_id: row.get(4)?,
                        token_id: row.get(5)?,
                        account_id: row.get(6)?,
                        account_name: row.get(7)?,
                        granted_scopes: row
                            .get::<_, Option<String>>(8)?
                            .and_then(|value| serde_json::from_str(&value).ok())
                            .unwrap_or_default(),
                        last_contact_at_ms: row.get(9)?,
                    })
                },
            )
            .optional()
            .map_err(StorageError::from)
    }

    pub fn save_server_profile(
        &self,
        base_url: &str,
        server_version: Option<&str>,
        credential_locator: Option<&str>,
    ) -> Result<(), StorageError> {
        self.connection.execute(
            r#"INSERT INTO server_profile(singleton, base_url, server_version, credential_locator, updated_at_ms)
               VALUES(1, ?1, ?2, ?3, ?4)
               ON CONFLICT(singleton) DO UPDATE SET
                 base_url = excluded.base_url,
                 server_version = excluded.server_version,
                 credential_locator = excluded.credential_locator,
                 updated_at_ms = excluded.updated_at_ms"#,
            params![base_url, server_version, credential_locator, now_ms()],
        )?;
        Ok(())
    }

    pub fn save_connection_profile(
        &self,
        profile: &StoredServerProfile,
    ) -> Result<(), StorageError> {
        let scopes = serde_json::to_string(&profile.granted_scopes).map_err(|error| {
            StorageError::Database(rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
        })?;
        self.connection.execute(
            r#"INSERT INTO server_profile(
                   singleton, base_url, server_version, credential_locator, updated_at_ms,
                   http_approved, ca_id, token_id, account_id, account_name,
                   granted_scopes_json, last_contact_at_ms
               ) VALUES(1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
               ON CONFLICT(singleton) DO UPDATE SET
                 base_url = excluded.base_url,
                 server_version = excluded.server_version,
                 credential_locator = excluded.credential_locator,
                 updated_at_ms = excluded.updated_at_ms,
                 http_approved = excluded.http_approved,
                 ca_id = excluded.ca_id,
                 token_id = excluded.token_id,
                 account_id = excluded.account_id,
                 account_name = excluded.account_name,
                 granted_scopes_json = excluded.granted_scopes_json,
                 last_contact_at_ms = excluded.last_contact_at_ms"#,
            params![
                &profile.base_url,
                profile.server_version.as_deref(),
                profile.credential_locator.as_deref(),
                now_ms(),
                if profile.http_approved { 1_i64 } else { 0_i64 },
                profile.ca_id.as_deref(),
                profile.token_id,
                profile.account_id,
                profile.account_name.as_deref(),
                scopes,
                profile.last_contact_at_ms,
            ],
        )?;
        Ok(())
    }

    pub fn clear_credential_locator(&self) -> Result<(), StorageError> {
        self.connection.execute(
            r#"UPDATE server_profile
               SET credential_locator = NULL,
                   token_id = NULL,
                   account_id = NULL,
                   account_name = NULL,
                   granted_scopes_json = NULL,
                   last_contact_at_ms = NULL,
                   updated_at_ms = ?1
               WHERE singleton = 1"#,
            [now_ms()],
        )?;
        Ok(())
    }

    pub fn load_device(&self) -> Result<Option<DeviceIdentity>, StorageError> {
        self.connection
            .query_row(
                r#"SELECT local_id, romm_device_id, display_name, platform, hostname,
                          client, client_version, sync_mode, registration_fingerprint,
                          registration_state, mapping_summary_json, created_at_ms,
                          registered_at_ms, verified_at_ms, updated_at_ms
                   FROM device WHERE singleton = 1"#,
                [],
                |row| {
                    let platform = parse_device_platform(&row.get::<_, String>(3)?)?;
                    let sync_mode = parse_device_sync_mode(&row.get::<_, String>(7)?)?;
                    let registration_state =
                        parse_device_registration_state(&row.get::<_, String>(9)?)?;
                    let mapping_summary_json = row.get::<_, String>(10)?;
                    let mapping_summary = serde_json::from_str::<
                        std::collections::BTreeMap<String, serde_json::Value>,
                    >(&mapping_summary_json)
                    .map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            10,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })?;
                    Ok(DeviceIdentity {
                        local_id: row.get(0)?,
                        romm_device_id: row.get(1)?,
                        display_name: row.get(2)?,
                        platform,
                        hostname: row.get(4)?,
                        client: row.get(5)?,
                        client_version: row.get(6)?,
                        sync_mode,
                        registration_fingerprint: row.get(8)?,
                        registration_state,
                        mapping_summary,
                        created_at_ms: row.get(11)?,
                        registered_at_ms: row.get(12)?,
                        verified_at_ms: row.get(13)?,
                        updated_at_ms: row.get(14)?,
                    })
                },
            )
            .optional()
            .map_err(StorageError::from)
    }

    pub fn save_device(&self, device: &DeviceIdentity) -> Result<(), StorageError> {
        let mapping_summary = serde_json::to_string(&device.mapping_summary).map_err(|error| {
            StorageError::Database(rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
        })?;
        self.connection.execute(
            r#"INSERT INTO device(
                   singleton, local_id, romm_device_id, display_name, platform, hostname,
                   client, client_version, sync_mode, registration_fingerprint,
                   registration_state, mapping_summary_json, created_at_ms, registered_at_ms,
                   verified_at_ms, updated_at_ms
               ) VALUES(1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
               ON CONFLICT(singleton) DO UPDATE SET
                 local_id = excluded.local_id,
                 romm_device_id = excluded.romm_device_id,
                 display_name = excluded.display_name,
                 platform = excluded.platform,
                 hostname = excluded.hostname,
                 client = excluded.client,
                 client_version = excluded.client_version,
                 sync_mode = excluded.sync_mode,
                 registration_fingerprint = excluded.registration_fingerprint,
                 registration_state = excluded.registration_state,
                 mapping_summary_json = excluded.mapping_summary_json,
                 created_at_ms = excluded.created_at_ms,
                 registered_at_ms = excluded.registered_at_ms,
                 verified_at_ms = excluded.verified_at_ms,
                 updated_at_ms = excluded.updated_at_ms"#,
            params![
                &device.local_id,
                device.romm_device_id.as_deref(),
                &device.display_name,
                device_platform_as_str(device.platform),
                &device.hostname,
                &device.client,
                &device.client_version,
                device_sync_mode_as_str(device.sync_mode),
                device.registration_fingerprint.as_deref(),
                device_registration_state_as_str(device.registration_state),
                mapping_summary,
                device.created_at_ms,
                device.registered_at_ms,
                device.verified_at_ms,
                device.updated_at_ms,
            ],
        )?;
        Ok(())
    }

    pub fn save_ca_assignment(&self, id: &str, server_origin: &str) -> Result<(), StorageError> {
        self.connection.execute(
            r#"INSERT INTO trusted_certificate(id, server_origin, imported_at_ms)
               VALUES(?1, ?2, ?3)
               ON CONFLICT(id) DO UPDATE SET
                 server_origin = excluded.server_origin,
                 imported_at_ms = excluded.imported_at_ms"#,
            params![id, server_origin, now_ms()],
        )?;
        Ok(())
    }

    pub fn ca_is_assigned_to(&self, id: &str, server_origin: &str) -> Result<bool, StorageError> {
        self.connection
            .query_row(
                "SELECT 1 FROM trusted_certificate WHERE id = ?1 AND server_origin = ?2",
                params![id, server_origin],
                |_| Ok(true),
            )
            .optional()
            .map(|value| value.unwrap_or(false))
            .map_err(StorageError::from)
    }

    pub fn get_app_state(&self, key: &str) -> Result<Option<String>, StorageError> {
        self.connection
            .query_row(
                "SELECT value_json FROM app_state WHERE key = ?1",
                [key],
                |row| row.get(0),
            )
            .optional()
            .map_err(StorageError::from)
    }

    pub fn set_app_state(&self, key: &str, value_json: &str) -> Result<(), StorageError> {
        self.connection.execute(
            r#"INSERT INTO app_state(key, value_json, updated_at_ms) VALUES(?1, ?2, ?3)
               ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at_ms = excluded.updated_at_ms"#,
            params![key, value_json, now_ms()],
        )?;
        Ok(())
    }

    pub fn load_onboarding_state(&self) -> Result<Option<OnboardingState>, StorageError> {
        self.get_app_state("onboarding_state")?
            .map(|value| {
                serde_json::from_str(&value).map_err(|error| StorageError::InvalidAppState {
                    key: "onboarding_state".to_owned(),
                    message: error.to_string(),
                })
            })
            .transpose()
    }

    pub fn save_onboarding_state(&self, state: &OnboardingState) -> Result<(), StorageError> {
        crate::onboarding::validate_onboarding_state(state).map_err(|message| {
            StorageError::InvalidAppState {
                key: "onboarding_state".to_owned(),
                message,
            }
        })?;
        let value =
            serde_json::to_string(state).map_err(|error| StorageError::InvalidAppState {
                key: "onboarding_state".to_owned(),
                message: error.to_string(),
            })?;
        self.set_app_state("onboarding_state", &value)
    }

    pub fn load_mapping_drafts(
        &self,
        server_origin: &str,
    ) -> Result<Vec<PlatformMappingDraft>, StorageError> {
        if self.get_app_state("mapping_draft_origin")?.as_deref() != Some(server_origin) {
            return Ok(Vec::new());
        }
        self.get_app_state("mapping_drafts")?
            .map(|value| {
                serde_json::from_str(&value).map_err(|error| StorageError::InvalidAppState {
                    key: "mapping_drafts".to_owned(),
                    message: error.to_string(),
                })
            })
            .transpose()
            .map(Option::unwrap_or_default)
    }

    pub fn save_mapping_drafts(
        &self,
        server_origin: &str,
        drafts: &[PlatformMappingDraft],
    ) -> Result<(), StorageError> {
        let value =
            serde_json::to_string(drafts).map_err(|error| StorageError::InvalidAppState {
                key: "mapping_drafts".to_owned(),
                message: error.to_string(),
            })?;
        self.set_app_state("mapping_drafts", &value)?;
        self.set_app_state("mapping_draft_origin", server_origin)
    }

    pub fn save_platform_mappings(
        &self,
        server_origin: &str,
        drafts: &[PlatformMappingDraft],
        no_platforms: bool,
    ) -> Result<(), StorageError> {
        let transaction = self.connection.unchecked_transaction()?;
        let updated_at_ms = now_ms();
        transaction.execute(
            r#"UPDATE platform_mapping
               SET enabled = 0, validation_status = 'disabled', updated_at_ms = ?1
               WHERE server_origin = ?2"#,
            params![updated_at_ms, server_origin],
        )?;
        for draft in drafts {
            transaction.execute(
                r#"INSERT INTO platform_mapping(
                       id, romm_platform_id, platform_name, platform_slug, preset_id,
                       preset_version, rom_root, save_roots_json, state_roots_json,
                       archive_policy, filename_strategy, enabled, validation_status,
                       custom_fields_json, source, updated_at_ms, server_origin
                   ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
                   ON CONFLICT(id) DO UPDATE SET
                       romm_platform_id = excluded.romm_platform_id,
                       platform_name = excluded.platform_name,
                       platform_slug = excluded.platform_slug,
                       preset_id = excluded.preset_id,
                       preset_version = excluded.preset_version,
                       rom_root = excluded.rom_root,
                       save_roots_json = excluded.save_roots_json,
                       state_roots_json = excluded.state_roots_json,
                       archive_policy = excluded.archive_policy,
                       filename_strategy = excluded.filename_strategy,
                       enabled = excluded.enabled,
                       validation_status = excluded.validation_status,
                       custom_fields_json = excluded.custom_fields_json,
                       source = excluded.source,
                       updated_at_ms = excluded.updated_at_ms,
                       server_origin = excluded.server_origin"#,
                params![
                    draft.id,
                    draft.platform_id,
                    draft.platform_name,
                    draft.platform_slug,
                    draft.preset_id,
                    draft.preset_version,
                    draft.rom_root,
                    serde_json::to_string(&draft.save_roots).unwrap_or_else(|_| "[]".to_owned()),
                    serde_json::to_string(&draft.state_roots).unwrap_or_else(|_| "[]".to_owned()),
                    archive_policy_as_str(draft.archive_policy),
                    draft.filename_strategy,
                    draft.enabled,
                    if draft.enabled { "valid" } else { "disabled" },
                    serde_json::to_string(&draft.custom_fields).unwrap_or_else(|_| "{}".to_owned()),
                    mapping_source_as_str(draft.source),
                    updated_at_ms,
                    server_origin,
                ],
            )?;
        }
        let no_platforms_json = if no_platforms { "true" } else { "false" };
        transaction.execute(
            r#"INSERT INTO app_state(key, value_json, updated_at_ms) VALUES('no_platforms_configured', ?1, ?2)
               ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at_ms = excluded.updated_at_ms"#,
            params![no_platforms_json, now_ms()],
        )?;
        transaction.execute(
            r#"INSERT INTO app_state(key, value_json, updated_at_ms) VALUES('mapping_server_origin', ?1, ?2)
               ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at_ms = excluded.updated_at_ms"#,
            params![server_origin, now_ms()],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn has_mapping_outcome(&self, server_origin: &str) -> Result<bool, StorageError> {
        if self.get_app_state("mapping_server_origin")?.as_deref() != Some(server_origin) {
            return Ok(false);
        }
        let count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM platform_mapping WHERE server_origin = ?1 AND enabled = 1",
            [server_origin],
            |row| row.get(0),
        )?;
        let no_platforms = self
            .get_app_state("no_platforms_configured")?
            .as_deref()
            .is_some_and(|value| value == "true");
        Ok(count > 0 || no_platforms)
    }

    pub fn save_library_page(
        &self,
        server_origin: &str,
        page: &RomPage,
    ) -> Result<(), StorageError> {
        self.save_library_view_page(server_origin, &LibraryQuery::all(), page)
    }

    pub fn save_library_view_page(
        &self,
        server_origin: &str,
        query: &LibraryQuery,
        page: &RomPage,
    ) -> Result<(), StorageError> {
        let transaction = self.connection.unchecked_transaction()?;
        save_library_page_rows(&transaction, server_origin, &query.cache_key(), page)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn load_library_page(
        &self,
        server_origin: &str,
        offset: u64,
        limit: u16,
    ) -> Result<Option<RomPage>, StorageError> {
        self.load_library_view_page(server_origin, &LibraryQuery::all(), offset, limit)
    }

    pub fn load_library_view_page(
        &self,
        server_origin: &str,
        query: &LibraryQuery,
        offset: u64,
        limit: u16,
    ) -> Result<Option<RomPage>, StorageError> {
        let cached = self
            .connection
            .query_row(
                r#"SELECT rom_ids_json, total, has_more, refreshed_at_ms
                   FROM library_page
                   WHERE server_origin = ?1 AND view_key = ?2
                     AND page_offset = ?3 AND page_limit = ?4"#,
                params![server_origin, query.cache_key(), offset, limit],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<u64>>(1)?,
                        row.get::<_, bool>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .optional()?;
        let Some((rom_ids_json, total, has_more, refreshed_at_ms)) = cached else {
            return Ok(None);
        };
        let rom_ids: Vec<i64> =
            serde_json::from_str(&rom_ids_json).map_err(|error| StorageError::InvalidAppState {
                key: "library_page.rom_ids".to_owned(),
                message: error.to_string(),
            })?;
        let mut items = Vec::with_capacity(rom_ids.len());
        for rom_id in rom_ids {
            let payload: Option<String> = self
                .connection
                .query_row(
                    r#"SELECT payload_json FROM library_rom
                       WHERE server_origin = ?1 AND romm_id = ?2"#,
                    params![server_origin, rom_id],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(payload) = payload else {
                return Ok(None);
            };
            let mut rom =
                serde_json::from_str(&payload).map_err(|error| StorageError::InvalidAppState {
                    key: format!("library_rom.{rom_id}"),
                    message: error.to_string(),
                })?;
            apply_local_game_state(&self.connection, &mut rom)?;
            items.push(rom);
        }
        Ok(Some(RomPage {
            items,
            offset,
            limit,
            total,
            has_more,
            source: LibrarySource::Cache,
            refreshed_at_ms,
            stale: now_ms().saturating_sub(refreshed_at_ms) > LIBRARY_STALE_AFTER_MS,
        }))
    }

    pub fn load_local_library_page(
        &self,
        server_origin: &str,
        query: &LibraryQuery,
        offset: u64,
        limit: u16,
    ) -> Result<RomPage, StorageError> {
        self.load_filtered_library_page(server_origin, query, offset, limit)
    }

    pub fn load_filtered_library_page(
        &self,
        server_origin: &str,
        query: &LibraryQuery,
        offset: u64,
        limit: u16,
    ) -> Result<RomPage, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT payload_json, refreshed_at_ms FROM library_rom WHERE server_origin = ?1",
        )?;
        let rows = statement
            .query_map([server_origin], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let refreshed_at_ms = rows
            .iter()
            .map(|(_, refreshed)| *refreshed)
            .max()
            .unwrap_or_else(now_ms);
        let mut items = Vec::new();
        for (payload, _) in &rows {
            let mut rom =
                serde_json::from_str(payload).map_err(|error| StorageError::InvalidAppState {
                    key: "library_rom.local_view".to_owned(),
                    message: error.to_string(),
                })?;
            apply_local_game_state(&self.connection, &mut rom)?;
            if library_query_matches(query, &rom) {
                items.push(rom);
            }
        }
        sort_library_items(&mut items, query.sort, query.kind);
        let total = items.len() as u64;
        let start = usize::try_from(offset)
            .unwrap_or(usize::MAX)
            .min(items.len());
        let end = start.saturating_add(usize::from(limit)).min(items.len());
        let items = items[start..end].to_vec();
        let loaded_through = offset.saturating_add(items.len() as u64);
        Ok(RomPage {
            items,
            offset,
            limit,
            total: Some(total),
            has_more: loaded_through < total,
            source: LibrarySource::Cache,
            refreshed_at_ms,
            stale: now_ms().saturating_sub(refreshed_at_ms) > LIBRARY_STALE_AFTER_MS,
        })
    }

    pub fn save_game_details(
        &self,
        server_origin: &str,
        details: &GameDetails,
    ) -> Result<(), StorageError> {
        let payload =
            serde_json::to_string(details).map_err(|error| StorageError::InvalidAppState {
                key: format!("library_game_detail.{}", details.rom.id),
                message: error.to_string(),
            })?;
        self.connection.execute(
            r#"INSERT INTO library_game_detail(server_origin, romm_id, payload_json, refreshed_at_ms)
               VALUES(?1, ?2, ?3, ?4)
               ON CONFLICT(server_origin, romm_id) DO UPDATE SET
                 payload_json = excluded.payload_json,
                 refreshed_at_ms = excluded.refreshed_at_ms"#,
            params![server_origin, details.rom.id, payload, details.refreshed_at_ms],
        )?;
        Ok(())
    }

    pub fn load_game_details(
        &self,
        server_origin: &str,
        rom_id: i64,
    ) -> Result<Option<GameDetails>, StorageError> {
        let payload = self.connection.query_row(
            "SELECT payload_json FROM library_game_detail WHERE server_origin = ?1 AND romm_id = ?2",
            params![server_origin, rom_id],
            |row| row.get::<_, String>(0),
        ).optional()?;
        let Some(payload) = payload else {
            return Ok(None);
        };
        let mut details: GameDetails =
            serde_json::from_str(&payload).map_err(|error| StorageError::InvalidAppState {
                key: format!("library_game_detail.{rom_id}"),
                message: error.to_string(),
            })?;
        apply_local_game_state(&self.connection, &mut details.rom)?;
        details.source = LibrarySource::Cache;
        details.stale = now_ms().saturating_sub(details.refreshed_at_ms) > LIBRARY_STALE_AFTER_MS;
        Ok(Some(details))
    }

    pub fn save_library_metadata(
        &self,
        server_origin: &str,
        metadata: &LibraryMetadata,
    ) -> Result<(), StorageError> {
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute(
            "DELETE FROM library_platform WHERE server_origin = ?1",
            [server_origin],
        )?;
        transaction.execute(
            "DELETE FROM library_collection WHERE server_origin = ?1",
            [server_origin],
        )?;
        for platform in &metadata.platforms {
            transaction.execute(
                r#"INSERT INTO library_platform(
                       server_origin, romm_id, name, slug, rom_count, payload_json, refreshed_at_ms
                   ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)"#,
                params![
                    server_origin,
                    platform.id,
                    platform.name,
                    platform.slug,
                    platform.rom_count,
                    serde_json::to_string(platform).map_err(|error| {
                        StorageError::InvalidAppState {
                            key: format!("library_platform.{}", platform.id),
                            message: error.to_string(),
                        }
                    })?,
                    metadata.refreshed_at_ms,
                ],
            )?;
        }
        for collection in &metadata.collections {
            transaction.execute(
                r#"INSERT INTO library_collection(
                       server_origin, romm_id, name, kind, rom_count, rom_ids_json,
                       payload_json, refreshed_at_ms
                   ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"#,
                params![
                    server_origin,
                    collection.id,
                    collection.name,
                    collection_kind_as_str(collection.kind),
                    collection.rom_count,
                    serde_json::to_string(&collection.rom_ids).unwrap_or_else(|_| "[]".to_owned()),
                    serde_json::to_string(collection).map_err(|error| {
                        StorageError::InvalidAppState {
                            key: format!("library_collection.{}", collection.id),
                            message: error.to_string(),
                        }
                    })?,
                    metadata.refreshed_at_ms,
                ],
            )?;
        }
        transaction.execute(
            r#"INSERT INTO library_snapshot(server_origin, refreshed_at_ms)
               VALUES(?1, ?2)
               ON CONFLICT(server_origin) DO UPDATE SET refreshed_at_ms = excluded.refreshed_at_ms"#,
            params![server_origin, metadata.refreshed_at_ms],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn load_library_metadata(
        &self,
        server_origin: &str,
    ) -> Result<Option<LibraryMetadata>, StorageError> {
        let refreshed_at_ms = self
            .connection
            .query_row(
                "SELECT refreshed_at_ms FROM library_snapshot WHERE server_origin = ?1",
                [server_origin],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        let Some(refreshed_at_ms) = refreshed_at_ms else {
            return Ok(None);
        };
        let mut platform_statement = self.connection.prepare(
            r#"SELECT payload_json FROM library_platform
               WHERE server_origin = ?1 ORDER BY lower(name), romm_id"#,
        )?;
        let platforms = platform_statement
            .query_map([server_origin], |row| row.get::<_, String>(0))?
            .map(|payload| deserialize_cache_payload("library_platform", payload))
            .collect::<Result<Vec<_>, _>>()?;
        let mut collection_statement = self.connection.prepare(
            r#"SELECT payload_json FROM library_collection
               WHERE server_origin = ?1 ORDER BY lower(name), romm_id"#,
        )?;
        let collections = collection_statement
            .query_map([server_origin], |row| row.get::<_, String>(0))?
            .map(|payload| deserialize_cache_payload("library_collection", payload))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Some(LibraryMetadata {
            platforms,
            collections,
            source: LibrarySource::Cache,
            refreshed_at_ms,
            stale: now_ms().saturating_sub(refreshed_at_ms) > LIBRARY_STALE_AFTER_MS,
        }))
    }

    pub fn save_initial_library_page(
        &self,
        server_origin: &str,
        page: &RomPage,
        onboarding: &OnboardingState,
    ) -> Result<(), StorageError> {
        crate::onboarding::validate_onboarding_state(onboarding).map_err(|message| {
            StorageError::InvalidAppState {
                key: "onboarding_state".to_owned(),
                message,
            }
        })?;
        let page_json =
            serde_json::to_string(page).map_err(|error| StorageError::InvalidAppState {
                key: "initial_library_page".to_owned(),
                message: error.to_string(),
            })?;
        let onboarding_json =
            serde_json::to_string(onboarding).map_err(|error| StorageError::InvalidAppState {
                key: "onboarding_state".to_owned(),
                message: error.to_string(),
            })?;
        let refreshed_at_ms = now_ms();
        let transaction = self.connection.unchecked_transaction()?;
        save_library_page_rows(&transaction, server_origin, "all", page)?;
        transaction.execute(
            "DELETE FROM rom_cache WHERE server_origin = ?1",
            [server_origin],
        )?;
        for rom in &page.items {
            transaction.execute(
                r#"INSERT INTO rom_cache(
                       romm_id, platform_id, platform_name, title, payload_json,
                       updated_at_ms, server_origin
                   ) VALUES(?1, NULL, ?2, ?3, ?4, ?5, ?6)
                   ON CONFLICT(romm_id) DO UPDATE SET
                       platform_id = excluded.platform_id,
                       platform_name = excluded.platform_name,
                       title = excluded.title,
                       payload_json = excluded.payload_json,
                       updated_at_ms = excluded.updated_at_ms,
                       server_origin = excluded.server_origin"#,
                params![
                    rom.id,
                    rom.platform,
                    rom.title,
                    serde_json::to_string(rom).unwrap_or_else(|_| "{}".to_owned()),
                    refreshed_at_ms,
                    server_origin,
                ],
            )?;
        }
        for (key, value) in [
            ("initial_library_page", page_json.as_str()),
            ("library_cache_origin", server_origin),
            ("onboarding_state", onboarding_json.as_str()),
        ] {
            transaction.execute(
                r#"INSERT INTO app_state(key, value_json, updated_at_ms) VALUES(?1, ?2, ?3)
                   ON CONFLICT(key) DO UPDATE SET
                       value_json = excluded.value_json,
                       updated_at_ms = excluded.updated_at_ms"#,
                params![key, value, refreshed_at_ms],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn load_initial_library_page(
        &self,
        server_origin: &str,
    ) -> Result<Option<RomPage>, StorageError> {
        if self.get_app_state("library_cache_origin")?.as_deref() != Some(server_origin) {
            return Ok(None);
        }
        self.get_app_state("initial_library_page")?
            .map(|value| {
                serde_json::from_str(&value).map_err(|error| StorageError::InvalidAppState {
                    key: "initial_library_page".to_owned(),
                    message: error.to_string(),
                })
            })
            .transpose()
    }

    pub fn checkpoint(&self) -> Result<(), StorageError> {
        self.connection
            .execute_batch("PRAGMA wal_checkpoint(PASSIVE)")?;
        Ok(())
    }
}

fn library_query_matches(query: &LibraryQuery, rom: &RomSummary) -> bool {
    if query.kind == LibraryViewKind::Favorites && !rom.user.favorite {
        return false;
    }
    if query.kind == LibraryViewKind::Downloaded && rom.local_status == LocalGameStatus::RemoteOnly
    {
        return false;
    }
    if query.kind == LibraryViewKind::ActiveDownloads && rom.active_download_id.is_none() {
        return false;
    }
    if query.favorite_only && !rom.user.favorite {
        return false;
    }
    if query.downloaded_only && rom.local_status == LocalGameStatus::RemoteOnly {
        return false;
    }
    if query
        .effective_platform_id()
        .is_some_and(|platform_id| rom.platform_id != Some(platform_id))
    {
        return false;
    }
    if query
        .effective_collection()
        .is_some_and(|(collection_id, _)| !rom.collection_ids.contains(&collection_id))
    {
        return false;
    }
    if let Some(search) = query.normalized_search() {
        let search = search.to_lowercase();
        let haystack = format!(
            "{} {} {} {}",
            rom.title,
            rom.platform,
            rom.remote_filename.as_deref().unwrap_or(""),
            rom.summary.as_deref().unwrap_or("")
        )
        .to_lowercase();
        if !search
            .split_whitespace()
            .all(|term| haystack.contains(term))
        {
            return false;
        }
    }
    true
}

fn sort_library_items(items: &mut [RomSummary], sort: LibrarySort, kind: LibraryViewKind) {
    match sort {
        LibrarySort::Title => items.sort_by(|left, right| {
            left.title
                .to_lowercase()
                .cmp(&right.title.to_lowercase())
                .then_with(|| left.id.cmp(&right.id))
        }),
        LibrarySort::RecentlyAdded => items.sort_by_key(|rom| std::cmp::Reverse(rom.id)),
        LibrarySort::ReleaseDate => items.sort_by(|left, right| {
            right
                .release_date_ms
                .cmp(&left.release_date_ms)
                .then_with(|| left.title.to_lowercase().cmp(&right.title.to_lowercase()))
        }),
        LibrarySort::Id if kind == LibraryViewKind::Recent => {
            items.sort_by_key(|rom| std::cmp::Reverse(rom.id));
        }
        LibrarySort::Id => items.sort_by_key(|rom| rom.id),
    }
}

fn save_library_page_rows(
    transaction: &Transaction<'_>,
    server_origin: &str,
    view_key: &str,
    page: &RomPage,
) -> Result<(), StorageError> {
    for rom in &page.items {
        let payload_json =
            serde_json::to_string(rom).map_err(|error| StorageError::InvalidAppState {
                key: format!("library_rom.{}", rom.id),
                message: error.to_string(),
            })?;
        transaction.execute(
            r#"INSERT INTO library_rom(
                   server_origin, romm_id, platform_id, platform_name, title, summary,
                   release_date_ms, remote_filename, remote_size_bytes, metadata_updated_at,
                   payload_json, refreshed_at_ms
               ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
               ON CONFLICT(server_origin, romm_id) DO UPDATE SET
                   platform_id = excluded.platform_id,
                   platform_name = excluded.platform_name,
                   title = excluded.title,
                   summary = excluded.summary,
                   release_date_ms = excluded.release_date_ms,
                   remote_filename = excluded.remote_filename,
                   remote_size_bytes = excluded.remote_size_bytes,
                   metadata_updated_at = excluded.metadata_updated_at,
                   payload_json = excluded.payload_json,
                   refreshed_at_ms = excluded.refreshed_at_ms"#,
            params![
                server_origin,
                rom.id,
                rom.platform_id,
                rom.platform,
                rom.title,
                rom.summary,
                rom.release_date_ms,
                rom.remote_filename,
                rom.remote_size_bytes,
                rom.metadata_updated_at,
                payload_json,
                page.refreshed_at_ms,
            ],
        )?;
        transaction.execute(
            r#"INSERT INTO library_user_rom(
                   server_origin, romm_id, favorite, hidden, rating, payload_json, refreshed_at_ms
               ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)
               ON CONFLICT(server_origin, romm_id) DO UPDATE SET
                   favorite = excluded.favorite,
                   hidden = excluded.hidden,
                   rating = excluded.rating,
                   payload_json = excluded.payload_json,
                   refreshed_at_ms = excluded.refreshed_at_ms"#,
            params![
                server_origin,
                rom.id,
                rom.user.favorite,
                rom.user.hidden,
                rom.user.rating,
                serde_json::to_string(&rom.user).unwrap_or_else(|_| "{}".to_owned()),
                page.refreshed_at_ms,
            ],
        )?;
        transaction.execute(
            "DELETE FROM library_artwork WHERE server_origin = ?1 AND romm_id = ?2",
            params![server_origin, rom.id],
        )?;
        for artwork in &rom.artwork {
            transaction.execute(
                r#"INSERT INTO library_artwork(
                       server_origin, romm_id, kind, cache_key, remote_path, remote_url,
                       refreshed_at_ms
                   ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)"#,
                params![
                    server_origin,
                    rom.id,
                    artwork_kind_as_str(artwork.kind),
                    artwork.cache_key,
                    artwork.remote_path,
                    artwork.remote_url,
                    page.refreshed_at_ms,
                ],
            )?;
        }
    }
    transaction.execute(
        r#"INSERT INTO library_page(
               server_origin, view_key, page_offset, page_limit, rom_ids_json, total, has_more,
               refreshed_at_ms
           ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
           ON CONFLICT(server_origin, view_key, page_offset, page_limit) DO UPDATE SET
               rom_ids_json = excluded.rom_ids_json,
               total = excluded.total,
               has_more = excluded.has_more,
               refreshed_at_ms = excluded.refreshed_at_ms"#,
        params![
            server_origin,
            view_key,
            page.offset,
            page.limit,
            serde_json::to_string(&page.items.iter().map(|rom| rom.id).collect::<Vec<_>>())
                .unwrap_or_else(|_| "[]".to_owned()),
            page.total,
            page.has_more,
            page.refreshed_at_ms,
        ],
    )?;
    Ok(())
}

fn apply_local_game_state(
    connection: &Connection,
    rom: &mut RomSummary,
) -> Result<(), StorageError> {
    let local_path = connection
        .query_row(
            "SELECT local_path FROM downloaded_rom WHERE romm_id = ?1",
            [rom.id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if let Some(path) = local_path {
        rom.local_status = if Path::new(&path).exists() {
            LocalGameStatus::Downloaded
        } else {
            LocalGameStatus::MissingLocal
        };
        rom.local_path = Some(path);
    } else {
        rom.local_status = LocalGameStatus::RemoteOnly;
        rom.local_path = None;
    }
    rom.active_download_id = connection
        .query_row(
            r#"SELECT id FROM download_job
               WHERE romm_id = ?1
                 AND state NOT IN ('complete', 'completed', 'cancelled', 'failed')
               ORDER BY updated_at_ms DESC LIMIT 1"#,
            [rom.id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    Ok(())
}

fn deserialize_cache_payload<T: serde::de::DeserializeOwned>(
    key: &str,
    payload: Result<String, rusqlite::Error>,
) -> Result<T, StorageError> {
    let payload = payload?;
    serde_json::from_str(&payload).map_err(|error| StorageError::InvalidAppState {
        key: key.to_owned(),
        message: error.to_string(),
    })
}

fn artwork_kind_as_str(kind: ArtworkKind) -> &'static str {
    match kind {
        ArtworkKind::CoverSmall => "cover_small",
        ArtworkKind::CoverLarge => "cover_large",
        ArtworkKind::RemoteCover => "remote_cover",
    }
}

fn collection_kind_as_str(kind: CollectionKind) -> &'static str {
    match kind {
        CollectionKind::Standard => "standard",
        CollectionKind::Smart => "smart",
    }
}

fn run_migrations(connection: &mut Connection) -> Result<(), StorageError> {
    let transaction = connection.transaction()?;
    transaction.execute_batch(INITIAL_SCHEMA)?;
    transaction.execute(
        "INSERT OR IGNORE INTO schema_migrations(version, applied_at_ms) VALUES(1, ?1)",
        [now_ms()],
    )?;
    let version: i64 = transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version < 2 {
        transaction.execute_batch(
            r#"ALTER TABLE server_profile ADD COLUMN http_approved INTEGER NOT NULL DEFAULT 0;
               ALTER TABLE server_profile ADD COLUMN ca_id TEXT;
               ALTER TABLE server_profile ADD COLUMN token_id INTEGER;
               ALTER TABLE server_profile ADD COLUMN account_id INTEGER;
               ALTER TABLE server_profile ADD COLUMN account_name TEXT;
               ALTER TABLE server_profile ADD COLUMN granted_scopes_json TEXT;
               ALTER TABLE server_profile ADD COLUMN last_contact_at_ms INTEGER;"#,
        )?;
        transaction.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at_ms) VALUES(2, ?1)",
            [now_ms()],
        )?;
    }
    if version < 3 {
        transaction.execute_batch(
            r#"ALTER TABLE device RENAME TO device_v2;
               CREATE TABLE device (
                   singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                   local_id TEXT NOT NULL,
                   romm_device_id TEXT,
                   display_name TEXT NOT NULL,
                   platform TEXT NOT NULL,
                   hostname TEXT NOT NULL,
                   client TEXT NOT NULL,
                   client_version TEXT NOT NULL,
                   sync_mode TEXT NOT NULL,
                   registration_fingerprint TEXT,
                   mapping_summary_json TEXT NOT NULL DEFAULT '{}',
                   created_at_ms INTEGER NOT NULL,
                   registered_at_ms INTEGER,
                   verified_at_ms INTEGER,
                   updated_at_ms INTEGER NOT NULL
               );
               INSERT INTO device(
                   singleton, local_id, romm_device_id, display_name, platform, hostname,
                   client, client_version, sync_mode, registration_fingerprint,
                   mapping_summary_json, created_at_ms, registered_at_ms, verified_at_ms,
                   updated_at_ms
               )
               SELECT singleton, registration_fingerprint, romm_device_id, display_name,
                      platform, hostname, 'romm-companion', '0.1.0', sync_mode,
                      registration_fingerprint, '{}', registered_at_ms, registered_at_ms,
                      verified_at_ms, registered_at_ms
               FROM device_v2;
               DROP TABLE device_v2;"#,
        )?;
        transaction.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at_ms) VALUES(3, ?1)",
            [now_ms()],
        )?;
    }
    if version < 4 {
        transaction.execute_batch(
            r#"ALTER TABLE device ADD COLUMN registration_state TEXT NOT NULL DEFAULT 'unregistered';
               UPDATE device SET registration_state = 'registered'
               WHERE romm_device_id IS NOT NULL;"#,
        )?;
        transaction.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at_ms) VALUES(4, ?1)",
            [now_ms()],
        )?;
    }
    if version < 5 {
        transaction.execute_batch(
            r#"ALTER TABLE platform_mapping ADD COLUMN platform_name TEXT NOT NULL DEFAULT '';
               ALTER TABLE platform_mapping ADD COLUMN platform_slug TEXT NOT NULL DEFAULT '';
               ALTER TABLE platform_mapping ADD COLUMN source TEXT NOT NULL DEFAULT 'custom';
               ALTER TABLE platform_mapping ADD COLUMN updated_at_ms INTEGER NOT NULL DEFAULT 0;"#,
        )?;
        transaction.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at_ms) VALUES(5, ?1)",
            [now_ms()],
        )?;
    }
    if version < 6 {
        transaction.execute_batch(
            r#"ALTER TABLE platform_mapping ADD COLUMN server_origin TEXT NOT NULL DEFAULT '';
               UPDATE platform_mapping
               SET server_origin = COALESCE(
                   (SELECT value_json FROM app_state WHERE key = 'mapping_server_origin'),
                   ''
               )
               WHERE server_origin = '';
               CREATE INDEX IF NOT EXISTS idx_platform_mapping_origin_enabled
               ON platform_mapping(server_origin, enabled);"#,
        )?;
        transaction.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at_ms) VALUES(6, ?1)",
            [now_ms()],
        )?;
    }
    if version < 7 {
        transaction.execute_batch(
            r#"ALTER TABLE rom_cache ADD COLUMN server_origin TEXT NOT NULL DEFAULT '';
               CREATE INDEX IF NOT EXISTS idx_rom_cache_origin_updated
               ON rom_cache(server_origin, updated_at_ms);"#,
        )?;
        transaction.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at_ms) VALUES(7, ?1)",
            [now_ms()],
        )?;
    }
    if version < 8 {
        transaction.execute_batch(
            r#"CREATE TABLE library_rom (
                   server_origin TEXT NOT NULL,
                   romm_id INTEGER NOT NULL,
                   platform_id INTEGER,
                   platform_name TEXT NOT NULL,
                   title TEXT NOT NULL,
                   summary TEXT,
                   release_date_ms INTEGER,
                   remote_filename TEXT,
                   remote_size_bytes INTEGER,
                   metadata_updated_at TEXT,
                   payload_json TEXT NOT NULL,
                   refreshed_at_ms INTEGER NOT NULL,
                   PRIMARY KEY(server_origin, romm_id)
               );
               CREATE INDEX idx_library_rom_origin_platform
               ON library_rom(server_origin, platform_id, romm_id);
               CREATE TABLE library_user_rom (
                   server_origin TEXT NOT NULL,
                   romm_id INTEGER NOT NULL,
                   favorite INTEGER NOT NULL DEFAULT 0,
                   hidden INTEGER NOT NULL DEFAULT 0,
                   rating INTEGER NOT NULL DEFAULT 0,
                   payload_json TEXT NOT NULL,
                   refreshed_at_ms INTEGER NOT NULL,
                   PRIMARY KEY(server_origin, romm_id)
               );
               CREATE TABLE library_artwork (
                   server_origin TEXT NOT NULL,
                   romm_id INTEGER NOT NULL,
                   kind TEXT NOT NULL,
                   cache_key TEXT NOT NULL,
                   remote_path TEXT,
                   remote_url TEXT,
                   refreshed_at_ms INTEGER NOT NULL,
                   PRIMARY KEY(server_origin, romm_id, kind, cache_key)
               );
               CREATE TABLE library_page (
                   server_origin TEXT NOT NULL,
                   page_offset INTEGER NOT NULL,
                   page_limit INTEGER NOT NULL,
                   rom_ids_json TEXT NOT NULL,
                   total INTEGER,
                   has_more INTEGER NOT NULL,
                   refreshed_at_ms INTEGER NOT NULL,
                   PRIMARY KEY(server_origin, page_offset, page_limit)
               );
               CREATE TABLE library_platform (
                   server_origin TEXT NOT NULL,
                   romm_id INTEGER NOT NULL,
                   name TEXT NOT NULL,
                   slug TEXT NOT NULL,
                   rom_count INTEGER,
                   payload_json TEXT NOT NULL,
                   refreshed_at_ms INTEGER NOT NULL,
                   PRIMARY KEY(server_origin, romm_id)
               );
               CREATE TABLE library_collection (
                   server_origin TEXT NOT NULL,
                   romm_id INTEGER NOT NULL,
                   name TEXT NOT NULL,
                   kind TEXT NOT NULL,
                   rom_count INTEGER,
                   rom_ids_json TEXT NOT NULL,
                   payload_json TEXT NOT NULL,
                   refreshed_at_ms INTEGER NOT NULL,
                   PRIMARY KEY(server_origin, kind, romm_id)
               );
               CREATE TABLE library_snapshot (
                   server_origin TEXT PRIMARY KEY,
                   refreshed_at_ms INTEGER NOT NULL
               );
               INSERT OR IGNORE INTO library_rom(
                   server_origin, romm_id, platform_id, platform_name, title,
                   payload_json, refreshed_at_ms
               )
               SELECT server_origin, romm_id, platform_id, platform_name, title,
                      payload_json, updated_at_ms
               FROM rom_cache
               WHERE server_origin != '';
               CREATE INDEX idx_library_collection_origin_name
               ON library_collection(server_origin, name);
               CREATE INDEX idx_library_artwork_origin_rom
               ON library_artwork(server_origin, romm_id);"#,
        )?;
        transaction.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at_ms) VALUES(8, ?1)",
            [now_ms()],
        )?;
    }
    if version < 9 {
        transaction.execute_batch(
            r#"ALTER TABLE library_page RENAME TO library_page_v8;
               CREATE TABLE library_page (
                   server_origin TEXT NOT NULL,
                   view_key TEXT NOT NULL,
                   page_offset INTEGER NOT NULL,
                   page_limit INTEGER NOT NULL,
                   rom_ids_json TEXT NOT NULL,
                   total INTEGER,
                   has_more INTEGER NOT NULL,
                   refreshed_at_ms INTEGER NOT NULL,
                   PRIMARY KEY(server_origin, view_key, page_offset, page_limit)
               );
               INSERT INTO library_page(
                   server_origin, view_key, page_offset, page_limit, rom_ids_json,
                   total, has_more, refreshed_at_ms
               )
               SELECT server_origin, 'all', page_offset, page_limit, rom_ids_json,
                      total, has_more, refreshed_at_ms
               FROM library_page_v8;
               DROP TABLE library_page_v8;
               CREATE INDEX idx_library_page_origin_view
               ON library_page(server_origin, view_key, page_offset);"#,
        )?;
        transaction.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at_ms) VALUES(9, ?1)",
            [now_ms()],
        )?;
    }
    if version < 10 {
        transaction.execute_batch(
            r#"CREATE TABLE library_game_detail (
                   server_origin TEXT NOT NULL,
                   romm_id INTEGER NOT NULL,
                   payload_json TEXT NOT NULL,
                   refreshed_at_ms INTEGER NOT NULL,
                   PRIMARY KEY(server_origin, romm_id)
               );"#,
        )?;
        transaction.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at_ms) VALUES(10, ?1)",
            [now_ms()],
        )?;
    }
    transaction.pragma_update(None, "user_version", 10)?;
    transaction.commit()?;
    Ok(())
}

fn archive_policy_as_str(policy: ArchivePolicy) -> &'static str {
    match policy {
        ArchivePolicy::Keep => "keep",
        ArchivePolicy::ExtractKeep => "extract_keep",
        ArchivePolicy::ExtractDelete => "extract_delete",
    }
}

fn mapping_source_as_str(source: MappingSource) -> &'static str {
    match source {
        MappingSource::Unconfigured => "unconfigured",
        MappingSource::EmuDeckInternal => "emudeck_internal",
        MappingSource::EmuDeckRemovable => "emudeck_removable",
        MappingSource::Custom => "custom",
    }
}

fn device_platform_as_str(platform: DevicePlatform) -> &'static str {
    match platform {
        DevicePlatform::Windows => "windows",
        DevicePlatform::SteamOs => "steamos",
        DevicePlatform::Linux => "linux",
    }
}

fn parse_device_platform(value: &str) -> Result<DevicePlatform, rusqlite::Error> {
    match value {
        "windows" => Ok(DevicePlatform::Windows),
        "steamos" => Ok(DevicePlatform::SteamOs),
        "linux" => Ok(DevicePlatform::Linux),
        _ => Err(invalid_device_field("platform", value)),
    }
}

fn device_sync_mode_as_str(sync_mode: DeviceSyncMode) -> &'static str {
    match sync_mode {
        DeviceSyncMode::PushPull => "push_pull",
    }
}

fn parse_device_sync_mode(value: &str) -> Result<DeviceSyncMode, rusqlite::Error> {
    match value {
        "push_pull" => Ok(DeviceSyncMode::PushPull),
        _ => Err(invalid_device_field("sync_mode", value)),
    }
}

fn device_registration_state_as_str(state: DeviceRegistrationState) -> &'static str {
    match state {
        DeviceRegistrationState::Unregistered => "unregistered",
        DeviceRegistrationState::Registered => "registered",
        DeviceRegistrationState::Missing => "missing",
        DeviceRegistrationState::PermissionError => "permission_error",
    }
}

fn parse_device_registration_state(
    value: &str,
) -> Result<DeviceRegistrationState, rusqlite::Error> {
    match value {
        "unregistered" => Ok(DeviceRegistrationState::Unregistered),
        "registered" => Ok(DeviceRegistrationState::Registered),
        "missing" => Ok(DeviceRegistrationState::Missing),
        "permission_error" => Ok(DeviceRegistrationState::PermissionError),
        _ => Err(invalid_device_field("registration_state", value)),
    }
}

fn invalid_device_field(field: &str, value: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("invalid device {field}: {value}"),
        )),
    )
}

fn preserve_corrupt_database(path: &Path) -> Result<(), StorageError> {
    if !path.exists() {
        return Ok(());
    }
    let backup = path.with_extension(format!("corrupt-{}.sqlite3", now_ms()));
    fs::copy(path, &backup).map_err(|source| StorageError::RecoveryCopy {
        path: backup,
        source,
    })?;
    fs::remove_file(path).map_err(|source| StorageError::RecoveryCopy {
        path: path.to_path_buf(),
        source,
    })?;
    for suffix in ["-wal", "-shm"] {
        let sidecar = PathBuf::from(format!("{}{}", path.display(), suffix));
        if sidecar.exists() {
            let _ = fs::remove_file(sidecar);
        }
    }
    Ok(())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library_rom(id: i64, title: &str) -> RomSummary {
        RomSummary {
            id,
            title: title.to_owned(),
            platform: "SNES".to_owned(),
            platform_id: Some(7),
            user: romm_ipc::UserRomState {
                rom_id: id,
                favorite: true,
                ..Default::default()
            },
            artwork: vec![romm_ipc::ArtworkReference {
                kind: ArtworkKind::CoverSmall,
                remote_path: Some(format!("/assets/rom/{id}/cover.webp")),
                remote_url: None,
                cache_key: format!("rom:{id}:cover-small"),
            }],
            ..RomSummary::default()
        }
    }

    fn library_page(id: i64, title: &str, refreshed_at_ms: i64) -> RomPage {
        RomPage {
            items: vec![library_rom(id, title)],
            offset: 0,
            limit: 48,
            total: Some(1),
            has_more: false,
            source: LibrarySource::Live,
            refreshed_at_ms,
            stale: false,
        }
    }

    fn mapping_draft(id: &str, platform_id: i64) -> PlatformMappingDraft {
        PlatformMappingDraft {
            id: id.to_owned(),
            platform_id,
            platform_name: format!("Platform {platform_id}"),
            platform_slug: format!("platform-{platform_id}"),
            enabled: true,
            rom_root: format!("C:/Roms/{platform_id}"),
            save_roots: Vec::new(),
            state_roots: Vec::new(),
            archive_policy: ArchivePolicy::Keep,
            filename_strategy: "romm_filename".to_owned(),
            source: MappingSource::Custom,
            preset_id: None,
            preset_version: None,
            custom_fields: Default::default(),
        }
    }

    #[test]
    fn creates_the_initial_schema_and_persists_profile_state() {
        let temporary = tempfile::tempdir().expect("temporary directory should be created");
        let paths = AppPaths::from_roots(
            temporary.path().join("config"),
            temporary.path().join("data"),
            temporary.path().join("cache"),
        );
        let database = Database::open(&paths).expect("database should open");
        database
            .save_server_profile(
                "https://romm.example.test/",
                Some("5.0.0"),
                Some("system:romm-client-token"),
            )
            .expect("profile should save");
        database
            .set_app_state("desktop_settings", r#"{"fullscreen":true}"#)
            .expect("state should save");

        assert_eq!(
            database.load_server_profile().expect("profile should load"),
            Some(StoredServerProfile {
                base_url: "https://romm.example.test/".to_owned(),
                server_version: Some("5.0.0".to_owned()),
                credential_locator: Some("system:romm-client-token".to_owned()),
                http_approved: false,
                ca_id: None,
                token_id: None,
                account_id: None,
                account_name: None,
                granted_scopes: Vec::new(),
                last_contact_at_ms: None,
            })
        );
        assert_eq!(
            database
                .get_app_state("desktop_settings")
                .expect("state should load")
                .as_deref(),
            Some(r#"{"fullscreen":true}"#)
        );
    }

    #[test]
    fn onboarding_state_round_trips_without_credentials() {
        let temporary = tempfile::tempdir().expect("temporary directory should be created");
        let paths = AppPaths::from_roots(
            temporary.path().join("config"),
            temporary.path().join("data"),
            temporary.path().join("cache"),
        );
        let database = Database::open(&paths).expect("database should open");
        let state = crate::onboarding::select_onboarding_server(
            &OnboardingState::default(),
            "https://romm.example.test",
            false,
        )
        .expect("server should validate");
        database
            .save_onboarding_state(&state)
            .expect("onboarding should save");
        assert_eq!(
            database
                .load_onboarding_state()
                .expect("onboarding should load"),
            Some(state)
        );
        let raw = database
            .get_app_state("onboarding_state")
            .expect("raw onboarding should load")
            .expect("raw onboarding should exist");
        assert!(!raw.contains("rmm_"));
        assert!(!raw.to_ascii_lowercase().contains("token"));
    }

    #[test]
    fn an_agent_lock_is_released_for_the_next_owner() {
        let temporary = tempfile::tempdir().expect("temporary directory should be created");
        let paths = AppPaths::from_roots(
            temporary.path().join("config"),
            temporary.path().join("data"),
            temporary.path().join("cache"),
        );
        let first = AgentLock::acquire(&paths).expect("first lock should succeed");
        #[cfg(unix)]
        assert!(paths.lock_path().is_file());
        drop(first);
        AgentLock::acquire(&paths).expect("the next owner should acquire a released lock");
    }

    #[test]
    fn an_agent_lock_excludes_another_process() {
        let temporary = tempfile::tempdir().expect("temporary directory should be created");
        let paths = AppPaths::from_roots(
            temporary.path().join("config"),
            temporary.path().join("data"),
            temporary.path().join("cache"),
        );
        let _first = AgentLock::acquire(&paths).expect("first lock should succeed");
        let status = std::process::Command::new(
            std::env::current_exe().expect("test executable should be discoverable"),
        )
        .args([
            "--exact",
            "storage::tests::agent_lock_child_process",
            "--nocapture",
        ])
        .env("ROMM_COMPANION_LOCK_TEST_ROOT", temporary.path())
        .status()
        .expect("child lock test should launch");
        assert!(status.success(), "a second process acquired the agent lock");
    }

    #[test]
    fn agent_lock_child_process() {
        let Some(root) = std::env::var_os("ROMM_COMPANION_LOCK_TEST_ROOT") else {
            return;
        };
        let root = PathBuf::from(root);
        let paths =
            AppPaths::from_roots(root.join("config"), root.join("data"), root.join("cache"));
        assert!(matches!(
            AgentLock::acquire(&paths),
            Err(StorageError::AlreadyRunning(_))
        ));
    }

    #[test]
    fn preserves_and_replaces_a_corrupt_database() {
        let temporary = tempfile::tempdir().expect("temporary directory should be created");
        let paths = AppPaths::from_roots(
            temporary.path().join("config"),
            temporary.path().join("data"),
            temporary.path().join("cache"),
        );
        paths.prepare().expect("paths should be prepared");
        fs::write(paths.database_path(), b"not a sqlite database")
            .expect("corrupt fixture should be written");

        let database = Database::open(&paths).expect("database should recover");
        assert!(database.load_server_profile().is_ok());
        let recovered = fs::read_dir(&paths.data_dir)
            .expect("data directory should be readable")
            .filter_map(Result::ok)
            .any(|entry| entry.file_name().to_string_lossy().contains(".corrupt-"));
        assert!(recovered, "the corrupt database should be preserved");
    }

    #[test]
    fn persists_auth_metadata_and_scopes_ca_to_one_origin() {
        let temporary = tempfile::tempdir().expect("temporary directory should be created");
        let paths = AppPaths::from_roots(
            temporary.path().join("config"),
            temporary.path().join("data"),
            temporary.path().join("cache"),
        );
        let database = Database::open(&paths).expect("database should open");
        database
            .save_ca_assignment("abc123", "https://romm.example.test")
            .expect("CA assignment should save");
        assert!(
            database
                .ca_is_assigned_to("abc123", "https://romm.example.test")
                .expect("CA assignment should load")
        );
        assert!(
            !database
                .ca_is_assigned_to("abc123", "https://other.example.test")
                .expect("CA assignment should stay scoped")
        );

        let profile = StoredServerProfile {
            base_url: "https://romm.example.test/".to_owned(),
            server_version: Some("5.0.0".to_owned()),
            credential_locator: Some("system:romm-client-token".to_owned()),
            http_approved: false,
            ca_id: Some("abc123".to_owned()),
            token_id: Some(42),
            account_id: Some(7),
            account_name: Some("justin".to_owned()),
            granted_scopes: vec!["me.read".to_owned(), "roms.read".to_owned()],
            last_contact_at_ms: Some(1234),
        };
        database
            .save_connection_profile(&profile)
            .expect("authenticated profile should save");
        assert_eq!(
            database.load_server_profile().expect("profile should load"),
            Some(profile)
        );
    }

    #[test]
    fn mapping_outcomes_do_not_leak_between_server_origins() {
        let temporary = tempfile::tempdir().expect("temporary directory should be created");
        let paths = AppPaths::from_roots(
            temporary.path().join("config"),
            temporary.path().join("data"),
            temporary.path().join("cache"),
        );
        let database = Database::open(&paths).expect("database should open");
        let first_origin = "https://first.example.test";
        let replacement_origin = "https://replacement.example.test";

        database
            .save_platform_mappings(first_origin, &[mapping_draft("platform-1", 1)], false)
            .expect("first server mapping should save");
        assert!(
            database
                .has_mapping_outcome(first_origin)
                .expect("first server outcome should load")
        );

        database
            .save_platform_mappings(replacement_origin, &[], false)
            .expect("replacement server outcome should save");
        assert!(
            !database
                .has_mapping_outcome(replacement_origin)
                .expect("replacement server outcome should load"),
            "an enabled mapping owned by another server must not satisfy onboarding"
        );
        let first_origin_enabled: i64 = database
            .connection
            .query_row(
                "SELECT COUNT(*) FROM platform_mapping WHERE server_origin = ?1 AND enabled = 1",
                [first_origin],
                |row| row.get(0),
            )
            .expect("first server mappings should remain queryable");
        assert_eq!(first_origin_enabled, 1);
    }

    #[test]
    fn replacing_a_mapping_selection_disables_stale_rows_for_that_server() {
        let temporary = tempfile::tempdir().expect("temporary directory should be created");
        let paths = AppPaths::from_roots(
            temporary.path().join("config"),
            temporary.path().join("data"),
            temporary.path().join("cache"),
        );
        let database = Database::open(&paths).expect("database should open");
        let origin = "https://romm.example.test";

        database
            .save_platform_mappings(
                origin,
                &[
                    mapping_draft("platform-1", 1),
                    mapping_draft("platform-2", 2),
                ],
                false,
            )
            .expect("initial mapping selection should save");
        database
            .save_platform_mappings(origin, &[mapping_draft("platform-1", 1)], false)
            .expect("replacement mapping selection should save");

        let stale_row: (i64, String) = database
            .connection
            .query_row(
                "SELECT enabled, validation_status FROM platform_mapping WHERE id = 'platform-2'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("stale mapping should remain recoverable");
        assert_eq!(stale_row, (0, "disabled".to_owned()));
        assert!(
            database
                .has_mapping_outcome(origin)
                .expect("current mapping outcome should load")
        );
    }

    #[test]
    fn an_explicit_no_platform_outcome_is_scoped_to_its_server() {
        let temporary = tempfile::tempdir().expect("temporary directory should be created");
        let paths = AppPaths::from_roots(
            temporary.path().join("config"),
            temporary.path().join("data"),
            temporary.path().join("cache"),
        );
        let database = Database::open(&paths).expect("database should open");
        let first_origin = "https://first.example.test";
        let replacement_origin = "https://replacement.example.test";

        database
            .save_platform_mappings(first_origin, &[mapping_draft("platform-1", 1)], false)
            .expect("first server mapping should save");
        database
            .save_platform_mappings(replacement_origin, &[], true)
            .expect("no-platform outcome should save");

        assert!(
            database
                .has_mapping_outcome(replacement_origin)
                .expect("replacement server outcome should load")
        );
        assert!(
            !database
                .has_mapping_outcome(first_origin)
                .expect("inactive server outcome should load"),
            "the current server's explicit outcome must not apply to another origin"
        );
    }

    #[test]
    fn initial_library_page_and_completion_marker_persist_together_per_origin() {
        let temporary = tempfile::tempdir().expect("temporary directory should be created");
        let paths = AppPaths::from_roots(
            temporary.path().join("config"),
            temporary.path().join("data"),
            temporary.path().join("cache"),
        );
        let database = Database::open(&paths).expect("database should open");
        let origin = "https://romm.example.test";
        let page = RomPage {
            items: vec![romm_ipc::RomSummary {
                id: 42,
                title: "Chrono Trigger".to_owned(),
                platform: "SNES".to_owned(),
                ..romm_ipc::RomSummary::default()
            }],
            offset: 0,
            limit: 48,
            total: Some(107),
            has_more: true,
            source: LibrarySource::Live,
            refreshed_at_ms: 1234,
            stale: false,
        };
        let onboarding = OnboardingState {
            current_step: romm_ipc::OnboardingStep::FirstRefresh,
            highest_completed_step: Some(romm_ipc::OnboardingStep::FirstRefresh),
            completed_steps: crate::onboarding::ONBOARDING_STEPS.to_vec(),
            server_origin: Some(origin.to_owned()),
            background_enabled: Some(false),
            ..OnboardingState::default()
        };

        database
            .save_initial_library_page(origin, &page, &onboarding)
            .expect("initial library transaction should save");

        assert_eq!(
            database
                .load_initial_library_page(origin)
                .expect("initial page should load"),
            Some(page)
        );
        assert_eq!(
            database
                .load_initial_library_page("https://other.example.test")
                .expect("other origin should not load this cache"),
            None
        );
        assert_eq!(
            database
                .load_onboarding_state()
                .expect("onboarding should load"),
            Some(onboarding)
        );
        let cached_origin: String = database
            .connection
            .query_row(
                "SELECT server_origin FROM rom_cache WHERE romm_id = 42",
                [],
                |row| row.get(0),
            )
            .expect("normalized cache row should load");
        assert_eq!(cached_origin, origin);
    }

    #[test]
    fn normalized_library_pages_are_origin_scoped_exact_and_include_local_state() {
        let temporary = tempfile::tempdir().expect("temporary directory should be created");
        let paths = AppPaths::from_roots(
            temporary.path().join("config"),
            temporary.path().join("data"),
            temporary.path().join("cache"),
        );
        let database = Database::open(&paths).expect("database should open");
        let first_origin = "https://first.example.test";
        let second_origin = "https://second.example.test";
        database
            .save_library_page(first_origin, &library_page(42, "First title", now_ms()))
            .expect("first page should save");
        database
            .save_library_page(second_origin, &library_page(42, "Second title", now_ms()))
            .expect("second page should save");

        let local_path = temporary.path().join("Chrono Trigger.sfc");
        fs::write(&local_path, b"rom").expect("local ROM fixture should write");
        database
            .connection
            .execute(
                r#"INSERT INTO downloaded_rom(romm_id, local_path, updated_at_ms)
                   VALUES(42, ?1, ?2)"#,
                params![local_path.to_string_lossy(), now_ms()],
            )
            .expect("download state should save");

        let first = database
            .load_library_page(first_origin, 0, 48)
            .expect("first page should load")
            .expect("first page should exist");
        let second = database
            .load_library_page(second_origin, 0, 48)
            .expect("second page should load")
            .expect("second page should exist");
        assert_eq!(first.items[0].title, "First title");
        assert_eq!(second.items[0].title, "Second title");
        assert_eq!(first.source, LibrarySource::Cache);
        assert_eq!(first.items[0].local_status, LocalGameStatus::Downloaded);
        assert_eq!(
            first.items[0].local_path,
            Some(local_path.to_string_lossy().into_owned())
        );
        assert!(
            database
                .load_library_page(first_origin, 0, 24)
                .expect("different page lookup should succeed")
                .is_none(),
            "offline fallback must never substitute a differently sized page"
        );
        let normalized_counts: (i64, i64) = database
            .connection
            .query_row(
                r#"SELECT
                       (SELECT COUNT(*) FROM library_user_rom),
                       (SELECT COUNT(*) FROM library_artwork)"#,
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("normalized rows should load");
        assert_eq!(normalized_counts, (2, 2));
    }

    #[test]
    fn library_view_pages_are_isolated_by_query_and_local_state() {
        let temporary = tempfile::tempdir().expect("temporary directory should be created");
        let paths = AppPaths::from_roots(
            temporary.path().join("config"),
            temporary.path().join("data"),
            temporary.path().join("cache"),
        );
        let database = Database::open(&paths).expect("database should open");
        let origin = "https://romm.example.test";
        let all = LibraryQuery::all();
        let favorites = LibraryQuery {
            kind: LibraryViewKind::Favorites,
            id: None,
            ..LibraryQuery::all()
        };
        database
            .save_library_view_page(origin, &all, &library_page(42, "All game", now_ms()))
            .expect("all page should save");
        database
            .save_library_view_page(
                origin,
                &favorites,
                &library_page(43, "Favorite game", now_ms()),
            )
            .expect("favorite page should save");

        assert_eq!(
            database
                .load_library_view_page(origin, &all, 0, 48)
                .expect("all page should load")
                .expect("all page should exist")
                .items[0]
                .id,
            42
        );
        assert_eq!(
            database
                .load_library_view_page(origin, &favorites, 0, 48)
                .expect("favorite page should load")
                .expect("favorite page should exist")
                .items[0]
                .id,
            43
        );

        let local_path = temporary.path().join("Favorite Game.rom");
        fs::write(&local_path, b"rom").expect("local ROM fixture should write");
        database
            .connection
            .execute(
                "INSERT INTO downloaded_rom(romm_id, local_path, updated_at_ms) VALUES(43, ?1, ?2)",
                params![local_path.to_string_lossy(), now_ms()],
            )
            .expect("downloaded state should save");
        database
            .connection
            .execute(
                "INSERT INTO download_job(id, romm_id, state, payload_json, updated_at_ms) VALUES('job-42', 42, 'running', '{}', ?1)",
                [now_ms()],
            )
            .expect("active job should save");

        let downloaded = database
            .load_local_library_page(
                origin,
                &LibraryQuery {
                    kind: LibraryViewKind::Downloaded,
                    id: None,
                    ..LibraryQuery::all()
                },
                0,
                48,
            )
            .expect("downloaded view should load");
        let active = database
            .load_local_library_page(
                origin,
                &LibraryQuery {
                    kind: LibraryViewKind::ActiveDownloads,
                    id: None,
                    ..LibraryQuery::all()
                },
                0,
                48,
            )
            .expect("active view should load");
        assert_eq!(downloaded.items[0].id, 43);
        assert_eq!(
            downloaded.items[0].local_status,
            LocalGameStatus::Downloaded
        );
        assert_eq!(active.items[0].id, 42);
        assert_eq!(
            active.items[0].active_download_id.as_deref(),
            Some("job-42")
        );
    }

    #[test]
    fn cached_library_supports_combined_offline_search_filters_and_sorting() {
        let temporary = tempfile::tempdir().expect("temporary directory should be created");
        let paths = AppPaths::from_roots(
            temporary.path().join("config"),
            temporary.path().join("data"),
            temporary.path().join("cache"),
        );
        let database = Database::open(&paths).expect("database should open");
        let origin = "https://romm.example.test";
        let mut matching = library_rom(42, "Chrono Trigger");
        matching.collection_ids = vec![3];
        matching.release_date_ms = Some(811_036_800_000);
        let mut wrong_collection = library_rom(43, "Chrono Cross");
        wrong_collection.collection_ids = vec![4];
        let mut wrong_platform = library_rom(44, "Chrono Trigger DS");
        wrong_platform.platform_id = Some(8);
        wrong_platform.collection_ids = vec![3];
        let page = RomPage {
            items: vec![wrong_platform, wrong_collection, matching],
            offset: 0,
            limit: 48,
            total: Some(3),
            has_more: false,
            source: LibrarySource::Live,
            refreshed_at_ms: now_ms(),
            stale: false,
        };
        database
            .save_library_view_page(origin, &LibraryQuery::all(), &page)
            .expect("library fixture should save");
        let local_path = temporary.path().join("Chrono Trigger.sfc");
        fs::write(&local_path, b"rom").expect("local ROM fixture should write");
        database
            .connection
            .execute(
                "INSERT INTO downloaded_rom(romm_id, local_path, updated_at_ms) VALUES(42, ?1, ?2)",
                params![local_path.to_string_lossy(), now_ms()],
            )
            .expect("downloaded state should save");
        let query = LibraryQuery {
            search: Some("chrono trigger".to_owned()),
            platform_id: Some(7),
            collection_id: Some(3),
            collection_kind: Some(CollectionKind::Standard),
            favorite_only: true,
            downloaded_only: true,
            sort: LibrarySort::Title,
            ..LibraryQuery::all()
        };

        let filtered = database
            .load_filtered_library_page(origin, &query, 0, 48)
            .expect("offline filters should load");

        assert_eq!(filtered.items.len(), 1);
        assert_eq!(filtered.items[0].id, 42);
        assert_eq!(filtered.items[0].local_status, LocalGameStatus::Downloaded);
        assert_eq!(filtered.total, Some(1));
        assert_eq!(filtered.source, LibrarySource::Cache);
    }

    #[test]
    fn complete_game_details_round_trip_by_origin_with_current_local_state() {
        let temporary = tempfile::tempdir().expect("temporary directory should be created");
        let paths = AppPaths::from_roots(
            temporary.path().join("config"),
            temporary.path().join("data"),
            temporary.path().join("cache"),
        );
        let database = Database::open(&paths).expect("database should open");
        let origin = "https://romm.example.test";
        let details = GameDetails {
            rom: library_rom(42, "Chrono Trigger"),
            genres: vec!["Role-playing".to_owned()],
            average_rating: Some("91.5".to_owned()),
            save_count: 2,
            refreshed_at_ms: now_ms(),
            source: LibrarySource::Live,
            ..GameDetails::default()
        };
        database
            .save_game_details(origin, &details)
            .expect("details should save");
        let local_path = temporary.path().join("Chrono Trigger.sfc");
        fs::write(&local_path, b"rom").expect("local ROM fixture should write");
        database
            .connection
            .execute(
                "INSERT INTO downloaded_rom(romm_id, local_path, updated_at_ms) VALUES(42, ?1, ?2)",
                params![local_path.to_string_lossy(), now_ms()],
            )
            .expect("download state should save");

        let cached = database
            .load_game_details(origin, 42)
            .expect("details should load")
            .expect("details should exist");

        assert_eq!(cached.genres, vec!["Role-playing"]);
        assert_eq!(cached.average_rating.as_deref(), Some("91.5"));
        assert_eq!(cached.rom.local_status, LocalGameStatus::Downloaded);
        assert_eq!(cached.source, LibrarySource::Cache);
        assert!(
            database
                .load_game_details("https://other.example.test", 42)
                .expect("other origin should be readable")
                .is_none()
        );
    }

    #[test]
    fn cached_library_data_becomes_stale_after_twenty_four_hours() {
        let temporary = tempfile::tempdir().expect("temporary directory should be created");
        let paths = AppPaths::from_roots(
            temporary.path().join("config"),
            temporary.path().join("data"),
            temporary.path().join("cache"),
        );
        let database = Database::open(&paths).expect("database should open");
        let refreshed_at_ms = now_ms().saturating_sub(LIBRARY_STALE_AFTER_MS + 1);
        database
            .save_library_page(
                "https://romm.example.test",
                &library_page(42, "Chrono Trigger", refreshed_at_ms),
            )
            .expect("stale page fixture should save");

        let page = database
            .load_library_page("https://romm.example.test", 0, 48)
            .expect("cached page should load")
            .expect("cached page should exist");
        assert!(page.stale);
    }

    #[test]
    fn platform_and_collection_metadata_round_trip_per_origin() {
        let temporary = tempfile::tempdir().expect("temporary directory should be created");
        let paths = AppPaths::from_roots(
            temporary.path().join("config"),
            temporary.path().join("data"),
            temporary.path().join("cache"),
        );
        let database = Database::open(&paths).expect("database should open");
        let metadata = LibraryMetadata {
            platforms: vec![romm_ipc::LibraryPlatform {
                id: 7,
                name: "SNES".to_owned(),
                slug: "snes".to_owned(),
                rom_count: Some(12),
            }],
            collections: vec![romm_ipc::LibraryCollection {
                id: 3,
                name: "Favorites".to_owned(),
                kind: CollectionKind::Standard,
                rom_ids: vec![42],
                rom_count: Some(1),
                updated_at: Some("2026-08-31T12:00:00Z".to_owned()),
            }],
            source: LibrarySource::Live,
            refreshed_at_ms: now_ms(),
            stale: false,
        };
        database
            .save_library_metadata("https://romm.example.test", &metadata)
            .expect("metadata should save");

        let cached = database
            .load_library_metadata("https://romm.example.test")
            .expect("metadata should load")
            .expect("metadata should exist");
        assert_eq!(cached.platforms, metadata.platforms);
        assert_eq!(cached.collections, metadata.collections);
        assert_eq!(cached.source, LibrarySource::Cache);
        assert!(
            database
                .load_library_metadata("https://other.example.test")
                .expect("other origin lookup should succeed")
                .is_none()
        );
    }

    #[test]
    fn failed_initial_library_cache_write_rolls_back_the_completion_marker() {
        let temporary = tempfile::tempdir().expect("temporary directory should be created");
        let paths = AppPaths::from_roots(
            temporary.path().join("config"),
            temporary.path().join("data"),
            temporary.path().join("cache"),
        );
        let database = Database::open(&paths).expect("database should open");
        let origin = "https://romm.example.test";
        let prior = OnboardingState {
            current_step: romm_ipc::OnboardingStep::FirstRefresh,
            highest_completed_step: Some(romm_ipc::OnboardingStep::Background),
            completed_steps: crate::onboarding::ONBOARDING_STEPS[..7].to_vec(),
            server_origin: Some(origin.to_owned()),
            background_enabled: Some(false),
            ..OnboardingState::default()
        };
        database
            .save_onboarding_state(&prior)
            .expect("prior onboarding should save");
        database
            .connection
            .execute_batch(
                r#"CREATE TRIGGER reject_initial_cache
                   BEFORE INSERT ON rom_cache
                   BEGIN
                       SELECT RAISE(ABORT, 'simulated cache write failure');
                   END;"#,
            )
            .expect("failure trigger should install");
        let page = RomPage {
            items: vec![romm_ipc::RomSummary {
                id: 42,
                title: "Chrono Trigger".to_owned(),
                platform: "SNES".to_owned(),
                ..romm_ipc::RomSummary::default()
            }],
            offset: 0,
            limit: 48,
            total: Some(1),
            has_more: false,
            source: LibrarySource::Live,
            refreshed_at_ms: 1234,
            stale: false,
        };
        let completed = OnboardingState {
            current_step: romm_ipc::OnboardingStep::FirstRefresh,
            highest_completed_step: Some(romm_ipc::OnboardingStep::FirstRefresh),
            completed_steps: crate::onboarding::ONBOARDING_STEPS.to_vec(),
            ..prior.clone()
        };

        assert!(
            database
                .save_initial_library_page(origin, &page, &completed)
                .is_err()
        );
        assert_eq!(
            database
                .load_onboarding_state()
                .expect("onboarding should load"),
            Some(prior)
        );
        assert_eq!(
            database
                .load_initial_library_page(origin)
                .expect("cache lookup should succeed"),
            None
        );
    }

    #[test]
    fn persists_one_unregistered_device_identity() {
        let temporary = tempfile::tempdir().expect("temporary directory should be created");
        let paths = AppPaths::from_roots(
            temporary.path().join("config"),
            temporary.path().join("data"),
            temporary.path().join("cache"),
        );
        let database = Database::open(&paths).expect("database should open");
        let mut device = DeviceIdentity {
            local_id: "4a33ec5d-1d66-41bd-9c0c-cfe8a56ab635".to_owned(),
            romm_device_id: None,
            display_name: "Windows PC - JUSTIN-DESKTOP".to_owned(),
            platform: DevicePlatform::Windows,
            hostname: "JUSTIN-DESKTOP".to_owned(),
            client: "romm-companion".to_owned(),
            client_version: "0.1.0".to_owned(),
            sync_mode: DeviceSyncMode::PushPull,
            registration_fingerprint: None,
            registration_state: DeviceRegistrationState::Unregistered,
            mapping_summary: Default::default(),
            created_at_ms: 1234,
            registered_at_ms: None,
            verified_at_ms: None,
            updated_at_ms: 1234,
        };
        database.save_device(&device).expect("device should save");
        assert_eq!(
            database.load_device().expect("device should load"),
            Some(device.clone())
        );

        device.display_name = "Living Room PC".to_owned();
        device.updated_at_ms = 5678;
        database.save_device(&device).expect("device should update");
        assert_eq!(
            database.load_device().expect("updated device should load"),
            Some(device)
        );
        let count: i64 = database
            .connection
            .query_row("SELECT COUNT(*) FROM device", [], |row| row.get(0))
            .expect("device count should load");
        assert_eq!(count, 1);
        let version: i64 = database
            .connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("schema version should load");
        assert_eq!(version, 10);
    }

    #[test]
    fn migrates_v5_mapping_rows_to_their_recorded_server_origin() {
        let mut connection = Connection::open_in_memory().expect("database should open");
        connection
            .execute_batch(INITIAL_SCHEMA)
            .expect("legacy schema should initialize");
        connection
            .execute_batch(
                r#"ALTER TABLE platform_mapping ADD COLUMN platform_name TEXT NOT NULL DEFAULT '';
                   ALTER TABLE platform_mapping ADD COLUMN platform_slug TEXT NOT NULL DEFAULT '';
                   ALTER TABLE platform_mapping ADD COLUMN source TEXT NOT NULL DEFAULT 'custom';
                   ALTER TABLE platform_mapping ADD COLUMN updated_at_ms INTEGER NOT NULL DEFAULT 0;
                   INSERT INTO app_state(key, value_json, updated_at_ms)
                   VALUES('mapping_server_origin', 'https://romm.example.test', 1234);
                   INSERT INTO platform_mapping(
                       id, romm_platform_id, rom_root, save_roots_json, state_roots_json,
                       archive_policy, filename_strategy, validation_status, custom_fields_json
                   ) VALUES(
                       'platform-1', 1, 'C:/Roms/1', '[]', '[]', 'keep',
                       'romm_filename', 'valid', '{}'
                   );"#,
            )
            .expect("v5 mapping fixture should initialize");
        connection
            .pragma_update(None, "user_version", 5)
            .expect("legacy version should save");

        run_migrations(&mut connection).expect("mapping migration should succeed");

        let server_origin: String = connection
            .query_row(
                "SELECT server_origin FROM platform_mapping WHERE id = 'platform-1'",
                [],
                |row| row.get(0),
            )
            .expect("migrated mapping origin should load");
        assert_eq!(server_origin, "https://romm.example.test");
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("schema version should load");
        assert_eq!(version, 10);
    }

    #[test]
    fn migrates_an_existing_registered_device_into_the_v4_model() {
        let mut connection = Connection::open_in_memory().expect("database should open");
        connection
            .execute_batch(INITIAL_SCHEMA)
            .expect("legacy schema should initialize");
        connection
            .execute(
                r#"INSERT INTO device(
                       singleton, romm_device_id, display_name, platform, hostname,
                       sync_mode, registration_fingerprint, registered_at_ms, verified_at_ms
                   ) VALUES(1, 'device-123', 'Living Room', 'windows', 'JUSTIN-DESKTOP',
                            'push_pull', 'legacy-fingerprint', 1234, 2345)"#,
                [],
            )
            .expect("legacy device should save");
        connection
            .pragma_update(None, "user_version", 2)
            .expect("legacy version should save");

        run_migrations(&mut connection).expect("device migration should succeed");
        let migrated = connection
            .query_row(
                r#"SELECT local_id, romm_device_id, client, client_version,
                          registration_fingerprint, registration_state, created_at_ms,
                          registered_at_ms, verified_at_ms
                   FROM device WHERE singleton = 1"#,
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, i64>(8)?,
                    ))
                },
            )
            .expect("migrated device should load");
        assert_eq!(
            migrated,
            (
                "legacy-fingerprint".to_owned(),
                "device-123".to_owned(),
                "romm-companion".to_owned(),
                "0.1.0".to_owned(),
                "legacy-fingerprint".to_owned(),
                "registered".to_owned(),
                1234,
                1234,
                2345,
            )
        );
    }
}
