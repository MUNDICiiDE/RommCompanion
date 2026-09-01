mod controller;
mod desktop_settings;

use std::{
    io::Write,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use controller::{
    ControllerActionKind, ControllerActionPhase, ControllerInfo, ControllerPipeline,
    ControllerStatus, ControllerStatusChange, ProcessedInput, action_for_button, axis_direction,
    mapping_family,
};
use desktop_settings::{
    CloseBehavior, DesktopSettingsState, get_desktop_settings, update_desktop_settings,
};
use gilrs::{Axis, Button, EventType, GamepadId, Gilrs};
use romm_ipc::{
    AgentRequest, ControllerBindings, ControllerButton, ControllerSettings, IPC_SCHEMA_VERSION,
    RequestEnvelope, ResponseEnvelope, local_ipc_endpoint,
};
use tauri::{
    AppHandle, Emitter, Manager, State, WindowEvent,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[cfg(windows)]
type LocalAgentStream = tokio::net::windows::named_pipe::NamedPipeClient;
#[cfg(unix)]
type LocalAgentStream = tokio::net::UnixStream;

#[cfg(windows)]
type SyncLocalAgentStream = std::fs::File;
#[cfg(unix)]
type SyncLocalAgentStream = std::os::unix::net::UnixStream;

pub(crate) struct AgentClient {
    sequence: AtomicU64,
    child: Mutex<Option<Child>>,
}

impl AgentClient {
    fn executable_path() -> Result<PathBuf, String> {
        let current_executable = std::env::current_exe()
            .map_err(|error| format!("Unable to locate the desktop executable: {error}"))?;
        let directory = current_executable
            .parent()
            .ok_or_else(|| "The desktop executable has no parent directory.".to_owned())?;
        Ok(directory.join(format!("romm-sync-agent{}", std::env::consts::EXE_SUFFIX)))
    }

    fn start_agent(&self) -> Result<(), String> {
        let mut child_slot = self
            .child
            .lock()
            .map_err(|_| "The agent process manager is unavailable.".to_owned())?;
        if let Some(child) = child_slot.as_mut() {
            match child.try_wait() {
                Ok(None) => return Ok(()),
                Ok(Some(_)) => *child_slot = None,
                Err(error) => return Err(format!("Unable to inspect the agent process: {error}")),
            }
        }

        let executable = Self::executable_path()?;
        if !executable.is_file() {
            return Err(format!(
                "The RomM sync agent is missing from {}. Rebuild or reinstall RomM Companion.",
                executable.display()
            ));
        }

        let mut command = Command::new(&executable);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let child = command.spawn().map_err(|error| {
            format!(
                "Unable to start the RomM sync agent at {}: {error}",
                executable.display()
            )
        })?;
        *child_slot = Some(child);
        Ok(())
    }

    #[cfg(windows)]
    async fn open_local_stream() -> Result<LocalAgentStream, std::io::Error> {
        let endpoint = local_ipc_endpoint().map_err(std::io::Error::other)?;
        tokio::net::windows::named_pipe::ClientOptions::new().open(endpoint)
    }

    #[cfg(unix)]
    async fn open_local_stream() -> Result<LocalAgentStream, std::io::Error> {
        let endpoint = local_ipc_endpoint().map_err(std::io::Error::other)?;
        tokio::net::UnixStream::connect(endpoint).await
    }

    #[cfg(windows)]
    fn open_sync_stream() -> Result<SyncLocalAgentStream, std::io::Error> {
        let endpoint = local_ipc_endpoint().map_err(std::io::Error::other)?;
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(endpoint)
    }

    #[cfg(unix)]
    fn open_sync_stream() -> Result<SyncLocalAgentStream, std::io::Error> {
        let endpoint = local_ipc_endpoint().map_err(std::io::Error::other)?;
        std::os::unix::net::UnixStream::connect(endpoint)
    }

    async fn connect(&self) -> Result<LocalAgentStream, String> {
        if let Ok(stream) = Self::open_local_stream().await {
            return Ok(stream);
        }

        self.start_agent()?;
        let mut last_error = None;
        for delay in [50_u64, 250, 1_000, 4_000] {
            tokio::time::sleep(Duration::from_millis(delay)).await;
            match Self::open_local_stream().await {
                Ok(stream) => return Ok(stream),
                Err(error) => last_error = Some(error),
            }
        }

        Err(format!(
            "The RomM sync agent did not become ready: {}",
            last_error
                .map(|error| error.to_string())
                .unwrap_or_else(|| "unknown startup error".to_owned())
        ))
    }

    pub(crate) async fn send(&self, body: AgentRequest) -> Result<ResponseEnvelope, String> {
        let request_id = format!("desktop-{}", self.sequence.fetch_add(1, Ordering::Relaxed));
        let request = RequestEnvelope::new(request_id.clone(), body);
        let mut stream = self.connect().await?;
        let payload = serde_json::to_string(&request)
            .map_err(|error| format!("Unable to encode the agent request: {error}"))?;
        stream
            .write_all(format!("{payload}\n").as_bytes())
            .await
            .map_err(|error| format!("Unable to send the agent request: {error}"))?;

        let mut response = String::new();
        BufReader::new(stream)
            .read_line(&mut response)
            .await
            .map_err(|error| format!("Unable to read the agent response: {error}"))?;
        let response: ResponseEnvelope = serde_json::from_str(&response)
            .map_err(|error| format!("The agent returned invalid data: {error}"))?;
        if response.request_id != request_id {
            return Err("The desktop app received a response for a different request.".to_owned());
        }
        if response.schema_version != IPC_SCHEMA_VERSION {
            return Err(
                "The desktop app and agent versions do not match. Quit completely and reinstall or restart RomM Companion."
                    .to_owned(),
            );
        }
        Ok(response)
    }

    fn shutdown_owned(&self) {
        let Ok(mut child_slot) = self.child.lock() else {
            return;
        };
        let Some(mut child) = child_slot.take() else {
            return;
        };
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }

        let request_id = format!("desktop-{}", self.sequence.fetch_add(1, Ordering::Relaxed));
        if let Ok(mut stream) = Self::open_sync_stream() {
            let request = RequestEnvelope::new(request_id, AgentRequest::Shutdown);
            if let Ok(payload) = serde_json::to_string(&request) {
                let _ = stream.write_all(format!("{payload}\n").as_bytes());
            }
        }

        for _ in 0..20 {
            if matches!(child.try_wait(), Ok(Some(_))) {
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}

impl Drop for AgentClient {
    fn drop(&mut self) {
        self.shutdown_owned();
    }
}

#[tauri::command]
async fn agent_request(
    state: State<'_, AgentClient>,
    request: AgentRequest,
) -> Result<ResponseEnvelope, String> {
    state.send(request).await
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn quit_completely(app: &AppHandle) {
    let settings = app.state::<DesktopSettingsState>();
    settings.mark_quitting();
    app.state::<AgentClient>().shutdown_owned();
    app.exit(0);
}

fn setup_tray(app: &mut tauri::App) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "tray-open", "Open RomM Companion", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "tray-quit", "Quit completely", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    let mut tray = TrayIconBuilder::with_id("main-tray")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("RomM Companion")
        .on_menu_event(|app, event| match event.id().as_ref() {
            "tray-open" => show_main_window(app),
            "tray-quit" => quit_completely(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

struct ControllerStatusState(Mutex<Option<ControllerStatus>>);

#[tauri::command]
fn get_controller_status(
    state: State<'_, ControllerStatusState>,
) -> Result<Option<ControllerStatus>, String> {
    state
        .0
        .lock()
        .map(|status| status.clone())
        .map_err(|_| "The controller status is unavailable.".to_owned())
}

fn emit_controller_status(app: &AppHandle, status: ControllerStatus) {
    if let Ok(mut stored) = app.state::<ControllerStatusState>().0.lock() {
        *stored = Some(status.clone());
    }
    let _ = app.emit("controller-status", status);
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn controller_guid(uuid: [u8; 16]) -> String {
    uuid.iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

fn controller_id(id: GamepadId, guid: &str) -> String {
    format!("{guid}-{id}")
}

fn controller_info(gilrs: &Gilrs, id: GamepadId) -> ControllerInfo {
    let gamepad = gilrs.gamepad(id);
    let name = gamepad.name().trim();
    let name = if name.is_empty() {
        "Unknown controller"
    } else {
        name
    };
    let guid = controller_guid(gamepad.uuid());
    ControllerInfo {
        id: controller_id(id, &guid),
        guid,
        name: name.to_owned(),
        mapping_family: mapping_family(gamepad.vendor_id(), name),
    }
}

fn configurable_button(button: Button) -> Option<ControllerButton> {
    match button {
        Button::South => Some(ControllerButton::South),
        Button::East => Some(ControllerButton::East),
        Button::West => Some(ControllerButton::West),
        Button::North => Some(ControllerButton::North),
        Button::LeftTrigger => Some(ControllerButton::LeftShoulder),
        Button::RightTrigger => Some(ControllerButton::RightShoulder),
        _ => None,
    }
}

fn button_action(button: Button, bindings: &ControllerBindings) -> Option<ControllerActionKind> {
    match button {
        Button::DPadUp => Some(ControllerActionKind::Up),
        Button::DPadDown => Some(ControllerActionKind::Down),
        Button::DPadLeft => Some(ControllerActionKind::Left),
        Button::DPadRight => Some(ControllerActionKind::Right),
        _ => configurable_button(button).and_then(|button| action_for_button(button, bindings)),
    }
}

fn emit_processed(
    app: &AppHandle,
    pipeline: &ControllerPipeline,
    processed: ProcessedInput,
    controller_id: &str,
    timestamp_ms: u64,
) {
    if processed.active_changed {
        let status = pipeline.status(
            ControllerStatusChange::ActiveChanged,
            pipeline.controller_info(controller_id),
            timestamp_ms,
        );
        emit_controller_status(app, status);
    }
    for action in processed.actions {
        let _ = app.emit("controller-action", action);
    }
}

fn start_controller_bridge(app: AppHandle) {
    thread::spawn(move || {
        let Ok(mut gilrs) = Gilrs::new() else {
            let _ = app.emit("controller-unavailable", ());
            return;
        };
        let mut pipeline = ControllerPipeline::default();
        for (id, _) in gilrs.gamepads() {
            pipeline.connect(controller_info(&gilrs, id));
        }
        emit_controller_status(
            &app,
            pipeline.status(ControllerStatusChange::Initialized, None, now_ms()),
        );

        loop {
            let settings = app
                .state::<DesktopSettingsState>()
                .snapshot()
                .map(|settings| settings.controller)
                .unwrap_or_else(|_| ControllerSettings::default());
            while let Some(event) = gilrs.next_event() {
                let timestamp_ms = now_ms();
                let info = controller_info(&gilrs, event.id);
                let id = info.id.clone();
                match event.event {
                    EventType::Connected => {
                        pipeline.connect(info.clone());
                        emit_controller_status(
                            &app,
                            pipeline.status(
                                ControllerStatusChange::Connected,
                                Some(info),
                                timestamp_ms,
                            ),
                        );
                    }
                    EventType::Disconnected => {
                        let removed = pipeline.disconnect(&id).or(Some(info));
                        emit_controller_status(
                            &app,
                            pipeline.status(
                                ControllerStatusChange::Disconnected,
                                removed,
                                timestamp_ms,
                            ),
                        );
                    }
                    EventType::ButtonPressed(button, _) => {
                        let bindings = settings.bindings_for(&info.guid);
                        if let Some(action) = button_action(button, bindings) {
                            let processed = pipeline.button(
                                &id,
                                action,
                                ControllerActionPhase::Pressed,
                                timestamp_ms,
                                settings.initial_repeat_delay_ms,
                            );
                            emit_processed(&app, &pipeline, processed, &id, timestamp_ms);
                        }
                    }
                    EventType::ButtonRepeated(_, _) => {}
                    EventType::ButtonReleased(button, _) => {
                        let bindings = settings.bindings_for(&info.guid);
                        if let Some(action) = button_action(button, bindings) {
                            let processed = pipeline.button(
                                &id,
                                action,
                                ControllerActionPhase::Released,
                                timestamp_ms,
                                settings.initial_repeat_delay_ms,
                            );
                            emit_processed(&app, &pipeline, processed, &id, timestamp_ms);
                        }
                    }
                    EventType::AxisChanged(Axis::LeftStickX, value, _) => {
                        let processed = pipeline.horizontal_axis(
                            &id,
                            axis_direction(value, settings.dead_zone_percent),
                            timestamp_ms,
                            settings.initial_repeat_delay_ms,
                        );
                        emit_processed(&app, &pipeline, processed, &id, timestamp_ms);
                    }
                    EventType::AxisChanged(Axis::LeftStickY, value, _) => {
                        let processed = pipeline.vertical_axis(
                            &id,
                            axis_direction(value, settings.dead_zone_percent),
                            timestamp_ms,
                            settings.initial_repeat_delay_ms,
                        );
                        emit_processed(&app, &pipeline, processed, &id, timestamp_ms);
                    }
                    _ => {}
                }
            }
            for action in pipeline.poll_repeats(now_ms(), settings.repeat_interval_ms) {
                let _ = app.emit("controller-action", action);
            }
            thread::sleep(Duration::from_millis(8));
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AgentClient {
            sequence: AtomicU64::new(1),
            child: Mutex::new(None),
        })
        .manage(ControllerStatusState(Mutex::new(None)))
        .invoke_handler(tauri::generate_handler![
            agent_request,
            get_controller_status,
            get_desktop_settings,
            update_desktop_settings
        ])
        .setup(|app| {
            let desktop_settings = DesktopSettingsState::load(app.handle())?;
            let start_fullscreen = desktop_settings.fullscreen();
            app.manage(desktop_settings);
            setup_tray(app)?;
            if start_fullscreen && let Some(window) = app.get_webview_window("main") {
                window.set_fullscreen(true)?;
            }
            start_controller_bridge(app.handle().clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let settings = window.state::<DesktopSettingsState>();
                if settings.is_quitting() {
                    return;
                }

                api.prevent_close();
                match settings.close_behavior() {
                    CloseBehavior::MinimizeToTray => {
                        let _ = window.hide();
                    }
                    CloseBehavior::Quit => quit_completely(window.app_handle()),
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running RomM Companion");
}
