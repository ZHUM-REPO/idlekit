// The parts of the interface that come from IdleKit being a desktop program:
// the bar that announces a newer version, and the question asked the first
// time the window is closed.
import { X } from "lucide-react";
import { useState } from "react";
import { openExternal } from "../lib/api";
import { useApp } from "../lib/store";
import type { UpdateState } from "../lib/types";
import { Button, IconButton, Modal } from "./ui";

/** "45%" while a download is under way and its size is known. */
export function downloadPercent(u: UpdateState): string | null {
  if (!u.total) return null;
  return `${Math.min(100, Math.round((u.downloaded / u.total) * 100))}%`;
}

/** What an update in progress is doing, or null when nothing is. */
export function updateProgressText(u: UpdateState): string | null {
  const version = u.available?.version ?? "";
  if (u.phase === "downloading") {
    const percent = downloadPercent(u);
    return `Downloading IdleKit ${version}${percent ? `, ${percent}` : ""}`;
  }
  if (u.phase === "installing") return `Installing IdleKit ${version}. It restarts by itself when done.`;
  return null;
}

/** One line under the top bar when a newer version exists or is being installed. */
export function UpdateBar() {
  const u = useApp((s) => s.update);
  const dismissed = useApp((s) => s.updateDismissed);
  const install = useApp((s) => s.installUpdate);
  const dismiss = useApp((s) => s.dismissUpdate);
  if (!u?.available) return null;

  const progress = updateProgressText(u);
  if (!progress && dismissed === u.available.version) return null;

  return (
    <div
      role="status"
      className="flex min-h-9 items-center gap-3 border-b border-run/40 bg-run-soft px-5 py-1 text-[13px]"
    >
      <span className="lamp on flex-none" />
      {progress ? (
        <span className="min-w-0 flex-1">{progress}</span>
      ) : (
        <>
          <span className="min-w-0 flex-1">
            {u.error ?? `IdleKit ${u.available.version} is available. You have ${u.current}.`}
          </span>
          <Button size="sm" variant="primary" onClick={() => void install()}>
            {u.error ? "Try again" : "Install and restart"}
          </Button>
          {u.releasesUrl && (
            <Button size="sm" variant="ghost" onClick={() => void openExternal(u.releasesUrl!)}>
              What's new
            </Button>
          )}
          <IconButton compact label="Not now" onClick={dismiss}>
            <X size={14} />
          </IconButton>
        </>
      )}
    </div>
  );
}

/** Asked the first time the window is closed while a tray icon exists. */
export function CloseDialog() {
  const open = useApp((s) => s.closePrompt);
  const answer = useApp((s) => s.answerClose);
  const [remember, setRemember] = useState(true);
  if (!open) return null;

  return (
    <Modal title="Keep IdleKit running?" width={500} onClose={() => void answer("cancel", false)}>
      <p className="text-[13px] text-muted">
        Idling, card farming and the achievement unlocker stop when IdleKit quits. It can stay in the system tray
        instead. Click its icon there to bring this window back.
      </p>
      <label className="mt-4 flex w-fit cursor-pointer items-center gap-2 text-[13px]">
        <input
          type="checkbox"
          className="size-4 accent-[var(--ink)]"
          checked={remember}
          onChange={(e) => setRemember(e.target.checked)}
        />
        Do the same next time
      </label>
      <div className="mt-5 flex flex-wrap justify-end gap-2">
        <Button onClick={() => void answer("quit", remember)}>Quit IdleKit</Button>
        <Button variant="primary" onClick={() => void answer("tray", remember)}>
          Keep running in the tray
        </Button>
      </div>
    </Modal>
  );
}
