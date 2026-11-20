import { ArrowDown, ArrowUp, EyeOff, Lock, Play, Plus, RotateCcw, Search, ShieldLock, Square, X } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { GameArt, GameIcon } from "../components/GameArt";
import { Odometer } from "../components/Odometer";
import {
  Button,
  Callout,
  Empty,
  Field,
  Modal,
  PageHeader,
  Pill,
  Section,
  Select,
  Switch,
  Tabs,
  TextInput,
  cx,
  inputClass,
} from "../components/ui";
import { api, errorText } from "../lib/api";
import { dateText, until } from "../lib/format";
import { useNow } from "../lib/hooks";
import { useActiveAccount, useApp, useEnsureGames } from "../lib/store";
import type { AchItem, AchView, Account, GameEntry, StatItem } from "../lib/types";

type Tab = "manager" | "unlocker";

export function AchievementsPage() {
  const account = useActiveAccount();
  const [tab, setTab] = useState<Tab>("manager");
  const unlock = useApp((s) => (account ? s.unlock[account.steamId] : undefined));
  if (!account) return null;
  return (
    <div className="pb-10">
      <PageHeader
        title="Achievements"
        sub="Lock and unlock achievements, edit stats, or let the unlocker work through a queue at a human pace."
      />
      {!account.localActive && (
        <div className="mb-5">
          <Callout tone="run">
            <div className="font-medium">Sign in to the Steam client on this PC as {account.accountName || "this account"}</div>
            <div className="mt-1 text-muted">
              Achievements are changed through the Steam client, the same way Steam Achievement Manager does it. Direct
              connections can't change them.
            </div>
          </Callout>
        </div>
      )}
      <div className="mb-5">
        <Tabs
          value={tab}
          onChange={setTab}
          tabs={[
            { value: "manager", label: "Manager" },
            { value: "unlocker", label: "Unlocker", count: account.unlockQueue.length },
          ]}
        />
      </div>
      {tab === "manager" ? <Manager account={account} /> : <Unlocker account={account} running={!!unlock?.running} />}
    </div>
  );
}

// ------------------------------------------------------------ manager ---

function Manager({ account }: { account: Account }) {
  const games = useApp((s) => s.games[account.steamId]);
  const achAppid = useApp((s) => s.achAppid);
  const go = useApp((s) => s.go);
  const [query, setQuery] = useState("");
  useEnsureGames(account.steamId);

  const list = useMemo(() => {
    const q = query.trim().toLowerCase();
    const fav = new Set(account.favorites);
    return (games ?? [])
      .filter((g) => !q || g.name.toLowerCase().includes(q) || String(g.appid) === q)
      .sort((a, b) => Number(fav.has(b.appid)) - Number(fav.has(a.appid)) || a.name.localeCompare(b.name));
  }, [games, query, account.favorites]);

  const selected = games?.find((g) => g.appid === achAppid) ?? null;

  return (
    <div className="grid gap-5 lg:grid-cols-[280px_minmax(0,1fr)]">
      <div className="flex max-h-[calc(100vh-250px)] flex-col overflow-hidden rounded-xl border border-line bg-panel">
        <div className="border-b border-line p-2.5">
          <label className="relative block">
            <Search size={15} className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-faint" />
            <TextInput
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Find a game"
              aria-label="Find a game"
              className="w-full pl-8"
            />
          </label>
        </div>
        <ul className="flex-1 overflow-y-auto py-1">
          {list.map((g) => (
            <li key={g.appid}>
              <button
                onClick={() => go("achievements", { achAppid: g.appid })}
                className={cx(
                  "flex w-full items-center gap-2.5 px-3 py-1.5 text-left text-[13px]",
                  g.appid === achAppid ? "bg-raised text-ink" : "text-muted hover:bg-raised/60 hover:text-ink",
                )}
              >
                <GameIcon appid={g.appid} name={g.name} size={24} />
                <span className="flex-1 truncate">{g.name}</span>
                {account.unlockQueue.includes(g.appid) && (
                  <span className="lamp" title="In the unlock queue" />
                )}
              </button>
            </li>
          ))}
          {games && list.length === 0 && <li className="px-3 py-3 text-muted">No games match.</li>}
        </ul>
      </div>
      {selected ? (
        <GamePanel key={selected.appid} account={account} game={selected} />
      ) : (
        <Empty title="Pick a game" body="Choose a game on the left to see its achievements and stats." />
      )}
    </div>
  );
}

