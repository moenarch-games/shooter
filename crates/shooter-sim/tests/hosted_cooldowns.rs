//! Gun and blade cooldowns predicted on a host at latency (shooter#163): snapshots carry what is
//! left of every gun copy's refire, the rocket launchers' salvo spacing, a swing's lockout, the
//! blade's and the grapple module's refire and the flame's ignition
//! ([`shooter_sim::weapons::Cooldowns`]), and a predicting client resumes them when it corrects,
//! as shooter#104 does for the grapple. Its own clock runs ahead through every correction, so
//! without them a rapid refire or swing is predicted allowed and refused by the host, or the
//! other way round.
//!
//! A person plays a host through delayed inputs and snapshots, predicted as the shipped online
//! client predicts (`shooter-client`'s `game::online`: placed back at the host's state at the
//! acknowledged input when the prediction is off, the unacknowledged inputs predicted again):
//! rapid fire from several weapon families and concurrent copies, and rapid blade swings over a
//! held trigger, at several latencies and through a late burst of inputs. Each input's predicted
//! shots and swings are the ones the host fires when it applies it, and the host's ticks replay
//! exactly.

use shooter_sim::physics::Vec3i;
use shooter_sim::robot::{Assembly, BETA, PartId, ROCKET_POD, STANDARD, STOKER, TWIN_RAIL};
use shooter_sim::weapons::{Cooldowns, Instance, Triggers, Weapon};
use shooter_sim::wire::{decode_snapshot, encode_snapshot};
use shooter_sim::{Arena, Event, Input, Intent, Rules, Session, Snapshot};

/// The person's player ID on the host.
const PERSON: u32 = 7;
/// The online client's tolerance (units, any axis) before it corrects a prediction.
const TOLERANCE_UNITS: f32 = 1.0;
/// Fixed delays each way, from none to a quarter second: the ticks before an input reaches the
/// host, and before a snapshot reaches the client.
const LATENCIES: [(u32, u32); 4] = [(0, 0), (2, 3), (6, 6), (9, 14)];
/// A second standing before the first shot, for the link to settle.
const SETTLE: u32 = 60;

/// A weapon copy that fired and the tube it left.
type Shot = (Instance, u8);

/// The shots and swings `events` hold for slot 0.
fn shots(events: &[Event]) -> Vec<Shot> {
    events
        .iter()
        .filter_map(|event| match *event {
            Event::Fired {
                slot: 0,
                weapon,
                part,
                tube,
                ..
            } => Some((Instance { part, weapon }, tube)),
            _ => None,
        })
        .collect()
}

/// An open floor, 40 m across: shots fly off into the distance.
fn floor() -> Arena {
    Arena {
        half_size: 2_000,
        pieces: Vec::new(),
        pads: Vec::new(),
        posts: Vec::new(),
        spawn: Vec3i::new(0, 0, 0),
        spawn_yaw: 0,
        spawns: Vec::new(),
        module_pads: Vec::new(),
        waypoints: Vec::new(),
        placements: Vec::new(),
        repair_anchors: Vec::new(),
    }
}

/// An input the client predicted and the host has not acknowledged yet: the predicted state
/// after it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Pending {
    sequence: u32,
    intent: Intent,
    center: [f32; 3],
    motion: shooter_sim::player::Motion,
    energy: [u32; 2],
    held: Instance,
    tubes: u32,
    cooldowns: Cooldowns,
}

/// One acknowledged prediction checked against the host's state.
#[derive(Clone, Copy, Debug)]
struct Check {
    ack: u32,
    /// How far (units, any axis) the prediction was from the host.
    distance: f32,
    /// Whether its cooldowns were the host's.
    same_cooldowns: bool,
    /// Whether its motion, electricity, fuel, held copy and next tubes were the host's.
    same_rest: bool,
    corrected: bool,
}

impl Check {
    /// Whether the prediction was the host's: every interval, the resources and the rest
    /// exactly, the robot to 2.5 mm.
    fn agrees(&self) -> bool {
        self.same_cooldowns && self.same_rest && self.distance < 0.25
    }
}

/// The online client's prediction, without its connection: snapshots in, inputs predicted.
struct Client {
    predictor: Session,
    predicting: bool,
    parts: shooter_sim::damage::PartDamage,
    pending: std::collections::VecDeque<Pending>,
    checks: Vec<Check>,
}

