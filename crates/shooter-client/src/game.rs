//! Where the client's match comes from: offline practice runs the deathmatch in-process
//! (`shooter-sim`, the local player in slot 0 against bots), online play (native and in the
//! browser) follows a game-server host and predicts the local fighter. Both hand the frame the
//! same [`Snapshot`] of the match, so drawing and the HUD do not care which it is.

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
        }
    }

    /// One local tick holding `intent`.
    pub fn tick(&mut self, intent: Intent) -> Result<Tick, String> {
        match self {
            Self::Practice { session, snapshot } => {
                let report = session.tick(intent).map_err(|error| error.to_string())?;
                *snapshot = session.snapshot();
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
    use crate::net::{Connection, NetEvent, Trust};
    use shooter_sim::grapple::Hook;
    use shooter_sim::player::Motion;
    use shooter_sim::session::MAX_HEALTH;
    use shooter_sim::wire::encode_input;
    use shooter_sim::{Arena, Event, Intent, Session, Snapshot, StepReport};
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
        player_id: Option<u32>,
        predictor: Session,
        predicting: bool,
        pending: VecDeque<Pending>,
        sequence: u32,
        latest: Option<Snapshot>,
        events: Vec<Event>,
        status: Option<String>,
    }

    impl Online {
        /// # Errors
        ///
        /// If the arena does not build.
        pub fn connect(url: String, certificate: Trust) -> Result<Self, String> {
            let status = format!("connecting to {url}");
            Ok(Self {
                connection: Connection::open(url, certificate),
                player_id: None,
                predictor: Session::new(Arena::greybox()).map_err(|error| error.to_string())?,
                predicting: false,
                pending: VecDeque::new(),
                sequence: 0,
                latest: None,
                events: Vec::new(),
                status: Some(status),
            })
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
                        self.events.extend_from_slice(&snapshot.events);
                        self.reconcile(&snapshot)?;
                        self.latest = Some(*snapshot);
                    }
                    NetEvent::Closed(reason) => {
                        self.status = Some(format!("disconnected: {reason}"));
                        self.predicting = false;
                    }
                }
            }
            let mut report = StepReport::default();
            if self.player_id.is_some() && self.status.is_none() {
                self.sequence += 1;
                let view_tick = self.latest.as_ref().map_or(0, |latest| latest.tick);
                self.connection
                    .send(self.sequence, &encode_input(intent, view_tick));
                if self.predicting {
                    report = self.predict(intent)?;
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
