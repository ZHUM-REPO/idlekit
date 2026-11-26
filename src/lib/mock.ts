// In-memory stand-in for the Rust backend, used when the UI runs in a plain
// browser (`npm run dev` without Tauri). Lets you preview every screen with
// believable data. Nothing here talks to Steam.
import type {
  AchItem,
  AchView,
  Account,
  AuthEvent,
  Bootstrap,
  CmStatus,
  EventList,
  FarmStatus,
  GameEntry,
  IdleGame,
  InvItem,
  Listing,
  LogLine,
  PriceInfo,
  Settings,
  SteamEvent,
  UnlockStatus,
  UpdateState,
} from "./types";
import { US_DOLLAR_WALLET } from "./format";

type Cb = (payload: unknown) => void;

const ME = "76561198012345678";
const ALT = "76561198087654321";

/** A made-up game that the sample store labels as adult-only, for trying "Hide NSFW games". */
const ADULT_SAMPLE = 99900010;

const GAMES: [number, string, number][] = [
  [730, "Counter-Strike 2", 51234],
  [570, "Dota 2", 10500],
  [440, "Team Fortress 2", 8820],
  [620, "Portal 2", 1340],
  [1145360, "Hades", 4210],
  [413150, "Stardew Valley", 9930],
  [105600, "Terraria", 7710],
  [367520, "Hollow Knight", 3120],
  [504230, "Celeste", 1460],
  [646570, "Slay the Spire", 12020],
  [1794680, "Vampire Survivors", 2240],
  [2379780, "Balatro", 3380],
  [548430, "Deep Rock Galactic", 6650],
  [632360, "Risk of Rain 2", 2890],
  [427520, "Factorio", 15240],
  [294100, "RimWorld", 8340],
  [588650, "Dead Cells", 1990],
  [268910, "Cuphead", 640],
  [753640, "Outer Wilds", 1210],
  [632470, "Disco Elysium", 2560],
  [1086940, "Baldur's Gate 3", 13400],
  [892970, "Valheim", 4100],
  [1091500, "Cyberpunk 2077", 3900],
  [1966720, "Lethal Company", 95],
  [1868140, "Dave the Diver", 30],
  [ADULT_SAMPLE, "Sample Adults-Only Game", 410],
];

const defaultSettings: Settings = {
  theme: "graphite",
  accent: "amber",
  accentColor: "#f0b43c",
  cardStyle: "cover",
  hideNsfw: false,
  language: "english",
  maxIdleGames: 32,
  idleStatusText: "",
  autoIdleOnLaunch: false,
  farmOnLaunch: false,
  afterFarming: "none",
  connectDirectOnLaunch: true,
  closeAction: "ask",
  startInTray: true,
  checkUpdates: true,
  autoInstallUpdates: true,
  farming: {
    thresholdHours: 2,
    maxGames: 32,
    soloGames: 1,
    checkMinutes: 10,
    restartOnCheck: true,
    skipRefundable: true,
    order: "mostDrops",
  },
  unlocker: {
    minDelayMinutes: 15,
    maxDelayMinutes: 75,
    scheduleEnabled: false,
    scheduleStart: "18:00",
    scheduleEnd: "23:30",
    order: "common",
    idleWhileUnlocking: true,
    parallelGames: 1,
    importedTimeScale: 1,
  },
  market: { strategy: "undercutLowest", undercutCents: 1, minPriceCents: 3, maxPriceCents: 0, delayMs: 1500 },
  views: { libraryFilter: "all", librarySort: "name", marketTab: "inventory", marketFilter: "all", logsLevel: "all" },
};

function account(partial: Partial<Account>): Account {
  return {
    steamId: "",
    accountName: "",
    personaName: "",
    avatarUrl: "",
    mode: "local",
    personaState: 1,
    favorites: [],
    manualGames: [],
    autoIdle: [],
    farmBlacklist: [],
    farmWhitelist: [],
    unlockQueue: [],
    unlockSkip: {},
    unlockOrder: {},
    unlockImported: {},
    boosterTargets: {},
    addedAt: 1730000000,
    hasToken: true,
    hasCookie: false,
    localActive: false,
    ...partial,
  };
}

