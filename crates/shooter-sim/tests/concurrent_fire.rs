//! Independently bound weapons fire concurrently (shooter#127): direct weapon buttons and
//! overlapping weapon groups name exact mounted copies, each copy fires on its own refire and
//! energy, a refused copy refuses only itself, and the rocket launchers' shared salvo limit
//! staggers a pod; the wire, a predictor, the bots and replays follow.
// The platform's floats only check results here; the simulation uses libm (clippy.toml).
#![allow(clippy::disallowed_methods)]

use shooter_sim::aim;
use shooter_sim::bindings::{
    ALL_GUNS_BUTTON, Binding, BindingError, BindingState, Bindings, Mode, Output, Pressed, Source,
    Target, key,
};
use shooter_sim::mechanics::Layout;
use shooter_sim::physics::Vec3i;
use shooter_sim::robot::{ARMORY, BETA, PartId, ROCKET_POD, STANDARD, TWIN_RAIL};
use shooter_sim::weapons::{Instance, Triggers, Weapon};
use shooter_sim::wire::{
    decode_bindings, decode_input, decode_snapshot, encode_bindings, encode_input, encode_snapshot,
};
use shooter_sim::{Arena, Assembly, Event, Input, Intent, Rules, Session, Spawn};

const A: u32 = 1;
const B: u32 = 2;

/// A (slot 0, built as `a`) faces B (slot 1, built as `b`) 12 m apart on an open floor, both
/// pools full.
fn duel(a: Assembly, b: Assembly) -> Session {
    let arena = Arena {
        half_size: 2_000,
        pieces: Vec::new(),
        pads: Vec::new(),
        posts: Vec::new(),
        spawn: Vec3i::new(0, 0, -600),
        spawn_yaw: 0,
        spawns: vec![Spawn {
            feet: Vec3i::new(0, 0, 600),
            yaw: 32_768,
        }],
        module_pads: Vec::new(),
        waypoints: Vec::new(),
        placements: Vec::new(),
        repair_anchors: Vec::new(),
    };
    let rules = Rules {
        slots: 2,
        bots: false,
        local: false,
        seed: 0,
        frag_limit: 0,
        time_limit_ticks: 0,
        intermission_ticks: 60,
        respawn_ticks: 30,
    };
    let mut session = Session::with_rules(arena, rules).unwrap();
    assert_eq!(session.join(A).unwrap(), Some(0));
    assert_eq!(session.join(B).unwrap(), Some(1));
    for (slot, build) in [(0, a), (1, b)] {
        session.fighters[slot].configured = build;
        session.rebuild(u8::try_from(slot).unwrap(), build).unwrap();
        let fighter = &mut session.fighters[slot];
        fighter.energy = fighter.pool().capacity;
    }
    session
}

/// One tick: A holds `a`, B stands idle.
fn step(session: &mut Session, a: Intent) {
    for (id, slot, intent) in [(A, 0, a), (B, 1, Intent::IDLE)] {
        let sequence = session.fighters[slot].ack + 1;
        let view_tick = session.ticks();
        session.queue(
            id,
            Input {
                sequence,
                intent,
                view_tick,
            },
        );
    }
    session.step().unwrap();
}

/// The copies `slot` fired in `events`, and where from.
fn fired(events: &[Event], slot: u8) -> Vec<(Instance, [f32; 3])> {
    events
        .iter()
        .filter_map(|event| match *event {
            Event::Fired {
                slot: shooter,
                weapon,
                part,
                from,
                ..
            } if shooter == slot => Some((Instance { part, weapon }, from)),
            _ => None,
        })
        .collect()
}

fn copies(events: &[Event], slot: u8) -> Vec<Instance> {
    fired(events, slot)
        .into_iter()
        .map(|(copy, _)| copy)
        .collect()
}

/// A's intent with its reticle on B's chassis, asking `triggers` to fire.
fn at_b(session: &Session, triggers: Triggers) -> Intent {
    let b = &session.fighters[1];
    let index = b
        .build
        .parts()
        .iter()
        .position(|part| part.id == PartId(0))
        .unwrap();
    let target = b
        .hitboxes()
        .get(index)
        .map_or([0.0, 100.0, 600.0], |chassis| chassis.center);
    let (yaw, pitch) = aim::at(Some(&session.world), session.fighters[0].eye(), target);
    Intent {
        yaw,
        pitch,
        triggers,
        ..Intent::IDLE
    }
}

