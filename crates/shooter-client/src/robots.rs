//! The fighters drawn as their robot builds from asset-tooling's kits (Atlas D5.i46.1,
//! shooter#54): the pinned robot part, weapons and locomotion kits read and decoded ([`kit`],
//! [`load`]), each validated build laid out where the simulation places its parts, with each
//! weapon's mod on its anchor and each wheel's and track's axle where the simulation turns it
//! ([`model`]), posed every frame where the simulation has its mechanical robot's bodies
//! ([`frames`], [`pose`]; its weapons' states and glow channels in [`weapon`]) and drawn in
//! render-engine's world ([`draw`]). Every catalog part is a kit model; a missing animation is
//! marked, never a silent dummy.

pub mod draw;
pub mod frames;
pub mod kit;
pub mod load;
pub mod model;
pub mod pose;
pub mod track;
pub mod weapon;

pub use draw::{Drawn, RobotView, Robots};
pub use frames::Frames;
pub use pose::{Muzzle, PartStates, RobotState};
