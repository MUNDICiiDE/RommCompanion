use std::{
    fs,
    path::PathBuf,
    sync::{
        RwLock,
        atomic::{AtomicBool, Ordering},
    },
};

use romm_ipc::{AgentRequest, AgentResponse};
pub use romm_ipc::{AppSettings as DesktopSettings, CloseBehavior};
use tauri::{AppHandle, Manager, State};

use crate::AgentClient;

pub struct DesktopSettingsState {
    path: PathBuf,
    settings: RwLock<DesktopSettings>,
    quitting: AtomicBool,
}

impl DesktopSettingsState {
    pub fn load(app: &AppHandle) -> Result<Self, String> {
        let config_root = app.path().config_dir().map_err(|error| {
            format!("Unable to resolve the application config directory: {error}")
        })?;
        #[cfg(target_os = "windows")]
        let config_directory = config_root.join("RommCompanion");
        #[cfg(not(target_os = "windows"))]
        let config_directory = config_root.join("romm-companion");
        let path = config_directory.join("desktop-settings.json");
        let settings = match fs::read(&path) {
            Ok(bytes) => {
                let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
                let fullscreen_was_saved = value.get("fullscreen").is_some();
                let mut settings: DesktopSettings =
                    serde_json::from_value(value).unwrap_or_default();
                if !fullscreen_was_saved {
                    settings.fullscreen = is_steam_deck();
                }
                settings
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => DesktopSettings {
                fullscreen: is_steam_deck(),
                ..DesktopSettings::default()
            },
            Err(error) => return Err(format!("Unable to read desktop settings: {error}")),
        };

        Ok(Self {
            path,
            settings: RwLock::new(settings),
            quitting: AtomicBool::new(false),
        })
    }

    pub fn close_behavior(&self) -> CloseBehavior {
        self.settings
            .read()
            .map(|settings| settings.close_behavior)
            .unwrap_or_default()
    }

    pub fn fullscreen(&self) -> bool {
        self.settings
            .read()
            .map(|settings| settings.fullscreen)
            .unwrap_or(false)
    }

    pub fn is_quitting(&self) -> bool {
        self.quitting.load(Ordering::Acquire)
    }

    pub fn mark_quitting(&self) {
        self.quitting.store(true, Ordering::Release);
    }

    pub fn snapshot(&self) -> Result<DesktopSettings, String> {
        self.settings
            .read()
            .map(|settings| settings.clone())
            .map_err(|_| "Desktop settings are temporarily unavailable.".to_owned())
    }

    fn update_memory(&self, settings: DesktopSettings) -> Result<DesktopSettings, String> {
        *self
            .settings
            .write()
            .map_err(|_| "Desktop settings are temporarily unavailable.".to_owned())? =
            settings.clone();
        Ok(settings)
    }

    fn finish_legacy_migration(&self) {
        if self.path.exists() {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[tauri::command]
pub async fn get_desktop_settings(
    app: AppHandle,
    state: State<'_, DesktopSettingsState>,
    agent: State<'_, AgentClient>,
) -> Result<DesktopSettings, String> {
    let response = agent.send(AgentRequest::GetSettings).await?.body;
    let settings = match response {
        AgentResponse::Settings {
            settings: Some(settings),
        } => settings,
        AgentResponse::Settings { settings: None } => {
            let initial = state.snapshot()?;
            match agent
                .send(AgentRequest::UpdateSettings {
                    settings: initial.clone(),
                })
                .await?
                .body
            {
                AgentResponse::Settings {
                    settings: Some(settings),
                } => settings,
                AgentResponse::Error { error } => return Err(error.message),
                _ => return Err("The agent returned an unexpected settings response.".to_owned()),
            }
        }
        AgentResponse::Error { error } => return Err(error.message),
        _ => return Err("The agent returned an unexpected settings response.".to_owned()),
    };
    settings.validate()?;

    if let Some(window) = app.get_webview_window("main") {
        window
            .set_fullscreen(settings.fullscreen)
            .map_err(|error| format!("Unable to restore fullscreen mode: {error}"))?;
    }
    let settings = state.update_memory(settings)?;
    state.finish_legacy_migration();
    Ok(settings)
}

#[tauri::command]
pub async fn update_desktop_settings(
    app: AppHandle,
    state: State<'_, DesktopSettingsState>,
    agent: State<'_, AgentClient>,
    settings: DesktopSettings,
) -> Result<DesktopSettings, String> {
    settings.validate()?;
    let persisted = match agent
        .send(AgentRequest::UpdateSettings {
            settings: settings.clone(),
        })
        .await?
        .body
    {
        AgentResponse::Settings {
            settings: Some(settings),
        } => settings,
        AgentResponse::Error { error } => return Err(error.message),
        _ => return Err("The agent returned an unexpected settings response.".to_owned()),
    };
    if let Some(window) = app.get_webview_window("main") {
        window
            .set_fullscreen(persisted.fullscreen)
            .map_err(|error| format!("Unable to change fullscreen mode: {error}"))?;
    }
    let persisted = state.update_memory(persisted)?;
    state.finish_legacy_migration();
    Ok(persisted)
}

#[cfg(target_os = "linux")]
fn is_steam_deck() -> bool {
    if std::env::var("SteamDeck").is_ok_and(|value| value == "1") {
        return true;
    }

    let dmi_paths = [
        "/sys/class/dmi/id/sys_vendor",
        "/sys/class/dmi/id/board_vendor",
        "/sys/class/dmi/id/product_name",
        "/sys/class/dmi/id/board_name",
    ];
    let dmi = dmi_paths
        .iter()
        .filter_map(|path| fs::read_to_string(path).ok())
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    if dmi.contains("valve") || dmi.contains("jupiter") || dmi.contains("galileo") {
        return true;
    }

    fs::read_to_string("/etc/os-release").is_ok_and(|release| {
        release.lines().any(|line| {
            line.split_once('=').is_some_and(|(key, value)| {
                key.trim().eq_ignore_ascii_case("VARIANT_ID")
                    && value
                        .trim()
                        .trim_matches('"')
                        .eq_ignore_ascii_case("steamdeck")
            })
        })
    })
}

#[cfg(not(target_os = "linux"))]
fn is_steam_deck() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_use_the_frontend_contract() {
        let default_json =
            serde_json::to_string(&DesktopSettings::default()).expect("settings should serialize");
        assert!(default_json.contains(r#""closeBehavior":"minimizeToTray""#));
        assert!(default_json.contains(r#""deadZonePercent":25"#));
        let settings: DesktopSettings =
            serde_json::from_str(r#"{"closeBehavior":"quit","fullscreen":true}"#)
                .expect("frontend settings should deserialize");
        assert!(matches!(settings.close_behavior, CloseBehavior::Quit));
        assert!(settings.fullscreen);
        assert!(!settings.stable_update_checks_enabled);
        assert_eq!(settings.controller.initial_repeat_delay_ms, 350);
        assert!(settings.validate().is_ok());
    }
}
