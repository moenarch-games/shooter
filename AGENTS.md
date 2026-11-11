# Agent Instructions

A fast first-person sci-fi arena shooter, native (wgpu) and browser (WebGPU on Pages), built on
the owner's foundation crates. Read `README.md`, the Decision Atlas (`docs/decisions/atlas.md`)
and `.conventions/index.md` before a task. Follow Decisions; ask the owner only an open Question
that materially changes the task.

## Layout

| Crate | Role |
| --- | --- |
| `shooter-sim` | Deterministic simulation the clients and the host share: the greybox arena, movement, the energy weapons, the grapple hook and the energy blade (quick melee), health, deaths, respawns and deathmatch rounds, lag-compensated hitscan and swings, the seeded bots, and the wire format (`wire`). No rendering, input devices or I/O |
| `shooter-server` | The deathmatch as a game-server `GameSimulation` and the `shooter-server` WebTransport host binary |
| `shooter-client` | One client crate for native and `wasm32`: winit window, render-engine frame, input-bindings controls, render-engine weapon lights and particles, game-audio cues, the HUD stopgap (page DOM / window title), offline practice in-process and online play with prediction (wtransport natively, game-server's `BrowserSession` in the browser) |
| `web/` | The page around the `wasm32` build; `scripts/build-web.sh` writes `web/dist` |

## Commands

```sh
cargo fmt --all -- --check
cargo test -p shooter-sim --locked                                         # movement and weapons on physics-engine
cargo test -p shooter-sim --features physics-ab --test physics_ab --locked # Rapier A/B of the tour, moves and weapons
cargo test -p shooter-sim --test deathmatch --locked                       # bots, scoring, rounds, lag compensation
cargo test -p shooter-client --test online --locked                        # two clients through a real host on loopback
cargo run -p shooter-client --release                                      # native client, practice vs bots
scripts/dev.sh [clients|web]                                               # local host + native clients (or the browser build) online
scripts/build-web.sh && python3 -m http.server -d web/dist                 # browser client (?connect=…&certHash=… joins a host)
```

CI (`.github/workflows/validate.yml`) runs fmt, Clippy, the full test suite with `physics-ab`, the
native build and a `wasm32` check; `pages.yml` builds and deploys `web/dist` from `main`.

## Foundations

- Physics is `sim-foundation` (moenarch-games/mmorpg), which wraps physics-engine and compiles the
  Rapier oracle only under `physics-ab`. Rapier never ships in a client.
- Rendering is render-engine; input semantics are `input-bindings-core`; audio is game-audio.
- Hosting is game-server (WebTransport, sessions, snapshots); the shooter owns its rules, input
  queue and lag compensation in `shooter-sim`.
- Foundations are pinned by git rev in the root `Cargo.toml`. Bump pins intentionally.
- When a foundation lacks something, file an issue on the owning repository and use the
  off-the-shelf equivalent for that piece only (Atlas D4.1). Never fork a foundation here.

## Working rules

- Happy path first; run focused tests and fmt locally, CI runs the rest.
- Simulation code stays deterministic: integer yaw and fixed-point velocities, fixed 60 Hz ticks,
  no wall clock or ambient randomness.
- One issue, one branch, one PR. Pick the simplest option for open choices and list it in the PR.
