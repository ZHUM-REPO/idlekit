import { useEffect } from "react";
import { CloseDialog, UpdateBar } from "./components/Desktop";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { Sidebar, Toasts, TopBar } from "./components/Shell";
import { Button } from "./components/ui";
import { inTauri, logLine } from "./lib/api";
import { useApp } from "./lib/store";
import { AccountsPage } from "./pages/Accounts";
import { AchievementsPage } from "./pages/Achievements";
import { FarmingPage } from "./pages/Farming";
import { IdlingPage } from "./pages/Idling";
import { LibraryPage } from "./pages/Library";
import { LogsPage } from "./pages/Logs";
import { MarketPage } from "./pages/Market";
import { SettingsPage } from "./pages/Settings";

let shown = false;

/**
 * Writes one line to the log when the first page is on screen. It sits beside
 * the page inside the same error boundary, so a page that fails to draw never
 * gets this line.
 */
function Shown({ page }: { page: string }) {
  useEffect(() => {
    if (shown) return;
    shown = true;
    logLine("info", "ui", `The window is ready: showing the ${page} page`);
  }, [page]);
  return null;
}

export function App() {
  const ready = useApp((s) => s.ready);
  const fatal = useApp((s) => s.fatal);
  const page = useApp((s) => s.page);
  const helperFound = useApp((s) => s.helperFound);
  const init = useApp((s) => s.init);

  useEffect(() => {
    void init();
  }, [init]);

  if (fatal) {
    return (
      <div className="flex h-full items-center justify-center p-10">
        <div className="max-w-lg">
          <h1 className="heading text-[24px] font-semibold">IdleKit couldn't start</h1>
          <p className="mt-2 text-muted">{fatal}</p>
          <Button className="mt-4" onClick={() => window.location.reload()}>
            Try again
          </Button>
        </div>
      </div>
    );
  }
  if (!ready) return <div className="h-full" />;

  return (
    <div className="flex h-full">
      <Sidebar />
      <div className="flex min-w-0 flex-1 flex-col">
        <ErrorBoundary>
          <TopBar />
        </ErrorBoundary>
        {!inTauri && (
          <div className="border-b border-line bg-panel px-5 py-1.5 text-[12px] text-faint">
            Browser preview with sample data. Nothing here talks to Steam; run the desktop app for real use.
          </div>
        )}
        {inTauri && !helperFound && (
          <div className="border-b border-bad/40 bg-bad-soft px-5 py-1.5 text-[12px]">
            The Steam client helper is missing, so idling through the Steam client and achievements won't work. Reinstall
            IdleKit, or run <code>npm run helper</code> in development.
          </div>
        )}
        <UpdateBar />
        <main className="min-h-0 flex-1 overflow-y-auto px-7 pt-6">
          <ErrorBoundary resetKey={page}>
            {page === "library" && <LibraryPage />}
            {page === "idling" && <IdlingPage />}
            {page === "farming" && <FarmingPage />}
            {page === "achievements" && <AchievementsPage />}
            {page === "market" && <MarketPage />}
            {page === "accounts" && <AccountsPage />}
            {page === "settings" && <SettingsPage />}
            {page === "logs" && <LogsPage />}
            <Shown page={page} />
          </ErrorBoundary>
        </main>
      </div>
      <CloseDialog />
      <Toasts />
    </div>
  );
}
