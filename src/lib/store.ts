import { useEffect } from "react";
import { create } from "zustand";
import { api, errorText, logLine, on, type EventMap } from "./api";
import { rememberIcons } from "./art";
import type {
  Account,
  AuthEvent,
  CmStatus,
  DesktopInfo,
  FarmStatus,
  GameEntry,
  IdleGame,
  LocalSteam,
  LogLine,
  MarketProgress,
  Settings,
  UnlockStatus,
  UpdateState,
} from "./types";

export type Page = "library" | "idling" | "farming" | "achievements" | "market" | "events" | "accounts" | "settings" | "logs";
const PAGES: Page[] = ["library", "idling", "farming", "achievements", "market", "events", "accounts", "settings", "logs"];

export interface Toast {
  id: number;
  kind: "info" | "ok" | "error";
  text: string;
}

interface State {
  ready: boolean;
  fatal: string | null;
  version: string;
  dataDir: string;
  /** Where log.txt is, or null when there is none. */
  logFile: string | null;
  helperFound: boolean;
  settings: Settings | null;
  accounts: Account[];
  activeId: string | null;
  localSteam: LocalSteam | null;
  cm: Record<string, CmStatus>;
  idle: Record<string, IdleGame[]>;
  farm: Record<string, FarmStatus>;
  unlock: Record<string, UnlockStatus>;
  logs: LogLine[];
  games: Record<string, GameEntry[]>;
  gamesLoading: Record<string, boolean>;
  gamesError: Record<string, string | null>;
  page: Page;
  achAppid: number | null;
  auth: Record<string, AuthEvent>;
  marketProgress: MarketProgress | null;
  toasts: Toast[];
  desktop: DesktopInfo | null;
  update: UpdateState | null;
  /** The "keep running or quit?" question is showing. */
  closePrompt: boolean;
  /** The version whose "update available" bar was put off. */
  updateDismissed: string | null;

  init(): Promise<void>;
  go(page: Page, opts?: { achAppid?: number }): void;
  setActive(steamId: string): Promise<void>;
  refreshAccounts(): Promise<void>;
  refreshLocal(): Promise<void>;
  loadGames(steamId: string, refresh?: boolean): Promise<void>;
  saveSettings(patch: Partial<Settings>): Promise<void>;
  /** Like saveSettings, for a choice that keeps changing while it is made (a
   *  color being dragged): shown at once, written once the changes pause. */
  saveSettingsSoon(patch: Partial<Settings>): void;
  setAutostart(enabled: boolean): Promise<void>;
  checkForUpdates(): Promise<void>;
  installUpdate(): Promise<void>;
  dismissUpdate(): void;
  answerClose(action: "tray" | "quit" | "cancel", remember: boolean): Promise<void>;
  toast(text: string, kind?: Toast["kind"]): void;
  dismiss(id: number): void;
  run<T>(label: string | null, fn: () => Promise<T>): Promise<T | undefined>;
}

let toastId = 1;

/** Stable empty value for selectors: a fresh [] each read would make
 *  useSyncExternalStore see a "change" on every render and loop forever. */
export const EMPTY: never[] = [];

/** The light themes: the preset running colors come in a darker shade for them. */
export function isLightTheme(theme: string): boolean {
  return ["daylight", "paper", "frost", "contrast"].includes(theme);
}

/** A color as the color picker gives it, #rrggbb; anything else isn't one. */
export function hexColor(value: string | undefined): string | null {
  return value && /^#[0-9a-f]{6}$/i.test(value) ? value.toLowerCase() : null;
}

