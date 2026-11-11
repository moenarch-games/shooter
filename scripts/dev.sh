#!/usr/bin/env bash
# Local online play in one command: a development certificate, the deathmatch host
# (shooter-server, WebTransport on UDP 4433) and native clients joining it. Bots fill the slots
# the clients leave. Ctrl-C (or closing the last client) stops everything.
#
#   scripts/dev.sh        # host + one client
#   scripts/dev.sh 2      # host + two clients in the same match
set -euo pipefail
cd "$(dirname "$0")/.."

clients="${1:-1}"
port="${SHOOTER_PORT:-4433}"
dir="target/dev-host"
mkdir -p "$dir"

# WebTransport accepts a self-signed certificate by its hash only if it is ECDSA and valid for
# at most 14 days: make a fresh one when the last is older than 10 days.
if [[ ! -f "$dir/cert.pem" ]] || [[ -n "$(find "$dir/cert.pem" -mtime +10)" ]]; then
  openssl ecparam -name prime256v1 -genkey -noout -out "$dir/key.pem"
  openssl req -new -x509 -sha256 -key "$dir/key.pem" -out "$dir/cert.pem" -days 14 \
    -subj "/CN=localhost" -addext "subjectAltName=DNS:localhost,IP:127.0.0.1" 2>/dev/null
fi

cargo build --release --locked -p shooter-server -p shooter-client

SHOOTER_PORT="$port" SHOOTER_CERT_PEM="$dir/cert.pem" SHOOTER_KEY_PEM="$dir/key.pem" \
  target/release/shooter-server &
host=$!
trap 'kill "$host" 2>/dev/null || true' EXIT
sleep 1

pids=()
for _ in $(seq "$clients"); do
  target/release/shooter-client --connect "https://localhost:$port/shooter/matches/dm" \
    --cert "$dir/cert.pem" &
  pids+=("$!")
done
wait "${pids[@]}"
