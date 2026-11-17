//! The deathmatch on physics-engine: bots fight and score deterministically, people take slots
//! from bots, lag-compensated hitscan, the round's end and restart, rocket self-damage, and the
//! wire round trip.
// The platform's floats only check results here; the simulation uses libm (clippy.toml).
#![allow(clippy::disallowed_methods)]

use shooter_sim::aim;
use shooter_sim::damage::PartDamage;
use shooter_sim::melee;
use shooter_sim::physics::Vec3i;
use shooter_sim::robot::parts::{CHASSIS_L, CHASSIS_S};
use shooter_sim::robot::{ARMORY, HEAVY, MARKSMAN, PYRO, SKIRMISHER, STANDARD};
use shooter_sim::scenarios;
use shooter_sim::session::{SELF_SPLASH_PERCENT, body_of, slot_of};
use shooter_sim::tour::run_legs;
use shooter_sim::weapons;
use shooter_sim::weapons::Delivery;
use shooter_sim::wire::{decode_input, decode_snapshot, encode_input, encode_snapshot};
use shooter_sim::{
    Arena, Assembly, Controller, Event, Input, Intent, MapId, Phase, Piece, Pilot, Rules, Session,
    Spawn, Weapon,
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
    // No frag limit: twelve bots reach it within the 90 s, and a fresh round clears the scores.
    let rules = Rules {
        frag_limit: 0,
        ..Rules::deathmatch(7)
    };
    let mut session = Session::with_rules(Arena::greybox(), rules).unwrap();
    assert_eq!(session.fighters.len(), 12);
    let (mut kills, mut shots, mut spawns, mut modules, mut drops) = (0, 0, 0, 0, 0);
    for _ in 0..90 * 60 {
        session.step().unwrap();
        for event in &session.events {
            match event {
                Event::Killed { .. } => kills += 1,
                Event::Fired { .. } => shots += 1,
                Event::Spawned { .. } => spawns += 1,
                Event::ModuleTaken { .. } => modules += 1,
                Event::ModuleDropped { .. } => drops += 1,
                _ => {}
            }
        }
    }
    println!(
        "{shots} shots, {kills} kills, {spawns} respawns, {modules} modules taken, {drops} dropped"
    );
    assert!(shots > 100, "{shots} shots");
    assert!(kills >= 3, "{kills} kills in 90 s");
    assert!(spawns >= kills - 12, "{spawns} respawns for {kills} kills");
    // Bots do not seek the module pads, but they cross them.
    assert!(modules > 0, "no module taken");
    // And a carrier dies with one, dropping it where it fell.
    assert!(drops > 0, "no module dropped");
    let deaths: i32 = session.fighters.iter().map(|f| f.deaths).sum();
    assert!(deaths > 0);
    for fighter in &session.fighters {
        assert!(matches!(fighter.pilot, Pilot::Bot(_)));
        assert!(fighter.chassis_health() <= CHASSIS_L.health);
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
    assert_eq!(
        session.fighters[0].parts,
        PartDamage::fresh(&session.fighters[0].build)
    );
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
        module_pads: Vec::new(),
        waypoints: Vec::new(),
        placements: Vec::new(),
        repair_anchors: Vec::new(),
    }
}

/// `slot` is built as `build` this life and the next (a fighter fires only what it carries).
fn equip(session: &mut Session, slot: usize, build: Assembly) {
    session.fighters[slot].configured = build;
    session.rebuild(u8::try_from(slot).unwrap(), build).unwrap();
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
    modded_duel_shot(seen_ticks_ago, STANDARD, Weapon::EnergyRifle).is_some()
}

/// The centre of `slot`'s root chassis, where the host places it.
fn chassis_of(session: &Session, slot: usize) -> [f32; 3] {
    let target = &session.fighters[slot];
    target
        .hitboxes()
        .get(PartDamage::root(&target.build).unwrap())
        .unwrap()
        .center
}

/// The yaw and pitch that put `from`'s reticle (the camera's centre ray, [`aim::at`]) on the
/// centre of `to`'s root chassis.
fn at_chassis(session: &Session, from: usize, to: usize) -> (u16, i16) {
    aim::at(
        Some(&session.world),
        session.fighters[from].eye(),
        chassis_of(session, to),
    )
}

