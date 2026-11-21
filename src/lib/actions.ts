// What each command means in words, for the log: every action taken in the
// window is written down with how it ended.
//
// Only what is listed here is ever logged about a command. Passwords, Steam
// Guard codes, cookies, sign-in names and other people's profile addresses are
// never part of a description; the backend scrubs every line again on top.

type Args = Record<string, unknown>;

interface Action {
  /** What the user did, in words. */
  label: string;
  /** The safe details worth keeping. */
  detail?: (a: Args) => string;
  /** Background refreshes: only written down when they fail. */
  quiet?: boolean;
}

const count = (v: unknown) => (Array.isArray(v) ? v.length : 0);
const ids = (v: unknown) => {
  const list = (Array.isArray(v) ? v : []).map((x) => (typeof x === "object" && x ? (x as { appid?: number }).appid : x));
  return list.length > 8 ? `${list.slice(0, 8).join(", ")}, …` : list.join(", ");
};
const account = (a: Args) => (typeof a.steamId === "string" && a.steamId ? `account …${a.steamId.slice(-4)}` : "");
const game = (a: Args) => `game ${String(a.appid)}`;
const onOff = (v: unknown) => (v ? "on" : "off");

export const ACTIONS: Record<string, Action> = {
  get_bootstrap: { label: "Load the app's state", quiet: true },
  get_accounts: { label: "Refresh accounts", quiet: true },
  detect_local_steam: { label: "Look for the Steam client", quiet: true },
  get_logs: { label: "Read the log", quiet: true },
  farm_status: { label: "Read farming status", quiet: true },
  resolve_game_art: { label: "Look up game pictures", detail: (a) => `${count(a.appids)} game(s)`, quiet: true },
  // Pictures are counted by the backend: one line per run, not two per picture.
  pictures_cached: { label: "Check pictures on disk", detail: (a) => `${count(a.items)} picture(s)`, quiet: true },
  picture_fetch: { label: "Fetch a picture", detail: (a) => String(a.kind), quiet: true },
  // Settings changes are written down by the backend, value by value.
  save_settings: { label: "Save settings", quiet: true },

  clear_logs: { label: "Clear the log shown in the window" },
  open_log_file: { label: "Show the log file" },
  set_active_account: { label: "Switch account", detail: account },

  add_local_account: { label: "Add this PC's Steam account", detail: account },
  auth_start_qr: { label: "Start sign-in by QR code", detail: (a) => `as ${String(a.mode)} account` },
  auth_start_credentials: { label: "Start sign-in by name and password", detail: (a) => `as ${String(a.mode)} account` },
  auth_submit_code: { label: "Enter a Steam Guard code" },
  auth_cancel: { label: "Cancel sign-in" },
  set_web_cookie: { label: "Save a web session cookie", detail: account },
  remove_account: { label: "Remove account", detail: account },
  set_account_mode: { label: "Change how an account connects", detail: (a) => `${account(a)} to ${String(a.mode)}` },
  account_connect: { label: "Connect to Steam directly", detail: account },
  account_disconnect: { label: "Disconnect from Steam", detail: account },
  set_persona_state: { label: "Set online status", detail: (a) => `${account(a)} to state ${String(a.state)}` },
  web_check: { label: "Check the web session", detail: account },

  get_games: { label: "Load the games list", detail: (a) => `${account(a)}${a.refresh ? ", refreshed from Steam" : ""}` },
  add_manual_game: { label: "Add a game by hand", detail: (a) => `${game(a)} "${String(a.name)}"` },
  remove_manual_game: { label: "Remove a hand-added game", detail: game },
  set_in_list: { label: "Change a games list", detail: (a) => `${game(a)} ${a.on ? "added to" : "removed from"} ${String(a.list)}` },

  idle_start: { label: "Start idling", detail: (a) => `${count(a.games)} game(s) (${ids(a.games)})` },
  idle_stop: { label: "Stop idling", detail: (a) => `${count(a.appids)} game(s) (${ids(a.appids)})` },
  idle_stop_all: { label: "Stop idling everything", detail: account },
  auto_idle_start: { label: "Start the auto-idle list", detail: account },
  booster_start: { label: "Start a playtime boost", detail: (a) => `${game(a)} for ${String(a.minutes)} min` },

  farm_scan: { label: "Check badges for card drops", detail: account },
  farm_start: { label: "Start card farming", detail: account },
  farm_stop: { label: "Stop card farming", detail: account },

  ach_open: { label: "Open a game's achievements", detail: game },
  ach_close: { label: "Close a game's achievements", detail: game },
  ach_apply: {
    label: "Change achievements",
    detail: (a) => {
      const changes = (Array.isArray(a.changes) ? a.changes : []) as { achieved?: boolean }[];
      const unlock = changes.filter((c) => c.achieved).length;
      return `${game(a)}: unlock ${unlock}, lock ${changes.length - unlock}`;
    },
  },
  ach_set_stats: { label: "Change stats", detail: (a) => `${game(a)}: ${count(a.stats)} stat(s)` },
  ach_reset_all: { label: "Reset a game's stats", detail: (a) => `${game(a)}, achievements too: ${a.achievements ? "yes" : "no"}` },
  ach_set_prefs: { label: "Change unlock order or skips", detail: game },
  ach_import_timings: { label: "Copy unlock timing from a profile", detail: game },
  ach_clear_imported: { label: "Forget copied unlock timing", detail: game },
  unlock_start: { label: "Start the achievement unlocker", detail: account },
  unlock_stop: { label: "Stop the achievement unlocker", detail: account },

  market_inventory: { label: "Load the inventory", detail: account },
  market_prices: { label: "Get market prices", detail: (a) => `${count(a.hashNames)} item(s)` },
  market_sell: {
    label: "List items on the market",
    detail: (a) => {
      const items = (Array.isArray(a.items) ? a.items : []) as { assetIds?: unknown[]; buyerCents?: number }[];
      const copies = items.reduce((n, i) => n + count(i.assetIds), 0);
      const total = items.reduce((n, i) => n + (i.buyerCents ?? 0) * count(i.assetIds), 0);
      return `${items.length} item(s), ${copies} listing(s), buyers pay ${(total / 100).toFixed(2)} in all`;
    },
  },
  market_listings: { label: "Load market listings", detail: account },
  market_remove: { label: "Remove market listings", detail: (a) => `${count(a.listingIds)} listing(s)` },
  market_cancel: { label: "Cancel the market task" },

  set_autostart: { label: "Start with the computer", detail: (a) => onOff(a.enabled) },
  window_close_choice: { label: "Answer the close question", detail: (a) => String(a.action) },
  update_check: { label: "Check for updates" },
  update_install: { label: "Install the update" },
};

