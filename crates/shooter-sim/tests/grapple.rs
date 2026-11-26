//! The grapple module (Atlas D2.i104.1, D2.i104.2, shooter#104): an optional socket-mounted
//! robot part on its own key, never a gun in the rotation. A robot without a working module has
//! no grapple; the hook leaves the module's muzzle; it pulls toward its anchor and swings on a
//! rope that never stretches; letting go, losing the module or dying ends it; each hook costs
//! the module's energy from the shared pool and waits its interval.

use shooter_sim::grapple::{MIN_LENGTH_UNITS, RANGE_UNITS};
use shooter_sim::physics::{SUBUNITS_PER_UNIT, Vec3i};
use shooter_sim::robot::{BETA, STANDARD};
use shooter_sim::{Arena, Event, Intent, Piece, Session, Weapon, scenarios};

/// The grapple scenario's hook: facing the pillar on the ledge, the grapple key held.
fn hook_intent() -> Intent {
    let leg = scenarios::GRAPPLE
        .legs
        .iter()
        .find(|leg| leg.name == "hook and reel in")
        .unwrap();
    assert!(leg.intent.grapple && !leg.intent.fire);
    leg.intent
}

/// The grapple scenario's arena with the local robot settled on the floor as `build`, its pool
/// full.
fn settled(build: shooter_sim::robot::Assembly) -> Session {
    let mut session = Session::new((scenarios::GRAPPLE.arena)()).unwrap();
    session.rebuild(0, build).unwrap();
    let aim = Intent {
        grapple: false,
        ..hook_intent()
    };
    for _ in 0..20 {
        session.tick(aim).unwrap();
    }
    let fighter = &mut session.fighters[0];
    fighter.energy = fighter.pool().capacity;
    session
}

/// The standard build without its grapple module (part 9): still a valid robot.
fn standard_without_module() -> shooter_sim::robot::Assembly {
    let mut candidate = STANDARD.candidate();
    let before = candidate.parts.len();
    candidate.parts.retain(|part| part.id != 9);
    assert_eq!(candidate.parts.len(), before - 1);
    BETA.validate(&candidate).unwrap()
}

fn module_index(session: &Session) -> usize {
    let build = session.fighters[0].build;
    build
        .parts()
        .iter()
        .position(|part| BETA.weapon(part) == Some(Weapon::Grapple))
        .unwrap()
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3)
        .map(|axis| (a[axis] - b[axis]) * (a[axis] - b[axis]))
        .sum::<f32>()
        .sqrt()
}

/// Rope length in units.
#[allow(clippy::cast_precision_loss)]
fn units(subunits: i32) -> f32 {
    subunits as f32 / SUBUNITS_PER_UNIT as f32
}

#[test]
fn a_robot_without_the_module_has_no_grapple() {
    let mut session = settled(standard_without_module());
    let charge = session.fighters[0].energy;
    for _ in 0..90 {
        let report = session.tick(hook_intent()).unwrap();
        assert_ne!(
            report.fired.map(|fired| fired.weapon),
            Some(Weapon::Grapple)
        );
        assert!(session.local().hook.is_none());
        assert!(!session.events.iter().any(|event| matches!(
            event,
            Event::Fired {
                weapon: Weapon::Grapple,
                ..
            }
        )));
    }
    // Nothing spent: the pool stays full.
    assert_eq!(session.fighters[0].energy, charge);
    // And no select key makes it a held weapon.
    session
        .tick(Intent {
            select: Some(Weapon::Grapple.into()),
            ..Intent::IDLE
        })
        .unwrap();
    assert_eq!(session.local().arsenal.held.weapon, Weapon::EnergyRifle);
}

