import { invoke } from "@tauri-apps/api/core";
import type { AppSettings } from "./types";

export type CloseBehavior = "minimizeToTray" | "quit";

export type DesktopSettings = AppSettings;

export function getDesktopSettings() {
  return invoke<DesktopSettings>("get_desktop_settings");
}

export function updateDesktopSettings(settings: DesktopSettings) {
  return invoke<DesktopSettings>("update_desktop_settings", { settings });
}
