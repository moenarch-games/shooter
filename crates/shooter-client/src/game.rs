//! Where the client's match comes from: offline practice runs the deathmatch in-process
//! (`shooter-sim`, the local player in slot 0 against bots), online play (native and in the
//! browser) follows a game-server host and predicts the local fighter. Both hand the frame the
//! same [`Snapshot`] of the match, so drawing and the HUD do not care which it is.

use crate::interpolation::Interpolation;
use shooter_sim::session::LOCAL_PLAYER;
use shooter_sim::{Arena, Event, Intent, Rules, Session, Snapshot, StepReport};

/// The practice match's bot seed.
pub const PRACTICE_SEED: u64 = 1;

/// What one local tick produced.
pub struct Tick {
    pub report: StepReport,
    /// The match's events to show and sound since the last tick.
    pub events: Vec<Event>,
}

pub enum Game {
    Practice {
        session: Box<Session>,
        snapshot: Snapshot,
        others: Interpolation,
    },
    Online(Box<online::Online>),
}

impl Game {
    /// Offline practice against bots.
    ///
    /// # Panics
    ///
    /// If the greybox arena does not build.
    #[must_use]
    pub fn practice() -> Self {
        let session = Session::with_rules(Arena::greybox(), Rules::practice(PRACTICE_SEED))
            .expect("the greybox arena builds");
        let snapshot = session.snapshot();
        Self::Practice {
            session: Box::new(session),
            snapshot,
            // Drawn between the last two ticks; shots see the match as it is.
            others: Interpolation::new(1.0),
        }
    }

    /// One local tick holding `intent`.
    pub fn tick(&mut self, intent: Intent) -> Result<Tick, String> {
        match self {
            Self::Practice {
                session,
                snapshot,
                others,
            } => {
                let report = session.tick(intent).map_err(|error| error.to_string())?;
                *snapshot = session.snapshot();
                others.push(snapshot);
                others.advance();
                Ok(Tick {
                    report,
                    events: session.events.clone(),
                })
            }
            Self::Online(online) => online.tick(intent),
        }
    }

    /// The latest view of the match, once there is one.
    #[must_use]
    pub fn snapshot(&self) -> Option<&Snapshot> {
        match self {
            Self::Practice { snapshot, .. } => Some(snapshot),
            Self::Online(online) => online.snapshot(),
        }
    }

    /// Where another fighter is drawn `blend` (0..1) of a tick after the last local tick (units),
    /// `None` while it is dead there.
    #[must_use]
    pub fn other_center(&self, slot: u8, blend: f32) -> Option<[f32; 3]> {
        match self {
            Self::Practice { others, .. } => others.center(slot, blend),
            Self::Online(online) => online.other_center(slot, blend),
        }
    }

    /// The local player's slot.
    #[must_use]
    pub fn local_slot(&self) -> Option<u8> {
        let id = match self {
            Self::Practice { .. } => LOCAL_PLAYER,
            Self::Online(online) => online.player_id()?,
        };
        Some(self.snapshot()?.of_player(id)?.slot)
    }

    /// The local fighter's centre (units), predicted online, while it is alive.
    #[must_use]
    pub fn local_center(&self) -> Option<[f32; 3]> {
        match self {
            Self::Practice { session, .. } => session
                .local()
                .alive()
                .then(|| session.local().player.center(&session.world)),
            Self::Online(online) => online.local_center(),
        }
    }

    /// Whether the local fighter is sliding (the camera dips).
    #[must_use]
    pub fn local_sliding(&self) -> bool {
        match self {
            Self::Practice { session, .. } => session.local().player.motion.sliding,
            Self::Online(online) => online.sliding(),
        }
    }

    /// Where the local fighter's grapple hook holds (units), predicted online, while it is set.
    #[must_use]
    pub fn local_hook(&self) -> Option<[f32; 3]> {
        match self {
            Self::Practice { session, .. } => session.local().hook.map(|hook| hook.anchor_units()),
            Self::Online(online) => online.hook(),
        }
    }

