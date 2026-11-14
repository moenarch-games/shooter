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
  Superseded in part by D2.i70.2 (the core is the only kill), D2.i70.8 (damage and permanent
  destruction) and D2.i70.9 (repair).
- **D2.i46.7** (owner, 2026-11-13): After death and before respawning a player may substantially
  reconfigure the robot; while alive, only repairs, not rebuilds.
  Respawn flow refined by D2.i70.11.
- **D2.i46.8** (owner, 2026-11-13): Weapons build on the shipped rifle, rail gun, rockets, shotgun,
  laser, flamethrower, grapple hook and blade, and allow weapon modifications. Full
  weapon-from-parts engineering is later, not beta.
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
2026-11-13 (choices 01–29; shooter#70, tracker shooter#45) and the owner's 2026-11-14 follow-ups on
shooter#57 and shooter#61. They are the one intended beta contract; D2.i70.12 says what ships today
and which issue migrates it. Each capability has one owning issue (D5.i70.1).

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
- **D2.i70.2** (owner, 2026-11-13; shooter#70): Every robot has exactly one control core, on an
  exterior socket of the player's choice. Armor may cover it (penetration exists, D2.i70.8); there
  is no exposed-face check. Destroying the core, and only that, eliminates the robot: losing
  weapons, wheels, legs or batteries can incapacitate it but not kill it, and an immobile robot
  whose core survives stays alive. Placement and validation: shooter#71; the kill condition:
  shooter#56.
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
- **D2.i70.4** (owner, 2026-11-13; shooter#70; supersedes D2.i46.5's cooling constraint): The beta
  has no heat or cooling mechanic: weapons and parts are limited by energy (D2.i70.3) and
  cooldowns, not heat accumulation or cooling modules. The heat and cooling that ship today stay
  until an explicit migration replaces them (D2.i70.12, shooter#72).
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
- **D2.i70.12** (owner, 2026-11-13; shooter#70): What ships today is the baseline, not the beta
  contract: `shooter_sim::robot` (shooter#47) with its anatomical torso, head, arm and locomotion
  sockets, hand and shoulder hardpoints, named loadouts and its mass, power, heat and cooling
  budgets, and the weapon mods (shooter#51), including the heat sink and the heat the other mods
  add; the defaults are recorded under Q2.i46.4 and described in the README. They stay truthful
  until replaced, and replacing them is an explicit migration of the one schema, never a second
  assembly system beside it: shooter#71 migrates the assembly, the named loadouts and the mods'
  mounting; shooter#72 removes heat and cooling and rebalances the heat-related mods.

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
  penetration and the core kill shooter#56 (D2.i70.2, D2.i70.8); the builder and respawn flow
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
  heavy, skirmisher, marksman), sent through the existing build request.
  Attachment rules answered by D2.i70.1 and D2.i70.2, cooling superseded by D2.i70.4; the numbers
  remain open as Q2.i70.1 and Q2.i70.2. The defaults above stay shipped until migrated
  (D2.i70.12).
- **Q2.i46.5**: Layout details of the three arenas: topology, destructible pieces, pickup points
  and each map's player capacity (D2.i46.11, D2.i46.12).
  Defaults in use until decided (shooter#48): the map catalog (`shooter_sim::maps`) holds only
  the greybox, a test and A/B fixture recommending 2 to 12 players; a map's recommended capacity
  does not change the slot count. Since shooter#50 a deathmatch has 12 slots unless the host sets
  another count per match (`SHOOTER_MATCH_IDS=<id>=<map>:<slots>`), bounded by one slot per
  spawn point of the map (the greybox has 13); no global maximum (D2.i46.12).
- **Q2.i70.1**: The numeric budgets of the beta contract: the build-point cap, the technical
  part-count ceiling, each part's points, mass and health, motor torque and speed, the core's
  capacity and regeneration, battery capacity, action energy costs and cooldowns, and penetration
  values (D2.i70.1, D2.i70.3, D2.i70.5, D2.i70.8). Implementations record provisional defaults
  here; none is owner-approved.
- **Q2.i70.2**: The predetermined chassis sizes and their costs, and whether the 20 cm socket cell
  is final (D2.i70.1).
- **Q2.i70.3**: Repair targeting and range: the module's radius, the target-selection gesture, the
  repair rate, energy cost and cooldown, and whether incoming damage interrupts a repair
  (D2.i70.9).
- **Q2.i70.4**: The details of first-come-first-served energy requests: how requests in the same
  tick are ordered, how continuous motor draws and one-off activations share a tick, and whether
  a request can be granted in part (D2.i70.3).
- **Q2.i70.5**: Impact thresholds: the speed, impulse and fall heights at which collisions start to
  damage, how damage scales above them, and the battery blast's radius and damage (D2.i70.3,
  D2.i70.10).
- **Q2.i70.6**: Performance limits: parts per robot, articulated bodies and active debris per match,
  debris lifetime, and the representative multi-robot scenes they are measured on (D2.i70.1,
  D2.i70.5, D2.i70.10).
