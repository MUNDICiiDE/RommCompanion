<!-- markdownlint-disable MD013 MD024 -->

# RomM Companion v1 Feature Specifications

> **Status:** Implementation specification for the authoritative [[romm-companion-plan|RomM Companion v1 Master Plan]]. If the documents conflict, the master plan wins and this document must be corrected before implementation continues.
> **Tracking rule:** Each feature is divided into numbered delivery milestones. Check a milestone only after its stated result and the relevant tests pass. The [[romm-companion-plan#Delivery roadmap|master-plan roadmap]] summarizes the same state.

## Table of contents

1. [[#1. Application foundation|Application foundation]]
2. [[#2. Server connection and pairing|Server connection and pairing]]
3. [[#3. Device registration|Device registration]]
4. [[#4. Controller and spatial navigation|Controller and spatial navigation]]
5. [[#5. Onboarding|Onboarding]]
6. [[#6. Library browsing|Library browsing]]
7. [[#7. Favorites|Favorites]]
8. [[#8. EmuDeck detection and platform mappings|EmuDeck detection and platform mappings]]
9. [[#9. Download queue|Download queue]]
10. [[#10. Archive processing and local copies|Archive processing and local copies]]
11. [[#11. Offline mode and local cache|Offline mode and local cache]]
12. [[#12. Background sync agent|Background sync agent]]
13. [[#13. Save and state synchronization|Save and state synchronization]]
14. [[#14. Settings and diagnostics|Settings and diagnostics]]
15. [[#15. Updates and distribution|Updates and distribution]]
16. [[#16. Cross-cutting test matrix|Cross-cutting test matrix]]
17. [[#Deferred features|Deferred features]]

## Shared conventions

Every feature below specifies its objective, user experience, behavior/defaults, data/interfaces, failure handling, and tests/acceptance criteria. These conventions apply throughout:

- Milestones use `- [x]` for complete and `- [ ]` for incomplete. An incomplete milestone may be labeled **In progress** when verified pieces already exist.
- Each milestone must leave the application buildable and its completed behavior testable. A feature is complete only when all of its milestones and feature-level acceptance criteria pass.
- When milestone state changes, update the roadmap in both planning documents in the same change.

- RomM-supplied timestamps crossing an interface remain RFC 3339 UTC strings. Companion-owned contact, refresh, cache, and event timestamps use UTC epoch milliseconds in IPC and SQLite.
- RomM ROM, platform, collection, asset, and user IDs remain opaque 64-bit integers. RomM device IDs and sync-session IDs remain opaque strings. Local IDs are UUID v4 values.
- Paths are stored as native absolute paths, displayed in platform-native form, and never assembled by string concatenation.
- Byte counts are unsigned 64-bit integers. UI sizes use IEC units.
- Recoverable domain failures return typed `AppError` values; programming errors may terminate the failing task but must not crash the agent.
- Destructive local actions require confirmation and never imply a server deletion.
- Logs use structured fields and must pass through the central redaction layer.
- Network mutations are idempotent where the RomM API permits; otherwise the local journal records the server result before retry can occur.

## 1. Application foundation

> **Implementation status (2026-08-25):** The native per-user IPC transport, single-agent lock, bundled SQLite schema/migrations, WAL/checkpoint behavior, corrupt-cache preservation, OS credential storage, persisted server/settings restoration, schema negotiation, redaction, and rotating agent logs are implemented. Linux credential-file opt-in, diagnostics-only migration failure mode, and installed background-agent lifecycle remain assigned to their onboarding, diagnostics, and distribution milestones.

### Objective

Provide a reproducible Tauri/React application and a shared Rust foundation in which the agent exclusively owns network, database, and managed-file operations.

### User experience

The GUI starts to a loading shell immediately, connects to the existing agent or starts an ephemeral one, and then restores the last route and focused item. Startup displays a recoverable error screen if agent initialization fails; it never presents an indefinitely spinning window.

### Delivery milestones

- [x] **1.1 Workspace and shared contracts** — Establish the Cargo/pnpm workspace, React/Tauri shell, shared Rust crates, TypeScript parity, and reproducible builds.
- [x] **1.2 Agent ownership and IPC** — Enforce one agent, same-user transport, schema negotiation, typed requests/responses, and recoverable GUI connection errors.
- [x] **1.3 Persistence and credentials** — Add transactional SQLite migrations, WAL behavior, corruption preservation, OS credential storage, settings/profile restoration, redaction, and rotating logs.
- [x] **1.4 Desktop lifecycle** — Implement neutral startup restoration, fullscreen persistence, close-to-tray versus quit, tray actions, and graceful foreground-agent shutdown.
- [x] **1.5 Foundation validation** — Pass Rust/frontend tests, type checking, formatting, linting, production builds, restart checks, and the Windows agent smoke test. Linux runtime validation remains assigned to the release matrix.

### Behavior and defaults

- Use a Cargo workspace containing `romm-core`, `romm-ipc`, `romm-sync-agent`, and the Tauri crate; use one pnpm React workspace.
- The agent acquires a process lock before opening SQLite. The GUI never opens SQLite directly.
- Run embedded, numbered SQLite migrations in one transaction before serving IPC.
- Use SQLite WAL mode, foreign keys, a five-second busy timeout, and a daily checkpoint while idle.
- Configuration locations:
  - Windows config: `%APPDATA%\RommCompanion`; data/cache/logs: `%LOCALAPPDATA%\RommCompanion`.
  - Linux config: `$XDG_CONFIG_HOME/romm-companion`; data: `$XDG_DATA_HOME/romm-companion`; cache: `$XDG_CACHE_HOME/romm-companion`, with XDG fallbacks below the user's home.
- Retain seven log files of at most 10 MiB each. Default log level is `info`.
- When background mode is off, an ephemeral agent pauses active downloads and flushes state during a graceful GUI shutdown; a forced exit is recovered from the persisted journal next time.
- The persisted window-close preference is `minimizeToTray | quit`, defaulting to `minimizeToTray`. Minimize-to-tray hides the GUI and leaves its foreground agent running; Quit completely flushes and stops a GUI-owned ephemeral agent. Tray actions are Open RomM Companion and Quit completely. SteamOS Gaming Mode relies on the user-level agent rather than a tray.
- Persist fullscreen mode and expose a controller-focusable top-bar toggle. The first-launch default is enabled on detected Steam Deck hardware and disabled on Windows/general Linux. Detection checks Valve/Jupiter/Galileo DMI identifiers first, then `SteamDeck=1` and SteamOS `VARIANT_ID=steamdeck`; any saved user choice overrides detection.

### Data and interfaces

The initial schema contains `schema_migrations`, `app_state`, `server_profile`, `device`, `platform_mapping`, `rom_cache`, `collection_cache`, `downloaded_rom`, `download_job`, `local_asset`, `sync_journal`, `sync_conflict`, and `cache_entry`. The single `server_profile` row contains server metadata and a credential locator, never the token.

IPC envelopes are `{ schemaVersion, requestId, body }`. Initial commands are `agent.getStatus`, `settings.get`, `settings.update`, `connection.*`, `library.*`, `favorites.set`, `mappings.*`, `downloads.*`, `sync.*`, `conflicts.*`, and `diagnostics.export`. Events are `agent.statusChanged`, `connection.changed`, `library.changed`, `download.changed`, `sync.changed`, `conflict.changed`, and `controller.action`.

`AppError` contains `code`, `message`, `retryable`, optional `field`, optional redacted `details`, and optional `causeCode`. Stable error codes are documented and tested as part of the IPC contract.

### Failure handling

- A failed migration restores the pre-migration database and puts the app in diagnostics-only mode.
- A corrupt database is copied to a timestamped recovery file before a new empty cache database is created; user-managed ROM/save/state files are untouched.
- IPC disconnect triggers bounded reconnection with delays of 250 ms, 1 s, 4 s, and then 10 s until canceled.
- Agent/GUI schema mismatch blocks mutations and directs the user to finish or roll back the update.

### Tests and acceptance criteria

- Unit-test path selection, migrations, redaction, error serialization, process locking, and IPC version negotiation.
- Integration-test clean start, restart, forced termination, corrupt database recovery, and two simultaneous GUI launches.
- Acceptance: one agent owns the database; the GUI becomes interactive or displays a useful error within 10 seconds; no credential appears in the database or logs; each window-close preference produces the selected behavior after restart; the tray reopens the hidden GUI and can fully terminate it.

## 2. Server connection and pairing

### Objective

Establish a secure connection to one compatible RomM 5.x instance without storing a password.

### User experience

The user enters a server URL, sees the normalized host, tests it, and then enters an eight-character alphanumeric pairing code created in RomM and displayed as `XXXX-XXXX`. An advanced action accepts a Client API Token. Successful connection shows the account name, server version, and permission summary.

### Delivery milestones

- [x] **2.1 URL probing and compatibility** — Normalize URLs, probe public OpenAPI metadata, accept supported RomM 5.x servers, and classify offline/incompatible failures.
- [x] **2.2 Transport trust controls** — Require origin-scoped HTTP approval and support origin-scoped imported PEM/DER certificate authorities without a TLS bypass.
- [x] **2.3 Pairing and manual-token entry** — Support controller-friendly `XXXX-XXXX` codes, five-minute local expiry, secure clearing, manual-token validation, and unambiguous manual-token correlation.
- [x] **2.4 Identity, scopes, and persistence** — Verify `/api/users/me`, validate the complete required scope set, and persist credentials only after validation succeeds.
- [x] **2.5 Reconnect, logout, and repair UI** — Restore through a neutral startup screen, expose account/server/token/scope status, classify recovery actions, and clear local authentication safely.
- [x] **2.6 Connection validation** — Pass mocked API, TLS, IPC parity, controller dialog, Rust/frontend, production-build, and Windows runtime checks.

### Behavior and defaults

- Accept `http` and `https` URLs only. Remove trailing slashes while preserving a non-root base path.
- Reject embedded credentials, query strings, fragments, non-HTTP schemes, and non-host URLs.
- HTTPS is normal. HTTP requires a one-time confirmation naming the host and explaining that credentials and content are not transport-encrypted.
- Fetch `/openapi.json`, identify a RomM 5.x-compatible schema, then exchange `POST /api/client-tokens/exchange` with the normalized uppercase `XXXX-XXXX` code.
- Pairing codes are held only in memory and cleared after success, failure, cancel, or five minutes.
- Manual tokens must match `rmm_` followed by 64 hexadecimal characters before transmission.
- Validate authenticated identity and all required scopes after obtaining the token. Extra scopes are allowed but reported.
- Read paired-token ID and scopes from the exchange response. For a manual token, correlate two authenticated `GET /api/client-tokens` responses by the uniquely updated `last_used_at`; reject an ambiguous result and direct the user to pairing.
- Persist the token only after `GET /api/users/me` succeeds and the effective token/account scope intersection contains every required scope.
- Imported private CA certificates are stored as local configuration and applied only to the selected server host. There is no skip-verification setting.

### Data and interfaces

`ConnectionStatus` is `unconfigured | probing | pairing | connected | offline | unauthorized | incompatible | tls_error | scope_error` and includes redacted host, server version, account display name, last successful contact, and missing scopes.

Commands: `connection.probe(url, confirmHttp, caId?)`, `connection.exchangePairingCode(code)`, `connection.setManualToken(token)`, `connection.reconnect`, `connection.importCa(url, certificate)`, and `connection.logout(removeDevice: boolean)`.

The credential locator identifies a Windows Credential Manager item, Linux Secret Service item, or approved `0600` fallback file. Required scopes are exactly those listed in the master plan.

### Failure handling

- DNS, timeout, refused connection, and 5xx responses retain the current configuration and offer retry.
- A 401 transitions to `unauthorized`, pauses network work, and asks the user to pair again.
- A 403 identifies missing scopes and pauses only operations requiring them.
- Expired/used pairing codes are reported without retrying the same code.
- TLS hostname, trust-chain, and expiry failures remain blocking; diagnostics include non-secret certificate facts.
- Logout stops network jobs, clears the local credential and authenticated cache state, and optionally attempts device deletion before the token is removed. V1 does not request `me.write`, so local logout does not revoke the remote token; the UI reports the token ID for optional revocation in RomM.

### Tests and acceptance criteria

- Test URL normalization, base paths, malformed tokens, pairing expiry, single use, HTTP confirmation, private CA scoping, 401/403/5xx, and additive OpenAPI fields.
- Acceptance: a compatible server can pair by controller alone; an invalid or incompatible server cannot create a device; no code/token survives in UI state, logs, or SQLite.

## 3. Device registration

> **Implementation status (2026-08-28):** Feature 3 is complete through milestone 3.5. IPC schema v9 includes device proposal, registration, verification, update, and structured logout-removal outcomes; SQLite schema v4 persists one nullable-remote-ID device record plus its registration state. Platform/hostname detection, stable proposed-name restoration, authenticated registration, request fingerprinting, serialized operations, reconnect verification, deletion/permission recovery, and re-registration without loss of mappings or downloaded data are implemented. The controller-accessible device settings dialog reports the durable identity and coalesces valid renames for five seconds. Logout preserves the RomM device by default, offers separately confirmed best-effort removal, always clears local authentication, and reports when remote removal fails.

### Objective

Represent the installation as one persistent RomM device used for bidirectional synchronization.

### User experience

After pairing, the user reviews a generated device name such as `Steam Deck - Living Room` or `Windows PC - JUSTIN-DESKTOP`. The name is editable before registration and later in Settings.

### Delivery milestones

- [x] **3.1 Device API and local model** — Confirm the RomM 5.x contract, add device IPC/status types and SQLite fields, detect platform/hostname, and generate an editable default name.
- [x] **3.2 Basic registration** — Register with `push_pull`, persist the returned identity/fingerprint, and guarantee one active local device record without discarding valid authentication on failure.
- [x] **3.3 Registration UI** — Add the resumable, controller-complete name/review/progress/error step between authentication and mapping setup.
- [x] **3.4 Verification and recovery** — Verify on reconnect, detect remote deletion or permission loss, pause dependent sync, and re-register without losing mappings or downloads.
- [x] **3.5 Device settings and validation** — Show status, coalesce renames, support best-effort removal during logout, and pass persistence, restart, API, UI, and controller tests.

### Behavior and defaults

- Register after authentication and before the first sync using `sync_mode: push_pull`, `allow_existing: true`, and `allow_duplicate: false`.
- Send platform (`steamos`, `windows`, or `linux`), hostname, client name/version, and the configured ROM/save/state mapping summary in `sync_config`. Do not collect or send IP/MAC addresses.
- Store the returned device ID and registration fingerprint.
- Serialize registration inside the agent. If an earlier request already registered the local singleton, return that identity without another network request. RomM's hostname/platform matching and `allow_existing: true` make a retry safe if the server accepted a request but local persistence failed.
- Update the device when its name or root mapping summary changes, coalescing updates for five seconds.
- On startup, verify the cached device still exists and belongs to the token identity.
- Persist registration state as `unregistered`, `registered`, `missing`, or `permission_error`. Only a successful authenticated lookup or registration permits future device-dependent synchronization.

### Data and interfaces

The singleton `device` row stores a local UUID, nullable RomM device ID, display name, platform, hostname, client name/version, sync mode, registration fingerprint/state, mapping-summary JSON, created/updated timestamps, registration timestamp, and last verified timestamp. An unregistered draft has no RomM device ID or registration fingerprint. A missing record retains its former remote ID until replacement registration succeeds so diagnostics can identify what was removed.

Commands: `device.proposeName`, `device.register(name)`, `device.update(name)`, and `device.verify`. IPC v9 exposes them as `proposeDevice`, `registerDevice`, `updateDevice`, and `verifyDevice`. Verification returns `verified`, `missing`, or `permission_denied` with the persisted device identity. A successful registration returns the authoritative device identity plus whether RomM created a new record; a successful update returns RomM's authoritative device representation. Logout returns `not_requested`, `removed`, `already_missing`, or `failed` for the optional device-removal attempt, including a sanitized error on failure. The complete nullable device identity is included in `AgentStatus`.

### Failure handling

- If the server-side device was deleted, pause sync and offer a one-button re-registration that reuses mappings.
- A verification 404 marks only the registration as missing. A 403 marks it permission-blocked and directs the user to restore `devices.read`; neither condition deletes the remote ID, local UUID, mappings, downloads, or game data.
- DNS, timeout, connection-refusal, 5xx, invalid-response, and account-mismatch failures do not rewrite the last durable registration state. They block entry to device-dependent work and expose retry/sign-out recovery.
- If registration fails after pairing, keep the credential and resume onboarding at this step.
- Persist the edited local draft before attempting registration. Network, authorization, server, parse, and local-save failures never clear the authenticated server profile.
- Keep restored and newly authenticated sessions on a neutral preparation screen until device state resolves. An unregistered draft opens the device step with focus on its name; a registered device proceeds to the library. Until folder mapping is implemented, successful registration advances to the existing library as the temporary next surface.
- A name collision does not silently rename the device; show the server response and request a different name if required.
- Normal logout is the primary/default action and keeps the RomM device registration. A separate confirmed action attempts device deletion before logout. Deletion and an already-missing response clear only the remote registration fields while preserving the local UUID, name, mappings, and downloaded data; failure preserves the last durable device record and never blocks local credential removal.
- After either logout choice completes, the GUI immediately treats the session as unauthenticated and returns to server authentication even if its last polled agent-status snapshot still says `connected`; the retained local device is not shown as registration onboarding until authentication succeeds again.

### Tests and acceptance criteria

- Test authenticated create and existing-device responses, safe payload fields, owner matching, scope loss, 404 deletion, server/network failure, simultaneous requests, schema migration, atomic persistence, re-registration state preservation, rename coalescing, and interrupted onboarding.
- Acceptance: exactly one active local device ID exists; deleting it in RomM produces a recoverable state; re-registration preserves paths and local downloads; rapid name edits produce one update after five quiet seconds; controller focus reaches name editing, save, close, normal logout, optional removal, and cancel; a failed removal still signs out locally and explains the remaining server record.

## 4. Controller and spatial navigation

> **Implementation status (2026-08-30):** Milestones 4.1 through 4.5 are complete. The Tauri process owns a tested, platform-neutral native controller state machine over `gilrs`, discovers and hot-plugs devices, promotes the most recently active controller, classifies controller families, and emits normalized actions. React coalesces native and Steam Input events and routes both through the same deterministic focus graph. IPC schema v10 persists validated global and stable-hardware-GUID binding profiles, a 10-50% stick dead zone, a 200-1000 ms initial repeat delay, and a 50-300 ms repeat interval. The controller settings dialog exposes live input, per-controller fallback/reset behavior, collision-safe binding cycling, protected Confirm/Back bindings, and shoulder-button profile tabs. Search focuses the loaded-library filter, Context opens the focused game's action surface, and helper glyphs reflect rebound physical buttons. Every implemented text field has a controller-only virtual-keyboard entry path with cursor-aware editing, field-specific validation, direct desktop editing, modal focus containment, and invoking-field restoration. Input-modality styling now hides the spatial ring after pointer/touch use without clearing logical focus and restores it on keyboard/controller input; dialogs trap Tab, support Escape and backdrop dismissal, controls have accessible names/states and 44-pixel targets, game cards use button semantics, forced-colors focus remains visible, and CSS plus Motion honor reduced-motion preferences. Milestone 4.6 retains only the physical-device qualification that cannot be automated in this workspace.

### Objective

Make every v1 workflow operable with a Steam Deck or standard controller without sacrificing desktop inputs.

### User experience

The most recently used input method determines visible glyphs. Focus is always visible for controller/keyboard navigation and returns to the invoking item after closing a dialog or detail surface.

### Delivery milestones

- [x] **4.1 Native controller pipeline** — Discover controllers in Rust, normalize event delivery, handle hot-plugging and active-controller promotion, suppress duplicate native/Steam Input actions, and keep the state machine platform-neutral for Windows and Linux.
- [x] **4.2 Spatial focus graph** — Give every implemented route and interactive region deterministic entry and exit targets, bound modal focus with invoking-control restoration, remove disabled controls from spatial navigation, preserve focus across loading/refresh/pagination, and recover safely when focused content disappears. Future feature surfaces extend the same graph before their owning milestone is accepted.
- [x] **4.3 Actions, bindings, and glyphs** — Route Search, Context, previous-tab, and next-tab actions; persist global/per-controller bindings by stable hardware GUID; apply validated dead-zone and repeat timing live; prevent collisions and simultaneous Confirm/Back unbinding; expose controller settings with live feedback; and render rebound physical glyphs for PC/Xbox/PlayStation/Nintendo/Steam families.
- [x] **4.4 Controller text entry and input parity** — Provide every implemented text field with controller-only virtual-keyboard activation, cursor-aware editing, safe Back-to-Backspace routing, mode-specific filtering/validation, direct pointer/keyboard editing, modal isolation, and focus restoration without changing native desktop field behavior.
- [x] **4.5 Accessibility and automated validation** — Implement modality-aware and forced-colors focus styling, reduced motion, accessible labels/states, modal Tab/Escape containment, 44-pixel targets, keyboard/mouse/touch action parity, and automated focus/accessibility regressions.
- [ ] **4.6 Physical-device qualification** — **Deferred test gate:** record built-in Steam Deck, Xbox-family, and PlayStation-family controller results on Windows and Steam Deck/SteamOS hardware before Feature 4 and the release candidate are declared complete.

### Behavior and defaults

- Normalize native events into `up`, `down`, `left`, `right`, `confirm`, `back`, `search`, `context`, `previousTab`, and `nextTab`.
- Default bindings: D-pad/left stick for direction, south/A for confirm, east/B for back, west/X for context, north/Y for search, and shoulder buttons for tabs.
- Apply a 0.25 stick dead zone, 350 ms initial repeat delay, and 100 ms repeat interval. A direction must return below the dead zone before an opposite-direction press is accepted.
- The most recently active controller is primary. Disconnecting it promotes the next active device and displays a non-blocking notice.
- Steam Input keyboard equivalents are arrow keys, Enter, Escape, `x`, `/`, PageUp, and PageDown. Standard Tab/Shift+Tab remain accessible.
- Use spatial navigation groups for the app shell, each shelf, dialogs, menus, forms, and virtualized grids. Explicit escape targets prevent focus traps.
- Pointer use hides the focus ring only until the next directional/keyboard action; it never clears the logical focus key.
- User bindings are per physical mapping GUID, with a global fallback. Reserved confirm/back actions cannot both be unbound.
- Virtual-keyboard state stores a canonical value and raw cursor index. Pairing codes store eight uppercase alphanumeric characters and render/submit the separator without counting it as editable input; token entry accepts the `rmm_` prefix plus its hexadecimal body; URL entry rejects whitespace/control characters while preserving internationalized host text; general text preserves Unicode while rejecting control characters.
- Controller Back performs one cursor-aware backspace while the keyboard is open and never closes the keyboard or navigates the underlying view. Explicit Cancel discards the edit; Done commits only a valid value. Pointer or physical-keyboard focus on an ordinary field never opens the virtual keyboard.
- Modal surfaces cycle physical Tab and Shift+Tab through enabled controls, close through Escape/controller Back/their explicit action, and restore their invoking key. Pointer/touch backdrop dismissal is supported where it cannot commit a destructive choice.
- Interactive targets are at least 44 CSS pixels in both desktop and compact layouts. Forced-colors mode uses the system Highlight color for focus and selection; reduced-motion mode disables repeated decorative movement and delegates Motion animations to the operating-system preference.

### Data and interfaces

Rust emits `ControllerAction { action, phase, controllerId, controllerName, mappingFamily, timestampMs }`, where phase is `pressed | repeated | released`. A retained `ControllerStatus` snapshot exposes connected controller identities, the active controller, the triggering connect/disconnect/active change, and its timestamp so the WebView cannot miss startup discovery. Raw axis values never cross IPC.

React focusable components expose stable `focusKey`, `group`, accessible label, and optional preferred entry child. Settings store controller GUID, mapping family, bindings, dead zone, and repeat values.

### Failure handling

- Unknown controllers use the generic Xbox-style mapping and generic glyphs.
- Duplicate native and Steam Input events within 30 ms are coalesced by action and phase.
- If layout measurement fails after a route transition, focus falls back to the route heading and retries once after the next animation frame.
- A disconnected controller never blocks keyboard/mouse/touch input.
- Invalid or incomplete controller-entered values remain editable with an inline repair message and a disabled Done action. Cancel and successful submission restore the invoking field's stable focus key even when the input method changes while the keyboard is open.

### Tests and acceptance criteria

- Unit-test controller discovery state, independent axes, direction neutralization, active-controller promotion, mapping-family fallbacks, event serialization, duplicate suppression, dead zones, repeat, binding validation, focus restoration, virtual-keyboard normalization, cursor mapping, insertion, backspace/delete boundaries, and field validation. Tests owned by later milestones remain unchecked until those policies are implemented.
- Component-test every route, shelf edge, modal, menu, virtualized list, and loading-state replacement.
- Physically test built-in Steam Deck controls plus one Xbox and one PlayStation-family controller on both targets.
- Acceptance: all onboarding, browse, search, download, mapping, sync, conflict, settings, and update actions complete without pointer or keyboard; no reachable state loses focus.
- Milestone 4.5 acceptance: every currently implemented action has matching controller, physical-keyboard, and pointer/touch access; pointer use suppresses only the decorative spatial ring; keyboard/controller input restores it; modal Tab focus cannot escape; reduced-motion and forced-colors fallbacks remain usable; automated frontend, Rust, lint, and release-build checks pass.
- Milestone 4.6 acceptance: record controller name/mapping family, target OS/build, connect/disconnect behavior, every normalized action, glyphs, text entry, modal recovery, and suspend/resume for built-in Steam Deck controls plus one Xbox-family and one PlayStation-family controller. This evidence is required before Feature 4 is marked complete.

## 5. Onboarding

> **Implementation status (2026-08-31):** Feature 5 is complete through milestone 5.5. IPC schema v14 adds serialized `startInitialRefresh`; SQLite schema v7 adds server-origin ownership for normalized ROM cache rows. The final screen automatically requests the fixed 48-game first page, and the agent commits that page plus the `first_refresh` completion marker in one transaction before the library is shown. Empty libraries complete with an explicit empty state; transport/server failures retain all seven prior steps and expose controller-focusable retry while allowing the app to close and resume later. Device registration/verification no longer fetches library data before mappings and preferences are complete. The 5.3 correction remains schema v6, and 5.4 background registration remains opt-in. Physical SteamOS startup qualification remains in Feature 12; save/state hashing and inventory remain Feature 13.1 and do not block onboarding or cause this milestone to scan user files.

### Objective

Convert a fresh installation into a connected, mapped, optionally background-synchronizing client without requiring controller text entry beyond URL and pairing code.

### User experience

An eight-step wizard displays progress, explains why each permission is needed, validates before advancing, and saves completed non-secret steps. Back navigation does not discard valid state.

### Delivery milestones

- [x] **5.1 Resumable onboarding state machine** — Persist a validated, versioned, non-secret eight-step state; restore the first incomplete step; support non-destructive Back, Cancel, and Resume; preserve an explicit pause; and invalidate only server-dependent progress while retaining reusable custom path drafts.
- [x] **5.2 Connection and device steps** — Join connection and Feature 3 registration into one controller-complete sequence; advance agent-owned markers only after successful validated operations; reconcile existing saved installations; invalidate dependent progress on logout, authorization loss, server replacement, or device deletion; and render the shared eight-step progress shell.
- [x] **5.3 Detection, mappings, and archive review** — Detect local candidates without writing, provide controller-complete review and archive-policy editing, validate the accepted outcome, persist drafts and accepted rows by normalized server origin, and prevent one server from satisfying another server's onboarding state.
- [x] **5.4 Background and update choices** - Offer opt-in background startup and stable update checks without undoing earlier progress if either setup fails.
- [x] **5.5 First refresh and interruption validation** — Serialize the first-page refresh in the agent, atomically persist origin-scoped first-shelf metadata and completion, enter the library only afterward, and preserve/retry every wizard boundary without replaying earlier remote mutations.

### Behavior and defaults

- Steps are server, authentication, permissions, device, detection, mappings, background sync, and first refresh.
- Persist a `highestCompletedStep` and step data only after validation.
- Resume at the first incomplete/invalid step after restart.
- A changed server URL invalidates authentication, device, mappings proposed from server platforms, and initial refresh, but preserves custom path drafts for explicit reuse.
- Background startup defaults off and requires an explicit toggle.
- The final step starts the first metadata refresh. The user enters the library only after the first shelf or authoritative empty result is persisted. Save/state hashing and filesystem inventory begin with Feature 13.1 after its association and path-safety contracts exist; onboarding never performs a premature recursive scan of user-managed roots.

### Data and interfaces

`OnboardingState` contains version, current step, completed steps, acknowledged HTTP warning host, selected platforms, mapping draft IDs, and background choice. Credentials are referenced, not embedded.

IPC v14 exposes `getOnboarding`, `updateOnboarding { action: back | cancel | resume }`, the mapping operations, `configureOnboardingPreferences { backgroundEnabled, stableUpdateChecksEnabled }`, and `startInitialRefresh`. Completion transitions remain agent-internal and must be invoked only after the corresponding operation validates successfully; there is deliberately no public `completeStep` request. `startInitialRefresh` is serialized, requests offset zero with the fixed 48-item limit, and returns the page plus authoritative onboarding state only after the page and completion marker commit together. State is stored as the `onboarding_state` key in the agent-owned `app_state` table. `AppSettings.stableUpdateChecksEnabled` persists only the opt-in; signed network checks and installation remain Feature 15 work.

SQLite schema v7 adds `server_origin` to `rom_cache` and stores the initial-page snapshot with a matching cache-origin marker. A successful replacement deletes only disposable metadata rows for that origin inside the same transaction; it never deletes mappings, downloaded ROMs, saves, states, or other user-managed data.

The GUI composes existing connection/device/mapping/agent commands; onboarding has no privileged bypass command.

### Failure handling

- Each step reports errors inline and retains editable values.
- Loss of connectivity after authentication permits exit and later resume.
- Failure to register background startup does not undo pairing or mappings; onboarding records the choice as disabled and offers diagnostics.
- Cancel before authentication leaves no server profile; cancel afterward keeps the valid profile and resumes later.

### Tests and acceptance criteria

- Test fresh start, interruption at every step, backward navigation, changed URL invalidation, controller keyboard use, detection with no results, and partial first refresh.
- Acceptance: restarting at any point resumes without repeating completed remote mutations; completion creates one profile/device and at least one valid platform mapping or an explicit no-platform configuration.
- Milestone 5.2 acceptance: a compatible probe advances Server; durable validated credentials advance Authentication and Permissions; a registered or verified existing identity advances Device; existing installations resume at Folder detection without creating a second device; affected markers roll back on logout, authorization loss, server replacement, or server-side device deletion; connection, device, and library handoff screens show one consistent eight-step state; Rust, frontend, TypeScript, and release-build checks pass.
- Milestone 5.3 acceptance: detection reads the RomM platform catalog and only existing recognizable local directories; review persists origin-scoped drafts and advances Detection; valid existing absolute paths or an explicitly confirmed no-platform outcome persist atomically and advance Mappings; archive policy defaults to Keep and is editable per platform; SQLite schema v6 migrates existing mappings to their recorded origin, replacement selections disable stale same-origin rows without deletion, and restart reconciliation never applies one server's mappings to another; no scan, review, validation, or save operation creates a directory or modifies game data.
- Milestone 5.4 acceptance: both choices default off; applying background startup is idempotent and uses an unelevated per-user Windows Run value or SteamOS user service; disabling removes registration without deleting settings or content; stable update checks are persisted but no unsigned/unconfigured update is contacted; successful application advances only Background; a registration failure preserves all earlier progress, remains at Background with background disabled after an enable failure, and offers retry or foreground/tray continuation; IPC parity, controller focus, backward-compatible settings, Rust, frontend, TypeScript, and release-build checks pass.
- Milestone 5.5 acceptance: the final screen starts exactly one first-page request at a time; a one-of-107 partial page, full page, or authoritative empty page becomes usable before library handoff; metadata and `first_refresh` completion persist atomically by server origin; network/5xx/local-save failures keep all seven prior markers and expose retry; every persisted prefix of the eight steps restores its first gap without repeating completed work; no onboarding refresh touches game files; Rust, frontend, TypeScript, formatting, production-build, and documentation checks pass.

## 6. Library browsing

> **Implementation status (2026-09-01):** Milestones 6.1 through 6.4 are complete. IPC schema v17 and SQLite schema v10 provide normalized, origin-scoped library records, view-scoped exact page snapshots, full game-detail snapshots, source/staleness reporting, and retryable offline fallback. The catalog requests stable 48-item pages, reconciles cursors and totals outside React render timing, serializes automatic/manual/refresh requests, preserves known totals and focused content, replaces removed rows on refresh, and stops a malformed empty `hasMore` page from looping. Home loads recent additions, read-only favorites, downloaded games, and active downloads; typed platform, standard collection, and smart collection views page independently. Six controller/mouse/keyboard tabs have deterministic focus on every switch. Server-backed 250 ms search, immediate offline cached search, combinable platform/collection/favorite/downloaded filters, three sort modes, and controller-safe complete game details are implemented. Milestone 6.5 is next for refined stale/empty/error states, authenticated artwork loading/caching, and final performance validation.

### Objective

Provide responsive online and offline discovery of the user's RomM library in a controller-first media layout.

### User experience

Home includes recent additions, favorites, platforms, collections, downloaded games, and current downloads. Search and filters open without losing the selected game's context. Game details show metadata, favorite state, remote/local file state, destination, archive rule, and applicable actions.

### Delivery milestones

- [x] **6.1 Library API and normalized cache model** — Define platform, collection, ROM, user-ROM, artwork-reference, and local-state contracts; persist them by normalized server origin; report live/cache source and 24-hour staleness; and use exact-page offline fallback without masking authentication failures.
- [x] **6.2 Complete paged catalog** - Retain 48-item offset pagination, deduplicate overlaps with fresh metadata, preserve progress totals, serialize near-end/manual/refresh requests, keep content and focus stable, stop no-progress loops, and validate 107-ROM plus 10,000-ROM catalog state.
- [x] **6.3 Home shelves and library views** — Add recent, favorites, platform, collection, downloaded, and active-download views with stable controller focus.
- [x] **6.4 Search, filters, and game details** — Add online/offline search, combinable filters, complete read-only detail metadata/local context, neutral cover behavior, deferred-action labels, and return-to-selection restoration.
- [ ] **6.5 Offline, empty, error, and performance validation** — Add stale/source indicators, cached fallbacks, retry/empty states, artwork placeholders, and the 10,000-ROM responsiveness checks.

### Behavior and defaults

- Fetch stable `id asc` pages of 48 items from RomM using `limit=48` and an explicit `offset`. Recent additions use `id desc`; favorites use `favorite=true`; platform, standard collection, and smart collection views use `platform_ids`, `collection_id`, and `smart_collection_id` respectively. Render the first page immediately and request the next page when the scroll view is within 600 CSS pixels of its end.
- Show `loaded of total games` when RomM returns a total; otherwise show `loaded games`. Keep a controller-focusable **Load more games** action visible while `hasMore` is true so loading never depends on scroll events.
- Refresh begins again at offset zero. Appended pages preserve existing order, replace duplicate metadata, and deduplicate by RomM ROM ID.
- Cache normalized ROM, platform, collection, user-ROM, and artwork-reference data.
- Search debounces text by 250 ms online and queries the local index immediately offline.
- Filters include platform, favorite, downloaded, and collection; sorting includes title, recently added, and release date.
- Pull-to-refresh/button refresh invalidates relevant queries but retains cached content until replacement succeeds.
- Artwork loads progressively with a neutral aspect-ratio placeholder and an accessible text fallback.
- Mark cached server content stale after 24 hours. Show `Offline` when unreachable and `Last updated <time>` when stale.
- Restore route, scroll anchor, filters, and focus key per session.

### Data and interfaces

`LocalGame` includes RomM ID, platform ID/name, title, summary, release date, artwork cache key, collection IDs, favorite, remote filename/size, local status/path, active download ID, and metadata update timestamp.

IPC v17 exposes `listLibrary { query, limit, offset }`, `getGameDetails { romId }`, legacy `listRoms` for the onboarding refresh path, and `getLibraryMetadata`. `LibraryQuery` permits `all`, `recent`, `favorites`, `platform`, `collection`, `smart_collection`, `downloaded`, and `active_downloads`; only the three ID-scoped kinds accept a positive `id`. Optional discovery fields are `search`, `platformId`, paired `collectionId`/`collectionKind`, `favoriteOnly`, `downloadedOnly`, and `sort: id | title | recently_added | release_date`. Search is trimmed, limited to 200 characters, and translated to RomM's `search_term`; filters use `platform_ids`, `collection_id` or `smart_collection_id`, and `favorite`; sort uses RomM's `name`, `id`, or `first_release_date` ordering. The v1 RomM cursor is `{ offset: u64, limit: 48 }`; page results include `items`, optional `total`, `hasMore`, `source: live | cache`, `refreshedAtMs`, and `stale`. Downloaded, active-download, and `downloadedOnly` queries are answered from local state and never contact RomM. The raw RomM adapter disables character, filter-value, and ROM-ID indexes on page reads because those indexes are not used to render cards. `getGameDetails` calls `GET /api/roms/{id}` and returns normalized metadata, files, collections, sibling ROMs, save/state/screenshot counts, manual/soundtrack availability, favorite/local status, and source/staleness.

SQLite schema v10 uses `library_rom`, `library_user_rom`, `library_artwork`, `library_page`, `library_platform`, `library_collection`, `library_snapshot`, and `library_game_detail`. `library_page` keys include normalized server origin plus the complete typed query key, offset, and limit, so pages from different shelves, searches, sorts, and filter combinations cannot collide; v8 pages migrate to the `all` view. Standard and smart collection IDs are disambiguated by collection kind. Local downloaded state and active download IDs are joined when pages and details are read instead of being copied into disposable remote payloads. When an exact filtered page is unavailable during a retryable outage, the cache evaluates the same tokenized case-insensitive search and filter rules over known normalized records. The v7 `rom_cache` remains only as a backward-compatible onboarding snapshot and migration source.

### Failure handling

- Network failure returns cache results when available and a non-blocking stale indicator.
- Missing/corrupt artwork is evicted and retried once; failure uses the placeholder.
- A game removed remotely is hidden after successful refresh but its local copy remains visible in Downloaded under an `Unavailable on server` state.
- Empty library, empty filter, and server error use distinct screens/actions.

### Tests and acceptance criteria

- Test exact and absent totals, overlapping pages, final short pages, cache normalization, search escaping, filter combinations, stale transitions, remote deletion, focus preservation after page append, controller activation of **Load more games**, and artwork failure.
- Milestone 6.1 acceptance: RomM 5.x ROM fields normalize into the typed cache; platform plus standard/smart collection metadata round-trips; equal ROM IDs from different origins remain isolated; exact pages restore with current local status; cached data becomes stale after 24 hours; retryable `5xx`/network failures return cached pages; `401` never falls back; IPC/TypeScript schema parity, migrations, Rust tests, frontend tests, type checking, formatting, and the production build pass.
- Milestone 6.2 acceptance: a 107-ROM fixture reports `48 of 107 games`, retains that total when an appended response omits it, reaches 107 unique ordered rows through offsets 0/48/96, and removes stale rows on offset-zero refresh; frontend and agent gates reject concurrent page requests; automatic loading begins at exactly 600 pixels; manual loading remains focusable and returns focus safely when it disappears; an empty page cannot repeat one cursor forever; 10,000 incremental records normalize within the two-second test budget; Rust/frontend tests, type checking, formatting, Markdown lint, and the production build pass.
- Milestone 6.3 acceptance: Home presents recent, favorite, downloaded, and active-download shelves from authoritative remote or local sources; platform, standard collection, and smart collection selectors open independently paged catalogs; the six top-level tabs work with pointer, keyboard, and configured shoulder actions and always focus the selected tab after a switch; every remote view has an isolated origin/view/offset/limit cache key; local views never make a RomM request; favorite display is read-only until Feature 7; empty local views explain that downloads have not started; IPC v16/TypeScript parity, SQLite v9 migration, RomM query translation, frontend navigation, Rust tests, TypeScript checking, formatting, Markdown lint, and the production build pass.
- Milestone 6.4 acceptance: online search waits 250 ms after the last edit while an offline session queries cached normalized records immediately; platform, standard/smart collection, favorite, downloaded, and sort controls combine into one stable query without concurrent-request gaps; downloaded-only discovery never contacts RomM; a detail request uses `GET /api/roms/{id}`, caches the complete normalized response by server origin, falls back only on retryable failures, and never masks `401`; the dialog shows metadata, files, activity, favorite/local status, destination, archive rule, and unavailable Feature 7/9 actions without claiming those mutations exist; closing by controller, keyboard, pointer, or backdrop restores the invoking game; neutral artwork remains visible until authenticated artwork caching lands in 6.5; IPC v17/TypeScript parity, SQLite v10 migration, Rust/frontend tests, type checking, formatting, Markdown lint, and the production build pass.
- Acceptance: a 107-ROM server first displays `48 of 107 games`, eventually displays all 107 unique games through automatic or explicit loading, issues no page request concurrently with another, and refresh replaces the list from offset zero.
- Acceptance: a 10,000-ROM mocked library remains navigable; first cached content renders within two seconds on reference hardware; all library data except uncached artwork remains browsable offline.

## 7. Favorites

### Objective

Allow the user to change personal RomM favorite state with immediate feedback and reliable reconciliation.

### User experience

Favorite can be toggled from a card context action or game details. The icon updates immediately. A failed mutation reverts visibly and offers retry.

### Delivery milestones

- [ ] **7.1 Favorite model and online API** — Add cached server state, typed read/write operations, authoritative responses, and permission/error classification.
- [ ] **7.2 Optimistic UI** — Toggle immediately from cards/details, serialize per-ROM writes, replace with server truth, and visibly roll back terminal failures.
- [ ] **7.3 Offline mutation journal** — Persist one final desired state per ROM, coalesce rapid toggles, survive restart, and expose a queued indicator.
- [ ] **7.4 Reconnect reconciliation and validation** — Replay in order, handle refresh races and 401/403/404 outcomes, then pass online/offline/controller acceptance tests.

### Behavior and defaults

- Apply an optimistic cache update and enqueue one mutation per ROM.
- Coalesce repeated offline toggles to the final desired state.
- Serialize mutations for the same ROM; mutations for different ROMs may run concurrently.
- On success, replace optimistic state with the server representation.
- On reconnect, submit queued changes in creation order and refresh affected user-ROM data.

### Data and interfaces

`favorites.set(romId, desired)` returns the authoritative state. `library_user_rom` stores server favorite state; `sync_journal` stores a pending desired state and base revision/update time.

### Failure handling

- 401 pauses the queue for re-pairing; 403 reverts the optimistic change and identifies the missing `roms.user.write` scope.
- 404 marks the ROM unavailable and removes its pending favorite change.
- Other terminal 4xx responses revert. Retryable failures retain the pending mutation and show a queued indicator.

### Tests and acceptance criteria

- Test rapid toggles, offline coalescing, restart, 401/403/404, server disagreement, and cache refresh during a pending mutation.
- Acceptance: online changes settle to server truth; offline toggles survive restart and converge after reconnect; no input sequence creates more than one pending final state per ROM.

## 8. EmuDeck detection and platform mappings

> **Implementation status (2026-08-31):** Milestones 8.1 and 8.2 are complete as part of onboarding 5.3. The editable, controller-compatible portion of 8.3 and server-origin-owned persistence from 8.5 are implemented and regression-tested. SQLite schema v6 backfills existing accepted rows, scopes active outcomes to the normalized server origin, and retains deselected rows in a disabled state. Directory browsing/confirmed creation, full access/free-space/symlink validation, removable-mount recovery, preset upgrades, and physical Steam Deck qualification remain in 8.3-8.5 and are not implied by completion of onboarding 5.3.

### Objective

Map RomM platforms to safe local ROM, save, and state locations using proposed EmuDeck conventions or fully custom paths.

### User experience

Detection presents proposed mappings with a confidence label and source. The user reviews each enabled platform and may browse or type different ROM/save/state paths. Nothing is created or watched until the mapping is accepted.

### Delivery milestones

- [x] **8.1 Platform catalog and mapping model** — Load RomM platforms and define versioned mapping, preset, archive-policy, filename-strategy, and per-field custom-override records.
- [x] **8.2 Steam Deck and EmuDeck detection** — Detect internal/microSD conventions and evidence without writing; leave Windows unconfigured unless recognizable configuration exists.
- [ ] **8.3 Controller-complete mapping editor** — Review, enable, browse/type, override every ROM/save/state path, choose archive policy, and confirm intentional directory creation.
- [ ] **8.4 Path and removable-storage safety** — Validate absolute/canonical paths, access, space, overlaps, symlinks, duplicate destinations, mounts, and automatic remount recovery.
- [ ] **8.5 Persistence, preset upgrades, and validation** — Preserve custom fields across detection/preset changes and pass Steam Deck, Windows, removable-media, and unintended-write tests.

### Behavior and defaults

- Detect SteamOS home/internal storage and mounted storage roots, then known EmuDeck configuration and directory conventions.
- Treat detection as a proposal. Never overwrite a custom mapping during later detection.
- Windows begins with no EmuDeck assumption unless recognizable EmuDeck configuration is present; otherwise the user chooses roots.
- Each platform has one ROM destination and zero or more save/state watch roots with filename rules.
- Validate absolute path, parent existence, read/write access, free space, overlapping roots, and whether removable storage is currently mounted.
- Create a missing final directory only after confirmation; never create a missing multi-level root from an unverified preset.
- Archive default is `keep`, except an accepted preset may propose another value and must show it during review.

### Data and interfaces

`PlatformMapping` contains local UUID, normalized server origin, RomM platform ID, preset ID/version, ROM root, save roots, state roots, archive policy, filename strategy, enabled flag, validation status, and `isCustom` flags per field. Accepted selections are updated transactionally: rows for the same origin are disabled first and current drafts are then upserted; rows are never deleted by selection changes.

Commands: `mappings.detect`, `mappings.validate(draft)`, `mappings.save(draft)`, `mappings.disable(id)`, and `mappings.recheck`. Detection returns evidence and never writes.

### Failure handling

- An unavailable removable mount changes the mapping to `temporarily_unavailable`, pauses relevant downloads/sync, and automatically rechecks on mount/network events.
- Duplicate ROM destinations across platforms require explicit correction.
- Nested save/state roots are allowed only if their inclusion/exclusion rules do not overlap; otherwise validation blocks save.
- Permissions failures show the exact path and operation without exposing unrelated home-directory contents.

### Tests and acceptance criteria

- Test internal/microSD Steam Deck layouts, custom paths, missing mount recovery, symlinks, case differences on Windows, overlapping paths, read-only roots, and preset upgrade preserving overrides.
- Acceptance: accepting a preset places a test download where the external frontend expects; any individual ROM/save/state path can be overridden; unavailable storage never redirects writes to an unintended directory.

## 9. Download queue

### Objective

Download authenticated RomM content safely and resumably into configured platform destinations.

### User experience

The Downloads screen shows queued/running/paused/completed/failed jobs, progress, rolling speed, ETA, destination, and actions. Closing the GUI does not interrupt work when background mode is enabled.

### Delivery milestones

- [ ] **9.1 Persistent queue and destination snapshot** — Define jobs/events, FIFO ordering, archive-policy snapshotting, safe target ownership, and restart-safe states.
- [ ] **9.2 Resumable authenticated transfer engine** — Stream to `.part`, check space, persist validators/progress, use Range/If-Range, verify length/checksum, and finalize atomically.
- [ ] **9.3 Queue controls and concurrency** — Add pause/resume/cancel/retry/move-to-top, one-to-four concurrency, bounded progress events, rolling speed, and ETA.
- [ ] **9.4 Failure and lifecycle recovery** — Handle retry backoff, authentication errors, disk-full, mount loss, validator changes, GUI closure, and forced agent termination without corrupt output.
- [ ] **9.5 Downloads UI and validation** — Build controller-complete queue/history/actions and pass multi-gigabyte, restart, concurrent-job, filesystem, and network scenarios.

### Behavior and defaults

- Default concurrency is two, configurable from one through four.
- Queue order is FIFO with a user `Move to top` action; already-running jobs are not preempted.
- Download into a unique `.part` file in the destination filesystem to preserve atomic rename semantics.
- Before starting, require expected bytes plus 256 MiB free. If size is unknown, require 512 MiB and monitor space while streaming.
- Persist received bytes and validators (`ETag`, `Last-Modified`) every 1 MiB or two seconds.
- Resume with `Range` and `If-Range`. If the server returns full content or validators changed, truncate and restart after confirmation in logs/state, not a user prompt.
- Update displayed progress at most four times per second. ETA uses a rolling 10-second throughput window and remains hidden until three seconds of samples exist.
- Automatically retry transport/5xx failures three times after 1, 4, and 16 seconds. Manual retry resets the count.
- Validate server checksum when supplied; otherwise validate content length. Then hand the result to archive processing and atomically finalize.

### Data and interfaces

`DownloadJob` contains local ID, RomM ROM ID, source path, target mapping/path, archive policy snapshot, state, expected/received bytes, validators, speed, ETA, attempts, created/started/completed times, and typed error.

Commands: `downloads.enqueue`, `downloads.pause`, `downloads.resume`, `downloads.cancel`, `downloads.retry`, `downloads.moveToTop`, `downloads.removeHistory`, and `downloads.removeLocalCopy`. `download.changed` is the progress event.

### Failure handling

- Cancel removes the `.part` file after path ownership validation; pause keeps it.
- 401 pauses all jobs for re-pairing. 403/404 fail only the affected job.
- Disk-full immediately pauses the job, preserves the partial, and reports required space.
- Destination loss pauses all jobs for that mapping and waits for remount/revalidation.
- App/agent termination recovers `running` jobs as `paused_recoverable`, then resumes according to the user's prior pause state.

### Tests and acceptance criteria

- Test range/no-range servers, changed validators, unknown length, cancellation, pause/restart, concurrency changes, disk-full, remount, 401/403/404/5xx, and forced agent termination.
- Acceptance: an interrupted multi-gigabyte download resumes without corrupting its target; incomplete content is never visible at the final path; two default jobs run while later jobs remain ordered.

## 10. Archive processing and local copies

### Objective

Apply predictable per-platform archive rules without exposing the filesystem to unsafe archive entries or deleting user data implicitly.

### User experience

Game details and download jobs show the selected archive policy. Extraction has its own progress/state. Removing a local copy lists exactly which managed ROM/archive files will be removed and explicitly states that saves/states are retained.

### Delivery milestones

- [ ] **10.1 Archive policy and managed-file model** — Snapshot `keep`, `extract_keep`, or `extract_delete` per job and record every app-owned archive/installed path.
- [ ] **10.2 Safe ZIP and 7z staging** — Detect by signature, extract into a sibling temporary directory, enforce space/entry limits, and reject traversal, links, device paths, and normalized collisions.
- [ ] **10.3 Atomic finalization and cleanup recovery** — Validate/move files without overwriting unmanaged content, commit ownership records, then conditionally remove the archive with retryable cleanup errors.
- [ ] **10.4 Explicit local-copy removal** — Preview an immutable owned-file set, confirm it in the UI, validate paths again, and never include saves or states.
- [ ] **10.5 Adversarial archive and deletion validation** — Pass the malicious/corrupt corpus, partial failure, disk-full, collision, all-policy, and user-data preservation tests.

### Behavior and defaults

- Recognize ZIP and 7z by content signature, not extension alone.
- `keep` finalizes the downloaded file unchanged.
- `extract_keep` extracts into a temporary sibling directory, atomically moves validated entries into the platform destination, and retains the archive.
- `extract_delete` performs the same operation and deletes the managed archive only after all final moves and database writes succeed.
- Reject absolute paths, drive/UNC prefixes, `..` traversal, NULs, links, device files, and canonicalized paths outside the staging root.
- Reject ambiguous duplicate normalized paths, including case-insensitive collisions on Windows.
- Require enough free space for declared expanded size plus 256 MiB. Limit entry count to 100,000 and abort on size overflow or inconsistent archive metadata.
- Track every file created by a job. Local removal may delete only tracked managed files beneath the current validated ROM root.

### Data and interfaces

`downloaded_rom` records ROM ID, mapping ID, source filename, archive path if retained, install state, and validation timestamp. A child table records normalized path, size, checksum if calculated, and ownership type for every installed file.

Archive work is a download sub-state: `validating | extracting | finalizing`. Removal uses `downloads.previewRemoval` followed by `downloads.removeLocalCopy(previewToken)` so the confirmed list cannot be swapped.

### Failure handling

- Extraction failure removes only the job's staging directory and retains the valid archive unless the user explicitly cancels and removes it.
- Existing unmanaged destination files are never overwritten. The job stops with a collision report.
- Failure deleting an archive after successful extraction records `completed_with_cleanup_error`; installed files remain usable and cleanup can be retried.
- Save/state roots are excluded from removal even if misconfigured beneath a ROM root.

### Tests and acceptance criteria

- Test malicious traversal, absolute paths, links, case collisions, duplicate entries, corrupt/truncated archives, disk-full, existing files, huge entry counts, and all three policies.
- Acceptance: no archive can write outside its staging/destination roots; removal deletes only previewed managed ROM files; saves and states survive every removal and cleanup path.

## 11. Offline mode and local cache

### Objective

Keep the library useful without a server connection while clearly distinguishing cached remote facts from current local state.

### User experience

When offline, the shell displays an unobtrusive status and last successful refresh. Browse, search, mappings, local-copy removal, queue reordering, and settings remain available. Network actions show `Queued for reconnect` or explain why they cannot run.

### Delivery milestones

- [ ] **11.1 Persistent metadata and local-state cache** — **In progress:** Feature 6.1 supplies origin-scoped ROM/platform/collection/user/artwork records, exact page snapshots, downloaded/active-download joins, timestamps, and source/staleness fields; queue-wide facts, reset/recovery controls, and offline search remain.
- [ ] **11.2 Artwork LRU cache** — Add progressive storage, configurable 256 MiB-to-10 GiB limits, access/pin tracking, startup enforcement, missing-file recovery, and safe clearing.
- [ ] **11.3 Offline library and search UX** — Build the FTS index and retain browse, search, filters, settings, mapping, removal, and queue operations with clear stale/offline labels.
- [ ] **11.4 Reconnect convergence and cache reset** — Revalidate identity, replay favorites, resume downloads, reconcile sync, refresh stale pages in order, and keep game data separate from disposable cache resets.
- [ ] **11.5 Offline, corruption, and storage validation** — Pass restart-without-network, stale transitions, LRU, missing files, reset, disk-full, and no-user-data-deletion tests.

### Behavior and defaults

- Cache metadata indefinitely until replaced or explicitly cleared; mark it stale after 24 hours.
- Cache artwork under an LRU budget of 1 GiB, configurable from 256 MiB through 10 GiB. Enforce the budget at startup and after downloads, excluding pinned/currently displayed entries.
- Maintain an SQLite FTS index over title and platform name for offline search.
- Network reachability is inferred from actual RomM requests, not a generic internet probe.
- After reconnect, refresh identity/scopes, submit queued favorite mutations, resume eligible downloads, reconcile sync, and refresh stale library pages in that order.
- `Clear artwork cache` affects only disposable images. `Reset local cache` requires confirmation, preserves credentials/mappings/downloaded files, rebuilds associations from the managed-file records, and fetches metadata when online.

### Data and interfaces

`cache_entry` stores key, kind, local path, size, ETag, last access, and pinned state. `ConnectionStatus` supplies offline state. Library responses include `source` and `refreshedAt`.

Commands: `cache.status`, `cache.setLimit`, `cache.clearArtwork`, and `cache.resetMetadata`.

### Failure handling

- A missing cache file evicts its row and falls back without failing the page.
- Artwork write failure leaves metadata usable and reports one aggregated warning.
- Database recovery follows Application Foundation rules.
- Reconnect processing is resumable and never drops journal entries merely because refresh failed.

### Tests and acceptance criteria

- Test offline startup, FTS search, stale labels, LRU ordering/budget, missing files, reconnect ordering, cache reset, and no-disk-space behavior.
- Acceptance: previously loaded library metadata is searchable after restart with no network; clearing cache cannot delete ROM/save/state content; reconnect converges queued changes without user intervention.

## 12. Background sync agent

> **Implementation status (2026-08-31):** Milestone 5.4 implements the opt-in registration slice of 12.2-12.4: quoted HKCU per-user startup on Windows and a versioned user-level systemd service on Linux/SteamOS, with idempotent disable and no elevation. Registration affects the next user session and deliberately does not start or stop the agent that is serving the current request. Automated tests use an injected registrar and never alter the development machine's startup state. Full service health/status controls, current-session restart coordination, upgrade coordination, clean-login Windows runtime evidence, and physical Steam Deck qualification remain open, so no Feature 12 milestone is marked complete yet.

### Objective

Continue downloads and save/state synchronization reliably outside the GUI using an opt-in, user-level service.

### User experience

Settings shows whether the agent is foreground-only or registered, whether it is running/paused, its version, last heartbeat, active work, and last error. Enable/disable, pause, resume, restart, and `Sync now` are explicit actions.

### Delivery milestones

- [ ] **12.1 Foreground lifecycle and health** — **In progress:** complete single-instance foreground operation, heartbeat/status, pause/resume, graceful shutdown, stale-lock recovery, and GUI fallback behavior.
- [ ] **12.2 Background service control contract** — Add idempotent install/uninstall/enable/disable/restart commands, transactional binary staging, health checks, and foreground fallback on failure.
- [ ] **12.3 Windows per-user startup** — Install the stable sidecar path and quoted HKCU Run entry without elevation; verify paths with spaces, start-at-login, disable, and upgrade preservation.
- [ ] **12.4 SteamOS user service** — Install the versioned agent below `~/.local`, manage the validated `current` link and user-level systemd service, and verify Desktop/Gaming Mode behavior.
- [ ] **12.5 Lifecycle, suspend, and update validation** — Cover crash restart, sleep/resume, network restoration, active finalization, IPC/version coordination, and exactly-one-agent guarantees.

### Behavior and defaults

- One `romm-sync-agent` binary supports `--foreground`, `--service`, `--install-user-service`, and `--uninstall-user-service` modes; install/uninstall commands validate all target paths.
- Windows NSIS installs the agent at a stable per-user application path. Opt-in registration uses `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` with a quoted absolute command and no elevation.
- SteamOS copies the versioned agent below `~/.local/lib/romm-companion/`, updates a validated `current` link, writes `~/.config/systemd/user/romm-companion.service`, and invokes user-level enable/start. No system path is changed.
- Heartbeat is emitted every five seconds. The GUI considers the agent degraded after 15 seconds and disconnected after 30 seconds.
- Pause stops new sync/download work but lets atomic finalization finish. Shutdown allows 10 seconds for checkpoints before forced exit.
- Sleep/resume and network restoration schedule immediate reconciliation with a randomized zero-to-five-second delay.

### Data and interfaces

`AgentStatus` contains version, IPC schema version, mode, process ID, started time, heartbeat time, paused reason, background registration status, active download count, sync state, and last redacted error.

Commands: `agent.enableBackground`, `agent.disableBackground`, `agent.pause`, `agent.resume`, `agent.restart`, and `agent.getStatus`.

### Failure handling

- Installation is transactional: write to a temporary/versioned location, validate executable/version, update registration, start, health-check, then remove obsolete versions.
- Failure to register leaves foreground mode intact and provides platform-specific diagnostics.
- A crash is restarted by the OS service manager where available; the startup recovery journal prevents duplicate finalization.
- Stale locks are reclaimed only after confirming the recorded process is absent and the IPC endpoint cannot respond.

### Tests and acceptance criteria

- Test install/uninstall idempotence, quoted paths with spaces, stale locks, crash recovery, sleep/resume, GUI close, pause during finalization, version mismatch, and upgrade rollback.
- Physically validate user services in SteamOS Gaming and Desktop modes and startup behavior on Windows.
- Acceptance: opt-in requires no elevation; work continues after GUI closure; disabling removes startup registration without deleting settings or content; only one agent runs per user.

## 13. Save and state synchronization

### Objective

Synchronize configured save files and save states bidirectionally with RomM while preserving every conflicting version.

### User experience

The Sync screen shows current phase, scanned files, uploads/downloads, last success, next reconciliation, and conflicts. `Sync now` is always available when connected. Conflict details compare device, timestamps, sizes, and paths and offer keep local, keep server, keep both, or dismiss after external resolution.

### Delivery milestones

- [ ] **13.1 Asset associations and SHA-1 inventory** — Map accepted save/state files to RomM ROM IDs, persist hashes/metadata, report unmatched files, and never upload speculative associations.
- [ ] **13.2 Watchers and stable-file detection** — Observe accepted roots, debounce events, verify size/mtime stability, serialize same-path work, and pause only missing mappings.
- [ ] **13.3 Device-sync protocol engine** — Implement negotiation, bounded upload/download execution, `.sync-part` validation, journaled results, and accurate session completion counts.
- [ ] **13.4 Lifecycle reconciliation and retry** — Run at startup, resume, reconnect, remount, manual request, and every 15 minutes; recover crashes without reusing closed sessions.
- [ ] **13.5 Conflict preservation and resolution** — Apply `keep_both`, create safe conflict filenames/records, expose keep-local/server/both/manual outcomes, and preserve both versions on failure.
- [ ] **13.6 End-to-end sync validation** — Pass watcher bursts, both directions, partial failure, crash/remount, concurrent edits, controller UI, and explicit no-play-session tests.

### Behavior and defaults

- Watch accepted save/state roots only. Debounce a path for two seconds after the last event, then require matching size/mtime from two reads 750 ms apart before hashing.
- Build SHA-1 inventories grouped by RomM ROM ID and asset type. Filename-to-ROM association comes from downloaded-file records and mapping filename strategies; unmatched files are reported but never uploaded speculatively.
- Trigger negotiation for stable changes, manual sync, startup, resume, network restoration, mapping remount, and a 15-minute reconciliation interval.
- Execute server operations with a maximum of two asset transfers concurrently and serialize writes to the same local path.
- Download to a sibling `.sync-part`, verify declared size/hash when available, then atomically move.
- Upload through the server-provided destination and record its returned asset identity before completing the operation.
- Complete every server sync session with accurate completed and failed counts. Retain failed operations for retry; do not reuse a closed session ID.
- For `keep_both`, preserve the existing file and place the incoming version beside it as `<stem>.conflict-<sanitized-device>-<UTC timestamp><ext>`, then create a `sync_conflict` row.
- Do not infer or submit play sessions.

### Data and interfaces

`local_asset` stores ROM ID, type (`save | state`), path, size, mtime, SHA-1, server asset ID if known, and last sync outcome. `sync_journal` stores session/operation IDs, direction, attempt, state, and result. `SyncConflict` stores both versions and resolution state.

Commands: `sync.now(scope?)`, `sync.pause`, `sync.resume`, `sync.status`, `conflicts.list`, and `conflicts.resolve(id, resolution)`. Resolution never deletes a version until the previewed target is confirmed.

### Failure handling

- A file changing during hash/transfer is deferred and re-queued.
- A missing mount pauses only mappings on that mount.
- 401/403 follow connection rules; retryable operations use the shared backoff policy and remain journaled.
- An agent crash before server completion re-negotiates rather than assuming prior operations failed; hashes make duplicate file transfer safe.
- Conflict resolution failure preserves both files and the unresolved record.

### Tests and acceptance criteria

- Test event bursts, stable-file detection, unmatched files, upload/download, partial failure, session completion counts, same-path serialization, crash recovery, remount, remote deletion, and every conflict resolution.
- Acceptance: local and server-only changes propagate in the correct direction; simultaneous changes leave two intact versions; missed events are caught within the next 15-minute reconciliation; no play-session endpoint is called.

## 14. Settings and diagnostics

### Objective

Expose all supported configuration and enough redacted evidence to troubleshoot public installations safely.

### User experience

Settings groups Connection, Storage & Mappings, Downloads, Sync, Controller, Appearance, Background Agent, Updates, and Diagnostics. Changed values validate before saving. Diagnostics can be reviewed before exporting.

### Delivery milestones

- [ ] **14.1 Versioned settings model and screens** — Expose every v1 setting in controller-complete groups, validate field changes, retain unknown safe values, and return normalized authoritative state.
- [ ] **14.2 Safe live and staged application** — Apply harmless changes immediately and stage mapping/storage changes behind active work with clear pause/cancel choices and restart requirements.
- [ ] **14.3 Diagnostics status and preview** — Collect versions, sanitized settings/mappings, schema/table counts, queue/sync summaries, recent errors, included categories, and redaction warnings.
- [ ] **14.4 Atomic redacted export** — Centralize secret/path redaction, exclude credentials/content/CAs/database, generate a temporary ZIP, choose a destination, and clean abandoned/failed exports.
- [ ] **14.5 Settings and seeded-secret validation** — Test every constraint, staged changes, downgrade preservation, destination failures, controller access, and zero seeded-secret matches in exports.

### Behavior and defaults

- Show server URL/status/version/account/scopes without showing the token.
- Allow mapping edits, archive policies, download concurrency (1-4), artwork cache limit, controller bindings/dead zone/repeat, reduced-motion override, background registration, and stable update checks.
- Apply safe changes immediately. Mapping changes with active work are staged until jobs complete or the user pauses/cancels them.
- Diagnostics export is a ZIP containing app/agent/OS/WebView versions, sanitized settings, mapping validation states, database schema version/table counts, queue/sync summaries, and redacted recent logs.
- Redact bearer/basic authorization values, `rmm_` tokens, pairing-code fields, URL user-info/query values, and configured sensitive path segments. Do not include database, ROMs, saves, states, artwork, imported CAs, or credentials.
- Generate the export in the cache directory and let the user choose its final destination. Delete abandoned temporary exports after 24 hours.

### Data and interfaces

`AppSettings` is versioned and contains only supported values. `settings.update` accepts a field mask and returns the normalized authoritative settings plus restart requirements.

`diagnostics.preview` returns included categories and redaction warnings; `diagnostics.export(destination)` creates the bundle atomically.

### Failure handling

- Invalid settings return field-specific errors without changing other fields.
- An unavailable mapping remains configured but disabled for work.
- Export failure removes the temporary bundle and reports the failing stage.
- Unknown settings from a newer version are retained during read/write when safe, preventing downgrade from silently erasing them.

### Tests and acceptance criteria

- Test every constraint, staged mapping edits, redaction variants/casing, path sanitization, export cleanup, read-only destinations, and downgrade preservation.
- Acceptance: every v1 default is inspectable; changing a setting produces the documented effect; seeded tokens/codes/authorization headers cannot be found in an exported diagnostics archive.

## 15. Updates and distribution

### Objective

Ship trustworthy Windows and Steam Deck builds and update the GUI, agent, and data schema as a coordinated unit.

### User experience

Stable update checks are opt-in during onboarding and configurable later. An available update shows version, release notes link, download size, and whether restart is required. The user chooses when to install; active transfers must be paused or completed first.

### Delivery milestones

- [ ] **15.1 Unified release version and migration contract** — Drive GUI, agent, IPC, schema, updater metadata, artifacts, and compatibility checks from one release version source.
- [ ] **15.2 Windows NSIS package** — Produce a clean per-user installer/uninstaller containing the GUI and target-triple sidecar with stable background-agent paths and no external runtime requirement.
- [ ] **15.3 Steam Deck AppImage package** — Build on the Ubuntu 22.04 baseline, embed/stage the x86-64 sidecar, add desktop/non-Steam instructions, and run without development libraries.
- [ ] **15.4 Signed updater and coordinated install** — Publish signed GitHub Release metadata/artifacts and implement opt-in check, download, signature/space validation, checkpoint, migration, agent replacement, and health commit.
- [ ] **15.5 Rollback, clean-machine, and documentation validation** — Restore prior binaries/database/config after simulated failures, preserve content and background registration, and pass clean Windows/Steam Deck install-update-uninstall checks.

### Behavior and defaults

- Produce signed Windows NSIS and x86-64 AppImage artifacts plus signed Tauri updater metadata in GitHub Releases.
- Treat official sidecar embedding as part of this final distribution feature: build `romm-sync-agent` for each target triple, stage it under Tauri `bundle.externalBin`, launch it through the native sidecar API, and verify that neither NSIS nor AppImage users must locate or install a second executable.
- Build Linux on an Ubuntu 22.04-compatible runner. Smoke-test the AppImage without development libraries installed.
- Version GUI, agent, IPC schema, migrations, and updater metadata from one release version source.
- Update sequence: download/verify signature, pause new work, checkpoint jobs, stage binaries, back up database/config, install GUI and agent, run transactional migrations, start/health-check agent, then commit and remove the backup after one successful launch.
- On failure, restore binaries/database/config and display the rollback reason. Never roll back ROM/save/state content.
- Windows install/uninstall is per-user by default. Steam Deck docs cover making the AppImage executable, desktop entry integration, optional background service, and manual `Add a Non-Steam Game` steps.
- Do not publish Flatpak, macOS, ARM Linux, or automatic Steam shortcut support in v1.

### Data and interfaces

`UpdateStatus` is `idle | checking | available | downloading | ready | installing | rollback | failed` and includes current/target version, progress, release URL, and typed error.

Commands: `updates.check`, `updates.download`, `updates.install`, and `updates.setEnabled`. Agent update coordination uses an internal authenticated IPC shutdown/checkpoint command unavailable to the WebView directly.

### Failure handling

- Invalid signatures or hashes permanently reject the artifact and report a security error.
- Network failure retains the current version and permits retry.
- Insufficient disk space blocks staging before any installed file changes.
- Failure to stop the agent defers installation; it never overwrites a running binary.
- Rollback artifacts are retained if rollback itself is incomplete and diagnostics-only recovery is offered.

### Tests and acceptance criteria

- Test fresh install/uninstall, paths with spaces, update/no-update, interrupted download, invalid signature, low disk, active work, migration failure, agent health failure, rollback, and background registration preservation.
- Acceptance: both artifacts install/run on clean target systems; a prior release updates without losing settings, queue, mappings, or content; simulated migration failure returns to the usable prior version.

## 16. Cross-cutting test matrix

### Objective

Define the evidence required to declare a feature and the complete v1 release ready.

### User experience

There is no direct UI beyond test-only diagnostics. Public release notes state tested operating systems, RomM versions, known limitations, and upgrade requirements.

### Delivery milestones

- [ ] **16.1 Shared fixtures and harnesses** — **In progress:** maintain mock RomM responses, API snapshots, gamepad streams, filesystem layouts, archives, interrupted transfers, and every released database schema.
- [ ] **16.2 Required continuous integration** — Run frontend/Rust tests and checks, production target builds, dependency/license audits, Markdown/internal/external-link validation, and publish actionable results on every change.
- [ ] **16.3 Security and data-integrity gates** — Make credential redaction, archive traversal, ownership/deletion, migration, synchronization-conflict, and forced-termination failures non-quarantinable release blockers.
- [ ] **16.4 Performance and resilience qualification** — Measure startup/cache render, controller latency, 10,000-ROM navigation, transfer overhead, idle resources, soak behavior, suspend/network/server/storage/disk/update interruptions.
- [ ] **16.5 Physical release qualification** — Record artifact, RomM/OS versions, controllers, tester/date, and every required Windows/Steam Deck matrix result before tagging v1.

### Behavior and defaults

CI is required on every pull request; physical-device and installer suites are required for release candidates. Tests use temporary, resolved directories and never touch a developer's real ROM/save/state folders.

### Data and interfaces

Maintain fixtures for RomM 5.x OpenAPI responses, a controllable mock RomM server, synthetic archives, gamepad action streams, interrupted transfer state, databases from every released schema version, and platform filesystem layouts.

### Failure handling

A failing required check blocks merge/release. Flaky tests are quarantined only with an issue, owner, expiry date, and a deterministic manual release check; security, migration, archive-safety, and data-loss tests may never be quarantined.

### Tests and acceptance criteria

| Area | Automated coverage | Windows physical | Steam Deck physical | Release requirement |
| --- | --- | --- | --- | --- |
| Foundation/IPC | Unit, integration, crash recovery, migrations | Required | Required | No corruption or duplicate agent |
| Pairing/device | Mock API, TLS, scopes, expiry | Required | Required | Pair and recover using controller |
| Controller/UI | Action streams, component focus graph | Xbox/PlayStation | Built-in + external | Every workflow controller-complete |
| Library/favorites | Pagination, cache, offline, mutation journal | Smoke | Smoke | 10,000-item fixture responsive |
| Mappings | Path/symlink/mount fixtures | Custom paths | Internal + microSD | No unintended destination writes |
| Downloads | Resume, retry, disk, termination | Multi-GB file | Multi-GB file | Atomic, resumable, validated output |
| Archives/removal | Malicious/corrupt fixture corpus | Smoke | Smoke | No traversal or unowned deletion |
| Offline/cache | FTS, LRU, reconnect | Required | Required | Offline restart and convergence |
| Agent lifecycle | Registration, pause, crash, update | Startup | Gaming/Desktop modes | No elevation; survives GUI close |
| Save/state sync | Protocol, watcher, conflict, crash | Custom paths | EmuDeck paths | Both directions; conflicts preserved |
| Diagnostics | Seeded-secret redaction scan | Smoke | Smoke | Zero seeded-secret matches |
| Installer/updater | Install/update/rollback pipeline | Clean VM/device | Clean user profile | Prior release upgrades and rolls back |

Additionally:

- Run TypeScript type checking, frontend unit/component tests, Rustfmt, Clippy with warnings denied, Rust tests, dependency vulnerability auditing, and license auditing.
- Run Markdown lint, internal-link validation, and external-link checks for both planning documents and user documentation.
- Measure cached home rendering, controller action latency, 10,000-ROM navigation, download throughput overhead, idle agent CPU, and idle memory on reference hardware. Release thresholds are: cached home within two seconds, input-to-focus response within 100 ms at the 95th percentile, idle agent below 1% CPU after settling, and no unbounded memory growth during a two-hour browse/sync soak.
- Run suspend/resume, network transition, token revocation, server restart, removable-storage removal, disk-full, forced termination, and update interruption scenarios.
- Perform a final terminology/scope audit against the master plan before tagging a release.

Release acceptance is all required matrix cells passing with recorded artifact versions, RomM version, OS build, controller models, and tester/date. Known failures in any data-integrity or security scenario block release.

## Deferred features

The following are post-v1 and must not be partially exposed in v1 UI, IPC, permissions, or packaging:

- Emulator launching, monitoring, or command templates.
- Embedded EmulatorJS or other in-app emulation.
- Play-session tracking or inferred playtime.
- Collection creation/editing and ROM metadata editing.
- Automatic ES-DE/Steam ROM Manager rescans or Steam shortcut management.
- Multiple RomM instances or account profiles.
- Flatpak, macOS, and ARM Linux distributions.
- Multiplayer and netplay.

Proposals for these capabilities require a master-plan revision before implementation.
