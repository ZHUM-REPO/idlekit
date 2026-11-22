// The "Hide NSFW games" setting: which games it keeps out of the window's lists.
//
// What a game contains comes from Steam's store, with the same lookup that
// finds its pictures (art.ts), so it is known per app id and remembered on
// disk. With the setting on, a game is shown only once Steam has answered for
// it: until then it could be one to hide. Hiding is only about what the window
// shows; a hidden game that is set to idle or farm keeps doing so.
import { useEffect, useMemo } from "react";
import { requestArt, retryArt, useArtStore } from "./art";
import { useApp } from "./store";
import type { GameArtUrls } from "./types";

export interface Hiding {
  /** The setting is on. */
  on: boolean;
  /** Whether this game must stay out of sight. Always false with the setting off. */
  hides: (appid: number) => boolean;
  /** Of the games given: how many Steam hasn't answered for yet, so they aren't shown. */
  checking: number;
  /** Of the games given: how many Steam couldn't be asked about, so they aren't shown. */
  unanswered: number;
  /** Ask Steam again about the ones it couldn't be asked about. */
  retry: () => void;
}

const SHOW_ALL: Hiding = { on: false, hides: () => false, checking: 0, unanswered: 0, retry: () => {} };
const NOTHING: Record<number, GameArtUrls | null> = {};
/** How often to ask again by itself while some games are still unanswered. */
const ASK_AGAIN_MS = 2 * 60_000 + 5_000;

/** True when the answer for a game says it may be shown. */
function cleared(entry: GameArtUrls | null | undefined): boolean {
  return entry?.nsfw === false;
}

/**
 * What the setting hides. Pass the app ids a page lists (a stable array, e.g.
 * from useMemo) and Steam is asked about the ones not known yet.
 */
export function useHiding(appids?: readonly number[]): Hiding {
  const on = useApp((s) => !!s.settings?.hideNsfw);
  // With the setting off, a page doesn't redraw when lookups come in.
  const byApp = useArtStore((s) => (on ? s.byApp : NOTHING));

  useEffect(() => {
    if (!on || !appids) return;
    // Cheap for games already asked about; runs again as answers arrive, so
    // ones forgotten after a lost connection are asked about again.
    for (const id of appids) requestArt(id);
  }, [on, appids, byApp]);

  const counts = useMemo(() => {
    let checking = 0;
    let unanswered = 0;
    if (on && appids) {
      for (const id of new Set(appids)) {
        const entry = byApp[id];
        if (entry === undefined) checking++;
        else if (!cleared(entry) && entry?.nsfw !== true) unanswered++;
      }
    }
    return { checking, unanswered };
  }, [on, appids, byApp]);

  const waiting = counts.unanswered > 0;
  useEffect(() => {
    if (!on || !appids || !waiting) return;
    const id = setInterval(() => {
      for (const appid of appids) requestArt(appid);
    }, ASK_AGAIN_MS);
    return () => clearInterval(id);
  }, [on, appids, waiting]);

  return useMemo(() => {
    if (!on) return SHOW_ALL;
    return {
      on,
      hides: (appid: number) => !cleared(byApp[appid]),
      ...counts,
      retry: () => appids && retryArt(appids),
    };
  }, [on, byApp, counts, appids]);
}

/** A stable list of the app ids in `items`, for useHiding. */
export function useAppIds(items: readonly { appid: number }[] | undefined): number[] | undefined {
  return useMemo(() => items?.map((g) => g.appid), [items]);
}
