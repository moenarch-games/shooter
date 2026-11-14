//! Robot builds: the beta catalog, valid and invalid candidates, the budgets at their exact
//! limits, builds from untrusted bytes, and builds through a match (configured while alive,
//! applied at the next spawn, carried in snapshots, replayed exactly).

use shooter_sim::physics::Vec3i;
use shooter_sim::robot::{
    AssemblyError, BETA, Candidate, Catalog, Component, ComponentId, HEAVY, Hardpoint, Kind,
    LOADOUTS, MARKSMAN, ModId, Module, SKIRMISHER, STANDARD, Size, Socket, Tuning, loadout,
};
use shooter_sim::session::MAX_HEALTH;
use shooter_sim::weapons::Delivery;
use shooter_sim::wire::{WireError, decode_input, decode_snapshot, encode_input, encode_snapshot};
use shooter_sim::{Arena, Assembly, Event, Input, Intent, Rules, Session, Spawn, Weapon};

const RAPID_CYCLER: ModId = ModId(1);
const OVERCHARGER: ModId = ModId(2);
const LONG_BARREL: ModId = ModId(3);
const HEAT_SINK: ModId = ModId(4);

const TORSO: u8 = Socket::Torso.index();
const HEAD: u8 = Socket::Head.index();
const LEFT_ARM: u8 = Socket::LeftArm.index();
const RIGHT_ARM: u8 = Socket::RightArm.index();
const LEGS: u8 = Socket::Locomotion.index();
const LEFT_HAND: u8 = Hardpoint::LeftHand.index();
const RIGHT_HAND: u8 = Hardpoint::RightHand.index();
const LEFT_SHOULDER: u8 = Hardpoint::LeftShoulder.index();

fn candidate(parts: &[(u8, u8)], mounts: &[(u8, Weapon)]) -> Candidate {
    Candidate {
        parts: parts.to_vec(),
        mounts: mounts
            .iter()
            .map(|&(hardpoint, weapon)| (hardpoint, weapon.index()))
            .collect(),
        mods: Vec::new(),
    }
}

/// The standard build's parts, as raw codes.
fn standard_parts() -> Vec<(u8, u8)> {
    STANDARD.candidate().parts
}

#[test]
fn the_catalog_is_consistent_and_its_builds_are_valid() {
    let mut ids: Vec<u8> = BETA.components.iter().map(|c| c.id.0).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), BETA.components.len(), "unique component codes");
    let mut keys: Vec<&str> = BETA.components.iter().map(|c| c.key).collect();
    keys.extend(BETA.modules.iter().map(|m| m.key));
    for key in &keys {
        assert!(
            key.starts_with("robot.") || key.starts_with("weapon."),
            "{key}"
        );
        assert!(
            key.bytes()
                .all(|b| b.is_ascii_lowercase() || b == b'.' || b == b'-'),
            "{key}"
        );
    }
    let count = keys.len();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), count, "unique data keys");
    for weapon in Weapon::ALL {
        assert_eq!(
            BETA.modules.iter().filter(|m| m.weapon == weapon).count(),
            1,
            "{weapon:?} is a module from the start"
        );
    }
    for kind in [Kind::Torso, Kind::Head, Kind::Arm, Kind::Locomotion] {
        assert!(BETA.components.iter().any(|c| c.kind == kind), "{kind:?}");
    }
    for component in BETA.components {
        assert_eq!(component.kind == Kind::Torso, component.power > 0);
        assert_eq!(component.kind == Kind::Locomotion, component.load > 0);
        assert!(component.points > 0);
    }
    for build in [STANDARD, HEAVY] {
        assert_eq!(BETA.validate(&build.candidate()), Ok(build));
        let budget = BETA.budget(&build);
        assert!(budget.carried <= budget.load && budget.draw <= budget.power);
        assert!(budget.heat <= budget.cooling && budget.points <= budget.build_points);
    }
    assert_eq!(BETA.budget(&HEAVY).points, u32::from(BETA.build_points));
    assert_eq!(Assembly::default(), STANDARD);
}

#[test]
fn a_build_is_canonical_whatever_order_it_arrives_in() {
    let mut shuffled = HEAVY.candidate();
    shuffled.parts.reverse();
    shuffled.mounts.rotate_left(1);
    assert_ne!(shuffled, HEAVY.candidate());
    assert_eq!(BETA.validate(&shuffled), Ok(HEAVY));
    assert_eq!(
        BETA.validate(&shuffled).unwrap().candidate(),
        HEAVY.candidate()
    );
}

