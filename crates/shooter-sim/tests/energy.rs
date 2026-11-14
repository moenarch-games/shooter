//! The shared robot energy pool (`shooter_sim::energy`, Atlas D2.i70.3): the core's capacity and
//! regeneration and the batteries' stored capacity, first-come-first-served requests within a
//! tick in their fixed order (thruster jump, blade swing, gun), refused requests that leave the
//! charge untouched, regeneration restarting a starved weapon, the charge on the wire and
//! builds that no longer exist (the heat sink) rejected.

use shooter_sim::energy::{MILLI, Pool, spend};
use shooter_sim::physics::Vec3i;
use shooter_sim::pickups::{THRUSTER_JUMP_ENERGY, THRUSTER_JUMPS};
use shooter_sim::robot::{AssemblyError, HEAVY, MARKSMAN, SKIRMISHER, STANDARD};
use shooter_sim::wire::{WireError, decode_input, decode_snapshot, encode_input, encode_snapshot};
use shooter_sim::{
    Arena, ArenaModule, Carried, Event, Input, Intent, MapId, Rules, Session, Spawn, Weapon,
};

const A: u32 = 1;
const B: u32 = 2;

/// Two people 12 m apart on an open floor, no bots, no pads.
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
        frag_limit: 50,
        time_limit_ticks: 0,
        intermission_ticks: 60,
        respawn_ticks: 30,
    };
    let mut session = Session::with_rules(arena, rules).unwrap();
    assert_eq!(session.join(A).unwrap(), Some(0));
    assert_eq!(session.join(B).unwrap(), Some(1));
    // Settle on the floor.
    for _ in 0..30 {
        session.step().unwrap();
    }
    assert!(session.fighters[0].grounded());
    session
}

/// Person A's `intent` for one tick; the weapons slot 0 fired that tick.
fn act(session: &mut Session, sequence: &mut u32, intent: Intent) -> Vec<Weapon> {
    *sequence += 1;
    let view_tick = session.ticks();
    session.queue(
        A,
        Input {
            sequence: *sequence,
            intent,
            view_tick,
        },
    );
    session.step().unwrap();
    session
        .events
        .iter()
        .filter_map(|event| match *event {
            Event::Fired {
                slot: 0, weapon, ..
            } => Some(weapon),
            _ => None,
        })
        .collect()
}

/// Facing away from B, jumping, swinging and firing the held rifle all in one tick.
const EVERYTHING: Intent = Intent {
    yaw: 32_768,
    jump: true,
    melee: true,
    fire: true,
    ..Intent::IDLE
};

#[test]
fn a_request_is_granted_whole_or_not_at_all() {
    let mut charge = 7_000;
    assert!(spend(&mut charge, 5_000));
    assert_eq!(charge, 2_000);
    assert!(!spend(&mut charge, 2_001), "no part grants, no debt");
    assert_eq!(charge, 2_000);
    assert!(spend(&mut charge, 2_000));
    assert_eq!(charge, 0);
    assert!(!spend(&mut charge, 1));
    assert!(spend(&mut charge, 0));
    assert_eq!(charge, 0);
}

#[test]
fn the_core_and_batteries_set_the_pool_and_a_life_starts_full() {
    assert_eq!(
        Pool::of(&STANDARD),
        Pool {
            capacity: 100 * MILLI,
            regen: 400
        }
    );
    assert_eq!(Pool::of(&HEAVY).capacity, 160 * MILLI);
    assert_eq!(
        Pool::of(&HEAVY).regen,
        400,
        "a battery stores, never regenerates"
    );
    // Regeneration stops at the capacity.
    let mut charge = 99_800;
    Pool::of(&STANDARD).regenerate(&mut charge);
    assert_eq!(charge, 100 * MILLI);
    Pool::of(&STANDARD).regenerate(&mut charge);
    assert_eq!(charge, 100 * MILLI);

    // Fighters spawn with their own build's pool full (a configured build's at its next spawn:
    // `tests/robot.rs`).
    let session = duel();
    assert_eq!(session.fighters[0].energy, 100 * MILLI);
    assert_eq!(session.fighters[1].energy, 100 * MILLI);
}

