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
  /** Whether Steam labels this game as one the setting hides. Known only while labels are being read. */
  labelled: (appid: number) => boolean;
  /** Of the games given: how many Steam hasn't answered for yet, so they aren't shown. */
  checking: number;
  /** Of the games given: how many Steam couldn't be asked about, so they aren't shown. */
  unanswered: number;
  /** Ask Steam again about the ones it couldn't be asked about. */
  retry: () => void;
}

const SHOW_ALL: Hiding = { on: false, hides: () => false, labelled: () => false, checking: 0, unanswered: 0, retry: () => {} };
const NOTHING: Record<number, GameArtUrls | null> = {};
/** How often to ask again by itself while some games are still unanswered. */
const ASK_AGAIN_MS = 2 * 60_000 + 5_000;

/** True when the answer for a game says it may be shown. */
function cleared(entry: GameArtUrls | null | undefined): boolean {
  return entry?.nsfw === false;
}

/**
 * What the setting hides. Pass the app ids a page lists (a stable array, e.g.
 * from useMemo) and Steam is asked about the ones not known yet. `wanted`
 * reads the labels with the setting off too, for a page that shows which
 * games carry them.
 */
export function useHiding(appids?: readonly number[], wanted = false): Hiding {
  const on = useApp((s) => !!s.settings?.hideNsfw);
  const reading = on || wanted;
  // With nothing to read, a page doesn't redraw when lookups come in.
  const byApp = useArtStore((s) => (reading ? s.byApp : NOTHING));

  useEffect(() => {
    if (!reading || !appids) return;
    // Cheap for games already asked about; runs again as answers arrive, so
    // ones forgotten after a lost connection are asked about again.
    for (const id of appids) requestArt(id);
  }, [reading, appids, byApp]);

  const counts = useMemo(() => {
    let checking = 0;
    let unanswered = 0;
    if (reading && appids) {
      for (const id of new Set(appids)) {
        const entry = byApp[id];
        if (entry === undefined) checking++;
        else if (!cleared(entry) && entry?.nsfw !== true) unanswered++;
      }
    }
    return { checking, unanswered };
  }, [reading, appids, byApp]);

  const waiting = counts.unanswered > 0;
  useEffect(() => {
    if (!reading || !appids || !waiting) return;
    const id = setInterval(() => {
      for (const appid of appids) requestArt(appid);
    }, ASK_AGAIN_MS);
    return () => clearInterval(id);
  }, [reading, appids, waiting]);

  return useMemo(() => {
    if (!reading) return SHOW_ALL;
    return {
      on,
      hides: on ? (appid: number) => !cleared(byApp[appid]) : SHOW_ALL.hides,
      labelled: (appid: number) => byApp[appid]?.nsfw === true,
      ...counts,
      retry: () => appids && retryArt(appids),
    };
  }, [on, reading, byApp, counts, appids]);
}

/** A stable list of the app ids in `items`, for useHiding. */
export function useAppIds(items: readonly { appid: number }[] | undefined): number[] | undefined {
  return useMemo(() => items?.map((g) => g.appid), [items]);
}
