//! Where the client's match comes from: offline practice runs the deathmatch in-process
//! (`shooter-sim`, the local player in slot 0 against bots), online play (native and in the
//! browser) follows a game-server host and predicts the local fighter. Both hand the frame the
//! same [`Snapshot`] of the match, so drawing and the HUD do not care which it is. Practice
//! plays the map it is given; online, the host's snapshots name the map.

use crate::interpolation::Interpolation;
use crate::robots::Frames;
use crate::scene::METRES_PER_UNIT;
use render_engine::glam::Vec3;
use shooter_sim::physics::World;
use shooter_sim::session::{Fighter, LOCAL_PLAYER};
use shooter_sim::{Assembly, Event, Intent, MapId, Rules, Session, Snapshot, StepReport};

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
        snapshot: Box<Snapshot>,
        others: Interpolation,
    },
    Online(Box<online::Online>),
}

impl Game {
    /// Offline practice against bots on `map`.
    ///
    /// # Panics
    ///
    /// If the map's arena does not build.
    #[must_use]
    pub fn practice(map: MapId) -> Self {
        let session =
            Session::on_map(map, Rules::practice(PRACTICE_SEED)).expect("a catalogued map builds");
        let snapshot = session.snapshot();
        Self::Practice {
            session: Box::new(session),
            snapshot: Box::new(snapshot),
            // Drawn between the last two ticks; shots see the match as it is.
            others: Interpolation::new(1.0),
        }
    }

    /// Plays as `build` (a named loadout, `shooter_sim::robot::LOADOUTS`) from the start, called
    /// before the first tick ([`Game::configure`] before the robot has played).
    #[must_use]
    pub fn with_loadout(mut self, build: Assembly) -> Self {
        self.configure(build);
        self
    }

    /// Asks for the local robot to be `build` (a validated build, the builder's Ready): in
    /// practice the in-process session applies it, online it rides along with the inputs to the
    /// host. Either way the session's one rule decides when ([`Session::configure`]): at the
    /// next spawn, or at once before the robot has played.
    pub fn configure(&mut self, build: Assembly) {
        match self {
            Self::Practice {
                session, snapshot, ..
            } => {
                session.configure(LOCAL_PLAYER, build);
                **snapshot = session.snapshot();
            }
            Self::Online(online) => online.request_build(build),
        }
    }

