#!/usr/bin/env bash
# Profiles the scripted bot match (profiles/bot-match.yaml) with runtime-profiler and records the
# same match as a replay. Writes OUT (default target/profile): bundle/ (the runtime-profiler
# evidence bundle), summary.txt and bot-match.shrp. CI publishes OUT as an artifact; nothing gates
# on the numbers. See docs/foundation-data.md.
set -euo pipefail
cd "$(dirname "$0")/.."

out="${1:-target/profile}"
# runtime-profiler's public CLI asset for this source revision (its main at the time of pinning).
profiler_rev=fd26af06b649d319fdd9c1cf14fce63f1abaa324
profiler="target/runtime-profiler-$profiler_rev"

if [[ ! -x "$profiler" ]]; then
  mkdir -p target
  curl --fail --location --silent --show-error --retry 3 --output "$profiler.part" \
    "https://github.com/moritzbrantner/runtime-profiler/releases/download/cli-artifacts/runtime-profiler-linux-x86_64-$profiler_rev"
  chmod +x "$profiler.part"
  mv "$profiler.part" "$profiler"
fi

cargo build --release --locked -p shooter-server --bin shooter-replay
rm -rf "$out"
mkdir -p "$out"
"$profiler" capture --scenario profiles/bot-match.yaml --output "$out/bundle" > /dev/null
"$profiler" summarize --bundle "$out/bundle" | tee "$out/summary.txt"
target/release/shooter-replay record --ticks 20000 -o "$out/bot-match.shrp" | tee -a "$out/summary.txt"