fn parts(ids: &[u8]) -> Triggers {
    ids.iter().copied().map(PartId).collect()
}

/// Lets A settle a quarter second with its pool full.
fn settle(session: &mut Session) {
    for _ in 0..15 {
        let aim = at_b(session, Triggers::NONE);
        step(session, aim);
    }
    let a = &mut session.fighters[0];
    a.energy = a.pool().capacity;
}

fn fire_binding(source: Source, mode: Mode, guns: &[u8]) -> Binding {
    Binding {
        source,
        mode,
        output: Output::Fire,
        invert: false,
        strength: 100,
        targets: guns
            .iter()
            .map(|&part| Target {
                part: PartId(part),
                invert: false,
            })
            .collect(),
    }
}

fn held(codes: &[&str], buttons: u8) -> Pressed {
    let mut pressed = codes
        .iter()
        .fold(Pressed::default(), |pressed, code| pressed.with_key(code));
    pressed.buttons = buttons;
    pressed
}

/// Direct weapon buttons and overlapping groups resolve to exact copies, each once; the layout
/// is checked against the build's guns and travels on the wire.
#[test]
fn direct_buttons_and_overlapping_groups_name_exact_copies() {
    let layout = Layout::of(&TWIN_RAIL);
    assert_eq!(layout.guns, [PartId(7), PartId(9)]);
    let z = Source::Key(key("KeyZ").unwrap());
    let x = Source::Key(key("KeyX").unwrap());
    let c = Source::Key(key("KeyC").unwrap());
    let bindings = Bindings {
        bindings: vec![
            // A direct button for each rail gun...
            fire_binding(z, Mode::Hold, &[7]),
            fire_binding(x, Mode::Hold, &[9]),
            // ...a group of both, and an overlapping group of the second toggled.
            fire_binding(c, Mode::Hold, &[7, 9]),
            fire_binding(Source::Button(3), Mode::Toggle, &[9]),
        ],
        ..Bindings::default()
    };
    assert_eq!(bindings.check(&layout), Ok(()));
    let mut state = BindingState::default();
    let mut act = |pressed| bindings.act(&layout, &mut state, pressed).triggers;
    assert_eq!(act(held(&[], 0)), Triggers::NONE);
    assert_eq!(act(held(&["KeyZ"], 0)), parts(&[7]));
    assert_eq!(act(held(&["KeyX"], 0)), parts(&[9]));
    // Two buttons and a group at once name each copy once.
    assert_eq!(act(held(&["KeyZ", "KeyX", "KeyC"], 0)), parts(&[7, 9]));
    // The toggle stays on after its button is let go.
    assert_eq!(act(held(&[], 1 << 3)), parts(&[9]));
    assert_eq!(act(held(&["KeyZ"], 0)), parts(&[7, 9]));
    assert_eq!(act(held(&[], 1 << 3)), Triggers::NONE);
    // Fire bindings move no actuator.
    assert_eq!(
        bindings
            .resolve(&layout, &mut BindingState::default(), held(&["KeyC"], 0))
            .commands(),
        []
    );

    // Only guns: not the blade (8), not a leg, not from a mouse axis.
    let refused = |binding: Binding| {
        Bindings {
            bindings: vec![binding],
            ..Bindings::default()
        }
        .check(&layout)
        .unwrap_err()
    };
    assert_eq!(
        refused(fire_binding(z, Mode::Hold, &[7, 8])),
        BindingError::NotAGun(0, PartId(8))
    );
    assert_eq!(
        refused(fire_binding(Source::MouseX, Mode::Relative, &[7])),
        BindingError::Mode(0)
    );
    // A gun the robot has lost is pruned away, and a binding left with none with it.
    let index = TWIN_RAIL
        .parts()
        .iter()
        .position(|part| part.id == PartId(7))
        .unwrap();
    let pruned = bindings.pruned(&Layout::connected(&TWIN_RAIL, 1 << index));
    assert_eq!(pruned.bindings.len(), 3, "{pruned:?}");
    assert!(pruned.bindings.iter().all(|binding| {
        binding
            .targets
            .iter()
            .all(|target| target.part == PartId(9))
    }));
    assert_eq!(pruned.check(&Layout::of(&TWIN_RAIL)), Ok(()));

    // The preset fires every gun on the middle button; the left button stays the primary.
    let preset = Bindings::preset(&Layout::of(&ROCKET_POD));
    let all = preset
        .bindings
        .iter()
        .find(|binding| binding.output == Output::Fire)
        .unwrap();
    assert_eq!(all.source, Source::Button(ALL_GUNS_BUTTON));
    let mut state = BindingState::default();
    assert_eq!(
        preset
            .act(
                &Layout::of(&ROCKET_POD),
                &mut state,
                held(&[], 1 << ALL_GUNS_BUTTON)
            )
            .triggers,
        parts(&[7, 8, 9, 10])
    );
    assert_eq!(
        decode_bindings(&encode_bindings(&bindings)).unwrap(),
        bindings
    );
}

