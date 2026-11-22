import { Check, Ellipsis, Play, Plus, RefreshCw, Search, Square, Star, Trophy } from "lucide-react";
import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { GameArt } from "../components/GameArt";
import { HiddenNotice } from "../components/HiddenNotice";
import { Button, Callout, Empty, Field, IconButton, Modal, NumberInput, PageHeader, Select, TextInput, cx } from "../components/ui";
import { api } from "../lib/api";
import { clock, hoursText } from "../lib/format";
import { useNow } from "../lib/hooks";
import { useAppIds, useHiding } from "../lib/hidden";
import { EMPTY, useActiveAccount, useApp } from "../lib/store";
import type { Account, GameEntry, IdleGame } from "../lib/types";

type Filter = "all" | "favorites" | "unplayed" | "manual" | "idling";
type Sort = "name" | "playtime" | "recent";
/** The NSFW box beside the sort box: the "Hide NSFW games" setting, or a look at just those games. */
type Nsfw = "hide" | "show" | "only";
const PAGE = 150;

export function LibraryPage() {
  const account = useActiveAccount();
  const games = useApp((s) => (account ? s.games[account.steamId] : undefined));
  const loading = useApp((s) => (account ? !!s.gamesLoading[account.steamId] : false));
  const error = useApp((s) => (account ? s.gamesError[account.steamId] : null));
  const idle = useApp((s) => (account ? (s.idle[account.steamId] ?? EMPTY) : EMPTY));
  const loadGames = useApp((s) => s.loadGames);
  const cardStyle = useApp((s) => s.settings?.cardStyle ?? "cover");
  const run = useApp((s) => s.run);
  const go = useApp((s) => s.go);

  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<Filter>("all");
  const [sort, setSort] = useState<Sort>("name");
  const [limit, setLimit] = useState(PAGE);
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [adding, setAdding] = useState(false);
  const [boosting, setBoosting] = useState<GameEntry | null>(null);

  useEffect(() => {
    if (account && !games && !loading) void loadGames(account.steamId);
    setSelected(new Set());
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [account?.steamId]);

  const idleMap = useMemo(() => new Map(idle.map((g) => [g.appid, g])), [idle]);
  // "Hide NSFW games": what stays out of the grid, the count and the selection.
  // "Only NSFW games" is a look at just those games on this page, for as long
  // as the page is open; it changes nothing about the setting.
  const saveSettings = useApp((s) => s.saveSettings);
  const [onlyNsfw, setOnlyNsfw] = useState(false);
  const hiding = useHiding(useAppIds(games), onlyNsfw);
  const nsfw: Nsfw = onlyNsfw ? "only" : hiding.on ? "hide" : "show";
  const listed = useMemo(() => {
    if (!games) return games;
    if (onlyNsfw) return games.filter((g) => hiding.labelled(g.appid));
    return hiding.on ? games.filter((g) => !hiding.hides(g.appid)) : games;
  }, [games, hiding, onlyNsfw]);
  useEffect(() => {
    setSelected(new Set());
  }, [nsfw]);
  const chooseNsfw = (v: Nsfw) => {
    setOnlyNsfw(v === "only");
    setLimit(PAGE);
    if (v !== "only" && (v === "hide") !== hiding.on) void saveSettings({ hideNsfw: v === "hide" });
  };
  const waitingForSteam = hiding.checking + hiding.unanswered;
  const favorites = account?.favorites;
  const favSet = useMemo(() => new Set(favorites ?? []), [favorites]);
  const steamId = account?.steamId;

  const shown = useMemo(() => {
    if (!listed) return [];
    const q = query.trim().toLowerCase();
    let list = listed.filter((g) => {
      if (q && !g.name.toLowerCase().includes(q) && String(g.appid) !== q) return false;
      if (filter === "favorites") return favSet.has(g.appid);
      if (filter === "unplayed") return g.playtimeMinutes === 0;
      if (filter === "manual") return g.manual;
      if (filter === "idling") return idleMap.has(g.appid);
      return true;
    });
    if (sort === "playtime") list = [...list].sort((a, b) => b.playtimeMinutes - a.playtimeMinutes);
    if (sort === "recent") list = [...list].sort((a, b) => b.lastPlayed - a.lastPlayed);
    if (filter === "all" && !q) {
      // Favorites first in the default view.
      list = [...list.filter((g) => favSet.has(g.appid)), ...list.filter((g) => !favSet.has(g.appid))];
    }
    return list;
  }, [listed, favSet, query, filter, sort, idleMap]);

  // What a tile is given stays the same from one draw to the next, so a tile
  // is only drawn again when something about that game changed.
  const idleGames = useCallback(
    async (list: { appid: number; name: string }[]) => {
      if (!steamId) return;
      const results = await run(null, () => api.idleStart(steamId, list));
      if (!results) return;
      const failed = results.filter((r) => !r.ok);
      const ok = results.length - failed.length;
      if (ok) useApp.getState().toast(`Idling ${ok} ${ok === 1 ? "game" : "games"}`, "ok");
      if (failed.length) useApp.getState().toast(failed[0].error ?? "Some games could not start", "error");
    },
    [steamId, run],
  );
  const toggleSelect = useCallback(
    (appid: number) =>
      setSelected((s) => {
        const n = new Set(s);
        if (n.has(appid)) n.delete(appid);
        else n.add(appid);
        return n;
      }),
    [],
  );
  const idleOne = useCallback((g: GameEntry) => void idleGames([{ appid: g.appid, name: g.name }]), [idleGames]);
  const stopOne = useCallback(
    (appid: number) => {
      if (steamId) void run(null, () => api.idleStop(steamId, [appid]));
    },
    [steamId, run],
  );

  if (!account) {
    return (
      <div>
        <PageHeader title="Library" />
        <Empty
          title="No Steam account yet"
          body="Add the account signed in to your Steam client, or sign in directly with a QR code or password."
          action={
            <Button variant="primary" onClick={() => go("accounts")}>
              Add an account
            </Button>
          }
        />
      </div>
    );
  }

  return (
    <div className="pb-20">
      <PageHeader
        title="Library"
        sub={
          listed
            ? `${listed.length.toLocaleString()} games on ${account.personaName || account.accountName}`
            : "Loading your games…"
        }
        actions={
          <>
            <Button icon={<Plus size={15} />} onClick={() => setAdding(true)}>
              Add by AppID
            </Button>
            <Button
              icon={<RefreshCw size={15} className={cx(loading && "animate-spin")} />}
              disabled={loading}
              onClick={() => void loadGames(account.steamId, true)}
            >
              Refresh
            </Button>
          </>
        }
      />

      <div className="mb-4 flex flex-wrap items-center gap-2">
        <label className="relative">
          <Search size={15} className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-faint" />
          <TextInput
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setLimit(PAGE);
            }}
            placeholder="Search by name or AppID"
            aria-label="Search games"
            className="w-72 pl-8"
          />
        </label>
        <Select
          label="Show"
          value={filter}
          onChange={(v) => {
            setFilter(v);
            setLimit(PAGE);
          }}
          options={[
            { value: "all", label: "All games" },
            { value: "favorites", label: "Favorites" },
            { value: "idling", label: "Idling now" },
            { value: "unplayed", label: "Never played" },
            { value: "manual", label: "Added by AppID" },
          ]}
        />
        <Select
          label="Sort"
          value={sort}
          onChange={setSort}
          options={[
            { value: "name", label: "Name" },
            { value: "playtime", label: "Most played" },
            { value: "recent", label: "Recently played" },
          ]}
        />
        <Select
          label="NSFW games"
          value={nsfw}
          onChange={chooseNsfw}
          options={[
            { value: "hide", label: "NSFW games hidden" },
            { value: "show", label: "NSFW games shown" },
            { value: "only", label: "Only NSFW games" },
          ]}
        />
      </div>

      {error && (
        <div className="mb-4">
          <Callout tone="bad">
            <div className="font-medium">Couldn't load this account's games</div>
            <div className="mt-1 text-muted">{error}</div>
            <div className="mt-3 flex gap-2">
              <Button size="sm" onClick={() => go("accounts")}>
                Open accounts
              </Button>
              <Button size="sm" onClick={() => setAdding(true)}>
                Add a game by AppID
              </Button>
            </div>
          </Callout>
        </div>
      )}

      {onlyNsfw ? (
        waitingForSteam > 0 && (
          <p className="mb-4 flex flex-wrap items-center gap-3 text-[13px] text-muted" role="status">
            {hiding.unanswered > 0
              ? `Steam couldn't be asked about ${hiding.unanswered.toLocaleString()} ${hiding.unanswered === 1 ? "game" : "games"}, so some may be missing here.`
              : `Asking Steam about ${hiding.checking.toLocaleString()} ${hiding.checking === 1 ? "game" : "games"}…`}
            {hiding.unanswered > 0 && (
              <Button size="sm" onClick={hiding.retry}>
                Ask again
              </Button>
            )}
          </p>
        )
      ) : (
        <HiddenNotice hiding={hiding} />
      )}

      {games && shown.length === 0 && !error && !(waitingForSteam > 0 && listed?.length === 0) && (
        <Empty
          title={query || filter !== "all" ? "No games match" : onlyNsfw ? "No NSFW games" : "No games yet"}
          body={
            query || filter !== "all"
              ? "Try a different search or filter."
              : onlyNsfw
                ? "Steam labels none of this account's games “Adult Only Sexual Content” or “Frequent Nudity or Sexual Content”."
                : "Refresh to load your library, or add a game by its AppID (the number in its store page URL)."
          }
        />
      )}

      <div
        className={cx(
          "grid gap-x-4 gap-y-5",
          cardStyle === "cover"
            ? "grid-cols-[repeat(auto-fill,minmax(150px,1fr))]"
            : "grid-cols-[repeat(auto-fill,minmax(216px,1fr))]",
        )}
      >
        {shown.slice(0, limit).map((g) => (
          <GameTile
            key={g.appid}
            game={g}
            shape={cardStyle}
            steamId={account.steamId}
            fav={favSet.has(g.appid)}
            idle={idleMap.get(g.appid)}
            selected={selected.has(g.appid)}
            onSelect={toggleSelect}
            onIdle={idleOne}
            onStop={stopOne}
            onBoost={setBoosting}
          />
        ))}
      </div>
      {shown.length > limit && (
        <div className="mt-6">
          <Button onClick={() => setLimit((l) => l + PAGE)}>Show {Math.min(PAGE, shown.length - limit)} more</Button>
        </div>
      )}

      {selected.size > 0 && (
        <div className="fixed bottom-5 left-1/2 z-30 flex -translate-x-1/2 items-center gap-3 rounded-xl border border-line-strong bg-panel px-4 py-2.5 shadow-2xl">
          <span className="num text-[14px]">{selected.size} selected</span>
          <Button
            variant="primary"
            size="sm"
            icon={<Play size={14} />}
            onClick={() => {
              const list = (games ?? []).filter((g) => selected.has(g.appid)).map((g) => ({ appid: g.appid, name: g.name }));
              void idleGames(list).then(() => setSelected(new Set()));
            }}
          >
            Idle selected
          </Button>
          <Button
            size="sm"
            onClick={() => {
              void run(null, () => api.setInListMany(account.steamId, "autoIdle", [...selected], true)).then((done) => {
                if (done !== undefined) useApp.getState().toast("Added to auto-idle", "ok");
              });
              setSelected(new Set());
            }}
          >
            Add to auto-idle
          </Button>
          <Button size="sm" variant="ghost" onClick={() => setSelected(new Set())}>
            Clear
          </Button>
        </div>
      )}

      {adding && <AddGameModal account={account} onClose={() => setAdding(false)} />}
      {boosting && <BoostModal account={account} game={boosting} onClose={() => setBoosting(null)} />}
    </div>
  );
}

