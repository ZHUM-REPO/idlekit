// Mirrors the Rust structs (serde camelCase). Keep in sync with src-tauri.

export type Mode = "local" | "direct";

export interface FarmingSettings {
  thresholdHours: number;
  maxGames: number;
  soloGames: number;
  checkMinutes: number;
  restartOnCheck: boolean;
  skipRefundable: boolean;
  order: "mostDrops" | "leastDrops" | "name" | "playtime";
}

export interface UnlockerSettings {
  minDelayMinutes: number;
  maxDelayMinutes: number;
  scheduleEnabled: boolean;
  scheduleStart: string;
  scheduleEnd: string;
  order: "common" | "rare" | "schema";
  idleWhileUnlocking: boolean;
  parallelGames: number;
  importedTimeScale: number;
}

/** How pages were left, saved so they come back the same after a restart. */
export interface ViewSettings {
  libraryFilter: "all" | "favorites" | "idling" | "unplayed" | "manual";
  librarySort: "name" | "playtime" | "recent";
  marketTab: "inventory" | "listings";
  marketFilter: "all" | "card" | "foil" | "emoticon" | "background" | "booster" | "gems" | "other";
  logsLevel: "all" | "problems" | "debug";
}

export interface MarketSettings {
  strategy: "undercutLowest" | "matchLowest" | "aboveHighestBuy";
  undercutCents: number;
  minPriceCents: number;
  maxPriceCents: number;
  delayMs: number;
}

export interface Settings {
  theme: "graphite" | "midnight" | "obsidian" | "harbor" | "moss" | "plum" | "daylight" | "paper" | "frost" | "contrast";
  accent: "amber" | "orange" | "rose" | "orchid" | "violet" | "blue" | "cyan" | "mint" | "lime" | "silver" | "custom";
  /** The running color picked by hand, as #rrggbb: used when accent is "custom". */
  accentColor: string;
  /** Library cards: portrait cover art, or the wide banner. */
  cardStyle: "cover" | "banner";
  /** Keep games Steam labels as adult-only or as frequent nudity or sexual content out of every list. */
  hideNsfw: boolean;
  language: string;
  maxIdleGames: number;
  idleStatusText: string;
  autoIdleOnLaunch: boolean;
  farmOnLaunch: boolean;
  afterFarming: "none" | "autoIdle" | "unlocker";
  connectDirectOnLaunch: boolean;
  /** What the window's close button does. "ask" asks the first time. */
  closeAction: "ask" | "tray" | "quit";
  /** When started at sign-in, stay in the tray instead of opening the window. */
  startInTray: boolean;
  /** Look for a newer version each time IdleKit opens. */
  checkUpdates: boolean;
  /** Install a newer version without asking, when nothing is running. */
  autoInstallUpdates: boolean;
  farming: FarmingSettings;
  unlocker: UnlockerSettings;
  market: MarketSettings;
  views: ViewSettings;
}

export interface ImportedTiming {
  apiName: string;
  offsetSecs: number;
}

export interface Account {
  steamId: string;
  accountName: string;
  personaName: string;
  avatarUrl: string;
  mode: Mode;
  personaState: number;
  favorites: number[];
  manualGames: { appid: number; name: string }[];
  autoIdle: number[];
  farmBlacklist: number[];
  farmWhitelist: number[];
  unlockQueue: number[];
  unlockSkip: Record<string, string[]>;
  unlockOrder: Record<string, string[]>;
  unlockImported: Record<string, ImportedTiming[]>;
  boosterTargets: Record<string, number>;
  addedAt: number;
  hasToken: boolean;
  hasCookie: boolean;
  localActive: boolean;
}

export interface LocalUser {
  steamId: string;
  accountName: string;
  personaName: string;
  mostRecent: boolean;
  active: boolean;
}

export interface LocalSteam {
  installPath: string | null;
  running: boolean;
  activeSteamId: string | null;
  users: LocalUser[];
}

export type CmState = "offline" | "connecting" | "online" | "blocked" | "reconnecting" | "error";

export interface CmStatus {
  steamId: string;
  status: CmState;
  message: string;
  games: number[];
  personaState: number;
}

export interface IdleGame {
  appid: number;
  name: string;
  startedAt: number;
  sources: string[];
  targetMinutes: number | null;
  /** What targetMinutes is counted from: when the boost was set. */
  boostStartedAt: number | null;
}

export interface IdleSnapshot {
  steamId: string;
  games: IdleGame[];
}

export interface StartResult {
  appid: number;
  ok: boolean;
  error: string | null;
}

export interface FarmGame {
  appid: number;
  name: string;
  dropsRemaining: number;
  playtimeHours: number;
  status: "queued" | "idling" | "done" | "skipped" | "error";
  note: string;
}

export interface FarmStatus {
  steamId: string;
  running: boolean;
  phase: "idle" | "scanning" | "phase1" | "phase2" | "restarting" | "done" | "stopped" | "error";
  message: string;
  games: FarmGame[];
  active: number[];
  totalDrops: number;
  cardsFarmed: number;
  startedAt: number;
  nextCheckAt: number;
}

export interface UnlockGame {
  appid: number;
  name: string;
  total: number;
  remaining: number;
  status: "queued" | "running" | "waiting" | "done" | "error";
  message: string;
  nextAt: number;
  nextName: string;
}

