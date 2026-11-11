#!/usr/bin/env bash
# Local online play in one command: a development certificate, the deathmatch host
# (shooter-server, WebTransport on UDP 4433) and clients joining it. Bots fill the slots the
# clients leave. Ctrl-C (or closing the last native client) stops everything.
#
#   scripts/dev.sh        # host + one native client
#   scripts/dev.sh 2      # host + two native clients in the same match
#   scripts/dev.sh web    # host + the browser build on http://localhost:8080, joined to the host
set -euo pipefail
cd "$(dirname "$0")/.."

mode="${1:-1}"
port="${SHOOTER_PORT:-4433}"
web_port="${SHOOTER_WEB_PORT:-8080}"
dir="target/dev-host"
mkdir -p "$dir"

# WebTransport accepts a self-signed certificate by its hash only if it is ECDSA and valid for
# at most 14 days: make a fresh one when the last is older than 10 days.
if [[ ! -f "$dir/cert.pem" ]] || [[ -n "$(find "$dir/cert.pem" -mtime +10)" ]]; then
  openssl ecparam -name prime256v1 -genkey -noout -out "$dir/key.pem"
  openssl req -new -x509 -sha256 -key "$dir/key.pem" -out "$dir/cert.pem" -days 14 \
    -subj "/CN=localhost" -addext "subjectAltName=DNS:localhost,IP:127.0.0.1" 2>/dev/null
fi

if [[ "$mode" == web ]]; then
  scripts/build-web.sh
  cargo build --release --locked -p shooter-server
else
  cargo build --release --locked -p shooter-server -p shooter-client
fi

SHOOTER_PORT="$port" SHOOTER_CERT_PEM="$dir/cert.pem" SHOOTER_KEY_PEM="$dir/key.pem" \
  target/release/shooter-server &
host=$!
trap 'kill "$host" 2>/dev/null || true' EXIT
sleep 1

if [[ "$mode" == web ]]; then
  hash="$(openssl x509 -in "$dir/cert.pem" -outform der | openssl dgst -sha256 -r | cut -d' ' -f1)"
  echo
  echo "Open in Chrome or Edge (WebTransport + WebGPU):"
  echo "  http://localhost:$web_port/?connect=https://127.0.0.1:$port/shooter/matches/dm&certHash=$hash"
  echo
  python3 -m http.server "$web_port" -d web/dist
  exit
fi

pids=()
for _ in $(seq "$mode"); do
  target/release/shooter-client --connect "https://localhost:$port/shooter/matches/dm" \
    --cert "$dir/cert.pem" &
  pids+=("$!")
done
wait "${pids[@]}"
