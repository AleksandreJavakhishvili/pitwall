#!/bin/sh
# Rebuilds questions/tile.wasm, the committed component of the questions
# extension (the agents' questions group's fixture), from questions-tile/
# (Rust, written with the pitwall-extension SDK in sdk/). Needs the
# wasm32-wasip2 target (rustup installs it from
# questions-tile/rust-toolchain.toml). Commit the rebuilt tile.wasm with a
# change to wit/ or to the tile.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
cd "$here/questions-tile"
cargo build --release --target wasm32-wasip2
cp target/wasm32-wasip2/release/questions_tile.wasm "$here/questions/tile.wasm"
ls -l "$here/questions/tile.wasm"
