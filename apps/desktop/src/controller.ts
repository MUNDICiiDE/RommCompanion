import { invoke } from "@tauri-apps/api/core";
import type {
  ControllerAction,
  ControllerBindings,
  ControllerButton,
  ControllerSettings,
  ControllerStatus,
} from "./types";

export const CROSS_SOURCE_DUPLICATE_WINDOW_MS = 30;
export const CONTROLLER_BUTTONS: ControllerButton[] = [
  "south",
  "east",
  "west",
  "north",
  "left_shoulder",
  "right_shoulder",
];

export const DEFAULT_CONTROLLER_BINDINGS: Required<ControllerBindings> = {
  confirm: "south",
  back: "east",
  context: "west",
  search: "north",
  previousTab: "left_shoulder",
  nextTab: "right_shoulder",
};

export type ControllerBindingAction = keyof ControllerBindings;

export function nextControllerBinding(
  bindings: ControllerBindings,
  action: ControllerBindingAction,
): ControllerBindings {
  const current = bindings[action];
  const candidates: Array<ControllerButton | undefined> = [undefined, ...CONTROLLER_BUTTONS];
  const start = candidates.findIndex((candidate) => candidate === current);
  const used = new Set(
    Object.entries(bindings)
      .filter(([key]) => key !== action)
      .map(([, button]) => button)
      .filter((button): button is ControllerButton => Boolean(button)),
  );
  for (let offset = 1; offset <= candidates.length; offset += 1) {
    const candidate = candidates[(start + offset + candidates.length) % candidates.length];
    if (candidate && used.has(candidate)) continue;
    if (!candidate && (action === "confirm" || action === "back")) {
      const other = action === "confirm" ? bindings.back : bindings.confirm;
      if (!other) continue;
    }
    return { ...bindings, [action]: candidate };
  }
  return bindings;
}

export function defaultControllerSettings(): ControllerSettings {
  return {
    deadZonePercent: 25,
    initialRepeatDelayMs: 350,
    repeatIntervalMs: 100,
    globalBindings: { ...DEFAULT_CONTROLLER_BINDINGS },
    controllerBindings: {},
  };
}

export function validateControllerSettings(settings: ControllerSettings): string | null {
  if (settings.deadZonePercent < 10 || settings.deadZonePercent > 50) {
    return "Controller dead zone must be between 10% and 50%.";
  }
  if (settings.initialRepeatDelayMs < 200 || settings.initialRepeatDelayMs > 1000) {
    return "Initial repeat delay must be between 200 and 1000 ms.";
  }
  if (settings.repeatIntervalMs < 50 || settings.repeatIntervalMs > 300) {
    return "Repeat interval must be between 50 and 300 ms.";
  }
  for (const bindings of [settings.globalBindings, ...Object.values(settings.controllerBindings)]) {
    if (!bindings.confirm && !bindings.back) return "Confirm and Back cannot both be unbound.";
    const assigned = Object.values(bindings).filter((button): button is ControllerButton => Boolean(button));
    if (new Set(assigned).size !== assigned.length) {
      return "Each physical controller button can be assigned only once.";
    }
  }
  return null;
}

export function effectiveControllerBindings(
  settings: ControllerSettings,
  guid?: string,
): ControllerBindings {
  return guid && settings.controllerBindings[guid]
    ? settings.controllerBindings[guid]
    : settings.globalBindings;
}

export function getControllerStatus() {
  return invoke<ControllerStatus | null>("get_controller_status");
}

export interface RecentInputAction {
  action: ControllerAction["action"];
  source: "controller" | "keyboard";
  timestampMs: number;
}

export function keyboardControllerAction(key: string): ControllerAction["action"] | null {
  switch (key) {
    case "ArrowUp": return "up";
    case "ArrowDown": return "down";
    case "ArrowLeft": return "left";
    case "ArrowRight": return "right";
    case "Enter": return "confirm";
    case "Escape": return "back";
    case "x":
    case "X": return "context";
    case "/": return "search";
    case "PageUp": return "previousTab";
    case "PageDown": return "nextTab";
    default: return null;
  }
}

export function shouldHandleControllerAction(action: ControllerAction): boolean {
  if (action.phase === "released") return false;
  if (action.phase === "pressed") return true;
  return ["up", "down", "left", "right"].includes(action.action);
}

export function isCrossSourceDuplicate(
  previous: RecentInputAction | null,
  current: RecentInputAction,
): boolean {
  return previous !== null &&
    previous.source !== current.source &&
    previous.action === current.action &&
    Math.abs(current.timestampMs - previous.timestampMs) <= CROSS_SOURCE_DUPLICATE_WINDOW_MS;
}
