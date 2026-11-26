//! A robot's resources (`shooter_sim::energy`, Atlas D2.i70.3, D2.i89.1 as D2.i123.1–4 amend
//! them, shooter#128): electricity stored by the root chassis and batteries, fuel stored by
//! tanks, generators turning fuel into bounded electricity and nothing recharging by itself;
//! motors paying by their work (nothing at rest), first-come-first-served requests within a tick
//! in their fixed order (motors, thruster jump, blade swing, guns, repair) with no double
//! spending, depletion stopping only what depends on the empty resource, both amounts on the
//! wire and builds that no longer exist (the heat sink) rejected.

use shooter_sim::energy::{ELECTRICITY_PER_FUEL, MILLI, Pool, spend};
use shooter_sim::mechanics::{Layout, load};
use shooter_sim::physics::Vec3i;
use shooter_sim::pickups::{THRUSTER_JUMP_ENERGY, THRUSTER_JUMPS};
use shooter_sim::repair::{ENERGY_PER_TICK, Order};
use shooter_sim::robot::parts::SHOTGUN;
use shooter_sim::robot::{
    ARMORY, Assembly, AssemblyError, HEAVY, MARKSMAN, MECHANIC, PartId, SKIRMISHER, STANDARD,
    TWIN_RAIL,
};
use shooter_sim::session::{walk_activity, walk_draw};
use shooter_sim::weapons::Triggers;
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
        waypoints: Vec::new(),
        placements: Vec::new(),
        repair_anchors: Vec::new(),
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