/// The two rail guns fire in one tick, each from its own muzzle, each paying its own energy and
/// keeping its own refire.
#[test]
fn two_rail_guns_fire_together() {
    let mut session = duel(TWIN_RAIL, STANDARD);
    settle(&mut session);
    let before = session.fighters[0].energy;
    let both = at_b(&session, parts(&[9, 7]));
    step(&mut session, both);
    let shots = fired(&session.events, 0);
    let shot: Vec<Instance> = shots.iter().map(|(copy, _)| *copy).collect();
    assert_eq!(
        shot,
        [
            Instance::new(7, Weapon::RailGun),
            Instance::new(9, Weapon::RailGun)
        ]
    );
    let gap = (0..3)
        .map(|axis| (shots[0].1[axis] - shots[1].1[axis]).powi(2))
        .sum::<f32>()
        .sqrt();
    assert!(gap > 15.0, "{shots:?}");
    // 30 each, less a tick's regeneration.
    let spent = before - session.fighters[0].energy;
    assert!((59_000..=60_000).contains(&spent), "{spent}");
    // Held on: neither fires again until its refire is over, then both do.
    let mut again = Vec::new();
    for tick in 1..=80 {
        let both = at_b(&session, parts(&[7, 9]));
        step(&mut session, both);
        if !fired(&session.events, 0).is_empty() {
            again.push((tick, copies(&session.events, 0).len()));
        }
    }
    assert_eq!(again, [(72, 2)]);
}

/// Several families fire together with no switch: the rifle, the shotgun and the rail gun of the
/// armory beside the laser it holds on the primary trigger.
#[test]
fn mixed_families_fire_without_switching() {
    let mut session = duel(ARMORY, STANDARD);
    settle(&mut session);
    let held = session.fighters[0].arsenal.held;
    let intent = Intent {
        fire: true,
        ..at_b(&session, parts(&[7, 10, 13]))
    };
    step(&mut session, intent);
    let mut shot = copies(&session.events, 0);
    shot.sort();
    let mut expected = vec![
        Instance::new(7, Weapon::EnergyRifle),
        Instance::new(10, Weapon::Shotgun),
        Instance::new(13, Weapon::RailGun),
        held,
    ];
    expected.sort();
    expected.dedup();
    assert_eq!(shot, expected);
    assert_eq!(session.fighters[0].arsenal.held, held);
    // The blade (8) and the grapple module (9) are no guns: asked by part, they do nothing.
    for _ in 0..60 {
        let odd = at_b(&session, parts(&[8, 9]));
        step(&mut session, odd);
        assert!(fired(&session.events, 0).is_empty());
        assert!(session.fighters[0].hook.is_none());
    }
}

/// A copy asked twice in one tick (the primary trigger on the held copy, a direct button and
/// a group) fires once.
#[test]
fn a_copy_asked_twice_fires_once() {
    let mut session = duel(TWIN_RAIL, STANDARD);
    settle(&mut session);
    assert_eq!(
        session.fighters[0].arsenal.held,
        Instance::new(7, Weapon::RailGun)
    );
    let mut triggers = parts(&[7]);
    assert!(!triggers.press(PartId(7)));
    let intent = Intent {
        fire: true,
        ..at_b(&session, triggers)
    };
    step(&mut session, intent);
    assert_eq!(
        copies(&session.events, 0),
        [Instance::new(7, Weapon::RailGun)]
    );
}

