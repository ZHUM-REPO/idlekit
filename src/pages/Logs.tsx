import { useEffect, useMemo, useRef, useState } from "react";
import { Button, PageHeader, Select, TextInput, cx } from "../components/ui";
import { api } from "../lib/api";
import { timeOfDay } from "../lib/format";
import { useApp } from "../lib/store";

export function LogsPage() {
  const logs = useApp((s) => s.logs);
  const logFile = useApp((s) => s.logFile);
  const run = useApp((s) => s.run);
  const [level, setLevel] = useState<"all" | "problems" | "debug">("all");
  const [query, setQuery] = useState("");
  const end = useRef<HTMLDivElement>(null);

  const shown = useMemo(() => {
    const q = query.trim().toLowerCase();
    return logs.filter((l) => {
      if (level === "problems" && l.level !== "warn" && l.level !== "error") return false;
      if (level === "all" && l.level === "debug") return false;
      return !q || l.message.toLowerCase().includes(q) || l.scope.includes(q);
    });
  }, [logs, level, query]);

  useEffect(() => {
    end.current?.scrollIntoView({ block: "end" });
  }, [shown.length]);

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
          <div key={i} className="flex gap-3">
            <span className="flex-none text-faint">{timeOfDay(l.ts)}</span>
            <span
              className={cx(
                "w-16 flex-none truncate",
                l.level === "error" ? "text-bad" : l.level === "warn" ? "text-run" : "text-faint",
              )}
            >
              {l.scope}
            </span>
            <span className={cx("min-w-0 break-words", l.level === "error" ? "text-bad" : l.level === "debug" ? "text-faint" : "text-ink")}>
              {l.message}
            </span>
          </div>
        ))}
        <div ref={end} />
      </div>
      <p className="mt-2 text-[12px] text-faint">
        {logFile
          ? `This list is this session. The same lines, from earlier sessions too, are kept in ${logFile}.`
          : "This list is this session only: the log file couldn't be opened."}
      </p>
    </div>
  );
}
