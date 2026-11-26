import { CalendarClock, ExternalLink, RefreshCw } from "lucide-react";
import { Button, Callout, Empty, PageHeader, Pill, Section, cx } from "../components/ui";
import { openExternal } from "../lib/api";
import {
  countdown,
  dayRange,
  isOn,
  KIND_LABEL,
  length,
  monthOf,
  readAt,
  shortDay,
  stillAhead,
  undated,
  useEvents,
  clock,
} from "../lib/events";
import { useNow } from "../lib/hooks";
import type { SteamEvent } from "../lib/types";

/** Valve's page the list is read from, for people who want the details. */
const VALVE_PAGE = "https://partner.steamgames.com/doc/marketing/upcoming_events";

export function EventsPage() {
  const list = useEvents((s) => s.list);
  const loading = useEvents((s) => s.loading);
  const load = useEvents((s) => s.load);
  // The countdowns move by the hour at most; twice a minute is plenty.
  const now = useNow(30_000);

  const events = list?.events ?? [];
  const ahead = stillAhead(events, now);
  // The first is shown large; the rest are listed under it.
  const [next, ...rest] = ahead;
  const on = rest.filter((e) => isOn(e, now));
  const coming = rest.filter((e) => !isOn(e, now));
  const months = new Map<string, SteamEvent[]>();
  for (const e of coming) {
    const month = monthOf(e.starts!);
    months.set(month, [...(months.get(month) ?? []), e]);
  }
  const notDated = undated(events);

  return (
    <div className="pb-10">
      <PageHeader
        title="Upcoming Events"
        sub="Steam's sales and fests from Valve's own schedule, in your time zone. The list is read again every day at midnight."
        actions={
          <Button icon={<RefreshCw size={14} />} busy={loading} onClick={() => void load(true)}>
            Refresh
          </Button>
        }
      />

      {list?.error && (
        <div className="mb-4">
          <Callout tone="bad">
            The list couldn't be updated: {list.error}.{" "}
            {list.fetchedAt
              ? `This is the list read ${readAt(list.fetchedAt, now)}.`
              : "This is the list that came with this version of IdleKit."}{" "}
            It's tried again within the hour.
          </Callout>
        </div>
      )}

      {!list ? (
        <Empty title="Reading Valve's schedule…" body="Steam's upcoming sales and fests show here in a moment." />
      ) : !next ? (
        <Empty
          title="Nothing on Valve's schedule right now"
          body="Valve adds sales and fests through the year. The list is read again at midnight, or now with Refresh."
        />
      ) : (
        <div className="flex flex-col gap-4">
          <NextUp event={next} now={now} />
          {on.length > 0 && (
            <Section title="Also on now">
              <Rows events={on} now={now} />
            </Section>
          )}
          {[...months].map(([month, list]) => (
            <Section key={month} title={month}>
              <Rows events={list} now={now} />
            </Section>
          ))}
          {notDated.length > 0 && (
            <Section title="Dates not announced yet">
              <ul className="divide-y divide-line">
                {notDated.map((e) => (
                  <li key={e.name} className="flex items-center justify-between gap-4 px-4 py-3">
                    <span className="font-medium">{e.name}</span>
                    <Pill>{KIND_LABEL[e.kind]}</Pill>
                  </li>
                ))}
              </ul>
            </Section>
          )}
        </div>
      )}

      {list && (
        <div className="mt-5 flex flex-wrap items-center gap-x-3 gap-y-1 text-[12px] text-faint">
          <span>
            {list.fetchedAt
              ? `Read from Valve's page ${readAt(list.fetchedAt, now)}.`
              : "The list that came with this version of IdleKit."}
          </span>
          <button
            onClick={() => void openExternal(VALVE_PAGE)}
            className="inline-flex items-center gap-1 text-muted underline-offset-2 hover:text-ink hover:underline"
          >
            Valve's page <ExternalLink size={11} />
          </button>
        </div>
      )}
    </div>
  );
}

/** The event that is on, or the next to start, with its countdown large. */
function NextUp({ event, now }: { event: SteamEvent; now: number }) {
  const on = isOn(event, now);
  const left = countdown(event, now);
  return (
    <section
      className={cx(
        "flex flex-wrap items-center justify-between gap-6 rounded-xl border bg-panel px-5 py-5",
        on ? "border-run/45" : "border-line",
      )}
    >
      <div className="min-w-0">
        <div className="flex items-center gap-2 text-[13px] text-muted">
          {on ? <span className="lamp on" /> : <CalendarClock size={14} className="text-faint" />}
          {on ? "On now" : "Next event"}
          <span className="text-faint">·</span>
          {KIND_LABEL[event.kind]}
        </div>
        <h2 className="heading mt-2 text-[26px] font-semibold leading-tight">{event.name}</h2>
        <div className="mt-1.5 text-muted">
          {shortDay(event.starts!, true)}, {clock(event.starts!)} – {shortDay(event.ends!, true)}, {clock(event.ends!)}
          <span className="text-faint"> · {length(event)}</span>
        </div>
      </div>
      {left && (
        <div className="text-right">
          <div className={cx("num heading text-[52px] font-semibold leading-none", on && "text-run")}>{left.value}</div>
          <div className="mt-1.5 text-[13px] text-muted">{left.unit}</div>
        </div>
      )}
    </section>
  );
}

function Rows({ events, now }: { events: SteamEvent[]; now: number }) {
  return (
    <ul className="divide-y divide-line">
      {events.map((e) => {
        const on = isOn(e, now);
        const left = countdown(e, now);
        const first = new Date(e.starts!);
        return (
          <li key={`${e.name}-${e.firstDay}`} className="flex items-center gap-4 px-4 py-3">
            {/* The day it starts, as on a calendar page. */}
            <div
              aria-hidden="true"
              className={cx(
                "flex w-12 flex-none flex-col items-center rounded-lg border py-1.5 leading-none",
                on ? "border-run/45 bg-run-soft" : "border-line bg-well",
              )}
            >
              <span className={cx("text-[11px]", on ? "text-run" : "text-muted")}>
                {first.toLocaleDateString([], { month: "short" })}
              </span>
              <span className="num mt-1 text-[19px] font-semibold">{first.getDate()}</span>
            </div>
            <div className="min-w-0 flex-1">
              <div className="flex flex-wrap items-center gap-2">
                <span className="truncate font-medium">{e.name}</span>
                <Pill>{KIND_LABEL[e.kind]}</Pill>
              </div>
              <div className="mt-0.5 text-[12px] text-muted">
                {dayRange(e, true)} · {length(e)}
              </div>
            </div>
            <div className="flex-none text-right">
              {on ? (
                <Pill tone="run" lamp>
                  On now
                </Pill>
              ) : null}
              {left && <div className={cx("text-[12px] text-muted", on && "mt-1")}>{left.words}</div>}
            </div>
          </li>
        );
      })}
    </ul>
  );
}
