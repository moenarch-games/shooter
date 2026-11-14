//! Per-part damage (Atlas D2.i89.1, D2.i70.8): every part of a robot's build has its own
//! health, strikes land on the parts they meet, a part at zero is destroyed and falls away with
//! everything that hung from it, and the root chassis is the only kill.
//!
//! **State** ([`PartDamage`]): one health a part (in the build's part-ID order) and which parts
//! are lost. A part works fully until its health reaches zero (no gradual degradation); at zero
//! it is destroyed for the rest of that life. Lost means not connected to the root chassis any
//! more: the parts reachable from the root through the single-parent tree without crossing a
//! destroyed part stay; the rest are lost, destroyed or not ([`PartDamage::lost`]). That is the
//! subtree below the destroyed part; when the root chassis itself is destroyed, everything is
//! lost and the robot is dead. Lost is derived from the healths alone, so the wire carries the
//! healths only. Nothing comes back mid-life: health only goes down, and respawn restores the configured build
//! ([`PartDamage::fresh`]).
//!
//! **Hitboxes** ([`Hitboxes`]). The match's fighters still move on the capsule controller
//! (shooter#73's second half moves them onto [`crate::mechanics`]), so a fighter's parts are
//! placed at rest where [`crate::mechanics::placements`] puts them in the robot's frame, the
//! robot's lowest point on the capsule's feet, turned to face the fighter's yaw (+X forward,
//! like the drawn robot). Each connected part is an oriented box: armor plates are real boxes
//! like the rest. Lag-compensated strikes place them at the rewound centre and yaw. The capsule
//! still collides with the arena; strikes meet the part boxes, not the capsule, so a shot through
//! a gap between the legs flies on.
//!
//! **Resolution**, deterministic and once per strike:
//! - A ray (hitscan beam, pellet, rocket in flight) meets the boxes in order along it (ties by
//!   part ID). The first non-armor part it meets takes all of the damage left and stops it. An
//!   armor plate stops it too, unless the weapon penetrates ([`Weapon::penetration`]): then the
//!   plate takes the rest and that percentage of the damage (rounded down) goes on to the next
//!   part along the ray ([`Hitboxes::pierce`]). Each part is struck at most once.
//! - A blast (a rocket's splash, a battery's) damages every connected part whose box comes
//!   within its radius, by the falloff to the box's nearest point ([`Hitboxes::blast`]), in
//!   part-ID order; armor does not shelter what is behind it.
//! - The blade and the flame strike the one connected part nearest the attacker's eye (the
//!   part's nearest point; ties by part ID, [`Hitboxes::nearest`]).
//!
//! **Consequences** (applied by [`crate::session`]): the root chassis at zero kills the robot (the only
//! kill; a fall out of the arena still ends a life); a fighter holds, fires and swings only the
//! weapons its build mounts and still has ([`PartDamage::armed`]): a lost weapon part no longer
//! fires and cannot be selected, nor can a weapon the build does not carry (shooter#92); a lost
//! battery takes its capacity and its share of the charge with it, once, and a destroyed one
//! sets off a small blast ([`BATTERY_BLAST_RADIUS_UNITS`], [`BATTERY_BLAST_DAMAGE`]) that
//! damages nearby parts through the same resolution (another battery it destroys blasts in turn,
//! at most once each; no chain-reaction subsystem); and a robot with no connected locomotion
//! part (legs, wheels, tracks) cannot walk, jump or slide ([`PartDamage::mobile`]). Losing some
//! of its legs or wheels changes nothing yet: how the surviving ones move it is the mechanical
//! robots' (shooter#73's second half); the capsule invents no fallback.
//!
//! The part healths (the catalog's, [`crate::robot::PartType::health`]), the penetration and the
//! battery blast are provisional (Atlas Q2.i70.1, Q2.i70.5).

use crate::mechanics::placements;
use crate::player::PLAYER_HALF_EXTENTS;
use crate::robot::{Assembly, BETA, Kind, MAX_PARTS, Part, PartId, PartType};
use crate::trig::{self, TRIG_ONE};
use crate::weapons::{self, Weapon};

/// A destroyed battery's blast reaches this far (units) from the battery's centre...
pub const BATTERY_BLAST_RADIUS_UNITS: i32 = 80;
/// ...and deals this much at its centre, falling off to nothing at its edge. Provisional (Atlas
/// Q2.i70.5).
pub const BATTERY_BLAST_DAMAGE: i32 = 40;

fn part_type(part: &Part) -> &'static PartType {
    BETA.part_type(part.kind)
        .expect("validated parts are in the catalog")
}

/// One robot's part healths and which parts it has lost, in its build's part-ID order.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct PartDamage {
    health: [u16; MAX_PARTS],
    /// Bit `i`: part `i` (by index) is no longer connected to the root chassis.
    lost: u32,
    len: u8,
}

