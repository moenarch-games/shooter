//! Per-part damage on the host (`shooter_sim::damage`, Atlas D2.i70.2, D2.i70.8): which part a
//! shot from the front or the side strikes, splash on every part in reach once, armor plates
//! and penetration, several parts lost at once and what they take with them (weapons that no
//! longer fire, a robot that no longer walks), the core as the only kill and respawn restoring
//! the build, a destroyed battery's energy and blast, the wire and a predictor following the
//! host, and bot matches that lose parts and replay exactly.

use shooter_sim::damage::{BATTERY_BLAST_DAMAGE, PartDamage};
use shooter_sim::energy::Pool;
use shooter_sim::physics::Vec3i;
use shooter_sim::robot::parts::{ARMOR_PLATE, CHASSIS_S};
use shooter_sim::robot::{BETA, Face, HEAVY, PartId, RawMount, RawPart, STANDARD};
use shooter_sim::trig;
use shooter_sim::weapons::{SPLASH_DAMAGE, Weapon};
use shooter_sim::wire::{decode_snapshot, encode_snapshot};
use shooter_sim::{Arena, Assembly, Event, Input, Intent, Rules, Session, Spawn};

const A: u32 = 1;
const B: u32 = 2;

/// A (slot 0) faces B (slot 1) 12 m apart on an open floor; B respawns after half a second.
fn duel() -> Session {
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
    session
}

/// `slot` is built as `build`, whole and fully charged, this life.
fn rebuild(session: &mut Session, slot: usize, build: Assembly) {
    let fighter = &mut session.fighters[slot];
    fighter.build = build;
    fighter.configured = build;
    fighter.parts = PartDamage::fresh(&build);
    fighter.energy = fighter.pool().capacity;
}

/// Where `slot`'s part `part` is now (units).
fn part_center(session: &Session, slot: usize, part: PartId) -> [f32; 3] {
    let fighter = &session.fighters[slot];
    let index = fighter
        .build
        .parts()
        .iter()
        .position(|p| p.id == part)
        .unwrap();
    let parts = fighter.hitboxes(fighter.player.center(&session.world), fighter.intent.yaw);
    parts.get(index).expect("a connected part").center
}

/// The yaw and pitch from `slot`'s eye to `point`.
#[allow(clippy::cast_possible_truncation)]
fn aim_at(session: &Session, slot: usize, point: [f32; 3]) -> (u16, i16) {
    let eye = session.fighters[slot].player.eye(&session.world);
    let d = [0, 1, 2].map(|axis| point[axis] - eye[axis]);
    let yaw = trig::yaw_from_vector(d[0].round() as i32, d[2].round() as i32).unwrap();
    let run = d[0].hypot(d[2]).round() as i32;
    let pitch = trig::yaw_from_vector(d[1].round() as i32, run).unwrap() as i16;
    (yaw, pitch)
}

/// One tick: A holds `a`, B holds `b` (both seeing the match as it is).
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

/// B standing still facing `yaw`.
fn facing(yaw: u16) -> Intent {
    Intent {
        yaw,
        ..Intent::IDLE
    }
}

/// A takes up `weapon` (B faces `b_yaw`), then fires it once at `point` of B (recomputed as
/// it fires): the tick's events.
fn shoot(
    session: &mut Session,
    weapon: Weapon,
    b_yaw: u16,
    point: impl Fn(&Session) -> [f32; 3],
) -> Vec<Event> {
    let take = Intent {
        select: Some(weapon),
        ..Intent::IDLE
    };
    for _ in 0..120 {
        step(session, take, facing(b_yaw));
        let held = session.fighters[0].arsenal.held == weapon;
        if held && session.fighters[0].energy == session.fighters[0].pool().capacity {
            break;
        }
    }
    for _ in 0..120 {
        let (yaw, pitch) = aim_at(session, 0, point(session));
        let fire = Intent {
            yaw,
            pitch,
            fire: true,
            select: Some(weapon),
            ..Intent::IDLE
        };
        step(session, fire, facing(b_yaw));
        if session
            .events
            .iter()
            .any(|event| matches!(event, Event::Fired { slot: 0, .. }))
        {
            return session.events.clone();
        }
    }
    panic!("A never fired");
}

/// The parts of B struck by A in `events`, with the damage each took.
fn struck(events: &[Event]) -> Vec<(PartId, i32)> {
    events
        .iter()
        .filter_map(|event| match *event {
            Event::Hit {
                victim: 1,
                attacker: Some(0),
                part,
                damage,
                ..
            } => Some((part, damage)),
            _ => None,
        })
        .collect()
}

