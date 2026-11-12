#!/usr/bin/env bash
# Profiles the shooter with runtime-profiler and writes OUT (default target/profile):
#   bot-match/      the scripted bot match without a window (profiles/bot-match.yaml): bundle/,
#                   summary.txt and the same match as a replay, bot-match.shrp
#   native-client/  the native client drawing a scripted practice match offscreen
#                   (profiles/native-client.yaml): bundle/ and summary.txt with frame times
# CI publishes OUT as an artifact; nothing gates on the numbers. See docs/foundation-data.md.
# The client takes any wgpu adapter (WGPU_FORCE_FALLBACK_ADAPTER=1 picks lavapipe, as CI has).
set -euo pipefail
cd "$(dirname "$0")/.."

out="${1:-target/profile}"
# runtime-profiler's public CLI asset for this source revision (its main at the time of pinning).
profiler_rev=a2a9c7591d5b10ac977debcd507b872faf32ab10
profiler="target/runtime-profiler-$profiler_rev"

if [[ ! -x "$profiler" ]]; then
  mkdir -p target
  curl --fail --location --silent --show-error --retry 3 --output "$profiler.part" \
    "https://github.com/moritzbrantner/runtime-profiler/releases/download/cli-artifacts/runtime-profiler-linux-x86_64-$profiler_rev"
  chmod +x "$profiler.part"
  mv "$profiler.part" "$profiler"
fi

cargo build --release --locked -p shooter-server --bin shooter-replay -p shooter-client --bin shooter-client
rm -rf "$out"
mkdir -p "$out/bot-match" "$out/native-client"

"$profiler" capture --scenario profiles/bot-match.yaml --output "$out/bot-match/bundle" > /dev/null
"$profiler" summarize --bundle "$out/bot-match/bundle" | tee "$out/bot-match/summary.txt"
target/release/shooter-replay record --ticks 20000 -o "$out/bot-match/bot-match.shrp" |
  tee -a "$out/bot-match/summary.txt"

"$profiler" capture --scenario profiles/native-client.yaml --output "$out/native-client/bundle" > /dev/null
"$profiler" summarize --bundle "$out/native-client/bundle" | tee "$out/native-client/summary.txt"
# The profiler discards the client's output: one more run records its frame times.
target/release/shooter-client --frames 600 2>&1 | tee -a "$out/native-client/summary.txt"