/// The fixed order within a tick: a thruster jump asks first, then the blade swing, then the gun;
/// each is granted whole from what the earlier ones left, or refused.
#[test]
fn simultaneous_requests_exhaust_the_pool_in_a_fixed_order() {
    let blade = Weapon::EnergyBlade.energy();
    let rifle = Weapon::EnergyRifle.energy();
    let regen = Pool::of(&STANDARD).regen;
    // (charge before the tick, boosted jump?, what fired, charge after the tick's regeneration)
    let cases = [
        // Enough for everything: the jump and the swing; the swing keeps the gun down.
        (
            THRUSTER_JUMP_ENERGY + blade + rifle,
            true,
            vec![Weapon::EnergyBlade],
            rifle + regen,
        ),
        // The jump takes 15, the swing is refused, the gun gets its turn with the 2 left.
        (
            THRUSTER_JUMP_ENERGY + rifle,
            true,
            vec![Weapon::EnergyRifle],
            regen,
        ),
        // The jump takes 15; neither the swing nor the gun can be paid.
        (THRUSTER_JUMP_ENERGY + 1_000, true, vec![], 1_000 + regen),
        // The jump cannot be paid (a plain jump, the use kept); the swing can.
        (
            THRUSTER_JUMP_ENERGY - 1,
            false,
            vec![Weapon::EnergyBlade],
            THRUSTER_JUMP_ENERGY - 1 - blade + regen,
        ),
        // Nothing can be paid: a plain jump, no swing, no shot; only regeneration.
        (1_000, false, vec![], 1_000 + regen),
    ];
    for (charge, boosted, fired, after) in cases {
        let mut session = duel();
        session.fighters[0].carried = Some(Carried::fresh(ArenaModule::Thrusters));
        session.fighters[0].energy = charge;
        let mut sequence = 0;
        let shots = act(&mut session, &mut sequence, EVERYTHING);
        assert_eq!(shots, fired, "from {charge}");
        assert_eq!(session.fighters[0].energy, after, "from {charge}");
        let uses = if boosted {
            THRUSTER_JUMPS - 1
        } else {
            THRUSTER_JUMPS
        };
        assert_eq!(
            session.fighters[0].carried.map(|carried| carried.uses_left),
            Some(uses),
            "from {charge}"
        );
        // It jumped either way.
        assert!(!session.fighters[0].grounded(), "from {charge}");
    }
}

/// An empty pool stops the gun without spending its cooldown; regeneration restarts it as soon as
/// a shot can be paid, and the charge never goes below zero.
#[test]
fn regeneration_restarts_a_starved_weapon() {
    let mut session = duel();
    let mut sequence = 0;
    let rail = Intent {
        yaw: 32_768,
        select: Some(Weapon::RailGun),
        ..Intent::IDLE
    };
    act(&mut session, &mut sequence, rail);
    for _ in 0..20 {
        act(
            &mut session,
            &mut sequence,
            Intent {
                select: None,
                ..rail
            },
        );
    }
    session.fighters[0].energy = 0;
    let hold = Intent {
        fire: true,
        select: None,
        ..rail
    };
    let mut fired_at = Vec::new();
    for tick in 0..300u32 {
        let before = session.fighters[0].energy;
        if act(&mut session, &mut sequence, hold).contains(&Weapon::RailGun) {
            fired_at.push(tick);
            assert!(before >= Weapon::RailGun.energy());
        }
    }
    // 30 energy at 0.4 a tick, regenerated after each tick's requests: 75 ticks refill it, so
    // the first shot goes out on the tick after, then one every 75 ticks (the rail gun's own 72
    // is shorter: the pool is the limit).
    assert_eq!(fired_at, [75, 150, 225]);

    // The laser drains the pool (0.6 a tick against 0.4) and, once it is dry, sputters at what
    // regeneration pays: it never fires on a charge it cannot pay.
    let mut session = duel();
    let mut sequence = 0;
    let laser = Intent {
        yaw: 32_768,
        select: Some(Weapon::Laser),
        ..Intent::IDLE
    };
    act(&mut session, &mut sequence, laser);
    for _ in 0..20 {
        act(
            &mut session,
            &mut sequence,
            Intent {
                select: None,
                ..laser
            },
        );
    }
    let hold = Intent {
        fire: true,
        select: None,
        ..laser
    };
    let mut beams = 0;
    for _ in 0..500 {
        beams += act(&mut session, &mut sequence, hold).len();
    }
    // 100 at a net 0.2 a tick: a full pool holds the beam for about 500 ticks.
    assert_eq!(beams, 499);
    let mut sputter = Vec::new();
    for _ in 0..12 {
        sputter.push(act(&mut session, &mut sequence, hold).len());
    }
    assert!(sputter.contains(&0) && sputter.contains(&1), "{sputter:?}");
    assert!(session.fighters[0].energy < Weapon::Laser.energy() + 400);
    // Let go: the pool refills to its capacity and stops there.
    for _ in 0..300 {
        act(
            &mut session,
            &mut sequence,
            Intent {
                fire: false,
                ..hold
            },
        );
    }
    assert_eq!(session.fighters[0].energy, 100 * MILLI);
}

