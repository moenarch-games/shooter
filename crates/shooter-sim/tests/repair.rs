//! Field repair on the host (`shooter_sim::repair`, Atlas D2.i70.9, shooter#61): a mounted
//! repair module restores a chosen damaged leg or weapon over time and for energy; a direct hit
//! on the part or the module pauses it (work kept, no health and no energy meanwhile, resuming by
//! itself), hits elsewhere do not; refused orders (whole, unknown, out-of-reach or destroyed
//! parts), a destroyed module, the shared pool shared with shots and motion, an empty pool,
//! respawn, the wire, a predicting client and a replay.
// The platform's floats only check results here; the simulation uses libm (clippy.toml).
#![allow(clippy::disallowed_methods)]

use shooter_sim::aim;
use shooter_sim::physics::Vec3i;
use shooter_sim::repair::{ENERGY_PER_TICK, Order, PAUSE_TICKS, RADIUS_UNITS, TICKS_PER_HEALTH};
use shooter_sim::robot::parts::{BEAM_LONG, CHASSIS_S, REPAIR_MODULE};
use shooter_sim::robot::{BETA, Face, MECHANIC, PartId, RawMount, RawPart, STANDARD};
use shooter_sim::weapons::Weapon;
use shooter_sim::wire::{decode_input, decode_snapshot, encode_input, encode_snapshot};
use shooter_sim::{Arena, Assembly, Event, Input, Intent, Rules, Session, Spawn};

const A: u32 = 1;
const B: u32 = 2;
const LEG: PartId = PartId(5);
const RIFLE: PartId = PartId(7);
const MODULE: PartId = PartId(11);
/// B faces A...
const TOWARD: u16 = 32_768;
/// ...or turns its back (and its repair module) to A.
const AWAY: u16 = 0;

/// A (slot 0, the standard build: its rifle) faces B (slot 1, the mechanic, a repair module on
/// its back) 12 m apart on an open floor; B respawns after half a second.
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
            yaw: TOWARD,
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
    rebuild(&mut session, 1, MECHANIC);
    session
}

/// `slot` is built as `build`, whole and fully charged, this life (and comes back as it).
fn rebuild(session: &mut Session, slot: usize, build: Assembly) {
    session.fighters[slot].configured = build;
    session.rebuild(u8::try_from(slot).unwrap(), build).unwrap();
    let fighter = &mut session.fighters[slot];
    fighter.energy = fighter.pool().capacity;
}

fn index(session: &Session, slot: usize, part: PartId) -> usize {
    session.fighters[slot]
        .build
        .parts()
        .iter()
        .position(|p| p.id == part)
        .unwrap()
}

fn health(session: &Session, slot: usize, part: PartId) -> u16 {
    session.fighters[slot].parts.health()[index(session, slot, part)]
}

/// `amount` of damage straight to B's `part` (no hit: no pause).
fn wear(session: &mut Session, part: PartId, amount: i32) {
    let at = index(session, 1, part);
    let fighter = &mut session.fighters[1];
    let build = fighter.build;
    fighter.parts.hurt(&build, at, amount);
}

/// Where `slot`'s part `part` is now (units).
fn part_center(session: &Session, slot: usize, part: PartId) -> [f32; 3] {
    let at = index(session, slot, part);
    session.fighters[slot]
        .hitboxes()
        .get(at)
        .expect("a connected part")
        .center
}

/// The yaw and pitch that put `slot`'s reticle on `point` ([`aim::at`]).
fn aim_at(session: &Session, slot: usize, point: [f32; 3]) -> (u16, i16) {
    aim::at(Some(&session.world), session.fighters[slot].eye(), point)
}