#[test]
fn the_hook_leaves_the_module_muzzle_on_its_own_key_whatever_gun_is_held() {
    let mut session = settled(STANDARD);
    // The trigger alone never hooks (the rifle fires), and the grapple is never held.
    let report = session
        .tick(Intent {
            fire: true,
            ..hook_intent()
        })
        .unwrap();
    assert_eq!(session.local().arsenal.held.weapon, Weapon::EnergyRifle);
    // The rifle and the hook both went out this tick: separate keys, separate cooldowns.
    let fired: Vec<(Weapon, [f32; 3])> = session
        .events
        .iter()
        .filter_map(|event| match event {
            Event::Fired { weapon, from, .. } => Some((*weapon, *from)),
            _ => None,
        })
        .collect();
    assert!(
        fired
            .iter()
            .any(|(weapon, _)| *weapon == Weapon::EnergyRifle)
    );
    assert_eq!(
        report.fired.map(|fired| fired.weapon),
        Some(Weapon::EnergyRifle)
    );
    let hook = session.local().hook.expect("the hook set");

    // From its own muzzle, not the eye: where the module's anchor sits on the robot.
    let mut session = settled(STANDARD);
    let muzzle = session.local().muzzle(Weapon::Grapple).unwrap();
    let eye = session.local().eye();
    let report = session.tick(hook_intent()).unwrap();
    assert_eq!(
        report.fired.map(|fired| fired.weapon),
        Some(Weapon::Grapple)
    );
    let from = session
        .events
        .iter()
        .find_map(|event| match event {
            Event::Fired {
                weapon: Weapon::Grapple,
                from,
                ..
            } => Some(*from),
            _ => None,
        })
        .unwrap();
    assert!(distance(from, muzzle.at) < 0.01, "{from:?} {muzzle:?}");
    assert!(distance(from, eye) > 20.0, "{from:?} {eye:?}");
    let line = session.events.iter().find_map(|event| match event {
        Event::Beam {
            weapon: Weapon::Grapple,
            from,
            struck,
            ..
        } => Some((*from, *struck)),
        _ => None,
    });
    assert_eq!(line, Some((from, true)));
    // It set on the pillar's face (arena geometry), within reach, with the rope tied at the
    // module: measured from its muzzle, not the eye.
    let set = session.local().hook.unwrap();
    assert_eq!(set.anchor, hook.anchor);
    let anchor = set.anchor_units();
    assert!((anchor[2] - 650.0).abs() < 1.0, "{anchor:?}");
    assert!(distance(anchor, from) < RANGE_UNITS as f32);
    assert!((units(set.length) - distance(anchor, from)).abs() < 25.0);
    assert!((distance(anchor, eye) - distance(anchor, from)).abs() > 25.0);

    // Turned a quarter round, the mount turns with the robot: the hook leaves it still.
    let mut session = settled(STANDARD);
    let turned = Intent {
        yaw: 16_384,
        ..Intent::IDLE
    };
    for _ in 0..60 {
        session.tick(turned).unwrap();
    }
    let muzzle = session.local().muzzle(Weapon::Grapple).unwrap();
    assert!(muzzle.at[0] - session.local().eye()[0] > 20.0, "{muzzle:?}");
    session
        .tick(Intent {
            grapple: true,
            ..turned
        })
        .unwrap();
    let from = session.events.iter().find_map(|event| match event {
        Event::Fired {
            weapon: Weapon::Grapple,
            from,
            ..
        } => Some(*from),
        _ => None,
    });
    assert!(
        distance(from.unwrap(), muzzle.at) < 0.01,
        "{from:?} {muzzle:?}"
    );
}

#[test]
fn the_hook_pulls_toward_its_anchor_and_the_rope_only_shortens() {
    let mut session = settled(STANDARD);
    let start = session.local().center();
    session.tick(hook_intent()).unwrap();
    let anchor = session.local().hook.unwrap().anchor_units();
    let mut length = session.local().hook.unwrap().length;
    let first = distance(session.local().eye(), anchor);
    for _ in 0..60 {
        session.tick(hook_intent()).unwrap();
        let hook = session.local().hook.expect("held while the key is");
        assert_eq!(hook.anchor_units(), anchor);
        assert!(hook.length <= length, "the rope never pays out");
        length = hook.length;
    }
    let now = distance(session.local().eye(), anchor);
    assert!(now < first - 500.0, "pulled in: {first} -> {now}");
    assert!(session.local().center()[1] > start[1] + 50.0, "lifted");
    assert!(units(length) >= MIN_LENGTH_UNITS as f32);
}

/// A ceiling slab 12 m up and 8 m ahead: hooked under it from the floor, the robot is reeled up
/// and its momentum carries it past under the anchor, where the rope swings it back.
fn under_a_ceiling() -> Arena {
    Arena {
        half_size: 3_000,
        pieces: vec![Piece::Block {
            center: Vec3i::new(0, 1_220, 800),
            half: Vec3i::new(800, 20, 400),
        }],
        pads: Vec::new(),
        posts: Vec::new(),
        spawn: Vec3i::new(0, 0, -600),
        spawn_yaw: 0,
        spawns: Vec::new(),
        module_pads: Vec::new(),
        waypoints: Vec::new(),
        placements: Vec::new(),
        repair_anchors: Vec::new(),
    }
}

