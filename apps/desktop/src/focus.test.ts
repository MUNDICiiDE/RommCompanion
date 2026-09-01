import { describe, expect, it } from "vitest";
import {
  directionalFocusTarget,
  libraryBottomFocusTarget,
  libraryRequestFocusTarget,
  primaryFocusTarget,
  registeredDirectionalFocusTarget,
  recoverLibraryFocus,
} from "./focus";

describe("spatial focus graph", () => {
  it("chooses deterministic entry controls for every interactive screen", () => {
    const options = {
      connectionTarget: "PAIRING-CODE",
      deviceTarget: "DEVICE-NAME",
      preferencesTarget: "PREF-BACKGROUND",
      refreshTarget: "RETRY-INITIAL-REFRESH",
      firstRomId: 42,
    };
    expect(primaryFocusTarget("startup", options)).toBeNull();
    expect(primaryFocusTarget("connection", options)).toBe("PAIRING-CODE");
    expect(primaryFocusTarget("device", options)).toBe("DEVICE-NAME");
    expect(primaryFocusTarget("preferences", options)).toBe("PREF-BACKGROUND");
    expect(primaryFocusTarget("refresh", options)).toBe("RETRY-INITIAL-REFRESH");
    expect(primaryFocusTarget("library", options)).toBe("ROM-42");
  });

  it("preserves a selected ROM across refresh and append", () => {
    expect(recoverLibraryFocus({
      currentFocusKey: "ROM-2",
      previousRomIds: [1, 2, 3],
      visibleRomIds: [1, 2, 3, 4],
      reset: false,
      hasMore: true,
    })).toBe("ROM-2");
    expect(recoverLibraryFocus({
      currentFocusKey: "REFRESH",
      previousRomIds: [1, 2, 3],
      visibleRomIds: [1, 2, 3],
      reset: true,
      hasMore: false,
    })).toBe("REFRESH");
    expect(recoverLibraryFocus({
      currentFocusKey: "LOAD-MORE",
      previousRomIds: [1, 2, 3],
      visibleRomIds: [1, 2, 3, 4],
      reset: false,
      hasMore: true,
    })).toBe("LOAD-MORE");
    expect(recoverLibraryFocus({
      currentFocusKey: "PERMISSION-DISCLOSURE",
      previousRomIds: [1, 2, 3],
      visibleRomIds: [1, 2, 3],
      reset: true,
      hasMore: false,
      hasPermissionDetails: true,
    })).toBe("PERMISSION-DISCLOSURE");
  });

  it("recovers safely when a focused ROM or load-more control disappears", () => {
    expect(recoverLibraryFocus({
      currentFocusKey: "ROM-2",
      previousRomIds: [1, 2, 3],
      visibleRomIds: [1, 3],
      reset: true,
      hasMore: false,
    })).toBe("ROM-3");
    expect(recoverLibraryFocus({
      currentFocusKey: "LOAD-MORE",
      previousRomIds: [1, 2],
      visibleRomIds: [1, 2, 3],
      reset: false,
      hasMore: false,
    })).toBe("ROM-3");
    expect(recoverLibraryFocus({
      currentFocusKey: "ROM-1",
      previousRomIds: [1],
      visibleRomIds: [],
      reset: true,
      hasMore: false,
    })).toBe("REFRESH");
  });

  it("uses explicit directional links only where the graph defines one", () => {
    const navigation = { up: "TOGGLE-FULLSCREEN", right: "LOGOUT" } as const;
    expect(directionalFocusTarget(navigation, "up")).toBe("TOGGLE-FULLSCREEN");
    expect(directionalFocusTarget(navigation, "right")).toBe("LOGOUT");
    expect(directionalFocusTarget(navigation, "down")).toBeNull();
  });

  it("routes native controller directions through the registered focus graph", () => {
    const nodes = new Map([
      ["CLOSE-TO-TRAY", {
        focusable: true,
        navigation: { up: "ROM-107", right: "CLOSE-TO-QUIT" },
      }],
      ["CLOSE-TO-QUIT", { focusable: true }],
      ["ROM-107", { focusable: true }],
    ]);
    expect(registeredDirectionalFocusTarget("CLOSE-TO-TRAY", "up", nodes)).toBe("ROM-107");
    expect(registeredDirectionalFocusTarget("CLOSE-TO-TRAY", "right", nodes)).toBe("CLOSE-TO-QUIT");
    nodes.set("ROM-107", { focusable: false });
    expect(registeredDirectionalFocusTarget("CLOSE-TO-TRAY", "up", nodes)).toBeNull();
  });

  it("keeps automatic pagination anchored to library content", () => {
    expect(libraryRequestFocusTarget("ROM-3", [1, 2, 3], true)).toBe("ROM-3");
    expect(libraryRequestFocusTarget("CLOSE-TO-TRAY", [1, 2, 3], true)).toBe("ROM-3");
    expect(libraryRequestFocusTarget("LOAD-MORE", [1, 2, 3], false)).toBe("LOAD-MORE");
  });

  it("never routes the footer to a disabled load-more control", () => {
    expect(libraryBottomFocusTarget([1, 2, 3], true, false)).toBe("LOAD-MORE");
    expect(libraryBottomFocusTarget([1, 2, 3], true, true)).toBe("ROM-3");
    expect(libraryBottomFocusTarget([], false, false)).toBe("REFRESH");
  });
});
