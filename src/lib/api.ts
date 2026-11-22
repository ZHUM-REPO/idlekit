// Typed bridge to the Rust commands. Outside Tauri (plain browser preview)
// every call is served by the in-memory mock backend instead.
import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen, type UnlistenFn } from "@tauri-apps/api/event";
import { endLine, startLine } from "./actions";
import type {
  AchView,
  Account,
  AuthEvent,
  Bootstrap,
  CmStatus,
  FarmGame,
  FarmStatus,
  GameArtUrls,
  GameEntry,
  IdleSnapshot,
  InvItem,
  Listing,
  LocalSteam,
  LogLine,
  MarketProgress,
  PriceInfo,
  SellOutcome,
  Settings,
  StartResult,
  UnlockStatus,
  UpdateState,
  Wallet,
} from "./types";

export const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

type Mock = {
  invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;
  listen: (event: string, cb: (payload: unknown) => void) => () => void;
};
let mockPromise: Promise<Mock> | null = null;
function mock(): Promise<Mock> {
  mockPromise ??= import("./mock").then((m) => m.createMock());
  return mockPromise;
}

async function send<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (inTauri) return tauriInvoke<T>(cmd, args);
  return (await mock()).invoke(cmd, args) as Promise<T>;
}

/** Writes a line to the app's log (and log.txt). Never throws: logging must not break anything. */
export function logLine(level: "debug" | "info" | "warn" | "error", scope: "action" | "ui", message: string): void {
  void send<void>("log_event", { level, scope, message }).catch(() => undefined);
}

/** Runs a command and writes down what it was and how it ended. */
async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const began = performance.now();
  const start = startLine(cmd, args);
  if (start) logLine(start.level, "action", start.message);
  try {
    const result = await send<T>(cmd, args);
    const end = endLine(cmd, args, true, performance.now() - began);
    if (end) logLine(end.level, "action", end.message);
    return result;
  } catch (e) {
    const end = endLine(cmd, args, false, performance.now() - began, errorText(e));
    if (end) logLine(end.level, "action", end.message);
    throw e;
  }
}

export interface EventMap {
  "log://line": LogLine;
  "state://accounts": Account[];
  /** An account's games list changed by itself (names arrived): its Steam ID. */
  "games://updated": string;
  "cm://status": CmStatus;
  "idle://status": IdleSnapshot;
  "farm://status": FarmStatus;
  "unlock://status": UnlockStatus;
  "auth://status": AuthEvent;
  "market://progress": MarketProgress;
  "update://state": UpdateState;
  /** The close button was pressed and the app wants to know what to do. */
  "window://close-requested": null;
  /** The tray menu asks the window to show a page. */
  "app://navigate": string;
  /** Achievement lists closed because the window went to the tray: their app ids. */
  "ach://closed": number[];
  /** The window came back from the tray (or from being minimized). */
  "window://shown": null;
}

export async function on<K extends keyof EventMap>(event: K, cb: (payload: EventMap[K]) => void): Promise<UnlistenFn> {
  if (inTauri) return tauriListen<EventMap[K]>(event, (e) => cb(e.payload));
  const m = await mock();
  return m.listen(event, (p) => cb(p as EventMap[K]));
}

export function errorText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return JSON.stringify(e);
}

