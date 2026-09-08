pub mod credentials;
pub mod device;
pub mod mapping;
pub mod onboarding;
pub mod redaction;
pub mod storage;

use std::time::{SystemTime, UNIX_EPOCH};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use device::{
    DeviceCreateResponse, DeviceRegistrationResult, RemoteDevice, create_payload,
    registration_fingerprint, update_payload,
};
use reqwest::{
    Certificate, Client, Method, StatusCode,
    header::{CONTENT_LENGTH, ETAG},
    multipart::Form,
};
use romm_ipc::{
    AppError, ArtworkKind, ArtworkReference, CollectionKind, DeviceIdentity,
    FavoriteMutationResult, GameCollection, GameDetails, GameFile, GameSibling, LibraryCollection,
    LibraryMetadata, LibraryPlatform, LibraryQuery, LibrarySort, LibrarySource, LibraryViewKind,
    LocalGameStatus, PlatformSummary, ProbeResult, REQUIRED_SCOPES, RomPage, RomSummary,
    UserRomState,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use url::Url;
use x509_parser::{parse_x509_certificate, pem::parse_x509_pem};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedAuth {
    pub token: String,
    pub token_id: i64,
    pub account_id: i64,
    pub account_name: String,
    pub granted_scopes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedArtwork {
    pub mime_type: String,
    pub bytes: Vec<u8>,
    pub etag: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoritativeFavorite {
    pub result: FavoriteMutationResult,
    pub favorite_rom_ids: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FavoriteCollection {
    id: i64,
    rom_ids: Vec<i64>,
    updated_at: Option<String>,
}

pub const MAX_ARTWORK_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct RommSession {
    client: Client,
    base_url: Option<Url>,
    server_version: Option<String>,
    token: Option<String>,
    token_id: Option<i64>,
    account_id: Option<i64>,
    account_name: Option<String>,
    granted_scopes: Vec<String>,
    last_contact_at_ms: Option<i64>,
    ca_id: Option<String>,
    http_approved: bool,
}

impl RommSession {
    pub fn new() -> Self {
        Self {
            client: build_client(None).expect("the default HTTP client must be constructible"),
            base_url: None,
            server_version: None,
            token: None,
            token_id: None,
            account_id: None,
            account_name: None,
            granted_scopes: Vec::new(),
            last_contact_at_ms: None,
            ca_id: None,
            http_approved: false,
        }
    }

    pub fn with_certificate(certificate: &[u8], ca_id: String) -> Result<Self, AppError> {
        Ok(Self {
            client: build_client(Some(certificate))?,
            ca_id: Some(ca_id),
            ..Self::new()
        })
    }

    pub fn base_url(&self) -> Option<String> {
        self.base_url.as_ref().map(ToString::to_string)
    }

    pub fn server_version(&self) -> Option<String> {
        self.server_version.clone()
    }

    pub fn server_origin(&self) -> Option<String> {
        self.base_url.as_ref().map(server_origin)
    }

    pub fn token_id(&self) -> Option<i64> {
        self.token_id
    }

    pub fn account_id(&self) -> Option<i64> {
        self.account_id
    }

    pub fn account_name(&self) -> Option<String> {
        self.account_name.clone()
    }

    pub fn granted_scopes(&self) -> Vec<String> {
        self.granted_scopes.clone()
    }

    pub fn missing_scopes(&self) -> Vec<String> {
        missing_required_scopes(&self.granted_scopes)
    }

    pub fn last_contact_at_ms(&self) -> Option<i64> {
        self.last_contact_at_ms
    }

    pub fn ca_id(&self) -> Option<String> {
        self.ca_id.clone()
    }

    pub fn http_approved(&self) -> bool {
        self.http_approved
    }

    pub fn credential(&self) -> Option<&str> {
        self.token.as_deref()
    }

    pub fn is_authenticated(&self) -> bool {
        self.token.is_some()
    }

    pub fn restore(
        &mut self,
        base_url: &str,
        server_version: Option<String>,
        token: &str,
    ) -> Result<(), AppError> {
        self.restore_configuration(base_url, server_version)?;
        if !is_client_token(token) {
            return Err(AppError::new(
                "invalid_saved_credential",
                "The saved RomM credential is invalid. Pair this device again.",
                false,
            ));
        }
        self.token = Some(token.to_owned());
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn restore_metadata(
        &mut self,
        token_id: Option<i64>,
        account_id: Option<i64>,
        account_name: Option<String>,
        granted_scopes: Vec<String>,
        last_contact_at_ms: Option<i64>,
        http_approved: bool,
        ca_id: Option<String>,
    ) {
        self.token_id = token_id;
        self.account_id = account_id;
        self.account_name = account_name;
        self.granted_scopes = granted_scopes;
        self.last_contact_at_ms = last_contact_at_ms;
        self.http_approved = http_approved;
        self.ca_id = ca_id;
    }

    pub fn restore_configuration(
        &mut self,
        base_url: &str,
        server_version: Option<String>,
    ) -> Result<(), AppError> {
        self.base_url = Some(normalize_base_url(base_url)?);
        self.server_version = server_version;
        self.token = None;
        Ok(())
    }

    pub async fn probe(
        &mut self,
        input: &str,
        confirm_http: bool,
    ) -> Result<ProbeResult, AppError> {
        let base_url = normalize_base_url(input)?;
        if base_url.scheme() == "http" && !confirm_http {
            return Err(AppError::new(
                "insecure_http_confirmation_required",
                format!(
                    "{} uses HTTP. Credentials and downloaded content will not be encrypted.",
                    server_origin(&base_url)
                ),
                false,
            )
            .details(json!({ "serverOrigin": server_origin(&base_url) })));
        }
        let openapi_url = base_url
            .join("openapi.json")
            .map_err(|error| internal_url_error(error.to_string()))?;
        let response = self
            .client
            .get(openapi_url)
            .send()
            .await
            .map_err(network_error)?;

        if !response.status().is_success() {
            return Err(http_error(
                response.status(),
                "RomM compatibility probe failed",
            ));
        }

        let document: Value = response.json().await.map_err(|error| {
            AppError::new(
                "invalid_openapi",
                format!("The server did not return valid OpenAPI JSON: {error}"),
                false,
            )
        })?;
        let version = document
            .pointer("/info/version")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_owned();
        let compatible = version.trim_start_matches('v').starts_with("5.");

        if compatible {
            self.base_url = Some(base_url.clone());
            self.server_version = Some(version.clone());
            self.token = None;
            self.token_id = None;
            self.account_id = None;
            self.account_name = None;
            self.granted_scopes.clear();
            self.last_contact_at_ms = Some(now_ms());
            self.http_approved = base_url.scheme() == "http" && confirm_http;
        }

        Ok(ProbeResult {
            normalized_url: base_url.to_string(),
            server_version: version,
            compatible,
        })
    }

    pub async fn exchange_pairing_code(&mut self, code: &str) -> Result<ValidatedAuth, AppError> {
        let code = normalize_pairing_code(code).ok_or_else(|| {
            AppError::new(
                "invalid_pairing_code",
                "Pairing codes must contain eight letters or numbers in XXXX-XXXX format.",
                false,
            )
            .field("code")
        })?;

        let url = self.endpoint("api/client-tokens/exchange")?;
        let response = self
            .client
            .post(url)
            .json(&json!({ "code": code }))
            .send()
            .await
            .map_err(network_error)?;
        let status = response.status();
        if status == StatusCode::NOT_FOUND || status == StatusCode::GONE {
            return Err(AppError::new(
                "expired_pairing_code",
                "The pairing code expired or was already used. Generate a new code in RomM.",
                false,
            )
            .field("code"));
        }
        if !status.is_success() {
            return Err(http_error(status, "RomM rejected the pairing code"));
        }
        let payload: Value = response.json().await.map_err(|error| {
            AppError::new(
                "invalid_pairing_response",
                format!("RomM returned an unreadable pairing response: {error}"),
                false,
            )
        })?;

        let record = parse_token_record(&payload).ok_or_else(|| {
            AppError::new(
                "missing_client_token",
                "RomM accepted the request but did not return complete Client API Token details.",
                false,
            )
        })?;
        let validated = self.validate_token(record).await?;
        self.commit_auth(&validated);
        Ok(validated)
    }

    pub async fn authenticate_manual_token(
        &mut self,
        token: &str,
    ) -> Result<ValidatedAuth, AppError> {
        if !is_client_token(token) {
            return Err(AppError::new(
                "invalid_client_token",
                "A Client API Token must start with rmm_ and contain 64 hexadecimal characters.",
                false,
            )
            .field("token"));
        }
        if self.base_url.is_none() {
            return Err(AppError::new(
                "server_not_configured",
                "Test the RomM server before authenticating.",
                false,
            ));
        }
        let first = self.list_token_records(token).await?;
        let second = self.list_token_records(token).await?;
        let first_id = uniquely_most_recent_token(&first);
        let second_id = uniquely_most_recent_token(&second);
        let token_id = match (first_id, second_id) {
            (Some(first), Some(second)) if first == second => first,
            _ => {
                return Err(AppError::new(
                    "manual_token_ambiguous",
                    "RomM could not uniquely identify this manual token. Use device pairing so its permissions can be verified safely.",
                    false,
                ));
            }
        };
        let record = second
            .into_iter()
            .find(|record| record.id == token_id)
            .ok_or_else(|| {
                AppError::new(
                    "manual_token_ambiguous",
                    "RomM could not identify the manual token after authentication.",
                    false,
                )
            })?;
        let validated = self
            .validate_token(TokenRecord {
                raw_token: token.to_owned(),
                ..record
            })
            .await?;
        self.commit_auth(&validated);
        Ok(validated)
    }

    pub async fn reconnect(&mut self) -> Result<ValidatedAuth, AppError> {
        let token = self.token.clone().ok_or_else(|| {
            AppError::new(
                "authentication_required",
                "No saved RomM credential is available.",
                false,
            )
        })?;
        let record = if let Some(token_id) = self.token_id {
            TokenRecord {
                id: token_id,
                raw_token: token,
                scopes: self.granted_scopes.clone(),
                last_used_at: None,
            }
        } else {
            self.token = None;
            return self.authenticate_manual_token(&token).await;
        };
        let validated = self.validate_token(record).await?;
        self.commit_auth(&validated);
        Ok(validated)
    }

    pub async fn register_device(
        &mut self,
        device: &DeviceIdentity,
    ) -> Result<DeviceRegistrationResult, AppError> {
        let token = self.token.clone().ok_or_else(|| {
            AppError::new(
                "authentication_required",
                "Connect to RomM before registering this device.",
                false,
            )
        })?;
        if !self
            .granted_scopes
            .iter()
            .any(|scope| scope == "devices.write")
        {
            return Err(missing_scope_error(vec!["devices.write".to_owned()]));
        }

        let payload = create_payload(device);
        let fingerprint = registration_fingerprint(&payload)?;
        let url = self.endpoint("api/devices")?;
        let response = self
            .client
            .post(url)
            .bearer_auth(token)
            .json(&payload)
            .send()
            .await
            .map_err(network_error)?;
        let status = response.status();
        if status == StatusCode::FORBIDDEN {
            return Err(missing_scope_error(vec!["devices.write".to_owned()]));
        }
        if !status.is_success() {
            return Err(http_error(status, "RomM rejected device registration"));
        }
        let response: DeviceCreateResponse = response.json().await.map_err(|error| {
            AppError::new(
                "invalid_device_response",
                format!("RomM returned unreadable device registration data: {error}"),
                false,
            )
        })?;
        let device_id = response.device_id.trim();
        if device_id.is_empty() {
            return Err(AppError::new(
                "invalid_device_response",
                "RomM registered the device without returning a device ID.",
                false,
            ));
        }
        self.mark_contact();

        Ok(DeviceRegistrationResult {
            device_id: device_id.to_owned(),
            display_name: response.name,
            registration_fingerprint: fingerprint,
            newly_registered: status == StatusCode::CREATED,
            registered_at_ms: now_ms(),
        })
    }

    pub async fn verify_device(
        &mut self,
        device_id: &str,
    ) -> Result<Option<RemoteDevice>, AppError> {
        let token = self.token.clone().ok_or_else(|| {
            AppError::new(
                "authentication_required",
                "Connect to RomM before verifying this device.",
                false,
            )
        })?;
        if !self
            .granted_scopes
            .iter()
            .any(|scope| scope == "devices.read")
        {
            return Err(missing_scope_error(vec!["devices.read".to_owned()]));
        }
        let device_id = device_id.trim();
        if device_id.is_empty() {
            return Err(AppError::new(
                "invalid_device_id",
                "The saved RomM device ID is empty.",
                false,
            ));
        }

        let url = self.endpoint(&format!("api/devices/{device_id}"))?;
        let response = self
            .client
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(network_error)?;
        let status = response.status();
        if status == StatusCode::NOT_FOUND {
            self.mark_contact();
            return Ok(None);
        }
        if status == StatusCode::FORBIDDEN {
            return Err(missing_scope_error(vec!["devices.read".to_owned()]));
        }
        if !status.is_success() {
            return Err(http_error(status, "RomM rejected device verification"));
        }
        let remote = response.json::<RemoteDevice>().await.map_err(|error| {
            AppError::new(
                "invalid_device_response",
                format!("RomM returned unreadable device verification data: {error}"),
                false,
            )
        })?;
        if remote.id != device_id {
            return Err(AppError::new(
                "device_id_mismatch",
                "RomM returned a different device than the one requested.",
                false,
            ));
        }
        if self.account_id != Some(remote.user_id) {
            return Err(AppError::new(
                "device_owner_mismatch",
                "The saved device does not belong to the authenticated RomM account.",
                false,
            ));
        }
        self.mark_contact();
        Ok(Some(remote))
    }

    pub async fn update_device(
        &mut self,
        device: &DeviceIdentity,
    ) -> Result<Option<RemoteDevice>, AppError> {
        let token = self.token.clone().ok_or_else(|| {
            AppError::new(
                "authentication_required",
                "Connect to RomM before updating this device.",
                false,
            )
        })?;
        if !self
            .granted_scopes
            .iter()
            .any(|scope| scope == "devices.write")
        {
            return Err(missing_scope_error(vec!["devices.write".to_owned()]));
        }
        let device_id = device.romm_device_id.as_deref().ok_or_else(|| {
            AppError::new(
                "device_not_registered",
                "Register this installation before updating it.",
                false,
            )
        })?;

        let url = self.endpoint(&format!("api/devices/{device_id}"))?;
        let response = self
            .client
            .put(url)
            .bearer_auth(token)
            .json(&update_payload(device))
            .send()
            .await
            .map_err(network_error)?;
        let status = response.status();
        if status == StatusCode::NOT_FOUND {
            self.mark_contact();
            return Ok(None);
        }
        if status == StatusCode::FORBIDDEN {
            return Err(missing_scope_error(vec!["devices.write".to_owned()]));
        }
        if !status.is_success() {
            return Err(http_error(status, "RomM rejected the device update"));
        }
        let remote = response.json::<RemoteDevice>().await.map_err(|error| {
            AppError::new(
                "invalid_device_response",
                format!("RomM returned unreadable device update data: {error}"),
                false,
            )
        })?;
        if remote.id != device_id {
            return Err(AppError::new(
                "device_id_mismatch",
                "RomM returned a different device than the one updated.",
                false,
            ));
        }
        if self.account_id != Some(remote.user_id) {
            return Err(AppError::new(
                "device_owner_mismatch",
                "The updated device does not belong to the authenticated RomM account.",
                false,
            ));
        }
        self.mark_contact();
        Ok(Some(remote))
    }

    pub async fn delete_device(&mut self, device_id: &str) -> Result<bool, AppError> {
        let token = self.token.clone().ok_or_else(|| {
            AppError::new(
                "authentication_required",
                "Connect to RomM before removing this device.",
                false,
            )
        })?;
        if !self
            .granted_scopes
            .iter()
            .any(|scope| scope == "devices.write")
        {
            return Err(missing_scope_error(vec!["devices.write".to_owned()]));
        }
        let device_id = device_id.trim();
        if device_id.is_empty() {
            return Err(AppError::new(
                "invalid_device_id",
                "The saved RomM device ID is empty.",
                false,
            ));
        }

        let url = self.endpoint(&format!("api/devices/{device_id}"))?;
        let response = self
            .client
            .delete(url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(network_error)?;
        let status = response.status();
        if status == StatusCode::NOT_FOUND {
            self.mark_contact();
            return Ok(false);
        }
        if status == StatusCode::FORBIDDEN {
            return Err(missing_scope_error(vec!["devices.write".to_owned()]));
        }
        if !status.is_success() {
            return Err(http_error(status, "RomM rejected device removal"));
        }
        self.mark_contact();
        Ok(true)
    }

    async fn validate_token(&mut self, record: TokenRecord) -> Result<ValidatedAuth, AppError> {
        let token_missing = missing_required_scopes(&record.scopes);
        if !token_missing.is_empty() {
            return Err(missing_scope_error(token_missing));
        }
        let url = self.endpoint("api/users/me")?;
        let response = self
            .client
            .get(url)
            .bearer_auth(&record.raw_token)
            .send()
            .await
            .map_err(network_error)?;
        let status = response.status();
        if status == StatusCode::FORBIDDEN {
            return Err(missing_scope_error(vec!["me.read".to_owned()]));
        }
        if !status.is_success() {
            return Err(http_error(status, "Unable to verify the RomM account"));
        }
        let payload: Value = response.json().await.map_err(|error| {
            AppError::new(
                "invalid_identity_response",
                format!("RomM returned unreadable account data: {error}"),
                false,
            )
        })?;
        let account_id = payload.get("id").and_then(Value::as_i64).ok_or_else(|| {
            AppError::new(
                "invalid_identity_response",
                "RomM did not return the current account ID.",
                false,
            )
        })?;
        let account_name = payload
            .get("username")
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty())
            .ok_or_else(|| {
                AppError::new(
                    "invalid_identity_response",
                    "RomM did not return the current account name.",
                    false,
                )
            })?
            .to_owned();
        let account_scopes: Vec<String> = payload
            .get("oauth_scopes")
            .and_then(Value::as_array)
            .map(|scopes| {
                scopes
                    .iter()
                    .filter_map(Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let granted_scopes: Vec<String> = record
            .scopes
            .into_iter()
            .filter(|scope| {
                account_scopes
                    .iter()
                    .any(|account_scope| account_scope == scope)
            })
            .collect();
        let missing = missing_required_scopes(&granted_scopes);
        if !missing.is_empty() {
            return Err(missing_scope_error(missing));
        }

        Ok(ValidatedAuth {
            token: record.raw_token,
            token_id: record.id,
            account_id,
            account_name,
            granted_scopes,
        })
    }

    async fn list_token_records(&self, token: &str) -> Result<Vec<TokenRecord>, AppError> {
        let url = self.endpoint("api/client-tokens")?;
        let response = self
            .client
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(network_error)?;
        let status = response.status();
        if status == StatusCode::FORBIDDEN {
            return Err(missing_scope_error(vec!["me.read".to_owned()]));
        }
        if !status.is_success() {
            return Err(http_error(
                status,
                "Unable to inspect the manual token permissions",
            ));
        }
        let payload: Value = response.json().await.map_err(|error| {
            AppError::new(
                "invalid_token_list_response",
                format!("RomM returned unreadable token data: {error}"),
                false,
            )
        })?;
        parse_token_records(&payload)
    }

    fn commit_auth(&mut self, auth: &ValidatedAuth) {
        self.token = Some(auth.token.clone());
        self.token_id = Some(auth.token_id);
        self.account_id = Some(auth.account_id);
        self.account_name = Some(auth.account_name.clone());
        self.granted_scopes = auth.granted_scopes.clone();
        self.last_contact_at_ms = Some(now_ms());
    }

    pub async fn list_roms(&self, limit: u16, offset: u64) -> Result<RomPage, AppError> {
        self.list_library(&LibraryQuery::all(), limit, offset).await
    }

    pub async fn list_library(
        &self,
        query: &LibraryQuery,
        limit: u16,
        offset: u64,
    ) -> Result<RomPage, AppError> {
        query.validate().map_err(|message| {
            AppError::new("invalid_library_query", message, false).field("query")
        })?;
        if query.is_local() {
            return Err(AppError::new(
                "local_library_query",
                "Downloaded and active-download views are loaded from local storage.",
                false,
            ));
        }
        let token = self.token.as_deref().ok_or_else(|| {
            AppError::new(
                "authentication_required",
                "Pair this device before loading the library.",
                false,
            )
        })?;
        let limit = limit.clamp(1, 100);
        let mut url = self.endpoint("api/roms")?;
        {
            let mut pairs = url.query_pairs_mut();
            let (order_by, order_dir) = match query.sort {
                LibrarySort::Title => ("name", "asc"),
                LibrarySort::RecentlyAdded => ("id", "desc"),
                LibrarySort::ReleaseDate => ("first_release_date", "desc"),
                LibrarySort::Id => (
                    "id",
                    if query.kind == LibraryViewKind::Recent {
                        "desc"
                    } else {
                        "asc"
                    },
                ),
            };
            pairs
                .append_pair("limit", &limit.to_string())
                .append_pair("offset", &offset.to_string())
                .append_pair("order_by", order_by)
                .append_pair("order_dir", order_dir)
                .append_pair("with_char_index", "false")
                .append_pair("with_filter_values", "false")
                .append_pair("with_rom_id_index", "false");
            match query.kind {
                LibraryViewKind::Favorites => {
                    pairs.append_pair("favorite", "true");
                }
                LibraryViewKind::Platform
                | LibraryViewKind::Collection
                | LibraryViewKind::SmartCollection
                | LibraryViewKind::All
                | LibraryViewKind::Recent => {}
                LibraryViewKind::Downloaded | LibraryViewKind::ActiveDownloads => unreachable!(),
            }
            if query.favorite_only && query.kind != LibraryViewKind::Favorites {
                pairs.append_pair("favorite", "true");
            }
            if let Some(search) = query.normalized_search() {
                pairs.append_pair("search_term", &search);
            }
            if let Some(platform_id) = query.effective_platform_id() {
                pairs.append_pair("platform_ids", &platform_id.to_string());
            }
            if let Some((collection_id, kind)) = query.effective_collection() {
                pairs.append_pair(
                    match kind {
                        CollectionKind::Standard => "collection_id",
                        CollectionKind::Smart => "smart_collection_id",
                    },
                    &collection_id.to_string(),
                );
            }
        }

        let response = self
            .client
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(network_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(http_error(status, "Unable to load the RomM library"));
        }
        let payload: Value = response.json().await.map_err(|error| {
            AppError::new(
                "invalid_library_response",
                format!("RomM returned unreadable library data: {error}"),
                false,
            )
        })?;

        parse_rom_page(&payload, limit, offset)
    }

    pub async fn get_game_details(&self, rom_id: i64) -> Result<GameDetails, AppError> {
        if rom_id < 1 {
            return Err(AppError::new(
                "invalid_rom_id",
                "ROM id must be positive.",
                false,
            ));
        }
        let token = self.token.as_deref().ok_or_else(|| {
            AppError::new(
                "authentication_required",
                "Pair before loading game details.",
                false,
            )
        })?;
        let response = self
            .client
            .get(self.endpoint(&format!("api/roms/{rom_id}"))?)
            .bearer_auth(token)
            .send()
            .await
            .map_err(network_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(http_error(status, "Unable to load game details"));
        }
        let payload: Value = response.json().await.map_err(|error| {
            AppError::new(
                "invalid_game_details_response",
                format!("RomM returned unreadable game details: {error}"),
                false,
            )
        })?;
        parse_game_details(&payload)
    }

    pub async fn set_favorite(
        &self,
        rom_id: i64,
        desired: bool,
    ) -> Result<AuthoritativeFavorite, AppError> {
        if rom_id < 1 {
            return Err(AppError::new(
                "invalid_rom_id",
                "ROM id must be positive.",
                false,
            ));
        }
        let token = self.token.as_deref().ok_or_else(|| {
            AppError::new(
                "authentication_required",
                "Pair before changing favorites.",
                false,
            )
        })?;
        for scope in ["collections.read", "collections.write"] {
            if !self.granted_scopes.iter().any(|granted| granted == scope) {
                return Err(favorite_permission_error(scope));
            }
        }
        let account_id = self.account_id.ok_or_else(|| {
            AppError::new(
                "authentication_required",
                "The authenticated account must be verified before changing favorites.",
                false,
            )
        })?;

        let payload = self
            .get_authenticated_json(
                "api/collections",
                token,
                "Unable to load the RomM favorites collection",
            )
            .await
            .map_err(classify_favorite_error)?;
        let mut collection = parse_favorite_collection(&payload, account_id)?;
        if collection.is_none() && desired {
            let mut url = self.endpoint("api/collections")?;
            url.query_pairs_mut()
                .append_pair("is_favorite", "true")
                .append_pair("is_public", "false");
            let response = self
                .client
                .post(url)
                .bearer_auth(token)
                .multipart(Form::new().text("name", "Favorites"))
                .send()
                .await
                .map_err(network_error)?;
            let status = response.status();
            if !status.is_success() {
                return Err(classify_favorite_error(http_error(
                    status,
                    "Unable to create the RomM favorites collection",
                )));
            }
            let payload: Value = response.json().await.map_err(|error| {
                AppError::new(
                    "invalid_favorite_response",
                    format!("RomM returned unreadable favorite data: {error}"),
                    false,
                )
            })?;
            collection = Some(parse_collection_membership(&payload)?);
        }

        let Some(mut collection) = collection else {
            return Ok(AuthoritativeFavorite {
                result: FavoriteMutationResult {
                    rom_id,
                    requested: desired,
                    favorite: false,
                    collection_id: None,
                    collection_updated_at: None,
                },
                favorite_rom_ids: Vec::new(),
            });
        };
        let currently_favorite = collection.rom_ids.contains(&rom_id);
        if currently_favorite != desired {
            let method = if desired {
                Method::POST
            } else {
                Method::DELETE
            };
            let response = self
                .client
                .request(
                    method,
                    self.endpoint(&format!("api/collections/{}/roms", collection.id))?,
                )
                .bearer_auth(token)
                .json(&json!({ "rom_ids": [rom_id] }))
                .send()
                .await
                .map_err(network_error)?;
            let status = response.status();
            if !status.is_success() {
                return Err(classify_favorite_error(http_error(
                    status,
                    "Unable to update the RomM favorites collection",
                )));
            }
            let payload: Value = response.json().await.map_err(|error| {
                AppError::new(
                    "invalid_favorite_response",
                    format!("RomM returned unreadable favorite data: {error}"),
                    false,
                )
            })?;
            collection = parse_collection_membership(&payload)?;
        }

        let favorite = collection.rom_ids.contains(&rom_id);
        if favorite != desired {
            return Err(AppError::new(
                "favorite_not_applied",
                "RomM accepted the request but returned a different favorite state.",
                false,
            )
            .details(json!({ "romId": rom_id, "requested": desired, "favorite": favorite })));
        }
        Ok(AuthoritativeFavorite {
            result: FavoriteMutationResult {
                rom_id,
                requested: desired,
                favorite,
                collection_id: Some(collection.id),
                collection_updated_at: collection.updated_at,
            },
            favorite_rom_ids: collection.rom_ids,
        })
    }

    pub async fn fetch_artwork(
        &self,
        artwork: &ArtworkReference,
    ) -> Result<FetchedArtwork, AppError> {
        let token = self.token.as_deref().ok_or_else(|| {
            AppError::new(
                "authentication_required",
                "Pair before loading artwork.",
                false,
            )
        })?;
        let remote_path = artwork.remote_path.as_deref().ok_or_else(|| {
            AppError::new(
                "artwork_unavailable",
                "RomM has no locally hosted artwork for this game.",
                false,
            )
        })?;
        let remote_path = remote_path.trim_start_matches('/');
        let normalized_path = remote_path.to_ascii_lowercase();
        if remote_path.is_empty()
            || remote_path.split('/').any(|part| part == "..")
            || remote_path.contains('\\')
            || remote_path.contains('\0')
            || remote_path.contains('?')
            || remote_path.contains('#')
            || normalized_path.contains("%2e")
        {
            return Err(AppError::new(
                "invalid_artwork_path",
                "RomM returned an unsafe artwork path.",
                false,
            ));
        }
        let path = if remote_path.starts_with("assets/") {
            remote_path.to_owned()
        } else {
            format!("assets/romm/resources/{remote_path}")
        };
        let response = self
            .client
            .get(self.endpoint(&path)?)
            .bearer_auth(token)
            .send()
            .await
            .map_err(network_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(http_error(status, "Unable to load game artwork"));
        }
        if response
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .is_some_and(|length| length > MAX_ARTWORK_BYTES)
        {
            return Err(AppError::new(
                "artwork_too_large",
                "RomM artwork exceeds the 8 MiB safety limit.",
                false,
            ));
        }
        let etag = response
            .headers()
            .get(ETAG)
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned);
        let bytes = response.bytes().await.map_err(network_error)?;
        if bytes.len() as u64 > MAX_ARTWORK_BYTES {
            return Err(AppError::new(
                "artwork_too_large",
                "RomM artwork exceeds the 8 MiB safety limit.",
                false,
            ));
        }
        let mime_type = image_mime_type(&bytes).ok_or_else(|| {
            AppError::new(
                "invalid_artwork",
                "RomM returned artwork in an unsupported or corrupt image format.",
                false,
            )
        })?;
        Ok(FetchedArtwork {
            mime_type: mime_type.to_owned(),
            bytes: bytes.to_vec(),
            etag,
        })
    }

    pub async fn list_platforms(&self) -> Result<Vec<PlatformSummary>, AppError> {
        let token = self.token.as_deref().ok_or_else(|| {
            AppError::new(
                "authentication_required",
                "Pair this device before loading RomM platforms.",
                false,
            )
        })?;
        let response = self
            .client
            .get(self.endpoint("api/platforms")?)
            .bearer_auth(token)
            .send()
            .await
            .map_err(network_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(http_error(status, "Unable to load RomM platforms"));
        }
        let payload: Value = response.json().await.map_err(|error| {
            AppError::new(
                "invalid_platform_response",
                format!("RomM returned invalid platform JSON: {error}"),
                false,
            )
        })?;
        parse_platforms(&payload)
    }

    pub async fn get_library_metadata(&self) -> Result<LibraryMetadata, AppError> {
        let token = self.token.as_deref().ok_or_else(|| {
            AppError::new(
                "authentication_required",
                "Pair this device before loading library metadata.",
                false,
            )
        })?;
        let platforms = self
            .get_authenticated_json("api/platforms", token, "Unable to load RomM platforms")
            .await?;
        let collections = self
            .get_authenticated_json("api/collections", token, "Unable to load RomM collections")
            .await?;
        let smart_collections = self
            .get_authenticated_json(
                "api/collections/smart",
                token,
                "Unable to load RomM smart collections",
            )
            .await?;
        let mut parsed_collections = parse_collections(&collections, CollectionKind::Standard)?;
        parsed_collections.extend(parse_collections(
            &smart_collections,
            CollectionKind::Smart,
        )?);
        parsed_collections.sort_by(|left, right| {
            left.name
                .to_lowercase()
                .cmp(&right.name.to_lowercase())
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(LibraryMetadata {
            platforms: parse_library_platforms(&platforms)?,
            collections: parsed_collections,
            source: LibrarySource::Live,
            refreshed_at_ms: now_ms(),
            stale: false,
        })
    }

    async fn get_authenticated_json(
        &self,
        path: &str,
        token: &str,
        context: &str,
    ) -> Result<Value, AppError> {
        let response = self
            .client
            .get(self.endpoint(path)?)
            .bearer_auth(token)
            .send()
            .await
            .map_err(network_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(http_error(status, context));
        }
        response.json().await.map_err(|error| {
            AppError::new(
                "invalid_library_metadata_response",
                format!("RomM returned unreadable library metadata: {error}"),
                false,
            )
        })
    }

    pub fn logout(&mut self) {
        self.token = None;
        self.token_id = None;
        self.account_id = None;
        self.account_name = None;
        self.granted_scopes.clear();
    }

    pub fn mark_contact(&mut self) {
        self.last_contact_at_ms = Some(now_ms());
    }

    fn endpoint(&self, path: &str) -> Result<Url, AppError> {
        self.base_url
            .as_ref()
            .ok_or_else(|| {
                AppError::new(
                    "server_not_configured",
                    "Test the RomM server before continuing.",
                    false,
                )
            })?
            .join(path)
            .map_err(|error| internal_url_error(error.to_string()))
    }
}

impl Default for RommSession {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
struct TokenRecord {
    id: i64,
    raw_token: String,
    scopes: Vec<String>,
    last_used_at: Option<String>,
}

fn build_client(certificate: Option<&[u8]>) -> Result<Client, AppError> {
    let mut builder =
        Client::builder().user_agent(concat!("RommCompanion/", env!("CARGO_PKG_VERSION")));
    if let Some(bytes) = certificate {
        let certificate = Certificate::from_pem(bytes)
            .or_else(|_| Certificate::from_der(bytes))
            .map_err(|_| {
                AppError::new(
                    "invalid_ca_certificate",
                    "The selected file is not a valid PEM or DER certificate authority.",
                    false,
                )
            })?;
        builder = builder.add_root_certificate(certificate);
    }
    builder.build().map_err(|error| {
        AppError::new(
            "http_client_failed",
            format!("Unable to configure secure networking: {error}"),
            false,
        )
    })
}

pub fn decode_certificate_payload(payload: &str) -> Result<Vec<u8>, AppError> {
    let bytes = BASE64.decode(payload.trim()).map_err(|_| {
        AppError::new(
            "invalid_ca_certificate",
            "The imported certificate could not be decoded.",
            false,
        )
    })?;
    validate_x509_certificate(&bytes)?;
    Ok(bytes)
}

fn validate_x509_certificate(bytes: &[u8]) -> Result<(), AppError> {
    let valid = if bytes.starts_with(b"-----BEGIN") {
        parse_x509_pem(bytes)
            .ok()
            .is_some_and(|(_, pem)| parse_x509_certificate(&pem.contents).is_ok())
    } else {
        parse_x509_certificate(bytes).is_ok()
    };
    if valid {
        Ok(())
    } else {
        Err(AppError::new(
            "invalid_ca_certificate",
            "The selected file is not a valid PEM or DER X.509 certificate authority.",
            false,
        ))
    }
}

pub fn certificate_fingerprint(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn server_origin(url: &Url) -> String {
    url.origin().ascii_serialization()
}

fn normalize_pairing_code(code: &str) -> Option<String> {
    let compact: String = code
        .trim()
        .chars()
        .filter(|character| *character != '-')
        .map(|character| character.to_ascii_uppercase())
        .collect();

    if compact.len() != 8 || !compact.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return None;
    }

    Some(format!("{}-{}", &compact[..4], &compact[4..]))
}

pub fn normalize_base_url(input: &str) -> Result<Url, AppError> {
    let mut url = Url::parse(input.trim()).map_err(|_| {
        AppError::new(
            "invalid_server_url",
            "Enter a complete http:// or https:// RomM URL.",
            false,
        )
        .field("baseUrl")
    })?;

    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(AppError::new(
            "invalid_server_url",
            "Only http:// and https:// server URLs are supported.",
            false,
        )
        .field("baseUrl"));
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(AppError::new(
            "unsafe_server_url",
            "The server URL cannot contain credentials, a query, or a fragment.",
            false,
        )
        .field("baseUrl"));
    }

    let trimmed_path = url.path().trim_end_matches('/');
    let base_path = trimmed_path.strip_suffix("/api").unwrap_or(trimmed_path);
    url.set_path(&format!("{}/", base_path.trim_end_matches('/')));
    Ok(url)
}

fn is_client_token(token: &str) -> bool {
    token.len() == 68
        && token.starts_with("rmm_")
        && token[4..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn parse_token_record(payload: &Value) -> Option<TokenRecord> {
    let id = payload.get("id")?.as_i64()?;
    let raw_token = payload
        .get("raw_token")
        .or_else(|| payload.get("rawToken"))
        .and_then(Value::as_str)?
        .to_owned();
    if !is_client_token(&raw_token) {
        return None;
    }
    let scopes = payload
        .get("scopes")?
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .map(ToOwned::to_owned)
        .collect();
    Some(TokenRecord {
        id,
        raw_token,
        scopes,
        last_used_at: payload
            .get("last_used_at")
            .or_else(|| payload.get("lastUsedAt"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
    })
}

fn parse_token_records(payload: &Value) -> Result<Vec<TokenRecord>, AppError> {
    let records = payload.as_array().ok_or_else(|| {
        AppError::new(
            "invalid_token_list_response",
            "RomM did not return a token list.",
            false,
        )
    })?;
    Ok(records
        .iter()
        .filter_map(|value| {
            let id = value.get("id")?.as_i64()?;
            let scopes = value
                .get("scopes")?
                .as_array()?
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect();
            Some(TokenRecord {
                id,
                raw_token: String::new(),
                scopes,
                last_used_at: value
                    .get("last_used_at")
                    .or_else(|| value.get("lastUsedAt"))
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
            })
        })
        .collect())
}

fn uniquely_most_recent_token(records: &[TokenRecord]) -> Option<i64> {
    let latest = records
        .iter()
        .filter_map(|record| record.last_used_at.as_deref())
        .max()?;
    let mut matching = records
        .iter()
        .filter(|record| record.last_used_at.as_deref() == Some(latest));
    let id = matching.next()?.id;
    matching.next().is_none().then_some(id)
}

pub fn missing_required_scopes(granted: &[String]) -> Vec<String> {
    REQUIRED_SCOPES
        .iter()
        .filter(|required| !granted.iter().any(|scope| scope == **required))
        .map(|scope| (*scope).to_owned())
        .collect()
}

fn missing_scope_error(missing: Vec<String>) -> AppError {
    AppError::new(
        "missing_required_scopes",
        format!(
            "The Client API Token is missing required permissions: {}.",
            missing.join(", ")
        ),
        false,
    )
    .details(json!({ "missingScopes": missing }))
}

fn parse_rom_page(payload: &Value, limit: u16, offset: u64) -> Result<RomPage, AppError> {
    let values = payload
        .as_array()
        .or_else(|| payload.get("items").and_then(Value::as_array))
        .or_else(|| payload.get("results").and_then(Value::as_array))
        .or_else(|| payload.get("roms").and_then(Value::as_array))
        .ok_or_else(|| {
            AppError::new(
                "unsupported_library_shape",
                "The RomM library response did not contain a ROM list.",
                false,
            )
        })?;

    let items: Vec<RomSummary> = values
        .iter()
        .filter_map(|value| {
            let id = value.get("id")?.as_i64()?;
            let title = value
                .get("name")
                .or_else(|| value.get("title"))
                .and_then(Value::as_str)
                .unwrap_or("Untitled ROM")
                .to_owned();
            let platform = value
                .get("platform_display_name")
                .or_else(|| value.get("platformDisplayName"))
                .or_else(|| value.get("platform_name"))
                .or_else(|| value.get("platformName"))
                .and_then(Value::as_str)
                .or_else(|| {
                    value
                        .get("platform")
                        .and_then(|platform| {
                            platform
                                .get("display_name")
                                .or_else(|| platform.get("displayName"))
                                .or_else(|| platform.get("name"))
                                .or_else(|| platform.get("slug"))
                        })
                        .and_then(Value::as_str)
                })
                .or_else(|| value.get("platform_slug").and_then(Value::as_str))
                .unwrap_or("Unknown platform")
                .to_owned();
            let platform_id = value
                .get("platform_id")
                .or_else(|| value.get("platformId"))
                .and_then(Value::as_i64)
                .or_else(|| value.pointer("/platform/id").and_then(Value::as_i64));
            let summary = value
                .get("summary")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            let release_date_ms = value
                .pointer("/metadatum/first_release_date")
                .or_else(|| value.pointer("/metadata/first_release_date"))
                .or_else(|| value.get("first_release_date"))
                .and_then(Value::as_i64)
                .map(normalize_epoch_ms);
            let artwork = parse_artwork(value, id);
            let collection_ids = value
                .get("user_collections")
                .or_else(|| value.get("userCollections"))
                .and_then(Value::as_array)
                .map(|collections| {
                    collections
                        .iter()
                        .filter_map(|collection| collection.get("id").and_then(Value::as_i64))
                        .collect()
                })
                .unwrap_or_default();
            let user = parse_user_rom_state(value, id);
            Some(RomSummary {
                id,
                title,
                platform,
                platform_id,
                summary,
                release_date_ms,
                artwork,
                collection_ids,
                user,
                remote_filename: value
                    .get("fs_name")
                    .or_else(|| value.get("fsName"))
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                remote_size_bytes: value
                    .get("fs_size_bytes")
                    .or_else(|| value.get("fsSizeBytes"))
                    .and_then(Value::as_u64),
                local_status: LocalGameStatus::RemoteOnly,
                local_path: None,
                active_download_id: None,
                metadata_updated_at: value
                    .get("updated_at")
                    .or_else(|| value.get("updatedAt"))
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
            })
        })
        .collect();
    let total = find_total(payload);
    let loaded_through = offset.saturating_add(items.len() as u64);
    let has_more = total
        .map(|total| loaded_through < total)
        .unwrap_or(items.len() == usize::from(limit));

    Ok(RomPage {
        items,
        offset,
        limit,
        total,
        has_more,
        source: LibrarySource::Live,
        refreshed_at_ms: now_ms(),
        stale: false,
    })
}

fn parse_game_details(payload: &Value) -> Result<GameDetails, AppError> {
    let mut page = parse_rom_page(&json!({ "items": [payload] }), 1, 0)?;
    let rom = page.items.pop().ok_or_else(|| {
        AppError::new(
            "invalid_game_details_response",
            "RomM game details did not include a valid ROM.",
            false,
        )
    })?;
    let metadata = payload
        .get("metadatum")
        .or_else(|| payload.get("metadata"))
        .unwrap_or(&Value::Null);
    let strings = |value: Option<&Value>| {
        value
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    };
    let files = payload
        .get("files")
        .and_then(Value::as_array)
        .map(|files| {
            files
                .iter()
                .filter_map(|file| {
                    Some(GameFile {
                        id: file.get("id")?.as_i64()?,
                        name: file.get("file_name")?.as_str()?.to_owned(),
                        size_bytes: file
                            .get("file_size_bytes")
                            .and_then(Value::as_u64)
                            .unwrap_or_default(),
                        category: file
                            .get("category")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned),
                        crc_hash: file
                            .get("crc_hash")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned),
                        sha1_hash: file
                            .get("sha1_hash")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let collections = payload
        .get("user_collections")
        .and_then(Value::as_array)
        .map(|collections| {
            collections
                .iter()
                .filter_map(|collection| {
                    Some(GameCollection {
                        id: collection.get("id")?.as_i64()?,
                        name: collection.get("name")?.as_str()?.to_owned(),
                        kind: if collection
                            .get("is_smart")
                            .and_then(Value::as_bool)
                            .unwrap_or(false)
                        {
                            CollectionKind::Smart
                        } else {
                            CollectionKind::Standard
                        },
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let siblings = payload
        .get("sibling_roms")
        .and_then(Value::as_array)
        .map(|siblings| {
            siblings
                .iter()
                .filter_map(|sibling| {
                    Some(GameSibling {
                        id: sibling.get("id")?.as_i64()?,
                        name: sibling
                            .get("name")
                            .or_else(|| sibling.get("fs_name_no_tags"))
                            .and_then(Value::as_str)?
                            .to_owned(),
                        is_main: sibling
                            .get("is_main_sibling")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let refreshed_at_ms = now_ms();
    Ok(GameDetails {
        rom,
        alternative_names: strings(payload.get("alternative_names")),
        genres: strings(metadata.get("genres")),
        franchises: strings(metadata.get("franchises")),
        companies: strings(metadata.get("companies")),
        game_modes: strings(metadata.get("game_modes")),
        age_ratings: strings(metadata.get("age_ratings")),
        player_count: metadata
            .get("player_count")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        average_rating: metadata
            .get("average_rating")
            .and_then(Value::as_f64)
            .map(|rating| format!("{rating:.1}")),
        regions: strings(payload.get("regions")),
        languages: strings(payload.get("languages")),
        tags: strings(payload.get("tags")),
        files,
        collections,
        siblings,
        screenshot_paths: strings(payload.get("merged_screenshots")),
        save_count: payload
            .get("user_saves")
            .and_then(Value::as_array)
            .map_or(0, |values| values.len() as u64),
        state_count: payload
            .get("user_states")
            .and_then(Value::as_array)
            .map_or(0, |values| values.len() as u64),
        has_manual: payload
            .get("has_manual")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        has_soundtrack: payload
            .get("has_soundtrack")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        source: LibrarySource::Live,
        refreshed_at_ms,
        stale: false,
    })
}

fn parse_artwork(value: &Value, rom_id: i64) -> Vec<ArtworkReference> {
    let revision = value
        .get("updated_at")
        .or_else(|| value.get("updatedAt"))
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    [
        (
            ArtworkKind::CoverSmall,
            "path_cover_small",
            "pathCoverSmall",
        ),
        (
            ArtworkKind::CoverLarge,
            "path_cover_large",
            "pathCoverLarge",
        ),
    ]
    .into_iter()
    .filter_map(|(kind, snake, camel)| {
        let remote_path = value
            .get(snake)
            .or_else(|| value.get(camel))
            .and_then(Value::as_str)
            .filter(|path| !path.is_empty())?
            .to_owned();
        Some(ArtworkReference {
            kind,
            cache_key: format!(
                "rom:{rom_id}:{}:{remote_path}:{revision}",
                artwork_kind_key(kind)
            ),
            remote_path: Some(remote_path),
            remote_url: None,
        })
    })
    .chain(
        value
            .get("url_cover")
            .or_else(|| value.get("urlCover"))
            .and_then(Value::as_str)
            .filter(|url| !url.is_empty())
            .map(|url| ArtworkReference {
                kind: ArtworkKind::RemoteCover,
                cache_key: format!("rom:{rom_id}:remote-cover:{url}:{revision}"),
                remote_path: None,
                remote_url: Some(url.to_owned()),
            }),
    )
    .collect()
}

fn artwork_kind_key(kind: ArtworkKind) -> &'static str {
    match kind {
        ArtworkKind::CoverSmall => "cover-small",
        ArtworkKind::CoverLarge => "cover-large",
        ArtworkKind::RemoteCover => "remote-cover",
    }
}

pub fn image_mime_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12
        && &bytes[4..8] == b"ftyp"
        && (&bytes[8..12] == b"avif" || &bytes[8..12] == b"avis")
    {
        Some("image/avif")
    } else {
        None
    }
}

fn parse_user_rom_state(value: &Value, rom_id: i64) -> UserRomState {
    let user = value
        .get("rom_user")
        .or_else(|| value.get("romUser"))
        .unwrap_or(&Value::Null);
    let number = |key: &str| user.get(key).and_then(Value::as_i64).unwrap_or_default();
    UserRomState {
        rom_id,
        favorite: value
            .get("is_favorite")
            .or_else(|| value.get("isFavorite"))
            .or_else(|| user.get("favorite"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        favorite_pending: false,
        backlogged: user
            .get("backlogged")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        hidden: user.get("hidden").and_then(Value::as_bool).unwrap_or(false),
        rating: number("rating"),
        difficulty: number("difficulty"),
        completion: number("completion"),
        status: user
            .get("status")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        last_played: user
            .get("last_played")
            .or_else(|| user.get("lastPlayed"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        updated_at: user
            .get("updated_at")
            .or_else(|| user.get("updatedAt"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
    }
}

fn normalize_epoch_ms(value: i64) -> i64 {
    if value.abs() < 10_000_000_000 {
        value.saturating_mul(1_000)
    } else {
        value
    }
}

fn parse_library_platforms(payload: &Value) -> Result<Vec<LibraryPlatform>, AppError> {
    let values = payload
        .as_array()
        .or_else(|| payload.get("items").and_then(Value::as_array))
        .or_else(|| payload.get("platforms").and_then(Value::as_array))
        .ok_or_else(|| {
            AppError::new(
                "unsupported_platform_shape",
                "The RomM platform response did not contain a platform list.",
                false,
            )
        })?;
    let summaries = parse_platforms(payload)?;
    Ok(summaries
        .into_iter()
        .map(|platform| LibraryPlatform {
            rom_count: values
                .iter()
                .find(|value| value.get("id").and_then(Value::as_i64) == Some(platform.id))
                .and_then(|value| {
                    value
                        .get("rom_count")
                        .or_else(|| value.get("romCount"))
                        .and_then(Value::as_u64)
                }),
            id: platform.id,
            name: platform.name,
            slug: platform.slug,
        })
        .collect())
}

fn parse_collections(
    payload: &Value,
    kind: CollectionKind,
) -> Result<Vec<LibraryCollection>, AppError> {
    let values = payload
        .as_array()
        .or_else(|| payload.get("items").and_then(Value::as_array))
        .or_else(|| payload.get("collections").and_then(Value::as_array))
        .ok_or_else(|| {
            AppError::new(
                "unsupported_collection_shape",
                "The RomM collection response did not contain a collection list.",
                false,
            )
        })?;
    Ok(values
        .iter()
        .filter_map(|value| {
            let id = value.get("id")?.as_i64()?;
            let name = value.get("name")?.as_str()?.trim().to_owned();
            if id <= 0 || name.is_empty() {
                return None;
            }
            let rom_ids = value
                .get("rom_ids")
                .or_else(|| value.get("romIds"))
                .and_then(Value::as_array)
                .map(|ids| ids.iter().filter_map(Value::as_i64).collect::<Vec<_>>())
                .unwrap_or_default();
            let rom_count = value
                .get("rom_count")
                .or_else(|| value.get("romCount"))
                .and_then(Value::as_u64)
                .or_else(|| (!rom_ids.is_empty()).then_some(rom_ids.len() as u64));
            Some(LibraryCollection {
                id,
                name,
                kind,
                is_favorite: value
                    .get("is_favorite")
                    .or_else(|| value.get("isFavorite"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                rom_ids,
                rom_count,
                updated_at: value
                    .get("updated_at")
                    .or_else(|| value.get("updatedAt"))
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
            })
        })
        .collect())
}

fn parse_favorite_collection(
    payload: &Value,
    account_id: i64,
) -> Result<Option<FavoriteCollection>, AppError> {
    let values = payload
        .as_array()
        .or_else(|| payload.get("items").and_then(Value::as_array))
        .or_else(|| payload.get("collections").and_then(Value::as_array))
        .ok_or_else(|| {
            AppError::new(
                "invalid_favorite_response",
                "RomM did not return a collection list while resolving favorites.",
                false,
            )
        })?;
    let matches = values
        .iter()
        .filter(|value| {
            value
                .get("is_favorite")
                .or_else(|| value.get("isFavorite"))
                .and_then(Value::as_bool)
                == Some(true)
                && value
                    .get("user_id")
                    .or_else(|| value.get("userId"))
                    .and_then(Value::as_i64)
                    == Some(account_id)
        })
        .map(parse_collection_membership)
        .collect::<Result<Vec<_>, _>>()?;
    match matches.as_slice() {
        [] => Ok(None),
        [collection] => Ok(Some(collection.clone())),
        _ => Err(AppError::new(
            "ambiguous_favorite_collection",
            "RomM returned more than one private favorites collection for this account.",
            false,
        )),
    }
}

fn parse_collection_membership(payload: &Value) -> Result<FavoriteCollection, AppError> {
    let id = payload.get("id").and_then(Value::as_i64).ok_or_else(|| {
        AppError::new(
            "invalid_favorite_response",
            "RomM returned a favorites collection without an ID.",
            false,
        )
    })?;
    let rom_ids = payload
        .get("rom_ids")
        .or_else(|| payload.get("romIds"))
        .and_then(Value::as_array)
        .ok_or_else(|| {
            AppError::new(
                "invalid_favorite_response",
                "RomM returned a favorites collection without its ROM membership.",
                false,
            )
        })?
        .iter()
        .filter_map(Value::as_i64)
        .collect();
    Ok(FavoriteCollection {
        id,
        rom_ids,
        updated_at: payload
            .get("updated_at")
            .or_else(|| payload.get("updatedAt"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
    })
}

fn favorite_permission_error(scope: &str) -> AppError {
    AppError::new(
        "favorite_permission_denied",
        format!("Changing favorites requires the `{scope}` token permission."),
        false,
    )
    .details(json!({ "missingScope": scope }))
}

fn classify_favorite_error(error: AppError) -> AppError {
    match error.code.as_str() {
        "forbidden" => favorite_permission_error("collections.write"),
        "not_found" => AppError::new(
            "favorite_rom_not_found",
            "The game or favorites collection is no longer available on RomM.",
            false,
        ),
        "http_error" => AppError::new(
            "favorite_rejected",
            "RomM rejected the favorite change.",
            false,
        ),
        _ => error,
    }
}

fn parse_platforms(payload: &Value) -> Result<Vec<PlatformSummary>, AppError> {
    let values = payload
        .as_array()
        .or_else(|| payload.get("items").and_then(Value::as_array))
        .or_else(|| payload.get("platforms").and_then(Value::as_array))
        .ok_or_else(|| {
            AppError::new(
                "unsupported_platform_shape",
                "The RomM platform response did not contain a platform list.",
                false,
            )
        })?;
    let mut platforms = values
        .iter()
        .filter_map(|value| {
            let id = value.get("id")?.as_i64()?;
            let slug = value
                .get("slug")
                .or_else(|| value.get("fs_slug"))
                .or_else(|| value.get("fsSlug"))
                .and_then(Value::as_str)?
                .trim()
                .to_owned();
            if id <= 0 || slug.is_empty() {
                return None;
            }
            let name = value
                .get("display_name")
                .or_else(|| value.get("displayName"))
                .or_else(|| value.get("name"))
                .and_then(Value::as_str)
                .unwrap_or(&slug)
                .trim()
                .to_owned();
            Some(PlatformSummary { id, name, slug })
        })
        .collect::<Vec<_>>();
    platforms.sort_by_key(|platform| platform.name.to_lowercase());
    platforms.dedup_by_key(|platform| platform.id);
    if platforms.is_empty() && !values.is_empty() {
        return Err(AppError::new(
            "unsupported_platform_shape",
            "RomM returned platforms without usable IDs and slugs.",
            false,
        ));
    }
    Ok(platforms)
}

fn find_total(payload: &Value) -> Option<u64> {
    [
        "/total",
        "/total_count",
        "/totalCount",
        "/count",
        "/pagination/total",
        "/meta/total",
    ]
    .iter()
    .find_map(|pointer| payload.pointer(pointer).and_then(Value::as_u64))
}

fn network_error(error: reqwest::Error) -> AppError {
    let detail = error.to_string();
    let lower = detail.to_ascii_lowercase();
    if lower.contains("certificate")
        || lower.contains("tls")
        || lower.contains("unknown issuer")
        || lower.contains("invalid peer")
    {
        AppError::new(
            "tls_error",
            "RomM's TLS certificate could not be verified. Import the server's certificate authority or correct the certificate hostname.",
            false,
        )
        .cause("certificate_validation_failed")
    } else {
        AppError::new(
            "network_error",
            format!("Unable to reach RomM: {detail}"),
            true,
        )
    }
}

fn http_error(status: StatusCode, context: &str) -> AppError {
    let (code, retryable) = match status {
        StatusCode::UNAUTHORIZED => ("unauthorized", false),
        StatusCode::FORBIDDEN => ("forbidden", false),
        StatusCode::NOT_FOUND => ("not_found", false),
        status if status.is_server_error() => ("server_error", true),
        _ => ("http_error", false),
    };
    AppError::new(code, format!("{context} (HTTP {status})."), retryable)
}

fn internal_url_error(detail: String) -> AppError {
    AppError::new(
        "url_construction_failed",
        format!("Unable to construct a RomM API URL: {detail}"),
        false,
    )
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
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::oneshot,
    };

    async fn mock_json_server(responses: Vec<(u16, String)>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock server should bind");
        let address = listener.local_addr().expect("mock address should resolve");
        tokio::spawn(async move {
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().await.expect("request should connect");
                let mut request = vec![0_u8; 8192];
                let _ = stream.read(&mut request).await;
                let reason = match status {
                    200 => "OK",
                    403 => "Forbidden",
                    404 => "Not Found",
                    500 => "Internal Server Error",
                    _ => "Response",
                };
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream
                    .write_all(response.as_bytes())
                    .await
                    .expect("response should write");
            }
        });
        format!("http://{address}")
    }

    async fn mock_captured_json_server(
        status: u16,
        body: String,
    ) -> (String, oneshot::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock server should bind");
        let address = listener.local_addr().expect("mock address should resolve");
        let (request_tx, request_rx) = oneshot::channel();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("request should connect");
            let mut request = vec![0_u8; 16_384];
            let read = stream
                .read(&mut request)
                .await
                .expect("request should read");
            request.truncate(read);
            let _ = request_tx.send(String::from_utf8_lossy(&request).into_owned());
            let reason = match status {
                200 => "OK",
                201 => "Created",
                401 => "Unauthorized",
                403 => "Forbidden",
                500 => "Internal Server Error",
                _ => "Response",
            };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("response should write");
        });
        (format!("http://{address}"), request_rx)
    }

    fn authenticated_session(base_url: &str) -> RommSession {
        let token = format!("rmm_{}", "a".repeat(64));
        let mut session = RommSession::new();
        session
            .restore(base_url, Some("5.1.0".to_owned()), &token)
            .expect("mock session should restore");
        session.restore_metadata(
            Some(42),
            Some(7),
            Some("justin".to_owned()),
            REQUIRED_SCOPES
                .iter()
                .map(|scope| (*scope).to_owned())
                .collect(),
            Some(1234),
            true,
            None,
        );
        session
    }

    #[test]
    fn normalizes_root_and_api_urls() {
        assert_eq!(
            normalize_base_url("https://romm.example.test/")
                .expect("root URL should parse")
                .as_str(),
            "https://romm.example.test/"
        );
        assert_eq!(
            normalize_base_url("https://host.test/romm/api/")
                .expect("API URL should parse")
                .as_str(),
            "https://host.test/romm/"
        );
    }

    #[test]
    fn rejects_credentials_and_non_http_schemes() {
        assert!(normalize_base_url("https://user:pass@host.test").is_err());
        assert!(normalize_base_url("file:///tmp/romm").is_err());
    }

    #[test]
    fn validates_and_parses_exchange_tokens() {
        let token = format!("rmm_{}", "a".repeat(64));
        assert!(is_client_token(&token));
        let record = parse_token_record(&json!({
            "id": 7,
            "raw_token": token,
            "scopes": REQUIRED_SCOPES,
        }))
        .expect("exchange token should parse");
        assert_eq!(record.id, 7);
        assert_eq!(record.scopes.len(), REQUIRED_SCOPES.len());
    }

    #[test]
    fn normalizes_alphanumeric_pairing_codes() {
        assert_eq!(
            normalize_pairing_code("jm38-mhsa"),
            Some("JM38-MHSA".to_owned())
        );
        assert_eq!(
            normalize_pairing_code("JM38MHSA"),
            Some("JM38-MHSA".to_owned())
        );
        assert_eq!(normalize_pairing_code("1234"), None);
        assert_eq!(normalize_pairing_code("JM38-!HSA"), None);
    }

    #[test]
    fn parses_paginated_rom_lists() {
        let payload = json!({
            "items": [{ "id": 42, "name": "Chrono Trigger", "platform_display_name": "Super Nintendo Entertainment System" }],
            "total": 107
        });
        let page = parse_rom_page(&payload, 48, 0).expect("ROMs should parse");
        assert_eq!(page.items[0].title, "Chrono Trigger");
        assert_eq!(
            page.items[0].platform,
            "Super Nintendo Entertainment System"
        );
        assert_eq!(page.total, Some(107));
        assert!(page.has_more);
    }

    #[tokio::test]
    async fn requests_stable_lightweight_rom_pages_with_explicit_offsets() {
        let (base_url, request) =
            mock_captured_json_server(200, json!({ "items": [], "total": 107 }).to_string()).await;
        let session = authenticated_session(&base_url);

        let page = session
            .list_roms(48, 96)
            .await
            .expect("ROM page should load");
        let request = request.await.expect("request should be captured");

        assert!(request.starts_with("GET /api/roms?"));
        assert!(request.contains("limit=48"));
        assert!(request.contains("offset=96"));
        assert!(request.contains("order_by=id"));
        assert!(request.contains("order_dir=asc"));
        assert!(request.contains("with_char_index=false"));
        assert!(request.contains("with_filter_values=false"));
        assert!(request.contains("with_rom_id_index=false"));
        assert_eq!(page.offset, 96);
        assert_eq!(page.limit, 48);
    }

    #[tokio::test]
    async fn translates_typed_library_views_to_romm_filters() {
        let cases = [
            (
                LibraryQuery {
                    kind: LibraryViewKind::Recent,
                    id: None,
                    ..LibraryQuery::all()
                },
                "order_dir=desc",
            ),
            (
                LibraryQuery {
                    kind: LibraryViewKind::Favorites,
                    id: None,
                    ..LibraryQuery::all()
                },
                "favorite=true",
            ),
            (
                LibraryQuery {
                    kind: LibraryViewKind::Platform,
                    id: Some(7),
                    ..LibraryQuery::all()
                },
                "platform_ids=7",
            ),
            (
                LibraryQuery {
                    kind: LibraryViewKind::Collection,
                    id: Some(8),
                    ..LibraryQuery::all()
                },
                "collection_id=8",
            ),
            (
                LibraryQuery {
                    kind: LibraryViewKind::SmartCollection,
                    id: Some(9),
                    ..LibraryQuery::all()
                },
                "smart_collection_id=9",
            ),
        ];
        for (query, expected) in cases {
            let (base_url, request) =
                mock_captured_json_server(200, json!({ "items": [], "total": 0 }).to_string())
                    .await;
            authenticated_session(&base_url)
                .list_library(&query, 12, 0)
                .await
                .expect("library view should load");
            let request = request.await.expect("request should be captured");
            assert!(
                request.contains(expected),
                "missing {expected} in {request}"
            );
        }
    }

    #[tokio::test]
    async fn translates_combined_discovery_filters_and_sorting() {
        let (base_url, request) =
            mock_captured_json_server(200, json!({ "items": [], "total": 0 }).to_string()).await;
        let query = LibraryQuery {
            search: Some("  chrono trigger  ".to_owned()),
            platform_id: Some(7),
            collection_id: Some(9),
            collection_kind: Some(CollectionKind::Smart),
            favorite_only: true,
            sort: LibrarySort::ReleaseDate,
            ..LibraryQuery::all()
        };

        authenticated_session(&base_url)
            .list_library(&query, 48, 0)
            .await
            .expect("filtered library should load");
        let request = request.await.expect("request should be captured");

        assert!(request.starts_with("GET /api/roms?"));
        assert!(request.contains("search_term=chrono+trigger"));
        assert!(request.contains("platform_ids=7"));
        assert!(request.contains("smart_collection_id=9"));
        assert!(request.contains("favorite=true"));
        assert!(request.contains("order_by=first_release_date"));
        assert!(request.contains("order_dir=desc"));
    }

    #[tokio::test]
    async fn loads_and_normalizes_complete_game_details() {
        let payload = json!({
            "id": 42,
            "name": "Chrono Trigger",
            "platform_id": 7,
            "platform_display_name": "SNES",
            "summary": "A time-travel adventure.",
            "fs_name": "Chrono Trigger.sfc",
            "fs_size_bytes": 4194304,
            "alternative_names": ["Chrono"],
            "metadatum": {
                "genres": ["Role-playing"],
                "franchises": ["Chrono"],
                "companies": ["Square"],
                "game_modes": ["Single player"],
                "age_ratings": ["ESRB E"],
                "player_count": "1",
                "first_release_date": 811036800,
                "average_rating": 91.54
            },
            "regions": ["USA"],
            "languages": ["English"],
            "tags": ["Classic"],
            "files": [{
                "id": 5,
                "file_name": "Chrono Trigger.sfc",
                "file_size_bytes": 4194304,
                "category": "game",
                "sha1_hash": "abc"
            }],
            "sibling_roms": [{
                "id": 43,
                "name": "Chrono Trigger (Japan)",
                "is_main_sibling": false
            }],
            "user_collections": [{ "id": 3, "name": "Best RPGs", "is_smart": false }],
            "user_saves": [{ "id": 1 }],
            "user_states": [{ "id": 2 }, { "id": 3 }],
            "merged_screenshots": ["/assets/one.webp"],
            "has_manual": true,
            "has_soundtrack": false,
            "rom_user": {
                "backlogged": false,
                "hidden": false,
                "rating": 9,
                "difficulty": 4,
                "completion": 100
            },
            "is_favorite": true
        });
        let (base_url, request) = mock_captured_json_server(200, payload.to_string()).await;

        let details = authenticated_session(&base_url)
            .get_game_details(42)
            .await
            .expect("game details should load");
        let request = request.await.expect("request should be captured");

        assert!(request.starts_with("GET /api/roms/42 "));
        assert_eq!(details.rom.title, "Chrono Trigger");
        assert_eq!(details.genres, vec!["Role-playing"]);
        assert_eq!(details.files[0].size_bytes, 4_194_304);
        assert_eq!(details.collections[0].name, "Best RPGs");
        assert_eq!(details.siblings[0].id, 43);
        assert_eq!(details.save_count, 1);
        assert_eq!(details.state_count, 2);
        assert_eq!(details.average_rating.as_deref(), Some("91.5"));
        assert!(details.has_manual);
        assert_eq!(details.source, LibrarySource::Live);
    }

    #[test]
    fn normalizes_rom_user_artwork_collection_and_remote_file_fields() {
        let payload = json!({
            "items": [{
                "id": 42,
                "name": "Chrono Trigger",
                "platform_id": 7,
                "platform_display_name": "Super Nintendo Entertainment System",
                "summary": "A time-travel adventure.",
                "fs_name": "Chrono Trigger.sfc",
                "fs_size_bytes": 4194304,
                "path_cover_small": "/assets/rom/42/cover-small.webp",
                "url_cover": "https://images.example.test/chrono.jpg",
                "updated_at": "2026-08-31T12:00:00Z",
                "metadatum": { "first_release_date": 811036800 },
                "rom_user": {
                    "backlogged": true,
                    "hidden": false,
                    "rating": 9,
                    "difficulty": 4,
                    "completion": 100,
                    "status": "completed",
                    "last_played": "2026-08-30T12:00:00Z",
                    "updated_at": "2026-08-30T12:01:00Z"
                },
                "is_favorite": true,
                "user_collections": [{ "id": 3 }, { "id": 9 }]
            }],
            "total": 1
        });

        let page = parse_rom_page(&payload, 48, 0).expect("ROMs should normalize");
        let rom = &page.items[0];
        assert_eq!(rom.platform_id, Some(7));
        assert_eq!(rom.release_date_ms, Some(811_036_800_000));
        assert_eq!(rom.collection_ids, vec![3, 9]);
        assert!(rom.user.favorite);
        assert!(rom.user.backlogged);
        assert_eq!(rom.user.rating, 9);
        assert_eq!(rom.artwork.len(), 2);
        assert!(
            rom.artwork[0].cache_key.contains("2026-08-31T12:00:00Z"),
            "artwork cache keys must change when RomM updates the ROM"
        );
        assert_eq!(rom.remote_filename.as_deref(), Some("Chrono Trigger.sfc"));
        assert_eq!(rom.remote_size_bytes, Some(4_194_304));
        assert_eq!(page.source, LibrarySource::Live);
        assert!(!page.stale);
    }

    #[test]
    fn accepts_supported_image_signatures_and_rejects_active_content() {
        assert_eq!(image_mime_type(b"\x89PNG\r\n\x1a\nrest"), Some("image/png"));
        assert_eq!(image_mime_type(b"\xff\xd8\xffrest"), Some("image/jpeg"));
        assert_eq!(
            image_mime_type(b"RIFF\x00\x00\x00\x00WEBPrest"),
            Some("image/webp")
        );
        assert_eq!(image_mime_type(b"GIF89arest"), Some("image/gif"));
        assert_eq!(
            image_mime_type(b"\x00\x00\x00\x18ftypavifrest"),
            Some("image/avif")
        );
        assert_eq!(image_mime_type(b"<svg onload='alert(1)'></svg>"), None);
        assert_eq!(image_mime_type(b"<html>not artwork</html>"), None);
    }

    #[tokio::test]
    async fn rejects_unsafe_or_external_artwork_before_contacting_the_server() {
        let session = authenticated_session("http://127.0.0.1:9");
        for remote_path in ["../secret.png", "covers\\secret.png", ""] {
            let error = session
                .fetch_artwork(&ArtworkReference {
                    kind: ArtworkKind::CoverSmall,
                    remote_path: Some(remote_path.to_owned()),
                    remote_url: None,
                    cache_key: "unsafe".to_owned(),
                })
                .await
                .expect_err("unsafe artwork paths must be rejected");
            assert_eq!(error.code, "invalid_artwork_path");
        }
        let error = session
            .fetch_artwork(&ArtworkReference {
                kind: ArtworkKind::RemoteCover,
                remote_path: None,
                remote_url: Some("https://images.example.test/cover.png".to_owned()),
                cache_key: "external".to_owned(),
            })
            .await
            .expect_err("external URLs must not be fetched by the authenticated client");
        assert_eq!(error.code, "artwork_unavailable");
    }

    #[tokio::test]
    async fn loads_platform_and_standard_and_smart_collection_contracts() {
        let base_url = mock_json_server(vec![
            (
                200,
                json!([{ "id": 7, "display_name": "SNES", "slug": "snes", "rom_count": 12 }])
                    .to_string(),
            ),
            (
                200,
                json!([{ "id": 3, "name": "Favorites", "rom_ids": [42], "updated_at": "2026-08-31T12:00:00Z" }])
                    .to_string(),
            ),
            (
                200,
                json!([{ "id": 4, "name": "Recently Added", "rom_ids": [42, 43] }])
                    .to_string(),
            ),
        ])
        .await;
        let session = authenticated_session(&base_url);

        let metadata = session
            .get_library_metadata()
            .await
            .expect("library metadata should load");

        assert_eq!(metadata.platforms[0].rom_count, Some(12));
        assert_eq!(metadata.collections.len(), 2);
        assert_eq!(metadata.collections[0].kind, CollectionKind::Standard);
        assert_eq!(metadata.collections[1].kind, CollectionKind::Smart);
        assert_eq!(metadata.source, LibrarySource::Live);
    }

    #[tokio::test]
    async fn adds_and_removes_favorites_through_the_private_favorites_collection() {
        let add_server = mock_json_server(vec![
            (
                200,
                json!([{
                    "id": 3,
                    "name": "Favorites",
                    "is_favorite": true,
                    "user_id": 7,
                    "rom_ids": []
                }])
                .to_string(),
            ),
            (
                200,
                json!({
                    "id": 3,
                    "is_favorite": true,
                    "user_id": 7,
                    "rom_ids": [42],
                    "updated_at": "2026-09-04T12:00:00Z"
                })
                .to_string(),
            ),
        ])
        .await;
        let added = authenticated_session(&add_server)
            .set_favorite(42, true)
            .await
            .expect("favorite should be added");
        assert!(added.result.favorite);
        assert_eq!(added.result.collection_id, Some(3));
        assert_eq!(added.favorite_rom_ids, vec![42]);

        let remove_server = mock_json_server(vec![
            (
                200,
                json!([{
                    "id": 3,
                    "name": "Favorites",
                    "is_favorite": true,
                    "user_id": 7,
                    "rom_ids": [42]
                }])
                .to_string(),
            ),
            (
                200,
                json!({
                    "id": 3,
                    "is_favorite": true,
                    "user_id": 7,
                    "rom_ids": []
                })
                .to_string(),
            ),
        ])
        .await;
        let removed = authenticated_session(&remove_server)
            .set_favorite(42, false)
            .await
            .expect("favorite should be removed");
        assert!(!removed.result.favorite);
        assert!(removed.favorite_rom_ids.is_empty());
    }

    #[tokio::test]
    async fn creates_favorites_only_when_the_first_addition_needs_it() {
        let create_server = mock_json_server(vec![
            (200, "[]".to_owned()),
            (
                200,
                json!({
                    "id": 3,
                    "is_favorite": true,
                    "user_id": 7,
                    "rom_ids": []
                })
                .to_string(),
            ),
            (
                200,
                json!({
                    "id": 3,
                    "is_favorite": true,
                    "user_id": 7,
                    "rom_ids": [42]
                })
                .to_string(),
            ),
        ])
        .await;
        let created = authenticated_session(&create_server)
            .set_favorite(42, true)
            .await
            .expect("first favorite should create the collection");
        assert!(created.result.favorite);
        assert_eq!(created.result.collection_id, Some(3));

        let no_op_server = mock_json_server(vec![(200, "[]".to_owned())]).await;
        let removed = authenticated_session(&no_op_server)
            .set_favorite(42, false)
            .await
            .expect("removing without a favorites collection should be a no-op");
        assert!(!removed.result.favorite);
        assert_eq!(removed.result.collection_id, None);
    }

    #[tokio::test]
    async fn favorite_failures_identify_permissions_and_missing_roms() {
        let forbidden = mock_json_server(vec![(403, "{}".to_owned())]).await;
        let error = authenticated_session(&forbidden)
            .set_favorite(42, true)
            .await
            .expect_err("403 should fail");
        assert_eq!(error.code, "favorite_permission_denied");
        assert_eq!(
            error.details.as_deref(),
            Some(&json!({ "missingScope": "collections.write" }))
        );

        let missing = mock_json_server(vec![
            (
                200,
                json!([{
                    "id": 3,
                    "is_favorite": true,
                    "user_id": 7,
                    "rom_ids": []
                }])
                .to_string(),
            ),
            (404, "{}".to_owned()),
        ])
        .await;
        let error = authenticated_session(&missing)
            .set_favorite(42, true)
            .await
            .expect_err("404 should fail");
        assert_eq!(error.code, "favorite_rom_not_found");
        assert!(!error.retryable);
    }

    #[test]
    fn resolves_only_the_authenticated_accounts_favorites_collection() {
        let payload = json!([
            { "id": 1, "is_favorite": true, "user_id": 99, "rom_ids": [5] },
            { "id": 3, "is_favorite": true, "user_id": 7, "rom_ids": [42] }
        ]);
        let collection = parse_favorite_collection(&payload, 7)
            .expect("collection response should parse")
            .expect("own favorites collection should exist");
        assert_eq!(collection.id, 3);
        assert_eq!(collection.rom_ids, vec![42]);
    }

    #[test]
    fn infers_the_last_page_when_total_is_unavailable() {
        let payload = json!([
            { "id": 42, "name": "Chrono Trigger", "platform_name": "SNES" }
        ]);
        let page = parse_rom_page(&payload, 48, 96).expect("ROMs should parse");
        assert_eq!(page.total, None);
        assert!(!page.has_more);
    }

    #[test]
    fn reports_the_exact_missing_required_scopes() {
        let granted = REQUIRED_SCOPES
            .iter()
            .filter(|scope| **scope != "me.read" && **scope != "devices.write")
            .map(|scope| (*scope).to_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            missing_required_scopes(&granted),
            vec!["me.read".to_owned(), "devices.write".to_owned()]
        );
    }

    #[test]
    fn correlates_only_a_unique_most_recent_manual_token() {
        let records = vec![
            TokenRecord {
                id: 1,
                raw_token: String::new(),
                scopes: Vec::new(),
                last_used_at: Some("2026-08-25T20:00:00Z".to_owned()),
            },
            TokenRecord {
                id: 2,
                raw_token: String::new(),
                scopes: Vec::new(),
                last_used_at: Some("2026-08-25T20:00:01Z".to_owned()),
            },
        ];
        assert_eq!(uniquely_most_recent_token(&records), Some(2));

        let mut tied = records;
        tied[0].last_used_at = tied[1].last_used_at.clone();
        assert_eq!(uniquely_most_recent_token(&tied), None);
    }

    #[tokio::test]
    async fn requires_confirmation_before_contacting_an_http_server() {
        let error = RommSession::new()
            .probe("http://romm.example.test", false)
            .await
            .expect_err("HTTP should require confirmation");
        assert_eq!(error.code, "insecure_http_confirmation_required");
        assert_eq!(
            error.details.as_deref(),
            Some(&json!({ "serverOrigin": "http://romm.example.test" }))
        );
    }

    #[test]
    fn rejects_invalid_imported_certificates() {
        let payload = BASE64.encode(b"not a certificate");
        let error = decode_certificate_payload(&payload)
            .expect_err("invalid certificate should be rejected");
        assert_eq!(error.code, "invalid_ca_certificate");
    }

    #[tokio::test]
    async fn pairs_only_after_identity_and_scope_validation() {
        let token = format!("rmm_{}", "a".repeat(64));
        let scopes = REQUIRED_SCOPES.to_vec();
        let base_url = mock_json_server(vec![
            (200, json!({ "info": { "version": "5.0.0" } }).to_string()),
            (
                200,
                json!({ "id": 42, "raw_token": token, "scopes": scopes }).to_string(),
            ),
            (
                200,
                json!({ "id": 7, "username": "justin", "oauth_scopes": REQUIRED_SCOPES })
                    .to_string(),
            ),
        ])
        .await;
        let mut session = RommSession::new();
        session
            .probe(&base_url, true)
            .await
            .expect("server should probe");
        let auth = session
            .exchange_pairing_code("JM38-MHSA")
            .await
            .expect("pairing should validate");
        assert_eq!(auth.token_id, 42);
        assert_eq!(auth.account_name, "justin");
        assert!(session.is_authenticated());
    }

    #[tokio::test]
    async fn rejects_pairing_before_identity_when_scopes_are_missing() {
        let token = format!("rmm_{}", "b".repeat(64));
        let base_url = mock_json_server(vec![
            (200, json!({ "info": { "version": "5.0.0" } }).to_string()),
            (
                200,
                json!({ "id": 43, "raw_token": token, "scopes": ["me.read", "roms.read"] })
                    .to_string(),
            ),
        ])
        .await;
        let mut session = RommSession::new();
        session
            .probe(&base_url, true)
            .await
            .expect("server should probe");
        let error = session
            .exchange_pairing_code("JM38-MHSA")
            .await
            .expect_err("missing scopes should block authentication");
        assert_eq!(error.code, "missing_required_scopes");
        assert!(!session.is_authenticated());
    }

    #[tokio::test]
    async fn classifies_expired_pairing_codes_before_parsing_the_body() {
        let base_url = mock_json_server(vec![
            (200, json!({ "info": { "version": "5.0.0" } }).to_string()),
            (404, String::new()),
        ])
        .await;
        let mut session = RommSession::new();
        session
            .probe(&base_url, true)
            .await
            .expect("server should probe");
        let error = session
            .exchange_pairing_code("JM38-MHSA")
            .await
            .expect_err("expired code should fail");
        assert_eq!(error.code, "expired_pairing_code");
    }

    #[tokio::test]
    async fn rejects_openapi_documents_without_a_romm_5_version() {
        let base_url = mock_json_server(vec![(200, json!({ "info": {} }).to_string())]).await;
        let result = RommSession::new()
            .probe(&base_url, true)
            .await
            .expect("OpenAPI response should parse");
        assert_eq!(result.server_version, "unknown");
        assert!(!result.compatible);
    }

    #[tokio::test]
    async fn registers_a_device_with_auth_and_the_safe_romm_payload() {
        let (base_url, request_rx) = mock_captured_json_server(
            201,
            json!({
                "device_id": "device-123",
                "name": "Living Room PC",
                "created_at": "2026-08-27T12:00:00Z"
            })
            .to_string(),
        )
        .await;
        let mut session = authenticated_session(&base_url);
        let proposed = device::propose_device_identity("0.1.0");
        let proposed = device::with_display_name(&proposed, "Living Room PC")
            .expect("device name should be valid");

        let registration = session
            .register_device(&proposed)
            .await
            .expect("device should register");
        let request = request_rx.await.expect("request should be captured");
        let request_lower = request.to_ascii_lowercase();
        let body = request
            .split_once("\r\n\r\n")
            .map(|(_, body)| body)
            .expect("request should contain a body");
        let payload: Value = serde_json::from_str(body).expect("request body should be JSON");

        assert!(request.starts_with("POST /api/devices HTTP/1.1"));
        assert!(request_lower.contains("authorization: bearer rmm_"));
        assert_eq!(payload["name"], "Living Room PC");
        assert_eq!(payload["sync_mode"], "push_pull");
        assert_eq!(payload["allow_existing"], true);
        assert_eq!(payload["allow_duplicate"], false);
        assert!(payload.get("ip_address").is_none());
        assert!(payload.get("mac_address").is_none());
        assert_eq!(registration.device_id, "device-123");
        assert!(registration.newly_registered);
        assert_eq!(registration.registration_fingerprint.len(), 64);
        assert!(session.last_contact_at_ms().is_some());
    }

    #[tokio::test]
    async fn registration_accepts_an_existing_server_device_without_duplication() {
        let base_url = mock_json_server(vec![(
            200,
            json!({
                "device_id": "existing-device",
                "name": "Existing Name",
                "created_at": "2026-08-20T12:00:00Z"
            })
            .to_string(),
        )])
        .await;
        let mut session = authenticated_session(&base_url);
        let device = device::propose_device_identity("0.1.0");

        let registration = session
            .register_device(&device)
            .await
            .expect("existing device should be returned");
        assert_eq!(registration.device_id, "existing-device");
        assert_eq!(registration.display_name.as_deref(), Some("Existing Name"));
        assert!(!registration.newly_registered);
    }

    #[tokio::test]
    async fn registration_failure_keeps_the_authenticated_session_and_draft_unchanged() {
        let base_url =
            mock_json_server(vec![(500, json!({ "detail": "later" }).to_string())]).await;
        let mut session = authenticated_session(&base_url);
        let device = device::propose_device_identity("0.1.0");
        let original = device.clone();

        let error = session
            .register_device(&device)
            .await
            .expect_err("server failure should not register");
        assert_eq!(error.code, "server_error");
        assert!(error.retryable);
        assert!(session.is_authenticated());
        assert_eq!(device, original);
    }

    #[tokio::test]
    async fn registration_403_identifies_the_required_device_scope() {
        let base_url =
            mock_json_server(vec![(403, json!({ "detail": "forbidden" }).to_string())]).await;
        let mut session = authenticated_session(&base_url);
        let device = device::propose_device_identity("0.1.0");

        let error = session
            .register_device(&device)
            .await
            .expect_err("permission failure should be reported");
        assert_eq!(error.code, "missing_required_scopes");
        assert_eq!(
            error.details.as_deref(),
            Some(&json!({ "missingScopes": ["devices.write"] }))
        );
        assert!(session.is_authenticated());
    }

    #[tokio::test]
    async fn verifies_the_saved_device_for_the_authenticated_owner() {
        let (base_url, request_rx) = mock_captured_json_server(
            200,
            json!({
                "id": "device-123",
                "user_id": 7,
                "name": "Living Room PC",
                "platform": "windows",
                "client": "romm-companion",
                "client_version": "0.1.0",
                "hostname": "JUSTIN-DESKTOP",
                "sync_mode": "push_pull",
                "sync_enabled": true,
                "sync_config": {},
                "last_seen": "2026-08-28T12:00:00Z",
                "created_at": "2026-08-27T12:00:00Z",
                "updated_at": "2026-08-28T12:00:00Z"
            })
            .to_string(),
        )
        .await;
        let mut session = authenticated_session(&base_url);

        let remote = session
            .verify_device("device-123")
            .await
            .expect("device lookup should succeed")
            .expect("device should exist");
        let request = request_rx.await.expect("request should be captured");

        assert!(request.starts_with("GET /api/devices/device-123 HTTP/1.1"));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("authorization: bearer rmm_")
        );
        assert_eq!(remote.user_id, 7);
        assert!(session.last_contact_at_ms().is_some());
    }

    #[tokio::test]
    async fn device_verification_distinguishes_missing_and_permission_failures() {
        let base_url = mock_json_server(vec![
            (404, json!({ "detail": "not found" }).to_string()),
            (403, json!({ "detail": "forbidden" }).to_string()),
        ])
        .await;
        let mut session = authenticated_session(&base_url);

        assert_eq!(
            session
                .verify_device("deleted-device")
                .await
                .expect("404 is a valid missing result"),
            None
        );
        let error = session
            .verify_device("device-123")
            .await
            .expect_err("403 should identify permission loss");
        assert_eq!(error.code, "missing_required_scopes");
        assert_eq!(
            error.details.as_deref(),
            Some(&json!({ "missingScopes": ["devices.read"] }))
        );
    }

    #[tokio::test]
    async fn device_verification_rejects_an_owner_mismatch() {
        let base_url = mock_json_server(vec![(
            200,
            json!({
                "id": "device-123",
                "user_id": 99,
                "name": "Other Device",
                "platform": "windows",
                "client": "romm-companion",
                "client_version": "0.1.0",
                "hostname": "OTHER-PC",
                "sync_mode": "push_pull",
                "sync_enabled": true,
                "sync_config": {},
                "last_seen": null,
                "created_at": "2026-08-27T12:00:00Z",
                "updated_at": "2026-08-28T12:00:00Z"
            })
            .to_string(),
        )])
        .await;
        let mut session = authenticated_session(&base_url);

        let error = session
            .verify_device("device-123")
            .await
            .expect_err("another owner's device must be rejected");
        assert_eq!(error.code, "device_owner_mismatch");
    }

    #[tokio::test]
    async fn updates_a_device_with_the_safe_authoritative_payload() {
        let (base_url, request_rx) = mock_captured_json_server(
            200,
            json!({
                "id": "device-123",
                "user_id": 7,
                "name": "Arcade Room",
                "platform": "windows",
                "client": "romm-companion",
                "client_version": "0.1.0",
                "hostname": "JUSTIN-DESKTOP",
                "sync_mode": "push_pull",
                "sync_enabled": true,
                "sync_config": {"snes": {"romRoot": "D:/ROMs/SNES"}},
                "last_seen": null,
                "created_at": "2026-08-27T12:00:00Z",
                "updated_at": "2026-08-28T12:00:00Z"
            })
            .to_string(),
        )
        .await;
        let mut session = authenticated_session(&base_url);
        let mut device = device::propose_device_identity("0.1.0");
        device.romm_device_id = Some("device-123".to_owned());
        device.display_name = "Arcade Room".to_owned();
        device
            .mapping_summary
            .insert("snes".to_owned(), json!({"romRoot": "D:/ROMs/SNES"}));

        let remote = session
            .update_device(&device)
            .await
            .expect("device update should succeed")
            .expect("updated device should exist");
        let request = request_rx.await.expect("request should be captured");
        let body = request
            .split_once("\r\n\r\n")
            .map(|(_, body)| body)
            .expect("request should contain a body");
        let payload: Value = serde_json::from_str(body).expect("request body should be JSON");

        assert!(request.starts_with("PUT /api/devices/device-123 HTTP/1.1"));
        assert_eq!(payload["name"], "Arcade Room");
        assert_eq!(
            payload["sync_config"],
            serde_json::to_value(&device.mapping_summary)
                .expect("mapping summary should serialize")
        );
        assert!(payload.get("ip_address").is_none());
        assert!(payload.get("mac_address").is_none());
        assert_eq!(remote.name.as_deref(), Some("Arcade Room"));
    }

    #[tokio::test]
    async fn device_removal_distinguishes_deleted_and_already_missing() {
        let base_url = mock_json_server(vec![
            (204, String::new()),
            (404, json!({ "detail": "not found" }).to_string()),
        ])
        .await;
        let mut session = authenticated_session(&base_url);

        assert!(
            session
                .delete_device("device-123")
                .await
                .expect("204 should report a deletion")
        );
        assert!(
            !session
                .delete_device("device-123")
                .await
                .expect("404 should be an idempotent missing result")
        );
    }
}
