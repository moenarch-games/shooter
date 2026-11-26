//! Deliberate self-destruct on the host (`shooter_sim::destruct`, Atlas D2.i123.10, shooter#131):
//! a press arms a warning countdown and another cancels it, death and the round's end cancel it,
//! a stranded, empty or airborne robot can still arm it, the robot dies by its own kill (a
//! suicide) and its blast is the volatile contents it still carries (full, empty, a tank lost, a
//! battery destroyed beforehand), resolved through the part damage with range and cover,
//! simultaneous blasts, scoring and respawn, the wire, a predicting client and a replay.
// The platform's floats only check results here; the simulation uses libm (clippy.toml).
#![allow(clippy::disallowed_methods)]

use shooter_sim::destruct::{COUNTDOWN_TICKS, Charge};
use shooter_sim::energy::Pool;
use shooter_sim::physics::Vec3i;
use shooter_sim::player::KILL_HEIGHT_UNITS;
use shooter_sim::robot::{ARMORY, HEAVY, PartId, STANDARD};
use shooter_sim::wire::{decode_input, decode_snapshot, encode_input, encode_snapshot};
use shooter_sim::{Arena, Assembly, Event, Input, Intent, Phase, Piece, Rules, Session, Spawn};

const PEOPLE: [u32; 3] = [1, 2, 3];
/// HEAVY's battery and fuel tank.
const BATTERY: PartId = PartId(11);
const TANK: PartId = PartId(20);

/// An open floor with a tall wall standing at x = 10 m (z within ±3 m).
fn arena() -> Arena {
    Arena {
        half_size: 3_000,
        pieces: vec![Piece::Block {
            center: Vec3i::new(1_000, 200, 0),
            half: Vec3i::new(10, 200, 300),
        }],
        pads: Vec::new(),
        posts: Vec::new(),
        spawn: Vec3i::new(0, 0, -600),
        spawn_yaw: 0,
        spawns: vec![
            Spawn {
                feet: Vec3i::new(0, 0, 600),
                yaw: 32_768,
            },
            Spawn {
                feet: Vec3i::new(-1_500, 0, 0),
                yaw: 0,
            },
        ],
        module_pads: Vec::new(),
        waypoints: Vec::new(),
        placements: Vec::new(),
        repair_anchors: Vec::new(),
    }
}

fn rules(slots: u8) -> Rules {
    Rules {
        slots,
        bots: false,
        local: false,
        seed: 0,
        frag_limit: 0,
        time_limit_ticks: 0,
        intermission_ticks: 60,
        respawn_ticks: 30,
    }
}

/// `slots` people (A, B, C), A the [`ARMORY`] (many parts to strike), the others `build`.
fn match_of(slots: u8, build: Assembly) -> Session {
    let mut session = Session::with_rules(arena(), rules(slots)).unwrap();
    for (slot, id) in PEOPLE.iter().take(usize::from(slots)).enumerate() {
        assert_eq!(
            session.join(*id).unwrap(),
            Some(u8::try_from(slot).unwrap())
        );
        rebuild(&mut session, slot, if slot == 0 { ARMORY } else { build });
    }
    session
}

/// `slot` is built as `build`, whole, full of electricity and fuel, this life.
fn rebuild(session: &mut Session, slot: usize, build: Assembly) {
    session.fighters[slot].configured = build;
    session.rebuild(u8::try_from(slot).unwrap(), build).unwrap();
    let fighter = &mut session.fighters[slot];
    fighter.energy = fighter.pool().capacity;
    fighter.fuel = fighter.pool().fuel;
}

/// One tick, each person holding its intent (`IDLE` for those not given).
fn step(session: &mut Session, intents: &[Intent]) {
    for slot in 0..session.fighters.len() {
        let intent = intents.get(slot).copied().unwrap_or(Intent::IDLE);
        let sequence = session.fighters[slot].ack + 1;
        let view_tick = session.ticks();
        session.queue(
            PEOPLE[slot],
            Input {
                sequence,
                intent,
                view_tick,
            },
        );
    }
    session.step().unwrap();
}

