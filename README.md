<!-- markdownlint-disable MD013 -->

# RomM Companion

RomM Companion is a controller-first Windows and Steam Deck client for a RomM 5.x server. The current foundation includes a React/Tauri desktop UI, a separate Rust agent, versioned same-user IPC, hot-pluggable native controller discovery and normalized events, RomM compatibility probing, persistent device-code/manual-token authentication, agent-owned SQLite state, and a paginated ROM library view.

Onboarding progress uses an agent-owned, versioned eight-step state machine. It stores only non-secret progress, restores the first incomplete step, preserves completed work during Back/Cancel, and invalidates server-dependent work without deleting reusable custom path drafts. The visible connection/device wizard integration is the next milestone.

The authoritative product plan is in [docs/romm-companion-plan.md](docs/romm-companion-plan.md). Feature-level decisions and acceptance criteria are in [docs/romm-companion-feature-specs.md](docs/romm-companion-feature-specs.md).

## Prerequisites

- Node.js 24 LTS
- pnpm 11.19 or compatible
- Stable Rust with `rustfmt` and `clippy`
- Windows: Visual Studio C++ desktop workload and WebView2
- Linux builds: WebKitGTK 4.1, appindicator, librsvg, libudev, pkg-config, and standard build tools

## Development

Install JavaScript dependencies:

```powershell
pnpm install
```

Build the development agent and start the Tauri desktop window. The desktop launches the agent automatically:

```powershell
pnpm dev
```

The desktop and agent communicate through a per-user Windows named pipe or a Linux Unix socket below `$XDG_RUNTIME_DIR` with `0600` permissions. The desktop launches an ephemeral adjacent agent automatically when no agent is available. A Windows named kernel mutex or Linux file lock prevents multiple agents from opening the database.

SQLite is bundled into the agent, uses WAL mode, and requires no separate installation. RomM tokens are stored in Windows Credential Manager or Linux Secret Service; SQLite stores only the credential locator and non-secret server metadata.

Inside the app:

1. Enter the base URL for a RomM 5.x server and select **Test server**.
2. Create a pairing code in RomM and enter the eight digits, or expand the manual-token section.
3. Pair and load the first 48 ROMs. More pages load automatically near the bottom, with a controller-focusable **Load more games** fallback.
4. Navigate with arrow keys, Steam Input keyboard mappings, a D-pad, or a left stick. Screens and dialogs use deterministic focus entry, exits, and restoration, including across library refresh and pagination. Search focuses the loaded-library filter, Context opens the focused game's action surface, and shoulder buttons change controller-settings profiles. The Controller dialog stores global or hardware-specific bindings plus dead-zone/repeat timing; helper glyphs update to the rebound physical buttons. Press the displayed confirm button on a text field to open the in-app virtual keyboard. It supports cursor movement, insert, Backspace, forward Delete, Home/End, field validation, and URL/token shortcuts; Cancel or Done restores the invoking field. The displayed back button acts only as Backspace while the keyboard is open. Mouse and physical-keyboard users edit ordinary fields natively without opening it, and can directly edit its preview if they switch input methods mid-entry.
   Pointer/touch use keeps logical focus but suppresses the controller-style ring until the next keyboard or controller action. Dialogs contain Tab focus and support Escape, controller Back, explicit close controls, and safe backdrop dismissal. Reduced-motion and forced-colors operating-system preferences are honored.
5. Choose whether the window close button minimizes RomM Companion to the tray or quits completely. The tray menu can reopen the window or fully exit.
6. Use the controller-focusable **Fullscreen** button in the top bar to enter or leave fullscreen. New Steam Deck installations default to fullscreen; Windows and other Linux desktops default to a window.

## Validation

```powershell
pnpm check
pnpm test
pnpm build
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Current limitations

- Remote device registration, controller-complete onboarding, reconnect verification, deletion/permission recovery, coalesced device renaming, and optional best-effort device removal during logout are implemented. ROM metadata caching, downloads, and save/state sync are not implemented yet. Connection and pairing verify the RomM account and the complete v1 token scope set before credentials are persisted.
- Linux Secret Service failure currently blocks credential persistence; the explicitly approved `0600` credential-file fallback will be added to onboarding rather than enabled silently.
- Background startup registration and diagnostics-only recovery after a migration failure remain part of the later background-agent/distribution milestones.
- ROM parsing accepts common RomM array and wrapper shapes until generated models are pinned from a target RomM 5.x OpenAPI document.
- Linux is compile-checked in CI but requires later physical Steam Deck verification.
- Final controller qualification still requires built-in Steam Deck controls plus Xbox- and PlayStation-family hardware on Windows and Steam Deck; the automated controller/focus state machines do not replace that release gate.
- Final distribution will package `romm-sync-agent` as an official Tauri sidecar inside the Windows NSIS installer and Linux AppImage; the current no-bundle artifact keeps both executables adjacent.

## Architecture

```text
React UI
   │ Tauri invoke + controller events
   ▼
Tauri desktop backend
   │ versioned JSON request/response
   ▼
romm-sync-agent
   │ authenticated HTTPS
   ▼
RomM 5.x
```

Domain/API behavior lives in `romm-core`; serialized contracts live in `romm-ipc`. The UI never sends requests directly to RomM.

## License

MIT
