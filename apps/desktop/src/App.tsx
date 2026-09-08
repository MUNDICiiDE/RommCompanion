import {
  FocusContext,
  getCurrentFocusKey,
  navigateByDirection,
  setFocus,
  updateAllLayouts,
  useFocusable,
} from "@noriginmedia/norigin-spatial-navigation";
import { listen } from "@tauri-apps/api/event";
import { useQuery } from "@tanstack/react-query";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import type { FormEvent, ReactNode, UIEvent } from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { requestAgent } from "./agent";
import { RomArtwork } from "./Artwork";
import {
  modalityForKeyboardEvent,
  nextTrappedFocusIndex,
  TABBABLE_SELECTOR,
  type InputModality,
} from "./accessibility";
import { formatPairingCode, remembersHttpApproval, REQUIRED_SCOPES } from "./connection";
import {
  defaultControllerSettings,
  effectiveControllerBindings,
  getControllerStatus,
  isCrossSourceDuplicate,
  keyboardControllerAction,
  nextControllerBinding,
  shouldHandleControllerAction,
  validateControllerSettings,
  type ControllerBindingAction,
  type RecentInputAction,
} from "./controller";
import {
  getDesktopSettings,
  updateDesktopSettings,
  type CloseBehavior,
} from "./desktop";
import {
  DEVICE_DISPLAY_NAME_MAX_LENGTH,
  DEVICE_RENAME_DEBOUNCE_MS,
  devicePlatformLabel,
  normalizePendingDeviceRename,
  resolvePrimaryView,
  validateDeviceDisplayName,
  type PrimaryView,
} from "./device";
import {
  applyRomPage,
  buildDiscoveryQuery,
  createLibraryRequestGate,
  createPerRomMutationQueue,
  DEFAULT_LIBRARY_FILTERS,
  HOME_SHELF_SIZE,
  LIBRARY_PAGE_SIZE,
  LIBRARY_TAB_LABELS,
  LIBRARY_TABS,
  LIBRARY_SORT_LABELS,
  libraryQueryKey,
  libraryTabFocusKey,
  nextLibraryTab,
  nextLibrarySort,
  queryForLibraryTab,
  setFavoriteInHomeShelves,
  setFavoriteInRoms,
  setRomFavorite,
  shouldAutoLoadLibrary,
  type LibraryCatalogState,
  type LibraryDiscoveryFilters,
  type LibraryTab,
} from "./library";
import {
  addMappingPath,
  ARCHIVE_POLICY_LABELS,
  issueForDraft,
  normalizeMappingPaths,
  removeMappingPath,
  updateMappingArchivePolicy,
  updateMappingEnabled,
  updateMappingPath,
  updateMappingPathAt,
  type MappingPathField,
} from "./mapping";
import {
  getOnboardingState,
  onboardingProgress,
  onboardingStepStates,
  ONBOARDING_STEPS,
  ONBOARDING_STEP_LABELS,
  requiresInitialRefresh,
} from "./onboarding";
import {
  directionalFocusTarget,
  libraryBottomFocusTarget,
  libraryRequestFocusTarget,
  primaryFocusTarget,
  registeredDirectionalFocusTarget,
  recoverLibraryFocus,
  type FocusDirection,
  type FocusNavigation,
  type RegisteredFocusNode,
} from "./focus";
import {
  controllerButtonGlyph,
  InputFamilyIcon,
  InputHint,
  inputFamilyLabel,
  type InputFamily,
} from "./InputGlyphs";
import type {
  AppError,
  AgentResponse,
  AuthResult,
  ControllerAction,
  ControllerBindings,
  ControllerSettings,
  ControllerStatus,
  DeviceIdentity,
  DetectionEvidence,
  DirectoryListing,
  FavoriteReconciliationResult,
  MappingValidationIssue,
  MappingPathValidation,
  LibraryMetadata,
  LibraryQuery,
  GameDetails,
  OnboardingState,
  OnboardingStep,
  PlatformMappingDraft,
  ProbeResult,
  RomSummary,
} from "./types";
import {
  VirtualKeyboard,
  VIRTUAL_KEYBOARD_COMMAND_EVENT,
  type VirtualKeyboardMode,
} from "./VirtualKeyboard";

const DIRECTION_ACTIONS = new Set(["up", "down", "left", "right"]);
const PAIRING_CODE_PATTERN = /^[A-Z0-9]{4}-[A-Z0-9]{4}$/;
const PAIRING_TTL_SECONDS = 5 * 60;
const authenticationNotice = (result: AuthResult) => {
  const messages = result.warning ? [result.warning] : [];
  const reconciliation = result.favoriteReconciliation;
  if (reconciliation) {
    if (reconciliation.applied > 0) {
      messages.push(`${reconciliation.applied} queued favorite ${reconciliation.applied === 1 ? "change was" : "changes were"} synced.`);
    }
    if (reconciliation.discarded > 0) {
      messages.push(`${reconciliation.discarded} queued favorite ${reconciliation.discarded === 1 ? "change was" : "changes were"} discarded because RomM could not apply ${reconciliation.discarded === 1 ? "it" : "them"}.`);
    }
    if (reconciliation.pausedBy && reconciliation.remaining === 0) {
      messages.push(reconciliation.pausedBy.message);
    }
    if (reconciliation.remaining > 0) {
      messages.push(`${reconciliation.remaining} favorite ${reconciliation.remaining === 1 ? "change remains" : "changes remain"} queued. ${reconciliation.pausedBy?.message ?? "Reconnect to try again."}`);
    }
  }
  return messages.length > 0 ? messages.join(" ") : null;
};
const formatBytes = (bytes?: number) => {
  if (!bytes) return "Unknown size";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const power = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  return `${(bytes / 1024 ** power).toFixed(power > 1 ? 1 : 0)} ${units[power]}`;
};
const CONTROLLER_BINDING_ACTIONS: Array<{
  action: ControllerBindingAction;
  label: string;
}> = [
  { action: "confirm", label: "Confirm" },
  { action: "back", label: "Back" },
  { action: "context", label: "Context" },
  { action: "search", label: "Search" },
  { action: "previousTab", label: "Previous tab" },
  { action: "nextTab", label: "Next tab" },
];

function OnboardingProgress({
  state,
  fallbackCurrent,
  compact = false,
}: {
  state: OnboardingState | null;
  fallbackCurrent: OnboardingStep;
  compact?: boolean;
}) {
  const currentStep = state?.currentStep ?? fallbackCurrent;
  const currentIndex = ONBOARDING_STEPS.indexOf(currentStep);
  const completedSteps = new Set(
    state?.completedSteps ?? ONBOARDING_STEPS.slice(0, Math.max(0, currentIndex)),
  );
  const progress = state
    ? onboardingProgress(state)
    : { completed: completedSteps.size, total: ONBOARDING_STEPS.length, percent: 0 };
  const steps = state
    ? onboardingStepStates(state)
    : ONBOARDING_STEPS.map((step) => ({
        step,
        label: ONBOARDING_STEP_LABELS[step],
        complete: completedSteps.has(step),
        current: step === currentStep,
      }));

  return (
    <div className={`onboarding-progress-shell ${compact ? "is-compact" : ""}`}>
      <p className="onboarding-progress-summary">
        Setup {progress.completed} of {progress.total} complete
      </p>
      <div className="onboarding-progress" aria-label="Onboarding progress">
        {steps.map(({ step, label, complete, current }) => {
          return (
            <span
              key={step}
              className={`${complete ? "is-complete" : ""} ${current ? "is-current" : ""}`.trim()}
              aria-current={current ? "step" : undefined}
            >
              {label}
            </span>
          );
        })}
      </div>
    </div>
  );
}

function responseError(response: AgentResponse): string | null {
  return response.type === "error" ? response.error.message : null;
}

interface FocusButtonProps {
  children: ReactNode;
  onPress: () => void;
  variant?: "primary" | "secondary" | "quiet";
  disabled?: boolean;
  focusKey: string;
  selected?: boolean;
  ariaLabel?: string;
  navigation?: FocusNavigation;
}

const registeredFocusNodes = new Map<string, RegisteredFocusNode>();

function useRegisteredFocusNode(
  focusKey: string,
  focusable: boolean,
  navigation?: FocusNavigation,
) {
  useEffect(() => {
    const node = { focusable, navigation };
    registeredFocusNodes.set(focusKey, node);
    return () => {
      if (registeredFocusNodes.get(focusKey) === node) {
        registeredFocusNodes.delete(focusKey);
      }
    };
  }, [focusKey, focusable, navigation]);
}

function syncSpatialFocus(focusKey: string) {
  if (getCurrentFocusKey() !== focusKey) setFocus(focusKey);
}

interface FocusDialogProps {
  boundaryKey: string;
  initialFocusKey: string;
  className: string;
  labelledBy: string;
  onDismiss: () => void;
  children: ReactNode;
}

