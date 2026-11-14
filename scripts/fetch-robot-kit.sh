#!/usr/bin/env bash
# Fetches the pinned asset-tooling robot kit (release `shooter-combat-robot`, Atlas D5.i46.1) into
# DIR (default target/robot-kit): the bundle's manifest, scenes and resources as the release tar
# holds them. The release is rolling, so it is pinned by its tar's SHA-256 and its manifest's
# (assets/robot-kit.pin.json): a republished release fails here until the pin is bumped. The
# native client reads DIR (or $SHOOTER_ROBOT_KIT); scripts/build-web.sh copies it into web/dist.
set -euo pipefail
cd "$(dirname "$0")/.."

dir="${1:-target/robot-kit}"
pin=assets/robot-kit.pin.json
field() { python3 -I -c 'import json, sys; print(json.load(open(sys.argv[1]))[sys.argv[2]])' "$pin" "$1"; }
tar_url="$(field tar)"
tar_sha="$(field sha256)"
manifest_sha="$(field manifestSha256)"

if [[ -f "$dir/SHA256" && "$(cat "$dir/SHA256")" == "$tar_sha" ]]; then
  exit 0
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
# Bounded: the release is about 2 MB.
curl --fail --location --silent --show-error --retry 3 --max-time 120 --max-filesize 33554432 \
  --output "$work/kit.tar" "$tar_url"
got="$(sha256sum "$work/kit.tar" | cut -d' ' -f1)"
if [[ "$got" != "$tar_sha" ]]; then
  echo "robot kit: $tar_url has SHA-256 $got, pinned $tar_sha (release replaced? bump $pin)" >&2
  exit 1
fi
mkdir "$work/kit"
tar -xf "$work/kit.tar" -C "$work/kit" --no-same-owner --no-same-permissions
got="$(sha256sum "$work/kit/manifest.json" | cut -d' ' -f1)"
if [[ "$got" != "$manifest_sha" ]]; then
  echo "robot kit: manifest SHA-256 $got, pinned $manifest_sha" >&2
  exit 1
fi
echo "$tar_sha" > "$work/kit/SHA256"
rm -rf "$dir"
mkdir -p "$(dirname "$dir")"
mv "$work/kit" "$dir"
echo "robot kit $(field release) ($tar_sha) in $dir"
