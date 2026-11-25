//! Several mounted copies of one weapon (shooter#125): two rail guns or four rocket launchers on
//! one robot are distinct weapons, each addressed by its part, with its own mod, refire, muzzle,
//! health and loss; a switch names a copy or a weapon; the wire, a predictor and the bots
//! follow.
// The platform's floats only check results here; the simulation uses libm (clippy.toml).
#![allow(clippy::disallowed_methods)]

use shooter_sim::aim;
use shooter_sim::physics::Vec3i;
use shooter_sim::robot::{
    ARMORY, BETA, OVERCHARGER, PartId, ROCKET_POD, STANDARD, TWIN_RAIL, parts::RAIL_GUN,
};
use shooter_sim::weapons::{self, Instance, Select, Weapon};
use shooter_sim::wire::{decode_input, decode_snapshot, encode_input, encode_snapshot};
use shooter_sim::{Arena, Assembly, Event, Input, Intent, Rules, Session, Spawn};

const A: u32 = 1;
const B: u32 = 2;

/// A (slot 0, built as `a`) faces B (slot 1, built as `b`) 12 m apart on an open floor.
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

/// One tick: A holds `a`, B holds `b`.
fn step(session: &mut Session, a: Intent, b: Intent) {
    for (id, slot, intent) in [(A, 0, a), (B, 1, b)] {
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

/// The shots `slot` fired in `events`: the copy and where each left from.
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

fn index_of(build: &Assembly, part: PartId) -> usize {
    build.parts().iter().position(|p| p.id == part).unwrap()
}

/// Where `slot`'s part `part` is now (units).
fn part_center(session: &Session, slot: usize, part: PartId) -> [f32; 3] {
    let fighter = &session.fighters[slot];
    let index = index_of(&fighter.build, part);
    fighter
        .hitboxes()
        .get(index)
        .expect("a connected part")
        .center
}

/// A's intent with its reticle on B's chassis (where B spawns while it is dead).
fn at_b(session: &Session) -> Intent {
    let b = &session.fighters[1];
    let target = b
        .hitboxes()
        .get(index_of(&b.build, PartId(0)))
        .map_or([0.0, 100.0, 600.0], |chassis| chassis.center);
    let (yaw, pitch) = aim::at(Some(&session.world), session.fighters[0].eye(), target);
    Intent {
        yaw,
        pitch,
        ..Intent::IDLE
    }
}

/// A takes up `select` at B's chassis, waits until its pool is full, then fires once: the copy
/// that fired, where from, where A's muzzle for that copy was the tick before, and the energy
/// the shot took (against a full pool, so the regeneration is the same every time).
fn fire_once(session: &mut Session, select: Select) -> (Instance, [f32; 3], [f32; 3], u32) {
    for _ in 0..240 {
        let take = Intent {
            select: Some(select),
            ..at_b(session)
        };
        step(session, take, Intent::IDLE);
        let a = &session.fighters[0];
        if a.energy == a.pool().capacity && a.arsenal.cooldown(session.ticks()) == 0 {
            break;
        }
    }
    let before = session.fighters[0].energy;
    let muzzle = session.fighters[0]
        .muzzle(session.fighters[0].arsenal.held)
        .unwrap()
        .at;
    let fire = Intent {
        fire: true,
        select: Some(select),
        ..at_b(session)
    };
    step(session, fire, Intent::IDLE);
    let shots = fired(&session.events, 0);
    assert_eq!(shots.len(), 1, "{select:?}: {shots:?}");
    let (instance, from) = shots[0];
    (instance, from, muzzle, before - session.fighters[0].energy)
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3)
        .map(|axis| (a[axis] - b[axis]).powi(2))
        .sum::<f32>()
        .sqrt()
}

/// Two rail guns, the second overcharged: each copy fires from its own muzzle at its own cost
/// and damage, and keeps its own refire.
#[test]
fn two_rail_guns_are_two_weapons_with_their_own_mods_muzzles_and_refire() {
    let mut candidate = TWIN_RAIL.candidate();
    candidate
        .parts
        .iter_mut()
        .find(|part| part.id == 9)
        .unwrap()
        .modification = Some(OVERCHARGER.0);
    let twin = BETA.validate(&candidate).unwrap();
    let mut session = duel(twin, STANDARD);
    let (first, second) = (
        Instance::new(7, Weapon::RailGun),
        Instance::new(9, Weapon::RailGun),
    );
    // It spawns holding the first copy (the lowest part ID).
    assert_eq!(session.fighters[0].arsenal.held, first);
    let a = &session.fighters[0];
    assert_eq!(a.tuning(first).energy(Weapon::RailGun), 30_000);
    assert_eq!(a.tuning(second).energy(Weapon::RailGun), 45_000);
    assert_eq!(a.tuning(first).damage_percent, 100);
    assert_eq!(a.tuning(second).damage_percent, 125);

    let (shot, from, muzzle, cost) = fire_once(&mut session, first.into());
    assert_eq!(shot, first);
    assert!(distance(from, muzzle) < 1.0, "{from:?} {muzzle:?}");
    let now = session.ticks();
    let arsenal = session.fighters[0].arsenal;
    assert!(arsenal.refire_left(first.part, now) > 0);
    assert_eq!(arsenal.refire_left(second.part, now), 0);

    let (shot, second_from, second_muzzle, second_cost) = fire_once(&mut session, second.into());
    assert_eq!(shot, second);
    assert!(distance(second_from, second_muzzle) < 1.0);
    // The two muzzles are a socket apart (20 cm), the overcharged copy's shot half again dearer.
    assert!(
        distance(from, second_from) > 15.0,
        "{from:?} {second_from:?}"
    );
    assert_eq!(second_cost - cost, 15_000);
}

/// B shoots A's first rail gun off: A's second rail gun keeps its health and fires on, the lost
/// copy is refused, and a switch by weapon finds the copy it still has.
#[test]
fn losing_one_rail_gun_leaves_the_other() {
    let mut session = duel(TWIN_RAIL, ARMORY);
    let first = PartId(7);
    let mut lost = false;
    for _ in 0..600 {
        let target = part_center(&session, 0, first);
        let (yaw, pitch) = aim::at(Some(&session.world), session.fighters[1].eye(), target);
        let b = Intent {
            yaw,
            pitch,
            fire: true,
            select: Some(Weapon::RailGun.into()),
            ..Intent::IDLE
        };
        step(&mut session, Intent::IDLE, b);
        if session
            .events
            .iter()
            .any(|event| matches!(event, Event::Detached { slot: 0, part, .. } if *part == first))
        {
            lost = true;
            break;
        }
    }
    assert!(lost, "B never shot the rail gun off");
    let a = &session.fighters[0];
    let second = index_of(&a.build, PartId(9));
    assert_eq!(a.parts.health()[second], RAIL_GUN.health);
    assert!(a.parts.attached(second));
    assert_eq!(
        a.instances()
            .into_iter()
            .filter(|instance| instance.weapon == Weapon::RailGun)
            .collect::<Vec<_>>(),
        [Instance::new(9, Weapon::RailGun)]
    );
    assert!(a.parts.armed(&a.build, Weapon::RailGun));
    assert!(!a.parts.weapon_lost(&a.build, Weapon::RailGun));
    assert_eq!(a.muzzle(Instance::new(7, Weapon::RailGun)), None);
    assert_eq!(a.arsenal.held.part, first);

    // The lost copy neither fires nor may be taken up again.
    for _ in 0..90 {
        let a = Intent {
            fire: true,
            select: Some(Select::Part(first)),
            ..at_b(&session)
        };
        step(&mut session, a, Intent::IDLE);
        assert!(fired(&session.events, 0).is_empty());
    }
    // The rail gun by weapon: the copy it still has, from its own muzzle.
    let (shot, from, muzzle, _) = fire_once(&mut session, Weapon::RailGun.into());
    assert_eq!(shot, Instance::new(9, Weapon::RailGun));
    assert!(distance(from, muzzle) < 1.0);
}

/// The rocket pod's four launchers are four places in the cycle, each fired from its own muzzle,
/// and never two in one tick (their salvo limit, shooter#127).
#[test]
fn the_rocket_pod_cycles_its_four_launchers_one_at_a_time() {
    let mut session = duel(ROCKET_POD, STANDARD);
    let pods: Vec<Instance> = [7, 8, 9, 10]
        .map(|part| Instance::new(part, Weapon::RocketLauncher))
        .into();
    assert_eq!(session.fighters[0].instances(), pods);
    assert_eq!(weapons::holdable(&session.fighters[0].instances()), pods);
    assert_eq!(
        weapons::cycled(pods[3], 1, &session.fighters[0].instances()),
        Some(pods[0])
    );
    let mut froms = Vec::new();
    for pod in [pods[1], pods[3], pods[0], pods[2]] {
        let (shot, from, muzzle, _) = fire_once(&mut session, pod.into());
        assert_eq!(shot, pod);
        assert!(distance(from, muzzle) < 1.0);
        froms.push(from);
    }
    for (a, first) in froms.iter().enumerate() {
        for second in &froms[a + 1..] {
            assert!(distance(*first, *second) > 15.0, "{froms:?}");
        }
    }
    // Held down with a switch to another launcher every tick: one rocket at a time, no faster
    // than the family's salvo spacing, though faster than one launcher alone.
    let mut volleys = 0;
    for tick in 0..240 {
        let a = Intent {
            fire: true,
            select: Some(pods[tick % 4].into()),
            ..at_b(&session)
        };
        step(&mut session, a, Intent::IDLE);
        let shots = fired(&session.events, 0).len();
        assert!(shots <= 1);
        volleys += shots;
    }
    let spacing = Weapon::RocketLauncher.salvo().unwrap().spacing as usize;
    assert!(volleys <= 240 / spacing + 1, "{volleys} rockets");
    assert!(volleys > 240 / 48 + 1, "{volleys} rockets");
}

/// The wire carries a switch to a copy or to a weapon, the copy held and the copy that fired;
/// a predictor resuming from a snapshot fires the same copies on the same ticks as the host.
#[test]
fn copies_travel_on_the_wire_and_a_predictor_follows() {
    for select in [
        None,
        Some(Select::Part(PartId(9))),
        Some(Select::Weapon(Weapon::RocketLauncher)),
    ] {
        let intent = Intent {
            select,
            fire: true,
            ..Intent::IDLE
        };
        let command = decode_input(&encode_input(intent, 5, None, Some(&ROCKET_POD))).unwrap();
        assert_eq!(command.intent, intent);
        assert_eq!(command.build, Some(ROCKET_POD));
    }
    // An unknown switch tag is malformed.
    let mut bytes = encode_input(Intent::IDLE, 5, None, None);
    // The version, the moves, the yaw, the pitch and the flags come first.
    bytes[1 + 2 + 2 + 2 + 1] = 7;
    assert!(decode_input(&bytes).is_err());

    let mut host = duel(TWIN_RAIL, STANDARD);
    let second = Instance::new(9, Weapon::RailGun);
    let (shot, ..) = fire_once(&mut host, second.into());
    assert_eq!(shot, second);
    let snapshot = host.snapshot();
    assert!(
        snapshot
            .events
            .iter()
            .any(|event| matches!(event, Event::Fired { part, .. } if *part == second.part))
    );
    assert_eq!(
        decode_snapshot(&encode_snapshot(&snapshot)).unwrap(),
        snapshot
    );
    // Once the shot's refire is over, a predictor resumes from the host's word.
    while host.fighters[0].arsenal.cooldown(host.ticks()) > 0 {
        let aim = at_b(&host);
        step(&mut host, aim, Intent::IDLE);
    }
    let snapshot = decode_snapshot(&encode_snapshot(&host.snapshot())).unwrap();
    let me = snapshot.slot(0).unwrap().clone();
    assert_eq!(me.held, second);
    let mut predictor = Session::new(host.arena.clone()).unwrap();
    predictor
        .place(0, me.robot.as_ref().unwrap(), me.motion, me.held, me.hook)
        .unwrap();
    predictor.fighters[0].build = me.build;
    predictor.fighters[0].parts = me.parts;
    predictor.fighters[0].energy = me.energy;
    let (mut on_host, mut predicted) = (Vec::new(), Vec::new());
    for tick in 0..200 {
        let select = match tick {
            0 => Some(Select::Part(PartId(7))),
            90 => Some(Select::Weapon(Weapon::RailGun)),
            120 => Some(Select::Part(PartId(9))),
            _ => None,
        };
        let intent = Intent {
            fire: true,
            select,
            ..at_b(&host)
        };
        step(&mut host, intent, Intent::IDLE);
        predictor.tick(intent).unwrap();
        on_host.extend(
            fired(&host.events, 0)
                .iter()
                .map(|(fired, _)| (tick, *fired)),
        );
        predicted.extend(
            fired(&predictor.events, 0)
                .iter()
                .map(|(fired, _)| (tick, *fired)),
        );
    }
    assert_eq!(predicted, on_host);
    let copies: Vec<PartId> = on_host.iter().map(|(_, fired)| fired.part).collect();
    assert!(
        copies.contains(&PartId(7)) && copies.contains(&PartId(9)),
        "{copies:?}"
    );
}

/// Bots built with copies fire them (the copy they hold, else the first they still have), and a
/// match of them replays exactly.
#[test]
fn bots_with_copies_fire_them_and_replay_exactly() {
    let play = || {
        let mut session = Session::with_rules(Arena::greybox(), Rules::deathmatch(11)).unwrap();
        let slots = session.fighters.len();
        for slot in 0..slots {
            let build = if slot % 2 == 0 { TWIN_RAIL } else { ROCKET_POD };
            session.fighters[slot].configured = build;
            session.rebuild(u8::try_from(slot).unwrap(), build).unwrap();
        }
        let mut shots: Vec<(u8, Instance)> = Vec::new();
        let mut bytes = Vec::new();
        for _ in 0..30 * 60 {
            session.step().unwrap();
            for slot in 0..slots {
                let slot = u8::try_from(slot).unwrap();
                shots.extend(
                    fired(&session.events, slot)
                        .iter()
                        .map(|(fired, _)| (slot, *fired)),
                );
            }
            bytes.extend(encode_snapshot(&session.snapshot()));
        }
        (shots, bytes)
    };
    let (shots, bytes) = play();
    let guns = shots
        .iter()
        .filter(|(_, fired)| matches!(fired.weapon, Weapon::RailGun | Weapon::RocketLauncher))
        .count();
    assert!(guns > 20, "{guns} shots");
    for (slot, fired) in &shots {
        let build = if slot % 2 == 0 { TWIN_RAIL } else { ROCKET_POD };
        assert_eq!(BETA.instance(&build, fired.part), Some(*fired));
    }
    let again = play();
    assert_eq!(again.0, shots);
    assert!(again.1 == bytes, "the match diverged");
}
