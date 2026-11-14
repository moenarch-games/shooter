#!/usr/bin/env bash
# Fetches the pinned asset-tooling kits the fighters are drawn from (Atlas D5.i46.1): the robot
# part kit (release `shooter-combat-robot`, pin assets/robot-kit.pin.json) into ROBOT_DIR (default
# target/robot-kit), the weapons kit (release `shooter-weapons`, pin assets/weapon-kit.pin.json)
# into WEAPON_DIR (default target/weapon-kit) and the wheels and tracks (release
# `shooter-robot-locomotion`, pin assets/locomotion-kit.pin.json) into LOCOMOTION_DIR (default
# target/locomotion-kit): each bundle's manifest, scenes, resources and attachments as its release
# tar holds them, and the verified tar itself as kit.tar (the browser reads the tar: one request a
# kit). The releases are rolling, so each is pinned by its tar's SHA-256 and its manifest's: a
# republished release fails here until its pin is bumped. The native client reads the directories
# (or $SHOOTER_ROBOT_KIT, $SHOOTER_WEAPON_KIT and $SHOOTER_LOCOMOTION_KIT); scripts/build-web.sh
# copies them into web/dist.
#
#   scripts/fetch-robot-kit.sh [ROBOT_DIR [WEAPON_DIR [LOCOMOTION_DIR]]]
set -euo pipefail
cd "$(dirname "$0")/.."

fetch() {
  local pin="$1" dir="$2"
  field() { python3 -I -c 'import json, sys; print(json.load(open(sys.argv[1]))[sys.argv[2]])' "$pin" "$1"; }
  local tar_url tar_sha manifest_sha
  tar_url="$(field tar)"
  tar_sha="$(field sha256)"
  manifest_sha="$(field manifestSha256)"

  if [[ -f "$dir/SHA256" && -f "$dir/kit.tar" && "$(cat "$dir/SHA256")" == "$tar_sha" ]]; then
    return 0
  fi

  local work
  work="$(mktemp -d)"
  # Bounded: each release is about 2 MB.
  if ! curl --fail --location --silent --show-error --retry 3 --max-time 120 \
    --max-filesize 33554432 --output "$work/kit.tar" "$tar_url"; then
    rm -rf "$work"
    return 1
  fi
  local got
  got="$(sha256sum "$work/kit.tar" | cut -d' ' -f1)"
  if [[ "$got" != "$tar_sha" ]]; then
    echo "$(field release): $tar_url has SHA-256 $got, pinned $tar_sha (release replaced? bump $pin)" >&2
    rm -rf "$work"
    return 1
  fi
  mkdir "$work/kit"
  tar -xf "$work/kit.tar" -C "$work/kit" --no-same-owner --no-same-permissions
  got="$(sha256sum "$work/kit/manifest.json" | cut -d' ' -f1)"
  if [[ "$got" != "$manifest_sha" ]]; then
    echo "$(field release): manifest SHA-256 $got, pinned $manifest_sha" >&2
    rm -rf "$work"
    return 1
  fi
  echo "$tar_sha" > "$work/kit/SHA256"
  mv "$work/kit.tar" "$work/kit/kit.tar"
  rm -rf "$dir"
  mkdir -p "$(dirname "$dir")"
  mv "$work/kit" "$dir"
  rm -rf "$work"
  echo "kit $(field release) ($tar_sha) in $dir"
}

fetch assets/robot-kit.pin.json "${1:-target/robot-kit}"
fetch assets/weapon-kit.pin.json "${2:-target/weapon-kit}"
fetch assets/locomotion-kit.pin.json "${3:-target/locomotion-kit}"
