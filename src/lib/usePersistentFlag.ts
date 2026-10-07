import { useEffect, useState } from "react";

/** Per-window boolean remembered in localStorage (a convenience; never critical). */
export function usePersistentFlag(key: string, initial: boolean) {
  const [v, setV] = useState(() => {
    try {
      const s = localStorage.getItem(key);
      return s === null ? initial : s === "1";
    } catch {
      return initial;
    }
  });
  useEffect(() => {
    try {
      localStorage.setItem(key, v ? "1" : "0");
    } catch {
      /* ignore */
    }
  }, [key, v]);
  return [v, setV] as const;
}
