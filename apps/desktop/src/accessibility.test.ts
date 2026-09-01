import { describe, expect, it } from "vitest";
import { modalityForKeyboardEvent, nextTrappedFocusIndex } from "./accessibility";

describe("accessible input and modal behavior", () => {
  it("restores navigation presentation for meaningful keyboard input", () => {
    expect(modalityForKeyboardEvent("Tab")).toBe("navigation");
    expect(modalityForKeyboardEvent("ArrowDown")).toBe("navigation");
    expect(modalityForKeyboardEvent("a")).toBe("navigation");
    expect(modalityForKeyboardEvent("Shift")).toBeNull();
  });

  it("wraps forward Tab focus inside a modal", () => {
    expect(nextTrappedFocusIndex(0, 3, false)).toBe(1);
    expect(nextTrappedFocusIndex(2, 3, false)).toBe(0);
    expect(nextTrappedFocusIndex(-1, 3, false)).toBe(0);
  });

  it("wraps Shift+Tab focus inside a modal", () => {
    expect(nextTrappedFocusIndex(2, 3, true)).toBe(1);
    expect(nextTrappedFocusIndex(0, 3, true)).toBe(2);
    expect(nextTrappedFocusIndex(-1, 3, true)).toBe(2);
  });

  it("handles a modal with no tabbable controls safely", () => {
    expect(nextTrappedFocusIndex(0, 0, false)).toBe(-1);
  });
});
