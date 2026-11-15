import { useEffect, useState } from "react";

/** The current time, refreshed at an interval. Pass null to stop the clock. */
export function useNow(everyMs: number | null): number {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    if (everyMs === null) return;
    setNow(Date.now());
    const timer = setInterval(() => setNow(Date.now()), everyMs);
    return () => clearInterval(timer);
  }, [everyMs]);
  return now;
}
