#!/usr/bin/env bash
# Builds the browser client into web/dist: the client crate for wasm32 (WebGPU, else WebGL 2), bound with the
# wasm-bindgen CLI, and the pinned kits' release tars beside it in web/dist/robot-kit/kit.tar,
# web/dist/weapon-kit/kit.tar, web/dist/locomotion-kit/kit.tar and web/dist/arena-kit/kit.tar
# (scripts/fetch-robot-kit.sh).
# The CLI must match the wasm-bindgen
# version in Cargo.lock.
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
mkdir -p web/dist/fonts
cp crates/shooter-client/assets/fonts/BarlowSemiCondensed-SemiBold.ttf crates/shooter-client/assets/fonts/OFL.txt web/dist/fonts/
# The pinned kits' tars beside the page: the client fetches robot-kit/kit.tar,
# weapon-kit/kit.tar, locomotion-kit/kit.tar and arena-kit/kit.tar (one request a kit,
# shooter#85) and checks them against their pins.
scripts/fetch-robot-kit.sh
for kit in robot-kit weapon-kit locomotion-kit arena-kit; do
  mkdir -p "web/dist/$kit"
  cp "target/$kit/kit.tar" "web/dist/$kit/kit.tar"
done
echo "web/dist is ready: serve it with any static server, e.g. python3 -m http.server -d web/dist"
