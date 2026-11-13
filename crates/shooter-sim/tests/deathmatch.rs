//! The deathmatch on physics-engine: bots fight and score deterministically, people take slots
//! from bots, lag-compensated hitscan, the round's end and restart, rocket self-damage, and the
//! wire round trip.

use shooter_sim::melee;
use shooter_sim::physics::Vec3i;
use shooter_sim::scenarios;
use shooter_sim::session::{MAX_HEALTH, SELF_SPLASH_PERCENT};
use shooter_sim::tour::run_legs;
use shooter_sim::weapons;
use shooter_sim::wire::{decode_input, decode_snapshot, encode_input, encode_snapshot};
use shooter_sim::{
    Arena, Controller, Event, Input, Intent, Phase, Pilot, Rules, Session, Spawn, Weapon,
};

fn bot_match(seed: u64, ticks: u32) -> Session {
    let mut session = Session::with_rules(Arena::greybox(), Rules::deathmatch(seed)).unwrap();
    for _ in 0..ticks {
        session.step().unwrap();
    }
    session
}

#[test]
fn bots_fight_score_and_respawn() {
    let mut session = Session::with_rules(Arena::greybox(), Rules::deathmatch(7)).unwrap();
    let (mut kills, mut shots, mut spawns) = (0, 0, 0);
    for _ in 0..90 * 60 {
        session.step().unwrap();
        for event in &session.events {
            match event {
                Event::Killed { .. } => kills += 1,
                Event::Fired { .. } => shots += 1,
                Event::Spawned { .. } => spawns += 1,
                _ => {}
            }
        }
    }
    println!("{shots} shots, {kills} kills, {spawns} respawns");
    assert!(shots > 100, "{shots} shots");
    assert!(kills >= 3, "{kills} kills in 90 s");
    assert!(spawns >= kills - 6, "{spawns} respawns for {kills} kills");
    let deaths: i32 = session.fighters.iter().map(|f| f.deaths).sum();
    assert!(deaths > 0);
    for fighter in &session.fighters {
        assert!(matches!(fighter.pilot, Pilot::Bot(_)));
        assert!((0..=MAX_HEALTH).contains(&fighter.health));
    }
}

#[test]
fn a_bot_match_replays_exactly_and_seeds_differ() {
    let a = encode_snapshot(&bot_match(3, 1_200).snapshot());
    let b = encode_snapshot(&bot_match(3, 1_200).snapshot());
    let c = encode_snapshot(&bot_match(4, 1_200).snapshot());
    assert_eq!(a, b);
    assert_ne!(a, c);
}

#[test]
fn people_take_slots_from_bots_and_give_them_back() {
    let mut session = bot_match(1, 30);
    assert_eq!(session.join(41).unwrap(), Some(0));
    assert_eq!(session.join(42).unwrap(), Some(1));
    assert_eq!(session.join(41).unwrap(), Some(0));
    assert_eq!(session.fighters[0].pilot, Pilot::Human { id: 41 });
    assert!(session.fighters[0].alive());
    assert_eq!(session.fighters[0].health, MAX_HEALTH);
    let snapshot = session.snapshot();
    assert_eq!(snapshot.of_player(42).unwrap().slot, 1);
    assert!(session.leave(41).unwrap());
    assert!(!session.leave(41).unwrap());
    assert!(matches!(session.fighters[0].pilot, Pilot::Bot(_)));
    assert_eq!(
        session.snapshot().slot(0).unwrap().controller,
        Controller::Bot
    );
}

fn duel_arena() -> Arena {
    Arena {
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
    }
}

fn duel_rules() -> Rules {
    Rules {
        slots: 2,
        bots: false,
        local: false,
        seed: 0,
        frag_limit: 2,
        time_limit_ticks: 0,
        intermission_ticks: 60,
        respawn_ticks: 30,
    }
}

