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
rocket jumps, the grapple hook (a module your robot mounts) and air control. Fight with energy
guns and melee weapons.

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
six guns (energy rifle, rail gun, rocket launcher, shotgun, laser, flamethrower), an energy blade and the grapple hook, a mounted module on its own key. Twelve slots by default; bots fill every slot no
person holds. A kill scores a frag; dying by your own rocket, your own battery's blast or a fall costs one. The dead respawn after 2 s (unless they press B, Edit Robot, below) at the spawn point
farthest from everyone else. A round ends at 15 frags or after 5 minutes, the scores show for 8 s
and a fresh round starts. A fighter is its robot build with per-part health (below, **Part
damage**): destroying its root chassis is the kill. Each fighter is drawn as its robot build (below, **Robot
models**), its weapons and mods from asset-tooling's weapons kit; the sounds are placeholders
until asset-tooling delivers them.

**Practice offline.** The native client without arguments, and the browser build (Pages), run the
same simulation in-process: you in slot 0 against eleven bots, no server needed.

**On a phone or tablet.** The Pages build plays on a touch screen with a WebGPU browser (current
Chrome on Android, Safari on iOS 26), best in landscape: the page shows touch controls instead of
the keyboard help (`?touch=1` forces them, `?touch=0` keeps them off). The left thumb drags a move
stick wherever it lands, dragging the rest of the screen (or Fire and Hook while held) looks, and
buttons stand in for fire, jump, sprint (a toggle), slide, the energy blade, the grapple, the
weapon cycle, the scores, field repair and Edit Robot; ❚❚ pauses. The page keeps what they hold
in `window.shooterTouch` and the client presses the same keys and mouse buttons a keyboard and
mouse would (`shooter-client`'s `touch`), so layouts, the host's checks and prediction see nothing
new. In the builder a finger is the left mouse button (tap Ready to start).

**Without WebGPU.** The browser build draws with render-engine on WebGPU where the browser has
it, and otherwise with the client's lite renderer on WebGL 2 (`shooter_client::gfx::lite`;
render-engine has no WebGL 2 backend). The lite renderer draws the same arenas, robots, effects
and HUD with the sun, ambient and point lights, but without shadows, normal maps or MSAA.
`?renderer=lite` forces it in the browser, `SHOOTER_RENDERER=lite` natively (with
`WGPU_BACKEND=gl` it runs on OpenGL under WebGL 2's limits, as in the browser). In the browser
it scales the 3D scene's resolution to the frame rate (down to 0.3 of the window; the HUD is
drawn at full size, or at half size once the scene is at half or below), so WebGL 2 drawn in
software without a GPU stays close to 60 frames a second; with a GPU it stays at full size.
`?scale=0.5` fixes the scene's scale (`SHOOTER_RENDER_SCALE` natively).

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
scripts/build-web.sh                       # browser build (and the kit) into web/dist (WebGPU, else WebGL 2)
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

Click to capture the mouse. WASD moves, the mouse aims, Space jumps if the robot has a working leg
(press again in the air to double jump under the interim drive), Shift sprints, C while sprinting slides, Tab shows the scores, Esc releases the
mouse. Steer in the air with WASD. R opens the quick repair menu (below, **Field repair**). The
same keys and the mouse drive the robot's actuators through its control layout (below,
**Control layouts**), edited in the builder's controls panel (K).

**HUD.** The drawn HUD (`shooter-client`'s `overlay`) is the same natively and in the browser, in
the bottom corners clear of the reticle, laid out for 720 lines and scaled with the window. It
has no panels: text floats over the scene on a soft shadow, bars are pills on a dark track and
the weapons are rounded chips. Bottom right, a chip for every weapon copy the build carries (its
key on a badge, its short name and copy number; lost ones red and struck out), each with its own
readiness bar whether held or not (olive ready, filling orange while it recharges or waits out
its family's salvo spacing, red when the pool cannot pay it, #127), the held one opaque with an
orange badge and glowing for a moment after a switch; under them the held weapon's name, its
refire filling while it recharges, and a line with its mod and, when it cannot fire, why (lost,
low energy). Bottom left, the root chassis' health as a large reading over its bar (red under a
third) and under it the robot's resources (#128): its electricity ("power", a thin blue bar, with
the share one shot of the held weapon takes marked in orange) with what it used and made a
second, and its fuel (amber) with what the generator, an engine and a flamethrower burn a second;
in red instead, what cannot work ("drive: no power" when an electric walk was refused, "drive: no
fuel" or "drive: no engine" for a combustion one, #130, "generator: no fuel"), or, muted, "no
generator" when nothing recharges it. A chip's readiness reads the flamethrower's fuel and the
other weapons' electricity. Above them, in teal, the field repair panel (the quick repair
menu while it is open, else the repair under way, else how many parts may be repaired).
The client's UI (the HUD, the builder and the death prompt) is drawn as the renderer's screen
overlay (`ui`): rounded rectangles, and text as quads showing glyphs of an atlas texture in the
frame's world (render-engine's `OverlayQuad` corner radius and images, render-engine#163; the lite renderer
draws the same on WebGL 2), the text in Barlow Semi Condensed SemiBold (SIL Open Font License,
`crates/shooter-client/assets/fonts/OFL.txt`) rasterized into the atlas at the size it is
drawn. The rest (the parts damaged and lost, frags and place, the round clock, the equipped mods,
the carried module and its uses left, the kill and pickup feed, banners and the scoreboard) is
still text: the page's overlay in the browser and the window title natively.

