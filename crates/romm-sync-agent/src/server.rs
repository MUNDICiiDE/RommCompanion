use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use romm_core::{
    RommSession, ValidatedAuth,
    credentials::SystemCredentialStore,
    decode_certificate_payload,
    device::{
        apply_registration, apply_verification, clear_remote_registration, mark_device_missing,
        mark_device_permission_error, propose_device_identity, with_display_name,
    },
    mapping::{
        detect_platform_mappings, validate_mapping_drafts, validate_mapping_drafts_for_review,
    },
    normalize_base_url,
    onboarding::{
        complete_onboarding_step, invalidate_authenticated_onboarding,
        invalidate_device_onboarding, navigate_onboarding, restore_onboarding_state,
        select_onboarding_server, validate_onboarding_state,
    },
    server_origin,
    storage::{AgentLock, AppPaths, Database, StoredServerProfile},
};
use romm_ipc::{
    AgentRequest, AgentResponse, AgentStatus, AppError, AppSettings, AuthResult,
    BackgroundSetupResult, CaImportResult, ConnectionState, DeviceRegistrationState,
    DeviceRemovalOutcome, DeviceRemovalResult, DeviceVerificationOutcome, IPC_SCHEMA_VERSION,
    InitialRefreshResult, OnboardingState, OnboardingStep, PlatformMappingDraft, REQUIRED_SCOPES,
    RequestEnvelope, ResponseEnvelope, local_ipc_endpoint,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader},
    sync::{Mutex as AsyncMutex, RwLock, watch},
};

mod background;
mod logging;

const INITIAL_LIBRARY_PAGE_SIZE: u16 = 48;

struct AgentState {
    session: RwLock<RommSession>,
    connection: RwLock<ConnectionState>,
    database: Mutex<Database>,
    credentials: SystemCredentialStore,
    paths: AppPaths,
    device_registration: AsyncMutex<()>,
    library_request: AsyncMutex<()>,
    onboarding: AsyncMutex<OnboardingState>,
    background_configurer: fn(bool) -> Result<background::RegistrationOutcome, String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let paths = AppPaths::resolve().context("failed to resolve application directories")?;
    paths
        .prepare()
        .context("failed to prepare application directories")?;
    logging::initialize(&paths);
    logging::info("agent_starting", "RomM Companion agent is starting");
    let _agent_lock = AgentLock::acquire(&paths).context("failed to acquire the agent lock")?;
    let database =
        Database::open(&paths).context("failed to initialize the application database")?;
    let onboarding = database
        .load_onboarding_state()
        .ok()
        .flatten()
        .filter(|state| validate_onboarding_state(state).is_ok())
        .unwrap_or_default();
    let credentials = SystemCredentialStore;
    let stored_profile = database
        .load_server_profile()
        .context("failed to restore the server profile")?;
    let mut session = stored_profile
        .as_ref()
        .and_then(|profile| profile.ca_id.as_deref())
        .and_then(|ca_id| {
            paths
                .load_ca_certificate(ca_id)
                .ok()
                .map(|bytes| (ca_id, bytes))
        })
        .and_then(|(ca_id, bytes)| RommSession::with_certificate(&bytes, ca_id.to_owned()).ok())
        .unwrap_or_default();
    let mut initial_connection = ConnectionState::Unconfigured;

    if let Some(profile) = stored_profile {
        let restored = profile
            .credential_locator
            .as_deref()
            .and_then(|locator| credentials.load(locator).ok());
        if let Some(token) = restored {
            let _ = session.restore(&profile.base_url, profile.server_version.clone(), &token);
            session.restore_metadata(
                profile.token_id,
                profile.account_id,
                profile.account_name,
                profile.granted_scopes,
                profile.last_contact_at_ms,
                profile.http_approved,
                profile.ca_id,
            );
            initial_connection = ConnectionState::Offline;
            logging::info("session_restored", "Restored the saved RomM session");
        } else {
            let _ = session.restore_configuration(&profile.base_url, profile.server_version);
            initial_connection = ConnectionState::Pairing;
            logging::info("profile_restored", "Restored the saved RomM server profile");
        }
    }

    let state = Arc::new(AgentState {
        session: RwLock::new(session),
        connection: RwLock::new(initial_connection),
        database: Mutex::new(database),
        credentials,
        paths,
        device_registration: AsyncMutex::new(()),
        library_request: AsyncMutex::new(()),
        onboarding: AsyncMutex::new(onboarding),
        background_configurer: background::configure,
    });
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    tokio::spawn(checkpoint_daily(Arc::clone(&state), shutdown_rx.clone()));
    let result = serve(Arc::clone(&state), shutdown_tx, shutdown_rx).await;
    if let Ok(database) = state.database.lock() {
        let _ = database.checkpoint();
    }
    logging::info("agent_stopped", "RomM Companion agent stopped cleanly");
    result
}

async fn checkpoint_daily(state: Arc<AgentState>, mut shutdown: watch::Receiver<bool>) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(24 * 60 * 60));
    interval.tick().await;
    loop {
        tokio::select! {
            _ = interval.tick() => {
                if let Ok(database) = state.database.lock()
                    && let Err(error) = database.checkpoint()
                {
                    logging::error("database_checkpoint_failed", &error.to_string());
                }
            }
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
        }
    }
}

