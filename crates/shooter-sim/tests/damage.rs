//! Per-part damage on the host (`shooter_sim::damage`, Atlas D2.i89.1, D2.i70.8): which part a
//! shot from the front or the side strikes, splash on every part in reach once, armor plates
//! and penetration, several parts lost at once and what they take with them (weapons that no
//! longer fire, a robot that no longer walks), the root chassis as the only kill and respawn
//! restoring the build, a destroyed battery's energy and blast, the wire and a predictor following the
//! host, and bot matches that lose parts and replay exactly.

use shooter_sim::aim;
use shooter_sim::damage::{BATTERY_BLAST_DAMAGE, PartDamage};
use shooter_sim::energy::Pool;
use shooter_sim::physics::Vec3i;
use shooter_sim::robot::parts::{ARMOR_PLATE, BEAM_LONG, CHASSIS_L, CHASSIS_S, SENSOR};
use shooter_sim::robot::{ARMORY, BETA, Face, HEAVY, PartId, RawMount, RawPart, STANDARD};
use shooter_sim::weapons::{SPLASH_DAMAGE, Weapon};
use shooter_sim::wire::{decode_snapshot, encode_snapshot};
use shooter_sim::{Arena, Assembly, Event, Input, Intent, Rules, Session, Spawn};

const A: u32 = 1;
const B: u32 = 2;

/// A (slot 0, the [`ARMORY`]) faces B (slot 1) 12 m apart on an open floor; B respawns after half
/// a second.
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
    // A fires only what it carries: every weapon.
    rebuild(&mut session, 0, ARMORY);
    session
}

/// `slot` is built as `build`, whole and fully charged, this life.
fn rebuild(session: &mut Session, slot: usize, build: Assembly) {
    session.fighters[slot].configured = build;
    session.rebuild(u8::try_from(slot).unwrap(), build).unwrap();
    let fighter = &mut session.fighters[slot];
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
    let parts = fighter.hitboxes();
    parts.get(index).expect("a connected part").center
}

