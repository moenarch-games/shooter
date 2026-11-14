//! The fighters drawn as their robot builds from asset-tooling's kits (Atlas D5.i46.1,
//! shooter#54): the pinned robot part and weapons kits read and decoded ([`kit`], [`load`]), each
//! validated build laid out where the simulation places its parts, with each weapon's mod on its
//! anchor ([`model`]), posed every frame from the fighter's state ([`pose`], its weapons'
//! states and glow channels in [`weapon`]) and drawn in render-engine's world ([`draw`]). Parts
//! the kits do not make yet (the wheels and tracks) are labelled placeholders; a missing
//! animation is marked, never a silent dummy.

pub mod draw;
pub mod kit;
pub mod load;
pub mod model;
pub mod pose;
pub mod track;
pub mod weapon;

pub use draw::{Drawn, RobotView, Robots};
pub use pose::{Muzzle, PartStates, RobotState};
