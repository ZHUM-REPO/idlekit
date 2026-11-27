import { create } from "zustand";
import { api, on } from "./api";
import { US_DOLLAR_WALLET } from "./format";
import { useApp } from "./store";
import type { InvItem, Listing, PriceInfo, Wallet } from "./types";

/** Prices this old are checked again by themselves (the app keeps them as long). */
export const PRICES_FRESH_FOR = 60 * 60_000;
/** A check that stopped before every item had its price (a listing took
 *  over, or Steam asked to stop for a while) goes on by itself after this. */
const GO_ON_AFTER = 10 * 60_000;

/**
 * One account's market data, kept while IdleKit runs: switching between the
 * Inventory and My listings tabs, or to another page and back, doesn't ask
 * Steam again.
 */
export interface MarketData {
  items: InvItem[] | null;
  itemsLoading: boolean;
  listings: Listing[] | null;
  listingsLoading: boolean;
  /** By item, all in the wallet's currency. */
  prices: Record<string, PriceInfo>;
  wallet: Wallet;
  /** Loaded by itself already, while IdleKit runs. A load that failed is
   *  tried again the next time the Market opens for this account. */
  loadedByItself: boolean;
  /** The inventory whose prices were last checked by themselves, and when:
   *  each inventory once, and again once those prices are old. `held` once
   *  that check was cancelled: it doesn't go on by itself. */
  autoPriced: { items: InvItem[]; at: number; held: boolean } | null;
  /** What Refresh prices asked to check again, and when: a check that
   *  takes over from it still asks again about what it hadn't got to. */
  recheck: { names: string[]; since: number } | null;
}

const NOTHING_YET: MarketData = {
  items: null,
  itemsLoading: false,
  listings: null,
  listingsLoading: false,
  prices: {},
  wallet: US_DOLLAR_WALLET,
  loadedByItself: false,
  autoPriced: null,
  recheck: null,
};

interface MarketStore {
  byAccount: Record<string, MarketData>;
  loadInventory(steamId: string): Promise<void>;
  loadListings(steamId: string): Promise<void>;
  /** The inventory and the listings, once per account while IdleKit runs, when the account has a web session. */
  loadByItself(steamId: string): void;
  /**
   * Prices of these items, in this order: the ones checked lately come at
   * once, the rest as Steam answers. Those in `recheck` are asked about
   * again however recent their price. A newer check takes over from this one.
   */
  checkPrices(steamId: string, names: string[], recheck?: string[]): Promise<void>;
  /**
   * The automatic check of an inventory's prices: once per inventory, again
   * when they are old, and on with the items still without a price when a
   * check stopped halfway (unless it was cancelled). `checking`: a check is
   * under way.
   */
  priceByItself(steamId: string, items: InvItem[], names: string[], checking: boolean): void;
  /** Cancel was pressed: the automatic checks under way don't go on by themselves. */
  holdAutoPricing(): void;
  priced(steamId: string, wallet: Wallet, items: PriceInfo[]): void;
}

export const useMarketStore = create<MarketStore>((set, get) => {
  const data = (steamId: string) => get().byAccount[steamId] ?? NOTHING_YET;
  const change = (steamId: string, part: Partial<MarketData>) =>
    set((s) => ({ byAccount: { ...s.byAccount, [steamId]: { ...(s.byAccount[steamId] ?? NOTHING_YET), ...part } } }));
  return {
    byAccount: {},
    async loadInventory(steamId) {
      if (data(steamId).itemsLoading) return;
      change(steamId, { itemsLoading: true });
      const items = await useApp.getState().run(null, () => api.inventory(steamId));
      change(steamId, items ? { items, itemsLoading: false } : { itemsLoading: false });
    },
    async loadListings(steamId) {
      if (data(steamId).listingsLoading) return;
      change(steamId, { listingsLoading: true });
      const listings = await useApp.getState().run(null, () => api.listings(steamId));
      change(steamId, listings ? { listings, listingsLoading: false } : { listingsLoading: false });
    },
    loadByItself(steamId) {
      const d = data(steamId);
      if (d.loadedByItself) return;
      change(steamId, { loadedByItself: true });
      const loads = [!d.items && get().loadInventory(steamId), !d.listings && get().loadListings(steamId)];
      void Promise.all(loads).then(() => {
        const now = data(steamId);
        if ((!now.items && !now.itemsLoading) || (!now.listings && !now.listingsLoading)) change(steamId, { loadedByItself: false });
      });
    },
    async checkPrices(steamId, names, recheck = []) {
      if (names.length === 0) return;
      const d = data(steamId);
      let again = recheck;
      if (recheck.length > 0) {
        change(steamId, { recheck: { names: recheck, since: Date.now() } });
      } else if (d.recheck) {
        const { names: asked, since } = d.recheck;
        again = asked.filter((name) => !((d.prices[name]?.checkedAt ?? 0) >= since));
        change(steamId, { recheck: again.length > 0 ? { names: again, since } : null });
      }
      const r = await useApp.getState().run(null, () => api.prices(steamId, names, again));
      // Each price came on its own already; this is all of them, as they ended.
      if (r) get().priced(steamId, r.wallet, r.items);
    },
    priceByItself(steamId, items, names, checking) {
      const d = data(steamId);
      const last = d.autoPriced;
      const age = last ? Date.now() - last.at : Infinity;
      const due =
        !last ||
        last.items !== items ||
        age >= PRICES_FRESH_FOR ||
        (!checking && !last.held && age >= GO_ON_AFTER && names.some((name) => !d.prices[name]));
      if (!due) return;
      change(steamId, { autoPriced: { items, at: Date.now(), held: false } });
      void get().checkPrices(steamId, names);
    },
    holdAutoPricing() {
      set((s) => ({
        byAccount: Object.fromEntries(
          Object.entries(s.byAccount).map(([id, d]) => [
            id,
            { ...d, recheck: null, autoPriced: d.autoPriced ? { ...d.autoPriced, held: true } : null },
          ]),
        ),
      }));
    },
    priced(steamId, wallet, items) {
      const d = data(steamId);
      // Prices in another currency than the wallet's are of no use: a wallet
      // in a new currency starts over.
      const prices = d.wallet.currency === wallet.currency ? { ...d.prices } : {};
      for (const item of items) {
        const had = prices[item.marketHashName];
        // An older answer, arriving late, doesn't replace a newer one.
        if (item.currency === wallet.currency && !(had && had.checkedAt > item.checkedAt)) prices[item.marketHashName] = item;
      }
      change(steamId, { wallet, prices });
    },
  };
});

// Prices come one by one while a check goes on.
void on("market://prices", (u) => useMarketStore.getState().priced(u.steamId, u.wallet, u.items));

/** An account's market data, as kept. */
export function useMarket(steamId: string): MarketData {
  return useMarketStore((s) => s.byAccount[steamId] ?? NOTHING_YET);
}
