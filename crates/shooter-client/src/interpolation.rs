//! Drawing the other fighters smoothly from snapshots that arrive unevenly: a short buffer of
//! snapshots and a playback clock that runs one tick per local tick, a fixed delay behind the
//! newest snapshot, easing toward that target instead of jumping with every arrival. The other
//! fighters are drawn between the two buffered snapshots around the clock, and online shots name
//! the clock's tick as their view tick, so the host rewinds to what the player saw.

use shooter_sim::Snapshot;
use std::collections::VecDeque;

/// Online, other fighters are drawn this many ticks (50 ms) behind the newest snapshot: one lost
/// snapshot plus a tick of arrival jitter never leaves the clock without a snapshot ahead.
pub const ONLINE_DELAY_TICKS: f32 = 3.0;
/// Snapshots kept (the delay plus a lost one or two).
const FRAMES: usize = 8;
/// The clock closes this share of its distance to the target each tick (no visible speed-up)...
const EASE: f32 = 0.05;
/// ...but jumps when it is further off than this (joining, a stall, a long gap).
const SNAP_TICKS: f32 = 8.0;

struct Frame {
    tick: u32,
    /// Each slot's centre (units) while alive.
    centers: Vec<Option<[f32; 3]>>,
}

pub struct Interpolation {
    delay: f32,
    frames: VecDeque<Frame>,
    /// The tick the other fighters are drawn at (fractional), once there is a snapshot.
    clock: Option<f32>,
}

impl Interpolation {
    /// Draws `delay` ticks behind the newest snapshot (practice: 1, between the last two ticks).
    #[must_use]
    pub const fn new(delay: f32) -> Self {
        Self {
            delay,
            frames: VecDeque::new(),
            clock: None,
        }
    }

    /// Forgets every snapshot (a new connection).
    pub fn clear(&mut self) {
        self.frames.clear();
        self.clock = None;
    }

    /// Buffers a snapshot (older or repeated ticks are ignored).
    pub fn push(&mut self, snapshot: &Snapshot) {
        if self
            .frames
            .back()
            .is_some_and(|frame| frame.tick >= snapshot.tick)
        {
            return;
        }
        let mut centers = vec![None; usize::from(u8::MAX)];
        for fighter in snapshot.fighters.iter().filter(|fighter| fighter.alive) {
            centers[usize::from(fighter.slot)] = Some(fighter.position);
        }
        self.frames.push_back(Frame {
            tick: snapshot.tick,
            centers,
        });
        while self.frames.len() > FRAMES {
            self.frames.pop_front();
        }
    }

    /// One local tick: the clock moves a tick, eased toward the delay behind the newest snapshot
    /// and never past it.
    pub fn advance(&mut self) {
        let Some(newest) = self.frames.back() else {
            return;
        };
        #[allow(clippy::cast_precision_loss)]
        let newest = newest.tick as f32;
        let target = newest - self.delay;
        let clock = match self.clock {
            Some(clock) if (target - clock).abs() <= SNAP_TICKS => {
                clock + 1.0 + (target - (clock + 1.0)) * EASE
            }
            _ => target,
        };
        self.clock = Some(clock.min(newest));
    }

    /// The whole tick the other fighters show at (an online shot's view tick).
    #[must_use]
    pub fn view_tick(&self) -> Option<u32> {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        self.clock.map(|clock| clock.round().max(0.0) as u32)
    }

