// The CS2 market and trade offers for the browser preview: a small made-up
// market that behaves like the real one closely enough to try every screen.
// Nothing here talks to Steam, and no money or item is real.
//
//   ?cs2search=other   the search answers in another currency than the wallet's
//   ?cs2facets=down    Steam's list of filters can't be read
//   ?cs2buy=confirm    purchases wait to be confirmed (twice asked, then done)
//   ?cs2buy=refuse     Steam refuses purchases
//   ?cs2wallet=<cents> what the wallet holds (2500)
//   ?cs2trade=refuse   Steam refuses sending and accepting offers
import { tagName } from "./cs2";
import { US_DOLLAR_WALLET, buyerPays, sellerReceives } from "./format";
import type {
  AssetRef,
  Cs2About,
  Cs2Bought,
  Cs2Buyer,
  Cs2Depth,
  Cs2Facet,
  Cs2History,
  Cs2Hit,
  Cs2Hits,
  Cs2Item,
  Cs2ItemPage,
  Cs2Listing,
  Cs2Mine,
  Cs2OrderStatus,
  Cs2Query,
  Cs2Suggestion,
  PartnerItems,
  SellOutcome,
  TradeItem,
  TradeOffer,
  TradeOffers,
  TradeOutcome,
} from "./types";

type Handlers = Record<string, (a: Record<string, any>) => unknown | Promise<unknown>>;

/** A number from 0 to 1 that is always the same for the same words. */
function chance(seed: string): number {
  let h = 2166136261;
  for (let i = 0; i < seed.length; i++) h = Math.imul(h ^ seed.charCodeAt(i), 16777619);
  return ((h >>> 0) % 100000) / 100000;
}

/** A stand-in picture: the item's initials on its grade's color, marked as a sample. */
function art(name: string, color: string): string {
  const initials = name
    .replace(/[^A-Za-z0-9 ]/g, " ")
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((w) => w[0].toUpperCase())
    .join("");
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 96 96"><rect width="96" height="96" rx="10" fill="#${color}" opacity="0.28"/><rect x="3" y="3" width="90" height="90" rx="8" fill="none" stroke="#${color}" stroke-width="2"/><text x="48" y="54" font-family="sans-serif" font-size="30" font-weight="700" text-anchor="middle" fill="#${color}">${initials}</text><text x="48" y="84" font-family="sans-serif" font-size="9" text-anchor="middle" fill="#${color}" opacity="0.8">sample art</text></svg>`;
  return `data:image/svg+xml,${encodeURIComponent(svg)}`;
}

const EXTERIORS = [
  { tag: "WearCategory0", name: "Factory New", from: 0, to: 0.07, worth: 2.4 },
  { tag: "WearCategory1", name: "Minimal Wear", from: 0.07, to: 0.15, worth: 1.5 },
  { tag: "WearCategory2", name: "Field-Tested", from: 0.15, to: 0.38, worth: 1 },
  { tag: "WearCategory3", name: "Well-Worn", from: 0.38, to: 0.45, worth: 0.9 },
  { tag: "WearCategory4", name: "Battle-Scarred", from: 0.45, to: 1, worth: 0.8 },
];

interface Thing {
  id: string;
  name: string;
  kind: string;
  color: string;
  type: string;
  weapon?: string;
  collection?: string;
  /** Sold by the piece. */
  commodity: boolean;
  /** A middle price in cents. */
  worth: number;
  /** How many are for sale. */
  stock: number;
  about: string;
}

const THINGS: Thing[] = [
  ["Kilowatt Case", "Base Grade Container", "b0c3d9", "CSGO_Type_WeaponCase", 9, 670581, "Container Series #386. Contains one item from the Kilowatt Collection."],
  ["Revolution Case", "Base Grade Container", "b0c3d9", "CSGO_Type_WeaponCase", 12, 545401, "Container Series #321. Contains one item from the Revolution Collection."],
  ["Dreams & Nightmares Case", "Base Grade Container", "b0c3d9", "CSGO_Type_WeaponCase", 82, 545238, "Container Series #315."],
  ["Fracture Case", "Base Grade Container", "b0c3d9", "CSGO_Type_WeaponCase", 31, 412004, "Container Series #307. Contains one item from the Fracture Collection."],
  ["Gallery Case", "Base Grade Container", "b0c3d9", "CSGO_Type_WeaponCase", 55, 91822, "Container Series #390."],
  ["CS:GO Case Key", "Base Grade Key", "b0c3d9", "CSGO_Tool_WeaponCase_KeyTag", 771, 1204, "This key only opens CS:GO Weapon Cases."],
  ["Operation Hydra Case Key", "Base Grade Key", "b0c3d9", "CSGO_Tool_WeaponCase_KeyTag", 277, 310, "This key only opens Operation Hydra Cases."],
  ["Sticker | Team Liquid (Holo) | Paris 2023", "Exotic Sticker", "d32ce6", "CSGO_Tool_Sticker", 240, 3105, "This sticker can be applied to any weapon you own."],
  ["Sticker | Natus Vincere | Copenhagen 2024", "High Grade Sticker", "4b69ff", "CSGO_Tool_Sticker", 6, 88120, "This sticker can be applied to any weapon you own."],
  ["Charm | Die-cast AK", "Remarkable Charm", "8847ff", "CSGO_Tool_Keychain", 310, 1740, "This charm can be attached to any weapon you own."],
  ["Music Kit | NIGHTMODE", "High Grade Music Kit", "4b69ff", "CSGO_Type_MusicKit", 506, 402, "A music kit for the main menu and your rounds."],
].map(([name, kind, color, type, worth, stock, about]) => ({
  id: `G${Math.floor(chance(name as string) * 0xfffffff).toString(16).toUpperCase()}`,
  name: name as string,
  kind: kind as string,
  color: color as string,
  type: type as string,
  commodity: true,
  worth: worth as number,
  stock: stock as number,
  about: about as string,
}));

