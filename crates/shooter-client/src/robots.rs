//! The fighters drawn as their robot builds from asset-tooling's modular robot kit (Atlas
//! D5.i46.1, shooter#54): the pinned kit read and decoded ([`kit`], [`load`]), each validated
//! build laid out where the simulation places its parts ([`model`]), posed every frame from the
//! fighter's state ([`pose`]) and drawn in render-engine's world ([`draw`]). Parts the kit does
//! not make yet (the weapons, wheels and tracks) are labelled placeholders; a missing animation
//! is marked, never a silent dummy.

pub mod draw;
pub mod kit;
pub mod load;
pub mod model;
pub mod pose;
pub mod track;

pub use draw::{Drawn, RobotView, Robots};
pub use pose::{PartStates, RobotState};