function FocusDialog({
  boundaryKey,
  initialFocusKey,
  className,
  labelledBy,
  onDismiss,
  children,
}: FocusDialogProps) {
  const { ref, focusKey } = useFocusable({
    focusKey: boundaryKey,
    focusable: false,
    preferredChildFocusKey: initialFocusKey,
    isFocusBoundary: true,
    focusBoundaryDirections: ["up", "down", "left", "right"],
    trackChildren: true,
  });

  useEffect(() => {
    const frame = window.requestAnimationFrame(() => setFocus(initialFocusKey));
    return () => window.cancelAnimationFrame(frame);
  }, [initialFocusKey]);

  return (
    <FocusContext.Provider value={focusKey}>
      <motion.section
        ref={ref}
        className={className}
        role="dialog"
        aria-modal="true"
        aria-labelledby={labelledBy}
        tabIndex={-1}
        onKeyDownCapture={(event) => {
          if (event.key === "Escape") {
            event.preventDefault();
            event.stopPropagation();
            onDismiss();
            return;
          }
          if (event.key !== "Tab") return;
          const dialog = event.currentTarget;
          const tabbable = [...dialog.querySelectorAll<HTMLElement>(TABBABLE_SELECTOR)]
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
        initial={{ opacity: 0, scale: 0.96, y: 20 }}
        animate={{ opacity: 1, scale: 1, y: 0 }}
        exit={{ opacity: 0, scale: 0.98, y: 12 }}
      >
        {children}
      </motion.section>
    </FocusContext.Provider>
  );
}

interface FocusScreenProps {
  scopeKey: string;
  entryFocusKey: string;
  className: string;
  label: string;
  children: ReactNode;
  onScroll?: (event: UIEvent<HTMLElement>) => void;
  animatedOffset?: boolean;
}

function FocusScreen({
  scopeKey,
  entryFocusKey,
  className,
  label,
  children,
  onScroll,
  animatedOffset = false,
}: FocusScreenProps) {
  const { ref, focusKey } = useFocusable({
    focusKey: scopeKey,
    focusable: false,
    preferredChildFocusKey: entryFocusKey,
    saveLastFocusedChild: true,
    trackChildren: true,
    autoRestoreFocus: true,
  });

  useEffect(() => {
    const frame = window.requestAnimationFrame(() => {
      updateAllLayouts();
      setFocus(entryFocusKey);
    });
    return () => window.cancelAnimationFrame(frame);
  }, [entryFocusKey]);

  return (
    <FocusContext.Provider value={focusKey}>
      <motion.section
        ref={ref}
        className={className}
        aria-label={label}
        initial={{ opacity: 0, y: animatedOffset ? 10 : 0 }}
        animate={{ opacity: 1, y: 0 }}
        exit={{ opacity: 0, y: animatedOffset ? -8 : 0 }}
        onScroll={onScroll}
      >
        {children}
      </motion.section>
    </FocusContext.Provider>
  );
}

function FocusButton({
  children,
  onPress,
  variant = "secondary",
  disabled = false,
  focusKey,
  selected,
  ariaLabel,
  navigation,
}: FocusButtonProps) {
  useRegisteredFocusNode(focusKey, !disabled, navigation);
  const { ref, focused } = useFocusable({
    focusKey,
    focusable: !disabled,
    onEnterPress: () => !disabled && onPress(),
    onArrowPress: (direction) => {
      const target = directionalFocusTarget(navigation, direction as FocusDirection);
      if (!target) return true;
      setFocus(target);
      return false;
    },
  });

  return (
    <button
      ref={ref}
      type="button"
      disabled={disabled}
      aria-label={ariaLabel}
      aria-pressed={selected || undefined}
      onClick={onPress}
      onFocus={() => syncSpatialFocus(focusKey)}
      onPointerDown={() => syncSpatialFocus(focusKey)}
      className={`focus-control ${variant} ${selected ? "is-selected" : ""} ${focused ? "is-focused" : ""}`}
    >
      {children}
    </button>
  );
}

interface FocusInputProps {
  label: string;
  value: string;
  onChange: (value: string) => void;
  placeholder: string;
  type?: "text" | "password";
  inputMode?: "text" | "numeric" | "url";
  maxLength?: number;
  focusKey: string;
  onOpenKeyboard: () => void;
  disabled?: boolean;
  invalid?: boolean;
  ariaDescribedBy?: string;
  navigation?: FocusNavigation;
}

function FocusInput({
  label,
  value,
  onChange,
  placeholder,
  type = "text",
  inputMode = "text",
  maxLength,
  focusKey,
  onOpenKeyboard,
  disabled = false,
  invalid = false,
  ariaDescribedBy,
  navigation,
}: FocusInputProps) {
  useRegisteredFocusNode(focusKey, !disabled, navigation);
  const { ref, focused } = useFocusable({
    focusKey,
    focusable: !disabled,
    onArrowPress: (direction) => {
      const target = directionalFocusTarget(navigation, direction as FocusDirection);
      if (!target) return true;
      setFocus(target);
      return false;
    },
  });

  useEffect(() => {
    const input = ref.current;
    if (!input) return;

    input.addEventListener("controller-activate", onOpenKeyboard);
    return () => input.removeEventListener("controller-activate", onOpenKeyboard);
  }, [onOpenKeyboard, ref]);

  return (
    <label className="field">
      <span>{label}</span>
      <input
        ref={ref}
        className={focused ? "is-focused" : ""}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        placeholder={placeholder}
        type={type}
        inputMode={inputMode}
        maxLength={maxLength}
        autoComplete="off"
        disabled={disabled}
        aria-invalid={invalid}
        aria-describedby={ariaDescribedBy}
        onFocus={() => syncSpatialFocus(focusKey)}
        onPointerDown={() => syncSpatialFocus(focusKey)}
      />
    </label>
  );
}

interface KeyboardSession {
  focusKey: string;
  label: string;
  value: string;
  mode: VirtualKeyboardMode;
  maxLength?: number;
  secret?: boolean;
  validate?: (value: string) => string | null;
  commit: (value: string) => void;
}

interface LibraryProvenance {
  source: "live" | "cache" | "local";
  stale: boolean;
  refreshedAtMs: number;
}

interface FavoriteMutationFailure {
  rom: RomSummary;
  desired: boolean;
  error: AppError;
}

interface DirectoryBrowserTarget {
  draftId: string;
  platformName: string;
  field: MappingPathField;
  index: number;
  returnFocusKey: string;
}

function RomCard({
  rom,
  index,
  onOpen,
  onToggleFavorite,
  favoritePending,
  focusKey: providedFocusKey,
}: {
  rom: RomSummary;
  index: number;
  onOpen: (rom: RomSummary) => void;
  onToggleFavorite: (rom: RomSummary) => void;
  favoritePending: boolean;
  focusKey?: string;
}) {
  const reduceMotion = useReducedMotion();
  const focusKey = providedFocusKey ?? `ROM-${rom.id}`;
  useRegisteredFocusNode(focusKey, true);
  const { ref, focused } = useFocusable({ focusKey });
  return (
    <motion.div
      className="rom-card-shell"
      initial={reduceMotion ? false : { opacity: 0, y: 12 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ delay: reduceMotion ? 0 : Math.min(index * 0.025, 0.3) }}
    >
      <button
        ref={ref}
        type="button"
        aria-label={`Open ${rom.title} for ${rom.platform}`}
        data-rom-id={rom.id}
        className={`rom-card ${focused ? "is-focused" : ""}`}
        onClick={() => onOpen(rom)}
        onFocus={() => syncSpatialFocus(focusKey)}
        onPointerDown={() => syncSpatialFocus(focusKey)}
      >
        <RomArtwork rom={rom} />
        <div>
          <p className="platform">
            {rom.platform}{rom.localStatus === "unavailable_on_server" ? " · unavailable" : ""}
          </p>
          <h3>{rom.title}</h3>
        </div>
      </button>
      <button
        type="button"
        className={`rom-favorite-action ${rom.user.favorite ? "is-favorite" : ""} ${rom.user.favoritePending ? "is-queued" : ""}`}
        aria-label={`${rom.user.favorite ? "Remove" : "Add"} ${rom.title} ${rom.user.favorite ? "from" : "to"} favorites${rom.user.favoritePending ? "; current change is queued" : ""}`}
        aria-pressed={rom.user.favorite}
        aria-busy={favoritePending}
        data-rom-id={rom.id}
        title={rom.user.favoritePending
          ? "Favorite change queued to sync"
          : rom.user.favorite ? "Remove from favorites" : "Add to favorites"}
        onClick={() => onToggleFavorite(rom)}
      >
        <span aria-hidden="true">{rom.user.favorite ? "★" : "☆"}</span>
        {favoritePending && <span className="favorite-saving-dot" aria-hidden="true" />}
        {!favoritePending && rom.user.favoritePending && (
          <span className="favorite-queued-badge" aria-hidden="true">Q</span>
        )}
      </button>
    </motion.div>
  );
}

export default function App() {
  const reduceMotion = useReducedMotion();
  const { ref, focusKey, focusSelf } = useFocusable({ focusKey: "APP" });
  const [serverUrl, setServerUrl] = useState("");
  const [pairingCode, setPairingCode] = useState("");
  const [manualToken, setManualToken] = useState("");
  const [manualTokenOpen, setManualTokenOpen] = useState(false);
  const [probe, setProbe] = useState<ProbeResult | null>(null);
  const [connected, setConnected] = useState(false);
  const [signedOutLocally, setSignedOutLocally] = useState(false);
  const [device, setDevice] = useState<DeviceIdentity | null>(null);
  const [deviceDisplayName, setDeviceDisplayName] = useState("");
  const [deviceResolved, setDeviceResolved] = useState(false);
  const [deviceVerified, setDeviceVerified] = useState(false);
  const [roms, setRoms] = useState<RomSummary[]>([]);
  const [libraryTab, setLibraryTab] = useState<LibraryTab>("home");
  const [libraryQuery, setLibraryQuery] = useState<LibraryQuery>({ kind: "all" });
  const [libraryMetadata, setLibraryMetadata] = useState<LibraryMetadata | null>(null);
  const [libraryFilters, setLibraryFilters] = useState<LibraryDiscoveryFilters>(DEFAULT_LIBRARY_FILTERS);
  const [libraryFiltersOpen, setLibraryFiltersOpen] = useState(false);
  const [homeShelves, setHomeShelves] = useState<Record<"recent" | "favorites" | "downloaded" | "downloads", RomSummary[]>>({
    recent: [],
    favorites: [],
    downloaded: [],
    downloads: [],
  });
  const [homeInitialized, setHomeInitialized] = useState(false);
  const [romTotal, setRomTotal] = useState<number | null>(null);
  const [hasMoreRoms, setHasMoreRoms] = useState(false);
  const [libraryInitialized, setLibraryInitialized] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [librarySourceNotice, setLibrarySourceNotice] = useState<string | null>(null);
  const [libraryProvenance, setLibraryProvenance] = useState<LibraryProvenance | null>(null);
  const [libraryError, setLibraryError] = useState<AppError | null>(null);
  const [lastError, setLastError] = useState<AppError | null>(null);
  const [httpWarningOrigin, setHttpWarningOrigin] = useState<string | null>(null);
  const [caId, setCaId] = useState<string | undefined>();
  const [pairingExpiresAt, setPairingExpiresAt] = useState<number | null>(null);
  const [pairingSecondsLeft, setPairingSecondsLeft] = useState(PAIRING_TTL_SECONDS);
  const [restoringCredential, setRestoringCredential] = useState(false);
  const [controllerName, setControllerName] = useState<string | null>(null);
  const [inputFamily, setInputFamily] = useState<InputFamily>("pc");
  const [controllerNotice, setControllerNotice] = useState<string | null>(null);
  const [controllerStatusSnapshot, setControllerStatusSnapshot] = useState<ControllerStatus | null>(null);
  const [controllerSettings, setControllerSettings] = useState<ControllerSettings>(defaultControllerSettings);
  const [controllerSettingsDraft, setControllerSettingsDraft] = useState<ControllerSettings>(defaultControllerSettings);
  const [controllerSettingsOpen, setControllerSettingsOpen] = useState(false);
  const [controllerSettingsScope, setControllerSettingsScope] = useState<"global" | "controller">("global");
  const [controllerSettingsError, setControllerSettingsError] = useState<string | null>(null);
  const [lastControllerAction, setLastControllerAction] = useState<ControllerAction["action"] | null>(null);
  const [librarySearch, setLibrarySearch] = useState("");
  const [contextRom, setContextRom] = useState<RomSummary | null>(null);
  const [gameDetails, setGameDetails] = useState<GameDetails | null>(null);
  const [gameDetailsLoading, setGameDetailsLoading] = useState(false);
  const [gameDetailsError, setGameDetailsError] = useState<string | null>(null);
  const [favoritePendingIds, setFavoritePendingIds] = useState<ReadonlySet<number>>(
    () => new Set(),
  );
  const [favoriteFailure, setFavoriteFailure] = useState<FavoriteMutationFailure | null>(null);
  const [keyboard, setKeyboard] = useState<KeyboardSession | null>(null);
  const [inputModality, setInputModality] = useState<InputModality>("navigation");
  const [closeBehavior, setCloseBehavior] = useState<CloseBehavior>("minimizeToTray");
  const [fullscreen, setFullscreen] = useState(false);
  const [stableUpdateChecksEnabled, setStableUpdateChecksEnabled] = useState(false);
  const [backgroundStartupEnabled, setBackgroundStartupEnabled] = useState(false);
  const [backgroundSetupWarning, setBackgroundSetupWarning] = useState<string | null>(null);
  const [settingsBusy, setSettingsBusy] = useState(false);
  const [deviceSettingsOpen, setDeviceSettingsOpen] = useState(false);
  const [settingsDeviceName, setSettingsDeviceName] = useState("");
  const [deviceRenameStatus, setDeviceRenameStatus] = useState<
    "idle" | "pending" | "saving" | "saved" | "error"
  >("idle");
  const [logoutDialogOpen, setLogoutDialogOpen] = useState(false);
  const [permissionDetailsOpen, setPermissionDetailsOpen] = useState(false);
  const [mappingDrafts, setMappingDrafts] = useState<PlatformMappingDraft[]>([]);
  const [mappingEvidence, setMappingEvidence] = useState<DetectionEvidence[]>([]);
  const [mappingScanned, setMappingScanned] = useState(false);
  const [mappingIssues, setMappingIssues] = useState<MappingValidationIssue[]>([]);
  const [mappingPathChecks, setMappingPathChecks] = useState<MappingPathValidation[]>([]);
  const [mappingPlatformSearch, setMappingPlatformSearch] = useState("");
  const [showAllMappingPlatforms, setShowAllMappingPlatforms] = useState(false);
  const [noPlatformsArmed, setNoPlatformsArmed] = useState(false);
  const [directoryBrowserTarget, setDirectoryBrowserTarget] = useState<DirectoryBrowserTarget | null>(null);
  const [directoryListing, setDirectoryListing] = useState<DirectoryListing | null>(null);
  const [directoryBrowserBusy, setDirectoryBrowserBusy] = useState(false);
  const [directoryBrowserError, setDirectoryBrowserError] = useState<string | null>(null);
  const [directoryBrowserNotice, setDirectoryBrowserNotice] = useState<string | null>(null);
  const [directoryCreationOpen, setDirectoryCreationOpen] = useState(false);
  const [directoryName, setDirectoryName] = useState("");
  const libraryRequestGate = useRef(createLibraryRequestGate());
  const activeLibraryQueryRef = useRef<LibraryQuery>({ kind: "all" });
  const baseLibraryQueryRef = useRef<LibraryQuery>({ kind: "all" });
  const libraryCatalogsRef = useRef(new Map<string, LibraryCatalogState>());
  const romTotalRef = useRef<number | null>(null);
  const nextRomOffsetRef = useRef(0);
  const hasMoreRomsRef = useRef(false);
  const romsRef = useRef<RomSummary[]>([]);
  const restoredConnection = useRef(false);
  const reconnectAttempted = useRef(false);
  const deviceProposalInFlight = useRef(false);
  const deviceNameDirty = useRef(false);
  const deviceRef = useRef<DeviceIdentity | null>(null);
  const settingsDeviceNameRef = useRef("");
  const deviceRenameTimer = useRef<number | null>(null);
  const controllerNoticeTimer = useRef<number | null>(null);
  const recentInputAction = useRef<RecentInputAction | null>(null);
  const primaryViewRef = useRef<PrimaryView>("startup");
  const logoutReturnFocusKey = useRef("LOGOUT");
  const caFileInput = useRef<HTMLInputElement>(null);
  const mappingDraftLoadAttempted = useRef(false);
  const initialRefreshAttempted = useRef(false);
  const contextReturnFocusKey = useRef("LIBRARY-TAB-HOME");
  const gameDetailsRequestRef = useRef(0);
  const favoriteMutationQueue = useRef(createPerRomMutationQueue());
  const favoriteMutationRevisions = useRef(new Map<number, number>());
  const favoriteValues = useRef(new Map<number, boolean>());
  const favoriteAuthoritativeValues = useRef(new Map<number, boolean>());
  const favoriteQueuedDesiredValues = useRef(new Map<number, boolean>());
  const favoritePendingRomIds = useRef(new Set<number>());
  const favoriteSessionGeneration = useRef(0);

  const closeKeyboard = useCallback((restoreFocusKey: string) => {
    setKeyboard(null);
    window.setTimeout(() => setFocus(restoreFocusKey), 0);
  }, []);

  const status = useQuery({
    queryKey: ["agent-status"],
    queryFn: () => requestAgent({ type: "getStatus" }),
    refetchInterval: 5_000,
  });
  const onboarding = useQuery({
    queryKey: ["onboarding-state"],
    queryFn: getOnboardingState,
    enabled: status.data?.type === "status",
    staleTime: 30_000,
  });
  const onboardingState = onboarding.data ?? null;

  useEffect(() => {
    if (
      onboardingState?.currentStep === "background" &&
      onboardingState.backgroundEnabled !== null
    ) {
      setBackgroundStartupEnabled(onboardingState.backgroundEnabled);
    }
  }, [onboardingState?.backgroundEnabled, onboardingState?.currentStep]);

  const run = useCallback(
    async (label: string, operation: () => Promise<AgentResponse>) => {
      setBusy(label);
      setMessage(null);
      setLastError(null);
      try {
        const response = await operation();
        if (response.type === "error") {
          setMessage(response.error.message);
          setLastError(response.error);
        }
        return response;
      } catch (error) {
        setMessage(error instanceof Error ? error.message : String(error));
        return null;
      } finally {
        setBusy(null);
      }
    },
    [],
  );

  const reconcileLoadedFavorite = useCallback((rom: RomSummary) => {
    const pendingFavorite = favoriteValues.current.get(rom.id);
    if (favoritePendingRomIds.current.has(rom.id) && pendingFavorite !== undefined) {
      return setRomFavorite(
        rom,
        pendingFavorite,
        favoriteQueuedDesiredValues.current.has(rom.id),
      );
    }
    favoriteValues.current.set(rom.id, rom.user.favorite);
    if (rom.user.favoritePending) {
      favoriteQueuedDesiredValues.current.set(rom.id, rom.user.favorite);
    } else {
      favoriteQueuedDesiredValues.current.delete(rom.id);
      favoriteAuthoritativeValues.current.set(rom.id, rom.user.favorite);
    }
    return rom;
  }, []);

  const loadRoms = useCallback(async (
    reset = true,
    automatic = false,
    queryOverride?: LibraryQuery,
  ) => {
    if (!reset && !hasMoreRomsRef.current) return;
    if (!libraryRequestGate.current.tryStart()) return;

    const query = queryOverride ?? activeLibraryQueryRef.current;
    const queryKey = libraryQueryKey(query);
    const priorCatalog = libraryCatalogsRef.current.get(queryKey);
    const previous = priorCatalog?.items ?? (queryKey === libraryQueryKey(activeLibraryQueryRef.current)
      ? romsRef.current
      : []);
    const requestedFocusKey = libraryRequestFocusTarget(
      getCurrentFocusKey(),
      previous.map((rom) => rom.id),
      automatic,
    );
    if (reset && previous.length === 0) setLibraryInitialized(false);
    const offset = reset ? 0 : priorCatalog?.nextOffset ?? nextRomOffsetRef.current;
    try {
      const response = await run(reset ? "library" : "library-more", () =>
        requestAgent({ type: "listLibrary", query, limit: LIBRARY_PAGE_SIZE, offset }),
      );
      if (response?.type === "error") {
        setLibraryError(response.error);
      } else if (response === null) {
        setLibraryError({
          code: "library_request_failed",
          message: "The library request could not be completed.",
          retryable: true,
        });
      }
      if (response?.type === "libraryPage" && libraryQueryKey(response.query) === queryKey) {
        const page = {
          ...response.page,
          items: response.page.items.map(reconcileLoadedFavorite),
        };
        setLibraryError(null);
        setLibraryProvenance({
          source: query.kind === "downloaded" || query.kind === "active_downloads"
            ? "local"
            : page.source,
          stale: page.stale,
          refreshedAtMs: page.refreshedAtMs,
        });
        const catalog = applyRomPage({
          items: previous,
          total: priorCatalog?.total ?? null,
          nextOffset: priorCatalog?.nextOffset ?? 0,
          hasMore: priorCatalog?.hasMore ?? false,
        }, page, reset);
        libraryCatalogsRef.current.set(queryKey, catalog);
        if (libraryQueryKey(activeLibraryQueryRef.current) !== queryKey) return;
        const next = catalog.items;
        const targetFocusKey = recoverLibraryFocus({
          currentFocusKey: requestedFocusKey,
          previousRomIds: previous.map((rom) => rom.id),
          visibleRomIds: next.map((rom) => rom.id),
          reset,
          hasMore: catalog.hasMore,
          hasPermissionDetails: true,
        });
        romsRef.current = next;
        setRoms(next);
        romTotalRef.current = catalog.total;
        setRomTotal(catalog.total);
        nextRomOffsetRef.current = catalog.nextOffset;
        hasMoreRomsRef.current = catalog.hasMore;
        setHasMoreRoms(catalog.hasMore);
        if (query.kind === "downloaded" || query.kind === "active_downloads") {
          setLibrarySourceNotice(null);
        } else if (page.source === "cache") {
          setLibrarySourceNotice(
            page.stale
              ? "RomM is offline. Showing library data cached more than 24 hours ago."
              : "RomM is offline. Showing the most recently cached library data.",
          );
        } else {
          setLibrarySourceNotice(null);
        }
        if (page.hasMore && page.items.length === 0) {
          setNotice("RomM returned an empty page before the end of the catalog. Loading stopped to avoid repeating the same request.");
        }
        window.requestAnimationFrame(() => {
          updateAllLayouts();
          window.requestAnimationFrame(() => setFocus(targetFocusKey));
        });
      }
    } finally {
      libraryRequestGate.current.finish();
      if (reset) setLibraryInitialized(true);
    }
  }, [reconcileLoadedFavorite, run]);

  const startInitialRefresh = useCallback(async () => {
    setLibraryInitialized(false);
    const response = await run("initial-refresh", () =>
      requestAgent({ type: "startInitialRefresh" }),
    );
    if (response?.type !== "initialRefresh") {
      window.requestAnimationFrame(() => setFocus("RETRY-INITIAL-REFRESH"));
      return;
    }

    const page = {
      ...response.result.page,
      items: response.result.page.items.map(reconcileLoadedFavorite),
    };
    const catalog = applyRomPage({
      items: [],
      total: null,
      nextOffset: 0,
      hasMore: false,
    }, page, true);
    libraryCatalogsRef.current.set("all", catalog);
    romsRef.current = catalog.items;
    setRoms(catalog.items);
    romTotalRef.current = catalog.total;
    setRomTotal(catalog.total);
    nextRomOffsetRef.current = catalog.nextOffset;
    hasMoreRomsRef.current = catalog.hasMore;
    setHasMoreRoms(hasMoreRomsRef.current);
    setLibrarySourceNotice(null);
    setLibraryProvenance({
      source: page.source,
      stale: page.stale,
      refreshedAtMs: page.refreshedAtMs,
    });
    setLibraryError(null);
    setLibraryInitialized(true);
    setNotice(
      catalog.items.length === 0
        ? "Setup is complete. This RomM account currently has no games."
        : `Setup is complete. ${catalog.items.length} ${catalog.items.length === 1 ? "game is" : "games are"} ready to browse.`,
    );
    await onboarding.refetch();
    window.requestAnimationFrame(() => {
      updateAllLayouts();
      window.requestAnimationFrame(() => setFocus(catalog.items[0] ? `ROM-${catalog.items[0].id}` : "REFRESH"));
    });
  }, [onboarding, reconcileLoadedFavorite, run]);

  const handleLibraryScroll = useCallback((event: UIEvent<HTMLElement>) => {
    const view = event.currentTarget;
    if (shouldAutoLoadLibrary(view, hasMoreRomsRef.current, libraryRequestGate.current.isActive())) {
      void loadRoms(false, true);
    }
  }, [loadRoms]);

  const showCatalog = useCallback((query: LibraryQuery, catalog?: LibraryCatalogState) => {
    activeLibraryQueryRef.current = query;
    setLibraryQuery(query);
    const next = catalog ?? { items: [], total: null, nextOffset: 0, hasMore: false };
    romsRef.current = next.items;
    setRoms(next.items);
    romTotalRef.current = next.total;
    setRomTotal(next.total);
    nextRomOffsetRef.current = next.nextOffset;
    hasMoreRomsRef.current = next.hasMore;
    setHasMoreRoms(next.hasMore);
    setLibraryInitialized(Boolean(catalog));
    setLibraryError(null);
    setLibraryProvenance(catalog?.refreshedAtMs ? {
      source: query.kind === "downloaded" || query.kind === "active_downloads"
        ? "local"
        : catalog.source ?? "cache",
      stale: catalog.stale ?? false,
      refreshedAtMs: catalog.refreshedAtMs,
    } : null);
  }, []);

  const loadLibraryHome = useCallback(async () => {
    setBusy("library-home");
    setMessage(null);
    try {
      const responses = await Promise.all([
        requestAgent({ type: "getLibraryMetadata" }),
        requestAgent({ type: "listLibrary", query: { kind: "recent" }, limit: HOME_SHELF_SIZE, offset: 0 }),
        requestAgent({ type: "listLibrary", query: { kind: "favorites" }, limit: HOME_SHELF_SIZE, offset: 0 }),
        requestAgent({ type: "listLibrary", query: { kind: "downloaded" }, limit: HOME_SHELF_SIZE, offset: 0 }),
        requestAgent({ type: "listLibrary", query: { kind: "active_downloads" }, limit: HOME_SHELF_SIZE, offset: 0 }),
      ]);
      const [metadataResponse, recentResponse, favoritesResponse, downloadedResponse, downloadsResponse] = responses;
      if (metadataResponse.type === "libraryMetadata") setLibraryMetadata(metadataResponse.metadata);
      const pageItems = (response: AgentResponse) => response.type === "libraryPage"
        ? response.page.items.map(reconcileLoadedFavorite)
        : [];
      setHomeShelves({
        recent: pageItems(recentResponse),
        favorites: pageItems(favoritesResponse),
        downloaded: pageItems(downloadedResponse),
        downloads: pageItems(downloadsResponse),
      });
      const firstError = responses.find((response) => response.type === "error");
      if (firstError?.type === "error") {
        setMessage(firstError.error.message);
        setLibraryError(firstError.error);
      } else {
        setLibraryError(null);
      }
      const cached = responses.some((response) =>
        response.type === "libraryPage" && response.page.source === "cache" && !response.query.kind.includes("download"),
      ) || (metadataResponse.type === "libraryMetadata" && metadataResponse.metadata.source === "cache");
      setLibrarySourceNotice(cached
        ? "RomM is offline. Home shelves are using the most recently cached data."
        : null);
      const remoteSnapshots = responses.flatMap((response) => {
        if (response.type === "libraryPage" && response.query.kind !== "downloaded" && response.query.kind !== "active_downloads") {
          return [response.page];
        }
        return [];
      });
      const refreshedAtMs = Math.max(
        metadataResponse.type === "libraryMetadata" ? metadataResponse.metadata.refreshedAtMs : 0,
        ...remoteSnapshots.map((page) => page.refreshedAtMs),
      );
      if (refreshedAtMs > 0) {
        setLibraryProvenance({
          source: cached ? "cache" : "live",
          stale: (metadataResponse.type === "libraryMetadata" && metadataResponse.metadata.stale) ||
            remoteSnapshots.some((page) => page.stale),
          refreshedAtMs,
        });
      }
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setMessage(message);
      setLibraryError({ code: "library_request_failed", message, retryable: true });
    } finally {
      setHomeInitialized(true);
      setBusy(null);
    }
  }, [reconcileLoadedFavorite]);

  const applyFavoriteEverywhere = useCallback((
    sourceRom: RomSummary,
    favorite: boolean,
    favoriteQueued: boolean,
  ) => {
    const updatedRom = setRomFavorite(sourceRom, favorite, favoriteQueued);
    favoriteValues.current.set(sourceRom.id, favorite);

    for (const [queryKey, catalog] of libraryCatalogsRef.current) {
      const items = setFavoriteInRoms(
        catalog.items,
        sourceRom.id,
        favorite,
        favoriteQueued,
      );
      if (items !== catalog.items) {
        libraryCatalogsRef.current.set(queryKey, { ...catalog, items });
      }
    }

    setRoms((current) => {
      const next = setFavoriteInRoms(current, sourceRom.id, favorite, favoriteQueued);
      romsRef.current = next;
      return next;
    });
    setHomeShelves((current) =>
      setFavoriteInHomeShelves(current, updatedRom, favorite, HOME_SHELF_SIZE, favoriteQueued),
    );
    setContextRom((current) =>
      current?.id === sourceRom.id
        ? setRomFavorite(current, favorite, favoriteQueued)
        : current,
    );
    setGameDetails((current) =>
      current?.rom.id === sourceRom.id
        ? { ...current, rom: setRomFavorite(current.rom, favorite, favoriteQueued) }
        : current,
    );
  }, []);

  const applyFavoriteReconciliation = useCallback((result?: FavoriteReconciliationResult) => {
    if (!result) return;
    const settledIds = new Set<number>();
    for (const outcome of result.outcomes) {
      favoriteMutationRevisions.current.set(
        outcome.romId,
        (favoriteMutationRevisions.current.get(outcome.romId) ?? 0) + 1,
      );
      favoritePendingRomIds.current.delete(outcome.romId);
      settledIds.add(outcome.romId);
      if (outcome.status === "pending") {
        favoriteQueuedDesiredValues.current.set(outcome.romId, outcome.favorite);
      } else {
        favoriteQueuedDesiredValues.current.delete(outcome.romId);
        favoriteAuthoritativeValues.current.set(outcome.romId, outcome.favorite);
      }
      const sourceRom = romsRef.current.find((rom) => rom.id === outcome.romId) ??
        [...libraryCatalogsRef.current.values()]
          .flatMap((catalog) => catalog.items)
          .find((rom) => rom.id === outcome.romId) ??
        (contextRom?.id === outcome.romId ? contextRom : undefined) ??
        (gameDetails?.rom.id === outcome.romId ? gameDetails.rom : undefined);
      if (sourceRom) {
        applyFavoriteEverywhere(
          sourceRom,
          outcome.favorite,
          outcome.status === "pending",
        );
      }
    }
    if (settledIds.size > 0) {
      setFavoritePendingIds((current) => {
        const next = new Set(current);
        for (const romId of settledIds) next.delete(romId);
        return next;
      });
    }
  }, [applyFavoriteEverywhere, contextRom, gameDetails]);

  const setFavoriteOptimistically = useCallback((rom: RomSummary, desired: boolean) => {
    const romId = rom.id;
    const sessionGeneration = favoriteSessionGeneration.current;
    const previousFavorite = favoriteValues.current.get(romId) ?? rom.user.favorite;
    if (!favoriteAuthoritativeValues.current.has(romId)) {
      favoriteAuthoritativeValues.current.set(romId, previousFavorite);
    }
    const revision = (favoriteMutationRevisions.current.get(romId) ?? 0) + 1;
    favoriteMutationRevisions.current.set(romId, revision);
    favoritePendingRomIds.current.add(romId);
    setFavoriteFailure(null);
    applyFavoriteEverywhere(
      rom,
      desired,
      favoriteQueuedDesiredValues.current.has(romId),
    );
    setFavoritePendingIds((current) => new Set(current).add(romId));

    void favoriteMutationQueue.current.enqueue(romId, async () => {
      let authoritativeFavorite: boolean | null = null;
      let queuedFavorite: boolean | null = null;
      let failure: AppError | null = null;
      try {
        const response = await requestAgent({ type: "setFavorite", romId, desired });
        if (response.type === "favoriteUpdated" && response.result.romId === romId) {
          authoritativeFavorite = response.result.favorite;
        } else if (response.type === "favoriteQueued" && response.mutation.romId === romId) {
          queuedFavorite = response.mutation.desired;
        } else if (response.type === "error") {
          failure = response.error;
        } else {
          failure = {
            code: "favorite_update_failed",
            message: "RomM returned an unexpected response while updating this favorite.",
            retryable: true,
          };
        }
      } catch (error) {
        failure = {
          code: "favorite_update_failed",
          message: error instanceof Error ? error.message : String(error),
          retryable: true,
        };
      }

      if (favoriteSessionGeneration.current !== sessionGeneration) return;

      const isLatest = favoriteMutationRevisions.current.get(romId) === revision;
      if (authoritativeFavorite !== null) {
        favoriteAuthoritativeValues.current.set(romId, authoritativeFavorite);
        favoriteQueuedDesiredValues.current.delete(romId);
      } else if (queuedFavorite !== null) {
        favoriteQueuedDesiredValues.current.set(romId, queuedFavorite);
      }
      if (isLatest && failure) {
        const queuedDesired = favoriteQueuedDesiredValues.current.get(romId);
        const rollbackFavorite = queuedDesired ??
          favoriteAuthoritativeValues.current.get(romId) ?? previousFavorite;
        const rollbackQueued = queuedDesired !== undefined;
        applyFavoriteEverywhere(rom, rollbackFavorite, rollbackQueued);
        setFavoriteFailure({
          rom: setRomFavorite(rom, rollbackFavorite, rollbackQueued),
          desired,
          error: failure,
        });
      } else if (isLatest && authoritativeFavorite !== null) {
        applyFavoriteEverywhere(rom, authoritativeFavorite, false);
      } else if (isLatest && queuedFavorite !== null) {
        applyFavoriteEverywhere(rom, queuedFavorite, true);
      }

      if (isLatest) {
        favoritePendingRomIds.current.delete(romId);
        setFavoritePendingIds((current) => {
          const next = new Set(current);
          next.delete(romId);
          return next;
        });
      }
    });
  }, [applyFavoriteEverywhere]);

  const toggleFavoriteOptimistically = useCallback((rom: RomSummary) => {
    const current = favoriteValues.current.get(rom.id) ?? rom.user.favorite;
    setFavoriteOptimistically(rom, !current);
  }, [setFavoriteOptimistically]);

  const selectLibraryTab = useCallback((tab: LibraryTab) => {
    if (busy !== null) return;
    setLibraryTab(tab);
    setLibrarySearch("");
    setLibrarySourceNotice(null);
    const query = queryForLibraryTab(tab);
    if (query) {
      baseLibraryQueryRef.current = query;
      const discoveryQuery = buildDiscoveryQuery(query, "", libraryFilters);
      const cached = libraryCatalogsRef.current.get(libraryQueryKey(discoveryQuery));
      showCatalog(discoveryQuery, cached);
      if (!cached || discoveryQuery.kind === "active_downloads" || discoveryQuery.kind === "downloaded") {
        void loadRoms(true, false, discoveryQuery);
      }
    } else if (tab === "home" && !homeInitialized) {
      void loadLibraryHome();
    } else if (tab === "platforms" || tab === "collections") {
      setLibraryQuery({ kind: "all" });
      if (!libraryMetadata) void loadLibraryHome();
    }
    const target = libraryTabFocusKey(tab);
    window.requestAnimationFrame(() => {
      updateAllLayouts();
      window.requestAnimationFrame(() => setFocus(target));
    });
  }, [busy, homeInitialized, libraryFilters, libraryMetadata, loadLibraryHome, loadRoms, showCatalog]);

  const openLibraryScope = useCallback((query: LibraryQuery) => {
    baseLibraryQueryRef.current = query;
    const discoveryQuery = buildDiscoveryQuery(query, librarySearch, libraryFilters);
    const cached = libraryCatalogsRef.current.get(libraryQueryKey(discoveryQuery));
    showCatalog(discoveryQuery, cached);
    void loadRoms(true, false, discoveryQuery);
  }, [libraryFilters, librarySearch, loadRoms, showCatalog]);

  const acceptDeviceIdentity = useCallback((nextDevice: DeviceIdentity, resolved = true) => {
    deviceRef.current = nextDevice;
    setDevice(nextDevice);
    setDeviceResolved(resolved);
    if (!deviceNameDirty.current || nextDevice.rommDeviceId !== null) {
      deviceNameDirty.current = false;
      setDeviceDisplayName(nextDevice.displayName);
    }
  }, []);

  useEffect(() => {
    deviceRef.current = device;
  }, [device]);

  useEffect(() => {
    settingsDeviceNameRef.current = settingsDeviceName;
  }, [settingsDeviceName]);

  useEffect(() => () => {
    if (deviceRenameTimer.current !== null) {
      window.clearTimeout(deviceRenameTimer.current);
    }
    if (controllerNoticeTimer.current !== null) {
      window.clearTimeout(controllerNoticeTimer.current);
    }
  }, []);

  const verifyDeviceIdentity = useCallback(async (candidate: DeviceIdentity) => {
    setDeviceVerified(false);
    acceptDeviceIdentity(candidate, false);
    const response = await run("device-verification", () =>
      requestAgent({ type: "verifyDevice" }),
    );
    setDeviceResolved(true);

    if (response?.type === "deviceVerification") {
      acceptDeviceIdentity(response.device);
      if (response.outcome === "verified") {
        setDeviceVerified(true);
        const refreshedOnboarding = await onboarding.refetch();
        if (
          refreshedOnboarding.data?.completedSteps.includes("first_refresh") &&
          !restoredConnection.current
        ) {
          restoredConnection.current = true;
          await loadRoms(true);
        }
      } else {
        window.setTimeout(() => setFocus("DEVICE-RECOVERY-ACTION"), 0);
      }
    } else {
      setDevice(candidate);
      window.setTimeout(() => setFocus("DEVICE-RECOVERY-ACTION"), 0);
    }
  }, [acceptDeviceIdentity, loadRoms, onboarding, run]);

  const prepareDeviceRegistration = useCallback(async () => {
    if (deviceProposalInFlight.current) return;
    deviceProposalInFlight.current = true;
    setDeviceResolved(false);
    const response = await run("device-proposal", () => requestAgent({ type: "proposeDevice" }));
    deviceProposalInFlight.current = false;
    setDeviceResolved(true);

    if (response?.type !== "deviceProposed") return;
    setDeviceVerified(false);
    if (
      response.device.rommDeviceId &&
      response.device.registrationState !== "missing"
    ) {
      await verifyDeviceIdentity(response.device);
    } else {
      acceptDeviceIdentity(response.device);
      window.setTimeout(() => setFocus("DEVICE-NAME"), 0);
    }
  }, [acceptDeviceIdentity, run, verifyDeviceIdentity]);

  useEffect(() => {
    if (status.data?.type !== "status") return;
    const agentStatus = status.data.status;
    if (agentStatus.serverUrl) {
      setServerUrl(agentStatus.serverUrl);
    }
    setCaId(agentStatus.caId);
    if (signedOutLocally && agentStatus.connection === "connected") return;
    if (agentStatus.connection === "connected") {
      setConnected(true);
      if (!deviceResolved && !deviceProposalInFlight.current) {
        void prepareDeviceRegistration();
      }
    } else if (agentStatus.connection === "pairing" && agentStatus.serverUrl) {
      setProbe({
        normalizedUrl: agentStatus.serverUrl,
        serverVersion: agentStatus.serverVersion ?? "unknown",
        compatible: true,
      });
    } else if (
      agentStatus.connection === "offline" &&
      agentStatus.credentialConfigured &&
      !reconnectAttempted.current
    ) {
      reconnectAttempted.current = true;
      setRestoringCredential(true);
      void reconnect(true);
    }
  }, [deviceResolved, prepareDeviceRegistration, signedOutLocally, status.data]);

  useEffect(() => {
    if (!probe?.compatible) {
      setPairingExpiresAt(null);
      return;
    }
    const expiresAt = Date.now() + PAIRING_TTL_SECONDS * 1_000;
    setPairingExpiresAt(expiresAt);
    setPairingSecondsLeft(PAIRING_TTL_SECONDS);
    const timer = window.setInterval(() => {
      const seconds = Math.max(0, Math.ceil((expiresAt - Date.now()) / 1_000));
      setPairingSecondsLeft(seconds);
      if (seconds === 0) {
        setPairingCode("");
        window.clearInterval(timer);
      }
    }, 1_000);
    return () => window.clearInterval(timer);
  }, [probe?.normalizedUrl, probe?.compatible]);

  useEffect(() => {
    focusSelf();
  }, [focusSelf]);

  useEffect(() => {
    void getDesktopSettings()
      .then((settings) => {
        setCloseBehavior(settings.closeBehavior);
        setFullscreen(settings.fullscreen);
        setStableUpdateChecksEnabled(settings.stableUpdateChecksEnabled);
        setControllerSettings(settings.controller);
        setControllerSettingsDraft(settings.controller);
      })
      .catch((error) => setMessage(error instanceof Error ? error.message : String(error)));
  }, []);

  useEffect(() => {
    function applyControllerStatus(controllerStatus: ControllerStatus) {
      setControllerStatusSnapshot(controllerStatus);
      const active = controllerStatus.controllers.find(
        (controller) => controller.id === controllerStatus.activeControllerId,
      );
      setControllerName(active?.name ?? null);
      if (active) setInputFamily(active.mappingFamily);
      let nextNotice: string | null = null;
      if (controllerStatus.change === "connected" && controllerStatus.changedController) {
        nextNotice = `${controllerStatus.changedController.name} connected.`;
      } else if (
        controllerStatus.change === "disconnected" &&
        controllerStatus.changedController
      ) {
        nextNotice = active
          ? `${controllerStatus.changedController.name} disconnected. ${active.name} is now active.`
          : `${controllerStatus.changedController.name} disconnected. Keyboard and mouse remain available.`;
      }
      if (nextNotice) {
        setControllerNotice(nextNotice);
        if (controllerNoticeTimer.current !== null) {
          window.clearTimeout(controllerNoticeTimer.current);
        }
        controllerNoticeTimer.current = window.setTimeout(() => {
          setControllerNotice(null);
          controllerNoticeTimer.current = null;
        }, 5_000);
      }
    }

    void getControllerStatus().then((controllerStatus) => {
      if (controllerStatus) applyControllerStatus(controllerStatus);
    }).catch(() => undefined);

    const actionUnlistenPromise = listen<ControllerAction>("controller-action", ({ payload }) => {
      setInputModality("navigation");
      setControllerName(payload.controllerName);
      setInputFamily(payload.mappingFamily);
      setLastControllerAction(payload.action);
      if (!shouldHandleControllerAction(payload)) return;
      const current: RecentInputAction = {
        action: payload.action,
        source: "controller",
        timestampMs: payload.timestampMs,
      };
      if (isCrossSourceDuplicate(recentInputAction.current, current)) return;
      recentInputAction.current = current;
      if (DIRECTION_ACTIONS.has(payload.action)) {
        const direction = payload.action as FocusDirection;
        const explicitTarget = registeredDirectionalFocusTarget(
          getCurrentFocusKey(),
          direction,
          registeredFocusNodes,
        );
        if (explicitTarget) {
          setFocus(explicitTarget);
        } else {
          navigateByDirection(direction, {});
        }
      } else if (payload.action === "confirm") {
        const activeElement = document.activeElement as HTMLElement | null;
        if (!keyboard && activeElement instanceof HTMLInputElement) {
          activeElement.dispatchEvent(new CustomEvent("controller-activate"));
        } else {
          activeElement?.click();
        }
      } else if (payload.action === "back" && keyboard) {
        window.dispatchEvent(new CustomEvent(VIRTUAL_KEYBOARD_COMMAND_EVENT, {
          detail: "backspace",
        }));
      } else if (payload.action === "back" && directoryBrowserTarget) {
        if (directoryCreationOpen) {
          setDirectoryCreationOpen(false);
          setDirectoryName("");
          setDirectoryBrowserError(null);
          window.requestAnimationFrame(() => setFocus("DIRECTORY-NEW"));
        } else if (!directoryBrowserBusy) {
          closeDirectoryBrowser();
        }
      } else if (payload.action === "back" && logoutDialogOpen) {
        closeLogoutDialog();
      } else if (payload.action === "back" && contextRom) {
        closeGameContext();
      } else if (payload.action === "back" && controllerSettingsOpen) {
        closeControllerSettings();
      } else if (payload.action === "back" && deviceSettingsOpen) {
        closeDeviceSettings();
      } else if (payload.action === "search") {
        focusLibrarySearch();
      } else if (payload.action === "context") {
        openFocusedGameContext();
      } else if (
        controllerSettingsOpen &&
        (payload.action === "previousTab" || payload.action === "nextTab")
      ) {
        selectControllerSettingsScope(
          payload.action === "previousTab" ? "global" : "controller",
        );
      } else if (
        primaryViewRef.current === "library" &&
        (payload.action === "previousTab" || payload.action === "nextTab")
      ) {
        selectLibraryTab(nextLibraryTab(
          libraryTab,
          payload.action === "previousTab" ? -1 : 1,
        ));
      } else if (payload.action === "back") {
        if (manualTokenOpen) {
          setManualTokenOpen(false);
          window.requestAnimationFrame(() => setFocus("TOKEN-DISCLOSURE"));
        } else if (primaryViewRef.current === "connection" && httpWarningOrigin) {
          setHttpWarningOrigin(null);
          window.requestAnimationFrame(() => setFocus("TEST-SERVER"));
        } else if (primaryViewRef.current === "connection" && probe?.compatible) {
          setPairingCode("");
          setPairingExpiresAt(null);
          setProbe(null);
          window.requestAnimationFrame(() => setFocus("SERVER-URL"));
        } else if (primaryViewRef.current === "connection") {
          setFocus("SERVER-URL");
        } else if (primaryViewRef.current === "device") {
          setFocus("DEVICE-SIGN-OUT");
        } else if (primaryViewRef.current === "mapping") {
          setFocus(onboardingState?.currentStep === "mappings" ? "SAVE-MAPPINGS" : "SCAN-MAPPINGS");
        } else if (primaryViewRef.current === "preferences") {
          setFocus("SAVE-ONBOARDING-PREFERENCES");
        } else if (primaryViewRef.current === "library") {
          setFocus("REFRESH");
        }
      }
    });
    const statusUnlistenPromise = listen<ControllerStatus>("controller-status", ({ payload }) => {
      applyControllerStatus(payload);
    });
    return () => {
      void actionUnlistenPromise.then((unlisten) => unlisten());
      void statusUnlistenPromise.then((unlisten) => unlisten());
    };
  }, [
    closeKeyboard,
    contextRom,
    controllerSettingsOpen,
    directoryBrowserBusy,
    directoryBrowserTarget,
    directoryCreationOpen,
    deviceSettingsOpen,
    deviceVerified,
    httpWarningOrigin,
    keyboard,
    libraryTab,
    logoutDialogOpen,
    manualTokenOpen,
    onboardingState?.currentStep,
    probe?.compatible,
    selectLibraryTab,
  ]);

  useEffect(() => {
    function coalesceSteamInput(event: KeyboardEvent) {
      const nextModality = modalityForKeyboardEvent(event.key);
      if (nextModality) setInputModality(nextModality);
      const action = keyboardControllerAction(event.key);
      if (!action) {
        setControllerName(null);
        setInputFamily("pc");
        return;
      }
      if (event.repeat) return;
      const current: RecentInputAction = {
        action,
        source: "keyboard",
        timestampMs: Date.now(),
      };
      if (isCrossSourceDuplicate(recentInputAction.current, current)) {
        event.preventDefault();
        event.stopImmediatePropagation();
        return;
      }
      recentInputAction.current = current;
      setControllerName(null);
      setInputFamily("pc");
      if (event.target instanceof HTMLInputElement) return;
      if (action === "search") {
        event.preventDefault();
        focusLibrarySearch();
      } else if (action === "context") {
        event.preventDefault();
        openFocusedGameContext();
      } else if (
        controllerSettingsOpen &&
        (action === "previousTab" || action === "nextTab")
      ) {
        event.preventDefault();
        selectControllerSettingsScope(action === "previousTab" ? "global" : "controller");
      } else if (
        primaryViewRef.current === "library" &&
        (action === "previousTab" || action === "nextTab")
      ) {
        event.preventDefault();
        selectLibraryTab(nextLibraryTab(libraryTab, action === "previousTab" ? -1 : 1));
      }
    }
    window.addEventListener("keydown", coalesceSteamInput, true);
    const usePointerInput = () => {
      setInputModality("pointer");
      setControllerName(null);
      setInputFamily("pc");
    };
    window.addEventListener("pointerdown", usePointerInput, true);
    return () => {
      window.removeEventListener("keydown", coalesceSteamInput, true);
      window.removeEventListener("pointerdown", usePointerInput, true);
    };
  }, [controllerSettingsOpen, libraryTab, selectLibraryTab]);

  const agentLabel = useMemo(() => {
    if (status.isPending) return "Connecting to agent…";
    if (status.isError) return "Agent unavailable";
    if (status.data?.type === "status") return `Agent ${status.data.status.version}`;
    return "Agent ready";
  }, [status.data, status.isError, status.isPending]);

  const restoredAgentStatus = status.data?.type === "status" ? status.data.status : null;
  const activeController = controllerStatusSnapshot?.controllers.find(
    (controller) => controller.id === controllerStatusSnapshot.activeControllerId,
  );
  const activeControllerBindings = effectiveControllerBindings(
    controllerSettings,
    activeController?.guid,
  );
  const controllerSettingsFamily: InputFamily = activeController?.mappingFamily ?? inputFamily;
  const draftBindings: ControllerBindings = controllerSettingsScope === "controller" && activeController
    ? controllerSettingsDraft.controllerBindings[activeController.guid] ?? controllerSettingsDraft.globalBindings
    : controllerSettingsDraft.globalBindings;
  const filteredRoms = roms;
  const visibleMappingDrafts = useMemo(() => {
    const query = mappingPlatformSearch.trim().toLocaleLowerCase();
    if (query) {
      return mappingDrafts.filter((draft) =>
        `${draft.platformName} ${draft.platformSlug}`.toLocaleLowerCase().includes(query),
      );
    }
    if (showAllMappingPlatforms) return mappingDrafts;
    const enabled = mappingDrafts.filter((draft) => draft.enabled);
    return enabled.length > 0 ? enabled : mappingDrafts.slice(0, 24);
  }, [mappingDrafts, mappingPlatformSearch, showAllMappingPlatforms]);
  const isAuthenticated = !signedOutLocally &&
    (connected || restoredAgentStatus?.connection === "connected");
  const isResolvingDevice = isAuthenticated && !deviceResolved;
  const deviceNameError = validateDeviceDisplayName(deviceDisplayName);
  const deviceNeedsRegistration = device?.registrationState === "unregistered";
  const deviceIsMissing = device?.registrationState === "missing";
  const devicePermissionBlocked = device?.registrationState === "permission_error";
  const restorableCredential = restoredAgentStatus?.connection === "offline" &&
    restoredAgentStatus.credentialConfigured &&
    !reconnectAttempted.current;
  const resolvedPrimaryView = resolvePrimaryView({
    signedOutLocally,
    statusPending: status.isPending,
    restoringCredential,
    restorableCredential,
    connected,
    agentConnected: !signedOutLocally && restoredAgentStatus?.connection === "connected",
    deviceResolved,
    deviceReady: deviceVerified,
  });
  const mappingSetupRequired = onboardingState?.currentStep === "detection" ||
    onboardingState?.currentStep === "mappings";
  const preferenceSetupRequired = onboardingState?.currentStep === "background";
  const initialRefreshRequired = requiresInitialRefresh(onboardingState);
  const primaryView: PrimaryView = resolvedPrimaryView === "library" && onboarding.isPending
    ? "startup"
    : resolvedPrimaryView === "library" && mappingSetupRequired
      ? "mapping"
      : resolvedPrimaryView === "library" && preferenceSetupRequired
        ? "preferences"
        : resolvedPrimaryView === "library" && initialRefreshRequired
          ? "refresh"
          : resolvedPrimaryView;
  const catalogViewActive = queryForLibraryTab(libraryTab) !== null ||
    (libraryTab === "platforms" && libraryQuery.kind === "platform") ||
    (libraryTab === "collections" &&
      (libraryQuery.kind === "collection" || libraryQuery.kind === "smart_collection"));
  primaryViewRef.current = primaryView;
  useEffect(() => {
    if (primaryView !== "mapping" || onboardingState?.currentStep !== "mappings") return;
    if (mappingDraftLoadAttempted.current) return;
    mappingDraftLoadAttempted.current = true;
    void run("mapping-drafts", () => requestAgent({ type: "getMappingDrafts" })).then((response) => {
      if (response?.type === "mappingDrafts") {
        setMappingDrafts(response.drafts);
        setMappingScanned(true);
      }
    });
  }, [onboardingState?.currentStep, primaryView, run]);
  useEffect(() => {
    setMappingIssues([]);
    setMappingPathChecks([]);
  }, [mappingDrafts]);
  useEffect(() => {
    if (primaryView !== "mapping" || onboardingState?.currentStep !== "mappings") return;
    if (!mappingDrafts.some((draft) => draft.enabled)) return;
    let active = true;
    const recheck = () => {
      const drafts = normalizeMappingPaths(mappingDrafts);
      void requestAgent({ type: "recheckMappings", drafts, noPlatforms: false })
        .then((response) => {
          if (!active || response.type !== "mappingValidation") return;
          setMappingIssues(response.result.issues);
          setMappingPathChecks(response.result.paths);
        })
        .catch(() => undefined);
    };
    const timer = window.setInterval(recheck, 15_000);
    window.addEventListener("online", recheck);
    window.addEventListener("focus", recheck);
    return () => {
      active = false;
      window.clearInterval(timer);
      window.removeEventListener("online", recheck);
      window.removeEventListener("focus", recheck);
    };
  }, [mappingDrafts, onboardingState?.currentStep, primaryView]);
  useEffect(() => {
    if (primaryView !== "refresh") {
      initialRefreshAttempted.current = false;
      return;
    }
    if (initialRefreshAttempted.current) return;
    initialRefreshAttempted.current = true;
    void startInitialRefresh();
  }, [primaryView, startInitialRefresh]);
  useEffect(() => {
    if (primaryView === "library" && libraryTab === "home" && !homeInitialized && busy === null) {
      void loadLibraryHome();
    }
  }, [busy, homeInitialized, libraryTab, loadLibraryHome, primaryView]);
  useEffect(() => {
    if (primaryView !== "library" || !catalogViewActive || busy !== null) return;
    const delay = librarySourceNotice?.startsWith("RomM is offline") ||
      (status.data?.type === "status" && status.data.status.connection === "offline")
      ? 0
      : 250;
    const timer = window.setTimeout(() => {
      const query = buildDiscoveryQuery(
        baseLibraryQueryRef.current,
        librarySearch,
        libraryFilters,
      );
      if (libraryQueryKey(query) === libraryQueryKey(activeLibraryQueryRef.current)) return;
      const cached = libraryCatalogsRef.current.get(libraryQueryKey(query));
      showCatalog(query, cached);
      void loadRoms(true, false, query);
    }, delay);
    return () => window.clearTimeout(timer);
  }, [busy, catalogViewActive, libraryFilters, librarySearch, librarySourceNotice, loadRoms, primaryView, showCatalog, status.data]);
  useEffect(() => {
    if (
      primaryView !== "library" || !catalogViewActive || !libraryInitialized ||
      !hasMoreRoms || busy !== null
    ) {
      return;
    }
    const frame = window.requestAnimationFrame(() => {
      const view = document.querySelector<HTMLElement>(".library-view");
      if (
        view &&
        shouldAutoLoadLibrary(
          view,
          hasMoreRomsRef.current,
          libraryRequestGate.current.isActive(),
        )
      ) {
        void loadRoms(false, true);
      }
    });
    return () => window.cancelAnimationFrame(frame);
  }, [busy, catalogViewActive, hasMoreRoms, libraryInitialized, loadRoms, primaryView, roms.length]);
  const reconnectVisible = Boolean(
    restoredAgentStatus?.credentialConfigured &&
      restoredAgentStatus.connection !== "connected",
  );
  const connectionEntryFocusKey = probe?.compatible
    ? "PAIRING-CODE"
    : reconnectVisible
      ? "RECONNECT"
      : "SERVER-URL";
  const deviceEntryFocusKey = !device
    ? "RETRY-DEVICE-PROPOSAL"
    : device.registrationState === "unregistered"
      ? "DEVICE-NAME"
      : "DEVICE-RECOVERY-ACTION";
  const resolvedScreenEntryFocusKey = primaryFocusTarget(primaryView, {
    connectionTarget: connectionEntryFocusKey,
    deviceTarget: deviceEntryFocusKey,
    mappingTarget: onboardingState?.currentStep === "mappings" ? "MAPPING-SEARCH" : "SCAN-MAPPINGS",
    preferencesTarget: "PREF-BACKGROUND",
    refreshTarget: busy === "initial-refresh" ? "CLOSE-TO-TRAY" : "RETRY-INITIAL-REFRESH",
    firstRomId: roms[0]?.id,
  });
  const screenEntryFocusKey = primaryView === "library"
    ? libraryTabFocusKey(libraryTab)
    : resolvedScreenEntryFocusKey;
  const libraryBottomFocusKey = libraryBottomFocusTarget(
    filteredRoms.map((rom) => rom.id),
    hasMoreRoms,
    busy !== null,
  );
  const connectionAfterTestFocusKey = httpWarningOrigin
    ? "CONFIRM-HTTP"
    : lastError?.code === "tls_error"
      ? "IMPORT-CA"
      : probe?.compatible
        ? "PAIRING-CODE"
        : "CLOSE-TO-TRAY";
  const devicePrimaryActionFocusKey = !device
    ? "RETRY-DEVICE-PROPOSAL"
    : deviceNeedsRegistration || deviceIsMissing
      ? deviceIsMissing
        ? "DEVICE-RECOVERY-ACTION"
        : "REGISTER-DEVICE"
      : "DEVICE-RECOVERY-ACTION";
  const libraryFirstContentFocusKey = libraryTabFocusKey(libraryTab);
  const hasLibraryConstraints = librarySearch.trim().length > 0 ||
    libraryFilters.platformId !== null ||
    libraryFilters.collectionId !== null ||
    libraryFilters.favoriteOnly ||
    libraryFilters.downloadedOnly;
  const footerReturnFocusKey = primaryView === "library"
    ? catalogViewActive
      ? libraryError
        ? "LIBRARY-RETRY"
        : filteredRoms.length === 0 && hasLibraryConstraints
          ? "LIBRARY-EMPTY-CLEAR"
          : libraryBottomFocusKey
      : libraryFirstContentFocusKey
    : primaryView === "device"
      ? "DEVICE-SIGN-OUT"
      : primaryView === "mapping"
        ? onboardingState?.currentStep === "mappings" ? "SAVE-MAPPINGS" : "SCAN-MAPPINGS"
      : primaryView === "preferences"
        ? "SAVE-ONBOARDING-PREFERENCES"
      : primaryView === "refresh"
        ? busy === "initial-refresh" ? "CLOSE-TO-TRAY" : "RETRY-INITIAL-REFRESH"
      : primaryView === "connection"
        ? probe?.compatible
          ? manualTokenOpen
            ? "USE-TOKEN"
            : "TOKEN-DISCLOSURE"
          : "TEST-SERVER"
        : "TOGGLE-FULLSCREEN";
  const activeServerUrl = serverUrl || restoredAgentStatus?.serverUrl || "RomM";
  const selectedPlatform = libraryQuery.kind === "platform"
    ? libraryMetadata?.platforms.find((platform) => platform.id === libraryQuery.id)
    : null;
  const selectedCollection = libraryQuery.kind === "collection" || libraryQuery.kind === "smart_collection"
    ? libraryMetadata?.collections.find((collection) => collection.id === libraryQuery.id)
    : null;
  const filterPlatform = libraryMetadata?.platforms.find(
    (platform) => platform.id === libraryFilters.platformId,
  );
  const filterCollection = libraryMetadata?.collections.find(
    (collection) => collection.id === libraryFilters.collectionId && collection.kind === libraryFilters.collectionKind,
  );
  const contextPlatformId = gameDetails?.rom.platformId ?? contextRom?.platformId;
  const contextMapping = contextPlatformId
    ? mappingDrafts.find((draft) => draft.enabled && draft.platformId === contextPlatformId)
    : undefined;
  const libraryHeadingTitle = libraryTab === "home"
    ? "Home"
    : selectedPlatform?.name ?? selectedCollection?.name ?? LIBRARY_TAB_LABELS[libraryTab];
  const librarySourceLabel = libraryProvenance?.source === "live"
    ? "Live from RomM"
    : libraryProvenance?.source === "local"
      ? "On this device"
      : libraryProvenance?.stale
        ? "Offline cache · stale"
        : "Offline cache";
  function refreshCurrentLibrary() {
    if (libraryTab === "home" || !catalogViewActive) {
      void loadLibraryHome();
    } else {
      void loadRoms(true, false, activeLibraryQueryRef.current);
    }
  }

  function cyclePlatformFilter() {
    const options = libraryMetadata?.platforms ?? [];
    const current = options.findIndex((platform) => platform.id === libraryFilters.platformId);
    const next = current + 1 < options.length ? options[current + 1].id : null;
    setLibraryFilters((filters) => ({ ...filters, platformId: next }));
  }

  function cycleCollectionFilter() {
    const options = libraryMetadata?.collections ?? [];
    const current = options.findIndex((collection) =>
      collection.id === libraryFilters.collectionId && collection.kind === libraryFilters.collectionKind,
    );
    const next = current + 1 < options.length ? options[current + 1] : null;
    setLibraryFilters((filters) => ({
      ...filters,
      collectionId: next?.id ?? null,
      collectionKind: next?.kind ?? null,
    }));
  }

  function clearLibraryFilters() {
    setLibraryFilters(DEFAULT_LIBRARY_FILTERS);
    setLibrarySearch("");
  }

  async function probeServer(confirmHttp = false, caOverride = caId) {
    const rememberedHttpApproval = remembersHttpApproval(
      serverUrl,
      restoredAgentStatus?.serverOrigin,
      restoredAgentStatus?.httpApproved ?? false,
    );
    const response = await run("probe", () =>
      requestAgent({
        type: "probe",
        baseUrl: serverUrl,
        confirmHttp: confirmHttp || rememberedHttpApproval,
        caId: caOverride,
      }),
    );
    if (response?.type === "error") {
      if (response.error.code === "insecure_http_confirmation_required") {
        const details = response.error.details as { serverOrigin?: string } | undefined;
        setHttpWarningOrigin(details?.serverOrigin ?? serverUrl);
        window.requestAnimationFrame(() => setFocus("CONFIRM-HTTP"));
      } else if (response.error.code === "tls_error") {
        window.requestAnimationFrame(() => setFocus("IMPORT-CA"));
      }
      return;
    }
    if (response?.type === "probe") {
      setHttpWarningOrigin(null);
      setProbe(response.result);
      setServerUrl(response.result.normalizedUrl);
      if (!response.result.compatible) {
        setMessage(`RomM ${response.result.serverVersion} is outside the supported 5.x range.`);
      } else {
        await onboarding.refetch();
        window.setTimeout(() => setFocus("PAIRING-CODE"), 0);
      }
    }
  }

  async function submitProbe(event: FormEvent) {
    event.preventDefault();
    await probeServer();
  }

  async function pair() {
    const response = await run("pair", () =>
      requestAgent({ type: "exchangePairingCode", code: pairingCode }),
    );
    if (response?.type === "error") {
      setPairingCode("");
      if (response.error.code === "expired_pairing_code") {
        setPairingExpiresAt(null);
      }
      window.requestAnimationFrame(() => setFocus("PAIRING-CODE"));
    } else if (response?.type === "authenticated") {
      const connectionReady = response.result.connectionState === "connected";
      applyFavoriteReconciliation(response.result.favoriteReconciliation);
      setSignedOutLocally(false);
      setConnected(connectionReady);
      setPairingCode("");
      setNotice(authenticationNotice(response.result));
      await onboarding.refetch();
      await status.refetch();
      if (!connectionReady) return;
      setDeviceResolved(false);
      await prepareDeviceRegistration();
    }
  }

  async function useManualToken() {
    const response = await run("token", () =>
      requestAgent({ type: "setManualToken", token: manualToken }),
    );
    if (response?.type === "authenticated") {
      const connectionReady = response.result.connectionState === "connected";
      applyFavoriteReconciliation(response.result.favoriteReconciliation);
      setSignedOutLocally(false);
      setConnected(connectionReady);
      setManualToken("");
      setNotice(authenticationNotice(response.result));
      await onboarding.refetch();
      await status.refetch();
      if (!connectionReady) return;
      setDeviceResolved(false);
      await prepareDeviceRegistration();
    } else if (response?.type === "error") {
      window.requestAnimationFrame(() => setFocus("MANUAL-TOKEN"));
    }
  }

  async function reconnect(restoring = false) {
    const response = await run("reconnect", () => requestAgent({ type: "reconnect" }));
    setRestoringCredential(false);
    if (response?.type === "authenticated") {
      const connectionReady = response.result.connectionState === "connected";
      applyFavoriteReconciliation(response.result.favoriteReconciliation);
      setSignedOutLocally(false);
      setConnected(connectionReady);
      setNotice(authenticationNotice(response.result));
      await onboarding.refetch();
      await status.refetch();
      if (!connectionReady) {
        if (!restoring) window.setTimeout(() => setFocus("RECONNECT"), 0);
        return;
      }
      setDeviceResolved(false);
      await prepareDeviceRegistration();
    } else if (!restoring) {
      window.setTimeout(() => setFocus("RECONNECT"), 0);
    }
  }

  async function registerDevice() {
    if (!device) {
      await prepareDeviceRegistration();
      return;
    }
    const validationError = validateDeviceDisplayName(deviceDisplayName);
    if (validationError) {
      setMessage(validationError);
      window.setTimeout(() => setFocus("DEVICE-NAME"), 0);
      return;
    }

    const response = await run("device-registration", () =>
      requestAgent({ type: "registerDevice", displayName: deviceDisplayName }),
    );
    if (response?.type === "deviceRegistered") {
      acceptDeviceIdentity(response.device);
      setDeviceVerified(true);
      setNotice(
        response.newlyRegistered
          ? `${response.device.displayName} is registered with RomM.`
          : `${response.device.displayName} was already registered and has been restored.`,
      );
      restoredConnection.current = true;
      await onboarding.refetch();
      await status.refetch();
    } else if (response?.type === "error") {
      window.setTimeout(
        () => setFocus(deviceIsMissing ? "DEVICE-RECOVERY-ACTION" : "REGISTER-DEVICE"),
        0,
      );
    }
  }

  async function browseDirectories(path?: string) {
    setDirectoryBrowserBusy(true);
    setDirectoryBrowserError(null);
    try {
      const response = await requestAgent({ type: "browseDirectories", path });
      if (response.type === "directoryListing") {
        setDirectoryListing(response.listing);
        window.requestAnimationFrame(() => {
          updateAllLayouts();
          setFocus(
            response.listing.currentPath
              ? "DIRECTORY-USE"
              : response.listing.entries.length > 0
                ? "DIRECTORY-ENTRY-0"
                : "DIRECTORY-CANCEL",
          );
        });
      } else if (response.type === "error") {
        setDirectoryBrowserError(response.error.message);
      } else {
        setDirectoryBrowserError("The agent returned an unexpected directory response.");
      }
    } catch (error) {
      setDirectoryBrowserError(error instanceof Error ? error.message : String(error));
    } finally {
      setDirectoryBrowserBusy(false);
    }
  }

  function openDirectoryBrowser(
    target: DirectoryBrowserTarget,
    currentPath: string,
  ) {
    setDirectoryBrowserTarget(target);
    setDirectoryListing(null);
    setDirectoryBrowserError(null);
    setDirectoryBrowserNotice(null);
    setDirectoryCreationOpen(false);
    setDirectoryName("");
    void browseDirectories(currentPath.trim() || undefined);
  }

  function closeDirectoryBrowser() {
    const returnFocusKey = directoryBrowserTarget?.returnFocusKey;
    setDirectoryBrowserTarget(null);
    setDirectoryListing(null);
    setDirectoryBrowserError(null);
    setDirectoryBrowserNotice(null);
    setDirectoryCreationOpen(false);
    setDirectoryName("");
    if (returnFocusKey) window.requestAnimationFrame(() => setFocus(returnFocusKey));
  }

  function chooseBrowsedDirectory() {
    const path = directoryListing?.currentPath;
    if (!directoryBrowserTarget || !path) return;
    setMappingDrafts((current) => updateMappingPathAt(
      current,
      directoryBrowserTarget.draftId,
      directoryBrowserTarget.field,
      directoryBrowserTarget.index,
      path,
    ));
    setMappingIssues([]);
    closeDirectoryBrowser();
  }

  async function createBrowsedDirectory() {
    const parentPath = directoryListing?.currentPath;
    const name = directoryName.trim();
    if (!parentPath || !name) {
      setDirectoryBrowserError("Enter one folder name before confirming creation.");
      window.requestAnimationFrame(() => setFocus("DIRECTORY-NEW-NAME"));
      return;
    }
    setDirectoryBrowserBusy(true);
    setDirectoryBrowserError(null);
    try {
      const response = await requestAgent({
        type: "createDirectory",
        parentPath,
        name,
        confirmed: true,
      });
      if (response.type === "directoryCreated") {
        setDirectoryCreationOpen(false);
        setDirectoryName("");
        setDirectoryBrowserNotice(
          response.result.created
            ? `Created ${response.result.path}. Review it, then choose Use this folder.`
            : "That folder already existed. Review it, then choose Use this folder.",
        );
        await browseDirectories(response.result.path);
      } else if (response.type === "error") {
        setDirectoryBrowserError(response.error.message);
      } else {
        setDirectoryBrowserError("The agent returned an unexpected directory-creation response.");
      }
    } catch (error) {
      setDirectoryBrowserError(error instanceof Error ? error.message : String(error));
    } finally {
      setDirectoryBrowserBusy(false);
    }
  }

  async function scanMappings() {
    setMappingIssues([]);
    setNoPlatformsArmed(false);
    const response = await run("mapping-detection", () => requestAgent({ type: "detectMappings" }));
    if (response?.type === "mappingDetection") {
      setMappingDrafts(response.result.drafts);
      setMappingEvidence(response.result.evidence);
      setMappingScanned(true);
      setShowAllMappingPlatforms(response.result.detectedCount === 0);
      if (response.result.presetUpdates.length > 0) {
        const preserved = response.result.presetUpdates.reduce(
          (count, update) => count + update.preservedCustomFields.length,
          0,
        );
        setNotice(
          `${response.result.presetUpdates.length} mapping ${response.result.presetUpdates.length === 1 ? "preset was" : "presets were"} updated.${preserved > 0 ? ` ${preserved} custom ${preserved === 1 ? "field was" : "fields were"} preserved.` : ""}`,
        );
      }
      window.requestAnimationFrame(() => {
        updateAllLayouts();
        setFocus("REVIEW-MAPPINGS");
      });
    }
  }

  async function reviewDetectedMappings() {
    const response = await run("mapping-draft-save", () =>
      requestAgent({ type: "saveMappingDrafts", drafts: mappingDrafts }),
    );
    if (response?.type === "mappingValidation") {
      setMappingIssues(response.result.issues);
      setMessage(response.result.issues[0]?.message ?? "Review the mapping drafts.");
      return;
    }
    if (response?.type === "mappingDrafts") {
      setMappingDrafts(response.drafts);
      mappingDraftLoadAttempted.current = true;
      await onboarding.refetch();
      window.requestAnimationFrame(() => setFocus("MAPPING-SEARCH"));
    }
  }

  async function saveReviewedMappings(noPlatforms: boolean) {
    const drafts = normalizeMappingPaths(noPlatforms
      ? mappingDrafts.map((draft) => ({ ...draft, enabled: false }))
      : mappingDrafts);
    const response = await run("mapping-save", () =>
      requestAgent({ type: "saveMappings", drafts, noPlatforms }),
    );
    if (response?.type === "mappingValidation") {
      setMappingIssues(response.result.issues);
      setMappingPathChecks(response.result.paths);
      setMessage(response.result.issues[0]?.message ?? "Review the highlighted mapping.");
      const firstDraft = response.result.issues.find((issue) => issue.draftId)?.draftId;
      window.requestAnimationFrame(() => setFocus(firstDraft ? `MAPPING-TOGGLE-${firstDraft}` : "MAPPING-SEARCH"));
      return;
    }
    if (response?.type === "mappingsSaved") {
      setMappingDrafts(drafts);
      setMappingIssues([]);
      setNoPlatformsArmed(false);
      await onboarding.refetch();
      setNotice(noPlatforms
        ? "Setup will continue without local platform folders."
        : `${response.mappings.length} platform ${response.mappings.length === 1 ? "mapping" : "mappings"} saved.`);
    }
  }

  async function recheckMappingSafety() {
    const drafts = normalizeMappingPaths(mappingDrafts);
    const response = await run("mapping-recheck", () =>
      requestAgent({ type: "validateMappings", drafts, noPlatforms: false }),
    );
    if (response?.type !== "mappingValidation") return;
    setMappingIssues(response.result.issues);
    setMappingPathChecks(response.result.paths);
    if (response.result.valid) {
      setMessage(null);
      setNotice(`${response.result.paths.length} configured ${response.result.paths.length === 1 ? "folder is" : "folders are"} available.`);
    } else {
      setMessage(response.result.issues[0]?.message ?? "One or more folders need attention.");
    }
  }

  async function saveOnboardingPreferences() {
    setBackgroundSetupWarning(null);
    const response = await run("onboarding-preferences", () =>
      requestAgent({
        type: "configureOnboardingPreferences",
        backgroundEnabled: backgroundStartupEnabled,
        stableUpdateChecksEnabled,
      }),
    );
    if (response?.type !== "onboardingPreferences") return;
    setBackgroundStartupEnabled(response.result.backgroundEnabled);
    setStableUpdateChecksEnabled(response.result.stableUpdateChecksEnabled);
    if (response.result.warning) {
      setBackgroundSetupWarning(response.result.warning);
      setNotice(null);
      window.requestAnimationFrame(() => setFocus("PREF-BACKGROUND"));
      return;
    }
    setNotice(
      response.result.backgroundEnabled
        ? "Background startup is enabled. RomM Companion can continue work without the window open."
        : "Background startup is off. RomM Companion will work while the app remains open or in the tray.",
    );
    await onboarding.refetch();
    await status.refetch();
  }

  async function persistDeviceRename(value: string) {
    const current = deviceRef.current;
    if (!current) return;
    const nextName = normalizePendingDeviceRename(value, current.displayName);
    if (!nextName) {
      setDeviceRenameStatus(validateDeviceDisplayName(value) ? "error" : "idle");
      return;
    }

    setDeviceRenameStatus("saving");
    const response = await run("device-update", () =>
      requestAgent({ type: "updateDevice", displayName: nextName }),
    );
    if (response?.type === "deviceUpdated") {
      deviceRef.current = response.device;
      setDevice(response.device);
      setDeviceVerified(true);
      if (settingsDeviceNameRef.current.trim() === nextName) {
        setSettingsDeviceName(response.device.displayName);
        setDeviceRenameStatus("saved");
      } else {
        setDeviceRenameStatus("pending");
      }
      await status.refetch();
    } else if (response?.type === "error") {
      setDeviceRenameStatus("error");
      if (response.error.code === "device_missing" && current) {
        const missing: DeviceIdentity = { ...current, registrationState: "missing" };
        deviceRef.current = missing;
        setDevice(missing);
        setDeviceVerified(false);
        setDeviceSettingsOpen(false);
      }
    }
  }

  function scheduleDeviceRename(value: string) {
    setSettingsDeviceName(value);
    settingsDeviceNameRef.current = value;
    if (deviceRenameTimer.current !== null) {
      window.clearTimeout(deviceRenameTimer.current);
      deviceRenameTimer.current = null;
    }
    const current = deviceRef.current;
    const pending = current
      ? normalizePendingDeviceRename(value, current.displayName)
      : null;
    if (!pending) {
      setDeviceRenameStatus(validateDeviceDisplayName(value) ? "error" : "idle");
      return;
    }
    setDeviceRenameStatus("pending");
    deviceRenameTimer.current = window.setTimeout(() => {
      deviceRenameTimer.current = null;
      void persistDeviceRename(pending);
    }, DEVICE_RENAME_DEBOUNCE_MS);
  }

  function openDeviceSettings() {
    if (!device) return;
    setSettingsDeviceName(device.displayName);
    settingsDeviceNameRef.current = device.displayName;
    setDeviceRenameStatus("idle");
    setDeviceSettingsOpen(true);
    window.setTimeout(() => setFocus("SETTINGS-DEVICE-NAME"), 0);
  }

  function copyControllerSettings(source: ControllerSettings): ControllerSettings {
    return {
      ...source,
      globalBindings: { ...source.globalBindings },
      controllerBindings: Object.fromEntries(
        Object.entries(source.controllerBindings).map(([guid, bindings]) => [guid, { ...bindings }]),
      ),
    };
  }

  function openControllerSettings() {
    const draft = copyControllerSettings(controllerSettings);
    setControllerSettingsDraft(draft);
    setControllerSettingsError(null);
    setControllerSettingsScope(activeController ? "controller" : "global");
    setControllerSettingsOpen(true);
    window.setTimeout(
      () => setFocus(activeController ? "CONTROLLER-SCOPE-CONTROLLER" : "CONTROLLER-SCOPE-GLOBAL"),
      0,
    );
  }

  function closeControllerSettings() {
    setControllerSettingsOpen(false);
    setControllerSettingsError(null);
    window.setTimeout(() => setFocus("CONTROLLER-SETTINGS"), 0);
  }

  function selectControllerSettingsScope(scope: "global" | "controller") {
    const next = scope === "controller" && !activeController ? "global" : scope;
    setControllerSettingsScope(next);
    window.setTimeout(
      () => setFocus(next === "global" ? "CONTROLLER-SCOPE-GLOBAL" : "CONTROLLER-SCOPE-CONTROLLER"),
      0,
    );
  }

  function changeControllerBinding(action: ControllerBindingAction) {
    setControllerSettingsDraft((current) => {
      const next = copyControllerSettings(current);
      if (controllerSettingsScope === "controller" && activeController) {
        const currentBindings = next.controllerBindings[activeController.guid] ?? {
          ...next.globalBindings,
        };
        next.controllerBindings[activeController.guid] = nextControllerBinding(
          currentBindings,
          action,
        );
      } else {
        next.globalBindings = nextControllerBinding(next.globalBindings, action);
      }
      setControllerSettingsError(validateControllerSettings(next));
      return next;
    });
  }

  function resetControllerBindings() {
    setControllerSettingsDraft((current) => {
      const next = copyControllerSettings(current);
      if (controllerSettingsScope === "controller" && activeController) {
        delete next.controllerBindings[activeController.guid];
      } else {
        next.globalBindings = defaultControllerSettings().globalBindings;
      }
      setControllerSettingsError(null);
      return next;
    });
  }

  function adjustControllerTiming(
    field: "deadZonePercent" | "initialRepeatDelayMs" | "repeatIntervalMs",
    delta: number,
  ) {
    const limits = {
      deadZonePercent: [10, 50],
      initialRepeatDelayMs: [200, 1000],
      repeatIntervalMs: [50, 300],
    } as const;
    setControllerSettingsDraft((current) => {
      const [minimum, maximum] = limits[field];
      const next = {
        ...current,
        [field]: Math.max(minimum, Math.min(maximum, current[field] + delta)),
      };
      setControllerSettingsError(validateControllerSettings(next));
      return next;
    });
  }

  async function saveControllerSettings() {
    const validationError = validateControllerSettings(controllerSettingsDraft);
    if (validationError) {
      setControllerSettingsError(validationError);
      return;
    }
    setSettingsBusy(true);
    try {
      const settings = await updateDesktopSettings({
        closeBehavior,
        fullscreen,
        stableUpdateChecksEnabled,
        controller: controllerSettingsDraft,
      });
      setCloseBehavior(settings.closeBehavior);
      setFullscreen(settings.fullscreen);
      setControllerSettings(settings.controller);
      setControllerSettingsDraft(copyControllerSettings(settings.controller));
      setControllerSettingsOpen(false);
      setNotice("Controller settings saved and applied.");
      window.setTimeout(() => setFocus("CONTROLLER-SETTINGS"), 0);
    } catch (error) {
      setControllerSettingsError(error instanceof Error ? error.message : String(error));
    } finally {
      setSettingsBusy(false);
    }
  }

  function focusLibrarySearch() {
    if (document.querySelector('[aria-modal="true"]')) return;
    if (primaryViewRef.current !== "library") return;
    const searchVisible = queryForLibraryTab(libraryTab) !== null ||
      (libraryTab === "platforms" && libraryQuery.kind === "platform") ||
      (libraryTab === "collections" &&
        (libraryQuery.kind === "collection" || libraryQuery.kind === "smart_collection"));
    if (searchVisible) {
      setFocus("LIBRARY-SEARCH");
    } else {
      setFocus(`LIBRARY-TAB-${libraryTab.toUpperCase()}`);
    }
  }

  function openFocusedGameContext() {
    if (document.querySelector('[aria-modal="true"]')) return;
    if (primaryViewRef.current !== "library") return;
    const activeElement = document.activeElement as HTMLElement | null;
    const romId = Number(activeElement?.dataset.romId);
    if (!Number.isFinite(romId)) return;
    const rom = [...romsRef.current, ...Object.values(homeShelves).flat()]
      .find((candidate) => candidate.id === romId);
    if (!rom) return;
    openGameContext(rom);
  }

  function openGameContext(rom: RomSummary) {
    if (document.querySelector('[aria-modal="true"]')) return;
    contextReturnFocusKey.current = getCurrentFocusKey() ?? `ROM-${rom.id}`;
    setContextRom(rom);
    setGameDetails(null);
    setGameDetailsError(null);
    setGameDetailsLoading(true);
    const requestNumber = ++gameDetailsRequestRef.current;
    const favoriteRevisionAtRequest = favoriteMutationRevisions.current.get(rom.id) ?? 0;
    if (mappingDrafts.length === 0) {
      void requestAgent({ type: "getMappingDrafts" }).then((response) => {
        if (response.type === "mappingDrafts") setMappingDrafts(response.drafts);
      });
    }
    void requestAgent({ type: "getGameDetails", romId: rom.id })
      .then((response) => {
        if (gameDetailsRequestRef.current !== requestNumber) return;
        if (response.type === "gameDetails" && response.details.rom.id === rom.id) {
          const currentFavoriteRevision = favoriteMutationRevisions.current.get(rom.id) ?? 0;
          const currentFavorite = favoriteValues.current.get(rom.id);
          if (currentFavoriteRevision !== favoriteRevisionAtRequest && currentFavorite !== undefined) {
            setGameDetails({
              ...response.details,
              rom: setRomFavorite(
                response.details.rom,
                currentFavorite,
                favoriteQueuedDesiredValues.current.has(rom.id),
              ),
            });
          } else {
            setGameDetails({
              ...response.details,
              rom: reconcileLoadedFavorite(response.details.rom),
            });
          }
        } else if (response.type === "error") {
          setGameDetailsError(response.error.message);
        }
      })
      .catch((error) => {
        if (gameDetailsRequestRef.current === requestNumber) {
          setGameDetailsError(error instanceof Error ? error.message : String(error));
        }
      })
      .finally(() => {
        if (gameDetailsRequestRef.current === requestNumber) setGameDetailsLoading(false);
      });
    window.setTimeout(() => setFocus("GAME-FAVORITE"), 0);
  }

  function closeGameContext() {
    gameDetailsRequestRef.current += 1;
    setContextRom(null);
    setGameDetails(null);
    setGameDetailsError(null);
    setGameDetailsLoading(false);
    window.setTimeout(() => setFocus(contextReturnFocusKey.current), 0);
  }

  function closeDeviceSettings() {
    if (deviceRenameTimer.current !== null) {
      window.clearTimeout(deviceRenameTimer.current);
      deviceRenameTimer.current = null;
    }
    const current = deviceRef.current;
    const pending = current
      ? normalizePendingDeviceRename(settingsDeviceNameRef.current, current.displayName)
      : null;
    setDeviceSettingsOpen(false);
    if (pending) void persistDeviceRename(pending);
    window.setTimeout(() => setFocus("DEVICE-SETTINGS"), 0);
  }

  function openLogoutDialog() {
    if (deviceRenameTimer.current !== null) {
      window.clearTimeout(deviceRenameTimer.current);
      deviceRenameTimer.current = null;
    }
    logoutReturnFocusKey.current = deviceSettingsOpen
      ? "DEVICE-SETTINGS"
      : deviceVerified
        ? "LOGOUT"
        : "DEVICE-SIGN-OUT";
    setDeviceSettingsOpen(false);
    setLogoutDialogOpen(true);
    window.setTimeout(() => setFocus("LOGOUT-ONLY"), 0);
  }

  function closeLogoutDialog() {
    setLogoutDialogOpen(false);
    window.setTimeout(() => setFocus(logoutReturnFocusKey.current), 0);
  }

  async function logout(removeDevice: boolean) {
    const tokenId = restoredAgentStatus?.tokenId;
    const response = await run("logout", () =>
      requestAgent({ type: "logout", removeDevice }),
    );
    if (response?.type === "loggedOut") {
      setLogoutDialogOpen(false);
      setSignedOutLocally(true);
      setConnected(false);
      setDevice(null);
      deviceRef.current = null;
      setDeviceDisplayName("");
      setDeviceResolved(false);
      setDeviceVerified(false);
      deviceNameDirty.current = false;
      restoredConnection.current = false;
      setProbe(serverUrl ? {
        normalizedUrl: serverUrl,
        serverVersion: restoredAgentStatus?.serverVersion ?? "unknown",
        compatible: true,
      } : null);
      setRoms([]);
      romsRef.current = [];
      setHomeShelves({ recent: [], favorites: [], downloaded: [], downloads: [] });
      setHomeInitialized(false);
      libraryCatalogsRef.current.clear();
      favoriteSessionGeneration.current += 1;
      favoriteMutationQueue.current = createPerRomMutationQueue();
      favoriteMutationRevisions.current.clear();
      favoriteValues.current.clear();
      favoriteAuthoritativeValues.current.clear();
      favoriteQueuedDesiredValues.current.clear();
      favoritePendingRomIds.current.clear();
      setFavoritePendingIds(new Set());
      setFavoriteFailure(null);
      romTotalRef.current = null;
      setRomTotal(null);
      nextRomOffsetRef.current = 0;
      hasMoreRomsRef.current = false;
      setHasMoreRoms(false);
      setLibrarySourceNotice(null);
      setLibrarySearch("");
      setContextRom(null);
      setControllerSettingsOpen(false);
      setMappingDrafts([]);
      setMappingEvidence([]);
      setMappingScanned(false);
      setMappingIssues([]);
      mappingDraftLoadAttempted.current = false;
      await onboarding.refetch();
      const tokenInstruction = tokenId
        ? `Token #${tokenId} remains active until you revoke it in RomM.`
        : "The client token remains active until you revoke it in RomM.";
      switch (response.deviceRemoval.outcome) {
        case "removed":
          setNotice(`The RomM device was removed and you were signed out locally. ${tokenInstruction}`);
          break;
        case "already_missing":
          setNotice(`The RomM device was already absent and you were signed out locally. ${tokenInstruction}`);
          break;
        case "failed":
          setNotice(
            `Signed out locally, but RomM could not remove device ${response.deviceRemoval.deviceId ?? "registration"}: ${response.deviceRemoval.error?.message ?? "unknown error"} ${tokenInstruction}`,
          );
          break;
        default:
          setNotice(`Signed out locally. Your RomM device registration was kept. ${tokenInstruction}`);
      }
      reconnectAttempted.current = true;
      void status.refetch();
      window.setTimeout(() => setFocus("PAIRING-CODE"), 0);
    }
  }

  async function importCertificate(file: File) {
    setBusy("certificate");
    setMessage(null);
    setLastError(null);
    try {
      const bytes = new Uint8Array(await file.arrayBuffer());
      let binary = "";
      for (let offset = 0; offset < bytes.length; offset += 0x8000) {
        binary += String.fromCharCode(...bytes.subarray(offset, offset + 0x8000));
      }
      const response = await requestAgent({
        type: "importCa",
        baseUrl: serverUrl,
        certificate: window.btoa(binary),
      });
      if (response.type === "caImported") {
        setCaId(response.result.caId);
        setNotice(`Certificate authority imported for ${response.result.serverOrigin}.`);
        await probeServer(false, response.result.caId);
      } else if (response.type === "error") {
        setLastError(response.error);
        setMessage(response.error.message);
      }
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(null);
      if (caFileInput.current) caFileInput.current.value = "";
    }
  }

  async function chooseCloseBehavior(nextBehavior: CloseBehavior) {
    setSettingsBusy(true);
    setMessage(null);
    try {
      const settings = await updateDesktopSettings({
        closeBehavior: nextBehavior,
        fullscreen,
        stableUpdateChecksEnabled,
        controller: controllerSettings,
      });
      setCloseBehavior(settings.closeBehavior);
      setFullscreen(settings.fullscreen);
      setControllerSettings(settings.controller);
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setSettingsBusy(false);
    }
  }

  async function toggleFullscreen() {
    setSettingsBusy(true);
    setMessage(null);
    try {
      const settings = await updateDesktopSettings({
        closeBehavior,
        fullscreen: !fullscreen,
        stableUpdateChecksEnabled,
        controller: controllerSettings,
      });
      setCloseBehavior(settings.closeBehavior);
      setFullscreen(settings.fullscreen);
      setControllerSettings(settings.controller);
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setSettingsBusy(false);
    }
  }

  return (
    <FocusContext.Provider value={focusKey}>
      <main ref={ref} className="app-shell" data-input-modality={inputModality}>
        <header className="topbar">
          <div>
            <p className="eyebrow">Architecture spike · 0.1.0</p>
            <h1>RomM Companion</h1>
          </div>
          <div className="status-cluster" role="status" aria-live="polite" aria-atomic="true">
            <span className={`status-dot ${status.isError ? "error" : ""}`} aria-hidden="true" />
            <span>{agentLabel}</span>
            <span className="controller-pill" title={inputFamilyLabel(inputFamily)}>
              <InputFamilyIcon family={inputFamily} />
              {controllerName ?? "PC input"}
            </span>
            {controllerNotice && <span className="controller-notice">{controllerNotice}</span>}
            <FocusButton
              onPress={() => void toggleFullscreen()}
              variant="quiet"
              disabled={settingsBusy}
              selected={fullscreen}
              focusKey="TOGGLE-FULLSCREEN"
              navigation={{ down: screenEntryFocusKey ?? "CLOSE-TO-TRAY" }}
            >
              {fullscreen ? "Exit fullscreen" : "Fullscreen"}
            </FocusButton>
          </div>
        </header>

        <AnimatePresence mode="wait">
          {primaryView === "startup" ? (
            <motion.section
              key="startup"
              className="startup-view"
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
            >
              <motion.div
                className="startup-mark"
                animate={reduceMotion
                  ? { scale: 1, opacity: 1 }
                  : { scale: [1, 1.08, 1], opacity: [0.65, 1, 0.65] }}
                transition={reduceMotion
                  ? { duration: 0 }
                  : { duration: 1.4, repeat: Infinity, ease: "easeInOut" }}
                aria-hidden="true"
              >
                R
              </motion.div>
              <p className="eyebrow accent">
                {isResolvingDevice ? "Preparing this device" : "Restoring your session"}
              </p>
              <h2>
                {isResolvingDevice ? "Loading device registration…" : "Starting RomM Companion…"}
              </h2>
            </motion.section>
          ) : primaryView === "connection" ? (
            <FocusScreen
              key="connect"
              scopeKey="CONNECTION-SCREEN"
              entryFocusKey={connectionEntryFocusKey}
              className="connect-layout"
              label="RomM server connection"
            >
              <div className="hero-copy">
                <p className="eyebrow accent">Your collection, ready everywhere</p>
                <h2>Connect this device to RomM.</h2>
                <p>
                  This first slice proves native controller navigation, the separate Rust agent,
                  RomM 5.x pairing, and a real library response.
                </p>
                <OnboardingProgress
                  state={onboardingState}
                  fallbackCurrent={probe?.compatible ? "authentication" : "server"}
                />
                <div className="input-legend">
                  <span><InputHint family={inputFamily} action="confirm" button={activeControllerBindings.confirm} /> Select</span>
                  <span><InputHint family={inputFamily} action="back" button={activeControllerBindings.back} /> Back</span>
                  <span><InputHint family={inputFamily} action="navigate" /> Navigate</span>
                </div>
              </div>

              <form className="connection-card" onSubmit={submitProbe}>
                <div className="step-number">01</div>
                <h2>RomM server</h2>
                <p>Enter the address you use to open RomM. The agent validates its OpenAPI document.</p>
                {reconnectVisible && (
                  <div className="trust-panel">
                    <strong>Saved connection needs attention</strong>
                    <p>
                      Status: {restoredAgentStatus?.connection.replaceAll("_", " ")}.
                      Your saved credential has not been deleted.
                    </p>
                    <FocusButton
                      onPress={() => void reconnect(false)}
                      variant="quiet"
                      disabled={busy !== null}
                      focusKey="RECONNECT"
                      navigation={{ up: "TOGGLE-FULLSCREEN", down: "SERVER-URL" }}
                    >
                      {busy === "reconnect" ? "Reconnecting…" : "Reconnect"}
                    </FocusButton>
                  </div>
                )}
                <FocusInput
                  label="Server URL"
                  value={serverUrl}
                  onChange={setServerUrl}
                  placeholder="https://romm.example.com"
                  inputMode="url"
                  focusKey="SERVER-URL"
                  navigation={{
                    up: reconnectVisible ? "RECONNECT" : "TOGGLE-FULLSCREEN",
                    down: "TEST-SERVER",
                  }}
                  onOpenKeyboard={() => setKeyboard({
                    focusKey: "SERVER-URL",
                    label: "Server URL",
                    value: serverUrl,
                    mode: "url",
                    commit: setServerUrl,
                  })}
                />
                <FocusButton
                  onPress={() => void probeServer()}
                  variant="primary"
                  disabled={!serverUrl || busy !== null}
                  focusKey="TEST-SERVER"
                  navigation={{ up: "SERVER-URL", down: connectionAfterTestFocusKey }}
                >
                  {busy === "probe" ? "Testing…" : "Test server"}
                </FocusButton>

                {httpWarningOrigin && (
                  <motion.div className="trust-panel warning" initial={{ opacity: 0 }} animate={{ opacity: 1 }}>
                    <strong>Unencrypted connection</strong>
                    <p>
                      {httpWarningOrigin} uses HTTP. Your token and downloaded content can be
                      read by other systems on the network.
                    </p>
                    <div className="action-row">
                      <FocusButton
                        onPress={() => void probeServer(true)}
                        variant="primary"
                        disabled={busy !== null}
                        focusKey="CONFIRM-HTTP"
                        navigation={{ up: "TEST-SERVER", right: "CANCEL-HTTP" }}
                      >
                        Connect anyway
                      </FocusButton>
                      <FocusButton
                        onPress={() => setHttpWarningOrigin(null)}
                        variant="quiet"
                        focusKey="CANCEL-HTTP"
                        navigation={{ up: "TEST-SERVER", left: "CONFIRM-HTTP" }}
                      >
                        Cancel
                      </FocusButton>
                    </div>
                  </motion.div>
                )}

                {lastError?.code === "tls_error" && (
                  <div className="trust-panel">
                    <strong>Certificate verification failed</strong>
                    <p>Import the private certificate authority used by this RomM server.</p>
                    <input
                      ref={caFileInput}
                      className="visually-hidden"
                      type="file"
                      accept=".pem,.crt,.cer,application/x-pem-file,application/pkix-cert"
                      onChange={(event) => {
                        const file = event.target.files?.[0];
                        if (file) void importCertificate(file);
                      }}
                    />
                    <FocusButton
                      onPress={() => caFileInput.current?.click()}
                      variant="quiet"
                      disabled={busy !== null}
                      focusKey="IMPORT-CA"
                      navigation={{ up: "TEST-SERVER" }}
                    >
                      {busy === "certificate" ? "Importing…" : "Import certificate"}
                    </FocusButton>
                  </div>
                )}

                {probe?.compatible && (
                  <motion.div className="pair-panel" initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }}>
                    <div className="success-line">
                      <span>Connected</span>
                      <strong>RomM {probe.serverVersion}</strong>
                    </div>
                    <p
                      className={`pairing-timer ${pairingSecondsLeft === 0 ? "expired" : ""}`}
                      role="timer"
                      aria-live="polite"
                    >
                      {pairingSecondsLeft > 0
                        ? `Complete pairing within ${Math.floor(pairingSecondsLeft / 60)}:${String(pairingSecondsLeft % 60).padStart(2, "0")}`
                        : "Pairing entry expired. Test the server again to restart."}
                    </p>
                    <FocusInput
                      label="Pairing code"
                      value={pairingCode}
                      onChange={(value) => setPairingCode(formatPairingCode(value))}
                      placeholder="JM38-MHSA"
                      inputMode="text"
                      maxLength={9}
                      focusKey="PAIRING-CODE"
                      navigation={{ up: "TEST-SERVER", down: "PAIR" }}
                      onOpenKeyboard={() => setKeyboard({
                        focusKey: "PAIRING-CODE",
                        label: "Pairing code",
                        value: pairingCode,
                        mode: "pairing",
                        maxLength: 9,
                        commit: (value) => setPairingCode(formatPairingCode(value)),
                      })}
                    />
                    <FocusButton
                      onPress={() => void pair()}
                      variant="primary"
                      disabled={
                        !PAIRING_CODE_PATTERN.test(pairingCode) ||
                        busy !== null ||
                        pairingExpiresAt === null ||
                        pairingSecondsLeft === 0
                      }
                      focusKey="PAIR"
                      navigation={{ up: "PAIRING-CODE", down: "CANCEL-PAIRING" }}
                    >
                      {busy === "pair" ? "Pairing…" : "Pair device"}
                    </FocusButton>
                    <FocusButton
                      onPress={() => {
                        setPairingCode("");
                        setPairingExpiresAt(null);
                        setProbe(null);
                        window.setTimeout(() => setFocus("SERVER-URL"), 0);
                      }}
                      variant="quiet"
                      disabled={busy !== null}
                      focusKey="CANCEL-PAIRING"
                      navigation={{ up: "PAIR", down: "TOKEN-DISCLOSURE" }}
                    >
                      Cancel pairing
                    </FocusButton>

                    <div className="focus-disclosure">
                      <FocusButton
                        onPress={() => {
                          const next = !manualTokenOpen;
                          setManualTokenOpen(next);
                          if (next) {
                            window.requestAnimationFrame(() => setFocus("MANUAL-TOKEN"));
                          }
                        }}
                        variant="quiet"
                        focusKey="TOKEN-DISCLOSURE"
                        selected={manualTokenOpen}
                        navigation={{ up: "CANCEL-PAIRING", down: manualTokenOpen ? "MANUAL-TOKEN" : "CLOSE-TO-TRAY" }}
                      >
                        {manualTokenOpen ? "Hide Client API Token" : "Use a Client API Token instead"}
                      </FocusButton>
                      {manualTokenOpen && (
                        <div className="disclosure-content">
                      <FocusInput
                        label="Client API Token"
                        value={manualToken}
                        onChange={setManualToken}
                        placeholder="rmm_…"
                        type="password"
                        maxLength={68}
                        focusKey="MANUAL-TOKEN"
                        navigation={{ up: "TOKEN-DISCLOSURE", down: "USE-TOKEN" }}
                        onOpenKeyboard={() => setKeyboard({
                          focusKey: "MANUAL-TOKEN",
                          label: "Client API Token",
                          value: manualToken,
                          mode: "token",
                          maxLength: 68,
                          secret: true,
                          commit: setManualToken,
                        })}
                      />
                      <FocusButton
                        onPress={() => void useManualToken()}
                        variant="quiet"
                        disabled={manualToken.length !== 68 || busy !== null}
                        focusKey="USE-TOKEN"
                        navigation={{ up: "MANUAL-TOKEN", down: "CLOSE-TO-TRAY" }}
                      >
                        Use token
                      </FocusButton>
                        </div>
                      )}
                    </div>
                  </motion.div>
                )}
                {message && <p className="error-banner" role="alert">{message}</p>}
                {lastError?.code === "missing_required_scopes" && (
                  <div className="scope-panel" role="status">
                    <strong>Required token permissions</strong>
                    <ul>
                      {((lastError.details as { missingScopes?: string[] } | undefined)?.missingScopes ?? REQUIRED_SCOPES)
                        .map((scope) => <li key={scope}>{scope}</li>)}
                    </ul>
                    <p>Edit or recreate the Client API Token in RomM, then pair again.</p>
                  </div>
                )}
                {notice && <p className="notice-banner" role="status">{notice}</p>}
              </form>
            </FocusScreen>
          ) : primaryView === "device" ? (
            <FocusScreen
              key="device-registration"
              scopeKey="DEVICE-SCREEN"
              entryFocusKey={deviceEntryFocusKey}
              className="device-registration-layout"
              label="Device registration"
              animatedOffset
            >
              <div className="device-registration-copy">
                <p className="eyebrow accent">Connected to {activeServerUrl}</p>
                <h2>{deviceIsMissing ? "Reconnect this device." : "Name this device."}</h2>
                <p>
                  {deviceIsMissing
                    ? "The RomM registration is gone, but your local identity and future folder mappings remain safe."
                    : "RomM uses this identity for future ROM, save, and save-state synchronization. You can change the name later in Settings."}
                </p>
                <OnboardingProgress state={onboardingState} fallbackCurrent="device" />
                <div className="input-legend">
                  <span><InputHint family={inputFamily} action="confirm" button={activeControllerBindings.confirm} /> Select</span>
                  <span><InputHint family={inputFamily} action="back" button={activeControllerBindings.back} /> Backspace keyboard</span>
                  <span><InputHint family={inputFamily} action="navigate" /> Navigate</span>
                </div>
              </div>

              <div
                className="device-registration-card"
                aria-busy={busy === "device-registration" || busy === "device-verification"}
              >
                <div className="step-number">02</div>
                <p className="eyebrow accent">Device registration</p>
                <h2>
                  {!device
                    ? "Device setup needs attention"
                    : deviceIsMissing
                      ? "Registration not found"
                      : devicePermissionBlocked
                        ? "Device permission required"
                        : deviceNeedsRegistration
                          ? "Review this device"
                          : "Verification needs attention"}
                </h2>
                {device ? (
                  <>
                    <p>
                      {deviceIsMissing
                        ? "This device was deleted from RomM. Register it again with the same local settings and name."
                        : devicePermissionBlocked
                          ? "RomM did not allow this token to read the saved device. Restore devices.read permission, then retry verification."
                          : deviceNeedsRegistration
                            ? "This information identifies this installation in RomM. Network addresses are not collected or sent."
                            : lastError?.retryable
                              ? "RomM could not be reached to verify this saved registration. Your local data has not been changed."
                              : "RomM could not confirm that this saved registration belongs to the connected account. Your local data has not been changed."}
                    </p>
                    {deviceNeedsRegistration && (
                      <FocusInput
                        label="Device name"
                        value={deviceDisplayName}
                        onChange={(value) => {
                          deviceNameDirty.current = true;
                          setDeviceDisplayName(value);
                        }}
                        placeholder="Living Room PC"
                        inputMode="text"
                        maxLength={DEVICE_DISPLAY_NAME_MAX_LENGTH}
                        focusKey="DEVICE-NAME"
                        disabled={busy === "device-registration"}
                        invalid={Boolean(deviceNameError)}
                        ariaDescribedBy={deviceNameError ? "device-name-error" : undefined}
                        navigation={{ up: "TOGGLE-FULLSCREEN", down: devicePrimaryActionFocusKey }}
                    onOpenKeyboard={() => setKeyboard({
                          focusKey: "DEVICE-NAME",
                          label: "Device name",
                          value: deviceDisplayName,
                          mode: "text",
                      maxLength: DEVICE_DISPLAY_NAME_MAX_LENGTH,
                      validate: validateDeviceDisplayName,
                      commit: (value) => {
                            deviceNameDirty.current = true;
                            setDeviceDisplayName(value);
                          },
                        })}
                      />
                    )}
                    {deviceNeedsRegistration && deviceNameError && (
                      <p id="device-name-error" className="field-error" role="alert">{deviceNameError}</p>
                    )}
                    <dl className="device-facts">
                      <div>
                        <dt>Platform</dt>
                        <dd>{devicePlatformLabel(device.platform)}</dd>
                      </div>
                      <div>
                        <dt>Hostname</dt>
                        <dd>{device.hostname}</dd>
                      </div>
                      <div>
                        <dt>Synchronization</dt>
                        <dd>Bidirectional push + pull</dd>
                      </div>
                      <div>
                        <dt>Client</dt>
                        <dd>RomM Companion {device.clientVersion}</dd>
                      </div>
                      <div>
                        <dt>RomM status</dt>
                        <dd className={`device-status-value ${device.registrationState}`}>
                          {deviceIsMissing
                            ? "Registration removed"
                            : devicePermissionBlocked
                              ? "Permission blocked"
                              : deviceNeedsRegistration
                                ? "Not registered"
                                : "Verification incomplete"}
                        </dd>
                      </div>
                    </dl>
                    {message && <p className="error-banner" role="alert">{message}</p>}
                    {lastError && deviceNeedsRegistration && (
                      <p className="repair-copy">
                        {lastError.retryable
                          ? "Your login and device name are saved. Check the connection and retry."
                          : "Your login has been kept. Review the error, then retry or sign out to change the server."}
                      </p>
                    )}
                    {(deviceNeedsRegistration || deviceIsMissing) ? (
                      <FocusButton
                        onPress={() => void registerDevice()}
                        variant="primary"
                        disabled={busy !== null || deviceNameError !== null}
                        focusKey={deviceIsMissing ? "DEVICE-RECOVERY-ACTION" : "REGISTER-DEVICE"}
                        navigation={{
                          up: deviceNeedsRegistration ? "DEVICE-NAME" : "TOGGLE-FULLSCREEN",
                          down: "DEVICE-SIGN-OUT",
                        }}
                      >
                        {busy === "device-registration"
                          ? "Registering device…"
                          : deviceIsMissing
                            ? "Register again"
                            : "Register device"}
                      </FocusButton>
                    ) : (
                      <FocusButton
                        onPress={() => void verifyDeviceIdentity(device)}
                        variant="primary"
                        disabled={busy !== null}
                        focusKey="DEVICE-RECOVERY-ACTION"
                        navigation={{ up: "TOGGLE-FULLSCREEN", down: "DEVICE-SIGN-OUT" }}
                      >
                        {busy === "device-verification" ? "Verifying device…" : "Retry verification"}
                      </FocusButton>
                    )}
                  </>
                ) : (
                  <>
                    <p>
                      The local device proposal could not be loaded. Your RomM authentication has
                      been kept, so it is safe to retry.
                    </p>
                    {message && <p className="error-banner" role="alert">{message}</p>}
                    <FocusButton
                      onPress={() => void prepareDeviceRegistration()}
                      variant="primary"
                      disabled={busy !== null}
                      focusKey="RETRY-DEVICE-PROPOSAL"
                      navigation={{ up: "TOGGLE-FULLSCREEN", down: "DEVICE-SIGN-OUT" }}
                    >
                      {busy === "device-proposal" ? "Preparing device…" : "Retry device setup"}
                    </FocusButton>
                  </>
                )}
                <FocusButton
                  onPress={openLogoutDialog}
                  variant="quiet"
                  disabled={busy !== null}
                  focusKey="DEVICE-SIGN-OUT"
                  navigation={{ up: devicePrimaryActionFocusKey, down: "CLOSE-TO-TRAY" }}
                >
                  Sign out
                </FocusButton>
              </div>
            </FocusScreen>
          ) : primaryView === "mapping" ? (
            <FocusScreen
              key="mapping-onboarding"
              scopeKey="MAPPING-ONBOARDING-SCREEN"
              entryFocusKey={onboardingState?.currentStep === "mappings" ? "MAPPING-SEARCH" : "SCAN-MAPPINGS"}
              className="mapping-onboarding-view"
              label="Folder detection and platform mappings"
              animatedOffset
            >
              <div className="mapping-onboarding-heading">
                <div>
                  <p className="eyebrow accent">Local setup</p>
                  <h2>
                    {onboardingState?.currentStep === "mappings"
                      ? "Review platform folders."
                      : "Find your ROM folders."}
                  </h2>
                  <p>
                    {onboardingState?.currentStep === "mappings"
                      ? "Browsing and editing do not change the filesystem. Creating one final folder requires a separate confirmation, and ROMs, saves, and states are never touched."
                      : "Detection is read-only. RomM Companion will not create directories or touch ROMs, saves, or states during this step."}
                  </p>
                </div>
                <OnboardingProgress
                  state={onboardingState}
                  fallbackCurrent={onboardingState?.currentStep === "mappings" ? "mappings" : "detection"}
                  compact
                />
              </div>

              {onboardingState?.currentStep !== "mappings" ? (
                <div className="mapping-detection-card">
                  <p className="eyebrow accent">Step 05 · Folder detection</p>
                  <h3>Scan known EmuDeck locations</h3>
                  <p>
                    Windows checks recognizable EmuDeck roots only. SteamOS also checks internal
                    storage and currently mounted removable media.
                  </p>
                  <FocusButton
                    onPress={() => void scanMappings()}
                    variant="primary"
                    disabled={busy !== null}
                    focusKey="SCAN-MAPPINGS"
                    navigation={{ up: "TOGGLE-FULLSCREEN", down: mappingScanned ? "REVIEW-MAPPINGS" : "CLOSE-TO-TRAY" }}
                  >
                    {busy === "mapping-detection" ? "Scanning folders…" : mappingScanned ? "Scan again" : "Scan folders"}
                  </FocusButton>
                  {mappingScanned && (
                    <div className="mapping-detection-results" role="status">
                      <strong>
                        {mappingDrafts.filter((draft) => draft.enabled).length} matching platform folders found
                      </strong>
                      {mappingEvidence.length > 0 ? (
                        <ul>
                          {mappingEvidence.map((evidence) => (
                            <li key={evidence.id}>
                              <span>{evidence.label} · {evidence.confidence}% confidence</span>
                              <code>{evidence.path}</code>
                            </li>
                          ))}
                        </ul>
                      ) : (
                        <p>No known root was found. You can choose existing folders manually during review.</p>
                      )}
                    </div>
                  )}
                  {message && <p className="error-banner" role="alert">{message}</p>}
                  <FocusButton
                    onPress={() => void reviewDetectedMappings()}
                    variant="secondary"
                    disabled={!mappingScanned || busy !== null}
                    focusKey="REVIEW-MAPPINGS"
                    navigation={{ up: "SCAN-MAPPINGS", down: "CLOSE-TO-TRAY" }}
                  >
                    Review mappings
                  </FocusButton>
                </div>
              ) : (
                <>
                  <div className="mapping-review-toolbar">
                    <FocusInput
                      label="Find a RomM platform"
                      value={mappingPlatformSearch}
                      onChange={setMappingPlatformSearch}
                      placeholder="Game Boy, PlayStation, SNES…"
                      inputMode="text"
                      focusKey="MAPPING-SEARCH"
                      navigation={{ up: "TOGGLE-FULLSCREEN", right: "MAPPING-SHOW-ALL", down: visibleMappingDrafts[0] ? `MAPPING-TOGGLE-${visibleMappingDrafts[0].id}` : "SAVE-MAPPINGS" }}
                      onOpenKeyboard={() => setKeyboard({
                        focusKey: "MAPPING-SEARCH",
                        label: "Find a RomM platform",
                        value: mappingPlatformSearch,
                        mode: "text",
                        commit: setMappingPlatformSearch,
                      })}
                    />
                    <FocusButton
                      onPress={() => setShowAllMappingPlatforms((shown) => !shown)}
                      variant="quiet"
                      selected={showAllMappingPlatforms}
                      focusKey="MAPPING-SHOW-ALL"
                      navigation={{ left: "MAPPING-SEARCH", right: "MAPPING-RECHECK", down: visibleMappingDrafts[0] ? `MAPPING-TOGGLE-${visibleMappingDrafts[0].id}` : "SAVE-MAPPINGS" }}
                    >
                      {showAllMappingPlatforms ? "Show selected" : "Show all platforms"}
                    </FocusButton>
                    <FocusButton
                      onPress={() => void recheckMappingSafety()}
                      variant="quiet"
                      disabled={busy !== null || !mappingDrafts.some((draft) => draft.enabled)}
                      focusKey="MAPPING-RECHECK"
                      navigation={{ left: "MAPPING-SHOW-ALL", down: visibleMappingDrafts[0] ? `MAPPING-TOGGLE-${visibleMappingDrafts[0].id}` : "SAVE-MAPPINGS" }}
                    >
                      {busy === "mapping-recheck" ? "Checking folders…" : "Check folder safety"}
                    </FocusButton>
                    <span>{mappingDrafts.filter((draft) => draft.enabled).length} selected</span>
                  </div>

                  <div className="mapping-review-list">
                    {visibleMappingDrafts.map((draft) => {
                      const issue = issueForDraft(mappingIssues, draft.id);
                      const pathChecks = mappingPathChecks.filter((check) => check.draftId === draft.id);
                      return (
                        <section className={`mapping-review-card ${draft.enabled ? "is-enabled" : ""}`} key={draft.id}>
                          <div className="mapping-card-heading">
                            <div>
                              <p className="eyebrow">{draft.platformSlug}</p>
                              <h3>{draft.platformName}</h3>
                            </div>
                            <FocusButton
                              onPress={() => setMappingDrafts((current) => updateMappingEnabled(
                                current,
                                draft.id,
                                !draft.enabled,
                              ))}
                              variant="quiet"
                              selected={draft.enabled}
                              focusKey={`MAPPING-TOGGLE-${draft.id}`}
                            >
                              {draft.enabled ? "Enabled" : "Enable"}
                            </FocusButton>
                          </div>
                          {draft.enabled && (
                            <div className="mapping-card-fields">
                              <div className="mapping-path-group">
                                <div className="mapping-path-row">
                                  <FocusInput
                                    label="ROM folder"
                                    value={draft.romRoot}
                                    onChange={(value) => setMappingDrafts((current) => updateMappingPath(current, draft.id, "romRoot", value))}
                                    placeholder="Absolute existing directory"
                                    inputMode="text"
                                    focusKey={`MAPPING-ROM-${draft.id}`}
                                    onOpenKeyboard={() => setKeyboard({
                                      focusKey: `MAPPING-ROM-${draft.id}`,
                                      label: `${draft.platformName} ROM folder`,
                                      value: draft.romRoot,
                                      mode: "text",
                                      commit: (value) => setMappingDrafts((current) => updateMappingPath(current, draft.id, "romRoot", value)),
                                    })}
                                  />
                                  <FocusButton
                                    onPress={() => openDirectoryBrowser({
                                      draftId: draft.id,
                                      platformName: draft.platformName,
                                      field: "romRoot",
                                      index: 0,
                                      returnFocusKey: `MAPPING-ROM-BROWSE-${draft.id}`,
                                    }, draft.romRoot)}
                                    variant="quiet"
                                    focusKey={`MAPPING-ROM-BROWSE-${draft.id}`}
                                  >
                                    Browse
                                  </FocusButton>
                                </div>
                              </div>
                              <div className="mapping-path-group">
                                {(draft.saveRoots.length > 0 ? draft.saveRoots : [""]).map((path, index) => (
                                  <div className="mapping-path-row" key={`save-${index}`}>
                                    <FocusInput
                                      label={`Save folder ${index + 1} (optional)`}
                                      value={path}
                                      onChange={(value) => setMappingDrafts((current) => updateMappingPathAt(current, draft.id, "saveRoots", index, value))}
                                      placeholder="Absolute existing directory"
                                      inputMode="text"
                                      focusKey={`MAPPING-SAVE-${draft.id}-${index}`}
                                      onOpenKeyboard={() => setKeyboard({
                                        focusKey: `MAPPING-SAVE-${draft.id}-${index}`,
                                        label: `${draft.platformName} save folder ${index + 1}`,
                                        value: path,
                                        mode: "text",
                                        commit: (value) => setMappingDrafts((current) => updateMappingPathAt(current, draft.id, "saveRoots", index, value)),
                                      })}
                                    />
                                    <FocusButton
                                      onPress={() => openDirectoryBrowser({
                                        draftId: draft.id,
                                        platformName: draft.platformName,
                                        field: "saveRoots",
                                        index,
                                        returnFocusKey: `MAPPING-SAVE-BROWSE-${draft.id}-${index}`,
                                      }, path)}
                                      variant="quiet"
                                      focusKey={`MAPPING-SAVE-BROWSE-${draft.id}-${index}`}
                                    >
                                      Browse
                                    </FocusButton>
                                    {draft.saveRoots.length > 0 && (
                                      <FocusButton
                                        onPress={() => setMappingDrafts((current) => removeMappingPath(current, draft.id, "saveRoots", index))}
                                        variant="quiet"
                                        focusKey={`MAPPING-SAVE-REMOVE-${draft.id}-${index}`}
                                        ariaLabel={`Remove ${draft.platformName} save folder ${index + 1}`}
                                      >
                                        Remove
                                      </FocusButton>
                                    )}
                                  </div>
                                ))}
                                <FocusButton
                                  onPress={() => {
                                    const index = draft.saveRoots.length;
                                    if (index > 0) setMappingDrafts((current) => addMappingPath(current, draft.id, "saveRoots"));
                                    window.requestAnimationFrame(() => setFocus(`MAPPING-SAVE-${draft.id}-${index}`));
                                  }}
                                  variant="quiet"
                                  focusKey={`MAPPING-SAVE-ADD-${draft.id}`}
                                >
                                  Add save folder
                                </FocusButton>
                              </div>
                              <div className="mapping-path-group">
                                {(draft.stateRoots.length > 0 ? draft.stateRoots : [""]).map((path, index) => (
                                  <div className="mapping-path-row" key={`state-${index}`}>
                                    <FocusInput
                                      label={`State folder ${index + 1} (optional)`}
                                      value={path}
                                      onChange={(value) => setMappingDrafts((current) => updateMappingPathAt(current, draft.id, "stateRoots", index, value))}
                                      placeholder="Absolute existing directory"
                                      inputMode="text"
                                      focusKey={`MAPPING-STATE-${draft.id}-${index}`}
                                      onOpenKeyboard={() => setKeyboard({
                                        focusKey: `MAPPING-STATE-${draft.id}-${index}`,
                                        label: `${draft.platformName} state folder ${index + 1}`,
                                        value: path,
                                        mode: "text",
                                        commit: (value) => setMappingDrafts((current) => updateMappingPathAt(current, draft.id, "stateRoots", index, value)),
                                      })}
                                    />
                                    <FocusButton
                                      onPress={() => openDirectoryBrowser({
                                        draftId: draft.id,
                                        platformName: draft.platformName,
                                        field: "stateRoots",
                                        index,
                                        returnFocusKey: `MAPPING-STATE-BROWSE-${draft.id}-${index}`,
                                      }, path)}
                                      variant="quiet"
                                      focusKey={`MAPPING-STATE-BROWSE-${draft.id}-${index}`}
                                    >
                                      Browse
                                    </FocusButton>
                                    {draft.stateRoots.length > 0 && (
                                      <FocusButton
                                        onPress={() => setMappingDrafts((current) => removeMappingPath(current, draft.id, "stateRoots", index))}
                                        variant="quiet"
                                        focusKey={`MAPPING-STATE-REMOVE-${draft.id}-${index}`}
                                        ariaLabel={`Remove ${draft.platformName} state folder ${index + 1}`}
                                      >
                                        Remove
                                      </FocusButton>
                                    )}
                                  </div>
                                ))}
                                <FocusButton
                                  onPress={() => {
                                    const index = draft.stateRoots.length;
                                    if (index > 0) setMappingDrafts((current) => addMappingPath(current, draft.id, "stateRoots"));
                                    window.requestAnimationFrame(() => setFocus(`MAPPING-STATE-${draft.id}-${index}`));
                                  }}
                                  variant="quiet"
                                  focusKey={`MAPPING-STATE-ADD-${draft.id}`}
                                >
                                  Add state folder
                                </FocusButton>
                              </div>
                              <FocusButton
                                onPress={() => setMappingDrafts((current) => updateMappingArchivePolicy(
                                  current,
                                  draft.id,
                                ))}
                                variant="secondary"
                                focusKey={`MAPPING-ARCHIVE-${draft.id}`}
                              >
                                Archive: {ARCHIVE_POLICY_LABELS[draft.archivePolicy]}
                              </FocusButton>
                            </div>
                          )}
                          {pathChecks.length > 0 && (
                            <ul className="mapping-path-health" aria-label={`${draft.platformName} folder safety`}>
                              {pathChecks.map((check) => (
                                <li className={`is-${check.status}`} key={`${check.field}-${check.path}`}>
                                  <span>{check.field === "romRoot" ? "ROM" : check.field.startsWith("saveRoots") ? "Save" : "State"}</span>
                                  <strong>{check.status === "ready" ? "Ready" : check.status === "temporarily_unavailable" ? "Storage unavailable" : check.status === "permission_denied" ? "Access denied" : "Unsafe overlap"}</strong>
                                  {check.availableBytes !== undefined && <small>{formatBytes(check.availableBytes)} free</small>}
                                  {check.removable && <small>Removable storage</small>}
                                  {check.containsSymlink && <small>Linked path → {check.canonicalPath}</small>}
                                </li>
                              ))}
                            </ul>
                          )}
                          {issue && <p className="mapping-card-issue" role="alert">{issue.message}</p>}
                        </section>
                      );
                    })}
                  </div>

                  {mappingIssues.filter((issue) => issue.draftId === null).map((issue) => (
                    <p className="error-banner" role="alert" key={issue.code}>{issue.message}</p>
                  ))}
                  {message && mappingIssues.length === 0 && <p className="error-banner" role="alert">{message}</p>}
                  <div className="mapping-final-actions">
                    <FocusButton
                      onPress={() => void saveReviewedMappings(false)}
                      variant="primary"
                      disabled={busy !== null}
                      focusKey="SAVE-MAPPINGS"
                    >
                      {busy === "mapping-save" ? "Validating and saving…" : "Save platform mappings"}
                    </FocusButton>
                    {!noPlatformsArmed ? (
                      <FocusButton
                        onPress={() => {
                          setNoPlatformsArmed(true);
                          window.requestAnimationFrame(() => setFocus("CONFIRM-NO-PLATFORMS"));
                        }}
                        variant="quiet"
                        disabled={busy !== null}
                        focusKey="NO-PLATFORMS"
                      >
                        Continue without local platforms
                      </FocusButton>
                    ) : (
                      <div className="no-platform-confirmation" role="alertdialog" aria-label="Confirm no local platforms">
                        <p>No download destination or save synchronization will be configured yet.</p>
                        <FocusButton
                          onPress={() => void saveReviewedMappings(true)}
                          variant="secondary"
                          disabled={busy !== null}
                          focusKey="CONFIRM-NO-PLATFORMS"
                        >
                          Confirm no platforms
                        </FocusButton>
                        <FocusButton
                          onPress={() => setNoPlatformsArmed(false)}
                          variant="quiet"
                          focusKey="CANCEL-NO-PLATFORMS"
                        >
                          Go back
                        </FocusButton>
                      </div>
                    )}
                  </div>
                </>
              )}
            </FocusScreen>
          ) : primaryView === "preferences" ? (
            <FocusScreen
              key="onboarding-preferences"
              scopeKey="ONBOARDING-PREFERENCES-SCREEN"
              entryFocusKey="PREF-BACKGROUND"
              className="onboarding-preferences-view"
              label="Background and update preferences"
              animatedOffset
            >
              <div className="preferences-heading">
                <div>
                  <p className="eyebrow accent">Local setup</p>
                  <h2>Choose how Companion runs.</h2>
                  <p>
                    Both choices are opt-in and can be changed later. Neither choice affects your
                    RomM connection, registered device, or platform mappings.
                  </p>
                </div>
                <OnboardingProgress state={onboardingState} fallbackCurrent="background" compact />
              </div>

              <div className="preference-choice-grid">
                <section className={`preference-choice ${backgroundStartupEnabled ? "is-selected" : ""}`}>
                  <p className="eyebrow accent">Background agent</p>
                  <h3>Start for this user at sign-in</h3>
                  <p>
                    Keeps the agent available for future downloads and save syncing. Closing to
                    the tray keeps work running; Quit completely still stops the current session.
                  </p>
                  <FocusButton
                    onPress={() => {
                      setBackgroundStartupEnabled((enabled) => !enabled);
                      setBackgroundSetupWarning(null);
                    }}
                    variant="secondary"
                    selected={backgroundStartupEnabled}
                    focusKey="PREF-BACKGROUND"
                    navigation={{ up: "TOGGLE-FULLSCREEN", right: "PREF-UPDATES", down: "SAVE-ONBOARDING-PREFERENCES" }}
                  >
                    {backgroundStartupEnabled ? "Enabled" : "Off by default"}
                  </FocusButton>
                </section>

                <section className={`preference-choice ${stableUpdateChecksEnabled ? "is-selected" : ""}`}>
                  <p className="eyebrow accent">Stable updates</p>
                  <h3>Remember update-check preference</h3>
                  <p>
                    Opt in to signed stable-channel checks when the release updater is available.
                    Updates will never install without your confirmation.
                  </p>
                  <FocusButton
                    onPress={() => setStableUpdateChecksEnabled((enabled) => !enabled)}
                    variant="secondary"
                    selected={stableUpdateChecksEnabled}
                    focusKey="PREF-UPDATES"
                    navigation={{ up: "TOGGLE-FULLSCREEN", left: "PREF-BACKGROUND", down: "SAVE-ONBOARDING-PREFERENCES" }}
                  >
                    {stableUpdateChecksEnabled ? "Enabled" : "Off by default"}
                  </FocusButton>
                </section>
              </div>

              {backgroundSetupWarning && (
                <div className="preference-warning" role="alert">
                  <strong>Background startup could not be enabled.</strong>
                  <p>{backgroundSetupWarning}</p>
                  <p>The choice was reset to off. You can retry or continue in foreground/tray mode.</p>
                </div>
              )}
              {message && !backgroundSetupWarning && <p className="error-banner" role="alert">{message}</p>}

              <div className="preferences-final-actions">
                <FocusButton
                  onPress={() => void saveOnboardingPreferences()}
                  variant="primary"
                  disabled={busy !== null}
                  focusKey="SAVE-ONBOARDING-PREFERENCES"
                  navigation={{ up: backgroundStartupEnabled ? "PREF-BACKGROUND" : "PREF-UPDATES", down: "CLOSE-TO-TRAY" }}
                >
                  {busy === "onboarding-preferences" ? "Applying choices…" : "Apply and continue"}
                </FocusButton>
              </div>
            </FocusScreen>
          ) : primaryView === "refresh" ? (
            <FocusScreen
              key="initial-refresh"
              scopeKey="INITIAL-REFRESH-SCREEN"
              entryFocusKey={busy === "initial-refresh" ? "CLOSE-TO-TRAY" : "RETRY-INITIAL-REFRESH"}
              className="initial-refresh-view"
              label="First library refresh"
              animatedOffset
            >
              <div className="initial-refresh-card">
                <motion.div
                  className="startup-mark"
                  animate={reduceMotion || busy !== "initial-refresh"
                    ? { scale: 1, opacity: 1 }
                    : { scale: [1, 1.08, 1], opacity: [0.65, 1, 0.65] }}
                  transition={reduceMotion || busy !== "initial-refresh"
                    ? { duration: 0 }
                    : { duration: 1.4, repeat: Infinity, ease: "easeInOut" }}
                  aria-hidden="true"
                >
                  R
                </motion.div>
                <p className="eyebrow accent">Final setup step</p>
                <h2>{busy === "initial-refresh" ? "Preparing your first shelf…" : "The first refresh needs attention."}</h2>
                <p>
                  Companion is loading the first {LIBRARY_PAGE_SIZE} games from RomM and saving
                  that metadata locally. Platform folders and game files are not modified.
                </p>
                <OnboardingProgress state={onboardingState} fallbackCurrent="first_refresh" compact />
                {busy === "initial-refresh" ? (
                  <div className="initial-refresh-progress" role="status" aria-live="polite">
                    <span className="loading-pulse" aria-hidden="true" />
                    Contacting RomM and committing the first page…
                  </div>
                ) : (
                  <>
                    {message && <p className="error-banner" role="alert">{message}</p>}
                    <FocusButton
                      onPress={() => void startInitialRefresh()}
                      variant="primary"
                      focusKey="RETRY-INITIAL-REFRESH"
                      navigation={{ up: "TOGGLE-FULLSCREEN", down: "CLOSE-TO-TRAY" }}
                    >
                      Retry first refresh
                    </FocusButton>
                    <p className="initial-refresh-help">
                      Your completed connection, device, mappings, and preferences are safe. You
                      can close Companion and resume this step later.
                    </p>
                  </>
                )}
              </div>
            </FocusScreen>
          ) : (
            <FocusScreen
              key="library"
              scopeKey="LIBRARY-SCREEN"
              entryFocusKey={screenEntryFocusKey ?? "REFRESH"}
              className="library-view"
              label="RomM library"
              onScroll={catalogViewActive ? handleLibraryScroll : undefined}
            >
              <div className="library-heading">
                <div>
                  <p className="eyebrow accent">Connected to {activeServerUrl}</p>
                  <h2>{libraryHeadingTitle}</h2>
                  <p className="library-progress">
                    {libraryTab === "home"
                      ? homeInitialized ? "Your RomM activity and shortcuts" : "Building your home shelves…"
                      : !catalogViewActive
                        ? `${libraryTab === "platforms" ? libraryMetadata?.platforms.length ?? 0 : libraryMetadata?.collections.length ?? 0} available`
                        : !libraryInitialized
                          ? "Restoring this view…"
                          : `${roms.length} ${romTotal === null ? "games loaded" : `of ${romTotal} games`}`}
                  </p>
                </div>
                <div className="library-actions">
                  <FocusButton
                    onPress={refreshCurrentLibrary}
                    variant="quiet"
                    disabled={busy !== null}
                    focusKey="REFRESH"
                    navigation={{
                      up: "TOGGLE-FULLSCREEN",
                      right: "LOGOUT",
                      down: libraryFirstContentFocusKey,
                    }}
                  >
                    {busy?.startsWith("library") ? "Refreshing…" : "Refresh"}
                  </FocusButton>
                  <FocusButton
                    onPress={openLogoutDialog}
                    variant="quiet"
                    disabled={busy !== null}
                    focusKey="LOGOUT"
                    navigation={{
                      up: "TOGGLE-FULLSCREEN",
                      left: "REFRESH",
                      right: "DEVICE-SETTINGS",
                      down: libraryFirstContentFocusKey,
                    }}
                  >
                    {busy === "logout" ? "Signing out…" : "Sign out"}
                  </FocusButton>
                  <FocusButton
                    onPress={openDeviceSettings}
                    variant="quiet"
                    disabled={busy !== null || !device}
                    focusKey="DEVICE-SETTINGS"
                    navigation={{
                      up: "TOGGLE-FULLSCREEN",
                      left: "LOGOUT",
                      right: "CONTROLLER-SETTINGS",
                      down: libraryFirstContentFocusKey,
                    }}
                  >
                    Device settings
                  </FocusButton>
                  <FocusButton
                    onPress={openControllerSettings}
                    variant="quiet"
                    disabled={busy !== null || settingsBusy}
                    focusKey="CONTROLLER-SETTINGS"
                    navigation={{
                      up: "TOGGLE-FULLSCREEN",
                      left: "DEVICE-SETTINGS",
                      down: libraryFirstContentFocusKey,
                    }}
                  >
                    Controller
                  </FocusButton>
                </div>
              </div>
              <nav className="library-tabs" aria-label="Library views">
                {LIBRARY_TABS.map((tab, index) => (
                  <FocusButton
                    key={tab}
                    onPress={() => selectLibraryTab(tab)}
                    variant="quiet"
                    selected={libraryTab === tab}
                    focusKey={`LIBRARY-TAB-${tab.toUpperCase()}`}
                    navigation={{
                      up: "REFRESH",
                      left: index > 0 ? `LIBRARY-TAB-${LIBRARY_TABS[index - 1].toUpperCase()}` : undefined,
                      right: index < LIBRARY_TABS.length - 1
                        ? `LIBRARY-TAB-${LIBRARY_TABS[index + 1].toUpperCase()}`
                        : undefined,
                      down: catalogViewActive
                        ? "LIBRARY-SEARCH"
                        : libraryTab === "platforms" && libraryMetadata?.platforms[0]
                          ? `PLATFORM-${libraryMetadata.platforms[0].id}`
                          : libraryTab === "collections" && libraryMetadata?.collections[0]
                            ? `COLLECTION-${libraryMetadata.collections[0].kind}-${libraryMetadata.collections[0].id}`
                            : "CLOSE-TO-TRAY",
                    }}
                  >
                    {LIBRARY_TAB_LABELS[tab]}
                  </FocusButton>
                ))}
              </nav>
              {onboardingState && !onboardingState.completedSteps.includes("first_refresh") && (
                <div className="onboarding-continuation" role="status">
                  <div>
                    <p className="eyebrow accent">Setup continues</p>
                    <strong>Next: {ONBOARDING_STEP_LABELS[onboardingState.currentStep]}</strong>
                    <p>
                      Your completed setup choices are saved. Resume the indicated step to finish
                      onboarding.
                    </p>
                  </div>
                  <OnboardingProgress
                    state={onboardingState}
                    fallbackCurrent="detection"
                    compact
                  />
                </div>
              )}
              {libraryTab === "home" && (
                <div className="home-shelves">
                  {([
                    ["recent", "Recent additions", "No recent additions were returned."],
                    ["favorites", "Favorites", "You have not favorited any games yet."],
                    ["downloaded", "Downloaded games", "No local game copies are tracked yet."],
                    ["downloads", "Active downloads", "No downloads are currently active."],
                  ] as const).map(([shelf, title, empty]) => (
                    <section className="library-shelf" key={shelf} aria-labelledby={`shelf-${shelf}`}>
                      <div className="shelf-heading">
                        <h3 id={`shelf-${shelf}`}>{title}</h3>
                        <span>{homeShelves[shelf].length}</span>
                      </div>
                      {homeShelves[shelf].length > 0 ? (
                        <div className="shelf-row">
                          {homeShelves[shelf].map((rom, index) => (
                            <RomCard
                              key={rom.id}
                              rom={rom}
                              index={index}
                              focusKey={`HOME-${shelf.toUpperCase()}-ROM-${rom.id}`}
                              onOpen={openGameContext}
                              onToggleFavorite={toggleFavoriteOptimistically}
                              favoritePending={favoritePendingIds.has(rom.id)}
                            />
                          ))}
                        </div>
                      ) : (
                        <p className="shelf-empty">{homeInitialized ? empty : "Loading shelf…"}</p>
                      )}
                    </section>
                  ))}
                </div>
              )}
              {libraryTab === "platforms" && libraryQuery.kind !== "platform" && (
                <div className="library-entity-grid" aria-label="Platforms">
                  {(libraryMetadata?.platforms ?? []).map((platform) => (
                    <FocusButton
                      key={platform.id}
                      onPress={() => openLibraryScope({ kind: "platform", id: platform.id })}
                      variant="quiet"
                      focusKey={`PLATFORM-${platform.id}`}
                    >
                      <span>{platform.name}</span>
                      <small>{platform.romCount ?? "—"} games</small>
                    </FocusButton>
                  ))}
                  {homeInitialized && (libraryMetadata?.platforms.length ?? 0) === 0 && (
                    <div className="empty-state"><h3>No platforms returned</h3><p>Refresh to try RomM again.</p></div>
                  )}
                </div>
              )}
              {libraryTab === "collections" && libraryQuery.kind !== "collection" && libraryQuery.kind !== "smart_collection" && (
                <div className="library-entity-grid" aria-label="Collections">
                  {(libraryMetadata?.collections ?? []).map((collection) => (
                    <FocusButton
                      key={`${collection.kind}-${collection.id}`}
                      onPress={() => openLibraryScope({
                        kind: collection.kind === "smart" ? "smart_collection" : "collection",
                        id: collection.id,
                      })}
                      variant="quiet"
                      focusKey={`COLLECTION-${collection.kind}-${collection.id}`}
                    >
                      <span>{collection.name}</span>
                      <small>{collection.kind === "smart" ? "Smart collection" : `${collection.romCount ?? collection.romIds.length} games`}</small>
                    </FocusButton>
                  ))}
                  {homeInitialized && (libraryMetadata?.collections.length ?? 0) === 0 && (
                    <div className="empty-state"><h3>No collections returned</h3><p>Create collections in RomM, then refresh.</p></div>
                  )}
                </div>
              )}
              {catalogViewActive && <>
              {(selectedPlatform || selectedCollection) && (
                <FocusButton
                  onPress={() => selectLibraryTab(libraryTab)}
                  variant="quiet"
                  focusKey="LIBRARY-SCOPE-BACK"
                  navigation={{ up: libraryFirstContentFocusKey, down: "LIBRARY-SEARCH" }}
                >
                  Back to {libraryTab}
                </FocusButton>
              )}
              <div className="library-search-panel">
                <FocusInput
                  label="Search games"
                  value={librarySearch}
                  onChange={setLibrarySearch}
                  placeholder="Title or platform"
                  inputMode="text"
                  focusKey="LIBRARY-SEARCH"
                  navigation={{
                    up: "REFRESH",
                    down: libraryError
                      ? "LIBRARY-RETRY"
                      : filteredRoms[0]
                        ? `ROM-${filteredRoms[0].id}`
                        : hasMoreRoms
                          ? "LOAD-MORE"
                          : hasLibraryConstraints
                            ? "LIBRARY-EMPTY-CLEAR"
                            : "CLOSE-TO-TRAY",
                  }}
                  onOpenKeyboard={() => setKeyboard({
                    focusKey: "LIBRARY-SEARCH",
                    label: "Search games",
                    value: librarySearch,
                    mode: "text",
                    commit: setLibrarySearch,
                  })}
                />
                <FocusButton
                  onPress={() => setLibraryFiltersOpen((open) => !open)}
                  variant="quiet"
                  selected={libraryFiltersOpen}
                  focusKey="LIBRARY-FILTERS"
                  navigation={{ left: "LIBRARY-SEARCH", down: libraryFiltersOpen ? "FILTER-FAVORITES" : undefined }}
                >
                  {libraryFiltersOpen ? "Hide filters" : "Filters"}
                </FocusButton>
                <span>{filteredRoms.length} visible</span>
              </div>
              {libraryFiltersOpen && (
                <div className="library-filter-panel" aria-label="Library filters">
                  {baseLibraryQueryRef.current.kind !== "platform" && (
                    <FocusButton onPress={cyclePlatformFilter} variant="quiet" focusKey="FILTER-PLATFORM">
                      Platform: {filterPlatform?.name ?? "Any"}
                    </FocusButton>
                  )}
                  {baseLibraryQueryRef.current.kind !== "collection" && baseLibraryQueryRef.current.kind !== "smart_collection" && (
                    <FocusButton onPress={cycleCollectionFilter} variant="quiet" focusKey="FILTER-COLLECTION">
                      Collection: {filterCollection?.name ?? "Any"}
                    </FocusButton>
                  )}
                  <FocusButton
                    onPress={() => setLibraryFilters((filters) => ({ ...filters, favoriteOnly: !filters.favoriteOnly }))}
                    variant="quiet"
                    selected={libraryFilters.favoriteOnly}
                    focusKey="FILTER-FAVORITES"
                  >
                    Favorites: {libraryFilters.favoriteOnly ? "Only" : "Any"}
                  </FocusButton>
                  <FocusButton
                    onPress={() => setLibraryFilters((filters) => ({ ...filters, downloadedOnly: !filters.downloadedOnly }))}
                    variant="quiet"
                    selected={libraryFilters.downloadedOnly}
                    focusKey="FILTER-DOWNLOADED"
                  >
                    Local copy: {libraryFilters.downloadedOnly ? "Downloaded" : "Any"}
                  </FocusButton>
                  <FocusButton
                    onPress={() => setLibraryFilters((filters) => ({ ...filters, sort: nextLibrarySort(filters.sort) }))}
                    variant="quiet"
                    focusKey="FILTER-SORT"
                  >
                    Sort: {LIBRARY_SORT_LABELS[libraryFilters.sort]}
                  </FocusButton>
                  <FocusButton onPress={clearLibraryFilters} variant="quiet" focusKey="FILTER-CLEAR">
                    Clear search and filters
                  </FocusButton>
                </div>
              )}
              <div className="input-legend library-input-legend">
                <span><InputHint family={inputFamily} action="search" button={activeControllerBindings.search} /> Search</span>
                <span><InputHint family={inputFamily} action="context" button={activeControllerBindings.context} /> Game actions</span>
                <span><kbd className={`input-glyph ${inputFamily}`}>{controllerButtonGlyph(inputFamily, activeControllerBindings.previousTab)}</kbd>/<kbd className={`input-glyph ${inputFamily}`}>{controllerButtonGlyph(inputFamily, activeControllerBindings.nextTab)}</kbd> Tabs</span>
              </div>
              </>}
              {restoredAgentStatus?.accountName && (
                <div className="connection-status-card">
                  <span><strong>{restoredAgentStatus.accountName}</strong> · token #{restoredAgentStatus.tokenId}</span>
                  <span>RomM {restoredAgentStatus.serverVersion ?? "5.x"}</span>
                  <span>{restoredAgentStatus.grantedScopes.length} permissions verified</span>
                  {restoredAgentStatus.pendingFavoriteCount > 0 && (
                    <span>{restoredAgentStatus.pendingFavoriteCount} favorite {restoredAgentStatus.pendingFavoriteCount === 1 ? "change" : "changes"} queued</span>
                  )}
                  {restoredAgentStatus.lastContactAtMs && (
                    <span>Last contact {new Date(restoredAgentStatus.lastContactAtMs).toLocaleString()}</span>
                  )}
                  <div className="permission-details">
                    <FocusButton
                      onPress={() => setPermissionDetailsOpen((open) => !open)}
                      variant="quiet"
                      selected={permissionDetailsOpen}
                      focusKey="PERMISSION-DISCLOSURE"
                      navigation={{
                        up: catalogViewActive ? "LIBRARY-SEARCH" : libraryFirstContentFocusKey,
                        down: libraryError
                          ? "LIBRARY-RETRY"
                          : filteredRoms[0]
                            ? `ROM-${filteredRoms[0].id}`
                            : hasMoreRoms
                              ? "LOAD-MORE"
                              : hasLibraryConstraints
                                ? "LIBRARY-EMPTY-CLEAR"
                                : "CLOSE-TO-TRAY",
                      }}
                    >
                      {permissionDetailsOpen ? "Hide permission details" : "Permission details"}
                    </FocusButton>
                    {permissionDetailsOpen && (
                      <ul>
                        {restoredAgentStatus.grantedScopes.map((scope) => <li key={scope}>{scope}</li>)}
                      </ul>
                    )}
                  </div>
                </div>
              )}
              {message && !libraryError && <p className="error-banner" role="alert">{message}</p>}
              {libraryProvenance && (
                <div className={`library-source ${libraryProvenance.source}`} role="status">
                  <strong>{librarySourceLabel}</strong>
                  <span>Updated {new Date(libraryProvenance.refreshedAtMs).toLocaleString()}</span>
                </div>
              )}
              {librarySourceNotice && (
                <p className="notice-banner" role="status">{librarySourceNotice}</p>
              )}
              {libraryError && (
                <div className="library-error-state" role="alert">
                  <div>
                    <strong>Library refresh failed</strong>
                    <p>{libraryError.message}</p>
                    {roms.length > 0 && <p>Your last available library remains visible.</p>}
                  </div>
                  <FocusButton
                    onPress={refreshCurrentLibrary}
                    variant="secondary"
                    disabled={busy !== null}
                    focusKey="LIBRARY-RETRY"
                    navigation={{
                      up: catalogViewActive ? "LIBRARY-SEARCH" : libraryFirstContentFocusKey,
                      down: filteredRoms[0] ? `ROM-${filteredRoms[0].id}` : "CLOSE-TO-TRAY",
                    }}
                  >
                    {busy ? "Retrying…" : "Retry"}
                  </FocusButton>
                </div>
              )}
              {notice && <p className="notice-banner" role="status">{notice}</p>}
              {favoriteFailure && !contextRom && (
                <div className="favorite-error-state" role="alert">
                  <div>
                    <strong>Favorite was not changed</strong>
                    <p>{favoriteFailure.error.message}</p>
                  </div>
                  <FocusButton
                    onPress={() => setFavoriteOptimistically(
                      favoriteFailure.rom,
                      favoriteFailure.desired,
                    )}
                    variant="secondary"
                    focusKey="RETRY-FAVORITE"
                  >
                    Retry
                  </FocusButton>
                </div>
              )}
              {catalogViewActive && <>
              <div className={`rom-grid ${!libraryInitialized ? "is-loading" : ""}`}>
                {filteredRoms.map((rom, index) => (
                  <RomCard
                    key={rom.id}
                    rom={rom}
                    index={index}
                    onOpen={openGameContext}
                    onToggleFavorite={toggleFavoriteOptimistically}
                    favoritePending={favoritePendingIds.has(rom.id)}
                  />
                ))}
              </div>
              {!libraryInitialized && (
                <div className="library-loading" role="status">
                  <span className="loading-pulse" aria-hidden="true" />
                  Loading your saved RomM library…
                </div>
              )}
              {hasMoreRoms && (
                <div className="load-more-panel">
                  <FocusButton
                    onPress={() => void loadRoms(false)}
                    variant="quiet"
                    disabled={busy !== null}
                    focusKey="LOAD-MORE"
                    navigation={{
                      up: filteredRoms.length > 0
                        ? `ROM-${filteredRoms.at(-1)?.id}`
                        : "LIBRARY-SEARCH",
                      down: "CLOSE-TO-TRAY",
                    }}
                  >
                    {busy === "library-more" ? "Loading more." : "Load more games"}
                  </FocusButton>
                  <p>More games load automatically as you scroll.</p>
                </div>
              )}
              {libraryInitialized && !busy && !libraryError && roms.length === 0 && (
                <div className="empty-state">
                  <h3>{hasLibraryConstraints ? "No games match these filters" : libraryQuery.kind === "active_downloads" ? "No active downloads" : libraryQuery.kind === "downloaded" ? "No downloaded games" : "No games in this view"}</h3>
                  <p>{libraryQuery.kind === "active_downloads"
                    ? "Queued and running transfers will appear here once downloads are implemented."
                    : libraryQuery.kind === "downloaded"
                      ? "Games with a tracked local copy will appear here."
                      : hasLibraryConstraints
                        ? "Try clearing the current search and filters."
                        : "RomM connected successfully, but this view is empty."}</p>
                  {hasLibraryConstraints && (
                    <FocusButton
                      onPress={clearLibraryFilters}
                      variant="quiet"
                      focusKey="LIBRARY-EMPTY-CLEAR"
                      navigation={{ up: "LIBRARY-SEARCH", down: "CLOSE-TO-TRAY" }}
                    >
                      Clear search and filters
                    </FocusButton>
                  )}
                </div>
              )}
              </>}
            </FocusScreen>
          )}
        </AnimatePresence>

        <AnimatePresence>
          {directoryBrowserTarget && (
            <motion.div
              className="dialog-backdrop"
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              onPointerDown={(event) => {
                if (event.target === event.currentTarget && !directoryBrowserBusy) {
                  closeDirectoryBrowser();
                }
              }}
            >
              <FocusDialog
                boundaryKey="DIRECTORY-BROWSER-DIALOG"
                initialFocusKey="DIRECTORY-CANCEL"
                className="settings-dialog directory-browser-dialog"
                labelledBy="directory-browser-title"
                onDismiss={closeDirectoryBrowser}
              >
                <p className="eyebrow accent">Platform mapping</p>
                <h2 id="directory-browser-title">
                  Choose {directoryBrowserTarget.platformName} {
                    directoryBrowserTarget.field === "romRoot"
                      ? "ROM folder"
                      : directoryBrowserTarget.field === "saveRoots"
                        ? "save folder"
                        : "state folder"
                  }
                </h2>
                <p>
                  Browse with the controller or mouse. Selecting a folder only updates this draft;
                  no ROM, save, or state files are changed.
                </p>
                <div className="directory-current-path" aria-live="polite">
                  <span>{directoryListing?.locations ? "Locations" : "Current folder"}</span>
                  <code>{directoryListing?.currentPath ?? "Choose a location"}</code>
                </div>
                <div className="directory-browser-toolbar">
                  {!directoryListing?.locations && (
                    <FocusButton
                      onPress={() => void browseDirectories()}
                      variant="quiet"
                      disabled={directoryBrowserBusy}
                      focusKey="DIRECTORY-LOCATIONS"
                    >
                      Locations
                    </FocusButton>
                  )}
                  {directoryListing?.parentPath && (
                    <FocusButton
                      onPress={() => void browseDirectories(directoryListing.parentPath)}
                      variant="quiet"
                      disabled={directoryBrowserBusy}
                      focusKey="DIRECTORY-UP"
                    >
                      Up one folder
                    </FocusButton>
                  )}
                  {directoryListing?.currentPath && (
                    <FocusButton
                      onPress={chooseBrowsedDirectory}
                      variant="primary"
                      disabled={directoryBrowserBusy}
                      focusKey="DIRECTORY-USE"
                    >
                      Use this folder
                    </FocusButton>
                  )}
                </div>
                {directoryBrowserBusy && (
                  <div className="library-loading compact" role="status">
                    <span className="loading-pulse" /> Reading folders…
                  </div>
                )}
                {!directoryBrowserBusy && directoryListing && (
                  <div className="directory-entry-list" aria-label="Directories">
                    {directoryListing.entries.map((entry, index) => (
                      <FocusButton
                        key={entry.path}
                        onPress={() => void browseDirectories(entry.path)}
                        variant="quiet"
                        focusKey={`DIRECTORY-ENTRY-${index}`}
                        ariaLabel={`Open ${entry.name}`}
                      >
                        <span aria-hidden="true">▸</span>
                        <span>{entry.name}</span>
                        {entry.isSymlink && <small>Linked folder</small>}
                      </FocusButton>
                    ))}
                    {directoryListing.entries.length === 0 && (
                      <p className="directory-empty">This folder contains no subfolders.</p>
                    )}
                    {directoryListing.truncated && (
                      <p className="directory-limit" role="status">
                        Showing the first 500 folders. Type a more specific path if needed.
                      </p>
                    )}
                  </div>
                )}
                {directoryBrowserError && (
                  <p className="error-banner" role="alert">{directoryBrowserError}</p>
                )}
                {directoryBrowserNotice && !directoryBrowserError && (
                  <p className="notice-banner" role="status">{directoryBrowserNotice}</p>
                )}
                {directoryCreationOpen ? (
                  <div className="directory-create-confirmation" role="alertdialog" aria-label="Confirm new folder">
                    <p>
                      Create one folder inside <code>{directoryListing?.currentPath}</code>. Parent
                      folders will never be created automatically.
                    </p>
                    <FocusInput
                      label="New folder name"
                      value={directoryName}
                      onChange={setDirectoryName}
                      placeholder="Folder name"
                      inputMode="text"
                      maxLength={128}
                      focusKey="DIRECTORY-NEW-NAME"
                      disabled={directoryBrowserBusy}
                      onOpenKeyboard={() => setKeyboard({
                        focusKey: "DIRECTORY-NEW-NAME",
                        label: "New folder name",
                        value: directoryName,
                        mode: "text",
                        maxLength: 128,
                        commit: setDirectoryName,
                      })}
                    />
                    <div className="dialog-actions">
                      <FocusButton
                        onPress={() => void createBrowsedDirectory()}
                        variant="primary"
                        disabled={directoryBrowserBusy || !directoryName.trim()}
                        focusKey="DIRECTORY-CREATE-CONFIRM"
                      >
                        {directoryBrowserBusy ? "Creating…" : "Confirm and create"}
                      </FocusButton>
                      <FocusButton
                        onPress={() => {
                          setDirectoryCreationOpen(false);
                          setDirectoryName("");
                          setDirectoryBrowserError(null);
                          window.requestAnimationFrame(() => setFocus("DIRECTORY-NEW"));
                        }}
                        variant="quiet"
                        disabled={directoryBrowserBusy}
                        focusKey="DIRECTORY-CREATE-CANCEL"
                      >
                        Cancel creation
                      </FocusButton>
                    </div>
                  </div>
                ) : directoryListing?.currentPath && (
                  <FocusButton
                    onPress={() => {
                      setDirectoryCreationOpen(true);
                      setDirectoryBrowserError(null);
                      setDirectoryBrowserNotice(null);
                      window.requestAnimationFrame(() => setFocus("DIRECTORY-NEW-NAME"));
                    }}
                    variant="secondary"
                    disabled={directoryBrowserBusy}
                    focusKey="DIRECTORY-NEW"
                  >
                    Create a folder here…
                  </FocusButton>
                )}
                <div className="dialog-actions">
                  {directoryBrowserError && (
                    <FocusButton
                      onPress={() => void browseDirectories(directoryListing?.currentPath)}
                      variant="secondary"
                      disabled={directoryBrowserBusy}
                      focusKey="DIRECTORY-RETRY"
                    >
                      Retry
                    </FocusButton>
                  )}
                  <FocusButton
                    onPress={closeDirectoryBrowser}
                    variant="quiet"
                    disabled={directoryBrowserBusy}
                    focusKey="DIRECTORY-CANCEL"
                  >
                    Cancel
                  </FocusButton>
                </div>
              </FocusDialog>
            </motion.div>
          )}
        </AnimatePresence>

        <AnimatePresence>
          {deviceSettingsOpen && device && (
            <motion.div
              className="dialog-backdrop"
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              onPointerDown={(event) => {
                if (event.target === event.currentTarget) closeDeviceSettings();
              }}
            >
              <FocusDialog
                boundaryKey="DEVICE-SETTINGS-DIALOG"
                initialFocusKey="SETTINGS-DEVICE-NAME"
                className="settings-dialog"
                labelledBy="device-settings-title"
                onDismiss={closeDeviceSettings}
              >
                <p className="eyebrow accent">Device settings</p>
                <h2 id="device-settings-title">This installation</h2>
                <p>
                  Renames save automatically after five seconds without changes. Closing this
                  panel saves a valid pending name immediately.
                </p>
                <FocusInput
                  label="Device name"
                  value={settingsDeviceName}
                  onChange={scheduleDeviceRename}
                  placeholder="Living Room PC"
                  inputMode="text"
                  maxLength={DEVICE_DISPLAY_NAME_MAX_LENGTH}
                  focusKey="SETTINGS-DEVICE-NAME"
                  disabled={busy === "device-update"}
                  invalid={Boolean(validateDeviceDisplayName(settingsDeviceName))}
                  ariaDescribedBy={validateDeviceDisplayName(settingsDeviceName) ? "settings-device-name-error" : undefined}
                  onOpenKeyboard={() => setKeyboard({
                    focusKey: "SETTINGS-DEVICE-NAME",
                    label: "Device name",
                    value: settingsDeviceName,
                    mode: "text",
                    maxLength: DEVICE_DISPLAY_NAME_MAX_LENGTH,
                    validate: validateDeviceDisplayName,
                    commit: scheduleDeviceRename,
                  })}
                />
                {validateDeviceDisplayName(settingsDeviceName) && (
                  <p id="settings-device-name-error" className="field-error" role="alert">
                    {validateDeviceDisplayName(settingsDeviceName)}
                  </p>
                )}
                <p className={`save-status ${deviceRenameStatus}`} aria-live="polite">
                  {deviceRenameStatus === "pending" && "Waiting for changes to finish…"}
                  {deviceRenameStatus === "saving" && "Saving to RomM…"}
                  {deviceRenameStatus === "saved" && "Saved to RomM."}
                  {deviceRenameStatus === "error" && "The device name has not been saved."}
                  {deviceRenameStatus === "idle" && "Device name is up to date."}
                </p>
                <dl className="device-facts">
                  <div><dt>Status</dt><dd>Registered</dd></div>
                  <div><dt>RomM device ID</dt><dd>{device.rommDeviceId}</dd></div>
                  <div><dt>Platform</dt><dd>{devicePlatformLabel(device.platform)}</dd></div>
                  <div><dt>Hostname</dt><dd>{device.hostname}</dd></div>
                  <div><dt>Sync mode</dt><dd>Bidirectional push + pull</dd></div>
                  <div><dt>Last verified</dt><dd>{device.verifiedAtMs ? new Date(device.verifiedAtMs).toLocaleString() : "Not yet"}</dd></div>
                </dl>
                {message && <p className="error-banner" role="alert">{message}</p>}
                <div className="dialog-actions">
                  <FocusButton
                    onPress={() => {
                      if (deviceRenameTimer.current !== null) {
                        window.clearTimeout(deviceRenameTimer.current);
                        deviceRenameTimer.current = null;
                      }
                      void persistDeviceRename(settingsDeviceNameRef.current);
                    }}
                    variant="primary"
                    disabled={
                      busy !== null ||
                      normalizePendingDeviceRename(settingsDeviceName, device.displayName) === null
                    }
                    focusKey="SAVE-DEVICE-NOW"
                  >
                    {busy === "device-update" ? "Saving…" : "Save now"}
                  </FocusButton>
                  <FocusButton
                    onPress={closeDeviceSettings}
                    variant="secondary"
                    disabled={busy === "device-update"}
                    focusKey="CLOSE-DEVICE-SETTINGS"
                  >
                    Done
                  </FocusButton>
                  <FocusButton
                    onPress={openLogoutDialog}
                    variant="quiet"
                    disabled={busy !== null}
                    focusKey="SETTINGS-SIGN-OUT"
                  >
                    Sign out options
                  </FocusButton>
                </div>
              </FocusDialog>
            </motion.div>
          )}
        </AnimatePresence>

        <AnimatePresence>
          {controllerSettingsOpen && (
            <motion.div
              className="dialog-backdrop"
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              onPointerDown={(event) => {
                if (event.target === event.currentTarget) closeControllerSettings();
              }}
            >
              <FocusDialog
                boundaryKey="CONTROLLER-SETTINGS-DIALOG"
                initialFocusKey={activeController ? "CONTROLLER-SCOPE-CONTROLLER" : "CONTROLLER-SCOPE-GLOBAL"}
                className="settings-dialog controller-settings-dialog"
                labelledBy="controller-settings-title"
                onDismiss={closeControllerSettings}
              >
                <p className="eyebrow accent">Input settings</p>
                <h2 id="controller-settings-title">Controller</h2>
                <div className="controller-live-status">
                  <InputFamilyIcon family={activeController?.mappingFamily ?? "generic"} />
                  <div>
                    <strong>{activeController?.name ?? "No controller connected"}</strong>
                    <span>
                      {lastControllerAction
                        ? `Last input: ${lastControllerAction.replace(/([A-Z])/g, " $1").toLowerCase()}`
                        : "Press a controller button to verify input."}
                    </span>
                  </div>
                </div>
                <div className="controller-scope-tabs" aria-label="Binding profile">
                  <FocusButton
                    onPress={() => selectControllerSettingsScope("global")}
                    variant="quiet"
                    selected={controllerSettingsScope === "global"}
                    focusKey="CONTROLLER-SCOPE-GLOBAL"
                    navigation={{ right: activeController ? "CONTROLLER-SCOPE-CONTROLLER" : "DEAD-ZONE-DOWN" }}
                  >
                    Global fallback
                  </FocusButton>
                  {activeController && (
                    <FocusButton
                      onPress={() => selectControllerSettingsScope("controller")}
                      variant="quiet"
                      selected={controllerSettingsScope === "controller"}
                      focusKey="CONTROLLER-SCOPE-CONTROLLER"
                      navigation={{ left: "CONTROLLER-SCOPE-GLOBAL", down: "DEAD-ZONE-DOWN" }}
                    >
                      This controller
                    </FocusButton>
                  )}
                </div>
                <p className="settings-help">
                  Shoulder buttons switch these tabs. Per-controller changes use the stable hardware GUID and fall back to global settings when removed.
                  Select a binding repeatedly to cycle through free buttons; unbind another action first when swapping two assignments.
                </p>
                <div className="controller-timing-grid">
                  <div>
                    <span>Stick dead zone</span>
                    <div className="stepper-row">
                      <FocusButton onPress={() => adjustControllerTiming("deadZonePercent", -5)} variant="quiet" focusKey="DEAD-ZONE-DOWN" ariaLabel="Decrease stick dead zone">−</FocusButton>
                      <strong>{controllerSettingsDraft.deadZonePercent}%</strong>
                      <FocusButton onPress={() => adjustControllerTiming("deadZonePercent", 5)} variant="quiet" focusKey="DEAD-ZONE-UP" ariaLabel="Increase stick dead zone">+</FocusButton>
                    </div>
                  </div>
                  <div>
                    <span>Initial repeat</span>
                    <div className="stepper-row">
                      <FocusButton onPress={() => adjustControllerTiming("initialRepeatDelayMs", -50)} variant="quiet" focusKey="REPEAT-DELAY-DOWN" ariaLabel="Decrease initial repeat delay">−</FocusButton>
                      <strong>{controllerSettingsDraft.initialRepeatDelayMs} ms</strong>
                      <FocusButton onPress={() => adjustControllerTiming("initialRepeatDelayMs", 50)} variant="quiet" focusKey="REPEAT-DELAY-UP" ariaLabel="Increase initial repeat delay">+</FocusButton>
                    </div>
                  </div>
                  <div>
                    <span>Repeat interval</span>
                    <div className="stepper-row">
                      <FocusButton onPress={() => adjustControllerTiming("repeatIntervalMs", -10)} variant="quiet" focusKey="REPEAT-INTERVAL-DOWN" ariaLabel="Decrease repeat interval">−</FocusButton>
                      <strong>{controllerSettingsDraft.repeatIntervalMs} ms</strong>
                      <FocusButton onPress={() => adjustControllerTiming("repeatIntervalMs", 10)} variant="quiet" focusKey="REPEAT-INTERVAL-UP" ariaLabel="Increase repeat interval">+</FocusButton>
                    </div>
                  </div>
                </div>
                <div className="binding-list">
                  {CONTROLLER_BINDING_ACTIONS.map(({ action, label }) => (
                    <div className="binding-row" key={action}>
                      <span>{label}</span>
                      <FocusButton
                        onPress={() => changeControllerBinding(action)}
                        variant="quiet"
                        focusKey={`BIND-${action.toUpperCase()}`}
                      >
                        <kbd className={`input-glyph ${controllerSettingsFamily}`}>
                          {controllerButtonGlyph(controllerSettingsFamily, draftBindings[action])}
                        </kbd>
                        {draftBindings[action]?.replaceAll("_", " ") ?? "Unbound"}
                      </FocusButton>
                    </div>
                  ))}
                </div>
                {controllerSettingsError && <p className="error-banner" role="alert">{controllerSettingsError}</p>}
                <div className="dialog-actions controller-settings-actions">
                  <FocusButton
                    onPress={() => void saveControllerSettings()}
                    variant="primary"
                    disabled={settingsBusy || controllerSettingsError !== null}
                    focusKey="SAVE-CONTROLLER-SETTINGS"
                  >
                    {settingsBusy ? "Saving…" : "Save and apply"}
                  </FocusButton>
                  <FocusButton onPress={resetControllerBindings} variant="secondary" focusKey="RESET-CONTROLLER-BINDINGS">
                    {controllerSettingsScope === "controller" ? "Use global bindings" : "Restore default bindings"}
                  </FocusButton>
                  <FocusButton onPress={closeControllerSettings} variant="quiet" disabled={settingsBusy} focusKey="CANCEL-CONTROLLER-SETTINGS">
                    Cancel
                  </FocusButton>
                </div>
              </FocusDialog>
            </motion.div>
          )}
        </AnimatePresence>

        <AnimatePresence>
          {contextRom && (
            <motion.div
              className="dialog-backdrop"
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              onPointerDown={(event) => {
                if (event.target === event.currentTarget) closeGameContext();
              }}
            >
              <FocusDialog
                boundaryKey="GAME-CONTEXT-DIALOG"
                initialFocusKey="GAME-FAVORITE"
                className="logout-dialog game-context-dialog"
                labelledBy="game-context-title"
                onDismiss={closeGameContext}
              >
                <p className="eyebrow accent">Game details</p>
                <h2 id="game-context-title">{contextRom.title}</h2>
                <p className="game-context-platform">{contextRom.platform}</p>
                <div className="game-detail-layout">
                  <div className="game-detail-cover">
                    <RomArtwork rom={gameDetails?.rom ?? contextRom} large />
                  </div>
                  <div className="game-detail-overview">
                    <p>{gameDetails?.rom.summary ?? contextRom.summary ?? "RomM has no summary for this game."}</p>
                    <dl className="game-detail-facts">
                      <div><dt>Release</dt><dd>{(gameDetails?.rom.releaseDateMs ?? contextRom.releaseDateMs) ? new Date(gameDetails?.rom.releaseDateMs ?? contextRom.releaseDateMs ?? 0).toLocaleDateString() : "Unknown"}</dd></div>
                      <div><dt>Remote size</dt><dd>{formatBytes(gameDetails?.rom.remoteSizeBytes ?? contextRom.remoteSizeBytes)}</dd></div>
                      <div><dt>Local state</dt><dd>{(gameDetails?.rom.localStatus ?? contextRom.localStatus).replaceAll("_", " ")}</dd></div>
                      <div><dt>Favorite</dt><dd>{(gameDetails?.rom.user.favorite ?? contextRom.user.favorite) ? "Yes" : "No"}</dd></div>
                      <div><dt>Destination</dt><dd>{contextMapping?.romRoot ?? "No platform mapping"}</dd></div>
                      <div><dt>Archive rule</dt><dd>{contextMapping ? ARCHIVE_POLICY_LABELS[contextMapping.archivePolicy] : "Not configured"}</dd></div>
                      {(gameDetails?.rom.localPath ?? contextRom.localPath) && (
                        <div><dt>Local file</dt><dd>{gameDetails?.rom.localPath ?? contextRom.localPath}</dd></div>
                      )}
                    </dl>
                  </div>
                </div>
                {gameDetailsLoading && <div className="library-loading compact" role="status"><span className="loading-pulse" /> Loading complete metadata…</div>}
                {gameDetailsError && <p className="error-banner" role="alert">{gameDetailsError}</p>}
                {favoriteFailure?.rom.id === contextRom.id && (
                  <div className="favorite-error-state compact" role="alert">
                    <div>
                      <strong>Favorite was not changed</strong>
                      <p>{favoriteFailure.error.message}</p>
                    </div>
                    <FocusButton
                      onPress={() => setFavoriteOptimistically(
                        favoriteFailure.rom,
                        favoriteFailure.desired,
                      )}
                      variant="secondary"
                      focusKey="RETRY-GAME-FAVORITE"
                    >
                      Retry
                    </FocusButton>
                  </div>
                )}
                {gameDetails && (
                  <div className="game-detail-sections">
                    {gameDetails.source === "cache" && (
                      <p className="notice-banner">Offline details{gameDetails.stale ? " - cached more than 24 hours ago" : " - cached recently"}</p>
                    )}
                    <section><h3>Metadata</h3><p>{[
                      ...gameDetails.genres,
                      ...gameDetails.gameModes,
                      ...gameDetails.regions,
                      ...gameDetails.languages,
                      ...gameDetails.ageRatings,
                      ...gameDetails.tags,
                    ].join(" / ") || "No genre, mode, region, or language metadata."}</p></section>
                    <section><h3>Companies and series</h3><p>{[
                      ...gameDetails.companies,
                      ...gameDetails.franchises,
                      ...gameDetails.alternativeNames,
                    ].join(" / ") || "No company or series metadata."}</p></section>
                    <section><h3>Files</h3>{gameDetails.files.length > 0 ? (
                      <ul>{gameDetails.files.map((file) => <li key={file.id}><strong>{file.name}</strong><span>{formatBytes(file.sizeBytes)}</span></li>)}</ul>
                    ) : <p>No file records returned.</p>}</section>
                    <section><h3>RomM activity</h3><p>{gameDetails.saveCount} saves / {gameDetails.stateCount} states / {gameDetails.screenshotPaths.length} screenshots{gameDetails.hasManual ? " / Manual" : ""}{gameDetails.hasSoundtrack ? " / Soundtrack" : ""}</p></section>
                    {gameDetails.collections.length > 0 && <section><h3>Collections</h3><p>{gameDetails.collections.map((collection) => collection.name).join(" / ")}</p></section>}
                    {(gameDetails.playerCount || gameDetails.averageRating) && (
                      <section><h3>Game facts</h3><p>{gameDetails.playerCount ? `${gameDetails.playerCount} players` : "Player count unknown"}{gameDetails.averageRating ? ` / Rating ${gameDetails.averageRating}` : ""}</p></section>
                    )}
                  </div>
                )}
                <div className="dialog-actions">
                  <FocusButton
                    onPress={() => {
                      const current = gameDetails?.rom ?? contextRom;
                      toggleFavoriteOptimistically(current);
                    }}
                    variant={(gameDetails?.rom.user.favorite ?? contextRom.user.favorite) ? "secondary" : "quiet"}
                    selected={gameDetails?.rom.user.favorite ?? contextRom.user.favorite}
                    focusKey="GAME-FAVORITE"
                  >
                    {favoritePendingIds.has(contextRom.id) ? "Saving · " : ""}
                    {!favoritePendingIds.has(contextRom.id) &&
                      (gameDetails?.rom.user.favoritePending ?? contextRom.user.favoritePending)
                      ? "Queued · "
                      : ""}
                    {(gameDetails?.rom.user.favorite ?? contextRom.user.favorite)
                      ? "★ Remove from favorites"
                      : "☆ Add to favorites"}
                  </FocusButton>
                  <FocusButton onPress={() => undefined} variant="quiet" disabled focusKey="GAME-DOWNLOAD-DEFERRED">
                    Download - Feature 9
                  </FocusButton>
                  <FocusButton onPress={closeGameContext} variant="primary" focusKey="CLOSE-GAME-CONTEXT">
                    Close
                  </FocusButton>
                </div>
              </FocusDialog>
            </motion.div>
          )}
        </AnimatePresence>

        <AnimatePresence>
          {logoutDialogOpen && (
            <motion.div
              className="dialog-backdrop"
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              onPointerDown={(event) => {
                if (event.target === event.currentTarget && busy === null) closeLogoutDialog();
              }}
            >
              <FocusDialog
                boundaryKey="LOGOUT-DIALOG"
                initialFocusKey="LOGOUT-ONLY"
                className="logout-dialog"
                labelledBy="logout-title"
                onDismiss={closeLogoutDialog}
              >
                <p className="eyebrow accent">Account</p>
                <h2 id="logout-title">How would you like to sign out?</h2>
                <p>
                  Normal sign out keeps this device registered in RomM, ready for your next login.
                  Your client token must still be revoked separately in RomM if it should stop working.
                </p>
                <div className="logout-choices">
                  <FocusButton
                    onPress={() => void logout(false)}
                    variant="primary"
                    disabled={busy !== null}
                    focusKey="LOGOUT-ONLY"
                  >
                    {busy === "logout" ? "Signing out…" : "Sign out only"}
                  </FocusButton>
                  {device?.rommDeviceId && (
                    <FocusButton
                      onPress={() => void logout(true)}
                      variant="secondary"
                      disabled={busy !== null}
                      focusKey="LOGOUT-REMOVE-DEVICE"
                    >
                      Remove RomM device and sign out
                    </FocusButton>
                  )}
                  <FocusButton
                    onPress={closeLogoutDialog}
                    variant="quiet"
                    disabled={busy !== null}
                    focusKey="CANCEL-LOGOUT"
                  >
                    Cancel
                  </FocusButton>
                </div>
              </FocusDialog>
            </motion.div>
          )}
        </AnimatePresence>

        {keyboard && (
          <VirtualKeyboard
            label={keyboard.label}
            initialValue={keyboard.value}
            mode={keyboard.mode}
            inputFamily={inputFamily}
            bindings={activeControllerBindings}
            maxLength={keyboard.maxLength}
            secret={keyboard.secret}
            validate={keyboard.validate}
            onCancel={() => closeKeyboard(keyboard.focusKey)}
            onSubmit={(value) => {
              keyboard.commit(value);
              closeKeyboard(keyboard.focusKey);
            }}
          />
        )}

        <footer>
          <span>Native input → Tauri → React spatial focus</span>
          <div className="footer-settings" aria-label="Window close behavior">
            <span>Close window:</span>
            <FocusButton
              onPress={() => void chooseCloseBehavior("minimizeToTray")}
              variant="quiet"
              disabled={settingsBusy}
              selected={closeBehavior === "minimizeToTray"}
              focusKey="CLOSE-TO-TRAY"
              navigation={{ up: footerReturnFocusKey, right: "CLOSE-TO-QUIT" }}
            >
              To tray
            </FocusButton>
            <FocusButton
              onPress={() => void chooseCloseBehavior("quit")}
              variant="quiet"
              disabled={settingsBusy}
              selected={closeBehavior === "quit"}
              focusKey="CLOSE-TO-QUIT"
              navigation={{ up: footerReturnFocusKey, left: "CLOSE-TO-TRAY" }}
            >
              Quit
            </FocusButton>
          </div>
        </footer>
      </main>
    </FocusContext.Provider>
  );
}
