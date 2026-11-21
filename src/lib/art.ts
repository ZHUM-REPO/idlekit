// Game images: which URLs a game's icon, banner and cover can be fetched from,
// plus the batched lookup that asks the backend (Steam's public store API) for
// the paths that can't be built from an app id alone. The fetching itself, a
// few pictures at a time and kept on disk, is in pictures.ts.
//
// Three kinds of art:
//   icon   - the small square Steam shows next to a game's name
//   header - the 460x215 banner
//   cover  - the 600x900 portrait, as in the Steam library
import { create } from "zustand";
import { api } from "./api";
import type { GameArtUrls } from "./types";

export type ArtKind = "icon" | "header" | "cover";

/** `undefined`: not looked up yet. `null`: Steam couldn't be reached, so only guesses are available. */
export type ArtEntry = GameArtUrls | null | undefined;

interface ArtState {
  byApp: Record<number, GameArtUrls | null>;
}

export const useArtStore = create<ArtState>(() => ({ byApp: {} }));

/** Where looked-up assets live, and a second CDN carrying the same paths. */
const STORE_CDN = "https://shared.fastly.steamstatic.com";
const STORE_MIRROR = "https://shared.akamai.steamstatic.com";
/** The classic path, buildable from the app id. Works for most games, but not ones with hashed art. */
const LEGACY = "https://cdn.cloudflare.steamstatic.com/steam/apps";

const CHUNK = 100;
const RETRY_LOOKUP_MS = 2 * 60_000;

// ------------------------------------------------------------------ lookup

const queue: number[] = []; // in request order, so what's on screen first resolves first
const waiting = new Set<number>(); // queued or in flight
const retryAt = new Map<number, number>();
let timer: ReturnType<typeof setTimeout> | null = null;
let draining = false;

/** Ask for a game's art. Calls are collected for a moment and sent together. */
export function requestArt(appid: number) {
  if (!appid || waiting.has(appid)) return;
  const current = useArtStore.getState().byApp[appid];
  if (current) return;
  if (current === null && Date.now() < (retryAt.get(appid) ?? 0)) return;
  waiting.add(appid);
  queue.push(appid);
  if (!draining && timer === null) timer = setTimeout(() => void drain(), 30);
}

async function drain() {
  timer = null;
  draining = true;
  while (queue.length > 0) {
    const ids = queue.splice(0, CHUNK);
    let found: Record<string, GameArtUrls> = {};
    try {
      found = await api.resolveGameArt(ids);
    } catch {
      // Treated the same as Steam being unreachable: fall back to guesses.
    }
    const now = Date.now();
    const patch: Record<number, GameArtUrls | null> = {};
    for (const id of ids) {
      waiting.delete(id);
      const hit = found[id];
      patch[id] = hit ?? null;
      if (!hit) retryAt.set(id, now + RETRY_LOOKUP_MS);
    }
    useArtStore.setState((s) => ({ byApp: { ...s.byApp, ...patch } }));
  }
  draining = false;
}

// ----------------------------------------------------------------- sources

/** Icons that arrived with the games list (Steam includes them when it can). */
const ownedIcons = new Map<number, string>();

export function rememberIcons(games: { appid: number; iconUrl: string }[]) {
  for (const g of games) if (g.iconUrl) ownedIcons.set(g.appid, g.iconUrl);
}

function withMirror(url: string | null | undefined): string[] {
  if (!url) return [];
  return url.startsWith(STORE_CDN) ? [url, STORE_MIRROR + url.slice(STORE_CDN.length)] : [url];
}

/** The same icon at the address Steam has served icons from for years. */
function classicIcon(url: string | null | undefined): string | undefined {
  const m = url?.match(/\/apps\/(\d+)\/([0-9a-f]{16,})\.jpg/);
  return m ? `https://cdn.cloudflare.steamstatic.com/steamcommunity/public/images/apps/${m[1]}/${m[2]}.jpg` : undefined;
}

/** Every URL worth trying for this art, best first. The backend takes the first that works. */
export function sources(kind: ArtKind, appid: number, art: ArtEntry): string[] {
  const list =
    kind === "icon"
      ? [ownedIcons.get(appid), art?.icon, classicIcon(art?.icon)]
      : kind === "header"
        ? [
            ...withMirror(art?.header),
            `${STORE_CDN}/store_item_assets/steam/apps/${appid}/header.jpg`,
            `${LEGACY}/${appid}/header.jpg`,
          ]
        : [...withMirror(art?.cover), `${LEGACY}/${appid}/library_600x900.jpg`];
  return [...new Set(list.filter((u): u is string => !!u))];
}

// Back online: forget what failed while the connection was down and look again.
if (typeof window !== "undefined") {
  window.addEventListener("online", () => {
    retryAt.clear();
    useArtStore.setState((s) => ({
      byApp: Object.fromEntries(Object.entries(s.byApp).filter(([, v]) => v !== null)),
    }));
  });
}
