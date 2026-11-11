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

The first slice is a greybox arena to walk, look and jump around in (no weapons yet).

```sh
cargo run -p shooter-client --release      # native window
scripts/build-web.sh                       # browser build into web/dist (needs a WebGPU browser)
python3 -m http.server -d web/dist
```

Click to capture the mouse. WASD or the left stick moves, the mouse or the right stick looks,
Space or the south face button jumps, Esc releases the mouse.

The movement is checked against Rapier on a scripted tour of the arena:
`cargo test -p shooter-sim --features physics-ab --test physics_ab`.

When a crate can't do something yet, the shooter files an issue on that crate and uses Rapier
(or the off-the-shelf equivalent) for that piece only, until the crate catches up.