const PRESS: Intent = Intent {
    destruct: true,
    ..Intent::IDLE
};

/// Every person idle for `ticks`.
fn idle(session: &mut Session, ticks: u32) {
    for _ in 0..ticks {
        step(session, &[]);
    }
}

/// `slot` presses the self-destruct this tick (the others idle).
fn press(session: &mut Session, slot: usize) {
    let mut intents = vec![Intent::IDLE; session.fighters.len()];
    intents[slot] = PRESS;
    step(session, &intents);
}

/// The damage `victim` took from `attacker` in the last tick.
fn taken(session: &Session, victim: u8, attacker: u8) -> i32 {
    session
        .events
        .iter()
        .filter_map(|event| match *event {
            Event::Hit {
                victim: hurt,
                attacker: Some(by),
                damage,
                ..
            } if hurt == victim && by == attacker => Some(damage),
            _ => None,
        })
        .sum()
}

/// The last tick's detonations: (slot, damage, radius).
fn detonations(session: &Session) -> Vec<(u8, i32, i32)> {
    session
        .events
        .iter()
        .filter_map(|event| match *event {
            Event::Detonated {
                slot,
                damage,
                radius,
                ..
            } => Some((slot, damage, radius)),
            _ => None,
        })
        .collect()
}

fn index_of(session: &Session, slot: usize, part: PartId) -> usize {
    session.fighters[slot]
        .build
        .parts()
        .iter()
        .position(|p| p.id == part)
        .unwrap()
}

/// Stands A at `a` and B at `b` (feet, units), then lets them settle.
fn stand(session: &mut Session, a: [f32; 3], b: [f32; 3]) {
    session.stand(0, a).unwrap();
    session.stand(1, b).unwrap();
    idle(session, 10);
}

/// B arms, counts down and goes off: the countdown's last tick and the blast's tick are
/// checked, and the damage A took from it is returned with the detonation.
fn detonate_b(session: &mut Session) -> (i32, (u8, i32, i32)) {
    press(session, 1);
    assert!(session.events.iter().any(|event| matches!(
        event,
        Event::Destruct {
            slot: 1,
            armed: true,
            ..
        }
    )));
    for _ in 1..COUNTDOWN_TICKS {
        step(session, &[]);
        assert!(session.fighters[1].alive(), "it waits out the countdown");
        assert!(detonations(session).is_empty());
    }
    step(session, &[]);
    let went = detonations(session);
    assert_eq!(went.len(), 1, "{:?}", session.events);
    (taken(session, 0, 1), went[0])
}

#[test]
fn a_press_arms_a_warning_countdown_and_the_robot_dies_by_its_own_blast() {
    let mut session = match_of(2, HEAVY);
    stand(&mut session, [0.0, 0.0, 0.0], [0.0, 0.0, 150.0]);
    let (frags, deaths) = (session.fighters[1].frags, session.fighters[1].deaths);
    press(&mut session, 1);
    let armed_at = session.ticks() - 1;
    assert_eq!(
        session.fighters[1].destruct,
        Some(armed_at + COUNTDOWN_TICKS)
    );
    // Everyone sees the countdown.
    let snapshot = session.snapshot();
    assert_eq!(
        snapshot.slot(1).unwrap().destruct_in,
        Some(COUNTDOWN_TICKS - 1)
    );
    assert_eq!(snapshot.slot(0).unwrap().destruct_in, None);
    for _ in 1..COUNTDOWN_TICKS {
        step(&mut session, &[]);
    }
    assert_eq!(session.snapshot().slot(1).unwrap().destruct_in, Some(0));
    step(&mut session, &[]);
    // It dies by its own kill, a suicide (a frag lost), and blows up where it stood.
    assert!(!session.fighters[1].alive());
    assert!(session.events.contains(&Event::Killed {
        victim: 1,
        killer: Some(1),
        weapon: None,
    }));
    assert_eq!(session.fighters[1].frags, frags - 1);
    assert_eq!(session.fighters[1].deaths, deaths + 1);
    assert_eq!(session.fighters[1].destruct, None);
    let (_, damage, radius) = detonations(&session)[0];
    // A full HEAVY: its tank's 1000 fuel (100) and its battery's 60 charge (60).
    assert_eq!((damage, radius), (160, 310));
    // Its own death set off no battery blast: the battery is in the self-destruct's charge.
    assert!(
        !session
            .events
            .iter()
            .any(|event| matches!(event, Event::Exploded { .. }))
    );
    // A, 1.5 m away in the open, took it on its parts, credited to B.
    assert!(taken(&session, 0, 1) > 0);
    assert!(session.fighters[0].alive());
    // B comes back as usual, whole and without a countdown.
    idle(&mut session, 40);
    let b = &session.fighters[1];
    assert!(b.alive());
    assert_eq!(b.destruct, None);
    assert_eq!(b.parts, shooter_sim::damage::PartDamage::fresh(&HEAVY));
}