#[test]
fn free_combinations_are_valid() {
    // No arms: the shoulders still carry weapons.
    let armless = candidate(
        &[(TORSO, 1), (HEAD, 10), (LEGS, 30)],
        &[(LEFT_SHOULDER, Weapon::Grapple)],
    );
    let build = BETA.validate(&armless).unwrap();
    assert_eq!(build.part(Socket::LeftArm), None);
    assert_eq!(build.mount(Hardpoint::LeftShoulder), Some(Weapon::Grapple));
    // Mixed arms, no weapons at all.
    let mixed = candidate(
        &[
            (TORSO, 2),
            (HEAD, 11),
            (LEFT_ARM, 21),
            (RIGHT_ARM, 20),
            (LEGS, 31),
        ],
        &[],
    );
    assert!(BETA.validate(&mixed).is_ok());
    // A heavy weapon on a heavy arm of the light torso.
    let mut parts = standard_parts();
    parts[2] = (LEFT_ARM, 21);
    assert!(
        BETA.validate(&candidate(&parts, &[(LEFT_HAND, Weapon::RailGun)]))
            .is_ok()
    );
}

#[test]
fn invalid_codes_sockets_and_mounts_are_rejected() {
    let parts = standard_parts();
    let with_parts = |parts: &[(u8, u8)]| BETA.validate(&candidate(parts, &[]));
    let with_mounts = |mounts: &[(u8, Weapon)]| BETA.validate(&candidate(&parts, mounts));

    let mut bad = parts.clone();
    bad.push((5, 20));
    assert_eq!(with_parts(&bad), Err(AssemblyError::UnknownSocket(5)));
    bad[5] = (u8::MAX, 20);
    assert_eq!(with_parts(&bad), Err(AssemblyError::UnknownSocket(u8::MAX)));
    bad[5] = (HEAD, 99);
    assert_eq!(with_parts(&bad), Err(AssemblyError::UnknownComponent(99)));
    bad[5] = (HEAD, 11);
    assert_eq!(
        with_parts(&bad),
        Err(AssemblyError::DuplicateSocket(Socket::Head))
    );
    let mut wrong = parts.clone();
    wrong[1] = (HEAD, 20);
    assert_eq!(
        with_parts(&wrong),
        Err(AssemblyError::WrongKind(Socket::Head, ComponentId(20)))
    );
    wrong[1] = (HEAD, 10);
    wrong[4] = (LEGS, 2);
    assert_eq!(
        with_parts(&wrong),
        Err(AssemblyError::WrongKind(Socket::Locomotion, ComponentId(2)))
    );
    for (socket, index) in [
        (Socket::Torso, 0),
        (Socket::Head, 1),
        (Socket::Locomotion, 4),
    ] {
        let mut missing = parts.clone();
        missing.remove(index);
        assert_eq!(with_parts(&missing), Err(AssemblyError::Missing(socket)));
    }
    assert_eq!(with_parts(&[]), Err(AssemblyError::Missing(Socket::Torso)));

    let raw = |mounts: &[(u8, u8)]| {
        BETA.validate(&Candidate {
            parts: parts.clone(),
            mounts: mounts.to_vec(),
            mods: Vec::new(),
        })
    };
    assert_eq!(raw(&[(4, 0)]), Err(AssemblyError::UnknownHardpoint(4)));
    assert_eq!(raw(&[(0, 8)]), Err(AssemblyError::UnknownWeapon(8)));
    assert_eq!(
        with_mounts(&[(LEFT_HAND, Weapon::Laser), (LEFT_HAND, Weapon::Shotgun)]),
        Err(AssemblyError::DuplicateHardpoint(Hardpoint::LeftHand))
    );
    assert_eq!(
        with_mounts(&[(LEFT_HAND, Weapon::Laser), (RIGHT_HAND, Weapon::Laser)]),
        Err(AssemblyError::DuplicateWeapon(Weapon::Laser))
    );
    assert_eq!(
        with_mounts(&[(LEFT_HAND, Weapon::RocketLauncher)]),
        Err(AssemblyError::TooHeavyForMount(
            Hardpoint::LeftHand,
            Weapon::RocketLauncher
        ))
    );
    assert_eq!(
        with_mounts(&[(LEFT_SHOULDER, Weapon::RailGun)]),
        Err(AssemblyError::TooHeavyForMount(
            Hardpoint::LeftShoulder,
            Weapon::RailGun
        ))
    );
    assert_eq!(
        with_mounts(&[(LEFT_SHOULDER, Weapon::EnergyBlade)]),
        Err(AssemblyError::HandOnly(
            Hardpoint::LeftShoulder,
            Weapon::EnergyBlade
        ))
    );
    let armless = [(TORSO, 1), (HEAD, 10), (LEGS, 30)];
    assert_eq!(
        BETA.validate(&candidate(&armless, &[(RIGHT_HAND, Weapon::EnergyRifle)])),
        Err(AssemblyError::NoHardpoint(Hardpoint::RightHand))
    );
}