export const api = {
  bootstrap: () => call<Bootstrap>("get_bootstrap"),
  accounts: () => call<Account[]>("get_accounts"),
  saveSettings: (settings: Settings) => call<void>("save_settings", { settings }),
  setActiveAccount: (steamId: string) => call<void>("set_active_account", { steamId }),
  detectLocalSteam: () => call<LocalSteam>("detect_local_steam"),
  logs: () => call<LogLine[]>("get_logs"),
  clearLogs: () => call<void>("clear_logs"),
  /** Opens the folder that holds log.txt, with the file selected. */
  openLogFile: () => call<void>("open_log_file"),

  addLocalAccount: (steamId: string) => call<void>("add_local_account", { steamId }),
  authStartQr: (mode: string) => call<string>("auth_start_qr", { mode }),
  authStartCredentials: (username: string, password: string, mode: string) =>
    call<string>("auth_start_credentials", { username, password, mode }),
  authSubmitCode: (sessionId: string, code: string) => call<void>("auth_submit_code", { sessionId, code }),
  authCancel: (sessionId: string) => call<void>("auth_cancel", { sessionId }),
  setWebCookie: (steamId: string, cookie: string) => call<void>("set_web_cookie", { steamId, cookie }),
  removeAccount: (steamId: string) => call<void>("remove_account", { steamId }),
  setAccountMode: (steamId: string, mode: string) => call<void>("set_account_mode", { steamId, mode }),
  connect: (steamId: string) => call<void>("account_connect", { steamId }),
  disconnect: (steamId: string) => call<void>("account_disconnect", { steamId }),
  setPersonaState: (steamId: string, state: number) => call<void>("set_persona_state", { steamId, state }),
  webCheck: (steamId: string) => call<string>("web_check", { steamId }),

  games: (steamId: string, refresh = false) => call<GameEntry[]>("get_games", { steamId, refresh }),
  addManualGame: (steamId: string, appid: number, name: string) =>
    call<void>("add_manual_game", { steamId, appid, name }),
  removeManualGame: (steamId: string, appid: number) => call<void>("remove_manual_game", { steamId, appid }),
  setInList: (steamId: string, list: string, appid: number, on: boolean) =>
    call<void>("set_in_list", { steamId, list, appid, on }),
  /** The same for many games in one call. Answers how many were given. */
  setInListMany: (steamId: string, list: string, appids: number[], on: boolean) =>
    call<number>("set_in_list_many", { steamId, list, appids, on }),
  /** Keyed by app id. Games Steam couldn't be reached for are left out. */
  resolveGameArt: (appids: number[]) => call<Record<string, GameArtUrls>>("resolve_game_art", { appids }),
  /** Which of these pictures are on disk already ("kind:id" → file). No network. */
  picturesCached: (items: { kind: string; id: string; current?: string }[]) =>
    call<Record<string, { file: string; stale: boolean; none: boolean }>>("pictures_cached", { items }),
  /** A picture's file on disk, fetched first from the first of `urls` that works. `certain`: the addresses are not guesses. */
  pictureFetch: (kind: string, id: string, urls: string[], current: string | null, certain: boolean) =>
    call<string | null>("picture_fetch", { kind, id, urls, current, certain }),

  idleStart: (steamId: string, games: { appid: number; name: string }[]) =>
    call<StartResult[]>("idle_start", { steamId, games }),
  idleStop: (steamId: string, appids: number[]) => call<void>("idle_stop", { steamId, appids }),
  idleStopAll: (steamId: string) => call<void>("idle_stop_all", { steamId }),
  autoIdleStart: (steamId: string) => call<StartResult[]>("auto_idle_start", { steamId }),
  boosterStart: (steamId: string, appid: number, name: string, minutes: number) =>
    call<StartResult[]>("booster_start", { steamId, appid, name, minutes }),

  farmScan: (steamId: string) => call<FarmGame[]>("farm_scan", { steamId }),
  farmStart: (steamId: string) => call<void>("farm_start", { steamId }),
  farmStop: (steamId: string) => call<void>("farm_stop", { steamId }),

  achOpen: (steamId: string, appid: number) => call<AchView>("ach_open", { steamId, appid }),
  achClose: (appid: number) => call<void>("ach_close", { appid }),
  achApply: (steamId: string, appid: number, changes: { apiName: string; achieved: boolean }[]) =>
    call<{ results: Record<string, boolean>; stored: boolean }>("ach_apply", { steamId, appid, changes }),
  achSetStats: (steamId: string, appid: number, stats: { name: string; type: string; value: number }[]) =>
    call<{ results: Record<string, boolean>; stored: boolean }>("ach_set_stats", { steamId, appid, stats }),
  achResetAll: (steamId: string, appid: number, achievements: boolean) =>
    call<{ stored: boolean }>("ach_reset_all", { steamId, appid, achievements }),
  achSetPrefs: (steamId: string, appid: number, prefs: { order?: string[]; skip?: string[] }) =>
    call<void>("ach_set_prefs", { steamId, appid, order: prefs.order ?? null, skip: prefs.skip ?? null }),
  achImportTimings: (steamId: string, appid: number, profile: string, names: { apiName: string; name: string }[]) =>
    call<number>("ach_import_timings", { steamId, appid, profile, names }),
  achClearImported: (steamId: string, appid: number) => call<void>("ach_clear_imported", { steamId, appid }),
  unlockStart: (steamId: string) => call<void>("unlock_start", { steamId }),
  unlockStop: (steamId: string) => call<void>("unlock_stop", { steamId }),

  inventory: (steamId: string) => call<InvItem[]>("market_inventory", { steamId }),
  prices: (steamId: string, hashNames: string[]) =>
    call<{ currency: number; wallet: Wallet; items: PriceInfo[] }>("market_prices", { steamId, hashNames }),
  sell: (steamId: string, items: { marketHashName: string; assetIds: string[]; buyerCents: number }[]) =>
    call<SellOutcome[]>("market_sell", { steamId, items }),
  listings: (steamId: string) => call<Listing[]>("market_listings", { steamId }),
  removeListings: (steamId: string, listingIds: string[]) => call<number>("market_remove", { steamId, listingIds }),
  marketCancel: () => call<void>("market_cancel"),

  /** Returns whether IdleKit is registered to start at sign-in afterwards. */
  setAutostart: (enabled: boolean) => call<boolean>("set_autostart", { enabled }),
  windowCloseChoice: (action: "tray" | "quit" | "cancel") => call<void>("window_close_choice", { action }),
  /** The app's own title bar buttons and dragging. Answers whether the window is maximized. */
  windowControl: (action: "minimize" | "maximize" | "close" | "drag" | "state") => call<boolean>("window_control", { action }),
  updateCheck: () => call<UpdateState>("update_check"),
  /** Resolves only on failure: a successful install replaces and restarts the app. */
  updateInstall: () => call<void>("update_install"),
};

export async function openExternal(url: string) {
  if (inTauri) {
    const { openUrl } = await import("@tauri-apps/plugin-opener");
    await openUrl(url);
  } else {
    window.open(url, "_blank", "noopener");
  }
}
