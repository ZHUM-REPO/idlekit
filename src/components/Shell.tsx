import {
  CalendarClock,
  CalendarDays,
  ChevronDown,
  Layers,
  Library,
  ScrollText,
  Settings2,
  Store,
  Timer,
  Trophy,
  Users,
  X,
} from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { inTauri } from "../lib/api";
import { countdown, dayRange, isOn, nextEvent, useEvents, useEventsKeptFresh } from "../lib/events";
import { useNow } from "../lib/hooks";
import { pictureBroken, usePicture } from "../lib/pictures";
import { dragRegion, WindowControls } from "./WindowBar";
import { until } from "../lib/format";
import { useActiveAccount, useApp, type Page } from "../lib/store";
import type { Account } from "../lib/types";
import { cx } from "./ui";

const NAV: { page: Page; label: string; icon: ReactNode }[] = [
  { page: "library", label: "Library", icon: <Library size={17} /> },
  { page: "idling", label: "Idling", icon: <Timer size={17} /> },
  { page: "farming", label: "Card Farming", icon: <Layers size={17} /> },
  { page: "achievements", label: "Achievements", icon: <Trophy size={17} /> },
  { page: "market", label: "Market", icon: <Store size={17} /> },
  { page: "events", label: "Upcoming Events", icon: <CalendarDays size={17} /> },
];

const NAV_BOTTOM: { page: Page; label: string; icon: ReactNode }[] = [
  { page: "accounts", label: "Accounts", icon: <Users size={17} /> },
  { page: "settings", label: "Settings", icon: <Settings2 size={17} /> },
  { page: "logs", label: "Logs", icon: <ScrollText size={17} /> },
];

function Logo() {
  return (
    <svg viewBox="0 0 1024 1024" width="26" height="26" aria-hidden="true">
      <rect x="40" y="40" width="944" height="944" rx="220" fill="var(--raised)" />
      <g transform="rotate(-14 470 540)">
        <rect x="250" y="250" width="380" height="520" rx="56" fill="var(--faint)" />
      </g>
      <g transform="rotate(8 590 520)">
        <rect x="400" y="240" width="380" height="520" rx="56" fill="var(--run)" />
        <path d="M540 405 L540 595 L700 500 Z" fill="var(--run-ink)" stroke="var(--run-ink)" strokeWidth="28" strokeLinejoin="round" />
      </g>
    </svg>
  );
}

/**
 * The sale or fest that is on, or the next one, in a square above Accounts:
 * how long until it starts (or ends), its name and its days. In a short
 * window it gives up the big number and fits in a few lines.
 */
function NextEvent({ onPage, open }: { onPage: boolean; open: () => void }) {
  const events = useEvents((s) => s.list?.events);
  const now = useNow(30_000, !!events?.length);
  const next = events ? nextEvent(events, now) : null;
  const left = next ? countdown(next, now) : null;
  if (!next || !left) return <div className="mt-auto" />;
  const on = isOn(next, now);
  return (
    <div className="mt-auto pt-3 pb-3">
      <button
        onClick={open}
        title={`${next.name}: ${left.words}. Open Upcoming Events.`}
        className={cx(
          "flex aspect-square w-full flex-col justify-between rounded-xl border p-3.5 text-left transition-colors",
          "short-window:aspect-auto short-window:gap-2",
          on ? "border-run/45 bg-run-soft hover:border-run/70" : "border-line bg-well hover:border-line-strong",
          onPage && !on && "border-line-strong",
        )}
      >
        <span className="flex items-center gap-1.5 text-[12px] text-muted">
          {on ? <span className="lamp on" /> : <CalendarClock size={13} className="text-faint" />}
          {on ? "On now" : "Next event"}
        </span>
        <span className="short-window:hidden">
          <span className={cx("num heading block text-[46px] font-semibold leading-none", on && "text-run")}>{left.value}</span>
          <span className="mt-1.5 block text-[12px] text-muted">{left.unit}</span>
        </span>
        <span className="block">
          <span className="line-clamp-2 text-[15px] leading-snug font-semibold text-ink">{next.name}</span>
          <span className="num mt-1 block text-[12px] text-muted">{dayRange(next)}</span>
          <span className="mt-0.5 hidden text-[12px] text-muted short-window:block">{left.words}</span>
        </span>
      </button>
    </div>
  );
}