/// A catalog whose one build lands exactly on every limit: 100 kg on a 100 kg load, 50 power
/// of 50, 30 heat of 30 cooling, 40 of 40 points.
fn tight(build_points: u16) -> Catalog {
    let part = |id, kind, mass, power, draw, cooling, load, mounts| Component {
        id: ComponentId(id),
        key: "robot.test",
        kind,
        mass,
        power,
        draw,
        cooling,
        load,
        points: 10,
        mounts,
    };
    let components: &'static [Component] = Box::leak(Box::new([
        part(1, Kind::Torso, 60, 50, 0, 20, 0, Some(Size::Light)),
        part(2, Kind::Head, 10, 0, 10, 10, 0, None),
        part(3, Kind::Locomotion, 40, 0, 20, 0, 100, None),
        part(4, Kind::Locomotion, 40, 0, 20, 0, 99, None),
        part(5, Kind::Torso, 60, 49, 0, 20, 0, Some(Size::Light)),
        part(6, Kind::Head, 10, 0, 10, 9, 0, None),
    ]));
    let modules: &'static [Module] = Box::leak(Box::new([Module {
        weapon: Weapon::EnergyRifle,
        key: "weapon.test",
        size: Size::Light,
        hand_only: false,
        mass: 30,
        draw: 20,
        heat: 30,
        points: 10,
    }]));
    Catalog {
        components,
        modules,
        mods: &[],
        build_points,
    }
}

#[test]
fn budgets_hold_exactly_at_their_limits_and_fail_one_over() {
    let catalog = tight(40);
    let build = |torso, head, legs| {
        catalog.validate(&candidate(
            &[(TORSO, torso), (HEAD, head), (LEGS, legs)],
            &[(LEFT_SHOULDER, Weapon::EnergyRifle)],
        ))
    };
    let exact = build(1, 2, 3).unwrap();
    let budget = catalog.budget(&exact);
    assert_eq!((budget.carried, budget.load), (100, 100));
    assert_eq!((budget.draw, budget.power), (50, 50));
    assert_eq!((budget.heat, budget.cooling), (30, 30));
    assert_eq!((budget.points, budget.build_points), (40, 40));
    assert_eq!(
        build(1, 2, 4),
        Err(AssemblyError::Overweight {
            carried: 100,
            load: 99
        })
    );
    assert_eq!(
        build(5, 2, 3),
        Err(AssemblyError::Underpowered {
            draw: 50,
            power: 49
        })
    );
    assert_eq!(
        build(1, 6, 3),
        Err(AssemblyError::Overheated {
            heat: 30,
            cooling: 29
        })
    );
    assert_eq!(
        tight(39).validate(&exact.candidate()),
        Err(AssemblyError::OverBudget {
            points: 40,
            build_points: 39
        })
    );
    // The budgets are checked in a fixed order: mass, power, cooling, points.
    assert!(matches!(
        tight(0).validate(&candidate(
            &[(TORSO, 5), (HEAD, 6), (LEGS, 4)],
            &[(LEFT_SHOULDER, Weapon::EnergyRifle)],
        )),
        Err(AssemblyError::Overweight { .. })
    ));
    // In the beta catalog the heavy build spends every point: the costlier head tips it over.
    let mut over = HEAVY.candidate();
    over.parts[1] = (HEAD, 11);
    assert_eq!(
        BETA.validate(&over),
        Err(AssemblyError::OverBudget {
            points: 125,
            build_points: 120
        })
    );
}

