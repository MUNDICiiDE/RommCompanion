export type AgentRequest =
  | { type: "getStatus" }
  | { type: "getSettings" }
  | { type: "getOnboarding" }
  | { type: "updateOnboarding"; action: OnboardingNavigationAction }
  | { type: "updateSettings"; settings: AppSettings }
  | { type: "probe"; baseUrl: string; confirmHttp: boolean; caId?: string }
  | { type: "importCa"; baseUrl: string; certificate: string }
  | { type: "exchangePairingCode"; code: string }
  | { type: "setManualToken"; token: string }
  | { type: "reconnect" }
  | { type: "proposeDevice" }
  | { type: "registerDevice"; displayName: string }
  | { type: "updateDevice"; displayName: string }
  | { type: "verifyDevice" }
  | { type: "detectMappings" }
  | { type: "getMappingDrafts" }
  | { type: "saveMappingDrafts"; drafts: PlatformMappingDraft[] }
  | { type: "validateMappings"; drafts: PlatformMappingDraft[]; noPlatforms: boolean }
  | { type: "saveMappings"; drafts: PlatformMappingDraft[]; noPlatforms: boolean }
  | {
      type: "configureOnboardingPreferences";
      backgroundEnabled: boolean;
      stableUpdateChecksEnabled: boolean;
    }
  | { type: "startInitialRefresh" }
  | { type: "listRoms"; limit: number; offset: number }
  | { type: "listLibrary"; query: LibraryQuery; limit: number; offset: number }
  | { type: "getGameDetails"; romId: number }
  | { type: "getLibraryMetadata" }
  | { type: "logout"; removeDevice: boolean }
  | { type: "shutdown" };

export interface AppError {
  code: string;
  message: string;
  retryable: boolean;
  field?: string;
  details?: unknown;
  causeCode?: string;
}

export interface AgentStatus {
  version: string;
  ipcSchemaVersion: number;
  connection:
    | "unconfigured"
    | "probing"
    | "pairing"
    | "connected"
    | "offline"
    | "unauthorized"
    | "incompatible"
    | "tls_error"
    | "scope_error";
  serverUrl?: string;
  serverOrigin?: string;
  serverVersion?: string;
  accountId?: number;
  accountName?: string;
  lastContactAtMs?: number;
  grantedScopes: string[];
  missingScopes: string[];
  tokenId?: number;
  credentialConfigured: boolean;
  caId?: string;
  httpApproved: boolean;
  device: DeviceIdentity | null;
}

export type OnboardingStep =
  | "server"
  | "authentication"
  | "permissions"
  | "device"
  | "detection"
  | "mappings"
  | "background"
  | "first_refresh";

export type OnboardingNavigationAction = "back" | "cancel" | "resume";

export interface OnboardingState {
  version: number;
  currentStep: OnboardingStep;
  highestCompletedStep: OnboardingStep | null;
  completedSteps: OnboardingStep[];
  serverOrigin: string | null;
  acknowledgedHttpWarningOrigin: string | null;
  selectedPlatformIds: number[];
  mappingDraftIds: string[];
  customPathDrafts: Record<string, string>;
  backgroundEnabled: boolean | null;
  cancelled: boolean;
}

export interface DeviceIdentity {
  localId: string;
  rommDeviceId: string | null;
  displayName: string;
  platform: "windows" | "steamos" | "linux";
  hostname: string;
  client: string;
  clientVersion: string;
  syncMode: "push_pull";
  registrationFingerprint: string | null;
  registrationState: "unregistered" | "registered" | "missing" | "permission_error";
  mappingSummary: Record<string, unknown>;
  createdAtMs: number;
  registeredAtMs: number | null;
  verifiedAtMs: number | null;
  updatedAtMs: number;
}

export type ArchivePolicy = "keep" | "extract_keep" | "extract_delete";
export type MappingSource =
  | "unconfigured"
  | "emu_deck_internal"
  | "emu_deck_removable"
  | "custom";

export interface PlatformSummary {
  id: number;
  name: string;
  slug: string;
}

export interface DetectionEvidence {
  id: string;
  label: string;
  path: string;
  source: MappingSource;
  confidence: number;
}