/// A (slot 0) faces B (slot 1) 12 m away; B strafes out of the line of fire for 12 ticks, then A
/// fires the rifle having seen B at `seen_ticks_ago`. Whether B was hit.
fn duel_shot(seen_ticks_ago: u32) -> bool {
    let mut session = Session::with_rules(duel_arena(), duel_rules()).unwrap();
    assert_eq!(session.join(1).unwrap(), Some(0));
    assert_eq!(session.join(2).unwrap(), Some(1));
    let mut sequence = 0;
    let mut step = |session: &mut Session, a: Intent, b: Intent, view_tick: u32| {
        sequence += 1;
        session.queue(
            1,
            Input {
                sequence,
                intent: a,
                view_tick,
            },
        );
        session.queue(
            2,
            Input {
                sequence,
                intent: Intent { yaw: 32_768, ..b },
                view_tick,
            },
        );
        session.step().unwrap();
    };
    for _ in 0..10 {
        let now = session.ticks();
        step(&mut session, Intent::IDLE, Intent::IDLE, now);
    }
    let strafe = Intent {
        strafe: 1,
        ..Intent::IDLE
    };
    for _ in 0..12 {
        let now = session.ticks();
        step(&mut session, Intent::IDLE, strafe, now);
    }
    let fire = Intent {
        fire: true,
        ..Intent::IDLE
    };
    let seen = session.ticks() - seen_ticks_ago;
    step(&mut session, fire, strafe, seen);
    assert!(
        session
            .events
            .iter()
            .any(|event| matches!(event, Event::Fired { slot: 0, .. }))
    );
    session
        .events
        .iter()
        .any(|event| matches!(event, Event::Hit { victim: 1, .. }))
}

#[test]
fn hitscan_is_lag_compensated() {
    assert!(!duel_shot(0), "B has moved out of the line of fire");
    assert!(duel_shot(12), "A saw B in the line of fire 12 ticks ago");
}

/// A (slot 0) and B (slot 1) face each other 2 m apart; B strafes for 24 ticks (out of the
/// blade's arc), then A swings having seen B at `seen_ticks_ago`. Whether B was struck.
fn melee_duel(seen_ticks_ago: u32) -> bool {
    let arena = Arena {
        spawn: Vec3i::new(0, 0, -100),
        spawns: vec![Spawn {
            feet: Vec3i::new(0, 0, 100),
            yaw: 32_768,
        }],
        ..duel_arena()
    };
    let mut session = Session::with_rules(arena, duel_rules()).unwrap();
    session.join(1).unwrap();
    session.join(2).unwrap();
    let mut sequence = 0;
    let mut step = |session: &mut Session, a: Intent, b: Intent, view_tick: u32| {
        sequence += 1;
        for (id, intent) in [(1, a), (2, Intent { yaw: 32_768, ..b })] {
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
    };
    let strafe = Intent {
        strafe: 1,
        ..Intent::IDLE
    };
    for _ in 0..24 {
        let now = session.ticks();
        step(&mut session, Intent::IDLE, strafe, now);
    }
    let swing = Intent {
        melee: true,
        ..Intent::IDLE
    };
    let seen = session.ticks() - seen_ticks_ago;
    step(&mut session, swing, Intent::IDLE, seen);
    assert!(session.events.iter().any(|event| matches!(
        event,
        Event::Fired {
            slot: 0,
            weapon: Weapon::EnergyBlade,
            ..
        }
    )));
    session.events.iter().any(|event| {
        matches!(
            event,
            Event::Hit {
                victim: 1,
                damage: melee::DAMAGE,
                ..
            }
        )
    })
}

#[test]
fn blade_swings_are_lag_compensated() {
    assert!(!melee_duel(0), "B has strafed out of the blade's arc");
    assert!(melee_duel(15), "A saw B in front 15 ticks ago");
}

#[test]
fn the_blade_kills_in_four_swings_whatever_gun_is_held() {
    let arena = Arena {
        spawn: Vec3i::new(0, 0, -100),
        spawns: vec![Spawn {
            feet: Vec3i::new(0, 0, 100),
            yaw: 32_768,
        }],
        ..duel_arena()
    };
    let mut session = Session::with_rules(arena, duel_rules()).unwrap();
    session.join(1).unwrap();
    session.join(2).unwrap();
    session.step().unwrap();
    let mut killed_at = None;
    for sequence in 1..=120 {
        let now = session.ticks();
        // A follows B up after each shove, stopping 1.5 m short.
        let gap = session.fighters[1].player.center(&session.world)[2]
            - session.fighters[0].player.center(&session.world)[2];
        let swing = Intent {
            melee: true,
            forward: i8::from(gap > 150.0),
            select: Some(Weapon::RocketLauncher),
            ..Intent::IDLE
        };
        session.queue(
            1,
            Input {
                sequence,
                intent: swing,
                view_tick: now,
            },
        );
        session.step().unwrap();
        if session.events.iter().any(|event| {
            matches!(
                event,
                Event::Killed {
                    victim: 1,
                    killer: Some(0),
                    weapon: Some(Weapon::EnergyBlade),
                }
            )
        }) {
            killed_at = Some(sequence);
            break;
        }
    }
    // Swings at ticks 0, 30, 60 and 90; A walks after the shoves.
    assert_eq!(killed_at, Some(91));
    assert_eq!(session.fighters[0].arsenal.held, Weapon::RocketLauncher);
}

#[test]
fn bots_swing_the_blade_up_close() {
    let mut session = Session::with_rules(Arena::greybox(), Rules::deathmatch(7)).unwrap();
    let mut swings = 0;
    for _ in 0..180 * 60 {
        session.step().unwrap();
        swings += session
            .events
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    Event::Fired {
                        weapon: Weapon::EnergyBlade,
                        ..
                    }
                )
            })
            .count();
    }
    assert!(swings > 0, "no bot swung the blade in 3 minutes");
}