#[test]
fn the_rope_swings_the_robot_round_its_anchor_without_stretching() {
    let mut session = Session::new(under_a_ceiling()).unwrap();
    for _ in 0..20 {
        session.tick(Intent::IDLE).unwrap();
    }
    // Up at the ceiling's near edge, about 41° above level.
    let swing = Intent {
        pitch: 7_500,
        grapple: true,
        ..Intent::IDLE
    };
    session.tick(swing).unwrap();
    let hook = session.local().hook.expect("hooked the ceiling");
    let anchor = hook.anchor_units();
    assert!((anchor[1] - 1_200.0).abs() < 1.0, "{anchor:?}");
    let mut beyond = false;
    let mut back = false;
    let mut track = Vec::new();
    for _ in 0..240 {
        session.tick(swing).unwrap();
        let fighter = session.local();
        let hook = fighter.hook.expect("held");
        // The rope holds the module within its length: the give of a tick's gravity, and of a
        // tick's travel when the module crosses its length mid-tick (the rope acts from the
        // next, [`shooter_sim::grapple::Hook::pull`]).
        let velocity = fighter.velocity();
        let travel = distance(velocity, [0.0; 3]) / 60.0;
        let span = distance(fighter.rope_tie().expect("the module on"), anchor);
        assert!(
            span <= units(hook.length) + 15.0 + travel,
            "{span} > {} + {travel}",
            units(hook.length)
        );
        track.push((fighter.eye(), velocity));
        // Past under the anchor, still moving on, then drawn back the other way: a pendulum.
        beyond |= fighter.eye()[2] > anchor[2] + 50.0 && velocity[2] > 0.0;
        back |= beyond && velocity[2] < -50.0;
    }
    assert!(beyond && back, "{track:?}");
}

#[test]
fn letting_go_releases_the_rope_and_keeps_the_momentum() {
    let mut session = settled(STANDARD);
    for _ in 0..50 {
        session.tick(hook_intent()).unwrap();
    }
    assert!(session.local().hook.is_some());
    let moving = session.local().velocity();
    let let_go = Intent {
        grapple: false,
        ..hook_intent()
    };
    session.tick(let_go).unwrap();
    assert!(session.local().hook.is_none());
    let after = session.local().velocity();
    // Still flying up the face: no more than the reel's and a tick of gravity's pull lost.
    assert!(
        after[1] > 0.0 && after[1] > moving[1] - 40.0,
        "{moving:?} -> {after:?}"
    );
    // Its interval passed while it held: pressed again, it hooks again at once.
    session.tick(hook_intent()).unwrap();
    assert!(session.local().hook.is_some());
}

#[test]
fn losing_the_module_mid_pull_ends_the_grapple() {
    let mut session = settled(STANDARD);
    for _ in 0..10 {
        session.tick(hook_intent()).unwrap();
    }
    assert!(session.local().hook.is_some());
    let index = module_index(&session);
    let fighter = &mut session.fighters[0];
    let build = fighter.build;
    fighter.parts.hurt(&build, index, 1_000);
    assert!(!fighter.parts.armed(&build, Weapon::Grapple));
    let charge = fighter.energy;
    session.tick(hook_intent()).unwrap();
    assert!(
        session.local().hook.is_none(),
        "the rope goes with the module"
    );
    // Held on, it never hooks again this life and spends nothing.
    for _ in 0..90 {
        let report = session.tick(hook_intent()).unwrap();
        assert!(session.local().hook.is_none());
        assert_ne!(
            report.fired.map(|fired| fired.weapon),
            Some(Weapon::Grapple)
        );
    }
    assert!(session.fighters[0].energy >= charge);
}

#[test]
fn each_hook_pays_its_energy_from_the_shared_pool_and_waits_its_interval() {
    let cost = Weapon::Grapple.energy();
    // Too little charge: refused, nothing spent and no cooldown.
    let mut session = settled(STANDARD);
    session.fighters[0].energy = cost - 1;
    session.tick(hook_intent()).unwrap();
    assert!(session.local().hook.is_none());
    assert!(session.fighters[0].energy >= cost - 1);
    assert_eq!(session.local().arsenal.grapple_cooldown(session.ticks()), 0);

    // Enough: the hook takes its cost (less a tick's regeneration).
    let mut session = settled(STANDARD);
    let full = session.fighters[0].energy;
    session.tick(hook_intent()).unwrap();
    assert!(session.local().hook.is_some());
    let spent = full - session.fighters[0].energy;
    assert!(
        (cost - 1_000..=cost).contains(&spent),
        "spent {spent} of {full}"
    );

    // Let go and pressed again at once: refused until its interval has passed.
    let let_go = Intent {
        grapple: false,
        ..hook_intent()
    };
    session.tick(let_go).unwrap();
    let mut rehooked = None;
    for tick in 0..Weapon::Grapple.refire_ticks() {
        session.tick(hook_intent()).unwrap();
        if session.local().hook.is_some() {
            rehooked = Some(tick);
            break;
        }
    }
    let waited = rehooked.expect("hooks again once ready");
    assert!(waited + 2 >= Weapon::Grapple.refire_ticks() - 1, "{waited}");
}