export interface PlatformMappingDraft {
  id: string;
  platformId: number;
  platformName: string;
  platformSlug: string;
  enabled: boolean;
  romRoot: string;
  saveRoots: string[];
  stateRoots: string[];
  archivePolicy: ArchivePolicy;
  filenameStrategy: string;
  source: MappingSource;
  presetId: string | null;
  presetVersion: number | null;
  customFields: Record<string, boolean>;
}

export interface MappingValidationIssue {
  draftId: string | null;
  field: string | null;
  code: string;
  message: string;
}

export interface MappingValidationResult {
  valid: boolean;
  issues: MappingValidationIssue[];
}

export interface MappingDetectionResult {
  platforms: PlatformSummary[];
  drafts: PlatformMappingDraft[];
  evidence: DetectionEvidence[];
  detectedCount: number;
}

export interface ProbeResult {
  normalizedUrl: string;
  serverVersion: string;
  compatible: boolean;
}

export interface AppSettings {
  closeBehavior: "minimizeToTray" | "quit";
  fullscreen: boolean;
  stableUpdateChecksEnabled: boolean;
  controller: ControllerSettings;
}

export type ControllerButton =
  | "south"
  | "east"
  | "west"
  | "north"
  | "left_shoulder"
  | "right_shoulder";

export interface ControllerBindings {
  confirm?: ControllerButton;
  back?: ControllerButton;
  context?: ControllerButton;
  search?: ControllerButton;
  previousTab?: ControllerButton;
  nextTab?: ControllerButton;
}

export interface ControllerSettings {
  deadZonePercent: number;
  initialRepeatDelayMs: number;
  repeatIntervalMs: number;
  globalBindings: ControllerBindings;
  controllerBindings: Record<string, ControllerBindings>;
}

export interface RomSummary {
  id: number;
  title: string;
  platform: string;
  platformId?: number;
  summary?: string;
  releaseDateMs?: number;
  artwork: ArtworkReference[];
  collectionIds: number[];
  user: UserRomState;
  remoteFilename?: string;
  remoteSizeBytes?: number;
  localStatus: LocalGameStatus;
  localPath?: string;
  activeDownloadId?: string;
  metadataUpdatedAt?: string;
}

export type ArtworkKind = "cover_small" | "cover_large" | "remote_cover";
export type LocalGameStatus = "remote_only" | "downloaded" | "missing_local";
export type LibrarySource = "live" | "cache";
export type LibraryViewKind =
  | "all"
  | "recent"
  | "favorites"
  | "platform"
  | "collection"
  | "smart_collection"
  | "downloaded"
  | "active_downloads";

export interface LibraryQuery {
  kind: LibraryViewKind;
  id?: number;
  search?: string;
  platformId?: number;
  collectionId?: number;
  collectionKind?: "standard" | "smart";
  favoriteOnly?: boolean;
  downloadedOnly?: boolean;
  sort?: LibrarySort;
}
export type LibrarySort = "id" | "title" | "recently_added" | "release_date";

export interface ArtworkReference {
  kind: ArtworkKind;
  remotePath?: string;
  remoteUrl?: string;
  cacheKey: string;
}

export interface UserRomState {
  romId: number;
  favorite: boolean;
  backlogged: boolean;
  hidden: boolean;
  rating: number;
  difficulty: number;
  completion: number;
  status?: string;
  lastPlayed?: string;
  updatedAt?: string;
}

export interface LibraryPlatform {
  id: number;
  name: string;
  slug: string;
  romCount?: number;
}

export interface LibraryCollection {
  id: number;
  name: string;
  kind: "standard" | "smart";
  romIds: number[];
  romCount?: number;
  updatedAt?: string;
}

export interface LibraryMetadata {
  platforms: LibraryPlatform[];
  collections: LibraryCollection[];
  source: LibrarySource;
  refreshedAtMs: number;
  stale: boolean;
}

export interface RomPage {
  items: RomSummary[];
  offset: number;
  limit: number;
  total?: number;
  hasMore: boolean;
  source: LibrarySource;
  refreshedAtMs: number;
  stale: boolean;
}

