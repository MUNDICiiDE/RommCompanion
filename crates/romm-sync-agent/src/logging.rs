use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

use romm_core::{redaction::redact, storage::AppPaths};
use serde_json::json;

const MAX_LOG_BYTES: u64 = 10 * 1024 * 1024;
const RETAINED_ARCHIVES: usize = 6;
static LOGGER: OnceLock<Mutex<RotatingLogger>> = OnceLock::new();

struct RotatingLogger {
    directory: PathBuf,
}

pub fn initialize(paths: &AppPaths) {
    let _ = LOGGER.set(Mutex::new(RotatingLogger {
        directory: paths.log_dir.clone(),
    }));
}

pub fn info(event: &str, message: &str) {
    write("info", event, message);
}

pub fn error(event: &str, message: &str) {
    write("error", event, message);
}

fn write(level: &str, event: &str, message: &str) {
    let Some(logger) = LOGGER.get() else {
        return;
    };
    let Ok(logger) = logger.lock() else {
        return;
    };
    let line = json!({
        "timestampMs": now_ms(),
        "level": level,
        "event": event,
        "message": redact(message),
    })
    .to_string()
        + "\n";
    let current = logger.directory.join("agent.log");
    if current
        .metadata()
        .is_ok_and(|metadata| metadata.len().saturating_add(line.len() as u64) > MAX_LOG_BYTES)
    {
        rotate(&logger.directory);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(current) {
        let _ = file.write_all(line.as_bytes());
    }
}

fn rotate(directory: &std::path::Path) {
    let oldest = directory.join(format!("agent.{RETAINED_ARCHIVES}.log"));
    let _ = fs::remove_file(oldest);
    for index in (1..RETAINED_ARCHIVES).rev() {
        let source = directory.join(format!("agent.{index}.log"));
        let destination = directory.join(format!("agent.{}.log", index + 1));
        if source.exists() {
            let _ = fs::rename(source, destination);
        }
    }
    let current = directory.join("agent.log");
    if current.exists() {
        let _ = fs::rename(current, directory.join("agent.1.log"));
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