const ACH_NAMES = [
  ["First steps", "Finish the tutorial."],
  ["Night shift", "Play for an hour after midnight."],
  ["Collector", "Find 25 hidden items."],
  ["Untouchable", "Clear a level without taking damage."],
  ["Speed run", "Finish a run in under 20 minutes."],
  ["Pacifist", "Complete a chapter without defeating anyone."],
  ["Big spender", "Spend 10,000 gold in one visit."],
  ["Well read", "Read every journal entry."],
  ["Cartographer", "Uncover the whole map."],
  ["Lucky streak", "Win five times in a row."],
  ["The long way", "Walk 100 km in total."],
  ["Completionist", "Unlock every other achievement."],
  ["Hidden depths", "Reach the lowest floor."],
  ["Online regular", "Win 50 online matches."],
];

/**
 * Steam's schedule as the preview shows it: dated from today, so there is
 * always something on and something coming. 10 AM Pacific is 18:00 UTC here,
 * as in winter.
 */
function sampleEvents(now: number): SteamEvent[] {
  const DAY = 24 * 60 * 60_000;
  const today = new Date(now);
  today.setUTCHours(18, 0, 0, 0);
  const at = (days: number) => today.getTime() + days * DAY;
  const year = (days: number) => new Date(at(days)).getUTCFullYear();
  const month = (days: number) => new Date(at(days)).toLocaleDateString("en-US", { month: "long", year: "numeric", timeZone: "UTC" });
  const event = (name: string, kind: SteamEvent["kind"], from: number, to: number): SteamEvent => ({
    name,
    kind,
    firstDay: new Date(at(from)).toISOString().slice(0, 10),
    lastDay: new Date(at(to)).toISOString().slice(0, 10),
    starts: at(from),
    ends: at(to),
  });
  return [
    event("Auto-Battler RPG Fest", "fest", -11, -4),
    event(`Black Friday ${year(-1)}`, "event", -1, 3),
    event(`Winter Sale ${year(20)}`, "sale", 20, 38),
    event("Desktop Companion Fest", "fest", 48, 52),
    event("Shop Keeper Fest", "fest", 59, 66),
    event("Sheep Fest", "fest", 69, 73),
    event("Couch Co-Op Fest", "fest", 73, 80),
    event(`Next Fest: ${month(87)}`, "nextfest", 87, 94),
    event("Rhythm Fest", "fest", 101, 108),
    event(`Spring Sale ${year(111)}`, "sale", 111, 118),
    { name: `Autumn Sale ${year(111)}`, kind: "sale", firstDay: null, lastDay: null, starts: null, ends: null },
  ];
}

