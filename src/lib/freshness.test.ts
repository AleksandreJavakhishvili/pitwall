// Source-control freshness (lib/freshness.ts): refresh on open, ↻ / ⌘⇧R,
// errors with Retry, polling paused while the window is hidden.
import { describe, expect, it } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { FreshController, onRefreshRequest, requestRefresh, updatedLabel, type Fresh, type FreshEnv, type FreshSource } from "./freshness";
import { FreshError, RefreshControl } from "../components/Freshness";
import { DEFAULT_HOST, setHost } from "./host";

/** A promise the test settles by hand. */
function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

const flush = () => new Promise((r) => setTimeout(r, 0));

/** Manual time, timers and visibility. */
function fakeEnv() {
  const env = {
    t: 1_000_000,
    isHidden: false,
    ticks: [] as (() => void)[],
    visible: [] as (() => void)[],
    now: () => env.t,
    hidden: () => env.isHidden,
    every(_ms: number, fn: () => void) {
      env.ticks.push(fn);
      return () => (env.ticks = env.ticks.filter((f) => f !== fn));
    },
    onVisible(fn: () => void) {
      env.visible.push(fn);
      return () => (env.visible = env.visible.filter((f) => f !== fn));
    },
    tick: () => env.ticks.forEach((f) => f()),
    show() {
      env.isHidden = false;
      env.visible.forEach((f) => f());
    },
  };
  return env satisfies FreshEnv & Record<string, unknown>;
}

/** A source whose reads the test answers one by one. */
function source(everyMs = 5_000) {
  const loads: ReturnType<typeof deferred<string[]>>[] = [];
  const forces: ReturnType<typeof deferred<string[]>>[] = [];
  const src: FreshSource<string[]> = {
    load: () => {
      const d = deferred<string[]>();
      loads.push(d);
      return d.promise;
    },
    force: () => {
      const d = deferred<string[]>();
      forces.push(d);
      return d.promise;
    },
    everyMs,
  };
  return { src, loads, forces };
}

function setup(everyMs = 5_000) {
  const env = fakeEnv();
  const s = source(everyMs);
  const states: Fresh<string[]>[] = [];
  const c = new FreshController(s.src, (st) => states.push(st), env);
  return { env, c, states, ...s, last: () => states[states.length - 1] };
}

describe("FreshController", () => {
  it("opens with one forced refresh, showing refreshing… then when it was read", async () => {
    const { c, env, forces, loads, last } = setup();
    c.start();
    expect(forces).toHaveLength(1);
    expect(loads).toHaveLength(0);
    expect(last()).toMatchObject({ refreshing: true, data: null, updatedAt: null });
    env.t += 300;
    forces[0].resolve(["a.ts"]);
    await flush();
    expect(last()).toEqual({ data: ["a.ts"], error: null, refreshing: false, updatedAt: env.t });
    c.stop();
  });

  it("the refresh button coalesces: asked again while one runs, the same one is awaited", async () => {
    const { c, forces, last } = setup();
    c.start();
    const again = c.refresh();
    const third = c.refresh();
    expect(forces).toHaveLength(1);
    expect(again).toBe(third);
    forces[0].resolve(["x"]);
    await again;
    expect(last().refreshing).toBe(false);
    // Once done, the button forces a new one.
    void c.refresh();
    expect(forces).toHaveLength(2);
    c.stop();
  });

  it("polls while visible, not while hidden, and once on coming back", async () => {
    const { c, env, forces, loads, last } = setup();
    c.start();
    env.tick();
    expect(loads).toHaveLength(0); // the forced read is still running
    forces[0].resolve(["a"]);
    await flush();
    env.tick();
    expect(loads).toHaveLength(1);
    env.tick();
    expect(loads).toHaveLength(1); // one read at a time
    env.t += 5_000;
    loads[0].resolve(["a", "b"]);
    await flush();
    expect(last()).toMatchObject({ data: ["a", "b"], updatedAt: env.t, refreshing: false });

    env.isHidden = true;
    env.tick();
    env.tick();
    expect(loads).toHaveLength(1);
    env.show();
    expect(loads).toHaveLength(2);
    c.stop();
    env.tick();
    expect(loads).toHaveLength(2);
  });

  it("keeps the data on an error, says why, and Retry forces again", async () => {
    const { c, forces, loads, env, last } = setup();
    c.start();
    forces[0].resolve(["a"]);
    await flush();
    env.tick();
    loads[0].reject("git: machine went away");
    await flush();
    expect(last()).toMatchObject({ data: ["a"], error: "git: machine went away" });
    void c.refresh(); // Retry
    expect(forces).toHaveLength(2);
    forces[1].resolve(["a", "c"]);
    await flush();
    expect(last()).toMatchObject({ data: ["a", "c"], error: null, refreshing: false });
    c.stop();
  });

  it("a one-off forced read (picking an agent) replaces the source's this time", async () => {
    const { c, forces, last } = setup(0);
    c.start();
    forces[0].resolve(["a"]);
    await flush();
    await c.refresh(async () => ["picked"]);
    expect(forces).toHaveLength(1);
    expect(last().data).toEqual(["picked"]);
    c.stop();
  });

  it("nothing lands after the view closes", async () => {
    const { c, forces, states } = setup();
    c.start();
    const n = states.length;
    c.stop();
    forces[0].resolve(["late"]);
    await flush();
    expect(states).toHaveLength(n);
  });

  it("keeps the same data object when a read finds nothing new", async () => {
    const { c, forces, loads, env, last } = setup();
    c.start();
    forces[0].resolve(["a"]);
    await flush();
    const before = last().data;
    env.tick();
    loads[0].resolve(["a"]);
    await flush();
    expect(last().data).toBe(before);
    c.stop();
  });
});