#[test]
fn the_blast_is_what_is_left_full_empty_a_tank_lost_or_a_battery_destroyed() {
    let near = |session: &mut Session| stand(session, [0.0, 0.0, 0.0], [0.0, 0.0, 150.0]);

    // Full: the tank's fuel and the battery's charge.
    let mut session = match_of(2, HEAVY);
    near(&mut session);
    let (full, (_, damage, _)) = detonate_b(&mut session);
    assert_eq!(damage, 160);
    assert!(full > 0);

    // Empty: no fuel and no charge. It dies, and nothing is struck.
    let mut session = match_of(2, HEAVY);
    near(&mut session);
    session.fighters[1].energy = 0;
    session.fighters[1].fuel = 0;
    let (empty, (_, damage, radius)) = detonate_b(&mut session);
    assert_eq!((empty, damage, radius), (0, 0, 0));
    assert!(!session.fighters[1].alive());

    // No fuel, the battery full: the battery's share only.
    let mut session = match_of(2, HEAVY);
    near(&mut session);
    session.fighters[1].fuel = 0;
    let (charged, (_, damage, _)) = detonate_b(&mut session);
    assert_eq!(damage, 60);
    assert!(0 < charged && charged < full, "{charged} of {full}");

    // The tank lost before (its fuel went with it): the battery's share only again.
    let mut session = match_of(2, HEAVY);
    near(&mut session);
    let tank = index_of(&session, 1, TANK);
    let b = &mut session.fighters[1];
    let build = b.build;
    let before = b.pool();
    b.parts.hurt(&build, tank, 1_000).unwrap();
    b.fuel = before.shrink_fuel(b.pool(), b.fuel);
    assert_eq!(b.fuel, 0);
    let (_, (_, damage, _)) = detonate_b(&mut session);
    assert_eq!(damage, 60);
}

/// A rail beam destroys B's battery first: it blasts then, once, and its charge is gone, so the
/// self-destruct blows with the fuel alone and no battery blasts again.
#[test]
fn a_battery_destroyed_before_the_blast_counts_once() {
    let mut session = match_of(2, HEAVY);
    // A faces B's back (its battery) 12 m away.
    stand(&mut session, [0.0, 0.0, -600.0], [0.0, 0.0, 600.0]);
    let battery = index_of(&session, 1, BATTERY);
    let rail = Intent {
        select: Some(shooter_sim::Weapon::RailGun.into()),
        ..Intent::IDLE
    };
    for _ in 0..40 {
        step(&mut session, &[rail, Intent::IDLE]);
    }
    let center = session.fighters[1].hitboxes().get(battery).unwrap().center;
    let (yaw, pitch) =
        shooter_sim::aim::at(Some(&session.world), session.fighters[0].eye(), center);
    let mut blasts = 0;
    for _ in 0..30 {
        step(
            &mut session,
            &[
                Intent {
                    yaw,
                    pitch,
                    fire: true,
                    ..rail
                },
                Intent::IDLE,
            ],
        );
        blasts += session
            .events
            .iter()
            .filter(|event| matches!(event, Event::Exploded { .. }))
            .count();
        if !session.fighters[1].parts.attached(battery) {
            break;
        }
    }
    assert!(!session.fighters[1].parts.attached(battery));
    assert_eq!(blasts, 1, "the battery blasted once when destroyed");
    let b = &session.fighters[1];
    assert_eq!(
        Charge::of(&b.build, &b.parts, b.energy, b.fuel).electricity,
        0
    );
    let (_, (_, damage, _)) = detonate_b(&mut session);
    assert_eq!(damage, 100, "the fuel alone");
    assert!(
        !session
            .events
            .iter()
            .any(|event| matches!(event, Event::Exploded { .. })),
        "no battery blasts twice"
    );
}

