//! Electric and combustion powertrains (Atlas D2.i123.3, shooter#130) on the one resource model
//! (`shooter_sim::energy`): the root chassis' native power, the engine, the flamethrower on fuel
//! and the other weapons, the joints and repairs on electricity; hybrids built from the same
//! parts; the builder's advisory feedback; a lost engine, tank, generator or battery stopping
//! only what depends on it; and the host, a predicting client and the wire agreeing.
// The platform's floats only check results here; the simulation uses libm (clippy.toml).
#![allow(clippy::disallowed_methods)]

use shooter_sim::energy::{
    COMBUSTION_ELECTRICITY_PER_FUEL, ELECTRICITY_PER_FUEL, MILLI, Pool, Resource,
};
use shooter_sim::mechanics::{Layout, load};
use shooter_sim::physics::Vec3i;
use shooter_sim::robot::parts::{
    BATTERY, CHASSIS_M, COMBUSTION_L, COMBUSTION_M, COMBUSTION_S, ENGINE,
};
use shooter_sim::robot::{
    Assembly, BETA, Kind, Notice, PYRO, PartId, Power, STANDARD, STOKER, loadout,
};
use shooter_sim::session::{walk_activity, walk_draw};
use shooter_sim::wire::{decode_snapshot, encode_snapshot};
use shooter_sim::{Arena, Event, Input, Intent, MapId, Rules, Session, Spawn, Weapon};

const A: u32 = 1;
const B: u32 = 2;
const TANK: PartId = PartId(20);
const ENGINE_ID: PartId = PartId(22);

