use std::{path::Path, process::Command};

pub struct RegistrationOutcome {
    pub enabled: bool,
    pub method: &'static str,
}

pub fn configure(enabled: bool) -> Result<RegistrationOutcome, String> {
    configure_platform(enabled)
}

pub fn method_name() -> &'static str {
    if cfg!(windows) {
        "windows_user_startup"
    } else if cfg!(target_os = "linux") {
        "systemd_user"
    } else {
        "unsupported"
    }
}

fn validate_agent_executable(path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("The background agent path is not absolute.".to_owned());
    }
    if !path.is_file() {
        return Err(format!(
            "The background agent executable is missing from {}.",
            path.display()
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn configure_platform(enabled: bool) -> Result<RegistrationOutcome, String> {
    const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
    const VALUE_NAME: &str = "RomM Companion Sync Agent";

    let executable = std::env::current_exe()
        .map_err(|error| format!("Unable to locate the background agent: {error}"))?;
    validate_agent_executable(&executable)?;
    let command = windows_startup_command(&executable)?;
    let status = if enabled {
        Command::new("reg.exe")
            .args([
                "ADD", RUN_KEY, "/v", VALUE_NAME, "/t", "REG_SZ", "/d", &command, "/f",
            ])
            .status()
    } else {
        let query = Command::new("reg.exe")
            .args(["QUERY", RUN_KEY, "/v", VALUE_NAME])
            .status()
            .map_err(|error| format!("Unable to inspect Windows startup registration: {error}"))?;
        if !query.success() {
            return Ok(RegistrationOutcome {
                enabled: false,
                method: "windows_user_startup",
            });
        }
        Command::new("reg.exe")
            .args(["DELETE", RUN_KEY, "/v", VALUE_NAME, "/f"])
            .status()
    }
    .map_err(|error| format!("Unable to update Windows startup registration: {error}"))?;

    if !status.success() {
        return Err(format!(
            "Windows rejected the per-user startup registration (exit code {}).",
            status.code().unwrap_or(-1)
        ));
    }
    Ok(RegistrationOutcome {
        enabled,
        method: "windows_user_startup",
    })
}

#[cfg(windows)]
fn windows_startup_command(executable: &Path) -> Result<String, String> {
    let value = executable
        .to_str()
        .ok_or_else(|| "The background agent path is not valid Unicode.".to_owned())?;
    if value.contains('"') {
        return Err(
            "The background agent path contains an unsupported quote character.".to_owned(),
        );
    }
    Ok(format!("\"{value}\" --service"))
}

#[cfg(target_os = "linux")]
fn configure_platform(enabled: bool) -> Result<RegistrationOutcome, String> {
    use std::{
        fs,
        os::unix::fs::{PermissionsExt, symlink},
    };

    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .ok_or_else(|| "HOME is unavailable for the user service.".to_owned())?;
    let unit_dir = home.join(".config/systemd/user");
    let unit_path = unit_dir.join("romm-companion.service");

    if !enabled {
        if unit_path.exists() {
            let disabled = Command::new("systemctl")
                .args(["--user", "disable", "romm-companion.service"])
                .status()
                .map_err(|error| format!("Unable to disable the SteamOS user service: {error}"))?;
            if !disabled.success() {
                return Err("SteamOS could not disable background startup.".to_owned());
            }
            fs::remove_file(&unit_path).map_err(|error| {
                format!("Unable to remove the user service definition: {error}")
            })?;
            let _ = Command::new("systemctl")
                .args(["--user", "daemon-reload"])
                .status();
        }
        return Ok(RegistrationOutcome {
            enabled: false,
            method: "systemd_user",
        });
    }

    let source = std::env::current_exe()
        .map_err(|error| format!("Unable to locate the background agent: {error}"))?;
    validate_agent_executable(&source)?;
    let version_dir = home
        .join(".local/lib/romm-companion")
        .join(env!("CARGO_PKG_VERSION"));
    fs::create_dir_all(&version_dir)
        .map_err(|error| format!("Unable to create the versioned agent directory: {error}"))?;
    let installed = version_dir.join("romm-sync-agent");
    let staged = version_dir.join("romm-sync-agent.tmp");
    fs::copy(&source, &staged)
        .map_err(|error| format!("Unable to stage the background agent: {error}"))?;
    fs::set_permissions(&staged, fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("Unable to secure the staged background agent: {error}"))?;
    fs::rename(&staged, &installed)
        .map_err(|error| format!("Unable to activate the staged background agent: {error}"))?;

    let root = home.join(".local/lib/romm-companion");
    let current_tmp = root.join("current.tmp");
    let current = root.join("current");
    let _ = fs::remove_file(&current_tmp);
    symlink(&version_dir, &current_tmp)
        .map_err(|error| format!("Unable to stage the current-agent link: {error}"))?;
    if current.exists() || current.symlink_metadata().is_ok() {
        fs::remove_file(&current)
            .map_err(|error| format!("Unable to replace the current-agent link: {error}"))?;
    }
    fs::rename(&current_tmp, &current)
        .map_err(|error| format!("Unable to activate the current-agent link: {error}"))?;

    fs::create_dir_all(&unit_dir)
        .map_err(|error| format!("Unable to create the user service directory: {error}"))?;
    let service_executable = current.join("romm-sync-agent");
    let service = render_systemd_unit(&service_executable)?;
    let staged_unit = unit_dir.join("romm-companion.service.tmp");
    fs::write(&staged_unit, service)
        .map_err(|error| format!("Unable to stage the user service: {error}"))?;
    fs::rename(&staged_unit, &unit_path)
        .map_err(|error| format!("Unable to activate the user service: {error}"))?;

    for args in [["--user", "daemon-reload"], ["--user", "enable"]] {
        let mut command = Command::new("systemctl");
        if args[1] == "daemon-reload" {
            command.args([args[0], args[1]]);
        } else {
            command.args([args[0], args[1], "romm-companion.service"]);
        }
        let status = command
            .status()
            .map_err(|error| format!("Unable to configure the SteamOS user service: {error}"))?;
        if !status.success() {
            return Err("SteamOS could not enable the background agent user service.".to_owned());
        }
    }
    Ok(RegistrationOutcome {
        enabled: true,
        method: "systemd_user",
    })
}

#[cfg(target_os = "linux")]
fn render_systemd_unit(executable: &Path) -> Result<String, String> {
    let value = executable
        .to_str()
        .ok_or_else(|| "The background agent path is not valid Unicode.".to_owned())?;
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    Ok(format!(
        "[Unit]\nDescription=RomM Companion background agent\nAfter=network-online.target\n\n[Service]\nType=simple\nExecStart=\"{escaped}\" --service\nRestart=on-failure\nRestartSec=5\n\n[Install]\nWantedBy=default.target\n"
    ))
}

#[cfg(not(any(windows, target_os = "linux")))]
fn configure_platform(_enabled: bool) -> Result<RegistrationOutcome, String> {
    Err("Background startup is not supported on this platform in v1.".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_missing_or_relative_agent_paths() {
        assert!(validate_agent_executable(Path::new("romm-sync-agent")).is_err());
        assert!(validate_agent_executable(Path::new("Z:\\definitely-missing\\agent.exe")).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windows_registration_quotes_paths_with_spaces() {
        let command = windows_startup_command(Path::new(
            r"C:\Program Files\RomM Companion\romm-sync-agent.exe",
        ))
        .expect("valid Windows path");
        assert_eq!(
            command,
            r#""C:\Program Files\RomM Companion\romm-sync-agent.exe" --service"#
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn systemd_unit_quotes_the_stable_agent_path() {
        let unit = render_systemd_unit(Path::new(
            "/home/deck/.local/lib/romm-companion/current/romm-sync-agent",
        ))
        .expect("valid Linux path");
        assert!(unit.contains(
            "ExecStart=\"/home/deck/.local/lib/romm-companion/current/romm-sync-agent\" --service"
        ));
        assert!(unit.contains("WantedBy=default.target"));
    }
}
