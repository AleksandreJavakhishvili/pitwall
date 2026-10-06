import { useEffect, useState } from "react";
import { api, getApi } from "../../api";
import type { Project } from "../../types";

/** Live project list (list_projects + projects-changed) and the onboarded flag. */
export function useProjects() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [onboarded, setOnboarded] = useState<boolean | null>(null);

  useEffect(() => {
    let alive = true;
    let off: (() => void) | null = null;
    getApi().then(async (a) => {
      const u = await a.onProjectsChanged((list) => alive && setProjects(list));
      if (!alive) return u();
      off = u;
      const [list, done] = await Promise.all([
        api.listProjects().catch(() => [] as Project[]),
        // An older backend without the flag: don't block the app on a welcome screen.
        api.getOnboarded().catch(() => true),
      ]);
      if (alive) {
        setProjects(list);
        setOnboarded(done);
      }
    });
    return () => {
      alive = false;
      off?.();
    };
  }, []);

  return { projects, setProjects, onboarded, setOnboarded };
}