/// B's parts that came off in `events`, and whether each was destroyed.
fn detached(events: &[Event]) -> Vec<(PartId, bool)> {
    events
        .iter()
        .filter_map(|event| match *event {
            Event::Detached {
                slot: 1,
                part,
                destroyed,
            } => Some((part, destroyed)),
            _ => None,
        })
        .collect()
}

/// B's chassis middle at the height of its arms (150 units above its feet).
fn arm_height(session: &Session) -> [f32; 3] {
    let center = session.fighters[1].player.center(&session.world);
    [center[0], center[1] - 90.0 + 150.0, center[2]]
}

#[test]
fn front_and_side_shots_strike_the_parts_they_meet() {
    // From the front, at the chassis' middle: the chassis, the whole bolt.
    let mut session = duel();
    let events = shoot(&mut session, Weapon::EnergyRifle, 32_768, |session| {
        part_center(session, 1, PartId(0))
    });
    assert_eq!(struck(&events), [(PartId(0), 12)]);
    // From the front at arm height the chest's shotgun is in the way.
    let mut session = duel();
    let events = shoot(&mut session, Weapon::EnergyRifle, 32_768, |session| {
        let mut at = arm_height(session);
        at[0] -= 10.0;
        at
    });
    assert_eq!(struck(&events), [(PartId(10), 12)]);
    // From B's left side at arm height: the rifle at the tip of the left arm, not the chassis
    // behind it.
    let mut session = duel();
    let events = shoot(&mut session, Weapon::EnergyRifle, 16_384, arm_height);
    assert_eq!(struck(&events), [(PartId(7), 12)]);
    // From its right side: the blade on the right arm's tip.
    let mut session = duel();
    let events = shoot(&mut session, Weapon::EnergyRifle, 49_152, arm_height);
    assert_eq!(struck(&events), [(PartId(8), 12)]);
}

#[test]
fn a_rocket_splash_strikes_each_part_in_reach_once() {
    let mut session = duel();
    let events = shoot(&mut session, Weapon::RocketLauncher, 32_768, |session| {
        let center = session.fighters[1].player.center(&session.world);
        [center[0], 0.0, center[2] - 60.0]
    });
    assert!(struck(&events).is_empty(), "the rocket is still flying");
    let mut hits = Vec::new();
    for _ in 0..120 {
        step(&mut session, Intent::IDLE, facing(32_768));
        hits = struck(&session.events);
        if !hits.is_empty() {
            break;
        }
    }
    let mut parts: Vec<PartId> = hits.iter().map(|(part, _)| *part).collect();
    let count = parts.len();
    parts.dedup();
    assert_eq!(parts.len(), count, "each part once: {hits:?}");
    assert!(count >= 4, "{hits:?}");
    assert!(
        hits.iter()
            .all(|(_, damage)| (1..=SPLASH_DAMAGE).contains(damage))
    );
    // The legs, nearest the blast at B's feet, take the most.
    let leg = hits.iter().find(|(part, _)| *part == PartId(5)).unwrap().1;
    assert!(hits.iter().all(|(_, damage)| *damage <= leg), "{hits:?}");
}

/// The standard build with an armor plate (part 11) over the front of its chassis, between the
/// chest's shotgun and the legs.
fn plated() -> Assembly {
    let mut candidate = STANDARD.candidate();
    candidate.parts.push(RawPart {
        id: 11,
        kind: ARMOR_PLATE.id.0,
        mount: Some(RawMount {
            parent: 0,
            socket: CHASSIS_S.socket(Face::PosX, 0, 1).unwrap().0,
            ..RawMount::default()
        }),
        modification: None,
    });
    BETA.validate(&candidate).unwrap()
}

#[test]
fn armor_plates_stop_strikes_and_penetrating_beams_pass_a_share() {
    let plate = PartId(11);
    let chest = |session: &Session| part_center(session, 1, PartId(0));
    // The rifle pierces 20 %: the plate takes 10 of 12, the chassis behind it 2.
    let mut session = duel();
    rebuild(&mut session, 1, plated());
    let events = shoot(&mut session, Weapon::EnergyRifle, 32_768, chest);
    assert_eq!(struck(&events), [(plate, 10), (PartId(0), 2)]);
    // The rail gun pierces half: 43 and 42.
    let mut session = duel();
    rebuild(&mut session, 1, plated());
    let events = shoot(&mut session, Weapon::RailGun, 32_768, chest);
    assert_eq!(struck(&events), [(plate, 43), (PartId(0), 42)]);
    // The laser pierces like the rifle; the blade and pellets do not pierce at all.
    assert_eq!(Weapon::Laser.penetration(), 20);
    assert_eq!(Weapon::Shotgun.penetration(), 0);
    assert_eq!(Weapon::EnergyBlade.penetration(), 0);
    // Without the plate the rail's whole beam lands on the chassis.
    let mut session = duel();
    let events = shoot(&mut session, Weapon::RailGun, 32_768, chest);
    assert_eq!(struck(&events), [(PartId(0), 85)]);
}