impl Client {
    fn new() -> Self {
        Self {
            predictor: Session::new(floor()).unwrap(),
            predicting: false,
            parts: shooter_sim::damage::PartDamage::default(),
            pending: std::collections::VecDeque::new(),
            checks: Vec::new(),
        }
    }

    /// The host's state at the acknowledged input, as the online client places it.
    fn place(&mut self, snapshot: &Snapshot) {
        let me = snapshot.of_player(PERSON).unwrap();
        let predictor = &mut self.predictor;
        predictor
            .place(0, me.robot.as_ref().unwrap(), me.motion, me.held, me.hook)
            .unwrap();
        predictor.place_debris(&snapshot.debris).unwrap();
        let tick = predictor.ticks();
        let fighter = &mut predictor.fighters[0];
        fighter.energy = me.energy;
        fighter.fuel = me.fuel;
        fighter.repair = me.repair;
        fighter.arsenal.tubes = me.tubes;
        fighter.arsenal.resume(tick, &me.cooldowns);
    }

    /// Predicts `intent`: the shots and swings the prediction fired.
    fn predict(&mut self, intent: Intent) -> Vec<Shot> {
        self.predictor.fighters[0].parts = self.parts;
        self.predictor.tick(intent).unwrap();
        shots(&self.predictor.events)
    }

    fn state(&self, sequence: u32, intent: Intent) -> Pending {
        let local = self.predictor.local();
        Pending {
            sequence,
            intent,
            center: local.center(),
            motion: local.motion,
            energy: [local.energy, local.fuel],
            held: local.arsenal.held,
            tubes: local.arsenal.tubes,
            cooldowns: local.arsenal.cooldowns(self.predictor.ticks()),
        }
    }

    fn reconcile(&mut self, snapshot: &Snapshot) {
        let me = snapshot.of_player(PERSON).unwrap();
        assert!(me.alive);
        self.parts = me.parts;
        self.predictor.fighters[0].build = me.build;
        self.predictor.fighters[0].parts = me.parts;
        self.predictor.fighters[0].carried = me.carried;
        if !self.predicting {
            self.place(snapshot);
            self.predicting = true;
            self.pending.clear();
            return;
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
            return;
        };
        self.pending.pop_front();
        let distance = (0..3)
            .map(|axis| (acked.center[axis] - me.position[axis]).abs())
            .fold(0.0, f32::max);
        let same_rest = acked.motion == me.motion
            && acked.energy == [me.energy, me.fuel]
            && acked.held == me.held
            && acked.tubes == me.tubes;
        let same_cooldowns = acked.cooldowns == me.cooldowns;
        let off = distance > TOLERANCE_UNITS || !same_rest || !same_cooldowns;
        self.checks.push(Check {
            ack: me.ack,
            distance,
            same_cooldowns,
            same_rest,
            corrected: off,
        });
        if off {
            self.place(snapshot);
            let replay: Vec<Pending> = self.pending.drain(..).collect();
            for pending in replay {
                self.predict(pending.intent);
                let state = self.state(pending.sequence, pending.intent);
                self.pending.push_back(state);
            }
        }
    }
}

/// How a hosted fight is played: the robot, the person's inputs tick by tick and the link's
/// delays.
struct Link<'a> {
    build: Assembly,
    /// The intent of each input, by sequence (1 the first).
    intents: &'a [Intent],
    /// The host tick each input (by sequence) reaches the host on: in order, never before it
    /// was sent.
    arrives: &'a dyn Fn(u32) -> u32,
    /// Ticks a snapshot takes to reach the client.
    down: u32,
}

/// What a hosted fight did.
struct Played {
    client: Client,
    /// Each host tick (1 the first): the input it applied, if a new one, and what it fired.
    host: Vec<(Option<u32>, Vec<Shot>)>,
    /// What the client predicted each input (by sequence) fired the first time it predicted it
    /// (the shots it shows), `None` before it predicted.
    predicted: Vec<Option<Vec<Shot>>>,
    /// Every host tick's snapshot, encoded.
    snapshots: Vec<Vec<u8>>,
    /// The inputs as the host queued them: the tick before which each was queued.
    log: Vec<(u32, Input)>,
}

