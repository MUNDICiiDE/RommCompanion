use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const IPC_SCHEMA_VERSION: u16 = 17;
pub const ONBOARDING_STATE_VERSION: u16 = 1;

pub const REQUIRED_SCOPES: [&str; 10] = [
    "me.read",
    "roms.read",
    "platforms.read",
    "collections.read",
    "roms.user.read",
    "roms.user.write",
    "assets.read",
    "assets.write",
    "devices.read",
    "devices.write",
];

#[cfg(windows)]
pub fn local_ipc_endpoint() -> Result<String, String> {
    let local_data =
        std::env::var_os("LOCALAPPDATA").ok_or_else(|| "LOCALAPPDATA is unavailable".to_owned())?;
    let identity = local_data.to_string_lossy();
    let hash = identity.bytes().fold(0xcbf29ce484222325_u64, |hash, byte| {
        hash.wrapping_mul(0x100000001b3) ^ u64::from(byte.to_ascii_lowercase())
    });
    Ok(format!(r"\\.\pipe\romm-companion-{hash:016x}"))
}

#[cfg(unix)]
pub fn local_ipc_endpoint() -> Result<std::path::PathBuf, String> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .ok_or_else(|| "XDG_RUNTIME_DIR is unavailable".to_owned())?;
    Ok(runtime.join("romm-companion-agent.sock"))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RequestEnvelope {
    pub schema_version: u16,
    pub request_id: String,
    pub body: AgentRequest,
}