#[test]
fn rail_kills_score_and_the_round_ends_and_restarts() {
    let mut session = Session::with_rules(duel_arena(), duel_rules()).unwrap();
    session.join(1).unwrap();
    session.join(2).unwrap();
    let rail = Intent {
        fire: true,
        select: Some(Weapon::RailGun),
        ..Intent::IDLE
    };
    let mut kills = 0;
    let mut over = false;
    for sequence in 1..400 {
        let now = session.ticks();
        session.queue(
            1,
            Input {
                sequence,
                intent: rail,
                view_tick: now,
            },
        );
        session.step().unwrap();
        for event in &session.events {
            match *event {
                Event::Killed {
                    victim: 1,
                    killer: Some(0),
                    weapon: Some(Weapon::RailGun),
                } => kills += 1,
                Event::RoundOver { winner } => {
                    assert_eq!(winner, Some(0));
                    over = true;
                }
                _ => {}
            }
        }
        if over {
            break;
        }
    }
    assert_eq!(kills, 2);
    assert!(over);
    assert_eq!(session.fighters[0].frags, 2);
    assert_eq!(session.fighters[1].deaths, 2);
    let Phase::Intermission { restarts_at, .. } = session.phase else {
        panic!("{:?}", session.phase);
    };
    while session.ticks() <= restarts_at {
        session.step().unwrap();
    }
    assert!(matches!(session.phase, Phase::Playing { .. }));
    assert_eq!(session.fighters[0].frags, 0);
    assert!(session.fighters.iter().all(|f| f.alive() && f.deaths == 0));
}

#[test]
fn a_rocket_jump_costs_health() {
    let scenario = scenarios::ROCKET_JUMP;
    let mut session = Session::new((scenario.arena)()).unwrap();
    run_legs(&mut session, scenario.legs).unwrap();
    let lost = MAX_HEALTH - session.local().health;
    assert!(
        (20..=100 * SELF_SPLASH_PERCENT / 100).contains(&lost),
        "lost {lost}"
    );
}

