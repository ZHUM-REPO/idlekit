import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { logLine } from "./lib/api";
import "./styles.css";

// Faults in the window that nothing else caught still end up in the log.
window.addEventListener("error", (e) => {
  // The browser's own notice about layout timing, not a fault in the app.
  if (e.message.startsWith("ResizeObserver loop")) return;
  logLine("error", "ui", `Uncaught error: ${e.message} (${e.filename.split("/").pop() ?? ""}:${e.lineno})`);
});
window.addEventListener("unhandledrejection", (e) => {
  const reason = e.reason instanceof Error ? `${e.reason.name}: ${e.reason.message}` : String(e.reason);
  logLine("error", "ui", `Unhandled failure: ${reason}`);
});

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);

// The window's script is running. "The window is ready" follows from App once
// the first page is on screen; a window that stays blank stops between the two.
logLine("debug", "ui", "The window's script started");