impl RequestEnvelope {
    pub fn new(request_id: impl Into<String>, body: AgentRequest) -> Self {
        Self {
            schema_version: IPC_SCHEMA_VERSION,
            request_id: request_id.into(),
            body,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AgentRequest {
    GetStatus,
    GetSettings,
    GetOnboarding,
    UpdateOnboarding {
        action: OnboardingNavigationAction,
    },
    UpdateSettings {
        settings: AppSettings,
    },
    Probe {
        base_url: String,
        confirm_http: bool,
        ca_id: Option<String>,
    },
    ImportCa {
        base_url: String,
        certificate: String,
    },
    ExchangePairingCode {
        code: String,
    },
    SetManualToken {
        token: String,
    },
    Reconnect,
    ProposeDevice,
    RegisterDevice {
        display_name: String,
    },
    UpdateDevice {
        display_name: String,
    },
    VerifyDevice,
    DetectMappings,
    GetMappingDrafts,
    SaveMappingDrafts {
        drafts: Vec<PlatformMappingDraft>,
    },
    ValidateMappings {
        drafts: Vec<PlatformMappingDraft>,
        no_platforms: bool,
    },
    SaveMappings {
        drafts: Vec<PlatformMappingDraft>,
        no_platforms: bool,
    },
    ConfigureOnboardingPreferences {
        background_enabled: bool,
        stable_update_checks_enabled: bool,
    },
    StartInitialRefresh,
    ListRoms {
        limit: u16,
        offset: u64,
    },
    ListLibrary {
        query: LibraryQuery,
        limit: u16,
        offset: u64,
    },
    GetGameDetails {
        rom_id: i64,
    },
    GetLibraryMetadata,
    Logout {
        remove_device: bool,
    },
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResponseEnvelope {
    pub schema_version: u16,
    pub request_id: String,
    pub body: AgentResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AgentResponse {
    Status {
        status: Box<AgentStatus>,
    },
    Settings {
        settings: Option<AppSettings>,
    },
    Onboarding {
        state: OnboardingState,
    },
    Probe {
        result: ProbeResult,
    },
    CaImported {
        result: CaImportResult,
    },
    Authenticated {
        result: AuthResult,
    },
    DeviceProposed {
        device: DeviceIdentity,
    },
    DeviceRegistered {
        device: DeviceIdentity,
        newly_registered: bool,
    },
    DeviceUpdated {
        device: DeviceIdentity,
    },
    DeviceVerification {
        device: DeviceIdentity,
        outcome: DeviceVerificationOutcome,
    },
    MappingDetection {
        result: MappingDetectionResult,
    },
    MappingDrafts {
        drafts: Vec<PlatformMappingDraft>,
    },
    MappingValidation {
        result: MappingValidationResult,
    },
    MappingsSaved {
        mappings: Vec<PlatformMappingDraft>,
        no_platforms: bool,
        onboarding: OnboardingState,
    },
    OnboardingPreferences {
        result: BackgroundSetupResult,
    },
    InitialRefresh {
        result: InitialRefreshResult,
    },
    Roms {
        page: RomPage,
    },
    LibraryPage {
        query: LibraryQuery,
        page: RomPage,
    },
    GameDetails {
        details: Box<GameDetails>,
    },
    LibraryMetadata {
        metadata: LibraryMetadata,
    },
    LoggedOut {
        device_removal: DeviceRemovalResult,
    },
    ShuttingDown,
    Error {
        error: AppError,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentStatus {
    pub version: String,
    pub ipc_schema_version: u16,
    pub connection: ConnectionState,
    pub server_url: Option<String>,
    pub server_origin: Option<String>,
    pub server_version: Option<String>,
    pub account_id: Option<i64>,
    pub account_name: Option<String>,
    pub last_contact_at_ms: Option<i64>,
    pub granted_scopes: Vec<String>,
    pub missing_scopes: Vec<String>,
    pub token_id: Option<i64>,
    pub credential_configured: bool,
    pub ca_id: Option<String>,
    pub http_approved: bool,
    pub device: Option<DeviceIdentity>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum OnboardingStep {
    #[default]
    Server,
    Authentication,
    Permissions,
    Device,
    Detection,
    Mappings,
    Background,
    FirstRefresh,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OnboardingNavigationAction {
    Back,
    Cancel,
    Resume,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct OnboardingState {
    pub version: u16,
    pub current_step: OnboardingStep,
    pub highest_completed_step: Option<OnboardingStep>,
    pub completed_steps: Vec<OnboardingStep>,
    pub server_origin: Option<String>,
    pub acknowledged_http_warning_origin: Option<String>,
    pub selected_platform_ids: Vec<i64>,
    pub mapping_draft_ids: Vec<String>,
    pub custom_path_drafts: BTreeMap<String, String>,
    pub background_enabled: Option<bool>,
    pub cancelled: bool,
}

impl Default for OnboardingState {
    fn default() -> Self {
        Self {
            version: ONBOARDING_STATE_VERSION,
            current_step: OnboardingStep::Server,
            highest_completed_step: None,
            completed_steps: Vec::new(),
            server_origin: None,
            acknowledged_http_warning_origin: None,
            selected_platform_ids: Vec::new(),
            mapping_draft_ids: Vec::new(),
            custom_path_drafts: BTreeMap::new(),
            background_enabled: None,
            cancelled: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PlatformSummary {
    pub id: i64,
    pub name: String,
    pub slug: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LibraryPlatform {
    pub id: i64,
    pub name: String,
    pub slug: String,
    pub rom_count: Option<u64>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CollectionKind {
    #[default]
    Standard,
    Smart,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LibraryCollection {
    pub id: i64,
    pub name: String,
    pub kind: CollectionKind,
    pub rom_ids: Vec<i64>,
    pub rom_count: Option<u64>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArchivePolicy {
    #[default]
    Keep,
    ExtractKeep,
    ExtractDelete,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MappingSource {
    #[default]
    Unconfigured,
    EmuDeckInternal,
    EmuDeckRemovable,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DetectionEvidence {
    pub id: String,
    pub label: String,
    pub path: String,
    pub source: MappingSource,
    pub confidence: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PlatformMappingDraft {
    pub id: String,
    pub platform_id: i64,
    pub platform_name: String,
    pub platform_slug: String,
    pub enabled: bool,
    pub rom_root: String,
    pub save_roots: Vec<String>,
    pub state_roots: Vec<String>,
    pub archive_policy: ArchivePolicy,
    pub filename_strategy: String,
    pub source: MappingSource,
    pub preset_id: Option<String>,
    pub preset_version: Option<u16>,
    pub custom_fields: BTreeMap<String, bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MappingValidationIssue {
    pub draft_id: Option<String>,
    pub field: Option<String>,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MappingValidationResult {
    pub valid: bool,
    pub issues: Vec<MappingValidationIssue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MappingDetectionResult {
    pub platforms: Vec<PlatformSummary>,
    pub drafts: Vec<PlatformMappingDraft>,
    pub evidence: Vec<DetectionEvidence>,
    pub detected_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundSetupResult {
    pub requested_background: bool,
    pub background_enabled: bool,
    pub stable_update_checks_enabled: bool,
    pub registration_method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
    pub onboarding: OnboardingState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InitialRefreshResult {
    pub page: RomPage,
    pub onboarding: OnboardingState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    Unconfigured,
    Probing,
    Pairing,
    Connected,
    Offline,
    Unauthorized,
    Incompatible,
    TlsError,
    ScopeError,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProbeResult {
    pub normalized_url: String,
    pub server_version: String,
    pub compatible: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CaImportResult {
    pub ca_id: String,
    pub fingerprint: String,
    pub server_origin: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AuthResult {
    pub server_url: String,
    pub token_kind: String,
    pub token_id: i64,
    pub account_id: i64,
    pub account_name: String,
    pub granted_scopes: Vec<String>,
    pub credential_persisted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeviceIdentity {
    pub local_id: String,
    pub romm_device_id: Option<String>,
    pub display_name: String,
    pub platform: DevicePlatform,
    pub hostname: String,
    pub client: String,
    pub client_version: String,
    pub sync_mode: DeviceSyncMode,
    pub registration_fingerprint: Option<String>,
    pub registration_state: DeviceRegistrationState,
    pub mapping_summary: BTreeMap<String, serde_json::Value>,
    pub created_at_ms: i64,
    pub registered_at_ms: Option<i64>,
    pub verified_at_ms: Option<i64>,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeviceRegistrationState {
    #[default]
    Unregistered,
    Registered,
    Missing,
    PermissionError,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeviceVerificationOutcome {
    Verified,
    Missing,
    PermissionDenied,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeviceRemovalResult {
    pub outcome: DeviceRemovalOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<AppError>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeviceRemovalOutcome {
    NotRequested,
    Removed,
    AlreadyMissing,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum DevicePlatform {
    #[serde(rename = "windows")]
    Windows,
    #[serde(rename = "steamos")]
    SteamOs,
    #[serde(rename = "linux")]
    Linux,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeviceSyncMode {
    PushPull,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CloseBehavior {
    #[default]
    MinimizeToTray,
    Quit,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ControllerButton {
    South,
    East,
    West,
    North,
    LeftShoulder,
    RightShoulder,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct ControllerBindings {
    pub confirm: Option<ControllerButton>,
    pub back: Option<ControllerButton>,
    pub context: Option<ControllerButton>,
    pub search: Option<ControllerButton>,
    pub previous_tab: Option<ControllerButton>,
    pub next_tab: Option<ControllerButton>,
}

impl Default for ControllerBindings {
    fn default() -> Self {
        Self {
            confirm: Some(ControllerButton::South),
            back: Some(ControllerButton::East),
            context: Some(ControllerButton::West),
            search: Some(ControllerButton::North),
            previous_tab: Some(ControllerButton::LeftShoulder),
            next_tab: Some(ControllerButton::RightShoulder),
        }
    }
}

impl ControllerBindings {
    pub fn validate(&self) -> Result<(), String> {
        if self.confirm.is_none() && self.back.is_none() {
            return Err("Confirm and Back cannot both be unbound.".to_owned());
        }
        let mut assigned = std::collections::BTreeSet::new();
        for button in [
            self.confirm,
            self.back,
            self.context,
            self.search,
            self.previous_tab,
            self.next_tab,
        ]
        .into_iter()
        .flatten()
        {
            if !assigned.insert(button) {
                return Err("Each physical controller button can be assigned only once.".to_owned());
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct ControllerSettings {
    pub dead_zone_percent: u8,
    pub initial_repeat_delay_ms: u16,
    pub repeat_interval_ms: u16,
    pub global_bindings: ControllerBindings,
    pub controller_bindings: BTreeMap<String, ControllerBindings>,
}

impl Default for ControllerSettings {
    fn default() -> Self {
        Self {
            dead_zone_percent: 25,
            initial_repeat_delay_ms: 350,
            repeat_interval_ms: 100,
            global_bindings: ControllerBindings::default(),
            controller_bindings: BTreeMap::new(),
        }
    }
}

impl ControllerSettings {
    pub fn validate(&self) -> Result<(), String> {
        if !(10..=50).contains(&self.dead_zone_percent) {
            return Err("Controller dead zone must be between 10% and 50%.".to_owned());
        }
        if !(200..=1_000).contains(&self.initial_repeat_delay_ms) {
            return Err("Initial repeat delay must be between 200 and 1000 ms.".to_owned());
        }
        if !(50..=300).contains(&self.repeat_interval_ms) {
            return Err("Repeat interval must be between 50 and 300 ms.".to_owned());
        }
        self.global_bindings.validate()?;
        for (guid, bindings) in &self.controller_bindings {
            if guid.len() != 32 || !guid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(
                    "Per-controller settings require a 32-character controller GUID.".to_owned(),
                );
            }
            bindings.validate()?;
        }
        Ok(())
    }

    pub fn bindings_for(&self, guid: &str) -> &ControllerBindings {
        self.controller_bindings
            .get(guid)
            .unwrap_or(&self.global_bindings)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct AppSettings {
    pub close_behavior: CloseBehavior,
    pub fullscreen: bool,
    pub stable_update_checks_enabled: bool,
    pub controller: ControllerSettings,
}

impl AppSettings {
    pub fn validate(&self) -> Result<(), String> {
        self.controller.validate()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RomSummary {
    pub id: i64,
    pub title: String,
    pub platform: String,
    #[serde(default)]
    pub platform_id: Option<i64>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub release_date_ms: Option<i64>,
    #[serde(default)]
    pub artwork: Vec<ArtworkReference>,
    #[serde(default)]
    pub collection_ids: Vec<i64>,
    #[serde(default)]
    pub user: UserRomState,
    #[serde(default)]
    pub remote_filename: Option<String>,
    #[serde(default)]
    pub remote_size_bytes: Option<u64>,
    #[serde(default)]
    pub local_status: LocalGameStatus,
    #[serde(default)]
    pub local_path: Option<String>,
    #[serde(default)]
    pub active_download_id: Option<String>,
    #[serde(default)]
    pub metadata_updated_at: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtworkKind {
    CoverSmall,
    CoverLarge,
    RemoteCover,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ArtworkReference {
    pub kind: ArtworkKind,
    pub remote_path: Option<String>,
    pub remote_url: Option<String>,
    pub cache_key: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct UserRomState {
    pub rom_id: i64,
    pub favorite: bool,
    pub backlogged: bool,
    pub hidden: bool,
    pub rating: i64,
    pub difficulty: i64,
    pub completion: i64,
    pub status: Option<String>,
    pub last_played: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LocalGameStatus {
    #[default]
    RemoteOnly,
    Downloaded,
    MissingLocal,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LibrarySource {
    Live,
    #[default]
    Cache,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LibraryViewKind {
    All,
    Recent,
    Favorites,
    Platform,
    Collection,
    SmartCollection,
    Downloaded,
    ActiveDownloads,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LibrarySort {
    #[default]
    Id,
    Title,
    RecentlyAdded,
    ReleaseDate,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LibraryQuery {
    pub kind: LibraryViewKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collection_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collection_kind: Option<CollectionKind>,
    #[serde(default)]
    pub favorite_only: bool,
    #[serde(default)]
    pub downloaded_only: bool,
    #[serde(default)]
    pub sort: LibrarySort,
}

impl Default for LibraryQuery {
    fn default() -> Self {
        Self::all()
    }
}

impl LibraryQuery {
    pub fn all() -> Self {
        Self {
            kind: LibraryViewKind::All,
            id: None,
            search: None,
            platform_id: None,
            collection_id: None,
            collection_kind: None,
            favorite_only: false,
            downloaded_only: false,
            sort: LibrarySort::Id,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        let requires_id = matches!(
            self.kind,
            LibraryViewKind::Platform
                | LibraryViewKind::Collection
                | LibraryViewKind::SmartCollection
        );
        if requires_id && self.id.is_none_or(|id| id < 1) {
            return Err("Platform and collection library views require a positive id.".to_owned());
        }
        if !requires_id && self.id.is_some() {
            return Err("This library view does not accept an id.".to_owned());
        }
        if self
            .search
            .as_ref()
            .is_some_and(|search| search.trim().len() > 200)
        {
            return Err("Library search is limited to 200 characters.".to_owned());
        }
        if self.platform_id.is_some_and(|id| id < 1) || self.collection_id.is_some_and(|id| id < 1)
        {
            return Err("Library filter ids must be positive.".to_owned());
        }
        if self.collection_id.is_some() != self.collection_kind.is_some() {
            return Err("Collection id and kind must be provided together.".to_owned());
        }
        Ok(())
    }

    pub fn cache_key(&self) -> String {
        let kind = match self.kind {
            LibraryViewKind::All => "all",
            LibraryViewKind::Recent => "recent",
            LibraryViewKind::Favorites => "favorites",
            LibraryViewKind::Platform => "platform",
            LibraryViewKind::Collection => "collection",
            LibraryViewKind::SmartCollection => "smart_collection",
            LibraryViewKind::Downloaded => "downloaded",
            LibraryViewKind::ActiveDownloads => "active_downloads",
        };
        let scope = match self.id {
            Some(id) => format!("{kind}:{id}"),
            None => kind.to_owned(),
        };
        format!(
            "{scope}|q={}|p={}|c={}:{}|f={}|d={}|s={:?}",
            self.search.as_deref().unwrap_or("").trim().to_lowercase(),
            self.platform_id
                .map_or_else(String::new, |id| id.to_string()),
            self.collection_id
                .map_or_else(String::new, |id| id.to_string()),
            self.collection_kind.map_or("", |kind| match kind {
                CollectionKind::Standard => "standard",
                CollectionKind::Smart => "smart",
            }),
            self.favorite_only,
            self.downloaded_only,
            self.sort,
        )
        .to_lowercase()
    }

    pub fn is_local(&self) -> bool {
        self.downloaded_only
            || matches!(
                self.kind,
                LibraryViewKind::Downloaded | LibraryViewKind::ActiveDownloads
            )
    }

    pub fn effective_platform_id(&self) -> Option<i64> {
        self.platform_id.or_else(|| {
            (self.kind == LibraryViewKind::Platform)
                .then_some(self.id)
                .flatten()
        })
    }

    pub fn effective_collection(&self) -> Option<(i64, CollectionKind)> {
        if let (Some(id), Some(kind)) = (self.collection_id, self.collection_kind) {
            return Some((id, kind));
        }
        match self.kind {
            LibraryViewKind::Collection => self.id.map(|id| (id, CollectionKind::Standard)),
            LibraryViewKind::SmartCollection => self.id.map(|id| (id, CollectionKind::Smart)),
            _ => None,
        }
    }

    pub fn normalized_search(&self) -> Option<String> {
        self.search
            .as_deref()
            .map(str::trim)
            .filter(|search| !search.is_empty())
            .map(ToOwned::to_owned)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct GameDetails {
    pub rom: RomSummary,
    pub alternative_names: Vec<String>,
    pub genres: Vec<String>,
    pub franchises: Vec<String>,
    pub companies: Vec<String>,
    pub game_modes: Vec<String>,
    pub age_ratings: Vec<String>,
    pub player_count: Option<String>,
    pub average_rating: Option<String>,
    pub regions: Vec<String>,
    pub languages: Vec<String>,
    pub tags: Vec<String>,
    pub files: Vec<GameFile>,
    pub collections: Vec<GameCollection>,
    pub siblings: Vec<GameSibling>,
    pub screenshot_paths: Vec<String>,
    pub save_count: u64,
    pub state_count: u64,
    pub has_manual: bool,
    pub has_soundtrack: bool,
    pub source: LibrarySource,
    pub refreshed_at_ms: i64,
    pub stale: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct GameFile {
    pub id: i64,
    pub name: String,
    pub size_bytes: u64,
    pub category: Option<String>,
    pub crc_hash: Option<String>,
    pub sha1_hash: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct GameCollection {
    pub id: i64,
    pub name: String,
    pub kind: CollectionKind,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct GameSibling {
    pub id: i64,
    pub name: String,
    pub is_main: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RomPage {
    pub items: Vec<RomSummary>,
    pub offset: u64,
    pub limit: u16,
    pub total: Option<u64>,
    pub has_more: bool,
    #[serde(default)]
    pub source: LibrarySource,
    #[serde(default)]
    pub refreshed_at_ms: i64,
    #[serde(default)]
    pub stale: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LibraryMetadata {
    pub platforms: Vec<LibraryPlatform>,
    pub collections: Vec<LibraryCollection>,
    pub source: LibrarySource,
    pub refreshed_at_ms: i64,
    pub stale: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Box<serde_json::Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause_code: Option<String>,
}

impl AppError {
    pub fn new(code: impl Into<String>, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable,
            field: None,
            details: None,
            cause_code: None,
        }
    }

    pub fn field(mut self, field: impl Into<String>) -> Self {
        self.field = Some(field.into());
        self
    }

    pub fn cause(mut self, cause_code: impl Into<String>) -> Self {
        self.cause_code = Some(cause_code.into());
        self
    }

    pub fn details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(Box::new(details));
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trips_with_a_stable_tag() {
        let envelope = RequestEnvelope::new(
            "request-1",
            AgentRequest::ListRoms {
                limit: 24,
                offset: 48,
            },
        );
        let json = serde_json::to_string(&envelope).expect("request should serialize");
        assert!(json.contains("\"type\":\"listRoms\""));
        assert!(json.contains("\"offset\":48"));
        assert_eq!(
            serde_json::from_str::<RequestEnvelope>(&json).expect("request should deserialize"),
            envelope
        );
    }

    #[test]
    fn library_queries_are_typed_validated_and_cacheable() {
        let query = LibraryQuery {
            kind: LibraryViewKind::SmartCollection,
            id: Some(9),
            ..LibraryQuery::all()
        };
        let request = AgentRequest::ListLibrary {
            query: query.clone(),
            limit: 48,
            offset: 96,
        };
        let json = serde_json::to_string(&request).expect("request should serialize");
        assert_eq!(
            json,
            r#"{"type":"listLibrary","query":{"kind":"smart_collection","id":9,"favoriteOnly":false,"downloadedOnly":false,"sort":"id"},"limit":48,"offset":96}"#
        );
        assert_eq!(
            query.cache_key(),
            "smart_collection:9|q=|p=|c=:|f=false|d=false|s=id"
        );
        assert!(query.validate().is_ok());
        assert!(
            LibraryQuery {
                kind: LibraryViewKind::Platform,
                id: None,
                ..LibraryQuery::all()
            }
            .validate()
            .is_err()
        );
        assert!(
            LibraryQuery {
                kind: LibraryViewKind::Downloaded,
                id: Some(1),
                ..LibraryQuery::all()
            }
            .validate()
            .is_err()
        );
        assert!(
            LibraryQuery {
                search: Some("x".repeat(201)),
                ..LibraryQuery::all()
            }
            .validate()
            .is_err()
        );
        assert!(
            LibraryQuery {
                collection_id: Some(4),
                collection_kind: None,
                ..LibraryQuery::all()
            }
            .validate()
            .is_err()
        );
        let filtered = LibraryQuery {
            search: Some(" Chrono ".to_owned()),
            platform_id: Some(7),
            collection_id: Some(4),
            collection_kind: Some(CollectionKind::Smart),
            favorite_only: true,
            downloaded_only: true,
            sort: LibrarySort::ReleaseDate,
            ..LibraryQuery::all()
        };
        assert!(filtered.validate().is_ok());
        assert!(filtered.is_local());
        assert_eq!(filtered.normalized_search().as_deref(), Some("Chrono"));
        assert_eq!(filtered.effective_platform_id(), Some(7));
        assert_eq!(
            filtered.effective_collection(),
            Some((4, CollectionKind::Smart))
        );
        assert!(filtered.cache_key().contains("q=chrono"));
    }

    #[test]
    fn game_detail_request_and_response_use_the_versioned_camel_case_contract() {
        let request = AgentRequest::GetGameDetails { rom_id: 42 };
        assert_eq!(
            serde_json::to_string(&request).expect("detail request should serialize"),
            r#"{"type":"getGameDetails","romId":42}"#
        );
        let response = AgentResponse::GameDetails {
            details: Box::new(GameDetails {
                rom: RomSummary {
                    id: 42,
                    title: "Chrono Trigger".to_owned(),
                    ..RomSummary::default()
                },
                average_rating: Some("91.5".to_owned()),
                ..GameDetails::default()
            }),
        };
        let json = serde_json::to_string(&response).expect("detail response should serialize");
        assert!(json.contains("\"type\":\"gameDetails\""));
        assert!(json.contains("\"averageRating\":\"91.5\""));
        assert_eq!(
            serde_json::from_str::<AgentResponse>(&json)
                .expect("detail response should deserialize"),
            response
        );
    }

    #[test]
    fn request_variant_fields_use_camel_case() {
        let request = AgentRequest::Probe {
            base_url: "https://romm.example.test".to_owned(),
            confirm_http: false,
            ca_id: None,
        };
        let json = serde_json::to_string(&request).expect("request should serialize");

        assert!(json.contains("\"baseUrl\""));
        assert!(!json.contains("base_url"));
        assert_eq!(
            serde_json::from_str::<AgentRequest>(
                r#"{"type":"probe","baseUrl":"https://romm.example.test","confirmHttp":false,"caId":null}"#,
            )
            .expect("frontend request should deserialize"),
            request
        );
    }

    #[test]
    fn onboarding_contract_is_versioned_and_has_no_completion_request() {
        let request = AgentRequest::UpdateOnboarding {
            action: OnboardingNavigationAction::Back,
        };
        assert_eq!(
            serde_json::to_string(&request).expect("onboarding request should serialize"),
            r#"{"type":"updateOnboarding","action":"back"}"#
        );
        let state = OnboardingState::default();
        let json = serde_json::to_string(&AgentResponse::Onboarding {
            state: state.clone(),
        })
        .expect("onboarding response should serialize");
        assert!(json.contains(r#""type":"onboarding""#));
        assert!(json.contains(r#""version":1"#));
        assert!(json.contains(r#""currentStep":"server""#));
        assert_eq!(
            serde_json::from_str::<AgentResponse>(&json)
                .expect("onboarding response should deserialize"),
            AgentResponse::Onboarding { state }
        );
    }

    #[test]
    fn device_contract_round_trips_with_frontend_field_names() {
        let request = AgentRequest::ProposeDevice;
        assert_eq!(
            serde_json::to_string(&request).expect("device request should serialize"),
            r#"{"type":"proposeDevice"}"#
        );

        let device = DeviceIdentity {
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
            mapping_summary: BTreeMap::new(),
            created_at_ms: 1234,
            registered_at_ms: None,
            verified_at_ms: None,
            updated_at_ms: 1234,
        };
        let json = serde_json::to_string(&AgentResponse::DeviceProposed {
            device: device.clone(),
        })
        .expect("device response should serialize");
        assert!(json.contains(r#""type":"deviceProposed""#));
        assert!(json.contains(r#""localId":"4a33ec5d-1d66-41bd-9c0c-cfe8a56ab635""#));
        assert!(json.contains(r#""rommDeviceId":null"#));
        assert!(json.contains(r#""platform":"windows""#));
        assert!(json.contains(r#""syncMode":"push_pull""#));
        assert_eq!(
            serde_json::from_str::<AgentResponse>(&json)
                .expect("frontend-shaped device response should deserialize"),
            AgentResponse::DeviceProposed { device }
        );
    }

    #[test]
    fn device_registration_fields_use_the_frontend_contract() {
        let request = AgentRequest::RegisterDevice {
            display_name: "Living Room PC".to_owned(),
        };
        let json = serde_json::to_string(&request).expect("register request should serialize");
        assert_eq!(
            json,
            r#"{"type":"registerDevice","displayName":"Living Room PC"}"#
        );
        assert_eq!(
            serde_json::from_str::<AgentRequest>(&json)
                .expect("frontend-shaped register request should deserialize"),
            request
        );
    }

    #[test]
    fn device_verification_uses_stable_frontend_names() {
        assert_eq!(
            serde_json::to_string(&AgentRequest::VerifyDevice)
                .expect("verification request should serialize"),
            r#"{"type":"verifyDevice"}"#
        );
        assert_eq!(
            serde_json::to_string(&DeviceVerificationOutcome::PermissionDenied)
                .expect("verification outcome should serialize"),
            r#""permission_denied""#
        );
        assert_eq!(
            serde_json::to_string(&DeviceRegistrationState::Missing)
                .expect("registration state should serialize"),
            r#""missing""#
        );
    }

    #[test]
    fn device_update_and_logout_outcomes_use_stable_frontend_names() {
        assert_eq!(
            serde_json::to_string(&AgentRequest::UpdateDevice {
                display_name: "Arcade Room".to_owned(),
            })
            .expect("update request should serialize"),
            r#"{"type":"updateDevice","displayName":"Arcade Room"}"#
        );
        assert_eq!(
            serde_json::to_string(&AgentResponse::LoggedOut {
                device_removal: DeviceRemovalResult {
                    outcome: DeviceRemovalOutcome::AlreadyMissing,
                    device_id: Some("device-123".to_owned()),
                    error: None,
                },
            })
            .expect("logout response should serialize"),
            r#"{"type":"loggedOut","deviceRemoval":{"outcome":"already_missing","deviceId":"device-123"}}"#
        );
    }

    #[test]
    fn mapping_requests_use_the_frontend_contract() {
        let draft = PlatformMappingDraft {
            id: "platform-7".to_owned(),
            platform_id: 7,
            platform_name: "Game Boy Advance".to_owned(),
            platform_slug: "gba".to_owned(),
            enabled: true,
            rom_root: r"C:\Emulation\roms\gba".to_owned(),
            save_roots: Vec::new(),
            state_roots: Vec::new(),
            archive_policy: ArchivePolicy::ExtractKeep,
            filename_strategy: "server_filename".to_owned(),
            source: MappingSource::EmuDeckInternal,
            preset_id: Some("emudeck".to_owned()),
            preset_version: Some(1),
            custom_fields: BTreeMap::new(),
        };
        let request = AgentRequest::SaveMappings {
            drafts: vec![draft.clone()],
            no_platforms: false,
        };
        let json = serde_json::to_string(&request).expect("mapping request should serialize");
        assert!(json.contains(r#""type":"saveMappings""#));
        assert!(json.contains(r#""platformId":7"#));
        assert!(json.contains(r#""archivePolicy":"extract_keep""#));
        assert!(json.contains(r#""source":"emu_deck_internal""#));
        assert_eq!(
            serde_json::from_str::<AgentRequest>(&json)
                .expect("frontend-shaped mapping request should deserialize"),
            request
        );
    }

    #[test]
    fn onboarding_preferences_use_the_frontend_contract() {
        let request = AgentRequest::ConfigureOnboardingPreferences {
            background_enabled: true,
            stable_update_checks_enabled: false,
        };
        let json = serde_json::to_string(&request).expect("preference request should serialize");
        assert_eq!(
            json,
            r#"{"type":"configureOnboardingPreferences","backgroundEnabled":true,"stableUpdateChecksEnabled":false}"#
        );
        assert_eq!(
            serde_json::from_str::<AgentRequest>(&json)
                .expect("frontend preference request should deserialize"),
            request
        );
    }

    #[test]
    fn initial_refresh_uses_an_agent_owned_completion_response() {
        assert_eq!(
            serde_json::to_string(&AgentRequest::StartInitialRefresh)
                .expect("initial refresh request should serialize"),
            r#"{"type":"startInitialRefresh"}"#
        );
        let result = InitialRefreshResult {
            page: RomPage {
                items: vec![RomSummary {
                    id: 42,
                    title: "Chrono Trigger".to_owned(),
                    platform: "SNES".to_owned(),
                    ..RomSummary::default()
                }],
                offset: 0,
                limit: 48,
                total: Some(107),
                has_more: true,
                source: LibrarySource::Live,
                refreshed_at_ms: 1234,
                stale: false,
            },
            onboarding: OnboardingState::default(),
        };
        let response = AgentResponse::InitialRefresh {
            result: result.clone(),
        };
        let json = serde_json::to_string(&response).expect("initial refresh should serialize");
        assert!(json.contains(r#""type":"initialRefresh""#));
        assert!(json.contains(r#""hasMore":true"#));
        assert_eq!(
            serde_json::from_str::<AgentResponse>(&json)
                .expect("frontend initial refresh response should deserialize"),
            AgentResponse::InitialRefresh { result }
        );
    }

    #[test]
    fn library_metadata_and_cache_provenance_use_frontend_names() {
        assert_eq!(
            serde_json::to_string(&AgentRequest::GetLibraryMetadata)
                .expect("library metadata request should serialize"),
            r#"{"type":"getLibraryMetadata"}"#
        );
        let metadata = LibraryMetadata {
            platforms: vec![LibraryPlatform {
                id: 7,
                name: "SNES".to_owned(),
                slug: "snes".to_owned(),
                rom_count: Some(12),
            }],
            collections: vec![LibraryCollection {
                id: 3,
                name: "Favorites".to_owned(),
                kind: CollectionKind::Standard,
                rom_ids: vec![42],
                rom_count: Some(1),
                updated_at: None,
            }],
            source: LibrarySource::Cache,
            refreshed_at_ms: 1234,
            stale: true,
        };
        let json = serde_json::to_string(&AgentResponse::LibraryMetadata {
            metadata: metadata.clone(),
        })
        .expect("library metadata should serialize");
        assert!(json.contains(r#""type":"libraryMetadata""#));
        assert!(json.contains(r#""source":"cache""#));
        assert!(json.contains(r#""romCount":12"#));
        assert_eq!(
            serde_json::from_str::<AgentResponse>(&json)
                .expect("frontend library metadata should deserialize"),
            AgentResponse::LibraryMetadata { metadata }
        );
    }

    #[test]
    fn controller_settings_default_and_validate_with_backward_compatible_json() {
        let old: AppSettings =
            serde_json::from_str(r#"{"closeBehavior":"quit","fullscreen":true}"#)
                .expect("older settings should gain controller defaults");
        assert_eq!(old.controller.dead_zone_percent, 25);
        assert_eq!(
            old.controller.global_bindings.previous_tab,
            Some(ControllerButton::LeftShoulder)
        );
        assert!(old.validate().is_ok());

        let mut invalid = old;
        invalid.controller.global_bindings.confirm = None;
        invalid.controller.global_bindings.back = None;
        assert_eq!(
            invalid
                .validate()
                .expect_err("both reserved actions cannot be empty"),
            "Confirm and Back cannot both be unbound."
        );
    }

    #[test]
    fn controller_settings_reject_duplicate_buttons_and_invalid_ranges() {
        let mut settings = AppSettings::default();
        settings.controller.dead_zone_percent = 5;
        assert!(settings.validate().is_err());
        settings.controller.dead_zone_percent = 25;
        settings.controller.global_bindings.back = Some(ControllerButton::South);
        assert_eq!(
            settings
                .validate()
                .expect_err("duplicate buttons should fail"),
            "Each physical controller button can be assigned only once."
        );
    }
}
