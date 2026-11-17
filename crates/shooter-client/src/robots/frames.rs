//! Where the simulation has a robot's bodies (shooter#54, shooter#73): the authored frame of each
//! of its mechanical robot's bodies in the world (`shooter_sim::mechanics`), read from the exact
//! state a snapshot carries ([`Frames::of_state`]) or from a running [`Mechanics`] (the local
//! prediction, [`Frames::of_robot`]), and which bodies carry each connected part. The drawing
//! ([`super::pose`]) places every part on its body and turns legs, wheels, tracks and joints by
//! how their bodies sit against the body they hang from: the gait's stride, the wheels' roll, a
//! legless robot lying on its chassis are the simulation's, not the renderer's.
//!
//! A body's authored frame is the robot's frame (+X forward, +Y up, metres here) as that body
//! has moved: at rest every body's frame is the robot's.

use crate::scene::METRES_PER_UNIT;
use render_engine::glam::{Mat4, Quat, Vec3};
use shooter_sim::Assembly;
use shooter_sim::mechanics::{BodyKey, Layout, Mechanics, RobotState};
use shooter_sim::physics::articulated::{Motion, Pose, origin_of};
use shooter_sim::robot::PartId;

/// The bodies of one connected part.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartBodies {
    pub part: PartId,
    /// The body it hangs from: its own for a rigid part (and for a joint's box), the body its
    /// mount turns against for a leg, wheel or track.
    pub base: usize,
    /// The bodies that turn against `base`: a leg's thigh then shin, a wheel, a track's road
    /// wheels, the body a joint's top face carries.
    pub moving: Vec<usize>,
}

/// One robot's bodies in the world.
#[derive(Clone, Debug, PartialEq)]
pub struct Frames {
    pub build: Assembly,
    pub lost: u32,
    /// The point the robot is drawn around (metres): its fighter's centre, where the match
    /// had it at the state's tick.
    pub center: Vec3,
    /// Each body's authored frame in the world (metres), in [`Layout::bodies`] order.
    pub bodies: Vec<(Vec3, Quat)>,
    /// Each connected part's bodies, in part-ID order.
    pub parts: Vec<PartBodies>,
}

fn frame(pose: Pose) -> (Vec3, Quat) {
    (
        Vec3::from(pose.position) * METRES_PER_UNIT,
        Quat::from_array(pose.orientation).normalize(),
    )
}

impl Frames {
    /// The frames of `bodies` laid out as `layout` (the connected bodies of `build` without
    /// `lost`), drawn around `center` (metres). `None` if they do not match.
    #[must_use]
    pub fn new(
        build: Assembly,
        lost: u32,
        layout: &Layout,
        bodies: &[Motion],
        center: Vec3,
    ) -> Option<Self> {
        if bodies.len() != layout.bodies.len() {
            return None;
        }
        let bodies = layout
            .bodies
            .iter()
            .zip(bodies)
            .map(|(plan, motion)| origin_of(&plan.children, motion).ok().map(frame))
            .collect::<Option<Vec<_>>>()?;
        let mut parts: Vec<PartBodies> = Vec::new();
        for segment in &layout.segments {
            match parts.last_mut() {
                Some(last) if last.part == segment.part => {
                    if !last.moving.contains(&segment.body) && last.base != segment.body {
                        last.moving.push(segment.body);
                    }
                }
                _ => {
                    let moving_part = matches!(
                        layout.bodies[segment.body].key,
                        BodyKey::Moving(part, _) if part == segment.part
                    );
                    let (base, moving) = if moving_part {
                        // A leg, wheel or track: it turns against the body its hinge hangs on.
                        let base = layout
                            .hinges
                            .iter()
                            .find(|hinge| hinge.b == segment.body)
                            .map_or(0, |hinge| hinge.a);
                        (base, vec![segment.body])
                    } else {
                        // A rigid part; a joint also turns the body its top face carries.
                        let carried = layout
                            .bodies
                            .iter()
                            .position(|body| body.key == BodyKey::Rigid(Some(segment.part)))
                            .filter(|carried| *carried != segment.body);
                        (segment.body, carried.into_iter().collect())
                    };
                    parts.push(PartBodies {
                        part: segment.part,
                        base,
                        moving,
                    });
                }
            }
        }
        Some(Self {
            build,
            lost,
            center,
            bodies,
            parts,
        })
    }

    /// A robot's exact state (a snapshot's), its fighter's centre at `center` (metres).
    #[must_use]
    pub fn of_state(state: &RobotState, center: Vec3) -> Option<Self> {
        Self::new(
            state.build,
            state.lost,
            &state.layout(),
            &state.bodies,
            center,
        )
    }

    /// The robot in `slot` of `mechanics` as it stands now, its fighter's centre at `center`
    /// (metres).
    #[must_use]
    pub fn of_robot(mechanics: &Mechanics, slot: u8, center: Vec3) -> Option<Self> {
        let robot = mechanics.robot(slot)?;
        let bodies = (0..robot.layout.bodies.len())
            .map(|index| mechanics.world().motion(robot.body(index)))
            .collect::<Option<Vec<_>>>()?;
        Self::new(robot.build, robot.lost, &robot.layout, &bodies, center)
    }

    /// The same bodies drawn with the centre at `center` (metres): the robot moved whole.
    #[must_use]
    pub fn at(&self, center: Vec3) -> Self {
        let by = center - self.center;
        Self {
            center,
            bodies: self
                .bodies
                .iter()
                .map(|(position, rotation)| (*position + by, *rotation))
                .collect(),
            ..self.clone()
        }
    }

    /// `t` (0..1) of the way from `self` to `to`: each body between the two where both have
    /// the same bodies, else the nearer of the two.
    #[must_use]
    pub fn blend(&self, to: &Self, t: f32) -> Self {
        if self.build != to.build || self.lost != to.lost || self.bodies.len() != to.bodies.len() {
            return if t < 0.5 { self.clone() } else { to.clone() };
        }
        Self {
            center: self.center.lerp(to.center, t),
            bodies: self
                .bodies
                .iter()
                .zip(&to.bodies)
                .map(|((from, a), (to, b))| (from.lerp(*to, t), a.slerp(*b, t)))
                .collect(),
            ..self.clone()
        }
    }

    /// Body `index`'s frame: the world from its robot frame (metres).
    #[must_use]
    pub fn body(&self, index: usize) -> Mat4 {
        self.bodies.get(index).map_or(Mat4::IDENTITY, |(at, turn)| {
            Mat4::from_rotation_translation(*turn, *at)
        })
    }

    /// The root body's frame: where the robot as a whole is.
    #[must_use]
    pub fn root(&self) -> Mat4 {
        self.body(0)
    }

    /// Part `part`'s bodies, while it is connected.
    #[must_use]
    pub fn part(&self, part: PartId) -> Option<&PartBodies> {
        self.parts.iter().find(|bodies| bodies.part == part)
    }

    /// How body `to` sits against body `from`, in the robot's frame at rest: the turn of the
    /// hinge between them about its anchor (identity at the authored pose).
    #[must_use]
    pub fn relative(&self, from: usize, to: usize) -> Mat4 {
        self.body(from).inverse() * self.body(to)
    }
}
