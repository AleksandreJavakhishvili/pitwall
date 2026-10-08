#!/usr/bin/env python3
"""A synthetic busy coding agent for benchmarks (docs/spec/perf.md).

Writes what Claude Code / Codex style TUIs write while working: every frame
(~10 per second) it erases and redraws a live region at the bottom (spinner
line with elapsed time and tokens, a boxed input prompt, a hint line) with
truecolor and 256-colour SGR, and every few frames it appends finished
"tool output" above it (bullets, diff lines with backgrounds, bold/dim/
italic/underline text, wide characters). Harmless: it only prints.

  tui-agent.py                 # live, forever (a bench agent kind)
  tui-agent.py --frames 600    # 600 frames as fast as possible (a recording)
"""

import argparse
import os
import sys
import time

SPIN = "·✢✳✶✻✽"
WORDS = ["Thinking", "Reading", "Editing", "Planning", "Searching", "Compiling"]
ESC = "\x1b["


def rgb(r, g, b):
    return f"{ESC}38;2;{r};{g};{b}m"


def bg_rgb(r, g, b):
    return f"{ESC}48;2;{r};{g};{b}m"


RESET = f"{ESC}0m"


def size():
    try:
        c, r = os.get_terminal_size(1)
        return max(40, c), max(12, r)
    except OSError:
        return 100, 30


def live_region(i, cols):
    """The bottom region redrawn every frame: (text, number of lines)."""
    w = min(cols, 120) - 2
    spin = SPIN[i % len(SPIN)]
    word = WORDS[(i // 40) % len(WORDS)]
    lines = [
        f"{rgb(215, 119, 87)}{spin} {word}…{RESET} {ESC}2m({i // 10}s · ↑ {i * 37 % 9000 / 1000:.1f}k tokens · esc to interrupt){RESET}",
        "",
        f"{rgb(136, 136, 136)}╭{'─' * (w - 2)}╮{RESET}",
        f"{rgb(136, 136, 136)}│{RESET} {ESC}1m>{RESET} {ESC}3mtry \"fix the flaky test in src/engine\"{RESET}{' ' * max(0, w - 42)}{rgb(136, 136, 136)}│{RESET}",
        f"{rgb(136, 136, 136)}╰{'─' * (w - 2)}╯{RESET}",
        f"  {ESC}38;5;244m? for shortcuts{RESET}{' ' * max(0, w - 40)}{ESC}38;5;108m✓ auto-accept edits on{RESET}",
    ]
    return "\r\n".join(lines), len(lines)


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
    return "\r\n".join(x[: cols * 4] for x in out) + "\r\n"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--frames", type=int, default=0, help="write this many frames without sleeping, then exit")
    ap.add_argument("--cols", type=int, default=0)
    ap.add_argument("--fps", type=float, default=10.0)
    args = ap.parse_args()
    cols, _ = (args.cols, 0) if args.cols else size()
    out = sys.stdout
    out.write(f"{ESC}?25l")  # TUIs hide the cursor while drawing
    prev = 0
    i = 0
    while args.frames == 0 or i < args.frames:
        buf = []
        if prev:
            # Ink-style erase of the previous live region.
            buf.append(f"{ESC}2K{ESC}1A" * (prev - 1) + f"{ESC}2K{ESC}G")
        if i % 8 == 0:
            buf.append(output_block(i, cols))
        text, prev = live_region(i, cols)
        buf.append(text)
        out.write("".join(buf))
        out.flush()
        i += 1
        if not args.frames:
            time.sleep(1.0 / args.fps)


if __name__ == "__main__":
    try:
        main()
    except (BrokenPipeError, KeyboardInterrupt):
        pass
