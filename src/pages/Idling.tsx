import { Play, Square, X } from "lucide-react";
import { useMemo, useState } from "react";
import { GameArt, GameIcon } from "../components/GameArt";
import { Odometer } from "../components/Odometer";
import { Button, Empty, PageHeader, Pill, Section, Switch, cx } from "../components/ui";
import { api } from "../lib/api";
import { clock } from "../lib/format";
import { HiddenNotice } from "../components/HiddenNotice";
import { useHiding } from "../lib/hidden";
import { useNow } from "../lib/hooks";
import { EMPTY, useActiveAccount, useApp, useEnsureGames } from "../lib/store";
import type { IdleGame } from "../lib/types";

const SOURCE_LABEL: Record<string, string> = {
  manual: "Started by you",
  auto: "Auto-idle",
  booster: "Playtime booster",
  farming: "Card farming",
  unlocker: "Achievement unlocker",
};

export function IdlingPage() {
  const account = useActiveAccount();
  const idle = useApp((s) => (account ? (s.idle[account.steamId] ?? EMPTY) : EMPTY));
  const games = useApp((s) => (account ? s.games[account.steamId] : undefined));
  const settings = useApp((s) => s.settings);
  const saveSettings = useApp((s) => s.saveSettings);
  const run = useApp((s) => s.run);
  const go = useApp((s) => s.go);
  const now = useNow(1000, idle.length > 0);
  const [busy, setBusy] = useState<string | null>(null);
  useEnsureGames(account?.steamId);
  // "Hide NSFW games" covers both lists on this page.
  const autoIdle = account?.autoIdle;
  const listedIds = useMemo(() => [...idle.map((g) => g.appid), ...(autoIdle ?? [])], [idle, autoIdle]);
  const hiding = useHiding(listedIds);

  if (!account || !settings) return null;

  const nameOf = (appid: number) =>
    games?.find((g) => g.appid === appid)?.name ?? idle.find((g) => g.appid === appid)?.name ?? `App ${appid}`;
  const sorted = idle.filter((g) => !hiding.hides(g.appid)).sort((a, b) => a.startedAt - b.startedAt);
  const unseen = idle.length - sorted.length;
  const total = sorted.reduce((sum, g) => sum + (now - g.startedAt), 0);
  const autoIdleShown = account.autoIdle.filter((appid) => !hiding.hides(appid));

  return (
    <div className="pb-10">
      <PageHeader
        title="Idling"
        sub={
          account.mode === "direct"
            ? "Games run over IdleKit's own connection to Steam, so the Steam client doesn't need to be open. Up to 32 at once."
            : "Each game runs as a tiny background process that the Steam client on this PC counts as playing. Up to 32 at once."
        }
        actions={
          <>
            <Button
              icon={<Play size={15} />}
              disabled={account.autoIdle.length === 0}
              busy={busy === "auto"}
              onClick={async () => {
                setBusy("auto");
                const r = await run(null, () => api.autoIdleStart(account.steamId));
                setBusy(null);
                const failed = r?.filter((x) => !x.ok) ?? [];
                if (failed.length) useApp.getState().toast(failed[0].error ?? "Some games could not start", "error");
              }}
            >
              Start auto-idle list
            </Button>
            <Button
              variant="danger"
              icon={<Square size={13} fill="currentColor" />}
              disabled={idle.length === 0}
              busy={busy === "stop"}
              onClick={async () => {
                setBusy("stop");
                await run("Stopped all games", () => api.idleStopAll(account.steamId));
                setBusy(null);
              }}
            >
              Stop all
            </Button>
          </>
        }
      />

      <HiddenNotice hiding={hiding} />

      {idle.length === 0 ? (
        <Empty
          title="Nothing is idling"
          body="Pick games in your library and choose Idle, or start your auto-idle list. Card farming and the achievement unlocker start games here on their own."
          action={<Button onClick={() => go("library")}>Open library</Button>}
        />
      ) : sorted.length === 0 ? (
        <p className="mb-3 text-muted">
          <span className="num text-ink">{unseen}</span> {unseen === 1 ? "game is" : "games are"} running, not shown here.
        </p>
      ) : (
        <>
          <p className="mb-3 text-muted">
            <span className="num text-ink">{sorted.length}</span> {sorted.length === 1 ? "game" : "games"} running,{" "}
            <span className="num text-ink">{clock(total)}</span> of playtime this session
            {unseen > 0 && <>, and {unseen} more not shown</>}
          </p>
          <div className="overflow-hidden rounded-xl border border-line bg-panel">
            {sorted.map((g, i) => (
              <IdleRow
                key={g.appid}
                game={g}
                now={now}
                first={i === 0}
                onStop={() => void run(null, () => api.idleStop(account.steamId, [g.appid]))}
              />
            ))}
          </div>
        </>
      )}

      <div className="mt-8 grid gap-5 lg:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
        <Section
          title="Auto-idle list"
          aside={
            <label className="flex items-center gap-2 text-[13px] text-muted">
              Start when IdleKit opens
              <Switch
                label="Start the auto-idle list when IdleKit opens"
                checked={settings.autoIdleOnLaunch}
                onChange={(v) => void saveSettings({ autoIdleOnLaunch: v })}
              />
            </label>
          }
        >
          {autoIdleShown.length === 0 ? (
            <p className="px-4 py-4 text-muted">
              {account.autoIdle.length === 0
                ? "Games you add here (from a game's menu in the library) can be started together with one click."
                : `${account.autoIdle.length} on this list, not shown here.`}
            </p>
          ) : (
            <ul className="divide-y divide-line">
              {autoIdleShown.map((appid) => (
                <li key={appid} className="flex items-center gap-3 px-4 py-2">
                  <GameIcon appid={appid} name={nameOf(appid)} size={24} />
                  <span className="flex-1 truncate">{nameOf(appid)}</span>
                  {idle.some((g) => g.appid === appid) && <Pill tone="run" lamp>Idling</Pill>}
                  <button
                    aria-label={`Remove ${nameOf(appid)} from auto-idle`}
                    onClick={() => void api.setInList(account.steamId, "autoIdle", appid, false)}
                    className="text-faint hover:text-ink"
                  >
                    <X size={15} />
                  </button>
                </li>
              ))}
            </ul>
          )}
        </Section>

        <Section title="How playtime is counted">
          <div className="flex flex-col gap-2 px-4 py-4 text-muted">
            <p>
              Steam records playtime for every game that is running, so idled hours show up on your profile and count
              toward card drops.
            </p>
            <p>
              Steam only allows one place to play an account at a time. If you start a game on another PC, Steam pauses
              IdleKit's games{account.mode === "direct" ? " and IdleKit resumes them when you stop" : ""}.
            </p>
          </div>
        </Section>
      </div>
    </div>
  );
}