const MENU_WIDTH = 240;

/**
 * One game in the grid. Memoized, and given only what is about this game, so
 * typing in the filter, selecting another game or a change to some list
 * doesn't draw every tile again.
 */
const GameTile = memo(function GameTile({
  game,
  shape,
  steamId,
  fav,
  idle,
  selected,
  onSelect,
  onIdle,
  onStop,
  onBoost,
}: {
  game: GameEntry;
  shape: "cover" | "banner";
  steamId: string;
  fav: boolean;
  idle?: IdleGame;
  selected: boolean;
  onSelect: (appid: number) => void;
  onIdle: (game: GameEntry) => void;
  onStop: (appid: number) => void;
  onBoost: (game: GameEntry) => void;
}) {
  // The menu is wider than a cover card, so it opens toward whichever side has room.
  const [menu, setMenu] = useState<null | "left" | "right">(null);
  const tileRef = useRef<HTMLDivElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const menuButtonRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!menu) return;
    const close = (e: MouseEvent) => {
      const t = e.target as Node;
      if (!menuRef.current?.contains(t) && !menuButtonRef.current?.contains(t)) setMenu(null);
    };
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  }, [menu]);
  const toggleMenu = () => {
    const left = tileRef.current?.getBoundingClientRect().left ?? 0;
    setMenu((m) => (m ? null : window.innerWidth - left < MENU_WIDTH + 24 ? "right" : "left"));
  };

  const toggle = (list: string, on: boolean) => void api.setInList(steamId, list, game.appid, on);

  return (
    <div className="group relative" ref={tileRef}>
      <div
        className={cx(
          "relative overflow-hidden rounded-md border",
          idle ? "border-run" : selected ? "border-ink/60" : "border-line",
        )}
      >
        <GameArt appid={game.appid} name={game.name} shape={shape} />
        <button
          onClick={() => onSelect(game.appid)}
          aria-label={selected ? `Deselect ${game.name}` : `Select ${game.name}`}
          aria-pressed={selected}
          className={cx(
            "absolute left-2 top-2 flex size-5.5 items-center justify-center rounded border transition-opacity",
            selected
              ? "border-transparent bg-ink text-bg opacity-100"
              : "border-white/60 bg-black/40 text-transparent opacity-0 group-hover:opacity-100 focus:opacity-100 [@media(hover:none)]:opacity-100",
          )}
        >
          <Check size={13} strokeWidth={3} />
        </button>
        {/* Shown on hover; always shown where there is no pointer to hover with (touch screens). */}
        <div className="absolute right-2 top-2 flex gap-1 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100 [@media(hover:none)]:opacity-100">
          {idle ? (
            <button
              onClick={() => onStop(game.appid)}
              className="flex h-7 items-center gap-1.5 rounded-md bg-black/70 px-2.5 text-[12px] text-white hover:bg-black/85"
            >
              <Square size={12} fill="currentColor" /> Stop
            </button>
          ) : (
            <button
              onClick={() => onIdle(game)}
              className="flex h-7 items-center gap-1.5 rounded-md bg-run px-2.5 text-[12px] font-semibold text-run-ink hover:brightness-110"
            >
              <Play size={12} fill="currentColor" /> Idle
            </button>
          )}
        </div>
      </div>
      <div className="mt-2">
        <div className="truncate text-[13px] font-medium" title={game.name}>
          {game.name}
        </div>
        <div className="flex h-6.5 items-center">
          <div className="min-w-0 flex-1 truncate">
            {idle ? (
              <IdleMeta since={idle.startedAt} />
            ) : (
              <span className="num text-[12px] text-faint">
                {game.manual && game.playtimeMinutes === 0 ? "Added by AppID" : hoursText(game.playtimeMinutes)}
              </span>
            )}
          </div>
          <IconButton compact label={fav ? "Remove from favorites" : "Add to favorites"} active={fav} onClick={() => toggle("favorites", !fav)}>
            <Star size={15} fill={fav ? "currentColor" : "none"} />
          </IconButton>
          <div ref={menuButtonRef} className="flex">
            <IconButton compact label="More actions" aria-expanded={!!menu} onClick={toggleMenu}>
              <Ellipsis size={16} />
            </IconButton>
          </div>
        </div>
      </div>
      {menu && (
        <TileMenu
          ref={menuRef}
          side={menu}
          game={game}
          steamId={steamId}
          toggle={toggle}
          onBoost={() => {
            setMenu(null);
            onBoost(game);
          }}
          onRemoved={() => setMenu(null)}
        />
      )}
    </div>
  );
});

