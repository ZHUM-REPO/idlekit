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
  /** The inventory column the items are sorted by, and which way; "none" keeps the inventory's order. */
  marketSort: "none" | `${MarketSortColumn}-${"asc" | "desc"}`;
  logsLevel: "all" | "problems" | "debug";
}

/** The market inventory's columns that sort. */
export type MarketSortColumn = "owned" | "sell" | "lowest" | "highest" | "buyer" | "receive";

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
  /** Steam refused the saved sign-in: only signing in again brings the web session back. */
  signInRefused?: boolean;
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
  /** Where the line comes among everything the backend sent (see Bootstrap.seq). */
  seq?: number;
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
  /** The currency of every price here (Steam's id for it). */
  currency: number;
  lowestSell: number | null;
  highestBuy: number | null;
  sellListings: number;
  suggested: number | null;
  youReceive: number | null;
  /** When Steam was asked (ms since 1970). */
  checkedAt: number;
  /** When the buy orders (`highestBuy`) were read, which takes a request of
   *  the item's own: 0 while only the market's search has answered. */
  offersAt: number;
  /** Why there is no price, when Steam couldn't say. */
  problem: string | null;
}

/** Prices as they come in while a price check goes on. */
export interface PricesUpdate {
  steamId: string;
  wallet: Wallet;
  items: PriceInfo[];
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
  /** The number of the moment this was read: events numbered up to it are in here already. */
  seq?: number;
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

// ---- Counter-Strike 2 market ----

export interface Cs2Tag {
  id: string;
  name: string;
  /** How many items on the market carry it. */
  count: number;
  color: string;
}

/** One of the things the search can be narrowed by (Type, Weapon, Exterior…). */
export interface Cs2Facet {
  id: string;
  name: string;
  tags: Cs2Tag[];
}

export interface Cs2Query {
  text: string;
  /** Filter → the tags picked in it. */
  filters: Record<string, string[]>;
  /** Steam's sort column: 1 name, 2 quantity, 3 price, 4 popularity, 5 trending. */
  sort: number;
  /** 1 lowest first, 2 highest first. */
  direction: number;
  start: number;
  count: number;
  descriptions: boolean;
  /** A price range in the wallet currency's minor units. */
  priceMin: number | null;
  priceMax: number | null;
}

export interface Cs2Hit {
  /** What the item's page is asked for by. */
  id: string;
  hashName: string;
  name: string;
  kind: string;
  iconUrl: string;
  color: string;
  /** Sold by the piece from one order book (cases, keys, stickers). */
  commodity: boolean;
  /** Stands for a group of items that differ in wear, StatTrak or Souvenir. */
  grouped: boolean;
  listings: number;
  currency: number;
  price: number | null;
  priceText: string;
}

export interface Cs2Hits {
  total: number;
  start: number;
  results: Cs2Hit[];
  inDescriptions: boolean;
  walletCurrency: number;
}

export interface Cs2Suggestion {
  id: string;
  name: string;
  iconUrl: string;
}

export interface Cs2Sticker {
  name: string;
  kind: string;
  iconUrl: string;
  wear: number | null;
}

/** One item for sale; prices in the currency's minor units. */
export interface Cs2Listing {
  listingId: string;
  assetId: string;
  hashName: string;
  variant: string;
  name: string;
  color: string;
  /** What the seller gets, the fees on top, and the two together. */
  price: number;
  fee: number;
  total: number;
  currency: number;
  priceText: string;
  float: number | null;
  pattern: number | null;
  inspect: string | null;
  stickers: Cs2Sticker[];
  image: string | null;
  iconUrl: string;
  mine: boolean;
}

export interface Cs2TagCount {
  facet: string;
  facetName: string;
  tag: string;
  tagName: string;
  color: string;
  listings: number;
}

export interface Cs2ItemPage {
  total: number;
  start: number;
  more: boolean;
  listings: Cs2Listing[];
  facets: Cs2TagCount[];
}

export interface Cs2About {
  hashName: string;
  name: string;
  kind: string;
  iconUrl: string;
  color: string;
  commodity: boolean;
  groupId: string;
  groupName: string;
  variant: string;
  lines: string[];
  /** Days a bought item can't be traded or sold on. */
  holdDays: number;
}

export interface Cs2Level {
  price: number;
  quantity: number;
}

export interface Cs2Depth {
  currency: number;
  lowestSell: number | null;
  highestBuy: number | null;
  sellCount: number;
  buyCount: number;
  sells: Cs2Level[];
  buys: Cs2Level[];
  moreSells: number;
  moreBuys: number;
}

export interface Cs2History {
  currency: number;
  /** Per point: seconds since 1970, the median price in minor units, how many sold. */
  points: [number, number, number][];
}

/** The account as a buyer. */
export interface Cs2Buyer {
  currency: number;
  balance: number | null;
  country: string;
  needsBilling: boolean;
  hasBilling: boolean;
  /** 0 none, 1 by e-mail, 2 in the Steam mobile app. */
  confirmation: number;
  taxPercent: number;
  wallet: Wallet;
}

export interface Cs2Bought {
  ok: boolean;
  needsConfirmation: boolean;
  confirmationId: string | null;
  orderId: string | null;
  message: string;
  balance: number | null;
}

export interface Cs2OrderStatus {
  active: boolean;
  quantity: number;
  remaining: number;
  bought: number;
  paidText: string;
}

export interface Cs2MyListing {
  listingId: string;
  hashName: string;
  name: string;
  iconUrl: string;
  color: string;
  buyerPays: number;
  youReceive: number;
  currency: number;
  status: "active" | "pending" | "held";
  created: number;
}

export interface Cs2MyOrder {
  orderId: string;
  hashName: string;
  name: string;
  iconUrl: string;
  color: string;
  price: number;
  quantity: number;
  remaining: number;
  currency: number;
}

export interface Cs2Mine {
  listings: Cs2MyListing[];
  orders: Cs2MyOrder[];
}

export interface Cs2Item {
  assetId: string;
  context: string;
  classId: string;
  instanceId: string;
  hashName: string;
  name: string;
  kind: string;
  iconUrl: string;
  color: string;
  exterior: string;
  marketable: boolean;
  tradable: boolean;
  /** Received in a trade in the last week: not to sell or trade on yet. */
  protected: boolean;
  commodity: boolean;
  float: number | null;
  pattern: number | null;
  notes: string[];
}

// ---- trade offers ----

export interface TradeItem {
  app: number;
  context: string;
  assetId: string;
  name: string;
  kind: string;
  iconUrl: string;
  color: string;
  missing: boolean;
}

export interface TradeOffer {
  id: string;
  /** The other player's Steam ID. */
  partner: string;
  message: string;
  state: number;
  stateText: string;
  ours: boolean;
  created: number;
  updated: number;
  expires: number;
  give: TradeItem[];
  receive: TradeItem[];
  heldUntil: number;
}

export interface TradeOffers {
  received: TradeOffer[];
  sent: TradeOffer[];
}

export interface TradeOutcome {
  ok: boolean;
  offerId: string | null;
  needsConfirmation: boolean;
  byEmail: boolean;
  message: string;
}

export interface AssetRef {
  app: number;
  context: string;
  assetId: string;
}

export interface PartnerItems {
  partner: string;
  items: Cs2Item[];
}
