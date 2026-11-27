// The Counter-Strike 2 market: what its search can be narrowed by (laid out
// the way Steam's own market page lays it out), and what the page keeps
// between visits: the filters Steam lists, the CS2 inventory, the account's
// listings and buy orders, its trade offers and its wallet.
import { create } from "zustand";
import { api } from "./api";
import { useApp } from "./store";
import type { Cs2Buyer, Cs2Facet, Cs2Item, Cs2Mine, Cs2Query, TradeOffers } from "./types";

const KNIVES = [
  "weapon_knife_push",
  "weapon_knife_survival_bowie",
  "weapon_knife_gypsy_jackknife",
  "weapon_knife_falchion",
  "weapon_knife_tactical",
  "weapon_knife_gut",
  "weapon_knife_kukri",
  "weapon_knife_canis",
  "weapon_knife_cord",
  "weapon_knife_ursus",
  "weapon_knife_flip",
  "weapon_knife_outdoor",
  "weapon_knife_css",
  "weapon_knife_skeleton",
  "weapon_knife_stiletto",
  "weapon_knife_widowmaker",
  "weapon_knife_butterfly",
  "weapon_knife_m9_bayonet",
  "weapon_knife_karambit",
  "weapon_bayonet",
];

/** The weapon classes of Steam's CS2 search, each with the weapons it offers, in Steam's order. */
export const CLASSES: { type: string; label: string; all: string; weapons: string[] }[] = [
  {
    type: "CSGO_Type_Pistol",
    label: "Pistol",
    all: "All pistols",
    weapons: ["weapon_p250", "weapon_tec9", "weapon_fiveseven", "weapon_revolver", "weapon_deagle", "weapon_glock", "weapon_usp_silencer", "weapon_elite", "weapon_hkp2000", "weapon_cz75a"],
  },
  { type: "CSGO_Type_SMG", label: "SMG", all: "All SMGs", weapons: ["weapon_bizon", "weapon_mac10", "weapon_mp7", "weapon_ump45", "weapon_mp9", "weapon_mp5sd", "weapon_p90"] },
  {
    type: "CSGO_Type_Rifle",
    label: "Rifle",
    all: "All rifles",
    weapons: ["weapon_aug", "weapon_ak47", "weapon_sg556", "weapon_m4a1_silencer", "weapon_galilar", "weapon_famas", "weapon_m4a1"],
  },
  { type: "CSGO_Type_SniperRifle", label: "Sniper Rifle", all: "All sniper rifles", weapons: ["weapon_ssg08", "weapon_scar20", "weapon_g3sg1", "weapon_awp"] },
  { type: "CSGO_Type_Shotgun", label: "Shotgun", all: "All shotguns", weapons: ["weapon_mag7", "weapon_nova", "weapon_sawedoff", "weapon_xm1014"] },
  { type: "CSGO_Type_Machinegun", label: "Machinegun", all: "All machineguns", weapons: ["weapon_negev", "weapon_m249"] },
  { type: "CSGO_Type_Knife", label: "Knife", all: "All knives", weapons: KNIVES },
];

/** The kinds Steam's page offers with one click. */
export const QUICK_TYPES: { type: string; label: string }[] = [
  { type: "CSGO_Type_WeaponCase", label: "Container" },
  { type: "CSGO_Tool_Sticker", label: "Sticker" },
  { type: "Type_Hands", label: "Gloves" },
];

const GUNS = ["CSGO_Type_Machinegun", "CSGO_Type_Pistol", "CSGO_Type_Rifle", "CSGO_Type_SMG", "CSGO_Type_Shotgun", "CSGO_Type_SniperRifle"];
const WEAPON_TYPES = ["CSGO_Type_Knife", ...GUNS];