#[test]
fn builds_from_untrusted_bytes_are_validated() {
    let sent = encode_input(Intent::IDLE, 3, None, Some(&HEAVY));
    assert_eq!(decode_input(&sent).unwrap().build, Some(HEAVY));
    // The standard build's light arm swapped for the heavy rail gun mount, byte by byte: the
    // build's codes follow the name's (empty) length byte and the build flag.
    let plain = encode_input(Intent::IDLE, 3, None, None);
    let mut tampered = plain[..plain.len() - 1].to_vec();
    tampered.extend_from_slice(&[
        1, 5, TORSO, 1, HEAD, 10, LEFT_ARM, 20, RIGHT_ARM, 20, LEGS, 30,
    ]);
    let parts = tampered.clone();
    tampered.extend_from_slice(&[1, LEFT_HAND, Weapon::RailGun.index(), 0]);
    assert_eq!(
        decode_input(&tampered),
        Err(WireError::Build(AssemblyError::TooHeavyForMount(
            Hardpoint::LeftHand,
            Weapon::RailGun
        )))
    );
    // A long barrel (a heavy mod) on the standard frame's light-arm rifle: refused, not dropped.
    let mut modded = parts.clone();
    modded.extend_from_slice(&[1, LEFT_HAND, Weapon::EnergyRifle.index()]);
    modded.extend_from_slice(&[1, LEFT_HAND, LONG_BARREL.0]);
    assert_eq!(
        decode_input(&modded),
        Err(WireError::Build(AssemblyError::TooHeavyForMount(
            Hardpoint::LeftHand,
            Weapon::EnergyRifle
        )))
    );
    let mut unknown_mod = parts;
    unknown_mod.extend_from_slice(&[1, LEFT_HAND, Weapon::EnergyRifle.index()]);
    unknown_mod.extend_from_slice(&[1, LEFT_HAND, 99]);
    assert_eq!(
        decode_input(&unknown_mod),
        Err(WireError::Build(AssemblyError::UnknownMod(99)))
    );
    let mut unknown = plain[..plain.len() - 1].to_vec();
    unknown.extend_from_slice(&[1, 1, TORSO, 200, 0, 0]);
    assert_eq!(
        decode_input(&unknown),
        Err(WireError::Build(AssemblyError::UnknownComponent(200)))
    );
    let mut flag = plain.clone();
    *flag.last_mut().unwrap() = 2;
    assert!(matches!(decode_input(&flag), Err(WireError::Malformed(_))));
    let mut truncated = plain[..plain.len() - 1].to_vec();
    truncated.extend_from_slice(&[1, 3, TORSO]);
    assert!(matches!(
        decode_input(&truncated),
        Err(WireError::Malformed(_))
    ));
}

/// Two people face each other 12 m apart, alone, and respawn after half a second.
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
    assert_eq!(session.join(1).unwrap(), Some(0));
    assert_eq!(session.join(2).unwrap(), Some(1));
    session
}

/// Person 1 holds the rail gun's trigger for `ticks` ticks.
fn rail(session: &mut Session, ticks: u32) {
    let intent = Intent {
        fire: true,
        select: Some(Weapon::RailGun),
        ..Intent::IDLE
    };
    for _ in 0..ticks {
        let sequence = session.fighters[0].ack + 1;
        let view_tick = session.ticks();
        session.queue(
            1,
            Input {
                sequence,
                intent,
                view_tick,
            },
        );
        session.step().unwrap();
    }
}

#[test]
fn a_configured_build_applies_at_the_next_spawn_and_travels_in_snapshots() {
    let mut session = duel();
    assert_eq!(session.fighters[1].build, STANDARD);
    assert!(session.configure(2, HEAVY));
    assert!(!session.configure(3, HEAVY));
    rail(&mut session, 1);
    // Alive: rebuilt only between lives.
    assert_eq!(session.fighters[1].build, STANDARD);
    assert_eq!(session.fighters[1].configured, HEAVY);
    let snapshot = session.snapshot();
    let victim = snapshot.slot(1).unwrap();
    assert_eq!((victim.build, victim.configured), (STANDARD, HEAVY));
    assert_eq!(
        decode_snapshot(&encode_snapshot(&snapshot)).unwrap(),
        snapshot
    );

    while session.fighters[1].deaths == 0 {
        rail(&mut session, 1);
    }
    assert_eq!(session.fighters[1].build, STANDARD, "dead, not yet back");
    while !session.fighters[1].alive() {
        session.step().unwrap();
    }
    assert_eq!(session.fighters[1].build, HEAVY);
    assert_eq!(session.fighters[0].build, STANDARD);
    let snapshot = session.snapshot();
    assert_eq!(snapshot.slot(1).unwrap().build, HEAVY);
    assert_eq!(
        decode_snapshot(&encode_snapshot(&snapshot)).unwrap(),
        snapshot
    );
    // A new person in the slot starts from the standard build.
    assert!(session.leave(2).unwrap());
    assert_eq!(session.join(4).unwrap(), Some(1));
    assert_eq!(session.fighters[1].build, STANDARD);
}

