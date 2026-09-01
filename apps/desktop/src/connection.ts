export const REQUIRED_SCOPES = [
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
] as const;

export function formatPairingCode(value: string) {
  const compact = value.toUpperCase().replace(/[^A-Z0-9]/g, "").slice(0, 8);
  return compact.length > 4 ? `${compact.slice(0, 4)}-${compact.slice(4)}` : compact;
}

export function remembersHttpApproval(
  serverUrl: string,
  approvedOrigin: string | undefined,
  approved: boolean,
) {
  if (!approved || !approvedOrigin) return false;
  try {
    return new URL(serverUrl).origin === approvedOrigin;
  } catch {
    return false;
  }
}