impl Default for PartDamage {
    fn default() -> Self {
        Self::fresh(&Assembly::default())
    }
}

/// What one strike on one part did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Change {
    /// The health it took (at most the part's health).
    pub dealt: u16,
    /// The part reached zero.
    pub destroyed: bool,
    /// The parts lost by it (bits by index), the destroyed one included.
    pub lost: u32,
}

impl PartDamage {
    /// `build` unhurt: every part at its catalog health, all connected.
    #[must_use]
    pub fn fresh(build: &Assembly) -> Self {
        let mut health = [0; MAX_PARTS];
        for (slot, part) in health.iter_mut().zip(build.parts()) {
            *slot = part_type(part).health;
        }
        Self {
            health,
            lost: 0,
            len: u8::try_from(build.parts().len()).unwrap_or(0),
        }
    }

    /// The state with `health` (one a part of `build`, in its order), from the wire: what is
    /// lost follows from it.
    ///
    /// # Errors
    ///
    /// If there is not one health a part, or a health is above its part's.
    pub fn from_health(build: &Assembly, health: &[u16]) -> Result<Self, &'static str> {
        if health.len() != build.parts().len() {
            return Err("not one health a part");
        }
        let mut state = Self::fresh(build);
        for (index, (&value, part)) in health.iter().zip(build.parts()).enumerate() {
            if value > part_type(part).health {
                return Err("a part's health above its catalog health");
            }
            state.health[index] = value;
        }
        state.lost = state.connect(build);
        Ok(state)
    }

    /// Every part's health, in part-ID order.
    #[must_use]
    pub fn health(&self) -> &[u16] {
        &self.health[..usize::from(self.len)]
    }

    /// The parts no longer connected to the root chassis (bits by index), the destroyed ones
    /// included.
    #[must_use]
    pub const fn lost(&self) -> u32 {
        self.lost
    }

    /// Part `index` is still connected (and so not destroyed).
    #[must_use]
    pub const fn attached(&self, index: usize) -> bool {
        index < self.len as usize && self.lost & (1 << index) == 0
    }

    /// The root chassis' index in `build`: the vital part (D2.i89.1).
    #[must_use]
    pub fn root(build: &Assembly) -> Option<usize> {
        build.parts().iter().position(|part| part.mount.is_none())
    }

    /// The root chassis' health: the robot's life (0 without one).
    #[must_use]
    pub fn chassis_health(&self, build: &Assembly) -> u16 {
        Self::root(build).map_or(0, |index| self.health[index])
    }

    /// `build` carries `weapon` as a part and has lost it: it cannot fire.
    #[must_use]
    pub fn weapon_lost(&self, build: &Assembly, weapon: Weapon) -> bool {
        build
            .parts()
            .iter()
            .position(|part| part_type(part).kind == Kind::Weapon(weapon))
            .is_some_and(|index| !self.attached(index))
    }

    /// `build` carries `weapon` as a part and still has it: it may hold, fire or swing it. A
    /// weapon the build does not mount is never armed.
    #[must_use]
    pub fn armed(&self, build: &Assembly, weapon: Weapon) -> bool {
        build
            .parts()
            .iter()
            .position(|part| part_type(part).kind == Kind::Weapon(weapon))
            .is_some_and(|index| self.attached(index))
    }

    /// It may hold `weapon` now ([`crate::weapons::holdable`]): the host refuses any other.
    #[must_use]
    pub fn holdable(&self, build: &Assembly, weapon: Weapon) -> bool {
        crate::weapons::holdable(|weapon| self.armed(build, weapon)).any(|held| held == weapon)
    }

    /// It still has a connected leg, wheel or track: the capsule may walk, jump and slide.
    #[must_use]
    pub fn mobile(&self, build: &Assembly) -> bool {
        build.parts().iter().enumerate().any(|(index, part)| {
            matches!(part_type(part).kind, Kind::Locomotion(_)) && self.attached(index)
        })
    }

    /// The parts no longer reachable from the root chassis without crossing a destroyed part
    /// (all of them once the root is destroyed).
    fn connect(&self, build: &Assembly) -> u32 {
        let parts = build.parts();
        let everything = if parts.len() >= 32 {
            u32::MAX
        } else {
            (1u32 << parts.len()) - 1
        };
        let Some(root) = Self::root(build).filter(|&root| self.health[root] > 0) else {
            return everything;
        };
        let mut reached = 1u32 << root;
        let mut stack = vec![root];
        while let Some(at) = stack.pop() {
            let children = parts
                .iter()
                .enumerate()
                .filter(|(_, part)| part.mount.is_some_and(|mount| mount.parent == parts[at].id))
                .map(|(child, _)| child);
            for next in children {
                if reached & (1 << next) == 0 && self.health[next] > 0 {
                    reached |= 1 << next;
                    stack.push(next);
                }
            }
        }
        everything & !reached
    }

    /// `amount` of damage to part `index` of `build`: nothing if it is already lost, else its
    /// health goes down (never below zero) and, at zero, it is destroyed and what hung from it
    /// is lost.
    pub fn hurt(&mut self, build: &Assembly, index: usize, amount: i32) -> Option<Change> {
        if amount <= 0 || !self.attached(index) {
            return None;
        }
        let health = self.health[index];
        let dealt = u16::try_from(amount).unwrap_or(u16::MAX).min(health);
        self.health[index] = health - dealt;
        let destroyed = self.health[index] == 0;
        let before = self.lost;
        if destroyed {
            self.lost = self.connect(build);
        }
        Some(Change {
            dealt,
            destroyed,
            lost: self.lost & !before,
        })
    }
}