#[test]
fn bots_never_grapple() {
    let mut session =
        Session::with_rules(Arena::greybox(), shooter_sim::Rules::deathmatch(7)).unwrap();
    for _ in 0..600 {
        session.step().unwrap();
        assert!(
            session
                .fighters
                .iter()
                .all(|fighter| fighter.hook.is_none())
        );
    }
}

/// The scripted people of the replays (`shooter-replay`) hold the grapple key now and then: a
/// recorded match logs the module's activations, and replaying it checks them tick by tick.
#[test]
fn the_scripted_people_grapple() {
    let mut session =
        Session::with_rules(Arena::greybox(), shooter_sim::Rules::deathmatch(7)).unwrap();
    let slot = session.join(1).unwrap().unwrap();
    let mut hooks = 0;
    let mut set = 0;
    for tick in 0..1_200 {
        session.queue(
            1,
            shooter_sim::Input {
                sequence: tick + 1,
                intent: scenarios::scripted_intent(0, tick),
                view_tick: session.ticks(),
            },
        );
        session.step().unwrap();
        hooks += session
            .events
            .iter()
            .filter(|event| {
                matches!(event, Event::Fired { slot: from, weapon: Weapon::Grapple, .. } if *from == slot)
            })
            .count();
        set += usize::from(session.fighters[usize::from(slot)].hook.is_some());
    }
    assert!(hooks > 0 && set > 0, "{hooks} hooks, {set} ticks hooked");
}

// Hosted latency (shooter#104): a person grapples on a host through a link that delays their
// inputs and the host's snapshots, and their client predicts as the shipped online client does
// (`shooter-client`'s `game::online::Online`: every input predicted at once and kept until the
// host acknowledges it, the acknowledged prediction checked against the host's state, and a
// mismatch placed back at the host's state with the unacknowledged inputs predicted again).

/// The person's player ID on the host.
const PERSON: u32 = 7;
/// The online client's tolerance (units, any axis) before it corrects a prediction.
const TOLERANCE_UNITS: f32 = 1.0;

/// An input the client predicted and the host has not acknowledged yet.
#[derive(Clone, Copy)]
struct Pending {
    sequence: u32,
    intent: Intent,
    center: [f32; 3],
    motion: shooter_sim::player::Motion,
    hook: Option<shooter_sim::grapple::Hook>,
    energy: [u32; 2],
    held: shooter_sim::weapons::Instance,
    tubes: u32,
}

/// One acknowledged prediction checked against the host's state.
#[derive(Clone, Copy, Debug)]
struct Check {
    ack: u32,
    /// How far (units, any axis) the prediction was from the host.
    distance: f32,
    /// Whether the prediction's hook was set where the host's was ([`same_hook`]).
    same_hook: bool,
    /// How far (subunits) the prediction's rope length was from the host's.
    rope_off: i32,
    /// Whether its motion, electricity, fuel, held copy and next tubes were the host's.
    same_rest: bool,
    /// Whether the client corrected it.
    corrected: bool,
}

/// 2.5 mm: the most a prediction that agrees with the host is off, any axis (the client corrects
/// past 1 cm). The articulated world's restore keeps a robot within float rounding of the host's
/// path, not bit for bit, through contacts (a ledge's, a ceiling's), so a rope or an anchor
/// measured to the subunit can come out a subunit apart: the client then corrects by nothing
/// anyone sees.
const AGREE_UNITS: f32 = 0.25;
/// The rope's length and its anchor to the same 2.5 mm (an anchor is where the hook's
/// line from the predicted muzzle meets the arena, to the subunit).
const AGREE_ROPE_SUBUNITS: i32 = SUBUNITS_PER_UNIT / 4;

/// Whether both hooks are set at the same anchor (to 2.5 mm), or neither is.
fn same_hook(
    predicted: Option<shooter_sim::grapple::Hook>,
    host: Option<shooter_sim::grapple::Hook>,
) -> bool {
    match (predicted, host) {
        (Some(predicted), Some(host)) => (0..3)
            .all(|axis| (predicted.anchor[axis] - host.anchor[axis]).abs() <= AGREE_ROPE_SUBUNITS),
        (predicted, host) => predicted.is_none() == host.is_none(),
    }
}

impl Check {
    /// Whether the prediction was the host's to within what nobody sees: the hook (set or not,
    /// and where), motion and energy exactly, the robot and the rope to 2.5 mm.
    fn agrees(&self) -> bool {
        self.same_hook
            && self.same_rest
            && self.distance < AGREE_UNITS
            && self.rope_off <= AGREE_ROPE_SUBUNITS
    }
}