    /// The build the local robot comes back as, as the match (the host, online) last said.
    #[must_use]
    pub fn configured(&self) -> Option<Assembly> {
        let slot = self.local_slot()?;
        Some(self.snapshot()?.slot(slot)?.configured)
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
                **snapshot = session.snapshot();
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

    /// The map the match is played on, once it is known (online: from the first snapshot).
    #[must_use]
    pub fn map(&self) -> Option<MapId> {
        match self {
            Self::Practice { session, .. } => session.map,
            Self::Online(online) => online.map(),
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

    /// Where `slot`'s mechanical robot's bodies are drawn `blend` (0..1) of a tick after the
    /// drawing clock (between the snapshots around it), while it is alive there.
    #[must_use]
    pub fn other_frames(&self, slot: u8, blend: f32) -> Option<Frames> {
        match self {
            Self::Practice { others, .. } => others.frames(slot, blend),
            Self::Online(online) => online.other_frames(slot, blend),
        }
    }

    /// Where the local fighter's mechanical robot's bodies are now (predicted online), while it
    /// is alive.
    #[must_use]
    pub fn local_frames(&self) -> Option<Frames> {
        let session = match self {
            Self::Practice { session, .. } => Some(&**session),
            Self::Online(online) => online.live(),
        }?;
        let fighter = session.local();
        if !fighter.alive() {
            return None;
        }
        let center = Vec3::from(fighter.center()) * METRES_PER_UNIT;
        Frames::of_robot(&session.mechanics, fighter.slot, center)
    }

    /// The physics world of the match's map, once it is known: the practice match's own, or
    /// online the predictor's. The camera keeps clear of its arena and the reticle traces it.
    #[must_use]
    pub fn world(&self) -> Option<&World> {
        match self {
            Self::Practice { session, .. } => Some(&session.world),
            Self::Online(online) => online.world(),
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
            Self::Practice { session, .. } => {
                session.local().alive().then(|| session.local().center())
            }
            Self::Online(online) => online.local_center(),
        }
    }

    /// The local fighter as it stands now (predicted online, while it is alive) and the tick it
    /// stands at: the weapon it holds and when that may fire, its energy, its build and parts.
    /// The weapon HUD and the weapon cycle read it.
    #[must_use]
    pub fn local_fighter(&self) -> Option<(&Fighter, u32)> {
        let session = match self {
            Self::Practice { session, .. } => Some(&**session),
            Self::Online(online) => online.live(),
        }?;
        let fighter = session.local();
        fighter.alive().then(|| (fighter, session.ticks()))
    }

    /// Whether the local fighter is sliding (the camera dips).
    #[must_use]
    pub fn local_sliding(&self) -> bool {
        match self {
            Self::Practice { session, .. } => session.local().motion.sliding,
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
    //! one-fighter session of the map the host's snapshots name, and reconciled with every snapshot (reset to the
    //! server's state at the acknowledged input and the unacknowledged inputs replayed, when the
    //! prediction was off).

    use super::Tick;
    use crate::interpolation::{Interpolation, ONLINE_DELAY_TICKS};
    use crate::net::{Connection, NetEvent, Trust};
    use shooter_sim::damage::{Hitboxes, PartDamage};
    use shooter_sim::grapple::Hook;
    use shooter_sim::names;
    use shooter_sim::player::Motion;
    use shooter_sim::weapons::Delivery;
    use shooter_sim::wire::encode_input;
    use shooter_sim::{
        Assembly, Event, Intent, MapId, Rules, Session, Snapshot, StepReport, Weapon,
    };
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
        /// The charge in its energy pool (milli-units).
        energy: u32,
        /// The weapon it holds (the host may refuse a switch the prediction made).
        held: Weapon,
    }

    pub struct Online {
        /// The host, until the first tick opens the connection to it (nobody joins while the
        /// person is still at the builder before the match, shooter#57).
        host: Option<(String, Trust)>,
        connection: Option<Connection>,
        /// The name picked for this match (sanitized as the host does).
        name: Option<String>,
        /// The loadout asked of the host, if any.
        build: Option<Assembly>,
        /// The local fighter's part damage as the host last said (the predictor keeps it).
        parts: PartDamage,
        player_id: Option<u32>,
        /// A one-fighter session on the host's map, from the first snapshot on.
        predictor: Option<Session>,
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
        #[must_use]
        pub fn connect(url: String, certificate: Trust, name: Option<&str>) -> Self {
            let status = format!("connecting to {url}");
            Self {
                host: Some((url, certificate)),
                connection: None,
                name: name.and_then(names::sanitize),
                build: None,
                parts: PartDamage::default(),
                player_id: None,
                predictor: None,
                predicting: false,
                pending: VecDeque::new(),
                sequence: 0,
                latest: None,
                others: Interpolation::new(ONLINE_DELAY_TICKS),
                events: Vec::new(),
                status: Some(status),
                resumed: 0,
            }
        }

        /// Asks the host to bring the local fighter back as `build` (it rides along with the
        /// inputs until a snapshot shows it configured).
        pub const fn request_build(&mut self, build: Assembly) {
            self.build = Some(build);
        }

        /// The host's map, once a snapshot has named it.
        pub fn map(&self) -> Option<MapId> {
            self.predictor.as_ref()?.map
        }

        /// The predictor for the map `snapshot` names, made (or remade, if the map is another)
        /// on the first snapshot.
        fn follow_map(&mut self, snapshot: &Snapshot) -> Result<(), String> {
            let map = snapshot
                .map
                .ok_or("the host plays an arena outside the map catalog")?;
            if self.map() != Some(map) {
                self.predictor =
                    Some(Session::on_map(map, Rules::solo()).map_err(|error| error.to_string())?);
                self.predicting = false;
                self.pending.clear();
            }
            Ok(())
        }

        /// The predictor's world, once a snapshot has named the map.
        pub fn world(&self) -> Option<&shooter_sim::physics::World> {
            self.predictor.as_ref().map(|predictor| &predictor.world)
        }

        /// The predictor while it predicts the local fighter.
        pub(super) fn live(&self) -> Option<&Session> {
            self.predictor.as_ref().filter(|_| self.predicting)
        }

        fn predictor_mut(&mut self) -> Result<&mut Session, String> {
            self.predictor
                .as_mut()
                .ok_or_else(|| "no snapshot has named the map yet".to_owned())
        }

        /// Drops the connection without closing it, as a network loss would (testing the
        /// reconnect).
        #[cfg(not(target_arch = "wasm32"))]
        #[doc(hidden)]
        pub fn sever(&self) {
            if let Some(connection) = &self.connection {
                connection.sever();
            }
        }

        pub fn other_center(&self, slot: u8, blend: f32) -> Option<[f32; 3]> {
            self.others.center(slot, blend)
        }

        pub fn other_frames(&self, slot: u8, blend: f32) -> Option<crate::robots::Frames> {
            self.others.frames(slot, blend)
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
            let others: Vec<Hitboxes> = self
                .latest
                .iter()
                .flat_map(|snapshot| &snapshot.fighters)
                .filter(|fighter| fighter.slot != local && fighter.alive)
                .filter_map(|fighter| {
                    let center = self.others.center(fighter.slot, 0.0)?;
                    Some(Hitboxes::place(
                        &fighter.build,
                        &fighter.parts,
                        center,
                        fighter.yaw,
                    ))
                })
                .collect();
            let mut shots = Vec::new();
            for event in self
                .live()
                .into_iter()
                .flat_map(|predictor| &predictor.events)
            {
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
                                    .filter_map(|parts| parts.first(from, translation))
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
            self.live().map(|predictor| predictor.local().center())
        }

        pub fn sliding(&self) -> bool {
            self.live()
                .is_some_and(|predictor| predictor.local().motion.sliding)
        }

        fn predict(&mut self, intent: Intent) -> Result<StepReport, String> {
            let parts = self.parts;
            let predictor = self.predictor_mut()?;
            // Part damage is the host's alone: the predicted fighter keeps the host's last word.
            predictor.fighters[0].parts = parts;
            predictor.tick(intent).map_err(|error| error.to_string())
        }

        fn state(&self) -> ([f32; 3], Motion, Option<Hook>, u32, Weapon) {
            self.live().map_or(
                ([0.0; 3], Motion::default(), None, 0, Weapon::default()),
                |predictor| {
                    let local = predictor.local();
                    (
                        local.center(),
                        local.motion,
                        local.hook,
                        local.energy,
                        local.arsenal.held,
                    )
                },
            )
        }
        pub fn hook(&self) -> Option<[f32; 3]> {
            self.live()?.local().hook.map(|hook| hook.anchor_units())
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
            // Predicted shots fire at the host's pace for this life's build (its weapon mods)
            // and module, and jumps leave as the host's do; the pads are as the host has them.
            // The energy pool is predicted like the body: reset to the host's charge with it. The
            // part damage is the host's: a lost weapon does not fire and a robot without
            // connected locomotion does not walk in the prediction either.
            self.parts = me.parts;
            if let Some(predictor) = self.predictor.as_mut() {
                predictor.fighters[0].build = me.build;
                predictor.fighters[0].parts = me.parts;
                predictor.fighters[0].carried = me.carried;
                for (pickup, host) in predictor.pickups.iter_mut().zip(&snapshot.pickups) {
                    pickup.ready_in = host.ready_in;
                }
            }
            let place = |predictor: &mut Session| {
                // Its robot resumes in the host's exact state (bodies, lost parts, gait clock).
                let robot = me
                    .robot
                    .as_ref()
                    .ok_or("the host sent a living robot without its state")?;
                predictor
                    .place(0, robot, me.motion, me.held, me.hook)
                    .map_err(|error| error.to_string())?;
                // Among the host's physical debris, exactly (shooter#75).
                predictor
                    .place_debris(&snapshot.debris)
                    .map_err(|error| error.to_string())?;
                predictor.fighters[0].energy = me.energy;
                Ok::<(), String>(())
            };
            if !self.predicting {
                place(self.predictor_mut()?)?;
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
                || acked.hook != me.hook
                || acked.energy != me.energy
                || acked.held != me.held;
            if off {
                place(self.predictor_mut()?)?;
                let replay: Vec<Pending> = self.pending.drain(..).collect();
                for mut pending in replay {
                    self.predict(pending.intent)?;
                    (
                        pending.position,
                        pending.motion,
                        pending.hook,
                        pending.energy,
                        pending.held,
                    ) = self.state();
                    self.pending.push_back(pending);
                }
            }
            Ok(())
        }

        pub fn tick(&mut self, intent: Intent) -> Result<Tick, String> {
            if let Some((url, certificate)) = self.host.take() {
                self.connection = Some(Connection::open(url, certificate));
            }
            let events = self
                .connection
                .as_ref()
                .map(Connection::poll)
                .unwrap_or_default();
            for event in events {
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
                        self.follow_map(&snapshot)?;
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
                // So does the requested loadout, until a snapshot shows it configured.
                let configured = self
                    .latest
                    .as_ref()
                    .and_then(|latest| latest.of_player(self.player_id?).map(|me| me.configured));
                let build = self.build.filter(|build| configured != Some(*build));
                if let Some(connection) = &self.connection {
                    connection.send(
                        self.sequence,
                        &encode_input(intent, view_tick, name, build.as_ref()),
                    );
                }
                if self.predicting {
                    report = self.predict(intent)?;
                    let shots = self.predicted_shots();
                    self.events.extend(shots);
                    let (position, motion, hook, energy, held) = self.state();
                    self.pending.push_back(Pending {
                        sequence: self.sequence,
                        intent,
                        position,
                        motion,
                        hook,
                        energy,
                        held,
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
