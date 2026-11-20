import { useEffect, useState, type ReactNode } from "react";
import { updateProgressText } from "../components/Desktop";
import { Button, Field, NumberInput, PageHeader, Section, Select, Switch, TextInput, cx, inputClass } from "../components/ui";
import { openExternal } from "../lib/api";
import { useApp } from "../lib/store";
import type { FarmingSettings, MarketSettings, Settings, UnlockerSettings } from "../lib/types";

const THEMES: { id: Settings["theme"]; label: string; bg: string; panel: string; ink: string }[] = [
  { id: "graphite", label: "Graphite", bg: "#15171b", panel: "#22262c", ink: "#ebe8e2" },
  { id: "midnight", label: "Midnight", bg: "#0e1320", panel: "#1a2234", ink: "#e6eaf2" },
  { id: "daylight", label: "Daylight", bg: "#eceeea", panel: "#ffffff", ink: "#1c1e22" },
];

const ACCENTS: { id: Settings["accent"]; label: string; color: string }[] = [
  { id: "amber", label: "Amber", color: "#f0b43c" },
  { id: "cyan", label: "Cyan", color: "#3cc8e0" },
  { id: "lime", label: "Lime", color: "#a6d83a" },
  { id: "rose", label: "Rose", color: "#f07a95" },
  { id: "violet", label: "Violet", color: "#ab92ff" },
];

function Group({ title, children, note }: { title: string; children: ReactNode; note?: string }) {
  return (
    <Section title={title}>
      <div className="divide-y divide-line px-4">{children}</div>
      {note && <div className="border-t border-line px-4 py-2.5 text-[12px] text-faint">{note}</div>}
    </Section>
  );
}

