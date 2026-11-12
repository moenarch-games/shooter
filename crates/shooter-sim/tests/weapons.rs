//! The energy weapons on physics-engine: hitscan damage, walls stopping bolts, rocket splash and
//! rocket jumping, shotgun blasts, the laser and the flamethrower, and the greybox's dummies.

use shooter_sim::dummies::DUMMY_HEALTH;
use shooter_sim::player::PLAYER_HALF_EXTENTS;
use shooter_sim::scenarios::{self, Scenario};
use shooter_sim::sim_foundation::physics::Vec3i;
use shooter_sim::tour::{Sample, end_of, run_legs};
use shooter_sim::{Arena, Event, Intent, Post, Session, Weapon};

fn run(scenario: Scenario) -> (Session, Vec<Sample>) {
    let mut session = Session::new((scenario.arena)()).expect("arena builds");
    let samples = run_legs(&mut session, scenario.legs).expect("scenario runs");
    (session, samples)
}

const HALF: f32 = PLAYER_HALF_EXTENTS.y as f32;

fn damage_in(samples: &[Sample], leg: &str) -> i32 {
    samples
        .iter()
        .filter(|s| s.leg == leg)
        .map(|s| s.damage)
        .sum()
}

#[test]
fn a_rocket_jump_reaches_a_ledge_a_double_jump_cannot() {
    let (_, samples) = run(scenarios::ROCKET_JUMP);
    assert_eq!(samples.iter().filter(|s| s.exploded).count(), 1);
    let peak = samples.iter().map(|s| s.center[1]).fold(f32::MIN, f32::max);
    assert!(peak - HALF > 300.0, "apex {}", peak - HALF);
    let top = end_of(&samples, "on the ledge").unwrap();
    assert!(top.grounded, "{top:?}");
    assert!((top.center[1] - (250.0 + HALF)).abs() < 2.0, "{top:?}");
}

#[test]
fn the_rifle_and_rail_gun_down_a_dummy_and_walls_stop_bolts() {
    let (session, samples) = run(scenarios::SHOOTING_RANGE);
    let rifle = damage_in(&samples, "rifle the middle");
    assert_eq!(rifle, 10 * 12, "ten rifle bolts");
    assert_eq!(damage_in(&samples, "rail the middle"), 85);
    let middle = session.dummies[0];
    assert!(!middle.alive() && middle.health == 0, "{middle:?}");

    let rocket = damage_in(&samples, "rocket flies") + damage_in(&samples, "rocket the east");
    assert!((50..=100).contains(&rocket), "rocket {rocket}");
    assert_eq!(session.dummies[1].health, DUMMY_HEALTH - rocket);

    let fired = samples
        .iter()
        .filter(|s| s.leg == "rifle the wall" && s.fired.is_some())
        .count();
    assert_eq!(fired, 5);
    assert_eq!(damage_in(&samples, "rifle the wall"), 0);
    assert_eq!(session.dummies[2].health, DUMMY_HEALTH);
}

#[test]
fn a_downed_dummy_comes_back() {
    let (mut session, _) = run(scenarios::SHOOTING_RANGE);
    for _ in 0..200 {
        session.tick(Intent::IDLE).unwrap();
    }
    let middle = session.dummies[0];
    assert!(
        middle.alive() && middle.health == DUMMY_HEALTH,
        "{middle:?}"
    );
    assert!(middle.center(&session.world).is_some());
}

