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

The current slice is a greybox arena to run, slide and jump around in, with two jump pads onto the
central platform, three energy weapons and five target dummies to shoot (one walks to and fro).
Downed dummies come back after 3 s. There are no other players or bots yet, and the guns, dummies
and sounds are placeholders until asset-tooling delivers models and clips.

```sh
cargo run -p shooter-client --release      # native window
scripts/build-web.sh                       # browser build into web/dist (needs a WebGPU browser)
python3 -m http.server -d web/dist
```

Click to capture the mouse. WASD or the left stick moves, the mouse or the right stick looks,
Space or the south face button jumps (press again in the air to double jump), Shift sprints, C
while sprinting slides, Esc releases the mouse. Steer in the air with WASD.

The left mouse button fires; 1, 2 and 3 pick the weapon:

| Key | Weapon | Shot |
| --- | --- | --- |
| 1 | Energy rifle | Hitscan bolts, 12 damage, 10 a second |
| 2 | Rail gun | One hitscan beam, 85 damage, every 1.2 s |
| 3 | Rocket launcher | A 24 m/s rocket, every 0.8 s: up to 100 splash damage within 3 m, and a push |

Rocket jump by firing at your feet as you jump: the blast throws you about 5 m up. Rockets do not
hurt their owner.

The movement and the weapons are checked against Rapier on a scripted tour of the arena and on
scripted moves (sprint, slide, double jump, jump pad, ramp slides, a thin wall, a step, a rocket
jump, a shooting range): `cargo test -p shooter-sim --features physics-ab --test physics_ab`.

When a crate can't do something yet, the shooter files an issue on that crate and uses Rapier
(or the off-the-shelf equivalent) for that piece only, until the crate catches up.
