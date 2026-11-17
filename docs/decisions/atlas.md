# Decision Atlas

The canonical map of product intent for the shooter. Leaf entries are Decisions (approved by the
owner) and Questions (open). Answered entries are never rewritten; a later Decision links to and
supersedes them. New entries take an issue-derived ID (parent ID + `i` + issue number, e.g.
`3.i12`) so parallel PRs never collide; several entries from one issue add a running number
(`D2.i46.1`, `D2.i46.2`, …).

Decisions state product intent, not shipped behaviour. What works today is described in the
[README](../../README.md); the robot beta (D1.i46.1) is tracked in shooter#45 and is not
implemented yet.

## 1. Purpose

- **D1.1** (owner, 2026-11-11): The shooter exists both as a game and as an additional, demanding
  consumer of the owner's foundation crates, so they gather more real-world data. It must never
  block other projects: it gets worker time only when another lane is idle.
- **D1.i46.1** (owner, 2026-11-13; shooter#45, shooter#46): Move the playable proof of concept to a
  beta-quality third-person modular robot arena shooter, on the existing `shooter-sim`, server,
  client and foundations (D4); their capabilities are extended, not reimplemented. The beta is
  done when real native and browser matches run with real assets, the beta player loop and
  deterministic replay and physics tests. D1.1 still governs scheduling.

## 2. Experience

- **D2.1** (owner, 2026-11-11): First-person arena shooter.
  Superseded by D2.i46.1.
- **D2.2** (owner, 2026-11-11): First playable: online PvP deathmatch on one arena, hosted through
  game-server, with bots filling empty slots. Bots also make it playable and testable alone.
- **D2.3** (owner, 2026-11-11): Fast arena movement: sprint, slide, double jump, jump pads, rocket
  jumping and air control. No aim-down-sights; large health pools.
- **D2.4** (owner, 2026-11-11): Stylized sci-fi look: a clean modular arena (panels, ramps, jump
  pads, glowing trims), energy weapons (energy rifle, rail gun, rockets). No realistic gore.
  Styling superseded by D2.i46.2.
- **D2.5** (owner, 2026-11-11): One of the guns is a **grapple hook**: fire it at arena geometry and
  get pulled or swing toward the hit point. It is a movement tool in the weapon rotation.
- **D2.6** (owner, 2026-11-11): Players also have **melee weapons** alongside the guns.
- **D2.7** (owner, 2026-11-12; answers Q1): Free-for-all deathmatch is the only mode for now.
  Revisit other modes after people have played it live.
  Superseded by D2.i46.3.
- **D2.8** (owner, 2026-11-12; answers Q2): Keep the shipped bots: one beatable skill level
  (0.35 s reactions, wandering aim error, weapon by range, blade up close, no grapple), seeded
  and deterministic.
- **D2.i46.1** (owner, 2026-11-13; supersedes D2.1): Third-person view.
- **D2.i46.2** (owner, 2026-11-13; revises D2.4's styling): Primarily clean, relatively realistic
  futuristic sci-fi with restrained industrial wear, variation driven by hardware; not strongly
  stylized or cartoonish. No biological gore.
- **D2.i46.3** (owner, 2026-11-13; supersedes D2.7): Free-for-all deathmatch stays the base mode;
  team and objective modes are planned. Which mode comes first and in which beta phase is open
  (Q2.i46.1). Bots remain for solo play and empty slots (D2.2, D2.8).
- **D2.i46.4** (owner, 2026-11-13): Fighters are fully mechanical modular robots, genuinely modeled
  and animated, with replaceable body, head, limb, locomotion and weapon modules. No box or block
  placeholder fighters in the beta.
- **D2.i46.5** (owner, 2026-11-13): Combat stays fast and acrobatic (shooting and grappling, D2.3,
  D2.5) with meaningful build trade-offs. Robots are assembled free-form, bounded by engineering
  constraints (weight, power, cooling, attachment compatibility) and by competitive build points
  (values open, Q2.i46.4). Basic weapons and components are available immediately; no unlock
  grind in the beta.
  Assembly rules superseded by D2.i70.1 and D2.i70.2; cooling superseded by D2.i70.4 (no heat or
  cooling in the beta), power by D2.i70.3 (shared energy).
- **D2.i46.6** (owner, 2026-11-13): Limbs are targetable and detachable; losing one changes the
  robot's actual abilities and movement in physically believable, authored ways. Robots can be
  repaired during combat (how, Q2.i46.2); respawn restores the configured build. Fully emergent
  physics-based self-balancing is later, not beta.
  Superseded in part by D2.i70.2 (the core is the only kill; itself superseded by D2.i89.1, the
  root chassis is), D2.i70.8 (damage and permanent destruction) and D2.i70.9 (repair).
- **D2.i46.7** (owner, 2026-11-13): After death and before respawning a player may substantially
  reconfigure the robot; while alive, only repairs, not rebuilds.
  Respawn flow refined by D2.i70.11.
- **D2.i46.8** (owner, 2026-11-13): Weapons build on the shipped rifle, rail gun, rockets, shotgun,
  laser, flamethrower, grapple hook and blade, and allow weapon modifications. Full
  weapon-from-parts engineering is later, not beta.
  Since shooter#92 (owner request, 2026-11-14) a build is its arsenal: a fighter spawns holding
  its build's first weapon and holds, fires and swings only the weapons the build mounts and still
  has; the host refuses a switch to any other. The mouse wheel and Q/E cycle those weapons in the
  fixed rotation order; the blade stays on its own key unless it is the last weapon. No named
  loadout mounts the laser or the flamethrower yet, so until the builder (shooter#57) only the
  tests' over-budget armory build fires them.
  Since shooter#96 (owner, 2026-11-14) a fifth named loadout, `pyro`, mounts the laser (with a
  long barrel) and the flamethrower beside the blade and the grapple, so every weapon stays
  playable until the builder. A fighter spawns holding its build's first weapon in the rotation
  that is not the grapple; the grapple, then the blade, only when it carries nothing else.
  Since shooter#57 the builder mounts any weapon on any build (D2.i70.11).
- **D2.i46.9** (owner, 2026-11-13): Some powerful weapon and movement modules spawn as pickups at
  contested arena points. Any robot can use them, with no compatibility gate; a build may exploit
  one better (for example grapple plus a sniper position). Their lifetime is open (Q2.i46.3).
- **D2.i46.10** (owner, 2026-11-13): The beta has more than one locomotion configuration (for
  example legs and wheels or tracks). Exotic machines (quadruped, hovering, spider) are later.
  Superseded by D2.i70.6 (four-leg robo-dogs are beta builds).
- **D2.i46.11** (owner, 2026-11-13): Three initial arenas, small, medium and large, with some
  destructible cover, doors and machinery around a stable core topology. Maps differ in combat
  opportunities and player capacity (layouts open, Q2.i46.5).
- **D2.i46.12** (owner, 2026-11-13): About 12 player slots by default, scalable by map and mode.
  Not a commitment to a 32-player beta or to a global maximum.
- **D2.i46.13** (owner, 2026-11-13): No story in the beta. Cosmetic unlocks, elaborate salvage from
  defeated robots, extensive arena destruction and advanced progression are post-beta.

The D2.i70 entries reconcile the owner's detailed robot construction and combat record of
2026-11-13 (choices 01–29; shooter#70, tracker shooter#45) and the owner's 2026-11-14 follow-ups
on shooter#57 and shooter#61. They are the one intended beta contract; D2.i70.12 says what ships
today and which issue migrates it. Each capability has one owning issue (D5.i70.1).

- **D2.i70.1** (owner, 2026-11-13; shooter#70; supersedes D2.i46.5's assembly rules, answers
  Q2.i46.4's attachment rules): Robots are assembled free-form on universal mounts: a socket grid
  of 20 cm surface cells (a 20 × 40 × 80 cm chassis exposes 28 face sockets). Parts are oriented
  freely in three dimensions, with optional snaps and exact angles, and attach to a socket on any
  compatible component; a larger part occupies one parent socket and reserves its neighbouring
  footprint without becoming multiply attached. The assembly is a single-parent, acyclic component
  tree; cuboid chassis sections in several predetermined sizes and beams extend it. Ordinary
  attachments are rigid; only joint components move. A build is bounded by a build-point cap and a
  separate technical part-count ceiling. Only objective rules invalidate a build: attachment,
  connectivity, physical occupancy, core count (D2.i70.2) and the budgets. Stationary,
  asymmetrical, sideways-driving, grappling-only or otherwise unconventional robots are valid; no
  build is rejected, warned about or corrected for lacking standard locomotion. Owner: shooter#71.
  The core count is gone with the core (D2.i89.1).
- **D2.i70.2** (owner, 2026-11-13; shooter#70): Every robot has exactly one control core, on an
  exterior socket of the player's choice. Armor may cover it (penetration exists, D2.i70.8); there
  is no exposed-face check. Destroying the core, and only that, eliminates the robot: losing
  weapons, wheels, legs or batteries can incapacitate it but not kill it, and an immobile robot
  whose core survives stays alive. Placement and validation: shooter#71; the kill condition:
  shooter#56.
  Superseded by D2.i89.1: there is no control core; the root chassis is the vital part.
- **D2.i70.3** (owner, 2026-11-13; shooter#70; supersedes D2.i46.5's power constraint): The core
  holds a baseline energy capacity and regenerates it continuously. Optional battery modules add
  stored energy only, and have mass, build-point cost and component health. Every connected part
  draws from one shared pool: no wiring graph, special power sockets or separate circuits. Weapon
  activations, continuous motors and repairs (D2.i70.9) request energy, granted deterministically
  first come, first served; once the pool is empty later requests fail or stop until regeneration
  refills it. No automatic equalisation and no player-set priorities in the beta. A destroyed
  battery loses its charge and capacity and sets off a small local explosion that damages nearby
  components through the ordinary component damage (D2.i70.8), with no chain-reaction mechanic.
  Batteries raising the maximum output, and arena charging stations, are later. Strong pickup
  modules also pay from this pool (owner, 2026-11-14, shooter#62). Owner: shooter#72.
  Since shooter#72's first half (split approved by the owner, 2026-11-14) the pool ships
  (`shooter_sim::energy`): the core's capacity and regeneration, batteries adding stored capacity,
  weapon activations and the pickup boosts paying first come, first served (Q2.i70.4), host-owned,
  in snapshots, predicted and replayed. Its second half, a destroyed battery losing its charge
  and capacity and its local blast, needed per-part damage and ships with it: since shooter#56 a
  lost battery (destroyed, or carried off with a destroyed part it hung from) takes its capacity
  and its share of the charge once, and a destroyed one sets off a small blast through the
  ordinary part damage (a battery it destroys blasts in turn, once each; no chain-reaction
  subsystem). Since shooter#73's first half, motors draw from the pool too: a flat cost each commanded tick, in part-ID order
  (provisional).
  Superseded in part by D2.i89.1: the root chassis, not a core, holds the baseline capacity and
  regenerates it; batteries are unchanged.
- **D2.i70.4** (owner, 2026-11-13; shooter#70; supersedes D2.i46.5's cooling constraint): The beta
  has no heat or cooling mechanic: weapons and parts are limited by energy (D2.i70.3) and
  cooldowns, not heat accumulation or cooling modules. The heat and cooling that ship today stay
  until an explicit migration replaces them (D2.i70.12, shooter#72).
  Migrated by shooter#72: the schema has no heat, cooling, power or draw left, the heat sink mod
  is removed (its code retired) and replaced by the efficiency coil, and the other mods trade
  energy a shot instead of heat (Q2.i46.4).
- **D2.i70.5** (owner, 2026-11-13; shooter#70): Robots move mechanically. Rigidly joined parts form
  one compound rigid body with per-part hitboxes and health; movable joints create separate
  articulated bodies. Mass at its actual mounting position sets the centre of mass, rotational
  inertia and stability. Hinge servos turn about one local axis toward an angle within limits;
  continuous rotary motors turn either way with finite torque, angular speed and energy cost, and
  stall under heavy load. A two-axis turret is an assembly of joints, not a separate physics model.
  A part's mounted orientation is authoritative (a wheel mounted sideways drives sideways, a
  mismatched drive spins, asymmetry tips); nothing auto-corrects direction. A beta leg is one
  self-contained module with simple movement controls; biped and quadruped gait presets are
  suggested from the attached legs and can be overridden, and they command physical movement
  without teleporting or guaranteeing stability. No structural stress or fatigue, full
  configurable joint solver or multipart leg construction in the beta. Owner: shooter#73.
  Since shooter#73's first half the mechanics ship in `shooter_sim::mechanics` on
  physics-engine's compound bodies and revolute motors (`shooter_sim::physics::articulated`,
  mirrored on the Rapier oracle): every valid build becomes compound bodies on hinge, rotor,
  wheel, track and leg joints (a leg is one module: hip and knee servos and a flat foot), driven
  by per-actuator commands with stall detection, the biped and quadruped presets, per-part poses
  and hits, and an exactly restorable state. The match's fighters still use the capsule
  controller; moving them onto the mechanics (snapshots, prediction, replays) is its second half.
- **D2.i70.6** (owner, 2026-11-13; shooter#70; supersedes D2.i46.10): Valid beta builds include
  humanoids and other bipeds, four-leg robo-dogs on built-in quadruped walking presets, wheels and
  tracks, stationary platforms and grappling-only machines, all from the same part types and rules.
  Multipart user-assembled legs, sophisticated self-balancing, spider gaits and hovering are later.
  Owner of the locomotion set: shooter#58, on the mechanics of D2.i70.5 (shooter#73).
- **D2.i70.7** (owner, 2026-11-13; shooter#70): Players drive their robot through one active control
  layout of direct bindings: keys, mouse axes and buttons bound to individual actuators or weapons,
  or to action groups of several parts, each binding with its own direction inversion,
  strength or sensitivity, and hold or toggle. Mouse input steers by relative movement or by cursor
  offset, chosen per binding; two joint axes can aim a turret. Presets are editable mappings onto
  the same commands, and inputs never apply fake chassis forces. No programmable or timed
  automation, conditional scripts, automatic attacks, combo planners or automatic correction of
  unusual builds; manually switchable control modes are later. Owner: shooter#74.
- **D2.i70.8** (owner, 2026-11-13; shooter#70; refines D2.i46.6): Armor is individual plates with
  real hitboxes, and penetration is weapon-specific, so a hit may reach components behind a plate.
  A part works fully until its health reaches zero (no gradual degradation); at zero it is
  destroyed and detached for the rest of that life, and the branch below it in the component tree
  disconnects. Destroyed parts never come back mid-life; respawn restores the configured build.
  Owner: shooter#56.
  Since shooter#56 this ships (`shooter_sim::damage`): every part has health, the host resolves
  hitscan, pellet, rocket, splash, blade and flame strikes onto the parts placed at the fighter's
  (rewound) centre and facing, each part at most once a strike; plates are boxes like the rest
  and a penetrating weapon passes a share of the damage to the part behind. Lost means no longer
  connected to the core through the tree (the core is a leaf: when one of its ancestors is
  destroyed, it is the rest that falls away). A lost weapon part does not fire, and a robot with
  no connected locomotion part does not walk; how surviving legs and wheels move it waits on the
  mechanical robots in the match (shooter#73's second half). The core's destruction is the kill
  (D2.i70.2), replacing fighter health. Snapshots, prediction and replays carry the part
  healths. Field repair is shooter#61, collision damage and physical debris shooter#75.
  Since shooter#89 (D2.i89.1) lost means no longer connected to the root chassis: a destroyed
  part takes its subtree with it, and the root chassis' destruction is the kill.
- **D2.i70.9** (owner, 2026-11-13 and 2026-11-14; shooter#61; answers Q2.i46.2): Repair is done by
  physically mounted repair modules (mass, health, build points) that the player activates
  manually. A module restores, over time and for energy from the shared pool (D2.i70.3), a
  player-selected component that is damaged but still above zero health and still connected,
  within a limited physical radius around the module's mounting position, so placement and
  coverage matter (2026-11-14, B). Repairs may run while the robot moves and fires; they compete
  with motors and weapons for the same energy (2026-11-14, A). No passive regeneration,
  reconstruction of destroyed parts, salvage from opponents or build change while alive. Owner:
  shooter#61.
- **D2.i70.10** (owner, 2026-11-13; shooter#70): High-energy collisions between robots or objects,
  and high falls, damage the components actually hit, from relative impact speed, effective mass or
  impulse and contact geometry, under the same health and detachment rules as weapon hits
  (D2.i70.8). Ordinary walking, driving, small bumps and resting contact do no damage. Large
  detached pieces stay briefly physical and collidable; small shards are visual only; debris is
  retired deterministically by a bounded lifetime, sleep or budget policy. Persistent wreckage, full
  fracturing, stress and fatigue are later. Owner: shooter#75.
- **D2.i70.11** (owner, 2026-11-13 and 2026-11-14; shooter#57; refines D2.i46.7): The builder is a
  simplified blueprint workbench, not an elaborate 3D garage: a draggable part catalogue, a central
  blueprint surface with the rotatable chassis, its sockets, placement footprints and the real part
  geometry, a component inspector, and the budget, energy, control and validation status (no heat
  meter). A testing chamber is later. After death the robot respawns automatically with its last
  valid build after the normal countdown unless the player chooses Edit Robot; the builder then
  suspends respawn, with no time limit, until the player presses Ready with a host-accepted build
  (2026-11-14, A). Owner: shooter#57.
  Ships since shooter#57 (`shooter_client::builder`), with the simplest options for what was
  open (the gestures and the plumbing, the owner's to revisit): both clients open on the builder
  before the match, nothing playing and online nothing connecting until Ready; a robot that has
  applied no input since its person joined is still being set up, so a build configured then
  applies at once (`Session::configure`), every later one at the next spawn. Parts are dragged
  from the catalogue onto a socket, or clicked and then placed on sockets; R, T and F turn a part
  an eighth of a turn about its socket's axes (Shift back, Ctrl about a degree), M cycles a
  weapon's mods, X/Delete removes a part with its subtree (never the root chassis; a new frame
  starts from a chassis), right drag turns the view. The bench checks every edit through
  `Catalog::validate` with the build-point ceiling lifted, refusing structural breaks with the
  rule's reason and showing the beta catalog's verdict; Ready takes only a valid build. Edit
  Robot is B while dead, carried as an input flag (`Intent::edit`, wire 15) that holds the dead
  robot out of its respawn and out of a round's start; the builder closes when a snapshot shows
  the readied build configured, and a person whose inputs stop (a lost connection) lets go of it
  after the input hold. Not yet: action-group bindings (shooter#74) and typed exact angles.
- **D2.i70.12** (owner, 2026-11-13; shooter#70): What ships today is the baseline, not the beta
  contract: `shooter_sim::robot` (shooter#47) with its anatomical torso, head, arm and locomotion
  sockets, hand and shoulder hardpoints, named loadouts and its mass, power, heat and cooling
  budgets, and the weapon mods (shooter#51), including the heat sink and the heat the other mods
  add; the defaults are recorded under Q2.i46.4 and described in the README. They stay truthful
  until replaced, and replacing them is an explicit migration of the one schema, never a second
  assembly system beside it: shooter#71 migrates the assembly, the named loadouts and the mods'
  mounting; shooter#72 removes heat and cooling and rebalances the heat-related mods.
  Since shooter#71 the assembly ships as D2.i70.1 and D2.i70.2 describe (the socket-grid component
  tree, one core, the build points and part ceiling), replacing the anatomical sockets and
  hardpoints in place; the named loadouts are humanoid component trees with their mods on their
  weapon parts. The legs' load budget is gone (D2.i70.1 rejects no build for its locomotion).
  Since shooter#72 the power, heat and cooling budgets are gone too, replaced by the shared energy
  pool (D2.i70.3, D2.i70.4); the heat sink became the efficiency coil and the heavy loadout carries
  a battery.
  Since shooter#89 there is no core (D2.i89.1).
  Since shooter#91 (owner, 2026-11-14) the named loadouts carry no arm beams: a sensor on top,
  two legs underneath and every weapon, with its mod, on the root chassis' front facing the aim.
  Each costs the two long beams' 6 build points less (standard 69, skirmisher 84, heavy 99,
  marksman 99; the 110 cap is unchanged); the beams stay in the catalog.

The D2.i89 entry records the owner's decision of 2026-11-14 on shooter#89.

- **D2.i89.1** (owner, 2026-11-14; shooter#89; supersedes D2.i70.2 and the core's role in
  D2.i70.3): The chassis is the most important part of a robot; there is no separate control core.
  The root chassis (the tree's root part, always a chassis) is the robot's vital part: its
  destruction is the only kill (falling out of the arena still kills). Other chassis sections are
  ordinary structure, and destroying any non-root part still disconnects its subtree. No mandatory
  core, no exterior-socket rule and no core count; builds naming the retired core are rejected
  (its code is retired, not reused). The shared pool's baseline capacity and regeneration come
  from the root chassis; batteries still add storage. Armor may cover the root chassis;
  penetration as before.
  Ships since shooter#89: `robot.core` (code 30) is gone from the catalog; the root chassis holds
  the core's former 100 energy and 24 a second; connectivity counts from the root chassis; bots
  aim at it; the HUD shows its health. The chassis healths were raised so front kills take as
  long as before (Q2.i70.1), and the build points fell by the core's 10 so every build keeps its
  headroom. The pinned robot kit still ships a `robot.core` model, which the client skips by name
  until a kit release drops it.

- **D2.i102.1** (owner, 2026-11-16; shooter#102, option B): **Camera-targeted, muzzle-authoritative shots.** The third-person camera establishes the intended aim point, but a damaging ranged weapon's actual hit ray or projectile originates at its mounted, connected weapon muzzle and travels toward that point. Geometry between the muzzle and the aim point blocks or intercepts the shot even if the camera sees the target: camera peeking does not enable shooting through cover. The authoritative origin and intersection are simulation/host rules shared with native and WebGPU prediction, reticle and visual effects; the client renderer may not supply independent combat truth. Preserve the existing weapon damage, energy, part-loss, lag-compensation and replay contracts, reconciling muzzle transforms with the real weapon/robot kit and articulation. This applies to mounted damaging ranged weapons, with the grapple's separate movement/rope contract reconciled rather than silently changed. **Intent only; implementation/visual acceptance pending** in #102 and #49. Draft PR #84 remains its previously defined battery/laser-visual change, without this gameplay redesign unless separately integrated.


- **D2.i104.1** (owner, 2026-11-16; shooter#104; answers Q4's movement and
  target choices, option A): A grapple attached to **arena geometry** supports
  **momentum-based swinging and active pulling/reeling toward its anchor at the
  same time**. It does not latch onto another fighter in the first robot beta.
  The shipped 30 m reach and 0.75 s between hooks are provisional balance
  defaults, not fixed owner-approved values. Preserve deterministic host-owned
  anchor, rope, release and prediction semantics; implementation verification
  for the new mounted-module path remains in #104.

- **D2.i104.2** (owner, 2026-11-16; shooter#104; supersedes D2.5's
  gun-in-rotation classification and only the grapple-as-weapon aspect of
  D2.i46.8): The grappling hook is an **optional socket-mounted robot module**,
  not a standard gun in the weapon rotation or a mandatory movement ability.
  It occupies a legitimate free-form component attachment, spends build points
  and shared energy, and responds to its **own configurable control binding**.
  A robot without an operational mounted grapple cannot use the action; loss,
  detachment or disconnection disables it through the same authoritative part
  rules as other components. Reuse #71's component graph, #57's builder, #74's
  bindings and the existing grapple mechanics. Exact cost, attachment variants,
  use interval and reach remain implementation tuning, not new product policy.
  The legacy gun remains current shipped behavior until #104 migrates both
  clients and the host, with replay, prediction and live visual acceptance.

## 3. Platforms

- **D3.1** (owner, 2026-11-11): Native wgpu client on render-engine plus a browser client on Pages.
  Both join the same game-server match.
- **D3.2** (owner, 2026-11-11): Assume mouse and keyboard for now. Gamepad support is not a
  current goal; existing gamepad paths may stay, but no work or acceptance depends on them.

## 4. Foundations and data

- **D4.1** (owner, 2026-11-11): Use the owner's foundation crates from day one (physics-engine,
  render-engine, game-audio, input-bindings, game-server), checked against a Rapier A/B oracle in
  CI. When a crate lacks a feature, file an issue on that crate and use Rapier (or the
  off-the-shelf equivalent) for that piece only until it lands.
- **D4.2** (owner, 2026-11-11): Data flows back as (a) deterministic replays of bot matches, with
  chosen ones promoted to fixtures in the foundation repos (physics A/B, game-server lag
  compensation, render-engine scenes); (b) runtime-profiler traces from scripted bot matches on
  both clients in CI; (c) issues on the owning crate for gaps and regressions. Nothing is
  collected from real players.
- **D4.3** (owner, 2026-11-12): The shooter does not depend on mmorpg. It uses physics-engine
  directly through its own thin adapter (`shooter_sim::physics`) and builds every character move
  itself, choosing the most performant option; the integer trig, bot RNG and melee coverage
  live in `shooter-sim`. The Rapier A/B oracle is a shooter-owned backend under `physics-ab`
  (shooter#42).
- **D4.i70.1** (owner, 2026-11-13; shooter#70): The robot beta keeps the existing owners. Build
  validation, energy, damage, repair, controls and debris are authoritative in `shooter-sim`; the
  native and browser UI only consume validated build state through the same rules path. Compound
  bodies, joints and motors use physics-engine through `shooter_sim::physics`, checked against the
  Rapier oracle (D4.3); a shared capability that is genuinely missing is filed on physics-engine,
  never solved by a duplicate solver in the shooter (D4.1). Controls build on
  `input-bindings-core`; render-engine and asset-tooling keep their ownership (D5.i46.1).

## 5. Repository

- **D5.1** (owner, 2026-11-11): Lives at `moenarch-games/shooter`, public, with Pages for the
  browser client. Art is authored through asset-tooling (CC0 first; Mixamo only as inspiration).
- **D5.i46.1** (owner, 2026-11-13; shooter#45): asset-tooling owns the authored, reviewed and
  exported beta assets: the arena modules and destructible cover (moritzbrantner/asset-tooling#593),
  the third-person weapon, grapple and blade models with modular mounts
  (moritzbrantner/asset-tooling#594) and the modular combat robot with rig, detachable parts and
  animations (moritzbrantner/asset-tooling#595). The shooter owns simulation, loadouts, physics
  semantics, placement, rendering and game modes. Genuine foundation gaps go to their owners
  (D4.1).
- **D5.i70.1** (owner, 2026-11-13; shooter#70, tracker shooter#45): One issue owns each robot beta
  capability: the canonical decisions shooter#70; the socket-grid assembly, the control core's
  placement and the shipped schema's migration shooter#71 (D2.i70.1, D2.i70.2, D2.i70.12); shared
  energy, batteries and the removal of heat shooter#72 (D2.i70.3, D2.i70.4); compound bodies,
  motors and self-contained legs shooter#73 (D2.i70.5); bindings and action groups shooter#74
  (D2.i70.7); collision damage and debris shooter#75 (D2.i70.10); per-part damage, armor,
  penetration and the core kill shooter#56 (D2.i70.2, D2.i70.8), the root chassis as the vital
  part instead of a core shooter#89 (D2.i89.1); the builder and respawn flow
  shooter#57 (D2.i70.11); the locomotion set shooter#58 (D2.i70.6); field repair shooter#61
  (D2.i70.9); contested pickups shooter#62. shooter#63 is the integration gate that proves them
  together; it owns no capability itself.

## Questions

- **Q1**: Game modes beyond deathmatch (team deathmatch, capture the flag, …). Answered by D2.7.
- **Q2**: Bot skill and behavior beyond "fills empty slots and is beatable". Answered by D2.8.
- **Q3**: Weapon count and how distinct each one is; which melee weapons (energy blade, hammer, …);
  whether melee is a separate slot or part of the rotation.
  Defaults in use until decided (shooter#6): one melee weapon, an energy blade, as a quick melee
  on its own key (V, or the mouse's back button) usable whatever gun is held, not a rotation slot.
  50 damage, a swing every 0.5 s, the gun ready 0.3 s after a swing, a 6 m/s shove; mmorpg's player
  swing reach (2.5 m), arc (50° either side) and height band (1 m below to 2 m above the
  feet), striking every fighter it covers in line of sight.
- **Q4**: Grapple details: pull versus swing, range, cooldown, whether it can hook players.
  Defaults in use until decided (shooter#5): pull and swing together (a reel toward the hook
  while the rope swings), 30 m range, 0.75 s between hooks, arena geometry only (no players).
- **Q2.i46.1**: Which team or objective mode comes first, and in which beta phase (D2.i46.3).
- **Q2.i46.2**: What repairs cost or consume (resources, time, pickups) and how a player repairs
  during combat (D2.i46.6).
  Answered by D2.i70.9; its tuning stays open as Q2.i70.3.
- **Q2.i46.3**: How long pickup modules last once taken, and when they respawn (D2.i46.9).
  Owner, 2026-11-14 (shooter#62): a collected module lasts until its holder dies or the round
  ends, not on a timer, and a strong one is paid for in shared robot energy (#72) and/or limited
  uses; values are tuning, not approved. Open: the module list, costs, respawn intervals, any
  drop on death. Defaults in use until decided (shooter#62): `shooter_sim::pickups`, pads as map
  data (`Arena::module_pads`); the rail capacitor (rail gun 150% damage, 75% refire time, 5
  boosted shots) and thrusters (9 m/s jumps instead of 6, 6 boosted jumps); one module per robot,
  a new one replaces it; since shooter#72 a boosted rail shot costs 150% of the shot's energy and
  a thruster jump 15 energy (a jump the pool cannot pay is a plain one); nothing dropped on death; a
  pad is back 30 s after it was taken and every pad is ready at a round's or map's start; a pad
  contested in one tick goes to the nearest living robot, then the lowest slot.
- **Q2.i46.4**: The exact engineering budgets (weight, power, cooling), attachment rules and build
  point values (D2.i46.5).
  Defaults in use until decided (shooter#47): provisional, tunable catalog data in
  `shooter_sim::robot::BETA`. Sockets for torso, head, two arms and locomotion (torso, head and
  locomotion required); a hand hardpoint per arm and two shoulder hardpoints on the torso, each
  light or heavy; the blade mounts only on a hand; a weapon mounts at most once. The locomotion
  carries the mass of everything else, the torso supplies the power everything draws, the parts'
  cooling sheds the weapons' heat, and a build costs at most 120 build points. Two of each part
  (light and heavy) and the eight shipped weapons, all available from the start.
  Weapon mods (shooter#51, D2.i46.8), same catalog: at most one mod per mounted weapon, costing
  points, mass, power and heat or adding cooling, fitting named weapons only, and tuning its
  weapon's damage, refire and range in percent: rapid cycler, overcharger, long barrel (makes its
  weapon heavy) and heat sink. Every mod that changes the fire gives up damage, refire or range.
  Until the builder (shooter#57), people pick a named loadout (`robot::LOADOUTS`: standard,
  heavy, skirmisher, marksman; pyro since shooter#96), sent through the existing build request.
  Since shooter#57 they are where the builder starts from (D2.i70.11).
  Attachment rules answered by D2.i70.1 and D2.i70.2, cooling superseded by D2.i70.4; the numbers
  remain open as Q2.i70.1 and Q2.i70.2. The defaults above stay shipped until migrated
  (D2.i70.12). The sockets, hardpoints and load budget were migrated by shooter#71; the power and
  heat budgets by shooter#72, which also rebalanced the mods on energy (provisional): a mod costs
  points and mass only and tunes damage, refire, range and energy a shot; rapid cycler 75%
  damage, 65% refire time (half again the energy a second); overcharger 125% damage, 130% refire
  time, 150% energy a shot; long barrel unchanged; the heat sink (code 4, retired) replaced by
  the efficiency coil (code 5: rifle, rail gun, rockets, shotgun; 60% energy a shot, 125% refire
  time, 6 kg, 5 points). Loadouts: the heavy build gains a battery (115 points), the skirmisher's
  shotgun carries the efficiency coil instead of the heat sink, standard and marksman unchanged.
- **Q2.i46.5**: Layout details of the three arenas: topology, destructible pieces, pickup points
  and each map's player capacity (D2.i46.11, D2.i46.12).
  Defaults in use until decided (shooter#48, #82): the map catalog (`shooter_sim::maps`) holds
  two interim greybox maps, the default `greybox-yard` (76 m, a raised walkway, recommending 6 to
  12 players, 14 spawn points; owner request #82) and the 40 m `greybox`, the test and A/B
  fixture (2 to 12 players, 13 spawn points); a map's recommended capacity does not change the
  slot count. Since shooter#50 a deathmatch has 12 slots unless the host sets another count per
  match (`SHOOTER_MATCH_IDS=<id>=<map>:<slots>`), bounded by one slot per spawn point of the map;
  no global maximum (D2.i46.12). Since shooter#52 the medium arena is in: `assembly-works`, a
  56 m multilevel robotic assembly factory (owner decisions on #52) built from the arena kit
  (4 to 12 players, 14 spawn points), point-symmetric with mezzanines and a gantry bridge at 3 m
  and crane towers at 5 m; its name, size, layout, pickup and repair anchors and destruction
  candidates (its low cover, barriers and crates) are provisional choices, to be tuned in
  playtests. The yard stays the default.
- **Q2.i70.1**: The numeric budgets of the beta contract: the build-point cap, the technical
  part-count ceiling, each part's points, mass and health, motor torque and speed, the core's
  capacity and regeneration, battery capacity, action energy costs and cooldowns, and penetration
  values (D2.i70.1, D2.i70.3, D2.i70.5, D2.i70.8). Implementations record provisional defaults
  here; none is owner-approved.
  Defaults in use until decided (shooter#71, `shooter_sim::robot::BETA`): 120 build points and
  a ceiling of 32 parts. Points: chassis 10, 20 and 30 by size, beams 2 and 3, hinge 5, rotor 6,
  core 10, sensor 5, leg 7, wheel 6, track 12, armor plate 4; the weapons and mods keep their
  shipped points. Energy since shooter#72 (`shooter_sim::energy`, milli-unit fixed point): the
  core stores 100 and regenerates 24 a second (0.4 a tick); a battery (`robot.battery`, 20 × 40 ×
  20 cm, 20 kg, 5 points) stores 60. Activations: rifle bolt
  2, rail shot 30, rocket 20, grapple hook 10, blade swing 5, shotgun blast 12, laser 0.6 and
  flamethrower 0.5 a tick; cooldowns are the weapons' shipped refire times.
  Part damage since shooter#56 (`shooter_sim::damage`): component health chassis 120, 160 and
  200 by size, beams 50 (short) and 60 (long), hinge and rotor 40, core 100, battery 60, sensor
  30, leg 70, wheel 50, track 90, armor plate 120, light weapons 40 and heavy ones (rail gun,
  rocket launcher) 60. Penetration, the share of a strike's damage an armor plate passes to the
  part behind it: rail gun 50 %, energy rifle and laser 20 %, every other weapon 0. The weapons'
  damage is unchanged (rifle 12, rail 85, rocket splash up to 100 within 3 m, pellets 9, laser
  2 and flame 3 a tick, blade 50); the old 200 fighter health is gone.
  Since shooter#89 (D2.i89.1) there is no core: 110 build points (120 less the core's 10, so every
  build keeps its headroom); the root chassis stores the core's 100 and regenerates its 24 a
  second (other chassis sections nothing); chassis health 270, 290 and 310 by size (was 120, 160
  and 200), within the (255, 318] that keeps a front rail kill at four unmodded hits (85) or three
  overcharged ones (106), as when the chassis and then the core (100) had to break. A rail
  capacitor kill (127 a hit) now takes three hits instead of two.
- **Q2.i70.2**: The predetermined chassis sizes and their costs, and whether the 20 cm socket cell
  is final (D2.i70.1).
  Defaults in use until decided (shooter#71): a 20 cm cell; chassis sections of 20 × 40 × 80,
  40 × 40 × 80 and 40 × 60 × 80 cm (28, 40 and 52 sockets); beams of 20 × 20 × 40 and 80 cm.
  Orientations are binary angles (65 536 a turn, so quarter, eighth and finer binary turns are
  exact) in one canonical yaw-pitch-roll form; a footprint turns with the yaw's nearest quarter
  turn and may overhang its face's edge. Overlaps are checked per face, not between parts in
  space.
- **Q2.i70.3**: Repair targeting and range: the module's radius, the target-selection gesture, the
  repair rate, energy cost and cooldown, and whether incoming damage interrupts a repair
  (D2.i70.9).
- **Q2.i70.4**: The details of first-come-first-served energy requests: how requests in the same
  tick are ordered, how continuous motor draws and one-off activations share a tick, and whether
  a request can be granted in part (D2.i70.3).
  Defaults in use until decided (shooter#72): every robot has its own pool, so robots never
  compete; a robot's requests are granted in the order its tick resolves them, a thruster jump
  (during movement), then the blade swing, then the gun (a refused swing leaves the gun its
  turn); regeneration follows every request of the tick. A request is granted whole or refused,
  never in part, and a refused one costs nothing and spends no cooldown. Motor draw (shooter#73)
  and repairs (shooter#61) are not ordered yet.
- **Q2.i70.5**: Impact thresholds: the speed, impulse and fall heights at which collisions start to
  damage, how damage scales above them, and the battery blast's radius and damage (D2.i70.3,
  D2.i70.10).
  Defaults in use until decided (shooter#56): a destroyed battery's blast reaches 80 units
  (0.8 m) from its centre and deals 40 there, falling off linearly to nothing at its edge, to
  every connected part of every robot within reach; it pushes nothing.
- **Q2.i70.6**: Performance limits: parts per robot, articulated bodies and active debris per match,
  debris lifetime, and the representative multi-robot scenes they are measured on (D2.i70.1,
  D2.i70.5, D2.i70.10).
