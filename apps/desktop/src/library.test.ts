import { describe, expect, it } from "vitest";
import {
  applyRomPage,
  buildDiscoveryQuery,
  createLibraryRequestGate,
  createPerRomMutationQueue,
  DEFAULT_LIBRARY_FILTERS,
  libraryProgressLabel,
  libraryQueryKey,
  libraryTabFocusKey,
  mergeRomPages,
  nextLibrarySort,
  nextLibraryTab,
  queryForLibraryTab,
  setFavoriteInHomeShelves,
  setFavoriteInRoms,
  setRomFavorite,
  shouldAutoLoadLibrary,
  type LibraryCatalogState,
} from "./library";
import type { RomPage, RomSummary } from "./types";

function rom(id: number, title: string): RomSummary {
  return {
    id,
    title,
    platform: "Test",
    artwork: [],
    collectionIds: [],
    user: {
      romId: id,
      favorite: false,
      backlogged: false,
      hidden: false,
      rating: 0,
      difficulty: 0,
      completion: 0,
    },
    localStatus: "remote_only",
  };
}

function page(offset: number, items: RomSummary[], total?: number, hasMore = true): RomPage {
  return {
    items,
    offset,
    limit: 48,
    total,
    hasMore,
    source: "live",
    refreshedAtMs: 1234,
    stale: false,
  };
}

function emptyCatalog(): LibraryCatalogState {
  return { items: [], total: null, nextOffset: 0, hasMore: false };
}

