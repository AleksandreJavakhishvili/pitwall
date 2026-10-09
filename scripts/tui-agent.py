#!/usr/bin/env python3
"""A synthetic busy coding agent for benchmarks (docs/spec/perf.md).

Writes what coding-agent TUIs write while working. Three styles:

- claude (default): Ink-style. Every frame (~10-12 per second) it erases
  and redraws a live region at the bottom of the main screen (spinner line
  with elapsed time and tokens, a todo list, a boxed input prompt, a status
  line) inside a synchronized update (DEC 2026), with truecolor and
  256-colour SGR; every few frames it appends finished "tool output" above
  it (bullets, diff lines with backgrounds, bold/dim/italic/underline text,
  wide characters), which scrolls into the scrollback.
- codex: ratatui-style inline viewport. History lines are inserted above
  the viewport through a scroll region (DECSTBM + reverse index), the
  viewport is redrawn with absolute cursor moves, and only the cells that
  changed (a shimmering "Working" label, the timer, the status line).
- fullscreen: an alternate-screen app. A header, a log pane that scrolls
  inside a scroll region (SU), a sidebar and an inverse status bar, all
  positioned with CUP; most rows are repainted every frame.

Every style also has occasional bursts (a build log or a file listing: a
few hundred lines at once), as real agents do. Harmless: it only prints.

  tui-agent.py                          # live, forever (a bench agent kind)
  tui-agent.py --style codex --fps 12   # another style
  tui-agent.py --frames 600             # 600 frames as fast as possible (a recording)
  tui-agent.py --history 600            # start with ~600 KB of earlier output
"""

import argparse
import os
import random
import sys
import time

SPIN = "·✢✳✶✻✽"
BRAILLE = "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"
WORDS = ["Thinking", "Reading", "Editing", "Planning", "Searching", "Compiling"]
ESC = "\x1b["
BSU = f"{ESC}?2026h"
ESU = f"{ESC}?2026l"


def rgb(r, g, b):
    return f"{ESC}38;2;{r};{g};{b}m"


def bg_rgb(r, g, b):
    return f"{ESC}48;2;{r};{g};{b}m"


RESET = f"{ESC}0m"


def size():
    try:
        c, r = os.get_terminal_size(1)
        return max(40, c), max(8, r)
    except OSError:
        return 100, 30


def output_block(i, cols):
    n = i % 7
    out = [f"{rgb(95, 211, 141)}⏺{RESET} {ESC}1mUpdate{RESET}(src/module_{i % 50}.ts)"]
    out.append(f"  ⎿  Updated {ESC}1msrc/module_{i % 50}.ts{RESET} with {n + 1} additions and {n} removals")
    for k in range(n + 2):
        if k % 2:
            out.append(f"      {bg_rgb(34, 92, 43)}{rgb(220, 255, 220)}{120 + k:>4} +   const value{k} = compute({k}, \"日本語\");{' ' * 12}{RESET}")
        else:
            out.append(f"      {bg_rgb(122, 41, 54)}{rgb(255, 220, 220)}{120 + k:>4} -   const value{k} = legacy({k});{' ' * 18}{RESET}")
    out.append(f"  {ESC}4mhttps://example.invalid/docs/{i}{RESET} {ESC}7m inverse {RESET} {ESC}38;5;{16 + i % 216}m256-colour{RESET}")
    return [x[: cols * 4] for x in out]


def burst_lines(i, rng):
    """A build log or a file listing: what an agent dumps now and then."""
    n = rng.randint(80, 300)
    if rng.random() < 0.5:
        return [
            f"{ESC}32m   Compiling{RESET} made-up-crate-{k % 37} v0.{k % 9}.{k % 4} (/work/demo/crates/part_{k % 23})"
            for k in range(n)
        ]
    return [
        f"{ESC}2m{k:>5}{RESET}  src/{['engine', 'ui', 'core', 'net'][k % 4]}/file_{i}_{k}.rs  {ESC}38;5;244m{(k * 37) % 9000} bytes{RESET}"
        for k in range(n)
    ]