Screenshots: [docs/captures/92](docs/captures/92).

**Camera and aim.** Native and browser clients share one third-person follow camera
(`shooter-client`'s `camera`, on the simulation's rig in `shooter_sim::aim`): 3.5 m behind the
eye along the aim, 0.6 m right of it and 0.3 m above, a 1.2 rad (69°) vertical field of view.
It keeps 0.2 m clear of the arena (it is pulled in at once in front of a wall, floor or ceiling
and eases back out), never through it; fighters do not push it, and your own fighter is not
drawn while the camera is inside it. The camera picks the target, the weapon fires (#102, Atlas
D2.i102.1): the camera's centre ray (from level with your eye on) picks what you aim at, the
arena or another fighter as you see them, and the reticle sits on it at the screen's centre. A
ranged shot (rifle, rail gun, rockets, shotgun, laser, flamethrower) leaves the held weapon's
muzzle where your robot carries it (the weapons kit's muzzle anchor on the weapon part, the long
barrel's when it has one) and flies toward that point, so cover beside your robot that the
camera sees past stops it: a red marker then shows where the shot will really land. The host
resolves every shot with the same functions (`shooter_sim::aim`), lag-compensated as before, and
draws its flash, beam, pellets and rockets from that muzzle; the rocket pod's launches alternate
between its two tubes, the simulation choosing which (wire 28). Online, your client shows your
own shots at once, every ranged one (beams, rockets, shotgun blasts and flame) aimed again as the
host aims it among the others as you see them (`Session::predicted_shots`). The grapple module's
hook leaves its own muzzle toward the same point (#104; its rope is tied at the eye), the blade
the feet. Native and browser captures of a shot stopping on arena cover short of the reticle (the
profile run's `--scene cover` / `?scene=cover`): [docs/captures/102](docs/captures/102).

**Guns keep their mounts** (#132, Atlas D2.i123.8). A gun's shot flies toward the camera's
target only when the target lies within 10° of the gun's barrel as mounted (its part's own
outward axis, turned with every joint that carries it); otherwise it flies straight along the
barrel. A gun fixed on the chassis front therefore hits what the robot faces (the robot turns
to face your aim) but not a target well above, below or to the side of where it points, nor, up
close, a point the camera's right-shoulder view sees but the gun's offset mount cannot: a rocket
jump needs a launcher a joint can turn to the floor (the `ROCKET_JUMPER` fixture: the launcher
on a hinge). Every gun the robot still has draws its own small mark where its shot would land
now: green on the target, red where cover stops it short, amber where its mount cannot point at
the target and the shot goes elsewhere, so a weapon group's guns each show whether they reach.
The 10° cone is provisional (Atlas Q2.i123.2).

**Precision aim** (#132, Atlas D2.i123.8). Hold the right mouse button (`weapon.aim`, rebindable
like every action) with the pointer captured: the view zooms in twice over (the field of view's
half-angle tangent halved, eased in and out over 0.12 s), the mouse turns half as fast, and a
scope frames the reticle (the frame shaded round a square sight with fine cross hairs). The
camera itself stays where it is, so what you aim at and where every shot goes are the same as
unzoomed. The same button, through every preset control layout, makes the robot's turret joints
track the camera's target (**Control layouts** below). Zoom factor, slowdown and scope style are
provisional. Screenshots: [docs/captures/132](docs/captures/132) (normal and precision view, a
turret tracking a point up a wall, fixed guns' amber marks under an unreachable reticle).

The left mouse button fires. The mouse wheel steps through the weapons your robot carries and
still has (down or E for the next, up or Q for the previous), in the order below, wrapping round
and skipping the ones it does not carry or has lost; the blade stays on V, unless it is the last
weapon left, when it is held and the trigger swings it. One wheel notch is one step, whether the
wheel reports lines or pixels: a burst of events within 60 ms is one notch. 1 to 6 pick a weapon
directly (the copy held if it is one, else that weapon's first copy). A fighter holds and fires
only the weapons its build mounts (#92): the host refuses a switch to any other, and the
prediction follows the host. A build may mount one weapon several times (#125): every copy is
its own weapon, addressed by its part, with its own mount, mod, health, refire and muzzle, its own
step on the wheel and its own numbered slot on the HUD ("Rail gun 1/2"); losing one copy leaves
the others.

**Firing several weapons at once** (#127, Atlas D2.i123.5). Holding a weapon is a convenience,
not a gate: the left button fires the held copy, and any gun may also be bound to its own key or
mouse button, alone (a direct weapon button) or in a weapon group of several, in the builder's
controls panel (below, **Control layouts**). Groups may overlap and be held together; every
preset fires all the robot's guns together on the middle mouse button. Each tick the input names
the copies asked to fire (wire 24; replay format 17), and the host fires each one that is ready
by its own refire and its family's salvo limit and that the energy pool can pay, first come,
first served by part: the grapple's hook, the swing, then the guns in part order. A refusal
refuses only that copy, and a copy asked twice fires once. So two rail guns fire in the same
tick, a rifle beside a rocket launcher, the shotgun with the laser, with no switch. Every gun is
always out: a switch delays nothing (the 0.2 s draw is gone). Rocket launchers share one salvo
limit per robot: at most one rocket a tick and 0.1 s (6 ticks) between launches, so four
launchers stagger over 0.3 s rather than launching four rockets at once (provisional numbers,
Atlas Q2.i123.2); no other family has one. Bots fire every copy of the gun they pick through the
same per-copy requests.

| Key | Weapon | Shot |
| --- | --- | --- |
| 1 | Energy rifle | Hitscan bolts, 12 damage, 10 a second |
| 2 | Rail gun | One hitscan beam, 85 damage, every 1.2 s |
| 3 | Rocket launcher | A 24 m/s rocket, every 0.8 s: up to 100 splash damage within 3 m, and a push |
| 4 | Shotgun | A blast of 10 pellets in a fixed 4.5° spread, every 0.9 s: 240 m/s projectiles slowed by drag, 9 damage each up close down to 6 at their 25 m range |
| 5 | Laser | A continuous beam while the trigger is down: 2 damage a tick (120 a second) to the first target within 30 m |
| 6 | Flamethrower | A 6 m flame, 1.2 m across, cut short by walls: after a 0.25 s ignition, 3 damage a tick (180 a second) to everyone in it |

Rocket jump by firing at your feet as you jump, with a launcher a hinge has turned down (#132;
a fixed launcher keeps pointing ahead): the blast throws you about 5 m up. Your own
rocket hurts you at half damage (a rocket jump costs every part near your feet up to 50, the legs most).

**The grapple module** (#104, Atlas D2.i104.1, D2.i104.2) is an optional robot part, not a gun: a
build mounts the grapple hook on any free socket like any part (10 kg, 10 build points), and only
a robot whose module is still on and connected can grapple. Hold F (the right mouse button is
precision aim's since #132),
whatever gun you hold, and the hook leaves the module's muzzle toward what the camera targets and
sets on arena geometry within 30 m (not on fighters): it reels you toward the hook at up to 24 m/s
and the rope never lets you further away than you have come, so running or falling across it
swings you round the hook while it reels. Let go to drop off; the next hook is ready 0.75 s after
the last one set. Each hook costs 10 energy from the shared pool (none spent, no hook, when the
pool has less). Losing the module, or its connection to the root chassis, lets go of the rope at
once and refuses the key until a respawn restores the build; dying lets go too. Every named
loadout mounts one; bots never grapple. The 30 m reach, 0.75 s interval and 10 energy are
provisional tuning. It is a deterministic stand-in in
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
claim one cell. The catalog has three chassis sizes, each electric or combustion (#130, below), beams, mount rails, an adapter, hinge and rotor joints, a battery, a fuel tank, a generator, an engine, a sensor, a repair module, legs, wheels, tracks, an armor plate and the eight
shipped weapons. Beams, mount rails and the adapter extend the attachment space (Atlas D2.i123.11,
#126): they are ordinary parts on the same tree, with mass, health and build points, whose
sockets are further hardpoints. A mount rail (20 × 40 or 20 × 80 cm, 1 or 2 points) lies flat
on its parent's face, its whole length its footprint, and has a row of hardpoints on its top, so
one rail carries two or four light weapons, or one or two heavy ones; the adapter (a 20 cm cube,
1 point) has a socket on each of its five free faces. They turn on their sockets like any part
(a rail turned a quarter lies across a face, overhanging its edge), the same validator checks
their children's footprints, cycles, parents and the budget, the mechanics and the hitboxes put
what they carry where they hold it, and losing a support loses everything on it. The fixtures
`RAIL_RACK` (six weapons on two rails), `CROSS_RAIL` (two rail guns on a turned rail) and
`ADAPTER_MAST` (a turned rail on an adapter) show them
(`cargo test -p shooter-sim --test mount_rails`); the fifth scripted replay person plays the rail
rack. A build is valid with one chassis root, no cycles, at most 32 parts and at most
118 build points (110 until the loadouts' fuel tank and generator, #128), whatever weapons it mounts how often (two rail guns, four rocket launchers, #125); there is no power, heat or cooling budget. The root
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
holding its build's first gun in the rotation (the blade only when it carries no gun; #96, the
grapple module never, #104) and fires only what the build mounts (#92). The
`pyro` loadout
carries the laser and the flamethrower (#96); the tests' over-budget `ARMORY` build carries every
weapon. You build your robot in the builder (below, **Robot builder**).

**Robot builder** (`shooter_client::builder`, Atlas D2.i70.11, #57). Both clients open on a
blueprint workbench before the match: nothing plays (and online nothing connects) until you press
Ready. The robot stands in the middle, drawn from the kits as in the match, with its sockets
marked; the part catalogue (every part and weapon with its build points, each chassis size once
under a power switch that makes the robot electric or combustion, its root swapped in place, #130)
and the named loadouts to start from (two to a row) are on the left, the component inspector on
the right (the selected part, or the part in hand: cells, mass, health, points, electricity or fuel
stored, a chassis' drive, what a generator makes at the robot's rate, motor, the electricity or fuel
a shot costs, its turn in degrees and its mod), and the budget along the bottom: build points and
parts against their limits, the robot's power, mass, electricity, fuel and the generators' output,
whether the build is valid (or the rule it breaks), and, in orange, what its powertrain cannot
supply ("drive: combustion needs an engine", "part 9 needs a fuel tank"): advice only, the build
stays valid. Drag a part from the catalogue onto a socket, or click it and click sockets to place
copies; the sockets it fits light up (sockets hidden behind other parts are not shown). The
socket under the pointer is the cell of the face the pointer is over, found by a ray from the
camera through the pointer into the parts' boxes (#124), so crowded faces, cells beside a taken
footprint, large parts and turned parents resolve to the cell actually under it; over a part
without sockets, or just off the robot, the pointer snaps to the nearest lit socket. That socket
previews the part there turned as it is in hand, with its real geometry, its box and the cells
its footprint reserves (the simulation's own footprint), or, if the rules refuse it, the box
and footprint in red, the cell another part holds and that part marked and the reason (for a
taken cell, which part is in the way and to turn the part or pick a free socket). What a click
commits is the build the preview showed. Click a part to select it. R, T and F turn the part in
hand (or, with nothing in hand, the selected part) about its socket's normal, pitch and roll
axes by the step, a quarter turn (G steps to an eighth, then about a degree, and back; Shift
turns back, Ctrl a degree whatever the step; exact either way); M cycles a weapon's mods, X or
Delete removes a part and what hangs from it (never the root chassis: "new frame" starts over
from a chassis in hand), Z undoes an edit and Y or Shift+Z redoes it (back to the exact build),
Escape cancels a drag, drops what is in hand or deselects, right drag or the arrow keys turn the
view, the wheel zooms. A drop the rules refuse keeps the part in hand. A further chassis mounts
on a socket like any part, so frames of several cuboids build up. Every edit goes through the
simulation's one validation path: an edit that breaks a structural rule (a taken cell, a mod
that does not fit, the part ceiling) is refused with the reason, and only the build points may
run over while you work; Ready takes only a valid build, which the host validates again.
Nothing warns about odd builds. A weapon may go on again on another socket; the inspector names
each copy by its number (#125). In the browser the pointer and the frame are
both device pixels, and the page keeps Ctrl+R, Ctrl+F, Ctrl+Z and Ctrl+Y for the builder (the
browser keeps Ctrl+T, so the fine pitch step there is G's).
When your robot is destroyed, the respawn counts down as before, with "[B] Edit Robot" on
screen: B opens the builder on the build you come back as and holds the respawn, at no time
limit, even across a round's start, until you press Ready and the host shows the build
configured (an input flag, `Intent::edit`; a lost connection lets go of it). While alive there
is no rebuilding. In the browser the page hides its own overlays while the builder is open. Not
in the builder yet: typed exact angles
and a testing chamber (post-beta).

**Saved builds** (`shooter_client::builder::saved`, #140). In the builder, before the match and
between lives, B (or Builds) opens the builds panel in the inspector's place: type a name
(letters, digits, `-`, Space for `_`, up to 16; Backspace erases) and press Enter or Save to save
the build on the bench with its control layout (the build's preset if you kept it). A name
already saved asks first: Save again (Overwrite?) to write over it. Click a listed save and Load
puts it on the bench (one edit: Z takes it back), or Delete removes it on a second press. Escape
or Back closes the panel. A save is a versioned envelope (magic `SHBD`, format version, name, the
build in the simulation's wire encoding, `shooter_sim::wire::encode_build`, and the layout in
`encode_bindings`), kept as hex text. Loading puts the save through the bench's validation (every
rule but the build-point ceiling, so a build over budget saved while you were working on it comes
back to be finished; Ready still takes only a valid build) and the layout through the bindings
check against that build; a save of another format version, a damaged one or one the rules
refuse is refused with the reason and the bench keeps its build. The native client keeps one
file per build, `<NAME>.build`, in `shooter/builds` under the user's data directory
(`$XDG_DATA_HOME` or `~/.local/share` on Linux, `~/Library/Application Support` on macOS,
`%APPDATA%` on Windows); the browser keeps them in the page's `localStorage` as
`shooter.build.<NAME>`, and a blocked or full storage shows its error instead of saving.
[Capture](docs/captures/140/builds.png).
`cargo run -p shooter-client --release --example builder_views -- <dir>` renders the bench
(before the match, a part being placed, snapping on a crowded face, a part turned about each
axis, a refused placement, a turned parent, a part selected again, a selected weapon, the
controls panel, the saved builds panel, an over-budget build, a build between lives) offscreen;
[captures](docs/captures/57-builder/), [snapping and turning](docs/captures/124/).

**Energy and fuel** (#128, Atlas D2.i123.1–4). Every robot has two pools (`shooter_sim::energy`,
Atlas D2.i70.3, D2.i70.4, D2.i89.1): electricity and fuel. The root chassis stores 100
electricity (other chassis sections nothing) and recovers nothing by itself; each battery (a
20 × 40 × 20 cm part, 5 build points, 20 kg) adds 60 stored electricity, each fuel tank (20 cm,
3 points, 1 kg) 1000 fuel, and each generator (20 cm, 5 points, 1 kg) burns at most 12 fuel a
second from the tanks into twice as much electricity (24 a second, what the chassis used to
regenerate), only while there is room to store it: a full robot burns nothing, and no
electricity comes from nothing. Every named loadout carries a tank on its chassis' roof beside
the sensor with a generator on it (the fixtures too). A
robot spawns with both pools full; nothing refills them but its generators (arena stations are
#129). Every consumer pays electricity: a rifle bolt 2, a rail shot 30, a rocket 20, a grapple
hook 10, a blade swing 5, a shotgun blast 12, the laser 0.6 every tick it burns (36 a second,
more than a generator makes), a repair 15 a second, and the motors by their work (below):
walking costs by speed and load, standing nothing. The flamethrower burns fuel instead: 0.5 every
tick (30 a second, a tank's 1000 in about half a minute). A request is granted whole or
not at all: with too little electricity the shot, swing or hook simply does not happen (no
cooldown is spent), a walk does not start (the robot stands, its legs at rest), and each works
again once the generator has paid for it; nothing goes below zero and nothing is paid twice.
Within a tick a robot's requests go first come, first served in a fixed order: its motors (the
walk, then its joints), a thruster jump, the grapple module's hook, the blade swing, the guns,
then a repair; the generators run after them (provisional, Atlas Q2.i70.4). Depletion stops only
what needs the empty resource: out of fuel, the stored electricity still walks and fires; out of
electricity, the generator still burns fuel and restarts them; with neither the robot stands and
lives (no limp mode, no death; self-destruct is #131). There is no heat or cooling. The host owns
the pools; snapshots carry each robot's electricity and fuel, the client predicts its own,
replays reproduce them, and the HUD shows them. Values are provisional (Atlas Q2.i70.1). A lost
battery or tank (destroyed, or carried off with a part it hung from) takes its storage and its
share of what it held with it, once, and a lost generator stops the recharge; a destroyed
battery also blasts its neighbours (below, **Part damage**). A robot whose tank or generator is
shot off keeps fighting on what it holds, then stands until it dies.

**Powertrains** (#130, Atlas D2.i123.3). The root chassis sets the robot's power: each chassis size
comes electric (codes 1 to 3) or combustion (4 to 6, the same cuboids, mass, points and health).
An electric robot is the above: its root stores the 100 electricity and its locomotion draws
electricity. A combustion robot's root stores nothing; its locomotion burns fuel through an
engine (`robot.engine`, 20 × 40 × 20 cm, 5 points, 1 kg, 60 health), as much fuel as an electric
drive would draw electricity, and cannot walk without a connected engine or with empty tanks. The
two mix through the same parts and budgets, with no free conversion: an electric robot's
flamethrower needs a fuel tank, and a combustion robot's electrical weapons, blade, grapple,
joints and repairs need a battery to store electricity and a generator to refill it, which on a
combustion chassis makes 1 electricity a fuel instead of 2 (an electric robot recharges twice as
fast; a combustion robot drives on its tanks' large store, 1000 fuel for 3 points against a
battery's 60 for 5). Losing an engine, tank, generator or battery stops only what runs on it; the
root chassis stays the only vital part. The `stoker` loadout is a combustion hybrid (below); the
others are electric. Values are provisional (Atlas Q2.i123.2). The engine is drawn as the
battery and the combustion chassis as the electric ones until the robot kit models them
(moritzbrantner/asset-tooling#837).

**Part damage** (`shooter_sim::damage`, Atlas D2.i89.1, D2.i70.8, #56, #89). Every part of a
build has its own health (provisional, Atlas Q2.i70.1: chassis 270, 290 and 310 by size, so a front
rail kill still takes four hits, or three overcharged; beams 50 and 60, mount rails 45 and 55,
adapter 40, hinge and rotor 40,
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

**Collisions and debris** (`shooter_sim::impact`, `shooter_sim::mechanics`, Atlas D2.i70.10,
#75). A hard impact damages the part actually hit, under the same rules: after each step the
host measures how much a robot's contacts changed its centre of mass's velocity (less gravity,
and less what its own movement drive put in that tick, so walking or leaning into a wall counts
for nothing), and above 20 m/s (provisional, Atlas Q2.i70.5: a sprint, a slide, jump and pad
landings and the maps' drops stay below it; a fall of about 10 m or a broadside ram at 20 m/s
each reach it) every metre a second more deals 20 to the part reaching furthest toward what it
hit, through the weapons' ray resolution with no penetration (an armor plate there takes it all);
the root chassis broken so is the kill, costing a frag like a fall. A resting or pressed robot is
never hurt tick after tick. A part carried away whole when its parent breaks, at least 12 kg,
stays a physical body for a while (debris: it falls, collides with the arena, robots and other
debris), retiring after 5 s, after lying still 1 s, at its robot's respawn, or as the oldest past
16 at once (provisional, Q2.i70.6); destroyed and lighter parts shatter (clients let them fall
on their own). Snapshots carry the debris exactly (wire 20), the prediction resumes among it, and
clients draw those parts where the debris is. physics-engine does not report contact impulses
yet (moritzbrantner/physics-engine#575), hence the whole-robot measure.
`cargo test -p shooter-sim --test collisions` checks rams, a glancing hit, falls, low-speed and
resting contact, the struck part against contact order, plates, a broken parent's branch as
debris, the impact kill, the budget, exact continuation and the wire; with `--release --
--ignored --nocapture` it measures twelve robots shedding debris.

**Field repair** (`shooter_sim::repair`, Atlas D2.i70.9, #61). A build may mount repair modules
(`robot.repair-module`: 20 × 20 × 20 cm, 12 kg, 8 points, 40 health; the `mechanic` loadout has
one on its chassis' back). R opens a quick menu over the running match listing only the parts the
robot may repair now: damaged, still connected, and within 0.6 m of a working module's centre
where the robot's bodies carry them (a module does not repair itself). The wheel or Q/E moves
through it, R again repairs the chosen part (or stops the repair under way) and closes it; the
part is lit teal on the robot, and the robot keeps walking, aiming and firing. The order goes to
the host, which refuses a part that is not eligible (stale, out of reach, whole, destroyed). A
repair restores one health every 0.1 s for 15 electricity a second, after the tick's motors,
jump, swing and shots, so an empty pool holds it until the generator pays; it ends when the part is whole and stops when the part or its module is lost or out of
reach. A hit on the part under repair or on its module pauses it for 0.75 s (a further hit starts
the pause over): no health and no energy meanwhile, the work done is kept and it resumes by
itself; hits elsewhere do not pause it. Destroyed parts stay gone; death ends a repair and
respawn restores the build. Snapshots carry each fighter's repair (part, module, progress,
pause) and the parts it may repair; the client predicts its own, replays reproduce it, and bots
do not repair. The radius, rate, energy and pause are provisional (Atlas Q2.i70.3). The robot kit
has no repair module model yet (moritzbrantner/asset-tooling#742): it is drawn as the sensor.
Views (the menu with the leg lit, repairing, paused; `--example repair_views`):
[docs/captures/61](docs/captures/61).

**Control layouts** (`shooter_sim::bindings`, Atlas D2.i70.7, #74). A robot's actuators (its
legs, wheels, tracks, hinges and rotors) follow one control layout: up to 24 bindings, each from
a key, a mouse button or a mouse axis to one actuator or an action group of up to 8, with its own
inversion (and per part, so one side of a differential drive runs backward), strength (or mouse
sensitivity), hold or toggle for a key or button, and relative steering (stops when the mouse
stops) or cursor-offset steering (stays where the mouse left it) for a mouse axis; the legs' gait
(suggested, biped, quadruped, all at once) is part of it. Each binding gives a stride (legs
only), a rate or an angle; a part takes the kind of the first binding naming it and the sum of
that kind. Nothing is timed, sequenced or conditional. Every build starts on its preset, an
ordinary layout suggested from what is mounted where and correcting nothing: W strides every leg
forward and S back, D and A turn the legs differentially; W rolls each wheel and track the way its
axle rolls it ahead (a wheel on the front or back rolls the robot sideways: its mounting says
so), A and D differentially; the mouse's horizontal offset turns every rotor and its vertical
offset every hinge, so two joint axes aim a turret. A binding may instead fire guns (#127): one
gun is a direct weapon button, several a weapon group (keys and mouse buttons only, held or
toggled; the blade and the grapple stay on their own keys); the preset's is every gun on the
middle mouse button. A binding may also make joints **track the aim** (#132, keys and mouse
buttons only, held or toggled): while it is on, each joint in its group is told to track
instead of whatever else binds it, and the host, from its own state, turns that word into the
servo angle that points the first gun the joint carries at the camera's target, as far as the
joint's own axis can and clamped to its limits (a hinge a quarter turn either way, a rotor
without end); the joint gets there at its servo's speed and torque, paying its draw from the
pool, and a joint that has lost its gun holds where it is, a lost or unpowered one aims nothing.
Every preset tracks with every rotor and hinge on the right mouse button, the precision aim's,
so holding it zooms and turns the turret onto the reticle together; let go, the mouse offset
steers them again. In the builder K (or Controls) opens the
controls panel in the inspector's place: pick a binding, press Key... and a key (or Mouse to cycle
the mouse inputs), change its mode, command, inversion and strength, click a part on the robot
and Part +/- or Flip Part, New for a new binding on the selected part (on a gun: a fire binding),
Preset to go back; every edit is checked against the build's actuators and guns by the
simulation and refused with its reason.
Ready takes the layout with the build; while you have not edited it, it follows your build's
preset. The client resolves the layout every tick and its input carries the resulting
per-actuator commands and the guns asked to fire (wire 22 and on; wire 23 and replay format 16
address weapons by copy, #125; wire 24 and replay format 17 carry the fire requests, #127; wire
25 and replay format 18 a joint's track word, which carries no angle, #132); the
host takes only commands the robot's connected actuators can take and rests every other
actuator, and fires only guns the robot still has, so a tampered input moves or fires nothing
a layout could not. Bots press their build's preset from their movement. The layout lives for
the session (not saved yet).
**The bindings carry a robot on wheels or tracks alone** (#58, Atlas D2.i74.1): W and S roll
its wheels and tracks, A and D turn it differentially (a skid steer turns slowly on the spot),
the camera turns nothing, Space does nothing unless it carries the thruster module, and the
jump pads still throw it; its locomotion pays by its commands, so with its electricity (or, on a
combustion chassis, its fuel) gone it stops. A car tops out at about 3.6 m/s and a tank at about
3.5 m/s (provisional); a track's road wheels reach 4 cm past its cuboid, so a track level with
its hull's bottom holds the hull off the ground (the kit's drawn road wheels are 4 cm smaller).
Bots on wheels or tracks steer their chassis themselves and fire their chassis' guns once they
face their target. What it does not do yet: **legs.** A robot with a connected leg is still
moved by the interim movement drive below (WASD, Shift, C, Space with its legs, the camera
turning it), the bounded fallback until legs balance on their own servos (Atlas D2.i74.1
records the experiments: four-legged loadouts walk straight at about 1 m/s on a leg balance
assist but still topple turning, backing, leaping and on kerbs; bipeds fall); its legs stride
through the preset, which with the drive is mostly seen, not felt. The joints (turrets) are
moved by the layout alone. `cargo test -p shooter-sim --test bindings` covers the layouts, the presets driving
wheels, legs and turrets in the mechanics world (no drive), the codecs and the host's check.

**Mechanics** (`shooter_sim::mechanics`, Atlas D2.i70.5, #73). The simulation
builds any valid robot as a mechanical one in a dynamic world of its own
(`shooter_sim::physics::articulated`, on physics-engine's compound bodies and revolute joints
with motors): every part placed in space from its socket and exact orientation, rigidly joined
parts one compound body with each part's mass where it is mounted, and a separate body on a
motorised joint for what sits on a hinge (a servo within a quarter turn either way) or a rotor
(a continuous motor), for each wheel (a sphere: physics-engine has no cylinders), each of a
track's two road wheels and each leg (one self-contained module: a thigh on a hip servo and a
shin with a flat foot on a knee servo). Each tick a robot's actuators take one command each (a
rate, an angle, a leg's stride or idle) and ask the robot's electricity for their work, first
come, first served in part-ID order: the part's rated draw scaled by how hard the command drives
it (a stride's length, a rate against the top rate, a servo's angle off centre) and, for
locomotion, by the robot's mass against 200 kg (#128); a motor at rest costs nothing, and a
refused motor is off that tick. Mounting orientation is authoritative: a wheel turns about its own mount normal, so
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
a replay hold the same world. A robot without a connected leg moves on its own actuators (above);
for a robot with a connected leg the fast-movement rules (sprint, slide, jumps, jump pads, air
control, the grapple's rope) give the velocity a movement drive sets on the whole robot's centre
of mass, and the drive keeps it upright and turns it to face the aim (at most 16 rad/s). Its legs
stride and its wheels and tracks roll as its control layout tells them (above, **Control layouts**; until #74 at the rate its speed gave them), and
its motors pay first in the tick: its locomotion for the walk the drive carries it on, by speed
and load (the standard robot's two legs: about 4.7 a second walking, 6.7 sprinting, nothing
standing; provisional, #128), its joints as commanded. The simplest mapping
while the bindings do not carry a robot, with what it costs: the robot's bodies slide frictionless where the
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
the load fails, the HUD says why and the fighters stay boxes. The robot kit has no mount rail or
adapter models yet (moritzbrantner/asset-tooling#743): a rail is drawn as the beam of its length
laid flat, the adapter as the short beam at half its length. Each part sits where the simulation
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
flamethrower's loops while firing, the rail gun's recharge and the kick of the rocket pod's tube
that fired (the simulation alternates them, #102) and reload over their (mod-tuned) refire time,
the grapple's launch (its hook flies to where the host set it, the cable runs from the launcher)
and retract, and the blade extending when held or
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
with their weapons and mods and lie there until it respawns (the ones the host keeps as physical
debris are drawn where the snapshot has them and vanish when it retires them), while the rest of it stays on its
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
field): `standard`, `heavy` (with a battery: 160 electricity), `skirmisher` (rapid-cycler rifle,
efficiency-coiled shotgun), `marksman` (long-barrel rifle, overcharged rail gun: three hits a
front kill instead of four, on the root chassis) or `pyro` (long-barrel laser reaching 45 m,
flamethrower, blade and grapple on the small chassis: 182 kg, 87 of 118 points, the root
chassis' 100 electricity; it spawns holding the laser) or `mechanic` (the standard build with a
repair module on the chassis' back, **Field repair**) or `stoker` (a combustion hybrid on the
medium combustion chassis: an engine on its roof drives its legs on fuel, the flamethrower burns
the same tank, and a battery on its back stores the electricity its energy rifle, blade and grapple
use, **Powertrains**),
each a two-legged component tree with a sensor, a fuel tank and a generator
on top and its four weapons on the chassis' front, facing the aim (no arms since #91), so a front
shot at the upper half of the chassis meets a weapon first. The builder's "start from" list loads
them too. The HUD shows the held
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
`cargo test -p shooter-sim --test grapple` checks the grapple module: no module, no grapple; the
hook leaves the module's muzzle (turned with the robot) on its own key while a gun fires; the
pull and the rope that only shortens; a pendulum swing under a ceiling without stretching; letting
go with the momentum kept; the module lost mid-pull; its energy and interval; bots never grapple;
the replays' scripted people grapple.
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
