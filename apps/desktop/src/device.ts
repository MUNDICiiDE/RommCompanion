import type { DeviceIdentity } from "./types";

export const DEVICE_DISPLAY_NAME_MAX_LENGTH = 255;
export const DEVICE_RENAME_DEBOUNCE_MS = 5_000;

export type PrimaryView =
  | "startup"
  | "connection"
  | "device"
  | "mapping"
  | "preferences"
  | "refresh"
  | "library";

interface PrimaryViewState {
  signedOutLocally?: boolean;
  statusPending: boolean;
  restoringCredential: boolean;
  restorableCredential: boolean;
  connected: boolean;
  agentConnected: boolean;
  deviceResolved: boolean;
  deviceReady: boolean;
}

export function resolvePrimaryView(state: PrimaryViewState): PrimaryView {
  if (state.signedOutLocally) return "connection";
  if (
    (state.statusPending || state.restoringCredential || state.restorableCredential) &&
    !state.connected
  ) return "startup";
  if (!state.connected && !state.agentConnected) return "connection";
  if (!state.deviceResolved) return "startup";
  if (!state.deviceReady) return "device";
  return "library";
}

export function validateDeviceDisplayName(value: string): string | null {
  const trimmed = value.trim();
  if (!trimmed) return "Enter a name for this device.";
  if (trimmed.length > DEVICE_DISPLAY_NAME_MAX_LENGTH) {
    return `Device names must be ${DEVICE_DISPLAY_NAME_MAX_LENGTH} characters or fewer.`;
  }
  if (/\p{Cc}/u.test(trimmed)) return "Device names cannot contain control characters.";
  return null;
}

export function normalizePendingDeviceRename(
  value: string,
  persistedValue: string,
): string | null {
  if (validateDeviceDisplayName(value)) return null;
  const normalized = value.trim();
  return normalized === persistedValue ? null : normalized;
}

export function devicePlatformLabel(platform: DeviceIdentity["platform"]): string {
  switch (platform) {
    case "windows":
      return "Windows";
    case "steamos":
      return "Steam Deck / SteamOS";
    case "linux":
      return "Linux";
  }
}