function IdleRow({ game, now, first, onStop }: { game: IdleGame; now: number; first: boolean; onStop: () => void }) {
  const elapsed = now - game.startedAt;
  const target = game.targetMinutes ? game.targetMinutes * 60_000 : null;
  // A boost counts from when it was set, which can be long after the game began idling.
  const boosted = Math.max(0, now - (game.boostStartedAt ?? game.startedAt));
  const pct = target ? Math.min(100, (boosted / target) * 100) : null;
  return (
    <div className={cx("flex items-center gap-5 px-4 py-3.5", !first && "border-t border-line")}>
      <div className="w-[136px] flex-none overflow-hidden rounded border border-line">
        <GameArt appid={game.appid} name={game.name} />
      </div>
      <div className="min-w-0 flex-1">
        <div className="truncate text-[15px] font-medium">{game.name}</div>
        <div className="mt-1.5 flex flex-wrap gap-1.5">
          {game.sources.map((s) => (
            <Pill key={s}>{SOURCE_LABEL[s] ?? s}</Pill>
          ))}
        </div>
        {target && pct !== null && (
          <div className="mt-2.5 flex max-w-sm items-center gap-3">
            <div className="h-1 flex-1 overflow-hidden rounded-full bg-well">
              <div className="h-full bg-run" style={{ width: `${pct}%` }} />
            </div>
            <span className="num text-[12px] text-faint">
              {Math.floor(boosted / 60_000)} of {game.targetMinutes} min
            </span>
          </div>
        )}
      </div>
      <Odometer text={clock(elapsed)} live className="text-[26px] font-medium text-ink" label={`Running for ${clock(elapsed)}`} />
      <Button size="sm" variant="quiet" icon={<Square size={11} fill="currentColor" />} onClick={onStop}>
        Stop
      </Button>
    </div>
  );
}
