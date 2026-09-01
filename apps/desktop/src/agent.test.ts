import { describe, expect, it } from "vitest";
import { requireSuccessfulResponse } from "./agent";
import type { AgentResponse } from "./types";

describe("agent response handling", () => {
  it("returns successful responses", () => {
    const response: AgentResponse = {
      type: "roms",
      page: {
        items: [], offset: 0, limit: 48, total: 0, hasMore: false,
        source: "live", refreshedAtMs: 1234, stale: false,
      },
    };
    expect(requireSuccessfulResponse(response)).toBe(response);
  });

  it("keeps typed library view responses aligned with Rust IPC v16", () => {
    const response: AgentResponse = {
      type: "libraryPage",
      query: { kind: "smart_collection", id: 9 },
      page: {
        items: [], offset: 0, limit: 48, total: 0, hasMore: false,
        source: "cache", refreshedAtMs: 1234, stale: false,
      },
    };
    expect(requireSuccessfulResponse(response)).toEqual(response);
  });

  it("keeps the initial-refresh completion contract aligned with Rust IPC", () => {
    const response: AgentResponse = {
      type: "initialRefresh",
      result: {
        page: {
          items: [{
            id: 42,
            title: "Chrono Trigger",
            platform: "SNES",
            artwork: [],
            collectionIds: [],
            user: {
              romId: 42,
              favorite: false,
              backlogged: false,
              hidden: false,
              rating: 0,
              difficulty: 0,
              completion: 0,
            },
            localStatus: "remote_only",
          }],
          offset: 0,
          limit: 48,
          total: 107,
          hasMore: true,
          source: "live",
          refreshedAtMs: 1234,
          stale: false,
        },
        onboarding: {
          version: 1,
          currentStep: "first_refresh",
          highestCompletedStep: "first_refresh",
          completedSteps: [
            "server",
            "authentication",
            "permissions",
            "device",
            "detection",
            "mappings",
            "background",
            "first_refresh",
          ],
          serverOrigin: "https://romm.example.test",
          acknowledgedHttpWarningOrigin: null,
          selectedPlatformIds: [],
          mappingDraftIds: [],
          customPathDrafts: {},
          backgroundEnabled: false,
          cancelled: false,
        },
      },
    };

    expect(requireSuccessfulResponse(response)).toBe(response);
  });

  it("keeps the device proposal contract aligned with Rust IPC", () => {
    const response: AgentResponse = {
      type: "deviceProposed",
      device: {
        localId: "4a33ec5d-1d66-41bd-9c0c-cfe8a56ab635",
        rommDeviceId: null,
        displayName: "Windows PC - JUSTIN-DESKTOP",
        platform: "windows",
        hostname: "JUSTIN-DESKTOP",
        client: "romm-companion",
        clientVersion: "0.1.0",
        syncMode: "push_pull",
        registrationFingerprint: null,
        registrationState: "unregistered",
        mappingSummary: {},
        createdAtMs: 1234,
        registeredAtMs: null,
        verifiedAtMs: null,
        updatedAtMs: 1234,
      },
    };

    expect(requireSuccessfulResponse(response)).toEqual(response);
  });

  it("keeps the registered-device response aligned with Rust IPC", () => {
    const response: AgentResponse = {
      type: "deviceRegistered",
      newlyRegistered: true,
      device: {
        localId: "4a33ec5d-1d66-41bd-9c0c-cfe8a56ab635",
        rommDeviceId: "device-123",
        displayName: "Living Room PC",
        platform: "windows",
        hostname: "JUSTIN-DESKTOP",
        client: "romm-companion",
        clientVersion: "0.1.0",
        syncMode: "push_pull",
        registrationFingerprint: "abc123",
        registrationState: "registered",
        mappingSummary: {},
        createdAtMs: 1234,
        registeredAtMs: 5678,
        verifiedAtMs: null,
        updatedAtMs: 5678,
      },
    };

    expect(requireSuccessfulResponse(response)).toEqual(response);
  });

  it("throws the safe agent message for errors", () => {
    expect(() =>
      requireSuccessfulResponse({
        type: "error",
        error: {
          code: "network_error",
          message: "Unable to reach RomM.",
          retryable: true,
        },
      }),
    ).toThrow("Unable to reach RomM.");
  });

  it("keeps device verification outcomes aligned with Rust IPC", () => {
    const response: AgentResponse = {
      type: "deviceVerification",
      outcome: "missing",
      device: {
        localId: "4a33ec5d-1d66-41bd-9c0c-cfe8a56ab635",
        rommDeviceId: "device-123",
        displayName: "Living Room PC",
        platform: "windows",
        hostname: "JUSTIN-DESKTOP",
        client: "romm-companion",
        clientVersion: "0.1.0",
        syncMode: "push_pull",
        registrationFingerprint: "abc123",
        registrationState: "missing",
        mappingSummary: {},
        createdAtMs: 1234,
        registeredAtMs: 5678,
        verifiedAtMs: 6789,
        updatedAtMs: 7890,
      },
    };

    expect(requireSuccessfulResponse(response)).toEqual(response);
  });

  it("keeps device updates and best-effort logout outcomes aligned with Rust IPC", () => {
    const loggedOut: AgentResponse = {
      type: "loggedOut",
      deviceRemoval: {
        outcome: "failed",
        deviceId: "device-123",
        error: {
          code: "network_error",
          message: "RomM is unavailable.",
          retryable: true,
        },
      },
    };

    expect(requireSuccessfulResponse(loggedOut)).toEqual(loggedOut);
    expect(requireSuccessfulResponse({
      type: "deviceUpdated",
      device: {
        localId: "local-123",
        rommDeviceId: "device-123",
        displayName: "Arcade Room",
        platform: "windows",
        hostname: "JUSTIN-DESKTOP",
        client: "romm-companion",
        clientVersion: "0.1.0",
        syncMode: "push_pull",
        registrationFingerprint: "abc123",
        registrationState: "registered",
        mappingSummary: {},
        createdAtMs: 1234,
        registeredAtMs: 5678,
        verifiedAtMs: 6789,
        updatedAtMs: 7890,
      },
    }).type).toBe("deviceUpdated");
  });
});