#[test]
fn a_snapshot_with_a_tampered_build_is_rejected() {
    let session = Session::with_rules(Arena::greybox(), Rules::deathmatch(2)).unwrap();
    let bytes = encode_snapshot(&session.snapshot());
    // The first fighter's build: its torso code follows the fixed-size fields.
    let standard = STANDARD.candidate();
    let mut pattern = vec![u8::try_from(standard.parts.len()).unwrap()];
    for (socket, id) in &standard.parts {
        pattern.extend_from_slice(&[*socket, *id]);
    }
    let at = bytes
        .windows(pattern.len())
        .position(|window| window == pattern)
        .unwrap();
    let mut tampered = bytes.clone();
    tampered[at + 2] = 99;
    assert_eq!(
        decode_snapshot(&tampered),
        Err(WireError::Build(AssemblyError::UnknownComponent(99)))
    );
}

#[test]
fn matches_with_configured_builds_replay_exactly() {
    let play = || {
        let mut session = duel();
        session.configure(2, HEAVY);
        let mut snapshots = Vec::new();
        for _ in 0..600 {
            rail(&mut session, 1);
            snapshots.extend(encode_snapshot(&session.snapshot()));
        }
        assert!(session.fighters[1].deaths > 0);
        assert_eq!(session.fighters[1].build, HEAVY);
        snapshots
    };
    assert_eq!(play(), play());
}

/// `candidate` plus `(hardpoint, mod)` codes.
fn with_mods(mut candidate: Candidate, mods: &[(u8, ModId)]) -> Candidate {
    candidate.mods = mods
        .iter()
        .map(|&(hardpoint, id)| (hardpoint, id.0))
        .collect();
    candidate
}

#[test]
fn every_mod_trades_something_away() {
    let mut ids: Vec<u8> = BETA.mods.iter().map(|m| m.id.0).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), BETA.mods.len(), "unique mod codes");
    assert_eq!(
        BETA.mods.len(),
        4,
        "a cooling, an energy, a firing and a mount mod"
    );
    for modification in BETA.mods {
        let key = modification.key;
        assert!(key.starts_with("mod."), "{key}");
        assert!(!modification.fits.is_empty(), "{key} fits something");
        // Every mod costs build points and mass...
        assert!(modification.points > 0 && modification.mass > 0, "{key}");
        // ...and one that changes the fire gives something up for it: less damage, slower fire
        // or less range.
        let tuning = modification.tuning;
        if tuning != Tuning::BASE {
            assert!(
                tuning.damage_percent < 100
                    || tuning.refire_percent > 100
                    || tuning.range_percent < 100,
                "{key} is strictly better"
            );
        }
    }
}

