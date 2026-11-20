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
