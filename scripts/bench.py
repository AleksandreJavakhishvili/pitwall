#!/usr/bin/env python3
"""Pitwall performance benchmark (docs/spec/perf.md). Run via scripts/bench.sh.

Launches a *separately built* Pitwall binary as an isolated test instance:
its own PITWALL_HOME (a fresh temp dir: state, sockets, holders), the bench
hook on (PITWALL_BENCH=1), and harmless output-generator agents. It never
touches ~/.pitwall, a running /Applications/Pitwall.app or its holders, and
only ever stops processes it started, by exact PID.

Build the instance first, with its own bundle id so WebKit storage is
separate too, e.g.:

  CARGO_TARGET_DIR=/tmp/pw-bench pnpm tauri build --no-bundle --config "$(cat scripts/bench-tauri.json)"
  scripts/bench.sh /tmp/pw-bench/release/pitwall

(bench-tauri.json also opens the window off-screen and unfocused from the
first frame; with PITWALL_BENCH=1 the app itself never activates, moves its
windows off-screen and sends no notifications.)

What is measured (per scenario, after a settle period):
- app:      the app process (Rust) + transient children it reaped (git, …)
- webkit:   WebKit XPC processes whose *responsible* pid is the app
            (WebContent, GPU, Networking), via
            responsibility_get_pid_responsible_for_pid
- holders:  this instance's pitwall-hold processes (reported separately;
            the agents they run are never counted)
Memory: phys_footprint (what Activity Monitor shows as "Memory") and RSS.
CPU: user+system time over the sample window, as % of one core.
"""

from __future__ import annotations

import argparse
import ctypes
import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time
from pathlib import Path

# ── process introspection (macOS libproc) ────────────────────────────────────

libc = ctypes.CDLL(None)
_resp = libc.responsibility_get_pid_responsible_for_pid
_resp.restype = ctypes.c_int
_resp.argtypes = [ctypes.c_int]


class RUsageV2(ctypes.Structure):
    _fields_ = [
        ("ri_uuid", ctypes.c_uint8 * 16),
        ("ri_user_time", ctypes.c_uint64),
        ("ri_system_time", ctypes.c_uint64),
        ("ri_pkg_idle_wkups", ctypes.c_uint64),
        ("ri_interrupt_wkups", ctypes.c_uint64),
        ("ri_pageins", ctypes.c_uint64),
        ("ri_wired_size", ctypes.c_uint64),
        ("ri_resident_size", ctypes.c_uint64),
        ("ri_phys_footprint", ctypes.c_uint64),
        ("ri_proc_start_abstime", ctypes.c_uint64),
        ("ri_proc_exit_abstime", ctypes.c_uint64),
        ("ri_child_user_time", ctypes.c_uint64),
        ("ri_child_system_time", ctypes.c_uint64),
        ("ri_child_pkg_idle_wkups", ctypes.c_uint64),
        ("ri_child_interrupt_wkups", ctypes.c_uint64),
        ("ri_child_pageins", ctypes.c_uint64),
        ("ri_child_elapsed_abstime", ctypes.c_uint64),
        ("ri_diskio_bytesread", ctypes.c_uint64),
        ("ri_diskio_byteswritten", ctypes.c_uint64),
    ]


_rusage = libc.proc_pid_rusage
_rusage.restype = ctypes.c_int
_rusage.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.POINTER(RUsageV2)]


class Timebase(ctypes.Structure):
    _fields_ = [("numer", ctypes.c_uint32), ("denom", ctypes.c_uint32)]


_tb = Timebase()
libc.mach_timebase_info(ctypes.byref(_tb))
NS_PER_TICK = _tb.numer / _tb.denom


def rusage(pid: int) -> RUsageV2 | None:
    r = RUsageV2()
    return r if _rusage(pid, 2, ctypes.byref(r)) == 0 else None


