import { describe, expect, it } from "vitest";
import {
  CROSS_SOURCE_DUPLICATE_WINDOW_MS,
  defaultControllerSettings,
  isCrossSourceDuplicate,
  keyboardControllerAction,
  nextControllerBinding,
  shouldHandleControllerAction,
  validateControllerSettings,
} from "./controller";
import type { ControllerAction } from "./types";

const action: ControllerAction = {
  action: "down",
  phase: "pressed",
  controllerId: "controller-1",
  controllerName: "Xbox Controller",
  mappingFamily: "xbox",
  timestampMs: 1_000,
};

describe("native controller delivery", () => {
  it("maps Steam Input keyboard equivalents to normalized actions", () => {
    expect(keyboardControllerAction("ArrowDown")).toBe("down");
    expect(keyboardControllerAction("Enter")).toBe("confirm");
    expect(keyboardControllerAction("/")).toBe("search");
    expect(keyboardControllerAction("PageUp")).toBe("previousTab");
    expect(keyboardControllerAction("q")).toBeNull();
  });

  it("handles presses and directional repeats but ignores releases", () => {
    expect(shouldHandleControllerAction(action)).toBe(true);
    expect(shouldHandleControllerAction({ ...action, phase: "repeated" })).toBe(true);
    expect(shouldHandleControllerAction({ ...action, action: "confirm", phase: "repeated" })).toBe(false);
    expect(shouldHandleControllerAction({ ...action, phase: "released" })).toBe(false);
  });

  it("coalesces matching native and Steam Input actions only inside 30 ms", () => {
    const previous = { action: "down" as const, source: "controller" as const, timestampMs: 1_000 };
    expect(isCrossSourceDuplicate(previous, {
      action: "down",
      source: "keyboard",
      timestampMs: 1_000 + CROSS_SOURCE_DUPLICATE_WINDOW_MS,
    })).toBe(true);
    expect(isCrossSourceDuplicate(previous, {
      action: "down",
      source: "keyboard",
      timestampMs: 1_031,
    })).toBe(false);
    expect(isCrossSourceDuplicate(previous, {
      action: "up",
      source: "keyboard",
      timestampMs: 1_010,
    })).toBe(false);
  });

  it("validates defaults and cycles bindings without collisions", () => {
    const settings = defaultControllerSettings();
    expect(validateControllerSettings(settings)).toBeNull();
    const changed = nextControllerBinding(settings.globalBindings, "confirm");
    expect(changed.confirm).toBeUndefined();
    const withBackCleared = { ...settings.globalBindings, back: undefined };
    expect(nextControllerBinding(withBackCleared, "confirm").confirm).not.toBeUndefined();
    expect(validateControllerSettings({
      ...settings,
      globalBindings: { ...settings.globalBindings, back: "south" },
    })).toMatch(/only once/);
  });
});