    /// A line about the connection while there is something to say.
    #[must_use]
    pub fn status(&self) -> Option<&str> {
        match self {
            Self::Practice { .. } => None,
            Self::Online(online) => online.status(),
        }
    }
}

pub mod online {
    //! Following a hosted match: inputs go out once a tick, the local fighter is predicted by a
    //! one-fighter session of the same arena and reconciled with every snapshot (reset to the
    //! server's state at the acknowledged input and the unacknowledged inputs replayed, when the
    //! prediction was off).

    use super::Tick;
    use crate::interpolation::{Interpolation, ONLINE_DELAY_TICKS};
    use crate::net::{Connection, NetEvent, Trust};
    use shooter_sim::grapple::Hook;
    use shooter_sim::names;
    use shooter_sim::player::Motion;
    use shooter_sim::session::{HALF, MAX_HEALTH};
    use shooter_sim::weapons::{self, Delivery};
    use shooter_sim::wire::encode_input;
    use shooter_sim::{Arena, Event, Intent, Session, Snapshot, StepReport, Weapon};
    use std::collections::VecDeque;

    /// A prediction further than this (units, any axis) from the server's state is corrected.
    const TOLERANCE_UNITS: f32 = 1.0;

    #[derive(Clone, Copy)]
    struct Pending {
        sequence: u32,
        intent: Intent,
        /// The predicted state after this input.
        position: [f32; 3],
        motion: Motion,
        hook: Option<Hook>,
    }

    pub struct Online {
        connection: Connection,
        /// The name picked for this match (sanitized as the host does).
        name: Option<String>,
        player_id: Option<u32>,
        predictor: Session,
        predicting: bool,
        pending: VecDeque<Pending>,
        sequence: u32,
        latest: Option<Snapshot>,
        /// The other fighters as drawn, a little behind the newest snapshot.
        others: Interpolation,
        events: Vec<Event>,
        status: Option<String>,
        /// Lost sessions resumed so far.
        resumed: u32,
    }

    /// The host's events without the local fighter's own shots (`Fired` and the `Beam`s right
    /// after it), which were shown and heard when predicted; blade swings stay (their strikes
    /// come from the host).
    fn without_own_shots(events: &[Event], local: Option<u8>) -> impl Iterator<Item = Event> {
        let mut skipping = false;
        events.iter().copied().filter(move |event| {
            let own = match *event {
                Event::Fired { slot, weapon, .. } => {
                    skipping = Some(slot) == local && weapon != Weapon::EnergyBlade;
                    return !skipping;
                }
                // The shot's beam (a shotgun's pellets fly on the host and land as its own
                // `Pellet` events, which stay).
                Event::Beam { .. } => return !skipping,
                _ => false,
            };
            skipping = false;
            !own
        })
    }

    impl Online {
        /// # Errors
        ///
        /// If the arena does not build.
        pub fn connect(
            url: String,
            certificate: Trust,
            name: Option<&str>,
        ) -> Result<Self, String> {
            let status = format!("connecting to {url}");
            Ok(Self {
                connection: Connection::open(url, certificate),
                name: name.and_then(names::sanitize),
                player_id: None,
                predictor: Session::new(Arena::greybox()).map_err(|error| error.to_string())?,
                predicting: false,
                pending: VecDeque::new(),
                sequence: 0,
                latest: None,
                others: Interpolation::new(ONLINE_DELAY_TICKS),
                events: Vec::new(),
                status: Some(status),
                resumed: 0,
            })
        }

        /// Drops the connection without closing it, as a network loss would (testing the
        /// reconnect).
        #[cfg(not(target_arch = "wasm32"))]
        #[doc(hidden)]
        pub fn sever(&self) {
            self.connection.sever();
        }

        pub fn other_center(&self, slot: u8, blend: f32) -> Option<[f32; 3]> {
            self.others.center(slot, blend)
        }

