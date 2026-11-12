//! The shooter's deterministic simulation, shared by the native and browser clients and the
//! game-server host: the greybox arena, how a fighter moves in it (walk, sprint, slide, double
//! jump, jump pads, air control, rocket jumps), the energy weapons, health, deaths, respawns and
//! the deathmatch score, the bots, and the match on the wire. Physics is physics-engine through
//! [`physics`] (Rapier only as the A/B oracle under the `physics-ab` feature).

pub mod arena;
pub mod bots;
pub mod dummies;
pub mod fixed;
pub mod grapple;
pub mod melee;
pub mod names;
pub mod physics;
pub mod player;
pub mod rng;
pub mod scenarios;
pub mod session;
pub mod tour;
pub mod trig;
pub mod weapons;
pub mod wire;

pub use arena::{Arena, JumpPad, Piece, Post, Spawn};
pub use fixed::{SUBUNITS_PER_UNIT, TICK_HZ};
pub use player::{Intent, Player};
pub use session::{Event, Fighter, Input, Phase, Pilot, Rules, Session, StepReport};
pub use weapons::Weapon;
pub use wire::{Controller, FighterState, Snapshot};