/// The host, the person in slot 0 built as `build`, its pool and tanks full.
fn host(build: Assembly) -> Session {
    let rules = Rules {
        slots: 1,
        bots: false,
        local: false,
        ..Rules::solo()
    };
    let mut host = Session::with_rules(floor(), rules).unwrap();
    assert_eq!(host.join(PERSON).unwrap(), Some(0));
    host.fighters[0].configured = build;
    host.rebuild(0, build).unwrap();
    let fighter = &mut host.fighters[0];
    fighter.energy = fighter.pool().capacity;
    fighter.fuel = fighter.pool().fuel;
    host
}

/// Plays `link`: each tick the client takes the snapshots that have reached it, predicts and
/// sends its next input, and the host queues the inputs that have reached it and steps.
fn play(link: &Link) -> Played {
    let mut host = host(link.build);
    let mut client = Client::new();
    let ticks = u32::try_from(link.intents.len()).unwrap();
    let mut in_flight: Vec<(u32, Input)> = Vec::new();
    let mut snapshots_in_flight: Vec<(u32, Vec<u8>)> = Vec::new();
    let mut played = Played {
        client: Client::new(),
        host: Vec::new(),
        predicted: Vec::new(),
        snapshots: Vec::new(),
        log: Vec::new(),
    };
    let mut last_arrival = 0;
    let mut last_ack = 0;
    for tick in 1..=ticks {
        while snapshots_in_flight
            .first()
            .is_some_and(|(arrives, _)| *arrives <= tick)
        {
            let (_, bytes) = snapshots_in_flight.remove(0);
            client.reconcile(&decode_snapshot(&bytes).unwrap());
        }
        let intent = link.intents[usize::try_from(tick - 1).unwrap()];
        played.predicted.push(client.predicting.then(|| {
            let fired = client.predict(intent);
            let state = client.state(tick, intent);
            client.pending.push_back(state);
            fired
        }));
        let arrives = (link.arrives)(tick);
        assert!(arrives >= tick && arrives >= last_arrival, "in order");
        last_arrival = arrives;
        in_flight.push((
            arrives,
            Input {
                sequence: tick,
                intent,
                view_tick: 0,
            },
        ));
        while in_flight
            .first()
            .is_some_and(|(arrives, _)| *arrives <= tick)
        {
            let (_, input) = in_flight.remove(0);
            assert!(host.queue(PERSON, input));
            played.log.push((tick, input));
        }
        host.step().unwrap();
        let ack = host.fighters[0].ack;
        played
            .host
            .push(((ack != last_ack).then_some(ack), shots(&host.events)));
        last_ack = ack;
        let bytes = encode_snapshot(&host.snapshot());
        played.snapshots.push(bytes.clone());
        snapshots_in_flight.push((tick + link.down, bytes));
    }
    played.client = client;
    played
}

/// The host replayed from its input log: every tick's snapshot as it was.
fn replays_exactly(link: &Link, played: &Played) {
    let mut host = host(link.build);
    let mut log = played.log.iter().peekable();
    for (index, recorded) in played.snapshots.iter().enumerate() {
        let tick = u32::try_from(index).unwrap() + 1;
        while let Some((_, input)) = log.next_if(|(at, _)| *at == tick) {
            assert!(host.queue(PERSON, *input));
        }
        host.step().unwrap();
        assert!(
            encode_snapshot(&host.snapshot()) == *recorded,
            "the replay diverged on tick {tick}"
        );
    }
}

/// The acknowledged predictions from input `first` on that did not agree with the host.
fn disagreed(played: &Played, first: u32) -> Vec<Check> {
    played
        .client
        .checks
        .iter()
        .filter(|check| check.ack >= first && !check.agrees())
        .copied()
        .collect()
}