/// The grapple scenario played by a person on a host: the host sets the hook, the snapshot
/// carries it, and a client predicting from that snapshot follows the host's path exactly.
#[test]
fn a_hosted_grapple_is_carried_and_predicted() {
    let rules = Rules {
        slots: 1,
        ..duel_rules()
    };
    let mut host = Session::with_rules((scenarios::GRAPPLE.arena)(), rules).unwrap();
    assert_eq!(host.join(7).unwrap(), Some(0));
    let mut predictor = Session::new((scenarios::GRAPPLE.arena)()).unwrap();
    let mut predicting = false;
    let mut sequence = 0;
    for leg in scenarios::GRAPPLE.legs {
        for _ in 0..leg.ticks {
            sequence += 1;
            host.queue(
                7,
                Input {
                    sequence,
                    intent: leg.intent,
                    view_tick: 0,
                },
            );
            host.step().unwrap();
            let snapshot = decode_snapshot(&encode_snapshot(&host.snapshot())).unwrap();
            let me = *snapshot.of_player(7).unwrap();
            if predicting {
                predictor.tick(leg.intent).unwrap();
                let predicted = predictor.local();
                let center = predicted.player.center(&predictor.world);
                assert_eq!(predicted.hook, me.hook, "{} tick {sequence}", leg.name);
                for axis in 0..3 {
                    assert!(
                        (center[axis] - me.position[axis]).abs() < 0.01,
                        "{} tick {sequence}: {center:?} vs {:?}",
                        leg.name,
                        me.position
                    );
                }
            } else if me.hook.is_some() {
                predictor
                    .place(0, me.position, me.velocity, me.motion, me.held, me.hook)
                    .unwrap();
                predicting = true;
            }
        }
    }
    assert!(predicting, "the host set the hook");
    let me = *host.snapshot().of_player(7).unwrap();
    assert!(me.hook.is_none());
    assert!(me.position[1] > 380.0, "on the ledge: {:?}", me.position);
}

#[test]
fn inputs_and_snapshots_round_trip() {
    let intent = Intent {
        forward: -1,
        strafe: 1,
        yaw: 51_234,
        pitch: -4_000,
        jump: true,
        sprint: false,
        crouch: true,
        fire: true,
        melee: true,
        select: Some(Weapon::RocketLauncher),
    };
    let command = decode_input(&encode_input(intent, 77, None, None)).unwrap();
    assert_eq!((command.intent, command.view_tick), (intent, 77));
    assert_eq!((command.name, command.build), (None, None));
    let command = decode_input(&encode_input(intent, 77, Some("Ripley"), None)).unwrap();
    assert_eq!(command.name.as_deref(), Some("Ripley"));
    let mut session = bot_match(5, 0);
    let slot = session.join(9).unwrap().unwrap();
    assert!(session.rename(9, "  <Rip>  ley!  "));
    assert!(!session.rename(10, "Nobody"));
    assert_eq!(session.snapshot().name(slot), Some("Rip ley"));
    let mut saw_events = false;
    for _ in 0..1_200 {
        session.step().unwrap();
        let snapshot = session.snapshot();
        saw_events |= !snapshot.events.is_empty();
        assert_eq!(
            decode_snapshot(&encode_snapshot(&snapshot)).unwrap(),
            snapshot
        );
    }
    assert!(saw_events);
    assert!(decode_snapshot(&[1, 2, 3]).is_err());
}

/// A client whose clock runs fast (or that bursts inputs after a stall) builds a standing input
/// backlog: the host trims it back to `BACKLOG_KEEP` within a window, keeping the order and a
/// weapon switch from the dropped inputs. A person whose inputs stop (a lost connection) holds
/// the last one briefly, then stands still.
#[test]
fn a_standing_input_backlog_is_trimmed_and_a_silent_person_stops() {
    use shooter_sim::session::{BACKLOG_KEEP, BACKLOG_WINDOW_TICKS, HOLD_INPUT_TICKS};
    let mut session = bot_match(3, 0);
    let slot = usize::from(session.join(4).unwrap().unwrap());
    let forward = Intent {
        forward: 1,
        fire: true,
        ..Intent::IDLE
    };
    let mut sequence = 0;
    let mut send = |session: &mut Session, intent: Intent| {
        sequence += 1;
        assert!(session.queue(
            4,
            Input {
                sequence,
                intent,
                view_tick: 0,
            }
        ));
    };
    // A burst of six, then one input a tick: six stay queued until the window trims them.
    for _ in 0..5 {
        send(&mut session, forward);
    }
    send(
        &mut session,
        Intent {
            select: Some(Weapon::RocketLauncher),
            ..forward
        },
    );
    let mut most = 0;
    for _ in 0..2 * BACKLOG_WINDOW_TICKS {
        send(&mut session, forward);
        session.step().unwrap();
        most = most.max(session.fighters[slot].queued_inputs());
    }
    assert_eq!(most, 6);
    assert_eq!(session.fighters[slot].queued_inputs(), BACKLOG_KEEP - 1);
    assert_eq!(session.fighters[slot].arsenal.held, Weapon::RocketLauncher);
    let acked = session.fighters[slot].ack;
    assert_eq!(acked, sequence - (BACKLOG_KEEP as u32 - 1));

    for _ in 0..HOLD_INPUT_TICKS + 1 {
        session.step().unwrap();
    }
    let still = session.fighters[slot].intent;
    assert!(!still.fire && still.forward == 0, "{still:?}");
    assert_eq!(session.fighters[slot].ack, sequence);
}

