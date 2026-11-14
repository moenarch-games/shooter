//! What the robots' poses need beyond a snapshot, followed per fighter slot from the match's
//! events: seconds since each fighter's last shot, where each one died (a dead fighter has no
//! position in a snapshot), so it can topple there until it respawns, and where each part it has
//! lost this life came off, so the part falls from there to the floor and lies there until the
//! fighter respawns (presentation only: the host decides what is lost, [`shooter_sim::damage`];
//! the pieces are not physical, debris is shooter#75).

use crate::scene::METRES_PER_UNIT;
use render_engine::glam::{Mat3, Mat4, Quat, Vec3};
use shooter_sim::damage::PartDamage;
use shooter_sim::mechanics::placements;
use shooter_sim::robot::PartId;
use shooter_sim::{Assembly, Event, Weapon};

const SLOTS: usize = u8::MAX as usize + 1;
/// How fast a lost part falls (metres a second squared).
pub const GRAVITY: f32 = 9.81;

/// A part lost this life: where its robot's frame was when it came off, how far (metres) its
/// lowest point was above the robot's feet, and how long ago that was.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Lost {
    part: PartId,
    frame: Mat4,
    height: f32,
    age: f32,
}

/// The robot's frame at rest standing on a capsule centred at `center` (metres) looking along
/// `forward`, for `build`: +X forward, +Y up, its lowest point on the capsule's feet (the drawn
/// pose without its gait's lift).
#[must_use]
pub fn rest_frame(build: &Assembly, center: Vec3, forward: Vec3) -> Mat4 {
    let (floor, _) = rest(build);
    let forward = Vec3::new(forward.x, 0.0, forward.z).normalize_or(Vec3::Z);
    let turn = Mat3::from_cols(forward, Vec3::Y, forward.cross(Vec3::Y));
    let feet = center - Vec3::Y * crate::robots::pose::HALF_HEIGHT;
    Mat4::from_mat3_translation(turn, feet - Vec3::Y * floor)
}

/// `build`'s lowest point at rest and each part's lowest point (metres, robot frame).
fn rest(build: &Assembly) -> (f32, Vec<f32>) {
    let bottoms: Vec<f32> = placements(build)
        .into_iter()
        .map(|placement| {
            let axes = Mat3::from_quat(Quat::from_array(placement.orientation).normalize());
            let half = Vec3::from(placement.half_extents) * METRES_PER_UNIT;
            let reach = axes.x_axis.y.abs() * half.x
                + axes.y_axis.y.abs() * half.y
                + axes.z_axis.y.abs() * half.z;
            placement.centre[1] * METRES_PER_UNIT - reach
        })
        .collect();
    let floor = bottoms.iter().copied().fold(f32::MAX, f32::min);
    (floor, bottoms)
}

/// Where and as what a fighter was last drawn alive, and for how long it has been dead.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Remains {
    pub center: Vec3,
    pub forward: Vec3,
    pub build: Assembly,
    pub held: Weapon,
    pub dead_for: f32,
}

#[derive(Clone, Debug)]
pub struct Tracker {
    since_fired: Vec<f32>,
    alive: Vec<Option<Remains>>,
    dead: Vec<Option<Remains>>,
    lost: Vec<Vec<Lost>>,
}

impl Default for Tracker {
    fn default() -> Self {
        Self {
            since_fired: vec![f32::INFINITY; SLOTS],
            alive: vec![None; SLOTS],
            dead: vec![None; SLOTS],
            lost: vec![Vec::new(); SLOTS],
        }
    }
}

impl Tracker {
    /// One tick of `seconds`: `events` and the local fighter's own shot (`fired`, as predicted).
    pub fn tick(
        &mut self,
        events: &[Event],
        local: Option<u8>,
        fired: Option<Weapon>,
        seconds: f32,
    ) {
        for since in &mut self.since_fired {
            *since += seconds;
        }
        for remains in self.dead.iter_mut().flatten() {
            remains.dead_for += seconds;
        }
        for lost in self.lost.iter_mut().flatten() {
            lost.age += seconds;
        }
        if let (Some(slot), Some(_)) = (local, fired) {
            self.since_fired[usize::from(slot)] = 0.0;
        }
        for event in events {
            match *event {
                Event::Fired { slot, .. } => self.since_fired[usize::from(slot)] = 0.0,
                Event::Killed { victim, .. } => {
                    let slot = usize::from(victim);
                    self.dead[slot] = self.alive[slot].take();
                }
                Event::Spawned { slot, .. } => {
                    self.dead[usize::from(slot)] = None;
                    self.lost[usize::from(slot)].clear();
                }
                _ => {}
            }
        }
    }

    /// `slot` is drawn alive this frame.
    pub fn seen(&mut self, slot: u8, center: Vec3, forward: Vec3, build: Assembly, held: Weapon) {
        let slot = usize::from(slot);
        self.alive[slot] = Some(Remains {
            center,
            forward,
            build,
            held,
            dead_for: 0.0,
        });
        self.dead[slot] = None;
    }