/** Which further filters a choice opens up, as on Steam's page: Exterior only makes sense for a skin, a sticker's team only for stickers. */
const DEPENDENT: { facet: string; whenWeapon?: boolean; whenCollection?: boolean; types: string[] }[] = [
  { facet: "Exterior", whenWeapon: true, whenCollection: true, types: [...WEAPON_TYPES, "Type_Hands"] },
  { facet: "Quality", whenWeapon: true, whenCollection: true, types: [...WEAPON_TYPES, "CSGO_Type_MusicKit"] },
  { facet: "Rarity", whenWeapon: true, whenCollection: true, types: [...WEAPON_TYPES, "CSGO_Tool_Keychain", "CSGO_Tool_Sticker", "Type_CustomPlayer"] },
  { facet: "Tournament", types: ["CSGO_Type_WeaponCase"] },
  { facet: "StickerCategory", types: ["CSGO_Tool_Sticker"] },
  { facet: "StickerCapsule", types: ["CSGO_Tool_Sticker"] },
  { facet: "TournamentTeam", types: ["CSGO_Tool_Sticker"] },
  { facet: "KeychainCapsule", types: ["CSGO_Tool_Keychain"] },
  { facet: "PatchCategory", types: ["CSGO_Tool_Patch"] },
  { facet: "PatchCapsule", types: ["CSGO_Tool_Patch"] },
  { facet: "SprayCategory", types: ["CSGO_Type_Spray"] },
  { facet: "SprayCapsule", types: ["CSGO_Type_Spray"] },
  { facet: "SprayColorCategory", types: ["CSGO_Type_Spray"] },
];

/** The further filters to offer for what is picked so far. */
export function dependentFacets(filters: Record<string, string[]>): string[] {
  const types = filters.Type ?? [];
  const weapon = (filters.Weapon ?? []).length > 0;
  const collection = (filters.ItemSet ?? []).length > 0;
  return DEPENDENT.filter((d) => (d.whenWeapon && weapon) || (d.whenCollection && collection) || types.some((t) => d.types.includes(t))).map((d) => d.facet);
}

/** Filters without the ones that no longer apply to what is picked. */
export function pruned(filters: Record<string, string[]>): Record<string, string[]> {
  const open = new Set(["Type", "Weapon", "ItemSet", ...dependentFacets(filters)]);
  return Object.fromEntries(Object.entries(filters).filter(([facet, tags]) => open.has(facet) && tags.length > 0));
}