/** A tile's "more actions" menu. Only exists while open, so the lists it shows are read only then. */
function TileMenu({
  ref,
  side,
  game,
  steamId,
  toggle,
  onBoost,
  onRemoved,
}: {
  ref: React.Ref<HTMLDivElement>;
  side: "left" | "right";
  game: GameEntry;
  steamId: string;
  toggle: (list: string, on: boolean) => void;
  onBoost: () => void;
  onRemoved: () => void;
}) {
  const go = useApp((s) => s.go);
  const account = useActiveAccount();
  if (!account) return null;
  return (
    <div
      ref={ref}
      style={{ width: MENU_WIDTH }}
      className={cx(
        "absolute top-full z-30 mt-1 overflow-hidden rounded-lg border border-line-strong bg-panel py-1 shadow-2xl",
        side === "left" ? "left-0" : "right-0",
      )}
    >
      <MenuItem
        onClick={() => {
          go("achievements", { achAppid: game.appid });
        }}
      >
        <Trophy size={14} /> Achievements
      </MenuItem>
      <MenuItem onClick={onBoost}>Boost playtime…</MenuItem>
      <MenuItem onClick={() => toggle("autoIdle", !account.autoIdle.includes(game.appid))}>
        {account.autoIdle.includes(game.appid) ? "Remove from auto-idle" : "Add to auto-idle"}
      </MenuItem>
      <MenuItem onClick={() => toggle("farmBlacklist", !account.farmBlacklist.includes(game.appid))}>
        {account.farmBlacklist.includes(game.appid) ? "Allow card farming" : "Never farm cards for this game"}
      </MenuItem>
      <MenuItem onClick={() => toggle("unlockQueue", !account.unlockQueue.includes(game.appid))}>
        {account.unlockQueue.includes(game.appid) ? "Remove from unlock queue" : "Add to unlock queue"}
      </MenuItem>
      {game.manual && (
        <MenuItem
          danger
          onClick={() => {
            onRemoved();
            void api.removeManualGame(steamId, game.appid).then(() => useApp.getState().loadGames(steamId));
          }}
        >
          Remove from library
        </MenuItem>
      )}
    </div>
  );
}

