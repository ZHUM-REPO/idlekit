import { memo, useEffect, useMemo, useRef, useState } from "react";
import { Button, PageHeader, Select, TextInput, cx } from "../components/ui";
import { api } from "../lib/api";
import { timeOfDay } from "../lib/format";
import { useAppIds, useHiding } from "../lib/hidden";
import { useActiveAccount, useApp, useEnsureGames } from "../lib/store";
import type { LogLine } from "../lib/types";

/**
 * One line. Memoized and keyed by the line's own number, so a new line draws
 * one row and leaves the others alone. A row is always laid out at its real
 * height: the list scrolls to its end, and an estimated height for a line that
 * wraps would leave part of the newest line below the edge.
 */
const Row = memo(function Row({ line: l }: { line: LogLine }) {
  return (
    <div className="flex gap-3">
      <span className="flex-none text-faint">{timeOfDay(l.ts)}</span>
      <span
        className={cx("w-16 flex-none truncate", l.level === "error" ? "text-bad" : l.level === "warn" ? "text-run" : "text-faint")}
      >
        {l.scope}
      </span>
      <span className={cx("min-w-0 break-words", l.level === "error" ? "text-bad" : l.level === "debug" ? "text-faint" : "text-ink")}>
        {l.message}
      </span>
    </div>
  );
});

export function LogsPage() {
  const logs = useApp((s) => s.logs);
  const logFile = useApp((s) => s.logFile);
  const run = useApp((s) => s.run);
  const [level, setLevel] = useState<"all" | "problems" | "debug">("all");
  const [query, setQuery] = useState("");
  const end = useRef<HTMLDivElement>(null);

  // "Hide NSFW games": lines that name a hidden game are left out of this
  // list. The file keeps them, and the note under the list says so.
  const account = useActiveAccount();
  useEnsureGames(account?.steamId);
  const games = useApp((s) => (account ? s.games[account.steamId] : undefined));
  const hiding = useHiding(useAppIds(games));
  const hiddenNames = useMemo(
    () =>
      hiding.on && games
        ? games.filter((g) => hiding.hides(g.appid)).map((g) => g.name.trim().toLowerCase()).filter((name) => name.length >= 3)
        : [],
    [games, hiding],
  );
  const visible = useMemo(() => {
    if (hiddenNames.length === 0) return logs;
    return logs.filter((l) => {
      const text = l.message.toLowerCase();
      return !hiddenNames.some((name) => text.includes(name));
    });
  }, [logs, hiddenNames]);
  const leftOut = logs.length - visible.length;

  const shown = useMemo(() => {
    const q = query.trim().toLowerCase();
    return visible.filter((l) => {
      if (level === "problems" && l.level !== "warn" && l.level !== "error") return false;
      if (level === "all" && l.level === "debug") return false;
      return !q || l.message.toLowerCase().includes(q) || l.scope.includes(q);
    });
  }, [visible, level, query]);

  // Follows the newest line. The count alone can't tell: once the list is
  // full, a new line pushes the oldest out and the count stays the same.
  const newest = shown[shown.length - 1]?.n;
  useEffect(() => {
    end.current?.scrollIntoView({ block: "end" });
  }, [shown.length, newest]);

  return (
    <div className="flex h-full flex-col pb-4">
      <PageHeader
        title="Logs"
        sub="Every action and its result, and anything that went wrong. Sign-in names, Steam IDs, tokens and cookies are left out, so this is safe to share when reporting a problem."
        actions={
          <>
            <TextInput value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Filter" aria-label="Filter logs" className="w-56" />
            <Select
              label="Level"
              value={level}
              onChange={setLevel}
              options={[
                { value: "all", label: "Everything except debug" },
                { value: "problems", label: "Warnings and errors" },
                { value: "debug", label: "Everything, with debug" },
              ]}
            />
            <Button
              onClick={() => void navigator.clipboard.writeText(shown.map((l) => `${new Date(l.ts).toISOString()} ${l.level} ${l.scope}: ${l.message}`).join("\n"))}
            >
              Copy
            </Button>
            {logFile && <Button onClick={() => void run(null, () => api.openLogFile())}>Show log.txt</Button>}
            <Button
              variant="ghost"
              onClick={async () => {
                await api.clearLogs();
                useApp.setState({ logs: [] });
              }}
            >
              Clear
            </Button>
          </>
        }
      />
      <div className="min-h-0 flex-1 overflow-y-auto rounded-xl border border-line bg-well px-4 py-3 font-mono text-[12px] leading-[1.7]">
        {shown.length === 0 && <div className="text-faint">Nothing logged yet.</div>}
        {shown.map((l, i) => (
          <Row key={l.n ?? `i${i}`} line={l} />
        ))}
        <div ref={end} />
      </div>
      <p className="mt-2 text-[12px] text-faint">
        {logFile
          ? `This list is this session. The same lines, from earlier sessions too, are kept in ${logFile}.`
          : "This list is this session only: the log file couldn't be opened."}
        {leftOut > 0 && ` ${leftOut} ${leftOut === 1 ? "line that names a hidden game is" : "lines that name hidden games are"} left out here.`}
      </p>
    </div>
  );
}
