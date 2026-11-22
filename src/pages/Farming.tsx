import { Play, Search, Square } from "lucide-react";
import { useState } from "react";
import { GameIcon } from "../components/GameArt";
import { Odometer } from "../components/Odometer";
import { Button, Callout, Empty, PageHeader, Pill, Section, Switch, cx, type Tone } from "../components/ui";
import { api } from "../lib/api";
import { hoursFromFloat, until } from "../lib/format";
import { HiddenNotice } from "../components/HiddenNotice";
import { useAppIds, useHiding } from "../lib/hidden";
import { useNow } from "../lib/hooks";
import { EMPTY, useActiveAccount, useApp, useEnsureGames } from "../lib/store";
import type { FarmGame } from "../lib/types";

const STATUS: Record<FarmGame["status"], { label: string; tone: Tone }> = {
  idling: { label: "Idling", tone: "run" },
  queued: { label: "Waiting", tone: "neutral" },
  done: { label: "All cards received", tone: "ok" },
  skipped: { label: "Skipped", tone: "neutral" },
  // Given up on for the run: it never started, or its card page kept failing. The note says which.
  error: { label: "Couldn't farm", tone: "bad" },
};

export function FarmingPage() {
  const account = useActiveAccount();
  const status = useApp((s) => (account ? s.farm[account.steamId] : undefined));
  const settings = useApp((s) => s.settings);
  const run = useApp((s) => s.run);
  const go = useApp((s) => s.go);
  const now = useNow(1000, !!status?.running);
  const [preview, setPreview] = useState<FarmGame[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  useEnsureGames(account?.steamId);
  const running = !!status?.running;
  const games = running || !preview ? (status?.games ?? preview ?? EMPTY) : preview;
  // "Hide NSFW games": the totals still count every game, the table leaves these out.
  const hiding = useHiding(useAppIds(games));

  if (!account || !settings) return null;
  const fs = settings.farming;
  const rows = games.filter((g) => !hiding.hides(g.appid));
  const unseen = games.length - rows.length;
  const counted = games.filter((g) => g.status !== "skipped" && g.status !== "error");
  const dropsLeft = running ? status!.totalDrops : counted.reduce((n, g) => n + g.dropsRemaining, 0);
  const noWeb = !account.hasToken && !account.hasCookie;
  const phase = status?.phase ?? "idle";

  const toggleBlacklist = (appid: number, on: boolean) =>
    void api.setInList(account.steamId, "farmBlacklist", appid, on).then(() => {
      setPreview((p) =>
        p?.map((g) =>
          g.appid === appid
            ? { ...g, status: on ? "skipped" : "queued", note: on ? "Blacklisted" : "" }
            : g,
        ) ?? null,
      );
    });

  return (
    <div className="pb-10">
      <PageHeader
        title="Card farming"
        sub="Reads your badge pages to find games that still have trading card drops, then idles them until the drops run out."
        actions={
          running ? (
            <Button
              variant="danger"
              icon={<Square size={13} fill="currentColor" />}
              busy={busy === "stop"}
              onClick={async () => {
                setBusy("stop");
                await run(null, () => api.farmStop(account.steamId));
                setBusy(null);
              }}
            >
              Stop farming
            </Button>
          ) : (
            <>
              <Button
                icon={<Search size={15} />}
                disabled={noWeb}
                busy={busy === "scan"}
                onClick={async () => {
                  setBusy("scan");
                  const r = await run(null, () => api.farmScan(account.steamId));
                  if (r) setPreview(r);
                  setBusy(null);
                }}
              >
                Check badges
              </Button>
              <Button
                variant="primary"
                icon={<Play size={14} fill="currentColor" />}
                disabled={noWeb}
                busy={busy === "start"}
                onClick={async () => {
                  setBusy("start");
                  await run(null, () => api.farmStart(account.steamId));
                  setBusy(null);
                }}
              >
                Start farming
              </Button>
            </>
          )
        }
      />

      {noWeb && (
        <div className="mb-5">
          <Callout tone="run">
            <div className="font-medium">Card farming needs a web session for this account</div>
            <div className="mt-1 text-muted">
              IdleKit reads your badge pages on steamcommunity.com to see which games still drop cards. Sign in with a QR
              code or password, or paste a steamLoginSecure cookie, from Accounts.
            </div>
            <div className="mt-3">
              <Button size="sm" onClick={() => go("accounts")}>
                Set up web session
              </Button>
            </div>
          </Callout>
        </div>
      )}

      <Section className="mb-6">
        <div className="grid gap-6 px-6 py-5 lg:grid-cols-[auto_minmax(0,1fr)] lg:items-center">
          <div className="flex items-end gap-8">
            <div>
              <Odometer
                text={String(dropsLeft)}
                live={running}
                className={cx("text-[44px] font-semibold", running ? "text-ink" : "text-muted")}
                label={`${dropsLeft} card drops left`}
              />
              <div className="mt-2 text-[13px] text-muted">card drops left</div>
            </div>
            <div>
              <Odometer
                text={String(status?.cardsFarmed ?? 0)}
                className="text-[26px] font-semibold text-ok"
                label={`${status?.cardsFarmed ?? 0} cards dropped this run`}
              />
              <div className="mt-2 text-[13px] text-muted">dropped this run</div>
            </div>
          </div>

          <div className="flex flex-col gap-3 lg:border-l lg:border-line lg:pl-6">
            <ol className="flex flex-wrap items-center gap-2" aria-label="Farming phases">
              <PhaseStep n={1} title="Build playtime" detail={`Games under ${fs.thresholdHours} h, together`} active={phase === "phase1"} />
              <span className="h-px w-6 bg-line-strong" aria-hidden="true" />
              <PhaseStep
                n={2}
                title="Farm drops"
                detail={`${fs.soloGames} at a time, checked every ${fs.checkMinutes} min`}
                active={phase === "phase2" || phase === "restarting"}
              />
            </ol>
            <div className="text-[13px]">
              {running ? (
                <>
                  <span className="text-ink">{status!.message}</span>
                  {status!.nextCheckAt > now && (
                    <span className="text-muted">. Next drop check in </span>
                  )}
                  {status!.nextCheckAt > now && <span className="num text-ink">{until(status!.nextCheckAt, now)}</span>}
                </>
              ) : phase === "done" || phase === "stopped" || phase === "error" ? (
                <span className={phase === "error" ? "text-bad" : "text-muted"}>{status!.message}</span>
              ) : (
                <span className="text-muted">
                  Not running. Check badges to see what's left, or start farming right away.
                </span>
              )}
            </div>
            <label className="flex items-center gap-2.5 text-[13px] text-muted">
              <Switch
                label="Skip games that may still be refundable"
                checked={fs.skipRefundable}
                onChange={(v) => void useApp.getState().saveSettings({ farming: { ...fs, skipRefundable: v } })}
              />
              Skip games bought in the last 14 days with under 2 h played, so idling doesn't cost you a refund. Matched by name, so bundle purchases can be missed.
            </label>
          </div>
        </div>
      </Section>

      <HiddenNotice hiding={hiding} />

      {games.length === 0 ? (
        !noWeb && (
          <Empty
            title={status?.phase === "done" ? "Nothing left to farm" : "No badge data yet"}
            body={
              status?.phase === "done"
                ? "Every game you own has dropped all of its cards."
                : "Check badges to list your games that still have card drops."
            }
          />
        )
      ) : rows.length === 0 ? (
        <p className="text-muted">
          <span className="num text-ink">{unseen}</span> {unseen === 1 ? "game" : "games"} with card drops, not shown here.
        </p>
      ) : (
        <>
        <div className="overflow-x-auto rounded-xl border border-line bg-panel">
          <table className="w-full text-left text-[13px]">
            <thead className="border-b border-line text-[12px] text-faint [&_th]:whitespace-nowrap">
              <tr>
                <th className="px-4 py-2.5 font-normal">Game</th>
                <th className="px-3 py-2.5 text-right font-normal">Drops left</th>
                <th className="px-3 py-2.5 text-right font-normal">On record</th>
                <th className="px-3 py-2.5 font-normal">Status</th>
                <th className="px-4 py-2.5 text-right font-normal">Farm this game</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((g) => {
                const blacklisted = account.farmBlacklist.includes(g.appid);
                return (
                  <tr key={g.appid} className="border-t border-line first:border-t-0">
                    <td className="px-4 py-2.5">
                      <div className="flex items-center gap-3">
                        <GameIcon appid={g.appid} name={g.name} size={28} />
                        <div className="min-w-0">
                          <div className="truncate">{g.name}</div>
                          {g.note && <div className="text-[12px] text-faint">{g.note}</div>}
                        </div>
                      </div>
                    </td>
                    <td className="num px-3 py-2.5 text-right text-[15px]">{g.dropsRemaining}</td>
                    <td className="num px-3 py-2.5 text-right text-muted">{hoursFromFloat(g.playtimeHours)}</td>
                    <td className="px-3 py-2.5">
                      <Pill tone={STATUS[g.status].tone} lamp={g.status === "idling"}>
                        {STATUS[g.status].label}
                      </Pill>
                    </td>
                    <td className="px-4 py-2.5 text-right">
                      <Switch
                        label={blacklisted ? `Allow farming ${g.name}` : `Never farm ${g.name}`}
                        checked={!blacklisted}
                        onChange={(v) => toggleBlacklist(g.appid, !v)}
                      />
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
        {unseen > 0 && <p className="mt-2 text-[13px] text-muted">And {unseen} more not shown.</p>}
        </>
      )}
    </div>
  );
}

function PhaseStep({ n, title, detail, active }: { n: number; title: string; detail: string; active: boolean }) {
  return (
    <li
      className={cx(
        "flex items-center gap-2.5 rounded-lg border px-3 py-2",
        active ? "border-run bg-run-soft" : "border-line",
      )}
      aria-current={active ? "step" : undefined}
    >
      <span
        className={cx(
          "num flex size-6 items-center justify-center rounded-full text-[13px] font-semibold",
          active ? "bg-run text-run-ink" : "bg-raised text-muted",
        )}
      >
        {n}
      </span>
      <span className="flex flex-col leading-tight">
        <span className={cx("text-[13px]", active ? "text-ink" : "text-muted")}>{title}</span>
        <span className="text-[12px] text-faint">{detail}</span>
      </span>
    </li>
  );
}
