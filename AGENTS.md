# Agent Instructions

A sci-fi arena shooter, native (wgpu) and browser (WebGPU on Pages), built on the owner's
foundation crates. What ships today is a third-person, 12-slot deathmatch on one greybox arena
with the fighters drawn as their robot builds from asset-tooling's kit (still moving as capsules)
and placeholder guns. The rest of the robot beta (modular robots, about 12
slots, three arenas; Atlas D1.i46.1 and D2.i46.\*, tracker #45) is mostly intent, not
implementation: describe shipped behaviour as it is until a feature lands. Read `README.md`, the Decision Atlas
(`docs/decisions/atlas.md`) and `.conventions/index.md` before a task. Follow Decisions; ask the owner only an open Question
that materially changes the task.

## Layout

| Crate | Role |
| --- | --- |
| `shooter-sim` | Deterministic simulation the clients and the host share: the map catalog (`maps`: stable map IDs, each map's arena and recommended capacity; the greybox is the only map and the test and A/B fixture), movement, the energy weapons, the grapple hook and the energy blade (quick melee), per-part damage (`damage`: part health, part hitboxes on the fighter, penetration, blasts, lost parts, the core as the only kill), deaths, respawns and deathmatch rounds, lag-compensated hitscan and swings, the shared aim (`aim`: where a shot from the eye lands, for the host's hitscan and the clients' reticle), the seeded bots, the robot build schema, catalog and validation (`robot`), mechanical robots (`mechanics`: compound bodies, motorised joints, wheels, tracks and legs with gait presets, motor energy, exact state, on `physics::articulated`), the contested module pickups (`pickups`), and the wire format (`wire`). No rendering, input devices or I/O |
| `shooter-server` | The deathmatch as a game-server `GameSimulation`, the `shooter-server` WebTransport host binary, and `shooter-replay` (record, verify and promote scripted match replays into foundation fixtures, `docs/foundation-data.md`) |
| `shooter-client` | One client crate for native and `wasm32`: winit window, render-engine frame and the third-person camera and reticle (`camera`), input-bindings controls, render-engine weapon lights and particles, the fighters as robots from the pinned asset-tooling kit (`robots`: bundle pins and bounds, glTF and Basis decode, parts placed as the simulation places them, poses, labelled placeholders), game-audio cues, the HUD stopgap (page DOM / window title), offline practice in-process and online play with prediction (wtransport natively, game-server's `BrowserSession` in the browser) |
| `web/` | The page around the `wasm32` build; `scripts/build-web.sh` writes `web/dist` |

## Commands

```sh
cargo fmt --all -- --check
cargo test -p shooter-sim --locked                                         # movement and weapons on physics-engine
cargo test -p shooter-sim --features physics-ab --test physics_ab --locked # Rapier A/B of the tour, moves, weapons, mechanics
cargo test -p shooter-sim --test deathmatch --locked                       # bots, scoring, rounds, lag compensation
cargo test -p shooter-sim --test mechanics --locked                        # mechanical robots: fixtures, gaits, motor energy, exact restore
cargo test -p shooter-sim --test damage --locked                           # per-part damage: front/side hits, splash, plates, lost parts, core kill, battery blast
cargo test -p shooter-client --test online --locked                        # two clients through a real host on loopback
scripts/fetch-robot-kit.sh                                                 # the pinned asset-tooling robot kit into target/robot-kit (the client and its tests read it)
cargo test -p shooter-client --test robots --locked                        # the robot kit: pins, part mapping, placement, poses, lost parts, joint pivots
cargo run -p shooter-client --release --example robot_gallery -- DIR       # loadouts, mechanics fixtures and every pose rendered offscreen (PPM)
cargo run -p shooter-client --release                                      # native client, practice vs bots
scripts/dev.sh [clients|web]                                               # local host + native clients (or the browser build) online
scripts/build-web.sh && python3 -m http.server -d web/dist                 # browser client (?connect=…&certHash=… joins a host)
cargo run --release -p shooter-server --bin shooter-replay -- record -o dm.shrp  # scripted match replay (then verify / promote)
cargo run -p shooter-client --release -- --frames 600 [--capture DIR]      # native client, scripted match drawn offscreen (profile run); --capture writes every 30th frame
scripts/profile.sh [OUT] [native|web|all]                                  # runtime-profiler bundles: bot match, offscreen native client, browser (?frames=120)
```

CI (`.github/workflows/validate.yml`) runs fmt, Clippy, the robot kit fetch and the full test suite with `physics-ab`, the
native build and a `wasm32` check; `pages.yml` builds and deploys `web/dist` from `main`;
`profile.yml` uploads the bot-match and native-client (lavapipe) profiles, and in a second job the
browser client's (headless Chromium, SwiftShader WebGPU), as artifacts (no thresholds).

## Foundations

- Physics is physics-engine, used directly through `shooter_sim::physics` (Atlas D4.3): the
  shooter builds every character move itself. Its Rapier oracle backend compiles only under
  `physics-ab`; Rapier never ships in a client. The shooter does not depend on mmorpg.
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