export interface GameDetails {
  rom: RomSummary;
  alternativeNames: string[];
  genres: string[];
  franchises: string[];
  companies: string[];
  gameModes: string[];
  ageRatings: string[];
  playerCount?: string;
  averageRating?: string;
  regions: string[];
  languages: string[];
  tags: string[];
  files: GameFile[];
  collections: GameCollection[];
  siblings: GameSibling[];
  screenshotPaths: string[];
  saveCount: number;
  stateCount: number;
  hasManual: boolean;
  hasSoundtrack: boolean;
  source: LibrarySource;
  refreshedAtMs: number;
  stale: boolean;
}

export interface GameFile {
  id: number;
  name: string;
  sizeBytes: number;
  category?: string;
  crcHash?: string;
  sha1Hash?: string;
}

export interface GameCollection {
  id: number;
  name: string;
  kind: "standard" | "smart";
}

export interface GameSibling {
  id: number;
  name: string;
  isMain: boolean;
}

export type AgentResponse =
  | { type: "status"; status: AgentStatus }
  | { type: "settings"; settings?: AppSettings }
  | { type: "onboarding"; state: OnboardingState }
  | { type: "probe"; result: ProbeResult }
  | {
      type: "caImported";
      result: { caId: string; fingerprint: string; serverOrigin: string };
    }
  | {
      type: "authenticated";
      result: {
        serverUrl: string;
        tokenKind: string;
        tokenId: number;
        accountId: number;
        accountName: string;
        grantedScopes: string[];
        credentialPersisted: boolean;
        warning?: string;
      };
    }
  | { type: "deviceProposed"; device: DeviceIdentity }
  | { type: "deviceRegistered"; device: DeviceIdentity; newlyRegistered: boolean }
  | { type: "deviceUpdated"; device: DeviceIdentity }
  | {
      type: "deviceVerification";
      device: DeviceIdentity;
      outcome: "verified" | "missing" | "permission_denied";
    }
  | { type: "mappingDetection"; result: MappingDetectionResult }
  | { type: "mappingDrafts"; drafts: PlatformMappingDraft[] }
  | { type: "mappingValidation"; result: MappingValidationResult }
  | {
      type: "mappingsSaved";
      mappings: PlatformMappingDraft[];
      noPlatforms: boolean;
      onboarding: OnboardingState;
    }
  | {
      type: "onboardingPreferences";
      result: {
        requestedBackground: boolean;
        backgroundEnabled: boolean;
        stableUpdateChecksEnabled: boolean;
        registrationMethod: string;
        warning?: string;
        onboarding: OnboardingState;
      };
    }
  | {
      type: "initialRefresh";
      result: {
        page: RomPage;
        onboarding: OnboardingState;
      };
    }
  | { type: "roms"; page: RomPage }
  | { type: "libraryPage"; query: LibraryQuery; page: RomPage }
  | { type: "gameDetails"; details: GameDetails }
  | { type: "libraryMetadata"; metadata: LibraryMetadata }
  | {
      type: "loggedOut";
      deviceRemoval: {
        outcome: "not_requested" | "removed" | "already_missing" | "failed";
        deviceId?: string;
        error?: AppError;
      };
    }
  | { type: "shuttingDown" }
  | { type: "error"; error: AppError };

export interface ResponseEnvelope {
  schemaVersion: number;
  requestId: string;
  body: AgentResponse;
}

export interface ControllerAction {
  action:
    | "up"
    | "down"
    | "left"
    | "right"
    | "confirm"
    | "back"
    | "search"
    | "context"
    | "previousTab"
    | "nextTab";
  phase: "pressed" | "repeated" | "released";
  controllerId: string;
  controllerName: string;
  mappingFamily: ControllerMappingFamily;
  timestampMs: number;
}

export type ControllerMappingFamily =
  | "xbox"
  | "play_station"
  | "nintendo"
  | "steam"
  | "generic";

export interface ControllerInfo {
  id: string;
  guid: string;
  name: string;
  mappingFamily: ControllerMappingFamily;
}

export interface ControllerStatus {
  controllers: ControllerInfo[];
  activeControllerId?: string;
  change: "initialized" | "connected" | "disconnected" | "active_changed";
  changedController?: ControllerInfo;
  timestampMs: number;
}
