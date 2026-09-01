export type VirtualKeyboardMode = "text" | "numeric" | "pairing" | "token" | "url";

export interface KeyboardEditorState {
  value: string;
  cursor: number;
}

function clampCursor(cursor: number, length: number) {
  return Math.max(0, Math.min(cursor, length));
}

export function filterKeyboardText(text: string, mode: VirtualKeyboardMode) {
  if (mode === "pairing") return text.toUpperCase().replace(/[^A-Z0-9]/g, "");
  if (mode === "numeric") return text.replace(/\D/g, "");
  if (mode === "token") return text.replace(/[^A-Za-z0-9_]/g, "");
  if (mode === "url") {
    return [...text].filter((character) => !/[\s\p{Cc}]/u.test(character)).join("");
  }
  return [...text].filter((character) => !/\p{Cc}/u.test(character)).join("");
}

export function keyboardValueLimit(mode: VirtualKeyboardMode, maxLength?: number) {
  if (mode === "pairing") return 8;
  if (mode === "token") return maxLength ?? 68;
  return maxLength;
}

export function normalizeKeyboardValue(
  value: string,
  mode: VirtualKeyboardMode,
  maxLength?: number,
) {
  const filtered = filterKeyboardText(value, mode);
  const limit = keyboardValueLimit(mode, maxLength);
  return limit === undefined ? filtered : filtered.slice(0, limit);
}

export function displayKeyboardValue(value: string, mode: VirtualKeyboardMode) {
  if (mode !== "pairing" || value.length <= 4) return value;
  return `${value.slice(0, 4)}-${value.slice(4)}`;
}

export function submittedKeyboardValue(value: string, mode: VirtualKeyboardMode) {
  return displayKeyboardValue(value, mode);
}

export function rawCursorToDisplay(cursor: number, mode: VirtualKeyboardMode) {
  return mode === "pairing" && cursor > 4 ? cursor + 1 : cursor;
}

export function displayCursorToRaw(cursor: number, mode: VirtualKeyboardMode, rawLength: number) {
  const rawCursor = mode === "pairing" && cursor > 4 ? cursor - 1 : cursor;
  return clampCursor(rawCursor, rawLength);
}

export function insertKeyboardText(
  state: KeyboardEditorState,
  text: string,
  mode: VirtualKeyboardMode,
  maxLength?: number,
): KeyboardEditorState {
  const cursor = clampCursor(state.cursor, state.value.length);
  const insertion = filterKeyboardText(text, mode);
  const available = keyboardValueLimit(mode, maxLength);
  const accepted = available === undefined
    ? insertion
    : insertion.slice(0, Math.max(0, available - state.value.length));
  return {
    value: `${state.value.slice(0, cursor)}${accepted}${state.value.slice(cursor)}`,
    cursor: cursor + accepted.length,
  };
}

export function backspaceKeyboardValue(state: KeyboardEditorState): KeyboardEditorState {
  const cursor = clampCursor(state.cursor, state.value.length);
  if (cursor === 0) return { ...state, cursor };
  return {
    value: `${state.value.slice(0, cursor - 1)}${state.value.slice(cursor)}`,
    cursor: cursor - 1,
  };
}

export function deleteKeyboardValue(state: KeyboardEditorState): KeyboardEditorState {
  const cursor = clampCursor(state.cursor, state.value.length);
  if (cursor === state.value.length) return { ...state, cursor };
  return {
    value: `${state.value.slice(0, cursor)}${state.value.slice(cursor + 1)}`,
    cursor,
  };
}

export function moveKeyboardCursor(
  state: KeyboardEditorState,
  direction: "left" | "right" | "home" | "end",
): KeyboardEditorState {
  if (direction === "home") return { ...state, cursor: 0 };
  if (direction === "end") return { ...state, cursor: state.value.length };
  const delta = direction === "left" ? -1 : 1;
  return { ...state, cursor: clampCursor(state.cursor + delta, state.value.length) };
}

export function defaultKeyboardValidation(value: string, mode: VirtualKeyboardMode) {
  if (mode === "pairing" && value.length !== 8) return "Enter all eight letters or numbers.";
  if (mode === "token" && !/^rmm_[a-f0-9]{64}$/i.test(value)) {
    return "Enter rmm_ followed by 64 hexadecimal characters.";
  }
  if ((mode === "url" || mode === "numeric") && value.length === 0) return "This field is required.";
  return null;
}
