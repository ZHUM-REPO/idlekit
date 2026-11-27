// Pieces the CS2 market's tabs share: an item's picture, wear, the price
// history chart, the "I agree" box of a purchase, and loading data that a
// later request may overtake.
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { cx } from "../../components/ui";
import { openExternal } from "../../lib/api";
import { SSA_URL, itemColor, wearOf } from "../../lib/cs2";
import { money } from "../../lib/format";
import { useKept } from "../../lib/kept";
import type { Cs2History } from "../../lib/types";

/** Data that is asked for again whenever `key` changes; an answer that arrives after a newer question is dropped. */
export function useAsked<T>(key: string | null, ask: () => Promise<T>): { data: T | undefined; error: string | null; loading: boolean; reload: () => void } {
  const [state, setState] = useState<{ key: string | null; data: T | undefined; error: string | null; loading: boolean }>({
    key: null,
    data: undefined,
    error: null,
    loading: false,
  });
  const [again, setAgain] = useState(0);
  const latest = useRef(ask);
  latest.current = ask;
  useEffect(() => {
    if (key === null) return;
    let current = true;
    setState((s) => ({ key, data: s.key === key ? s.data : undefined, error: null, loading: true }));
    latest.current().then(
      (data) => current && setState({ key, data, error: null, loading: false }),
      (e) => current && setState({ key, data: undefined, error: String(e), loading: false }),
    );
    return () => {
      current = false;
    };
  }, [key, again]);
  const reload = useCallback(() => setAgain((n) => n + 1), []);
  // What belongs to an earlier key is never shown for this one.
  const mine = state.key === key;
  return { data: mine ? state.data : undefined, error: mine ? state.error : null, loading: mine ? state.loading : key !== null, reload };
}

/** An item's picture, or a blank square of the same size while there is none. */
export function ItemIcon({ src, size = 40, className }: { src: string | null | undefined; size?: number; className?: string }) {
  const [broken, setBroken] = useState(false);
  useEffect(() => setBroken(false), [src]);
  const box = { width: size, height: size };
  return src && !broken ? (
    <img src={src} alt="" loading="lazy" style={box} onError={() => setBroken(true)} className={cx("flex-none rounded bg-well object-contain", className)} />
  ) : (
    <span style={box} aria-hidden="true" className={cx("flex-none rounded bg-well", className)} />
  );
}

/** An item's name in the color Steam gives its grade, where it gives one. */
export function ItemName({ name, color, className }: { name: string; color?: string; className?: string }) {
  return (
    <span className={cx("truncate", className)} style={{ color: itemColor(color) }}>
      {name}
    </span>
  );
}

/** Wear as its number, with where it falls between new and scarred. */
export function Wear({ float }: { float: number }) {
  const wear = wearOf(float);
  return (
    <span className="inline-flex items-center gap-2" title={`${wear.name}: wear ${float.toFixed(6)} on a scale from 0 (new) to 1`}>
      <span className="num">{float.toFixed(4)}</span>
      <span className="relative h-1.5 w-16 flex-none overflow-hidden rounded-full bg-well" aria-hidden="true">
        <span className="absolute inset-y-0 w-0.5 rounded-full" style={{ left: `calc(${Math.min(1, Math.max(0, float)) * 100}% - 1px)`, background: wear.color }} />
        <span className="absolute inset-y-0 left-0 opacity-30" style={{ width: `${Math.min(1, float) * 100}%`, background: wear.color }} />
      </span>
      <span className="text-[12px] text-faint">{wear.short}</span>
    </span>
  );
}

/** A row of choices where several can be on at once. */
export function Chips({
  label,
  options,
  picked,
  onToggle,
}: {
  label: string;
  options: { id: string; name: string; note?: string; color?: string }[];
  picked: string[];
  onToggle: (id: string) => void;
}) {
  if (options.length === 0) return null;
  return (
    <div className="flex flex-wrap items-center gap-1.5" role="group" aria-label={label}>
      <span className="mr-1 text-[12px] text-faint">{label}</span>
      {options.map((o) => {
        const on = picked.includes(o.id);
        return (
          <button
            key={o.id}
            type="button"
            aria-pressed={on}
            onClick={() => onToggle(o.id)}
            className={cx(
              "inline-flex h-7 items-center gap-1.5 rounded-full border px-2.5 text-[12px] transition-colors",
              on ? "border-ink bg-ink text-bg" : "border-line bg-raised text-muted hover:border-line-strong hover:text-ink",
            )}
          >
            {o.color && itemColor(o.color) && <span className="size-2 rounded-full" style={{ background: itemColor(o.color) }} aria-hidden="true" />}
            {o.name}
            {o.note && <span className={cx("num", on ? "opacity-70" : "text-faint")}>{o.note}</span>}
          </button>
        );
      })}
    </div>
  );
}

