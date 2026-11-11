//! The deathmatch on physics-engine: bots fight and score deterministically, people take slots
//! from bots, lag-compensated hitscan, the round's end and restart, rocket self-damage, and the
//! wire round trip.

use shooter_sim::scenarios;
use shooter_sim::session::{MAX_HEALTH, SELF_SPLASH_PERCENT};
use shooter_sim::sim_foundation::physics::Vec3i;
use shooter_sim::tour::run_legs;
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
        select: Some(Weapon::RocketLauncher),
    };
    assert_eq!(
        decode_input(&encode_input(intent, 77)).unwrap(),
        (intent, 77)
    );
    let mut session = bot_match(5, 0);
    session.join(9).unwrap();
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
