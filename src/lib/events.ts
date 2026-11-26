import { useEffect } from "react";
import { create } from "zustand";
import { api } from "./api";
import type { EventList, SteamEvent } from "./types";

const HOUR = 60 * 60_000;
const DAY = 24 * HOUR;

interface EventsStore {
  list: EventList | null;
  loading: boolean;
  /** Asks for the list: the one read last, or (`refresh`) Valve's page again. */
  load(refresh: boolean): Promise<void>;
}

/** One ask at a time, in the order asked: a refresh asked for during another ask still happens. */
let asking: Promise<void> = Promise.resolve();

export const useEvents = create<EventsStore>((set) => ({
  list: null,
  loading: false,
  load(refresh) {
    asking = asking.then(async () => {
      set({ loading: true });
      try {
        set({ list: await api.steamEvents(refresh) });
      } catch {
        // Already in the log; the list shown stays as it was.
      } finally {
        set({ loading: false });
      }
    });
    return asking;
  },
}));

/** Midnight, local time, at the start of the day `ts` falls on. */
export function dayStart(ts: number): number {
  const d = new Date(ts);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

/** Whole days from one day to another, counted on the calendar (local time). */
function daysBetween(from: number, to: number): number {
  return Math.round((dayStart(to) - dayStart(from)) / DAY);
}

/** Read on a day before today, or never read from Valve's page. */
export function isStale(list: EventList | null, now = Date.now()): boolean {
  return !list?.fetchedAt || list.fetchedAt < dayStart(now);
}

/**
 * Keeps the list fresh while IdleKit runs: the list read last at once, then
 * Valve's page when it wasn't read today, and again each midnight. A page that
 * couldn't be read is tried again an hour later.
 */
export function useEventsKeptFresh() {
  useEffect(() => {
    let lastTry = 0;
    let today = dayStart(Date.now());
    const freshen = async (now: boolean) => {
      if (!isStale(useEvents.getState().list)) return;
      if (!now && Date.now() - lastTry < HOUR) return;
      lastTry = Date.now();
      await useEvents.getState().load(true);
    };
    void useEvents
      .getState()
      .load(false)
      .then(() => freshen(true));
    const id = setInterval(() => {
      const day = dayStart(Date.now());
      const newDay = day !== today;
      today = day;
      void freshen(newDay);
    }, 30_000);
    return () => clearInterval(id);
  }, []);
}

/** Events with dates that haven't ended yet, in the order they start. */
export function stillAhead(events: SteamEvent[], now: number): SteamEvent[] {
  return events
    .filter((e) => e.starts != null && e.ends != null && e.ends > now)
    .sort((a, b) => a.starts! - b.starts! || a.ends! - b.ends!);
}

/** Events Valve hasn't dated yet. */
export function undated(events: SteamEvent[]): SteamEvent[] {
  return events.filter((e) => e.starts == null || e.ends == null);
}

/** What the sidebar shows: an event that is on (the first to have started), else the next to start. */
export function nextEvent(events: SteamEvent[], now: number): SteamEvent | null {
  return stillAhead(events, now)[0] ?? null;
}

export function isOn(e: SteamEvent, now: number): boolean {
  return e.starts != null && e.ends != null && e.starts <= now && now < e.ends;
}

export const KIND_LABEL: Record<SteamEvent["kind"], string> = {
  sale: "Seasonal sale",
  fest: "Fest",
  nextfest: "Next Fest",
  event: "Event",
};

export interface Countdown {
  /** The number, big: "20", "5". */
  value: string;
  /** What it counts: "days to go", "hours left". */
  unit: string;
  /** All of it in words: "Starts in 20 days", "Ends tomorrow at 9:00 PM". */
  words: string;
}

/**
 * How long until the event starts, or, while it is on, until it ends. Under
 * a day it counts hours (then minutes); from there on, days on the calendar,
 * the way "Dec 17 is 20 days from today" is said.
 */
export function countdown(e: SteamEvent, now: number): Countdown | null {
  if (e.starts == null || e.ends == null || e.ends <= now) return null;
  const on = e.starts <= now;
  const at = on ? e.ends : e.starts;
  const left = at - now;
  const tail = on ? "left" : "to go";
  const verb = on ? "Ends" : "Starts";
  const days = daysBetween(now, at);
  const words =
    days === 0 ? `${verb} today at ${clock(at)}` : days === 1 ? `${verb} tomorrow at ${clock(at)}` : `${verb} in ${days} days`;
  if (left < HOUR) {
    const minutes = Math.max(1, Math.ceil(left / 60_000));
    return { value: String(minutes), unit: `${minutes === 1 ? "minute" : "minutes"} ${tail}`, words };
  }
  if (left < DAY) {
    const hours = Math.floor(left / HOUR);
    return { value: String(hours), unit: `${hours === 1 ? "hour" : "hours"} ${tail}`, words };
  }
  return { value: String(days), unit: `${days === 1 ? "day" : "days"} ${tail}`, words };
}

export function clock(ts: number): string {
  return new Date(ts).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
}

/** "Dec 17", or with the weekday: "Thu, Dec 17". */
export function shortDay(ts: number, weekday = false): string {
  return new Date(ts).toLocaleDateString([], { month: "short", day: "numeric", ...(weekday ? { weekday: "short" } : {}) });
}

/** "Dec 17 – Jan 4", in local time: the days the event is on where IdleKit runs. */
export function dayRange(e: SteamEvent, weekday = false): string {
  if (e.starts == null || e.ends == null) return "Dates not announced yet";
  return `${shortDay(e.starts, weekday)} – ${shortDay(e.ends, weekday)}`;
}

/** How many days the event runs: "7 days". */
export function length(e: SteamEvent): string {
  if (e.starts == null || e.ends == null) return "";
  const days = Math.max(1, Math.round((e.ends - e.starts) / DAY));
  return `${days} ${days === 1 ? "day" : "days"}`;
}

/** "December 2026": the month the event starts in, local time. */
export function monthOf(ts: number): string {
  return new Date(ts).toLocaleDateString([], { month: "long", year: "numeric" });
}

/** When the list was read, for the page: "today at 12:00 AM", "Nov 26 at 9:14 PM". */
export function readAt(ts: number, now = Date.now()): string {
  const days = daysBetween(ts, now);
  const day = days === 0 ? "today" : days === 1 ? "yesterday" : shortDay(ts);
  return `${day} at ${clock(ts)}`;
}