/** Commands that write to the log themselves: logging those would loop. */
export const UNLOGGED = new Set(["log_event"]);

/** A command in words with its safe details, e.g. "Start idling: 2 game(s) (440, 570)". */
export function describe(cmd: string, args?: Args): string {
  const action = ACTIONS[cmd] ?? { label: cmd };
  let detail = "";
  try {
    detail = action.detail?.(args ?? {}) ?? "";
  } catch {
    // A description must never be the thing that breaks an action.
  }
  return detail ? `${action.label}: ${detail}` : action.label;
}

export interface ActionLine {
  level: "debug" | "info" | "error";
  message: string;
}

/** The line written when a command begins, so a crash midway still shows what was going on. */
export function startLine(cmd: string, args?: Args): ActionLine | null {
  if (UNLOGGED.has(cmd) || ACTIONS[cmd]?.quiet) return null;
  return { level: "debug", message: `${describe(cmd, args)}: started` };
}

/** The line written when a command ends. Quiet commands only get one when they fail. */
export function endLine(cmd: string, args: Args | undefined, ok: boolean, ms: number, error?: string): ActionLine | null {
  if (UNLOGGED.has(cmd) || (ok && ACTIONS[cmd]?.quiet)) return null;
  const what = describe(cmd, args);
  return ok
    ? { level: "info", message: `${what}: done in ${Math.round(ms)} ms` }
    : { level: "error", message: `${what}: FAILED after ${Math.round(ms)} ms: ${error ?? "unknown error"}` };
}