/** English names for when Steam's own list of filters can't be read. */
const BUILT_IN_NAMES: Record<string, string> = {
  CSGO_Type_Pistol: "Pistol",
  CSGO_Type_SMG: "SMG",
  CSGO_Type_Rifle: "Rifle",
  CSGO_Type_SniperRifle: "Sniper Rifle",
  CSGO_Type_Shotgun: "Shotgun",
  CSGO_Type_Machinegun: "Machinegun",
  CSGO_Type_Knife: "Knife",
  CSGO_Type_WeaponCase: "Container",
  CSGO_Tool_Sticker: "Sticker",
  Type_Hands: "Gloves",
  Type_CustomPlayer: "Agent",
  CSGO_Tool_Keychain: "Charm",
  CSGO_Type_Spray: "Graffiti",
  CSGO_Type_MusicKit: "Music Kit",
  CSGO_Tool_Patch: "Patch",
  CSGO_Type_Collectible: "Collectible",
  CSGO_Type_Ticket: "Pass",
  CSGO_Tool_WeaponCase_KeyTag: "Key",
  CSGO_Tool_GiftTag: "Gift",
  CSGO_Tool_Name_TagTag: "Tag",
  CSGO_Type_Tool: "Tool",
  CSGO_Type_Equipment: "Equipment",
  weapon_p250: "P250",
  weapon_tec9: "Tec-9",
  weapon_fiveseven: "Five-SeveN",
  weapon_revolver: "R8 Revolver",
  weapon_deagle: "Desert Eagle",
  weapon_glock: "Glock-18",
  weapon_usp_silencer: "USP-S",
  weapon_elite: "Dual Berettas",
  weapon_hkp2000: "P2000",
  weapon_cz75a: "CZ75-Auto",
  weapon_bizon: "PP-Bizon",
  weapon_mac10: "MAC-10",
  weapon_mp7: "MP7",
  weapon_ump45: "UMP-45",
  weapon_mp9: "MP9",
  weapon_mp5sd: "MP5-SD",
  weapon_p90: "P90",
  weapon_aug: "AUG",
  weapon_ak47: "AK-47",
  weapon_sg556: "SG 553",
  weapon_m4a1_silencer: "M4A1-S",
  weapon_galilar: "Galil AR",
  weapon_famas: "FAMAS",
  weapon_m4a1: "M4A4",
  weapon_ssg08: "SSG 08",
  weapon_scar20: "SCAR-20",
  weapon_g3sg1: "G3SG1",
  weapon_awp: "AWP",
  weapon_mag7: "MAG-7",
  weapon_nova: "Nova",
  weapon_sawedoff: "Sawed-Off",
  weapon_xm1014: "XM1014",
  weapon_negev: "Negev",
  weapon_m249: "M249",
  weapon_knife_push: "Shadow Daggers",
  weapon_knife_survival_bowie: "Bowie Knife",
  weapon_knife_gypsy_jackknife: "Navaja Knife",
  weapon_knife_falchion: "Falchion Knife",
  weapon_knife_tactical: "Huntsman Knife",
  weapon_knife_gut: "Gut Knife",
  weapon_knife_kukri: "Kukri Knife",
  weapon_knife_canis: "Survival Knife",
  weapon_knife_cord: "Paracord Knife",
  weapon_knife_ursus: "Ursus Knife",
  weapon_knife_flip: "Flip Knife",
  weapon_knife_outdoor: "Nomad Knife",
  weapon_knife_css: "Classic Knife",
  weapon_knife_skeleton: "Skeleton Knife",
  weapon_knife_stiletto: "Stiletto Knife",
  weapon_knife_widowmaker: "Talon Knife",
  weapon_knife_butterfly: "Butterfly Knife",
  weapon_knife_m9_bayonet: "M9 Bayonet",
  weapon_knife_karambit: "Karambit",
  weapon_bayonet: "Bayonet",
};

/** What a tag is called: by Steam's own list when it was read, else by the names built in. */
export function tagName(facets: Cs2Facet[] | null, facet: string, tag: string): string {
  const found = facets?.find((f) => f.id === facet)?.tags.find((t) => t.id === tag)?.name;
  return found ?? BUILT_IN_NAMES[tag] ?? tag.replace(/^weapon_|^CSGO_(Type|Tool)_|^Type_/, "").replace(/_/g, " ");
}

/** The other kinds of item, beyond the classes and the one-click kinds: everything else Steam's Type filter lists. */
export function otherTypes(facets: Cs2Facet[] | null): string[] {
  const taken = new Set([...CLASSES.map((c) => c.type), ...QUICK_TYPES.map((q) => q.type)]);
  const all = facets?.find((f) => f.id === "Type")?.tags.map((t) => t.id) ?? Object.keys(BUILT_IN_NAMES).filter((k) => !k.startsWith("weapon_"));
  return all.filter((t) => !taken.has(t)).sort((a, b) => tagName(facets, "Type", a).localeCompare(tagName(facets, "Type", b)));
}

/** Grades from the most common to the rarest, the order Steam's own filter shows them in. */
const RARITY_ORDER = ["Common", "Uncommon", "Rare", "Mythical", "Legendary", "Ancient", "Contraband"];

