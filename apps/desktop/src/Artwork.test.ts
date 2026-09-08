import { describe, expect, it } from "vitest";
import { artworkDataUrl, artworkRequestKey } from "./Artwork";
import type { ArtworkPayload, RomSummary } from "./types";

function rom(cacheKey: string): RomSummary {
  return {
    id: 42,
    title: "Chrono Trigger",
    platform: "SNES",
    artwork: [{
      kind: "cover_small",
      remotePath: "roms/snes/chrono/cover/small.png",
      cacheKey,
    }],
    collectionIds: [],
    user: {
      romId: 42,
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

describe("artwork presentation", () => {
  it("keys UI artwork by ROM, requested size, and remote revision", () => {
    expect(artworkRequestKey(rom("revision-one"), "cover_small"))
      .not.toBe(artworkRequestKey(rom("revision-two"), "cover_small"));
    expect(artworkRequestKey(rom("revision-one"), "cover_small"))
      .not.toBe(artworkRequestKey(rom("revision-one"), "cover_large"));
  });

  it("creates a non-network data URL from the agent payload", () => {
    const payload: ArtworkPayload = {
      romId: 42,
      cacheKey: "artwork:abc",
      mimeType: "image/png",
      dataBase64: "iVBORw0KGgo=",
      source: "cache",
    };
    expect(artworkDataUrl(payload)).toBe("data:image/png;base64,iVBORw0KGgo=");
  });
});