/// [`duel_shot`] with A built as `build` and firing `weapon`: the damage B took, if hit.
fn modded_duel_shot(seen_ticks_ago: u32, build: Assembly, weapon: Weapon) -> Option<i32> {
    let mut session = Session::with_rules(duel_arena(), duel_rules()).unwrap();
    assert_eq!(session.join(1).unwrap(), Some(0));
    assert_eq!(session.join(2).unwrap(), Some(1));
    session.rebuild(0, build).unwrap();
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
    let take = Intent {
        select: Some(weapon),
        ..Intent::IDLE
    };
    for _ in 0..10 {
        let now = session.ticks();
        step(&mut session, take, Intent::IDLE, now);
    }
    // The reticle on B's chassis where it stands before it strafes.
    let (yaw, pitch) = at_chassis(&session, 0, 1);
    let aiming = Intent {
        yaw,
        pitch,
        ..Intent::IDLE
    };
    let strafe = Intent {
        strafe: 1,
        ..Intent::IDLE
    };
    for _ in 0..12 {
        let now = session.ticks();
        step(&mut session, aiming, strafe, now);
    }
    let fire = Intent {
        fire: true,
        ..aiming
    };
    let seen = session.ticks() - seen_ticks_ago;
    step(&mut session, fire, strafe, seen);
    assert!(session.events.iter().any(
        |event| matches!(event, Event::Fired { slot: 0, weapon: fired, .. } if *fired == weapon)
    ));
    session.events.iter().find_map(|event| match event {
        Event::Hit {
            victim: 1, damage, ..
        } => Some(*damage),
        _ => None,
    })
}

#[test]
fn hitscan_is_lag_compensated() {
    assert!(!duel_shot(0), "B has moved out of the line of fire");
    assert!(duel_shot(12), "A saw B in the line of fire 12 ticks ago");
}

/// Weapon mods change the host's lag-compensated hits, not whether they land: the same rewound
/// shot deals the modded damage.
#[test]
fn modded_hitscan_is_lag_compensated_for_the_modded_damage() {
    let shot = |build, weapon| {
        (
            modded_duel_shot(0, build, weapon),
            modded_duel_shot(12, build, weapon),
        )
    };
    assert_eq!(shot(STANDARD, Weapon::EnergyRifle), (None, Some(12)));
    // The rapid cycler's rifle bolts are lighter, the overcharged rail gun's beam heavier.
    assert_eq!(shot(SKIRMISHER, Weapon::EnergyRifle), (None, Some(9)));
    assert_eq!(shot(HEAVY, Weapon::RailGun), (None, Some(85)));
    assert_eq!(shot(MARKSMAN, Weapon::RailGun), (None, Some(106)));
    // The long barrel changes the range, not the damage.
    assert_eq!(shot(MARKSMAN, Weapon::EnergyRifle), (None, Some(12)));
}

/// A duel (A slot 0, B slot 1 12 m off) on `arena`, both people, A built as `build`, after
/// `settle` ticks of A holding `a` and B facing A.
fn duel_on(arena: Arena, build: Assembly) -> (Session, impl FnMut(&mut Session, Intent)) {
    let mut session = Session::with_rules(arena, duel_rules()).unwrap();
    assert_eq!(session.join(1).unwrap(), Some(0));
    assert_eq!(session.join(2).unwrap(), Some(1));
    equip(&mut session, 0, build);
    let mut sequence = 0;
    let step = move |session: &mut Session, intent: Intent| {
        sequence += 1;
        let view_tick = session.ticks();
        for (player, intent) in [
            (1, intent),
            (
                2,
                Intent {
                    yaw: 32_768,
                    ..Intent::IDLE
                },
            ),
        ] {
            session.queue(
                player,
                Input {
                    sequence,
                    intent,
                    view_tick,
                },
            );
        }
        session.step().unwrap();
    };
    (session, step)
}

/// A's rifle shot at `yaw` and `pitch` as the reticle has it ([`aim::aimed`]): the intended
/// target, and the shot's own ray from the muzzle.
fn reticle_of(session: &Session, yaw: u16, pitch: i16) -> (aim::Shot, aim::Sight) {
    let Delivery::Hitscan { range_units, .. } = Weapon::EnergyRifle.delivery() else {
        panic!("the rifle is hitscan");
    };
    let a = &session.fighters[0];
    let parts = session.fighters[1].hitboxes();
    let shot = aim::aimed(
        &session.world,
        a.eye(),
        a.muzzle(Weapon::EnergyRifle),
        yaw,
        pitch,
        body_of(0),
        |body| slot_of(body).is_none(),
        [(1, &parts)],
    );
    let actual = aim::sight(
        &session.world,
        shot.from,
        shot.direction(),
        range_units,
        body_of(0),
        |body| slot_of(body).is_none(),
        [(1, &parts)],
    );
    (shot, actual)
}

/// The rifle's beam this tick: where it left, where it ended and whether it struck.
fn beam_of(session: &Session) -> ([f32; 3], [f32; 3], bool) {
    session
        .events
        .iter()
        .find_map(|event| match *event {
            Event::Beam {
                from, end, struck, ..
            } => Some((from, end, struck)),
            _ => None,
        })
        .expect("the rifle fired")
}

fn hit_b(session: &Session) -> bool {
    session
        .events
        .iter()
        .any(|event| matches!(event, Event::Hit { victim: 1, .. }))
}

