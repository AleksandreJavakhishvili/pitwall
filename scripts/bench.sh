#!/bin/sh
# Pitwall performance benchmark (docs/spec/perf.md): RSS/footprint + CPU of an
# isolated Pitwall test instance's process tree (app + its WebKit processes;
# holders separately, agents never) across scenarios. See scripts/bench.py.
#
# Usage: scripts/bench.sh <path/to/separately-built/pitwall> [--counts 1,5,15,20] [--out results.json]
set -eu
exec python3 "$(dirname "$0")/bench.py" "$@"
