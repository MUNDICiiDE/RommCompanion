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

  it("keeps favorite queue and reconciliation aligned with Rust IPC v21", () => {
    const response: AgentResponse = {
      type: "favoriteQueued",
      mutation: {
        romId: 42,
        desired: true,
        baseFavorite: false,
        baseUpdatedAt: "2026-09-04T12:00:00Z",
        queuedAtMs: 1_725_451_200_000,
        updatedAtMs: 1_725_451_260_000,
      },
    };
    expect(requireSuccessfulResponse(response)).toEqual(response);

    const authenticated: AgentResponse = {
      type: "authenticated",
      result: {
        serverUrl: "https://romm.example.test/",
        tokenKind: "client_api_token",
        tokenId: 4,
        accountId: 7,
        accountName: "justin",
        grantedScopes: ["collections.read", "collections.write"],
        credentialPersisted: true,
        connectionState: "connected",
        favoriteReconciliation: {
          queued: 1,
          attempted: 1,
          applied: 1,
          discarded: 0,
          remaining: 0,
          outcomes: [{
            romId: 42,
            desired: true,
            favorite: true,
            status: "applied",
          }],
        },
      },
    };
    expect(requireSuccessfulResponse(authenticated)).toEqual(authenticated);
  });

  it("keeps mapping browsing and safety aligned with Rust IPC v24", () => {
    const response: AgentResponse = {
      type: "directoryListing",
      listing: {
        currentPath: "C:\\Emulation\\roms",
        parentPath: "C:\\Emulation",
        entries: [{
          name: "gba",
          path: "C:\\Emulation\\roms\\gba",
          isSymlink: false,
        }],
        locations: false,
        truncated: false,
      },
    };
    expect(requireSuccessfulResponse(response)).toEqual(response);

    const safety: AgentResponse = {
      type: "mappingValidation",
      result: {
        valid: true,
        issues: [],
        paths: [{
          draftId: "platform-7",
          field: "romRoot",
          path: "D:\\Emulation\\roms\\gba",
          canonicalPath: "D:\\Emulation\\roms\\gba",
          status: "ready",
          readable: true,
          writable: true,
          availableBytes: 8_589_934_592,
          removable: true,
          mounted: true,
          containsSymlink: false,
        }],
      },
    };
    expect(requireSuccessfulResponse(safety)).toEqual(safety);

    const detection: AgentResponse = {
      type: "mappingDetection",
      result: {
        platforms: [],
        drafts: [],
        evidence: [],
        detectedCount: 0,
        presetUpdates: [{
          draftId: "platform-7",
          platformId: 7,
          presetId: "emudeck",
          fromVersion: 1,
          toVersion: 2,
          updatedFields: ["saveRoots"],
          preservedCustomFields: ["romRoot"],
        }],
      },
    };
    expect(requireSuccessfulResponse(detection)).toEqual(detection);
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