/// The third-person reticle is [`aim::aimed`]: the camera's centre ray picks the target, the
/// shot leaves the rifle's muzzle toward it. The hosted beam leaves exactly there and ends where
/// the reticle's shot ray does: on B, on a pillar, on the floor or at its range.
#[test]
fn the_reticle_shot_is_where_the_hosted_beam_goes() {
    let mut arena = duel_arena();
    // A pillar 1 m left of the line between A and B, halfway.
    arena.pieces.push(Piece::Block {
        center: Vec3i::new(-100, 150, 0),
        half: Vec3i::new(20, 150, 20),
    });
    let (mut session, mut step) = duel_on(arena, STANDARD);
    for _ in 0..20 {
        step(&mut session, Intent::IDLE);
    }
    // B's chassis, the pillar, the floor, nothing in range (the camera's aim at each).
    let chassis = at_chassis(&session, 0, 1);
    let eye = session.fighters[0].eye();
    let aims: [((u16, i16), bool); 4] = [
        (chassis, true),
        (
            aim::at(Some(&session.world), eye, [-100.0, 150.0, 0.0]),
            false,
        ),
        (aim::at(Some(&session.world), eye, [0.0, 0.0, 300.0]), false),
        ((0, 4_000), false),
    ];
    let mut met = Vec::new();
    for ((yaw, pitch), on_b) in aims {
        let aiming = Intent {
            yaw,
            pitch,
            ..Intent::IDLE
        };
        for _ in 0..20 {
            step(&mut session, aiming);
        }
        let (shot, actual) = reticle_of(&session, yaw, pitch);
        assert_eq!(shot.target.fighter.is_some(), on_b, "{yaw} {pitch}");
        assert_eq!(actual.fighter.is_some(), on_b, "{yaw} {pitch}");
        assert_eq!(shot.blocked(&actual), None, "{yaw} {pitch}");
        let muzzle = session.fighters[0].muzzle(Weapon::EnergyRifle).unwrap();
        assert_eq!(shot.from, muzzle.at);
        step(
            &mut session,
            Intent {
                fire: true,
                ..aiming
            },
        );
        let (from, end, struck) = beam_of(&session);
        assert_eq!(from, shot.from, "{yaw} {pitch}");
        assert_eq!(end, actual.end, "{yaw} {pitch}");
        assert_eq!(struck, actual.met(), "{yaw} {pitch}");
        assert_eq!(hit_b(&session), on_b, "{yaw} {pitch}");
        if !on_b {
            met.push(actual.scenery.hit.is_some());
        }
    }
    assert_eq!(met, [true, true, false], "the pillar, the floor, the sky");
}

/// Shots leave the mounted weapon's muzzle, not the eye: the rifle's beam, a rocket and a
/// shotgun blast's pellets start there, toward the camera's target.
#[test]
fn shots_and_projectiles_leave_the_mounted_muzzle() {
    let (mut session, mut step) = duel_on(duel_arena(), ARMORY);
    for weapon in [Weapon::EnergyRifle, Weapon::RocketLauncher, Weapon::Shotgun] {
        let (yaw, pitch) = at_chassis(&session, 0, 1);
        let aiming = Intent {
            yaw,
            pitch,
            select: Some(weapon),
            ..Intent::IDLE
        };
        for _ in 0..60 {
            step(&mut session, aiming);
        }
        let a = &session.fighters[0];
        let muzzle = a.muzzle(weapon).expect("the armory mounts it");
        let eye = a.eye();
        assert!(
            (0..3).any(|axis| (muzzle.at[axis] - eye[axis]).abs() > 5.0),
            "{weapon:?}: the muzzle {muzzle:?} is not the eye {eye:?}"
        );
        step(
            &mut session,
            Intent {
                fire: true,
                ..aiming
            },
        );
        let fired = session.events.iter().find_map(|event| match *event {
            Event::Fired {
                slot: 0,
                weapon: fired,
                from,
                direction,
            } if fired == weapon => Some((from, direction)),
            _ => None,
        });
        let (from, direction) = fired.expect("A fired");
        assert_eq!(from, muzzle.at, "{weapon:?}");
        // Toward B, not along the eye's line.
        assert!(direction[2] > 0.95, "{weapon:?} {direction:?}");
        match weapon {
            Weapon::RocketLauncher => {
                let rocket = session.rockets.last().expect("in flight").position_units();
                // One tick out of the muzzle, toward B.
                assert!(
                    (rocket[0] - from[0]).abs() < 5.0 && rocket[2] > from[2],
                    "{rocket:?} from {from:?}"
                );
            }
            Weapon::Shotgun => {
                let pellet = session.pellets.first().expect("in flight");
                let launched = pellet.from_units();
                assert!((0..3).all(|axis| (launched[axis] - from[axis]).abs() < 0.01));
            }
            _ => assert_eq!(beam_of(&session).0, from),
        }
        // Let everything land and B come back if it broke.
        for _ in 0..90 {
            step(&mut session, Intent::IDLE);
        }
    }
}