/// One tick through the wire: A holds `a`, B holds `b` (both seeing the match as it is).
fn step(session: &mut Session, a: Intent, b: Intent) {
    for (id, slot, intent) in [(A, 0, a), (B, 1, b)] {
        let sequence = session.fighters[slot].ack + 1;
        let view_tick = session.ticks();
        let command = decode_input(&encode_input(intent, view_tick, None, None)).unwrap();
        session.queue(
            id,
            Input {
                sequence,
                intent: command.intent,
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

/// B facing `yaw` and asking to repair `part`.
fn order(yaw: u16, part: PartId) -> Intent {
    Intent {
        yaw,
        repair: Some(Order::Start(part)),
        ..Intent::IDLE
    }
}

/// The parts of B struck by A this tick.
fn struck(session: &Session) -> Vec<PartId> {
    session
        .events
        .iter()
        .filter_map(|event| match *event {
            Event::Hit {
                victim: 1,
                attacker: Some(0),
                part,
                ..
            } => Some(part),
            _ => None,
        })
        .collect()
}

/// A's rifle readies (B facing `yaw`), then one bolt at B's `part`; the parts it struck.
fn shoot(session: &mut Session, yaw: u16, part: PartId) -> Vec<PartId> {
    while session.fighters[0].arsenal.cooldown(session.ticks()) > 0 {
        step(session, Intent::IDLE, facing(yaw));
    }
    let (aim, pitch) = aim_at(session, 0, part_center(session, 1, part));
    let fire = Intent {
        yaw: aim,
        pitch,
        fire: true,
        ..Intent::IDLE
    };
    step(session, fire, facing(yaw));
    struck(session)
}

/// B's leg worn by 30 and its repair asked for, a few ticks after the match starts.
fn repairing_leg(yaw: u16) -> Session {
    let mut session = duel();
    for _ in 0..10 {
        step(&mut session, Intent::IDLE, facing(yaw));
    }
    wear(&mut session, LEG, 30);
    step(&mut session, Intent::IDLE, order(yaw, LEG));
    let repair = session.fighters[1].repair.expect("the repair started");
    assert_eq!((repair.target, repair.module), (LEG, MODULE));
    session
}

#[test]
fn the_mechanic_is_a_valid_loadout_whose_module_reaches_every_other_part() {
    assert_eq!(BETA.validate(&MECHANIC.candidate()), Ok(MECHANIC));
    assert_eq!(shooter_sim::robot::loadout("mechanic"), Some(MECHANIC));
    let mut session = duel();
    step(&mut session, Intent::IDLE, facing(TOWARD));
    // Nothing is damaged: nothing to repair.
    assert_eq!(session.fighters[1].repairable(), 0);
    for part in MECHANIC.parts().iter().map(|part| part.id) {
        wear(&mut session, part, 1);
    }
    let everything_but_the_module = ((1u32 << MECHANIC.parts().len()) - 1) & !(1 << 8);
    assert_eq!(index(&session, 1, MODULE), 8);
    assert_eq!(
        session.fighters[1].repairable(),
        everything_but_the_module,
        "a module does not repair itself"
    );
    // Without a module nothing is ever repairable.
    let mut plain = duel();
    rebuild(&mut plain, 1, STANDARD);
    step(&mut plain, Intent::IDLE, facing(TOWARD));
    plain.fighters[1].parts.hurt(&STANDARD, 2, 5);
    assert_eq!(plain.fighters[1].repairable(), 0);
}

#[test]
fn a_partially_damaged_leg_and_weapon_are_repaired_for_time_and_energy() {
    for (part, worn) in [(LEG, 30), (RIFLE, 20)] {
        let mut session = duel();
        // B's pool starts empty so the energy shows, against a control without a repair.
        let mut control = duel();
        for match_ in [&mut session, &mut control] {
            step(match_, Intent::IDLE, facing(TOWARD));
            wear(match_, part, worn);
            match_.fighters[1].energy = 0;
        }
        let whole = health(&session, 1, part) + u16::try_from(worn).unwrap();
        assert_ne!(
            session.fighters[1].repairable() & 1 << index(&session, 1, part),
            0
        );
        step(&mut session, Intent::IDLE, order(TOWARD, part));
        step(&mut control, Intent::IDLE, facing(TOWARD));
        // The first tick's pool was empty: the order took, the work waits.
        let repair = session.fighters[1].repair.unwrap();
        assert_eq!((repair.progress, repair.healed), (0, 0));
        let mut worked = 0;
        for _ in 0..u32::from(TICKS_PER_HEALTH) * 40 {
            step(&mut session, Intent::IDLE, facing(TOWARD));
            step(&mut control, Intent::IDLE, facing(TOWARD));
            if session.fighters[1].repair.is_none() {
                worked += 1;
                break;
            }
            worked += 1;
            let repair = session.fighters[1].repair.unwrap();
            assert_eq!(
                u32::from(repair.healed) * u32::from(TICKS_PER_HEALTH) + u32::from(repair.progress),
                worked,
                "every tick works once the pool pays"
            );
            assert_eq!(
                health(&session, 1, part),
                whole - u16::try_from(worn).unwrap() + repair.healed
            );
            assert_eq!(
                control.fighters[1].energy - session.fighters[1].energy,
                worked * ENERGY_PER_TICK,
                "each tick of work costs its energy"
            );
        }
        assert_eq!(health(&session, 1, part), whole, "{part:?} repaired whole");
        assert_eq!(
            worked,
            u32::try_from(worn).unwrap() * u32::from(TICKS_PER_HEALTH),
            "a health point every {TICKS_PER_HEALTH} ticks"
        );
        assert!(session.fighters[1].repair.is_none(), "over once whole");
        assert_eq!(
            session.fighters[1].repairable() & 1 << index(&session, 1, part),
            0
        );
    }
}

/// B repairing its leg; A shoots `part` (B facing `yaw`) after some work: whether it paused,
/// checked against a control that never repairs (the energy) along the way.
fn hit_while_repairing(yaw: u16, part: PartId) -> bool {
    let mut session = repairing_leg(yaw);
    for _ in 0..20 {
        step(&mut session, Intent::IDLE, facing(yaw));
    }
    let before = session.fighters[1].repair.unwrap();
    assert!(before.healed > 0 && before.paused == 0);
    assert_eq!(shoot(&mut session, yaw, part), [part], "A struck {part:?}");
    let after = session.fighters[1].repair.expect("still repairing");
    after.paused > 0
}

#[test]
fn a_direct_hit_on_the_target_or_the_module_pauses_it_and_others_do_not() {
    assert!(hit_while_repairing(TOWARD, LEG), "the leg under repair");
    assert!(hit_while_repairing(AWAY, MODULE), "the module repairing it");
    assert!(!hit_while_repairing(TOWARD, PartId(10)), "the shotgun");
    assert!(!hit_while_repairing(TOWARD, RIFLE), "the rifle");
}

#[test]
fn a_paused_repair_heals_nothing_spends_nothing_keeps_its_work_and_resumes() {
    let mut session = repairing_leg(TOWARD);
    for _ in 0..20 {
        step(&mut session, Intent::IDLE, facing(TOWARD));
    }
    // Low on energy (but enough to work), so a spend shows against the regeneration alone.
    session.fighters[1].energy = 10_000;
    assert_eq!(shoot(&mut session, TOWARD, LEG), [LEG]);
    let hit = session.fighters[1].repair.unwrap();
    assert_eq!(
        hit.paused,
        PAUSE_TICKS - 1,
        "the hit's tick is the pause's first"
    );
    let leg = health(&session, 1, LEG);
    let regen = session.fighters[1].pool().regen;
    for tick in 1..PAUSE_TICKS {
        let charge = session.fighters[1].energy;
        step(&mut session, Intent::IDLE, facing(TOWARD));
        let repair = session.fighters[1].repair.unwrap();
        assert_eq!(repair.paused, PAUSE_TICKS - 1 - tick);
        assert_eq!(
            (repair.progress, repair.healed),
            (hit.progress, hit.healed),
            "work kept"
        );
        assert_eq!(health(&session, 1, LEG), leg, "no health while paused");
        // Only regeneration (less the legs' motors standing still) moves the charge.
        let drawn = charge + regen - session.fighters[1].energy;
        assert!(
            drawn < ENERGY_PER_TICK,
            "no repair energy while paused ({drawn})"
        );
    }
    assert_eq!(session.fighters[1].repair.unwrap().paused, 0);
    // It resumes by itself.
    let charge = session.fighters[1].energy;
    step(&mut session, Intent::IDLE, facing(TOWARD));
    let repair = session.fighters[1].repair.unwrap();
    assert_eq!(
        u32::from(repair.healed) * u32::from(TICKS_PER_HEALTH) + u32::from(repair.progress),
        u32::from(hit.healed) * u32::from(TICKS_PER_HEALTH) + u32::from(hit.progress) + 1
    );
    assert!(charge + regen - session.fighters[1].energy >= ENERGY_PER_TICK);
    for _ in 0..600 {
        step(&mut session, Intent::IDLE, facing(TOWARD));
    }
    assert!(session.fighters[1].repair.is_none());
    assert_eq!(
        health(&session, 1, LEG),
        shooter_sim::robot::parts::LEG.health
    );
}

#[test]
fn repeated_hits_start_the_pause_over() {
    let mut session = repairing_leg(TOWARD);
    for _ in 0..20 {
        step(&mut session, Intent::IDLE, facing(TOWARD));
    }
    assert_eq!(shoot(&mut session, TOWARD, LEG), [LEG]);
    let first = session.ticks();
    let kept = session.fighters[1].repair.unwrap();
    // The rifle is ready again well inside the pause: a second hit starts it over.
    assert_eq!(shoot(&mut session, TOWARD, LEG), [LEG]);
    let second = session.ticks();
    assert!(second - first < u32::from(PAUSE_TICKS));
    let repair = session.fighters[1].repair.unwrap();
    assert_eq!(repair.paused, PAUSE_TICKS - 1);
    assert_eq!(
        (repair.progress, repair.healed),
        (kept.progress, kept.healed)
    );
    for _ in 1..PAUSE_TICKS {
        step(&mut session, Intent::IDLE, facing(TOWARD));
    }
    // Paused from the first hit until the second's pause ran out.
    let repair = session.fighters[1].repair.unwrap();
    assert_eq!(
        (repair.paused, repair.progress, repair.healed),
        (0, kept.progress, kept.healed)
    );
    step(&mut session, Intent::IDLE, facing(TOWARD));
    assert_ne!(session.fighters[1].repair.unwrap().progress, kept.progress);
}

/// The standard build with a repair module on top of a long beam on the chassis' top, beside
/// the sensor (parts 3 and 11): it reaches the beam it sits on and nothing else.
fn mast() -> Assembly {
    let mut candidate = STANDARD.candidate();
    candidate.parts.push(RawPart {
        id: 3,
        kind: BEAM_LONG.id.0,
        mount: Some(RawMount {
            parent: 0,
            socket: CHASSIS_S.socket(Face::PosZ, 0, 1).unwrap().0,
            ..RawMount::default()
        }),
        modification: None,
    });
    candidate.parts.push(RawPart {
        id: 11,
        kind: REPAIR_MODULE.id.0,
        mount: Some(RawMount {
            parent: 3,
            socket: BEAM_LONG.socket(Face::PosZ, 0, 0).unwrap().0,
            ..RawMount::default()
        }),
        modification: None,
    });
    BETA.validate(&candidate).unwrap()
}

#[test]
fn whole_unknown_out_of_reach_and_destroyed_parts_are_refused() {
    let mut session = duel();
    step(&mut session, Intent::IDLE, facing(TOWARD));
    // Whole, unknown, the module itself.
    for part in [LEG, PartId(30), MODULE] {
        step(&mut session, Intent::IDLE, order(TOWARD, part));
        assert!(session.fighters[1].repair.is_none(), "{part:?} refused");
    }
    wear(&mut session, MODULE, 5);
    step(&mut session, Intent::IDLE, order(TOWARD, MODULE));
    assert!(
        session.fighters[1].repair.is_none(),
        "a module does not repair itself"
    );
    // Out of reach: the module on the mast reaches the mast, not the legs.
    let mut session = duel();
    rebuild(&mut session, 1, mast());
    step(&mut session, Intent::IDLE, facing(TOWARD));
    wear(&mut session, LEG, 10);
    wear(&mut session, PartId(3), 10);
    let fighter = &session.fighters[1];
    let boxes = fighter.hitboxes();
    let module = boxes.get(index(&session, 1, MODULE)).unwrap().center;
    assert!(boxes.reach(index(&session, 1, LEG), module).unwrap() > RADIUS_UNITS);
    assert_eq!(fighter.repairable(), 1 << index(&session, 1, PartId(3)));
    step(&mut session, Intent::IDLE, order(TOWARD, LEG));
    assert!(
        session.fighters[1].repair.is_none(),
        "the leg is out of reach"
    );
    step(&mut session, Intent::IDLE, order(TOWARD, PartId(3)));
    assert_eq!(session.fighters[1].repair.unwrap().target, PartId(3));
    // A refused order leaves the repair under way as it was.
    step(&mut session, Intent::IDLE, order(TOWARD, LEG));
    assert_eq!(session.fighters[1].repair.unwrap().target, PartId(3));
    // Stop ends it.
    step(
        &mut session,
        Intent::IDLE,
        Intent {
            repair: Some(Order::Stop),
            ..facing(TOWARD)
        },
    );
    assert!(session.fighters[1].repair.is_none());

    // Destroyed under repair: it stops, stays at zero and cannot be repaired again.
    let mut session = repairing_leg(TOWARD);
    step(&mut session, Intent::IDLE, facing(TOWARD));
    wear(&mut session, LEG, 1_000);
    step(&mut session, Intent::IDLE, facing(TOWARD));
    assert!(session.fighters[1].repair.is_none());
    assert_eq!(health(&session, 1, LEG), 0);
    step(&mut session, Intent::IDLE, order(TOWARD, LEG));
    assert!(
        session.fighters[1].repair.is_none(),
        "zero health is irreparable"
    );
    for _ in 0..120 {
        step(&mut session, Intent::IDLE, facing(TOWARD));
    }
    assert_eq!(health(&session, 1, LEG), 0, "nothing comes back mid-life");

    // The module destroyed under repair: it stops, and nothing else repairs.
    let mut session = repairing_leg(TOWARD);
    wear(&mut session, MODULE, 1_000);
    step(&mut session, Intent::IDLE, facing(TOWARD));
    assert!(session.fighters[1].repair.is_none());
    assert_eq!(session.fighters[1].repairable(), 0);
}

#[test]
fn repair_moving_and_firing_share_the_pool_first_come_first_served() {
    // B walks and fires its rifle at A while it repairs: nothing locks it out.
    let mut session = repairing_leg(TOWARD);
    let start = session.fighters[1].feet();
    let mut fired = 0;
    for _ in 0..60 {
        let busy = Intent {
            forward: 1,
            fire: true,
            ..facing(TOWARD)
        };
        step(&mut session, Intent::IDLE, busy);
        fired += session
            .events
            .iter()
            .filter(|event| matches!(event, Event::Fired { slot: 1, .. }))
            .count();
    }
    let moved = (session.fighters[1].feet()[2] - start[2]).abs();
    assert!(moved > 50.0, "B walked {moved}");
    assert!(fired > 1, "B fired {fired} times");
    let repair = session.fighters[1].repair.unwrap();
    assert!(repair.healed > 0, "and repaired");

    // Contested: a pool that pays the shot first has nothing left for the repair that tick.
    let shot = BETA
        .tuning(
            &MECHANIC,
            BETA.first(&MECHANIC, Weapon::EnergyRifle).unwrap(),
        )
        .energy(Weapon::EnergyRifle);
    let mut session = repairing_leg(TOWARD);
    while session.fighters[1].arsenal.cooldown(session.ticks()) > 0 {
        step(&mut session, Intent::IDLE, facing(TOWARD));
    }
    let before = session.fighters[1].repair.unwrap();
    session.fighters[1].energy = shot + ENERGY_PER_TICK - 1;
    let fire = Intent {
        fire: true,
        ..facing(TOWARD)
    };
    step(&mut session, Intent::IDLE, fire);
    assert!(
        session
            .events
            .iter()
            .any(|event| matches!(event, Event::Fired { slot: 1, .. })),
        "the shot came first"
    );
    assert_eq!(
        session.fighters[1].repair.unwrap(),
        before,
        "the repair waited"
    );
    // Without the shot the same charge pays the repair.
    let mut session = repairing_leg(TOWARD);
    let before = session.fighters[1].repair.unwrap();
    session.fighters[1].energy = shot + ENERGY_PER_TICK - 1;
    step(&mut session, Intent::IDLE, facing(TOWARD));
    assert_ne!(session.fighters[1].repair.unwrap(), before);
}

#[test]
fn an_empty_pool_holds_the_repair_until_regeneration_pays() {
    let mut session = repairing_leg(TOWARD);
    let before = session.fighters[1].repair.unwrap();
    session.fighters[1].energy = 0;
    step(&mut session, Intent::IDLE, facing(TOWARD));
    assert_eq!(
        session.fighters[1].repair.unwrap(),
        before,
        "nothing to pay with"
    );
    // A tick of regeneration later it pays and works.
    step(&mut session, Intent::IDLE, facing(TOWARD));
    let repair = session.fighters[1].repair.unwrap();
    assert_eq!(
        u32::from(repair.healed) * u32::from(TICKS_PER_HEALTH) + u32::from(repair.progress),
        u32::from(before.healed) * u32::from(TICKS_PER_HEALTH) + u32::from(before.progress) + 1
    );
}

#[test]
fn death_ends_the_repair_and_respawn_restores_the_build_without_one() {
    let mut session = repairing_leg(TOWARD);
    for _ in 0..10 {
        step(&mut session, Intent::IDLE, facing(TOWARD));
    }
    assert!(session.fighters[1].repair.is_some());
    // The root chassis at one health; a bolt below the chest ends the life.
    let root = session.fighters[1].chassis_health();
    wear(&mut session, PartId(0), i32::from(root) - 1);
    let mut died = false;
    for _ in 0..3 {
        let mut at = part_center(&session, 1, PartId(0));
        at[1] -= 20.0;
        while session.fighters[0].arsenal.cooldown(session.ticks()) > 0 {
            step(&mut session, Intent::IDLE, facing(TOWARD));
        }
        let (yaw, pitch) = aim_at(&session, 0, at);
        let fire = Intent {
            yaw,
            pitch,
            fire: true,
            ..Intent::IDLE
        };
        step(&mut session, fire, facing(TOWARD));
        if !session.fighters[1].alive() {
            died = true;
            break;
        }
    }
    assert!(died, "B died");
    assert!(session.fighters[1].repair.is_none());
    for _ in 0..60 {
        step(&mut session, Intent::IDLE, facing(TOWARD));
    }
    let fighter = &session.fighters[1];
    assert!(fighter.alive(), "B came back");
    assert!(fighter.repair.is_none());
    assert_eq!(
        fighter.parts,
        shooter_sim::damage::PartDamage::fresh(&MECHANIC)
    );
    assert_eq!(fighter.repairable(), 0);
}

#[test]
fn repair_orders_and_states_travel_on_the_wire() {
    for repair in [None, Some(Order::Stop), Some(Order::Start(LEG))] {
        let intent = Intent {
            repair,
            ..Intent::IDLE
        };
        assert_eq!(
            decode_input(&encode_input(intent, 5, None, None))
                .unwrap()
                .intent,
            intent
        );
    }
    let mut session = repairing_leg(TOWARD);
    wear(&mut session, RIFLE, 4);
    step(&mut session, Intent::IDLE, facing(TOWARD));
    let snapshot = session.snapshot();
    let me = snapshot.of_player(B).unwrap();
    assert_eq!(me.repair, session.fighters[1].repair);
    assert!(me.repair.is_some());
    assert_eq!(me.repairable, session.fighters[1].repairable());
    assert_ne!(me.repairable & 1 << index(&session, 1, RIFLE), 0);
    assert_eq!(
        decode_snapshot(&encode_snapshot(&snapshot)).unwrap(),
        snapshot
    );
    // A repair by a part that is no module, or of a part the build lacks, is malformed.
    for tamper in [
        |repair: &mut shooter_sim::repair::Repair| repair.module = PartId(0),
        |repair: &mut shooter_sim::repair::Repair| repair.target = PartId(30),
        |repair: &mut shooter_sim::repair::Repair| repair.paused = PAUSE_TICKS + 1,
    ] {
        let mut tampered = snapshot.clone();
        let fighter = tampered.fighters.iter_mut().find(|f| f.slot == 1).unwrap();
        tamper(fighter.repair.as_mut().unwrap());
        assert!(decode_snapshot(&encode_snapshot(&tampered)).is_err());
    }
    let mut tampered = snapshot;
    tampered.fighters[1].repairable = 1 << 20;
    assert!(decode_snapshot(&encode_snapshot(&tampered)).is_err());
}

/// B's inputs in the scripted repair match: walking, turning and firing at A, with repair
/// orders every so often (some refused).
fn scripted(tick: u32) -> Intent {
    let parts = [LEG, RIFLE, PartId(6), PartId(30), PartId(9)];
    Intent {
        forward: if tick % 120 < 60 { 1 } else { -1 },
        yaw: TOWARD.wrapping_add(u16::try_from(tick % 200).unwrap() * 40),
        fire: tick % 50 < 10,
        repair: (tick % 45 == 7).then(|| Order::Start(parts[(tick / 45) as usize % parts.len()])),
        ..Intent::IDLE
    }
}

/// A host with B (one person, the mechanic) worn on several parts, and the inputs it follows.
fn worn_host() -> Session {
    let mut host = Session::with_rules(
        duel().arena.clone(),
        Rules {
            slots: 1,
            ..duel().rules
        },
    )
    .unwrap();
    host.join(B).unwrap();
    rebuild(&mut host, 0, MECHANIC);
    host
}

fn queue(session: &mut Session, sequence: u32, intent: Intent) {
    let view_tick = session.ticks();
    let command = decode_input(&encode_input(intent, view_tick, None, None)).unwrap();
    session.queue(
        B,
        Input {
            sequence,
            intent: command.intent,
            view_tick,
        },
    );
}

fn wear_slot0(session: &mut Session, part: PartId, amount: i32) {
    let at = index(session, 0, part);
    let fighter = &mut session.fighters[0];
    let build = fighter.build;
    fighter.parts.hurt(&build, at, amount);
}

#[test]
fn a_client_predicts_the_repair_like_the_host() {
    let mut host = worn_host();
    let mut predictor = Session::new(host.arena.clone()).unwrap();
    let mut sequence = 0;
    let mut repaired = 0;
    for tick in 0..400u32 {
        let intent = scripted(tick);
        sequence += 1;
        queue(&mut host, sequence, intent);
        if tick == 5 {
            for (part, amount) in [(LEG, 25), (RIFLE, 15), (PartId(6), 10), (PartId(9), 8)] {
                wear_slot0(&mut host, part, amount);
            }
        }
        host.step().unwrap();
        let snapshot = decode_snapshot(&encode_snapshot(&host.snapshot())).unwrap();
        let me = snapshot.of_player(B).unwrap().clone();
        if tick == 30 {
            // The predictor resumes from the host's word: robot, parts, energy and repair.
            predictor.fighters[0].parts = me.parts;
            predictor.fighters[0].energy = me.energy;
            predictor.fighters[0].repair = me.repair;
            predictor
                .place(0, me.robot.as_ref().unwrap(), me.motion, me.held, me.hook)
                .unwrap();
        } else if tick > 30 {
            predictor.tick(intent).unwrap();
            let local = predictor.local();
            assert_eq!(local.repair, me.repair, "tick {tick}");
            assert_eq!(local.parts, me.parts, "tick {tick}");
            assert_eq!(local.energy, me.energy, "tick {tick}");
            assert_eq!(local.repairable(), me.repairable, "tick {tick}");
            repaired += u32::from(me.repair.is_some());
        }
    }
    assert!(repaired > 100, "it repaired on {repaired} ticks");
}

#[test]
fn a_repair_match_replays_bit_for_bit() {
    let play = || {
        let mut session = duel();
        let mut bytes = Vec::new();
        let mut healed = 0;
        for tick in 0..900u32 {
            if tick == 20 {
                for (part, amount) in [(LEG, 25), (RIFLE, 15), (PartId(6), 10), (PartId(9), 8)] {
                    wear(&mut session, part, amount);
                }
            }
            // A shoots at B's legs and module now and then: pauses along the way.
            let a = if tick % 30 < 5 {
                let target = if tick % 60 < 30 { LEG } else { MODULE };
                session.fighters[1]
                    .alive()
                    .then(|| {
                        let index = index(&session, 1, target);
                        let at = session.fighters[1].hitboxes().get(index)?.center;
                        let (yaw, pitch) = aim_at(&session, 0, at);
                        Some(Intent {
                            yaw,
                            pitch,
                            fire: true,
                            ..Intent::IDLE
                        })
                    })
                    .flatten()
                    .unwrap_or(Intent::IDLE)
            } else {
                Intent::IDLE
            };
            step(&mut session, a, scripted(tick));
            healed += u32::from(
                session.fighters[1]
                    .repair
                    .is_some_and(|repair| repair.paused == 0 && repair.progress == 0),
            );
            bytes.extend(encode_snapshot(&session.snapshot()));
        }
        (healed, bytes)
    };
    let (healed, bytes) = play();
    assert!(healed > 0);
    assert!(play().1 == bytes, "the match replays bit for bit");
}