    /// Where `slot`'s fighter is drawn `blend` (0..1) of a tick after the clock, `None` while it
    /// is dead or absent there.
    #[must_use]
    pub fn center(&self, slot: u8, blend: f32) -> Option<[f32; 3]> {
        let newest = self.frames.back()?;
        #[allow(clippy::cast_precision_loss)]
        let at = (self.clock? + blend).min(newest.tick as f32);
        let slot = usize::from(slot);
        let after = self
            .frames
            .iter()
            .position(|frame| {
                #[allow(clippy::cast_precision_loss)]
                let tick = frame.tick as f32;
                tick >= at
            })
            .unwrap_or(self.frames.len() - 1);
        let next = &self.frames[after];
        let Some(before) = after.checked_sub(1).map(|index| &self.frames[index]) else {
            return next.centers[slot];
        };
        let from = before.centers[slot]?;
        let Some(to) = next.centers[slot] else {
            // Died between the two: drawn where it was until the snapshot that saw it gone.
            return Some(from);
        };
        #[allow(clippy::cast_precision_loss)]
        let t = (at - before.tick as f32) / (next.tick - before.tick) as f32;
        Some([0, 1, 2].map(|axis| from[axis] + (to[axis] - from[axis]) * t))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shooter_sim::{Arena, Rules, Session};

    /// Plays a bot match whose snapshots arrive `pattern[local tick]` at a time; returns how far
    /// the drawing clock's per-tick step strays from one tick (0: perfectly even motion) and
    /// how far a drawn fighter strays from where the match had it at the clock's tick.
    fn unevenness(snapshots: &[Snapshot], pattern: &[usize]) -> (f32, f32) {
        let mut interpolation = Interpolation::new(ONLINE_DELAY_TICKS);
        let mut arrived = snapshots.iter();
        let (mut stray, mut off) = (0.0f32, 0.0f32);
        let mut last = None;
        for local in 0..snapshots.len() - 20 {
            for snapshot in arrived.by_ref().take(pattern[local % pattern.len()]) {
                interpolation.push(snapshot);
            }
            interpolation.advance();
            let clock = interpolation.clock.unwrap_or_default();
            if local >= 60 {
                stray = stray.max((clock - last.unwrap_or(clock - 1.0) - 1.0).abs());
                let whole = interpolation.view_tick().unwrap();
                let truth = snapshots
                    .iter()
                    .find(|snapshot| snapshot.tick == whole)
                    .unwrap();
                for fighter in truth.fighters.iter().filter(|fighter| fighter.alive) {
                    #[allow(clippy::cast_precision_loss)]
                    let blend = whole as f32 - clock;
                    if let Some(drawn) = interpolation.center(fighter.slot, blend) {
                        for (drawn, truth) in drawn.iter().zip(fighter.position) {
                            off = off.max((drawn - truth).abs());
                        }
                    }
                }
            }
            last = Some(clock);
        }
        (stray, off)
    }

    /// A bot match's snapshots delivered in jittery bunches (0-3 a tick, one a tick on average)
    /// draw as evenly as punctual ones: the clock steps a steady tick and the fighters are where
    /// the match had them.
    #[test]
    fn jittery_snapshots_draw_smoothly() {
        let mut session = Session::with_rules(Arena::greybox(), Rules::deathmatch(11)).unwrap();
        let snapshots: Vec<Snapshot> = (0..400)
            .map(|_| {
                session.step().unwrap();
                session.snapshot()
            })
            .collect();
        for pattern in [&[1][..], &[1, 0, 2, 1, 0, 0, 3, 1, 1, 0, 2]] {
            let (stray, off) = unevenness(&snapshots, pattern);
            assert!(stray < 0.1, "{pattern:?}: the clock strayed {stray} ticks");
            assert!(
                off < 0.01,
                "{pattern:?}: a fighter was drawn {off} units off"
            );
        }
    }

    #[test]
    fn the_clock_never_runs_past_the_newest_snapshot() {
        let mut session = Session::with_rules(Arena::greybox(), Rules::deathmatch(2)).unwrap();
        let mut interpolation = Interpolation::new(ONLINE_DELAY_TICKS);
        session.step().unwrap();
        interpolation.push(&session.snapshot());
        for _ in 0..20 {
            interpolation.advance();
        }
        assert_eq!(interpolation.view_tick(), Some(session.ticks()));
    }
}