/** A filter's tags in the order that reads best: wear from new to scarred, grades from common to rare, the rest by name. */
export function orderedTags(facet: Cs2Facet): Cs2Facet["tags"] {
  if (facet.id === "Exterior") return [...facet.tags].sort((a, b) => a.id.localeCompare(b.id));
  if (facet.id === "Rarity") {
    const rank = (id: string) => {
      const at = RARITY_ORDER.findIndex((r) => id.startsWith(`Rarity_${r}`));
      return (at < 0 ? 99 : at) * 10 + (id.endsWith("_Weapon") ? 0 : id.endsWith("_Character") ? 2 : 1);
    };
    return [...facet.tags].sort((a, b) => rank(a.id) - rank(b.id));
  }
  return facet.tags;
}

/** Steam's sort choices for the search: column and direction, in the order its page offers them. */
export const SORTS: { value: string; label: string; sort: number; direction: number }[] = [
  { value: "5-2", label: "Trending", sort: 5, direction: 2 },
  { value: "4-2", label: "Popular", sort: 4, direction: 2 },
  { value: "3-1", label: "Price: lowest first", sort: 3, direction: 1 },
  { value: "3-2", label: "Price: highest first", sort: 3, direction: 2 },
  { value: "2-1", label: "Quantity: fewest first", sort: 2, direction: 1 },
  { value: "2-2", label: "Quantity: most first", sort: 2, direction: 2 },
  { value: "1-1", label: "Name: A to Z", sort: 1, direction: 1 },
  { value: "1-2", label: "Name: Z to A", sort: 1, direction: 2 },
];

/** How many results one answer of the search is asked for. */
export const RESULTS_PER_PAGE = 50;

/** What the search form holds. */
export interface SearchForm {
  text: string;
  descriptions: boolean;
  filters: Record<string, string[]>;
  sort: string;
  /** Typed as amounts in the wallet's currency ("2.50"); empty for no limit. */
  priceMin: string;
  priceMax: string;
}

export const EMPTY_FORM: SearchForm = { text: "", descriptions: false, filters: {}, sort: "5-2", priceMin: "", priceMax: "" };

/** An amount typed by hand, in minor units; null when it isn't one. */
export function typedAmount(text: string): number | null {
  const t = text.trim().replace(",", ".");
  if (!/^\d+(\.\d{0,2})?$/.test(t)) return null;
  const cents = Math.round(Number(t) * 100);
  return Number.isFinite(cents) && cents > 0 && cents < 4e9 ? cents : null;
}

/** The search form as Steam is asked it. */
export function toQuery(form: SearchForm, start: number): Cs2Query {
  const s = SORTS.find((x) => x.value === form.sort) ?? SORTS[0];
  return {
    text: form.text.trim(),
    filters: pruned(form.filters),
    sort: s.sort,
    direction: s.direction,
    start,
    count: RESULTS_PER_PAGE,
    descriptions: form.descriptions,
    priceMin: typedAmount(form.priceMin),
    priceMax: typedAmount(form.priceMax),
  };
}

/** Wear in words from its number, with the color Steam's wear bar gives each stretch. */
export const WEAR: { upTo: number; name: string; short: string; color: string }[] = [
  { upTo: 0.07, name: "Factory New", short: "FN", color: "#4A7D12" },
  { upTo: 0.15, name: "Minimal Wear", short: "MW", color: "#82B461" },
  { upTo: 0.38, name: "Field-Tested", short: "FT", color: "#DCB259" },
  { upTo: 0.45, name: "Well-Worn", short: "WW", color: "#BB6454" },
  { upTo: 1.01, name: "Battle-Scarred", short: "BS", color: "#84453B" },
];

export function wearOf(float: number): (typeof WEAR)[number] {
  return WEAR.find((w) => float < w.upTo) ?? WEAR[WEAR.length - 1];
}

/** A color Steam names for an item ("d32ce6"), usable in a style; undefined for none or its plain grey. */
export function itemColor(hex: string | undefined): string | undefined {
  if (!hex || !/^[0-9a-fA-F]{6}$/.test(hex)) return undefined;
  // Steam's default name colors are the page's own text color: leave those to the theme.
  return ["d2d2d2", "b0c3d9", "b2b2b2"].includes(hex.toLowerCase()) ? undefined : `#${hex}`;
}

