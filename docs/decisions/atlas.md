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
- **D2.i46.6** (owner, 2026-11-13): Limbs are targetable and detachable; losing one changes the
  robot's actual abilities and movement in physically believable, authored ways. Robots can be
  repaired during combat (how, Q2.i46.2); respawn restores the configured build. Fully emergent
  physics-based self-balancing is later, not beta.
- **D2.i46.7** (owner, 2026-11-13): After death and before respawning a player may substantially
  reconfigure the robot; while alive, only repairs, not rebuilds.
- **D2.i46.8** (owner, 2026-11-13): Weapons build on the shipped rifle, rail gun, rockets, shotgun,
  laser, flamethrower, grapple hook and blade, and allow weapon modifications. Full
  weapon-from-parts engineering is later, not beta.
- **D2.i46.9** (owner, 2026-11-13): Some powerful weapon and movement modules spawn as pickups at
  contested arena points. Any robot can use them, with no compatibility gate; a build may exploit
  one better (for example grapple plus a sniper position). Their lifetime is open (Q2.i46.3).
- **D2.i46.10** (owner, 2026-11-13): The beta has more than one locomotion configuration (for
  example legs and wheels or tracks). Exotic machines (quadruped, hovering, spider) are later.
- **D2.i46.11** (owner, 2026-11-13): Three initial arenas, small, medium and large, with some
  destructible cover, doors and machinery around a stable core topology. Maps differ in combat
  opportunities and player capacity (layouts open, Q2.i46.5).
- **D2.i46.12** (owner, 2026-11-13): About 12 player slots by default, scalable by map and mode.
  Not a commitment to a 32-player beta or to a global maximum.
- **D2.i46.13** (owner, 2026-11-13): No story in the beta. Cosmetic unlocks, elaborate salvage from
  defeated robots, extensive arena destruction and advanced progression are post-beta.

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
- **Q2.i46.5**: Layout details of the three arenas: topology, destructible pieces, pickup points
  and each map's player capacity (D2.i46.11, D2.i46.12).
  Defaults in use until decided (shooter#48): the map catalog (`shooter_sim::maps`) holds only
  the greybox, a test and A/B fixture recommending 2 to 12 players; a map's recommended capacity
  does not change the slot count. Since shooter#50 a deathmatch has 12 slots unless the host sets
  another count per match (`SHOOTER_MATCH_IDS=<id>=<map>:<slots>`), bounded by one slot per
  spawn point of the map (the greybox has 13); no global maximum (D2.i46.12).
