import type { LibraryQuery, LibrarySort, RomPage, RomSummary } from "./types";

export const LIBRARY_PAGE_SIZE = 48;
export const AUTO_LOAD_THRESHOLD_PX = 600;
export const HOME_SHELF_SIZE = 12;

export type LibraryTab = "home" | "all" | "platforms" | "collections" | "downloaded" | "downloads";

export const LIBRARY_TABS: readonly LibraryTab[] = [
  "home",
  "all",
  "platforms",
  "collections",
  "downloaded",
  "downloads",
];

export const LIBRARY_TAB_LABELS: Record<LibraryTab, string> = {
  home: "Home",
  all: "All games",
  platforms: "Platforms",
  collections: "Collections",
  downloaded: "Downloaded",
  downloads: "Downloads",
};

export function queryForLibraryTab(tab: LibraryTab): LibraryQuery | null {
  switch (tab) {
    case "all": return { kind: "all" };
    case "downloaded": return { kind: "downloaded" };
    case "downloads": return { kind: "active_downloads" };
    case "home":
    case "platforms":
    case "collections":
      return null;
  }
}

export function libraryQueryKey(query: LibraryQuery): string {
  return JSON.stringify({
    kind: query.kind,
    id: query.id ?? null,
    search: query.search?.trim().toLocaleLowerCase() || null,
    platformId: query.platformId ?? null,
    collectionId: query.collectionId ?? null,
    collectionKind: query.collectionKind ?? null,
    favoriteOnly: query.favoriteOnly ?? false,
    downloadedOnly: query.downloadedOnly ?? false,
    sort: query.sort ?? "id",
  });
}

export interface LibraryDiscoveryFilters {
  platformId: number | null;
  collectionId: number | null;
  collectionKind: "standard" | "smart" | null;
  favoriteOnly: boolean;
  downloadedOnly: boolean;
  sort: LibrarySort;
}

export const DEFAULT_LIBRARY_FILTERS: LibraryDiscoveryFilters = {
  platformId: null,
  collectionId: null,
  collectionKind: null,
  favoriteOnly: false,
  downloadedOnly: false,
  sort: "title",
};

export const LIBRARY_SORT_LABELS: Record<LibrarySort, string> = {
  id: "Library order",
  title: "Title A–Z",
  recently_added: "Recently added",
  release_date: "Release date",
};

export function buildDiscoveryQuery(
  base: LibraryQuery,
  search: string,
  filters: LibraryDiscoveryFilters,
): LibraryQuery {
  const normalizedSearch = search.trim();
  return {
    ...base,
    search: normalizedSearch || undefined,
    platformId: base.kind === "platform" ? undefined : filters.platformId ?? undefined,
    collectionId: base.kind === "collection" || base.kind === "smart_collection"
      ? undefined
      : filters.collectionId ?? undefined,
    collectionKind: base.kind === "collection" || base.kind === "smart_collection"
      ? undefined
      : filters.collectionKind ?? undefined,
    favoriteOnly: filters.favoriteOnly,
    downloadedOnly: filters.downloadedOnly,
    sort: filters.sort,
  };
}

export function nextLibrarySort(sort: LibrarySort): LibrarySort {
  const sorts: LibrarySort[] = ["title", "recently_added", "release_date"];
  return sorts[(sorts.indexOf(sort) + 1) % sorts.length];
}

export function nextLibraryTab(current: LibraryTab, direction: -1 | 1): LibraryTab {
  const index = LIBRARY_TABS.indexOf(current);
  return LIBRARY_TABS[(index + direction + LIBRARY_TABS.length) % LIBRARY_TABS.length];
}

export function libraryTabFocusKey(tab: LibraryTab): string {
  return `LIBRARY-TAB-${tab.toUpperCase()}`;
}

export interface LibraryCatalogState {
  items: RomSummary[];
  total: number | null;
  nextOffset: number;
  hasMore: boolean;
  source?: RomPage["source"];
  refreshedAtMs?: number;
  stale?: boolean;
}

export interface LibraryScrollMetrics {
  scrollHeight: number;
  scrollTop: number;
  clientHeight: number;
}

