//! The shooter's deterministic simulation, shared by the native and browser clients: the greybox
//! arena, how a player moves in it (walk, sprint, slide, double jump, jump pads, air control,
//! rocket jumps), the energy weapons and the target dummies they shoot at. Physics is
//! `sim-foundation`'s world (physics-engine; Rapier only as the A/B oracle under the
//! `physics-ab` feature).

pub mod arena;
pub mod dummies;
pub mod player;
pub mod scenarios;
pub mod session;
pub mod tour;
pub mod weapons;

pub use arena::{Arena, JumpPad, Piece, Post};
pub use player::{Intent, Player};
pub use session::{Event, Session, StepReport};
pub use sim_foundation::{self, TICK_HZ, physics::SUBUNITS_PER_UNIT};
pub use weapons::Weapon;
