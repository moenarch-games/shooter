//! The energy weapons on physics-engine: hitscan damage, walls stopping bolts, rocket splash and
//! rocket jumping, shotgun blasts and their pellets' falloff, the laser and the flamethrower's
//! ignition, and the greybox's dummies.

use shooter_sim::dummies::DUMMY_HEALTH;
use shooter_sim::player::PLAYER_HALF_EXTENTS;
use shooter_sim::scenarios::{self, Scenario};
use shooter_sim::sim_foundation::physics::Vec3i;
use shooter_sim::tour::{Sample, end_of, run_legs};
use shooter_sim::weapons::{self, FLAME_IGNITION_TICKS, PELLETS};
use shooter_sim::{Arena, Event, Intent, Piece, Post, Session, Weapon};

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
    let fired = |leg: &str| {
        samples
            .iter()
            .filter(|s| s.leg == leg && s.fired.is_some())
            .count()
    };
    // The flame ignites for a quarter of a second, then deals three a tick for half a second;
    // the pillar keeps the west dummy unburnt.
    assert_eq!(fired("flame the south"), 45);
    let ignition = FLAME_IGNITION_TICKS as usize;
    let south: Vec<i32> = samples
        .iter()
        .filter(|s| s.leg == "flame the south")
        .map(|s| s.damage)
        .collect();
    assert!(
        south[..ignition].iter().all(|damage| *damage == 0),
        "{south:?}"
    );
    assert!(
        south[ignition..].iter().all(|damage| *damage == 3),
        "{south:?}"
    );
    assert_eq!(damage_in(&samples, "flame the pillar"), 0);
    assert_eq!(session.dummies[2].health, DUMMY_HEALTH);
    // Let go, it ignites from the start again.
    assert_eq!(fired("ignite again"), 15);
    assert_eq!(damage_in(&samples, "ignite again"), 0);
    // Four metres out, the aim and the inner ring hit at full damage within the pellets' first
    // tick; the outer ring's pellets fly past.
    let blast = damage_in(&samples, "blast the south");
    assert_eq!(blast, 6 * 9, "blast");
    assert_eq!(fired("blast the south"), 1);
    // The laser burns every tick it is held.
    assert_eq!(fired("laser the east"), 30);
    assert_eq!(damage_in(&samples, "laser the east"), 30 * 2);
    assert_eq!(session.dummies[1].health, DUMMY_HEALTH - 60);
    assert_eq!(session.dummies[0].health, DUMMY_HEALTH - 90 - blast);
}

/// What a tick showed of the shotgun's pellets: where each one flew and where each struck.
fn pellet_tick(session: &Session) -> (Vec<[i32; 3]>, Vec<[f32; 3]>) {
    let flying = session
        .pellets
        .iter()
        .map(|pellet| pellet.position)
        .collect();
    let struck = session
        .events
        .iter()
        .filter_map(|event| match *event {
            Event::Pellet { at, .. } => Some(at),
            _ => None,
        })
        .collect();
    (flying, struck)
}

#[test]
fn a_shotgun_blast_is_a_fixed_pattern_of_pellets() {
    let mut session = Session::new((scenarios::CLOSE_RANGE.arena)()).unwrap();
    let blast = Intent {
        fire: true,
        select: Some(Weapon::Shotgun),
        ..Intent::IDLE
    };
    for _ in 0..12 {
        session.tick(blast).unwrap();
        assert!(session.pellets.is_empty());
    }
    let refire = Weapon::Shotgun.refire_ticks();
    let flight = |session: &mut Session| {
        (0..refire)
            .map(|_| {
                session.tick(blast).unwrap();
                pellet_tick(session)
            })
            .collect::<Vec<_>>()
    };
    let first = flight(&mut session);
    // Every pellet is launched (and the six that reach the dummy strike it) on the blast's tick.
    let (flying, struck) = &first[0];
    assert_eq!(flying.len() + struck.len(), PELLETS.len());
    assert_eq!(struck.len(), 6);
    // The rest are spent within the pellets' range, long before the next blast...
    assert!(
        first[10..]
            .iter()
            .all(|(flying, struck)| flying.is_empty() && struck.is_empty())
    );
    // ...which repeats the pattern exactly.
    assert_eq!(flight(&mut session), first);
}

/// A dummy `metres` south of the close range's shooter and nothing else in the way.
fn lone_dummy(metres: i32) -> Arena {
    Arena {
        pieces: Vec::new(),
        posts: vec![Post {
            feet: Vec3i::new(0, 0, -600 + metres * 100),
            patrol_units: 0,
        }],
        ..(scenarios::CLOSE_RANGE.arena)()
    }
}

/// The damage of every pellet of one blast at `arena`'s dummies.
fn blast_hits(arena: Arena) -> Vec<i32> {
    let mut session = Session::new(arena).unwrap();
    let blast = Intent {
        fire: true,
        select: Some(Weapon::Shotgun),
        ..Intent::IDLE
    };
    let mut hits = Vec::new();
    for tick in 0..40 {
        session
            .tick(if tick < 13 { blast } else { Intent::IDLE })
            .unwrap();
        hits.extend(session.events.iter().filter_map(|event| match *event {
            Event::Damaged { damage, .. } => Some(damage),
            _ => None,
        }));
    }
    hits
}

#[test]
fn far_pellets_hurt_a_little_less() {
    // Point-blank, the whole pattern lands at full damage.
    assert_eq!(blast_hits(lone_dummy(1)), vec![9; PELLETS.len()]);
    // 15 m out, only the aimed pellet hits, slowed by drag: still most of its damage.
    let far = blast_hits(lone_dummy(15));
    assert_eq!(far.len(), 1, "{far:?}");
    assert!((6..9).contains(&far[0]), "{far:?}");
    // Past the pellets' range, nothing.
    let range = weapons::PELLET_RANGE_UNITS / 100;
    assert!(blast_hits(lone_dummy(range + 2)).is_empty());
}

#[test]
fn pellets_never_tunnel_through_a_thin_wall() {
    // A wall 4 cm thick 10 m south: thinner than a pellet's 4 m stride.
    let mut arena = lone_dummy(12);
    arena.pieces = vec![Piece::Block {
        center: Vec3i::new(0, 150, 400),
        half: Vec3i::new(400, 150, 2),
    }];
    assert!(blast_hits(arena.clone()).is_empty());
    let mut session = Session::new(arena).unwrap();
    let mut struck = 0;
    for tick in 0..40 {
        session
            .tick(Intent {
                fire: tick < 13,
                select: Some(Weapon::Shotgun),
                ..Intent::IDLE
            })
            .unwrap();
        assert!(
            session
                .pellets
                .iter()
                .all(|pellet| pellet.position_units()[2] < 398.0)
        );
        struck += pellet_tick(&session).1.len();
    }
    assert_eq!(struck, PELLETS.len());
}