export function mergeRomPages(
  current: RomSummary[],
  incoming: RomSummary[],
): RomSummary[] {
  const merged = new Map(current.map((rom) => [rom.id, rom]));
  for (const rom of incoming) {
    merged.set(rom.id, rom);
  }
  return [...merged.values()];
}

export function setRomFavorite(
  rom: RomSummary,
  favorite: boolean,
  favoritePending?: boolean,
): RomSummary {
  if (
    rom.user.favorite === favorite &&
    (favoritePending === undefined || Boolean(rom.user.favoritePending) === favoritePending)
  ) return rom;
  return {
    ...rom,
    user: {
      ...rom.user,
      favorite,
      ...(favoritePending === undefined ? {} : { favoritePending }),
    },
  };
}

export function setFavoriteInRoms(
  roms: RomSummary[],
  romId: number,
  favorite: boolean,
  favoritePending?: boolean,
): RomSummary[] {
  let changed = false;
  const next = roms.map((rom) => {
    if (rom.id !== romId) return rom;
    const updated = setRomFavorite(rom, favorite, favoritePending);
    changed ||= updated !== rom;
    return updated;
  });
  return changed ? next : roms;
}

export function setFavoriteInHomeShelves<T extends Record<string, RomSummary[]>>(
  shelves: T,
  sourceRom: RomSummary,
  favorite: boolean,
  favoriteLimit = HOME_SHELF_SIZE,
  favoritePending?: boolean,
): T {
  const updatedRom = setRomFavorite(sourceRom, favorite, favoritePending);
  const next = Object.fromEntries(
    Object.entries(shelves).map(([name, items]) => {
      if (name === "favorites") {
        const withoutRom = items.filter((rom) => rom.id !== sourceRom.id);
        return [
          name,
          favorite ? [updatedRom, ...withoutRom].slice(0, favoriteLimit) : withoutRom,
        ];
      }
      return [name, setFavoriteInRoms(items, sourceRom.id, favorite, favoritePending)];
    }),
  );
  return next as T;
}

export function applyRomPage(
  current: LibraryCatalogState,
  page: RomPage,
  reset: boolean,
): LibraryCatalogState {
  const items = reset ? page.items : mergeRomPages(current.items, page.items);
  const reportedTotal = page.total ?? (reset ? null : current.total);
  const total = reportedTotal === null ? null : Math.max(reportedTotal, items.length);
  const madeCursorProgress = page.items.length > 0;

  return {
    items,
    total,
    nextOffset: page.offset + page.items.length,
    hasMore: page.hasMore && madeCursorProgress,
    source: page.source,
    refreshedAtMs: page.refreshedAtMs,
    stale: page.stale,
  };
}

export function libraryProgressLabel(
  loaded: number,
  total: number | null,
  initialized: boolean,
): string {
  if (!initialized) return "Restoring your library.";
  return total === null ? `${loaded} games loaded` : `${loaded} of ${Math.max(total, loaded)} games`;
}

export function shouldAutoLoadLibrary(
  metrics: LibraryScrollMetrics,
  hasMore: boolean,
  requestActive: boolean,
): boolean {
  if (!hasMore || requestActive) return false;
  const remaining = metrics.scrollHeight - metrics.scrollTop - metrics.clientHeight;
  return remaining <= AUTO_LOAD_THRESHOLD_PX;
}

export interface LibraryRequestGate {
  tryStart(): boolean;
  finish(): void;
  isActive(): boolean;
}

export interface PerRomMutationQueue {
  enqueue(romId: number, operation: () => Promise<void>): Promise<void>;
}

export function createPerRomMutationQueue(): PerRomMutationQueue {
  const tails = new Map<number, Promise<void>>();
  return {
    enqueue(romId, operation) {
      const prior = tails.get(romId) ?? Promise.resolve();
      const task = prior.catch(() => undefined).then(operation);
      tails.set(romId, task);
      const clear = () => {
        if (tails.get(romId) === task) tails.delete(romId);
      };
      void task.then(clear, clear);
      return task;
    },
  };
}

export function createLibraryRequestGate(): LibraryRequestGate {
  let active = false;
  return {
    tryStart() {
      if (active) return false;
      active = true;
      return true;
    },
    finish() {
      active = false;
    },
    isActive() {
      return active;
    },
  };
}