describe("library pagination", () => {
  it("maps top-level tabs to typed queries and wraps shoulder navigation", () => {
    expect(queryForLibraryTab("home")).toBeNull();
    expect(queryForLibraryTab("all")).toEqual({ kind: "all" });
    expect(queryForLibraryTab("downloaded")).toEqual({ kind: "downloaded" });
    expect(queryForLibraryTab("downloads")).toEqual({ kind: "active_downloads" });
    expect(JSON.parse(libraryQueryKey({ kind: "platform", id: 7 }))).toEqual({
      kind: "platform",
      id: 7,
      search: null,
      platformId: null,
      collectionId: null,
      collectionKind: null,
      favoriteOnly: false,
      downloadedOnly: false,
      sort: "id",
    });
    expect(nextLibraryTab("home", -1)).toBe("downloads");
    expect(nextLibraryTab("downloads", 1)).toBe("home");
    expect(nextLibraryTab("platforms", 1)).toBe("collections");
    expect(libraryTabFocusKey("downloaded")).toBe("LIBRARY-TAB-DOWNLOADED");
  });

  it("builds stable combinable discovery queries and cycles supported sorts", () => {
    const query = buildDiscoveryQuery(
      { kind: "all" },
      "  chrono trigger  ",
      {
        ...DEFAULT_LIBRARY_FILTERS,
        platformId: 7,
        collectionId: 3,
        collectionKind: "smart",
        favoriteOnly: true,
        downloadedOnly: true,
        sort: "release_date",
      },
    );
    expect(query).toEqual({
      kind: "all",
      search: "chrono trigger",
      platformId: 7,
      collectionId: 3,
      collectionKind: "smart",
      favoriteOnly: true,
      downloadedOnly: true,
      sort: "release_date",
    });
    expect(libraryQueryKey({ ...query, search: "CHRONO TRIGGER" })).toBe(libraryQueryKey(query));
    expect(nextLibrarySort("title")).toBe("recently_added");
    expect(nextLibrarySort("recently_added")).toBe("release_date");
    expect(nextLibrarySort("release_date")).toBe("title");
    expect(buildDiscoveryQuery({ kind: "platform", id: 8 }, "", {
      ...DEFAULT_LIBRARY_FILTERS,
      platformId: 7,
    }).platformId).toBeUndefined();
    expect(buildDiscoveryQuery({ kind: "smart_collection", id: 9 }, "", {
      ...DEFAULT_LIBRARY_FILTERS,
      collectionId: 3,
      collectionKind: "standard",
    })).toMatchObject({ kind: "smart_collection", id: 9, collectionId: undefined, collectionKind: undefined });
  });

  it("appends new ROMs while preserving page order", () => {
    expect(mergeRomPages(
      [rom(1, "One")],
      [rom(2, "Two")],
    ).map((rom) => rom.id)).toEqual([1, 2]);
  });

  it("deduplicates overlapping pages and keeps fresh metadata", () => {
    const merged = mergeRomPages(
      [rom(1, "Old title")],
      [rom(1, "New title")],
    );

    expect(merged).toEqual([rom(1, "New title")]);
  });

  it("loads a 107-ROM catalog in stable 48-item pages", () => {
    let catalog = applyRomPage(
      emptyCatalog(),
      page(0, Array.from({ length: 48 }, (_, index) => rom(index + 1, `Game ${index + 1}`)), 107),
      true,
    );
    expect(libraryProgressLabel(catalog.items.length, catalog.total, true)).toBe("48 of 107 games");
    expect(catalog.nextOffset).toBe(48);

    catalog = applyRomPage(
      catalog,
      page(48, Array.from({ length: 48 }, (_, index) => rom(index + 49, `Game ${index + 49}`)), undefined),
      false,
    );
    expect(catalog.total).toBe(107);
    expect(catalog.nextOffset).toBe(96);

    catalog = applyRomPage(
      catalog,
      page(96, Array.from({ length: 11 }, (_, index) => rom(index + 97, `Game ${index + 97}`)), 107, false),
      false,
    );
    expect(catalog.items).toHaveLength(107);
    expect(new Set(catalog.items.map((item) => item.id))).toHaveLength(107);
    expect(catalog.hasMore).toBe(false);
    expect(libraryProgressLabel(catalog.items.length, catalog.total, true)).toBe("107 of 107 games");
  });

  it("refresh replaces the catalog and an empty advancing page stops retry loops", () => {
    const prior: LibraryCatalogState = {
      items: [rom(1, "Old"), rom(2, "Removed")],
      total: 2,
      nextOffset: 2,
      hasMore: true,
    };
    const refreshed = applyRomPage(prior, page(0, [rom(1, "Fresh")], 1, false), true);
    expect(refreshed.items).toEqual([rom(1, "Fresh")]);
    expect(refreshed.nextOffset).toBe(1);

    const stalled = applyRomPage(refreshed, page(1, [], 2, true), false);
    expect(stalled.nextOffset).toBe(1);
    expect(stalled.hasMore).toBe(false);
  });

  it("loads near the 600px boundary and serializes frontend requests", () => {
    expect(shouldAutoLoadLibrary(
      { scrollHeight: 2_000, scrollTop: 900, clientHeight: 500 },
      true,
      false,
    )).toBe(true);
    expect(shouldAutoLoadLibrary(
      { scrollHeight: 2_000, scrollTop: 899, clientHeight: 500 },
      true,
      false,
    )).toBe(false);

    const gate = createLibraryRequestGate();
    expect(gate.tryStart()).toBe(true);
    expect(gate.tryStart()).toBe(false);
    expect(gate.isActive()).toBe(true);
    gate.finish();
    expect(gate.tryStart()).toBe(true);
  });

  it("normalizes 10,000 incrementally loaded ROMs within the two-second budget", () => {
    const startedAt = performance.now();
    let catalog = emptyCatalog();
    for (let offset = 0; offset < 10_000; offset += 48) {
      const count = Math.min(48, 10_000 - offset);
      catalog = applyRomPage(
        catalog,
        page(
          offset,
          Array.from({ length: count }, (_, index) => rom(offset + index + 1, `Game ${offset + index + 1}`)),
          10_000,
          offset + count < 10_000,
        ),
        offset === 0,
      );
    }
    const elapsedMs = performance.now() - startedAt;
    expect(catalog.items).toHaveLength(10_000);
    expect(catalog.nextOffset).toBe(10_000);
    expect(catalog.hasMore).toBe(false);
    expect(elapsedMs).toBeLessThan(2_000);
  });

  it("updates favorite state immutably across duplicate ROM snapshots", () => {
    const first = rom(1, "One");
    const second = rom(2, "Two");
    const updated = setFavoriteInRoms([first, second], 1, true);

    expect(updated).not.toEqual([first, second]);
    expect(updated[0].user.favorite).toBe(true);
    expect(updated[1]).toBe(second);
    expect(first.user.favorite).toBe(false);
    expect(setRomFavorite(updated[0], true)).toBe(updated[0]);
    const queued = setRomFavorite(updated[0], true, true);
    expect(queued.user.favoritePending).toBe(true);
    expect(setFavoriteInRoms([queued], 1, false, true)[0]).toMatchObject({
      user: { favorite: false, favoritePending: true },
    });
  });

  it("adds, updates, and removes games from the home favorites shelf", () => {
    const favorite = setRomFavorite(rom(1, "Existing"), true);
    const source = rom(2, "New favorite");
    const shelves = {
      recent: [source],
      favorites: [favorite],
      downloaded: [source],
      downloads: [] as RomSummary[],
    };

    const added = setFavoriteInHomeShelves(shelves, source, true, 12);
    expect(added.favorites.map((item) => item.id)).toEqual([2, 1]);
    expect(added.recent[0].user.favorite).toBe(true);
    expect(added.downloaded[0].user.favorite).toBe(true);

    const removed = setFavoriteInHomeShelves(added, source, false, 12);
    expect(removed.favorites.map((item) => item.id)).toEqual([1]);
    expect(removed.recent[0].user.favorite).toBe(false);

    const queued = setFavoriteInHomeShelves(removed, source, true, 12, true);
    expect(queued.favorites[0].user.favoritePending).toBe(true);
  });

  it("serializes mutations for one ROM while allowing other ROMs to proceed", async () => {
    const queue = createPerRomMutationQueue();
    const events: string[] = [];
    let releaseFirst: () => void = () => undefined;
    const firstGate = new Promise<void>((resolve) => {
      releaseFirst = resolve;
    });

    const first = queue.enqueue(1, async () => {
      events.push("one:start");
      await firstGate;
      events.push("one:end");
    });
    const second = queue.enqueue(1, async () => {
      events.push("one:second");
    });
    const other = queue.enqueue(2, async () => {
      events.push("two:start");
    });

    await other;
    expect(events).toEqual(["one:start", "two:start"]);
    releaseFirst();
    await Promise.all([first, second]);
    expect(events).toEqual(["one:start", "two:start", "one:end", "one:second"]);
  });
});