/** The box a buyer ticks before a purchase, as in Steam's own purchase dialog. */
export function Agree({ checked, onChange }: { checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <label className="flex cursor-pointer items-start gap-2.5 text-[13px]">
      <input type="checkbox" className="mt-0.5 size-4 accent-[var(--ink)]" checked={checked} onChange={(e) => onChange(e.target.checked)} />
      <span>
        I agree to the terms of the{" "}
        <button type="button" className="underline decoration-line-strong underline-offset-2 hover:text-ink" onClick={() => void openExternal(SSA_URL)}>
          Steam Subscriber Agreement
        </button>
        . Purchases with the Steam Wallet are final.
      </span>
    </label>
  );
}

/** A line of a price summary: what it is, and the amount. */
export function Line({ label, value, strong }: { label: ReactNode; value: ReactNode; strong?: boolean }) {
  return (
    <div className={cx("flex items-baseline justify-between gap-4 py-1", strong && "border-t border-line pt-2 text-[15px] font-semibold")}>
      <span className={strong ? undefined : "text-muted"}>{label}</span>
      <span className="num">{value}</span>
    </div>
  );
}

const RANGES = [
  { id: "week", label: "Week", days: 7 },
  { id: "month", label: "Month", days: 30 },
  { id: "year", label: "Year", days: 365 },
  { id: "all", label: "All", days: Infinity },
] as const;
type RangeId = (typeof RANGES)[number]["id"];

/** At most this many points are drawn: more than the chart has pixels for tells nothing more. */
const MOST_POINTS = 180;

interface Spot {
  time: number;
  price: number;
  sold: number;
}

/** The points of a stretch of time, thinned to what a chart can show: each drawn point is the middle price of its share of the stretch, with all that sold in it. */
export function thinned(points: Cs2History["points"], since: number): Spot[] {
  const within = points.filter((p) => p[0] >= since);
  if (within.length <= MOST_POINTS) return within.map(([time, price, sold]) => ({ time, price, sold }));
  const out: Spot[] = [];
  const per = within.length / MOST_POINTS;
  for (let i = 0; i < MOST_POINTS; i++) {
    const part = within.slice(Math.floor(i * per), Math.max(Math.floor(i * per) + 1, Math.floor((i + 1) * per)));
    const prices = part.map((p) => p[1]).sort((a, b) => a - b);
    out.push({ time: part[part.length - 1][0], price: prices[Math.floor(prices.length / 2)], sold: part.reduce((n, p) => n + p[2], 0) });
  }
  return out;
}

/** Round steps for an axis from `low` to `high`: three to five of them, on numbers that read easily. */
export function axisSteps(low: number, high: number): number[] {
  if (!(high > low)) return [low];
  const rough = (high - low) / 4;
  const power = 10 ** Math.floor(Math.log10(rough));
  const step = [1, 2, 2.5, 5, 10].map((m) => m * power).find((s) => s >= rough) ?? rough;
  const out: number[] = [];
  for (let v = Math.ceil(low / step) * step; v <= high + step * 1e-9; v += step) out.push(Math.round(v));
  return out;
}

const H = 200;
const PAD = { left: 64, right: 12, top: 12, bottom: 24 };

/** How wide an element is on screen, kept up to date: a chart is drawn in real pixels, so its text keeps its size. */
function useWidth(initial: number): [React.RefObject<HTMLDivElement | null>, number] {
  const ref = useRef<HTMLDivElement | null>(null);
  const [width, setWidth] = useState(initial);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const measure = () => setWidth((w) => (Math.abs(el.clientWidth - w) >= 1 && el.clientWidth > 0 ? el.clientWidth : w));
    measure();
    const watch = new ResizeObserver(measure);
    watch.observe(el);
    return () => watch.disconnect();
  }, []);
  return [ref, width];
}