        fn local_slot(&self) -> Option<u8> {
            let id = self.player_id?;
            Some(self.latest.as_ref()?.of_player(id)?.slot)
        }

        /// The predicted tick's own shots, shown and heard at once: the muzzle flash as fired,
        /// a beam stopped by the first other fighter where they are drawn (the host's hit
        /// decides the damage). Blade swings are left to the host's events.
        fn predicted_shots(&self) -> Vec<Event> {
            let Some(local) = self.local_slot() else {
                return Vec::new();
            };
            let others: Vec<[f32; 3]> = self
                .latest
                .iter()
                .flat_map(|snapshot| &snapshot.fighters)
                .filter(|fighter| fighter.slot != local)
                .filter_map(|fighter| self.others.center(fighter.slot, 0.0))
                .collect();
            let mut shots = Vec::new();
            for event in &self.predictor.events {
                match *event {
                    Event::Fired { weapon, .. } | Event::Beam { weapon, .. }
                        if weapon == Weapon::EnergyBlade => {}
                    Event::Fired {
                        weapon,
                        eye,
                        direction,
                        ..
                    } => shots.push(Event::Fired {
                        slot: local,
                        weapon,
                        eye,
                        direction,
                    }),
                    Event::Beam {
                        weapon,
                        from,
                        end,
                        normal,
                        struck,
                    } => {
                        let translation = [0, 1, 2].map(|axis| end[axis] - from[axis]);
                        let hit = matches!(weapon.delivery(), Delivery::Hitscan { .. })
                            .then(|| {
                                others
                                    .iter()
                                    .filter_map(|center| {
                                        weapons::ray_box(from, translation, *center, HALF)
                                    })
                                    .reduce(f32::min)
                            })
                            .flatten();
                        shots.push(match hit {
                            Some(fraction) => Event::Beam {
                                weapon,
                                from,
                                end: [0, 1, 2]
                                    .map(|axis| from[axis] + translation[axis] * fraction),
                                normal: {
                                    let length = translation
                                        .iter()
                                        .map(|axis| axis * axis)
                                        .sum::<f32>()
                                        .sqrt()
                                        .max(f32::EPSILON);
                                    translation.map(|axis| -axis / length)
                                },
                                struck: true,
                            },
                            None => Event::Beam {
                                weapon,
                                from,
                                end,
                                normal,
                                struck,
                            },
                        });
                    }
                    _ => {}
                }
            }
            shots
        }

        /// Lost sessions resumed so far.
        pub const fn resumed(&self) -> u32 {
            self.resumed
        }

        pub const fn player_id(&self) -> Option<u32> {
            self.player_id
        }

        pub const fn snapshot(&self) -> Option<&Snapshot> {
            self.latest.as_ref()
        }

        pub fn status(&self) -> Option<&str> {
            self.status.as_deref()
        }

        pub fn local_center(&self) -> Option<[f32; 3]> {
            self.predicting
                .then(|| self.predictor.local().player.center(&self.predictor.world))
        }

        pub fn sliding(&self) -> bool {
            self.predicting && self.predictor.local().player.motion.sliding
        }

        fn predict(&mut self, intent: Intent) -> Result<StepReport, String> {
            self.predictor.fighters[0].health = MAX_HEALTH;
            self.predictor
                .tick(intent)
                .map_err(|error| error.to_string())
        }

        fn state(&self) -> ([f32; 3], Motion, Option<Hook>) {
            let local = self.predictor.local();
            (
                local.player.center(&self.predictor.world),
                local.player.motion,
                local.hook,
            )
        }

        pub fn hook(&self) -> Option<[f32; 3]> {
            self.predicting
                .then(|| self.predictor.local().hook.map(|hook| hook.anchor_units()))
                .flatten()
        }