#[test]
fn the_modded_loadouts_are_valid_and_contrast() {
    for (name, build) in LOADOUTS {
        assert_eq!(BETA.validate(&build.candidate()), Ok(build), "{name}");
        assert_eq!(loadout(name), Some(build));
        let budget = BETA.budget(&build);
        assert!(budget.carried <= budget.load && budget.draw <= budget.power);
        assert!(budget.heat <= budget.cooling && budget.points <= budget.build_points);
    }
    assert_eq!(loadout(" Marksman "), Some(MARKSMAN));
    assert_eq!(loadout("sniper"), None);
    // The skirmisher draws every bit of the light torso's power; the marksman spends every point.
    let skirmisher = BETA.budget(&SKIRMISHER);
    assert_eq!((skirmisher.draw, skirmisher.power), (100, 100));
    assert_eq!(BETA.budget(&MARKSMAN).points, u32::from(BETA.build_points));
    assert_eq!(SKIRMISHER.mod_on(Weapon::EnergyRifle), Some(RAPID_CYCLER));
    assert_eq!(SKIRMISHER.mod_on(Weapon::Shotgun), Some(HEAT_SINK));
    assert_eq!(MARKSMAN.mod_on(Weapon::RailGun), Some(OVERCHARGER));
    assert_eq!(MARKSMAN.mod_on(Weapon::EnergyRifle), Some(LONG_BARREL));
    assert_eq!(STANDARD.mods().count(), 0);

    let fire = |build: &Assembly, weapon: Weapon| {
        let tuning = BETA.tuning(build, weapon);
        (tuning.refire_ticks(weapon), tuning.delivery(weapon))
    };
    // The skirmisher's rifle fires faster and lighter: more damage a second, less a bolt.
    assert_eq!(
        fire(&SKIRMISHER, Weapon::EnergyRifle),
        (
            4,
            Delivery::Hitscan {
                damage: 9,
                range_units: 8_000
            }
        )
    );
    // The marksman's rifle reaches 120 m but cycles slower; its rail gun kills in two hits
    // instead of three but fires nearly a third slower.
    assert_eq!(
        fire(&MARKSMAN, Weapon::EnergyRifle),
        (
            7,
            Delivery::Hitscan {
                damage: 12,
                range_units: 12_000
            }
        )
    );
    let (refire, rail) = fire(&MARKSMAN, Weapon::RailGun);
    assert_eq!(refire, 94);
    assert!(
        matches!(rail, Delivery::Hitscan { damage, .. } if damage == 106 && 2 * damage >= MAX_HEALTH)
    );
    // Without a mod every weapon fires as shipped (the basic guns need nothing), and a heat sink
    // leaves the fire alone.
    for weapon in Weapon::ALL {
        assert_eq!(
            fire(&STANDARD, weapon),
            (weapon.refire_ticks(), weapon.delivery())
        );
    }
    for (build, weapon) in [(SKIRMISHER, Weapon::Shotgun), (MARKSMAN, Weapon::Laser)] {
        assert_eq!(
            fire(&build, weapon),
            (weapon.refire_ticks(), weapon.delivery())
        );
    }
}

#[test]
fn invalid_mod_combinations_are_rejected_not_dropped() {
    let standard = || STANDARD.candidate();
    let modded = |mods: &[(u8, ModId)]| BETA.validate(&with_mods(standard(), mods));
    // Structure: unknown codes, two mods on one hardpoint, a mod with no weapon under it.
    assert_eq!(
        BETA.validate(&Candidate {
            mods: vec![(LEFT_HAND, 99)],
            ..standard()
        }),
        Err(AssemblyError::UnknownMod(99))
    );
    assert_eq!(
        modded(&[(7, HEAT_SINK)]),
        Err(AssemblyError::UnknownHardpoint(7))
    );
    assert_eq!(
        modded(&[(RIGHT_HAND, HEAT_SINK), (RIGHT_HAND, HEAT_SINK)]),
        Err(AssemblyError::DuplicateMod(Hardpoint::RightHand))
    );
    let mut bare = standard();
    bare.mounts
        .retain(|&(hardpoint, _)| hardpoint != RIGHT_HAND);
    assert_eq!(
        BETA.validate(&with_mods(bare, &[(RIGHT_HAND, HEAT_SINK)])),
        Err(AssemblyError::Unmounted(Hardpoint::RightHand))
    );
    // Attachment: mods fit only their weapons, a long barrel needs a heavy mount.
    assert_eq!(
        modded(&[(LEFT_SHOULDER, OVERCHARGER)]),
        Err(AssemblyError::DoesNotFit(OVERCHARGER, Weapon::Grapple))
    );
    assert_eq!(
        modded(&[(RIGHT_HAND, RAPID_CYCLER)]),
        Err(AssemblyError::DoesNotFit(RAPID_CYCLER, Weapon::EnergyBlade))
    );
    assert_eq!(
        modded(&[(Hardpoint::RightShoulder.index(), LONG_BARREL)]),
        Err(AssemblyError::DoesNotFit(LONG_BARREL, Weapon::Shotgun))
    );
    assert_eq!(
        modded(&[(LEFT_HAND, LONG_BARREL)]),
        Err(AssemblyError::TooHeavyForMount(
            Hardpoint::LeftHand,
            Weapon::EnergyRifle
        ))
    );
    let mut heavy_arm = standard();
    heavy_arm.parts[2] = (LEFT_ARM, 21);
    assert!(
        BETA.validate(&with_mods(heavy_arm, &[(LEFT_HAND, LONG_BARREL)]))
            .is_ok()
    );
    // Budgets: the rapid cycler overheats the standard frame without the heat sink, the
    // overcharger draws more than its torso supplies, and a mod past the build points is refused.
    assert_eq!(
        modded(&[(LEFT_HAND, RAPID_CYCLER)]),
        Err(AssemblyError::Overheated {
            heat: 55,
            cooling: 50
        })
    );
    assert_eq!(
        modded(&[(LEFT_HAND, OVERCHARGER)]),
        Err(AssemblyError::Underpowered {
            draw: 110,
            power: 100
        })
    );
    let mut over = MARKSMAN.candidate();
    over.mods
        .push((Hardpoint::RightShoulder.index(), HEAT_SINK.0));
    assert_eq!(
        BETA.validate(&over),
        Err(AssemblyError::OverBudget {
            points: 125,
            build_points: 120
        })
    );
}

