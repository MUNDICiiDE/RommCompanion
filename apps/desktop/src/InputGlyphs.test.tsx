import { describe, expect, it } from "vitest";
import { actionGlyph, controllerButtonGlyph, inputFamilyLabel } from "./InputGlyphs";

describe("input-family presentation", () => {
  it("uses the physical PlayStation face-button symbols", () => {
    expect(actionGlyph("play_station", "confirm")).toBe("✕");
    expect(actionGlyph("play_station", "back")).toBe("○");
    expect(actionGlyph("play_station", "context")).toBe("□");
    expect(actionGlyph("play_station", "search")).toBe("△");
  });

  it("uses Nintendo's physical south/east labels and PC key hints", () => {
    expect(actionGlyph("nintendo", "confirm")).toBe("B");
    expect(actionGlyph("nintendo", "back")).toBe("A");
    expect(actionGlyph("pc", "confirm")).toBe("↵");
    expect(actionGlyph("pc", "back")).toBe("Esc");
  });

  it("provides a readable label for every requested family", () => {
    expect(inputFamilyLabel("pc")).toBe("PC");
    expect(inputFamilyLabel("xbox")).toContain("Xbox");
    expect(inputFamilyLabel("play_station")).toContain("PlayStation");
    expect(inputFamilyLabel("steam")).toContain("Steam");
  });

  it("renders rebound physical buttons for each controller family", () => {
    expect(controllerButtonGlyph("play_station", "west")).toBe("□");
    expect(controllerButtonGlyph("nintendo", "south")).toBe("B");
    expect(controllerButtonGlyph("xbox", "left_shoulder")).toBe("LB");
    expect(controllerButtonGlyph("play_station", "right_shoulder")).toBe("R1");
    expect(controllerButtonGlyph("steam", undefined)).toBe("—");
  });
});
