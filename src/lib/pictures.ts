// Pictures (covers, banners, icons, avatars) as the window shows them.
//
// Nothing is loaded straight from the internet. For each picture the window
// first asks the backend what is on disk already, which costs no network.
// What is missing is fetched by the backend and kept on disk, a few at a time:
// pictures that are on screen first, then the ones just below, and nothing
// for tiles far away until they are scrolled near.
import { convertFileSrc } from "@tauri-apps/api/core";
import { useEffect, type RefObject } from "react";
import { create } from "zustand";
import { api, inTauri } from "./api";

export type PictureKind = "cover" | "header" | "icon" | "avatar";

/** Fetches running at once. The backend enforces the same number. */
const AT_ONCE = 3;
/** A picture that couldn't be fetched is left alone for this long. */
const RETRY_MS = 5 * 60_000;
/** How far below (or above) the screen a tile may be and still be fetched, in screen heights. */
const NEAR_SCREENS = 1;

interface Entry {
  /** What to show. `undefined`: not known yet. `null`: no picture (for now). */
  src: string | null | undefined;
  /** The disk has been asked about this picture. */
  checked: boolean;
  /** On disk but old or changed at the source: shown, and fetched again. */
  stale: boolean;
  /** The source address the disk copy was compared with. */
  comparedWith: string;
}

interface State {
  byKey: Record<string, Entry>;
}

export const usePictureStore = create<State>(() => ({ byKey: {} }));

const keyOf = (kind: PictureKind, id: string) => `${kind}:${id}`;

const UNKNOWN: Entry = { src: undefined, checked: false, stale: false, comparedWith: "" };

function put(key: string, patch: Partial<Entry>) {
  usePictureStore.setState((s) => ({
    byKey: { ...s.byKey, [key]: { ...(s.byKey[key] ?? UNKNOWN), ...patch } },
  }));
}

/** A file in the app's pictures folder as an address the window can load. */
function toSrc(file: string): string {
  // The browser preview has no pictures folder: there a "file" is the address itself.
  return /^(https?:|data:|blob:)/.test(file) || !inTauri ? file : convertFileSrc(file, "pic");
}

// --------------------------------------------------------- what is on disk

const toAsk = new Map<string, { kind: PictureKind; id: string; current?: string }>();
let askTimer: ReturnType<typeof setTimeout> | null = null;

/** Asks the backend what it has on disk. Calls are collected for a moment and sent together. */
function askDisk(kind: PictureKind, id: string, current: string | undefined) {
  toAsk.set(keyOf(kind, id), { kind, id, current });
  askTimer ??= setTimeout(() => void sendAsk(), 15);
}

async function sendAsk() {
  askTimer = null;
  const items = [...toAsk.values()];
  toAsk.clear();
  let found: Record<string, { file: string; stale: boolean; none: boolean }> = {};
  try {
    found = await api.picturesCached(items);
  } catch {
    // Treated as "nothing on disk": the pictures are fetched instead.
  }
  usePictureStore.setState((s) => {
    const byKey = { ...s.byKey };
    for (const item of items) {
      const key = keyOf(item.kind, item.id);
      const hit = found[key];
      // Steam has no such picture: nothing to show, and no point asking for it now.
      if (hit?.none) failedAt.set(key, Date.now());
      byKey[key] = {
        // A picture already showing stays while it is checked again.
        src: hit?.none ? null : hit ? toSrc(hit.file) : byKey[key]?.src,
        checked: true,
        stale: hit?.stale ?? false,
        comparedWith: item.current ?? "",
      };
    }
    return { byKey };
  });
}

// ------------------------------------------------------------ the fetching

interface Job {
  key: string;
  kind: PictureKind;
  id: string;
  urls: string[];
  current?: string;
  /** The addresses are the picture's real ones, not guesses. */
  certain: boolean;
  /** The tile the picture is for: decides whose turn it is. */
  el: RefObject<Element | null>;
}

const waiting = new Map<string, Job>();
const running = new Set<string>();
const failedAt = new Map<string, number>();

/** Whether a picture that failed earlier may be tried again. */
function mayTry(key: string) {
  const at = failedAt.get(key);
  return at === undefined || Date.now() - at > RETRY_MS;
}

/**
 * The next picture to fetch: the topmost one on screen, else the nearest one
 * within a screen's height. Tiles further away wait until they are scrolled
 * near, and a tile that is gone from the page gives up its place.
 */
function nextJob(): Job | undefined {
  const screen = window.innerHeight || 800;
  let best: Job | undefined;
  let bestScore = Infinity;
  for (const job of [...waiting.values()]) {
    const el = job.el.current;
    if (!el || !el.isConnected) {
      waiting.delete(job.key);
      continue;
    }
    const box = el.getBoundingClientRect();
    if (box.width === 0 && box.height === 0) continue; // on a hidden page
    const onScreen = box.bottom > 0 && box.top < screen;
    const away = onScreen ? 0 : box.top >= screen ? box.top - screen : -box.bottom;
    if (away > screen * NEAR_SCREENS) continue;
    const score = onScreen ? Math.max(box.top, 0) : screen * 2 + away;
    if (score < bestScore) {
      bestScore = score;
      best = job;
    }
  }
  return best;
}

