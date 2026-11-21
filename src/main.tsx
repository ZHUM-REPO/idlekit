import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { logLine } from "./lib/api";
import "./styles.css";

// Faults in the window that nothing else caught still end up in the log.
window.addEventListener("error", (e) => {
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

// Says the window's contents came up: a window that stays blank never writes this.
logLine("info", "ui", "The window loaded");
