//! The shooter's deterministic simulation, shared by the native and browser clients and the
//! game-server host: the map catalog ([`maps`]) and its arenas, how a fighter moves in it (walk, sprint, slide, double
//! jump, jump pads, air control, rocket jumps), the energy weapons and where their shots land ([`aim`]), the shared robot energy pool ([`energy`]), health, deaths, respawns and
//! the deathmatch score, the bots, the robot builds ([`robot`]), the contested module pickups ([`pickups`]) and the
//! match on the wire. Physics is physics-engine through
//! [`physics`] (Rapier only as the A/B oracle under the `physics-ab` feature).

pub mod aim;
pub mod arena;
pub mod bots;
pub mod dummies;
pub mod energy;
pub mod fixed;
pub mod grapple;
pub mod maps;
pub mod melee;
pub mod names;
pub mod physics;
pub mod pickups;
pub mod player;
pub mod rng;
pub mod robot;
pub mod scenarios;
pub mod session;
pub mod tour;
pub mod trig;
pub mod weapons;
pub mod wire;

pub use arena::{Arena, JumpPad, Piece, Post, Spawn};
pub use fixed::{SUBUNITS_PER_UNIT, TICK_HZ};
pub use maps::{Capacity, MapId, UnknownMap};
pub use pickups::{ArenaModule, Carried, ModulePad};
pub use player::{Intent, Player};
pub use robot::{Assembly, AssemblyError, Candidate};
pub use session::{Event, Fighter, Input, Phase, Pilot, Rules, Session, StepReport};
pub use weapons::Weapon;
pub use wire::{Controller, FighterState, Snapshot};