#[cfg(windows)]
async fn serve(
    state: Arc<AgentState>,
    shutdown_tx: watch::Sender<bool>,
    mut shutdown_rx: watch::Receiver<bool>,
) -> Result<()> {
    use tokio::net::windows::named_pipe::ServerOptions;

    let endpoint = local_ipc_endpoint().map_err(anyhow::Error::msg)?;
    let mut first_instance = true;
    loop {
        let server = ServerOptions::new()
            .first_pipe_instance(first_instance)
            .create(&endpoint)
            .with_context(|| format!("failed to create local agent pipe {endpoint}"))?;
        first_instance = false;
        tokio::select! {
            connected = server.connect() => {
                connected.context("failed to accept a local pipe client")?;
                let state = Arc::clone(&state);
                let shutdown_tx = shutdown_tx.clone();
                tokio::spawn(async move {
                    if let Err(error) = handle_connection(server, state, shutdown_tx).await {
                        logging::error("ipc_request_failed", &error.to_string());
                    }
                });
            }
            changed = shutdown_rx.changed() => {
                if changed.is_err() || *shutdown_rx.borrow() {
                    break;
                }
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
async fn serve(
    state: Arc<AgentState>,
    shutdown_tx: watch::Sender<bool>,
    mut shutdown_rx: watch::Receiver<bool>,
) -> Result<()> {
    use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};
    use tokio::net::{UnixListener, UnixStream};

    struct SocketCleanup(PathBuf);
    impl Drop for SocketCleanup {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    let endpoint = local_ipc_endpoint().map_err(anyhow::Error::msg)?;
    if endpoint.exists() {
        if UnixStream::connect(&endpoint).await.is_ok() {
            anyhow::bail!("another RomM Companion agent is already listening");
        }
        fs::remove_file(&endpoint).context("failed to remove a stale agent socket")?;
    }
    let listener = UnixListener::bind(&endpoint).with_context(|| {
        format!(
            "failed to bind local agent socket at {}",
            endpoint.display()
        )
    })?;
    fs::set_permissions(&endpoint, fs::Permissions::from_mode(0o600))
        .context("failed to restrict agent socket permissions")?;
    let _cleanup = SocketCleanup(endpoint);

    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted.context("failed to accept a local socket client")?;
                let state = Arc::clone(&state);
                let shutdown_tx = shutdown_tx.clone();
                tokio::spawn(async move {
                    if let Err(error) = handle_connection(stream, state, shutdown_tx).await {
                        logging::error("ipc_request_failed", &error.to_string());
                    }
                });
            }
            changed = shutdown_rx.changed() => {
                if changed.is_err() || *shutdown_rx.borrow() {
                    break;
                }
            }
        }
    }
    Ok(())
}

async fn handle_connection<S>(
    stream: S,
    state: Arc<AgentState>,
    shutdown_tx: watch::Sender<bool>,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (reader, mut writer) = tokio::io::split(stream);
    let mut lines = BufReader::new(reader).lines();
    let Some(line) = lines.next_line().await? else {
        return Ok(());
    };
    let raw_request: serde_json::Value =
        serde_json::from_str(&line).context("invalid request envelope")?;
    let request_id = raw_request
        .get("requestId")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown-request")
        .to_owned();
    let request_schema = raw_request
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_default();
    if request_schema != u64::from(IPC_SCHEMA_VERSION) {
        let response = ResponseEnvelope {
            schema_version: IPC_SCHEMA_VERSION,
            request_id,
            body: AgentResponse::Error {
                error: AppError::new(
                    "agent_upgrade_required",
                    "The desktop app and agent use different IPC versions. Restart or reinstall RomM Companion.",
                    false,
                ),
            },
        };
        writer
            .write_all(format!("{}\n", serde_json::to_string(&response)?).as_bytes())
            .await?;
        writer.shutdown().await?;
        return Ok(());
    }
    let request: RequestEnvelope =
        serde_json::from_value(raw_request).context("invalid request body")?;

    let shutdown_requested = matches!(&request.body, AgentRequest::Shutdown);
    let body = if shutdown_requested {
        AgentResponse::ShuttingDown
    } else {
        dispatch(request.body, state).await
    };
    let response = ResponseEnvelope {
        schema_version: IPC_SCHEMA_VERSION,
        request_id,
        body,
    };
    if shutdown_requested {
        let _ = shutdown_tx.send(true);
    }
    writer
        .write_all(format!("{}\n", serde_json::to_string(&response)?).as_bytes())
        .await?;
    writer.shutdown().await?;
    Ok(())
}

async fn dispatch(request: AgentRequest, state: Arc<AgentState>) -> AgentResponse {
    match request {
        AgentRequest::GetStatus => {
            let session = state.session.read().await;
            let connection = state.connection.read().await.clone();
            let device = state
                .database
                .lock()
                .ok()
                .and_then(|database| database.load_device().ok().flatten());
            AgentResponse::Status {
                status: Box::new(AgentStatus {
                    version: env!("CARGO_PKG_VERSION").to_owned(),
                    ipc_schema_version: IPC_SCHEMA_VERSION,
                    connection,
                    server_url: session.base_url(),
                    server_origin: session.server_origin(),
                    server_version: session.server_version(),
                    account_id: session.account_id(),
                    account_name: session.account_name(),
                    last_contact_at_ms: session.last_contact_at_ms(),
                    granted_scopes: session.granted_scopes(),
                    missing_scopes: session.missing_scopes(),
                    token_id: session.token_id(),
                    credential_configured: session.credential().is_some(),
                    ca_id: session.ca_id(),
                    http_approved: session.http_approved(),
                    device,
                }),
            }
        }
        AgentRequest::GetSettings => {
            let settings = state.database.lock().ok().and_then(|database| {
                database
                    .get_app_state("desktop_settings")
                    .ok()
                    .flatten()
                    .and_then(|value| serde_json::from_str::<AppSettings>(&value).ok())
            });
            AgentResponse::Settings { settings }
        }
        AgentRequest::GetOnboarding => match reconcile_onboarding_progress(&state).await {
            Ok(onboarding) => AgentResponse::Onboarding { state: onboarding },
            Err(error) => AgentResponse::Error { error },
        },
        AgentRequest::UpdateOnboarding { action } => {
            let mut onboarding = state.onboarding.lock().await;
            let next = navigate_onboarding(&onboarding, action);
            let saved = state
                .database
                .lock()
                .ok()
                .and_then(|database| database.save_onboarding_state(&next).ok());
            if saved.is_none() {
                return foundation_error(
                    "onboarding_save_failed",
                    "Onboarding progress could not be saved.",
                    true,
                );
            }
            *onboarding = next.clone();
            AgentResponse::Onboarding { state: next }
        }
        AgentRequest::UpdateSettings { settings } => {
            if let Err(message) = settings.validate() {
                return foundation_error("invalid_settings", &message, false);
            }
            if save_app_settings(&state, &settings).is_ok() {
                AgentResponse::Settings {
                    settings: Some(settings),
                }
            } else {
                foundation_error(
                    "settings_save_failed",
                    "Desktop settings could not be saved.",
                    true,
                )
            }
        }
        AgentRequest::Probe {
            base_url,
            confirm_http,
            ca_id,
        } => {
            *state.connection.write().await = ConnectionState::Probing;
            let requested_url = match normalize_base_url(&base_url) {
                Ok(url) => url,
                Err(error) => return connection_error(&state, error).await,
            };
            let mut candidate = if let Some(ca_id) = ca_id.as_deref() {
                let assigned = state.database.lock().ok().and_then(|database| {
                    database
                        .ca_is_assigned_to(ca_id, &server_origin(&requested_url))
                        .ok()
                });
                if assigned != Some(true) {
                    return connection_error(
                        &state,
                        AppError::new(
                            "ca_origin_mismatch",
                            "The imported certificate authority belongs to a different RomM server.",
                            false,
                        ),
                    )
                    .await;
                }
                match state.paths.load_ca_certificate(ca_id) {
                    Ok(bytes) => match RommSession::with_certificate(&bytes, ca_id.to_owned()) {
                        Ok(session) => session,
                        Err(error) => return connection_error(&state, error).await,
                    },
                    Err(_) => {
                        return connection_error(
                            &state,
                            AppError::new(
                                "ca_certificate_missing",
                                "The imported certificate authority is no longer available.",
                                false,
                            ),
                        )
                        .await;
                    }
                }
            } else {
                RommSession::new()
            };
            let result = candidate.probe(&base_url, confirm_http).await;
            match result {
                Ok(result) => {
                    if !result.compatible {
                        *state.connection.write().await = ConnectionState::Incompatible;
                        return AgentResponse::Probe { result };
                    }
                    if let Err(error) = record_onboarding_server(
                        &state,
                        &server_origin(&requested_url),
                        candidate.http_approved(),
                    )
                    .await
                    {
                        return AgentResponse::Error { error };
                    }
                    // The candidate remains in memory until authentication succeeds. This keeps a
                    // previously working durable profile intact when replacement pairing fails.
                    *state.session.write().await = candidate;
                    *state.connection.write().await = ConnectionState::Pairing;
                    AgentResponse::Probe { result }
                }
                Err(error) => connection_error(&state, error).await,
            }
        }
        AgentRequest::ImportCa {
            base_url,
            certificate,
        } => {
            let url = match normalize_base_url(&base_url) {
                Ok(url) => url,
                Err(error) => return AgentResponse::Error { error },
            };
            if url.scheme() != "https" {
                return foundation_error(
                    "ca_requires_https",
                    "A custom certificate authority can only be assigned to an HTTPS server.",
                    false,
                );
            }
            let bytes = match decode_certificate_payload(&certificate) {
                Ok(bytes) => bytes,
                Err(error) => return AgentResponse::Error { error },
            };
            let ca_id = match state.paths.save_ca_certificate(&bytes) {
                Ok(id) => id,
                Err(error) => {
                    return foundation_error(
                        "ca_save_failed",
                        &format!("The certificate authority could not be saved: {error}"),
                        true,
                    );
                }
            };
            let origin = server_origin(&url);
            let assignment_saved = state
                .database
                .lock()
                .ok()
                .and_then(|database| database.save_ca_assignment(&ca_id, &origin).ok());
            if assignment_saved.is_none() {
                return foundation_error(
                    "ca_save_failed",
                    "The certificate authority was parsed but its server assignment could not be saved.",
                    true,
                );
            }
            AgentResponse::CaImported {
                result: CaImportResult {
                    fingerprint: ca_id.clone(),
                    ca_id,
                    server_origin: origin,
                },
            }
        }
        AgentRequest::ExchangePairingCode { code } => {
            *state.connection.write().await = ConnectionState::Pairing;
            let token = state
                .session
                .write()
                .await
                .exchange_pairing_code(&code)
                .await;
            match token {
                Ok(auth) => persist_authentication(&state, &auth, true).await,
                Err(error) => connection_error(&state, error).await,
            }
        }
        AgentRequest::SetManualToken { token } => {
            *state.connection.write().await = ConnectionState::Pairing;
            let result = state
                .session
                .write()
                .await
                .authenticate_manual_token(&token)
                .await;
            match result {
                Ok(auth) => persist_authentication(&state, &auth, true).await,
                Err(error) => connection_error(&state, error).await,
            }
        }
        AgentRequest::Reconnect => {
            *state.connection.write().await = ConnectionState::Probing;
            let result = state.session.write().await.reconnect().await;
            match result {
                Ok(auth) => persist_authentication(&state, &auth, false).await,
                Err(error) => connection_error(&state, error).await,
            }
        }
        AgentRequest::ProposeDevice => {
            let existing = state
                .database
                .lock()
                .ok()
                .and_then(|database| database.load_device().ok().flatten());
            if let Some(device) = existing {
                AgentResponse::DeviceProposed { device }
            } else {
                let device = propose_device_identity(env!("CARGO_PKG_VERSION"));
                let saved = state
                    .database
                    .lock()
                    .ok()
                    .and_then(|database| database.save_device(&device).ok());
                if saved.is_some() {
                    AgentResponse::DeviceProposed { device }
                } else {
                    foundation_error(
                        "device_save_failed",
                        "The local device identity could not be saved.",
                        true,
                    )
                }
            }
        }
        AgentRequest::RegisterDevice { display_name } => {
            let _registration_guard = state.device_registration.lock().await;
            let existing = state
                .database
                .lock()
                .ok()
                .and_then(|database| database.load_device().ok().flatten());
            let device =
                existing.unwrap_or_else(|| propose_device_identity(env!("CARGO_PKG_VERSION")));
            if device.romm_device_id.is_some()
                && device.registration_state == DeviceRegistrationState::Registered
            {
                record_onboarding_step_best_effort(&state, OnboardingStep::Device).await;
                return AgentResponse::DeviceRegistered {
                    device,
                    newly_registered: false,
                };
            }
            if device.registration_state == DeviceRegistrationState::PermissionError {
                return foundation_error(
                    "device_verification_required",
                    "Restore the devices.read permission and verify this device before changing its registration.",
                    false,
                );
            }

            let draft = match with_display_name(&device, &display_name) {
                Ok(device) => device,
                Err(error) => return AgentResponse::Error { error },
            };
            let draft_saved = state
                .database
                .lock()
                .ok()
                .and_then(|database| database.save_device(&draft).ok());
            if draft_saved.is_none() {
                return foundation_error(
                    "device_save_failed",
                    "The device name could not be saved before registration.",
                    true,
                );
            }

            let result = state.session.write().await.register_device(&draft).await;
            match result {
                Ok(result) => {
                    let newly_registered = result.newly_registered;
                    let registered = apply_registration(&draft, &result);
                    let saved = state
                        .database
                        .lock()
                        .ok()
                        .and_then(|database| database.save_device(&registered).ok());
                    if saved.is_none() {
                        return foundation_error(
                            "device_save_failed",
                            "RomM registered the device, but its device ID could not be saved. Retry to recover the existing registration.",
                            true,
                        );
                    }
                    *state.connection.write().await = ConnectionState::Connected;
                    record_onboarding_step_best_effort(&state, OnboardingStep::Device).await;
                    AgentResponse::DeviceRegistered {
                        device: registered,
                        newly_registered,
                    }
                }
                Err(error) => connection_error(&state, error).await,
            }
        }
        AgentRequest::UpdateDevice { display_name } => {
            let _registration_guard = state.device_registration.lock().await;
            let existing = state
                .database
                .lock()
                .ok()
                .and_then(|database| database.load_device().ok().flatten());
            let Some(existing) = existing else {
                return foundation_error(
                    "device_not_configured",
                    "Register this installation before changing its device settings.",
                    false,
                );
            };
            if existing.registration_state != DeviceRegistrationState::Registered
                || existing.romm_device_id.is_none()
            {
                return foundation_error(
                    "device_not_registered",
                    "Restore this device registration before changing its settings.",
                    false,
                );
            }
            let draft = match with_display_name(&existing, &display_name) {
                Ok(device) => device,
                Err(error) => return AgentResponse::Error { error },
            };

            match state.session.write().await.update_device(&draft).await {
                Ok(Some(remote)) => {
                    let updated = apply_verification(&draft, &remote);
                    let saved = state
                        .database
                        .lock()
                        .ok()
                        .and_then(|database| database.save_device(&updated).ok());
                    if saved.is_none() {
                        return foundation_error(
                            "device_save_failed",
                            "RomM updated the device, but the new settings could not be saved locally.",
                            true,
                        );
                    }
                    *state.connection.write().await = ConnectionState::Connected;
                    AgentResponse::DeviceUpdated { device: updated }
                }
                Ok(None) => {
                    let missing = mark_device_missing(&existing);
                    let _ = state
                        .database
                        .lock()
                        .ok()
                        .and_then(|database| database.save_device(&missing).ok());
                    foundation_error(
                        "device_missing",
                        "This device was removed from RomM. Register it again before changing its settings.",
                        false,
                    )
                }
                Err(error) => connection_error(&state, error).await,
            }
        }
        AgentRequest::VerifyDevice => {
            let _registration_guard = state.device_registration.lock().await;
            let device = state
                .database
                .lock()
                .ok()
                .and_then(|database| database.load_device().ok().flatten());
            let Some(device) = device else {
                return foundation_error(
                    "device_not_configured",
                    "Register this installation before verifying it with RomM.",
                    false,
                );
            };
            let Some(device_id) = device.romm_device_id.as_deref() else {
                return foundation_error(
                    "device_not_registered",
                    "Register this installation before verifying it with RomM.",
                    false,
                );
            };

            let result = state.session.write().await.verify_device(device_id).await;
            let (verified, outcome, connection) = match result {
                Ok(Some(remote)) => (
                    apply_verification(&device, &remote),
                    DeviceVerificationOutcome::Verified,
                    ConnectionState::Connected,
                ),
                Ok(None) => (
                    mark_device_missing(&device),
                    DeviceVerificationOutcome::Missing,
                    ConnectionState::Connected,
                ),
                Err(error)
                    if matches!(error.code.as_str(), "forbidden" | "missing_required_scopes") =>
                {
                    (
                        mark_device_permission_error(&device),
                        DeviceVerificationOutcome::PermissionDenied,
                        ConnectionState::ScopeError,
                    )
                }
                Err(error) => return connection_error(&state, error).await,
            };
            let saved = state
                .database
                .lock()
                .ok()
                .and_then(|database| database.save_device(&verified).ok());
            if saved.is_none() {
                return foundation_error(
                    "device_save_failed",
                    "The device verification result could not be saved locally.",
                    true,
                );
            }
            *state.connection.write().await = connection;
            match &outcome {
                DeviceVerificationOutcome::Verified => {
                    record_onboarding_step_best_effort(&state, OnboardingStep::Device).await;
                }
                DeviceVerificationOutcome::Missing => {
                    update_onboarding_best_effort(&state, invalidate_device_onboarding).await;
                }
                DeviceVerificationOutcome::PermissionDenied => {}
            }
            AgentResponse::DeviceVerification {
                device: verified,
                outcome,
            }
        }
        AgentRequest::DetectMappings => {
            let result = {
                let session = state.session.read().await;
                session.list_platforms().await
            };
            match result {
                Ok(platforms) => {
                    state.session.write().await.mark_contact();
                    *state.connection.write().await = ConnectionState::Connected;
                    AgentResponse::MappingDetection {
                        result: detect_platform_mappings(&platforms),
                    }
                }
                Err(error) => connection_error(&state, error).await,
            }
        }
        AgentRequest::GetMappingDrafts => {
            let Some(origin) = state.session.read().await.server_origin() else {
                return foundation_error(
                    "server_not_configured",
                    "Connect to RomM before loading mapping drafts.",
                    false,
                );
            };
            match state
                .database
                .lock()
                .ok()
                .and_then(|database| database.load_mapping_drafts(&origin).ok())
            {
                Some(drafts) => AgentResponse::MappingDrafts { drafts },
                None => foundation_error(
                    "mapping_drafts_load_failed",
                    "Saved mapping drafts could not be loaded.",
                    true,
                ),
            }
        }
        AgentRequest::SaveMappingDrafts { drafts } => {
            let validation = validate_mapping_drafts_for_review(&drafts);
            if !validation.valid {
                return AgentResponse::MappingValidation { result: validation };
            }
            let Some(origin) = state.session.read().await.server_origin() else {
                return foundation_error(
                    "server_not_configured",
                    "Connect to RomM before saving mapping drafts.",
                    false,
                );
            };
            let saved = state
                .database
                .lock()
                .ok()
                .and_then(|database| database.save_mapping_drafts(&origin, &drafts).ok());
            if saved.is_none() {
                return foundation_error(
                    "mapping_drafts_save_failed",
                    "Mapping drafts could not be saved.",
                    true,
                );
            }
            if let Err(error) = advance_mapping_onboarding(&state, &drafts, false).await {
                return AgentResponse::Error { error };
            }
            AgentResponse::MappingDrafts { drafts }
        }
        AgentRequest::ValidateMappings {
            drafts,
            no_platforms,
        } => AgentResponse::MappingValidation {
            result: validate_mapping_drafts(&drafts, no_platforms),
        },
        AgentRequest::SaveMappings {
            drafts,
            no_platforms,
        } => {
            let validation = validate_mapping_drafts(&drafts, no_platforms);
            if !validation.valid {
                return AgentResponse::MappingValidation { result: validation };
            }
            let Some(origin) = state.session.read().await.server_origin() else {
                return foundation_error(
                    "server_not_configured",
                    "Connect to RomM before saving mappings.",
                    false,
                );
            };
            let saved = state.database.lock().ok().and_then(|database| {
                database
                    .save_platform_mappings(&origin, &drafts, no_platforms)
                    .ok()
            });
            if saved.is_none() {
                return foundation_error(
                    "mappings_save_failed",
                    "Platform mappings could not be saved.",
                    true,
                );
            }
            match advance_mapping_onboarding(&state, &drafts, true).await {
                Ok(onboarding) => AgentResponse::MappingsSaved {
                    mappings: drafts.into_iter().filter(|draft| draft.enabled).collect(),
                    no_platforms,
                    onboarding,
                },
                Err(error) => AgentResponse::Error { error },
            }
        }
        AgentRequest::ConfigureOnboardingPreferences {
            background_enabled,
            stable_update_checks_enabled,
        } => {
            let mut settings = load_app_settings(&state);
            settings.stable_update_checks_enabled = stable_update_checks_enabled;
            if let Err(error) = save_app_settings(&state, &settings) {
                return AgentResponse::Error { error };
            }

            let configure_background = state.background_configurer;
            let registration =
                tokio::task::spawn_blocking(move || configure_background(background_enabled))
                    .await
                    .map_err(|error| format!("Background setup task failed: {error}"))
                    .and_then(|result| result);
            let mut onboarding = state.onboarding.lock().await;
            let (applied_background, registration_method, warning) = match registration {
                Ok(outcome) => (outcome.enabled, outcome.method.to_owned(), None),
                Err(message) => (
                    if background_enabled {
                        false
                    } else {
                        onboarding.background_enabled.unwrap_or(true)
                    },
                    background::method_name().to_owned(),
                    Some(message),
                ),
            };
            let mut next = onboarding.clone();
            next.background_enabled = Some(applied_background);
            if warning.is_none() {
                next = match complete_onboarding_step(&next, OnboardingStep::Background) {
                    Ok(next) => next,
                    Err(message) => {
                        return foundation_error("onboarding_progress_invalid", &message, false);
                    }
                };
            }
            if let Err(error) = save_onboarding_progress(&state, &next) {
                return AgentResponse::Error { error };
            }
            *onboarding = next.clone();
            AgentResponse::OnboardingPreferences {
                result: BackgroundSetupResult {
                    requested_background: background_enabled,
                    background_enabled: applied_background,
                    stable_update_checks_enabled,
                    registration_method,
                    warning,
                    onboarding: next,
                },
            }
        }
        AgentRequest::StartInitialRefresh => {
            let _refresh_guard = state.library_request.lock().await;
            let progress_check = {
                let onboarding = state.onboarding.lock().await;
                complete_onboarding_step(&onboarding, OnboardingStep::FirstRefresh)
            };
            if let Err(message) = progress_check {
                return foundation_error("onboarding_progress_invalid", &message, false);
            }
            let Some(origin) = state.session.read().await.server_origin() else {
                return foundation_error(
                    "server_not_configured",
                    "Connect to RomM before refreshing the library.",
                    false,
                );
            };
            let result = {
                let session = state.session.read().await;
                session.list_roms(INITIAL_LIBRARY_PAGE_SIZE, 0).await
            };
            match result {
                Ok(page) => {
                    state.session.write().await.mark_contact();
                    *state.connection.write().await = ConnectionState::Connected;
                    let mut onboarding = state.onboarding.lock().await;
                    let next =
                        match complete_onboarding_step(&onboarding, OnboardingStep::FirstRefresh) {
                            Ok(next) => next,
                            Err(message) => {
                                return foundation_error(
                                    "onboarding_progress_invalid",
                                    &message,
                                    false,
                                );
                            }
                        };
                    let saved = state.database.lock().ok().and_then(|database| {
                        database
                            .save_initial_library_page(&origin, &page, &next)
                            .ok()
                    });
                    if saved.is_none() {
                        return foundation_error(
                            "initial_refresh_save_failed",
                            "The first library page loaded, but it could not be saved safely. Retry to finish setup.",
                            true,
                        );
                    }
                    *onboarding = next.clone();
                    AgentResponse::InitialRefresh {
                        result: InitialRefreshResult {
                            page,
                            onboarding: next,
                        },
                    }
                }
                Err(error) => connection_error(&state, error).await,
            }
        }
        AgentRequest::ListRoms { limit, offset } => {
            let _library_guard = state.library_request.lock().await;
            let limit = limit.clamp(1, 100);
            let Some(origin) = state.session.read().await.server_origin() else {
                return foundation_error(
                    "server_not_configured",
                    "Connect to RomM before loading the library.",
                    false,
                );
            };
            let result = {
                let session = state.session.read().await;
                session.list_roms(limit, offset).await
            };
            match result {
                Ok(page) => {
                    if state
                        .database
                        .lock()
                        .ok()
                        .and_then(|database| database.save_library_page(&origin, &page).ok())
                        .is_none()
                    {
                        return foundation_error(
                            "library_cache_save_failed",
                            "The library page loaded, but its local cache could not be updated.",
                            true,
                        );
                    }
                    state.session.write().await.mark_contact();
                    *state.connection.write().await = ConnectionState::Connected;
                    AgentResponse::Roms { page }
                }
                Err(error) if error.retryable => {
                    let cached = state.database.lock().ok().and_then(|database| {
                        database
                            .load_library_page(&origin, offset, limit)
                            .ok()
                            .flatten()
                    });
                    if let Some(page) = cached {
                        *state.connection.write().await = ConnectionState::Offline;
                        AgentResponse::Roms { page }
                    } else {
                        connection_error(&state, error).await
                    }
                }
                Err(error) => connection_error(&state, error).await,
            }
        }
        AgentRequest::ListLibrary {
            query,
            limit,
            offset,
        } => {
            let _library_guard = state.library_request.lock().await;
            if let Err(message) = query.validate() {
                return foundation_error("invalid_library_query", &message, false);
            }
            let limit = limit.clamp(1, 100);
            let Some(origin) = state.session.read().await.server_origin() else {
                return foundation_error(
                    "server_not_configured",
                    "Connect to RomM before loading the library.",
                    false,
                );
            };
            if query.is_local() {
                return match state.database.lock().ok().and_then(|database| {
                    database
                        .load_local_library_page(&origin, &query, offset, limit)
                        .ok()
                }) {
                    Some(page) => AgentResponse::LibraryPage { query, page },
                    None => foundation_error(
                        "local_library_load_failed",
                        "The local game view could not be loaded.",
                        true,
                    ),
                };
            }
            let result = {
                let session = state.session.read().await;
                session.list_library(&query, limit, offset).await
            };
            match result {
                Ok(page) => {
                    if state
                        .database
                        .lock()
                        .ok()
                        .and_then(|database| {
                            database.save_library_view_page(&origin, &query, &page).ok()
                        })
                        .is_none()
                    {
                        return foundation_error(
                            "library_cache_save_failed",
                            "The library view loaded, but its local cache could not be updated.",
                            true,
                        );
                    }
                    state.session.write().await.mark_contact();
                    *state.connection.write().await = ConnectionState::Connected;
                    AgentResponse::LibraryPage { query, page }
                }
                Err(error) if error.retryable => {
                    let cached = state.database.lock().ok().and_then(|database| {
                        database
                            .load_library_view_page(&origin, &query, offset, limit)
                            .ok()
                            .flatten()
                            .or_else(|| {
                                database
                                    .load_filtered_library_page(&origin, &query, offset, limit)
                                    .ok()
                            })
                    });
                    if let Some(page) = cached {
                        *state.connection.write().await = ConnectionState::Offline;
                        AgentResponse::LibraryPage { query, page }
                    } else {
                        connection_error(&state, error).await
                    }
                }
                Err(error) => connection_error(&state, error).await,
            }
        }
        AgentRequest::GetGameDetails { rom_id } => {
            let _library_guard = state.library_request.lock().await;
            if rom_id < 1 {
                return foundation_error("invalid_rom_id", "ROM id must be positive.", false);
            }
            let Some(origin) = state.session.read().await.server_origin() else {
                return foundation_error(
                    "server_not_configured",
                    "Connect to RomM before loading game details.",
                    false,
                );
            };
            let result = {
                let session = state.session.read().await;
                session.get_game_details(rom_id).await
            };
            match result {
                Ok(details) => {
                    if state
                        .database
                        .lock()
                        .ok()
                        .and_then(|database| database.save_game_details(&origin, &details).ok())
                        .is_none()
                    {
                        return foundation_error(
                            "game_details_cache_save_failed",
                            "Game details loaded, but their offline cache could not be updated.",
                            true,
                        );
                    }
                    state.session.write().await.mark_contact();
                    *state.connection.write().await = ConnectionState::Connected;
                    AgentResponse::GameDetails {
                        details: Box::new(details),
                    }
                }
                Err(error) if error.retryable => {
                    let cached = state.database.lock().ok().and_then(|database| {
                        database.load_game_details(&origin, rom_id).ok().flatten()
                    });
                    if let Some(details) = cached {
                        *state.connection.write().await = ConnectionState::Offline;
                        AgentResponse::GameDetails {
                            details: Box::new(details),
                        }
                    } else {
                        connection_error(&state, error).await
                    }
                }
                Err(error) => connection_error(&state, error).await,
            }
        }
        AgentRequest::GetLibraryMetadata => {
            let _library_guard = state.library_request.lock().await;
            let Some(origin) = state.session.read().await.server_origin() else {
                return foundation_error(
                    "server_not_configured",
                    "Connect to RomM before loading library metadata.",
                    false,
                );
            };
            let result = {
                let session = state.session.read().await;
                session.get_library_metadata().await
            };
            match result {
                Ok(metadata) => {
                    if state
                        .database
                        .lock()
                        .ok()
                        .and_then(|database| {
                            database.save_library_metadata(&origin, &metadata).ok()
                        })
                        .is_none()
                    {
                        return foundation_error(
                            "library_metadata_cache_save_failed",
                            "Library metadata loaded, but its local cache could not be updated.",
                            true,
                        );
                    }
                    state.session.write().await.mark_contact();
                    *state.connection.write().await = ConnectionState::Connected;
                    AgentResponse::LibraryMetadata { metadata }
                }
                Err(error) if error.retryable => {
                    let cached = state.database.lock().ok().and_then(|database| {
                        database.load_library_metadata(&origin).ok().flatten()
                    });
                    if let Some(metadata) = cached {
                        *state.connection.write().await = ConnectionState::Offline;
                        AgentResponse::LibraryMetadata { metadata }
                    } else {
                        connection_error(&state, error).await
                    }
                }
                Err(error) => connection_error(&state, error).await,
            }
        }
        AgentRequest::Logout { remove_device } => {
            let _registration_guard = state.device_registration.lock().await;
            let device = state
                .database
                .lock()
                .ok()
                .and_then(|database| database.load_device().ok().flatten());
            let mut device_removal = DeviceRemovalResult {
                outcome: DeviceRemovalOutcome::NotRequested,
                device_id: device
                    .as_ref()
                    .and_then(|device| device.romm_device_id.clone()),
                error: None,
            };
            if remove_device {
                if let Some(device_id) = device_removal.device_id.as_deref() {
                    match state.session.write().await.delete_device(device_id).await {
                        Ok(deleted) => {
                            if let Some(device) = device.as_ref() {
                                let local = clear_remote_registration(device);
                                if state
                                    .database
                                    .lock()
                                    .ok()
                                    .and_then(|database| database.save_device(&local).ok())
                                    .is_none()
                                {
                                    device_removal.outcome = DeviceRemovalOutcome::Failed;
                                    device_removal.error = Some(AppError::new(
                                        "device_save_failed",
                                        "RomM removed the device, but its local registration state could not be updated.",
                                        true,
                                    ));
                                } else {
                                    device_removal.outcome = if deleted {
                                        DeviceRemovalOutcome::Removed
                                    } else {
                                        DeviceRemovalOutcome::AlreadyMissing
                                    };
                                }
                            }
                        }
                        Err(error) => {
                            device_removal.outcome = DeviceRemovalOutcome::Failed;
                            device_removal.error = Some(error);
                        }
                    }
                } else {
                    device_removal.outcome = DeviceRemovalOutcome::AlreadyMissing;
                }
            }
            let locator = state
                .database
                .lock()
                .ok()
                .and_then(|database| database.load_server_profile().ok().flatten())
                .and_then(|profile| profile.credential_locator);
            if let Some(locator) = locator {
                let _ = state.credentials.delete(&locator);
            }
            if let Ok(database) = state.database.lock() {
                let _ = database.clear_credential_locator();
            }
            state.session.write().await.logout();
            *state.connection.write().await = ConnectionState::Pairing;
            update_onboarding_best_effort(&state, |onboarding| {
                let unauthenticated = invalidate_authenticated_onboarding(onboarding);
                if remove_device {
                    invalidate_device_onboarding(&unauthenticated)
                } else {
                    unauthenticated
                }
            })
            .await;
            AgentResponse::LoggedOut { device_removal }
        }
        AgentRequest::Shutdown => AgentResponse::ShuttingDown,
    }
}

async fn persist_authentication(
    state: &AgentState,
    auth: &ValidatedAuth,
    save_credential: bool,
) -> AgentResponse {
    let (base_url, server_version, http_approved, ca_id, last_contact_at_ms) = {
        let session = state.session.read().await;
        (
            session.base_url().unwrap_or_default(),
            session.server_version(),
            session.http_approved(),
            session.ca_id(),
            session.last_contact_at_ms(),
        )
    };
    let existing_profile = state
        .database
        .lock()
        .ok()
        .and_then(|database| database.load_server_profile().ok().flatten());
    let existing_locator = existing_profile
        .as_ref()
        .and_then(|profile| profile.credential_locator.clone());
    let existing_token = existing_locator
        .as_deref()
        .and_then(|locator| state.credentials.load(locator).ok());
    let locator = if save_credential {
        match state.credentials.save(&auth.token) {
            Ok(locator) => Some(locator.to_owned()),
            Err(error) => {
                *state.connection.write().await = ConnectionState::Connected;
                return AgentResponse::Authenticated {
                    result: auth_result(
                        &base_url,
                        auth,
                        false,
                        Some(format!(
                            "Connected for this session, but the credential could not be saved securely: {error}"
                        )),
                    ),
                };
            }
        }
    } else {
        existing_locator
    };
    let profile = StoredServerProfile {
        base_url: base_url.clone(),
        server_version,
        credential_locator: locator.clone(),
        http_approved,
        ca_id,
        token_id: Some(auth.token_id),
        account_id: Some(auth.account_id),
        account_name: Some(auth.account_name.clone()),
        granted_scopes: auth.granted_scopes.clone(),
        last_contact_at_ms,
    };
    let saved = state
        .database
        .lock()
        .ok()
        .and_then(|database| database.save_connection_profile(&profile).ok());
    if saved.is_none() {
        if save_credential {
            if let Some(token) = existing_token.as_deref() {
                let _ = state.credentials.save(token);
            } else if let Some(locator) = locator {
                let _ = state.credentials.delete(&locator);
            }
        }
        if let (Some(profile), Some(token)) = (existing_profile, existing_token) {
            let mut restored = profile
                .ca_id
                .as_deref()
                .and_then(|ca_id| {
                    state
                        .paths
                        .load_ca_certificate(ca_id)
                        .ok()
                        .and_then(|bytes| {
                            RommSession::with_certificate(&bytes, ca_id.to_owned()).ok()
                        })
                })
                .unwrap_or_default();
            let _ = restored.restore(&profile.base_url, profile.server_version.clone(), &token);
            restored.restore_metadata(
                profile.token_id,
                profile.account_id,
                profile.account_name,
                profile.granted_scopes,
                profile.last_contact_at_ms,
                profile.http_approved,
                profile.ca_id,
            );
            *state.session.write().await = restored;
            *state.connection.write().await = ConnectionState::Offline;
        } else {
            state.session.write().await.logout();
            *state.connection.write().await = ConnectionState::Pairing;
        }
        return foundation_error(
            "profile_save_failed",
            "The credential was not retained because the server profile could not be saved.",
            true,
        );
    }
    *state.connection.write().await = ConnectionState::Connected;

    let onboarding_warning = reconcile_onboarding_progress(state)
        .await
        .err()
        .map(|error| error.message);

    AgentResponse::Authenticated {
        result: auth_result(&base_url, auth, true, onboarding_warning),
    }
}

async fn advance_mapping_onboarding(
    state: &AgentState,
    drafts: &[PlatformMappingDraft],
    complete_mappings: bool,
) -> Result<OnboardingState, AppError> {
    let mut onboarding = state.onboarding.lock().await;
    let mut next = onboarding.clone();
    next.selected_platform_ids = drafts
        .iter()
        .filter(|draft| draft.enabled)
        .map(|draft| draft.platform_id)
        .collect();
    next.selected_platform_ids.sort_unstable();
    next.selected_platform_ids.dedup();
    next.mapping_draft_ids = drafts
        .iter()
        .filter(|draft| draft.enabled)
        .map(|draft| draft.id.clone())
        .collect();
    next.mapping_draft_ids.sort();
    next.mapping_draft_ids.dedup();
    for draft in drafts.iter().filter(|draft| draft.enabled) {
        if draft.custom_fields.get("romRoot") == Some(&true) && !draft.rom_root.trim().is_empty() {
            next.custom_path_drafts
                .insert(format!("{}:romRoot", draft.id), draft.rom_root.clone());
        }
        for (index, path) in draft.save_roots.iter().enumerate() {
            if draft.custom_fields.get("saveRoots") == Some(&true) && !path.trim().is_empty() {
                next.custom_path_drafts
                    .insert(format!("{}:saveRoots:{index}", draft.id), path.clone());
            }
        }
        for (index, path) in draft.state_roots.iter().enumerate() {
            if draft.custom_fields.get("stateRoots") == Some(&true) && !path.trim().is_empty() {
                next.custom_path_drafts
                    .insert(format!("{}:stateRoots:{index}", draft.id), path.clone());
            }
        }
    }
    if !next.completed_steps.contains(&OnboardingStep::Detection) {
        next = complete_onboarding_step(&next, OnboardingStep::Detection)
            .map_err(|message| AppError::new("onboarding_progress_invalid", message, false))?;
    }
    if complete_mappings && !next.completed_steps.contains(&OnboardingStep::Mappings) {
        next = complete_onboarding_step(&next, OnboardingStep::Mappings)
            .map_err(|message| AppError::new("onboarding_progress_invalid", message, false))?;
    }
    save_onboarding_progress(state, &next)?;
    *onboarding = next.clone();
    Ok(next)
}

async fn record_onboarding_server(
    state: &AgentState,
    origin: &str,
    http_approved: bool,
) -> Result<OnboardingState, AppError> {
    let mut onboarding = state.onboarding.lock().await;
    let next = select_onboarding_server(&onboarding, origin, http_approved)
        .map_err(|message| AppError::new("onboarding_progress_invalid", message, false))?;
    save_onboarding_progress(state, &next)?;
    *onboarding = next.clone();
    Ok(next)
}

async fn reconcile_onboarding_progress(state: &AgentState) -> Result<OnboardingState, AppError> {
    let (session_url, session_http_approved, session_has_credential) = {
        let session = state.session.read().await;
        (
            session.base_url(),
            session.http_approved(),
            session.credential().is_some(),
        )
    };
    let (profile, device) = state
        .database
        .lock()
        .map(|database| {
            (
                database.load_server_profile().ok().flatten(),
                database.load_device().ok().flatten(),
            )
        })
        .unwrap_or((None, None));

    let mut onboarding = state.onboarding.lock().await;
    let was_cancelled = onboarding.cancelled;
    let mut next = restore_onboarding_state(&onboarding).unwrap_or_default();

    let selected_origin = session_url
        .as_deref()
        .and_then(|url| normalize_base_url(url).ok())
        .map(|url| server_origin(&url))
        .or_else(|| {
            profile
                .as_ref()
                .and_then(|profile| normalize_base_url(&profile.base_url).ok())
                .map(|url| server_origin(&url))
        });
    if let Some(origin) = selected_origin.as_deref() {
        next = select_onboarding_server(&next, origin, session_http_approved)
            .map_err(|message| AppError::new("onboarding_progress_invalid", message, false))?;
    }

    let durable_authentication = profile.as_ref().is_some_and(|profile| {
        let profile_origin = normalize_base_url(&profile.base_url)
            .ok()
            .map(|url| server_origin(&url));
        session_has_credential
            && profile.credential_locator.is_some()
            && profile.account_id.is_some()
            && profile.account_name.is_some()
            && profile_origin == selected_origin
            && REQUIRED_SCOPES.iter().all(|required| {
                profile
                    .granted_scopes
                    .iter()
                    .any(|granted| granted == required)
            })
    });

    if durable_authentication {
        next = complete_onboarding_step(&next, OnboardingStep::Authentication)
            .map_err(|message| AppError::new("onboarding_progress_invalid", message, false))?;
        next = complete_onboarding_step(&next, OnboardingStep::Permissions)
            .map_err(|message| AppError::new("onboarding_progress_invalid", message, false))?;

        if device.as_ref().is_some_and(|device| {
            device.romm_device_id.is_some()
                && device.registration_state == DeviceRegistrationState::Registered
        }) {
            next = complete_onboarding_step(&next, OnboardingStep::Device)
                .map_err(|message| AppError::new("onboarding_progress_invalid", message, false))?;
            if let Some(origin) = selected_origin.as_deref() {
                let (has_drafts, has_outcome) = state
                    .database
                    .lock()
                    .map(|database| {
                        (
                            database
                                .load_mapping_drafts(origin)
                                .map(|drafts| !drafts.is_empty())
                                .unwrap_or(false),
                            database.has_mapping_outcome(origin).unwrap_or(false),
                        )
                    })
                    .unwrap_or((false, false));
                if (has_drafts || has_outcome)
                    && !next.completed_steps.contains(&OnboardingStep::Detection)
                {
                    next = complete_onboarding_step(&next, OnboardingStep::Detection).map_err(
                        |message| AppError::new("onboarding_progress_invalid", message, false),
                    )?;
                }
                if has_outcome && !next.completed_steps.contains(&OnboardingStep::Mappings) {
                    next = complete_onboarding_step(&next, OnboardingStep::Mappings).map_err(
                        |message| AppError::new("onboarding_progress_invalid", message, false),
                    )?;
                }
            }
        } else if next.completed_steps.contains(&OnboardingStep::Device) {
            next = invalidate_device_onboarding(&next);
        }
    } else {
        next = invalidate_authenticated_onboarding(&next);
    }

    if was_cancelled {
        next.cancelled = true;
    }
    if next != *onboarding {
        save_onboarding_progress(state, &next)?;
        *onboarding = next.clone();
    }
    Ok(next)
}

fn save_onboarding_progress(
    state: &AgentState,
    onboarding: &OnboardingState,
) -> Result<(), AppError> {
    state
        .database
        .lock()
        .ok()
        .and_then(|database| database.save_onboarding_state(onboarding).ok())
        .ok_or_else(|| {
            AppError::new(
                "onboarding_save_failed",
                "Onboarding progress could not be saved.",
                true,
            )
        })
}

fn load_app_settings(state: &AgentState) -> AppSettings {
    state
        .database
        .lock()
        .ok()
        .and_then(|database| database.get_app_state("desktop_settings").ok().flatten())
        .and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or_default()
}

fn save_app_settings(state: &AgentState, settings: &AppSettings) -> Result<(), AppError> {
    settings
        .validate()
        .map_err(|message| AppError::new("invalid_settings", message, false))?;
    let value = serde_json::to_string(settings).map_err(|error| {
        AppError::new(
            "settings_encode_failed",
            format!("Desktop settings could not be encoded: {error}"),
            false,
        )
    })?;
    state
        .database
        .lock()
        .ok()
        .and_then(|database| database.set_app_state("desktop_settings", &value).ok())
        .ok_or_else(|| {
            AppError::new(
                "settings_save_failed",
                "Desktop settings could not be saved.",
                true,
            )
        })
}

async fn record_onboarding_step_best_effort(state: &AgentState, step: OnboardingStep) {
    let mut onboarding = state.onboarding.lock().await;
    match complete_onboarding_step(&onboarding, step) {
        Ok(next) => {
            if let Err(error) = save_onboarding_progress(state, &next) {
                logging::error("onboarding_save_failed", &error.message);
            }
            *onboarding = next;
        }
        Err(error) => logging::error("onboarding_progress_invalid", &error),
    }
}

async fn update_onboarding_best_effort(
    state: &AgentState,
    update: impl FnOnce(&OnboardingState) -> OnboardingState,
) {
    let mut onboarding = state.onboarding.lock().await;
    let next = update(&onboarding);
    if let Err(error) = save_onboarding_progress(state, &next) {
        logging::error("onboarding_save_failed", &error.message);
    }
    *onboarding = next;
}

fn auth_result(
    base_url: &str,
    auth: &ValidatedAuth,
    credential_persisted: bool,
    warning: Option<String>,
) -> AuthResult {
    AuthResult {
        server_url: base_url.to_owned(),
        token_kind: "client_api_token".to_owned(),
        token_id: auth.token_id,
        account_id: auth.account_id,
        account_name: auth.account_name.clone(),
        granted_scopes: auth.granted_scopes.clone(),
        credential_persisted,
        warning,
    }
}

async fn connection_error(state: &AgentState, error: AppError) -> AgentResponse {
    let connection = match error.code.as_str() {
        "unauthorized" | "authentication_required" | "invalid_saved_credential" => {
            ConnectionState::Unauthorized
        }
        "forbidden" | "missing_required_scopes" => ConnectionState::ScopeError,
        "tls_error"
        | "invalid_ca_certificate"
        | "ca_certificate_missing"
        | "ca_origin_mismatch" => ConnectionState::TlsError,
        "incompatible_server" | "invalid_openapi" => ConnectionState::Incompatible,
        "network_error" | "server_error" => ConnectionState::Offline,
        "insecure_http_confirmation_required" | "invalid_server_url" | "unsafe_server_url" => {
            ConnectionState::Unconfigured
        }
        _ => state.connection.read().await.clone(),
    };
    *state.connection.write().await = connection;
    if matches!(
        error.code.as_str(),
        "unauthorized" | "authentication_required" | "invalid_saved_credential"
    ) {
        update_onboarding_best_effort(state, invalidate_authenticated_onboarding).await;
    }
    AgentResponse::Error { error }
}

fn foundation_error(code: &str, message: &str, retryable: bool) -> AgentResponse {
    AgentResponse::Error {
        error: AppError::new(code, message, retryable),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use romm_core::onboarding::ONBOARDING_STEPS;
    use romm_core::storage::AppPaths;
    use romm_ipc::{AppSettings, CloseBehavior, REQUIRED_SCOPES};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    fn test_state() -> (tempfile::TempDir, Arc<AgentState>) {
        test_state_with_background(background_success)
    }

    fn test_state_with_background(
        background_configurer: fn(bool) -> Result<background::RegistrationOutcome, String>,
    ) -> (tempfile::TempDir, Arc<AgentState>) {
        let temporary = tempfile::tempdir().expect("temporary directory should be created");
        let paths = AppPaths::from_roots(
            temporary.path().join("config"),
            temporary.path().join("data"),
            temporary.path().join("cache"),
        );
        let database = Database::open(&paths).expect("test database should open");
        (
            temporary,
            Arc::new(AgentState {
                session: RwLock::new(RommSession::new()),
                connection: RwLock::new(ConnectionState::Unconfigured),
                database: Mutex::new(database),
                credentials: SystemCredentialStore,
                paths,
                device_registration: AsyncMutex::new(()),
                library_request: AsyncMutex::new(()),
                onboarding: AsyncMutex::new(OnboardingState::default()),
                background_configurer,
            }),
        )
    }

    fn background_success(enabled: bool) -> Result<background::RegistrationOutcome, String> {
        Ok(background::RegistrationOutcome {
            enabled,
            method: "test_user_startup",
        })
    }

    fn background_failure(_enabled: bool) -> Result<background::RegistrationOutcome, String> {
        Err("test startup registration failed".to_owned())
    }

    async fn seed_background_onboarding(state: &AgentState) {
        let onboarding = OnboardingState {
            current_step: OnboardingStep::Background,
            highest_completed_step: Some(OnboardingStep::Mappings),
            completed_steps: ONBOARDING_STEPS[..6].to_vec(),
            server_origin: Some("https://romm.example.test".to_owned()),
            ..OnboardingState::default()
        };
        state
            .database
            .lock()
            .expect("database should lock")
            .save_onboarding_state(&onboarding)
            .expect("onboarding should save");
        *state.onboarding.lock().await = onboarding;
    }

    async fn seed_first_refresh_onboarding(state: &AgentState, origin: &str) {
        let onboarding = OnboardingState {
            current_step: OnboardingStep::FirstRefresh,
            highest_completed_step: Some(OnboardingStep::Background),
            completed_steps: ONBOARDING_STEPS[..7].to_vec(),
            server_origin: Some(origin.to_owned()),
            background_enabled: Some(false),
            ..OnboardingState::default()
        };
        state
            .database
            .lock()
            .expect("database should lock")
            .save_onboarding_state(&onboarding)
            .expect("onboarding should save");
        *state.onboarding.lock().await = onboarding;
    }

    async fn mock_registration_server(status: u16, body: serde_json::Value) -> String {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock server should bind");
        let address = listener.local_addr().expect("mock address should resolve");
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("request should connect");
            let mut request = [0_u8; 8192];
            let _ = stream.read(&mut request).await;
            let body = body.to_string();
            let reason = match status {
                200 => "OK",
                201 => "Created",
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
        });
        format!("http://{address}")
    }

    async fn mock_response_sequence(responses: Vec<(u16, serde_json::Value)>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock server should bind");
        let address = listener.local_addr().expect("mock address should resolve");
        tokio::spawn(async move {
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().await.expect("request should connect");
                let mut request = [0_u8; 8192];
                let _ = stream.read(&mut request).await;
                let body = body.to_string();
                let reason = match status {
                    200 => "OK",
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
            }
        });
        format!("http://{address}")
    }

    async fn authenticate_test_state(state: &AgentState, base_url: &str) {
        let token = format!("rmm_{}", "a".repeat(64));
        let mut session = state.session.write().await;
        session
            .restore(base_url, Some("5.1.0".to_owned()), &token)
            .expect("test session should restore");
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
        drop(session);
        *state.connection.write().await = ConnectionState::Connected;
    }

    fn save_registered_test_device(state: &AgentState) -> romm_ipc::DeviceIdentity {
        let mut device = propose_device_identity("0.1.0");
        device.romm_device_id = Some("device-123".to_owned());
        device.registration_fingerprint = Some("fingerprint".to_owned());
        device.registration_state = DeviceRegistrationState::Registered;
        device.registered_at_ms = Some(1234);
        state
            .database
            .lock()
            .expect("database should lock")
            .save_device(&device)
            .expect("registered device should save");
        device
    }

    #[tokio::test]
    async fn settings_round_trip_through_the_agent_database() {
        let (_temporary, state) = test_state();
        let settings = AppSettings {
            close_behavior: CloseBehavior::Quit,
            fullscreen: true,
            ..AppSettings::default()
        };
        let updated = dispatch(
            AgentRequest::UpdateSettings {
                settings: settings.clone(),
            },
            Arc::clone(&state),
        )
        .await;
        assert_eq!(
            updated,
            AgentResponse::Settings {
                settings: Some(settings.clone())
            }
        );
        assert_eq!(
            dispatch(AgentRequest::GetSettings, state).await,
            AgentResponse::Settings {
                settings: Some(settings)
            }
        );
    }

    #[tokio::test]
    async fn background_preferences_complete_only_after_successful_registration() {
        let (_temporary, state) = test_state();
        seed_background_onboarding(&state).await;
        let response = dispatch(
            AgentRequest::ConfigureOnboardingPreferences {
                background_enabled: true,
                stable_update_checks_enabled: true,
            },
            Arc::clone(&state),
        )
        .await;
        let AgentResponse::OnboardingPreferences { result } = response else {
            panic!("preferences should return their applied state");
        };
        assert!(result.background_enabled);
        assert!(result.stable_update_checks_enabled);
        assert!(result.warning.is_none());
        assert_eq!(result.onboarding.current_step, OnboardingStep::FirstRefresh);
        assert!(
            result
                .onboarding
                .completed_steps
                .contains(&OnboardingStep::Background)
        );
        assert!(load_app_settings(&state).stable_update_checks_enabled);
    }

    #[tokio::test]
    async fn failed_background_registration_preserves_progress_and_requires_a_choice() {
        let (_temporary, state) = test_state_with_background(background_failure);
        seed_background_onboarding(&state).await;
        let response = dispatch(
            AgentRequest::ConfigureOnboardingPreferences {
                background_enabled: true,
                stable_update_checks_enabled: false,
            },
            Arc::clone(&state),
        )
        .await;
        let AgentResponse::OnboardingPreferences { result } = response else {
            panic!("registration failure should be actionable");
        };
        assert!(!result.background_enabled);
        assert_eq!(
            result.warning.as_deref(),
            Some("test startup registration failed")
        );
        assert_eq!(result.onboarding.current_step, OnboardingStep::Background);
        assert!(
            !result
                .onboarding
                .completed_steps
                .contains(&OnboardingStep::Background)
        );
        assert!(
            result
                .onboarding
                .completed_steps
                .contains(&OnboardingStep::Mappings)
        );
    }

    #[tokio::test]
    async fn initial_refresh_atomically_caches_the_first_shelf_and_completes_onboarding() {
        let base_url = mock_registration_server(
            200,
            serde_json::json!({
                "items": [
                    {
                        "id": 42,
                        "name": "Chrono Trigger",
                        "platform_display_name": "SNES"
                    }
                ],
                "total": 107
            }),
        )
        .await;
        let (_temporary, state) = test_state();
        authenticate_test_state(&state, &base_url).await;
        let origin = state
            .session
            .read()
            .await
            .server_origin()
            .expect("server origin should exist");
        seed_first_refresh_onboarding(&state, &origin).await;

        let response = dispatch(AgentRequest::StartInitialRefresh, Arc::clone(&state)).await;
        let AgentResponse::InitialRefresh { result } = response else {
            panic!("initial refresh should return the first page");
        };
        assert_eq!(result.page.items.len(), 1);
        assert_eq!(result.page.total, Some(107));
        assert!(result.page.has_more);
        assert!(
            result
                .onboarding
                .completed_steps
                .contains(&OnboardingStep::FirstRefresh)
        );
        let database = state.database.lock().expect("database should lock");
        assert_eq!(
            database
                .load_initial_library_page(&origin)
                .expect("cached first page should load"),
            Some(result.page)
        );
        assert!(
            database
                .load_onboarding_state()
                .expect("onboarding should load")
                .expect("onboarding should exist")
                .completed_steps
                .contains(&OnboardingStep::FirstRefresh)
        );
    }

    #[tokio::test]
    async fn library_pages_fall_back_only_for_retryable_connection_failures() {
        let base_url = mock_response_sequence(vec![
            (
                200,
                serde_json::json!({
                    "items": [{
                        "id": 42,
                        "name": "Chrono Trigger",
                        "platform_id": 7,
                        "platform_display_name": "SNES"
                    }],
                    "total": 1
                }),
            ),
            (500, serde_json::json!({ "detail": "later" })),
            (401, serde_json::json!({ "detail": "expired" })),
        ])
        .await;
        let (_temporary, state) = test_state();
        authenticate_test_state(&state, &base_url).await;

        let live = dispatch(
            AgentRequest::ListRoms {
                limit: 48,
                offset: 0,
            },
            Arc::clone(&state),
        )
        .await;
        assert!(matches!(
            live,
            AgentResponse::Roms { page } if page.source == romm_ipc::LibrarySource::Live
        ));

        let cached = dispatch(
            AgentRequest::ListRoms {
                limit: 48,
                offset: 0,
            },
            Arc::clone(&state),
        )
        .await;
        assert!(matches!(
            cached,
            AgentResponse::Roms { page }
                if page.source == romm_ipc::LibrarySource::Cache && page.items.len() == 1
        ));
        assert_eq!(*state.connection.read().await, ConnectionState::Offline);

        let unauthorized = dispatch(
            AgentRequest::ListRoms {
                limit: 48,
                offset: 0,
            },
            Arc::clone(&state),
        )
        .await;
        assert!(matches!(
            unauthorized,
            AgentResponse::Error { error } if error.code == "unauthorized"
        ));
        assert_eq!(
            *state.connection.read().await,
            ConnectionState::Unauthorized
        );
    }

    #[tokio::test]
    async fn game_details_are_cached_and_used_only_for_retryable_failures() {
        let base_url = mock_response_sequence(vec![
            (
                200,
                serde_json::json!({
                    "id": 42,
                    "name": "Chrono Trigger",
                    "platform_id": 7,
                    "platform_display_name": "SNES",
                    "summary": "A time-travel adventure.",
                    "metadatum": { "genres": ["Role-playing"] },
                    "files": [],
                    "sibling_roms": [],
                    "user_collections": [],
                    "user_saves": [],
                    "user_states": []
                }),
            ),
            (500, serde_json::json!({ "detail": "later" })),
            (401, serde_json::json!({ "detail": "expired" })),
        ])
        .await;
        let (_temporary, state) = test_state();
        authenticate_test_state(&state, &base_url).await;

        let live = dispatch(
            AgentRequest::GetGameDetails { rom_id: 42 },
            Arc::clone(&state),
        )
        .await;
        assert!(matches!(
            live,
            AgentResponse::GameDetails { details }
                if details.source == romm_ipc::LibrarySource::Live
                    && details.genres == vec!["Role-playing"]
        ));

        let cached = dispatch(
            AgentRequest::GetGameDetails { rom_id: 42 },
            Arc::clone(&state),
        )
        .await;
        assert!(matches!(
            cached,
            AgentResponse::GameDetails { details }
                if details.source == romm_ipc::LibrarySource::Cache
                    && details.rom.id == 42
        ));
        assert_eq!(*state.connection.read().await, ConnectionState::Offline);

        let unauthorized = dispatch(
            AgentRequest::GetGameDetails { rom_id: 42 },
            Arc::clone(&state),
        )
        .await;
        assert!(matches!(
            unauthorized,
            AgentResponse::Error { error } if error.code == "unauthorized"
        ));
    }

    #[tokio::test]
    async fn an_authoritative_empty_library_still_completes_the_first_refresh() {
        let base_url =
            mock_registration_server(200, serde_json::json!({ "items": [], "total": 0 })).await;
        let (_temporary, state) = test_state();
        authenticate_test_state(&state, &base_url).await;
        let origin = state
            .session
            .read()
            .await
            .server_origin()
            .expect("server origin should exist");
        seed_first_refresh_onboarding(&state, &origin).await;

        let response = dispatch(AgentRequest::StartInitialRefresh, Arc::clone(&state)).await;
        let AgentResponse::InitialRefresh { result } = response else {
            panic!("empty initial refresh should still complete");
        };
        assert!(result.page.items.is_empty());
        assert_eq!(result.page.total, Some(0));
        assert!(!result.page.has_more);
        assert!(
            result
                .onboarding
                .completed_steps
                .contains(&OnboardingStep::FirstRefresh)
        );
    }

    #[tokio::test]
    async fn failed_initial_refresh_keeps_every_prior_step_and_remains_retryable() {
        let base_url =
            mock_registration_server(500, serde_json::json!({ "detail": "later" })).await;
        let (_temporary, state) = test_state();
        authenticate_test_state(&state, &base_url).await;
        let origin = state
            .session
            .read()
            .await
            .server_origin()
            .expect("server origin should exist");
        seed_first_refresh_onboarding(&state, &origin).await;

        let response = dispatch(AgentRequest::StartInitialRefresh, Arc::clone(&state)).await;
        assert!(matches!(
            response,
            AgentResponse::Error { error }
                if error.retryable && error.code == "server_error"
        ));
        let onboarding = state.onboarding.lock().await.clone();
        assert_eq!(onboarding.completed_steps, ONBOARDING_STEPS[..7]);
        assert_eq!(onboarding.current_step, OnboardingStep::FirstRefresh);
        assert!(
            state
                .database
                .lock()
                .expect("database should lock")
                .load_initial_library_page(&origin)
                .expect("cache lookup should succeed")
                .is_none()
        );
    }

    #[tokio::test]
    async fn onboarding_navigation_is_persisted_without_a_completion_bypass() {
        let (_temporary, state) = test_state();
        let cancelled = dispatch(
            AgentRequest::UpdateOnboarding {
                action: romm_ipc::OnboardingNavigationAction::Cancel,
            },
            Arc::clone(&state),
        )
        .await;
        assert!(matches!(
            cancelled,
            AgentResponse::Onboarding { state } if state.cancelled
        ));
        assert!(
            state
                .database
                .lock()
                .expect("database should lock")
                .load_onboarding_state()
                .expect("onboarding should load")
                .expect("onboarding should exist")
                .cancelled
        );
        let resumed = dispatch(
            AgentRequest::UpdateOnboarding {
                action: romm_ipc::OnboardingNavigationAction::Resume,
            },
            Arc::clone(&state),
        )
        .await;
        assert!(matches!(
            resumed,
            AgentResponse::Onboarding { state }
                if !state.cancelled && state.current_step == romm_ipc::OnboardingStep::Server
        ));
        assert!(matches!(
            dispatch(AgentRequest::GetOnboarding, state).await,
            AgentResponse::Onboarding { state }
                if !state.cancelled && state.current_step == romm_ipc::OnboardingStep::Server
        ));
    }

    #[tokio::test]
    async fn onboarding_reconciles_an_existing_profile_and_device_without_remote_mutation() {
        let (_temporary, state) = test_state();
        let base_url = "https://romm.example.test/";
        authenticate_test_state(&state, base_url).await;
        state
            .database
            .lock()
            .expect("database should lock")
            .save_connection_profile(&StoredServerProfile {
                base_url: base_url.to_owned(),
                server_version: Some("5.1.0".to_owned()),
                credential_locator: Some("romm-companion:test".to_owned()),
                http_approved: false,
                ca_id: None,
                token_id: Some(42),
                account_id: Some(7),
                account_name: Some("justin".to_owned()),
                granted_scopes: REQUIRED_SCOPES
                    .iter()
                    .map(|scope| (*scope).to_owned())
                    .collect(),
                last_contact_at_ms: Some(1234),
            })
            .expect("profile should save");
        save_registered_test_device(&state);

        let response = dispatch(AgentRequest::GetOnboarding, Arc::clone(&state)).await;
        let AgentResponse::Onboarding { state: progress } = response else {
            panic!("onboarding should reconcile");
        };
        assert_eq!(progress.completed_steps, ONBOARDING_STEPS[..4]);
        assert_eq!(progress.current_step, OnboardingStep::Detection);

        let repeated = dispatch(AgentRequest::GetOnboarding, Arc::clone(&state)).await;
        assert_eq!(repeated, AgentResponse::Onboarding { state: progress });
        assert_eq!(
            state
                .database
                .lock()
                .expect("database should lock")
                .load_device()
                .expect("device should load")
                .and_then(|device| device.romm_device_id),
            Some("device-123".to_owned())
        );
    }

    #[tokio::test]
    async fn invalid_controller_settings_are_not_persisted() {
        let (_temporary, state) = test_state();
        let mut settings = AppSettings::default();
        settings.controller.dead_zone_percent = 5;
        let response = dispatch(
            AgentRequest::UpdateSettings { settings },
            Arc::clone(&state),
        )
        .await;
        assert!(matches!(
            response,
            AgentResponse::Error { error } if error.code == "invalid_settings"
        ));
        assert_eq!(
            dispatch(AgentRequest::GetSettings, state).await,
            AgentResponse::Settings { settings: None }
        );
    }

    #[tokio::test]
    async fn connection_errors_transition_without_deleting_the_profile() {
        let (_temporary, state) = test_state();
        if let Ok(database) = state.database.lock() {
            database
                .save_server_profile("https://romm.example.test/", Some("5.0.0"), None)
                .expect("profile should save");
        }
        let response = connection_error(
            &state,
            AppError::new("network_error", "RomM is unavailable.", true),
        )
        .await;
        assert!(matches!(response, AgentResponse::Error { .. }));
        assert_eq!(*state.connection.read().await, ConnectionState::Offline);
        assert!(
            state
                .database
                .lock()
                .expect("database should lock")
                .load_server_profile()
                .expect("profile should load")
                .is_some()
        );
    }

    #[tokio::test]
    async fn proposing_a_device_is_persistent_and_idempotent() {
        let (_temporary, state) = test_state();
        let first = dispatch(AgentRequest::ProposeDevice, Arc::clone(&state)).await;
        let second = dispatch(AgentRequest::ProposeDevice, Arc::clone(&state)).await;

        let AgentResponse::DeviceProposed { device: first } = first else {
            panic!("first proposal should return a device");
        };
        let AgentResponse::DeviceProposed { device: second } = second else {
            panic!("second proposal should return a device");
        };
        assert_eq!(first, second);
        assert!(first.romm_device_id.is_none());
        assert_eq!(
            state
                .database
                .lock()
                .expect("database should lock")
                .load_device()
                .expect("device should load"),
            Some(first)
        );
    }

    #[tokio::test]
    async fn concurrent_registration_creates_and_persists_one_remote_identity() {
        let (_temporary, state) = test_state();
        let base_url = mock_registration_server(
            201,
            serde_json::json!({
                "device_id": "device-123",
                "name": "Living Room PC",
                "created_at": "2026-08-27T12:00:00Z"
            }),
        )
        .await;
        authenticate_test_state(&state, &base_url).await;

        let (first, second) = tokio::join!(
            dispatch(
                AgentRequest::RegisterDevice {
                    display_name: "Living Room PC".to_owned(),
                },
                Arc::clone(&state),
            ),
            dispatch(
                AgentRequest::RegisterDevice {
                    display_name: "Living Room PC".to_owned(),
                },
                Arc::clone(&state),
            )
        );
        let responses = [first, second]
            .into_iter()
            .map(|response| match response {
                AgentResponse::DeviceRegistered {
                    device,
                    newly_registered,
                } => (device, newly_registered),
                other => panic!("registration should succeed, got {other:?}"),
            })
            .collect::<Vec<_>>();

        assert_eq!(responses[0].0, responses[1].0);
        assert_eq!(
            responses
                .iter()
                .filter(|(_, newly_registered)| *newly_registered)
                .count(),
            1
        );
        assert_eq!(responses[0].0.romm_device_id.as_deref(), Some("device-123"));
        assert_eq!(
            state
                .database
                .lock()
                .expect("database should lock")
                .load_device()
                .expect("device should load"),
            Some(responses[0].0.clone())
        );
    }

    #[tokio::test]
    async fn failed_registration_preserves_authentication_and_the_local_draft() {
        let (_temporary, state) = test_state();
        let base_url =
            mock_registration_server(500, serde_json::json!({ "detail": "try later" })).await;
        authenticate_test_state(&state, &base_url).await;
        let profile = StoredServerProfile {
            base_url: format!("{base_url}/"),
            server_version: Some("5.1.0".to_owned()),
            credential_locator: Some("system:test-token".to_owned()),
            http_approved: true,
            ca_id: None,
            token_id: Some(42),
            account_id: Some(7),
            account_name: Some("justin".to_owned()),
            granted_scopes: REQUIRED_SCOPES
                .iter()
                .map(|scope| (*scope).to_owned())
                .collect(),
            last_contact_at_ms: Some(1234),
        };
        state
            .database
            .lock()
            .expect("database should lock")
            .save_connection_profile(&profile)
            .expect("profile should save");
        let proposal = dispatch(AgentRequest::ProposeDevice, Arc::clone(&state)).await;
        let AgentResponse::DeviceProposed { device: proposal } = proposal else {
            panic!("device should be proposed");
        };

        let response = dispatch(
            AgentRequest::RegisterDevice {
                display_name: "Retry Me".to_owned(),
            },
            Arc::clone(&state),
        )
        .await;
        let AgentResponse::Error { error } = response else {
            panic!("registration should fail");
        };
        assert_eq!(error.code, "server_error");
        assert!(state.session.read().await.is_authenticated());
        assert_eq!(*state.connection.read().await, ConnectionState::Offline);
        let database = state.database.lock().expect("database should lock");
        assert_eq!(
            database.load_server_profile().expect("profile should load"),
            Some(profile)
        );
        let draft = database
            .load_device()
            .expect("device should load")
            .expect("device should remain");
        assert_eq!(draft.local_id, proposal.local_id);
        assert_eq!(draft.display_name, "Retry Me");
        assert!(draft.romm_device_id.is_none());
    }

    #[tokio::test]
    async fn verification_persists_the_remote_device_and_timestamp() {
        let (_temporary, state) = test_state();
        let base_url = mock_registration_server(
            200,
            serde_json::json!({
                "id": "device-123",
                "user_id": 7,
                "name": "Verified Living Room",
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
            }),
        )
        .await;
        authenticate_test_state(&state, &base_url).await;
        save_registered_test_device(&state);

        let response = dispatch(AgentRequest::VerifyDevice, Arc::clone(&state)).await;
        let AgentResponse::DeviceVerification { device, outcome } = response else {
            panic!("verification should return a device outcome");
        };
        assert_eq!(outcome, DeviceVerificationOutcome::Verified);
        assert_eq!(device.display_name, "Verified Living Room");
        assert!(device.verified_at_ms.is_some());
        assert_eq!(
            device.registration_state,
            DeviceRegistrationState::Registered
        );
        assert_eq!(
            state
                .database
                .lock()
                .expect("database should lock")
                .load_device()
                .expect("device should load"),
            Some(device)
        );
    }

    #[tokio::test]
    async fn a_deleted_remote_device_can_reregister_without_losing_local_state() {
        let (_temporary, state) = test_state();
        let missing_url =
            mock_registration_server(404, serde_json::json!({ "detail": "not found" })).await;
        authenticate_test_state(&state, &missing_url).await;
        let mut original = save_registered_test_device(&state);
        original.mapping_summary.insert(
            "snes".to_owned(),
            serde_json::json!({ "romRoot": "D:/ROMs/SNES" }),
        );
        state
            .database
            .lock()
            .expect("database should lock")
            .save_device(&original)
            .expect("mapped device should save");

        let response = dispatch(AgentRequest::VerifyDevice, Arc::clone(&state)).await;
        let AgentResponse::DeviceVerification {
            device: missing,
            outcome,
        } = response
        else {
            panic!("missing device should return a recovery outcome");
        };
        assert_eq!(outcome, DeviceVerificationOutcome::Missing);
        assert_eq!(missing.registration_state, DeviceRegistrationState::Missing);
        assert_eq!(missing.romm_device_id.as_deref(), Some("device-123"));
        assert_eq!(missing.mapping_summary, original.mapping_summary);

        let registration_url = mock_registration_server(
            201,
            serde_json::json!({
                "device_id": "replacement-device",
                "name": "Replacement",
                "created_at": "2026-08-28T12:00:00Z"
            }),
        )
        .await;
        authenticate_test_state(&state, &registration_url).await;
        let response = dispatch(
            AgentRequest::RegisterDevice {
                display_name: missing.display_name.clone(),
            },
            Arc::clone(&state),
        )
        .await;
        let AgentResponse::DeviceRegistered { device, .. } = response else {
            panic!("missing device should register again");
        };
        assert_eq!(device.local_id, original.local_id);
        assert_eq!(device.romm_device_id.as_deref(), Some("replacement-device"));
        assert_eq!(device.mapping_summary, original.mapping_summary);
        assert_eq!(
            device.registration_state,
            DeviceRegistrationState::Registered
        );
    }

    #[tokio::test]
    async fn verification_permission_loss_is_persisted_without_deleting_the_device() {
        let (_temporary, state) = test_state();
        let base_url =
            mock_registration_server(403, serde_json::json!({ "detail": "forbidden" })).await;
        authenticate_test_state(&state, &base_url).await;
        let original = save_registered_test_device(&state);

        let response = dispatch(AgentRequest::VerifyDevice, Arc::clone(&state)).await;
        let AgentResponse::DeviceVerification { device, outcome } = response else {
            panic!("permission failure should return a recovery outcome");
        };
        assert_eq!(outcome, DeviceVerificationOutcome::PermissionDenied);
        assert_eq!(
            device.registration_state,
            DeviceRegistrationState::PermissionError
        );
        assert_eq!(device.romm_device_id, original.romm_device_id);
        assert_eq!(*state.connection.read().await, ConnectionState::ScopeError);
    }

    #[tokio::test]
    async fn device_update_persists_the_authoritative_name() {
        let (_temporary, state) = test_state();
        let base_url = mock_registration_server(
            200,
            serde_json::json!({
                "id": "device-123",
                "user_id": 7,
                "name": "Arcade Room",
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
            }),
        )
        .await;
        authenticate_test_state(&state, &base_url).await;
        save_registered_test_device(&state);

        let response = dispatch(
            AgentRequest::UpdateDevice {
                display_name: "Arcade Room".to_owned(),
            },
            Arc::clone(&state),
        )
        .await;
        let AgentResponse::DeviceUpdated { device } = response else {
            panic!("device update should succeed");
        };
        assert_eq!(device.display_name, "Arcade Room");
        assert_eq!(
            state
                .database
                .lock()
                .expect("database should lock")
                .load_device()
                .expect("device should load"),
            Some(device)
        );
    }

    #[tokio::test]
    async fn normal_logout_keeps_the_registered_device() {
        let (_temporary, state) = test_state();
        let device = save_registered_test_device(&state);

        let response = dispatch(
            AgentRequest::Logout {
                remove_device: false,
            },
            Arc::clone(&state),
        )
        .await;
        assert_eq!(
            response,
            AgentResponse::LoggedOut {
                device_removal: DeviceRemovalResult {
                    outcome: DeviceRemovalOutcome::NotRequested,
                    device_id: Some("device-123".to_owned()),
                    error: None,
                }
            }
        );
        assert_eq!(
            state
                .database
                .lock()
                .expect("database should lock")
                .load_device()
                .expect("device should load"),
            Some(device)
        );
        assert!(!state.session.read().await.is_authenticated());
    }

    #[tokio::test]
    async fn logout_can_remove_only_the_remote_registration() {
        let (_temporary, state) = test_state();
        let base_url = mock_registration_server(204, serde_json::json!(null)).await;
        authenticate_test_state(&state, &base_url).await;
        let original = save_registered_test_device(&state);

        let response = dispatch(
            AgentRequest::Logout {
                remove_device: true,
            },
            Arc::clone(&state),
        )
        .await;
        let AgentResponse::LoggedOut { device_removal } = response else {
            panic!("logout should always complete");
        };
        assert_eq!(device_removal.outcome, DeviceRemovalOutcome::Removed);
        let local = state
            .database
            .lock()
            .expect("database should lock")
            .load_device()
            .expect("device should load")
            .expect("local identity should remain");
        assert_eq!(local.local_id, original.local_id);
        assert_eq!(local.mapping_summary, original.mapping_summary);
        assert!(local.romm_device_id.is_none());
        assert_eq!(
            local.registration_state,
            DeviceRegistrationState::Unregistered
        );
        assert!(!state.session.read().await.is_authenticated());
    }

    #[tokio::test]
    async fn failed_remote_removal_never_blocks_local_logout() {
        let (_temporary, state) = test_state();
        let base_url =
            mock_registration_server(500, serde_json::json!({ "detail": "later" })).await;
        authenticate_test_state(&state, &base_url).await;
        let original = save_registered_test_device(&state);

        let response = dispatch(
            AgentRequest::Logout {
                remove_device: true,
            },
            Arc::clone(&state),
        )
        .await;
        let AgentResponse::LoggedOut { device_removal } = response else {
            panic!("logout should complete even when removal fails");
        };
        assert_eq!(device_removal.outcome, DeviceRemovalOutcome::Failed);
        assert!(device_removal.error.is_some());
        assert_eq!(
            state
                .database
                .lock()
                .expect("database should lock")
                .load_device()
                .expect("device should load"),
            Some(original)
        );
        assert!(!state.session.read().await.is_authenticated());
    }
}
