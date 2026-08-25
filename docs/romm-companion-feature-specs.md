<!-- markdownlint-disable MD013 MD024 -->

# RomM Companion v1 Feature Specifications

> **Status:** Implementation specification for the authoritative [RomM Companion v1 Master Plan](./romm-companion-plan.md). If the documents conflict, the master plan wins and this document must be corrected before implementation continues.

## Table of contents

1. [Application foundation](#1-application-foundation)
2. [Server connection and pairing](#2-server-connection-and-pairing)
3. [Device registration](#3-device-registration)
4. [Controller and spatial navigation](#4-controller-and-spatial-navigation)
5. [Onboarding](#5-onboarding)
6. [Library browsing](#6-library-browsing)
7. [Favorites](#7-favorites)
8. [EmuDeck detection and platform mappings](#8-emudeck-detection-and-platform-mappings)
9. [Download queue](#9-download-queue)
10. [Archive processing and local copies](#10-archive-processing-and-local-copies)
11. [Offline mode and local cache](#11-offline-mode-and-local-cache)
12. [Background sync agent](#12-background-sync-agent)
13. [Save and state synchronization](#13-save-and-state-synchronization)
14. [Settings and diagnostics](#14-settings-and-diagnostics)
15. [Updates and distribution](#15-updates-and-distribution)
16. [Cross-cutting test matrix](#16-cross-cutting-test-matrix)
17. [Deferred features](#deferred-features)

## Shared conventions

Every feature below specifies its objective, user experience, behavior/defaults, data/interfaces, failure handling, and tests/acceptance criteria. These conventions apply throughout:

- All timestamps crossing an interface are RFC 3339 UTC. SQLite stores UTC epoch milliseconds.
- IDs from RomM remain opaque 64-bit integers. Local IDs are UUID v4 values.
- Paths are stored as native absolute paths, displayed in platform-native form, and never assembled by string concatenation.
- Byte counts are unsigned 64-bit integers. UI sizes use IEC units.
- Recoverable domain failures return typed `AppError` values; programming errors may terminate the failing task but must not crash the agent.
- Destructive local actions require confirmation and never imply a server deletion.
- Logs use structured fields and must pass through the central redaction layer.
- Network mutations are idempotent where the RomM API permits; otherwise the local journal records the server result before retry can occur.

## 1. Application foundation

### Objective

Provide a reproducible Tauri/React application and a shared Rust foundation in which the agent exclusively owns network, database, and managed-file operations.

### User experience

The GUI starts to a loading shell immediately, connects to the existing agent or starts an ephemeral one, and then restores the last route and focused item. Startup displays a recoverable error screen if agent initialization fails; it never presents an indefinitely spinning window.

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
- Acceptance: one agent owns the database; the GUI becomes interactive or displays a useful error within 10 seconds; no credential appears in the database or logs.

## 2. Server connection and pairing

### Objective

Establish a secure connection to one compatible RomM 5.x instance without storing a password.

### User experience

The user enters a server URL, sees the normalized host, tests it, and then enters an eight-digit pairing code created in RomM. An advanced action accepts a Client API Token. Successful connection shows the account name, server version, and permission summary.

### Behavior and defaults

- Accept `http` and `https` URLs only. Remove trailing slashes while preserving a non-root base path.
- Reject embedded credentials, query strings, fragments, non-HTTP schemes, and non-host URLs.
- HTTPS is normal. HTTP requires a one-time confirmation naming the host and explaining that credentials and content are not transport-encrypted.
- Fetch `/openapi.json`, identify a RomM 5.x-compatible schema, then exchange `POST /api/client-tokens/exchange` with the exact eight-digit code.
- Pairing codes are held only in memory and cleared after success, failure, cancel, or five minutes.
- Manual tokens must match `rmm_` followed by 64 hexadecimal characters before transmission.
- Validate authenticated identity and all required scopes after obtaining the token. Extra scopes are allowed but reported.
- Imported private CA certificates are stored as local configuration and applied only to the selected server host. There is no skip-verification setting.

### Data and interfaces

`ConnectionStatus` is `unconfigured | probing | pairing | connected | offline | unauthorized | incompatible | tls_error | scope_error` and includes redacted host, server version, account display name, last successful contact, and missing scopes.

Commands: `connection.probe(url, caId?)`, `connection.exchangePairingCode(code)`, `connection.setManualToken(token)`, `connection.reconnect`, `connection.importCa(certificate)`, and `connection.logout(removeDevice: boolean)`.

The credential locator identifies a Windows Credential Manager item, Linux Secret Service item, or approved `0600` fallback file. Required scopes are exactly those listed in the master plan.

### Failure handling

- DNS, timeout, refused connection, and 5xx responses retain the current configuration and offer retry.
- A 401 transitions to `unauthorized`, pauses network work, and asks the user to pair again.
- A 403 identifies missing scopes and pauses only operations requiring them.
- Expired/used pairing codes are reported without retrying the same code.
- TLS hostname, trust-chain, and expiry failures remain blocking; diagnostics include non-secret certificate facts.
- Logout stops network jobs, clears the local credential and authenticated cache state, and optionally attempts device deletion before the token is removed.

### Tests and acceptance criteria

- Test URL normalization, base paths, malformed tokens, pairing expiry, single use, HTTP confirmation, private CA scoping, 401/403/5xx, and additive OpenAPI fields.
- Acceptance: a compatible server can pair by controller alone; an invalid or incompatible server cannot create a device; no code/token survives in UI state, logs, or SQLite.

## 3. Device registration

### Objective

Represent the installation as one persistent RomM device used for bidirectional synchronization.

### User experience

After pairing, the user reviews a generated device name such as `Steam Deck - Living Room` or `Windows PC - JUSTIN-DESKTOP`. The name is editable before registration and later in Settings.

### Behavior and defaults

- Register after authentication and before the first sync using `sync_mode: push_pull`.
- Send platform (`steamos` or `windows`), hostname, and configured ROM/save/state roots. Do not collect a MAC address; omit it unless RomM requires a nullable field.
- Store the returned device ID and registration fingerprint.
- Update the device when its name or root mapping summary changes, coalescing updates for five seconds.
- On startup, verify the cached device still exists and belongs to the token identity.

### Data and interfaces

The `device` row stores RomM device ID, display name, platform, hostname, sync mode, registration fingerprint, registration timestamp, and last verified timestamp.

Commands: `device.proposeName`, `device.register(name)`, `device.update(name)`, and `device.verify`. Device status is included in `AgentStatus`.

### Failure handling

- If the server-side device was deleted, pause sync and offer a one-button re-registration that reuses mappings.
- If registration fails after pairing, keep the credential and resume onboarding at this step.
- A name collision does not silently rename the device; show the server response and request a different name if required.
- Device deletion during logout is best-effort and never blocks local credential removal.

### Tests and acceptance criteria

- Test registration, rename coalescing, missing remote device, permission loss, and interrupted onboarding.
- Acceptance: exactly one active local device ID exists; deleting it in RomM produces a recoverable state; re-registration preserves paths and local downloads.

## 4. Controller and spatial navigation

### Objective

Make every v1 workflow operable with a Steam Deck or standard controller without sacrificing desktop inputs.

### User experience

The most recently used input method determines visible glyphs. Focus is always visible for controller/keyboard navigation and returns to the invoking item after closing a dialog or detail surface.

### Behavior and defaults

- Normalize native events into `up`, `down`, `left`, `right`, `confirm`, `back`, `search`, `context`, `previousTab`, and `nextTab`.
- Default bindings: D-pad/left stick for direction, south/A for confirm, east/B for back, west/X for context, north/Y for search, and shoulder buttons for tabs.
- Apply a 0.25 stick dead zone, 350 ms initial repeat delay, and 100 ms repeat interval. A direction must return below the dead zone before an opposite-direction press is accepted.
- The most recently active controller is primary. Disconnecting it promotes the next active device and displays a non-blocking notice.
- Steam Input keyboard equivalents are arrow keys, Enter, Escape, `x`, `/`, PageUp, and PageDown. Standard Tab/Shift+Tab remain accessible.
- Use spatial navigation groups for the app shell, each shelf, dialogs, menus, forms, and virtualized grids. Explicit escape targets prevent focus traps.
- Pointer use hides the focus ring only until the next directional/keyboard action; it never clears the logical focus key.
- User bindings are per physical mapping GUID, with a global fallback. Reserved confirm/back actions cannot both be unbound.

### Data and interfaces

Rust emits `ControllerAction { action, phase, controllerId, mappingFamily, timestamp }`, where phase is `pressed | repeated | released`. Raw axis values never cross IPC.

React focusable components expose stable `focusKey`, `group`, accessible label, and optional preferred entry child. Settings store controller GUID, mapping family, bindings, dead zone, and repeat values.

### Failure handling

- Unknown controllers use the generic Xbox-style mapping and generic glyphs.
- Duplicate native and Steam Input events within 30 ms are coalesced by action and phase.
- If layout measurement fails after a route transition, focus falls back to the route heading and retries once after the next animation frame.
- A disconnected controller never blocks keyboard/mouse/touch input.

### Tests and acceptance criteria

- Unit-test dead zones, repeat, duplicate suppression, binding validation, and focus restoration.
- Component-test every route, shelf edge, modal, menu, virtualized list, and loading-state replacement.
- Physically test built-in Steam Deck controls plus one Xbox and one PlayStation-family controller on both targets.
- Acceptance: all onboarding, browse, search, download, mapping, sync, conflict, settings, and update actions complete without pointer or keyboard; no reachable state loses focus.

## 5. Onboarding

### Objective

Convert a fresh installation into a connected, mapped, optionally background-synchronizing client without requiring controller text entry beyond URL and pairing code.

### User experience

An eight-step wizard displays progress, explains why each permission is needed, validates before advancing, and saves completed non-secret steps. Back navigation does not discard valid state.

### Behavior and defaults

- Steps are server, authentication, permissions, device, detection, mappings, background sync, and first refresh.
- Persist a `highestCompletedStep` and step data only after validation.
- Resume at the first incomplete/invalid step after restart.
- A changed server URL invalidates authentication, device, mappings proposed from server platforms, and initial refresh, but preserves custom path drafts for explicit reuse.
- Background startup defaults off and requires an explicit toggle.
- The final step starts metadata refresh and initial asset inventory independently; the user may enter the library once metadata needed for the first shelf is available.

### Data and interfaces

`OnboardingState` contains version, current step, completed steps, acknowledged HTTP warning host, selected platforms, mapping draft IDs, and background choice. Credentials are referenced, not embedded.

The GUI composes existing connection/device/mapping/agent commands; onboarding has no privileged bypass command.

### Failure handling

- Each step reports errors inline and retains editable values.
- Loss of connectivity after authentication permits exit and later resume.
- Failure to register background startup does not undo pairing or mappings; onboarding records the choice as disabled and offers diagnostics.
- Cancel before authentication leaves no server profile; cancel afterward keeps the valid profile and resumes later.

### Tests and acceptance criteria

- Test fresh start, interruption at every step, backward navigation, changed URL invalidation, controller keyboard use, detection with no results, and partial first refresh.
- Acceptance: restarting at any point resumes without repeating completed remote mutations; completion creates one profile/device and at least one valid platform mapping or an explicit no-platform configuration.

## 6. Library browsing

### Objective

Provide responsive online and offline discovery of the user's RomM library in a controller-first media layout.

### User experience

Home includes recent additions, favorites, platforms, collections, downloaded games, and current downloads. Search and filters open without losing the selected game's context. Game details show metadata, favorite state, remote/local file state, destination, archive rule, and applicable actions.

### Behavior and defaults

- Fetch pages of 100 items and prefetch the next page near the end of a shelf/grid.
- Cache normalized ROM, platform, collection, user-ROM, and artwork-reference data.
- Search debounces text by 250 ms online and queries the local index immediately offline.
- Filters include platform, favorite, downloaded, and collection; sorting includes title, recently added, and release date.
- Pull-to-refresh/button refresh invalidates relevant queries but retains cached content until replacement succeeds.
- Artwork loads progressively with a neutral aspect-ratio placeholder and an accessible text fallback.
- Mark cached server content stale after 24 hours. Show `Offline` when unreachable and `Last updated <time>` when stale.
- Restore route, scroll anchor, filters, and focus key per session.

### Data and interfaces

`LocalGame` includes RomM ID, platform ID/name, title, summary, release date, artwork cache key, collection IDs, favorite, remote filename/size, local status/path, active download ID, and metadata update timestamp.

Commands: `library.home`, `library.list(query, cursor)`, `library.game(id)`, `library.search(query, filters, cursor)`, and `library.refresh(scope)`. Results include `source: live | cache`, `refreshedAt`, and pagination cursor.

### Failure handling

- Network failure returns cache results when available and a non-blocking stale indicator.
- Missing/corrupt artwork is evicted and retried once; failure uses the placeholder.
- A game removed remotely is hidden after successful refresh but its local copy remains visible in Downloaded under an `Unavailable on server` state.
- Empty library, empty filter, and server error use distinct screens/actions.

### Tests and acceptance criteria

- Test pagination, cache normalization, search escaping, filter combinations, stale transitions, remote deletion, focus after page append, and artwork failure.
- Acceptance: a 10,000-ROM mocked library remains navigable; first cached content renders within two seconds on reference hardware; all library data except uncached artwork remains browsable offline.

## 7. Favorites

### Objective

Allow the user to change personal RomM favorite state with immediate feedback and reliable reconciliation.

### User experience

Favorite can be toggled from a card context action or game details. The icon updates immediately. A failed mutation reverts visibly and offers retry.

### Behavior and defaults

- Apply an optimistic cache update and enqueue one mutation per ROM.
- Coalesce repeated offline toggles to the final desired state.
- Serialize mutations for the same ROM; mutations for different ROMs may run concurrently.
- On success, replace optimistic state with the server representation.
- On reconnect, submit queued changes in creation order and refresh affected user-ROM data.

### Data and interfaces

`favorites.set(romId, desired)` returns the authoritative state. `rom_cache` stores server favorite state; `sync_journal` stores a pending desired state and base revision/update time.

### Failure handling

- 401 pauses the queue for re-pairing; 403 reverts the optimistic change and identifies the missing `roms.user.write` scope.
- 404 marks the ROM unavailable and removes its pending favorite change.
- Other terminal 4xx responses revert. Retryable failures retain the pending mutation and show a queued indicator.

### Tests and acceptance criteria

- Test rapid toggles, offline coalescing, restart, 401/403/404, server disagreement, and cache refresh during a pending mutation.
- Acceptance: online changes settle to server truth; offline toggles survive restart and converge after reconnect; no input sequence creates more than one pending final state per ROM.

## 8. EmuDeck detection and platform mappings

### Objective

Map RomM platforms to safe local ROM, save, and state locations using proposed EmuDeck conventions or fully custom paths.

### User experience

Detection presents proposed mappings with a confidence label and source. The user reviews each enabled platform and may browse or type different ROM/save/state paths. Nothing is created or watched until the mapping is accepted.

### Behavior and defaults

- Detect SteamOS home/internal storage and mounted storage roots, then known EmuDeck configuration and directory conventions.
- Treat detection as a proposal. Never overwrite a custom mapping during later detection.
- Windows begins with no EmuDeck assumption unless recognizable EmuDeck configuration is present; otherwise the user chooses roots.
- Each platform has one ROM destination and zero or more save/state watch roots with filename rules.
- Validate absolute path, parent existence, read/write access, free space, overlapping roots, and whether removable storage is currently mounted.
- Create a missing final directory only after confirmation; never create a missing multi-level root from an unverified preset.
- Archive default is `keep`, except an accepted preset may propose another value and must show it during review.

### Data and interfaces

`PlatformMapping` contains local UUID, RomM platform ID, preset ID/version, ROM root, save roots, state roots, archive policy, filename strategy, enabled flag, validation status, and `isCustom` flags per field.

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

### Objective

Continue downloads and save/state synchronization reliably outside the GUI using an opt-in, user-level service.

### User experience

Settings shows whether the agent is foreground-only or registered, whether it is running/paused, its version, last heartbeat, active work, and last error. Enable/disable, pause, resume, restart, and `Sync now` are explicit actions.

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

### Behavior and defaults

- Show server URL/status/version/account/scopes without showing the token.
- Allow mapping edits, archive policies, download concurrency (1-4), artwork cache limit, controller bindings/dead zone/repeat, reduced-motion override, background registration, and stable update checks.
- Apply safe changes immediately. Mapping changes with active work are staged until jobs complete or the user pauses/cancels them.
- Diagnostics export is a ZIP containing app/agent/OS/WebView versions, sanitized settings, mapping validation states, database schema version/table counts, queue/sync summaries, and redacted recent logs.
- Redact bearer/basic authorization values, `rmm_` tokens, eight-digit pairing-code fields, URL user-info/query values, and configured sensitive path segments. Do not include database, ROMs, saves, states, artwork, imported CAs, or credentials.
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

### Behavior and defaults

- Produce signed Windows NSIS and x86-64 AppImage artifacts plus signed Tauri updater metadata in GitHub Releases.
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