const SKINS: [string, string, string, string, string, string, number, number][] = [
  ["AK-47 | Redline", "Classified Rifle", "d32ce6", "CSGO_Type_Rifle", "weapon_ak47", "set_community_2", 2292, 58],
  ["AK-47 | Slate", "Restricted Rifle", "8847ff", "CSGO_Type_Rifle", "weapon_ak47", "set_community_28", 350, 47],
  ["AK-47 | Ice Coaled", "Classified Rifle", "d32ce6", "CSGO_Type_Rifle", "weapon_ak47", "set_community_31", 442, 41],
  ["M4A1-S | Nightmare", "Restricted Rifle", "8847ff", "CSGO_Type_Rifle", "weapon_m4a1_silencer", "set_community_26", 610, 33],
  ["AWP | Atheris", "Restricted Sniper Rifle", "8847ff", "CSGO_Type_SniperRifle", "weapon_awp", "set_community_22", 505, 39],
  ["AWP | Chromatic Aberration", "Classified Sniper Rifle", "d32ce6", "CSGO_Type_SniperRifle", "weapon_awp", "set_community_30", 1890, 27],
  ["Glock-18 | Water Elemental", "Classified Pistol", "d32ce6", "CSGO_Type_Pistol", "weapon_glock", "set_community_4", 780, 44],
  ["USP-S | Cortex", "Classified Pistol", "d32ce6", "CSGO_Type_Pistol", "weapon_usp_silencer", "set_community_19", 640, 36],
  ["Desert Eagle | Mecha Industries", "Classified Pistol", "d32ce6", "CSGO_Type_Pistol", "weapon_deagle", "set_community_24", 930, 29],
  ["MP9 | Starlight Protector", "Covert SMG", "eb4b4b", "CSGO_Type_SMG", "weapon_mp9", "set_community_30", 1450, 22],
  ["P90 | Asiimov", "Covert SMG", "eb4b4b", "CSGO_Type_SMG", "weapon_p90", "set_community_8", 1720, 25],
  ["Nova | Hyper Beast", "Classified Shotgun", "d32ce6", "CSGO_Type_Shotgun", "weapon_nova", "set_community_11", 410, 24],
  ["Negev | Power Loader", "Restricted Machinegun", "8847ff", "CSGO_Type_Machinegun", "weapon_negev", "set_community_10", 120, 21],
  ["★ Karambit | Doppler", "★ Covert Knife", "8650ac", "CSGO_Type_Knife", "weapon_knife_karambit", "", 89000, 9],
  ["★ Butterfly Knife | Night", "★ Covert Knife", "8650ac", "CSGO_Type_Knife", "weapon_knife_butterfly", "", 61000, 8],
  ["★ Sport Gloves | Vice", "★ Extraordinary Gloves", "8650ac", "Type_Hands", "", "", 142000, 6],
];
for (const [name, kind, color, type, weapon, collection, worth, stock] of SKINS) {
  THINGS.push({
    id: `G${Math.floor(chance(name) * 0xfffffff).toString(16).toUpperCase()}`,
    name,
    kind,
    color,
    type,
    weapon: weapon || undefined,
    collection: collection || undefined,
    commodity: false,
    worth,
    stock,
    about: `${name.split("|")[0].replace("★", "").trim()} with a custom finish. Its exterior ranges from Factory New to Battle-Scarred.`,
  });
}

