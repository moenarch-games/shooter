//! The shooter's deterministic simulation, shared by the native and browser clients: the greybox
//! arena and how a player moves in it (walk, sprint, slide, double jump, jump pads, air control).
//! Physics is `sim-foundation`'s world (physics-engine; Rapier only as the A/B oracle under the
//! `physics-ab` feature).

pub mod arena;
pub mod player;
pub mod scenarios;
pub mod tour;

pub use arena::{Arena, JumpPad, Piece};
pub use player::{Intent, Player, Session, StepReport};
pub use sim_foundation::{self, TICK_HZ, physics::SUBUNITS_PER_UNIT};
