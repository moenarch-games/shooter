# Hosting the deathmatch

`shooter-server` is the whole host: game-server's WebTransport listener with one deathmatch per
match ID, each on the map the host chose for it (clients follow the map its snapshots name). It needs no database and keeps no state across restarts (a restart starts fresh
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
| `SHOOTER_MATCH_IDS` | `dm` | comma-separated matches; each is one deathmatch seeded from its ID: `<id>` plays the default map (`greybox`), `<id>=<map>` a catalogued map (`shooter_sim::maps`); either may end in `:<slots>` (default 12, e.g. `duel=greybox:2`), at most one per spawn point of the map (`MapId::max_slots`, the greybox: 13). An unknown map or a slot count out of range stops the host at startup |
| `SHOOTER_ROUTE_PREFIX` | `/shooter` | a match is at `https://<host>:<port><prefix>/matches/<id>` |
| `SHOOTER_ALLOWED_ORIGINS` | unset | browser origins allowed to connect (e.g. `https://moenarch-games.github.io`); unset admits any client. It only gates sessions that send an Origin (browsers always do), so native clients, which send none, are always admitted |

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
   certificate is publicly trusted). They are admitted although `SHOOTER_ALLOWED_ORIGINS` is
   set: the allowlist applies only to sessions carrying an Origin header, which native clients
   don't send, so native and browser players share the same match. Browser players open the Pages build with
   `?connect=https://moenarch.com:4433/shooter/matches/dm` (no `certHash`), or paste that URL
   into the page's join field.
