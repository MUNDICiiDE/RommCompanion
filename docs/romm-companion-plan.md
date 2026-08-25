<!-- markdownlint-disable MD013 -->

# RomM Companion v1 Master Plan

> **Status:** Approved and authoritative for v1. Implementation details must conform to this document. See the [detailed feature specification](./romm-companion-feature-specs.md) for feature-level behavior and acceptance criteria.

## Table of contents

- [Product definition](#product-definition)
- [Technology choices](#technology-choices)
- [Architecture and interfaces](#architecture-and-interfaces)
- [Product behavior](#product-behavior)
- [Distribution and operations](#distribution-and-operations)
- [Quality and release criteria](#quality-and-release-criteria)
- [Security and privacy](#security-and-privacy)
- [v1 boundaries](#v1-boundaries)
- [Assumptions and defaults](#assumptions-and-defaults)
- [References](#references)

## Product definition

RomM Companion is a public, MIT-licensed, controller-first desktop client for one RomM 5.x instance. It targets Windows 10/11 and x86-64 Steam Deck/SteamOS.

The app lets a user:

- Pair with a RomM server using a short device code or a Client API Token.
- Browse, search, filter, and favorite games with an offline-capable metadata cache.
- Download ROMs into detected EmuDeck folders or user-defined platform folders.
- Apply a per-platform archive policy.
- Bidirectionally synchronize save files and save states with RomM.
- Operate the entire interface with a controller, while retaining keyboard, mouse, and touch support.

RomM Companion prepares files for EmuDeck, ES-DE, Steam ROM Manager, or another external frontend. It does not launch emulators or maintain external frontend databases.

## Technology choices

### Application stack

- **Desktop shell:** Tauri 2.
- **Frontend:** React with TypeScript and Vite.
- **Native implementation:** stable Rust with Tokio.
- **Styling:** Tailwind CSS.
- **Animation:** Motion for React, limited to navigation continuity and status transitions.
- **Remote/cache state:** TanStack Query.
- **Routing:** React Router.
- **Spatial focus:** Norigin Spatial Navigation.
- **Controller input:** native Rust `gilrs`, bridged to the frontend as normalized actions.
- **Persistence:** SQLite in WAL mode, owned exclusively by the agent process.
- **HTTP:** Rust `reqwest`; the WebView never calls RomM directly.
- **Filesystem observation:** Rust `notify`, followed by content-stability checks.

Native controller input avoids depending on differences between Windows WebView2 and Linux WebKitGTK gamepad implementations. Keyboard events produced by Steam Input remain a supported secondary input path.

### Repository shape

Use one Rust workspace and one frontend package:

```text
apps/
  desktop/                 React/Tauri GUI
crates/
  romm-core/               API, downloads, mappings, cache, and sync domain logic
  romm-ipc/                Versioned IPC messages and local transport
  romm-sync-agent/         Headless long-running process
docs/
  romm-companion-plan.md
  romm-companion-feature-specs.md
```

The project uses `pnpm` for JavaScript dependencies and Cargo for Rust dependencies. Generated RomM API types are checked in so builds are reproducible.

## Architecture and interfaces

### Process ownership

The headless `romm-sync-agent` is the single owner of:

- RomM network requests and authentication headers.
- Credential retrieval.
- SQLite reads, writes, and migrations.
- Download and extraction jobs.
- ROM/save/state filesystem access.
- Folder watchers and sync sessions.
- Structured logs and diagnostic state.

The Tauri GUI is a control and presentation client. It sends typed requests over a same-user local IPC connection and receives snapshots/events. It does not receive the bearer token.

When background operation is disabled, the GUI starts an ephemeral agent and terminates it after pending database writes are flushed. Downloads pause safely when that agent exits. When background operation is enabled, the GUI connects to the registered user-level agent and downloads/sync continue after the window closes.

### Local IPC

- Linux: Unix domain socket below `$XDG_RUNTIME_DIR`, with user-only permissions.
- Windows: a named pipe scoped to the current user SID.
- A single-instance lock prevents two agents from owning the database.
- Every message includes an IPC schema version and request identifier.
- Long-running operations emit events; callers do not hold a blocking request open.
- Version mismatch produces an explicit `agent_upgrade_required` status rather than attempting partial compatibility.

### Shared types

The Rust boundary publishes serialized equivalents of:

- `ConnectionStatus`
- `AgentStatus`
- `PlatformMapping`
- `LocalGame`
- `DownloadJob`
- `SyncStatus`
- `SyncConflict`
- `AppSettings`
- `AppError`

The detailed field definitions and command/event contracts live in the [feature specification](./romm-companion-feature-specs.md).

### RomM compatibility and authentication

- Support RomM 5.x only.
- Generate models from a pinned RomM 5.x OpenAPI snapshot and accept unknown additive response fields.
- Probe `/openapi.json` and authenticated identity/capability endpoints during setup.
- Reject an incompatible server before creating local mappings or a device record.
- Prefer the eight-digit, five-minute, single-use pairing-code exchange.
- Allow manual `rmm_...` Client API Token entry as a fallback.
- Never collect or persist a RomM account password.

Request only these scopes:

- `roms.read`
- `platforms.read`
- `collections.read`
- `roms.user.read`
- `roms.user.write`
- `assets.read`
- `assets.write`
- `devices.read`
- `devices.write`

Tokens are kept in Windows Credential Manager or Linux Secret Service. If Linux Secret Service is unavailable, the user may explicitly accept storage in a user-only `0600` credential file. The fallback is never silent.

## Product behavior

### Onboarding

Onboarding is resumable and ordered:

1. Normalize and test the RomM base URL.
2. Pair or accept a manually supplied Client API Token.
3. Validate server version, identity, and required scopes.
4. Register a single RomM device.
5. Detect EmuDeck paths and create proposed platform mappings.
6. Review ROM, save, state, and archive settings for each enabled platform.
7. Optionally enable start-at-login background sync.
8. Perform the first metadata refresh and sync inventory.

HTTP is allowed only after a visible warning. Invalid TLS certificates cannot be globally ignored; private CAs must be imported explicitly.

### Controller-first library

The default presentation is a media-library layout with large cover art, horizontal shelves, high-contrast focus rings, and a compact game-detail surface. Home exposes recent additions, favorites, platforms, collections, downloaded games, and active downloads.

All functionality is reachable through controller, keyboard, mouse, and touch. Directional focus is deterministic and restored when dialogs close or virtualized lists update. Motion honors the operating-system reduced-motion preference.

### Downloads and local files

- Two downloads run concurrently by default; the allowed setting is one through four.
- Jobs survive restart and support pause, cancel, retry, and HTTP range resume.
- Content is written to `.part` files, validated, and atomically moved into place.
- The agent checks for the expected content size plus 256 MiB of free-space headroom before starting.
- Local removal is always explicit and never deletes server content.
- Save files and states are never deleted as a side effect of removing a ROM.

Archive behavior is configured per platform:

- Keep the downloaded archive.
- Extract and keep the archive.
- Extract and delete the archive after successful validation.

ZIP and 7z are supported in v1. Extraction rejects absolute paths, traversal, links escaping the destination, and unsafe duplicate paths. Extraction occurs in a temporary sibling directory before an atomic final move.

### Save and state sync

The background agent watches configured save and state folders, waits until content is stable, inventories files with SHA-1, and uses RomM's device sync negotiation endpoints. It also reconciles at startup, resume, network restoration, and every 15 minutes so missed filesystem events cannot permanently suppress a sync.

Concurrent changes use RomM's `keep_both` outcome. Both files remain available and a conflict appears in the UI for explicit resolution. A sync session is marked complete only with accurate completed/failed counts.

The app does not upload play sessions because it does not own emulator launch or exit events.

### Offline behavior

Metadata, collections, favorite state, mappings, queue state, and local-file associations are retained in SQLite. Artwork uses a disposable LRU cache with a 1 GiB default limit. Offline mode clearly labels stale information, permits local browsing and queue management, and retries network operations after connectivity returns.

## Distribution and operations

- Publish a Windows NSIS installer and x86-64 Linux AppImage through GitHub Releases.
- Build AppImage artifacts against an Ubuntu 22.04-compatible baseline.
- Include a desktop entry and instructions for adding the AppImage as a non-Steam game.
- Do not create or modify Steam shortcuts automatically.
- Publish signed Tauri updater metadata on an opt-in stable update channel.
- Update the GUI, agent, and database schema as one coordinated release.
- Run database migrations transactionally and retain the previous executable/schema backup until startup health checks pass.
- Background startup is opt-in, reversible, and requires no administrator/root privileges.

On Windows, use a per-user startup registration referencing the stable NSIS install path. On SteamOS, install the agent beneath `~/.local/lib/romm-companion/` and register a user-level systemd service. The AppImage itself remains movable; the service never points directly at an arbitrary AppImage location.

## Quality and release criteria

CI must run:

- TypeScript type checking and frontend unit/component tests.
- Rust formatting, Clippy, unit tests, and integration tests.
- Dependency vulnerability and license auditing.
- Production builds for Windows and Linux.
- Mock-RomM API tests for authentication, pagination, downloads, and device sync.
- Markdown lint and external-link checks for project documentation.

A v1 release requires physical verification on a Steam Deck and a Windows PC. A user must be able to complete pairing, browse cached data offline, favorite a game, download and resume a ROM, use detected and custom paths, synchronize saves/states in both directions, preserve conflicts, enable/disable background startup, update the application, and perform every workflow using only a controller.

## Security and privacy

- The frontend has no general shell execution or unrestricted filesystem capability.
- Tauri permissions enumerate only required commands and path scopes.
- Credentials and authorization headers are redacted from logs and diagnostics.
- Pairing codes are redacted and discarded immediately after exchange.
- Diagnostics include application/server versions, sanitized mappings, state summaries, and recent redacted errors, but no tokens or file contents.
- Remote payloads, archive paths, image URLs, and local filenames are treated as untrusted input.
- No telemetry or analytics are enabled in v1.

## v1 boundaries

The following are explicitly deferred until after v1:

- Emulator launching or process monitoring.
- Embedded EmulatorJS or other in-app emulation.
- Play-session tracking.
- Collection creation or editing.
- ROM metadata editing.
- Automatic frontend rescans or Steam shortcut management.
- Multiple RomM instances or profiles.
- Flatpak, macOS, and ARM Linux packages.
- Multiplayer or netplay.

## Assumptions and defaults

- Product name: **RomM Companion**, pending a trademark/name check before public release.
- License: MIT.
- Server support: one RomM 5.x instance and one active account.
- Library writes: favorite state only; collections and metadata are read-only.
- Background agent: offered during onboarding and disabled until the user opts in.
- Sync mode: bidirectional push/pull with `keep_both` conflicts.
- Reconciliation interval: 15 minutes, plus event-driven and lifecycle-triggered runs.
- Archive default: keep the original unless an accepted platform preset specifies extraction.
- Update channel: opt-in stable releases from GitHub.

## References

- [RomM API authentication](https://docs.romm.app/latest/developers/api-authentication/)
- [RomM Client API Tokens and device pairing](https://docs.romm.app/latest/developers/client-api-tokens/)
- [RomM Device Sync Protocol](https://docs.romm.app/latest/developers/device-sync-protocol/)
- [RomM OpenAPI guidance](https://docs.romm.app/latest/developers/openapi/)
- [RomM downloads](https://docs.romm.app/latest/using/downloads/)
- [Tauri distribution](https://v2.tauri.app/distribute/)
- [Tauri AppImage guidance](https://v2.tauri.app/distribute/appimage/)
- [Tauri Windows installer guidance](https://v2.tauri.app/distribute/windows-installer/)
- [Tauri prerequisites and platform WebViews](https://v2.tauri.app/start/prerequisites/)
- [Norigin Spatial Navigation](https://github.com/NoriginMedia/Norigin-Spatial-Navigation)
- [gilrs](https://gitlab.com/gilrs-project/gilrs)
