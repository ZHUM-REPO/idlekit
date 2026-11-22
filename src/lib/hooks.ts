import { useEffect, useState } from "react";

/** Current time, re-rendering every `ms`. */
export function useNow(ms = 1000, running = true): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    // Nothing on screen depends on the time: no timer, no redraws.
    if (!running) return;
    setNow(Date.now());
    const id = setInterval(() => setNow(Date.now()), ms);
    return () => clearInterval(id);
  }, [ms, running]);
  return now;
}
