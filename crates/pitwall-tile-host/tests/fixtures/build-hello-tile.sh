#!/bin/sh
# Rebuilds hello-tile.wasm (the committed component the tests and benches
# load) from hello-tile/. Needs the wasm32-wasip2 target; rustup installs it
# from hello-tile/rust-toolchain.toml on first use (or run
# `rustup target add wasm32-wasip2`). The workspace build never runs this.
#
#   build-hello-tile.sh          rebuild the committed hello-tile.wasm
#   build-hello-tile.sh heavy    build a tile-sized variant (regex,
#                                serde_json, chrono) into
#                                hello-tile/target/heavy-tile.wasm, not
#                                committed; pass it to the tile_bench example
set -eu
here=$(cd "$(dirname "$0")" && pwd)
cd "$here/hello-tile"
if [ "${1:-}" = heavy ]; then
  cargo build --release --target wasm32-wasip2 --features heavy
  cp target/wasm32-wasip2/release/hello_tile.wasm target/heavy-tile.wasm
  ls -l target/heavy-tile.wasm
else
  cargo build --release --target wasm32-wasip2
  cp target/wasm32-wasip2/release/hello_tile.wasm "$here/hello-tile.wasm"
  ls -l "$here/hello-tile.wasm"
fi