#[test]
fn the_blast_reaches_only_so_far_and_cover_shelters_what_is_behind_it() {
    // In the open, 2.4 m apart (a full HEAVY reaches 3.1 m): struck.
    let mut session = match_of(2, HEAVY);
    stand(&mut session, [-1_120.0, 0.0, 0.0], [-880.0, 0.0, 0.0]);
    let (open, _) = detonate_b(&mut session);
    assert!(open > 0);
    // The same 2.4 m with the wall between them: sheltered.
    let mut session = match_of(2, HEAVY);
    stand(&mut session, [1_120.0, 0.0, 0.0], [880.0, 0.0, 0.0]);
    let (covered, _) = detonate_b(&mut session);
    assert_eq!(covered, 0);
    // In the open beyond its reach: untouched.
    let mut session = match_of(2, HEAVY);
    stand(&mut session, [-1_500.0, 0.0, 0.0], [-1_100.0, 0.0, 0.0]);
    let (far, _) = detonate_b(&mut session);
    assert_eq!(far, 0);
    // Nearer is harder.
    let mut session = match_of(2, HEAVY);
    stand(&mut session, [-1_000.0, 0.0, 0.0], [-880.0, 0.0, 0.0]);
    let (close, _) = detonate_b(&mut session);
    assert!(close > open, "{close} vs {open}");
}

#[test]
fn simultaneous_blasts_all_go_off_and_each_scores_its_own() {
    let mut session = match_of(3, STANDARD);
    // A in the middle, B and C 1.5 m either side of it.
    session.stand(0, [0.0, 0.0, 0.0]).unwrap();
    session.stand(1, [0.0, 0.0, 150.0]).unwrap();
    session.stand(2, [0.0, 0.0, -150.0]).unwrap();
    idle(&mut session, 10);
    step(&mut session, &[Intent::IDLE, PRESS, PRESS]);
    idle(&mut session, COUNTDOWN_TICKS - 1);
    assert!(session.fighters[1].alive() && session.fighters[2].alive());
    step(&mut session, &[]);
    // Both go off (a full STANDARD: its tank's fuel), neither cancelled by the other's blast.
    let went = detonations(&session);
    assert_eq!(went, [(1, 100, 250), (2, 100, 250)]);
    for slot in [1, 2] {
        assert!(session.events.contains(&Event::Killed {
            victim: slot,
            killer: Some(slot),
            weapon: None,
        }));
        assert_eq!(session.fighters[usize::from(slot)].frags, -1);
    }
    // A takes both; neither is credited with the other (each died by its own kill first).
    assert!(taken(&session, 0, 1) > 0 && taken(&session, 0, 2) > 0);
    assert_eq!(taken(&session, 1, 2) + taken(&session, 2, 1), 0);
    assert_eq!(
        session
            .events
            .iter()
            .filter(|event| matches!(event, Event::Killed { .. }))
            .count(),
        2
    );
}

/// A blast that destroys a robot's root chassis is a frag for the self-destructing robot, as
/// any kill: one opponent taken along scores nothing net.
#[test]
fn a_kill_by_the_blast_is_the_self_destructing_robots_frag() {
    let mut session = match_of(2, HEAVY);
    stand(&mut session, [0.0, 0.0, 0.0], [0.0, 0.0, 150.0]);
    // A's chassis worn down to almost nothing.
    let a = &mut session.fighters[0];
    let build = a.build;
    let root = shooter_sim::damage::PartDamage::root(&build).unwrap();
    let health = a.parts.health()[root];
    a.parts.hurt(&build, root, i32::from(health) - 1).unwrap();
    detonate_b(&mut session);
    assert!(!session.fighters[0].alive());
    assert!(session.events.contains(&Event::Killed {
        victim: 0,
        killer: Some(1),
        weapon: None,
    }));
    assert_eq!(session.fighters[1].frags, 0, "+1 for A, -1 for itself");
    assert_eq!(session.fighters[0].frags, 0);
}