export function Sidebar() {
  useEventsKeptFresh();
  const page = useApp((s) => s.page);
  const go = useApp((s) => s.go);
  const version = useApp((s) => s.version);
  const newer = useApp((s) => s.update?.available?.version);
  const activeId = useApp((s) => s.activeId);
  const idleCount = useApp((s) => (activeId ? (s.idle[activeId]?.length ?? 0) : 0));
  const farming = useApp((s) => (activeId ? !!s.farm[activeId]?.running : false));
  const unlocking = useApp((s) => (activeId ? !!s.unlock[activeId]?.running : false));
  const lampFor = (p: Page) =>
    (p === "idling" && idleCount > 0) || (p === "farming" && farming) || (p === "achievements" && unlocking);

  const item = (n: { page: Page; label: string; icon: ReactNode }) => (
    <button
      key={n.page}
      onClick={() => go(n.page)}
      aria-current={page === n.page ? "page" : undefined}
      className={cx(
        "flex h-9 w-full items-center gap-2.5 rounded-md px-2.5 text-[13px] transition-colors",
        page === n.page ? "bg-raised text-ink" : "text-muted hover:text-ink hover:bg-raised/60",
      )}
    >
      <span className={cx(page === n.page ? "text-ink" : "text-faint")}>{n.icon}</span>
      <span className="flex-1 text-left">{n.label}</span>
      {lampFor(n.page) && <span className="lamp on" />}
    </button>
  );

  return (
    <nav className="flex w-[212px] flex-none flex-col border-r border-line bg-panel px-3 pb-3">
      {/* The left end of the window's bar: the window can be moved by it. */}
      <div {...dragRegion} className="-mx-3 mb-2 flex h-[58px] flex-none items-center gap-2.5 px-[18px]">
        <Logo />
        <span className="heading text-[18px] font-semibold">IdleKit</span>
        {/* Which version this is, where it can't be missed. */}
        <span className="num mt-[3px] text-[12px] text-faint" title={`IdleKit version ${version}`}>
          {version}
        </span>
      </div>
      <div className="flex flex-col gap-0.5">{NAV.map(item)}</div>
      <NextEvent onPage={page === "events"} open={() => go("events")} />
      <div className="flex flex-col gap-0.5 border-t border-line pt-3">{NAV_BOTTOM.map(item)}</div>
      {newer ? (
        // Stays for as long as a newer version exists, also after its bar was closed.
        <button
          onClick={() => go("settings")}
          title={`IdleKit ${newer} is available. Open Settings to install it.`}
          className="mt-3 flex items-center gap-2 truncate rounded-md px-2.5 py-1 text-left text-[12px] text-ink hover:bg-raised"
        >
          <span className="lamp on flex-none" />
          <span className="truncate">
            Version {version}, {newer} available
          </span>
        </button>
      ) : (
        <div className="mt-3 truncate px-2.5 text-[11px] text-faint">
          Version {version}
          {!inTauri && ", browser preview"}
        </div>
      )}
    </nav>
  );
}

function Avatar({ account, size = 30 }: { account: Account; size?: number }) {
  const box = useRef<HTMLSpanElement>(null);
  const name = account.personaName || account.accountName || "?";
  // From disk when it is there: Steam's own copy on this PC, or one fetched
  // earlier. Otherwise fetched once from the profile's avatar address.
  const src = usePicture("avatar", account.steamId, box, account.avatarUrl ? [account.avatarUrl] : [], account.avatarUrl || undefined);
  return (
    <span
      ref={box}
      aria-hidden="true"
      className={cx(
        "inline-flex flex-none items-center justify-center overflow-hidden rounded-md text-[13px] font-semibold text-muted",
        src ? "bg-well" : "border border-line bg-raised",
      )}
      style={{ width: size, height: size }}
    >
      {src ? (
        <img src={src} alt="" className="size-full object-cover" onError={() => pictureBroken("avatar", account.steamId)} />
      ) : (
        name.slice(0, 1).toUpperCase()
      )}
    </span>
  );
}
export { Avatar };

export function modeLabel(a: Account) {
  return a.mode === "direct" ? "Direct connection" : "Steam client";
}

function AccountSwitcher() {
  const accounts = useApp((s) => s.accounts);
  const active = useActiveAccount();
  const setActive = useApp((s) => s.setActive);
  const go = useApp((s) => s.go);
  const cm = useApp((s) => (active ? s.cm[active.steamId] : undefined));
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => !ref.current?.contains(e.target as Node) && setOpen(false);
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  }, [open]);

  if (!active) {
    return (
      <button onClick={() => go("accounts")} className="text-[13px] text-muted hover:text-ink">
        Add a Steam account to begin
      </button>
    );
  }

  const status =
    active.mode === "direct"
      ? ({
          online: "Connected",
          blocked: "Paused, playing elsewhere",
          connecting: "Connecting…",
          reconnecting: "Reconnecting…",
          error: "Connection problem",
          offline: "Not connected",
        }[cm?.status ?? "offline"] ?? "Not connected")
      : active.localActive
        ? "Signed in to Steam"
        : "Steam client not signed in";

  return (
    <div className="relative" ref={ref}>
      <button
        onClick={() => setOpen((o) => !o)}
        aria-expanded={open}
        className="flex items-center gap-2.5 rounded-lg px-1.5 py-1 hover:bg-raised"
      >
        <Avatar account={active} />
        <span className="flex flex-col items-start leading-tight">
          <span className="text-[13px] font-medium">{active.personaName || active.accountName}</span>
          <span className="text-[12px] text-faint">
            {modeLabel(active)}, {status.toLowerCase()}
          </span>
        </span>
        <ChevronDown size={15} className="text-faint" />
      </button>
      {open && (
        <div className="absolute left-0 top-full z-40 mt-1.5 w-72 overflow-hidden rounded-lg border border-line-strong bg-panel shadow-2xl">
          {accounts.map((a) => (
            <button
              key={a.steamId}
              onClick={() => {
                void setActive(a.steamId);
                setOpen(false);
              }}
              className={cx(
                "flex w-full items-center gap-2.5 px-3 py-2 text-left hover:bg-raised",
                a.steamId === active.steamId && "bg-raised",
              )}
            >
              <Avatar account={a} size={26} />
              <span className="flex min-w-0 flex-col leading-tight">
                <span className="truncate text-[13px]">{a.personaName || a.accountName}</span>
                <span className="text-[12px] text-faint">
                  {a.accountName}, {modeLabel(a).toLowerCase()}
                </span>
              </span>
            </button>
          ))}
          <button
            onClick={() => {
              go("accounts");
              setOpen(false);
            }}
            className="w-full border-t border-line px-3 py-2.5 text-left text-[13px] text-muted hover:bg-raised hover:text-ink"
          >
            Manage accounts
          </button>
        </div>
      )}
    </div>
  );
}