/// A lost copy and a copy the pool cannot pay refuse only themselves.
#[test]
fn a_lost_or_unpaid_copy_refuses_only_itself() {
    // The pool holds enough for one rail shot (30) but not two: the first by part fires, the
    // second is refused and keeps no cooldown.
    let mut session = duel(TWIN_RAIL, STANDARD);
    settle(&mut session);
    session.fighters[0].energy = 40_000;
    let both = at_b(&session, parts(&[7, 9]));
    step(&mut session, both);
    assert_eq!(
        copies(&session.events, 0),
        [Instance::new(7, Weapon::RailGun)]
    );
    let now = session.ticks();
    let arsenal = session.fighters[0].arsenal;
    assert_eq!(
        arsenal.cooldown_of(Instance::new(9, Weapon::RailGun), now),
        0
    );
    session.fighters[0].energy = session.fighters[0].pool().capacity;
    let second = at_b(&session, parts(&[7, 9]));
    step(&mut session, second);
    assert_eq!(
        copies(&session.events, 0),
        [Instance::new(9, Weapon::RailGun)]
    );

    // The first rail gun shot off: asking both fires the second alone.
    let mut session = duel(TWIN_RAIL, STANDARD);
    settle(&mut session);
    let a = &mut session.fighters[0];
    let index = a
        .build
        .parts()
        .iter()
        .position(|part| part.id == PartId(7))
        .unwrap();
    let build = a.build;
    a.parts.hurt(&build, index, 10_000).unwrap();
    assert!(!a.parts.attached(index));
    let both = at_b(&session, parts(&[7, 9]));
    step(&mut session, both);
    assert_eq!(
        copies(&session.events, 0),
        [Instance::new(9, Weapon::RailGun)]
    );
}

/// The rocket pod's four launchers asked together: never two rockets in one tick, staggered by
/// the salvo spacing, each launcher then on its own refire; held on, let go and held again;
/// and with too little energy, rockets wait for the pool.
#[test]
fn four_rockets_never_launch_in_one_tick() {
    let spacing = Weapon::RocketLauncher.salvo().unwrap().spacing;
    let mut session = duel(ROCKET_POD, STANDARD);
    settle(&mut session);
    let pod = parts(&[7, 8, 9, 10]);
    let mut launches: Vec<(u32, u8)> = Vec::new();
    for tick in 0..240 {
        // Let go for a second in the middle, then held again.
        let triggers = if (100..160).contains(&tick) {
            Triggers::NONE
        } else {
            pod
        };
        let intent = at_b(&session, triggers);
        step(&mut session, intent);
        let shot = copies(&session.events, 0);
        assert!(shot.len() <= 1, "{tick}: {shot:?}");
        launches.extend(shot.iter().map(|copy| (tick, copy.part.0)));
        // Keep the pool full: the limit, not the energy, staggers them here.
        let a = &mut session.fighters[0];
        a.energy = a.pool().capacity;
    }
    assert_eq!(
        launches[..4],
        [(0, 7), (spacing, 8), (2 * spacing, 9), (3 * spacing, 10)]
    );
    // Never two launches closer than the spacing.
    for pair in launches.windows(2) {
        assert!(pair[1].0 - pair[0].0 >= spacing, "{launches:?}");
    }
    // Every launcher keeps its own refire (48 ticks).
    for part in 7..=10 {
        let ticks: Vec<u32> = launches
            .iter()
            .filter(|(_, launcher)| *launcher == part)
            .map(|(tick, _)| *tick)
            .collect();
        assert!(ticks.windows(2).all(|pair| pair[1] - pair[0] >= 48));
    }
    assert!(!launches.iter().any(|(tick, _)| (100..160).contains(tick)));
    assert!(launches.iter().any(|(tick, _)| *tick >= 160));

    // A pool that pays one rocket (20) at a time: they wait for it.
    let mut session = duel(ROCKET_POD, STANDARD);
    settle(&mut session);
    session.fighters[0].energy = 25_000;
    let intent = at_b(&session, pod);
    step(&mut session, intent);
    assert_eq!(copies(&session.events, 0).len(), 1);
    for _ in 0..spacing {
        let intent = at_b(&session, pod);
        step(&mut session, intent);
        assert!(fired(&session.events, 0).is_empty());
    }
}