#[test]
fn another_press_cancels_and_death_and_the_round_end_cancel_too() {
    // Pressed again: cancelled, and nothing goes off.
    let mut session = match_of(2, HEAVY);
    stand(&mut session, [0.0, 0.0, 0.0], [0.0, 0.0, 150.0]);
    press(&mut session, 1);
    idle(&mut session, 30);
    press(&mut session, 1);
    assert!(session.events.iter().any(|event| matches!(
        event,
        Event::Destruct {
            slot: 1,
            armed: false,
            ..
        }
    )));
    assert_eq!(session.fighters[1].destruct, None);
    for _ in 0..2 * COUNTDOWN_TICKS {
        step(&mut session, &[]);
        assert!(detonations(&session).is_empty());
    }
    assert!(session.fighters[1].alive());

    // Killed during the countdown (a fall out of the arena): it never goes off, and after its
    // respawn it counts nothing.
    let mut session = match_of(2, HEAVY);
    stand(&mut session, [0.0, 0.0, 0.0], [0.0, 0.0, 150.0]);
    press(&mut session, 1);
    #[allow(clippy::cast_precision_loss)]
    session
        .stand(1, [0.0, KILL_HEIGHT_UNITS as f32 - 100.0, 0.0])
        .unwrap();
    step(&mut session, &[]);
    assert!(!session.fighters[1].alive());
    assert_eq!(session.fighters[1].destruct, None);
    for _ in 0..2 * COUNTDOWN_TICKS {
        step(&mut session, &[]);
        assert!(detonations(&session).is_empty());
    }
    assert!(session.fighters[1].alive());
    assert_eq!(session.fighters[1].destruct, None);
    // A dead robot's press arms nothing.
    session
        .stand(1, [0.0, KILL_HEIGHT_UNITS as f32 - 100.0, 0.0])
        .unwrap();
    step(&mut session, &[]);
    assert!(!session.fighters[1].alive());
    press(&mut session, 1);
    assert_eq!(session.fighters[1].destruct, None);

    // The round ends during the countdown: cancelled; between rounds a press arms nothing; the
    // next round starts without one.
    let mut session = Session::with_rules(
        arena(),
        Rules {
            time_limit_ticks: 100,
            ..rules(2)
        },
    )
    .unwrap();
    for id in &PEOPLE[..2] {
        session.join(*id).unwrap();
    }
    idle(&mut session, 10);
    press(&mut session, 1);
    assert!(session.fighters[1].destruct.is_some());
    while matches!(session.phase, Phase::Playing { .. }) {
        step(&mut session, &[]);
    }
    assert_eq!(session.fighters[1].destruct, None);
    press(&mut session, 1);
    assert_eq!(session.fighters[1].destruct, None);
    assert!(
        !session
            .events
            .iter()
            .any(|event| matches!(event, Event::Destruct { .. }))
    );
    for _ in 0..2 * COUNTDOWN_TICKS {
        step(&mut session, &[]);
        assert!(detonations(&session).is_empty());
    }
    assert!(session.fighters[1].alive());
}