#[test]
fn the_flame_burns_other_fighters_never_its_owner() {
    let mut session = Session::with_rules(duel_arena(), duel_rules()).unwrap();
    session.join(1).unwrap();
    session.join(2).unwrap();
    let flame = Intent {
        forward: 1,
        fire: true,
        select: Some(Weapon::Flamethrower),
        ..Intent::IDLE
    };
    let (mut burnt, mut first) = (0, None);
    for sequence in 1..240 {
        let now = session.ticks();
        session.queue(
            1,
            Input {
                sequence,
                intent: flame,
                view_tick: now,
            },
        );
        session.step().unwrap();
        for event in &session.events {
            match *event {
                Event::Hit {
                    victim: 1,
                    attacker: Some(0),
                    damage,
                    ..
                } => {
                    burnt += damage;
                    let gap = session.fighters[1].player.center(&session.world)[2]
                        - session.fighters[0].player.center(&session.world)[2];
                    first.get_or_insert(gap);
                }
                Event::Hit { victim: 0, .. } => panic!("the flame burnt its owner"),
                _ => {}
            }
        }
    }
    // The flame reaches B only once A has walked within its 6 m (and B's half width).
    let reach = (weapons::FLAME_REACH_UNITS + weapons::FLAME_RADIUS_UNITS + 30) as f32;
    assert!(
        first.is_some_and(|gap| gap <= reach && gap > reach - 60.0),
        "first burn {first:?} apart"
    );
    assert!(burnt >= MAX_HEALTH, "burnt {burnt}");
    assert_eq!(session.fighters[0].health, MAX_HEALTH);
    assert_eq!(session.fighters[1].deaths, 1);
}

#[test]
fn the_flame_ignites_before_it_burns_a_fighter() {
    // B stands 3 m in front of A: well inside the flame from the start.
    let arena = Arena {
        spawns: vec![Spawn {
            feet: Vec3i::new(0, 0, -300),
            yaw: 32_768,
        }],
        ..duel_arena()
    };
    let mut session = Session::with_rules(arena, duel_rules()).unwrap();
    session.join(1).unwrap();
    session.join(2).unwrap();
    let mut burns = Vec::new();
    for sequence in 1..60 {
        let now = session.ticks();
        // The trigger is let go once, on the 35th tick: the flame goes out and ignites anew.
        let intent = Intent {
            fire: sequence != 35,
            select: Some(Weapon::Flamethrower),
            ..Intent::IDLE
        };
        session.queue(
            1,
            Input {
                sequence,
                intent,
                view_tick: now,
            },
        );
        session.step().unwrap();
        let burnt = session.events.iter().any(|event| {
            matches!(
                event,
                Event::Hit {
                    victim: 1,
                    attacker: Some(0),
                    ..
                }
            )
        });
        burns.push(burnt);
    }
    // The switch takes 12 ticks; then 15 ticks of ignition before the first burn.
    let switch = weapons::SWITCH_TICKS as usize;
    let ignition = weapons::FLAME_IGNITION_TICKS as usize;
    let first = switch + ignition;
    assert_eq!(burns.iter().position(|burnt| *burnt), Some(first));
    assert!(burns[first..34].iter().all(|burnt| *burnt));
    // After letting go on tick 35, it ignites again.
    assert!(
        burns[34..35 + ignition].iter().all(|burnt| !*burnt),
        "{burns:?}"
    );
    assert!(burns[35 + ignition..].iter().all(|burnt| *burnt));
}
