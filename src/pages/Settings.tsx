import { Pipette } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import { updateProgressText } from "../components/Desktop";
import { Button, Field, NumberInput, PageHeader, Section, Select, Switch, TextInput, cx, inputClass } from "../components/ui";
import { openExternal } from "../lib/api";
import { hexColor, isLightTheme, useApp } from "../lib/store";
import type { FarmingSettings, MarketSettings, Settings, UnlockerSettings } from "../lib/types";

type Theme = { id: Settings["theme"]; label: string; bg: string; panel: string; ink: string };

const DARK_THEMES: Theme[] = [
  { id: "graphite", label: "Graphite", bg: "#15171b", panel: "#22262c", ink: "#ebe8e2" },
  { id: "midnight", label: "Midnight", bg: "#0e1320", panel: "#1a2234", ink: "#e6eaf2" },
  { id: "obsidian", label: "Obsidian", bg: "#000000", panel: "#141416", ink: "#ececec" },
  { id: "harbor", label: "Harbor", bg: "#151b23", panel: "#212c3a", ink: "#dfe6ee" },
  { id: "moss", label: "Moss", bg: "#121713", panel: "#1e2720", ink: "#e4eae3" },
  { id: "plum", label: "Plum", bg: "#161220", panel: "#241e34", ink: "#ece6f4" },
];

const LIGHT_THEMES: Theme[] = [
  { id: "daylight", label: "Daylight", bg: "#eceeea", panel: "#ffffff", ink: "#1c1e22" },
  { id: "paper", label: "Paper", bg: "#f1ece1", panel: "#fffdf8", ink: "#2a241c" },
  { id: "frost", label: "Frost", bg: "#e6edf4", panel: "#ffffff", ink: "#17212c" },
  { id: "contrast", label: "High contrast", bg: "#ffffff", panel: "#e8e8e8", ink: "#000000" },
];

/** The preset running colors, as styles.css has them: one shade for dark
 *  themes and a deeper one that reads on light themes. */
const ACCENTS: { id: Settings["accent"]; label: string; dark: string; light: string }[] = [
  { id: "amber", label: "Amber", dark: "#f0b43c", light: "#a86b00" },
  { id: "orange", label: "Orange", dark: "#ff9150", light: "#b3500d" },
  { id: "rose", label: "Rose", dark: "#f07a95", light: "#b02d52" },
  { id: "orchid", label: "Orchid", dark: "#e488e9", light: "#962d9c" },
  { id: "violet", label: "Violet", dark: "#ab92ff", light: "#6544d4" },
  { id: "blue", label: "Blue", dark: "#6ea8ff", light: "#2658c2" },
  { id: "cyan", label: "Cyan", dark: "#3cc8e0", light: "#0a7a8e" },
  { id: "mint", label: "Mint", dark: "#48d7ab", light: "#0b7a5c" },
  { id: "lime", label: "Lime", dark: "#a6d83a", light: "#4d7a07" },
  { id: "silver", label: "Silver", dark: "#d4d8df", light: "#3d434c" },
];

function ThemeButton({ theme, on, pick }: { theme: Theme; on: boolean; pick: () => void }) {
  return (
    <button
      onClick={pick}
      aria-pressed={on}
      className={cx(
        "flex items-center gap-2 rounded-lg border px-2.5 py-1.5 text-[13px]",
        on ? "border-ink" : "border-line hover:border-line-strong",
      )}
    >
      <span className="flex h-5 w-8 overflow-hidden rounded border border-black/20" aria-hidden="true">
        <span className="w-1/2" style={{ background: theme.bg }} />
        <span className="flex w-1/2 items-center justify-center" style={{ background: theme.panel }}>
          <span className="h-0.5 w-2 rounded" style={{ background: theme.ink }} />
        </span>
      </span>
      {theme.label}
    </button>
  );
}

/**
 * Any running color, from the color picker. The window takes each color as it
 * is picked; it is written to the settings once the picking pauses, not at
 * every step of a drag.
 */
function AnyColor({ settings }: { settings: Settings }) {
  const saveSoon = useApp((st) => st.saveSettingsSoon);
  const on = settings.accent === "custom";
  const color = hexColor(settings.accentColor) ?? "#f0b43c";
  return (
    <label
      title="Any color"
      className={cx(
        "relative flex size-7 cursor-pointer items-center justify-center rounded-full border-2 has-[:focus-visible]:outline-2 has-[:focus-visible]:outline-offset-2 has-[:focus-visible]:outline-run",
        on ? "border-ink" : "border-transparent",
      )}
    >
      <span
        className="flex size-5 items-center justify-center rounded-full"
        style={{
          background: on ? color : "conic-gradient(#f07a95, #f0b43c, #a6d83a, #48d7ab, #3cc8e0, #6ea8ff, #ab92ff, #e488e9, #f07a95)",
        }}
      >
        {!on && <Pipette size={11} strokeWidth={2.5} className="text-black/70" aria-hidden="true" />}
      </span>
      <input
        type="color"
        aria-label="Any color"
        value={color}
        className="absolute inset-0 size-full cursor-pointer opacity-0"
        onChange={(e) => saveSoon({ accent: "custom", accentColor: e.target.value })}
      />
    </label>
  );
}

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
  const light = isLightTheme(s.theme);
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
          <div className="flex items-start justify-between gap-6 py-2.5">
            <div className="pt-1.5 text-[13px] text-ink">Theme</div>
            <div className="flex flex-col items-end gap-2">
              {[DARK_THEMES, LIGHT_THEMES].map((themes, row) => (
                <div key={row} className="flex flex-wrap justify-end gap-2">
                  {themes.map((t) => (
                    <ThemeButton key={t.id} theme={t} on={s.theme === t.id} pick={() => void save({ theme: t.id })} />
                  ))}
                </div>
              ))}
            </div>
          </div>
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
          <Field
            inline
            label="Hide NSFW games"
            hint="Leaves games that Steam labels “Adult Only Sexual Content” or “Frequent Nudity or Sexual Content” out of every list here. They still idle and farm if they are set to, and log.txt still names them."
          >
            <Switch label="Hide NSFW games" checked={s.hideNsfw} onChange={(v) => void save({ hideNsfw: v })} />
          </Field>
          <Field inline label="Running color" hint="Marks anything that is currently running. The last one takes any color you pick.">
            <div className="flex flex-wrap justify-end gap-1.5">
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
                  <span className="size-5 rounded-full" style={{ background: light ? a.light : a.dark }} />
                </button>
              ))}
              <AnyColor settings={s} />
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
          <Field
            inline
            label="Look for updates by itself"
            hint="When IdleKit opens, then every half hour for as long as it runs, the tray included."
          >
            <Switch
              label="Look for updates by itself"
              checked={s.checkUpdates}
              disabled={!canUpdate}
              onChange={(v) => void save({ checkUpdates: v })}
            />
          </Field>
          <Field
            inline
            label="Install updates by itself"
            hint="A newer version is installed without asking, and IdleKit restarts, when nothing is running: no idling, card farming, unlocking, market work or open achievement list. That includes the moment IdleKit starts, before it starts anything. While something is running it waits, and the update is offered as before."
          >
            <Switch
              label="Install updates by itself"
              checked={s.autoInstallUpdates}
              disabled={!canUpdate || !s.checkUpdates}
              onChange={(v) => void save({ autoInstallUpdates: v })}
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
