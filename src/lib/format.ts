import type { MarketSettings, Wallet } from "./types";
export function hoursText(minutes: number): string {
  if (minutes <= 0) return "Never played";
  const h = minutes / 60;
  if (h < 1) return `${minutes} min`;
  return `${h >= 100 ? Math.round(h).toLocaleString() : h.toFixed(1)} h`;
}

export function hoursFromFloat(h: number): string {
  if (h <= 0) return "0 h";
  return `${h >= 100 ? Math.round(h).toLocaleString() : h.toFixed(1)} h`;
}

/** Elapsed time as h:mm:ss (hours can exceed 99). */
export function clock(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  return `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
}

export function until(ts: number, now = Date.now()): string {
  const d = Math.max(0, ts - now);
  const m = Math.floor(d / 60_000);
  const s = Math.floor((d % 60_000) / 1000);
  if (m >= 60) return `${Math.floor(m / 60)} h ${m % 60} min`;
  if (m > 0) return `${m} min ${s.toString().padStart(2, "0")} s`;
  return `${s} s`;
}

export function timeOfDay(ts: number): string {
  return new Date(ts).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
}

export function dateText(unixSeconds: number): string {
  if (!unixSeconds) return "";
  return new Date(unixSeconds * 1000).toLocaleDateString([], { year: "numeric", month: "short", day: "numeric" });
}

/** A US-dollar wallet: what is assumed until Steam has said otherwise. */
export const US_DOLLAR_WALLET: Wallet = {
  currency: 1,
  steamPercent: 0.05,
  steamMinimum: 1,
  feeBase: 0,
  publisherPercent: 0.1,
  increment: 1,
  marketMinimum: 1,
};

/** Mirror of the Rust fee math: what a buyer pays for you to receive this much. */
export function buyerPays(seller: number, w: Wallet): number {
  if (seller <= 0) return 0;
  const steam = Math.floor(Math.max(seller * w.steamPercent, w.steamMinimum) + w.feeBase);
  const publisher = w.publisherPercent > 0 ? Math.floor(Math.max(seller * w.publisherPercent, 1)) : 0;
  return seller + steam + publisher;
}

/** Mirror of the Rust fee math: the most you receive for a buyer price. */
export function sellerReceives(buyer: number, w: Wallet): number {
  if (buyer < buyerPays(Math.max(w.marketMinimum, 1), w)) return 0;
  let s = Math.floor(Math.max(0, buyer - w.feeBase) / (1 + w.steamPercent + w.publisherPercent)) + 2;
  while (s > 0 && buyerPays(s, w) > buyer) s--;
  return s;
}

/** A price step: at least one increment, and a whole number of them. */
function step(cents: number, w: Wallet): number {
  const inc = Math.max(1, w.increment);
  return Math.ceil(Math.max(1, cents) / inc) * inc;
}

/**
 * Mirror of the Rust `suggest_price`: what to ask a buyer to pay, by the
 * settings' strategy and limits, from the order book. Never below the lowest
 * price the market takes, nor at or under a buyer's standing offer (unless
 * the strategy is to go just over it).
 */
export function suggestPrice(w: Wallet, lowest: number | null, highest: number | null, s: MarketSettings): number | null {
  const floor = buyerPays(Math.max(w.marketMinimum, 1), w);
  const undercut = s.undercutCents;
  const byStrategy =
    s.strategy === "matchLowest"
      ? lowest
      : s.strategy === "aboveHighestBuy"
        ? highest === null
          ? null
          : highest + step(undercut, w)
        : lowest === null
          ? null
          : undercut === 0
            ? lowest
            : Math.max(0, lowest - step(undercut, w));
  const base = byStrategy ?? lowest ?? (highest === null ? null : highest + step(1, w));
  if (base === null) return null;
  let p = Math.max(base, floor);
  if (s.minPriceCents > 0) p = Math.max(p, s.minPriceCents);
  if (s.maxPriceCents > 0) p = Math.min(p, s.maxPriceCents);
  if (highest !== null && s.strategy !== "aboveHighestBuy" && p <= highest) p = highest + step(1, w);
  return Math.max(p, floor);
}

// Steam wallet currency ids -> ISO 4217 codes.
const STEAM_CURRENCIES: Record<number, string> = {
  1: "USD", 2: "GBP", 3: "EUR", 4: "CHF", 5: "RUB", 6: "PLN", 7: "BRL", 8: "JPY", 9: "NOK", 10: "IDR",
  11: "MYR", 12: "PHP", 13: "SGD", 14: "THB", 15: "VND", 16: "KRW", 17: "TRY", 18: "UAH", 19: "MXN",
  20: "CAD", 21: "AUD", 22: "NZD", 23: "CNY", 24: "INR", 25: "CLP", 26: "PEN", 27: "COP", 28: "ZAR",
  29: "HKD", 30: "TWD", 31: "SAR", 32: "AED", 34: "ARS", 35: "ILS", 37: "KZT", 38: "KWD", 39: "QAR",
  40: "CRC", 41: "UYU",
};

/** Minor units -> localized money string in the Steam wallet's currency. */
export function money(cents: number | null | undefined, currency = 1): string {
  if (cents == null) return "—";
  const code = STEAM_CURRENCIES[currency] ?? "USD";
  try {
    return (cents / 100).toLocaleString(undefined, { style: "currency", currency: code });
  } catch {
    return (cents / 100).toFixed(2);
  }
}

export function personaLabel(state: number): string {
  return ({ 0: "Offline", 1: "Online", 2: "Busy", 3: "Away", 4: "Snooze", 7: "Invisible" } as Record<number, string>)[state] ?? "Online";
}

export function plural(n: number, one: string, many = `${one}s`): string {
  return `${n.toLocaleString()} ${n === 1 ? one : many}`;
}