/// Mods and modules change what a shot costs: the overcharger half again, the efficiency coil 60 %.
#[test]
fn tuned_weapons_pay_their_tuned_cost() {
    for (build, weapon, cost) in [
        (STANDARD, Weapon::RailGun, 30_000),
        (MARKSMAN, Weapon::RailGun, 45_000),
        (SKIRMISHER, Weapon::Shotgun, 7_200),
    ] {
        let mut session = duel();
        session.fighters[0].build = build;
        session.fighters[0].energy = Pool::of(&build).capacity;
        let mut sequence = 0;
        let aim = Intent {
            yaw: 32_768,
            select: Some(weapon),
            ..Intent::IDLE
        };
        act(&mut session, &mut sequence, aim);
        for _ in 0..20 {
            act(
                &mut session,
                &mut sequence,
                Intent {
                    select: None,
                    ..aim
                },
            );
        }
        let before = session.fighters[0].energy;
        let fired = act(
            &mut session,
            &mut sequence,
            Intent {
                fire: true,
                select: None,
                ..aim
            },
        );
        assert_eq!(fired, [weapon]);
        let regen = Pool::of(&build).regen;
        assert_eq!(
            session.fighters[0].energy,
            (before - cost + regen).min(Pool::of(&build).capacity),
            "{weapon:?}"
        );
    }
}

#[test]
fn the_charge_travels_in_snapshots_and_is_checked_against_the_build() {
    let mut session = duel();
    session.fighters[0].energy = 42_400;
    let snapshot = session.snapshot();
    assert_eq!(snapshot.slot(0).unwrap().energy, 42_400);
    let bytes = encode_snapshot(&snapshot);
    let decoded = decode_snapshot(&bytes).unwrap();
    assert_eq!(decoded.slot(0).unwrap().energy, 42_400);
    assert_eq!(decoded, snapshot);

    // A charge above what the build can hold is malformed.
    session.fighters[0].energy = 100 * MILLI + 1;
    let bytes = encode_snapshot(&session.snapshot());
    assert!(matches!(
        decode_snapshot(&bytes),
        Err(WireError::Malformed(_))
    ));
}

/// Builds naming what no longer exists are refused: the heat sink's retired code.
#[test]
fn a_build_with_the_retired_heat_sink_is_rejected() {
    let mut candidate = SKIRMISHER.candidate();
    let shotgun = candidate
        .parts
        .iter_mut()
        .find(|part| part.modification == Some(5))
        .unwrap();
    shotgun.modification = Some(4);
    let plain = encode_input(Intent::IDLE, 3, None, None);
    let mut forged = encode_input(Intent::IDLE, 3, None, Some(&SKIRMISHER));
    let valid = forged.clone();
    // Re-encode with the retired code by patching the mod byte of the build's last part.
    let last = forged.len() - 1;
    assert_eq!(forged[last], 5);
    forged[last] = 4;
    assert_eq!(
        decode_input(&forged),
        Err(WireError::Build(AssemblyError::UnknownMod(4)))
    );
    assert!(decode_input(&valid).is_ok());
    assert!(plain.len() < valid.len());
    assert_eq!(
        shooter_sim::robot::BETA.validate(&candidate),
        Err(AssemblyError::UnknownMod(4))
    );
}

/// A bot match: every living robot's charge stays within its build's pool every tick, the bots
/// do spend it, and the same seed spends it the same way.
#[test]
fn a_bot_match_keeps_every_pool_in_bounds_and_deterministic() {
    let run = || {
        let mut session = Session::on_map(MapId::DEFAULT, Rules::deathmatch(7)).unwrap();
        let mut trace = Vec::new();
        let mut spent = false;
        for _ in 0..1_800 {
            session.step().unwrap();
            for fighter in session.fighters.iter().filter(|fighter| fighter.alive()) {
                assert!(fighter.energy <= fighter.pool().capacity);
                spent |= fighter.energy < fighter.pool().capacity;
            }
            trace.push(
                session
                    .fighters
                    .iter()
                    .map(|fighter| fighter.energy)
                    .collect::<Vec<_>>(),
            );
        }
        assert!(spent, "the bots fire, so their pools drain");
        trace
    };
    assert_eq!(run(), run());
}
