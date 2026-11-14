# shooter

A fast third-person sci-fi arena shooter for native and browser, built on the moenarch
foundation crates. It is experimental. Its second purpose is to give those crates a new,
demanding consumer: every gap or regression it finds is filed on the crate that owns it.

Product intent lives in the [Decision Atlas](docs/decisions/atlas.md). The owner has set a beta
direction (Atlas D1.i46.1, D2.i46.\*, tracker #45): a third-person arena shooter with modular
robots, about 12 slots and three arenas. The third-person camera, the robot build schema and
the 12 slots are in; the rest is not implemented yet. Everything below describes what works
today: a third-person, 12-slot deathmatch on one greybox arena with box fighters.

## What a player does

Open the native client or the browser build on Pages and join a hosted deathmatch on one arena.
Bots fill the empty slots. Move fast: sprint, slide, step up low ledges, double jump, jump pads,
rocket jumps, the grapple hook and air control. Fight with energy guns, a grapple hook and melee
weapons.

## Foundations

| Need | Crate |
| --- | --- |
| Collision, character movement, rays, projectiles | physics-engine, used directly through `shooter_sim::physics` (Rapier A/B oracle in CI) |
| Grapple rope on a character | shooter-sim stand-in until physics-engine has it (moritzbrantner/physics-engine#557) |
| Lighting, particles (muzzle flashes, trims, impacts) | render-engine |
| Positional gunfire and impacts, native and Web Audio | game-audio |
| Mouse, keyboard, gamepad, pointer lock | input-bindings |
| Authoritative matches, lag-compensated hits, WebTransport | game-server |
| Arena kit, weapons, character | asset-tooling (beta kits: moritzbrantner/asset-tooling#593, #594, #595) |
| Frame and tick traces | runtime-profiler |

## Play

The current slice is a deathmatch in a greybox arena with two jump pads onto the central platform,
six guns (energy rifle, rail gun, rocket launcher, shotgun, laser, flamethrower), the grapple hook and an energy blade. Twelve slots by default; bots fill every slot no
person holds. A kill scores a frag; dying by your own rocket or a fall costs one. The dead respawn after 2 s at the spawn point
farthest from everyone else. A round ends at 15 frags or after 5 minutes, the scores show for 8 s
and a fresh round starts. Every fighter has 200 health. The guns, the fighters (boxes) and the
sounds are placeholders until asset-tooling delivers models and clips.

**Practice offline.** The native client without arguments, and the browser build (Pages), run the
same simulation in-process: you in slot 0 against eleven bots, no server needed.

**Maps.** Every match is played on a map from one catalog in the simulation
(`shooter_sim::maps`), which both clients and the host share: a stable ID (`greybox`), its arena
(geometry, collision, spawn points, jump pads, module pickup pads) and the player count it is laid out for (the
greybox: 2 to 12). The greybox is the only map so far; the beta's three arenas will join it. The
recommended capacity never changes the slot count (12 unless the host sets another per match, at
most one per spawn point: the greybox has 13). The host picks each match's map
(`SHOOTER_MATCH_IDS=dm=greybox`, [docs/hosting.md](docs/hosting.md)) and its snapshots name it,
so a joining client builds and predicts on that map; practice takes `--map <id>` natively or
`?map=<id>` in the browser. Replays record their map and restore it; an unknown map ID is
refused everywhere.

```sh
cargo run -p shooter-client --release      # native window, practice
scripts/build-web.sh                       # browser build into web/dist (needs a WebGPU browser)
python3 -m http.server -d web/dist
```

**Play online locally.** One command makes a development certificate, starts the deathmatch host
(`shooter-server`, game-server over WebTransport on UDP 4433) and opens clients joining it. Each
client takes a slot from a bot and gets it back to a bot when it leaves.

```sh
scripts/dev.sh        # host + one native client
scripts/dev.sh 2      # host + two native clients in the same match
scripts/dev.sh web    # host + the browser build on http://localhost:8080 (prints the join URL)
cargo run -p shooter-client --release -- --connect https://localhost:4433/shooter/matches/dm --cert target/dev-host/cert.pem
```

**Join from the browser.** The page practises offline by default. `?connect=<match URL>` joins a
hosted deathmatch over the browser's WebTransport (game-server's `BrowserSession`), with
`&certHash=<SHA-256 hex>` for a development host's self-signed certificate (colons allowed);
the overlay's join fields fill these in, and "Practice offline" goes back. A host with a publicly
trusted certificate needs no hash.

**Player names.** No accounts: `&name=<name>` in the browser (the join overlay's name field,
remembered in the browser's `localStorage`) or `--name <name>` natively (remembered in
`~/.config/shooter/name`, `$XDG_CONFIG_HOME`/`%APPDATA%` respected) names you on the
scoreboard and kill feed. Names are up to 16 ASCII letters, digits, spaces, `-`, `_` and `.`;
the host sanitizes whatever arrives (`shooter_sim::names`). Without one a person shows as
"Player N"; bots stay "Bot N".

The host is authoritative: clients (native and browser alike) send one input per tick, predict
their own movement and correct it from the snapshots, and see and hear their own shots at once.
Other fighters are drawn 50 ms (3 ticks) behind the newest snapshot, so uneven arrivals still
move smoothly; hitscan hits and blade swings are checked where the attacker saw them (up to
250 ms back). The host keeps at most 2 ticks of standing input backlog per person. A dropped
connection (not a closed one) shows "reconnecting" and rejoins the same slot, score kept, within
the host's reconnect grace. A public host is described in [docs/hosting.md](docs/hosting.md).

Click to capture the mouse. WASD moves, the mouse aims, Space jumps (press again in the air to
double jump), Shift sprints, C while sprinting slides, Tab shows the scores, Esc releases the
mouse. Steer in the air with WASD. The HUD (health, weapon, frags and place, the round clock, the
carried module and its uses left, the kill and pickup feed) is the page's overlay in the browser and the window title plus a health bar natively,
until render-engine draws screen overlays.

**Camera and aim.** Native and browser clients share one third-person follow camera
(`shooter-client`'s `camera`): 3.5 m behind the eye along the aim, 0.6 m right of it and 0.3 m
above, a 1.2 rad (69°) vertical field of view. It keeps 0.2 m clear of the arena (it is pulled
in at once in front of a wall, floor or ceiling and eases back out), never through it; fighters
do not push it, and your own fighter is not drawn while the camera is inside it. Shots and
swings still leave the fighter, not the camera: every shot from the eye along the aim, the blade
from the feet, exactly as before (`shooter_sim::aim`). The reticle is drawn where that shot
meets the arena or another fighter as you see them, the same function the host resolves
hitscan with, so it sits slightly off the screen's centre (more for close targets) and lands on
cover in front of your fighter even where the camera sees past it.

The left mouse button fires; 1 to 7 pick the weapon:

| Key | Weapon | Shot |
| --- | --- | --- |
| 1 | Energy rifle | Hitscan bolts, 12 damage, 10 a second |
| 2 | Rail gun | One hitscan beam, 85 damage, every 1.2 s |
| 3 | Rocket launcher | A 24 m/s rocket, every 0.8 s: up to 100 splash damage within 3 m, and a push |
| 4 | Grapple hook | A hook that holds on the arena while the trigger is down and reels you in |
| 5 | Shotgun | A blast of 10 pellets in a fixed 4.5° spread, every 0.9 s: 240 m/s projectiles slowed by drag, 9 damage each up close down to 6 at their 25 m range |
| 6 | Laser | A continuous beam while the trigger is down: 2 damage a tick (120 a second) to the first target within 30 m |
| 7 | Flamethrower | A 6 m flame, 1.2 m across, cut short by walls: after a 0.25 s ignition, 3 damage a tick (180 a second) to everyone in it |

Rocket jump by firing at your feet as you jump: the blast throws you about 5 m up. Your own
rocket hurts you at half damage (a rocket jump costs about 35 health).

The grapple hook (4) sets on arena geometry within 30 m (not on fighters) while you hold the
trigger: it reels you toward the hook at up to 24 m/s and the rope never lets you further away
than you have come, so running or falling across it swings you round the hook. Let go to drop
off; the next hook is ready 0.75 s after the last one fired. It is a deterministic stand-in in
`shooter-sim` until physics-engine can pull a character on a rope
(moritzbrantner/physics-engine#557).

V (or the mouse's back button) swings the energy blade, a quick melee attack whatever gun you
hold: 50 damage (four swings kill), every 0.5 s, and the gun is ready again 0.3 s after a swing.
A swing strikes everyone within 2.5 m across the ground, 50° either side of where you face and
from 1 m below your feet to 2 m above them, unless the arena is in the way, and shoves them 6 m/s
away. The reach, arc and height band are the values of mmorpg's player swing. Your swing
is drawn and heard as soon as you press; the host decides what it strikes.

**Robot builds.** The simulation already knows the beta's free-form robot build
(`shooter_sim::robot`, Atlas D2.i70.1, D2.i70.2): one component tree assembled on universal
sockets, one per 20 cm surface cell (a 20 × 40 × 80 cm chassis has 28). Each part has a stable ID,
one parent socket and an exact orientation (binary angles, stored in one canonical form); a larger
part reserves its footprint of neighbouring cells on its parent's face, and no two parts may
claim one cell. The catalog has three chassis sizes, beams, hinge and rotor joints (rigid for
now), the control core, a sensor, legs, wheels, tracks, an armor plate and the eight shipped
weapons. A build is valid with one chassis root, no cycles, exactly one control core on any free
socket, each weapon at most once, at most 32 parts, and within its budgets: the core's power
against what the parts draw, the parts' cooling against the heat they make (the shipped heat,
until shooter#72) and 120 build points. Nothing requires locomotion: tanks, four-legged dogs,
humanoids, stationary turrets and grappling-only machines are all valid. Every fighter starts as
the standard build; a person's command may ask for another, which the host validates (an invalid
one rejects the command) and applies at that fighter's next spawn, never while it is alive.
Snapshots carry each fighter's build and the one it comes back as. The cell size, chassis sizes,
part values and budgets are provisional (Atlas Q2.i70.1, Q2.i70.2), the part keys await the
asset kits, and a build does not yet change movement or the arsenal: every fighter still carries
every weapon, the box fighter is still drawn, and there is no builder screen yet.

**Weapon mods.** A weapon part may carry one mod, which costs build points, weighs on the
mass, power and heat budgets and changes how that weapon fires (on the host, in the client's
prediction and in lag-compensated hits alike); a weapon without one fires as shipped. Four
provisional mods: the *rapid cycler* (rifle, rail gun, shotgun: 65% refire time, 75% damage, hot
and power-hungry), the *overcharger* (same guns: 125% damage, 130% refire time, the most power),
the *long barrel* (rifle, rail gun, laser: 150% range, 110% refire time, and its weapon becomes a
two-cell module) and the *heat sink* (any weapon: sheds 20 heat, fire unchanged, mass and points).
A mod that does not fit its weapon, or breaks a budget, rejects the build. Pick a named loadout
with `--loadout <name>` natively or `&loadout=<name>` in the browser (the join overlay's loadout
field): `standard`, `heavy`, `skirmisher` (rapid-cycler rifle, heat-sunk shotgun) or `marksman`
(long-barrel rifle along an arm, overcharged rail gun: two hits a kill instead of three), each a
humanoid component tree. Practice starts as it; online the host applies it at your next spawn.
The HUD's status line shows the held weapon's mod and every equipped one.

**Module pickups.** A few powerful modules lie on pads at contested points of a map, placed in
the map's data (`shooter_sim::pickups`, `Arena::module_pads`). Any robot that walks (or jumps
low) onto a pad takes its module, whatever its build; the module works on top of the build and
its mods and never changes the build. Two provisional modules: the *rail capacitor* (rail gun:
150% damage, 75% refire time, stacking on a rail mod; 5 boosted shots) and *thrusters* (every
jump, the air jump too, leaves at 9 m/s instead of 6: a 2.25 m jump onto the platform; 6
boosted jumps). On the greybox the capacitor lies on the platform's south-east corner, the perch
that sees the whole hall, and the thrusters in the open south-west quarter under its sightline.
A module lasts until its uses are spent, its holder dies or the round ends (owner decision on
#62); nothing drops. A robot carries one module, and taking another replaces it. A pad is back
30 s after it was taken; every pad is ready at the start of a round. When several robots reach a
ready pad in the same tick the nearest (feet to pad centre) takes it, then the lowest slot. The
host owns all of it; snapshots carry the pads and each fighter's module, the HUD's status line
shows the carried module and its uses left, the feed announces pickups, and the pads are drawn
as tinted plates with the module floating over them while it is there. Values are provisional
(Atlas Q2.i46.3); the shared energy pool (#72) does not exist yet, so modules cost no energy.

Bots are deliberately beatable and fully deterministic (seeded): they walk between spawn points,
spot the nearest fighter in sight, react after 0.35 s, turn at most 360°/s and aim with a few
degrees of wandering error, strafing as they fire; the flamethrower inside 5 m, the rifle (even
slots) or the shotgun (odd slots) up close, rockets or the laser at mid range, the rail gun far off,
and inside 3 m they rush in and swing the blade.

The movement and the weapons are checked against Rapier on a scripted tour of the arena and on
scripted moves (sprint, slide, double jump, a thruster-module jump, jump pad, ramp slides, a thin wall, a step, a rocket
jump, a grapple onto a ledge, a shooting range, a close range for the shotgun, laser and flamethrower):
`cargo test -p shooter-sim --features physics-ab --test physics_ab`.
`cargo test -p shooter-sim --test robot` checks the sockets and canonical orientations, valid
builds of every form (the loadouts, a mini-tank, a robo-dog, a humanoid, stationary and
grappling-only machines), every rejection (core, cycles, double parents, overlaps, the part
ceiling, mods), the budgets at their limits, tampered builds on the wire, modded fire and builds
through a replayed match; the A/B also runs the shooting and close ranges as the modded loadouts.
`cargo test -p shooter-sim --test pickups` checks contested pads, pad respawn, modules lasting a
life until spent or death, round and map starts, the rail capacitor's lag-compensated hits and
frags, thrusters, robots of different builds and the wire.
`cargo test -p shooter-sim --test deathmatch` checks bot matches (and that they replay exactly),
lag compensation of shots and swings, that the reticle's aim ends where the hosted beam does, blade kills, scoring and rounds, input backlog trimming, and a hosted grapple carried in snapshots and predicted; `cargo test -p shooter-client --test online` plays two
clients through a real game-server host on loopback and resumes a severed connection in its slot.

When a crate can't do something yet, the shooter files an issue on that crate and uses Rapier
(or the off-the-shelf equivalent) for that piece only, until the crate catches up.