export function SettingsPage() {
  const s = useApp((st) => st.settings);
  const save = useApp((st) => st.saveSettings);
  const dataDir = useApp((st) => st.dataDir);
  const helperFound = useApp((st) => st.helperFound);
  const desktop = useApp((st) => st.desktop);
  const setAutostart = useApp((st) => st.setAutostart);
  const canUpdate = useApp((st) => !!st.update?.configured);
  if (!s) return null;

  const hasTray = !!desktop?.tray;
  const startLabel =
    desktop?.platform === "windows" ? "Start IdleKit when I sign in to Windows" : "Start IdleKit when I sign in";

  const farm = (patch: Partial<FarmingSettings>) => void save({ farming: { ...s.farming, ...patch } });
  const unl = (patch: Partial<UnlockerSettings>) => void save({ unlocker: { ...s.unlocker, ...patch } });
  const mkt = (patch: Partial<MarketSettings>) => void save({ market: { ...s.market, ...patch } });

  return (
    <div className="max-w-[880px] pb-12">
      <PageHeader title="Settings" sub="Changes save as you make them." />
      <div className="flex flex-col gap-5">
        <Group title="Appearance">
          <Field inline label="Theme">
            <div className="flex gap-2">
              {THEMES.map((t) => (
                <button
                  key={t.id}
                  onClick={() => void save({ theme: t.id })}
                  aria-pressed={s.theme === t.id}
                  className={cx(
                    "flex items-center gap-2 rounded-lg border px-2.5 py-1.5 text-[13px]",
                    s.theme === t.id ? "border-ink" : "border-line hover:border-line-strong",
                  )}
                >
                  <span className="flex h-5 w-8 overflow-hidden rounded border border-black/20" aria-hidden="true">
                    <span className="w-1/2" style={{ background: t.bg }} />
                    <span className="flex w-1/2 items-center justify-center" style={{ background: t.panel }}>
                      <span className="h-0.5 w-2 rounded" style={{ background: t.ink }} />
                    </span>
                  </span>
                  {t.label}
                </button>
              ))}
            </div>
          </Field>
          <Field inline label="Game cards" hint="How games look in the library. Covers are the tall artwork from the Steam library; banners are the wide store image.">
            <div className="flex gap-2">
              {(
                [
                  ["cover", "Cover", "h-5 w-3.5"],
                  ["banner", "Banner", "h-3.5 w-7"],
                ] as const
              ).map(([id, label, glyph]) => (
                <button
                  key={id}
                  onClick={() => void save({ cardStyle: id })}
                  aria-pressed={s.cardStyle === id}
                  className={cx(
                    "flex items-center gap-2 rounded-lg border px-2.5 py-1.5 text-[13px]",
                    s.cardStyle === id ? "border-ink" : "border-line hover:border-line-strong",
                  )}
                >
                  <span className="flex h-5 w-7 items-center justify-center" aria-hidden="true">
                    <span className={cx("rounded-[3px] bg-faint", glyph)} />
                  </span>
                  {label}
                </button>
              ))}
            </div>
          </Field>
          <Field inline label="Running color" hint="Marks anything that is currently running.">
            <div className="flex gap-2">
              {ACCENTS.map((a) => (
                <button
                  key={a.id}
                  onClick={() => void save({ accent: a.id })}
                  aria-label={a.label}
                  aria-pressed={s.accent === a.id}
                  title={a.label}
                  className={cx(
                    "flex size-7 items-center justify-center rounded-full border-2",
                    s.accent === a.id ? "border-ink" : "border-transparent",
                  )}
                >
                  <span className="size-5 rounded-full" style={{ background: a.color }} />
                </button>
              ))}
            </div>
          </Field>
        </Group>

        <Group title="Running in the background">
          <Field
            inline
            label="When I close the window"
            hint={
              hasTray
                ? "In the tray, idling, card farming and the unlocker keep going. Click the tray icon to bring the window back."
                : "No tray icon is available on this system, so closing the window quits IdleKit."
            }
          >
            <Select
              label="When I close the window"
              value={hasTray ? s.closeAction : "quit"}
              disabled={!hasTray}
              onChange={(v) => void save({ closeAction: v })}
              options={[
                { value: "ask", label: "Ask me" },
                { value: "tray", label: "Keep running in the tray" },
                { value: "quit", label: "Quit IdleKit" },
              ]}
            />
          </Field>
          <Field inline label={startLabel} hint="Combine this with the options under “When IdleKit opens” to have everything start by itself.">
            <Switch label={startLabel} checked={!!desktop?.autostart} onChange={(v) => void setAutostart(v)} />
          </Field>
          <Field inline label="Start in the tray" hint="When IdleKit starts at sign-in, don't open its window.">
            <Switch
              label="Start in the tray"
              checked={s.startInTray}
              disabled={!desktop?.autostart || !hasTray}
              onChange={(v) => void save({ startInTray: v })}
            />
          </Field>
        </Group>

        <Group title="When IdleKit opens">
          <Field inline label="Connect accounts that use a direct connection">
            <Switch label="Connect direct accounts on launch" checked={s.connectDirectOnLaunch} onChange={(v) => void save({ connectDirectOnLaunch: v })} />
          </Field>
          <Field inline label="Start each account's auto-idle list">
            <Switch label="Auto-idle on launch" checked={s.autoIdleOnLaunch} onChange={(v) => void save({ autoIdleOnLaunch: v })} />
          </Field>
          <Field inline label="Start card farming for the selected account">
            <Switch label="Farm on launch" checked={s.farmOnLaunch} onChange={(v) => void save({ farmOnLaunch: v })} />
          </Field>
          <Field inline label="When card farming finishes">
            <Select
              value={s.afterFarming}
              onChange={(v) => void save({ afterFarming: v })}
              options={[
                { value: "none", label: "Do nothing" },
                { value: "autoIdle", label: "Start the auto-idle list" },
                { value: "unlocker", label: "Start the achievement unlocker" },
              ]}
            />
          </Field>
        </Group>

        <Group title="Idling">
          <Field inline label="Most games at once" hint="Steam counts at most 32.">
            <NumberInput value={s.maxIdleGames} min={1} max={32} onChange={(v) => void save({ maxIdleGames: Math.max(1, Math.min(32, v)) })} />
          </Field>
          <Field
            inline
            label="Status text while idling"
            hint="Direct connections only. Friends see it as a non-Steam game while you idle. Leave empty to show the idled games."
          >
            <StatusTextInput value={s.idleStatusText} onSave={(v) => void save({ idleStatusText: v })} />
          </Field>
        </Group>

        <Group
          title="Card farming"
          note="Phase 1 idles games under the playtime threshold together, since Steam usually holds back drops until then. Phase 2 farms the rest a few at a time."
        >
          <Field inline label="Phase 1 threshold" hint="Set to 0 to skip phase 1.">
            <NumberInput value={s.farming.thresholdHours} min={0} max={20} step={0.5} suffix="hours" onChange={(v) => farm({ thresholdHours: Math.max(0, v) })} />
          </Field>
          <Field inline label="Games at once in phase 1">
            <NumberInput value={s.farming.maxGames} min={1} max={32} onChange={(v) => farm({ maxGames: Math.max(1, Math.min(32, v)) })} />
          </Field>
          <Field inline label="Games at once in phase 2" hint="One at a time drops cards fastest.">
            <NumberInput value={s.farming.soloGames} min={1} max={32} onChange={(v) => farm({ soloGames: Math.max(1, Math.min(32, v)) })} />
          </Field>
          <Field inline label="Check for drops every">
            <NumberInput value={s.farming.checkMinutes} min={1} max={240} suffix="minutes" onChange={(v) => farm({ checkMinutes: Math.max(1, v) })} />
          </Field>
          <Field inline label="Restart games at each check" hint="Briefly stopping a game often makes Steam hand out a pending drop.">
            <Switch label="Restart games at each check" checked={s.farming.restartOnCheck} onChange={(v) => farm({ restartOnCheck: v })} />
          </Field>
          <Field inline label="Skip games that may still be refundable" hint="Bought in the last 14 days with under 2 hours played.">
            <Switch label="Skip refundable games" checked={s.farming.skipRefundable} onChange={(v) => farm({ skipRefundable: v })} />
          </Field>
          <Field inline label="Farm first">
            <Select
              value={s.farming.order}
              onChange={(v) => farm({ order: v })}
              options={[
                { value: "mostDrops", label: "Games with the most drops" },
                { value: "leastDrops", label: "Games with the fewest drops" },
                { value: "playtime", label: "Most played games" },
                { value: "name", label: "Alphabetical" },
              ]}
            />
          </Field>
        </Group>

        <Group title="Achievement unlocker">
          <Field inline label="Wait between unlocks" hint="A random time in this range, unless imported timing is used.">
            <span className="flex items-center gap-2">
              <NumberInput value={s.unlocker.minDelayMinutes} min={1} max={1440} onChange={(v) => unl({ minDelayMinutes: Math.max(1, v) })} />
              <span className="text-faint">to</span>
              <NumberInput
                value={s.unlocker.maxDelayMinutes}
                min={1}
                max={1440}
                suffix="minutes"
                onChange={(v) => unl({ maxDelayMinutes: Math.max(1, v) })}
              />
            </span>
          </Field>
          <Field inline label="Unlock order">
            <Select
              value={s.unlocker.order}
              onChange={(v) => unl({ order: v })}
              options={[
                { value: "common", label: "Most common first" },
                { value: "rare", label: "Rarest first" },
                { value: "schema", label: "The game's own order" },
              ]}
            />
          </Field>
          <Field inline label="Only unlock during set hours">
            <span className="flex items-center gap-2">
              <input
                type="time"
                aria-label="Window start"
                value={s.unlocker.scheduleStart}
                disabled={!s.unlocker.scheduleEnabled}
                onChange={(e) => unl({ scheduleStart: e.target.value })}
                className={cx(inputClass, "num w-28 disabled:opacity-40")}
              />
              <span className="text-faint">to</span>
              <input
                type="time"
                aria-label="Window end"
                value={s.unlocker.scheduleEnd}
                disabled={!s.unlocker.scheduleEnabled}
                onChange={(e) => unl({ scheduleEnd: e.target.value })}
                className={cx(inputClass, "num w-28 disabled:opacity-40")}
              />
              <Switch label="Only unlock during set hours" checked={s.unlocker.scheduleEnabled} onChange={(v) => unl({ scheduleEnabled: v })} />
            </span>
          </Field>
          <Field inline label="Idle the game while unlocking" hint="Adds playtime, like really playing. Off: the game only runs for a moment at each unlock.">
            <Switch label="Idle while unlocking" checked={s.unlocker.idleWhileUnlocking} onChange={(v) => unl({ idleWhileUnlocking: v })} />
          </Field>
          <Field inline label="Games unlocking at once">
            <NumberInput value={s.unlocker.parallelGames} min={1} max={32} onChange={(v) => unl({ parallelGames: Math.max(1, Math.min(32, v)) })} />
          </Field>
          <Field inline label="Imported timing speed" hint="1 copies the other player's pace; 0.5 goes twice as fast.">
            <NumberInput
              value={s.unlocker.importedTimeScale}
              min={0.05}
              max={20}
              step={0.05}
              suffix="×"
              onChange={(v) => unl({ importedTimeScale: Math.max(0.05, v) })}
            />
          </Field>
        </Group>

        <Group title="Market">
          <Field inline label="Pricing">
            <Select
              value={s.market.strategy}
              onChange={(v) => mkt({ strategy: v })}
              options={[
                { value: "undercutLowest", label: "Just under the lowest listing" },
                { value: "matchLowest", label: "Match the lowest listing" },
                { value: "aboveHighestBuy", label: "Just above the highest buy offer" },
              ]}
            />
          </Field>
          <Field inline label="Step" hint="How far under (or over) the reference price to go.">
            <NumberInput value={s.market.undercutCents} min={0} max={1000} suffix="cents" onChange={(v) => mkt({ undercutCents: Math.max(0, v) })} />
          </Field>
          <Field inline label="Never list below" hint="Buyer price, in cents. 0 means no limit.">
            <NumberInput value={s.market.minPriceCents} min={0} max={100000} suffix="cents" onChange={(v) => mkt({ minPriceCents: Math.max(0, v) })} />
          </Field>
          <Field inline label="Never list above" hint="Buyer price, in cents. 0 means no limit.">
            <NumberInput value={s.market.maxPriceCents} min={0} max={1000000} suffix="cents" onChange={(v) => mkt({ maxPriceCents: Math.max(0, v) })} />
          </Field>
          <Field inline label="Pause between market requests" hint="Lower is faster but Steam starts refusing requests below about 1 second.">
            <NumberInput value={s.market.delayMs} min={300} max={30000} step={100} suffix="ms" onChange={(v) => mkt({ delayMs: Math.max(300, v) })} />
          </Field>
        </Group>

        <Group title="Updates">
          <UpdateField />
          <Field inline label="Look for updates when IdleKit opens">
            <Switch
              label="Look for updates when IdleKit opens"
              checked={s.checkUpdates}
              disabled={!canUpdate}
              onChange={(v) => void save({ checkUpdates: v })}
            />
          </Field>
        </Group>

        <Group title="About">
          <Field inline label="Data folder" hint="Settings and cached game lists. Sign-in tokens are kept in your OS credential store.">
            <span className="block max-w-[360px] truncate text-[12px] text-muted" title={dataDir}>
              {dataDir}
            </span>
          </Field>
          <Field inline label="Steam client helper">
            <span className={cx("text-[13px]", helperFound ? "text-ok" : "text-bad")}>
              {helperFound ? "Installed" : "Missing; reinstall IdleKit"}
            </span>
          </Field>
        </Group>
      </div>
    </div>
  );
}

