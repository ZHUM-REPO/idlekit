import { useEffect } from "react";
import { create } from "zustand";
import { api, errorText, logLine, on } from "./api";
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

export type Page = "library" | "idling" | "farming" | "achievements" | "market" | "accounts" | "settings" | "logs";
const PAGES: Page[] = ["library", "idling", "farming", "achievements", "market", "accounts", "settings", "logs"];

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

function applyTheme(s: Settings) {
  const root = document.documentElement;
  root.dataset.theme = s.theme;
  root.dataset.accent = s.accent;
}

/** Numbers log lines in the order the window got them. */
let nextLine = 1;

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
    try {
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
    } catch (e) {
      logLine("error", "ui", `IdleKit couldn't start: ${errorText(e)}`);
      set({ fatal: errorText(e) });
      return;
    }
    // Lines can come in bursts. They are gathered and added once per frame,
    // so a burst copies the list once, not once per line.
    let arrived: LogLine[] = [];
    on("log://line", (l) => {
      arrived.push({ ...l, n: nextLine++ });
      if (arrived.length > 1) return;
      requestAnimationFrame(() => {
        const batch = arrived;
        arrived = [];
        set((s) => ({ logs: [...s.logs, ...batch].slice(-1500) }));
      });
    });
    on("state://accounts", () => void get().refreshAccounts());
    on("cm://status", (c) => set((s) => ({ cm: { ...s.cm, [c.steamId]: c } })));
    on("idle://status", (snap) => set((s) => ({ idle: { ...s.idle, [snap.steamId]: snap.games } })));
    on("farm://status", (f) => set((s) => ({ farm: { ...s.farm, [f.steamId]: f } })));
    on("unlock://status", (u) => set((s) => ({ unlock: { ...s.unlock, [u.steamId]: u } })));
    on("auth://status", (a) => set((s) => ({ auth: { ...s.auth, [a.sessionId]: a } })));
    on("market://progress", (p) => set({ marketProgress: p.done >= p.total ? null : p }));
    on("update://state", (update) => set({ update }));
    on("window://close-requested", () => set({ closePrompt: true }));
    on("app://navigate", (page) => PAGES.includes(page as Page) && set({ page: page as Page }));
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
    try {
      await api.saveSettings(next);
    } catch (e) {
      get().toast(errorText(e), "error");
    }
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
