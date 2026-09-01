export type InputModality = "navigation" | "pointer";

export function modalityForKeyboardEvent(key: string): InputModality | null {
  return ["Alt", "AltGraph", "Control", "Meta", "Shift"].includes(key)
    ? null
    : "navigation";
}

export function nextTrappedFocusIndex(
  currentIndex: number,
  itemCount: number,
  backwards: boolean,
) {
  if (itemCount <= 0) return -1;
  if (currentIndex < 0) return backwards ? itemCount - 1 : 0;
  const delta = backwards ? -1 : 1;
  return (currentIndex + delta + itemCount) % itemCount;
}

export const TABBABLE_SELECTOR = [
  "button:not([disabled])",
  "input:not([disabled])",
  "select:not([disabled])",
  "textarea:not([disabled])",
  "a[href]",
  "[tabindex]:not([tabindex='-1'])",
].join(",");