/// Cover right in front of the muzzle stops a shot the camera sees past: the reticle's target
/// is B, the shot (and its blocked marker) ends on the cover, B is never hit, whatever the
/// camera sees. Moving the aim does not bring the muzzle past it either; a rocket blows up on it.
#[test]
fn cover_beside_the_muzzle_blocks_what_the_camera_sees() {
    let (session, _) = duel_on(duel_arena(), ARMORY);
    let muzzles = [Weapon::EnergyRifle, Weapon::RocketLauncher]
        .map(|weapon| session.fighters[0].muzzle(weapon).unwrap().at);
    // A small block 30 cm in front of the rifle's and the rocket pod's muzzles on A's chest,
    // below and left of the camera's line (over the right shoulder).
    #[allow(clippy::cast_possible_truncation)]
    let (centre, half) = {
        let low = [0, 1, 2].map(|axis| muzzles[0][axis].min(muzzles[1][axis]).round() as i32);
        let high = [0, 1, 2].map(|axis| muzzles[0][axis].max(muzzles[1][axis]).round() as i32);
        (
            Vec3i::new((low[0] + high[0]) / 2, (low[1] + high[1]) / 2, high[2] + 35),
            Vec3i::new((high[0] - low[0]) / 2 + 10, (high[1] - low[1]) / 2 + 10, 5),
        )
    };
    let mut arena = duel_arena();
    arena.pieces.push(Piece::Block {
        center: centre,
        half,
    });
    let (mut session, mut step) = duel_on(arena, ARMORY);
    for _ in 0..20 {
        step(&mut session, Intent::IDLE);
    }
    let (yaw, pitch) = at_chassis(&session, 0, 1);
    let aiming = Intent {
        yaw,
        pitch,
        select: Some(Weapon::EnergyRifle),
        ..Intent::IDLE
    };
    for _ in 0..60 {
        step(&mut session, aiming);
    }
    let (shot, actual) = reticle_of(&session, yaw, pitch);
    assert!(shot.target.fighter.is_some(), "the camera sees B");
    assert!(actual.fighter.is_none(), "the muzzle does not");
    let blocked = shot.blocked(&actual).expect("a blocked marker");
    assert!(
        (blocked[2] - (centre.z - 5) as f32).abs() < 1.0,
        "{blocked:?}"
    );
    for _ in 0..5 {
        step(
            &mut session,
            Intent {
                fire: true,
                ..aiming
            },
        );
        let (_, end, struck) = beam_of(&session);
        assert!(struck);
        // On the cover where the reticle marked it (the robot sways a little between shots).
        assert!(
            (0..3).all(|axis| (end[axis] - blocked[axis]).abs() < 1.0),
            "{end:?}"
        );
        assert!(!hit_b(&session), "shot through cover");
        for _ in 0..12 {
            step(&mut session, aiming);
        }
    }
    // A rocket at B meets the cover at once.
    let rocket = Intent {
        select: Some(Weapon::RocketLauncher),
        ..aiming
    };
    for _ in 0..60 {
        step(&mut session, rocket);
    }
    step(
        &mut session,
        Intent {
            fire: true,
            ..rocket
        },
    );
    let mut exploded = None;
    for _ in 0..3 {
        exploded = exploded.or_else(|| {
            session.events.iter().find_map(|event| match *event {
                Event::Exploded { at } => Some(at),
                _ => None,
            })
        });
        step(&mut session, rocket);
    }
    let at = exploded.expect("the rocket blew up on the cover");
    assert!(at[2] < (centre.z as f32), "{at:?}");
    assert_eq!(
        session.fighters[1].parts,
        PartDamage::fresh(&session.fighters[1].build)
    );
}

