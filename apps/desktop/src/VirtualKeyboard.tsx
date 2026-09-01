import {
  FocusContext,
  getCurrentFocusKey,
  setFocus,
  useFocusable,
} from "@noriginmedia/norigin-spatial-navigation";
import { useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import { nextTrappedFocusIndex, TABBABLE_SELECTOR } from "./accessibility";
import { InputHint, type InputFamily } from "./InputGlyphs";
import type { ControllerBindings } from "./types";
import {
  backspaceKeyboardValue,
  defaultKeyboardValidation,
  deleteKeyboardValue,
  displayCursorToRaw,
  displayKeyboardValue,
  insertKeyboardText,
  moveKeyboardCursor,
  normalizeKeyboardValue,
  rawCursorToDisplay,
  submittedKeyboardValue,
  type KeyboardEditorState,
  type VirtualKeyboardMode,
} from "./keyboardEditor";

export type { VirtualKeyboardMode } from "./keyboardEditor";

interface VirtualKeyboardProps {
  label: string;
  initialValue: string;
  mode: VirtualKeyboardMode;
  inputFamily: InputFamily;
  bindings: ControllerBindings;
  maxLength?: number;
  secret?: boolean;
  validate?: (value: string) => string | null;
  onCancel: () => void;
  onSubmit: (value: string) => void;
}

interface KeyboardKeyProps {
  focusKey: string;
  label: string;
  onPress: () => void;
  wide?: boolean;
  accent?: boolean;
  active?: boolean;
  disabled?: boolean;
}

const NUMBER_KEYS = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"];
const LETTER_KEYS = [
  "q", "w", "e", "r", "t", "y", "u", "i", "o", "p",
  "a", "s", "d", "f", "g", "h", "j", "k", "l",
  "z", "x", "c", "v", "b", "n", "m",
];
const SYMBOL_KEYS = [".", "/", ":", "-", "_", "@"];
export const VIRTUAL_KEYBOARD_COMMAND_EVENT = "virtual-keyboard-command";

interface VirtualKeyboardCommandEvent extends CustomEvent {
  detail: "backspace";
}

function KeyboardKey({
  focusKey,
  label,
  onPress,
  wide = false,
  accent = false,
  active = false,
  disabled = false,
}: KeyboardKeyProps) {
  const { ref, focused } = useFocusable({
    focusKey,
    focusable: !disabled,
    onEnterPress: disabled ? undefined : onPress,
  });

  return (
    <button
      ref={ref}
      type="button"
      className={`keyboard-key ${wide ? "wide" : ""} ${accent ? "accent-key" : ""} ${active ? "active" : ""} ${focused ? "is-focused" : ""}`}
      onClick={onPress}
      disabled={disabled}
      onFocus={() => {
        if (getCurrentFocusKey() !== focusKey) setFocus(focusKey);
      }}
      onPointerDown={() => {
        if (getCurrentFocusKey() !== focusKey) setFocus(focusKey);
      }}
    >
      {label}
    </button>
  );
}

export function VirtualKeyboard({
  label,
  initialValue,
  mode,
  inputFamily,
  bindings,
  maxLength,
  secret = false,
  validate,
  onCancel,
  onSubmit,
}: VirtualKeyboardProps) {
  const { ref, focusKey } = useFocusable({
    focusKey: "VIRTUAL-KEYBOARD",
    isFocusBoundary: true,
    focusBoundaryDirections: ["up", "down", "left", "right"],
  });
  const initialCanonicalValue = normalizeKeyboardValue(initialValue, mode, maxLength);
  const [editor, setEditor] = useState<KeyboardEditorState>(() => ({
    value: initialCanonicalValue,
    cursor: initialCanonicalValue.length,
  }));
  const [shifted, setShifted] = useState(false);
  const [directEditing, setDirectEditing] = useState(false);
  const directInputRef = useRef<HTMLInputElement>(null);
  const validationId = useId();

  const insert = useCallback((text: string) => {
    setEditor((current) => insertKeyboardText(current, text, mode, maxLength));
  }, [maxLength, mode]);

  const backspace = useCallback(() => {
    setEditor(backspaceKeyboardValue);
  }, []);

  const deleteCharacter = useCallback(() => {
    setEditor(deleteKeyboardValue);
  }, []);

  const moveCursor = useCallback((direction: "left" | "right" | "home" | "end") => {
    setEditor((current) => moveKeyboardCursor(current, direction));
  }, []);

  const submittedValue = submittedKeyboardValue(editor.value, mode);
  const validationError = validate?.(submittedValue)
    ?? defaultKeyboardValidation(editor.value, mode);
  const submit = useCallback(() => {
    if (!validationError) onSubmit(submittedValue);
  }, [onSubmit, submittedValue, validationError]);

  const keys = useMemo(
    () => {
      if (mode === "numeric") return NUMBER_KEYS;
      if (mode === "pairing") return [...NUMBER_KEYS, ...LETTER_KEYS];
      if (mode === "token") return [...NUMBER_KEYS, ...LETTER_KEYS, "_"];
      return [...NUMBER_KEYS, ...LETTER_KEYS, ...SYMBOL_KEYS];
    },
    [mode],
  );

  useEffect(() => {
    window.setTimeout(() => setFocus("KEY-0"), 0);
  }, []);

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopImmediatePropagation();
        onCancel();
      } else if (event.key === "Enter") {
        event.preventDefault();
        event.stopImmediatePropagation();
        submit();
      } else if (event.target === directInputRef.current) {
        return;
      } else if (event.key === "Backspace") {
        event.preventDefault();
        backspace();
      } else if (event.key === "Delete") {
        event.preventDefault();
        deleteCharacter();
      } else if (event.key === "Home") {
        event.preventDefault();
        moveCursor("home");
      } else if (event.key === "End") {
        event.preventDefault();
        moveCursor("end");
      } else if (
        event.key.length === 1 &&
        !event.ctrlKey &&
        !event.metaKey &&
        !event.altKey
      ) {
        event.preventDefault();
        insert(event.key);
      }
    };
    window.addEventListener("keydown", handleKeyDown, true);
    return () => window.removeEventListener("keydown", handleKeyDown, true);
  }, [backspace, deleteCharacter, insert, moveCursor, onCancel, submit]);

  useEffect(() => {
    const handleControllerCommand = (event: Event) => {
      const command = event as VirtualKeyboardCommandEvent;
      if (command.detail === "backspace") {
        backspace();
      }
    };
    window.addEventListener(VIRTUAL_KEYBOARD_COMMAND_EVENT, handleControllerCommand);
    return () => window.removeEventListener(VIRTUAL_KEYBOARD_COMMAND_EVENT, handleControllerCommand);
  }, [backspace]);

  const displayValue = displayKeyboardValue(editor.value, mode);
  const displayCursor = rawCursorToDisplay(editor.cursor, mode);
  const renderedValue = secret ? "•".repeat(displayValue.length) : displayValue;
  const beforeCursor = renderedValue.slice(0, displayCursor);
  const afterCursor = renderedValue.slice(displayCursor);
  const placeholder = mode === "url" ? "Enter the server address" : "Start typing";

  return (
    <div
      className="keyboard-backdrop"
      role="presentation"
      onPointerDown={(event) => {
        if (event.target === event.currentTarget) onCancel();
      }}
    >
      <FocusContext.Provider value={focusKey}>
        <section
          ref={ref}
          className="virtual-keyboard"
          role="dialog"
          aria-modal="true"
          aria-label={`${label} virtual keyboard`}
          tabIndex={-1}
          onKeyDownCapture={(event) => {
            if (event.key !== "Tab") return;
            const keyboard = event.currentTarget;
            const tabbable = [...keyboard.querySelectorAll<HTMLElement>(TABBABLE_SELECTOR)]
              .filter((element) => element.getAttribute("aria-hidden") !== "true");
            const nextIndex = nextTrappedFocusIndex(
              tabbable.indexOf(document.activeElement as HTMLElement),
              tabbable.length,
              event.shiftKey,
            );
            if (nextIndex < 0) return;
            event.preventDefault();
            tabbable[nextIndex]?.focus();
          }}
        >
          <div className="keyboard-heading">
            <div>
              <p className="eyebrow accent">Controller keyboard</p>
              <h2>{label}</h2>
            </div>
            <span>
              <InputHint family={inputFamily} action="confirm" button={bindings.confirm} /> Select&nbsp;&nbsp;
              <InputHint family={inputFamily} action="back" button={bindings.back} /> Backspace
            </span>
          </div>

          <div
            className={`keyboard-preview ${directEditing ? "is-direct-editing" : ""}`}
            onPointerDown={() => {
              if (directEditing) return;
              setDirectEditing(true);
              window.requestAnimationFrame(() => {
                directInputRef.current?.focus();
                const selection = rawCursorToDisplay(editor.cursor, mode);
                directInputRef.current?.setSelectionRange(selection, selection);
              });
            }}
          >
            <div className="keyboard-controller-value" aria-live="polite" aria-hidden={directEditing}>
              {!renderedValue && <span>{placeholder}</span>}
              {beforeCursor}
              <i aria-hidden="true" />
              {afterCursor}
            </div>
            <input
              ref={directInputRef}
              className="keyboard-direct-input"
              value={displayValue}
              type={secret ? "password" : "text"}
              inputMode={mode === "numeric" ? "numeric" : mode === "url" ? "url" : "text"}
              maxLength={mode === "pairing" ? 9 : maxLength}
              placeholder={placeholder}
              aria-label={`${label} value`}
              aria-invalid={Boolean(validationError)}
              aria-describedby={validationError ? validationId : undefined}
              tabIndex={directEditing ? 0 : -1}
              onChange={(event) => {
                const normalized = normalizeKeyboardValue(event.target.value, mode, maxLength);
                const selection = event.target.selectionStart ?? event.target.value.length;
                setEditor({
                  value: normalized,
                  cursor: displayCursorToRaw(selection, mode, normalized.length),
                });
              }}
              onSelect={(event) => {
                const selection = event.currentTarget.selectionStart ?? displayValue.length;
                setEditor((current) => ({
                  ...current,
                  cursor: displayCursorToRaw(selection, mode, current.value.length),
                }));
              }}
              onBlur={() => setDirectEditing(false)}
            />
          </div>

          <div className="keyboard-edit-actions" aria-label="Text editing controls">
            <KeyboardKey focusKey="KEY-HOME" label="Home" onPress={() => moveCursor("home")} />
            <KeyboardKey focusKey="KEY-LEFT" label="← Cursor" onPress={() => moveCursor("left")} />
            <span className="keyboard-cursor-position" aria-live="polite">
              {editor.cursor} / {editor.value.length}
            </span>
            <KeyboardKey focusKey="KEY-RIGHT" label="Cursor →" onPress={() => moveCursor("right")} />
            <KeyboardKey focusKey="KEY-END" label="End" onPress={() => moveCursor("end")} />
            <KeyboardKey focusKey="KEY-DELETE" label="Delete" onPress={deleteCharacter} />
          </div>

          <div className={`keyboard-grid ${mode === "numeric" ? "numeric" : ""}`}>
            {keys.map((key, index) => {
              const output = shifted && /^[a-z]$/.test(key) ? key.toUpperCase() : key;
              return (
                <KeyboardKey
                  key={`${key}-${index}`}
                  focusKey={`KEY-${index}`}
                  label={output.toUpperCase()}
                  onPress={() => insert(output)}
                />
              );
            })}
          </div>

          <div className="keyboard-actions">
            {mode === "url" && (
              <KeyboardKey focusKey="KEY-HTTPS" label="https://" onPress={() => insert("https://")} wide />
            )}
            {mode === "token" && (
              <KeyboardKey focusKey="KEY-TOKEN-PREFIX" label="rmm_" onPress={() => insert("rmm_")} wide />
            )}
            {mode !== "numeric" && mode !== "pairing" && (
              <KeyboardKey
                focusKey="KEY-SHIFT"
                label="Shift"
                onPress={() => setShifted((current) => !current)}
                active={shifted}
                wide
              />
            )}
            {mode === "text" && (
              <KeyboardKey focusKey="KEY-SPACE" label="Space" onPress={() => insert(" ")} wide />
            )}
            <KeyboardKey
              focusKey="KEY-BACKSPACE"
              label="⌫ Backspace"
              onPress={backspace}
              wide
            />
            <KeyboardKey
              focusKey="KEY-CLEAR"
              label="Clear"
              onPress={() => setEditor({ value: "", cursor: 0 })}
              wide
            />
            <KeyboardKey focusKey="KEY-CANCEL" label="Cancel" onPress={onCancel} wide />
            <KeyboardKey
              focusKey="KEY-DONE"
              label="Done"
              onPress={submit}
              disabled={Boolean(validationError)}
              wide
              accent
            />
          </div>
          {validationError && (
            <p id={validationId} className="keyboard-validation" role="status">
              {validationError}
            </p>
          )}
        </section>
      </FocusContext.Provider>
    </div>
  );
}
