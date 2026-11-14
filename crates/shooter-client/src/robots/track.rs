//! What the robots' poses need beyond a snapshot, followed per fighter slot from the match's
//! events: seconds since each fighter's last shot, and where each one died (a dead fighter has
//! no position in a snapshot), so it can topple there until it respawns.

use render_engine::glam::Vec3;
use shooter_sim::{Assembly, Event, Weapon};

const SLOTS: usize = u8::MAX as usize + 1;

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
}

impl Default for Tracker {
    fn default() -> Self {
        Self {
            since_fired: vec![f32::INFINITY; SLOTS],
            alive: vec![None; SLOTS],
            dead: vec![None; SLOTS],
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
                Event::Spawned { slot, .. } => self.dead[usize::from(slot)] = None,
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
    fn shots_reset_the_fire_clock() {
        let mut tracker = Tracker::default();
        tracker.tick(&[], Some(1), Some(Weapon::Laser), 0.1);
        assert!(tracker.since_fired(1) < 1e-6);
        tracker.tick(&[], Some(1), None, 0.1);
        assert!((tracker.since_fired(1) - 0.1).abs() < 1e-6);
        assert!(tracker.since_fired(2).is_infinite());
    }
}