/// Very close, the camera's target is still met from the muzzle: B a metre and a half off.
#[test]
fn a_very_close_target_is_hit_from_the_muzzle() {
    let arena = Arena {
        spawn: Vec3i::new(0, 0, -75),
        spawns: vec![Spawn {
            feet: Vec3i::new(0, 0, 75),
            yaw: 32_768,
        }],
        ..duel_arena()
    };
    let (mut session, mut step) = duel_on(arena, STANDARD);
    for _ in 0..20 {
        step(&mut session, Intent::IDLE);
    }
    let (yaw, pitch) = at_chassis(&session, 0, 1);
    let aiming = Intent {
        yaw,
        pitch,
        ..Intent::IDLE
    };
    for _ in 0..20 {
        step(&mut session, aiming);
    }
    let (shot, actual) = reticle_of(&session, yaw, pitch);
    assert!(shot.target.fighter.is_some() && actual.fighter.is_some());
    step(
        &mut session,
        Intent {
            fire: true,
            ..aiming
        },
    );
    assert!(hit_b(&session));
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
                attacker: Some(0),
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
fn the_blade_strikes_the_nearest_part_whatever_gun_is_held() {
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
    // A carries the rockets it holds (and the blade).
    equip(&mut session, 0, HEAVY);
    session.step().unwrap();
    let (mut strikes, mut lost) = (Vec::new(), Vec::new());
    for sequence in 1..=120 {
        let now = session.ticks();
        // A follows B up after each shove, stopping 1.5 m short.
        let gap = session.fighters[1].center()[2] - session.fighters[0].center()[2];
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
        for event in &session.events {
            match *event {
                Event::Hit {
                    victim: 1,
                    attacker: Some(0),
                    part,
                    damage,
                    ..
                } => strikes.push((sequence, part, damage)),
                Event::Detached {
                    slot: 1,
                    part,
                    destroyed,
                } => lost.push((sequence, part, destroyed)),
                _ => {}
            }
        }
    }
    // Swings at ticks 0, 30, 60 and 90, each on one part: B's part nearest A's eye.
    let ticks: Vec<u32> = strikes.iter().map(|(tick, _, _)| *tick).collect();
    assert_eq!(ticks, [1, 31, 61, 91]);
    assert!(
        strikes
            .iter()
            .all(|(_, _, damage)| *damage <= melee::DAMAGE)
    );
    // A light part takes one swing: it is destroyed and comes off.
    assert!(lost.iter().any(|(_, _, destroyed)| *destroyed), "{lost:?}");
    assert!(
        session.fighters[1].alive(),
        "four swings do not break the root chassis"
    );
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
    equip(&mut session, 0, HEAVY);
    let mut kills = 0;
    let mut over = false;
    // A aims at B's root chassis: four rail beams break it, the kill.
    for sequence in 1..4_000 {
        let now = session.ticks();
        let (yaw, pitch) = if session.fighters[1].alive() {
            at_chassis(&session, 0, 1)
        } else {
            (0, 0)
        };
        let rail = Intent {
            fire: true,
            yaw,
            pitch,
            select: Some(Weapon::RailGun),
            ..Intent::IDLE
        };
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
    equip(&mut session, 0, HEAVY);
    run_legs(&mut session, scenario.legs).unwrap();
    let local = session.local();
    assert!(local.alive());
    // The blast at its feet hurts every part within reach, each by at most half the splash.
    let fresh = PartDamage::fresh(&local.build);
    let lost: Vec<i32> = fresh
        .health()
        .iter()
        .zip(local.parts.health())
        .map(|(whole, now)| i32::from(*whole) - i32::from(*now))
        .collect();
    let most = lost.iter().copied().max().unwrap_or(0);
    assert!(
        (20..=100 * SELF_SPLASH_PERCENT / 100).contains(&most),
        "lost {lost:?}"
    );
    assert!(local.chassis_health() > 0);
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
            let me = snapshot.of_player(7).unwrap().clone();
            if predicting {
                predictor.tick(leg.intent).unwrap();
                let predicted = predictor.local();
                let center = predicted.center();
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
                    .place(0, me.robot.as_ref().unwrap(), me.motion, me.held, me.hook)
                    .unwrap();
                predicting = true;
            }
        }
    }
    assert!(predicting, "the host set the hook");
    let me = host.snapshot().of_player(7).unwrap().clone();
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
        grapple: true,
        select: Some(Weapon::RocketLauncher),
        edit: true,
        repair: Some(shooter_sim::repair::Order::Start(
            shooter_sim::robot::PartId(5),
        )),
        motors: {
            let mut motors = shooter_sim::bindings::Motors::REST;
            motors.gait = shooter_sim::mechanics::GaitChoice::Unison;
            motors.push(
                shooter_sim::robot::PartId(6),
                shooter_sim::mechanics::Command::Stride(-55),
            );
            motors.push(
                shooter_sim::robot::PartId(5),
                shooter_sim::mechanics::Command::Angle(-9_000),
            );
            motors
        },
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
    equip(&mut session, slot, HEAVY);
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
    equip(&mut session, 0, ARMORY);
    let (mut burnt, mut first) = (0, None);
    // A walks up to 3 m short of B, burning the parts nearest it (the front) and the root chassis
    // among them, until it breaks.
    for sequence in 1..1_200 {
        if session.fighters[1].deaths > 0 {
            break;
        }
        let now = session.ticks();
        let gap = session.fighters[1].center()[2] - session.fighters[0].center()[2];
        let flame = Intent {
            forward: i8::from(gap > 300.0),
            fire: true,
            select: Some(Weapon::Flamethrower),
            ..Intent::IDLE
        };
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
                    // From the flame's muzzle, where it leaves.
                    let muzzle = session.fighters[0].muzzle(Weapon::Flamethrower).unwrap();
                    let gap = session.fighters[1].center()[2] - muzzle.at[2];
                    first.get_or_insert(gap);
                }
                Event::Hit { victim: 0, .. } => panic!("the flame burnt its owner"),
                _ => {}
            }
        }
    }
    // The flame reaches B only once A's muzzle has come within its 6 m (and the reach of B's parts
    // toward A: its guns, about 45 units ahead of its centre).
    let reach = (weapons::FLAME_REACH_UNITS + weapons::FLAME_RADIUS_UNITS + 45) as f32;
    assert!(
        first.is_some_and(|gap| gap <= reach && gap > reach - 60.0),
        "first burn {first:?} apart"
    );
    assert!(burnt >= i32::from(CHASSIS_S.health), "burnt {burnt}");
    assert_eq!(
        session.fighters[0].parts,
        PartDamage::fresh(&session.fighters[0].build)
    );
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
    equip(&mut session, 0, ARMORY);
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