    /// `slot`, drawn alive as `build` at rest `frame` ([`rest_frame`]), has lost what `parts`
    /// says: each part newly lost comes off there now.
    pub fn parts(&mut self, slot: u8, build: &Assembly, parts: &PartDamage, frame: Mat4) {
        let lost = &mut self.lost[usize::from(slot)];
        let missing: Vec<usize> = (0..build.parts().len())
            .filter(|&index| !parts.attached(index))
            .filter(|&index| {
                lost.iter()
                    .all(|known| known.part != build.parts()[index].id)
            })
            .collect();
        if missing.is_empty() {
            return;
        }
        let (floor, bottoms) = rest(build);
        for index in missing {
            lost.push(Lost {
                part: build.parts()[index].id,
                frame,
                height: (bottoms[index] - floor).max(0.0),
                age: 0.0,
            });
        }
    }

    /// `slot`'s lost parts and where their robot frames are now: each falls from where it came
    /// off until it lies on the floor ([`super::PartStates::detached`]).
    #[must_use]
    pub fn detached(&self, slot: u8) -> Vec<(PartId, Mat4)> {
        self.lost[usize::from(slot)]
            .iter()
            .map(|lost| {
                let fallen = (0.5 * GRAVITY * lost.age * lost.age).min(lost.height);
                (
                    lost.part,
                    Mat4::from_translation(-Vec3::Y * fallen) * lost.frame,
                )
            })
            .collect()
    }

    #[must_use]
    pub fn since_fired(&self, slot: u8) -> f32 {
        self.since_fired[usize::from(slot)]
    }

    /// The fighters lying dead, by slot.
    pub fn dead(&self) -> impl Iterator<Item = (u8, Remains)> + '_ {
        self.dead
            .iter()
            .enumerate()
            .filter_map(|(slot, remains)| Some((u8::try_from(slot).ok()?, (*remains)?)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fighter_lies_where_it_died_until_it_respawns() {
        let mut tracker = Tracker::default();
        tracker.seen(3, Vec3::ONE, Vec3::Z, Assembly::default(), Weapon::Shotgun);
        tracker.tick(
            &[Event::Killed {
                victim: 3,
                killer: None,
                weapon: None,
            }],
            None,
            None,
            0.5,
        );
        tracker.tick(&[], None, None, 0.25);
        let dead: Vec<_> = tracker.dead().collect();
        assert_eq!(dead.len(), 1);
        assert_eq!(dead[0].0, 3);
        assert_eq!(dead[0].1.center, Vec3::ONE);
        assert!((dead[0].1.dead_for - 0.25).abs() < 1e-6);
        tracker.tick(&[Event::Spawned { slot: 3, yaw: 0 }], None, None, 0.1);
        assert_eq!(tracker.dead().count(), 0);
    }

    #[test]
    fn a_lost_part_falls_to_the_floor_and_is_gone_at_respawn() {
        let build = Assembly::default();
        let mut parts = PartDamage::fresh(&build);
        // The right arm (part 4) breaks: it carries its blade and mounted laser with it.
        let arm = build
            .parts()
            .iter()
            .position(|p| p.id == PartId(4))
            .unwrap();
        parts.hurt(&build, arm, 1_000);
        let mut tracker = Tracker::default();
        let frame = rest_frame(&build, Vec3::new(0.0, 0.9, 0.0), Vec3::Z);
        tracker.parts(2, &build, &parts, frame);
        tracker.parts(2, &build, &parts, Mat4::IDENTITY);
        let lost: Vec<PartId> = tracker.detached(2).iter().map(|(id, _)| *id).collect();
        assert_eq!(lost, [PartId(4), PartId(8), PartId(14)]);
        assert_eq!(
            tracker.detached(2)[0].1,
            frame,
            "it starts where it came off"
        );
        tracker.tick(&[], None, None, 5.0);
        let fallen = tracker.detached(2)[0].1;
        let drop = frame.w_axis.y - fallen.w_axis.y;
        assert!(drop > 0.5 && drop < 1.6, "{drop}");
        tracker.tick(&[Event::Spawned { slot: 2, yaw: 0 }], None, None, 0.1);
        assert!(tracker.detached(2).is_empty());
    }

    #[test]
    fn shots_reset_the_fire_clock() {
        let mut tracker = Tracker::default();
        tracker.tick(&[], Some(1), Some(Weapon::Laser), 0.1);
        assert!(tracker.since_fired(1) < 1e-6);
        tracker.tick(&[], Some(1), None, 0.1);
        assert!((tracker.since_fired(1) - 0.1).abs() < 1e-6);
        assert!(tracker.since_fired(2).is_infinite());
    }
}