function Gauge({
  on,
  label,
  value,
  onClick,
}: {
  on: boolean;
  label: string;
  value: string;
  onClick: () => void;
}) {
  return (
    <button onClick={onClick} className="flex items-center gap-2.5 rounded-md px-3 py-1 text-left hover:bg-raised">
      <span className={cx("lamp", on && "on")} />
      <span className="flex flex-col leading-tight">
        <span className="text-[11px] text-faint">{label}</span>
        <span className={cx("num text-[14px]", on ? "text-ink" : "text-muted")}>{value}</span>
      </span>
    </button>
  );
}

export function TopBar() {
  const active = useActiveAccount();
  const go = useApp((s) => s.go);
  const idle = useApp((s) => (active ? s.idle[active.steamId] : undefined));
  const farm = useApp((s) => (active ? s.farm[active.steamId] : undefined));
  const unlock = useApp((s) => (active ? s.unlock[active.steamId] : undefined));
  // The only thing here that counts down is the unlocker's "Next in …".
  const now = useNow(1000, !!unlock?.running);

  const idleN = idle?.length ?? 0;
  const nextUnlock = unlock?.games
    .filter((g) => g.nextAt > now)
    .sort((a, b) => a.nextAt - b.nextAt)[0];

  return (
    // Also the window's title bar: its empty parts move the window, and the
    // window's buttons sit at its right end.
    <header {...dragRegion} className="flex h-[58px] flex-none items-center gap-4 border-b border-line bg-bg pl-5">
      <AccountSwitcher />
      {active && (
        <div className="ml-auto flex items-center gap-1">
          <Gauge
            on={idleN > 0}
            label="Idling"
            value={idleN > 0 ? `${idleN} ${idleN === 1 ? "game" : "games"}` : "Nothing"}
            onClick={() => go("idling")}
          />
          <Gauge
            on={!!farm?.running}
            label="Card Farming"
            value={
              farm?.running
                ? farm.phase === "scanning"
                  ? "Scanning badges"
                  : `${farm.totalDrops} drops left`
                : farm?.phase === "done"
                  ? "Finished"
                  : "Not running"
            }
            onClick={() => go("farming")}
          />
          <Gauge
            on={!!unlock?.running}
            label="Unlocker"
            value={unlock?.running ? (nextUnlock ? `Next in ${until(nextUnlock.nextAt, now)}` : "Working") : "Not running"}
            onClick={() => go("achievements")}
          />
        </div>
      )}
      <WindowControls className={active ? undefined : "ml-auto"} />
    </header>
  );
}

export function Toasts() {
  const toasts = useApp((s) => s.toasts);
  const dismiss = useApp((s) => s.dismiss);
  return (
    <div className="pointer-events-none fixed bottom-4 right-4 z-[60] flex w-[380px] flex-col gap-2" aria-live="polite">
      {toasts.map((t) => (
        <div
          key={t.id}
          className={cx(
            "pointer-events-auto flex items-start gap-3 rounded-lg border bg-panel px-3.5 py-2.5 text-[13px] shadow-xl",
            t.kind === "error" ? "border-bad/50" : t.kind === "ok" ? "border-ok/40" : "border-line-strong",
          )}
        >
          <span className={cx("lamp mt-1.5", t.kind === "error" && "bad", t.kind === "ok" && "ok")} />
          <span className="flex-1">{t.text}</span>
          <button onClick={() => dismiss(t.id)} aria-label="Dismiss" className="text-faint hover:text-ink">
            <X size={14} />
          </button>
        </div>
      ))}
    </div>
  );
}
