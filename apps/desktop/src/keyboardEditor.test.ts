import { describe, expect, it } from "vitest";
import {
  backspaceKeyboardValue,
  defaultKeyboardValidation,
  deleteKeyboardValue,
  displayCursorToRaw,
  displayKeyboardValue,
  filterKeyboardText,
  insertKeyboardText,
  moveKeyboardCursor,
  normalizeKeyboardValue,
  rawCursorToDisplay,
  submittedKeyboardValue,
} from "./keyboardEditor";

describe("controller keyboard editing", () => {
  it("normalizes and formats observed RomM pairing codes", () => {
    expect(normalizeKeyboardValue("jm38-mhsa!", "pairing", 9)).toBe("JM38MHSA");
    expect(displayKeyboardValue("JM38MHSA", "pairing")).toBe("JM38-MHSA");
    expect(submittedKeyboardValue("JM38MHSA", "pairing")).toBe("JM38-MHSA");
  });

  it("maps pairing cursors around the display-only separator", () => {
    expect(rawCursorToDisplay(4, "pairing")).toBe(4);
    expect(rawCursorToDisplay(5, "pairing")).toBe(6);
    expect(displayCursorToRaw(6, "pairing", 8)).toBe(5);
    expect(displayCursorToRaw(99, "pairing", 8)).toBe(8);
  });

  it("inserts at the cursor rather than always appending", () => {
    expect(insertKeyboardText({ value: "Helo", cursor: 3 }, "l", "text")).toEqual({
      value: "Hello",
      cursor: 4,
    });
  });

  it("backspaces before the cursor and deletes after it", () => {
    expect(backspaceKeyboardValue({ value: "ABCDE", cursor: 3 })).toEqual({
      value: "ABDE",
      cursor: 2,
    });
    expect(deleteKeyboardValue({ value: "ABCDE", cursor: 2 })).toEqual({
      value: "ABDE",
      cursor: 2,
    });
  });

  it("clamps cursor movement at both ends", () => {
    expect(moveKeyboardCursor({ value: "abc", cursor: 0 }, "left").cursor).toBe(0);
    expect(moveKeyboardCursor({ value: "abc", cursor: 3 }, "right").cursor).toBe(3);
    expect(moveKeyboardCursor({ value: "abc", cursor: 1 }, "home").cursor).toBe(0);
    expect(moveKeyboardCursor({ value: "abc", cursor: 1 }, "end").cursor).toBe(3);
  });

  it("enforces mode filters and maximum lengths before insertion", () => {
    expect(filterKeyboardText("12x-3", "numeric")).toBe("123");
    expect(filterKeyboardText("https://romm box", "url")).toBe("https://rommbox");
    expect(filterKeyboardText("https://例え.test", "url")).toBe("https://例え.test");
    expect(filterKeyboardText("rmm_AZ-09", "token")).toBe("rmm_AZ09");
    expect(insertKeyboardText({ value: "1234567", cursor: 7 }, "AB", "pairing")).toEqual({
      value: "1234567A",
      cursor: 8,
    });
  });

  it("requires complete pairing codes and exact RomM token syntax", () => {
    expect(defaultKeyboardValidation("JM38MHSA", "pairing")).toBeNull();
    expect(defaultKeyboardValidation("JM38", "pairing")).toMatch(/eight/);
    expect(defaultKeyboardValidation(`rmm_${"a".repeat(64)}`, "token")).toBeNull();
    expect(defaultKeyboardValidation(`rmm_${"z".repeat(64)}`, "token")).toMatch(/hexadecimal/);
  });

  it("keeps backspace and delete harmless at their boundaries", () => {
    expect(backspaceKeyboardValue({ value: "abc", cursor: 0 })).toEqual({ value: "abc", cursor: 0 });
    expect(deleteKeyboardValue({ value: "abc", cursor: 3 })).toEqual({ value: "abc", cursor: 3 });
  });
});
