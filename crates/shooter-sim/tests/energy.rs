//! A robot's resources (`shooter_sim::energy`, Atlas D2.i70.3, D2.i89.1, D2.i123.1–.4;
//! shooter#72 migrated by shooter#128): electricity and fuel stores from the root chassis,
//! batteries and tanks; generation only from installed generators (burning fuel, never creating
//! electricity from nothing) and solar panels; first-come-first-served requests within a tick in
//! their one fixed order (thruster jump, blade swing, guns by part, motors), refused requests that
//! leave the stores untouched, one store running dry while the other still works, concurrent
//! firing and locomotion paid once each, the stores on the wire, and builds that no longer exist
//! (the heat sink) rejected.

use shooter_sim::energy::{Flow, Generated, MILLI, Pool, Resource, request, spend};
use shooter_sim::physics::Vec3i;
use shooter_sim::pickups::{THRUSTER_JUMP_ENERGY, THRUSTER_JUMPS};
use shooter_sim::robot::parts::{CHASSIS_S, SOLAR_PANEL};
use shooter_sim::robot::{
    ARMORY, AssemblyError, BETA, Face, HEAVY, MARKSMAN, PYRO, PartId, RawMount, RawPart,
    SKIRMISHER, STANDARD,
};
use shooter_sim::weapons::Triggers;
use shooter_sim::wire::{WireError, decode_input, decode_snapshot, encode_input, encode_snapshot};
use shooter_sim::{
    Arena, ArenaModule, Assembly, Carried, Event, Input, Intent, MapId, Rules, Session, Spawn,
    Weapon,
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
fn built(session: &mut Session, build: Assembly) {
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

/// A takes up `weapon` facing away from B and waits until it may fire.
fn take(session: &mut Session, sequence: &mut u32, weapon: Weapon) -> Intent {
    let aim = Intent {
        yaw: 32_768,
        select: Some(weapon.into()),
        ..Intent::IDLE
    };
    act(session, sequence, aim);
    let aim = Intent {
        select: None,
        ..aim
    };
    for _ in 0..80 {
        act(session, sequence, aim);
    }
    aim
}

/// The electricity A's motors used last tick (after their refunds).
fn motors(session: &Session) -> u32 {
    session.fighters[0].flow.motors
}

/// What A's requests other than its motors paid last tick: the electricity it spent less its
/// motors' share.
fn paid(session: &Session) -> u32 {
    session.fighters[0].flow.spent - motors(session)
}

/// Facing away from B, jumping, swinging and firing the held rifle all in one tick.
const EVERYTHING: Intent = Intent {
    yaw: 32_768,
    jump: true,
    melee: true,
    fire: true,
    ..Intent::IDLE
};

/// A build of the small chassis alone with `extra` parts on its back, valid in [`BETA`].
fn chassis_with(extra: &[(u8, &shooter_sim::robot::PartType, u8, u8)]) -> Assembly {
    let mut parts = vec![RawPart {
        id: 0,
        kind: CHASSIS_S.id.0,
        mount: None,
        modification: None,
    }];
    for &(id, kind, u, v) in extra {
        parts.push(RawPart {
            id,
            kind: kind.id.0,
            mount: Some(RawMount {
                parent: 0,
                socket: CHASSIS_S.socket(Face::NegX, u, v).unwrap().0,
                yaw: 0,
                pitch: 0,
                roll: 0,
            }),
            modification: None,
        });
    }
    BETA.validate(&shooter_sim::robot::Candidate { parts })
        .unwrap()
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
    // A request asks the store of its own resource, and only that one.
    let (mut charge, mut fuel) = (0, 3_000);
    assert!(!request(&mut charge, &mut fuel, Resource::Electricity, 1));
    assert!(request(&mut charge, &mut fuel, Resource::Fuel, 2_000));
    assert_eq!((charge, fuel), (0, 1_000));
    assert_eq!(Weapon::Flamethrower.resource(), Resource::Fuel);
    for weapon in Weapon::ALL
        .into_iter()
        .filter(|&w| w != Weapon::Flamethrower)
    {
        assert_eq!(weapon.resource(), Resource::Electricity, "{weapon:?}");
    }
}

#[test]
fn installed_parts_set_the_stores_and_a_life_starts_full() {
    // The root chassis' 100 electricity and no generation of its own; the named loadouts'
    // generator (20 a second for 2 fuel) and 100-fuel tank; a battery stores, never generates.
    assert_eq!(
        Pool::of(&STANDARD),
        Pool {
            capacity: 100 * MILLI,
            fuel: 100 * MILLI,
            solar: 0,
            output: 20,
            burn: 2,
        }
    );
    assert_eq!(Pool::of(&HEAVY).capacity, 160 * MILLI);
    assert_eq!(Pool::of(&HEAVY).output, 20);
    assert_eq!(Pool::of(&PYRO).fuel, 200 * MILLI, "the pyro's second tank");
    let bare = chassis_with(&[]);
    assert_eq!(
        Pool::of(&bare),
        Pool {
            capacity: 100 * MILLI,
            ..Pool::default()
        },
        "the root chassis generates nothing"
    );

    // Fighters spawn with their own build's stores full (a configured build's at its next
    // spawn: `tests/robot.rs`).
    let session = duel();
    for fighter in &session.fighters[..2] {
        assert_eq!(fighter.energy, 100 * MILLI);
        // Settling, its legs' servos drew a little, which the generator refilled from the tank.
        assert!(
            (99 * MILLI..100 * MILLI).contains(&fighter.fuel),
            "{}",
            fighter.fuel
        );
    }
}

/// A generator turns fuel into electricity at its fixed rate (10 a unit) for the room there is,
/// at most its output a tick, and stops without fuel; a panel gives its little for nothing;
/// nothing else creates electricity.
#[test]
fn generation_burns_fuel_and_never_creates_from_nothing() {
    let pool = Pool::of(&STANDARD);
    // Room and fuel: its full output, 20 a second (333 a tick), for the fuel it costs (rounded
    // up against the robot).
    let (mut charge, mut fuel) = (0, 100 * MILLI);
    let made = pool.generate(&mut charge, &mut fuel);
    assert_eq!(
        made,
        Generated {
            electricity: 333,
            burned: 34,
            starved: false
        }
    );
    assert_eq!((charge, fuel), (333, 100 * MILLI - 34));
    // Little room: only what fits, and only its fuel.
    let (mut charge, mut fuel) = (100 * MILLI - 100, 100 * MILLI);
    assert_eq!(pool.generate(&mut charge, &mut fuel).burned, 10);
    assert_eq!((charge, fuel), (100 * MILLI, 100 * MILLI - 10));
    // Full: idle, nothing burned.
    assert_eq!(pool.generate(&mut charge, &mut fuel), Generated::default());
    // Little fuel: what it buys, and all of it.
    let (mut charge, mut fuel) = (0, 20);
    assert_eq!(pool.generate(&mut charge, &mut fuel).electricity, 200);
    assert_eq!((charge, fuel), (200, 0));
    // No fuel: it cannot operate.
    let made = pool.generate(&mut charge, &mut fuel);
    assert_eq!(
        made,
        Generated {
            electricity: 0,
            burned: 0,
            starved: true
        }
    );
    assert_eq!(charge, 200);

    // Over a long drain and refill, the electricity made is never more than its fuel buys.
    let (mut charge, mut fuel) = (0, 100 * MILLI);
    let (mut made, mut burned) = (0, 0);
    for tick in 0u32..2_000 {
        if tick % 7 == 0 {
            spend(&mut charge, 1_500);
        }
        let generated = pool.generate(&mut charge, &mut fuel);
        made += generated.electricity;
        burned += generated.burned;
        assert!(charge <= pool.capacity);
    }
    assert!(made > 0 && fuel < 100 * MILLI);
    assert_eq!(burned, 100 * MILLI - fuel);
    assert!(made * 2 <= burned * 20, "{made} for {burned}");

    // A solar panel (3 a second: 50 a tick) needs no fuel; a chassis with only a battery makes
    // nothing at all.
    let panel = Pool::of(&chassis_with(&[(1, &SOLAR_PANEL, 0, 0)]));
    let (mut charge, mut fuel) = (0, 0);
    assert_eq!(panel.generate(&mut charge, &mut fuel).electricity, 50);
    assert_eq!((charge, fuel), (50, 0));
    let stored = Pool::of(&chassis_with(&[]));
    let mut charge = 0;
    for _ in 0..600 {
        stored.generate(&mut charge, &mut fuel);
    }
    assert_eq!(charge, 0, "no free regeneration");
}

/// The fixed order within a tick: a thruster jump asks first, then the blade swing, then the
/// gun, then the motors, each granted whole from what the earlier ones left, or refused; the
/// generator refills after them all.
#[test]
fn simultaneous_requests_exhaust_the_pool_in_a_fixed_order() {
    let blade = Weapon::EnergyBlade.energy();
    let rifle = Weapon::EnergyRifle.energy();
    // (charge before the tick, boosted jump?, what fired, what the requests before the motors
    // paid)
    let cases = [
        // Enough for everything: the jump and the swing; the swing keeps the gun down.
        (
            THRUSTER_JUMP_ENERGY + blade + rifle,
            true,
            vec![Weapon::EnergyBlade],
            THRUSTER_JUMP_ENERGY + blade,
        ),
        // The jump takes 15, the swing is refused, the gun gets its turn with the 2 left.
        (
            THRUSTER_JUMP_ENERGY + rifle,
            true,
            vec![Weapon::EnergyRifle],
            THRUSTER_JUMP_ENERGY + rifle,
        ),
        // The jump takes 15; neither the swing nor the gun can be paid.
        (
            THRUSTER_JUMP_ENERGY + 1_000,
            true,
            vec![],
            THRUSTER_JUMP_ENERGY,
        ),
        // The jump cannot be paid (a plain jump, the use kept); the swing can.
        (
            THRUSTER_JUMP_ENERGY - 1,
            false,
            vec![Weapon::EnergyBlade],
            blade,
        ),
        // Nothing can be paid: a plain jump, no swing, no shot.
        (1_000, false, vec![], 0),
    ];
    for (charge, boosted, fired, cost) in cases {
        let mut session = duel();
        session.fighters[0].carried = Some(Carried::fresh(ArenaModule::Thrusters));
        session.fighters[0].energy = charge;
        let mut sequence = 0;
        let shots = act(&mut session, &mut sequence, EVERYTHING);
        assert_eq!(shots, fired, "from {charge}");
        assert_eq!(paid(&session), cost, "from {charge}");
        // The motors had what was left, and the generator refilled after them.
        let flow = session.fighters[0].flow;
        assert!(motors(&session) <= charge - cost, "from {charge}");
        assert_eq!(
            session.fighters[0].energy,
            charge - flow.spent + flow.generated.electricity,
            "from {charge}"
        );
        assert!(flow.generated.electricity > 0, "from {charge}");
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

/// An empty battery stops the gun without spending its cooldown; the generator restarts it as
/// soon as a shot can be paid, the charge never goes below zero, and without fuel nothing
/// restarts it: no electricity from nothing.
#[test]
fn generation_restarts_a_starved_weapon_only_while_it_has_fuel() {
    let mut session = duel();
    built(&mut session, ARMORY);
    let mut sequence = 0;
    let rail = take(&mut session, &mut sequence, Weapon::RailGun);
    session.fighters[0].energy = 0;
    let fuel = session.fighters[0].fuel;
    let hold = Intent { fire: true, ..rail };
    let mut fired_at = Vec::new();
    for tick in 0..300u32 {
        let before = session.fighters[0].energy;
        if act(&mut session, &mut sequence, hold).contains(&Weapon::RailGun) {
            fired_at.push(tick);
            assert!(before >= Weapon::RailGun.energy());
        }
    }
    // 30 at the generator's 0.333 a tick (less the legs' servos): about 90 ticks a shot, longer
    // than the rail gun's own 72: the battery is the limit, and the fuel paid for it.
    assert!((2..=3).contains(&fired_at.len()), "{fired_at:?}");
    assert!(fired_at[0] >= 88, "{fired_at:?}");
    assert!(
        fired_at.windows(2).all(|pair| pair[1] - pair[0] >= 88),
        "{fired_at:?}"
    );
    assert!(session.fighters[0].fuel < fuel);

    // The same with the tank dry: the generator cannot operate, the battery stays empty and the
    // rail gun silent; the robot lives on.
    let mut session = duel();
    built(&mut session, ARMORY);
    let mut sequence = 0;
    let rail = take(&mut session, &mut sequence, Weapon::RailGun);
    session.fighters[0].energy = 0;
    session.fighters[0].fuel = 0;
    for _ in 0..300 {
        let fired = act(&mut session, &mut sequence, Intent { fire: true, ..rail });
        assert!(fired.is_empty());
        assert_eq!(session.fighters[0].energy, 0);
        assert!(session.fighters[0].flow.generated.starved);
    }
    assert!(session.fighters[0].alive());

    // The laser drains the battery (0.6 a tick against the generator's 0.333) and, once it is
    // dry, sputters at what generation pays: it never fires on a charge it cannot pay. Let go,
    // the battery refills to its capacity and the generator idles there.
    let mut session = duel();
    built(&mut session, ARMORY);
    let mut sequence = 0;
    let laser = take(&mut session, &mut sequence, Weapon::Laser);
    let hold = Intent {
        fire: true,
        ..laser
    };
    let mut beams = 0;
    for _ in 0..500 {
        beams += act(&mut session, &mut sequence, hold).len();
    }
    assert!((350..500).contains(&beams), "{beams}");
    let mut sputter = Vec::new();
    for _ in 0..12 {
        sputter.push(act(&mut session, &mut sequence, hold).len());
    }
    assert!(sputter.contains(&0) && sputter.contains(&1), "{sputter:?}");
    assert!(session.fighters[0].energy < Weapon::Laser.energy() + 400);
    for _ in 0..400 {
        act(&mut session, &mut sequence, laser);
    }
    assert_eq!(session.fighters[0].energy, 100 * MILLI);
    let fuel = session.fighters[0].fuel;
    act(&mut session, &mut sequence, laser);
    assert!(
        session.fighters[0].fuel >= fuel - 34,
        "a full battery idles it"
    );
}

/// Depletion stops only what depends on the store that ran dry (D2.i123.4): the flamethrower
/// burns fuel, the laser electricity, and each fires while its own store pays.
#[test]
fn one_store_runs_dry_while_the_other_still_works() {
    let flamer = PartId(9);
    let laser = PartId(7);
    let both = {
        let mut triggers = Triggers::NONE;
        triggers.press(laser);
        triggers.press(flamer);
        triggers
    };
    let ready = |session: &mut Session, sequence: &mut u32| {
        let aim = Intent {
            yaw: 32_768,
            ..Intent::IDLE
        };
        for _ in 0..80 {
            act(session, sequence, aim);
        }
        Intent {
            triggers: both,
            ..aim
        }
    };

    // Tanks dry, battery full: the laser fires, the flamethrower cannot, the generator stops.
    let mut session = duel();
    built(&mut session, PYRO);
    let mut sequence = 0;
    let fire = ready(&mut session, &mut sequence);
    session.fighters[0].fuel = 0;
    for _ in 0..30 {
        let fired = act(&mut session, &mut sequence, fire);
        assert_eq!(fired, [Weapon::Laser]);
        assert_eq!(session.fighters[0].fuel, 0);
    }

    // Battery dry, tanks full: the flamethrower burns on; the laser waits for the generator.
    let mut session = duel();
    built(&mut session, PYRO);
    let mut sequence = 0;
    let fire = ready(&mut session, &mut sequence);
    session.fighters[0].energy = 0;
    let fuel = session.fighters[0].fuel;
    let fired = act(&mut session, &mut sequence, fire);
    assert_eq!(fired, [Weapon::Flamethrower]);
    // The flame's fuel and the generator's both came out of the tanks.
    let flow = session.fighters[0].flow;
    assert_eq!(
        flow.fuel_spent,
        Weapon::Flamethrower.energy() + flow.generated.burned
    );
    assert_eq!(session.fighters[0].fuel, fuel - flow.fuel_spent);
    let mut lasers = 0;
    for _ in 0..30 {
        let fired = act(&mut session, &mut sequence, fire);
        assert!(fired.contains(&Weapon::Flamethrower));
        lasers += usize::from(fired.contains(&Weapon::Laser));
    }
    assert!(
        (1..30).contains(&lasers),
        "the generator pays some beams: {lasers}"
    );
}

/// Concurrent guns and locomotion in one tick: each gun and each motor is granted once from what
/// the earlier requests left; the stores never go below zero.
#[test]
fn concurrent_fire_and_locomotion_spend_each_request_once() {
    let rifle = PartId(7);
    let shotgun = PartId(10);
    let mut triggers = Triggers::NONE;
    triggers.press(rifle);
    triggers.press(shotgun);
    let run = Intent {
        yaw: 32_768,
        forward: 1,
        triggers,
        ..Intent::IDLE
    };
    let mut session = duel();
    let mut sequence = 0;
    for _ in 0..80 {
        act(
            &mut session,
            &mut sequence,
            Intent {
                yaw: 32_768,
                ..Intent::IDLE
            },
        );
    }
    let cost = Weapon::EnergyRifle.energy() + Weapon::Shotgun.energy();
    // Rich: both guns, the motors working, all paid once.
    let before = session.fighters[0].energy;
    let fired = act(&mut session, &mut sequence, run);
    assert_eq!(fired, [Weapon::EnergyRifle, Weapon::Shotgun]);
    assert_eq!(paid(&session), cost);
    assert!(motors(&session) > 0, "running legs draw");
    let flow = session.fighters[0].flow;
    assert_eq!(
        session.fighters[0].energy,
        before - flow.spent + flow.generated.electricity
    );
    // Short: the rifle by part order takes its 2, the shotgun is refused, the motors share what
    // is left; nothing goes below zero and nothing is paid twice.
    for _ in 0..60 {
        act(
            &mut session,
            &mut sequence,
            Intent {
                triggers: Triggers::NONE,
                ..run
            },
        );
    }
    session.fighters[0].energy = Weapon::EnergyRifle.energy() + 30;
    let fired = act(&mut session, &mut sequence, run);
    assert_eq!(fired, [Weapon::EnergyRifle]);
    assert_eq!(paid(&session), Weapon::EnergyRifle.energy());
    assert!(motors(&session) <= 30);
    let flow = session.fighters[0].flow;
    assert_eq!(
        session.fighters[0].energy,
        Weapon::EnergyRifle.energy() + 30 - flow.spent + flow.generated.electricity
    );
}

/// Mods and modules change what a shot costs: the overcharger half again, the efficiency coil
/// 60 %.
#[test]
fn tuned_weapons_pay_their_tuned_cost() {
    for (build, weapon, cost) in [
        (ARMORY, Weapon::RailGun, 30_000),
        (MARKSMAN, Weapon::RailGun, 45_000),
        (SKIRMISHER, Weapon::Shotgun, 7_200),
    ] {
        let mut session = duel();
        built(&mut session, build);
        let mut sequence = 0;
        let aim = take(&mut session, &mut sequence, weapon);
        session.fighters[0].energy = Pool::of(&build).capacity;
        let fired = act(&mut session, &mut sequence, Intent { fire: true, ..aim });
        assert_eq!(fired, [weapon]);
        assert_eq!(paid(&session), cost, "{weapon:?}");
    }
}

#[test]
fn the_stores_travel_in_snapshots_and_are_checked_against_the_build() {
    let mut session = duel();
    session.fighters[0].energy = 42_400;
    session.fighters[0].fuel = 17_000;
    let snapshot = session.snapshot();
    assert_eq!(snapshot.slot(0).unwrap().energy, 42_400);
    assert_eq!(snapshot.slot(0).unwrap().fuel, 17_000);
    let bytes = encode_snapshot(&snapshot);
    let decoded = decode_snapshot(&bytes).unwrap();
    assert_eq!(decoded.slot(0).unwrap().energy, 42_400);
    assert_eq!(decoded.slot(0).unwrap().fuel, 17_000);
    assert_eq!(
        decoded.slot(0).unwrap().robot.as_ref().unwrap().fuel,
        17_000
    );
    assert_eq!(decoded, snapshot);

    // Electricity or fuel above what the build can hold is malformed.
    session.fighters[0].energy = 100 * MILLI + 1;
    let bytes = encode_snapshot(&session.snapshot());
    assert!(matches!(
        decode_snapshot(&bytes),
        Err(WireError::Malformed(_))
    ));
    session.fighters[0].energy = 0;
    session.fighters[0].fuel = 100 * MILLI + 1;
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

/// A client's prediction resumed from a snapshot spends and generates both stores exactly as
/// the host does: walking (its motors working), the laser and the flamethrower burning at once.
#[test]
fn a_predictor_from_a_snapshot_spends_and_generates_as_the_host_does() {
    let mut host = duel();
    built(&mut host, PYRO);
    let mut sequence = 0;
    for _ in 0..40 {
        act(
            &mut host,
            &mut sequence,
            Intent {
                yaw: 32_768,
                ..Intent::IDLE
            },
        );
    }
    let snapshot = decode_snapshot(&encode_snapshot(&host.snapshot())).unwrap();
    let me = snapshot.slot(0).unwrap().clone();
    let mut predictor = Session::new(host.arena.clone()).unwrap();
    predictor
        .place(0, me.robot.as_ref().unwrap(), me.motion, me.held, me.hook)
        .unwrap();
    predictor.fighters[0].parts = me.parts;
    predictor.fighters[0].energy = me.energy;
    predictor.fighters[0].fuel = me.fuel;
    let mut triggers = Triggers::NONE;
    triggers.press(PartId(7));
    triggers.press(PartId(9));
    let intent = Intent {
        yaw: 32_768,
        forward: 1,
        triggers,
        ..Intent::IDLE
    };
    for tick in 0..90 {
        act(&mut host, &mut sequence, intent);
        predictor.tick(intent).unwrap();
        let (host, predicted) = (&host.fighters[0], &predictor.fighters[0]);
        assert_eq!(
            (predicted.energy, predicted.fuel),
            (host.energy, host.fuel),
            "tick {tick}"
        );
        // The tick's flow too (the smoothed rates carry each one's own history: display only).
        let flow = |flow: Flow| Flow {
            rates: [0; 3],
            ..flow
        };
        assert_eq!(flow(predicted.flow), flow(host.flow), "tick {tick}");
    }
    assert!(
        host.fighters[0].fuel < me.fuel,
        "the flame and the generator burned fuel"
    );
}

/// A bot match: every living robot's stores stay within its build's every tick, the bots do
/// spend electricity and their generators burn fuel, and the same seed spends them the same way.
#[test]
fn a_bot_match_keeps_every_store_in_bounds_and_deterministic() {
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
                spent |= fighter.energy < pool.capacity;
                burned |= fighter.fuel < pool.fuel;
            }
            trace.push(
                session
                    .fighters
                    .iter()
                    .map(|fighter| (fighter.energy, fighter.fuel))
                    .collect::<Vec<_>>(),
            );
        }
        assert!(spent, "the bots fire, so their batteries drain");
        assert!(burned, "and their generators burn fuel to refill them");
        trace
    };
    assert_eq!(run(), run());
}
