import type { ControllerButton, ControllerMappingFamily } from "./types";

export type InputFamily = ControllerMappingFamily | "pc";
export type HintAction = "confirm" | "back" | "context" | "search" | "navigate";

const FAMILY_LABELS: Record<InputFamily, string> = {
  pc: "PC",
  xbox: "Xbox controller",
  play_station: "PlayStation controller",
  nintendo: "Nintendo controller",
  steam: "Steam controller / Steam Deck",
  generic: "Controller",
};

const ACTION_GLYPHS: Record<InputFamily, Record<HintAction, string>> = {
  pc: { confirm: "↵", back: "Esc", context: "X", search: "/", navigate: "↕" },
  xbox: { confirm: "A", back: "B", context: "X", search: "Y", navigate: "✣" },
  play_station: { confirm: "✕", back: "○", context: "□", search: "△", navigate: "✣" },
  nintendo: { confirm: "B", back: "A", context: "Y", search: "X", navigate: "✣" },
  steam: { confirm: "A", back: "B", context: "X", search: "Y", navigate: "✣" },
  generic: { confirm: "A", back: "B", context: "X", search: "Y", navigate: "✣" },
};

export function inputFamilyLabel(family: InputFamily): string {
  return FAMILY_LABELS[family];
}

export function actionGlyph(family: InputFamily, action: HintAction): string {
  return ACTION_GLYPHS[family][action];
}

export function controllerButtonGlyph(
  family: InputFamily,
  button: ControllerButton | undefined,
): string {
  if (!button) return "—";
  const face = {
    xbox: { south: "A", east: "B", west: "X", north: "Y" },
    play_station: { south: "✕", east: "○", west: "□", north: "△" },
    nintendo: { south: "B", east: "A", west: "Y", north: "X" },
    steam: { south: "A", east: "B", west: "X", north: "Y" },
    generic: { south: "A", east: "B", west: "X", north: "Y" },
    pc: { south: "A", east: "B", west: "X", north: "Y" },
  }[family];
  if (button in face) return face[button as keyof typeof face];
  if (button === "left_shoulder") return family === "play_station" ? "L1" : "LB";
  return family === "play_station" ? "R1" : "RB";
}

export function InputHint({
  family,
  action,
  button,
}: {
  family: InputFamily;
  action: HintAction;
  button?: ControllerButton;
}) {
  const glyph = family === "pc" || !button
    ? actionGlyph(family, action)
    : controllerButtonGlyph(family, button);
  return <kbd className={`input-glyph ${family}`}>{glyph}</kbd>;
}

export function InputFamilyIcon({ family }: { family: InputFamily }) {
  const common = {
    className: `input-family-icon ${family}`,
    viewBox: "0 0 48 32",
    role: "img" as const,
    "aria-label": inputFamilyLabel(family),
  };

  if (family === "pc") {
    return (
      <svg {...common}>
        <rect x="7" y="3" width="34" height="21" rx="3" />
        <path d="M18 29h12M24 24v5" />
        <path className="icon-accent" d="M11 7h26v13H11z" />
      </svg>
    );
  }

  if (family === "steam") {
    return (
      <svg {...common}>
        <path d="M14 8h20c5 0 8 4 9 9l2 9c.5 3-3 5-5 3l-7-6H15l-7 6c-2 2-5.5 0-5-3l2-9c1-5 4-9 9-9Z" />
        <circle className="icon-accent" cx="14" cy="16" r="5" />
        <circle className="icon-accent" cx="34" cy="16" r="5" />
        <circle cx="24" cy="21" r="2" />
      </svg>
    );
  }

  if (family === "nintendo") {
    return (
      <svg {...common}>
        <rect x="6" y="3" width="15" height="26" rx="7" />
        <rect x="27" y="3" width="15" height="26" rx="7" />
        <circle className="icon-accent" cx="14" cy="11" r="3" />
        <circle className="icon-accent" cx="34" cy="20" r="3" />
        <path d="M11 20h6M14 17v6M32 10h4M34 8v4" />
      </svg>
    );
  }

  return (
    <svg {...common}>
      <path d="M14 7h20c5 0 8 4 9 9l2 9c.5 3-3 5-5 3l-7-6H15l-7 6c-2 2-5.5 0-5-3l2-9c1-5 4-9 9-9Z" />
      <path className="icon-accent" d="M11 15h8M15 11v8" />
      {family === "play_station" ? (
        <>
          <path className="icon-symbol" d="m33 11 2 3h-4l2-3Zm-4 6h3v3h-3z" />
          <circle className="icon-symbol" cx="37" cy="18" r="1.8" />
          <path className="icon-symbol" d="m36 13 3-3m0 3-3-3" />
        </>
      ) : (
        <>
          <circle className="icon-symbol" cx="33" cy="12" r="1.6" />
          <circle className="icon-symbol" cx="38" cy="16" r="1.6" />
          <circle className="icon-symbol" cx="33" cy="20" r="1.6" />
          <circle className="icon-symbol" cx="28" cy="16" r="1.6" />
        </>
      )}
      <circle cx="20" cy="22" r="2" />
      <circle cx="28" cy="22" r="2" />
    </svg>
  );
}