/// Each input from `first` on was predicted to fire what the host fired when it applied it
/// (every copy, its tube and every swing, refusals alike); how many shots and swings the host
/// fired from those inputs.
fn predicted_the_hosts_shots(played: &Played, first: u32, case: &str) -> usize {
    let mut fired = 0;
    for (tick, (applied, host)) in played.host.iter().enumerate() {
        let Some(sequence) = applied.filter(|sequence| *sequence >= first) else {
            continue;
        };
        let predicted = played.predicted[usize::try_from(sequence - 1).unwrap()]
            .as_ref()
            .unwrap_or_else(|| panic!("{case}: input {sequence} was never predicted"));
        assert_eq!(
            predicted,
            host,
            "{case}: input {sequence}, host tick {}",
            tick + 1
        );
        fired += host.len();
    }
    fired
}

/// The parts of `build`'s guns.
fn guns(build: &Assembly) -> Vec<PartId> {
    build
        .parts()
        .iter()
        .filter(|part| BETA.weapon(part).is_some_and(Weapon::gun))
        .map(|part| part.id)
        .collect()
}

/// `legs` of (ticks, intent) as one input per tick.
fn inputs(legs: &[(u32, Intent)]) -> Vec<Intent> {
    legs.iter()
        .flat_map(|(ticks, intent)| (0..*ticks).map(|_| *intent))
        .collect()
}

/// Every gun of `build` asked to fire at once, aimed level.
fn all_guns(build: &Assembly) -> Intent {
    let mut triggers = Triggers::NONE;
    for part in guns(build) {
        assert!(triggers.press(part));
    }
    Intent {
        triggers,
        ..Intent::IDLE
    }
}

/// A second standing, then the guns of `build` tapped faster than most refire (two ticks
/// held, one let go) for four seconds, then held for two, then a second standing.
fn rapid_fire(build: &Assembly) -> Vec<Intent> {
    let fire = all_guns(build);
    let mut legs = vec![(SETTLE, Intent::IDLE)];
    legs.extend((0..80).flat_map(|_| [(2, fire), (1, Intent::IDLE)]));
    legs.extend([(120, fire), (60, Intent::IDLE)]);
    inputs(&legs)
}

/// A second standing, then the blade's key tapped faster than it swings again (one tick down,
/// two up) for five seconds with the rifle held (each swing locks the guns), then a second
/// standing.
fn rapid_swings() -> Vec<Intent> {
    let rifle = all_guns(&STANDARD);
    let swing = Intent {
        melee: true,
        ..rifle
    };
    let mut legs = vec![(SETTLE, Intent::IDLE)];
    legs.extend((0..100).flat_map(|_| [(1, swing), (2, rifle)]));
    legs.push((60, Intent::IDLE));
    inputs(&legs)
}

/// The builds that fire rapidly: the standard build's rifle and shotgun side by side, the
/// twin rail guns (two copies of one gun, each on its own refire), the rocket pod (four
/// launchers under one salvo limit, alternating tubes) and the stoker's flamethrower (fuel and
/// ignition) beside its rifle.
const BUILDS: [(&str, Assembly); 4] = [
    ("standard", STANDARD),
    ("twin rail", TWIN_RAIL),
    ("rocket pod", ROCKET_POD),
    ("stoker", STOKER),
];

/// The first input firing or swinging in `intents`.
fn first_trigger(intents: &[Intent]) -> u32 {
    let at = intents
        .iter()
        .position(|intent| !intent.triggers.parts().is_empty() || intent.melee)
        .unwrap();
    u32::try_from(at).unwrap() + 1
}

#[test]
fn hosted_rapid_fire_is_predicted_at_every_latency() {
    for (name, build) in BUILDS {
        let intents = rapid_fire(&build);
        let first = first_trigger(&intents);
        let copies = guns(&build).len();
        for (up, down) in LATENCIES {
            let case = format!("{name} {up}/{down}");
            let arrives = move |sequence: u32| sequence + up;
            let link = Link {
                build,
                intents: &intents,
                arrives: &arrives,
                down,
            };
            let played = play(&link);
            let off = disagreed(&played, first);
            assert!(off.is_empty(), "{case}: {off:?}");
            let fired = predicted_the_hosts_shots(&played, first, &case);
            // The copies fired several times between them, until their resources ran out.
            assert!(fired >= 2 * copies, "{case}: {fired} shots");
            replays_exactly(&link, &played);
        }
    }
}