/**
 * What an item sold for over time: one line of median sale prices, a
 * stretch of time to choose above it, the price under the pointer (or the
 * arrow keys), and the same numbers as a table.
 */
export function PriceChart({ history, name }: { history: Cs2History; name: string }) {
  // How the chart was last looked at stays for the next item, and for the next kind of this one.
  const [range, setRange] = useKept<RangeId>("cs2.chart.range", "month");
  const [at, setAt] = useState<number | null>(null);
  const [asTable, setAsTable] = useKept("cs2.chart.table", false);
  const [box, W] = useWidth(720);
  const newest = history.points.length ? history.points[history.points.length - 1][0] : 0;
  const days = RANGES.find((r) => r.id === range)!.days;
  const spots = useMemo(() => thinned(history.points, Number.isFinite(days) ? newest - days * 86400 : 0), [history.points, newest, days]);
  useEffect(() => setAt(null), [range, history]);

  const dateOf = (t: number) => new Date(t * 1000).toLocaleDateString([], { year: "numeric", month: "short", day: "numeric" });
  const whenOf = (t: number) =>
    days <= 30 ? new Date(t * 1000).toLocaleString([], { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" }) : dateOf(t);
  const picker = (
    <div className="flex items-center gap-1" role="group" aria-label="Stretch of time">
      {RANGES.map((r) => (
        <button
          key={r.id}
          type="button"
          aria-pressed={range === r.id}
          onClick={() => setRange(r.id)}
          className={cx("h-6 rounded px-2 text-[12px]", range === r.id ? "bg-raised text-ink" : "text-muted hover:text-ink")}
        >
          {r.label}
        </button>
      ))}
      <button
        type="button"
        aria-pressed={asTable}
        onClick={() => setAsTable((v) => !v)}
        className={cx("ml-2 h-6 rounded px-2 text-[12px]", asTable ? "bg-raised text-ink" : "text-muted hover:text-ink")}
      >
        Table
      </button>
    </div>
  );
  if (spots.length < 2) {
    return (
      <div>
        <div className="mb-2 flex items-center justify-between gap-3">
          <div className="text-[13px] font-medium">Median sale price</div>
          {picker}
        </div>
        <div className="rounded-lg border border-dashed border-line px-4 py-6 text-[13px] text-muted">
          {history.points.length < 2 ? "Steam has no sales on record for this item yet." : "Nothing sold in this stretch of time. Pick a longer one."}
        </div>
      </div>
    );
  }
  const low = Math.min(...spots.map((s) => s.price));
  const high = Math.max(...spots.map((s) => s.price));
  const room = Math.max(1, (high - low) * 0.08);
  const [y0, y1] = [Math.max(0, low - room), high + room];
  const [t0, t1] = [spots[0].time, spots[spots.length - 1].time];
  const x = (t: number) => PAD.left + ((t - t0) / Math.max(1, t1 - t0)) * (W - PAD.left - PAD.right);
  const y = (p: number) => PAD.top + (1 - (p - y0) / Math.max(1, y1 - y0)) * (H - PAD.top - PAD.bottom);
  const path = spots.map((s, i) => `${i ? "L" : "M"}${x(s.time).toFixed(1)},${y(s.price).toFixed(1)}`).join("");
  const steps = axisSteps(y0, y1).filter((v) => v > y0 && v < y1);
  const last = spots[spots.length - 1];
  const shown = at === null ? null : spots[at];
  const sold = spots.reduce((n, s) => n + s.sold, 0);

  /** The drawn point nearest a place across the chart, 0 (left) to 1 (right). */
  const nearest = (across: number) => {
    const t = t0 + across * (t1 - t0);
    let best = 0;
    for (let i = 1; i < spots.length; i++) if (Math.abs(spots[i].time - t) < Math.abs(spots[best].time - t)) best = i;
    return best;
  };

  return (
    <div>
      <div className="mb-2 flex flex-wrap items-center justify-between gap-3">
        <div className="text-[13px]">
          <span className="font-medium">Median sale price</span>
          <span className="ml-2 text-muted">
            last <span className="num text-ink">{money(last.price, history.currency)}</span>, between{" "}
            <span className="num">{money(low, history.currency)}</span> and <span className="num">{money(high, history.currency)}</span>,{" "}
            <span className="num">{sold.toLocaleString()}</span> sold
          </span>
        </div>
        {picker}
      </div>
      {asTable ? (
        <div className="max-h-[240px] overflow-auto rounded-lg border border-line">
          <table className="w-full text-left text-[12px]">
            <thead className="sticky top-0 bg-panel text-faint">
              <tr>
                <th className="px-3 py-1.5 font-normal">When</th>
                <th className="px-3 py-1.5 text-right font-normal">Median price</th>
                <th className="px-3 py-1.5 text-right font-normal">Sold</th>
              </tr>
            </thead>
            <tbody>
              {[...spots].reverse().map((s) => (
                <tr key={s.time} className="border-t border-line">
                  <td className="px-3 py-1">{whenOf(s.time)}</td>
                  <td className="num px-3 py-1 text-right">{money(s.price, history.currency)}</td>
                  <td className="num px-3 py-1 text-right text-muted">{s.sold.toLocaleString()}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : (
        <div className="relative" ref={box}>
          <svg
            width={W}
            height={H}
            className="block rounded-lg border border-line bg-well/40 outline-none focus-visible:border-line-strong"
            role="img"
            tabIndex={0}
            aria-label={`Median sale price of ${name} over time: last ${money(last.price, history.currency)}, between ${money(low, history.currency)} and ${money(high, history.currency)}. Arrow keys step through the points.`}
            onPointerMove={(e) => {
              const px = e.clientX - e.currentTarget.getBoundingClientRect().left;
              setAt(nearest(Math.min(1, Math.max(0, (px - PAD.left) / (W - PAD.left - PAD.right)))));
            }}
            onPointerLeave={() => setAt(null)}
            onBlur={() => setAt(null)}
            onKeyDown={(e) => {
              if (e.key !== "ArrowLeft" && e.key !== "ArrowRight" && e.key !== "Home" && e.key !== "End") return;
              e.preventDefault();
              const from = at ?? spots.length - 1;
              setAt(e.key === "Home" ? 0 : e.key === "End" ? spots.length - 1 : Math.min(spots.length - 1, Math.max(0, from + (e.key === "ArrowLeft" ? -1 : 1))));
            }}
          >
            {steps.map((v) => (
              <g key={v}>
                <line x1={PAD.left} x2={W - PAD.right} y1={y(v)} y2={y(v)} stroke="var(--line)" strokeWidth="1" />
                <text x={PAD.left - 8} y={y(v) + 4} fontSize="11" fill="var(--faint)" textAnchor="end">
                  {money(v, history.currency)}
                </text>
              </g>
            ))}
            <text x={PAD.left} y={H - 6} fontSize="11" fill="var(--faint)">
              {dateOf(t0)}
            </text>
            <text x={W - PAD.right} y={H - 6} fontSize="11" fill="var(--faint)" textAnchor="end">
              {dateOf(t1)}
            </text>
            <path d={`${path}L${x(t1).toFixed(1)},${H - PAD.bottom}L${x(t0).toFixed(1)},${H - PAD.bottom}Z`} fill="var(--run)" opacity="0.1" />
            <path d={path} fill="none" stroke="var(--run)" strokeWidth="2" strokeLinejoin="round" strokeLinecap="round" />
            {shown && (
              <g>
                <line x1={x(shown.time)} x2={x(shown.time)} y1={PAD.top} y2={H - PAD.bottom} stroke="var(--line-strong)" strokeWidth="1" />
                <circle cx={x(shown.time)} cy={y(shown.price)} r="4.5" fill="var(--run)" stroke="var(--panel)" strokeWidth="2" />
              </g>
            )}
          </svg>
          {shown && (
            <div
              className="pointer-events-none absolute top-2 z-10 rounded-md border border-line-strong bg-panel px-2.5 py-1.5 text-[12px] shadow-lg"
              style={x(shown.time) > W / 2 ? { right: W - x(shown.time) + 12 } : { left: x(shown.time) + 12 }}
            >
              <div className="num text-[14px] font-semibold">{money(shown.price, history.currency)}</div>
              <div className="text-muted">
                <span className="num">{shown.sold.toLocaleString()}</span> sold
              </div>
              <div className="text-faint">{whenOf(shown.time)}</div>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
