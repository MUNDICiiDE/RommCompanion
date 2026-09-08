import { useEffect, useMemo, useRef, useState } from "react";
import { requestAgent } from "./agent";
import type { ArtworkKind, ArtworkPayload, RomSummary } from "./types";

const artworkUrls = new Map<string, string | null>();
const artworkRequests = new Map<string, Promise<string | null>>();
const MAX_MEMORY_ARTWORKS = 160;

function rememberArtwork(key: string, url: string | null) {
  artworkUrls.delete(key);
  artworkUrls.set(key, url);
  while (artworkUrls.size > MAX_MEMORY_ARTWORKS) {
    const oldest = artworkUrls.keys().next().value;
    if (oldest === undefined) break;
    artworkUrls.delete(oldest);
  }
}

export function artworkRequestKey(rom: RomSummary, preferredKind: ArtworkKind): string {
  const revision = rom.artwork.map((item) => item.cacheKey).join("|");
  return `${rom.id}:${preferredKind}:${revision}`;
}

export function artworkDataUrl(artwork: ArtworkPayload): string {
  return `data:${artwork.mimeType};base64,${artwork.dataBase64}`;
}

async function loadArtwork(
  rom: RomSummary,
  preferredKind: ArtworkKind,
  refresh: boolean,
): Promise<string | null> {
  const key = artworkRequestKey(rom, preferredKind);
  if (!refresh && artworkUrls.has(key)) return artworkUrls.get(key) ?? null;
  if (!refresh) {
    const pending = artworkRequests.get(key);
    if (pending) return pending;
  }

  const pending = requestAgent({
    type: "getArtwork",
    romId: rom.id,
    preferredKind,
    refresh,
  }).then((response) => {
    const url = response.type === "artwork" && response.artwork
      ? artworkDataUrl(response.artwork)
      : null;
    rememberArtwork(key, url);
    return url;
  }).catch(() => {
    rememberArtwork(key, null);
    return null;
  }).finally(() => artworkRequests.delete(key));

  artworkRequests.set(key, pending);
  return pending;
}

export function RomArtwork({
  rom,
  large = false,
}: {
  rom: RomSummary;
  large?: boolean;
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  const retried = useRef(false);
  const preferredKind: ArtworkKind = large ? "cover_large" : "cover_small";
  const key = useMemo(() => artworkRequestKey(rom, preferredKind), [preferredKind, rom]);
  const canLoad = rom.artwork.some((item) => Boolean(item.remotePath));
  const [visible, setVisible] = useState(false);
  const [url, setUrl] = useState<string | null>(() => artworkUrls.get(key) ?? null);
  const [loading, setLoading] = useState(canLoad && !artworkUrls.has(key));

  useEffect(() => {
    setUrl(artworkUrls.get(key) ?? null);
    setLoading(canLoad && !artworkUrls.has(key));
    retried.current = false;
  }, [canLoad, key]);

  useEffect(() => {
    if (!canLoad || visible) return;
    const element = containerRef.current;
    if (!element || typeof IntersectionObserver === "undefined") {
      setVisible(true);
      return;
    }
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          setVisible(true);
          observer.disconnect();
        }
      },
      { rootMargin: "320px" },
    );
    observer.observe(element);
    return () => observer.disconnect();
  }, [canLoad, visible]);

  useEffect(() => {
    if (!visible || !canLoad || url) return;
    let active = true;
    setLoading(true);
    void loadArtwork(rom, preferredKind, false).then((nextUrl) => {
      if (!active) return;
      setUrl(nextUrl);
      setLoading(false);
    });
    return () => { active = false; };
  }, [canLoad, preferredKind, rom, url, visible]);

  const retryCorruptArtwork = () => {
    if (retried.current) {
      setUrl(null);
      setLoading(false);
      return;
    }
    retried.current = true;
    setLoading(true);
    void loadArtwork(rom, preferredKind, true).then((nextUrl) => {
      setUrl(nextUrl);
      setLoading(false);
    });
  };

  return (
    <div
      ref={containerRef}
      className={`cover-placeholder artwork-frame ${loading ? "is-loading" : ""}`}
      aria-label={url ? `Cover artwork for ${rom.title}` : `Artwork unavailable for ${rom.title}`}
    >
      {url ? (
        <img src={url} alt={`Cover for ${rom.title}`} onError={retryCorruptArtwork} />
      ) : (
        <span aria-hidden="true">{rom.title.slice(0, 1).toUpperCase()}</span>
      )}
    </div>
  );
}