/// Two people 12 m apart on an open floor, no bots, no pads.
fn floor() -> Arena {
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

fn rules(slots: u8) -> Rules {
    Rules {
        slots,
        bots: false,
        local: false,
        seed: 0,
        frag_limit: 0,
        time_limit_ticks: 0,
        intermission_ticks: 60,
        respawn_ticks: 30,
    }
}

/// A duel with person A built as `build`, whole and full, settled on the floor facing away
/// from B.
fn duel(build: Assembly) -> Session {
    let mut session = Session::with_rules(floor(), rules(2)).unwrap();
    assert_eq!(session.join(A).unwrap(), Some(0));
    assert_eq!(session.join(B).unwrap(), Some(1));
    session.fighters[0].configured = build;
    session.rebuild(0, build).unwrap();
    fill(&mut session);
    for _ in 0..90 {
        act(&mut session, STAND);
    }
    fill(&mut session);
    session
}

/// A's stores full.
fn fill(session: &mut Session) {
    let fighter = &mut session.fighters[0];
    fighter.energy = fighter.pool().capacity;
    fighter.fuel = fighter.pool().fuel;
}

/// A's `intent` for one tick; the weapons A fired that tick.
fn act(session: &mut Session, intent: Intent) -> Vec<Weapon> {
    let sequence = session.fighters[0].ack + 1;
    let view_tick = session.ticks();
    session.queue(
        A,
        Input {
            sequence,
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

/// Facing away from B and walking forward.
const WALK: Intent = Intent {
    yaw: 32_768,
    forward: 1,
    ..Intent::IDLE
};

/// Facing away from B, standing.
const STAND: Intent = Intent {
    yaw: 32_768,
    ..Intent::IDLE
};

/// Facing away from B and firing `weapon` (its first copy).
fn firing(weapon: Weapon) -> Intent {
    Intent {
        fire: true,
        select: Some(weapon.into()),
        ..STAND
    }
}

/// What a tick of `intent`'s walk takes `build`, whole, on its feet.
fn walk(build: &Assembly, intent: Intent) -> u32 {
    walk_draw(
        &Layout::of(build),
        walk_activity(intent, true),
        load(build, |_| true),
    )
}

/// `build` without the parts `ids` (and still valid).
fn without(build: &Assembly, ids: &[u8]) -> Assembly {
    let mut candidate = build.candidate();
    candidate.parts.retain(|part| !ids.contains(&part.id));
    BETA.validate(&candidate).unwrap()
}

/// `build` on root chassis `root` instead (the same cuboid), still valid.
fn on_root(build: &Assembly, root: &shooter_sim::robot::PartType) -> Assembly {
    let mut candidate = build.candidate();
    candidate
        .parts
        .iter_mut()
        .find(|part| part.mount.is_none())
        .unwrap()
        .kind = root.id.0;
    BETA.validate(&candidate).unwrap()
}

/// The index of part `id` in A's build.
fn index(session: &Session, id: PartId) -> usize {
    session.fighters[0]
        .build
        .parts()
        .iter()
        .position(|part| part.id == id)
        .unwrap()
}

/// How far A's feet moved over `ticks` ticks of `intent`.
fn moved(session: &mut Session, intent: Intent, ticks: u32) -> f32 {
    let before = session.fighters[0].feet();
    for _ in 0..ticks {
        act(session, intent);
    }
    let after = session.fighters[0].feet();
    (after[0] - before[0]).abs() + (after[2] - before[2]).abs()
}

#[test]
fn the_catalog_offers_both_powertrains_an_engine_and_hybrids_that_validate() {
    // Each chassis size comes electric and combustion; the engine is a part like any.
    for chassis in [COMBUSTION_S, COMBUSTION_M, COMBUSTION_L] {
        assert_eq!(chassis.kind, Kind::Chassis(Power::Combustion));
        assert!(BETA.parts.contains(&chassis));
    }
    assert_eq!(CHASSIS_M.kind, Kind::Chassis(Power::Electric));
    assert_eq!((ENGINE.kind, ENGINE.cells), (Kind::Engine, [1, 2, 1]));
    assert!(BETA.parts.contains(&ENGINE));
    // What each consumer runs on.
    assert_eq!(Weapon::Flamethrower.resource(), Resource::Fuel);
    for weapon in [
        Weapon::EnergyRifle,
        Weapon::RailGun,
        Weapon::Laser,
        Weapon::Shotgun,
        Weapon::RocketLauncher,
        Weapon::EnergyBlade,
        Weapon::Grapple,
    ] {
        assert_eq!(weapon.resource(), Resource::Electricity, "{weapon:?}");
    }

    // The electric root stores 100; the combustion root nothing (the stoker's electricity is
    // its battery's). Its engine drives on fuel.
    let electric = Pool::of(&STANDARD);
    assert_eq!((electric.power, electric.engine), (Power::Electric, false));
    assert_eq!(electric.drive(), Some(Resource::Electricity));
    let stoker = Pool::of(&STOKER);
    assert_eq!(BETA.validate(&STOKER.candidate()), Ok(STOKER));
    assert_eq!(loadout("stoker"), Some(STOKER));
    assert_eq!((stoker.power, stoker.engine), (Power::Combustion, true));
    assert_eq!(stoker.capacity, u32::from(BATTERY.capacity) * MILLI);
    assert_eq!(stoker.fuel, 1_000 * MILLI);
    assert_eq!(stoker.drive(), Some(Resource::Fuel));
    assert_eq!(Pool::of(&without(&STOKER, &[22])).drive(), None);
    let budget = BETA.budget(&STOKER);
    assert_eq!((budget.power, budget.engines), (Power::Combustion, 1));

    // Feedback, never a rule: every one of these builds is valid.
    assert_eq!(BETA.notices(&STOKER), []);
    assert_eq!(BETA.notices(&STANDARD), []);
    assert_eq!(BETA.notices(&PYRO), []);
    assert_eq!(
        BETA.notices(&without(&STOKER, &[22])),
        [Notice::DriveNoEngine]
    );
    // The tank under the generator: no fuel for the drive or the flamethrower, no recharge.
    assert_eq!(
        BETA.notices(&without(&STOKER, &[20, 21])),
        [
            Notice::DriveNoFuel,
            Notice::NoFuel(PartId(7)),
            Notice::NoRecharge
        ]
    );
    // No battery: the combustion root stores no electricity for the blade, rifle and grapple.
    assert_eq!(
        BETA.notices(&without(&STOKER, &[11])),
        [
            Notice::NoElectricity(PartId(8)),
            Notice::NoElectricity(PartId(9)),
            Notice::NoElectricity(PartId(10)),
        ]
    );
    // An electric flamethrower without a tank.
    assert_eq!(
        BETA.notices(&without(&PYRO, &[20, 21])),
        [Notice::NoFuel(PartId(9)), Notice::NoRecharge]
    );
    // The standard build on a combustion chassis: no engine, and nowhere to store electricity.
    let swapped = on_root(&STANDARD, &COMBUSTION_S);
    assert_eq!(
        BETA.notices(&swapped),
        [
            Notice::DriveNoEngine,
            Notice::NoElectricity(PartId(7)),
            Notice::NoElectricity(PartId(8)),
            Notice::NoElectricity(PartId(9)),
            Notice::NoElectricity(PartId(10)),
        ]
    );
    assert_eq!(Pool::of(&swapped).capacity, 0);
    assert_eq!(
        Notice::DriveNoEngine.to_string(),
        "drive: combustion needs an engine"
    );
}

#[test]
fn an_electric_chassis_recharges_twice_as_fast_as_a_combustion_one() {
    assert_eq!(
        Pool::of(&STANDARD).electricity_per_fuel(),
        ELECTRICITY_PER_FUEL
    );
    assert_eq!(
        Pool::of(&STOKER).electricity_per_fuel(),
        COMBUSTION_ELECTRICITY_PER_FUEL
    );
    let made = |build: &Assembly| {
        let (mut charge, mut fuel) = (0, Pool::of(build).fuel);
        Pool::of(build).generate(&mut charge, &mut fuel)
    };
    // The same generator burns 12 fuel a second (200 milli-units a tick) on either.
    assert_eq!(made(&STANDARD), (200, 400));
    assert_eq!(made(&STOKER), (200, 200));
}

#[test]
fn a_combustion_robot_drives_on_fuel_and_an_electric_one_on_electricity() {
    let mut session = duel(STOKER);
    let cost = walk(&STOKER, WALK);
    assert!(cost > 0);
    act(&mut session, WALK);
    let flow = session.fighters[0].flow;
    assert!(!flow.stalled);
    assert_eq!((flow.fueled, flow.used), (cost, 0), "the walk burns fuel");
    assert!(moved(&mut session, WALK, 30) > 20.0, "and it walks");
    // Standing is free.
    act(&mut session, STAND);
    let flow = session.fighters[0].flow;
    assert_eq!((flow.fueled, flow.used), (0, 0));

    let mut session = duel(STANDARD);
    act(&mut session, WALK);
    let flow = session.fighters[0].flow;
    assert_eq!(
        (flow.fueled, flow.used),
        (0, walk(&STANDARD, WALK)),
        "the electric walk draws electricity"
    );
}

#[test]
fn a_combustion_drive_stops_without_fuel_or_an_engine_and_nothing_else_does() {
    // An empty tank: the engine cannot run, the walk is refused and it stands; its rifle still
    // fires from the battery, its flamethrower does not.
    let mut session = duel(STOKER);
    session.fighters[0].fuel = 0;
    let still = moved(&mut session, WALK, 30);
    assert!(still < 1.0, "it does not walk: {still}");
    assert!(session.fighters[0].flow.stalled);
    let mut fired = Vec::new();
    for _ in 0..30 {
        fired.extend(act(&mut session, firing(Weapon::EnergyRifle)));
    }
    assert!(fired.contains(&Weapon::EnergyRifle), "{fired:?}");
    for _ in 0..10 {
        assert_eq!(act(&mut session, firing(Weapon::Flamethrower)), []);
    }
    assert!(session.fighters[0].alive());

    // Built without an engine: full of fuel, yet the combustion drive never starts; the
    // flamethrower burns the fuel.
    let mut session = duel(without(&STOKER, &[22]));
    assert!(moved(&mut session, WALK, 30) < 1.0, "no engine, no walk");
    assert!(session.fighters[0].flow.stalled);
    let mut fired = Vec::new();
    for _ in 0..30 {
        fired.extend(act(&mut session, firing(Weapon::Flamethrower)));
    }
    assert!(fired.contains(&Weapon::Flamethrower), "{fired:?}");
    assert!(session.fighters[0].flow.fueled > 0);

    // The engine destroyed in a living robot: only the drive stops. The robot lives, its
    // chassis untouched, its fuel and electricity still there for its guns.
    let mut session = duel(STOKER);
    let health = session.fighters[0].chassis_health();
    let build = session.fighters[0].build;
    let engine = index(&session, ENGINE_ID);
    session.fighters[0].parts.hurt(&build, engine, 10_000);
    act(&mut session, STAND);
    assert!(!session.fighters[0].pool().engine);
    assert!(moved(&mut session, WALK, 30) < 1.0);
    let fighter = &session.fighters[0];
    assert!(fighter.alive() && fighter.flow.stalled);
    assert_eq!(fighter.chassis_health(), health);
    assert_eq!(fighter.fuel, 1_000 * MILLI, "the engine burned nothing");
    let mut fired = Vec::new();
    for _ in 0..30 {
        fired.extend(act(&mut session, firing(Weapon::EnergyRifle)));
    }
    for _ in 0..30 {
        fired.extend(act(&mut session, firing(Weapon::Flamethrower)));
    }
    assert!(fired.contains(&Weapon::EnergyRifle) && fired.contains(&Weapon::Flamethrower));

    // The tank destroyed: its fuel and the generator on it go; the drive and the flamethrower
    // stop, the battery's rifle fires on.
    let mut session = duel(STOKER);
    let build = session.fighters[0].build;
    let tank = index(&session, TANK);
    // As the host's damage does (a shot tank: `tests/damage.rs`), the fuel goes with it.
    let before = session.fighters[0].pool();
    session.fighters[0].parts.hurt(&build, tank, 10_000);
    let fighter = &mut session.fighters[0];
    fighter.fuel = before.shrink_fuel(fighter.pool(), fighter.fuel);
    act(&mut session, STAND);
    let fighter = &session.fighters[0];
    assert_eq!(
        (fighter.fuel, fighter.pool().fuel, fighter.pool().burn),
        (0, 0, 0)
    );
    assert!(moved(&mut session, WALK, 30) < 1.0);
    let mut fired = Vec::new();
    for _ in 0..30 {
        fired.extend(act(&mut session, firing(Weapon::Flamethrower)));
    }
    for _ in 0..30 {
        fired.extend(act(&mut session, firing(Weapon::EnergyRifle)));
    }
    assert!(!fired.contains(&Weapon::Flamethrower));
    assert!(fired.contains(&Weapon::EnergyRifle));
    assert!(session.fighters[0].alive());
}

#[test]
fn a_combustion_robot_needs_its_battery_and_generator_for_electrical_weapons() {
    // Its battery drained and the generator lost: the rifle stays silent, the flamethrower and
    // the drive work on fuel.
    let mut session = duel(without(&STOKER, &[21]));
    session.fighters[0].energy = 0;
    for _ in 0..30 {
        assert_eq!(act(&mut session, firing(Weapon::EnergyRifle)), []);
    }
    assert!(
        moved(&mut session, WALK, 30) > 20.0,
        "the drive runs on fuel"
    );
    let mut fired = Vec::new();
    for _ in 0..10 {
        fired.extend(act(&mut session, firing(Weapon::Flamethrower)));
    }
    assert!(fired.contains(&Weapon::Flamethrower));

    // With the generator, the drained battery refills at the combustion rate and the rifle
    // fires again.
    let mut session = duel(STOKER);
    session.fighters[0].energy = 0;
    act(&mut session, STAND);
    let flow = session.fighters[0].flow;
    assert_eq!((flow.burned, flow.made), (200, 200));
    let mut fired = Vec::new();
    for _ in 0..120 {
        fired.extend(act(&mut session, firing(Weapon::EnergyRifle)));
    }
    assert!(fired.contains(&Weapon::EnergyRifle));
}

#[test]
fn an_electric_robot_needs_a_fuel_supply_for_its_flamethrower() {
    // The pyro's flamethrower burns its tank's fuel, not its electricity.
    let mut session = duel(PYRO);
    let fired = act(&mut session, firing(Weapon::Flamethrower));
    let mut fired = fired;
    for _ in 0..30 {
        if fired.contains(&Weapon::Flamethrower) {
            break;
        }
        fired = act(&mut session, firing(Weapon::Flamethrower));
    }
    assert!(fired.contains(&Weapon::Flamethrower));
    let flow = session.fighters[0].flow;
    assert_eq!(flow.fueled, Weapon::Flamethrower.energy());
    assert_eq!(flow.used, 0);

    // Without the tank (and the generator on it) it never lights; the laser and the walk work
    // on the root's electricity.
    let mut session = duel(without(&PYRO, &[20, 21]));
    for _ in 0..30 {
        assert_eq!(act(&mut session, firing(Weapon::Flamethrower)), []);
    }
    let mut fired = Vec::new();
    for _ in 0..30 {
        fired.extend(act(&mut session, firing(Weapon::Laser)));
    }
    assert!(fired.contains(&Weapon::Laser));
    assert!(moved(&mut session, WALK, 30) > 20.0);
}

/// A person's combustion robot on a host, a client predicting it from the host's snapshot from
/// tick 30 on, both given the same inputs: the prediction follows the host's bodies, fuel and
/// electricity bit for bit while it walks, burns and fires.
#[test]
fn a_client_predicts_a_combustion_robot_exactly() {
    let mut host = Session::with_rules(floor(), rules(1)).unwrap();
    host.join(7).unwrap();
    host.fighters[0].configured = STOKER;
    host.rebuild(0, STOKER).unwrap();
    let mut predictor = Session::new(floor()).unwrap();
    let intents = [
        WALK,
        Intent {
            forward: 1,
            sprint: true,
            yaw: 16_000,
            ..Intent::IDLE
        },
        Intent {
            fire: true,
            select: Some(Weapon::Flamethrower.into()),
            yaw: 8_000,
            ..Intent::IDLE
        },
        Intent {
            fire: true,
            forward: 1,
            select: Some(Weapon::EnergyRifle.into()),
            yaw: 30_000,
            ..Intent::IDLE
        },
    ];
    for tick in 0..360u32 {
        let intent = intents[(tick / 40) as usize % intents.len()];
        host.queue(
            7,
            Input {
                sequence: tick + 1,
                intent,
                view_tick: host.ticks(),
            },
        );
        host.step().unwrap();
        let snapshot = decode_snapshot(&encode_snapshot(&host.snapshot())).unwrap();
        let me = snapshot.of_player(7).unwrap().clone();
        let robot = me.robot.clone().unwrap();
        assert_eq!(robot.build, STOKER);
        if tick == 30 {
            predictor.fighters[0].parts = me.parts;
            predictor.fighters[0].energy = me.energy;
            predictor.fighters[0].fuel = me.fuel;
            predictor
                .place(0, &robot, me.motion, me.held, me.hook)
                .unwrap();
        } else if tick > 30 {
            predictor.tick(intent).unwrap();
            let predicted = predictor.mechanics.state().robots[0].clone();
            assert_eq!(predicted.bodies, robot.bodies, "tick {tick}");
            assert_eq!(predictor.local().motion, me.motion, "tick {tick}");
            assert_eq!(
                (predictor.local().energy, predictor.local().fuel),
                (me.energy, me.fuel),
                "tick {tick}"
            );
        }
    }
    let fighter = &host.fighters[0];
    assert!(fighter.fuel < 1_000 * MILLI, "it burned fuel");
}

/// A bot match on the greybox yard with one bot built as the stoker: it drives on fuel, every
/// pool stays in bounds, every generator converts at its chassis' rate, and the match replays
/// the same.
#[test]
fn a_bot_match_with_a_combustion_bot_is_bounded_and_deterministic() {
    let run = || {
        let mut session = Session::on_map(MapId::Greybox, Rules::deathmatch(7)).unwrap();
        session.fighters[1].configured = STOKER;
        session.rebuild(1, STOKER).unwrap();
        let mut trace = Vec::new();
        let mut fueled = false;
        for _ in 0..1_200 {
            session.step().unwrap();
            for fighter in session.fighters.iter().filter(|fighter| fighter.alive()) {
                let pool = fighter.pool();
                assert!(fighter.energy <= pool.capacity && fighter.fuel <= pool.fuel);
                let flow = fighter.flow;
                assert_eq!(flow.made, flow.burned * pool.electricity_per_fuel());
            }
            let stoker = &session.fighters[1];
            assert_eq!(stoker.build, STOKER, "a bot keeps its build across lives");
            fueled |= stoker.alive() && stoker.flow.fueled > 0;
            trace.push(
                session
                    .fighters
                    .iter()
                    .map(|fighter| (fighter.energy, fighter.fuel, fighter.feet()))
                    .collect::<Vec<_>>(),
            );
        }
        assert!(fueled, "the combustion bot burned fuel walking or flaming");
        trace
    };
    let first = run();
    assert!(first == run());
}