/// The online client's prediction, without its connection: snapshots in, inputs predicted.
struct Client {
    predictor: Session,
    predicting: bool,
    parts: shooter_sim::damage::PartDamage,
    pending: std::collections::VecDeque<Pending>,
    /// Every acknowledged prediction, checked.
    checks: Vec<Check>,
}

impl Client {
    fn new(arena: Arena) -> Self {
        Self {
            predictor: Session::new(arena).unwrap(),
            predicting: false,
            parts: shooter_sim::damage::PartDamage::default(),
            pending: std::collections::VecDeque::new(),
            checks: Vec::new(),
        }
    }

    fn place(&mut self, snapshot: &shooter_sim::Snapshot) {
        let me = snapshot.of_player(PERSON).unwrap();
        let predictor = &mut self.predictor;
        predictor
            .place(0, me.robot.as_ref().unwrap(), me.motion, me.held, me.hook)
            .unwrap();
        predictor.place_debris(&snapshot.debris).unwrap();
        predictor.fighters[0].energy = me.energy;
        predictor.fighters[0].fuel = me.fuel;
        predictor.fighters[0].repair = me.repair;
        predictor.fighters[0].arsenal.tubes = me.tubes;
        let tick = predictor.ticks();
        predictor.fighters[0].arsenal.resume(tick, &me.cooldowns);
    }

    fn predict(&mut self, intent: Intent) {
        self.predictor.fighters[0].parts = self.parts;
        self.predictor.tick(intent).unwrap();
    }

    fn state(&self, sequence: u32, intent: Intent) -> Pending {
        let local = self.predictor.local();
        Pending {
            sequence,
            intent,
            center: local.center(),
            motion: local.motion,
            hook: local.hook,
            energy: [local.energy, local.fuel],
            held: local.arsenal.held,
            tubes: local.arsenal.tubes,
        }
    }

