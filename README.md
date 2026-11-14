# shooter

A fast third-person sci-fi arena shooter for native and browser, built on the moenarch
foundation crates. It is experimental. Its second purpose is to give those crates a new,
demanding consumer: every gap or regression it finds is filed on the crate that owns it.

Product intent lives in the [Decision Atlas](docs/decisions/atlas.md). The owner has set a beta
direction (Atlas D1.i46.1, D2.i46.\*, tracker #45): a third-person arena shooter with modular
robots, about 12 slots and three arenas. The third-person camera, the robot build schema and
the 12 slots are in, and the fighters are drawn as their robot builds from asset-tooling's robot
kit; the rest is not implemented yet. Everything below describes what works today: a
third-person, 12-slot deathmatch on greybox arenas (the 76 m greybox yard by default, the 40 m
greybox hall as the test fixture), the fighters still moving as capsules.

## What a player does

Open the native client or the browser build on Pages and join a hosted deathmatch on a greybox arena.
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

The current slice is a deathmatch in a greybox arena (by default the greybox yard, below) with ramps,
a raised level and jump pads onto it,
six guns (energy rifle, rail gun, rocket launcher, shotgun, laser, flamethrower), the grapple hook and an energy blade. Twelve slots by default; bots fill every slot no
person holds. A kill scores a frag; dying by your own rocket, your own battery's blast or a fall costs one. The dead respawn after 2 s at the spawn point
farthest from everyone else. A round ends at 15 frags or after 5 minutes, the scores show for 8 s
and a fresh round starts. A fighter is its robot build with per-part health (below, **Part
damage**): destroying its root chassis is the kill. Each fighter is drawn as its robot build (below, **Robot
models**), its weapons and mods from asset-tooling's weapons kit; the sounds are placeholders
until asset-tooling delivers them.

**Practice offline.** The native client without arguments, and the browser build (Pages), run the
same simulation in-process: you in slot 0 against eleven bots, no server needed.

**Maps.** Every match is played on a map from one catalog in the simulation
(`shooter_sim::maps`), which both clients and the host share: a stable ID, its arena
(geometry, collision, spawn points, jump pads, module pickup pads, bot waypoints) and the player
count it is laid out for. Two interim greybox maps, until the beta's three arenas arrive with
asset-tooling's arena kit:

- `greybox-yard`, the default for hosted matches and practice (6 to 12 players, 14 spawn points):
  a 76 m square with 5 m walls, point-symmetric (the north half turned half a turn is the south
  half). A raised walkway crosses the middle, a 3 m deck on two pillars that you can walk under,
  reached by a 12 m ramp at either end and by a jump pad on either side. A 5 m wall line with
  gaps at its middle and ends shuts off each outer yard from the central band; each outer yard
  has a 4 m cross wall, a 2 m platform with its own ramp, a 7 m tower only the grapple (or a
  rocket jump) reaches, a 2 m container, 4 m wing and bunker walls, 2 m and 1.2 m cover walls
  and crates. No spawn point is in sight of another, and none is a dead end. The rail capacitor
  lies on the walkway's middle and thrusters on each platform. Plan and a view:
  [docs/captures/82](docs/captures/82).
- `greybox`, the first 40 m hall (2 to 12 players, 13 spawn points), kept unchanged as the test
  and Rapier A/B fixture.

The recommended capacity never changes the slot count (12 unless the host sets another per
match, at most one per spawn point). The host picks each match's map
(`SHOOTER_MATCH_IDS=dm=greybox-yard`, [docs/hosting.md](docs/hosting.md)) and its snapshots name it,
so a joining client builds and predicts on that map; practice takes `--map <id>` natively or
`?map=<id>` in the browser. Replays record their map and restore it; an unknown map ID is
refused everywhere.

```sh
scripts/fetch-robot-kit.sh                 # the pinned robot and weapons kits into target/robot-kit, target/weapon-kit
cargo run -p shooter-client --release      # native window, practice
scripts/build-web.sh                       # browser build (and the kit) into web/dist (needs a WebGPU browser)
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
mouse. Steer in the air with WASD.

**HUD.** The drawn HUD (`shooter-client`'s `overlay`) is the same natively and in the browser:
render-engine's screen-overlay quads in the robot kits' palette (pale grey on graphite, olive, an
orange accent), in the bottom corners clear of the reticle, laid out for 720 lines and scaled with
the window. Bottom right, the held weapon: its silhouette, name and mod, the energy a shot costs
(a second, for the laser and the flamethrower), whether it is ready, recharging (a bar filling to
its next shot), short of energy or lost, and above it a slot for every weapon the build carries
with its key (lost ones red and struck out); a switch flashes the panel and sweeps a highlight
across it. Bottom left, the root chassis' health and the energy pool, with the share one shot of
the held weapon takes marked in orange. render-engine's overlay draws solid rectangles only, so
the text is a 5×7 bitmap font and the weapons are pixel silhouettes, both drawn as quads in the
client. The rest (the parts damaged and lost, frags and place, the round clock, the equipped mods,
the carried module and its uses left, the kill and pickup feed, banners and the scoreboard) is
still text: the page's overlay in the browser and the window title natively.

Screenshots: [docs/captures/92](docs/captures/92).

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

The left mouse button fires. The mouse wheel steps through the weapons your robot carries and
still has (down or E for the next, up or Q for the previous), in the order below, wrapping round
and skipping the ones it does not carry or has lost; the blade stays on V, unless it is the last
weapon left, when it is held and the trigger swings it. One wheel notch is one step, whether the
wheel reports lines or pixels: a burst of events within 60 ms is one notch. 1 to 7 pick a weapon
directly. A fighter holds and fires only the weapons its build mounts (#92): the host refuses a
switch to any other, and the prediction follows the host.

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
rocket hurts you at half damage (a rocket jump costs every part near your feet up to 50, the legs most).

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
(`shooter_sim::robot`, Atlas D2.i70.1, D2.i89.1): one component tree assembled on universal
sockets, one per 20 cm surface cell (a 20 × 40 × 80 cm chassis has 28). Each part has a stable ID,
one parent socket and an exact orientation (binary angles, stored in one canonical form); a larger
part reserves its footprint of neighbouring cells on its parent's face, and no two parts may
claim one cell. The catalog has three chassis sizes, beams, hinge and rotor joints, a battery, a sensor, legs, wheels, tracks, an armor plate and the eight
shipped weapons. A build is valid with one chassis root, no cycles, each weapon at most once, at
most 32 parts and at most 110 build points; there is no power, heat or cooling budget. The root
chassis is the robot's vital part (Atlas D2.i89.1, #89): there is no separate control core, and a
build naming the retired core (part code 30) is rejected as an unknown part. Nothing requires locomotion: tanks, four-legged dogs,
humanoids, stationary turrets and grappling-only machines are all valid. Every fighter starts as
the standard build; a person's command may ask for another, which the host validates (an invalid
one rejects the command) and applies at that fighter's next spawn, never while it is alive.
Snapshots carry each fighter's build and the one it comes back as. The cell size, chassis sizes,
part values and budgets are provisional (Atlas Q2.i70.1, Q2.i70.2), the part keys await the
asset kits, and a build does not yet change movement, but it is the arsenal: a fighter spawns
holding its build's first weapon and fires only what the build mounts (#92). The `pyro` loadout
carries the laser and the flamethrower (#96); the tests' over-budget `ARMORY` build carries every
weapon. There is no builder screen yet.

**Energy.** Every robot has one shared energy pool (`shooter_sim::energy`, Atlas D2.i70.3,
D2.i70.4, D2.i89.1): the root chassis stores 100 and regenerates 24 a second (other chassis
sections nothing), and each battery (a
20 × 40 × 20 cm part, 5 build points, 20 kg) adds 60 stored energy and nothing else. A robot
spawns with its pool full. Every activation pays from it: a rifle bolt 2, a rail shot 30, a
rocket 20, a grapple hook 10, a blade swing 5, a shotgun blast 12, and the laser and the
flamethrower 0.6 and 0.5 every tick they burn (36 and 30 a second, more than the root
chassis regenerates). A request is granted whole or not at all: with too little energy the shot, swing or
hook simply does not happen (no cooldown is spent) and works again once regeneration has paid for
it; the charge never goes below zero. Within a tick a robot's requests go first come, first
served in a fixed order: a thruster jump, then the blade swing, then the gun (provisional, Atlas
Q2.i70.4). There is no heat or cooling. The host owns the pool; snapshots carry each robot's
charge, the client predicts its own, replays reproduce it, and the HUD's energy bar shows it. Values are provisional (Atlas Q2.i70.1). A lost battery (destroyed, or carried off
with a part it hung from) takes its capacity and its share of the charge with it, once; a
destroyed one also blasts its neighbours (below, **Part damage**). Motors draw from the same
pool (below).

**Part damage** (`shooter_sim::damage`, Atlas D2.i89.1, D2.i70.8, #56, #89). Every part of a
build has its own health (provisional, Atlas Q2.i70.1: chassis 270, 290 and 310 by size, so a front
rail kill still takes four hits, or three overcharged; beams 50 and 60, hinge and rotor 40,
battery 60, sensor 30, leg 70, wheel 50, track 90, armor plate
120, light weapons 40, heavy ones 60). The host places each fighter's parts as boxes where the
simulation lays out its build, standing on the capsule's feet and facing its look (the capsule
still moves it; the mechanical robots do not drive fighters yet, #73), and resolves every strike
onto them, deterministically, each part at most once a strike: a hitscan beam, a pellet or a
rocket meets the parts along its path (a shot through a gap flies on); the first one takes the
damage, unless it is an armor plate and the weapon penetrates, when the plate takes the rest and
a share goes on to the part behind it (rail gun 50 %, rifle and laser 20 %, nothing else;
provisional). A blast (a rocket's splash, a battery's) hurts every part within reach by its
falloff to that part; the blade and the flame strike the part nearest the attacker's eye.
Lag-compensated shots and swings place the parts at the centre and facing the shooter saw. A part
works fully until zero; then it is destroyed for the rest of that life and everything no longer
connected to the root chassis through the part tree falls away with it (breaking a beam drops it
and whatever hangs from it; another chassis section is ordinary structure). Destroying the root chassis
is the only kill; respawn restores the configured build whole. A lost
weapon part no longer fires or swings and cannot be selected (nor can a gun the build never
carried, #92); a destroyed battery sets off a small blast (40 at its
centre, 0.8 m; provisional, Atlas Q2.i70.5) through the same damage, and a battery it destroys
blasts in turn, once each; a robot with no connected leg, wheel or track cannot walk, jump or
slide but stays alive while its root chassis does. Losing some of its legs changes nothing yet: how the
surviving ones move it is the mechanical robots' (#73), and the capsule invents no fallback.
Snapshots carry every part's health (the lost parts follow from it), with a hit's part and each
detachment as events; the client's prediction takes the host's part state, replays reproduce it
and bots aim at their target's root chassis and switch off a lost gun.

**Mechanics** (`shooter_sim::mechanics`, Atlas D2.i70.5, the first half of #73). The simulation
can build any valid robot as a mechanical one in a dynamic world of its own
(`shooter_sim::physics::articulated`, on physics-engine's compound bodies and revolute joints
with motors): every part placed in space from its socket and exact orientation, rigidly joined
parts one compound body with each part's mass where it is mounted, and a separate body on a
motorised joint for what sits on a hinge (a servo within a quarter turn either way) or a rotor
(a continuous motor), for each wheel (a sphere: physics-engine has no cylinders), each of a
track's two road wheels and each leg (one self-contained module: a thigh on a hip servo and a
shin with a flat foot on a knee servo). Each tick a robot's actuators take one command each (a
rate, an angle, a leg's stride or idle) and ask the shared pool for their draw, first come,
first served in part-ID order (a flat cost a commanded tick, provisional); a refused motor is off
that tick. Mounting orientation is authoritative: a wheel turns about its own mount normal, so
the same command spins a car in place, wheels on its front and back strafe it, and lopsided mass
tips it over. Legs walk through a gait preset suggested from the attached legs (two: a biped
that steps quickly, four: a crawl, one leg at a time) and can be overridden; the presets only
command the hip and knee servos, so legs mounted wrong go nowhere and nothing teleports. Each
actuator reports its angle, rate, torque and whether it stalled (at its torque, far from its
command). Part poses and per-part ray hits follow the bodies, and the state encodes exactly
(`wire::encode_mechanics`, `encode_controls`): a world restored from it continues bit for bit.
The fighters in a match still move on the capsule controller: putting mechanical
robots into the match, its snapshots, prediction and replays, is #73's second half, with the
bindings (#74). Masses, torques, rates, draws and gait timings are provisional; a 12-robot scene
(66 joints) steps in about 0.16 ms a tick natively on physics-engine (0.20 ms on Rapier).

**Robot models** (`shooter_client::robots`, #54). Both clients draw every fighter as its validated
build from two asset-tooling kits: `shooter-combat-robot` (the chassis, beams, hinge, rotor,
sensor, battery, the armor plate and its damaged look, and the rigged leg with its walk clips) and
`shooter-weapons` (the eight weapons, rigged with their states and glow channels, and the four
mods). Each release is pinned by SHA-256 (`assets/robot-kit.pin.json`,
`assets/weapon-kit.pin.json`); `scripts/fetch-robot-kit.sh` fetches them into `target/robot-kit`
and `target/weapon-kit` for the native client (`$SHOOTER_ROBOT_KIT` and `$SHOOTER_WEAPON_KIT` name
others) and `scripts/build-web.sh` puts each kit's release tar in `web/dist`, which the browser
fetches in one request a kit and reads in memory. Every file is checked against the bundles' pins
and bounds (4 MiB a file, 8 MiB a load, 16 MiB and 30 s a tar in the browser), every kit key must
be a catalog part or mod, every catalog mod a weapons-kit entry and every catalog part a kit entry
or a declared placeholder; otherwise the load fails, the HUD says why and the fighters stay
boxes. Each part sits where the simulation places it (`mechanics::placements`), so loadouts look
different, and each mod sits on its weapon's anchor from the weapons kit's descriptor. Wheels and
tracks (asset-tooling#634) are not modelled yet: they draw as translucent purple boxes of their
cells and the HUD lists them. The robot stands on its capsule's feet and faces its look; its legs
rest when idle, walk the kit's clips when running (at most 2.5 cycles a second, so feet slide at
full speed), tuck while airborne or grappling and crouch in a slide. The weapons play the kit's
states from the match (`robots::weapon`): a shot's recoil and muzzle flash, the laser's and
flamethrower's loops while firing, the rail gun's recharge and the rocket pod's alternating tubes
and reload over their (mod-tuned) refire time, the grapple's launch (its hook flies to where the
host set it, the cable runs from the launcher) and retract, and the blade extending when held or
swung, retracting otherwise, and the kit's quick-melee swing about its wrist (the part it is
mounted on stays rigid); between shots each weapon's charge light shows the robot's energy pool. Shots, beams,
flames and pellets are drawn from the weapon's muzzle frame (the long barrel's when it has one)
rather than a guessed offset. The named loadouts mount every weapon on the root chassis' front with
its muzzle along the aim (#91), so shots leave muzzles that face it; the simulation decides where
shots start and what they hit. A dead robot topples over until it
respawns. Strafing (no sideways clip) is a missing animation: a magenta marker floats over the
robot and the HUD names it. Hinges and rotors would turn about the simulation's anchor (the centre
of their top face, where the kit pivots at the cube's centre), but nothing in a match turns them
yet. Parts a robot has lost (from the host's per-part state in each snapshot) fall from where they
came off to the floor with their weapons and mods and lie there until it respawns, while the rest
of it keeps animating; damaged parts draw their damaged look where the kit has one (the armor
plate). `cargo run -p shooter-client --release --example robot_gallery -- <dir>` renders the
loadouts, the mechanics fixtures, every pose, the weapons' states and the mods offscreen.

**Weapon mods.** A weapon part may carry one mod, which costs build points and mass and changes
how that weapon fires (on the host, in the client's prediction and in lag-compensated hits
alike); a weapon without one fires as shipped. Four provisional mods: the *rapid cycler* (rifle,
rail gun, shotgun: 65% refire time, 75% damage, so each second of fire costs half again the
energy), the *overcharger* (same guns: 125% damage, 130% refire time, 150% energy a shot), the
*long barrel* (rifle, rail gun, laser: 150% range, 110% refire time, and its weapon becomes a
two-cell module) and the *efficiency coil* (rifle, rail gun, rockets, shotgun: 60% energy a shot,
125% refire time). The heat sink is gone with heat; a build naming it is rejected as an unknown
mod. A mod that does not fit its weapon rejects the build. Pick a named loadout
with `--loadout <name>` natively or `&loadout=<name>` in the browser (the join overlay's loadout
field): `standard`, `heavy` (with a battery: 160 energy), `skirmisher` (rapid-cycler rifle,
efficiency-coiled shotgun), `marksman` (long-barrel rifle, overcharged rail gun: three hits a
front kill instead of four, on the root chassis) or `pyro` (long-barrel laser reaching 45 m,
flamethrower, blade and grapple on the small chassis: 180 kg, 79 of 110 points, the root
chassis' 100 energy; it spawns holding the grapple, the first of its weapons in the rotation),
each a two-legged component tree with a sensor
on top and its four weapons on the chassis' front, facing the aim (no arms since #91), so a front
shot at the upper half of the chassis meets a weapon first. Practice
starts as it; online the host applies it at your next spawn. The weapon panel shows the held
weapon's mod and the status line every equipped one.

**Module pickups.** A few powerful modules lie on pads at contested points of a map, placed in
the map's data (`shooter_sim::pickups`, `Arena::module_pads`). Any robot that walks (or jumps
low) onto a pad takes its module, whatever its build; the module works on top of the build and
its mods and never changes the build. Two provisional modules: the *rail capacitor* (rail gun:
150% damage, 75% refire time and 150% energy a shot, stacking on a rail mod; 5 boosted shots)
and *thrusters* (every jump, the air jump too, leaves at 9 m/s instead of 6: a 2.25 m jump onto
the platform; 6 boosted jumps, each costing 15 energy; without the energy a jump is a plain one
and keeps its use). On the greybox the capacitor lies on the platform's south-east corner, the perch
that sees the whole hall, and the thrusters in the open south-west quarter under its sightline;
on the greybox yard the capacitor lies on the walkway's middle and thrusters on each platform.
A module lasts until its uses are spent, its holder dies or the round ends (owner decision on
#62); nothing drops. A robot carries one module, and taking another replaces it. A pad is back
30 s after it was taken; every pad is ready at the start of a round. When several robots reach a
ready pad in the same tick the nearest (feet to pad centre) takes it, then the lowest slot. The
host owns all of it; snapshots carry the pads and each fighter's module, the HUD's status line
shows the carried module and its uses left, the feed announces pickups, and the pads are drawn
as tinted plates with the module floating over them while it is there. Values are provisional
(Atlas Q2.i46.3); the boosts are paid from the robot's shared energy pool (owner option B on #62).

Bots are deliberately beatable and fully deterministic (seeded): they walk between spawn points
and the map's waypoints (wall gaps, ramp ends, the walkway, the platforms), taking only straight
walks no wall or ramp side blocks,
spot the nearest fighter in sight, react after 0.35 s, turn at most 360°/s and aim with a few
degrees of wandering error, strafing as they fire; the flamethrower inside 5 m, the rifle (even
slots) or the shotgun (odd slots) up close, rockets or the laser at mid range, the rail gun far off,
and inside 3 m they rush in and swing the blade.

The movement and the weapons are checked against Rapier on a scripted tour of the greybox, the
greybox yard's walkway (up its ramp, along the deck, off its edge, and its jump pad onto it) and on
scripted moves (sprint, slide, double jump, a thruster-module jump, jump pad, ramp slides, a thin wall, a step, a rocket
jump, a grapple onto a ledge, a shooting range, a close range for the shotgun, laser and flamethrower):
`cargo test -p shooter-sim --features physics-ab --test physics_ab`, which also runs the
mechanics fixtures on both (rolling, strafing, the stall, both turrets, tipping, per-part hits,
the robo-dog walking). `cargo test -p shooter-sim --test mechanics` checks those fixtures, the
biped and quadruped presets, badly mounted legs, motor energy draw, and that a restored world
and a replay continue bit for bit.
`cargo test -p shooter-sim --test robot` checks the sockets and canonical orientations, valid
builds of every form (the loadouts, a mini-tank, a robo-dog, a humanoid, stationary and
grappling-only machines), every rejection (the retired core code, cycles, double parents, overlaps, the part
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
