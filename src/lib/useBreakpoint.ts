import { useEffect, useState } from "react";

export type Breakpoint = "xl" | "lg" | "md" | "sm" | "xs";

export interface Responsive {
  bp: Breakpoint;
  /** Sidebar presentation when not collapsed. */
  sidebar: "full" | "rail" | "hidden";
  /** Right panel docks on wide windows, else slides in as a drawer. */
  rightDocked: boolean;
  maxPerRow: number;
}

export function compute(w: number): Responsive {
  if (w >= 2200) return { bp: "xl", sidebar: "full", rightDocked: true, maxPerRow: 4 };
  if (w >= 1500) return { bp: "lg", sidebar: "full", rightDocked: true, maxPerRow: 3 };
  if (w >= 1100) return { bp: "md", sidebar: "full", rightDocked: false, maxPerRow: 2 };
  if (w >= 700) return { bp: "sm", sidebar: "rail", rightDocked: false, maxPerRow: 1 };
  return { bp: "xs", sidebar: "hidden", rightDocked: false, maxPerRow: 1 };
}

/** Breakpoints from docs/spec/layout.md, per window width. */
export function useBreakpoint(): Responsive {
  const [r, setR] = useState(() => compute(window.innerWidth));
  useEffect(() => {
    let raf = 0;
    const on = () => {
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(() => setR((prev) => {
        const next = compute(window.innerWidth);
        return next.bp === prev.bp ? prev : next;
      }));
    };
    window.addEventListener("resize", on);
    return () => {
      cancelAnimationFrame(raf);
      window.removeEventListener("resize", on);
    };
  }, []);
  return r;
}
