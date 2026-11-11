# shooter

A fast first-person sci-fi arena shooter for native and browser, built on the moenarch
foundation crates. It is experimental. Its second purpose is to give those crates a new,
demanding consumer: every gap or regression it finds is filed on the crate that owns it.

Product intent lives in the [Decision Atlas](docs/decisions/atlas.md).

## What a player does

Open the native client or the browser build on Pages and join a hosted deathmatch on one arena.
Bots fill the empty slots. Move fast: sprint, slide, double jump, jump pads, rocket jumps and air
control. Fight with energy guns, a grapple hook and melee weapons.

## Foundations

| Need | Crate |
| --- | --- |
| Collision, character movement, rays, projectiles, grapple rope | physics-engine (Rapier A/B oracle in CI) |
| Lighting, particles (muzzle flashes, trims, impacts) | render-engine |
| Positional gunfire and impacts, native and Web Audio | game-audio |
| Mouse, keyboard, gamepad, pointer lock | input-bindings |
| Authoritative matches, lag-compensated hits, WebTransport | game-server |
| Arena kit, weapons, character | asset-tooling |
| Frame and tick traces | runtime-profiler |

## Play

The current slice is a deathmatch in a greybox arena with two jump pads onto the central platform
and three energy weapons. Six slots; bots fill every slot no person holds. A kill scores a frag;
dying by your own rocket or a fall costs one. The dead respawn after 2 s at the spawn point
farthest from everyone else. A round ends at 15 frags or after 5 minutes, the scores show for 8 s
and a fresh round starts. Every fighter has 200 health. The guns, the fighters (boxes) and the
sounds are placeholders until asset-tooling delivers models and clips.

**Practice offline.** The native client without arguments, and the browser build (Pages), run the
same simulation in-process: you in slot 0 against five bots, no server needed.

```sh
cargo run -p shooter-client --release      # native window, practice
scripts/build-web.sh                       # browser build into web/dist (needs a WebGPU browser)
python3 -m http.server -d web/dist
```

**Play online locally.** One command makes a development certificate, starts the deathmatch host
(`shooter-server`, game-server over WebTransport on UDP 4433) and opens native clients joining
it. Each client takes a slot from a bot and gets it back to a bot when it leaves.

```sh
scripts/dev.sh        # host + one client
scripts/dev.sh 2      # host + two clients in the same match
cargo run -p shooter-client --release -- --connect https://localhost:4433/shooter/matches/dm --cert target/dev-host/cert.pem
```

The host is authoritative: clients send one input per tick, predict their own movement and
correct it from the snapshots; hitscan hits are checked where the shooter saw the target (up to
250 ms back). The browser joins hosted matches once game-server's frames build for `wasm32`
(see the follow-up issues); a public host is described in
[docs/hosting.md](docs/hosting.md).

Click to capture the mouse. WASD moves, the mouse looks, Space jumps (press again in the air to
double jump), Shift sprints, C while sprinting slides, Tab shows the scores, Esc releases the
mouse. Steer in the air with WASD. The HUD (health, weapon, frags and place, the round clock, the
kill feed) is the page's overlay in the browser and the window title plus a health bar natively,
until render-engine draws screen overlays.

The left mouse button fires; 1, 2 and 3 pick the weapon:

| Key | Weapon | Shot |
| --- | --- | --- |
| 1 | Energy rifle | Hitscan bolts, 12 damage, 10 a second |
| 2 | Rail gun | One hitscan beam, 85 damage, every 1.2 s |
| 3 | Rocket launcher | A 24 m/s rocket, every 0.8 s: up to 100 splash damage within 3 m, and a push |

Rocket jump by firing at your feet as you jump: the blast throws you about 5 m up. Your own
rocket hurts you at half damage (a rocket jump costs about 35 health).

Bots are deliberately beatable and fully deterministic (seeded): they walk between spawn points,
spot the nearest fighter in sight, react after 0.35 s, turn at most 360°/s and aim with a few
degrees of wandering error, strafing as they fire; rifle up close, rockets at mid range, the rail
gun far off.

The movement and the weapons are checked against Rapier on a scripted tour of the arena and on
scripted moves (sprint, slide, double jump, jump pad, ramp slides, a thin wall, a step, a rocket
jump, a shooting range): `cargo test -p shooter-sim --features physics-ab --test physics_ab`.
`cargo test -p shooter-sim --test deathmatch` checks bot matches (and that they replay exactly),
lag compensation, scoring and rounds; `cargo test -p shooter-client --test online` plays two
clients through a real game-server host on loopback.

When a crate can't do something yet, the shooter files an issue on that crate and uses Rapier
(or the off-the-shelf equivalent) for that piece only, until the crate catches up.
