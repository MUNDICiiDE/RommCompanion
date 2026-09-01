import { describe, expect, it } from "vitest";
import {
  firstIncompleteOnboardingStep,
  isCompatibleOnboardingState,
  onboardingProgress,
  onboardingStepStates,
  ONBOARDING_STEPS,
  previousOnboardingStep,
  requiresInitialRefresh,
} from "./onboarding";
import type { OnboardingState } from "./types";

function state(completedSteps: OnboardingState["completedSteps"] = []): OnboardingState {
  return {
    version: 1,
    currentStep: firstIncompleteOnboardingStep({ completedSteps } as OnboardingState),
    highestCompletedStep: completedSteps.at(-1) ?? null,
    completedSteps,
    serverOrigin: completedSteps.includes("server") ? "https://romm.example.test" : null,
    acknowledgedHttpWarningOrigin: null,
    selectedPlatformIds: [],
    mappingDraftIds: [],
    customPathDrafts: {},
    backgroundEnabled: null,
    cancelled: false,
  };
}

describe("resumable onboarding client state", () => {
  it("uses the same ordered eight steps as the agent contract", () => {
    expect(ONBOARDING_STEPS).toEqual([
      "server",
      "authentication",
      "permissions",
      "device",
      "detection",
      "mappings",
      "background",
      "first_refresh",
    ]);
  });

  it("restores the first incomplete step even when later local work is retained", () => {
    const progress = state(["server", "detection", "background"]);
    expect(firstIncompleteOnboardingStep(progress)).toBe("authentication");
  });

  it("backs up without moving before the server step", () => {
    expect(previousOnboardingStep("server")).toBe("server");
    expect(previousOnboardingStep("device")).toBe("permissions");
  });

  it("reports deterministic progress without double-counting", () => {
    expect(onboardingProgress(state(["server", "authentication"]))).toEqual({
      completed: 2,
      total: 8,
      percent: 25,
    });
  });

  it("builds the shared eight-step presentation from authoritative markers", () => {
    const steps = onboardingStepStates(state([
      "server",
      "authentication",
      "permissions",
      "device",
    ]));
    expect(steps).toHaveLength(8);
    expect(steps.find(({ step }) => step === "device")).toMatchObject({ complete: true });
    expect(steps.find(({ step }) => step === "detection")).toMatchObject({
      complete: false,
      current: true,
      label: "Folder detection",
    });
  });

  it("rejects unknown versions and duplicate completion markers", () => {
    expect(isCompatibleOnboardingState(state(["server"]))).toBe(true);
    expect(isCompatibleOnboardingState({ ...state(), version: 2 })).toBe(false);
    expect(isCompatibleOnboardingState(state(["server", "server"]))).toBe(false);
  });

  it("holds the library handoff until the first refresh completion marker is durable", () => {
    const readyToRefresh = state(ONBOARDING_STEPS.slice(0, 7) as OnboardingState["completedSteps"]);
    expect(requiresInitialRefresh(readyToRefresh)).toBe(true);
    expect(requiresInitialRefresh(state([...ONBOARDING_STEPS]))).toBe(false);
    expect(requiresInitialRefresh(null)).toBe(false);
  });
});