#[test]
fn several_parts_come_off_at_once_and_lost_weapons_cannot_fire() {
    let mut session = duel();
    // A rail beam breaks B's left arm (60): the rifle at its tip comes off with it.
    let events = shoot(&mut session, Weapon::RailGun, 32_768, |session| {
        part_center(session, 1, PartId(3))
    });
    assert_eq!(struck(&events), [(PartId(3), 85)]);
    assert_eq!(detached(&events), [(PartId(3), true), (PartId(7), false)]);
    // Then the right arm and the blade.
    let events = shoot(&mut session, Weapon::RailGun, 32_768, |session| {
        part_center(session, 1, PartId(4))
    });
    assert_eq!(detached(&events), [(PartId(4), true), (PartId(8), false)]);
    let b = &session.fighters[1];
    assert!(b.alive());
    assert_eq!(b.parts.health()[3], 0);
    assert_eq!(b.parts.health()[7], 40, "carried off whole, not destroyed");
    assert_eq!(b.parts.lost().count_ones(), 4);
    assert!(b.parts.weapon_lost(&b.build, Weapon::EnergyRifle));
    assert!(b.parts.weapon_lost(&b.build, Weapon::EnergyBlade));
    assert!(!b.parts.weapon_lost(&b.build, Weapon::Shotgun));
    // B can no longer fire the rifle or swing the blade, but its shotgun still fires, and a gun
    // its build never carried fires as shipped.
    let mut fired = Vec::new();
    for weapon in [
        Weapon::EnergyRifle,
        Weapon::Shotgun,
        Weapon::Laser,
        Weapon::EnergyBlade,
    ] {
        for _ in 0..60 {
            let intent = Intent {
                yaw: 32_768,
                fire: weapon != Weapon::EnergyBlade,
                melee: weapon == Weapon::EnergyBlade,
                select: Some(weapon),
                ..Intent::IDLE
            };
            step(&mut session, Intent::IDLE, intent);
            fired.extend(session.events.iter().filter_map(|event| match *event {
                Event::Fired {
                    slot: 1, weapon, ..
                } => Some(weapon),
                _ => None,
            }));
        }
    }
    fired.dedup();
    assert_eq!(fired, [Weapon::Shotgun, Weapon::Laser]);
}

#[test]
fn losing_the_chassis_leaves_the_core_alive_and_immobile_and_the_core_is_the_kill() {
    let mut session = duel();
    let chest = |session: &Session| part_center(session, 1, PartId(0));
    shoot(&mut session, Weapon::RailGun, 32_768, chest);
    let events = shoot(&mut session, Weapon::RailGun, 32_768, chest);
    // The root breaks: everything but the core (a leaf on it) falls away; B lives on.
    let lost = detached(&events);
    assert_eq!(lost.len(), STANDARD.parts().len() - 1, "{lost:?}");
    assert_eq!(lost[0], (PartId(0), true));
    assert!(lost.iter().all(|(part, _)| *part != PartId(1)));
    let b = &session.fighters[1];
    assert!(b.alive());
    assert_eq!(b.core_health(), 100);
    assert!(!b.parts.mobile(&b.build));
    assert!(b.parts.weapon_lost(&b.build, Weapon::Shotgun));
    // It cannot walk or jump (no leg, wheel or track), only turn.
    let before = session.fighters[1].player.center(&session.world);
    for _ in 0..60 {
        let run = Intent {
            yaw: 32_768,
            forward: 1,
            jump: true,
            sprint: true,
            ..Intent::IDLE
        };
        step(&mut session, Intent::IDLE, run);
    }
    let after = session.fighters[1].player.center(&session.world);
    assert!(
        (0..3).all(|axis| (after[axis] - before[axis]).abs() < 0.5),
        "{before:?} {after:?}"
    );
    // The core, exposed now, is the kill: two rail hits.
    let core = |session: &Session| part_center(session, 1, PartId(1));
    let events = shoot(&mut session, Weapon::RailGun, 32_768, core);
    assert_eq!(struck(&events), [(PartId(1), 85)]);
    assert!(session.fighters[1].alive());
    let events = shoot(&mut session, Weapon::RailGun, 32_768, core);
    assert!(events.contains(&Event::Killed {
        victim: 1,
        killer: Some(0),
        weapon: Some(Weapon::RailGun),
    }));
    assert_eq!(session.fighters[0].frags, 1);
    // Respawn restores the configured build, whole.
    while !session.fighters[1].alive() {
        step(&mut session, Intent::IDLE, facing(32_768));
    }
    let b = &session.fighters[1];
    assert_eq!(b.parts, PartDamage::fresh(&STANDARD));
    assert_eq!(b.energy, Pool::of(&STANDARD).capacity);
}