/// Stranded (out of electricity and fuel, so nothing walks or recharges) or in the air, a robot
/// can still arm and go off.
#[test]
fn a_stranded_or_airborne_robot_can_still_self_destruct() {
    let mut session = match_of(2, STANDARD);
    stand(&mut session, [0.0, 0.0, 0.0], [0.0, 0.0, 300.0]);
    let b = &mut session.fighters[1];
    b.energy = 0;
    b.fuel = 0;
    let (_, (slot, damage, _)) = detonate_b(&mut session);
    assert_eq!((slot, damage), (1, 0));
    assert!(!session.fighters[1].alive());

    // In the air: thrown up just before it goes off, it blows where it is.
    let mut session = match_of(2, STANDARD);
    stand(&mut session, [0.0, 0.0, 0.0], [0.0, 0.0, 300.0]);
    let standing = session.fighters[1].center()[1];
    press(&mut session, 1);
    idle(&mut session, COUNTDOWN_TICKS - 12);
    session.launch(1, [0.0, 700.0, 0.0]).unwrap();
    idle(&mut session, 11);
    assert!(session.fighters[1].alive());
    let height = session.fighters[1].center()[1];
    assert!(height > standing + 30.0, "{height} vs {standing}");
    step(&mut session, &[]);
    let at = session
        .events
        .iter()
        .find_map(|event| match *event {
            Event::Detonated { at, .. } => Some(at),
            _ => None,
        })
        .unwrap();
    assert!(at[1] > standing + 30.0, "{at:?}");
}

/// After the blast, a person holding Edit Robot stays in the builder and comes back as the
/// build it configured there, as after any death.
#[test]
fn edit_robot_between_lives_follows_a_self_destruct() {
    let mut session = match_of(2, HEAVY);
    stand(&mut session, [0.0, 0.0, 0.0], [0.0, 0.0, 300.0]);
    detonate_b(&mut session);
    let edit = Intent {
        edit: true,
        ..Intent::IDLE
    };
    assert!(session.configure(PEOPLE[1], STANDARD));
    for _ in 0..90 {
        step(&mut session, &[Intent::IDLE, edit]);
        assert!(!session.fighters[1].alive(), "in the builder");
    }
    step(&mut session, &[]);
    let b = &session.fighters[1];
    assert!(b.alive());
    assert_eq!(b.build, STANDARD);
    assert_eq!(b.destruct, None);
}

#[test]
fn presses_countdowns_and_blasts_travel_on_the_wire() {
    let intent = decode_input(&encode_input(PRESS, 7, None, None))
        .unwrap()
        .intent;
    assert!(intent.destruct);
    assert!(
        !decode_input(&encode_input(Intent::IDLE, 7, None, None))
            .unwrap()
            .intent
            .destruct
    );
    let mut session = match_of(2, HEAVY);
    // Snapshots carry valid builds only (the armory is over budget).
    rebuild(&mut session, 0, HEAVY);
    stand(&mut session, [0.0, 0.0, 0.0], [0.0, 0.0, 150.0]);
    press(&mut session, 1);
    idle(&mut session, 20);
    let snapshot = session.snapshot();
    assert_eq!(
        decode_snapshot(&encode_snapshot(&snapshot)).unwrap(),
        snapshot
    );
    assert_eq!(
        snapshot.slot(1).unwrap().destruct_in,
        Some(COUNTDOWN_TICKS - 21)
    );
    // A countdown longer than any is malformed.
    let mut tampered = snapshot;
    tampered.fighters[1].destruct_in = Some(COUNTDOWN_TICKS);
    assert!(decode_snapshot(&encode_snapshot(&tampered)).is_err());
    // The blast's tick and its events.
    idle(&mut session, COUNTDOWN_TICKS - 20);
    let snapshot = session.snapshot();
    assert!(!detonations(&session).is_empty());
    assert_eq!(
        decode_snapshot(&encode_snapshot(&snapshot)).unwrap(),
        snapshot
    );
    assert_eq!(snapshot.slot(1).unwrap().destruct_in, None);
}

/// B's inputs: walking and turning, arming at 1 s, cancelling at 1.5 s, arming again at 3 s.
fn scripted(tick: u32) -> Intent {
    Intent {
        forward: if tick % 120 < 60 { 1 } else { -1 },
        yaw: u16::try_from(tick % 400).unwrap() * 100,
        destruct: matches!(tick, 60 | 90 | 180),
        ..Intent::IDLE
    }
}