/// The requests travel on the wire (tampered ones refused whole), and a predictor resuming from
/// the host's word fires the same copies on the same ticks as the host.
#[test]
fn requests_travel_on_the_wire_and_a_predictor_follows() {
    let intent = Intent {
        fire: true,
        triggers: parts(&[10, 7, 9]),
        ..Intent::IDLE
    };
    let bytes = encode_input(intent, 5, None, None);
    assert_eq!(decode_input(&bytes).unwrap().intent, intent);
    // The requests are the count and the parts, before the view tick, the name and the build
    // flag.
    let at = bytes.len() - (4 + 1 + 1) - (1 + 3);
    let tamper = |offset: usize, value: u8| {
        let mut bytes = bytes.clone();
        bytes[at + offset] = value;
        decode_input(&bytes)
    };
    assert_eq!(bytes[at..at + 4], [3, 7, 9, 10]);
    assert!(tamper(2, 7).is_err(), "a part twice");
    assert!(tamper(1, 10).is_err(), "out of order");
    assert!(tamper(0, 33).is_err(), "too many");

    for build in [TWIN_RAIL, ROCKET_POD] {
        let mut host = duel(build, STANDARD);
        settle(&mut host);
        let snapshot = decode_snapshot(&encode_snapshot(&host.snapshot())).unwrap();
        let me = snapshot.slot(0).unwrap().clone();
        let mut predictor = Session::new(host.arena.clone()).unwrap();
        predictor
            .place(0, me.robot.as_ref().unwrap(), me.motion, me.held, me.hook)
            .unwrap();
        predictor.fighters[0].build = me.build;
        predictor.fighters[0].parts = me.parts;
        predictor.fighters[0].energy = me.energy;
        let (mut on_host, mut predicted) = (Vec::new(), Vec::new());
        for tick in 0..200 {
            let triggers = if tick % 90 < 60 {
                parts(&[7, 8, 9, 10])
            } else {
                Triggers::NONE
            };
            let intent = at_b(&host, triggers);
            step(&mut host, intent);
            predictor.tick(intent).unwrap();
            on_host.extend(copies(&host.events, 0).into_iter().map(|copy| (tick, copy)));
            predicted.extend(
                copies(&predictor.events, 0)
                    .into_iter()
                    .map(|copy| (tick, copy)),
            );
            for part in [7, 8, 9, 10] {
                let Some(copy) = BETA.instance(&build, PartId(part)) else {
                    continue;
                };
                assert_eq!(
                    predictor.fighters[0]
                        .arsenal
                        .cooldown_of(copy, predictor.ticks()),
                    host.fighters[0].arsenal.cooldown_of(copy, host.ticks()),
                    "{tick}"
                );
            }
        }
        assert_eq!(predicted, on_host);
        assert!(on_host.len() > 4, "{on_host:?}");
    }
}

/// Bots fire every copy of the gun they pick through the same requests: the twin rails together,
/// the pod staggered; and a match of them replays exactly.
#[test]
fn bots_fire_their_copies_together_and_replay_exactly() {
    let play = || {
        let mut session = Session::with_rules(Arena::greybox(), Rules::deathmatch(17)).unwrap();
        let slots = session.fighters.len();
        for slot in 0..slots {
            let build = if slot % 2 == 0 { TWIN_RAIL } else { ROCKET_POD };
            session.fighters[slot].configured = build;
            session.rebuild(u8::try_from(slot).unwrap(), build).unwrap();
        }
        let (mut together, mut rockets) = (0, 0);
        let mut bytes = Vec::new();
        for _ in 0..30 * 60 {
            session.step().unwrap();
            for slot in 0..slots {
                let shot = copies(&session.events, u8::try_from(slot).unwrap());
                let count = |weapon| shot.iter().filter(|copy| copy.weapon == weapon).count();
                assert!(count(Weapon::RocketLauncher) <= 1, "{shot:?}");
                rockets += count(Weapon::RocketLauncher);
                if count(Weapon::RailGun) == 2 {
                    together += 1;
                }
            }
            bytes.extend(encode_snapshot(&session.snapshot()));
        }
        (together, rockets, bytes)
    };
    let (together, rockets, bytes) = play();
    assert!(together > 0, "no bot fired both rail guns at once");
    assert!(rockets > 4, "{rockets} rockets");
    let again = play();
    assert_eq!((again.0, again.1), (together, rockets));
    assert!(again.2 == bytes, "the match diverged");
}
