//! The shooter's deterministic simulation, shared by the native and browser clients and the
//! game-server host: the greybox arena, how a fighter moves in it (walk, sprint, slide, double
//! jump, jump pads, air control, rocket jumps), the energy weapons, health, deaths, respawns and
//! the deathmatch score, the bots, and the match on the wire. Physics is `sim-foundation`'s world
//! (physics-engine; Rapier only as the A/B oracle under the `physics-ab` feature).

pub mod arena;
pub mod bots;
pub mod dummies;
pub mod grapple;
pub mod player;
pub mod scenarios;
pub mod session;
pub mod tour;
pub mod weapons;
pub mod wire;

pub use arena::{Arena, JumpPad, Piece, Post, Spawn};
pub use player::{Intent, Player};
pub use session::{Event, Fighter, Input, Phase, Pilot, Rules, Session, StepReport};
pub use sim_foundation::{self, TICK_HZ, physics::SUBUNITS_PER_UNIT};
pub use weapons::Weapon;
pub use wire::{Controller, FighterState, Snapshot};