#[test]
fn hosted_rapid_swings_are_predicted_at_every_latency() {
    let intents = rapid_swings();
    let first = first_trigger(&intents);
    for (up, down) in LATENCIES {
        let case = format!("swings {up}/{down}");
        let arrives = move |sequence: u32| sequence + up;
        let link = Link {
            build: STANDARD,
            intents: &intents,
            arrives: &arrives,
            down,
        };
        let played = play(&link);
        let off = disagreed(&played, first);
        assert!(off.is_empty(), "{case}: {off:?}");
        predicted_the_hosts_shots(&played, first, &case);
        let swings = played
            .host
            .iter()
            .flat_map(|(_, shots)| shots)
            .filter(|(copy, _)| copy.weapon == Weapon::EnergyBlade)
            .count();
        // Refused taps: at most one swing per refire, but several.
        assert!((3..100).contains(&swings), "{case}: {swings} swings");
        replays_exactly(&link, &played);
    }
}

/// Inputs that arrive in a burst mid-fire (a late packet: the host holds the last input, firing
/// on, then works through the backlog): the client corrects, resumes the host's cooldowns and
/// predicts every shot and swing the host fires again.
#[test]
fn hosted_fire_and_swings_recover_from_late_inputs() {
    // From input 120 to 136 everything arrives at once, on tick 140; the rest three ticks late.
    let arrives = |sequence: u32| match sequence {
        120..=136 => 140,
        _ => sequence + 3,
    };
    let cases = BUILDS
        .iter()
        .map(|(name, build)| (*name, *build, rapid_fire(build)))
        .chain([("swings", STANDARD, rapid_swings())]);
    for (name, build, intents) in cases {
        let link = Link {
            build,
            intents: &intents,
            arrives: &arrives,
            down: 4,
        };
        let played = play(&link);
        let off = disagreed(&played, first_trigger(&intents));
        assert!(
            off.iter().any(|check| check.corrected),
            "{name}: the held inputs fired on the host past the prediction"
        );
        // Once the backlog is worked through, every acknowledged prediction agrees again, and
        // every input predicted after it fires what the host does.
        let settled = off.last().unwrap().ack;
        assert!(settled < 200, "{name}: still off at {off:?}");
        // The client corrected when the host's snapshot after `settled` reached it: the inputs
        // it predicted from then on.
        let applied = played
            .host
            .iter()
            .position(|(applied, _)| *applied == Some(settled))
            .unwrap();
        let corrected = u32::try_from(applied).unwrap() + 1 + link.down;
        predicted_the_hosts_shots(&played, corrected + 1, name);
        replays_exactly(&link, &played);
    }
}

/// A snapshot carries the cooldowns exactly: a fighter mid-refire, mid-salvo and mid-swing
/// decodes to the arsenal's own values, and a client resuming them refuses and grants as the
/// host's arsenal does from then on.
#[test]
fn snapshots_carry_the_cooldowns_a_client_resumes() {
    let mut session = host(ROCKET_POD);
    let fire = all_guns(&ROCKET_POD);
    for sequence in 1..=5 {
        session.queue(
            PERSON,
            Input {
                sequence,
                intent: fire,
                view_tick: 0,
            },
        );
        session.step().unwrap();
    }
    let tick = session.ticks();
    let arsenal = session.fighters[0].arsenal;
    let cooldowns = arsenal.cooldowns(tick);
    let rocket = usize::from(Weapon::RocketLauncher.index());
    assert!(cooldowns.families[rocket] > 0 || !cooldowns.refiring().is_empty());
    let snapshot = decode_snapshot(&encode_snapshot(&session.snapshot())).unwrap();
    assert_eq!(snapshot.of_player(PERSON).unwrap().cooldowns, cooldowns);
    // Resumed on a clock 1 000 ticks ahead, the same intervals are left.
    let mut resumed = shooter_sim::weapons::Arsenal::default();
    resumed.resume(tick + 1_000, &cooldowns);
    assert_eq!(resumed.cooldowns(tick + 1_000), cooldowns);
    for copy in guns(&ROCKET_POD) {
        let instance = Instance::new(copy.0, Weapon::RocketLauncher);
        for later in 0..120 {
            assert_eq!(
                resumed.cooldown_of(instance, tick + 1_000 + later),
                arsenal.cooldown_of(instance, tick + later),
            );
        }
    }
}
