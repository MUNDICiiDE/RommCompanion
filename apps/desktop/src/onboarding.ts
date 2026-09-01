import { requestAgent } from "./agent";
import type {
  OnboardingNavigationAction,
  OnboardingState,
  OnboardingStep,
} from "./types";

export const ONBOARDING_STATE_VERSION = 1;
export const ONBOARDING_STEPS: readonly OnboardingStep[] = [
  "server",
  "authentication",
  "permissions",
  "device",
  "detection",
  "mappings",
  "background",
  "first_refresh",
];

export const ONBOARDING_STEP_LABELS: Record<OnboardingStep, string> = {
  server: "Server",
  authentication: "Authentication",
  permissions: "Permissions",
  device: "Device",
  detection: "Folder detection",
  mappings: "Platform mappings",
  background: "Background sync",
  first_refresh: "First refresh",
};

export function firstIncompleteOnboardingStep(state: OnboardingState): OnboardingStep {
  return ONBOARDING_STEPS.find((step) => !state.completedSteps.includes(step)) ?? "first_refresh";
}

export function previousOnboardingStep(step: OnboardingStep): OnboardingStep {
  const index = Math.max(0, ONBOARDING_STEPS.indexOf(step));
  return ONBOARDING_STEPS[Math.max(0, index - 1)] ?? "server";
}

export function requiresInitialRefresh(state: OnboardingState | null): boolean {
  return state?.currentStep === "first_refresh" &&
    !state.completedSteps.includes("first_refresh");
}

export function onboardingProgress(state: OnboardingState) {
  const completed = new Set(state.completedSteps).size;
  return {
    completed,
    total: ONBOARDING_STEPS.length,
    percent: Math.round((completed / ONBOARDING_STEPS.length) * 100),
  };
}

export function onboardingStepStates(state: OnboardingState) {
  const completed = new Set(state.completedSteps);
  return ONBOARDING_STEPS.map((step) => ({
    step,
    label: ONBOARDING_STEP_LABELS[step],
    complete: completed.has(step),
    current: step === state.currentStep,
  }));
}

export function isCompatibleOnboardingState(state: OnboardingState) {
  return state.version === ONBOARDING_STATE_VERSION &&
    ONBOARDING_STEPS.includes(state.currentStep) &&
    state.completedSteps.every((step) => ONBOARDING_STEPS.includes(step)) &&
    new Set(state.completedSteps).size === state.completedSteps.length;
}

export async function getOnboardingState() {
  const response = await requestAgent({ type: "getOnboarding" });
  if (response.type === "onboarding") return response.state;
  if (response.type === "error") throw new Error(response.error.message);
  throw new Error("The agent returned an unexpected onboarding response.");
}

export async function navigateOnboarding(action: OnboardingNavigationAction) {
  const response = await requestAgent({ type: "updateOnboarding", action });
  if (response.type === "onboarding") return response.state;
  if (response.type === "error") throw new Error(response.error.message);
  throw new Error("The agent returned an unexpected onboarding response.");
}