def processes() -> list[tuple[int, int, str]]:
    out = subprocess.run(["ps", "-axo", "pid=,ppid=,args="], capture_output=True, text=True).stdout
    rows = []
    for line in out.splitlines():
        parts = line.strip().split(None, 2)
        if len(parts) >= 2:
            rows.append((int(parts[0]), int(parts[1]), parts[2] if len(parts) > 2 else ""))
    return rows


def classify(app_pid: int, home: str) -> dict[str, list[int]]:
    """The instance's processes by group. Holders (and everything under them:
    the agents) are kept out of the app's numbers."""
    rows = processes()
    children: dict[int, list[int]] = {}
    for pid, ppid, _ in rows:
        children.setdefault(ppid, []).append(pid)
    holders = {pid for pid, _, args in rows if "pitwall-hold" in args.split(" ")[0] and home in args}
    # Holders may not carry the path in argv: also take pitwall-hold processes
    # whose holder socket dir is ours (checked via lsof would be slow) — fall
    # back to: pitwall-hold descendants of the app.
    excluded: set[int] = set()

    def under(p: int, acc: set[int]):
        for c in children.get(p, []):
            if c not in acc:
                acc.add(c)
                under(c, acc)

    app_desc: set[int] = set()
    under(app_pid, app_desc)
    for pid, _, args in rows:
        if pid in app_desc and os.path.basename(args.split(" ")[0]) == "pitwall-hold":
            holders.add(pid)
    for h in holders:
        excluded.add(h)
        under(h, excluded)
    groups: dict[str, list[int]] = {"app": [app_pid], "webkit": [], "other": [], "holders": sorted(holders)}
    for pid, _, args in rows:
        if pid == app_pid or pid in excluded:
            continue
        name = os.path.basename(args.split(" ")[0])
        if pid in app_desc:
            groups["other"].append(pid)
        elif _resp(pid) == app_pid and "WebKit" in name:
            groups["webkit"].append(pid)
    return groups


def snapshot(groups: dict[str, list[int]]):
    snap = {}
    for g, pids in groups.items():
        for pid in pids:
            r = rusage(pid)
            if r:
                cpu = r.ri_user_time + r.ri_system_time
                if g == "app":
                    cpu += r.ri_child_user_time + r.ri_child_system_time
                snap[pid] = (g, cpu * NS_PER_TICK, r.ri_phys_footprint, r.ri_resident_size)
    return snap


def webkit_names(groups) -> dict[int, str]:
    rows = {pid: args for pid, _, args in processes()}
    return {pid: os.path.basename(rows.get(pid, "?").split(" ")[0]).replace("com.apple.WebKit.", "") for pid in groups["webkit"]}


def measure(app_pid: int, home: str, secs: float) -> dict:
    groups = classify(app_pid, home)
    a = snapshot(groups)
    t0 = time.monotonic()
    time.sleep(secs)
    dt = time.monotonic() - t0
    groups = classify(app_pid, home)
    b = snapshot(groups)
    names = webkit_names(groups)
    res: dict = {"groups": {}}
    for pid, (g, cpu, foot, rss) in b.items():
        e = res["groups"].setdefault(g, {"footprint_mb": 0.0, "rss_mb": 0.0, "cpu_pct": 0.0, "n": 0})
        e["footprint_mb"] += foot / 2**20
        e["rss_mb"] += rss / 2**20
        e["n"] += 1
        if pid in a:
            e["cpu_pct"] += (cpu - a[pid][1]) / 1e9 / dt * 100
        if g == "webkit":
            k = "wk_" + names.get(pid, "?")
            w = res.setdefault(k, {"footprint_mb": 0.0, "rss_mb": 0.0})
            w["footprint_mb"] += foot / 2**20
            w["rss_mb"] += rss / 2**20
    tree = [res["groups"].get(g, {}) for g in ("app", "webkit", "other")]
    res["tree_footprint_mb"] = sum(e.get("footprint_mb", 0) for e in tree)
    res["tree_rss_mb"] = sum(e.get("rss_mb", 0) for e in tree)
    res["tree_cpu_pct"] = sum(e.get("cpu_pct", 0) for e in tree)
    return res


