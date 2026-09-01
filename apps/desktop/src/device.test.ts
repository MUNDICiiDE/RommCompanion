import { describe, expect, it } from "vitest";
import {
  DEVICE_DISPLAY_NAME_MAX_LENGTH,
  DEVICE_RENAME_DEBOUNCE_MS,
  devicePlatformLabel,
  normalizePendingDeviceRename,
  resolvePrimaryView,
  validateDeviceDisplayName,
} from "./device";

describe("device registration onboarding", () => {
  it("uses the same display-name boundaries as the Rust core", () => {
    expect(validateDeviceDisplayName(" Living Room PC ")).toBeNull();
    expect(validateDeviceDisplayName("   ")).toBe("Enter a name for this device.");
    expect(validateDeviceDisplayName("Living\nRoom")).toBe(
      "Device names cannot contain control characters.",
    );
    expect(validateDeviceDisplayName("x".repeat(DEVICE_DISPLAY_NAME_MAX_LENGTH + 1))).toBe(
      "Device names must be 255 characters or fewer.",
    );
  });

  it("presents readable platform names", () => {
    expect(devicePlatformLabel("windows")).toBe("Windows");
    expect(devicePlatformLabel("steamos")).toBe("Steam Deck / SteamOS");
    expect(devicePlatformLabel("linux")).toBe("Linux");
  });

  it("coalesces only valid changed names on the five-second boundary", () => {
    expect(DEVICE_RENAME_DEBOUNCE_MS).toBe(5_000);
    expect(normalizePendingDeviceRename(" Arcade Room ", "Living Room PC")).toBe(
      "Arcade Room",
    );
    expect(normalizePendingDeviceRename(" Living Room PC ", "Living Room PC")).toBeNull();
    expect(normalizePendingDeviceRename("   ", "Living Room PC")).toBeNull();
  });

  it("never flashes connection UI while restoring an authenticated device", () => {
    expect(resolvePrimaryView({
      statusPending: true,
      restoringCredential: false,
      restorableCredential: false,
      connected: false,
      agentConnected: false,
      deviceResolved: false,
      deviceReady: false,
    })).toBe("startup");
    expect(resolvePrimaryView({
      statusPending: false,
      restoringCredential: false,
      restorableCredential: false,
      connected: false,
      agentConnected: true,
      deviceResolved: false,
      deviceReady: false,
    })).toBe("startup");
    expect(resolvePrimaryView({
      statusPending: false,
      restoringCredential: false,
      restorableCredential: true,
      connected: false,
      agentConnected: false,
      deviceResolved: false,
      deviceReady: true,
    })).toBe("startup");
  });

  it("routes authenticated installations by registration state", () => {
    const authenticated = {
      statusPending: false,
      restoringCredential: false,
      restorableCredential: false,
      connected: true,
      agentConnected: true,
    };
    expect(resolvePrimaryView({
      ...authenticated,
      deviceResolved: true,
      deviceReady: false,
    })).toBe("device");
    expect(resolvePrimaryView({
      ...authenticated,
      deviceResolved: true,
      deviceReady: true,
    })).toBe("library");
  });

  it("routes a completed local logout to authentication despite stale agent status", () => {
    expect(resolvePrimaryView({
      signedOutLocally: true,
      statusPending: false,
      restoringCredential: false,
      restorableCredential: false,
      connected: false,
      agentConnected: true,
      deviceResolved: false,
      deviceReady: false,
    })).toBe("connection");
  });
});