/// Person A is built as `build` (it fires only what it carries), whole and full.
fn build(session: &mut Session, build: Assembly) {
    session.fighters[0].configured = build;
    session.rebuild(0, build).unwrap();
    let fighter = &mut session.fighters[0];
    fighter.energy = fighter.pool().capacity;
    fighter.fuel = fighter.pool().fuel;
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

/// The standard loadout without its roof's fuel tank and generator.
fn unpowered() -> Assembly {
    let mut candidate = STANDARD.candidate();
    candidate.parts.retain(|part| part.id < 20);
    shooter_sim::robot::BETA.validate(&candidate).unwrap()
}

/// `charge` after one tick of `build`'s generators (its tanks holding plenty).
fn generated(build: &Assembly, charge: u32) -> u32 {
    let mut charge = charge;
    let mut fuel = Pool::of(build).fuel;
    Pool::of(build).generate(&mut charge, &mut fuel);
    charge
}

/// Facing away from B, jumping, swinging and firing the held rifle all in one tick.
const EVERYTHING: Intent = Intent {
    yaw: 32_768,
    jump: true,
    melee: true,
    fire: true,
    ..Intent::IDLE
};

/// Facing away from B and walking forward.
const WALK: Intent = Intent {
    yaw: 32_768,
    forward: 1,
    ..Intent::IDLE
};

/// The electricity a tick of `intent`'s walk takes `build`, whole, on its feet.
fn walk(build: &Assembly, intent: Intent) -> u32 {
    walk_draw(
        &Layout::of(build),
        walk_activity(intent, true),
        load(build, |_| true),
    )
}

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
fn storage_and_generation_come_from_the_parts_and_a_life_starts_full() {
    // The root chassis stores 100, the tank 1000 fuel, the generator burns 12 a second (200
    // milli-units a tick) into twice as much electricity.
    assert_eq!(
        Pool::of(&STANDARD),
        Pool {
            capacity: 100 * MILLI,
            fuel: 1_000 * MILLI,
            burn: 200,
        }
    );
    assert_eq!(Pool::of(&HEAVY).capacity, 160 * MILLI, "a battery stores");
    assert_eq!(Pool::of(&HEAVY).burn, 200, "a battery never generates");
    // Without its tank and generator nothing recharges it.
    assert_eq!(
        Pool::of(&unpowered()),
        Pool {
            capacity: 100 * MILLI,
            fuel: 0,
            burn: 0,
        }
    );

    // Fighters spawn with their own build's pools full (a configured build's at its next spawn:
    // `tests/robot.rs`).
    let session = duel();
    for fighter in &session.fighters {
        assert_eq!(fighter.energy, 100 * MILLI);
        assert_eq!(fighter.fuel, 1_000 * MILLI);
    }
}

/// A generator makes electricity only from fuel, [`ELECTRICITY_PER_FUEL`] for one, bounded by its
/// burn, the fuel there is and the room to store it: a full robot burns nothing.
#[test]
fn a_generator_turns_fuel_into_bounded_electricity() {
    let pool = Pool::of(&STANDARD);
    // (electricity, fuel) before -> (burned, made), and after.
    let cases = [
        // Its burn is the bound.
        ((50_000, 1_000_000), (200, 400)),
        // The room is: 100 milli-units take 50 of fuel.
        ((99_900, 1_000_000), (50, 100)),
        // Less room than one milli-unit of fuel makes: nothing.
        ((99_999, 1_000_000), (0, 0)),
        // Full: nothing.
        ((100_000, 1_000_000), (0, 0)),
        // The fuel is: 30 left make 60.
        ((0, 30), (30, 60)),
        // No fuel: nothing.
        ((0, 0), (0, 0)),
    ];
    for ((charge, fuel), expected) in cases {
        let (mut after, mut left) = (charge, fuel);
        let flow = pool.generate(&mut after, &mut left);
        assert_eq!(flow, expected, "from {charge}, {fuel}");
        assert_eq!(after, charge + expected.1);
        assert_eq!(left, fuel - expected.0);
        assert_eq!(
            expected.1,
            expected.0 * ELECTRICITY_PER_FUEL,
            "no free electricity"
        );
        assert!(after <= pool.capacity);
    }
    // Without a generator nothing is made, whatever the tanks hold.
    let (mut charge, mut fuel) = (0, 1_000_000);
    let mut bare = Pool::of(&STANDARD);
    bare.burn = 0;
    assert_eq!(bare.generate(&mut charge, &mut fuel), (0, 0));
    assert_eq!((charge, fuel), (0, 1_000_000));
}

/// Recharge comes from fuel only: without a generator, or without fuel, an idle robot neither
/// recharges nor drains; with both it refills, burning exactly the fuel the electricity took,
/// and stops burning once full.
#[test]
fn recharge_never_creates_resources_from_nothing_and_idling_costs_nothing() {
    let idle = |session: &mut Session, ticks: u32| {
        let mut sequence = 0;
        for _ in 0..ticks {
            act(
                session,
                &mut sequence,
                Intent {
                    yaw: 32_768,
                    ..Intent::IDLE
                },
            );
        }
    };
    // No generator.
    let mut session = duel();
    build(&mut session, unpowered());
    session.fighters[0].energy = 40 * MILLI;
    idle(&mut session, 300);
    assert_eq!(session.fighters[0].energy, 40 * MILLI);
    assert_eq!(session.fighters[0].flow.used, 0, "standing still is free");

    // A generator without fuel.
    let mut session = duel();
    build(&mut session, STANDARD);
    session.fighters[0].energy = 40 * MILLI;
    session.fighters[0].fuel = 0;
    idle(&mut session, 300);
    assert_eq!(session.fighters[0].energy, 40 * MILLI);

    // With fuel: 60 more electricity at 0.4 a tick take 150 ticks and 30 fuel.
    let mut session = duel();
    build(&mut session, STANDARD);
    session.fighters[0].energy = 40 * MILLI;
    idle(&mut session, 149);
    assert!(session.fighters[0].energy < 100 * MILLI);
    idle(&mut session, 1);
    assert_eq!(session.fighters[0].energy, 100 * MILLI);
    assert_eq!(session.fighters[0].fuel, 970 * MILLI);
    // Full: the generator idles and burns nothing.
    idle(&mut session, 100);
    assert_eq!(session.fighters[0].fuel, 970 * MILLI);
    assert_eq!(session.fighters[0].flow.burned, 0);
}

/// The fixed order within a tick: the motors (nothing to pay standing), a thruster jump, then
/// the blade swing, then the gun; each is granted whole from what the earlier ones left, or
/// refused. The generator runs after them.
#[test]
fn simultaneous_requests_exhaust_the_pool_in_a_fixed_order() {
    let blade = Weapon::EnergyBlade.energy();
    let rifle = Weapon::EnergyRifle.energy();
    let after = |left| generated(&STANDARD, left);
    // (charge before the tick, boosted jump?, what fired, charge after the tick's generation)
    let cases = [
        // Enough for everything: the jump and the swing; the swing keeps the gun down.
        (
            THRUSTER_JUMP_ENERGY + blade + rifle,
            true,
            vec![Weapon::EnergyBlade],
            after(rifle),
        ),
        // The jump takes 15, the swing is refused, the gun gets its turn with the 2 left.
        (
            THRUSTER_JUMP_ENERGY + rifle,
            true,
            vec![Weapon::EnergyRifle],
            after(0),
        ),
        // The jump takes 15; neither the swing nor the gun can be paid.
        (THRUSTER_JUMP_ENERGY + 1_000, true, vec![], after(1_000)),
        // The jump cannot be paid (a plain jump, the use kept); the swing can.
        (
            THRUSTER_JUMP_ENERGY - 1,
            false,
            vec![Weapon::EnergyBlade],
            after(THRUSTER_JUMP_ENERGY - 1 - blade),
        ),
        // Nothing can be paid: a plain jump, no swing, no shot; only the generator.
        (1_000, false, vec![], after(1_000)),
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

/// An empty pool stops the gun without spending its cooldown; the generator restarts it as soon
/// as a shot can be paid, and the charge never goes below zero.
#[test]
fn the_generator_restarts_a_starved_weapon() {
    let mut session = duel();
    build(&mut session, ARMORY);
    let mut sequence = 0;
    let rail = Intent {
        yaw: 32_768,
        select: Some(Weapon::RailGun.into()),
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
    // 30 at 0.4 a tick from the generator (the legs standing cost nothing): 75 ticks refill it,
    // so the first shot goes out then, and one every 75 ticks (the rail gun's own 72 is
    // shorter: the pool is the limit).
    assert_eq!(fired_at, [75, 150, 225]);

    // The laser drains the pool (0.6 a tick against 0.4) and, once it is dry, sputters at what
    // the generator makes: it never fires on a charge it cannot pay.
    let mut session = duel();
    build(&mut session, ARMORY);
    let mut sequence = 0;
    let laser = Intent {
        yaw: 32_768,
        select: Some(Weapon::Laser.into()),
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
    // 100 at a net 0.2 a tick: a full pool holds the beam for 498 ticks, and one of the next two
    // fires on what the generator made.
    assert_eq!(beams, 499);
    let mut sputter = Vec::new();
    for _ in 0..12 {
        sputter.push(act(&mut session, &mut sequence, hold).len());
    }
    assert!(sputter.contains(&0) && sputter.contains(&1), "{sputter:?}");
    assert!(session.fighters[0].energy < Weapon::Laser.energy() + 400);
    // Let go: the pool refills to its capacity and stops there.
    for _ in 0..400 {
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
    for (loadout, weapon, cost) in [
        (ARMORY, Weapon::RailGun, 30_000),
        (MARKSMAN, Weapon::RailGun, 45_000),
        (SKIRMISHER, Weapon::Shotgun, 7_200),
    ] {
        let mut session = duel();
        build(&mut session, loadout);
        let mut sequence = 0;
        let aim = Intent {
            yaw: 32_768,
            select: Some(weapon.into()),
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
        assert_eq!(session.fighters[0].flow.used, cost, "{weapon:?}");
        assert_eq!(
            session.fighters[0].energy,
            generated(&loadout, before - cost),
            "{weapon:?}"
        );
    }
}

/// The locomotion pays for the walk the movement drive carries it on, by speed and load:
/// standing is free, sprinting costs more than walking, a heavier robot more than a lighter one.
#[test]
fn walking_costs_by_speed_and_load_and_standing_is_free() {
    let one_tick = |loadout: Assembly, intent: Intent| {
        let mut session = duel();
        build(&mut session, loadout);
        let before = session.fighters[0].feet();
        let mut sequence = 0;
        act(&mut session, &mut sequence, intent);
        let after = session.fighters[0].feet();
        let moved = (after[0] - before[0]).abs() + (after[2] - before[2]).abs();
        (session.fighters[0].flow, moved)
    };
    let (stand, _) = one_tick(
        STANDARD,
        Intent {
            yaw: 32_768,
            ..Intent::IDLE
        },
    );
    assert_eq!(stand.used, 0);
    let (walking, moved) = one_tick(STANDARD, WALK);
    assert!(moved > 0.0);
    assert_eq!(walking.used, walk(&STANDARD, WALK));
    // Two legs at 4 a second each at their rated 200 kg, at a walk's 685 thousandths of a
    // sprint, under the standard robot's 167 kg: 39 milli-units a tick each, rounded up.
    assert_eq!(load(&STANDARD, |_| true), 167);
    assert_eq!(walking.used, 2 * 39);
    let sprint = Intent {
        sprint: true,
        ..WALK
    };
    let (sprinting, _) = one_tick(STANDARD, sprint);
    assert_eq!(sprinting.used, walk(&STANDARD, sprint));
    assert!(sprinting.used > walking.used);
    let (heavy, _) = one_tick(HEAVY, WALK);
    assert!(load(&HEAVY, |_| true) > load(&STANDARD, |_| true));
    assert!(heavy.used > walking.used);
    assert!(!walking.stalled && !sprinting.stalled && !heavy.stalled);
}

/// Motors, guns and a repair in one tick share one pool first come, first served: each request is
/// granted whole from what the earlier ones left, never paid twice, and the pool never goes
/// below zero.
#[test]
fn concurrent_motors_weapons_and_repair_never_double_spend() {
    // Walking while both rail guns fire: the walk and the first rail gun can be paid, the second
    // cannot.
    let mut session = duel();
    build(&mut session, TWIN_RAIL);
    let walk_cost = walk(&TWIN_RAIL, WALK);
    let rail = Weapon::RailGun.energy();
    session.fighters[0].energy = walk_cost + rail + rail - 1;
    let mut sequence = 0;
    let triggers: Triggers = [PartId(7), PartId(9)].into_iter().collect();
    let fired = act(&mut session, &mut sequence, Intent { triggers, ..WALK });
    assert_eq!(fired, [Weapon::RailGun]);
    let flow = session.fighters[0].flow;
    assert!(!flow.stalled);
    assert_eq!(flow.used, walk_cost + rail);
    assert_eq!(session.fighters[0].energy, generated(&TWIN_RAIL, rail - 1));

    // Walking, firing the rifle and repairing a leg: the walk and the shot are paid, the repair
    // tick (asked after every hit) is refused for the one milli-unit it lacks and does no work;
    // the next tick, with the generator's output, it works.
    let mut session = duel();
    build(&mut session, MECHANIC);
    let fighter = &mut session.fighters[0];
    let leg = fighter
        .build
        .parts()
        .iter()
        .position(|part| part.id == PartId(5))
        .unwrap();
    let assembly = fighter.build;
    fighter.parts.hurt(&assembly, leg, 10);
    let mut sequence = 0;
    let order = Intent {
        repair: Some(Order::Start(PartId(5))),
        ..WALK
    };
    act(&mut session, &mut sequence, order);
    assert!(session.fighters[0].repair.is_some(), "the order took");
    for _ in 0..10 {
        act(&mut session, &mut sequence, WALK);
    }
    let walk_cost = walk(&MECHANIC, WALK);
    let rifle = Weapon::EnergyRifle.energy();
    session.fighters[0].energy = walk_cost + rifle + ENERGY_PER_TICK - 1;
    let work = session.fighters[0].repair.unwrap();
    let fired = act(&mut session, &mut sequence, Intent { fire: true, ..WALK });
    assert_eq!(fired, [Weapon::EnergyRifle]);
    assert_eq!(session.fighters[0].flow.used, walk_cost + rifle);
    assert_eq!(session.fighters[0].repair.unwrap(), work, "no work unpaid");
    act(&mut session, &mut sequence, WALK);
    assert_eq!(
        session.fighters[0].flow.used,
        walk_cost + ENERGY_PER_TICK,
        "the walk and the repair"
    );
    assert_ne!(session.fighters[0].repair.unwrap(), work);
}

/// Depletion stops only what depends on the empty resource (D2.i123.4): with no fuel the
/// generator stops while the stored electricity still walks and fires; with no electricity the
/// walk and the guns stop while the generator still works and restarts them; with neither the
/// robot stands, unharmed and alive.
#[test]
fn depleting_one_resource_stops_only_what_depends_on_it() {
    // No fuel: the electricity it has still walks and fires; nothing recharges it.
    let mut session = duel();
    build(&mut session, STANDARD);
    session.fighters[0].fuel = 0;
    let mut sequence = 0;
    let start = session.fighters[0].feet();
    let mut shots = 0;
    let mut last = session.fighters[0].energy;
    for _ in 0..60 {
        shots += act(&mut session, &mut sequence, Intent { fire: true, ..WALK }).len();
        let flow = session.fighters[0].flow;
        assert_eq!((flow.burned, flow.made), (0, 0));
        assert!(!flow.stalled);
        assert!(session.fighters[0].energy < last);
        last = session.fighters[0].energy;
    }
    assert_eq!(shots, 10, "the rifle's ten shots a second");
    assert!(
        (session.fighters[0].feet()[2] - start[2]).abs() > 50.0,
        "it walked"
    );

    // No electricity: the walk is refused (it moves exactly as if it stood still) and the gun
    // too, while the generator burns fuel; the next tick its output pays the walk.
    let drained = || {
        let mut session = duel();
        build(&mut session, STANDARD);
        session.fighters[0].energy = 0;
        session
    };
    let mut still = drained();
    act(
        &mut still,
        &mut 0,
        Intent {
            yaw: WALK.yaw,
            ..Intent::IDLE
        },
    );
    let mut session = drained();
    let mut sequence = 0;
    let fired = act(&mut session, &mut sequence, Intent { fire: true, ..WALK });
    assert!(fired.is_empty());
    let flow = session.fighters[0].flow;
    assert!(flow.stalled);
    assert_eq!(flow.used, 0);
    assert_eq!((flow.burned, flow.made), (200, 400));
    assert_eq!(session.fighters[0].feet(), still.fighters[0].feet());
    act(&mut session, &mut sequence, WALK);
    let flow = session.fighters[0].flow;
    assert!(!flow.stalled);
    assert_eq!(flow.used, walk(&STANDARD, WALK));

    // Neither: it stands where it is, aims, and lives; nothing drains or kills it.
    let mut session = duel();
    build(&mut session, STANDARD);
    session.fighters[0].energy = 0;
    session.fighters[0].fuel = 0;
    let mut sequence = 0;
    let health = session.fighters[0].chassis_health();
    for _ in 0..120 {
        let fired = act(&mut session, &mut sequence, Intent { fire: true, ..WALK });
        assert!(fired.is_empty());
        assert!(session.fighters[0].flow.stalled);
    }
    let fighter = &session.fighters[0];
    assert!(fighter.alive());
    assert_eq!(fighter.chassis_health(), health);
    assert_eq!((fighter.energy, fighter.fuel), (0, 0));
}

#[test]
fn both_resources_travel_in_snapshots_and_are_checked_against_the_build() {
    let mut session = duel();
    session.fighters[0].energy = 42_400;
    session.fighters[0].fuel = 123_456;
    let snapshot = session.snapshot();
    assert_eq!(snapshot.slot(0).unwrap().energy, 42_400);
    assert_eq!(snapshot.slot(0).unwrap().fuel, 123_456);
    let robot = snapshot.slot(0).unwrap().robot.as_ref().unwrap();
    assert_eq!((robot.charge, robot.fuel), (42_400, 123_456));
    let bytes = encode_snapshot(&snapshot);
    let decoded = decode_snapshot(&bytes).unwrap();
    assert_eq!(decoded.slot(0).unwrap().fuel, 123_456);
    assert_eq!(decoded, snapshot);

    // A charge above what the build can hold is malformed, and so is fuel above its tanks.
    session.fighters[0].energy = 100 * MILLI + 1;
    let bytes = encode_snapshot(&session.snapshot());
    assert!(matches!(
        decode_snapshot(&bytes),
        Err(WireError::Malformed(_))
    ));
    session.fighters[0].energy = 100 * MILLI;
    session.fighters[0].fuel = 1_000 * MILLI + 1;
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
    // Re-encode with the retired code by patching the shotgun's mod byte: after its ID, its
    // type, its flags, its parent and its socket.
    let coil = forged
        .windows(2)
        .position(|pair| pair == [10, SHOTGUN.id.0])
        .unwrap()
        + 5;
    assert_eq!(forged[coil], 5);
    forged[coil] = 4;
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

/// A bot match: every living robot's electricity and fuel stay within its build's pools every
/// tick, the bots spend electricity and their generators burn fuel for it, and the same seed does
/// it the same way.
#[test]
fn a_bot_match_keeps_every_pool_in_bounds_and_deterministic() {
    let run = || {
        let mut session = Session::on_map(MapId::Greybox, Rules::deathmatch(7)).unwrap();
        let mut trace = Vec::new();
        let (mut spent, mut burned) = (false, false);
        for _ in 0..1_800 {
            session.step().unwrap();
            for fighter in session.fighters.iter().filter(|fighter| fighter.alive()) {
                let pool = fighter.pool();
                assert!(fighter.energy <= pool.capacity);
                assert!(fighter.fuel <= pool.fuel);
                let flow = fighter.flow;
                assert_eq!(flow.made, flow.burned * ELECTRICITY_PER_FUEL);
                spent |= flow.used > 0;
                burned |= flow.burned > 0;
            }
            trace.push(
                session
                    .fighters
                    .iter()
                    .map(|fighter| (fighter.energy, fighter.fuel))
                    .collect::<Vec<_>>(),
            );
        }
        assert!(spent, "the bots walk and fire, so their pools drain");
        assert!(burned, "and their generators refill them from fuel");
        trace
    };
    assert_eq!(run(), run());
}