/** How light a #rrggbb color looks, 0 to 1 (WCAG relative luminance). */
function lightness(hex: string): number {
  const [r, g, b] = [1, 3, 5]
    .map((i) => parseInt(hex.slice(i, i + 2), 16) / 255)
    .map((c) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** Near-black or white: whichever text reads better on this color. */
export function inkOn(hex: string): string {
  const [black, white] = ["#111111", "#ffffff"];
  const contrast = (a: number, b: number) => (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
  const l = lightness(hex);
  return contrast(l, lightness(black)) >= contrast(l, lightness(white)) ? black : white;
}

function applyTheme(s: Settings) {
  const root = document.documentElement;
  root.dataset.theme = s.theme;
  root.dataset.tone = isLightTheme(s.theme) ? "light" : "dark";
  // A running color picked by hand overrides the preset ones; without a
  // usable color it falls back to the theme's own amber.
  const picked = s.accent === "custom" ? hexColor(s.accentColor) : null;
  root.dataset.accent = picked ? "custom" : s.accent === "custom" ? "amber" : s.accent;
  if (picked) {
    root.style.setProperty("--run", picked);
    root.style.setProperty("--run-ink", inkOn(picked));
  } else {
    root.style.removeProperty("--run");
    root.style.removeProperty("--run-ink");
  }
}

/** The write saveSettingsSoon is holding back, while the changes go on. */
let pendingWrite: ReturnType<typeof setTimeout> | undefined;

/** Numbers log lines in the order the window got them. */
let nextLine = 1;
/** Whether init has run (see init). */
let initStarted = false;
/** How many lines the Logs page keeps. */
const LOG_LINES = 1500;
/** The longest new lines wait when no frame is drawn to add them on. */
const LOG_HAND_OVER_MS = 500;

export const useApp = create<State>((set, get) => ({
  ready: false,
  fatal: null,
  version: "",
  dataDir: "",
  logFile: null,
  helperFound: true,
  settings: null,
  accounts: [],
  activeId: null,
  localSteam: null,
  cm: {},
  idle: {},
  farm: {},
  unlock: {},
  logs: [],
  games: {},
  gamesLoading: {},
  gamesError: {},
  page: "library",
  achAppid: null,
  auth: {},
  marketProgress: null,
  toasts: [],
  desktop: null,
  update: null,
  closePrompt: false,
  updateDismissed: null,

  async init() {
    // Once per window: React's development mode runs effects twice, and a
    // second start would listen to everything twice.
    if (initStarted) return;
    initStarted = true;
    // Everything is listened to before the first look at the app's state,
    // and the backend numbers what it sends. What arrives while the window
    // starts is held, then applied if it is newer than that look: nothing is
    // missed (an account that came online meanwhile) and nothing is applied twice.
    let snapshotSeq: number | null = null;
    const held: (() => void)[] = [];
    const seqOf = (p: unknown) =>
      typeof p === "object" && p !== null && typeof (p as { seq?: unknown }).seq === "number" ? (p as { seq: number }).seq : undefined;
    const live = <K extends keyof EventMap>(event: K, apply: (payload: EventMap[K]) => void) =>
      on(event, (payload) => {
        const run = () => {
          const seq = seqOf(payload);
          if (seq === undefined || snapshotSeq === null || seq > snapshotSeq) apply(payload);
        };
        if (snapshotSeq === null) held.push(run);
        else run();
      });

    // Lines can come in bursts. They are gathered and added once per frame,
    // so a burst copies the list once, not once per line. A window that isn't
    // on screen may draw no frames, so a timer hands them over as well, and
    // no more lines wait than the list keeps.
    let arrived: LogLine[] = [];
    let frame = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const handOver = () => {
      cancelAnimationFrame(frame);
      clearTimeout(timer);
      timer = undefined;
      const batch = arrived;
      arrived = [];
      if (batch.length) set((s) => ({ logs: [...s.logs, ...batch].slice(-LOG_LINES) }));
    };
    try {
      await Promise.all([
        live("log://line", (l) => {
          arrived.push({ ...l, n: nextLine++ });
          if (arrived.length > LOG_LINES) arrived.splice(0, arrived.length - LOG_LINES);
          if (timer !== undefined) return;
          frame = requestAnimationFrame(handOver);
          timer = setTimeout(handOver, LOG_HAND_OVER_MS);
        }),
        live("state://accounts", () => void get().refreshAccounts()),
        // Names for games listed by number only arrive after the list itself.
        live("games://updated", (steamId) => {
          if (get().games[steamId]) void get().loadGames(steamId);
        }),
        live("cm://status", (c) => set((s) => ({ cm: { ...s.cm, [c.steamId]: c } }))),
        live("idle://status", (snap) => set((s) => ({ idle: { ...s.idle, [snap.steamId]: snap.games } }))),
        live("farm://status", (f) => set((s) => ({ farm: { ...s.farm, [f.steamId]: f } }))),
        live("unlock://status", (u) => set((s) => ({ unlock: { ...s.unlock, [u.steamId]: u } }))),
        live("auth://status", (a) => set((s) => ({ auth: { ...s.auth, [a.sessionId]: a } }))),
        live("market://progress", (p) => set({ marketProgress: p.done >= p.total ? null : p })),
        live("update://state", (update) => set({ update })),
        live("window://close-requested", () => set({ closePrompt: true })),
        live("app://navigate", (page) => PAGES.includes(page as Page) && set({ page: page as Page })),
      ]);
      const b = await api.bootstrap();
      applyTheme(b.settings);
      const params = new URLSearchParams(window.location.search);
      const requested = params.get("page") as Page | null;
      const initialPage = requested && PAGES.includes(requested) ? requested : null;
      const initialAch = Number(params.get("ach")) || null;
      set({
        ready: true,
        version: b.version,
        dataDir: b.dataDir,
        logFile: b.logFile,
        helperFound: b.helperFound,
        settings: b.settings,
        accounts: b.accounts,
        activeId: b.activeAccount ?? b.accounts[0]?.steamId ?? null,
        localSteam: b.localSteam,
        cm: Object.fromEntries(b.cm.map((c) => [c.steamId, c])),
        idle: Object.fromEntries(b.idle.map((s) => [s.steamId, s.games])),
        farm: Object.fromEntries(b.farming.map((f) => [f.steamId, f])),
        unlock: Object.fromEntries(b.unlocker.map((u) => [u.steamId, u])),
        logs: b.logs.map((l) => ({ ...l, n: nextLine++ })),
        desktop: b.desktop,
        update: b.update,
        page: initialPage ?? (b.accounts.length === 0 ? "accounts" : "library"),
        achAppid: initialAch,
      });
      snapshotSeq = b.seq ?? 0;
      const early = held.length;
      held.splice(0).forEach((run) => run());
      if (early) logLine("debug", "ui", `${early} event(s) came in while the window started; the newer ones were applied`);
    } catch (e) {
      logLine("error", "ui", `IdleKit couldn't start: ${errorText(e)}`);
      set({ fatal: errorText(e) });
      return;
    }
    // Keep "is the Steam client signed in" fresh without hammering anything.
    setInterval(() => void get().refreshLocal(), 15_000);
  },

  go(page, opts) {
    if (page !== get().page) logLine("info", "action", `Open the ${page} page`);
    set({ page, ...(opts?.achAppid !== undefined ? { achAppid: opts.achAppid } : {}) });
  },

  async setActive(steamId) {
    set({ activeId: steamId });
    await api.setActiveAccount(steamId).catch(() => undefined);
  },

  async refreshAccounts() {
    const accounts = await api.accounts();
    set((s) => ({
      accounts,
      activeId: accounts.some((a) => a.steamId === s.activeId) ? s.activeId : (accounts[0]?.steamId ?? null),
    }));
  },

  async refreshLocal() {
    try {
      const localSteam = await api.detectLocalSteam();
      const prev = get().localSteam;
      set({ localSteam });
      if (prev?.activeSteamId !== localSteam.activeSteamId) await get().refreshAccounts();
    } catch {
      /* not fatal */
    }
  },

  async loadGames(steamId, refresh = false) {
    set((s) => ({ gamesLoading: { ...s.gamesLoading, [steamId]: true }, gamesError: { ...s.gamesError, [steamId]: null } }));
    try {
      const list = await api.games(steamId, refresh);
      rememberIcons(list);
      set((s) => ({ games: { ...s.games, [steamId]: list } }));
    } catch (e) {
      set((s) => ({ gamesError: { ...s.gamesError, [steamId]: errorText(e) } }));
    } finally {
      set((s) => ({ gamesLoading: { ...s.gamesLoading, [steamId]: false } }));
    }
  },

  async saveSettings(patch) {
    const cur = get().settings;
    if (!cur) return;
    const next = { ...cur, ...patch };
    set({ settings: next });
    applyTheme(next);
    // This writes every setting, any held-back change included.
    clearTimeout(pendingWrite);
    try {
      await api.saveSettings(next);
    } catch (e) {
      get().toast(errorText(e), "error");
    }
  },

  saveSettingsSoon(patch) {
    const cur = get().settings;
    if (!cur) return;
    const next = { ...cur, ...patch };
    set({ settings: next });
    applyTheme(next);
    clearTimeout(pendingWrite);
    pendingWrite = setTimeout(() => {
      const latest = get().settings;
      if (latest) api.saveSettings(latest).catch((e) => get().toast(errorText(e), "error"));
    }, 400);
  },

  async setAutostart(enabled) {
    try {
      const autostart = await api.setAutostart(enabled);
      set((s) => ({ desktop: s.desktop && { ...s.desktop, autostart } }));
    } catch (e) {
      get().toast(errorText(e), "error");
    }
  },

  async checkForUpdates() {
    try {
      set({ update: await api.updateCheck(), updateDismissed: null });
    } catch (e) {
      get().toast(errorText(e), "error");
    }
  },

  async installUpdate() {
    try {
      await api.updateInstall();
    } catch (e) {
      // A failed install is already shown wherever the update is; only say more
      // when it never got started.
      const text = errorText(e);
      if (get().update?.error !== text) get().toast(text, "error");
    }
  },

  dismissUpdate() {
    set((s) => ({ updateDismissed: s.update?.available?.version ?? null }));
  },

  async answerClose(action, remember) {
    set({ closePrompt: false });
    if (remember && action !== "cancel") await get().saveSettings({ closeAction: action });
    await api.windowCloseChoice(action).catch(() => undefined);
  },

  toast(text, kind = "info") {
    // The same message twice at once says nothing more: two loads that fail
    // for one reason show it once.
    if (get().toasts.some((t) => t.text === text && t.kind === kind)) return;
    const id = toastId++;
    set((s) => ({ toasts: [...s.toasts.slice(-3), { id, kind, text }] }));
    setTimeout(() => get().dismiss(id), kind === "error" ? 9000 : 4500);
  },

  dismiss(id) {
    set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) }));
  },

  async run(label, fn) {
    try {
      const r = await fn();
      if (label) get().toast(label, "ok");
      return r;
    } catch (e) {
      get().toast(errorText(e), "error");
      return undefined;
    }
  },
}));

export function useActiveAccount(): Account | null {
  return useApp((s) => s.accounts.find((a) => a.steamId === s.activeId) ?? null);
}

/** Load an account's games once, for pages that only need names. */
export function useEnsureGames(steamId: string | undefined) {
  const has = useApp((s) => (steamId ? !!s.games[steamId] : true));
  const loading = useApp((s) => (steamId ? !!s.gamesLoading[steamId] : false));
  const failed = useApp((s) => (steamId ? !!s.gamesError[steamId] : false));
  useEffect(() => {
    if (steamId && !has && !loading && !failed) void useApp.getState().loadGames(steamId);
  }, [steamId, has, loading, failed]);
}