#[test]
fn a_destroyed_battery_takes_its_energy_once_and_blasts_its_neighbours() {
    let mut session = duel();
    rebuild(&mut session, 1, HEAVY);
    let battery = PartId(11);
    assert_eq!(session.fighters[1].pool().capacity, 160_000);
    // B turned away: its battery (60) on its back beside the core faces A.
    let events = shoot(&mut session, Weapon::RailGun, 0, |session| {
        part_center(session, 1, battery)
    });
    let hits = struck(&events);
    assert_eq!(hits[0], (battery, 85));
    assert!(detached(&events).contains(&(battery, true)));
    let blasts = events
        .iter()
        .filter(|event| matches!(event, Event::Exploded { .. }))
        .count();
    assert_eq!(blasts, 1);
    // The blast hurts its neighbours (the core beside it among them) through the same damage,
    // each part once, by at most the blast's damage, credited to A.
    let core = hits
        .iter()
        .find(|(part, _)| *part == PartId(1))
        .expect("the blast reached the core");
    assert!((1..=BATTERY_BLAST_DAMAGE).contains(&core.1), "{hits:?}");
    let b = &session.fighters[1];
    assert!(b.alive());
    // Its capacity and its share of the charge are gone: 160 full, 100 left.
    assert_eq!(b.pool().capacity, 100_000);
    assert_eq!(b.energy, 100_000);
    // Once: the battery stays gone, nothing blasts again and the capacity holds.
    for _ in 0..120 {
        step(&mut session, Intent::IDLE, facing(0));
        assert!(
            !session
                .events
                .iter()
                .any(|event| matches!(event, Event::Exploded { .. }))
        );
    }
    let b = &session.fighters[1];
    assert_eq!(b.pool().capacity, 100_000);
    assert_eq!(b.energy, 100_000);
}

#[test]
fn part_damage_travels_in_snapshots_and_a_predictor_follows_it() {
    let mut host = duel();
    shoot(&mut host, Weapon::RailGun, 32_768, |session| {
        part_center(session, 1, PartId(3))
    });
    let snapshot = decode_snapshot(&encode_snapshot(&host.snapshot())).unwrap();
    assert_eq!(snapshot, host.snapshot());
    let me = *snapshot.slot(1).unwrap();
    assert_eq!(me.parts, host.fighters[1].parts);
    assert!(me.parts.weapon_lost(&me.build, Weapon::EnergyRifle));
    assert_eq!(me.core_health(), 100);
    // A health above its part's is malformed, not stored.
    let mut tampered = host.snapshot();
    let fighter = &mut tampered.fighters[1];
    let mut health = fighter.parts.health().to_vec();
    health[0] = 121;
    assert!(PartDamage::from_health(&fighter.build, &health).is_err());
    // A predictor resuming from the snapshot takes the host's part damage: B's lost rifle does
    // not fire there either, its shotgun does.
    let mut predictor = Session::new(host.arena.clone()).unwrap();
    predictor
        .place(0, me.position, me.velocity, me.motion, me.held, me.hook)
        .unwrap();
    predictor.fighters[0].build = me.build;
    predictor.fighters[0].parts = me.parts;
    predictor.fighters[0].energy = me.energy;
    let mut fired = Vec::new();
    for weapon in [Weapon::EnergyRifle, Weapon::Shotgun] {
        let intent = Intent {
            yaw: 32_768,
            fire: true,
            select: Some(weapon),
            ..Intent::IDLE
        };
        for _ in 0..40 {
            predictor.tick(intent).unwrap();
            fired.extend(predictor.events.iter().filter_map(|event| match *event {
                Event::Fired { weapon, .. } => Some(weapon),
                _ => None,
            }));
        }
    }
    fired.dedup();
    assert_eq!(fired, [Weapon::Shotgun]);
    assert_eq!(predictor.fighters[0].parts, me.parts);
}

#[test]
fn bot_matches_lose_parts_and_replay_exactly() {
    let play = || {
        let mut session = Session::with_rules(Arena::greybox(), Rules::deathmatch(11)).unwrap();
        let (mut lost, mut kills) = (0, 0);
        let mut bytes = Vec::new();
        for _ in 0..40 * 60 {
            session.step().unwrap();
            for event in &session.events {
                match event {
                    Event::Detached { .. } => lost += 1,
                    Event::Killed { .. } => kills += 1,
                    _ => {}
                }
            }
            bytes.extend(encode_snapshot(&session.snapshot()));
        }
        (lost, kills, bytes)
    };
    let (lost, kills, bytes) = play();
    assert!(lost > 0 && kills > 0, "{lost} parts lost, {kills} kills");
    assert!(play().2 == bytes, "the match replays bit for bit");
}
