// The window's own title bar. Windows' bar is switched off (`decorations:
// false`), so the app provides what it did: a strip to drag the window by and
// the minimize, maximize and close buttons. Resizing from the window's edges
// needs nothing here: Tauri keeps that working for a window without a bar.
import { useEffect, useState, type MouseEvent, type ReactNode } from "react";
import { api, inTauri, logLine } from "../lib/api";
import { cx } from "./ui";

/** What can be clicked inside a bar. Pressing these never moves the window. */
const INTERACTIVE = "button, a, input, select, textarea, [role='button'], [role='menu'], [role='listbox'], [data-no-drag]";

/**
 * Put on a bar with `{...dragRegion}`: pressing its empty parts moves the
 * window, and a double click maximizes or restores it, as a title bar does.
 */
export const dragRegion = inTauri
  ? {
      onMouseDown: (e: MouseEvent<HTMLElement>) => {
        if (e.button !== 0 || (e.target as HTMLElement).closest(INTERACTIVE)) return;
        e.preventDefault();
        void api.windowControl(e.detail === 2 ? "maximize" : "drag").catch(() => {});
      },
    }
  : {};

function useMaximized() {
  const [maximized, setMaximized] = useState(false);
  useEffect(() => {
    if (!inTauri) return;
    // While the window is being resized this fires many times a second: ask
    // once the size has settled.
    let timer: ReturnType<typeof setTimeout> | undefined;
    const ask = () => void api.windowControl("state").then(setMaximized, () => {});
    const soon = () => {
      clearTimeout(timer);
      timer = setTimeout(ask, 120);
    };
    ask();
    window.addEventListener("resize", soon);
    return () => {
      clearTimeout(timer);
      window.removeEventListener("resize", soon);
    };
  }, []);
  return maximized;
}

function ControlButton({
  label,
  onClick,
  danger,
  children,
}: {
  label: string;
  onClick: () => void;
  danger?: boolean;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      onClick={onClick}
      // Not a tab stop, like the buttons of a system title bar: Alt+F4 and Win+arrows do the same from the keyboard.
      tabIndex={-1}
      className={cx(
        "flex h-full w-[46px] flex-none items-center justify-center text-muted transition-colors",
        danger ? "hover:bg-[#c42b1c] hover:text-white" : "hover:bg-raised hover:text-ink",
      )}
    >
      <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1" aria-hidden="true">
        {children}
      </svg>
    </button>
  );
}

/** Minimize, maximize or restore, and close. Fills the height of the bar it is in. */
export function WindowControls({ className }: { className?: string }) {
  const maximized = useMaximized();
  if (!inTauri) return null;
  const press = (action: "minimize" | "maximize" | "close") => () => {
    logLine("info", "action", `Window button: ${action}`);
    void api.windowControl(action).catch(() => {});
  };
  return (
    <div className={cx("flex h-full flex-none items-stretch", className)}>
      <ControlButton label="Minimize" onClick={press("minimize")}>
        <path d="M0.5 5.5h9" />
      </ControlButton>
      <ControlButton label={maximized ? "Restore" : "Maximize"} onClick={press("maximize")}>
        {maximized ? (
          <>
            <rect x="0.5" y="2.5" width="7" height="7" />
            <path d="M2.5 2.5v-2h7v7h-2" />
          </>
        ) : (
          <rect x="0.5" y="0.5" width="9" height="9" />
        )}
      </ControlButton>
      <ControlButton label="Close" danger onClick={press("close")}>
        <path d="M0.5 0.5l9 9M9.5 0.5l-9 9" />
      </ControlButton>
    </div>
  );
}

/**
 * A bare bar for screens that have no top bar of their own (while starting,
 * or when starting failed), so the window can still be moved and closed.
 */
export function WindowStrip() {
  if (!inTauri) return null;
  return (
    <div {...dragRegion} className="fixed inset-x-0 top-0 z-[80] flex h-9 items-stretch justify-end">
      <WindowControls />
    </div>
  );
}
