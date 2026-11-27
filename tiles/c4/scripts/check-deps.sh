#!/bin/sh
# The C4 tile is a seed tile: it may use no Pitwall crate but the public
# `pitwall-extension` (with its macros, `pitwall-extension-macros`), and
# `pitwall-extension-test` as a dev-dependency only
# (docs/spec/2026-11-27-c4-tile/hla.md §6). It uses the
# harness, whose own dependencies are Pitwall crates under crates/ (the tile
# runtime); those are allowed only behind it, in dev builds. So:
#   - its direct dependencies name no other pitwall-* crate and no path
#     outside tiles/c4 and sdk/ (the harness excepted, dev only);
#   - what tile.wasm is built from (normal and build dependencies, followed
#     down) has no pitwall-* crate but pitwall-extension and no path package
#     outside tiles/c4 and sdk/.
# Needs cargo and jq.
set -eu

here=$(cd "$(dirname "$0")/.." && pwd -P)
root=$(cd "$here/../.." && pwd -P)

problems=$(cargo metadata --format-version 1 --all-features --manifest-path "$here/Cargo.toml" |
  jq -r --arg here "$here/" --arg sdk "$root/sdk/" --arg harness "$root/crates/pitwall-extension-test/" '
    (.packages | map({key: .id, value: .}) | from_entries) as $pkgs
    | def inside: (.manifest_path | startswith($here)) or (.manifest_path | startswith($sdk));
      (.resolve.root) as $top
    | (.resolve.nodes | map({key: .id, value: .}) | from_entries) as $nodes
    # Direct dependencies of the tile.
    | [ $nodes[$top].deps[]
        | . as $d | $pkgs[$d.pkg] as $p
        | if $p.name == "pitwall-extension-test" then
            (if any($d.dep_kinds[]; .kind != "dev") then "pitwall-extension-test is allowed as a dev-dependency only" else empty end),
            (if ($p.manifest_path | startswith($harness)) or $p.source != null then empty else "pitwall-extension-test from an unexpected path: \($p.manifest_path)" end)
          elif ($p.name | startswith("pitwall")) and $p.name != "pitwall-extension" then "not allowed: \($p.name)"
          elif $p.source == null and ($p | inside | not) then "path dependency outside tiles/c4 and sdk: \($p.name) (\($p.manifest_path))"
          else empty end ]
    # What tile.wasm is built from: normal and build edges from the tile down.
    + ( [ $top ] as $start
        | { seen: [], todo: $start }
        | until(.todo | length == 0;
            .todo[0] as $id
            | .todo |= .[1:]
            | if (.seen | index([$id])) then .
              else .seen += [$id]
                | .todo += [ $nodes[$id].deps[] | select(any(.dep_kinds[]; .kind != "dev")) | .pkg ]
              end)
        | [ .seen[] | $pkgs[.] | select(.id != $top)
            | if (.name | startswith("pitwall")) and .name != "pitwall-extension" and .name != "pitwall-extension-macros" then "not allowed in tile.wasm: \(.name)"
              elif .source == null and (inside | not) then "path dependency outside tiles/c4 and sdk in tile.wasm: \(.name) (\(.manifest_path))"
              else empty end ] )
    | unique | .[]')

if [ -n "$problems" ]; then
  echo "tiles/c4 dependency check failed:" >&2
  echo "$problems" >&2
  exit 1
fi
echo "tiles/c4 dependency check passed"
