#!/usr/bin/env bash
# Checks that native and `wasm32` builds simulate the same match bit for bit (shooter#120): runs
# shooter-sim's `state_hashes` example (the clients' scripted practice match, a state hash per
# tick) natively and as wasm32-wasip1 under Node's WASI (scripts/wasi-run.mjs), and fails at the
# first tick where they differ, naming the map, the tick and the parts of the state that differ.
# Online play depends on this: a browser client predicts against a native host.
#
#   scripts/cross-build-hashes.sh [MAP|all [TICKS]]   (default: every catalogued map, 600 ticks)
#
# Needs Node 20+ and the wasm32-wasip1 target (added here when rustup is there). To see the
# values that differ, dump a tick's snapshot from both builds:
#   cargo run -p shooter-sim --example state_hashes -- MAP TICKS TICK
#   node scripts/wasi-run.mjs target/wasm32-wasip1/debug/examples/state_hashes.wasm MAP TICKS TICK
set -euo pipefail
cd "$(dirname "$0")/.."

map="${1:-all}"
ticks="${2:-600}"
out="target/cross-build-hashes"
mkdir -p "$out"

if command -v rustup >/dev/null && ! rustup target list --installed | grep -qx wasm32-wasip1; then
  rustup target add wasm32-wasip1
fi

cargo build -p shooter-sim --example state_hashes --locked
cargo build -p shooter-sim --example state_hashes --locked --target wasm32-wasip1

target/debug/examples/state_hashes "$map" "$ticks" >"$out/native.txt"
node --no-warnings scripts/wasi-run.mjs \
  target/wasm32-wasip1/debug/examples/state_hashes.wasm "$map" "$ticks" >"$out/wasm32.txt"

if cmp -s "$out/native.txt" "$out/wasm32.txt"; then
  echo "native and wasm32 agree: $(wc -l <"$out/native.txt") map ticks hashed ($map, $ticks ticks)"
  exit 0
fi

# The first differing line of each map.
paste -d '\n' "$out/native.txt" "$out/wasm32.txt" | paste - - | awk -F '\t' '
  $1 != $2 {
    split($1, line, " ")
    if (!(line[1] in seen)) {
      seen[line[1]] = 1
      printf "%s: native and wasm32 diverge at tick %s\n< %s\n> %s\n", line[1], line[2], $1, $2
    }
  }'
exit 1
