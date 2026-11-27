#!/bin/sh
# The C4 tile is a seed tile: it may use no Pitwall crate but the public
# `pitwall-extension` (and `pitwall-extension-test`, as a dev-dependency
# only), and no path dependency outside tiles/c4/ and sdk/
# (docs/spec/2026-11-27-c4-tile/hla.md §6). Needs cargo and jq.
set -eu

here=$(cd "$(dirname "$0")/.." && pwd -P)
root=$(cd "$here/../.." && pwd -P)

problems=$(cargo metadata --format-version 1 --all-features --manifest-path "$here/Cargo.toml" |
  jq -r --arg here "$here/" --arg sdk "$root/sdk/" '
    (.packages | map({key: .id, value: .}) | from_entries) as $pkgs
    | [ .resolve.nodes[]
        | $pkgs[.id]
        | select(.name | startswith("pitwall"))
        | select(.name != "pitwall-extension" and .name != "pitwall-extension-macros" and .name != "pitwall-extension-test")
        | "not allowed: \(.name)" ]
    + [ .resolve.nodes[] | .deps[]
        | select($pkgs[.pkg].name == "pitwall-extension-test")
        | select(any(.dep_kinds[]; .kind != "dev"))
        | "pitwall-extension-test is allowed as a dev-dependency only" ]
    + [ .packages[]
        | select(.source == null)
        | select((.manifest_path | startswith($here)) or (.manifest_path | startswith($sdk)) | not)
        | "path dependency outside tiles/c4 and sdk: \(.name) (\(.manifest_path))" ]
    | unique | .[]')

if [ -n "$problems" ]; then
  echo "tiles/c4 dependency check failed:" >&2
  echo "$problems" >&2
  exit 1
fi
echo "tiles/c4 dependency check passed"
