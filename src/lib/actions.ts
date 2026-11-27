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
  /** Background refreshes: written down as debug lines, at most once a
   *  minute each with how many times they ran; failures every time. */
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
  // The backend says what it read, or why it couldn't.
  steam_events: { label: "Read Steam's upcoming events", detail: (a) => (a.refresh ? "from Valve's page" : "as kept"), quiet: true },
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
  set_in_list_many: { label: "Change a games list", detail: (a) => `${count(a.appids)} game(s) ${a.on ? "added to" : "removed from"} ${String(a.list)}` },
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
  market_prices: {
    label: "Get market prices",
    detail: (a) =>
      `${count(a.hashNames)} item(s)${count(a.recheck) ? `, ${count(a.recheck)} checked again` : ""}${count(a.first) ? `, ${count(a.first)} ticked first` : ""}`,
  },
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

  // The CS2 market. A search's words and an item's name are kept; nothing about who trades with whom.
  cs2_facets: { label: "Read the CS2 market's filters", quiet: true },
  cs2_search: {
    label: "Search the CS2 market",
    detail: (a) => {
      const q = (a.query ?? {}) as { text?: string; filters?: Record<string, unknown[]>; start?: number };
      const tags = Object.values(q.filters ?? {}).reduce((n, list) => n + count(list), 0);
      return `${q.text?.trim() ? `"${q.text.trim()}"` : "everything"}${tags ? `, ${tags} filter tag(s)` : ""}${q.start ? `, from ${q.start} on` : ""}`;
    },
  },
  cs2_suggest: { label: "Suggest names for the search box", quiet: true },
  cs2_item: { label: "Open a CS2 item's listings", detail: (a) => `${String(a.id)}${a.start ? `, from ${String(a.start)} on` : ""}` },
  cs2_about: { label: "Read a CS2 item's description", detail: (a) => String(a.hashName), quiet: true },
  // Kept fresh while an item is open: the backend says what each read found.
  cs2_book: { label: "Read a CS2 item's order book", detail: (a) => String(a.hashName), quiet: true },
  cs2_history: { label: "Read a CS2 item's price history", detail: (a) => String(a.hashName), quiet: true },
  cs2_buyer: { label: "Read the wallet", detail: account, quiet: true },
  cs2_buy_listing: {
    label: "Buy a CS2 listing",
    detail: (a) => {
      const l = (a.listing ?? {}) as { hashName?: string; price?: number; fee?: number };
      return `${String(l.hashName)} for ${(((l.price ?? 0) + (l.fee ?? 0)) / 100).toFixed(2)}${a.confirmation ? ", after confirming" : ""}`;
    },
  },
  cs2_buy_order: {
    label: "Place a CS2 buy order",
    detail: (a) => `${String(a.quantity)} × ${String(a.hashName)} at ${(Number(a.priceEach) / 100).toFixed(2)} each${a.confirmation ? ", after confirming" : ""}`,
  },
  cs2_order_status: { label: "Check a buy order", quiet: true },
  cs2_cancel_order: { label: "Cancel a buy order" },
  cs2_mine: { label: "Load CS2 listings and buy orders", detail: account },
  cs2_inventory: { label: "Load the CS2 inventory", detail: account },
  cs2_sell: {
    label: "List CS2 items on the market",
    detail: (a) => {
      const items = (Array.isArray(a.items) ? a.items : []) as { buyerCents?: number }[];
      return `${items.length} item(s), buyers pay ${(items.reduce((n, i) => n + (i.buyerCents ?? 0), 0) / 100).toFixed(2)} in all`;
    },
  },

  // Trade offers. The trade link and the other player are never part of a line written here.
  trade_offers: { label: "Load trade offers", detail: account },
  trade_partner_items: { label: "Load the other player's items for an offer" },
  trade_send: { label: "Send a trade offer", detail: (a) => `${count(a.give)} item(s) given for ${count(a.receive)} item(s)` },
  trade_accept: { label: "Accept a trade offer" },
  trade_withdraw: { label: "Decline or cancel a trade offer", detail: (a) => String(a.how) },

  set_autostart: { label: "Start with the computer", detail: (a) => onOff(a.enabled) },
  window_close_choice: { label: "Answer the close question", detail: (a) => String(a.action) },
  // Moving the window and asking its state are not worth a line each.
  window_control: { label: "Window", detail: (a) => String(a.action), quiet: true },
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

/** How often a quiet command that keeps working is written down. */
export const QUIET_EVERY_MS = 60_000;
/** Per quiet command: when it was last written down, and the runs since. */
const quietSeen = new Map<string, { at: number; since: number }>();

/** The line written when a command ends. A quiet command that worked gets a
 *  debug line at most once a minute, saying how often it ran meanwhile. */
export function endLine(
  cmd: string,
  args: Args | undefined,
  ok: boolean,
  ms: number,
  error?: string,
  now: number = Date.now(),
): ActionLine | null {
  if (UNLOGGED.has(cmd)) return null;
  const what = describe(cmd, args);
  if (!ok) return { level: "error", message: `${what}: FAILED after ${Math.round(ms)} ms: ${error ?? "unknown error"}` };
  if (!ACTIONS[cmd]?.quiet) return { level: "info", message: `${what}: done in ${Math.round(ms)} ms` };
  const seen = quietSeen.get(cmd);
  if (seen && now - seen.at < QUIET_EVERY_MS) {
    seen.since += 1;
    return null;
  }
  const more = seen?.since ? ` (and ${seen.since} more time(s) in the last ${Math.round((now - seen.at) / 1000)} s)` : "";
  quietSeen.set(cmd, { at: now, since: 0 });
  return { level: "debug", message: `${what}: done in ${Math.round(ms)} ms${more}` };
}