/// One part placed in the world as an oriented box (units).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PartBox {
    /// Its index in the build (part-ID order).
    pub index: usize,
    pub part: PartId,
    pub armor: bool,
    pub center: [f32; 3],
    /// Its own X, Y and Z in the world (unit vectors).
    pub axes: [[f32; 3]; 3],
    pub half: [f32; 3],
}

/// One strike's damage on one part.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blow {
    /// The part's index in the build.
    pub index: usize,
    pub damage: i32,
    /// Where it was struck (units).
    pub at: [f32; 3],
}

/// A fighter's connected parts placed at its centre and yaw, in part-ID order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hitboxes {
    pub boxes: Vec<PartBox>,
}

/// A box in the robot's frame: its centre, axes and half extents.
type Oriented = ([f32; 3], [[f32; 3]; 3], [f32; 3]);

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// The rotation of a unit quaternion `[x, y, z, w]` as the images of X, Y and Z.
fn axes_of([x, y, z, w]: [f32; 4]) -> [[f32; 3]; 3] {
    [
        [
            1.0 - 2.0 * (y * y + z * z),
            2.0 * (x * y + z * w),
            2.0 * (x * z - y * w),
        ],
        [
            2.0 * (x * y - z * w),
            1.0 - 2.0 * (x * x + z * z),
            2.0 * (y * z + x * w),
        ],
        [
            2.0 * (x * z + y * w),
            2.0 * (y * z - x * w),
            1.0 - 2.0 * (x * x + y * y),
        ],
    ]
}

impl Hitboxes {
    /// `build`'s connected parts (by `damage`) on a fighter whose capsule is centred at `center`
    /// (units) facing `yaw`.
    #[must_use]
    pub fn place(build: &Assembly, damage: &PartDamage, center: [f32; 3], yaw: u16) -> Self {
        let placed = placements(build);
        let local: Vec<Oriented> = placed
            .iter()
            .map(|placement| {
                (
                    placement.centre,
                    axes_of(placement.orientation),
                    placement.half_extents,
                )
            })
            .collect();
        // The robot's lowest point at rest stands on the capsule's feet.
        let floor = local
            .iter()
            .map(|(centre, axes, half)| {
                let reach: f32 = (0..3).map(|axis| axes[axis][1].abs() * half[axis]).sum();
                centre[1] - reach
            })
            .fold(f32::MAX, f32::min);
        #[allow(clippy::cast_precision_loss)]
        let (fx, fz) = {
            let (x, z) = trig::direction(yaw);
            (x as f32 / TRIG_ONE as f32, z as f32 / TRIG_ONE as f32)
        };
        // Robot +X forward, +Y up, +Z its right (forward × up).
        let turn = |v: [f32; 3]| [v[0] * fx - v[2] * fz, v[1], v[0] * fz + v[2] * fx];
        #[allow(clippy::cast_precision_loss)]
        let feet = [
            center[0],
            center[1] - PLAYER_HALF_EXTENTS.y as f32,
            center[2],
        ];
        let boxes = build
            .parts()
            .iter()
            .zip(local)
            .enumerate()
            .filter(|(index, _)| damage.attached(*index))
            .map(|(index, (part, (centre, axes, half)))| {
                let offset = turn([centre[0], centre[1] - floor, centre[2]]);
                PartBox {
                    index,
                    part: part.id,
                    armor: part_type(part).kind == Kind::Armor,
                    center: [0, 1, 2].map(|axis| feet[axis] + offset[axis]),
                    axes: axes.map(turn),
                    half,
                }
            })
            .collect();
        Self { boxes }
    }

    /// The box of part `index`, if it is connected.
    #[must_use]
    pub fn get(&self, index: usize) -> Option<&PartBox> {
        self.boxes.iter().find(|part| part.index == index)
    }