/// The yaw and pitch that put `slot`'s reticle on `point` ([`aim::at`]).
fn aim_at(session: &Session, slot: usize, point: [f32; 3]) -> (u16, i16) {
    aim::at(Some(&session.world), session.fighters[slot].eye(), point)
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

#[test]
fn front_and_side_shots_strike_the_parts_they_meet() {
    // From the front, below the weapons on its chest: the chassis, the whole bolt.
    let mut session = duel();
    let events = shoot(&mut session, Weapon::EnergyRifle, 32_768, |session| {
        let mut at = part_center(session, 1, PartId(0));
        at[1] -= 20.0;
        at
    });
    assert_eq!(struck(&events), [(PartId(0), 12)]);
    // From the front at the shotgun on the chest: the shotgun, not the chassis behind it.
    let mut session = duel();
    let events = shoot(&mut session, Weapon::EnergyRifle, 32_768, |session| {
        part_center(session, 1, PartId(10))
    });
    assert_eq!(struck(&events), [(PartId(10), 12)]);
    // From B's left side at the rifle's height: the rifle on the left of its chest, not the
    // blade beside it.
    let mut session = duel();
    let events = shoot(&mut session, Weapon::EnergyRifle, 16_384, |session| {
        part_center(session, 1, PartId(7))
    });
    assert_eq!(struck(&events), [(PartId(7), 12)]);
    // From its right side: the blade on the right.
    let mut session = duel();
    let events = shoot(&mut session, Weapon::EnergyRifle, 49_152, |session| {
        part_center(session, 1, PartId(8))
    });
    assert_eq!(struck(&events), [(PartId(8), 12)]);
}

#[test]
fn a_rocket_splash_strikes_each_part_in_reach_once() {
    let mut session = duel();
    let events = shoot(&mut session, Weapon::RocketLauncher, 32_768, |session| {
        let center = session.fighters[1].center();
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

/// The standard build with an armor plate (part 11) over the front of its chassis, below the
/// weapons on its chest.
fn plated() -> Assembly {
    let mut candidate = STANDARD.candidate();
    candidate.parts.push(RawPart {
        id: 11,
        kind: ARMOR_PLATE.id.0,
        mount: Some(RawMount {
            parent: 0,
            socket: CHASSIS_S.socket(Face::PosX, 0, 0).unwrap().0,
            ..RawMount::default()
        }),
        modification: None,
    });
    BETA.validate(&candidate).unwrap()
}

#[test]
fn armor_plates_stop_strikes_and_penetrating_beams_pass_a_share() {
    let plate = PartId(11);
    let chest = |session: &Session| part_center(session, 1, plate);
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
    let events = shoot(&mut session, Weapon::RailGun, 32_768, |session| {
        let mut at = part_center(session, 1, PartId(0));
        at[1] -= 20.0;
        at
    });
    assert_eq!(struck(&events), [(PartId(0), 85)]);
}

/// The standard build with arms, as the loadouts had them before shooter#91: a long beam on
/// each side of the chassis (parts 3 and 4), the rifle and the blade moved onto their tips.
fn armed() -> Assembly {
    let socket = |on: &shooter_sim::robot::PartType, face, u, v| on.socket(face, u, v).unwrap().0;
    let mut candidate = STANDARD.candidate();
    for (id, face, u, v) in [(3, Face::PosY, 3, 0), (4, Face::NegY, 0, 3)] {
        candidate.parts.push(RawPart {
            id,
            kind: BEAM_LONG.id.0,
            mount: Some(RawMount {
                parent: 0,
                socket: socket(&CHASSIS_S, face, u, v),
                ..RawMount::default()
            }),
            modification: None,
        });
    }
    for part in &mut candidate.parts {
        if let Some(arm) = [(7, 3), (8, 4)]
            .iter()
            .find(|(id, _)| *id == part.id)
            .map(|(_, arm)| *arm)
        {
            part.mount = Some(RawMount {
                parent: arm,
                socket: socket(&BEAM_LONG, Face::PosZ, 0, 0),
                ..RawMount::default()
            });
        }
    }
    BETA.validate(&candidate).unwrap()
}

#[test]
fn several_parts_come_off_at_once_and_lost_weapons_cannot_fire() {
    let mut session = duel();
    rebuild(&mut session, 1, armed());
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
    // B can no longer fire the rifle or swing the blade, but its shotgun still fires; a gun its
    // build never carried (the laser) does not fire either (shooter#92).
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
    assert_eq!(fired, [Weapon::Shotgun]);
}

#[test]
fn losing_the_legs_leaves_the_robot_alive_and_immobile_and_the_root_chassis_is_the_kill() {
    let mut session = duel();
    // Each leg (70) breaks to one rail beam and comes off alone; B lives on without them.
    for leg in [PartId(5), PartId(6)] {
        let events = shoot(&mut session, Weapon::RailGun, 32_768, |session| {
            part_center(session, 1, leg)
        });
        assert_eq!(struck(&events), [(leg, 85)]);
        assert_eq!(detached(&events), [(leg, true)]);
    }
    let b = &session.fighters[1];
    assert!(b.alive());
    assert_eq!(b.chassis_health(), 270);
    assert!(!b.parts.mobile(&b.build));
    assert!(!b.parts.weapon_lost(&b.build, Weapon::Shotgun));
    // Its legs came off its robot (shooter#73): they no longer hold it up, so it drops onto
    // its chassis; and it cannot walk or jump (no leg, wheel or track), only turn.
    assert!(session.mechanics.robot(1).unwrap().lost != 0);
    let standing = session.fighters[1].center();
    for _ in 0..30 {
        step(&mut session, Intent::IDLE, facing(32_768));
    }
    let before = session.fighters[1].center();
    assert!(before[1] < standing[1] - 50.0, "{standing:?} {before:?}");
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
    let after = session.fighters[1].center();
    assert!(
        (0..3).all(|axis| (after[axis] - before[axis]).abs() < 15.0),
        "{before:?} {after:?}"
    );
    // The root chassis (270) is the kill: four rail beams to it (below the weapons on its
    // chest), the fourth kills.
    let chest = |session: &Session| {
        let mut at = part_center(session, 1, PartId(0));
        at[1] -= 20.0;
        at
    };
    for left in [185, 100, 15] {
        let events = shoot(&mut session, Weapon::RailGun, 32_768, chest);
        assert_eq!(struck(&events), [(PartId(0), 85)]);
        assert!(detached(&events).is_empty());
        assert!(session.fighters[1].alive());
        assert_eq!(session.fighters[1].chassis_health(), left);
    }
    let events = shoot(&mut session, Weapon::RailGun, 32_768, chest);
    assert_eq!(struck(&events), [(PartId(0), 85)]);
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
fn another_chassis_section_is_ordinary_structure_and_only_the_root_is_vital() {
    // The large chassis as the root, a small one on its roof and a sensor on top of that.
    let socket = |on: &shooter_sim::robot::PartType, face, u, v| on.socket(face, u, v).unwrap().0;
    let part = |id, kind: &shooter_sim::robot::PartType, parent, socket| RawPart {
        id,
        kind: kind.id.0,
        mount: Some(RawMount {
            parent,
            socket,
            ..RawMount::default()
        }),
        modification: None,
    };
    let build = BETA
        .validate(&shooter_sim::robot::Candidate {
            parts: vec![
                RawPart {
                    id: 0,
                    kind: CHASSIS_L.id.0,
                    mount: None,
                    modification: None,
                },
                part(1, &CHASSIS_S, 0, socket(&CHASSIS_L, Face::PosZ, 0, 0)),
                part(2, &SENSOR, 1, socket(&CHASSIS_S, Face::PosZ, 0, 0)),
            ],
        })
        .unwrap();
    let mut damage = PartDamage::fresh(&build);
    assert_eq!(PartDamage::root(&build), Some(0));
    assert_eq!(damage.chassis_health(&build), CHASSIS_L.health);
    // The section breaks: it and the sensor on it fall away, the robot lives.
    let change = damage.hurt(&build, 1, i32::from(CHASSIS_S.health)).unwrap();
    assert!(change.destroyed);
    assert_eq!(change.lost, 0b110);
    assert!(damage.attached(0));
    assert_eq!(damage.chassis_health(&build), CHASSIS_L.health);
    // The root breaks: everything is lost and its health, the robot's life, is gone.
    let change = damage.hurt(&build, 0, i32::from(CHASSIS_L.health)).unwrap();
    assert!(change.destroyed);
    assert_eq!(change.lost, 0b001);
    assert_eq!(damage.lost(), 0b111);
    assert_eq!(damage.chassis_health(&build), 0);
}

#[test]
fn a_destroyed_battery_takes_its_energy_once_and_blasts_its_neighbours() {
    let mut session = duel();
    rebuild(&mut session, 1, HEAVY);
    let battery = PartId(11);
    assert_eq!(session.fighters[1].pool().capacity, 160_000);
    // B turned away: its battery (60) on its back faces A.
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
    // The blast hurts its neighbours (the root chassis it sits on among them) through the same
    // damage, each part once, by at most the blast's damage, credited to A.
    let chassis = hits
        .iter()
        .find(|(part, _)| *part == PartId(0))
        .expect("the blast reached the root chassis");
    assert!((1..=BATTERY_BLAST_DAMAGE).contains(&chassis.1), "{hits:?}");
    let b = &session.fighters[1];
    assert!(b.alive());
    assert_eq!(b.chassis_health(), 290 - u16::try_from(chassis.1).unwrap());
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
    // Snapshots carry valid builds only: A takes up the heavy build's rail gun.
    rebuild(&mut host, 0, HEAVY);
    // A rail beam destroys B's rifle on the left of its chest.
    shoot(&mut host, Weapon::RailGun, 32_768, |session| {
        part_center(session, 1, PartId(7))
    });
    let snapshot = decode_snapshot(&encode_snapshot(&host.snapshot())).unwrap();
    assert_eq!(snapshot, host.snapshot());
    let me = snapshot.slot(1).unwrap().clone();
    assert_eq!(me.parts, host.fighters[1].parts);
    assert!(me.parts.weapon_lost(&me.build, Weapon::EnergyRifle));
    assert_eq!(me.chassis_health(), 270);
    // A health above its part's is malformed, not stored.
    let mut tampered = host.snapshot();
    let fighter = &mut tampered.fighters[1];
    let mut health = fighter.parts.health().to_vec();
    health[0] = 271;
    assert!(PartDamage::from_health(&fighter.build, &health).is_err());
    // A predictor resuming from the snapshot takes the host's part damage: B's lost rifle does
    // not fire there either, its shotgun does.
    let mut predictor = Session::new(host.arena.clone()).unwrap();
    predictor
        .place(0, me.robot.as_ref().unwrap(), me.motion, me.held, me.hook)
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

/// One tick of B (the standard build) holding `intent`: the weapons it fired.
fn b_takes(session: &mut Session, intent: Intent) -> Vec<Weapon> {
    step(session, Intent::IDLE, intent);
    session
        .events
        .iter()
        .filter_map(|event| match *event {
            Event::Fired {
                slot: 1, weapon, ..
            } => Some(weapon),
            _ => None,
        })
        .collect()
}

/// The host takes a weapon switch only to a weapon the build carries and still has
/// (shooter#92): an uncarried or lost one is refused, the held weapon stays. A fighter spawns
/// holding its build's first weapon, and with only the blade left it holds the blade and the
/// trigger swings it.
#[test]
fn the_host_refuses_switching_to_a_weapon_the_robot_does_not_have() {
    let mut session = duel();
    let select = |weapon| Intent {
        yaw: 32_768,
        select: Some(weapon),
        ..Intent::IDLE
    };
    assert_eq!(session.fighters[1].arsenal.held, Weapon::EnergyRifle);
    // Not carried: refused, whatever the client asks.
    for weapon in [Weapon::RailGun, Weapon::Laser, Weapon::Flamethrower] {
        b_takes(&mut session, select(weapon));
        assert_eq!(session.fighters[1].arsenal.held, Weapon::EnergyRifle);
    }
    // The blade is not held while a gun is left.
    b_takes(&mut session, select(Weapon::EnergyBlade));
    assert_eq!(session.fighters[1].arsenal.held, Weapon::EnergyRifle);
    b_takes(&mut session, select(Weapon::Shotgun));
    assert_eq!(session.fighters[1].arsenal.held, Weapon::Shotgun);
    // The grapple module is never held, whole or lost (Atlas D2.i104.2); back to the rifle is
    // fine.
    let b = &mut session.fighters[1];
    let build = b.build;
    let index = |weapon| {
        build
            .parts()
            .iter()
            .position(|part| BETA.weapon(part) == Some(weapon))
            .unwrap()
    };
    let grapple = index(Weapon::Grapple);
    b.parts.hurt(&build, grapple, 1_000);
    assert!(!b.parts.armed(&build, Weapon::Grapple));
    b_takes(&mut session, select(Weapon::Grapple));
    assert_eq!(session.fighters[1].arsenal.held, Weapon::Shotgun);
    b_takes(&mut session, select(Weapon::EnergyRifle));
    assert_eq!(session.fighters[1].arsenal.held, Weapon::EnergyRifle);
    // The rifle and the shotgun lost too: the blade is the last weapon, held and swung by the
    // trigger.
    let b = &mut session.fighters[1];
    for weapon in [Weapon::EnergyRifle, Weapon::Shotgun] {
        b.parts.hurt(&build, index(weapon), 1_000);
    }
    b_takes(&mut session, select(Weapon::EnergyBlade));
    assert_eq!(session.fighters[1].arsenal.held, Weapon::EnergyBlade);
    let mut fired = Vec::new();
    for _ in 0..40 {
        fired.extend(b_takes(
            &mut session,
            Intent {
                yaw: 32_768,
                fire: true,
                ..Intent::IDLE
            },
        ));
    }
    assert!(fired.iter().all(|weapon| *weapon == Weapon::EnergyBlade));
    assert!(!fired.is_empty());
    // Rebuilt (as at a spawn), it takes up its build's first weapon.
    session.rebuild(1, HEAVY).unwrap();
    assert_eq!(session.fighters[1].arsenal.held, Weapon::RailGun);
}