#[test]
fn dummies_stand_and_patrol() {
    let post = |feet: [i32; 3], patrol_units| Post {
        feet: Vec3i::new(feet[0], feet[1], feet[2]),
        patrol_units,
    };
    let arena = Arena {
        posts: vec![post([500, 0, 800], 0), post([-1_000, 0, -800], 300)],
        ..Arena::greybox()
    };
    let mut session = Session::new(arena).unwrap();
    let start = session.dummies[1].center(&session.world).unwrap();
    let mut xs = Vec::new();
    for _ in 0..240 {
        session.tick(Intent::IDLE).unwrap();
        xs.push(session.dummies[1].center(&session.world).unwrap()[0]);
    }
    let (low, high) = xs
        .iter()
        .fold((f32::MAX, f32::MIN), |(l, h), x| (l.min(*x), h.max(*x)));
    assert!(high - low > 500.0, "patrol {low}..{high} from {start:?}");
    for dummy in &session.dummies {
        let center = dummy.center(&session.world).unwrap();
        let feet = dummy.post.feet;
        assert!(
            (center[1] - HALF - feet.y as f32).abs() < 2.0,
            "{dummy:?} at {center:?}"
        );
    }
}

#[test]
fn a_hitscan_shot_reports_its_beam() {
    let mut session = Session::new((scenarios::SHOOTING_RANGE.arena)()).unwrap();
    session
        .tick(Intent {
            fire: true,
            ..Intent::IDLE
        })
        .unwrap();
    let beam = session.events.iter().find_map(|event| match *event {
        Event::Beam {
            weapon,
            end,
            struck,
            ..
        } => Some((weapon, end, struck)),
        _ => None,
    });
    let (weapon, end, struck) = beam.expect("a beam");
    assert_eq!(weapon, Weapon::EnergyRifle);
    assert!(struck);
    // The middle dummy's capsule is 30 units round its post at z = 0.
    assert!((end[2] + 30.0).abs() < 8.0, "{end:?}");
}

#[test]
fn weapon_scenarios_replay_exactly() {
    for scenario in [
        scenarios::ROCKET_JUMP,
        scenarios::SHOOTING_RANGE,
        scenarios::CLOSE_RANGE,
    ] {
        assert_eq!(run(scenario).1, run(scenario).1, "{}", scenario.name);
    }
}

#[test]
fn the_flamethrower_shotgun_and_laser_hurt_what_they_reach() {
    let (session, samples) = run(scenarios::CLOSE_RANGE);
    // Three damage a tick for half a second; the pillar keeps the west dummy unburnt.
    assert_eq!(damage_in(&samples, "flame the south"), 30 * 3);
    assert_eq!(damage_in(&samples, "flame the pillar"), 0);
    assert_eq!(session.dummies[2].health, DUMMY_HEALTH);
    // Four metres out, the aim and the inner ring hit; the outer ring's pellets fly past.
    let blast = damage_in(&samples, "blast the south");
    assert_eq!(blast, 6 * 9, "blast");
    let fired = |leg: &str| {
        samples
            .iter()
            .filter(|s| s.leg == leg && s.fired.is_some())
            .count()
    };
    assert_eq!(fired("blast the south"), 1);
    // The laser burns every tick it is held.
    assert_eq!(fired("laser the east"), 30);
    assert_eq!(damage_in(&samples, "laser the east"), 30 * 2);
    assert_eq!(session.dummies[1].health, DUMMY_HEALTH - 60);
    assert_eq!(session.dummies[0].health, DUMMY_HEALTH - 90 - blast);
}

#[test]
fn a_shotgun_blast_is_a_fixed_pattern_of_pellet_beams() {
    let mut session = Session::new((scenarios::CLOSE_RANGE.arena)()).unwrap();
    let blast = Intent {
        fire: true,
        select: Some(Weapon::Shotgun),
        ..Intent::IDLE
    };
    for _ in 0..13 {
        session.tick(blast).unwrap();
    }
    let beams = |session: &Session| {
        session
            .events
            .iter()
            .filter_map(|event| match *event {
                Event::Beam { weapon, end, .. } => Some((weapon, end)),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let first = beams(&session);
    assert_eq!(first.len(), shooter_sim::weapons::PELLETS.len());
    assert!(first.iter().all(|(weapon, _)| *weapon == Weapon::Shotgun));
    // The next blast waits out the refire, then repeats the pattern exactly.
    for _ in 0..53 {
        session.tick(blast).unwrap();
        assert!(beams(&session).is_empty());
    }
    session.tick(blast).unwrap();
    assert_eq!(beams(&session), first);
}
