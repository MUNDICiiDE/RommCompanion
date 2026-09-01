import type { PrimaryView } from "./device";

export type FocusDirection = "up" | "down" | "left" | "right";
export type FocusNavigation = Partial<Record<FocusDirection, string>>;

export interface RegisteredFocusNode {
  focusable: boolean;
  navigation?: FocusNavigation;
}

interface PrimaryFocusOptions {
  connectionTarget: string;
  deviceTarget: string;
  mappingTarget?: string;
  preferencesTarget?: string;
  refreshTarget?: string;
  firstRomId?: number;
}

export function primaryFocusTarget(
  view: PrimaryView,
  options: PrimaryFocusOptions,
): string | null {
  switch (view) {
    case "startup": return null;
    case "connection": return options.connectionTarget;
    case "device": return options.deviceTarget;
    case "mapping": return options.mappingTarget ?? "SCAN-MAPPINGS";
    case "preferences": return options.preferencesTarget ?? "PREF-BACKGROUND";
    case "refresh": return options.refreshTarget ?? "RETRY-INITIAL-REFRESH";
    case "library": return options.firstRomId === undefined ? "REFRESH" : `ROM-${options.firstRomId}`;
  }
}

interface LibraryFocusOptions {
  currentFocusKey: string | null;
  previousRomIds: number[];
  visibleRomIds: number[];
  reset: boolean;
  hasMore: boolean;
  hasPermissionDetails?: boolean;
}

export function recoverLibraryFocus({
  currentFocusKey,
  previousRomIds,
  visibleRomIds,
  reset,
  hasMore,
  hasPermissionDetails = false,
}: LibraryFocusOptions): string {
  const visibleKeys = new Set(visibleRomIds.map((id) => `ROM-${id}`));
  if (currentFocusKey && visibleKeys.has(currentFocusKey)) return currentFocusKey;
  const persistentKeys = [
    "REFRESH",
    "LOGOUT",
    "DEVICE-SETTINGS",
    "CONTROLLER-SETTINGS",
    "LIBRARY-SEARCH",
    "CLOSE-TO-TRAY",
    "CLOSE-TO-QUIT",
  ];
  if (hasPermissionDetails) persistentKeys.push("PERMISSION-DISCLOSURE");
  if (hasMore) persistentKeys.push("LOAD-MORE");
  if (persistentKeys.includes(currentFocusKey ?? "")) {
    return currentFocusKey as string;
  }

  if (!reset && currentFocusKey === "LOAD-MORE" && !hasMore && visibleRomIds.length > 0) {
    return `ROM-${visibleRomIds.at(-1)}`;
  }

  if (currentFocusKey?.startsWith("ROM-") && visibleRomIds.length > 0) {
    const removedId = Number(currentFocusKey.slice(4));
    const previousIndex = previousRomIds.indexOf(removedId);
    const nextIndex = Math.max(0, Math.min(previousIndex, visibleRomIds.length - 1));
    return `ROM-${visibleRomIds[nextIndex]}`;
  }

  if (visibleRomIds.length > 0) return `ROM-${visibleRomIds[0]}`;
  if (hasMore) return "LOAD-MORE";
  return "REFRESH";
}

export function directionalFocusTarget(
  navigation: FocusNavigation | undefined,
  direction: FocusDirection,
): string | null {
  return navigation?.[direction] ?? null;
}

export function registeredDirectionalFocusTarget(
  currentFocusKey: string | null,
  direction: FocusDirection,
  nodes: ReadonlyMap<string, RegisteredFocusNode>,
): string | null {
  if (!currentFocusKey) return null;
  const target = directionalFocusTarget(nodes.get(currentFocusKey)?.navigation, direction);
  return target && nodes.get(target)?.focusable ? target : null;
}

export function libraryRequestFocusTarget(
  currentFocusKey: string | null,
  visibleRomIds: number[],
  automatic: boolean,
): string | null {
  if (!automatic) return currentFocusKey;
  if (currentFocusKey?.startsWith("ROM-") && visibleRomIds.includes(Number(currentFocusKey.slice(4)))) {
    return currentFocusKey;
  }
  const lastRomId = visibleRomIds.at(-1);
  return lastRomId === undefined ? currentFocusKey : `ROM-${lastRomId}`;
}

export function libraryBottomFocusTarget(
  visibleRomIds: number[],
  hasMore: boolean,
  loadMoreBusy: boolean,
): string {
  if (hasMore && !loadMoreBusy) return "LOAD-MORE";
  const lastRomId = visibleRomIds.at(-1);
  return lastRomId === undefined ? "REFRESH" : `ROM-${lastRomId}`;
}
