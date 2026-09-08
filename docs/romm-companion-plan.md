<!-- markdownlint-disable MD013 -->

# RomM Companion v1 Master Plan

> **Status:** Approved and authoritative for v1. Implementation details must conform to this document. See the [[romm-companion-feature-specs|detailed feature specification]] for feature-level behavior and acceptance criteria.
> **Implementation tracking:** Work is divided into small, independently verifiable milestones in the [[#Delivery roadmap|delivery roadmap]]. Detailed checklists live in the [[romm-companion-feature-specs|feature specification]]. Update both documents whenever a milestone changes state.

## Table of contents

- [[#Product definition|Product definition]]
- [[#Delivery roadmap|Delivery roadmap]]
- [[#Technology choices|Technology choices]]
- [[#Architecture and interfaces|Architecture and interfaces]]
- [[#Product behavior|Product behavior]]
- [[#Distribution and operations|Distribution and operations]]
- [[#Quality and release criteria|Quality and release criteria]]
- [[#Security and privacy|Security and privacy]]
- [[#v1 boundaries|v1 boundaries]]
- [[#Assumptions and defaults|Assumptions and defaults]]
- [[#References|References]]

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

## Delivery roadmap

Status meanings: **Complete** has passed its milestone acceptance checks; **In progress** has verified implementation but unfinished acceptance checks; **Not started** has no accepted implementation yet; **Continuous** runs throughout development and is only complete for a release candidate.

| Feature | Small milestones | Status | Next action |
| --- | --- | --- | --- |
| 1. Application foundation | 1.1 workspace, 1.2 IPC/agent, 1.3 persistence, 1.4 desktop lifecycle, 1.5 foundation validation | Complete | Maintain while later features extend it |
| 2. Server connection and pairing | 2.1 probing, 2.2 trust controls, 2.3 pairing/manual token, 2.4 validation/persistence, 2.5 recovery/UI, 2.6 validation | Complete | Maintain against supported RomM 5.x releases |
| 3. Device registration | 3.1 API/local model, 3.2 registration, 3.3 onboarding UI, 3.4 verification/recovery, 3.5 settings/validation | Complete | Maintain while mappings and sync extend the device payload |
| 4. Controller and spatial navigation | 4.1 native input, 4.2 focus graph, 4.3 action model, 4.4 text entry, 4.5 accessibility/automation, 4.6 physical qualification | In progress | Defer the 4.6 physical Windows/Steam Deck controller qualification gate |
| 5. Onboarding | 5.1 state machine, 5.2 connection/device steps, 5.3 mapping/archive steps, 5.4 agent/update choices, 5.5 resume/validation | Complete | Maintain the completed handoff while library/cache contracts expand |
| 6. Library browsing | 6.1 API/cache model, 6.2 paged catalog, 6.3 shelves/views, 6.4 search/filter/details, 6.5 offline/error/validation | Complete | Maintain while favorites and downloads add actions |
| 7. Favorites | 7.1 model/API, 7.2 optimistic UI, 7.3 offline journal, 7.4 reconciliation/validation | Complete | Maintain while downloads and broader offline controls consume favorite state |
| 8. EmuDeck detection and mappings | 8.1 platform catalog, 8.2 detection, 8.3 mapping editor, 8.4 validation/removable storage, 8.5 persistence/validation | In progress (8.1-8.4 complete; 8.5 implementation complete) | Defer the 8.5 physical Steam Deck/removable-media qualification gate; development may proceed to 9.1 |
| 9. Download queue | 9.1 queue model, 9.2 transfer engine, 9.3 controls/concurrency, 9.4 recovery, 9.5 UI/validation | Not started | Begin after mappings provide safe destinations |
| 10. Archive processing and local copies | 10.1 policy model, 10.2 safe extraction, 10.3 finalization, 10.4 managed removal, 10.5 validation | Not started | Begin after the base download engine |
| 11. Offline mode and local cache | 11.1 metadata cache, 11.2 artwork LRU, 11.3 offline UX/search, 11.4 reconnect/reset, 11.5 validation | In progress | Add configurable cache controls, startup enforcement, reconnect convergence, and reset/recovery UI |
| 12. Background sync agent | 12.1 foreground lifecycle, 12.2 service controls, 12.3 Windows startup, 12.4 SteamOS systemd, 12.5 health/update/validation | In progress | Validate 5.4 registration on Windows and later qualify the SteamOS user service physically |
| 13. Save and state synchronization | 13.1 mappings/inventory, 13.2 watching/stability, 13.3 protocol engine, 13.4 reconciliation, 13.5 conflicts, 13.6 validation | Not started | Begin after mappings and device registration |
| 14. Settings and diagnostics | 14.1 settings model/UI, 14.2 staged application, 14.3 diagnostics preview, 14.4 redacted export, 14.5 validation | Not started | Add incrementally as feature settings become available |
| 15. Updates and distribution | 15.1 release versioning, 15.2 Windows packaging, 15.3 AppImage/sidecar, 15.4 signed updater, 15.5 rollback/docs/validation | Not started | Defer packaging completion until core features stabilize |
| 16. Cross-cutting test matrix | 16.1 fixtures/harnesses, 16.2 CI, 16.3 security/data integrity, 16.4 performance/resilience, 16.5 physical release qualification | Continuous | Extend coverage with every completed milestone |

The milestone checkboxes and completion evidence are maintained in the [[romm-companion-feature-specs|detailed feature specification]]. A parent feature is complete only when every milestone under it is checked and its feature-level acceptance criteria pass.

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

Window-close behavior is an explicit persisted desktop preference. The default is **Minimize to tray**, which hides the GUI while leaving its foreground agent available; **Quit completely** gracefully stops a GUI-owned ephemeral agent and exits. The tray menu always provides **Open RomM Companion** and **Quit completely**. On SteamOS Gaming Mode, where no conventional tray is available, an enabled user-level agent continues independently and the GUI reconnects when reopened.

A controller-focusable fullscreen toggle is always available near the top of the GUI and the preference persists. Windows and general Linux installations default to windowed mode. Steam Deck hardware defaults to fullscreen on first launch, detected from Valve/Jupiter/Galileo DMI data with SteamOS environment and `VARIANT_ID=steamdeck` fallbacks; the user can override it permanently.

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

The detailed field definitions and command/event contracts live in the [[romm-companion-feature-specs|feature specification]].

### RomM compatibility and authentication

- Support RomM 5.x only.
- Generate models from a pinned RomM 5.x OpenAPI snapshot and accept unknown additive response fields.
- Probe `/openapi.json` and authenticated identity/capability endpoints during setup.
- Reject an incompatible server before creating local mappings or a device record.
- Prefer the eight-character alphanumeric, five-minute, single-use pairing-code exchange (`XXXX-XXXX`).
- Allow manual `rmm_...` Client API Token entry as a fallback.
- Never collect or persist a RomM account password.

Request only these scopes:

- `me.read`
- `roms.read`
- `platforms.read`
- `collections.read`
- `collections.write`
- `roms.user.read`
- `roms.user.write`
- `assets.read`
- `assets.write`
- `devices.read`
- `devices.write`

`me.read` is required to verify the authenticated account and inspect the current user's Client API Tokens. The app deliberately does not request `me.write`: signing out clears the local credential and identifies the RomM token for optional revocation in RomM's web interface.

RomM 5.x represents favorites as the authenticated user's private collection marked `is_favorite`, not as a user-ROM property. Favorite reads therefore use `collections.read`; writes use the atomic `POST/DELETE /api/collections/{id}/roms` operations and require `collections.write`. The app creates the private Favorites collection only when the user adds their first favorite and RomM has not created one yet. `roms.user.read` and `roms.user.write` remain required for play-status and future user-ROM operations, but are not used to mutate favorites.

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

Library API reads use stable, offset-based pages of 48 ROMs. The first page renders immediately, progress is shown as `loaded of total`, and another page is requested within 600 CSS pixels of the scroll end. A controller-focusable **Load more games** action remains available whenever another page exists. Refresh restarts at offset zero and overlapping pages are deduplicated by RomM ROM ID. IPC schema v22 defines typed all, recent, favorite, platform, standard/smart collection, downloaded, and active-download queries plus combinable search, platform, collection, favorite, downloaded, sort, complete game-detail, authenticated artwork, authoritative online favorite, durable queued-favorite responses, typed reconnect reconciliation summaries, and local directory browsing/confirmed creation. Home and six controller-accessible tabs expose those views; downloaded-only queries never contact RomM. Online discovery translates to RomM's `search_term`, platform, collection, favorite, and ordering parameters; retryable failures immediately query the normalized local cache. The agent persists normalized origin-scoped ROM, user-ROM, artwork, platform, collection, local-state, view-scoped exact-page, full-detail, and pending favorite records in SQLite schema v13. Artwork is fetched only from the selected RomM origin with the bearer token, validated as a bounded raster image, stored in a fixed 1 GiB LRU for this milestone, and returned to the WebView as a data URL payload. Remote deletion reconciliation removes disposable remote-only rows while preserving downloaded copies as `Unavailable on server`. Every remote page, detail record, and metadata snapshot reports live/cache source, refresh time, and 24-hour stale state; only retryable connection/server failures may use remote-cache fallback. Online favorite persistence, optimistic card/detail controls, per-ROM serialization, authoritative settlement, visible rollback/retry, an origin-scoped coalescing offline journal, and ordered automatic replay after authentication are implemented. Replay refreshes normalized user-ROM state transactionally: `401` and retryable failures preserve and pause the remaining queue, `403` restores rejected optimistic values and identifies `collections.write`, and `404` restores the favorite while marking the remote ROM unavailable. Mapping review can browse the local filesystem without a platform-specific dialog, edit multiple save/state roots, and create only one explicitly confirmed final directory beneath an existing parent. Downloads remain owned by Feature 9.

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
- At the final distribution milestone, package `romm-sync-agent` with Tauri `bundle.externalBin` as a target-triple-specific sidecar. Users receive one NSIS installer or one AppImage and never manage the agent executable separately.
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
- Library writes: favorite membership only, implemented through RomM's special private Favorites collection; user-created collections and metadata remain read-only.
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
