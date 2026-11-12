#!/usr/bin/env bash
# Profiles the shooter with runtime-profiler. Usage: scripts/profile.sh [OUT] [native|web|all]
# (default target/profile, all). Writes into OUT:
#   bot-match/      native: the scripted bot match without a window (profiles/bot-match.yaml):
#                   bundle/, summary.txt and the same match as a replay, bot-match.shrp
#   native-client/  native: the native client drawing a scripted practice match offscreen
#                   (profiles/native-client.yaml): bundle/ and summary.txt with frame times
#   web-client/     web: the browser client's profile run in headless Chromium
#                   (profiles/web-client.yaml): bundle/, summary.txt and frames.json
# CI publishes OUT as an artifact; nothing gates on the numbers. See docs/foundation-data.md.
# The native client takes any wgpu adapter (WGPU_FORCE_FALLBACK_ADAPTER=1 picks lavapipe, as CI
# has). The web lane builds web/dist and needs Node, `bun install` and Playwright's Chromium.
set -euo pipefail
cd "$(dirname "$0")/.."

out="${1:-target/profile}"
lane="${2:-all}"
case "$lane" in
  native | web | all) ;;
  *)
    echo "usage: scripts/profile.sh [OUT] [native|web|all]" >&2
    exit 2
    ;;
esac
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

rm -rf "$out"
mkdir -p "$out"

if [[ "$lane" != web ]]; then
  cargo build --release --locked -p shooter-server --bin shooter-replay -p shooter-client --bin shooter-client
  mkdir -p "$out/bot-match" "$out/native-client"

  "$profiler" capture --scenario profiles/bot-match.yaml --output "$out/bot-match/bundle" > /dev/null
  "$profiler" summarize --bundle "$out/bot-match/bundle" | tee "$out/bot-match/summary.txt"
  target/release/shooter-replay record --ticks 20000 -o "$out/bot-match/bot-match.shrp" |
    tee -a "$out/bot-match/summary.txt"

  "$profiler" capture --scenario profiles/native-client.yaml --output "$out/native-client/bundle" > /dev/null
  "$profiler" summarize --bundle "$out/native-client/bundle" | tee "$out/native-client/summary.txt"
  # The profiler discards the client's output: one more run records its frame times.
  target/release/shooter-client --frames 600 2>&1 | tee -a "$out/native-client/summary.txt"
fi

if [[ "$lane" != native ]]; then
  scripts/build-web.sh
  mkdir -p "$out/web-client"
  # The journey writes the page's frame times here (the profiler keeps only the trace).
  status=0
  SHOOTER_WEB_PROFILE_OUT="$(cd "$out/web-client" && pwd)/frames.json" \
    "$profiler" capture --scenario profiles/web-client.yaml --output "$out/web-client/bundle" > /dev/null ||
    status=$?
  if [[ $status == 0 ]]; then
    "$profiler" summarize --bundle "$out/web-client/bundle" | tee "$out/web-client/summary.txt"
  fi
  # The page's frame times, or what it got to when it failed.
  if [[ -f "$out/web-client/frames.json" ]]; then
    tee -a "$out/web-client/summary.txt" < "$out/web-client/frames.json"
  fi
  exit "$status"
fi