class Claude:
    """Ink: erase the previous live region line by line, redraw it."""

    def __init__(self, cols, rows, redraw=False):
        self.cols, self.rows, self.prev = cols, rows, 0
        self.clear = redraw

    def live(self, i):
        w = min(self.cols, 120) - 2
        spin = SPIN[i % len(SPIN)]
        word = WORDS[(i // 40) % len(WORDS)]
        done = (i // 30) % 6
        lines = [
            f"{rgb(215, 119, 87)}{spin} {word}…{RESET} {ESC}2m({i // 10}s · ↑ {i * 37 % 9000 / 1000:.1f}k tokens · esc to interrupt){RESET}",
            f"  ⎿  {ESC}9m{ESC}2m☒ Read the made-up module{RESET}",
        ]
        for k in range(5):
            mark = "☒" if k < done else "☐"
            style = f"{ESC}2m" if k < done else (f"{ESC}1m" if k == done else "")
            lines.append(f"     {style}{mark} Step {k + 1} of the demo plan{RESET}")
        lines += [
            "",
            f"{rgb(136, 136, 136)}╭{'─' * (w - 2)}╮{RESET}",
            f"{rgb(136, 136, 136)}│{RESET} {ESC}1m>{RESET} {ESC}3mtry \"fix the flaky test in src/engine\"{RESET}{' ' * max(0, w - 42)}{rgb(136, 136, 136)}│{RESET}",
            f"{rgb(136, 136, 136)}╰{'─' * (w - 2)}╯{RESET}",
            f"  {ESC}38;5;244m? for shortcuts{RESET}{' ' * max(0, w - 52)}{ESC}38;5;108m✓ auto-accept edits on{RESET} {ESC}2m{(i * 7) % 100}% context{RESET}",
        ]
        return lines

    def frame(self, i, extra):
        buf = [BSU]
        if self.clear:
            # Ink redraws everything after a resize.
            buf.append(f"{ESC}2J{ESC}3J{ESC}H")
            self.clear = False
        if self.prev:
            buf.append(f"{ESC}2K{ESC}1A" * (self.prev - 1) + f"{ESC}2K{ESC}G")
        block = extra or (output_block(i, self.cols) if i % 8 == 0 else [])
        if block:
            buf.append("\r\n".join(block) + "\r\n")
        lines = self.live(i)
        buf.append("\r\n".join(lines))
        buf.append(ESU)
        self.prev = len(lines)
        return "".join(buf)


class Codex:
    """ratatui inline viewport: history goes in above through a scroll
    region; the viewport changes only where cells changed."""

    VIEW = 6

    def __init__(self, cols, rows, redraw=False):
        self.cols, self.rows = cols, rows
        self.top = rows - self.VIEW + 1  # first viewport row (1-based)
        self.started = False

    def insert(self, lines):
        # Scroll region = everything above the viewport; write at its bottom.
        out = [f"{ESC}1;{self.top - 1}r", f"{ESC}{self.top - 1};1H"]
        for line in lines:
            out.append(f"\r\n{line}")
        out.append(f"{ESC}r")
        return "".join(out)

    def shimmer(self, i):
        word = "Working"
        out = []
        for k, ch in enumerate(word):
            d = abs((i % 14) - k * 2)
            v = max(90, 230 - d * 22)
            out.append(f"{rgb(v, v, v)}{ch}")
        return "".join(out) + RESET

    def frame(self, i, extra):
        buf = []
        if not self.started:
            buf.append(f"{ESC}2J{ESC}H")
            self.started = True
        lines = extra or (output_block(i, self.cols) if i % 10 == 0 else [])
        if lines:
            buf.append(self.insert(lines))
        t = self.top
        spin = BRAILLE[i % len(BRAILLE)]
        buf.append(f"{ESC}{t};1H{ESC}2K{rgb(120, 170, 255)}{spin}{RESET} {self.shimmer(i)} {ESC}2m({i // 12}s • esc to interrupt){RESET}")
        if i % 12 == 0:
            buf.append(f"{ESC}{t + 2};1H{ESC}2K{ESC}36m▌{RESET} {ESC}2mAsk for follow-up changes{RESET}")
            buf.append(f"{ESC}{t + 4};1H{ESC}2K  {ESC}2m⏎ send   ⇧⏎ newline   ⌃T transcript   ⌃C quit   {(i * 13) % 100}% context left{RESET}")
        buf.append(f"{ESC}{t + 2};{4 + (i % 5)}H")
        return "".join(buf)


class Fullscreen:
    """An alternate-screen app: header, scrolling log, sidebar, status bar."""

    def __init__(self, cols, rows, redraw=False):
        self.cols, self.rows = cols, rows
        self.started = False
        self.log_w = max(20, cols - 28)

    def frame(self, i, extra):
        c, r, lw = self.cols, self.rows, self.log_w
        buf = [BSU]
        if not self.started:
            buf.append(f"{ESC}?1049h{ESC}?25l{ESC}2J")
            self.started = True
        buf.append(f"{ESC}1;1H{bg_rgb(40, 44, 52)}{rgb(200, 200, 200)}{' demo-agent · session ' + str(i // 100):<{c}}{RESET}")
        # The log pane scrolls inside its region; new lines at its bottom.
        new = extra[-40:] if extra else ([f"{ESC}2m{i:>6}{RESET} step {i}: {WORDS[i % len(WORDS)].lower()} src/part_{i % 31}.py"] if i % 2 == 0 else [])
        if new:
            buf.append(f"{ESC}2;{r - 1}r")
            for line in new:
                buf.append(f"{ESC}{r - 1};1H{ESC}S{ESC}{r - 1};1H{line[: lw * 3]}{ESC}{lw + 1}G{ESC}K")
            buf.append(f"{ESC}r")
        # Sidebar: every row repainted.
        for k in range(2, r):
            busy = (k + i) % 9 == 0
            cell = f"{BRAILLE[(i + k) % len(BRAILLE)]} task {k:>2}" if busy else f"  task {k:>2} ✓"
            buf.append(f"{ESC}{k};{lw + 2}H{rgb(100, 100, 120)}│{RESET} {rgb(180, 200, 255) if busy else ESC + '2m'}{cell:<22}{RESET}")
        buf.append(f"{ESC}{r};1H{ESC}7m {SPIN[i % len(SPIN)]} running · {i // 12}s · {(i * 37) % 9000} tokens{' ' * max(0, c - 44)}{RESET}")
        buf.append(ESU)
        return "".join(buf)


STYLES = {"claude": Claude, "codex": Codex, "fullscreen": Fullscreen}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--frames", type=int, default=0, help="write this many frames without sleeping, then exit")
    ap.add_argument("--cols", type=int, default=0)
    ap.add_argument("--rows", type=int, default=0)
    ap.add_argument("--fps", type=float, default=10.0)
    ap.add_argument("--style", choices=sorted(STYLES), default="claude")
    ap.add_argument("--burst", type=float, default=12.0, help="mean seconds between output bursts (0 = none)")
    ap.add_argument("--seed", type=int, default=None)
    ap.add_argument("--history", type=int, default=0, help="first print this many KB of finished output (a long session's scrollback)")
    args = ap.parse_args()
    cols, rows = size()
    cols, rows = args.cols or cols, args.rows or rows
    rng = random.Random(args.seed)
    ui = STYLES[args.style](cols, rows)
    out = sys.stdout
    if args.history:
        # A session that has been running a while: its output so far.
        n, k = 0, 0
        while n < args.history * 1024:
            chunk = "\r\n".join(output_block(k, cols) + burst_lines(k, rng)[:40]) + "\r\n"
            out.write(chunk)
            n += len(chunk.encode())
            k += 1
        out.flush()
    out.write(f"{ESC}?25l")  # TUIs hide the cursor while drawing
    i = 0
    next_burst = time.monotonic() + (rng.expovariate(1 / args.burst) if args.burst else float("inf"))
    while args.frames == 0 or i < args.frames:
        if not (args.cols or args.rows):
            # Follow the terminal's size, as real TUIs do on SIGWINCH.
            now_size = size()
            if now_size != (cols, rows):
                cols, rows = now_size
                ui = STYLES[args.style](cols, rows, redraw=True)
        extra = None
        now = time.monotonic()
        if args.frames == 0 and now >= next_burst:
            extra = burst_lines(i, rng)
            next_burst = now + rng.expovariate(1 / args.burst)
        out.write(ui.frame(i, extra))
        out.flush()
        i += 1
        if not args.frames:
            time.sleep(1.0 / args.fps)


if __name__ == "__main__":
    try:
        main()
    except (BrokenPipeError, KeyboardInterrupt):
        pass