export function createMock() {
  const listeners = new Map<string, Set<Cb>>();
  const emit = (event: string, payload: unknown) => listeners.get(event)?.forEach((cb) => cb(payload));
  const params = new URLSearchParams(window.location.search);
  const now = Date.now();

  const settings: Settings = structuredClone(defaultSettings);
  if (params.get("theme")) settings.theme = params.get("theme") as Settings["theme"];
  if (params.get("accent")) settings.accent = params.get("accent") as Settings["accent"];
  if (params.get("color")) settings.accentColor = `#${params.get("color")}`;
  if (params.get("cards")) settings.cardStyle = params.get("cards") as Settings["cardStyle"];
  // `?hide=1` starts with "Hide NSFW games" on. `?nsfw=<appid>,<appid>` says
  // which sample games carry Steam's label (default: the one made-up sample).
  // `?content=down` leaves the content lookup unanswered, as when Steam can't
  // be reached; `?content=slow` answers it after a few seconds.
  if (params.get("hide") === "1") settings.hideNsfw = true;
  const labelled = new Set((params.get("nsfw") ?? String(ADULT_SAMPLE)).split(",").map(Number));
  const contentLookup = params.get("content");
  // `?art=<url>` points looked-up art at a server laid out like Steam's image
  // hosts, for working on the UI offline. Without it the lookup "fails", as it
  // would with Steam unreachable, and the UI falls back to guessable URLs.
  const artBase = params.get("art")?.replace(/\/$/, "");
  const fetched = new Map<string, string>();

  // Desktop behaviour, for previewing its screens:
  //   ?update=available | downloading | error | off     what the updater reports
  //   ?closing=1                                        the "keep running?" question
  //   ?tray=0, ?autostart=1, ?platform=linux            what the system offers
  let autostart = params.get("autostart") === "1";
  const updateDemo = params.get("update");
  const update: UpdateState = {
    configured: updateDemo !== "off",
    current: "1.5",
    phase: updateDemo === "downloading" ? "downloading" : "idle",
    available:
      updateDemo && updateDemo !== "off"
        ? { version: "1.6", notes: "A sample release, for previewing how updates look." }
        : null,
    checkedAt: now - 4 * 60_000,
    error: updateDemo === "error" ? "Couldn't install the update: the connection was lost" : null,
    downloaded: updateDemo === "downloading" ? 4_300_000 : 0,
    total: updateDemo === "downloading" ? 9_600_000 : null,
    releasesUrl: "https://github.com/example/idlekit/releases/latest",
  };
  const pushUpdate = () => emit("update://state", { ...update });
  if (params.get("closing") === "1") setTimeout(() => emit("window://close-requested", null), 400);

  const accounts: Account[] = [
    account({
      steamId: ME,
      accountName: "nightowl_88",
      personaName: "Nightowl",
      mode: "local",
      localActive: true,
      favorites: [413150, 1145360, 646570],
      autoIdle: [730, 440],
      unlockQueue: [646570, 367520],
      farmBlacklist: [570],
    }),
    account({ steamId: ALT, accountName: "cardfarm_alt", personaName: "Alt farm", mode: "direct" }),
  ];
  let active = ME;

  const games: Record<string, GameEntry[]> = {
    [ME]: GAMES.map(([appid, name, minutes]) => ({
      appid,
      name,
      playtimeMinutes: minutes,
      iconUrl: "",
      hasStats: true,
      lastPlayed: Math.floor(now / 1000) - (appid % 97) * 86400,
      manual: false,
    })),
    [ALT]: GAMES.slice(0, 9).map(([appid, name, minutes]) => ({
      appid,
      name,
      playtimeMinutes: Math.round(minutes / 7),
      iconUrl: "",
      hasStats: true,
      lastPlayed: 0,
      manual: false,
    })),
  };

  const idle: Record<string, IdleGame[]> = {
    [ME]: [
      { appid: 730, name: "Counter-Strike 2", startedAt: now - 72 * 60_000 - 14_000, sources: ["manual"], targetMinutes: null, boostStartedAt: null },
      { appid: 440, name: "Team Fortress 2", startedAt: now - 3 * 3600_000 - 9 * 60_000, sources: ["auto"], targetMinutes: null, boostStartedAt: null },
      { appid: 620, name: "Portal 2", startedAt: now - 41 * 60_000, sources: ["booster"], targetMinutes: 120, boostStartedAt: now - 41 * 60_000 },
      { appid: 632470, name: "Disco Elysium", startedAt: now - 4 * 60_000 - 31_000, sources: ["farming"], targetMinutes: null, boostStartedAt: null },
    ],
    [ALT]: [
      { appid: 570, name: "Dota 2", startedAt: now - 9 * 3600_000, sources: ["manual"], targetMinutes: null, boostStartedAt: null },
    ],
  };

  const cm: Record<string, CmStatus> = {
    [ME]: { steamId: ME, status: "offline", message: "", games: [], personaState: 1 },
    [ALT]: { steamId: ALT, status: "online", message: "", games: [570], personaState: 7 },
  };

  const farm: Record<string, FarmStatus> = {
    [ME]: {
      steamId: ME,
      running: true,
      phase: "phase2",
      message: "Phase 2: farming 1 game(s) for drops",
      active: [632470],
      totalDrops: 11,
      cardsFarmed: 7,
      startedAt: now - 2 * 3600_000,
      nextCheckAt: now + 6 * 60_000 + 20_000,
      games: [
        { appid: 632470, name: "Disco Elysium", dropsRemaining: 3, playtimeHours: 42.7, status: "idling", note: "" },
        { appid: 892970, name: "Valheim", dropsRemaining: 4, playtimeHours: 68.3, status: "queued", note: "" },
        { appid: 1966720, name: "Lethal Company", dropsRemaining: 2, playtimeHours: 1.6, status: "queued", note: "" },
        { appid: 1091500, name: "Cyberpunk 2077", dropsRemaining: 1, playtimeHours: 65, status: "queued", note: "" },
        { appid: 1086940, name: "Baldur's Gate 3", dropsRemaining: 0, playtimeHours: 223.4, status: "done", note: "" },
        {
          appid: 1868140,
          name: "Dave the Diver",
          dropsRemaining: 3,
          playtimeHours: 0.5,
          status: "skipped",
          note: "Bought in the last 14 days, so idling could void a refund",
        },
        { appid: 570, name: "Dota 2", dropsRemaining: 1, playtimeHours: 175, status: "skipped", note: "Blacklisted" },
      ],
    },
  };

  const unlock: Record<string, UnlockStatus> = {
    [ME]: {
      steamId: ME,
      running: true,
      phase: "running",
      message: "",
      unlocked: 9,
      games: [
        {
          appid: 646570,
          name: "Slay the Spire",
          total: 46,
          remaining: 12,
          status: "waiting",
          message: "",
          nextAt: now + 23 * 60_000,
          nextName: "Lucky streak",
        },
        { appid: 367520, name: "Hollow Knight", total: 63, remaining: 0, status: "queued", message: "", nextAt: 0, nextName: "" },
      ],
    },
  };

  const logs: LogLine[] = [
    { ts: now - 3 * 3600_000, level: "info", scope: "app", message: "IdleKit 1.5 started" },
    { ts: now - 3 * 3600_000 + 2000, level: "info", scope: "direct", message: `${ALT} connected to Steam` },
    { ts: now - 3 * 3600_000 + 5000, level: "info", scope: "idle", message: "2 game(s) idling (auto)" },
    { ts: now - 2 * 3600_000, level: "info", scope: "farming", message: "Phase 1: building playtime on 3 game(s) under 2h" },
    { ts: now - 95 * 60_000, level: "info", scope: "farming", message: "Valheim: 1 card(s) dropped" },
    { ts: now - 61 * 60_000, level: "warn", scope: "farming", message: "Drop check failed for 892970: HTTP 429 from steamcommunity.com" },
    { ts: now - 40 * 60_000, level: "info", scope: "unlocker", message: 'Slay the Spire: unlocked "Night shift"' },
    { ts: now - 24 * 60_000, level: "info", scope: "farming", message: "Baldur's Gate 3: all card drops received" },
    { ts: now - 4 * 60_000, level: "info", scope: "idle", message: "1 game(s) idling (farming)" },
  ];

  const inventory: InvItem[] = [
    ["Disco Elysium", "card", "Harry", 3],
    ["Disco Elysium", "card", "Kim Kitsuragi", 1],
    ["Valheim", "card", "Troll", 2],
    ["Valheim", "foil", "Troll (Foil)", 1],
    ["Baldur's Gate 3", "card", "Shadowheart", 1],
    ["Baldur's Gate 3", "emoticon", ":owlbear:", 2],
    ["Hades", "background", "House of Hades", 1],
    ["Celeste", "booster", "Celeste Booster Pack", 1],
    ["Sample Adults-Only Game", "card", "Sample Adults-Only Card", 2],
    ["", "gems", "Gems", 1],
  ].map(([game, cls, name, count], i) => ({
    // As Steam names them: the game's app id, then the item (753 is Steam itself).
    marketHashName: `${cls === "gems" ? 753 : (GAMES.find((g) => g[1] === game)?.[0] ?? 100000 + i)}-${name}`,
    name: name as string,
    itemType: cls === "gems" ? "Steam Gems" : `${game} ${cls === "foil" ? "Foil Trading Card" : cls === "card" ? "Trading Card" : cls}`,
    class: cls as InvItem["class"],
    game: game as string,
    iconUrl: "",
    marketable: cls !== "gems",
    count: count as number,
    assetIds: Array.from({ length: count as number }, (_, k) => `${i}${k}0001`),
  }));

  const listings: Listing[] = [
    { listingId: "4422001", name: "Shadowheart", game: "Baldur's Gate 3 Trading Card", priceText: "$0.12 ($0.10)", status: "active" },
    { listingId: "4422002", name: "Troll", game: "Valheim Trading Card", priceText: "$0.09 ($0.07)", status: "active" },
    { listingId: "4422003", name: "Harry", game: "Disco Elysium Trading Card", priceText: "$0.14 ($0.12)", status: "pending" },
    { listingId: "4422004", name: "Sample Adults-Only Card", game: "Sample Adults-Only Game Trading Card", priceText: "$0.05 ($0.03)", status: "active" },
  ];

  const achievements = (appid: number): AchView => {
    const items: AchItem[] = ACH_NAMES.map(([name, description], i) => ({
      apiName: `ACH_${name.toUpperCase().replace(/[^A-Z]+/g, "_")}`,
      name,
      description,
      iconUrl: "",
      iconGrayUrl: "",
      hidden: i === 12,
      achieved: (i * 7 + appid) % 3 !== 0,
      unlockTime: (i * 7 + appid) % 3 !== 0 ? Math.floor(now / 1000) - i * 86400 * 3 : 0,
      percent: Math.max(0.4, 92 - i * 6.7 - (appid % 5)),
      protected: i === 13,
    }));
    return {
      appid,
      achievements: items,
      stats: [
        { name: "STAT_RUNS", displayName: "Runs started", kind: "int", value: 214, min: 0, max: null, incrementOnly: true, protected: false },
        { name: "STAT_WINS", displayName: "Runs won", kind: "int", value: 37, min: 0, max: null, incrementOnly: true, protected: false },
        { name: "STAT_DISTANCE", displayName: "Distance walked (km)", kind: "float", value: 812.4, min: 0, max: null, incrementOnly: false, protected: false },
        { name: "STAT_RANKED", displayName: "Ranked rating", kind: "int", value: 1520, min: 0, max: 5000, incrementOnly: false, protected: true },
      ],
      schemaFound: true,
      warning: null,
    };
  };

  const view = (a: Account) => ({ ...a });
  // A copy each time, as the real backend's messages are: the window only
  // notices a list that is a new object.
  const pushIdle = (sid: string) => emit("idle://status", { steamId: sid, games: [...(idle[sid] ?? [])] });
  const delay = (ms: number) => new Promise((r) => setTimeout(r, ms));

  const handlers: Record<string, (a: Record<string, any>) => unknown | Promise<unknown>> = {
    get_bootstrap: (): Bootstrap => ({
      version: "1.5",
      dataDir: "C:\\Users\\you\\AppData\\Roaming\\dev.idlekit.app",
      settings,
      accounts: accounts.map(view),
      activeAccount: active,
      localSteam: {
        installPath: "C:\\Program Files (x86)\\Steam",
        running: true,
        activeSteamId: ME,
        users: [
          { steamId: ME, accountName: "nightowl_88", personaName: "Nightowl", mostRecent: true, active: true },
          { steamId: "76561198011112222", accountName: "family_pc", personaName: "Living room", mostRecent: false, active: false },
        ],
      },
      helperFound: true,
      cm: Object.values(cm),
      idle: Object.entries(idle).map(([steamId, g]) => ({ steamId, games: g })),
      farming: Object.values(farm),
      unlocker: Object.values(unlock),
      logs,
      logFile: "C:\\Users\\you\\AppData\\Roaming\\dev.idlekit.app\\log.txt",
      desktop: {
        platform: (params.get("platform") as Bootstrap["desktop"]["platform"]) ?? "windows",
        tray: params.get("tray") !== "0",
        autostart,
      },
      update: { ...update },
    }),
    get_accounts: () => accounts.map(view),
    save_settings: ({ settings: s }) => Object.assign(settings, s),
    set_active_account: ({ steamId }) => {
      active = steamId;
    },
    detect_local_steam: () => handlers.get_bootstrap({}) && (handlers.get_bootstrap({}) as Bootstrap).localSteam,
    get_logs: () => logs,
    // `?events=down`: Valve's page can't be read, the list that came with the
    // app is shown. `?events=none`: nothing on the schedule. `?events=next`:
    // nothing on right now, only events to come.
    steam_events: async ({ refresh }) => {
      await delay(refresh ? 600 : 150);
      const mode = params.get("events");
      const sample = sampleEvents(now).filter((e) => mode !== "next" || e.starts == null || e.starts > now);
      const list: EventList = { events: mode === "none" ? [] : sample, fetchedAt: refresh ? Date.now() : now, from: "valve" };
      if (mode === "down") return { ...list, fetchedAt: null, from: "built-in", error: "Valve's page couldn't be reached" };
      return list;
    },
    log_event: ({ level, scope, message }) => {
      const line = { ts: Date.now(), level, scope, message };
      logs.push(line);
      emit("log://line", line);
    },
    open_log_file: () => undefined,
    clear_logs: () => {
      logs.length = 0;
    },
    add_local_account: ({ steamId }) => {
      if (!accounts.some((a) => a.steamId === steamId)) {
        accounts.push(account({ steamId, accountName: "family_pc", personaName: "Living room", hasToken: false }));
        emit("state://accounts", accounts);
      }
    },
    auth_start_qr: () => {
      const id = "qr" + Math.random().toString(16).slice(2);
      setTimeout(
        () =>
          emit("auth://status", {
            sessionId: id,
            status: "qr",
            challengeUrl: "https://s.team/q/1/2938475610293847561",
          } as AuthEvent),
        400,
      );
      return id;
    },
    auth_start_credentials: () => {
      const id = "pw" + Math.random().toString(16).slice(2);
      setTimeout(
        () =>
          emit("auth://status", {
            sessionId: id,
            status: "code",
            codeType: "device",
            canConfirm: true,
          } as AuthEvent),
        900,
      );
      return id;
    },
    auth_submit_code: async ({ sessionId }) => {
      emit("auth://status", { sessionId, status: "working" } as AuthEvent);
      await delay(1200);
      emit("auth://status", { sessionId, status: "done", steamId: ALT, accountName: "cardfarm_alt" } as AuthEvent);
    },
    auth_cancel: () => undefined,
    set_web_cookie: async () => {
      await delay(600);
    },
    remove_account: ({ steamId }) => {
      const i = accounts.findIndex((a) => a.steamId === steamId);
      if (i >= 0) accounts.splice(i, 1);
      emit("state://accounts", accounts);
    },
    set_account_mode: ({ steamId, mode }) => {
      const a = accounts.find((x) => x.steamId === steamId);
      if (a) a.mode = mode;
      emit("state://accounts", accounts);
    },
    account_connect: async ({ steamId }) => {
      cm[steamId] = { ...cm[steamId], steamId, status: "connecting" } as CmStatus;
      emit("cm://status", cm[steamId]);
      await delay(1000);
      cm[steamId] = { ...cm[steamId], status: "online" };
      emit("cm://status", cm[steamId]);
    },
    account_disconnect: ({ steamId }) => {
      cm[steamId] = { ...cm[steamId], status: "offline" };
      emit("cm://status", cm[steamId]);
    },
    set_persona_state: ({ steamId, state }) => {
      const a = accounts.find((x) => x.steamId === steamId);
      if (a) a.personaState = state;
      emit("state://accounts", accounts);
    },
    web_check: () => "Web session OK",
    get_games: async ({ steamId, refresh }) => {
      if (refresh) await delay(700);
      return games[steamId] ?? [];
    },
    add_manual_game: ({ steamId, appid, name }) => {
      games[steamId]?.push({ appid, name: name || `App ${appid}`, playtimeMinutes: 0, iconUrl: "", hasStats: false, lastPlayed: 0, manual: true });
      const a = accounts.find((x) => x.steamId === steamId);
      a?.manualGames.push({ appid, name });
      emit("state://accounts", accounts);
    },
    remove_manual_game: ({ steamId, appid }) => {
      games[steamId] = (games[steamId] ?? []).filter((g) => !(g.manual && g.appid === appid));
    },
    // The preview has no pictures folder. A "file" here is the picture's own
    // address, handed over once the browser has loaded it, so the queue in
    // pictures.ts behaves as in the app: a few fetches at a time, each kept.
    pictures_cached: ({ items }) =>
      Object.fromEntries(
        (items as { kind: string; id: string }[])
          .map((i) => [`${i.kind}:${i.id}`, fetched.get(`${i.kind}:${i.id}`)] as const)
          .filter(([, file]) => file)
          .map(([key, file]) => [key, { file, stale: false, none: false }]),
      ),
    picture_fetch: async ({ kind, id, urls }) => {
      for (const url of urls as string[]) {
        const ok = await new Promise<boolean>((resolve) => {
          const img = new Image();
          img.onload = () => resolve(true);
          img.onerror = () => resolve(false);
          img.src = url;
        });
        if (ok) {
          fetched.set(`${kind}:${id}`, url);
          return url;
        }
      }
      return null;
    },
    resolve_game_art: async ({ appids }) => {
      if (contentLookup === "down") return {};
      await delay(contentLookup === "slow" ? 4000 : 120);
      const apps = `${artBase}/store_item_assets/steam/apps`;
      return Object.fromEntries(
        (appids as number[]).map((id) => [
          id,
          {
            // Without `?art=`, no picture addresses: the UI falls back to guessable URLs.
            icon: artBase ? `${artBase}/community_assets/images/apps/${id}/icon.jpg` : null,
            header: artBase ? `${apps}/${id}/header.jpg` : null,
            cover: artBase ? `${apps}/${id}/library_600x900.jpg` : null,
            nsfw: labelled.has(id),
            name: GAMES.find((g) => g[0] === id)?.[1] ?? null,
          },
        ]),
      );
    },
    set_in_list_many: ({ steamId, list, appids, on }) => {
      const a = accounts.find((x) => x.steamId === steamId) as any;
      if (!a) return 0;
      a[list] = (a[list] as number[]).filter((x) => !(appids as number[]).includes(x));
      if (on) a[list].push(...(appids as number[]));
      emit("state://accounts", accounts);
      return (appids as number[]).length;
    },
    set_in_list: ({ steamId, list, appid, on }) => {
      const a = accounts.find((x) => x.steamId === steamId) as any;
      if (!a) return;
      a[list] = (a[list] as number[]).filter((x) => x !== appid);
      if (on) a[list].push(appid);
      emit("state://accounts", accounts);
    },
    idle_start: async ({ steamId, games: list }) => {
      await delay(300);
      const cur = (idle[steamId] ??= []);
      for (const g of list) {
        if (!cur.some((x) => x.appid === g.appid))
          cur.push({ appid: g.appid, name: g.name, startedAt: Date.now(), sources: ["manual"], targetMinutes: null, boostStartedAt: null });
      }
      pushIdle(steamId);
      return list.map((g: { appid: number }) => ({ appid: g.appid, ok: true, error: null }));
    },
    idle_stop: ({ steamId, appids }) => {
      idle[steamId] = (idle[steamId] ?? []).filter((g) => !appids.includes(g.appid));
      pushIdle(steamId);
    },
    idle_stop_all: ({ steamId }) => {
      idle[steamId] = [];
      pushIdle(steamId);
    },
    auto_idle_start: ({ steamId }) => {
      const a = accounts.find((x) => x.steamId === steamId)!;
      return handlers.idle_start({
        steamId,
        games: a.autoIdle.map((appid) => ({ appid, name: GAMES.find((g) => g[0] === appid)?.[1] ?? `App ${appid}` })),
      });
    },
    booster_start: ({ steamId, appid, name, minutes }) => {
      const cur = (idle[steamId] ??= []);
      cur.push({ appid, name, startedAt: Date.now(), sources: ["booster"], targetMinutes: minutes, boostStartedAt: Date.now() });
      pushIdle(steamId);
      return [{ appid, ok: true, error: null }];
    },
    farm_scan: async ({ steamId }) => {
      await delay(800);
      return farm[steamId]?.games ?? farm[ME].games;
    },
    farm_start: ({ steamId }) => {
      farm[steamId] = { ...(farm[steamId] ?? farm[ME]), steamId, running: true, phase: "phase2" };
      emit("farm://status", farm[steamId]);
    },
    farm_stop: ({ steamId }) => {
      if (!farm[steamId]) return;
      farm[steamId] = { ...farm[steamId], running: false, phase: "stopped", message: "Stopped", active: [] };
      emit("farm://status", farm[steamId]);
    },
    farm_status: ({ steamId }) => farm[steamId],
    ach_open: async ({ appid }) => {
      await delay(500);
      return achievements(appid);
    },
    ach_close: () => undefined,
    ach_apply: async ({ changes }) => {
      await delay(500);
      return { results: Object.fromEntries(changes.map((c: { apiName: string }) => [c.apiName, true])), stored: true };
    },
    ach_set_stats: async ({ stats }) => ({
      results: Object.fromEntries(stats.map((s: { name: string }) => [s.name, true])),
      stored: true,
    }),
    ach_reset_all: () => ({ stored: true }),
    ach_set_prefs: () => undefined,
    ach_import_timings: async () => {
      await delay(900);
      return 31;
    },
    ach_clear_imported: () => undefined,
    unlock_start: ({ steamId }) => {
      unlock[steamId] = { ...(unlock[steamId] ?? unlock[ME]), steamId, running: true, phase: "running" };
      emit("unlock://status", unlock[steamId]);
    },
    unlock_stop: ({ steamId }) => {
      if (!unlock[steamId]) return;
      unlock[steamId] = { ...unlock[steamId], running: false, phase: "stopped" };
      emit("unlock://status", unlock[steamId]);
    },
    market_inventory: async () => {
      await delay(500);
      return inventory;
    },
    market_prices: async ({ hashNames }) => {
      const out: PriceInfo[] = [];
      for (let i = 0; i < hashNames.length; i++) {
        emit("market://progress", { task: "prices", done: i, total: hashNames.length, message: `Pricing ${hashNames[i]}` });
        await delay(160);
        const seed = [...hashNames[i]].reduce((s: number, c: string) => s + c.charCodeAt(0), 0);
        const lowest = 6 + (seed % 40);
        const suggested = lowest - 1;
        out.push({
          marketHashName: hashNames[i],
          lowestSell: lowest,
          highestBuy: lowest - 3,
          sellListings: 120 + (seed % 900),
          suggested,
          youReceive: Math.max(1, Math.floor(suggested / 1.15)),
        });
      }
      emit("market://progress", { task: "prices", done: hashNames.length, total: hashNames.length, message: "Done" });
      return { currency: 1, wallet: US_DOLLAR_WALLET, items: out };
    },
    market_sell: async ({ items }) => {
      const out = [];
      for (const it of items) for (const id of it.assetIds) out.push({ assetId: id, ok: true, needsConfirmation: true, message: "" });
      await delay(700);
      return out;
    },
    market_listings: async () => {
      await delay(400);
      return listings;
    },
    market_remove: async ({ listingIds }) => {
      await delay(500);
      return listingIds.length;
    },
    market_cancel: () => undefined,

    set_autostart: ({ enabled }) => (autostart = !!enabled),
    window_close_choice: () => undefined,
    window_control: () => false,
    update_check: async () => {
      update.phase = "checking";
      update.error = null;
      pushUpdate();
      await delay(900);
      update.phase = "idle";
      update.checkedAt = Date.now();
      pushUpdate();
      return { ...update };
    },
    // Plays the download and install, then "restarts" on the new version.
    update_install: async () => {
      if (!update.available) throw "Check for updates first";
      update.error = null;
      update.phase = "downloading";
      update.total = 9_600_000;
      for (let done = 0; done <= 10; done++) {
        update.downloaded = (update.total / 10) * done;
        pushUpdate();
        await delay(250);
      }
      update.phase = "installing";
      pushUpdate();
      await delay(1500);
      update.current = update.available.version;
      update.available = null;
      update.phase = "idle";
      update.downloaded = 0;
      update.total = null;
      update.checkedAt = Date.now();
      pushUpdate();
    },
  };

  return {
    async invoke(cmd: string, args: Record<string, unknown> = {}) {
      const h = handlers[cmd];
      if (!h) throw new Error(`mock: no handler for ${cmd}`);
      return await h(args as Record<string, any>);
    },
    listen(event: string, cb: Cb) {
      if (!listeners.has(event)) listeners.set(event, new Set());
      listeners.get(event)!.add(cb);
      return () => listeners.get(event)?.delete(cb);
    },
  };
}