#[test]
fn modded_builds_travel_in_inputs_and_snapshots() {
    for build in [SKIRMISHER, MARKSMAN] {
        let sent = encode_input(Intent::IDLE, 3, None, Some(&build));
        assert_eq!(decode_input(&sent).unwrap().build, Some(build));
    }
    let mut session = duel();
    session.fighters[0].build = MARKSMAN;
    session.configure(2, SKIRMISHER);
    let snapshot = session.snapshot();
    assert_eq!(snapshot.slot(0).unwrap().build, MARKSMAN);
    assert_eq!(snapshot.slot(1).unwrap().configured, SKIRMISHER);
    assert_eq!(
        decode_snapshot(&encode_snapshot(&snapshot)).unwrap(),
        snapshot
    );
}

/// The ticks person 1, built as `build`, fired the rail gun on over `ticks` ticks of the trigger
/// held at person 2, and person 2's deaths.
fn rail_record(build: Assembly, ticks: u32) -> (Vec<u32>, i32) {
    let mut session = duel();
    session.fighters[0].build = build;
    let mut fired = Vec::new();
    for _ in 0..ticks {
        let tick = session.ticks();
        rail(&mut session, 1);
        let shot = session.events.iter().any(|event| {
            matches!(
                event,
                Event::Fired {
                    slot: 0,
                    weapon: Weapon::RailGun,
                    ..
                }
            )
        });
        if shot {
            fired.push(tick);
        }
    }
    (fired, session.fighters[1].deaths)
}

#[test]
fn the_overcharged_rail_gun_kills_in_fewer_hits_and_fires_slower() {
    // Shipped: 72 ticks apart, three hits a kill.
    assert_eq!(rail_record(STANDARD, 120), (vec![12, 84], 0));
    assert_eq!(rail_record(STANDARD, 200), (vec![12, 84, 156], 1));
    // Overcharged: 94 ticks apart, two hits a kill.
    assert_eq!(rail_record(MARKSMAN, 120), (vec![12, 106], 1));
}

#[test]
fn a_configured_modded_build_applies_at_the_next_spawn() {
    let mut session = duel();
    assert!(session.configure(1, MARKSMAN));
    let rail_tuning = |session: &Session| BETA.tuning(&session.fighters[0].build, Weapon::RailGun);
    assert_eq!(rail_tuning(&session), Tuning::BASE);
    // Person 1 falls to person 2's rail gun and comes back as the marksman.
    let intent = Intent {
        yaw: 32_768,
        fire: true,
        select: Some(Weapon::RailGun),
        ..Intent::IDLE
    };
    while session.fighters[0].deaths == 0 {
        let sequence = session.fighters[1].ack + 1;
        let view_tick = session.ticks();
        session.queue(
            2,
            Input {
                sequence,
                intent,
                view_tick,
            },
        );
        session.step().unwrap();
        assert_eq!(rail_tuning(&session), Tuning::BASE, "alive: not rebuilt");
    }
    while !session.fighters[0].alive() {
        session.step().unwrap();
    }
    assert_eq!(session.fighters[0].build, MARKSMAN);
    assert_eq!(rail_tuning(&session).damage_percent, 125);
}

#[test]
fn matches_with_modded_builds_replay_exactly() {
    let play = || {
        let mut session = duel();
        session.fighters[0].build = MARKSMAN;
        session.configure(2, SKIRMISHER);
        let mut snapshots = Vec::new();
        for _ in 0..600 {
            rail(&mut session, 1);
            snapshots.extend(encode_snapshot(&session.snapshot()));
        }
        assert!(session.fighters[1].deaths > 1);
        assert_eq!(session.fighters[1].build, SKIRMISHER);
        snapshots
    };
    assert_eq!(play(), play());
}
