//! The shooter's deterministic simulation, shared by the native and browser clients: the greybox
//! arena and how a player walks, looks and jumps in it. Physics is `sim-foundation`'s world
//! (physics-engine; Rapier only as the A/B oracle under the `physics-ab` feature).

pub mod arena;
pub mod player;
pub mod tour;

pub use arena::{Arena, Piece};
pub use player::{Intent, Player, Session, StepReport};
pub use sim_foundation::{self, TICK_HZ, physics::SUBUNITS_PER_UNIT};