        fn reconcile(&mut self, snapshot: &Snapshot) -> Result<(), String> {
            let Some(me) = self.player_id.and_then(|id| snapshot.of_player(id)) else {
                return Ok(());
            };
            if !me.alive {
                self.predicting = false;
                self.pending.clear();
                return Ok(());
            }
            let place = |predictor: &mut Session| {
                predictor
                    .place(0, me.position, me.velocity, me.motion, me.held, me.hook)
                    .map_err(|error| error.to_string())
            };
            if !self.predicting {
                place(&mut self.predictor)?;
                self.predicting = true;
                self.pending.clear();
                return Ok(());
            }
            while self
                .pending
                .front()
                .is_some_and(|pending| pending.sequence < me.ack)
            {
                self.pending.pop_front();
            }
            let Some(acked) = self
                .pending
                .front()
                .copied()
                .filter(|pending| pending.sequence == me.ack)
            else {
                return Ok(());
            };
            self.pending.pop_front();
            let off = (0..3)
                .any(|axis| (acked.position[axis] - me.position[axis]).abs() > TOLERANCE_UNITS)
                || acked.motion != me.motion
                || acked.hook != me.hook;
            if off {
                place(&mut self.predictor)?;
                let replay: Vec<Pending> = self.pending.drain(..).collect();
                for mut pending in replay {
                    self.predict(pending.intent)?;
                    (pending.position, pending.motion, pending.hook) = self.state();
                    self.pending.push_back(pending);
                }
            }
            Ok(())
        }

        pub fn tick(&mut self, intent: Intent) -> Result<Tick, String> {
            for event in self.connection.poll() {
                match event {
                    NetEvent::Welcome { player_id } => {
                        if self.player_id.is_some() {
                            self.resumed += 1;
                        }
                        self.player_id = Some(player_id);
                        self.status = None;
                    }
                    NetEvent::Snapshot(snapshot) => {
                        if self
                            .latest
                            .as_ref()
                            .is_some_and(|latest| latest.tick >= snapshot.tick)
                        {
                            continue;
                        }
                        // Own shots are shown when predicted, so only while predicting.
                        let local = self
                            .player_id
                            .filter(|_| self.predicting)
                            .and_then(|id| snapshot.of_player(id))
                            .map(|me| me.slot);
                        self.events
                            .extend(without_own_shots(&snapshot.events, local));
                        self.reconcile(&snapshot)?;
                        self.others.push(&snapshot);
                        self.latest = Some(*snapshot);
                    }
                    NetEvent::Reconnecting(reason) => {
                        // Prediction resumes from the first snapshot after the welcome.
                        self.status = Some(format!("reconnecting ({reason})"));
                        self.predicting = false;
                        self.pending.clear();
                    }
                    NetEvent::Closed(reason) => {
                        self.status = Some(format!("disconnected: {reason}"));
                        self.predicting = false;
                    }
                }
            }
            self.others.advance();
            let mut report = StepReport::default();
            if self.player_id.is_some() && self.status.is_none() {
                self.sequence += 1;
                let view_tick = self.others.view_tick().unwrap_or(0);
                // The name rides along until a snapshot shows it (inputs are unreliable).
                let shown = self.latest.as_ref().and_then(|latest| {
                    let me = latest.of_player(self.player_id?)?;
                    latest.name(me.slot)
                });
                let name = self.name.as_deref().filter(|name| shown != Some(*name));
                self.connection
                    .send(self.sequence, &encode_input(intent, view_tick, name));
                if self.predicting {
                    report = self.predict(intent)?;
                    let shots = self.predicted_shots();
                    self.events.extend(shots);
                    let (position, motion, hook) = self.state();
                    self.pending.push_back(Pending {
                        sequence: self.sequence,
                        intent,
                        position,
                        motion,
                        hook,
                    });
                    while self.pending.len() > 120 {
                        self.pending.pop_front();
                    }
                }
            }
            Ok(Tick {
                report,
                events: std::mem::take(&mut self.events),
            })
        }
    }
}
