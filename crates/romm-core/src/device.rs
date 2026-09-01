use std::{
    collections::BTreeMap,
    env,
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(target_os = "linux")]
use std::fs;

use romm_ipc::{AppError, DeviceIdentity, DevicePlatform, DeviceRegistrationState, DeviceSyncMode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const DEVICE_CLIENT: &str = "romm-companion";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DeviceCreatePayload {
    pub name: String,
    pub platform: String,
    pub client: String,
    pub client_version: String,
    pub hostname: String,
    pub sync_mode: DeviceSyncMode,
    pub sync_config: BTreeMap<String, serde_json::Value>,
    pub allow_existing: bool,
    pub allow_duplicate: bool,
    pub reset_syncs: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DeviceUpdatePayload {
    pub name: String,
    pub platform: String,
    pub client: String,
    pub client_version: String,
    pub hostname: String,
    pub sync_mode: DeviceSyncMode,
    pub sync_config: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct DeviceCreateResponse {
    pub device_id: String,
    pub name: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRegistrationResult {
    pub device_id: String,
    pub display_name: Option<String>,
    pub registration_fingerprint: String,
    pub newly_registered: bool,
    pub registered_at_ms: i64,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct RemoteDevice {
    pub id: String,
    pub user_id: i64,
    pub name: Option<String>,
    pub platform: Option<String>,
    pub client: Option<String>,
    pub client_version: Option<String>,
    pub hostname: Option<String>,
    pub sync_mode: DeviceSyncMode,
    pub sync_enabled: bool,
    pub sync_config: Option<BTreeMap<String, serde_json::Value>>,
    pub last_seen: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

pub fn create_payload(device: &DeviceIdentity) -> DeviceCreatePayload {
    DeviceCreatePayload {
        name: device.display_name.clone(),
        platform: platform_api_value(device.platform).to_owned(),
        client: device.client.clone(),
        client_version: device.client_version.clone(),
        hostname: device.hostname.clone(),
        sync_mode: device.sync_mode,
        sync_config: device.mapping_summary.clone(),
        allow_existing: true,
        allow_duplicate: false,
        reset_syncs: false,
    }
}

pub fn update_payload(device: &DeviceIdentity) -> DeviceUpdatePayload {
    DeviceUpdatePayload {
        name: device.display_name.clone(),
        platform: platform_api_value(device.platform).to_owned(),
        client: device.client.clone(),
        client_version: device.client_version.clone(),
        hostname: device.hostname.clone(),
        sync_mode: device.sync_mode,
        sync_config: device.mapping_summary.clone(),
    }
}

pub fn with_display_name(
    device: &DeviceIdentity,
    display_name: &str,
) -> Result<DeviceIdentity, AppError> {
    let display_name = validate_display_name(display_name)?;
    let mut updated = device.clone();
    updated.display_name = display_name;
    updated.updated_at_ms = now_ms();
    Ok(updated)
}

pub fn registration_fingerprint(payload: &DeviceCreatePayload) -> Result<String, AppError> {
    let bytes = serde_json::to_vec(payload).map_err(|error| {
        AppError::new(
            "device_fingerprint_failed",
            format!("The device registration fingerprint could not be created: {error}"),
            false,
        )
    })?;
    let digest = Sha256::digest(bytes);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub fn apply_registration(
    device: &DeviceIdentity,
    registration: &DeviceRegistrationResult,
) -> DeviceIdentity {
    let mut registered = device.clone();
    registered.romm_device_id = Some(registration.device_id.clone());
    if let Some(name) = registration
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        registered.display_name = name.to_owned();
    }
    registered.registration_fingerprint = Some(registration.registration_fingerprint.clone());
    registered.registration_state = DeviceRegistrationState::Registered;
    registered.registered_at_ms = Some(registration.registered_at_ms);
    registered.verified_at_ms = Some(registration.registered_at_ms);
    registered.updated_at_ms = registration.registered_at_ms;
    registered
}

pub fn apply_verification(device: &DeviceIdentity, remote: &RemoteDevice) -> DeviceIdentity {
    let mut verified = device.clone();
    if let Some(name) = remote
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        verified.display_name = name.to_owned();
    }
    let verified_at_ms = now_ms();
    verified.registration_state = DeviceRegistrationState::Registered;
    verified.verified_at_ms = Some(verified_at_ms);
    verified.updated_at_ms = verified_at_ms;
    verified
}

pub fn clear_remote_registration(device: &DeviceIdentity) -> DeviceIdentity {
    let mut local = device.clone();
    local.romm_device_id = None;
    local.registration_fingerprint = None;
    local.registration_state = DeviceRegistrationState::Unregistered;
    local.registered_at_ms = None;
    local.verified_at_ms = None;
    local.updated_at_ms = now_ms();
    local
}

pub fn mark_device_missing(device: &DeviceIdentity) -> DeviceIdentity {
    let mut missing = device.clone();
    missing.registration_state = DeviceRegistrationState::Missing;
    missing.updated_at_ms = now_ms();
    missing
}

pub fn mark_device_permission_error(device: &DeviceIdentity) -> DeviceIdentity {
    let mut blocked = device.clone();
    blocked.registration_state = DeviceRegistrationState::PermissionError;
    blocked.updated_at_ms = now_ms();
    blocked
}

pub fn propose_device_identity(client_version: &str) -> DeviceIdentity {
    let platform = detect_device_platform();
    let hostname = detect_hostname();
    let now = now_ms();
    build_device_identity(
        platform,
        &hostname,
        Uuid::new_v4().to_string(),
        client_version,
        now,
    )
}

pub fn detect_device_platform() -> DevicePlatform {
    #[cfg(windows)]
    {
        DevicePlatform::Windows
    }

    #[cfg(target_os = "linux")]
    {
        if linux_is_steam_deck() {
            DevicePlatform::SteamOs
        } else {
            DevicePlatform::Linux
        }
    }

    #[cfg(not(any(windows, target_os = "linux")))]
    {
        DevicePlatform::Linux
    }
}

pub fn detect_hostname() -> String {
    let environment_hostname = [env::var_os("COMPUTERNAME"), env::var_os("HOSTNAME")]
        .into_iter()
        .flatten()
        .map(|value| value.to_string_lossy().into_owned())
        .find(|value| !value.trim().is_empty());

    #[cfg(target_os = "linux")]
    let hostname = environment_hostname.or_else(|| fs::read_to_string("/etc/hostname").ok());
    #[cfg(not(target_os = "linux"))]
    let hostname = environment_hostname;

    normalize_hostname(hostname.as_deref().unwrap_or("unknown-device"))
}

fn build_device_identity(
    platform: DevicePlatform,
    hostname: &str,
    local_id: String,
    client_version: &str,
    now: i64,
) -> DeviceIdentity {
    let hostname = normalize_hostname(hostname);
    DeviceIdentity {
        local_id,
        romm_device_id: None,
        display_name: default_device_name(platform, &hostname),
        platform,
        hostname,
        client: DEVICE_CLIENT.to_owned(),
        client_version: client_version.to_owned(),
        sync_mode: DeviceSyncMode::PushPull,
        registration_fingerprint: None,
        registration_state: DeviceRegistrationState::Unregistered,
        mapping_summary: Default::default(),
        created_at_ms: now,
        registered_at_ms: None,
        verified_at_ms: None,
        updated_at_ms: now,
    }
}

fn default_device_name(platform: DevicePlatform, hostname: &str) -> String {
    let label = match platform {
        DevicePlatform::Windows => "Windows PC",
        DevicePlatform::SteamOs => "Steam Deck",
        DevicePlatform::Linux => "Linux PC",
    };
    if hostname == "unknown-device" {
        label.to_owned()
    } else {
        format!("{label} - {hostname}")
    }
}

fn platform_api_value(platform: DevicePlatform) -> &'static str {
    match platform {
        DevicePlatform::Windows => "windows",
        DevicePlatform::SteamOs => "steamos",
        DevicePlatform::Linux => "linux",
    }
}

fn normalize_hostname(hostname: &str) -> String {
    let first_line = hostname.lines().next().unwrap_or_default().trim();
    let normalized = first_line.chars().take(255).collect::<String>();
    if normalized.is_empty() {
        "unknown-device".to_owned()
    } else {
        normalized
    }
}

fn validate_display_name(display_name: &str) -> Result<String, AppError> {
    let display_name = display_name.trim();
    if display_name.is_empty() {
        return Err(AppError::new(
            "invalid_device_name",
            "Enter a device name before registering.",
            false,
        )
        .field("displayName"));
    }
    if display_name.chars().count() > 255 {
        return Err(AppError::new(
            "invalid_device_name",
            "Device names must contain no more than 255 characters.",
            false,
        )
        .field("displayName"));
    }
    if display_name.chars().any(char::is_control) {
        return Err(AppError::new(
            "invalid_device_name",
            "Device names cannot contain control characters or line breaks.",
            false,
        )
        .field("displayName"));
    }
    Ok(display_name.to_owned())
}

#[cfg(target_os = "linux")]
fn linux_is_steam_deck() -> bool {
    if env::var("SteamDeck").is_ok_and(|value| value == "1") {
        return true;
    }

    let dmi = [
        "/sys/class/dmi/id/sys_vendor",
        "/sys/class/dmi/id/board_vendor",
        "/sys/class/dmi/id/product_name",
        "/sys/class/dmi/id/board_name",
    ]
    .iter()
    .filter_map(|path| fs::read_to_string(path).ok())
    .collect::<Vec<_>>()
    .join(" ");
    let release = fs::read_to_string("/etc/os-release").unwrap_or_default();
    is_steam_deck_from(&dmi, &release)
}

#[cfg(any(target_os = "linux", test))]
fn is_steam_deck_from(dmi: &str, os_release: &str) -> bool {
    let dmi = dmi.to_ascii_lowercase();
    if dmi.contains("valve") || dmi.contains("jupiter") || dmi.contains("galileo") {
        return true;
    }

    os_release.lines().any(|line| {
        line.split_once('=').is_some_and(|(key, value)| {
            key.trim().eq_ignore_ascii_case("VARIANT_ID")
                && value
                    .trim()
                    .trim_matches('"')
                    .eq_ignore_ascii_case("steamdeck")
        })
    })
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

    #[test]
    fn proposes_platform_specific_names_and_push_pull_sync() {
        let device = build_device_identity(
            DevicePlatform::Windows,
            "JUSTIN-DESKTOP\nignored",
            "local-id".to_owned(),
            "0.1.0",
            1234,
        );

        assert_eq!(device.display_name, "Windows PC - JUSTIN-DESKTOP");
        assert_eq!(device.hostname, "JUSTIN-DESKTOP");
        assert_eq!(device.client, DEVICE_CLIENT);
        assert_eq!(device.sync_mode, DeviceSyncMode::PushPull);
        assert_eq!(device.local_id, "local-id");
        assert_eq!(device.created_at_ms, 1234);
        assert_eq!(device.updated_at_ms, 1234);
        assert!(device.romm_device_id.is_none());
    }

    #[test]
    fn uses_a_readable_name_when_hostname_is_unavailable() {
        let device = build_device_identity(
            DevicePlatform::SteamOs,
            "  ",
            "local-id".to_owned(),
            "0.1.0",
            1234,
        );

        assert_eq!(device.hostname, "unknown-device");
        assert_eq!(device.display_name, "Steam Deck");
    }

    #[test]
    fn recognizes_steam_deck_hardware_and_os_release_signals() {
        assert!(is_steam_deck_from("Valve Jupiter", ""));
        assert!(is_steam_deck_from("", "NAME=SteamOS\nVARIANT_ID=steamdeck"));
        assert!(!is_steam_deck_from(
            "Framework Laptop",
            "VARIANT_ID=desktop"
        ));
    }

    #[test]
    fn registration_payload_matches_romm_5_1_openapi_without_network_identifiers() {
        let device = build_device_identity(
            DevicePlatform::SteamOs,
            "steamdeck",
            "local-id".to_owned(),
            "0.1.0",
            1234,
        );
        let payload =
            serde_json::to_value(create_payload(&device)).expect("device payload should serialize");

        assert_eq!(payload["name"], "Steam Deck - steamdeck");
        assert_eq!(payload["platform"], "steamos");
        assert_eq!(payload["client"], DEVICE_CLIENT);
        assert_eq!(payload["sync_mode"], "push_pull");
        assert_eq!(payload["sync_config"], serde_json::json!({}));
        assert_eq!(payload["allow_existing"], true);
        assert_eq!(payload["allow_duplicate"], false);
        assert_eq!(payload["reset_syncs"], false);
        assert!(payload.get("ip_address").is_none());
        assert!(payload.get("mac_address").is_none());
    }

    #[test]
    fn parses_romm_device_create_and_lookup_responses() {
        let created: DeviceCreateResponse = serde_json::from_str(
            r#"{"device_id":"device-123","name":"Living Room","created_at":"2026-08-27T12:00:00Z","future_field":true}"#,
        )
        .expect("create response should accept additive fields");
        assert_eq!(created.device_id, "device-123");

        let remote: RemoteDevice = serde_json::from_str(
            r#"{
                "id":"device-123",
                "user_id":7,
                "name":"Living Room",
                "platform":"windows",
                "client":"romm-companion",
                "client_version":"0.1.0",
                "hostname":"JUSTIN-DESKTOP",
                "sync_mode":"push_pull",
                "sync_enabled":true,
                "sync_config":{},
                "last_seen":null,
                "created_at":"2026-08-27T12:00:00Z",
                "updated_at":"2026-08-27T12:00:00Z",
                "future_field":true
            }"#,
        )
        .expect("device lookup should accept additive fields");
        assert_eq!(remote.id, "device-123");
        assert_eq!(remote.sync_mode, DeviceSyncMode::PushPull);
    }

    #[test]
    fn validates_and_persists_an_edited_draft_name() {
        let device = build_device_identity(
            DevicePlatform::Windows,
            "JUSTIN-DESKTOP",
            "local-id".to_owned(),
            "0.1.0",
            1234,
        );
        let renamed = with_display_name(&device, "  Living Room PC  ")
            .expect("valid display name should be accepted");
        assert_eq!(renamed.display_name, "Living Room PC");
        assert_eq!(device.display_name, "Windows PC - JUSTIN-DESKTOP");

        assert_eq!(
            with_display_name(&device, "\n")
                .expect_err("blank name should fail")
                .field
                .as_deref(),
            Some("displayName")
        );
        assert!(with_display_name(&device, &"x".repeat(256)).is_err());
        assert!(with_display_name(&device, "Living\nRoom").is_err());
    }

    #[test]
    fn registration_fingerprint_is_stable_and_tracks_payload_changes() {
        let device = build_device_identity(
            DevicePlatform::Linux,
            "arcade-pc",
            "local-id".to_owned(),
            "0.1.0",
            1234,
        );
        let first = registration_fingerprint(&create_payload(&device))
            .expect("fingerprint should be created");
        let second = registration_fingerprint(&create_payload(&device))
            .expect("fingerprint should be stable");
        let renamed =
            with_display_name(&device, "Arcade Room").expect("renamed device should be valid");
        let changed = registration_fingerprint(&create_payload(&renamed))
            .expect("changed fingerprint should be created");

        assert_eq!(first, second);
        assert_eq!(first.len(), 64);
        assert_ne!(first, changed);
    }

    #[test]
    fn applies_the_authoritative_registration_result() {
        let device = build_device_identity(
            DevicePlatform::Windows,
            "JUSTIN-DESKTOP",
            "local-id".to_owned(),
            "0.1.0",
            1234,
        );
        let registered = apply_registration(
            &device,
            &DeviceRegistrationResult {
                device_id: "device-123".to_owned(),
                display_name: Some("Server Device Name".to_owned()),
                registration_fingerprint: "abc123".to_owned(),
                newly_registered: true,
                registered_at_ms: 5678,
            },
        );

        assert_eq!(registered.romm_device_id.as_deref(), Some("device-123"));
        assert_eq!(registered.display_name, "Server Device Name");
        assert_eq!(
            registered.registration_fingerprint.as_deref(),
            Some("abc123")
        );
        assert_eq!(registered.registered_at_ms, Some(5678));
        assert_eq!(
            registered.registration_state,
            DeviceRegistrationState::Registered
        );
        assert_eq!(registered.verified_at_ms, Some(5678));
    }

    #[test]
    fn verification_states_preserve_the_local_identity_and_mappings() {
        let mut device = build_device_identity(
            DevicePlatform::Windows,
            "JUSTIN-DESKTOP",
            "local-id".to_owned(),
            "0.1.0",
            1234,
        );
        device.romm_device_id = Some("device-123".to_owned());
        device.registration_state = DeviceRegistrationState::Registered;
        device.mapping_summary.insert(
            "snes".to_owned(),
            serde_json::json!({ "romRoot": "D:/ROMs/SNES" }),
        );
        let remote: RemoteDevice = serde_json::from_value(serde_json::json!({
            "id": "device-123",
            "user_id": 7,
            "name": "Server Name",
            "platform": "windows",
            "client": "romm-companion",
            "client_version": "0.1.0",
            "hostname": "JUSTIN-DESKTOP",
            "sync_mode": "push_pull",
            "sync_enabled": true,
            "sync_config": {},
            "last_seen": null,
            "created_at": "2026-08-27T12:00:00Z",
            "updated_at": "2026-08-28T12:00:00Z"
        }))
        .expect("remote device should parse");

        let verified = apply_verification(&device, &remote);
        assert_eq!(verified.display_name, "Server Name");
        assert_eq!(
            verified.registration_state,
            DeviceRegistrationState::Registered
        );
        assert!(verified.verified_at_ms.is_some());

        let missing = mark_device_missing(&verified);
        assert_eq!(missing.registration_state, DeviceRegistrationState::Missing);
        assert_eq!(missing.romm_device_id.as_deref(), Some("device-123"));
        assert_eq!(missing.local_id, "local-id");
        assert_eq!(missing.mapping_summary, device.mapping_summary);

        let blocked = mark_device_permission_error(&verified);
        assert_eq!(
            blocked.registration_state,
            DeviceRegistrationState::PermissionError
        );
        assert_eq!(blocked.mapping_summary, device.mapping_summary);
    }

    #[test]
    fn clearing_remote_registration_preserves_user_managed_local_state() {
        let mut device = build_device_identity(
            DevicePlatform::Windows,
            "JUSTIN-DESKTOP",
            "local-id".to_owned(),
            "0.1.0",
            1234,
        );
        device.romm_device_id = Some("device-123".to_owned());
        device.registration_fingerprint = Some("fingerprint".to_owned());
        device.registration_state = DeviceRegistrationState::Registered;
        device.registered_at_ms = Some(5678);
        device.verified_at_ms = Some(6789);
        device.mapping_summary.insert(
            "snes".to_owned(),
            serde_json::json!({ "romRoot": "D:/ROMs/SNES" }),
        );

        let local = clear_remote_registration(&device);

        assert_eq!(local.local_id, device.local_id);
        assert_eq!(local.display_name, device.display_name);
        assert_eq!(local.mapping_summary, device.mapping_summary);
        assert!(local.romm_device_id.is_none());
        assert!(local.registration_fingerprint.is_none());
        assert_eq!(
            local.registration_state,
            DeviceRegistrationState::Unregistered
        );
        assert!(local.registered_at_ms.is_none());
        assert!(local.verified_at_ms.is_none());
    }
}
