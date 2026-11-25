import { create } from "zustand";
import { api } from "./api";
import { US_DOLLAR_WALLET } from "./format";
import { useApp } from "./store";
import type { InvItem, Listing, PriceInfo, Wallet } from "./types";

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
  prices: Record<string, PriceInfo>;
  wallet: Wallet;
  /** Loaded by itself already, while IdleKit runs. A load that failed is
   *  tried again the next time the Market opens for this account. */
  loadedByItself: boolean;
}

const NOTHING_YET: MarketData = {
  items: null,
  itemsLoading: false,
  listings: null,
  listingsLoading: false,
  prices: {},
  wallet: US_DOLLAR_WALLET,
  loadedByItself: false,
};

interface MarketStore {
  byAccount: Record<string, MarketData>;
  loadInventory(steamId: string): Promise<void>;
  loadListings(steamId: string): Promise<void>;
  /** The inventory and the listings, once per account while IdleKit runs, when the account has a web session. */
  loadByItself(steamId: string): void;
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
    priced(steamId, wallet, items) {
      const prices = { ...data(steamId).prices, ...Object.fromEntries(items.map((x) => [x.marketHashName, x])) };
      change(steamId, { wallet, prices });
    },
  };
});

/** An account's market data, as kept. */
export function useMarket(steamId: string): MarketData {
  return useMarketStore((s) => s.byAccount[steamId] ?? NOTHING_YET);
}