function pump() {
  while (running.size < AT_ONCE) {
    const job = nextJob();
    if (!job) return;
    waiting.delete(job.key);
    running.add(job.key);
    void run(job);
  }
}

async function run(job: Job) {
  let file: string | null = null;
  try {
    file = await api.pictureFetch(job.kind, job.id, job.urls, job.current ?? null, job.certain);
  } catch {
    // The same as no picture.
  }
  running.delete(job.key);
  if (file) {
    failedAt.delete(job.key);
    put(job.key, { src: toSrc(file), checked: true, stale: false });
  } else {
    failedAt.set(job.key, Date.now());
    // Whatever was showing stays; with nothing to show, say so.
    put(job.key, { src: usePictureStore.getState().byKey[job.key]?.src ?? null, stale: false });
  }
  pump();
}

let pumpQueued = false;
/** Looks again at whose turn it is, at most once a frame. */
function pumpSoon() {
  if (pumpQueued) return;
  pumpQueued = true;
  requestAnimationFrame(() => {
    pumpQueued = false;
    pump();
  });
}

if (typeof window !== "undefined") {
  // Scrolling and resizing change which tiles are on screen.
  document.addEventListener("scroll", pumpSoon, { capture: true, passive: true });
  window.addEventListener("resize", pumpSoon, { passive: true });
  // Back online: what failed while the connection was down may be tried again.
  window.addEventListener("online", () => {
    failedAt.clear();
    usePictureStore.setState((s) => ({
      byKey: Object.fromEntries(Object.entries(s.byKey).map(([k, e]) => [k, e.src === null ? { ...e, src: undefined } : e])),
    }));
  });
}

/** The picture can't be shown after all (its file is gone or unreadable). */
export function pictureBroken(kind: PictureKind, id: string | number) {
  const key = keyOf(kind, String(id));
  failedAt.set(key, Date.now());
  put(key, { src: null });
}

// ---------------------------------------------------------------- the hooks

export interface PictureStatus {
  /** What to show: an address, `null` for no picture, `undefined` while not known. */
  src: string | null | undefined;
  /** Not on disk (or old there), so it has to be fetched: sources are needed. */
  missing: boolean;
}

/**
 * What is known about a picture. Asks the disk on first use, which never
 * touches the network. `current` is the picture's address at the source when
 * known; a copy on disk from another address counts as old.
 */
export function usePictureStatus(kind: PictureKind, id: string | number, enabled = true, current?: string): PictureStatus {
  const key = keyOf(kind, String(id));
  const entry = usePictureStore((s) => s.byKey[key]);
  const upToDate = entry?.checked === true && entry.comparedWith === (current ?? "");
  useEffect(() => {
    if (enabled && !upToDate) askDisk(kind, String(id), current);
  }, [enabled, upToDate, kind, id, current]);
  if (!enabled || !entry?.checked) return { src: enabled ? entry?.src : undefined, missing: false };
  const missing = entry.src === undefined || entry.stale || (entry.src === null && mayTry(key));
  return { src: entry.src, missing };
}

/**
 * Queues a picture to be fetched from the first of `urls` that works. `el` is
 * the tile it is for. With `ready` false the sources are still being looked
 * up, so nothing happens yet; with no sources at all there is no picture.
 * `certain` false marks the sources as guesses: a guess that fails is tried
 * again another day, where a real address that fails is remembered as "none".
 */
export function useFetchPicture(
  kind: PictureKind,
  id: string | number,
  el: RefObject<Element | null>,
  urls: string[],
  { enabled, ready = true, current, certain = true }: { enabled: boolean; ready?: boolean; current?: string; certain?: boolean },
) {
  const key = keyOf(kind, String(id));
  const list = urls.join("\n");
  useEffect(() => {
    if (!enabled || !ready) return;
    const sources = list ? list.split("\n") : [];
    if (sources.length === 0) {
      if (usePictureStore.getState().byKey[key]?.src === undefined) put(key, { src: null });
      return;
    }
    if (running.has(key) || !mayTry(key)) return;
    waiting.set(key, { key, kind, id: String(id), urls: sources, current, certain, el });
    pumpSoon();
    return () => {
      // The tile is going away or wants something else: give up its place.
      if (waiting.get(key)?.el === el) waiting.delete(key);
    };
  }, [enabled, ready, key, kind, id, list, current, certain, el]);
}

/** A picture with known sources, such as an account's avatar. */
export function usePicture(kind: PictureKind, id: string | number, el: RefObject<Element | null>, urls: string[], current?: string) {
  const status = usePictureStatus(kind, id, true, current);
  useFetchPicture(kind, id, el, urls, { enabled: status.missing && urls.length > 0, current });
  return status.src;
}