const FACETS: Cs2Facet[] = [
  {
    id: "Type",
    name: "Type",
    tags: [
      ["CSGO_Type_Pistol", "Pistol"],
      ["CSGO_Type_SMG", "SMG"],
      ["CSGO_Type_Rifle", "Rifle"],
      ["CSGO_Type_SniperRifle", "Sniper Rifle"],
      ["CSGO_Type_Shotgun", "Shotgun"],
      ["CSGO_Type_Machinegun", "Machinegun"],
      ["CSGO_Type_Knife", "Knife"],
      ["CSGO_Type_WeaponCase", "Container"],
      ["CSGO_Tool_Sticker", "Sticker"],
      ["Type_Hands", "Gloves"],
      ["CSGO_Tool_Keychain", "Charm"],
      ["CSGO_Type_MusicKit", "Music Kit"],
      ["CSGO_Tool_WeaponCase_KeyTag", "Key"],
      ["Type_CustomPlayer", "Agent"],
      ["CSGO_Type_Spray", "Graffiti"],
      ["CSGO_Tool_Patch", "Patch"],
    ].map(([id, name]) => ({ id, name, count: 1000, color: "" })),
  },
  { id: "Exterior", name: "Exterior", tags: [...EXTERIORS.map((e) => ({ id: e.tag, name: e.name, count: 1000, color: "" })), { id: "WearCategoryNA", name: "Not Painted", count: 12, color: "" }] },
  {
    id: "Quality",
    name: "Category",
    tags: [
      { id: "normal", name: "Normal", count: 1000, color: "B2B2B2" },
      { id: "strange", name: "StatTrak™", count: 300, color: "CF6A32" },
      { id: "tournament", name: "Souvenir", count: 100, color: "FFD700" },
    ],
  },
  {
    id: "Rarity",
    name: "Quality",
    tags: [
      { id: "Rarity_Rare_Weapon", name: "Mil-Spec Grade", count: 900, color: "4b69ff" },
      { id: "Rarity_Mythical_Weapon", name: "Restricted", count: 500, color: "8847ff" },
      { id: "Rarity_Legendary_Weapon", name: "Classified", count: 300, color: "d32ce6" },
      { id: "Rarity_Ancient_Weapon", name: "Covert", count: 120, color: "eb4b4b" },
    ],
  },
  {
    id: "ItemSet",
    name: "Collection",
    tags: [...new Set(SKINS.map((s) => s[5]).filter(Boolean))].sort().map((id) => ({ id, name: `The ${id.replace("set_community_", "Community ")} Collection`, count: 40, color: "" })),
  },
  { id: "Tournament", name: "Tournament", tags: [{ id: "Tournament21", name: "2023 BLAST.tv Paris", count: 60, color: "" }, { id: "Tournament22", name: "2024 PGL Copenhagen", count: 80, color: "" }] },
  { id: "StickerCategory", name: "Sticker Type", tags: [{ id: "TeamLogo", name: "Team Logo", count: 900, color: "" }, { id: "PlayerSignature", name: "Player Autograph", count: 4000, color: "" }, { id: "Tournament", name: "Tournament", count: 200, color: "" }] },
  {
    id: "Weapon",
    name: "Weapon",
    tags: [...new Set(SKINS.map((s) => s[4]).filter(Boolean))].map((id) => ({ id, name: tagName(null, "Weapon", id), count: 100, color: "" })),
  },
];

interface Sample extends Cs2Listing {
  exterior: string;
  quality: string;
}

/** The listings of a skin: the same every time, cheapest first. */
function listingsOf(t: Thing): Sample[] {
  const out: Sample[] = [];
  for (let i = 0; i < t.stock; i++) {
    const seed = `${t.name}#${i}`;
    const ext = EXTERIORS[Math.min(4, Math.floor(chance(`${seed}e`) * 5.4))];
    const stattrak = chance(`${seed}s`) < 0.16;
    const float = ext.from + chance(`${seed}f`) * (ext.to - ext.from);
    const total = Math.max(3, Math.round(t.worth * ext.worth * (stattrak ? 2.6 : 1) * (0.94 + chance(`${seed}p`) * 0.5)));
    const price = sellerReceives(total, US_DOLLAR_WALLET);
    const hashName = `${stattrak ? (t.name.startsWith("★") ? "★ StatTrak™ " : "StatTrak™ ") : ""}${stattrak ? t.name.replace("★ ", "") : t.name} (${ext.name})`;
    const stuck = chance(`${seed}k`) < 0.3;
    out.push({
      listingId: `${5_000_000_000_000 + Math.floor(chance(seed) * 1e12)}${i}`,
      assetId: `${40_000_000_000 + Math.floor(chance(`${seed}a`) * 1e9)}`,
      hashName,
      variant: `${stattrak ? "StatTrak™ " : ""}${ext.name}`,
      name: hashName,
      color: t.color,
      price,
      fee: buyerPays(price, US_DOLLAR_WALLET) - price,
      total: buyerPays(price, US_DOLLAR_WALLET),
      currency: 1,
      priceText: `$${(buyerPays(price, US_DOLLAR_WALLET) / 100).toLocaleString("en-US", { minimumFractionDigits: 2 })}`,
      float,
      pattern: Math.floor(chance(`${seed}n`) * 1000),
      inspect: `steam://run/730//+csgo_econ_action_preview%20${Math.floor(chance(seed) * 1e15).toString(16).toUpperCase()}`,
      stickers: stuck ? [{ name: "Sticker | Team Liquid (Holo) | Paris 2023", kind: "Exotic Sticker", iconUrl: art("Team Liquid", "d32ce6"), wear: chance(`${seed}w`) < 0.5 ? 0 : 0.4 }] : [],
      image: null,
      iconUrl: art(t.name, t.color),
      mine: false,
      exterior: ext.tag,
      quality: stattrak ? "strange" : "normal",
    });
  }
  return out.sort((a, b) => a.total - b.total);
}