describe("⌘⇧R", () => {
  it("belongs to the app and reaches every open view", async () => {
    const { OWNED_SHORTCUT } = await import("./shortcuts");
    const ev = { key: "R", code: "KeyR", metaKey: true, ctrlKey: false, altKey: false, shiftKey: true } as KeyboardEvent;
    expect(OWNED_SHORTCUT(ev)).toBe(true);
    let n = 0;
    const off = [onRefreshRequest(() => n++), onRefreshRequest(() => n++)];
    requestRefresh();
    expect(n).toBe(2);
    off.forEach((f) => f());
    requestRefresh();
    expect(n).toBe(2);
  });
});

describe("freshness UI", () => {
  it("says how old the data is", () => {
    const t = 10_000_000;
    expect(updatedLabel(t, t + 2_000)).toBe("updated just now");
    expect(updatedLabel(t, t + 12_000)).toBe("updated 12 s ago");
    expect(updatedLabel(t, t + 180_000)).toBe("updated 3 min ago");
    expect(updatedLabel(t, t + 2 * 3_600_000)).toBe("updated 2 h ago");
  });

  it("shows refreshing…, the age and a ↻ button with this desktop's shortcut", () => {
    const busy = renderToStaticMarkup(createElement(RefreshControl, { refreshing: true, updatedAt: null, onRefresh: () => {} }));
    expect(busy).toContain("refreshing…");
    expect(busy).toContain('title="Refresh (⌘⇧R)"');
    expect(busy).toContain('aria-busy="true"');
    const at = Date.now() - 12_000;
    const idle = renderToStaticMarkup(createElement(RefreshControl, { refreshing: false, updatedAt: at, onRefresh: () => {} }));
    expect(idle).toMatch(/updated 1[23] s ago/);
    setHost({ ...DEFAULT_HOST, shortcuts: "ctrlShift" });
    try {
      const other = renderToStaticMarkup(createElement(RefreshControl, { refreshing: false, updatedAt: null, onRefresh: () => {} }));
      expect(other).toContain('title="Refresh (Ctrl+Shift+Alt+R)"');
    } finally {
      setHost(DEFAULT_HOST);
    }
  });

  it("shows errors inline with Retry", () => {
    const html = renderToStaticMarkup(createElement(FreshError, { error: "fatal: gone\nmore", prefix: "Couldn't read changes: ", onRetry: () => {} }));
    expect(html).toContain("Couldn&#x27;t read changes: fatal: gone");
    expect(html).not.toContain("gone</span>more"); // only the first line is shown (the rest is in the tooltip)
    expect(html).toContain(`title="fatal: gone\nmore"`);
    expect(html).toContain(">Retry</button>");
  });
});

describe("worktree list refresh", () => {
  it("coalesces forced refreshes per project and records when it was read", async () => {
    const { forceRefreshWorktrees } = await import("./useWorktrees");
    const a = forceRefreshWorktrees("p1");
    expect(forceRefreshWorktrees("p1")).toBe(a);
    expect(forceRefreshWorktrees("p2")).not.toBe(a);
    const list = await a;
    expect(Array.isArray(list)).toBe(true);
  });
});