#[test]
fn a_client_predicts_the_countdown_and_the_blast_like_the_host() {
    let mut host = Session::with_rules(arena(), rules(1)).unwrap();
    host.join(PEOPLE[0]).unwrap();
    rebuild(&mut host, 0, HEAVY);
    let mut predictor = Session::new(host.arena.clone()).unwrap();
    let mut counted = 0;
    let mut died = None;
    for tick in 0..420u32 {
        let intent = scripted(tick);
        let view_tick = host.ticks();
        let command = decode_input(&encode_input(intent, view_tick, None, None)).unwrap();
        host.queue(
            PEOPLE[0],
            Input {
                sequence: tick + 1,
                intent: command.intent,
                view_tick,
            },
        );
        host.step().unwrap();
        let snapshot = decode_snapshot(&encode_snapshot(&host.snapshot())).unwrap();
        let me = snapshot.of_player(PEOPLE[0]).unwrap().clone();
        if tick == 30 {
            // The predictor resumes from the host's word: robot, parts, resources, countdown.
            predictor.fighters[0].build = me.build;
            predictor.fighters[0].parts = me.parts;
            predictor.fighters[0].energy = me.energy;
            predictor.fighters[0].fuel = me.fuel;
            predictor
                .place(0, me.robot.as_ref().unwrap(), me.motion, me.held, me.hook)
                .unwrap();
            predictor.fighters[0].destruct = me.destruct_in.map(|left| predictor.ticks() + left);
        } else if tick > 30 && died.is_none() {
            predictor.tick(intent).unwrap();
            let local = predictor.local();
            let predicted = local
                .destruct
                .map(|at| at.saturating_sub(predictor.ticks()));
            assert_eq!(predicted, me.destruct_in, "tick {tick}");
            assert_eq!(local.alive(), me.alive, "tick {tick}");
            assert_eq!(detonations(&predictor), detonations(&host), "tick {tick}");
            counted += u32::from(me.destruct_in.is_some());
            if !me.alive {
                died = Some(tick);
            }
        }
    }
    // Armed at 60, cancelled at 90, armed at 180: gone off 3 s later.
    assert_eq!(died, Some(180 + COUNTDOWN_TICKS));
    assert_eq!(counted, 30 + COUNTDOWN_TICKS);
}

#[test]
fn a_self_destruct_match_replays_bit_for_bit() {
    let play = || {
        let mut session = match_of(3, HEAVY);
        session.stand(0, [0.0, 0.0, 0.0]).unwrap();
        session.stand(1, [0.0, 0.0, 150.0]).unwrap();
        session.stand(2, [100.0, 0.0, -150.0]).unwrap();
        let mut bytes = Vec::new();
        let mut went = 0;
        for tick in 0..900u32 {
            let b = scripted(tick);
            let c = Intent {
                destruct: tick % 400 == 50,
                fire: tick % 30 < 5,
                ..scripted(tick + 77)
            };
            step(&mut session, &[Intent::IDLE, b, c]);
            went += detonations(&session).len();
            bytes.extend(encode_snapshot(&session.snapshot()));
        }
        (went, bytes)
    };
    let (went, bytes) = play();
    assert!(went >= 2, "{went}");
    assert!(play().1 == bytes, "the match replays bit for bit");
}

#[test]
fn bots_never_self_destruct() {
    let mut session = Session::with_rules(Arena::greybox(), Rules::deathmatch(5)).unwrap();
    for _ in 0..20 * 60 {
        session.step().unwrap();
        assert!(
            !session
                .events
                .iter()
                .any(|event| matches!(event, Event::Destruct { .. } | Event::Detonated { .. }))
        );
    }
}

#[test]
fn the_charge_is_the_connected_tanks_fuel_and_batteries_share() {
    let pool = Pool::of(&HEAVY);
    let parts = shooter_sim::damage::PartDamage::fresh(&HEAVY);
    // Half the electricity: the batteries hold their share of it (60 of 160 stored).
    let half = Charge::of(&HEAVY, &parts, pool.capacity / 2, pool.fuel / 2);
    assert_eq!(half.electricity, 30_000);
    assert_eq!(half.fuel, 500_000);
    assert_eq!(half.damage(), 80);
}