    /// Where along `translation` (0..=1) a ray from `origin` enters each box it meets: (the
    /// fraction, the box's position in [`Hitboxes::boxes`]), nearest first, ties by part ID.
    #[must_use]
    pub fn ray(&self, origin: [f32; 3], translation: [f32; 3]) -> Vec<(f32, usize)> {
        let mut hits: Vec<(f32, usize)> = self
            .boxes
            .iter()
            .enumerate()
            .filter_map(|(at, part)| {
                let offset = [0, 1, 2].map(|axis| origin[axis] - part.center[axis]);
                let local_origin = part.axes.map(|axis| dot(offset, axis));
                let local_translation = part.axes.map(|axis| dot(translation, axis));
                weapons::ray_box(local_origin, local_translation, [0.0; 3], part.half)
                    .map(|fraction| (fraction, at))
            })
            .collect();
        hits.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        hits
    }

    /// Where along `translation` a ray from `origin` first meets a part.
    #[must_use]
    pub fn first(&self, origin: [f32; 3], translation: [f32; 3]) -> Option<f32> {
        self.ray(origin, translation)
            .first()
            .map(|(fraction, _)| *fraction)
    }

    /// `damage` along a ray: the first part it meets takes it; an armor plate passes
    /// `penetration` percent (rounded down) on to the next part along the ray and takes the rest.
    #[must_use]
    pub fn pierce(
        &self,
        origin: [f32; 3],
        translation: [f32; 3],
        damage: i32,
        penetration: u8,
    ) -> Vec<Blow> {
        let mut blows = Vec::new();
        let mut left = damage;
        for (fraction, at) in self.ray(origin, translation) {
            if left <= 0 {
                break;
            }
            let part = &self.boxes[at];
            let point = [0, 1, 2].map(|axis| origin[axis] + translation[axis] * fraction);
            let passed = if part.armor {
                left * i32::from(penetration) / 100
            } else {
                0
            };
            blows.push(Blow {
                index: part.index,
                damage: left - passed,
                at: point,
            });
            left = passed;
        }
        blows
    }

    /// The point of a part's box nearest `point`.
    fn closest(part: &PartBox, point: [f32; 3]) -> [f32; 3] {
        let offset = [0, 1, 2].map(|axis| point[axis] - part.center[axis]);
        let mut closest = part.center;
        for (axis, half) in part.axes.iter().zip(part.half) {
            let along = dot(offset, *axis).clamp(-half, half);
            for (component, direction) in closest.iter_mut().zip(axis) {
                *component += direction * along;
            }
        }
        closest
    }

    /// The connected part nearest `point` and the point on it (ties by part ID).
    #[must_use]
    pub fn nearest(&self, point: [f32; 3]) -> Option<(usize, [f32; 3])> {
        let mut best: Option<(f32, usize, [f32; 3])> = None;
        for part in &self.boxes {
            let on = Self::closest(part, point);
            let gap = [0, 1, 2].map(|axis| on[axis] - point[axis]);
            let squared = dot(gap, gap);
            if best.is_none_or(|(nearest, _, _)| squared < nearest) {
                best = Some((squared, part.index, on));
            }
        }
        best.map(|(_, index, on)| (index, on))
    }

    /// A blast at `at` (units) of `damage` reaching `radius_units`: a blow on every part whose
    /// box comes within reach, by the falloff to its nearest point, in part-ID order.
    #[must_use]
    pub fn blast(&self, at: [f32; 3], radius_units: i32, damage: i32) -> Vec<Blow> {
        self.boxes
            .iter()
            .filter_map(|part| {
                let on = Self::closest(part, at);
                let damage = weapons::falloff(at, on, radius_units, damage)?;
                (damage > 0).then_some(Blow {
                    index: part.index,
                    damage,
                    at: on,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::robot::STANDARD;

    #[test]
    fn quaternion_axes_are_the_rotation() {
        let half = std::f32::consts::FRAC_1_SQRT_2;
        // A quarter turn about Y: X goes to −Z.
        let axes = axes_of([0.0, half, 0.0, half]);
        assert!((axes[0][2] + 1.0).abs() < 1e-6, "{axes:?}");
        assert!((axes[2][0] - 1.0).abs() < 1e-6, "{axes:?}");
    }

    #[test]
    fn the_standard_robot_stands_on_the_capsules_feet() {
        let boxes = Hitboxes::place(&STANDARD, &PartDamage::fresh(&STANDARD), [0.0; 3], 0);
        let low = boxes
            .boxes
            .iter()
            .map(|part| {
                let reach: f32 = (0..3)
                    .map(|axis| part.axes[axis][1].abs() * part.half[axis])
                    .sum();
                part.center[1] - reach
            })
            .fold(f32::MAX, f32::min);
        #[allow(clippy::cast_precision_loss)]
        let feet = -(PLAYER_HALF_EXTENTS.y as f32);
        assert!((low - feet).abs() < 1e-3, "{low}");
        assert_eq!(boxes.boxes.len(), STANDARD.parts().len());
    }
}