/** The installed version, what is known about newer ones, and the button to act on it. */
function UpdateField() {
  const version = useApp((st) => st.version);
  const u = useApp((st) => st.update);
  const check = useApp((st) => st.checkForUpdates);
  const install = useApp((st) => st.installUpdate);

  if (!u?.configured) {
    return (
      <Field
        inline
        label="Version"
        hint="This copy can't update itself. Copies installed from the project's GitHub releases can."
      >
        <span className="num text-muted">{version}</span>
      </Field>
    );
  }

  const progress = updateProgressText(u);
  const status = progress ? (
    progress
  ) : u.phase === "checking" ? (
    "Looking for a newer version"
  ) : u.error ? (
    <span className="text-bad">{u.error}</span>
  ) : u.available ? (
    <>
      Version {u.available.version} is available.{" "}
      {u.releasesUrl && (
        <button className="underline hover:text-ink" onClick={() => void openExternal(u.releasesUrl!)}>
          What's new
        </button>
      )}
    </>
  ) : u.checkedAt ? (
    `Up to date. Last checked at ${new Date(u.checkedAt).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}.`
  ) : (
    "Not checked yet."
  );

  return (
    <Field inline label="Version" hint={status}>
      <span className="flex items-center gap-3">
        <span className="num text-muted">{version}</span>
        {u.available ? (
          <Button variant="primary" disabled={u.phase !== "idle"} onClick={() => void install()}>
            {u.error ? "Try again" : "Install and restart"}
          </Button>
        ) : (
          <Button busy={u.phase === "checking"} disabled={u.phase !== "idle"} onClick={() => void check()}>
            Check for updates
          </Button>
        )}
      </span>
    </Field>
  );
}

/** Saved when the field loses focus (or on Enter), so Steam isn't sent a status per keystroke. */
function StatusTextInput({ value, onSave }: { value: string; onSave: (v: string) => void }) {
  const [draft, setDraft] = useState(value);
  useEffect(() => setDraft(value), [value]);
  const commit = () => draft !== value && onSave(draft.trim());
  return (
    <TextInput
      value={draft}
      onChange={(e) => setDraft(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => e.key === "Enter" && commit()}
      placeholder="e.g. Away from keyboard"
      maxLength={64}
      className="w-64"
    />
  );
}
