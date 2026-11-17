# shooter

A fast third-person sci-fi arena shooter for native and browser, built on the moenarch
foundation crates. It is experimental. Its second purpose is to give those crates a new,
demanding consumer: every gap or regression it finds is filed on the crate that owns it.

Product intent lives in the [Decision Atlas](docs/decisions/atlas.md). The owner has set a beta
direction (Atlas D1.i46.1, D2.i46.\*, tracker #45): a third-person arena shooter with modular
robots, about 12 slots and three arenas. The third-person camera, the robot build schema and
the 12 slots are in, the robot builder opens before the match and between lives, the fighters
are drawn as their robot builds from asset-tooling's robot,
weapons and locomotion kits, and the medium and small arenas are built from its arena kit; the
rest is not implemented yet. Everything below describes what works today: a third-person,
12-slot deathmatch on the 76 m greybox yard by default, the kit-built 56 m assembly works (the
medium arena) and 32 m proving cell (the small arena) on request, and the 40 m greybox hall as
the test fixture, every fighter a mechanical robot (its compound bodies on motorised joints)
carried by the fast-movement rules.

## What a player does

Open the native client or the browser build on Pages and join a hosted deathmatch.
Bots fill the empty slots. Move fast: sprint, slide, step up low ledges, double jump, jump pads,
rocket jumps, the grapple hook and air control. Fight with energy guns, a grapple hook and melee
weapons.

## Foundations

| Need | Crate |
| --- | --- |
| Collision, the robots' compound bodies and motorised joints, rays, projectiles | physics-engine, used directly through `shooter_sim::physics` (Rapier A/B oracle in CI) |
| Grapple rope on a robot | shooter-sim stand-in (a velocity rule, `grapple::Hook::pull`) |
| Lighting, particles (muzzle flashes, trims, impacts) | render-engine |
| Positional gunfire and impacts, native and Web Audio | game-audio |
| Mouse, keyboard, gamepad, pointer lock | input-bindings |
| Authoritative matches, lag-compensated hits, WebTransport | game-server |
| Arena kit, weapons, character | asset-tooling (beta kits: moritzbrantner/asset-tooling#593, #594, #595) |
| Frame and tick traces | runtime-profiler |

## Play

The current slice is a deathmatch in an arena (by default the greybox yard, below) with ramps,
a raised level and jump pads onto it,
six guns (energy rifle, rail gun, rocket launcher, shotgun, laser, flamethrower), the grapple hook and an energy blade. Twelve slots by default; bots fill every slot no
person holds. A kill scores a frag; dying by your own rocket, your own battery's blast or a fall costs one. The dead respawn after 2 s (unless they press B, Edit Robot, below) at the spawn point
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
count it is laid out for. The beta's three arenas, the medium, the small and the large one, are
in, built from asset-tooling's arena kit; two interim greybox maps stay:

- `assembly-works`, the medium arena (shooter#52; 4 to 12 players, 14 spawn points): a multilevel
  robotic assembly factory repurposed for combat (the owner's choice of setting), a 56 m hall
  inside 10 m walls, point-symmetric. Its ground is the central assembly floor, broken up by 2 m
  barriers, 1.2 m low cover, a parts container, 4 m pillars and generators, with a 2 m assembly
  cradle under the middle; behind a 4 m bay wall at either end lies a work bay, entered through a
  doorway or round the wall's open end, with containers, a blast screen and crates. At 3 m a
  mezzanine runs along each bay wall (the wall stands 1 m over its deck as a parapet), joined
  across the middle by a gantry bridge with staggered low cover and the rail capacitor at its
  centre, the contested high ground: a 4 m wide, 12 m ramp climbs each mezzanine, a jump pad
  beside the bridge throws a fighter onto it, and the grapple reaches it from under it. At 5 m a
  crane tower stands in the corner of each bay with thrusters on top, reached by four 1 m work
  platforms (a double jump onto the 2 m one, another onto the 4 m one, a jump onto the tower),
  by a jump pad at its foot that throws 15 m/s up, or by the grapple on its 3 m mast. 4 m deep
  charging bays in the side walls, the bay walls, the tower and the floor's 4 m pillars break
  the long lines; no spawn point is in sight of another, and none is a dead end. Its low cover,
  barriers and crates are destruction candidates that break (shooter#60, below)
  and four repair anchors (under the mezzanines and in the bays) are data for shooter#61. Plan,
  views and frames: [docs/captures/52](docs/captures/52).
- `proving-cell`, the small close-quarters arena (shooter#53; 2 to 6 players, 12 spawn points so
  the default 12 slots still fit): a purpose-built combat test cell (the owner's choice of
  setting), a 32 m square inside 10 m walls, point-symmetric, from the same kit as the works. In
  the middle stands the 2 m test stage, a 6 m dais with the rail capacitor at its centre and
  staggered low cover, climbed by an 8 m ramp from the north and from the south (or a double
  jump; a 2 m container west of it meets its corner). Along the north and the south wall runs a
  3 m observation gallery, 4 m deep, with low cover at its edge and thrusters on it, reached by
  a 12 m ramp along the side wall, a jump pad in front of it (12 m/s up, 6 m/s toward it) or the
  grapple (hooked on the wall above it, from the stage); under it a 2.6 m high passage runs from
  behind its ramp to a control booth, a 7 m room in the corner with a doorway onto the floor and
  one into the passage, each behind a 2 m blast screen. A 4 m deep test bay in each side wall,
  a barrier and crates break up the floor. Nothing is far and fights are close; the booths have two
  doorways and the passages open along their length, so no fight is forced down a one-way
  corridor; no spawn point is in sight of another. Its low cover, open-floor
  barriers and crates are destruction candidates for shooter#60 (the blast screens stay, the
  booths' spawn points rely on them) and a repair anchor in each booth is data for shooter#61.
  Views and frames: [docs/captures/53](docs/captures/53).
- `drydock`, the large long-sightline arena (shooter#55; 8 to 12 players, 14 spawn points): a
  repurposed spaceship shipyard (the owner's choice, Atlas D2.i55.1), an 80 m square inside 10 m
  walls, point-symmetric, from the same kit. Two docked spacecraft lie head to tail either side
  of a 32 m central industrial corridor; there are no ship interiors. Each hull, 12 m wide and
  56 m long, is built from the kit's hull modules: 2 m sides, a 45° shoulder up to a walkable
  4 m foredeck, 8 m wide, with staggered 80 cm hull ribs as low cover and a sensor mast at the
  bow, and a raised 6 m stern deck, 16 m long, with a rib, a mast and the thrusters, climbed from
  the foredeck by an 8 m ramp. Along each hull's corridor side runs a 4 m dock gantry with a
  16 m ramp down from either end and two gangways over the shoulder onto the foredeck; behind
  each hull a 12 m outer lane along the perimeter has containers, a barrier and a jump pad that
  throws a fighter over the shoulder onto the foredeck (15 m/s up, 9 m/s toward it), and a lane
  round each bow and stern joins it to the corridor. The corridor's floor has containers,
  barriers, low cover, crates and an 8 m crane pylon in the middle, with the rail capacitor at
  its foot. Every hull top has several routes up (the gantry's two ramps for wheels and tracks,
  the gangways, the pad, the grapple on a mast); each stern deck is in sight of the other ship's
  hull and of the gantries, and the pylon cuts the line between the two stern decks, so no hull
  top is an unanswered perch. All spawn points are on the ground and none is in sight of another.
  Its low cover, floor barriers and crates are destruction candidates for shooter#60 and four
  repair anchors (under the gantries and in the outer lanes) are data for shooter#61. Views and
  frames: [docs/captures/55](docs/captures/55).
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

**Arena kit.** A kit-built map lists the arena kit's module placements (a module, where its
origin sits on whole centimetres, a quarter-turn yaw, intact or broken; `shooter_sim::arena_kit`),
and the simulation derives the map's collision pieces from the modules' collision proxies:
blocks for walkable and blocked boxes, ramps turned with their yaw, nothing for the floor plates
and the jump pad's footprint, neighbouring boxes that meet over a whole face fused into one piece
(a wall of fifteen modules is one box, with no seams to snag on), destruction candidates kept as
pieces of their own. The proxies come from the pinned kit's descriptor through a checked-in table
(`scripts/gen-arena-kit.py` writes it, a client test checks it against the pinned kit), so the
simulation reads no files. Both clients draw each placement's kit mesh where the simulation puts
it and lay the floor with the kit's 2 m plates; until the kit is read a kit-built map waits, and
if it cannot be read the map is drawn as boxes with a note in the HUD. The greybox maps are
unchanged. The small and the large arena use the same path.
`cargo test -p shooter-client --test arena` checks the table against the pinned kit, the kit's
decoding and that every placed module is drawn inside the box its collision comes from;
`scripts/gen-arena-kit.py` rewrites the table after a pin bump; `cargo run -p shooter-client
--release --example arena_views -- assembly-works DIR` (or `proving-cell`, `drydock`) renders a map's overview and route views
offscreen.

The recommended capacity never changes the slot count (12 unless the host sets another per
match, at most one per spawn point). The host picks each match's map
(`SHOOTER_MATCH_IDS=dm=greybox-yard`, [docs/hosting.md](docs/hosting.md)) and its snapshots name it,
so a joining client builds and predicts on that map; practice takes `--map <id>` natively or
`?map=<id>` in the browser. Replays record their map and restore it; an unknown map ID is
refused everywhere.

**Arena destruction** (shooter#60). On a kit-built map the placements its map marks as
destruction candidates (low cover, barriers, crates; doors once a map places one) are damageable
placeables (`shooter_sim::scenery`), each with a stable ID, its placement's index in the map.
The host alone damages them, through the hit queries that already hurt robots: a hitscan ray or
a pellet that stops on one, the flame burning on one, a blade swing that meets one instead of a
robot, a rocket's blast by the point of its pieces nearest the blast (the same falloff as on
robot parts). At zero health it breaks: its placement turns to the kit's broken variant and its
collision to that variant's proxies, at the end of the tick that broke it, so from the next
tick on the physics world, the robots' world, every ray, projectile and flame, hooks (one set on
it lets go) and the bots' sight and walks all meet the broken pieces. Only the candidates break:
floors, decks, platforms, ramps, walls and the blast screens never do, and a candidate something
else rests on (the lower crate of a stack) carries it and never breaks either. Each candidate
keeps a run of body IDs of its own, so breaking one moves no other body. A broken placeable stays
broken through deaths and respawns until the round restarts, which brings every one back whole.
Health is provisional (Atlas Q2.i60.1): low cover 150, a barrier 250, a crate 60, a door 200.
The snapshot carries the damaged placeables' health by ID (wire 17), so replays verify it tick
for tick; a predicting client stands its arena as the host's last snapshot has it before every
predicted tick, so what its own predicted shots do to the arena lasts one tick at most, and both clients draw each placeable as
the kit's intact or broken mesh from the snapshot alone (no damage rule of their own, no rubble
or placeholder art; the box fallback for a kit that cannot be read stays whole).
`cargo test -p shooter-sim --test destruction` covers hitscan and blast destruction, overlapping
cover, a shot and a jump blocked before and open after, restoration, the wire, a predictor and
replays; `physics_ab` checks broken collision on the Rapier oracle;
`cargo run -p shooter-client --release --example arena_views -- MAP DIR broken` draws a map with
every placeable broken ([docs/captures/60](docs/captures/60)).

```sh
scripts/fetch-robot-kit.sh                 # the pinned robot, weapons, locomotion and arena kits into target/*-kit
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
one rejects the command) and applies at that fighter's next spawn, never while it is alive, unless
the robot has not played yet (no input applied since its person joined: the builder before the
match), when it is that build at once.
Snapshots carry each fighter's build and the one it comes back as. The cell size, chassis sizes,
part values and budgets are provisional (Atlas Q2.i70.1, Q2.i70.2), the part keys await the
asset kits, and a build changes movement only through its body (its shape collides, its locomotion
sets its share of the speed), but it is the arsenal: a fighter spawns
holding its build's first weapon in the rotation that is not the grapple (the grapple, then the
blade, only when it carries nothing else; #96) and fires only what the build mounts (#92). The
`pyro` loadout
carries the laser and the flamethrower (#96); the tests' over-budget `ARMORY` build carries every
weapon. You build your robot in the builder (below, **Robot builder**).

**Robot builder** (`shooter_client::builder`, Atlas D2.i70.11, #57). Both clients open on a
blueprint workbench before the match: nothing plays (and online nothing connects) until you press
Ready. The robot stands in the middle, drawn from the kits as in the match, with its sockets
marked; the part catalogue (every part and weapon with its build points) and the five named
loadouts to start from are on the left, the component inspector on the right (the selected part,
or the part in hand: cells, mass, health, points, energy, motor, the energy a shot costs, its
turn in degrees and its mod), and the budget along the bottom: build points and parts against
their limits, mass, energy and regeneration, and whether the build is valid (or the rule it
breaks). Drag a part from the catalogue onto a socket, or click it and click sockets to place
copies; the sockets it fits light up, and hovering one previews the part there with its real
geometry, its box and the cells its footprint takes. Click a part to select it. R, T and F turn
the selected part (or the one in hand) an eighth of a turn about its socket's normal, pitch and
roll axes (Shift back, Ctrl a degree instead, exact either way); M cycles a weapon's mods, X or
Delete removes a part and what hangs from it (never the root chassis: "new frame" starts over
from a chassis in hand), Escape drops or deselects, right drag or the arrow keys turn the view,
the wheel zooms. A further chassis mounts on a socket like any part, so frames of several
cuboids build up. Every edit goes through the simulation's one validation path: an edit that
breaks a structural rule (a taken cell, a mod that does not fit, a weapon twice, the part
ceiling) is refused with the reason, and only the build points may run over while you work;
Ready takes only a valid build, which the host validates again. Nothing warns about odd builds.
When your robot is destroyed, the respawn counts down as before, with "[B] Edit Robot" on
screen: B opens the builder on the build you come back as and holds the respawn, at no time
limit, even across a round's start, until you press Ready and the host shows the build
configured (an input flag, `Intent::edit`; a lost connection lets go of it). While alive there
is no rebuilding. In the browser the page hides its own overlays while the builder is open. Not
in the builder yet: actuator bindings and action groups (#74, Atlas D2.i70.7), typed exact angles
and a testing chamber (post-beta).
`cargo run -p shooter-client --release --example builder_views -- <dir>` renders the bench
(before the match, a part being placed, a selected weapon, an over-budget build, a build between
lives) offscreen; [captures](docs/captures/57-builder/).

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
120, light weapons 40, heavy ones 60). The host places each fighter's parts as boxes where its
mechanical robot's bodies carry them (below, **Mechanical robots in the match**), and resolves every strike
onto them, deterministically, each part at most once a strike: a hitscan beam, a pellet or a
rocket meets the parts along its path (a shot through a gap flies on); the first one takes the
damage, unless it is an armor plate and the weapon penetrates, when the plate takes the rest and
a share goes on to the part behind it (rail gun 50 %, rifle and laser 20 %, nothing else;
provisional). A blast (a rocket's splash, a battery's) hurts every part within reach by its
falloff to that part; the blade and the flame strike the part nearest the attacker's eye.
Lag-compensated shots and swings meet the parts where the shooter saw them. A part
works fully until zero; then it is destroyed for the rest of that life and everything no longer
connected to the root chassis through the part tree falls away with it (breaking a beam drops it
and whatever hangs from it; another chassis section is ordinary structure). Destroying the root chassis
is the only kill; respawn restores the configured build whole. A lost
weapon part no longer fires or swings and cannot be selected (nor can a gun the build never
carried, #92); a destroyed battery sets off a small blast (40 at its
centre, 0.8 m; provisional, Atlas Q2.i70.5) through the same damage, and a battery it destroys
blasts in turn, once each; a robot with no connected leg, wheel or track cannot walk, jump or
slide but stays alive while its root chassis does. A lost leg, wheel or track comes off the
robot's bodies: it no longer holds it up (a robot whose legs are shot off drops onto its chassis
and lies there), and the robot walks at the share of its locomotion it still has (half speed on
one of two legs).
Snapshots carry every part's health (the lost parts follow from it), with a hit's part and each
detachment as events; the client's prediction takes the host's part state, replays reproduce it
and bots aim at their target's root chassis and switch off a lost gun.

**Mechanics** (`shooter_sim::mechanics`, Atlas D2.i70.5, #73). The simulation
builds any valid robot as a mechanical one in a dynamic world of its own
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
Masses, torques, rates, draws and gait timings are provisional; a 12-robot scene
(66 joints) steps in about 0.18 ms a tick natively on physics-engine (0.21 ms on Rapier).

**Mechanical robots in the match** (#73). Every living fighter is its robot in that world, in its
slot (its bodies' ids follow from the slot; an open slot has none): no capsule. A spawn adds it
standing at its spawn point, a death or a slot changing hands removes it, and a part shot off
leaves its bodies (it no longer weighs, touches or drives anything; what stays keeps its pose and
motion); each change rebuilds the world from its exact state, so the host, a predicting client and
a replay hold the same world. The fast-movement rules (sprint, slide, double jump, jump pads, air
control, the grapple's rope) give the velocity a movement drive sets on the whole robot's centre
of mass, which also keeps it upright and turns it to face the aim (at most 16 rad/s); its legs
stride on the suggested gait and its wheels and tracks roll at the rate its speed gives them, and
every motor draws from the pool after the tick's jumps, swings and shots (two legs: 8 a second
against the chassis' 24, so weapons recharge a third slower; provisional). The simplest mapping
until the bindings (#74), with what it costs: the robot's bodies slide frictionless where the
drive takes them (its feet do not drag it), its legs bend only within the gaits' range (±0.4 rad
at the hip, 0.8 at the knee) so bumps cannot fold them, it walks only with a leg, wheel or track
on the ground, it hops onto ledges up to 35 cm its feet catch (the capsule's step-up), and a
landing does not bounce it back up. Its parts' boxes are where its bodies carry them; it collides
as its bodies (its guns reach over a 1.2 m cover and stop it a little further from a wall than
the capsule did; two of the capsule-tuned routes, the works' 4 m step from the 2 m one and the
thruster jump onto a 2 m ledge, no longer land for the standard robot). Snapshots carry each
living robot's exact state (every body's motion, its lost parts, its gait clock; wire version
18), from which a client resumes its prediction: bit for bit like the host, whole or one-legged
(`cargo test -p shooter-sim --test match_robots`). Both clients draw each robot where those
bodies are (below).

**Robot models** (`shooter_client::robots`, #54). Both clients draw every fighter as its validated
build from three asset-tooling kits: `shooter-combat-robot` (the chassis, beams, hinge, rotor,
sensor, battery, the armor plate and its damaged look, and the rigged leg with its walk clips),
`shooter-weapons` (the eight weapons, rigged with their states and glow channels, and the four
mods) and `shooter-robot-locomotion` (the wheel and the track, each with its `roll` clip and a
damaged look). Each release is pinned by SHA-256 (`assets/robot-kit.pin.json`,
`assets/weapon-kit.pin.json`, `assets/locomotion-kit.pin.json`); `scripts/fetch-robot-kit.sh`
fetches them into `target/robot-kit`, `target/weapon-kit` and `target/locomotion-kit` for the
native client (`$SHOOTER_ROBOT_KIT`, `$SHOOTER_WEAPON_KIT` and `$SHOOTER_LOCOMOTION_KIT` name
others) and `scripts/build-web.sh` puts each kit's release tar in `web/dist`, which the browser
fetches in one request a kit and reads in memory. Every file is checked against the bundles' pins
and bounds (4 MiB a file, 8 MiB a load, 16 MiB and 30 s a tar in the browser), every kit key must
be a catalog part or mod of that kit and every catalog part and mod must be in a kit; otherwise
the load fails, the HUD says why and the fighters stay boxes. Each part sits where the simulation
places it (`mechanics::placements`), so loadouts look different, and each mod sits on its
weapon's anchor from the weapons kit's descriptor. Wheels and tracks turn about the axles the
locomotion kit's descriptor (`socket-grid-kit-descriptor-v3`) puts exactly where the simulation's
mechanics turn them (the mount normal; the wheel's 0.18 m tyre, the track's two 0.1 m road
wheels), by their `roll` clip at the joint's angle (the belt moves with the wheels).

Every living robot is drawn where the simulation has its mechanical robot's bodies
(`robots::frames`): the local one from the prediction, the others between the two snapshots
around the drawing clock (each body's frame blended), moved with the fighter's drawn centre. Each
part sits on the body that carries it, so the robot stands, leans, and with its legs shot off lies
on its chassis as the simulation has it; a leg's hip and knee take the angles its thigh and shin
have turned (its ankle stays at rest: the simulation's foot is fixed to its shin), so the legs
stride with the simulation's gait; a wheel or a track rolls by the angle its body (a track's first
road wheel) has turned about its axle; a hinge's or rotor's moving piece, and everything on it,
turns as the body it carries has turned. The kit's leg clips are not played. Without a mechanical
robot (the builder's bench) a robot stands at rest on its feet. The weapons play the kit's
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
respawns (procedural: a dead fighter has no mechanical robot). Strafing is a missing animation
(neither the kit nor the simulation's gait has a sideways step: the legs stride while the robot
moves sideways): a magenta marker floats over the robot and the HUD names it. Parts a robot has
lost (from the host's per-part state in each snapshot) fall from where they came off to the floor
with their weapons and mods and lie there until it respawns, while the rest of it stays on its
bodies; damaged parts draw their damaged look where the kit has one (the armor plate, the wheel
and the track). `cargo run -p shooter-client --release --example robot_gallery -- <dir>`
renders the loadouts and mechanics fixtures as the simulation spawns them (the turret's rotor and
hinge driven), the standard build standing, walking, walking on one leg and lying on its chassis
without legs (each alone in the mechanics world, without the match's drive that holds a fighter
upright), the weapons' states, the mods, and the wheeled fixtures and the gallery's own mini-tank
as the simulation drove them (in the simulation its hull rests on the floor and its tracks spin
without moving it, #58) with damaged and lost wheels and tracks offscreen
([captures](docs/captures/54-locomotion/), [mechanics poses](docs/captures/54/)).

**Weapon mods.** A weapon part may carry one mod, which costs build points and mass and changes
how that weapon fires (on the host, in the client's prediction and in lag-compensated hits
alike); a weapon without one fires as shipped. Four provisional mods: the *rapid cycler* (rifle,
rail gun, shotgun: 65% refire time, 75% damage, so each second of fire costs half again the
energy), the *overcharger* (same guns: 125% damage, 130% refire time, 150% energy a shot), the
*long barrel* (rifle, rail gun, laser: 150% range, 110% refire time, and its weapon becomes a
two-cell module) and the *efficiency coil* (rifle, rail gun, rockets, shotgun: 60% energy a shot,
125% refire time). The heat sink is gone with heat; a build naming it is rejected as an unknown
mod. A mod that does not fit its weapon rejects the build. Start the builder from a named loadout
with `--loadout <name>` natively or `&loadout=<name>` in the browser (the join overlay's loadout
field): `standard`, `heavy` (with a battery: 160 energy), `skirmisher` (rapid-cycler rifle,
efficiency-coiled shotgun), `marksman` (long-barrel rifle, overcharged rail gun: three hits a
front kill instead of four, on the root chassis) or `pyro` (long-barrel laser reaching 45 m,
flamethrower, blade and grapple on the small chassis: 180 kg, 79 of 110 points, the root
chassis' 100 energy; it spawns holding the laser),
each a two-legged component tree with a sensor
on top and its four weapons on the chassis' front, facing the aim (no arms since #91), so a front
shot at the upper half of the chassis meets a weapon first. The builder's "start from" list loads
them too. The weapon panel shows the held
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
on the greybox yard the capacitor lies on the walkway's middle and thrusters on each platform;
on the assembly works the capacitor lies on the gantry bridge's middle and thrusters on each crane
tower; on the proving cell the capacitor lies on the test stage's middle and thrusters on each
gallery.
A module lasts until its uses are spent, its holder dies or the round ends (owner decision on
#62). A robot that dies carrying one drops it where it fell, with the uses it had left (a robot
falling out of the arena takes it along); any living robot, whatever its build, takes the drop
by walking onto it, the dead holder's effect is gone, and an untaken drop is gone after 30 s or
at the round's end. A robot carries one module, and taking another replaces it. A pad is back
30 s after it was taken, whatever happens to its module; every pad is ready at the start of a
round. When several robots reach a ready pad or a drop in the same tick the nearest (feet to its
centre) takes it, then the lowest slot (pads first, then drops, oldest first). The host owns all
of it; snapshots carry the pads, the drops and each fighter's module (wire version 18), clients
predict taking a drop, the HUD's status line shows the carried module and its uses left, the
feed announces pickups, drops and who took a drop with how many uses, and the pads are drawn as
tinted plates with the module floating over them while it is there, a drop as the same module
floating where it lies. Values are provisional (Atlas Q2.i46.3); the boosts are paid from the
robot's shared energy pool (owner option B on #62).

Bots are deliberately beatable and fully deterministic (seeded): they walk between spawn points
and the map's waypoints (wall gaps, doorways, ramp ends, the decks, the platforms), taking only straight
walks no wall or ramp side blocks (pressed against a wall, with no such walk, the nearest point),
spot the nearest fighter in sight, react after 0.35 s, turn at most 360°/s and aim with a few
degrees of wandering error, strafing as they fire; the flamethrower inside 5 m, the rifle (even
slots) or the shotgun (odd slots) up close, rockets or the laser at mid range, the rail gun far off,
and inside 3 m they rush in and swing the blade.

The movement is scripted on physics-engine on a tour of the greybox, the greybox yard's walkway
(up its ramp, along the deck, off its edge, and its jump pad onto it), the assembly works' routes
(up its ramp, along the mezzanine and the bridge and off its edge; into its low cover, over it
and into a barrier; its pad, its work platforms and the grapple on the crane mast onto the crane
tower), the proving cell's (up its gallery ramp, along the gallery and off its edge; up the stage
and down its other ramp; its pad and a grapple from the stage onto the gallery) and on scripted
moves (sprint, slide, double jump, a thruster-module jump, jump pad, ramp slides, a thin wall, a
step, a rocket jump, a grapple onto a ledge), each replaying exactly on both backends
(`tests/tour.rs`, `tests/moves.rs`, `tests/maps.rs`). The Rapier A/B
(`cargo test -p shooter-sim --features physics-ab --test physics_ab`) compares the match's robots
sprinting, sliding and stopping on flat ground, whole and on one leg, the shooting range and the
close range for the shotgun, laser and flamethrower, and the mechanics fixtures (rolling,
strafing, the stall, both turrets, tipping, per-part hits, the robo-dog walking); over ledges,
steps and pads the robots' legs catching a lip, and Rapier's servo spring under the heavier
builds, part the two backends' paths. A match's twelve robots step in about 0.45 ms a session
tick natively (`--release -- --ignored twelve`). `cargo test -p shooter-sim --test mechanics` checks those fixtures, the
biped and quadruped presets, badly mounted legs, motor energy draw, and that a restored world
and a replay continue bit for bit.
`cargo test -p shooter-sim --test robot` checks the sockets and canonical orientations, valid
builds of every form (the loadouts, a mini-tank, a robo-dog, a humanoid, stationary and
grappling-only machines), every rejection (the retired core code, cycles, double parents, overlaps, the part
ceiling, mods), the budgets at their limits, tampered builds on the wire, modded fire and builds
through a replayed match; the A/B also runs the shooting and close ranges as the modded loadouts.
`cargo test -p shooter-sim --test pickups` checks contested pads and drops, pad respawn, modules lasting a
life until spent or death, the drop at death with its uses left and its pickup by another build,
the drops' lifetime, round ends and map starts, the rail capacitor's lag-compensated hits and
frags, thrusters, robots of different builds, a client predicting a drop's pickup and the wire.
`cargo test -p shooter-sim --test deathmatch` checks bot matches (and that they replay exactly),
lag compensation of shots and swings, that the reticle's aim ends where the hosted beam does, blade kills, scoring and rounds, input backlog trimming, and a hosted grapple carried in snapshots and predicted; `cargo test -p shooter-client --test online` plays two
clients through a real game-server host on loopback, resumes a severed connection in its slot and
takes the builder's builds to the host before the match and between lives; the Edit Robot hold
(after a death, at a round's start, after a lost connection) and the build readied before playing
are checked in `deathmatch` too.

When a crate can't do something yet, the shooter files an issue on that crate and uses Rapier
(or the off-the-shelf equivalent) for that piece only, until the crate catches up.