function IdleMeta({ since }: { since: number }) {
  const now = useNow(1000);
  return (
    <span className="flex items-center gap-1.5 text-[12px] text-run" title="Idling">
      <span className="lamp on" />
      <span className="sr-only">Idling for</span>
      <span className="num">{clock(now - since)}</span>
    </span>
  );
}

function MenuItem({ children, onClick, danger }: { children: React.ReactNode; onClick: () => void; danger?: boolean }) {
  return (
    <button
      onClick={onClick}
      className={cx(
        "flex w-full items-center gap-2 px-3 py-2 text-left text-[13px] hover:bg-raised",
        danger ? "text-bad" : "text-ink",
      )}
    >
      {children}
    </button>
  );
}

function AddGameModal({ account, onClose }: { account: Account; onClose: () => void }) {
  const [appid, setAppid] = useState("");
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const run = useApp((s) => s.run);
  const id = Number(appid.replace(/\D/g, ""));
  return (
    <Modal title="Add a game by AppID" onClose={onClose}>
      <div className="flex flex-col gap-4">
        <p className="text-muted">
          For games you own that don't show up, like ones removed from the store. The AppID is the number in the game's store
          or SteamDB URL.
        </p>
        <Field label="AppID">
          <TextInput value={appid} onChange={(e) => setAppid(e.target.value)} inputMode="numeric" placeholder="e.g. 440" />
        </Field>
        <Field label="Name" hint="Optional. Shown in your library.">
          <TextInput value={name} onChange={(e) => setName(e.target.value)} placeholder="Team Fortress 2" />
        </Field>
        <div className="flex justify-end gap-2 pt-1">
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            busy={busy}
            disabled={!id}
            onClick={async () => {
              setBusy(true);
              await run("Game added", () => api.addManualGame(account.steamId, id, name));
              await useApp.getState().loadGames(account.steamId);
              setBusy(false);
              onClose();
            }}
          >
            Add game
          </Button>
        </div>
      </div>
    </Modal>
  );
}