function history(t: Thing, name: string): Cs2History {
  const now = Math.floor(Date.now() / 1000);
  const hour = now - (now % 3600);
  const points: Cs2History["points"] = [];
  let price = t.worth * (0.7 + chance(`${name}h`) * 0.3);
  for (let d = 420; d > 30; d--) {
    price = Math.max(3, price * (0.97 + chance(`${name}d${d}`) * 0.062));
    points.push([hour - d * 86400, Math.round(price), Math.round(20 + chance(`${name}v${d}`) * 400)]);
  }
  for (let h = 30 * 24; h >= 0; h--) {
    price = Math.max(3, price * (0.992 + chance(`${name}t${h}`) * 0.0163));
    points.push([hour - h * 3600, Math.round(price), Math.round(1 + chance(`${name}u${h}`) * 25)]);
  }
  return { currency: 1, points };
}

export function cs2Mock(params: URLSearchParams, me: string): { handlers: Handlers; removeListings: (listingIds: string[]) => void } {
  const delay = (ms: number) => new Promise((r) => setTimeout(r, ms));
  const buying = params.get("cs2buy");
  let balance = Number(params.get("cs2wallet") ?? 2500);
  const sold = new Map<string, Sample[]>();
  const market = (t: Thing) => {
    if (!sold.has(t.id)) sold.set(t.id, listingsOf(t));
    return sold.get(t.id)!;
  };
  const byId = (id: string) => THINGS.find((t) => t.id === id || t.name === id);
  const byHash = (hash: string) => THINGS.find((t) => t.name === hash || (!t.commodity && hash.replace("StatTrak™ ", "").replace(/ \([^)]*\)$/, "").replace("★ ★", "★") === t.name));

  const depthOf = (hash: string): Cs2Depth | null => {
    const t = byHash(hash);
    if (!t) return null;
    if (t.commodity) {
      const low = t.worth;
      return {
        currency: 1,
        lowestSell: low,
        highestBuy: Math.max(1, low - Math.max(1, Math.round(low * 0.04))),
        sellCount: t.stock,
        buyCount: Math.round(t.stock * 0.8),
        sells: Array.from({ length: 12 }, (_, i) => ({ price: low + i * Math.max(1, Math.round(low * 0.01)), quantity: 3 + Math.round(chance(`${hash}s${i}`) * 400) })),
        buys: Array.from({ length: 12 }, (_, i) => ({ price: Math.max(1, low - (i + 1) * Math.max(1, Math.round(low * 0.02))), quantity: 5 + Math.round(chance(`${hash}b${i}`) * 900) })),
        moreSells: 260,
        moreBuys: 140,
      };
    }
    const mine = market(t).filter((l) => l.hashName === hash);
    const levels = new Map<number, number>();
    for (const l of mine) levels.set(l.total, (levels.get(l.total) ?? 0) + 1);
    const low = mine[0]?.total ?? null;
    const base = low ?? t.worth;
    return {
      currency: 1,
      lowestSell: low,
      highestBuy: Math.max(3, Math.round(base * 0.93)),
      sellCount: mine.length,
      buyCount: 40 + Math.round(chance(hash) * 900),
      sells: [...levels].map(([price, quantity]) => ({ price, quantity })),
      buys: Array.from({ length: 10 }, (_, i) => ({ price: Math.max(3, Math.round(base * (0.93 - i * 0.012))), quantity: 1 + Math.round(chance(`${hash}b${i}`) * 12) })),
      moreSells: 0,
      moreBuys: 30,
    };
  };

  const item = (asset: string, name: string, kind: string, color: string, more: Partial<Cs2Item> = {}): Cs2Item => ({
    assetId: asset,
    context: "2",
    classId: String(Math.floor(chance(name) * 1e9)),
    instanceId: "0",
    hashName: name,
    name,
    kind,
    iconUrl: art(name, color),
    color,
    exterior: "",
    marketable: true,
    tradable: true,
    protected: false,
    commodity: false,
    float: null,
    pattern: null,
    notes: [],
    ...more,
  });
  let inventory: Cs2Item[] = [
    item("5001", "AK-47 | Slate (Field-Tested)", "Restricted Rifle", "8847ff", { float: 0.2231, pattern: 661, exterior: "Field-Tested" }),
    item("5002", "Glock-18 | Water Elemental (Minimal Wear)", "Classified Pistol", "d32ce6", { float: 0.1104, pattern: 88, exterior: "Minimal Wear" }),
    item("5003", "AWP | Atheris (Battle-Scarred)", "Restricted Sniper Rifle", "8847ff", { float: 0.6123, pattern: 402, exterior: "Battle-Scarred" }),
    item("5004", "StatTrak™ P90 | Asiimov (Field-Tested)", "StatTrak™ Covert SMG", "eb4b4b", { float: 0.2987, pattern: 17, exterior: "Field-Tested" }),
    ...["5010", "5011", "5012", "5013", "5014"].map((id) => item(id, "Fracture Case", "Base Grade Container", "b0c3d9", { commodity: true })),
    ...["5020", "5021"].map((id) => item(id, "Kilowatt Case", "Base Grade Container", "b0c3d9", { commodity: true })),
    item("5030", "Sticker | Natus Vincere | Copenhagen 2024", "High Grade Sticker", "4b69ff", { commodity: true }),
    item("5040", "Nova | Hyper Beast (Well-Worn)", "Classified Shotgun", "d32ce6", {
      float: 0.4012,
      pattern: 240,
      exterior: "Well-Worn",
      marketable: false,
      tradable: false,
      notes: ["Tradable/Marketable After Dec 04, 2026 (8:00:00) GMT"],
    }),
    item("5050", "Negev | Power Loader (Factory New)", "Restricted Machinegun", "8847ff", { float: 0.0312, pattern: 5, exterior: "Factory New", context: "16", protected: true, marketable: false, tradable: false }),
    item("5060", "Music Kit | NIGHTMODE", "High Grade Music Kit", "4b69ff", { commodity: true, marketable: false, notes: ["This item can't be sold on the market"] }),
  ];
  const mine: Cs2Mine = {
    listings: [
      { listingId: "7001", hashName: "Revolution Case", name: "Revolution Case", iconUrl: art("Revolution Case", "b0c3d9"), color: "b0c3d9", buyerPays: 14, youReceive: 12, currency: 1, status: "active", created: Date.now() / 1000 - 86400 },
      { listingId: "7002", hashName: "USP-S | Cortex (Field-Tested)", name: "USP-S | Cortex (Field-Tested)", iconUrl: art("USP-S | Cortex", "d32ce6"), color: "d32ce6", buyerPays: 690, youReceive: 600, currency: 1, status: "pending", created: Date.now() / 1000 - 600 },
    ],
    orders: [{ orderId: "8001", hashName: "Gallery Case", name: "Gallery Case", iconUrl: art("Gallery Case", "b0c3d9"), color: "b0c3d9", price: 48, quantity: 5, remaining: 4, currency: 1 }],
  };
  const orders = new Map<string, Cs2OrderStatus>();
  let confirmations = 0;

  const tradeItem = (i: Cs2Item): TradeItem => ({ app: 730, context: i.context, assetId: i.assetId, name: i.name, kind: i.kind, iconUrl: i.iconUrl, color: i.color, missing: false });
  const friend = "76561198006409530";
  const stranger = "76561198123409911";
  const theirs: Cs2Item[] = [
    item("9001", "AK-47 | Redline (Field-Tested)", "Classified Rifle", "d32ce6", { float: 0.2513, pattern: 310, exterior: "Field-Tested" }),
    item("9002", "M4A1-S | Nightmare (Minimal Wear)", "Restricted Rifle", "8847ff", { float: 0.0931, pattern: 77, exterior: "Minimal Wear" }),
    item("9003", "Desert Eagle | Mecha Industries (Factory New)", "Classified Pistol", "d32ce6", { float: 0.0411, pattern: 912, exterior: "Factory New" }),
    ...["9010", "9011", "9012"].map((id) => item(id, "Dreams & Nightmares Case", "Base Grade Container", "b0c3d9", { commodity: true })),
    item("9020", "Charm | Die-cast AK", "Remarkable Charm", "8847ff", { commodity: true }),
  ];
  const nowS = Math.floor(Date.now() / 1000);
  const offers: TradeOffers = {
    received: [
      {
        id: "6001",
        partner: friend,
        message: "my Redline for your Slate and a case?",
        state: 2,
        stateText: "Active",
        ours: false,
        created: nowS - 3600,
        updated: nowS - 3600,
        expires: nowS + 13 * 86400,
        give: [tradeItem(inventory[0]), tradeItem(inventory[4])],
        receive: [tradeItem(theirs[0])],
        heldUntil: 0,
      },
      {
        id: "6002",
        partner: stranger,
        message: "send first for verification, will return x2",
        state: 2,
        stateText: "Active",
        ours: false,
        created: nowS - 7200,
        updated: nowS - 7200,
        expires: nowS + 12 * 86400,
        give: [tradeItem(inventory[3])],
        receive: [],
        heldUntil: 0,
      },
    ],
    sent: [
      {
        id: "6003",
        partner: friend,
        message: "",
        state: 9,
        stateText: "Waiting to be confirmed",
        ours: true,
        created: nowS - 900,
        updated: nowS - 900,
        expires: nowS + 14 * 86400,
        give: [tradeItem(inventory[9])],
        receive: [tradeItem(theirs[3])],
        heldUntil: 0,
      },
    ],
  };
  const refuseTrade = params.get("cs2trade") === "refuse";

  /** Steam's answer to a purchase, by the preview's scenario. */
  const answer = (confirmation: string | null, done: () => Cs2Bought): Cs2Bought => {
    const none: Cs2Bought = { ok: false, needsConfirmation: false, confirmationId: null, orderId: null, message: "", balance: null };
    if (buying === "refuse") return { ...none, message: "The listing you tried to purchase is no longer available." };
    if (buying === "confirm") {
      if (!confirmation) {
        confirmations = 0;
        return { ...none, needsConfirmation: true, confirmationId: "8817263544" };
      }
      if (++confirmations < 2) return { ...none, needsConfirmation: true, confirmationId: confirmation };
    }
    return done();
  };

  const handlers: Handlers = {
    cs2_facets: async () => {
      await delay(300);
      if (params.get("cs2facets") === "down") throw "GET steamcommunity.com/market/appfacets/730: couldn't connect";
      return FACETS;
    },
    cs2_search: async ({ query }): Promise<Cs2Hits> => {
      const q = query as Cs2Query;
      await delay(450);
      const words = q.text.toLowerCase();
      const f = q.filters;
      const other = params.get("cs2search") === "other";
      let found = THINGS.filter((t) => {
        if (words && !t.name.toLowerCase().includes(words) && !(q.descriptions && t.about.toLowerCase().includes(words))) return false;
        if (f.Type?.length && !f.Type.includes(t.type)) return false;
        if (f.Weapon?.length && !(t.weapon && f.Weapon.includes(t.weapon))) return false;
        if (f.ItemSet?.length && !(t.collection && f.ItemSet.includes(t.collection))) return false;
        if ((f.Exterior?.length || f.Quality?.length) && t.commodity) return false;
        return true;
      }).map((t): Cs2Hit & { rank: number } => {
        const cheapest = t.commodity ? null : market(t).find((l) => (!f.Exterior?.length || f.Exterior.includes(l.exterior)) && (!f.Quality?.length || f.Quality.includes(l.quality)));
        const price = t.commodity ? t.worth : (cheapest?.total ?? null);
        return {
          id: t.id,
          hashName: t.commodity ? t.name : (cheapest?.hashName ?? `${t.name} (Field-Tested)`),
          name: t.name,
          kind: t.kind,
          iconUrl: art(t.name, t.color),
          color: t.color,
          commodity: t.commodity,
          grouped: !t.commodity,
          listings: t.commodity ? t.stock : market(t).length,
          currency: other ? 31 : 1,
          price,
          priceText: price === null ? "" : other ? `${((price * 3.75) / 100).toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 })} SR` : `$${(price / 100).toLocaleString("en-US", { minimumFractionDigits: 2 })}`,
          rank: chance(`${t.name}trend`),
        };
      });
      found = found.filter((h) => h.price !== null && (!q.priceMin || h.price >= q.priceMin) && (!q.priceMax || h.price <= q.priceMax));
      const up = q.direction === 1 ? 1 : -1;
      found.sort((a, b) =>
        q.sort === 1 ? a.name.localeCompare(b.name) * up : q.sort === 2 ? (a.listings - b.listings) * up : q.sort === 3 ? ((a.price ?? 0) - (b.price ?? 0)) * up : b.rank - a.rank,
      );
      return {
        total: found.length,
        start: q.start,
        results: found.slice(q.start, q.start + q.count).map(({ rank: _rank, ...hit }) => hit),
        inDescriptions: q.descriptions && words !== "",
        walletCurrency: 1,
      };
    },
    cs2_suggest: async ({ typed }): Promise<Cs2Suggestion[]> => {
      await delay(150);
      const words = String(typed).toLowerCase();
      return THINGS.filter((t) => t.name.toLowerCase().includes(words))
        .slice(0, 8)
        .map((t) => ({ id: t.id, name: t.name, iconUrl: art(t.name, t.color) }));
    },
    cs2_item: async ({ id, filters, start }): Promise<Cs2ItemPage> => {
      await delay(500);
      const t = byId(id);
      if (!t) throw "Steam gave no answer for this item's listings, three times in a row. Try again in a moment";
      if (t.commodity) return { total: 1, start: 0, more: true, listings: [], facets: [] };
      const all = market(t);
      const f = (filters ?? {}) as Record<string, string[]>;
      const shown = all.filter((l) => (!f.Exterior?.length || f.Exterior.includes(l.exterior)) && (!f.Quality?.length || f.Quality.includes(l.quality)));
      const tally = (facet: string, facetName: string, pick: (l: Sample) => string) =>
        [...new Set(all.map(pick))].map((tag) => ({
          facet,
          facetName,
          tag,
          tagName: FACETS.find((x) => x.id === facet)?.tags.find((x) => x.id === tag)?.name ?? tag,
          color: FACETS.find((x) => x.id === facet)?.tags.find((x) => x.id === tag)?.color ?? "",
          listings: all.filter((l) => pick(l) === tag).length,
        }));
      return {
        total: shown.length,
        start,
        more: start + 20 < shown.length,
        listings: shown.slice(start, start + 20).map(({ exterior: _e, quality: _q, ...l }) => l),
        facets: [...tally("Exterior", "Exterior", (l) => l.exterior), ...tally("Quality", "Category", (l) => l.quality)],
      };
    },
    cs2_about: async ({ hashName }): Promise<Cs2About | null> => {
      await delay(200);
      const t = byHash(hashName);
      if (!t) return null;
      const variant = /\(([^)]*)\)$/.exec(hashName)?.[1] ?? "";
      return {
        hashName,
        name: hashName,
        kind: t.kind,
        iconUrl: art(t.name, t.color),
        color: t.color,
        commodity: t.commodity,
        groupId: t.id,
        groupName: t.name,
        variant,
        lines: t.commodity ? [t.about] : [`Exterior: ${variant}`, t.about, "Never be afraid to push it to the limit", "Sample text: this item exists only in the preview."],
        holdDays: 7,
      };
    },
    cs2_book: async ({ hashName }) => {
      await delay(350);
      return depthOf(hashName);
    },
    cs2_history: async ({ hashName }) => {
      await delay(400);
      const t = byHash(hashName);
      return t ? history(t, hashName) : null;
    },
    cs2_buyer: async (): Promise<Cs2Buyer> => {
      await delay(250);
      return { currency: 1, balance, country: "US", needsBilling: false, hasBilling: false, confirmation: buying === "confirm" ? 2 : 0, taxPercent: 0, wallet: US_DOLLAR_WALLET };
    },
    cs2_buy_listing: async ({ listing, confirmation }): Promise<Cs2Bought> => {
      await delay(700);
      const total = listing.price + listing.fee;
      if (total > balance) throw `The wallet has USD ${(balance / 100).toFixed(2)} and this costs USD ${(total / 100).toFixed(2)}`;
      return answer(confirmation, () => {
        for (const list of sold.values()) {
          const at = list.findIndex((l) => l.listingId === listing.listingId);
          if (at >= 0) {
            const [l] = list.splice(at, 1);
            inventory = [item(l.assetId, l.hashName, byHash(l.hashName)?.kind ?? "", l.color, { float: l.float, pattern: l.pattern, marketable: false, tradable: false, notes: ["Tradable/Marketable in 7 days"] }), ...inventory];
          }
        }
        balance -= total;
        return { ok: true, needsConfirmation: false, confirmationId: null, orderId: null, message: "", balance };
      });
    },
    cs2_buy_order: async ({ hashName, priceEach, quantity, confirmation }): Promise<Cs2Bought> => {
      await delay(700);
      return answer(confirmation, () => {
        const orderId = String(8100 + orders.size);
        const d = depthOf(hashName);
        const fills = d?.lowestSell != null && priceEach >= d.lowestSell && d.lowestSell * quantity <= balance;
        if (fills) balance -= d!.lowestSell! * quantity;
        else mine.orders.push({ orderId, hashName, name: hashName, iconUrl: art(hashName, byHash(hashName)?.color ?? "b0c3d9"), color: byHash(hashName)?.color ?? "", price: priceEach, quantity, remaining: quantity, currency: 1 });
        orders.set(orderId, {
          active: !fills,
          quantity,
          remaining: fills ? 0 : quantity,
          bought: fills ? quantity : 0,
          paidText: fills ? `$${((d!.lowestSell! * quantity) / 100).toFixed(2)}` : "",
        });
        return { ok: true, needsConfirmation: false, confirmationId: null, orderId, message: "", balance: null };
      });
    },
    cs2_order_status: async ({ orderId }) => {
      await delay(200);
      const s = orders.get(orderId);
      if (!s) throw "Steam doesn't know this buy order (it may be filled or cancelled)";
      return s;
    },
    cs2_cancel_order: async ({ orderId }) => {
      await delay(400);
      mine.orders = mine.orders.filter((o) => o.orderId !== orderId);
    },
    cs2_mine: async () => {
      await delay(450);
      return structuredClone(mine);
    },
    cs2_inventory: async () => {
      await delay(500);
      return structuredClone(inventory);
    },
    cs2_sell: async ({ items }): Promise<SellOutcome[]> => {
      await delay(600);
      return (items as { assetId: string; hashName: string; buyerCents: number }[]).map((it) => {
        const had = inventory.find((i) => i.assetId === it.assetId);
        inventory = inventory.filter((i) => i.assetId !== it.assetId);
        mine.listings.push({
          listingId: `7${it.assetId}`,
          hashName: it.hashName,
          name: it.hashName,
          iconUrl: had?.iconUrl ?? "",
          color: had?.color ?? "",
          buyerPays: it.buyerCents,
          youReceive: sellerReceives(it.buyerCents, US_DOLLAR_WALLET),
          currency: 1,
          status: "pending",
          created: Date.now() / 1000,
        });
        return { assetId: it.assetId, ok: true, needsConfirmation: true, message: "" };
      });
    },

    trade_offers: async () => {
      await delay(450);
      return structuredClone(offers);
    },
    trade_partner_items: async ({ link }): Promise<PartnerItems> => {
      await delay(600);
      const text = String(link).trim();
      if (!/^\d{17}$/.test(text) && !/^https:\/\/steamcommunity\.com\/tradeoffer\/new\/?\?partner=\d+/.test(text))
        throw "That isn't a trade link: it should start with https://steamcommunity.com/tradeoffer/new/";
      if (text === me) throw "That is this account's own trade link";
      return { partner: friend, items: structuredClone(theirs) };
    },
    trade_send: async ({ give, receive, message }): Promise<TradeOutcome> => {
      await delay(800);
      if (refuseTrade)
        return {
          ok: false,
          offerId: null,
          needsConfirmation: false,
          byEmail: false,
          message:
            "There was an error sending your trade offer. Please try again later. (15) Usually this means the other player can't trade right now, their inventory is full, or the trade link is no longer good.",
        };
      const id = String(6100 + offers.sent.length);
      const pick = (refs: AssetRef[], from: Cs2Item[]) => refs.map((r) => from.find((i) => i.assetId === r.assetId)).filter((i): i is Cs2Item => !!i).map(tradeItem);
      const needs = (give as AssetRef[]).length > 0;
      const sent: TradeOffer = {
        id,
        partner: friend,
        message,
        state: needs ? 9 : 2,
        stateText: needs ? "Waiting to be confirmed" : "Active",
        ours: true,
        created: Date.now() / 1000,
        updated: Date.now() / 1000,
        expires: Date.now() / 1000 + 14 * 86400,
        give: pick(give, inventory),
        receive: pick(receive, theirs),
        heldUntil: 0,
      };
      offers.sent.unshift(sent);
      return { ok: true, offerId: id, needsConfirmation: needs, byEmail: false, message: "" };
    },
    trade_accept: async ({ offerId }): Promise<TradeOutcome> => {
      await delay(800);
      if (refuseTrade) return { ok: false, offerId: null, needsConfirmation: false, byEmail: false, message: "This trade offer is no longer valid. (28) Usually this means the offer has already been answered." };
      const offer = offers.received.find((o) => o.id === offerId);
      offers.received = offers.received.filter((o) => o.id !== offerId);
      return { ok: true, offerId: null, needsConfirmation: (offer?.give.length ?? 0) > 0, byEmail: false, message: "" };
    },
    trade_withdraw: async ({ offerId }) => {
      await delay(400);
      offers.received = offers.received.filter((o) => o.id !== offerId);
      offers.sent = offers.sent.filter((o) => o.id !== offerId);
    },
  };
  return {
    handlers,
    // Taking back a listing goes through the market's own command: the CS2 ones are kept here.
    removeListings: (listingIds) => {
      mine.listings = mine.listings.filter((l) => !listingIds.includes(l.listingId));
    },
  };
}