/// The pyro loadout (shooter#96) plays its laser and its flamethrower in a match: it holds the
/// laser from the start (not the grapple, though that comes first in the rotation) and burns B
/// with it, then takes up the flamethrower and burns B with that.
#[test]
fn the_pyro_fires_its_laser_and_its_flamethrower() {
    // B stands 3 m in front of A: in the laser's reach and the flame's.
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
    equip(&mut session, 0, PYRO);
    assert_eq!(session.fighters[0].arsenal.held, Weapon::Laser);
    let mut sequence = 0;
    // What A's hits on B dealt, while firing `weapon` for `ticks`.
    let mut fire = |session: &mut Session, weapon: Weapon, ticks: u32| {
        let (mut fired, mut damage) = (false, 0);
        for _ in 0..ticks {
            sequence += 1;
            let now = session.ticks();
            let (yaw, pitch) = at_chassis(session, 0, 1);
            let intent = Intent {
                yaw,
                pitch,
                fire: session.fighters[0].arsenal.held == weapon,
                select: Some(weapon),
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
            for event in &session.events {
                match *event {
                    Event::Fired {
                        slot: 0,
                        weapon: shot,
                        ..
                    } if shot == weapon => fired = true,
                    Event::Hit {
                        victim: 1,
                        attacker: Some(0),
                        damage: dealt,
                        ..
                    } => damage += dealt,
                    _ => {}
                }
            }
        }
        (fired, damage)
    };
    let (lasered, laser_damage) = fire(&mut session, Weapon::Laser, 30);
    assert!(lasered && laser_damage > 0, "laser {laser_damage}");
    let (burnt, flame_damage) = fire(&mut session, Weapon::Flamethrower, 45);
    assert!(burnt && flame_damage > 0, "flame {flame_damage}");
    assert_eq!(session.fighters[0].deaths, 0);
}

/// Twelve bots on the greybox yard (the default map): they fight and score across its 76 m, and
/// none is stranded: every bot covers ground, and a living bot that is not shooting at anyone
/// never stays within 1 m of one spot for long.
#[test]
fn bots_keep_moving_and_scoring_on_the_yard() {
    bots_keep_moving_and_scoring_on(MapId::GreyboxYard, 2_000.0);
}

/// The same on the assembly works (shooter#52), a smaller map of three levels: no bot is
/// stranded on its decks, in its bays or behind its cover.
#[test]
fn bots_keep_moving_and_scoring_on_the_assembly_works() {
    bots_keep_moving_and_scoring_on(MapId::AssemblyWorks, 1_500.0);
}

/// The same on the proving cell (shooter#53), the small map: twelve bots crowd its 32 m, and
/// none is stranded behind its booths' screens, in its test bays or on its stage and galleries.
#[test]
fn bots_keep_moving_and_scoring_on_the_proving_cell() {
    bots_keep_moving_and_scoring_on(MapId::ProvingCell, 1_500.0);
}

/// The same on the drydock (shooter#55), the large map: twelve bots find each other across its
/// 80 m, and none is stranded on its hulls, on its gantries or in its outer lanes.
#[test]
fn bots_keep_moving_and_scoring_on_the_drydock() {
    bots_keep_moving_and_scoring_on(MapId::Drydock, 2_000.0);
}

/// Twelve bots for 90 s on `map`: plenty of shots, at least ten kills, every bot at least
/// `roams` units from where it started and none idle (alive, mobile, not firing, within 1 m of
/// one spot) for 5 s.
fn bots_keep_moving_and_scoring_on(map: MapId, roams: f32) {
    let rules = Rules {
        frag_limit: 0,
        ..Rules::deathmatch(7)
    };
    let mut session = Session::on_map(map, rules).unwrap();
    let slots = session.fighters.len();
    let (mut kills, mut shots) = (0, 0);
    // Per slot: where it last moved 1 m from (or last fired) and when, its longest idle stay
    // anywhere, and the farthest it got from where it started.
    let mut anchor: Vec<Option<([f32; 3], u32)>> = vec![None; slots];
    let mut idle = vec![0u32; slots];
    let mut start: Vec<Option<[f32; 3]>> = vec![None; slots];
    let mut farthest = vec![0.0f32; slots];
    for _ in 0..90 * 60 {
        session.step().unwrap();
        let tick = session.ticks();
        for event in &session.events {
            match *event {
                Event::Killed { .. } => kills += 1,
                Event::Fired { slot, .. } => {
                    shots += 1;
                    anchor[usize::from(slot)] = None;
                }
                _ => {}
            }
        }
        for (slot, fighter) in session.fighters.iter().enumerate() {
            // The dead, and robots whose legs are shot off ([`PartDamage::mobile`]), stand.
            if !fighter.alive() || !fighter.parts.mobile(&fighter.build) {
                anchor[slot] = None;
                continue;
            }
            let feet = fighter.feet();
            let from = *start[slot].get_or_insert(feet);
            farthest[slot] = farthest[slot].max((feet[0] - from[0]).hypot(feet[2] - from[2]));
            match anchor[slot] {
                Some((at, since)) if (feet[0] - at[0]).hypot(feet[2] - at[2]) < 100.0 => {
                    idle[slot] = idle[slot].max(tick - since);
                }
                _ => anchor[slot] = Some((feet, tick)),
            }
        }
    }
    println!("{map}: {shots} shots, {kills} kills; longest idle {idle:?}; farthest {farthest:?}");
    assert!(shots > 100, "{shots} shots");
    assert!(kills >= 10, "{kills} kills in 90 s");
    for slot in 0..slots {
        assert!(
            farthest[slot] > roams,
            "{map}: slot {slot} got {} units",
            farthest[slot]
        );
        assert!(
            idle[slot] < 5 * 60,
            "slot {slot} idled {} ticks",
            idle[slot]
        );
    }
}

#[test]
fn a_bot_match_on_the_yard_replays_exactly() {
    let play = |seed| {
        let mut session = Session::on_map(MapId::GreyboxYard, Rules::deathmatch(seed)).unwrap();
        for _ in 0..1_200 {
            session.step().unwrap();
        }
        encode_snapshot(&session.snapshot())
    };
    assert_eq!(play(3), play(3));
    assert_ne!(play(3), play(4));
}

/// A bot match on the assembly works replays exactly from its seed, and another seed plays out
/// differently.
#[test]
fn a_bot_match_on_the_assembly_works_replays_exactly() {
    let play = |seed| {
        let mut session = Session::on_map(MapId::AssemblyWorks, Rules::deathmatch(seed)).unwrap();
        for _ in 0..1_200 {
            session.step().unwrap();
        }
        encode_snapshot(&session.snapshot())
    };
    assert_eq!(play(3), play(3));
    assert_ne!(play(3), play(4));
}

/// A bot match on the proving cell replays exactly from its seed, at its recommended six slots
/// and at the default twelve, and another seed plays out differently.
#[test]
fn a_bot_match_on_the_proving_cell_replays_exactly() {
    let play = |seed, slots| {
        let rules = Rules {
            slots,
            ..Rules::deathmatch(seed)
        };
        let mut session = Session::on_map(MapId::ProvingCell, rules).unwrap();
        for _ in 0..1_200 {
            session.step().unwrap();
        }
        encode_snapshot(&session.snapshot())
    };
    for slots in [6, 12] {
        assert_eq!(play(3, slots), play(3, slots));
        assert_ne!(play(3, slots), play(4, slots));
    }
}

/// One tick: A (person 1) rails B's root chassis while B lives, and B (person 2) holds `b`,
/// unless `b` is `None` (B sends nothing, a lost connection).
fn rail_tick(session: &mut Session, b: Option<Intent>) {
    let now = session.ticks();
    let (yaw, pitch) = if session.fighters[1].alive() {
        at_chassis(session, 0, 1)
    } else {
        (0, 0)
    };
    let rail = Intent {
        fire: session.fighters[1].alive(),
        yaw,
        pitch,
        select: Some(Weapon::RailGun),
        ..Intent::IDLE
    };
    for (id, intent) in [(1, Some(rail)), (2, b)] {
        let Some(intent) = intent else {
            continue;
        };
        let slot = id as usize - 1;
        let sequence = session.fighters[slot].ack + 1;
        session.queue(
            id,
            Input {
                sequence,
                intent,
                view_tick: now,
            },
        );
    }
    session.step().unwrap();
}

const EDITING: Intent = Intent {
    edit: true,
    ..Intent::IDLE
};

/// A duel in which A (heavy) has railed B dead, B holding Edit Robot all along (alive, it does
/// nothing).
fn edited_duel(rules: Rules) -> Session {
    let mut session = Session::with_rules(duel_arena(), rules).unwrap();
    session.join(1).unwrap();
    session.join(2).unwrap();
    equip(&mut session, 0, HEAVY);
    for _ in 0..2_000 {
        rail_tick(&mut session, Some(EDITING));
        if !session.fighters[1].alive() {
            return session;
        }
        assert_eq!(session.fighters[1].build, STANDARD, "alive: never rebuilt");
    }
    panic!("B never died");
}

/// Edit Robot after death suspends the respawn with no time limit (Atlas D2.i70.11); Ready (the
/// flag released) brings the robot back as the build configured meanwhile, at once.
#[test]
fn edit_robot_holds_the_respawn_until_ready() {
    let mut session = edited_duel(duel_rules());
    assert!(session.configure(2, MARKSMAN));
    // Ten times the respawn countdown: still in the builder.
    for _ in 0..10 * duel_rules().respawn_ticks {
        rail_tick(&mut session, Some(EDITING));
        assert!(!session.fighters[1].alive());
    }
    rail_tick(&mut session, Some(Intent::IDLE));
    let b = &session.fighters[1];
    assert!(b.alive());
    assert_eq!((b.build, b.configured), (MARKSMAN, MARKSMAN));
    assert!(
        session
            .events
            .iter()
            .any(|event| matches!(event, Event::Spawned { slot: 1, .. }))
    );
}

/// Without Edit Robot the dead come back after the normal countdown, as their last build.
#[test]
fn without_edit_robot_the_respawn_is_automatic() {
    let mut session = edited_duel(duel_rules());
    for _ in 0..duel_rules().respawn_ticks + 1 {
        rail_tick(&mut session, Some(Intent::IDLE));
    }
    let b = &session.fighters[1];
    assert!(b.alive());
    assert_eq!(b.build, STANDARD);
}

/// A person in the builder whose inputs stop (a lost connection) lets go of it: the robot comes
/// back after the countdown and the input hold.
#[test]
fn a_silent_editor_comes_back() {
    use shooter_sim::session::HOLD_INPUT_TICKS;
    let mut session = edited_duel(duel_rules());
    for _ in 0..duel_rules().respawn_ticks.max(HOLD_INPUT_TICKS) + 1 {
        rail_tick(&mut session, None);
    }
    assert!(session.fighters[1].alive());
}

/// A round's start does not pull a person out of the builder: the robot waits until Ready.
#[test]
fn a_new_round_waits_for_ready() {
    let rules = Rules {
        frag_limit: 1,
        ..duel_rules()
    };
    let mut session = edited_duel(rules);
    let Phase::Intermission { restarts_at, .. } = session.phase else {
        panic!("{:?}", session.phase);
    };
    while session.ticks() <= restarts_at + 30 {
        rail_tick(&mut session, Some(EDITING));
    }
    assert!(matches!(session.phase, Phase::Playing { .. }));
    assert!(session.fighters[0].alive());
    assert!(!session.fighters[1].alive());
    rail_tick(&mut session, Some(Intent::IDLE));
    assert!(session.fighters[1].alive());
}

/// A person who has not played yet is still at the builder before the match: a build it
/// configures applies at once. From its first input on, the robot alive keeps its build.
#[test]
fn a_build_configured_before_playing_applies_at_once() {
    let mut session = Session::with_rules(duel_arena(), duel_rules()).unwrap();
    session.join(1).unwrap();
    session.join(2).unwrap();
    assert!(session.configure(2, PYRO));
    let b = &session.fighters[1];
    assert_eq!((b.build, b.configured), (PYRO, PYRO));
    assert_eq!(b.arsenal.held, Weapon::Laser);
    assert_eq!(b.energy, b.pool().capacity);
    rail_tick(&mut session, Some(Intent::IDLE));
    assert!(session.configure(2, HEAVY));
    let b = &session.fighters[1];
    assert_eq!((b.build, b.configured), (PYRO, HEAVY));
}

/// A bot match on the drydock replays exactly from its seed, at its fewest recommended eight
/// slots and at the default twelve, and another seed plays out differently.
#[test]
fn a_bot_match_on_the_drydock_replays_exactly() {
    let play = |seed, slots| {
        let rules = Rules {
            slots,
            ..Rules::deathmatch(seed)
        };
        let mut session = Session::on_map(MapId::Drydock, rules).unwrap();
        for _ in 0..1_200 {
            session.step().unwrap();
        }
        encode_snapshot(&session.snapshot())
    };
    for slots in [8, 12] {
        assert_eq!(play(3, slots), play(3, slots));
        assert_ne!(play(3, slots), play(4, slots));
    }
}