function BoostModal({ account, game, onClose }: { account: Account; game: GameEntry; onClose: () => void }) {
  const [hours, setHours] = useState(2);
  const [busy, setBusy] = useState(false);
  const run = useApp((s) => s.run);
  return (
    <Modal title={`Boost playtime: ${game.name}`} onClose={onClose}>
      <div className="flex flex-col gap-4">
        <p className="text-muted">
          Idles the game and stops it automatically once this session reaches the target. It currently has{" "}
          <span className="num text-ink">{hoursText(game.playtimeMinutes)}</span> on record.
        </p>
        <Field label="Idle for">
          <NumberInput value={hours} onChange={setHours} min={0.1} max={1000} step={0.5} suffix="hours" />
        </Field>
        <div className="flex justify-end gap-2 pt-1">
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            busy={busy}
            disabled={hours <= 0}
            onClick={async () => {
              setBusy(true);
              const r = await run(null, () =>
                api.boosterStart(account.steamId, game.appid, game.name, Math.round(hours * 60)),
              );
              setBusy(false);
              if (r?.every((x) => x.ok)) {
                useApp.getState().toast(`Boosting ${game.name} for ${hours} h`, "ok");
                onClose();
              } else if (r) {
                useApp.getState().toast(r.find((x) => !x.ok)?.error ?? "Couldn't start", "error");
              }
            }}
          >
            Start boosting
          </Button>
        </div>
      </div>
    </Modal>
  );
}
