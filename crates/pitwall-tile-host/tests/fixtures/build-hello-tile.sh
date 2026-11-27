#!/bin/sh
# Rebuilds the committed components of the fixture extensions that the
# host's, the harness's and the app's tests load: hello/tile.wasm from
# hello-tile/ and blocks/tile.wasm from blocks-tile/ (canvases, drag and
# drop, code and diff views). Both are Rust, written with the
# pitwall-extension SDK in sdk/, whose wit/ copies ../../wit. Needs the
# wasm32-wasip2 target; rustup installs it from each fixture's
# rust-toolchain.toml on first use (or run
# `rustup target add wasm32-wasip2`). The workspace build never runs this
# (CI's Tile SDK job builds it but doesn't commit it): commit the rebuilt
# tile.wasm files with the change to wit/ or the tiles.
#
#   build-hello-tile.sh          rebuild the committed hello/tile.wasm and
#                                blocks/tile.wasm
#   build-hello-tile.sh heavy    build a tile-sized variant (regex,
#                                serde_json, chrono) into
#                                hello-tile/target/heavy-tile.wasm, not
#                                committed, for compile-time measurements
set -eu
here=$(cd "$(dirname "$0")" && pwd)
cd "$here/hello-tile"
if [ "${1:-}" = heavy ]; then
  cargo build --release --target wasm32-wasip2 --features heavy
  cp target/wasm32-wasip2/release/hello_tile.wasm target/heavy-tile.wasm
  ls -l target/heavy-tile.wasm
else
  cargo build --release --target wasm32-wasip2
  cp target/wasm32-wasip2/release/hello_tile.wasm "$here/hello/tile.wasm"
  ls -l "$here/hello/tile.wasm"
  cd "$here/blocks-tile"
  cargo build --release --target wasm32-wasip2
  cp target/wasm32-wasip2/release/blocks_tile.wasm "$here/blocks/tile.wasm"
  ls -l "$here/blocks/tile.wasm"
  # The services fixture (net, sql, notify, show, toolbar, service), the
  # same way.
  cd "$here/services-tile"
  cargo build --release --target wasm32-wasip2
  cp target/wasm32-wasip2/release/services_tile.wasm "$here/services/tile.wasm"
  ls -l "$here/services/tile.wasm"
fi