function GamePanel({ account, game }: { account: Account; game: GameEntry }) {
  const [view, setView] = useState<AchView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [sub, setSub] = useState<"achievements" | "stats">("achievements");

  const load = async () => {
    if (!account.localActive) return;
    setLoading(true);
    setError(null);
    try {
      setView(await api.achOpen(account.steamId, game.appid));
    } catch (e) {
      setError(errorText(e));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void load();
    return () => void api.achClose(game.appid).catch(() => undefined);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [game.appid, account.localActive]);

  const inQueue = account.unlockQueue.includes(game.appid);
  const unlocked = view?.achievements.filter((a) => a.achieved).length ?? 0;

  return (
    <div className="min-w-0">
      <div className="mb-4 flex items-center gap-4">
        <div className="w-[150px] flex-none overflow-hidden rounded-md border border-line">
          <GameArt appid={game.appid} name={game.name} />
        </div>
        <div className="min-w-0 flex-1">
          <h2 className="heading truncate text-[20px] font-semibold leading-tight">{game.name}</h2>
          {view && (
            <div className="mt-1 flex items-center gap-3">
              <span className="text-muted">
                <span className="num text-ink">{unlocked}</span> of <span className="num">{view.achievements.length}</span>{" "}
                unlocked
              </span>
              <div className="h-1 w-40 overflow-hidden rounded-full bg-well">
                <div
                  className="h-full bg-ok"
                  style={{ width: `${view.achievements.length ? (unlocked / view.achievements.length) * 100 : 0}%` }}
                />
              </div>
            </div>
          )}
        </div>
        <Button
          icon={inQueue ? <X size={14} /> : <Plus size={15} />}
          onClick={() => void api.setInList(account.steamId, "unlockQueue", game.appid, !inQueue)}
        >
          {inQueue ? "Remove from unlock queue" : "Add to unlock queue"}
        </Button>
      </div>

      {!account.localActive ? null : loading && !view ? (
        <div className="rounded-xl border border-line bg-panel px-5 py-8 text-muted">
          Starting the game through Steam to read its achievements…
        </div>
      ) : error ? (
        <Callout tone="bad">
          <div className="font-medium">Couldn't read achievements</div>
          <div className="mt-1 text-muted">{error}</div>
          <div className="mt-3">
            <Button size="sm" onClick={() => void load()}>
              Try again
            </Button>
          </div>
        </Callout>
      ) : view ? (
        <>
          {view.warning && (
            <div className="mb-3 text-[13px] text-faint">{view.warning}</div>
          )}
          <div className="mb-3">
            <Tabs
              value={sub}
              onChange={setSub}
              tabs={[
                { value: "achievements", label: "Achievements", count: view.achievements.length },
                { value: "stats", label: "Stats", count: view.stats.length },
              ]}
            />
          </div>
          {sub === "achievements" ? (
            <AchievementList account={account} appid={game.appid} items={view.achievements} onChanged={load} />
          ) : (
            <StatsTable account={account} appid={game.appid} stats={view.stats} onChanged={load} />
          )}
        </>
      ) : null}
    </div>
  );
}

function AchievementList({
  account,
  appid,
  items,
  onChanged,
}: {
  account: Account;
  appid: number;
  items: AchItem[];
  onChanged: () => Promise<void>;
}) {
  const [pending, setPending] = useState<Record<string, boolean>>({});
  const [filter, setFilter] = useState<"all" | "locked" | "unlocked">("all");
  const [query, setQuery] = useState("");
  const [busy, setBusy] = useState(false);
  const [confirmReset, setConfirmReset] = useState(false);
  const run = useApp((s) => s.run);

  const state = (a: AchItem) => (a.apiName in pending ? pending[a.apiName] : a.achieved);
  const toggle = (a: AchItem, v: boolean) =>
    setPending((p) => {
      const n = { ...p };
      if (v === a.achieved) delete n[a.apiName];
      else n[a.apiName] = v;
      return n;
    });
  const setAll = (v: boolean) =>
    setPending(() => {
      const n: Record<string, boolean> = {};
      for (const a of items) if (!a.protected && a.achieved !== v) n[a.apiName] = v;
      return n;
    });

  const q = query.trim().toLowerCase();
  const shown = items.filter((a) => {
    const s = state(a);
    if (filter === "locked" && s) return false;
    if (filter === "unlocked" && !s) return false;
    return !q || a.name.toLowerCase().includes(q) || a.description.toLowerCase().includes(q) || a.apiName.toLowerCase().includes(q);
  });
  const changes = Object.entries(pending);
  const toUnlock = changes.filter(([, v]) => v).length;

  return (
    <div className="pb-16">
      <div className="mb-3 flex flex-wrap items-center gap-2">
        <label className="relative">
          <Search size={15} className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-faint" />
          <TextInput
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search achievements"
            aria-label="Search achievements"
            className="w-64 pl-8"
          />
        </label>
        <Select
          label="Show"
          value={filter}
          onChange={setFilter}
          options={[
            { value: "all", label: "All" },
            { value: "locked", label: "Locked" },
            { value: "unlocked", label: "Unlocked" },
          ]}
        />
        <div className="ml-auto flex gap-2">
          <Button size="sm" onClick={() => setAll(true)}>
            Unlock all
          </Button>
          <Button size="sm" onClick={() => setAll(false)}>
            Lock all
          </Button>
          <Button size="sm" variant="danger" icon={<RotateCcw size={13} />} onClick={() => setConfirmReset(true)}>
            Reset…
          </Button>
        </div>
      </div>

      <div className="overflow-hidden rounded-xl border border-line bg-panel">
        {shown.map((a, i) => {
          const s = state(a);
          const changed = a.apiName in pending;
          return (
            <div
              key={a.apiName}
              className={cx(
                "flex items-center gap-3.5 px-4 py-2.5",
                i > 0 && "border-t border-line",
                changed && "bg-run-soft",
              )}
            >
              <AchIcon item={a} unlocked={s} />
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <span className={cx("truncate", s ? "text-ink" : "text-muted")}>{a.name}</span>
                  {a.hidden && (
                    <span title="Hidden achievement" className="text-faint">
                      <EyeOff size={13} />
                    </span>
                  )}
                </div>
                <div className="truncate text-[12px] text-faint">
                  {a.description || a.apiName}
                </div>
              </div>
              <div className="w-28 flex-none text-right text-[12px] text-faint">
                {a.achieved && a.unlockTime > 0 && !changed ? dateText(a.unlockTime) : changed ? (s ? "Will unlock" : "Will lock") : ""}
              </div>
              <div className="num w-14 flex-none text-right text-[13px] text-muted" title="Players who have it">
                {a.percent != null ? `${a.percent.toFixed(1)}%` : ""}
              </div>
              {a.protected ? (
                <span className="flex w-9 justify-end text-faint" title="Protected: only the game's servers can change this">
                  <ShieldLock size={16} />
                </span>
              ) : (
                <Switch label={`${s ? "Lock" : "Unlock"} ${a.name}`} checked={s} onChange={(v) => toggle(a, v)} />
              )}
            </div>
          );
        })}
        {shown.length === 0 && <div className="px-4 py-6 text-muted">No achievements match.</div>}
      </div>

      {changes.length > 0 && (
        <div className="sticky bottom-4 mt-4 flex items-center gap-3 rounded-xl border border-line-strong bg-panel px-4 py-3 shadow-2xl">
          <span className="text-[13px]">
            <span className="num">{changes.length}</span> pending: unlock <span className="num">{toUnlock}</span>, lock{" "}
            <span className="num">{changes.length - toUnlock}</span>
          </span>
          <div className="ml-auto flex gap-2">
            <Button variant="ghost" onClick={() => setPending({})}>
              Discard
            </Button>
            <Button
              variant="primary"
              busy={busy}
              onClick={async () => {
                setBusy(true);
                const r = await run(null, () =>
                  api.achApply(
                    account.steamId,
                    appid,
                    changes.map(([apiName, achieved]) => ({ apiName, achieved })),
                  ),
                );
                if (r) {
                  const failed = Object.values(r.results).filter((ok) => !ok).length;
                  useApp
                    .getState()
                    .toast(
                      failed ? `${failed} change(s) were refused by Steam` : `Applied ${changes.length} change(s)`,
                      failed ? "error" : "ok",
                    );
                  setPending({});
                  await onChanged();
                }
                setBusy(false);
              }}
            >
              Apply changes
            </Button>
          </div>
        </div>
      )}

      {confirmReset && (
        <Modal title="Reset this game's stats?" onClose={() => setConfirmReset(false)}>
          <ResetConfirm
            onCancel={() => setConfirmReset(false)}
            onConfirm={async (withAch) => {
              await run("Stats reset", () => api.achResetAll(account.steamId, appid, withAch));
              setConfirmReset(false);
              await onChanged();
            }}
          />
        </Modal>
      )}
    </div>
  );
}

function ResetConfirm({ onCancel, onConfirm }: { onCancel: () => void; onConfirm: (withAch: boolean) => Promise<void> }) {
  const [withAch, setWithAch] = useState(false);
  const [busy, setBusy] = useState(false);
  return (
    <div className="flex flex-col gap-4">
      <p className="text-muted">
        Every stat for this game goes back to its default value. This can't be undone, and Steam keeps no backup.
      </p>
      <label className="flex items-center gap-3">
        <Switch label="Also lock every achievement" checked={withAch} onChange={setWithAch} />
        Also lock every achievement
      </label>
      <div className="flex justify-end gap-2">
        <Button variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
        <Button
          variant="danger"
          busy={busy}
          onClick={async () => {
            setBusy(true);
            await onConfirm(withAch);
            setBusy(false);
          }}
        >
          Reset stats
        </Button>
      </div>
    </div>
  );
}

function AchIcon({ item, unlocked }: { item: AchItem; unlocked: boolean }) {
  const [failed, setFailed] = useState(false);
  const url = unlocked ? item.iconUrl : item.iconGrayUrl || item.iconUrl;
  if (!url || failed) {
    return (
      <span
        className={cx(
          "flex size-9 flex-none items-center justify-center rounded-md border",
          unlocked ? "border-ok/50 bg-ok-soft text-ok" : "border-line bg-well text-faint",
        )}
        aria-hidden="true"
      >
        {unlocked ? <TrophyMark /> : <Lock size={14} />}
      </span>
    );
  }
  return (
    <img
      src={url}
      alt=""
      onError={() => setFailed(true)}
      className={cx("size-9 flex-none rounded-md", !unlocked && "opacity-70")}
    />
  );
}

function TrophyMark() {
  return (
    <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden="true">
      <path d="M8 21h8M12 17v4M7 4h10v5a5 5 0 0 1-10 0V4Z" />
      <path d="M17 6h3v2a3 3 0 0 1-3 3M7 6H4v2a3 3 0 0 0 3 3" />
    </svg>
  );
}

function StatsTable({
  account,
  appid,
  stats,
  onChanged,
}: {
  account: Account;
  appid: number;
  stats: StatItem[];
  onChanged: () => Promise<void>;
}) {
  const [edits, setEdits] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const run = useApp((s) => s.run);
  if (stats.length === 0) {
    return <Empty title="No stats to show" body="This game has no stats, or Steam's local stats file for it wasn't found." />;
  }
  const changed = Object.entries(edits).filter(([name, v]) => {
    const s = stats.find((x) => x.name === name);
    return s && v.trim() !== "" && Number(v) !== s.value;
  });
  return (
    <div className="pb-16">
      <div className="overflow-x-auto rounded-xl border border-line bg-panel">
        <table className="w-full text-left text-[13px]">
          <thead className="border-b border-line text-[12px] text-faint [&_th]:whitespace-nowrap">
            <tr>
              <th className="px-4 py-2.5 font-normal">Stat</th>
              <th className="px-3 py-2.5 font-normal">Rules</th>
              <th className="px-4 py-2.5 text-right font-normal">Value</th>
            </tr>
          </thead>
          <tbody>
            {stats.map((s) => {
              const v = edits[s.name] ?? (s.value ?? "").toString();
              const rules = [
                s.kind === "int" ? "Whole number" : "Decimal",
                s.min != null || s.max != null ? `${s.min ?? "…"} to ${s.max ?? "…"}` : null,
                s.incrementOnly ? "Can only go up" : null,
              ].filter(Boolean);
              return (
                <tr key={s.name} className="border-t border-line first:border-t-0">
                  <td className="px-4 py-2.5">
                    <div>{s.displayName}</div>
                    <div className="text-[12px] text-faint">{s.name}</div>
                  </td>
                  <td className="px-3 py-2.5 text-[12px] text-muted">{rules.join(", ")}</td>
                  <td className="px-4 py-2.5 text-right">
                    {s.protected ? (
                      <span className="inline-flex items-center gap-2 text-muted" title="Protected: only the game's servers can change this">
                        <span className="num">{s.value ?? "—"}</span>
                        <ShieldLock size={15} />
                      </span>
                    ) : (
                      <input
                        aria-label={`Value for ${s.displayName}`}
                        value={v}
                        inputMode="decimal"
                        onChange={(e) => setEdits((x) => ({ ...x, [s.name]: e.target.value }))}
                        className={cx(inputClass, "num w-32 text-right")}
                      />
                    )}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
      {changed.length > 0 && (
        <div className="sticky bottom-4 mt-4 flex items-center gap-3 rounded-xl border border-line-strong bg-panel px-4 py-3 shadow-2xl">
          <span className="text-[13px]">
            <span className="num">{changed.length}</span> stat(s) changed
          </span>
          <div className="ml-auto flex gap-2">
            <Button variant="ghost" onClick={() => setEdits({})}>
              Discard
            </Button>
            <Button
              variant="primary"
              busy={busy}
              onClick={async () => {
                setBusy(true);
                const payload = changed.map(([name, value]) => {
                  const s = stats.find((x) => x.name === name)!;
                  return { name, type: s.kind === "int" ? "int" : "float", value: Number(value) };
                });
                const r = await run(null, () => api.achSetStats(account.steamId, appid, payload));
                if (r) {
                  const failed = Object.values(r.results).filter((ok) => !ok).length;
                  useApp.getState().toast(failed ? `${failed} stat(s) were refused` : "Stats saved", failed ? "error" : "ok");
                  setEdits({});
                  await onChanged();
                }
                setBusy(false);
              }}
            >
              Save stats
            </Button>
          </div>
        </div>
      )}
    </div>
  );
}

// ----------------------------------------------------------- unlocker ---

function Unlocker({ account, running }: { account: Account; running: boolean }) {
  const status = useApp((s) => s.unlock[account.steamId]);
  const settings = useApp((s) => s.settings)!;
  useEnsureGames(account.steamId);
  const games = useApp((s) => s.games[account.steamId]);
  const run = useApp((s) => s.run);
  const go = useApp((s) => s.go);
  const now = useNow(1000);
  const [busy, setBusy] = useState(false);
  const [customize, setCustomize] = useState<number | null>(null);
  const us = settings.unlocker;
  const nameOf = (appid: number) => games?.find((g) => g.appid === appid)?.name ?? `App ${appid}`;
  const statusOf = (appid: number) => status?.games.find((g) => g.appid === appid);
  const queue = Array.from(new Set([...account.unlockQueue, ...(running ? (status?.games.map((g) => g.appid) ?? []) : [])]));
  const next = status?.games.filter((g) => g.nextAt > now).sort((a, b) => a.nextAt - b.nextAt)[0];

  const orderLabel = { common: "most common first", rare: "rarest first", schema: "in the game's own order" }[us.order];

  return (
    <div className="flex flex-col gap-5">
      <Section>
        <div className="flex flex-wrap items-center gap-6 px-5 py-4">
          <div>
            <Odometer
              text={String(status?.unlocked ?? 0)}
              live={running}
              className={cx("text-[34px] font-semibold", running ? "text-ink" : "text-muted")}
              label={`${status?.unlocked ?? 0} unlocked this run`}
            />
            <div className="mt-1.5 text-[13px] text-muted">unlocked this run</div>
          </div>
          <div className="min-w-0 flex-1">
            {running && next ? (
              <>
                <div className="text-[15px]">
                  Next: <span className="font-medium">{next.nextName}</span> in {next.name}
                </div>
                <div className="heading num mt-0.5 text-[24px] text-ink">{until(next.nextAt, now)}</div>
              </>
            ) : running ? (
              <div className="text-[15px]">Loading the next game…</div>
            ) : (
              <div className="text-muted">
                Unlocks one achievement at a time with a random {us.minDelayMinutes}–{us.maxDelayMinutes} minute gap,{" "}
                {orderLabel}
                {us.scheduleEnabled ? `, only between ${us.scheduleStart} and ${us.scheduleEnd}` : ""}. Protected
                achievements are always skipped.{" "}
                <button className="text-ink underline decoration-line-strong underline-offset-4 hover:decoration-ink" onClick={() => go("settings")}>
                  Change pacing
                </button>
              </div>
            )}
          </div>
          {running ? (
            <Button
              variant="danger"
              icon={<Square size={12} fill="currentColor" />}
              busy={busy}
              onClick={async () => {
                setBusy(true);
                await run(null, () => api.unlockStop(account.steamId));
                setBusy(false);
              }}
            >
              Stop unlocker
            </Button>
          ) : (
            <Button
              variant="primary"
              icon={<Play size={13} fill="currentColor" />}
              disabled={account.unlockQueue.length === 0 || !account.localActive}
              busy={busy}
              onClick={async () => {
                setBusy(true);
                await run(null, () => api.unlockStart(account.steamId));
                setBusy(false);
              }}
            >
              Start unlocker
            </Button>
          )}
        </div>
      </Section>

      {queue.length === 0 ? (
        <Empty
          title="The unlock queue is empty"
          body="Open a game in the Manager tab and choose Add to unlock queue. Games are unlocked in the order you add them."
        />
      ) : (
        <div className="overflow-hidden rounded-xl border border-line bg-panel">
          {queue.map((appid, i) => {
            const st = statusOf(appid);
            const imported = account.unlockImported[String(appid)]?.length ?? 0;
            const custom = account.unlockOrder[String(appid)]?.length ?? 0;
            const skipped = account.unlockSkip[String(appid)]?.length ?? 0;
            return (
              <div key={appid} className={cx("flex items-center gap-4 px-4 py-3", i > 0 && "border-t border-line")}>
                <span className="num w-5 text-right text-faint">{i + 1}</span>
                <GameIcon appid={appid} name={nameOf(appid)} size={30} />
                <div className="min-w-0 flex-1">
                  <div className="truncate">{nameOf(appid)}</div>
                  <div className="text-[12px] text-faint">
                    {[
                      imported ? `Using imported timing (${imported})` : null,
                      custom ? "Custom order" : null,
                      skipped ? `${skipped} skipped` : null,
                    ]
                      .filter(Boolean)
                      .join(", ") || "Default order and pacing"}
                  </div>
                </div>
                {st && st.total > 0 && (
                  <span className="num text-[13px] text-muted">
                    {st.total - st.remaining} of {st.total}
                  </span>
                )}
                {st && (
                  <Pill
                    tone={st.status === "error" ? "bad" : st.status === "done" ? "ok" : st.status === "queued" ? "neutral" : "run"}
                    lamp={st.status === "waiting" || st.status === "running"}
                  >
                    {{ queued: "Queued", running: "Unlocking", waiting: "Waiting", done: "Done", error: "Error" }[st.status]}
                  </Pill>
                )}
                <Button size="sm" onClick={() => setCustomize(appid)} disabled={!account.localActive}>
                  Customize
                </Button>
                <button
                  aria-label={`Remove ${nameOf(appid)} from the queue`}
                  className="text-faint hover:text-ink"
                  onClick={() => void api.setInList(account.steamId, "unlockQueue", appid, false)}
                >
                  <X size={16} />
                </button>
              </div>
            );
          })}
        </div>
      )}

      {customize !== null && (
        <CustomizeModal account={account} appid={customize} name={nameOf(customize)} onClose={() => setCustomize(null)} />
      )}
    </div>
  );
}

function CustomizeModal({
  account,
  appid,
  name,
  onClose,
}: {
  account: Account;
  appid: number;
  name: string;
  onClose: () => void;
}) {
  const [view, setView] = useState<AchView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [order, setOrder] = useState<string[]>(account.unlockOrder[String(appid)] ?? []);
  const [defaultOrder, setDefaultOrder] = useState<string[]>([]);
  // Only a reorder by the user becomes a custom order; otherwise imported timing
  // or the settings' order still applies.
  const [touched, setTouched] = useState((account.unlockOrder[String(appid)]?.length ?? 0) > 0);
  const [skip, setSkip] = useState<Set<string>>(new Set(account.unlockSkip[String(appid)] ?? []));
  const [profile, setProfile] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const run = useApp((s) => s.run);
  const imported = account.unlockImported[String(appid)] ?? [];

  useEffect(() => {
    api
      .achOpen(account.steamId, appid)
      .then((v) => {
        setView(v);
        const locked = v.achievements.filter((a) => !a.achieved && !a.protected).map((a) => a.apiName);
        setDefaultOrder(locked);
        setOrder((o) => {
          const kept = o.filter((x) => locked.includes(x));
          return [...kept, ...locked.filter((x) => !kept.includes(x))];
        });
      })
      .catch((e) => setError(errorText(e)));
    return () => void api.achClose(appid).catch(() => undefined);
  }, [account.steamId, appid]);

  const byName = new Map(view?.achievements.map((a) => [a.apiName, a]) ?? []);
  const move = (i: number, d: -1 | 1) => {
    setTouched(true);
    setOrder((o) => {
      const n = [...o];
      const j = i + d;
      if (j < 0 || j >= n.length) return o;
      [n[i], n[j]] = [n[j], n[i]];
      return n;
    });
  };

  return (
    <Modal title={`Unlock plan: ${name}`} onClose={onClose} width={640}>
      {error ? (
        <Callout tone="bad">{error}</Callout>
      ) : !view ? (
        <div className="py-6 text-muted">Reading achievements…</div>
      ) : (
        <div className="flex flex-col gap-5">
          <Field
            label="Copy another player's order and timing"
            hint="Paste a public Steam profile (URL, custom name or SteamID64) that has this game's achievements. The unlocker then follows their order and the gaps between their unlocks."
          >
            <div className="flex gap-2">
              <TextInput
                value={profile}
                onChange={(e) => setProfile(e.target.value)}
                placeholder="https://steamcommunity.com/id/…"
                className="flex-1"
              />
              <Button
                busy={busy === "import"}
                disabled={!profile.trim()}
                onClick={async () => {
                  setBusy("import");
                  const n = await run(null, () =>
                    api.achImportTimings(
                      account.steamId,
                      appid,
                      profile,
                      view.achievements.map((a) => ({ apiName: a.apiName, name: a.name })),
                    ),
                  );
                  if (n !== undefined) useApp.getState().toast(`Imported ${n} unlock times`, "ok");
                  setBusy(null);
                }}
              >
                Import
              </Button>
            </div>
            {imported.length > 0 && (
              <div className="mt-2 flex items-center gap-3 text-[13px] text-muted">
                Using {imported.length} imported unlock times.
                <button
                  className="text-ink underline decoration-line-strong underline-offset-4"
                  onClick={() => void api.achClearImported(account.steamId, appid)}
                >
                  Stop using them
                </button>
              </div>
            )}
          </Field>

          <div>
            <div className="mb-2 text-[13px]">Order and skips</div>
            <div className="mb-2 text-[12px] text-faint">
              A custom order takes priority over imported timing. Unticked achievements are never unlocked by the unlocker.
            </div>
            <div className="max-h-[320px] overflow-y-auto rounded-lg border border-line">
              {order.map((api_, i) => {
                const a = byName.get(api_);
                if (!a) return null;
                const on = !skip.has(api_);
                return (
                  <div key={api_} className={cx("flex items-center gap-2 px-3 py-1.5", i > 0 && "border-t border-line")}>
                    <Switch
                      label={on ? `Skip ${a.name}` : `Include ${a.name}`}
                      checked={on}
                      onChange={(v) =>
                        setSkip((s) => {
                          const n = new Set(s);
                          if (v) n.delete(api_);
                          else n.add(api_);
                          return n;
                        })
                      }
                    />
                    <span className={cx("flex-1 truncate text-[13px]", !on && "text-faint line-through")}>{a.name}</span>
                    <span className="num w-12 text-right text-[12px] text-faint">
                      {a.percent != null ? `${a.percent.toFixed(1)}%` : ""}
                    </span>
                    <button aria-label="Move up" className="text-faint hover:text-ink disabled:opacity-30" disabled={i === 0} onClick={() => move(i, -1)}>
                      <ArrowUp size={15} />
                    </button>
                    <button
                      aria-label="Move down"
                      className="text-faint hover:text-ink disabled:opacity-30"
                      disabled={i === order.length - 1}
                      onClick={() => move(i, 1)}
                    >
                      <ArrowDown size={15} />
                    </button>
                  </div>
                );
              })}
              {order.length === 0 && <div className="px-3 py-4 text-muted">Everything is already unlocked.</div>}
            </div>
          </div>

          <div className="flex justify-end gap-2">
            <Button
              variant="ghost"
              onClick={() => {
                setOrder(defaultOrder);
                setTouched(false);
                setSkip(new Set());
              }}
            >
              Reset to default
            </Button>
            <Button
              variant="primary"
              busy={busy === "save"}
              onClick={async () => {
                setBusy("save");
                await run("Unlock plan saved", () =>
                  api.achSetPrefs(account.steamId, appid, { order: touched ? order : [], skip: [...skip] }),
                );
                setBusy(null);
                onClose();
              }}
            >
              Save plan
            </Button>
          </div>
        </div>
      )}
    </Modal>
  );
}