# ── the instance ─────────────────────────────────────────────────────────────

GEN = r"""#!/bin/sh
# Bench agent: a chatty but harmless output generator (~1.5 KB / 0.5 s,
# colours, a status line), like an agent streaming its work.
i=0
while :; do
  i=$((i+1))
  printf '\033[1;36m● step %d\033[0m reading src/module_%d.ts\n' "$i" "$((i % 50))"
  j=0
  while [ $j -lt 12 ]; do
    printf '  \033[32m+\033[0m line %d of a long-ish diff hunk with some text to wrap around the pane %d\n' "$j" "$i"
    j=$((j+1))
  done
  printf '\033[2m  tokens: %d · elapsed %ds\033[0m\n' "$((i * 137))" "$((i / 2))"
  sleep 0.5
done
"""

KIND = """id = "bench"
name = "Bench"
command = "{gen}"
new_args = []
resume_args = []
assign_session_id = false
hooks = "none"
"""


class Instance:
    def __init__(self, app: Path, cli: Path, home: Path, project: Path):
        self.app, self.cli, self.home, self.project = app, cli, home, project
        self.proc: subprocess.Popen | None = None
        self.n_cmd = 0
        self.agents = 0
        self.log = open(home / "app.log", "ab")

    def env(self):
        e = dict(os.environ)
        e["PITWALL_HOME"] = str(self.home)
        e["PITWALL_BENCH"] = "1"
        e.pop("PITWALL_CLI_SOCKET", None)
        return e

    def launch(self) -> float:
        ready = self.home / "bench-ready"
        ready.unlink(missing_ok=True)
        t0 = time.monotonic()
        self.proc = subprocess.Popen([str(self.app)], env=self.env(), stdout=self.log, stderr=self.log, start_new_session=True)
        while not ready.exists():
            if self.proc.poll() is not None:
                raise SystemExit(f"app exited early ({self.proc.returncode}); see {self.home / 'app.log'}")
            if time.monotonic() - t0 > 60:
                raise SystemExit("app never became ready (bench hook missing in this build?)")
            time.sleep(0.01)
        return (time.monotonic() - t0) * 1000

    def quit(self):
        if self.proc and self.proc.poll() is None:
            os.kill(self.proc.pid, signal.SIGTERM)  # exact PID: the instance we started
            try:
                self.proc.wait(10)
            except subprocess.TimeoutExpired:
                os.kill(self.proc.pid, signal.SIGKILL)
                self.proc.wait(5)
        self.proc = None

    def command(self, cmd: str):
        self.n_cmd += 1
        (self.home / "bench-cmd").write_text(f"{self.n_cmd} {cmd}\n")
        time.sleep(0.5)

    def add_agents(self, n: int):
        sock = self.home / "run" / "pitwalld.sock"
        while self.agents < n:
            name = f"bench-{self.agents + 1}"
            cmd = [str(self.cli), "--socket", str(sock), "agent", "new", "--kind", "bench", "--project", str(self.project), "--name", name]
            for attempt in range(20):
                r = subprocess.run(cmd, env=self.env(), capture_output=True, text=True, timeout=30)
                if r.returncode == 0:
                    break
                time.sleep(0.25)  # the socket may not be up yet
            else:
                raise SystemExit(f"could not create {name}: {r.stdout}{r.stderr}")
            self.agents += 1

    def stop_holders(self):
        """Stop this instance's holders and their agents, by exact PID."""
        rows = processes()
        children: dict[int, list[int]] = {}
        for pid, ppid, _ in rows:
            children.setdefault(ppid, []).append(pid)
        hold_dir = str(self.home / "run" / "hold")
        ours = [pid for pid, _, args in rows if os.path.basename(args.split(" ")[0]) == "pitwall-hold" and hold_dir in args]
        if not ours:
            # The holder's argv may not name its socket: match by open socket path.
            for pid, _, args in rows:
                if os.path.basename(args.split(" ")[0]) != "pitwall-hold":
                    continue
                files = subprocess.run(["lsof", "-a", "-p", str(pid), "-U", "-F", "n"], capture_output=True, text=True).stdout
                if hold_dir in files:
                    ours.append(pid)
        victims: list[int] = []

        def under(p):
            for c in children.get(p, []):
                victims.append(c)
                under(c)

        for h in ours:
            under(h)
            victims.append(h)
        for pid in victims:
            try:
                os.kill(pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
        return ours


def holders_of(home: Path) -> list[int]:
    hold_dir = str(home / "run" / "hold")
    out = []
    for pid, _, args in processes():
        if os.path.basename(args.split(" ")[0]) != "pitwall-hold":
            continue
        if hold_dir in args:
            out.append(pid)
            continue
        files = subprocess.run(["lsof", "-a", "-p", str(pid), "-U", "-F", "n"], capture_output=True, text=True).stdout
        if hold_dir in files:
            out.append(pid)
    return out


def holder_stats(home: Path) -> dict:
    pids = holders_of(home)
    foot = rss = 0
    for p in pids:
        r = rusage(p)
        if r:
            foot += r.ri_phys_footprint
            rss += r.ri_resident_size
    return {"n": len(pids), "footprint_mb": foot / 2**20, "rss_mb": rss / 2**20}


def make_project(root: Path) -> Path:
    p = root / "project"
    (p / "src").mkdir(parents=True)
    body = "\n".join(f"export function f{i}(x: number): number {{ return x * {i} + {i % 7}; }}" for i in range(1500))
    (p / "src" / "big.ts").write_text(body + "\n")
    (p / "README.md").write_text("bench project\n")
    git = ["git", "-C", str(p), "-c", "user.name=bench", "-c", "user.email=bench@example.invalid", "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null"]
    subprocess.run(git + ["init", "-q"], check=True, timeout=30)
    subprocess.run(git + ["add", "."], check=True, timeout=30)
    subprocess.run(git + ["commit", "-qm", "init"], check=True, timeout=30)
    # An uncommitted change, so Review has a diff to open.
    lines = body.splitlines()
    for k in range(0, len(lines), 40):
        lines[k] = lines[k].replace("return x", "return 2 * x")
    (p / "src" / "big.ts").write_text("\n".join(lines) + "\n")
    return p


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("app", help="the Pitwall binary to test (a separate build, never /Applications/Pitwall.app)")
    ap.add_argument("--cli", help="pitwall-cli next to it (default: sibling of APP)")
    ap.add_argument("--counts", default="1,5,15,20")
    ap.add_argument("--settle", type=float, default=8.0, help="seconds before each sample")
    ap.add_argument("--sample", type=float, default=10.0, help="sample window, seconds")
    ap.add_argument("--label", default="run")
    ap.add_argument("--out", help="write JSON results here")
    ap.add_argument("--keep", action="store_true", help="keep the temp PITWALL_HOME")
    ap.add_argument("--quick", action="store_true", help="one cold start, no Wall/Review scenarios")
    ap.add_argument("--then", action="append", default=[], help="after the last count: send this bench command and sample again (repeatable)")
    ap.add_argument("--profile", help="also run macOS `sample` on the app process for 5 s per scenario, writing <dir>/<scenario>.txt")
    ap.add_argument("--agent", choices=["bench", "tui"], default="bench", help="bench: a shell loop (~3 KB/s); tui: scripts/tui-agent.py, a Claude/Codex-like redrawing TUI (~12 KB/s, 10 frames/s)")
    args = ap.parse_args()

    app = Path(args.app).resolve()
    if "/Applications/" in str(app) or not app.is_file():
        raise SystemExit(f"refusing: {app} (pass a separately built binary)")
    cli = Path(args.cli).resolve() if args.cli else app.parent / "pitwall-cli"
    if not cli.is_file():
        raise SystemExit(f"no pitwall-cli at {cli}")

    # Short path: Unix socket paths (run/hold/<uuid>.sock) are limited to 104 bytes.
    root = Path(tempfile.mkdtemp(prefix="pwb-", dir="/tmp")).resolve()
    home = root / "home"
    (home / "agents").mkdir(parents=True)
    (home / "projects.json").write_text(json.dumps({"version": 1, "onboarded": True, "projects": []}))
    gen = root / "gen.sh"
    if args.agent == "tui":
        tui = Path(__file__).resolve().parent / "tui-agent.py"
        gen.write_text(f"#!/bin/sh\nexec python3 '{tui}'\n")
    else:
        gen.write_text(GEN)
    gen.chmod(0o755)
    (home / "agents" / "bench.toml").write_text(KIND.format(gen=gen))
    project = make_project(root)
    inst = Instance(app, cli, home, project)
    results: dict = {"label": args.label, "app": str(app), "scenarios": {}}

    def record(name: str, extra: dict | None = None):
        assert inst.proc
        time.sleep(args.settle)
        m = measure(inst.proc.pid, str(home), args.sample)
        m["holders"] = holder_stats(home)
        m.update(extra or {})
        if args.profile:
            Path(args.profile).mkdir(parents=True, exist_ok=True)
            out = Path(args.profile) / (name.replace(" ", "_").replace("+", "_") + ".txt")
            subprocess.run(["sample", str(inst.proc.pid), "5", "-f", str(out)], capture_output=True, timeout=60)
        results["scenarios"][name] = m
        g = m["groups"]
        wk = ", ".join(f"{k[3:]} {v['footprint_mb']:.0f}" for k, v in m.items() if k.startswith("wk_"))
        print(
            f"{name:<16} tree {m['tree_footprint_mb']:7.1f} MB footprint ({m['tree_rss_mb']:7.1f} RSS)  "
            f"cpu {m['tree_cpu_pct']:5.1f}%  | app {g.get('app', {}).get('footprint_mb', 0):6.1f} MB "
            f"{g.get('app', {}).get('cpu_pct', 0):4.1f}%  webkit [{wk}] {g.get('webkit', {}).get('cpu_pct', 0):4.1f}%  "
            f"| holders {m['holders']['n']} × {m['holders']['footprint_mb'] / max(1, m['holders']['n']):.1f} MB",
            flush=True,
        )

    try:
        # Cold start, 0 agents.
        starts = []
        for _ in range(0 if args.quick else 3):
            starts.append(inst.launch())
            time.sleep(2)
            inst.quit()
            time.sleep(1)
        ms = inst.launch()
        starts.append(ms)
        results["cold_start_ms"] = sorted(starts)[len(starts) // 2]
        print(f"cold start to first window (median of {len(starts)}): {results['cold_start_ms']:.0f} ms  {[round(s) for s in starts]}", flush=True)
        record("idle-0")
        inst.quit()

        counts = [int(c) for c in args.counts.split(",") if c]
        for n in counts:
            # A fresh app per count; agents live on in their holders between runs.
            inst.launch()
            inst.add_agents(n)
            inst.command("visit-all")
            time.sleep(0.15 * n)
            record(f"agents-{n}")
            if n == counts[-1] and args.then:
                for cmd in args.then:
                    inst.command(cmd)
                    record(f"agents-{n} {cmd}")
            if args.quick:
                inst.quit()
                continue
            inst.command("wall on")
            record(f"agents-{n}+wall")
            inst.command("wall off")
            if n == 5:
                record(f"agents-{n}-wall-closed")
                inst.command("review on")
                record(f"agents-{n}+review")
                inst.command("review off")
                record(f"agents-{n}-review-closed")
            inst.quit()
    finally:
        inst.quit()
        stopped = inst.stop_holders()
        print(f"stopped {len(stopped)} holders (exact PIDs)", flush=True)
        if args.out:
            Path(args.out).write_text(json.dumps(results, indent=2))
        if not args.keep:
            time.sleep(1)
            shutil.rmtree(root, ignore_errors=True)
        else:
            print(f"kept {root}")


if __name__ == "__main__":
    sys.exit(main())
