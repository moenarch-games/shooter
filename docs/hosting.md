# Hosting the deathmatch

`shooter-server` is the whole host: game-server's WebTransport listener with one deathmatch per
match ID. It needs no database and keeps no state across restarts (a restart starts fresh
rounds full of bots).

## Locally

`scripts/dev.sh [clients]` makes a 14-day ECDSA development certificate in `target/dev-host/`,
starts the host on UDP 4433 and opens native clients that trust that certificate by its hash.
`scripts/dev.sh web` starts the same host, builds the browser client and serves it on
`http://localhost:8080`, printing the page URL with `?connect=…&certHash=…` (the browser trusts
the development certificate by that SHA-256, through WebTransport's `serverCertificateHashes`).

## Configuration

| Variable | Default | |
| --- | --- | --- |
| `SHOOTER_PORT` | 4433 | UDP port of the WebTransport listener |
| `SHOOTER_CERT_PEM`, `SHOOTER_KEY_PEM` | `cert.pem`, `key.pem` | TLS identity (a publicly trusted certificate in production) |
| `SHOOTER_MATCH_IDS` | `dm` | comma-separated match IDs; each is one deathmatch seeded from its ID |
| `SHOOTER_ROUTE_PREFIX` | `/shooter` | a match is at `https://<host>:<port><prefix>/matches/<id>` |
| `SHOOTER_ALLOWED_ORIGINS` | unset | browser origins allowed to connect (e.g. `https://moenarch-games.github.io`); unset admits any client |

## A public host (not deployed)

Nothing is deployed. To run a public match on the shared Hetzner host the owner would:

1. Decide where it listens. The scrabble host already holds UDP 443 for `/game`; the
   shooter can share that listener only once both run in one game-server process, so the simple
   route is a UDP port of its own (e.g. 4433) opened in the firewall.
2. Build `cargo build --release -p shooter-server` for the host and install it as a systemd unit
   (as `scrabble-server` is, through moenarch.com's `deploy/`), with
   `SHOOTER_CERT_PEM`/`SHOOTER_KEY_PEM` pointing at the moenarch.com Certbot certificate
   (`/etc/letsencrypt/live/moenarch.com/fullchain.pem` and `privkey.pem`) and
   `SHOOTER_ALLOWED_ORIGINS=https://moenarch-games.github.io`.
3. Native players then join with
   `shooter-client --connect https://moenarch.com:4433/shooter/matches/dm` (no `--cert`: the
   certificate is publicly trusted). Browser players open the Pages build with
   `?connect=https://moenarch.com:4433/shooter/matches/dm` (no `certHash`), or paste that URL
   into the page's join field.