/** The Steam Subscriber Agreement, which Steam's own purchase dialog asks buyers to agree to. */
export const SSA_URL = "https://store.steampowered.com/subscriber_agreement/";

interface Cs2Data {
  inventory: Cs2Item[] | null;
  inventoryLoading: boolean;
  mine: Cs2Mine | null;
  mineLoading: boolean;
  offers: TradeOffers | null;
  offersLoading: boolean;
  buyer: Cs2Buyer | null;
}

const NOTHING: Cs2Data = { inventory: null, inventoryLoading: false, mine: null, mineLoading: false, offers: null, offersLoading: false, buyer: null };

interface Cs2Store {
  /** Steam's list of filters: read once while IdleKit runs. */
  facets: Cs2Facet[] | null;
  facetsLoading: boolean;
  /** Why it couldn't be read; the page then works from the names built in. */
  facetsFailed: string | null;
  byAccount: Record<string, Cs2Data>;
  loadFacets(steamId: string): Promise<void>;
  loadInventory(steamId: string): Promise<void>;
  loadMine(steamId: string): Promise<void>;
  loadOffers(steamId: string): Promise<void>;
  /** The wallet, read again: before a purchase and after one. Answers null when it can't be read. */
  loadBuyer(steamId: string, quiet?: boolean): Promise<Cs2Buyer | null>;
  setBuyerBalance(steamId: string, balance: number): void;
}

export const useCs2Store = create<Cs2Store>((set, get) => {
  const data = (steamId: string) => get().byAccount[steamId] ?? NOTHING;
  const change = (steamId: string, part: Partial<Cs2Data>) =>
    set((s) => ({ byAccount: { ...s.byAccount, [steamId]: { ...(s.byAccount[steamId] ?? NOTHING), ...part } } }));
  return {
    facets: null,
    facetsLoading: false,
    facetsFailed: null,
    byAccount: {},
    async loadFacets(steamId) {
      if (get().facets || get().facetsLoading) return;
      set({ facetsLoading: true, facetsFailed: null });
      try {
        set({ facets: await api.cs2Facets(steamId), facetsLoading: false });
      } catch (e) {
        // The search works without them, from the names built in.
        set({ facetsLoading: false, facetsFailed: String(e) });
      }
    },
    async loadInventory(steamId) {
      if (data(steamId).inventoryLoading) return;
      change(steamId, { inventoryLoading: true });
      const inventory = await useApp.getState().run(null, () => api.cs2Inventory(steamId));
      change(steamId, inventory ? { inventory, inventoryLoading: false } : { inventoryLoading: false });
    },
    async loadMine(steamId) {
      if (data(steamId).mineLoading) return;
      change(steamId, { mineLoading: true });
      const mine = await useApp.getState().run(null, () => api.cs2Mine(steamId));
      change(steamId, mine ? { mine, mineLoading: false } : { mineLoading: false });
    },
    async loadOffers(steamId) {
      if (data(steamId).offersLoading) return;
      change(steamId, { offersLoading: true });
      const offers = await useApp.getState().run(null, () => api.tradeOffers(steamId));
      change(steamId, offers ? { offers, offersLoading: false } : { offersLoading: false });
    },
    async loadBuyer(steamId, quiet = false) {
      try {
        const buyer = await api.cs2Buyer(steamId);
        change(steamId, { buyer });
        return buyer;
      } catch (e) {
        if (!quiet) useApp.getState().toast(String(e), "error");
        return null;
      }
    },
    setBuyerBalance(steamId, balance) {
      const buyer = data(steamId).buyer;
      if (buyer) change(steamId, { buyer: { ...buyer, balance } });
    },
  };
});

/** An account's CS2 data, as kept. */
export function useCs2(steamId: string): Cs2Data {
  return useCs2Store((s) => s.byAccount[steamId]) ?? NOTHING;
}
