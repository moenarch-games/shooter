#!/usr/bin/env bash
# Builds the browser client into web/dist: the client crate for wasm32 (WebGPU), bound with the
# wasm-bindgen CLI. The CLI must match the wasm-bindgen version in Cargo.lock.
set -euo pipefail
cd "$(dirname "$0")/.."

wanted="$(awk '/^name = "wasm-bindgen"$/ { getline; gsub(/version = |"/, ""); print }' Cargo.lock)"
bindgen=wasm-bindgen
have="$("$bindgen" --version 2>/dev/null | awk '{ print $2 }' || true)"
if [[ "$have" != "$wanted" ]]; then
  # The global CLI is another version: use (once: install) the matching one in a cache.
  root="${XDG_CACHE_HOME:-$HOME/.cache}/wasm-bindgen-cli/$wanted"
  bindgen="$root/bin/wasm-bindgen"
  if [[ ! -x "$bindgen" ]]; then
    echo "installing wasm-bindgen CLI $wanted into $root (found '${have:-none}')" >&2
    cargo install wasm-bindgen-cli --version "$wanted" --locked --root "$root"
  fi
fi

cargo build -p shooter-client --lib --target wasm32-unknown-unknown --release --locked
rm -rf web/dist
"$bindgen" --target web --no-typescript --out-dir web/dist \
  target/wasm32-unknown-unknown/release/shooter_client.wasm
cp web/index.html web/dist/
echo "web/dist is ready: serve it with any static server, e.g. python3 -m http.server -d web/dist"