    fn reconcile(&mut self, snapshot: &shooter_sim::Snapshot) {
        self.predictor.adopt_scenery(&snapshot.scenery).unwrap();
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
        let off = distance > TOLERANCE_UNITS || !same_rest || acked.hook != me.hook;
        self.checks.push(Check {
            ack: me.ack,
            distance,
            same_hook: same_hook(acked.hook, me.hook),
            rope_off: match (acked.hook, me.hook) {
                (Some(predicted), Some(host)) => (predicted.length - host.length).abs(),
                _ => 0,
            },
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

/// How a hosted grapple is played: its arena, the person's inputs tick by tick, the link's
/// delays and what happens to the robot on the host.
struct Link<'a> {
    arena: fn() -> Arena,
    /// The intent of each input, by sequence (1 the first).
    intents: &'a [Intent],
    /// The host tick each input (by sequence) reaches the host on: in order, never before it
    /// was sent.
    arrives: &'a dyn Fn(u32) -> u32,
    /// Ticks a snapshot takes to reach the client.
    down: u32,
    /// The host tick the robot's grapple module is shot off on, if it is.
    module_lost_at: Option<u32>,
}

/// What a hosted grapple did.
struct Played {
    client: Client,
    /// The host's hook and acknowledged sequence after each tick (1 the first).
    host: Vec<(u32, Option<shooter_sim::grapple::Hook>)>,
    /// The client's predicted hook after each of its inputs (by sequence).
    predicted: Vec<Option<shooter_sim::grapple::Hook>>,
    /// Every host tick's snapshot, encoded.
    snapshots: Vec<Vec<u8>>,
    /// The inputs as the host queued them: the tick before which each was queued.
    log: Vec<(u32, shooter_sim::Input)>,
}

fn host(arena: Arena) -> Session {
    let rules = shooter_sim::Rules {
        slots: 1,
        bots: false,
        local: false,
        ..shooter_sim::Rules::solo()
    };
    let mut host = Session::with_rules(arena, rules).unwrap();
    assert_eq!(host.join(PERSON).unwrap(), Some(0));
    host
}

fn lose_module(host: &mut Session) {
    let index = module_index(host);
    let fighter = &mut host.fighters[0];
    let build = fighter.build;
    fighter.parts.hurt(&build, index, 1_000);
    assert!(!fighter.parts.armed(&build, Weapon::Grapple));
}

/// Plays `link`: each tick the client takes the snapshots that have reached it, predicts and
/// sends its next input, and the host queues the inputs that have reached it and steps.
fn play(link: &Link) -> Played {
    let mut host = host((link.arena)());
    let mut client = Client::new((link.arena)());
    let ticks = u32::try_from(link.intents.len()).unwrap();
    let mut in_flight: Vec<(u32, shooter_sim::Input)> = Vec::new();
    let mut snapshots_in_flight: Vec<(u32, Vec<u8>)> = Vec::new();
    let mut played = Played {
        client: Client::new((link.arena)()),
        host: Vec::new(),
        predicted: Vec::new(),
        snapshots: Vec::new(),
        log: Vec::new(),
    };
    let mut last_arrival = 0;
    for tick in 1..=ticks {
        // The client: the snapshots that reached it, then this tick's input.
        while snapshots_in_flight
            .first()
            .is_some_and(|(arrives, _)| *arrives <= tick)
        {
            let (_, bytes) = snapshots_in_flight.remove(0);
            client.reconcile(&shooter_sim::wire::decode_snapshot(&bytes).unwrap());
        }
        let intent = link.intents[usize::try_from(tick - 1).unwrap()];
        if client.predicting {
            client.predict(intent);
            let state = client.state(tick, intent);
            client.pending.push_back(state);
        }
        played.predicted.push(client.predictor.local().hook);
        let arrives = (link.arrives)(tick);
        assert!(arrives >= tick && arrives >= last_arrival, "in order");
        last_arrival = arrives;
        in_flight.push((
            arrives,
            shooter_sim::Input {
                sequence: tick,
                intent,
                view_tick: 0,
            },
        ));
        // The host: the inputs that reached it, the shot that takes the module, the tick.
        while in_flight
            .first()
            .is_some_and(|(arrives, _)| *arrives <= tick)
        {
            let (_, input) = in_flight.remove(0);
            assert!(host.queue(PERSON, input));
            played.log.push((tick, input));
        }
        if link.module_lost_at == Some(tick) {
            lose_module(&mut host);
        }
        host.step().unwrap();
        let me = host.snapshot().of_player(PERSON).unwrap().clone();
        played.host.push((me.ack, me.hook));
        let bytes = shooter_sim::wire::encode_snapshot(&host.snapshot());
        played.snapshots.push(bytes.clone());
        snapshots_in_flight.push((tick + link.down, bytes));
    }
    played.client = client;
    played
}

/// The host replayed from its input log (and the module shot off on the same tick): every
/// tick's snapshot as it was.
fn replays_exactly(link: &Link, played: &Played) {
    let mut host = host((link.arena)());
    let mut log = played.log.iter().peekable();
    for (index, recorded) in played.snapshots.iter().enumerate() {
        let tick = u32::try_from(index).unwrap() + 1;
        while let Some((_, input)) = log.next_if(|(at, _)| *at == tick) {
            assert!(host.queue(PERSON, *input));
        }
        if link.module_lost_at == Some(tick) {
            lose_module(&mut host);
        }
        host.step().unwrap();
        assert!(
            shooter_sim::wire::encode_snapshot(&host.snapshot()) == *recorded,
            "the replay diverged on tick {tick}"
        );
    }
}

/// `legs` of (ticks, intent) as one input per tick.
fn inputs(legs: &[(u32, Intent)]) -> Vec<Intent> {
    legs.iter()
        .flat_map(|(ticks, intent)| (0..*ticks).map(|_| *intent))
        .collect()
}

/// The grapple scenario's legs (aim, hook and reel in, let go onto the ledge, stand), after a
/// second standing for the link to settle.
fn ledge_inputs() -> Vec<Intent> {
    let mut legs = vec![(60, scenarios::GRAPPLE.legs[0].intent)];
    legs.extend(
        scenarios::GRAPPLE
            .legs
            .iter()
            .map(|leg| (leg.ticks, leg.intent)),
    );
    inputs(&legs)
}

/// A second standing, four seconds swinging under the ceiling, a second after letting go.
fn swing_inputs() -> Vec<Intent> {
    let swing = Intent {
        pitch: 7_500,
        grapple: true,
        ..Intent::IDLE
    };
    let hang = Intent {
        grapple: false,
        ..swing
    };
    inputs(&[(60, hang), (240, swing), (60, hang)])
}

/// The first sequence whose input holds the grapple key.
fn first_hook_input(intents: &[Intent]) -> u32 {
    u32::try_from(intents.iter().position(|intent| intent.grapple).unwrap()).unwrap() + 1
}

/// The checks of every acknowledged prediction from input `first` on that did not agree with
/// the host ([`Check::agrees`]).
fn disagreed(played: &Played, first: u32) -> Vec<Check> {
    played
        .client
        .checks
        .iter()
        .filter(|check| check.ack >= first && !check.agrees())
        .copied()
        .collect()
}

/// The client's predicted hook after each input from `after` on was the host's when it applied
/// that input: set or not, and where.
fn predicted_the_hosts_hooks(played: &Played, after: u32) {
    for (ack, hook) in &played.host {
        if *ack > after {
            let predicted = played.predicted[usize::try_from(ack - 1).unwrap()];
            assert!(
                same_hook(predicted, *hook),
                "input {ack}: {predicted:?} {hook:?}"
            );
        }
    }
}

/// Fixed delays each way, from none to a quarter second: the delay before an input reaches the
/// host, and before a snapshot reaches the client.
const LATENCIES: [(u32, u32); 4] = [(0, 0), (2, 3), (6, 6), (9, 14)];

#[test]
fn a_hosted_hook_reel_and_release_are_predicted_at_every_latency() {
    let intents = ledge_inputs();
    let first = first_hook_input(&intents);
    for (up, down) in LATENCIES {
        let arrives = move |sequence: u32| sequence + up;
        let link = Link {
            arena: scenarios::GRAPPLE.arena,
            intents: &intents,
            arrives: &arrives,
            down,
            module_lost_at: None,
        };
        let played = play(&link);
        // The host hooked the pillar, reeled the robot up, and let go onto the ledge.
        let hooked = played
            .host
            .iter()
            .filter(|(_, hook)| hook.is_some())
            .count();
        assert!(hooked > 60, "{up}/{down}: hooked {hooked} ticks");
        assert!(played.host.last().unwrap().1.is_none());
        let host = shooter_sim::wire::decode_snapshot(played.snapshots.last().unwrap()).unwrap();
        let me = host.of_player(PERSON).unwrap();
        assert!(me.position[1] > 380.0, "{up}/{down}: {:?}", me.position);
        // The client predicted the hook, the reel and the release as the host had them.
        let off = disagreed(&played, first);
        assert!(off.is_empty(), "{up}/{down}: {off:?}");
        predicted_the_hosts_hooks(&played, first);
        replays_exactly(&link, &played);
    }
}

#[test]
fn a_hosted_swing_is_predicted_at_every_latency() {
    let intents = swing_inputs();
    let first = first_hook_input(&intents);
    for (up, down) in LATENCIES {
        let arrives = move |sequence: u32| sequence + up;
        let link = Link {
            arena: under_a_ceiling,
            intents: &intents,
            arrives: &arrives,
            down,
            module_lost_at: None,
        };
        let played = play(&link);
        let hooked = played
            .host
            .iter()
            .filter(|(_, hook)| hook.is_some())
            .count();
        assert_eq!(hooked, 240, "{up}/{down}: held while the key was");
        let off = disagreed(&played, first);
        assert!(off.is_empty(), "{up}/{down}: {off:?}");
        predicted_the_hosts_hooks(&played, first);
        replays_exactly(&link, &played);
    }
}

/// Inputs that arrive in a burst mid-swing (a late packet: the host holds the last input, then
/// works through the backlog): the client corrects, then predicts the host again.
#[test]
fn a_hosted_swing_recovers_from_late_inputs() {
    let intents = swing_inputs();
    let first = first_hook_input(&intents);
    // From input 120 to 136 everything arrives at once, on tick 140; the rest three ticks late.
    let arrives = |sequence: u32| match sequence {
        120..=136 => 140,
        _ => sequence + 3,
    };
    let link = Link {
        arena: under_a_ceiling,
        intents: &intents,
        arrives: &arrives,
        down: 4,
        module_lost_at: None,
    };
    let played = play(&link);
    let off = disagreed(&played, first);
    assert!(
        off.iter()
            .any(|check| check.corrected && !check.same_hook || check.distance >= AGREE_UNITS),
        "the held inputs moved the host off the prediction: {off:?}"
    );
    // Once the backlog is worked through, every acknowledged prediction agrees again.
    let settled = off.last().unwrap().ack;
    assert!(settled < 200, "still off at {off:?}");
    predicted_the_hosts_hooks(&played, settled);
    replays_exactly(&link, &played);
}

/// The module shot off on the host mid-swing: the host lets go that tick; the client, told late,
/// corrects once (the host's part damage and no hook), never hooks again and predicts the host
/// from then on; the host's ticks replay exactly.
#[test]
fn a_module_lost_mid_swing_on_a_host_is_reconciled_and_replays() {
    let intents = swing_inputs();
    let first = first_hook_input(&intents);
    let lost_at = 150;
    for (up, down) in LATENCIES {
        let arrives = move |sequence: u32| sequence + up;
        let link = Link {
            arena: under_a_ceiling,
            intents: &intents,
            arrives: &arrives,
            down,
            module_lost_at: Some(lost_at),
        };
        let played = play(&link);
        // The host hooked once its first grapple input arrived and let go on the tick the module
        // went.
        for (index, (_, hook)) in played.host.iter().enumerate() {
            let tick = u32::try_from(index).unwrap() + 1;
            assert_eq!(
                hook.is_some(),
                (first + up..lost_at).contains(&tick),
                "{up}/{down} tick {tick}"
            );
        }
        // The client's one disagreement is the input the host applied as the module went: the
        // snapshot that told it the module was gone.
        let (lost_ack, _) = played.host[usize::try_from(lost_at - 1).unwrap()];
        let off = disagreed(&played, first);
        assert_eq!(
            off.iter().map(|check| check.ack).collect::<Vec<_>>(),
            vec![lost_ack],
            "{up}/{down}: {off:?}"
        );
        assert!(off[0].corrected && !off[0].same_hook);
        // It predicted the hook until that snapshot reached it, and none from then on.
        let told = lost_at + down + 1;
        for (index, hook) in played.predicted.iter().enumerate() {
            let sequence = u32::try_from(index).unwrap() + 1;
            if sequence > first && sequence < told.min(lost_ack + 1) {
                assert!(hook.is_some(), "{up}/{down} input {sequence}");
            }
            if sequence >= told {
                assert!(hook.is_none(), "{up}/{down} input {sequence}");
            }
        }
        predicted_the_hosts_hooks(&played, told - 1);
        let local = played.client.predictor.local();
        assert!(!local.parts.armed(&local.build, Weapon::Grapple));
        replays_exactly(&link, &played);
    }
}

/// The grapple key tapped faster than the module's interval allows, on a host at every latency:
/// the host refuses the early hooks and pays for the rest, and the client predicts each refusal
/// and each hook (the module's own interval and the energy it spent) as the host has them.
#[test]
fn hosted_rapid_hooks_are_refused_and_predicted_at_every_latency() {
    let aim = Intent {
        pitch: 7_500,
        ..Intent::IDLE
    };
    let tap = Intent {
        grapple: true,
        ..aim
    };
    // A second standing, then 8 ticks held and 8 let go, 15 times over.
    let mut legs = vec![(60, aim)];
    legs.extend((0..15).flat_map(|_| [(8, tap), (8, aim)]));
    let intents = inputs(&legs);
    let first = first_hook_input(&intents);
    for (up, down) in LATENCIES {
        let arrives = move |sequence: u32| sequence + up;
        let link = Link {
            arena: under_a_ceiling,
            intents: &intents,
            arrives: &arrives,
            down,
            module_lost_at: None,
        };
        let played = play(&link);
        // Hooks set on the host: one every interval at most, not one a tap.
        let set = played
            .host
            .windows(2)
            .filter(|pair| pair[0].1.is_none() && pair[1].1.is_some())
            .count();
        assert!((4..15).contains(&set), "{up}/{down}: {set} hooks");
        let off = disagreed(&played, first);
        assert!(off.is_empty(), "{up}/{down}: {off:?}");
        predicted_the_hosts_hooks(&played, first);
        replays_exactly(&link, &played);
    }
}

/// Each staged hooked swing (the profile run's `swing` scene, this issue's captures) does what
/// it shows: standing there, the robot hooks the arena from its module's muzzle at
/// [`scenarios::SWING_HOOK_TICK`], high above, and the rope, tied at the module, reels it up off
/// the floor.
#[test]
fn the_staged_swings_hook_high_and_reel_the_robot_up() {
    use scenarios::{HOOKED_SWINGS, SWING_HOOK_TICK};
    for stage in HOOKED_SWINGS {
        let mut session = Session::on_map(stage.map, shooter_sim::Rules::solo()).unwrap();
        session.tick(Intent::IDLE).unwrap();
        session.fighters[0].intent.yaw = stage.yaw;
        session.stand(0, stage.feet).unwrap();
        let mut hooked_at = None;
        for tick in 1..=SWING_HOOK_TICK + 40 {
            session.tick(stage.intent(tick)).unwrap();
            let fighter = session.local();
            if let Some(hook) = fighter.hook {
                let tie = fighter.rope_tie().expect("the module on");
                let (start, anchor) =
                    *hooked_at.get_or_insert((fighter.feet(), hook.anchor_units()));
                assert_eq!(hook.anchor_units(), anchor, "{}", stage.map);
                assert!(distance(tie, anchor) <= units(hook.length) + 15.0);
                assert!(anchor[1] > start[1] + 500.0, "{}: {anchor:?}", stage.map);
            }
            if tick == SWING_HOOK_TICK {
                assert!(fighter.hook.is_some(), "{}: hooked at once", stage.map);
            }
        }
        let (start, _) = hooked_at.unwrap();
        let feet = session.local().feet();
        assert!(
            feet[1] > start[1] + 200.0,
            "{}: {start:?} -> {feet:?}",
            stage.map
        );
    }
}
