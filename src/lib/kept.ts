// What a page remembers about how it was left.
//
// Only the page being looked at exists: opening another page throws it away,
// and coming back builds it afresh. So a choice made on a page has to be kept
// somewhere else, or it is gone on the way back.
//
//   useKept  - until IdleKit closes: search text and other passing choices
//   useView  - across restarts too, saved with the settings: filters, sort, tabs
import { useCallback, useState } from "react";
import { useApp } from "./store";
import type { ViewSettings } from "./types";

const kept = new Map<string, unknown>();

/** Like useState, but the value is there again the next time the page is opened, until IdleKit closes. */
export function useKept<T>(key: string, initial: T): [T, (next: T | ((prev: T) => T)) => void] {
  const [value, setValue] = useState<T>(() => (kept.has(key) ? (kept.get(key) as T) : initial));
  const set = useCallback(
    (next: T | ((prev: T) => T)) =>
      setValue((prev) => {
        const value = typeof next === "function" ? (next as (prev: T) => T)(prev) : next;
        kept.set(key, value);
        return value;
      }),
    [key],
  );
  return [value, set];
}

/** What each saved choice may be; the first is what a page starts with. */
export const VIEW_CHOICES = {
  libraryFilter: ["all", "favorites", "idling", "unplayed", "manual"],
  librarySort: ["name", "playtime", "recent"],
  marketTab: ["inventory", "listings"],
  marketFilter: ["all", "card", "foil", "emoticon", "background", "booster", "gems", "other"],
  marketSort: [
    "none",
    "owned-asc",
    "owned-desc",
    "sell-asc",
    "sell-desc",
    "lowest-asc",
    "lowest-desc",
    "highest-asc",
    "highest-desc",
    "buyer-asc",
    "buyer-desc",
    "receive-asc",
    "receive-desc",
  ],
  logsLevel: ["all", "problems", "debug"],
} as const satisfies { [K in keyof ViewSettings]: readonly ViewSettings[K][] };

export const DEFAULT_VIEWS = Object.fromEntries(
  Object.entries(VIEW_CHOICES).map(([name, choices]) => [name, choices[0]]),
) as unknown as ViewSettings;

/** A saved choice as read from the settings: anything unknown counts as the default. */
export function readView<K extends keyof ViewSettings>(views: Partial<ViewSettings> | undefined, name: K): ViewSettings[K] {
  const saved = views?.[name];
  const choices = VIEW_CHOICES[name] as unknown as readonly ViewSettings[K][];
  return saved !== undefined && choices.includes(saved) ? saved : choices[0];
}

/** A page's choice that is saved with the settings, so it is the same after a restart. */
export function useView<K extends keyof ViewSettings>(name: K): [ViewSettings[K], (next: ViewSettings[K]) => void] {
  const value = useApp((s) => readView(s.settings?.views, name));
  const set = useCallback(
    (next: ViewSettings[K]) => {
      const { settings, saveSettings } = useApp.getState();
      if (!settings || readView(settings.views, name) === next) return;
      void saveSettings({ views: { ...DEFAULT_VIEWS, ...settings.views, [name]: next } });
    },
    [name],
  );
  return [value, set];
}