export interface UnlockStatus {
  steamId: string;
  running: boolean;
  phase: "idle" | "running" | "done" | "stopped" | "error";
  message: string;
  games: UnlockGame[];
  unlocked: number;
}

export interface LogLine {
  /** Given by the window as the line arrives, so a row keeps its identity when the list moves up. */
  n?: number;
  ts: number;
  level: "info" | "warn" | "error" | "debug";
  scope: string;
  message: string;
}

export interface GameEntry {
  appid: number;
  name: string;
  playtimeMinutes: number;
  iconUrl: string;
  hasStats: boolean;
  lastPlayed: number;
  manual: boolean;
}

/** Image URLs for one game, looked up from Steam's store. `null` where Steam has none. */
export interface GameArtUrls {
  icon: string | null;
  header: string | null;
  cover: string | null;
  /** Steam labels the game as adult-only or as frequent nudity or sexual content. */
  nsfw: boolean | null;
  /** What Steam's store calls the game. The backend uses it to name games listed by number only. */
  name: string | null;
}

export interface AchItem {
  apiName: string;
  name: string;
  description: string;
  iconUrl: string;
  iconGrayUrl: string;
  hidden: boolean;
  achieved: boolean;
  unlockTime: number;
  percent: number | null;
  protected: boolean;
}

export interface StatItem {
  name: string;
  displayName: string;
  kind: "int" | "float" | "avgrate";
  value: number | null;
  min: number | null;
  max: number | null;
  incrementOnly: boolean;
  protected: boolean;
}

export interface AchView {
  appid: number;
  achievements: AchItem[];
  stats: StatItem[];
  schemaFound: boolean;
  warning: string | null;
}

export interface InvItem {
  marketHashName: string;
  name: string;
  itemType: string;
  class: "card" | "foil" | "emoticon" | "background" | "booster" | "gems" | "other";
  game: string;
  iconUrl: string;
  marketable: boolean;
  count: number;
  assetIds: string[];
}

/** How Steam's market charges in the account's wallet currency (minor units). */
export interface Wallet {
  /** Steam's currency id (1 = USD, 3 = EUR, 31 = SAR, …). */
  currency: number;
  steamPercent: number;
  steamMinimum: number;
  feeBase: number;
  publisherPercent: number;
  increment: number;
  marketMinimum: number;
}

export interface PriceInfo {
  marketHashName: string;
  lowestSell: number | null;
  highestBuy: number | null;
  sellListings: number;
  suggested: number | null;
  youReceive: number | null;
}

export interface Listing {
  listingId: string;
  name: string;
  game: string;
  priceText: string;
  status: "active" | "pending";
}

export interface SellOutcome {
  assetId: string;
  ok: boolean;
  needsConfirmation: boolean;
  message: string;
}

export interface MarketProgress {
  task: "prices" | "sell" | "remove";
  done: number;
  total: number;
  message: string;
}

export type AuthStage = "qr" | "scanned" | "code" | "confirm" | "working" | "done" | "error" | "expired";

export interface AuthEvent {
  sessionId: string;
  status: AuthStage;
  challengeUrl: string | null;
  codeType: "email" | "device" | null;
  hint: string | null;
  canConfirm: boolean;
  message: string | null;
  steamId: string | null;
  accountName: string | null;
}

export interface DesktopInfo {
  platform: "windows" | "macos" | "linux";
  /** A tray icon exists, so the window can be closed without quitting. */
  tray: boolean;
  /** IdleKit is registered to start at sign-in. */
  autostart: boolean;
}

export interface UpdateState {
  /** False when this copy has nowhere to look for updates. */
  configured: boolean;
  current: string;
  phase: "idle" | "checking" | "downloading" | "installing";
  available: { version: string; notes: string } | null;
  /** Unix time in milliseconds of the last finished check. */
  checkedAt: number | null;
  error: string | null;
  downloaded: number;
  total: number | null;
  /** Where releases are listed, for "what's new". */
  releasesUrl: string | null;
}

export interface Bootstrap {
  version: string;
  dataDir: string;
  settings: Settings;
  accounts: Account[];
  activeAccount: string | null;
  localSteam: LocalSteam;
  helperFound: boolean;
  cm: CmStatus[];
  idle: IdleSnapshot[];
  farming: FarmStatus[];
  unlocker: UnlockStatus[];
  logs: LogLine[];
  /** Where log.txt is, or null when it couldn't be opened. */
  logFile: string | null;
  desktop: DesktopInfo;
  update: UpdateState;
}

/** One of Steam's sales or fests, from Valve's schedule. */
export interface SteamEvent {
  /** As Valve names it: "Winter Sale 2026", "Racing Fest", "Next Fest: February 2027". */
  name: string;
  /** The seasonal sales, the themed fests, Next Fest, or anything else ("event"). */
  kind: "sale" | "fest" | "nextfest" | "event";
  /** The first and last day as Valve gives them (YYYY-MM-DD); null while the dates are to be decided. */
  firstDay: string | null;
  lastDay: string | null;
  /** When it starts and ends (ms). Steam changes over at 10 AM Pacific time. */
  starts: number | null;
  ends: number | null;
}

export interface EventList {
  events: SteamEvent[];
  /** When the list was read from Valve's page; null for the list this version came with. */
  fetchedAt: number | null;
  from: "valve" | "built-in";
  /** Why Valve's page couldn't be read this time, when it couldn't. */
  error?: string;
}
