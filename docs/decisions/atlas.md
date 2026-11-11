# Decision Atlas

The canonical map of product intent for the shooter. Leaf entries are Decisions (approved by the
owner) and Questions (open). Answered entries are never rewritten; a later Decision links to and
supersedes them. New entries take an issue-derived ID (parent ID + `i` + issue number, e.g.
`3.i12`) so parallel PRs never collide.

## 1. Purpose

- **D1.1** (owner, 2026-11-11): The shooter exists both as a game and as an additional, demanding
  consumer of the owner's foundation crates, so they gather more real-world data. It must never
  block other projects: it gets worker time only when another lane is idle.

## 2. Experience

- **D2.1** (owner, 2026-11-11): First-person arena shooter.
- **D2.2** (owner, 2026-11-11): First playable: online PvP deathmatch on one arena, hosted through
  game-server, with bots filling empty slots. Bots also make it playable and testable alone.
- **D2.3** (owner, 2026-11-11): Fast arena movement: sprint, slide, double jump, jump pads, rocket
  jumping and air control. No aim-down-sights; large health pools.
- **D2.4** (owner, 2026-11-11): Stylized sci-fi look: a clean modular arena (panels, ramps, jump
  pads, glowing trims), energy weapons (energy rifle, rail gun, rockets). No realistic gore.
- **D2.5** (owner, 2026-11-11): One of the guns is a **grapple hook**: fire it at arena geometry and
  get pulled or swing toward the hit point. It is a movement tool in the weapon rotation.
- **D2.6** (owner, 2026-11-11): Players also have **melee weapons** alongside the guns.

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

## 5. Repository

- **D5.1** (owner, 2026-11-11): Lives at `moenarch-games/shooter`, public, with Pages for the
  browser client. Art is authored through asset-tooling (CC0 first; Mixamo only as inspiration).

## Questions

- **Q1**: Game modes beyond deathmatch (team deathmatch, capture the flag, …). Not yet decided.
- **Q2**: Bot skill and behavior beyond "fills empty slots and is beatable".
- **Q3**: Weapon count and how distinct each one is; which melee weapons (energy blade, hammer, …);
  whether melee is a separate slot or part of the rotation.
- **Q4**: Grapple details: pull versus swing, range, cooldown, whether it can hook players.
  Defaults in use until decided (shooter#5): pull and swing together (a reel toward the hook
  while the rope swings), 30 m range, 0.75 s between hooks, arena geometry only (no players).
