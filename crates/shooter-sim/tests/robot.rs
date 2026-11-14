//! Robot builds as socket-grid component trees: the beta catalog and its sockets, canonical
//! orientations, valid builds of every form (the named loadouts, a mini-tank, a robo-dog, a
//! humanoid, stationary and grappling-only machines) from the same parts and rules, every
//! rejection, the budgets at their limits, builds from untrusted bytes, and builds through a
//! match (configured while alive, applied at the next spawn, carried in snapshots, replayed
//! exactly, firing with their weapon mods).

use shooter_sim::damage::PartDamage;
use shooter_sim::energy::Pool;
use shooter_sim::physics::Vec3i;
use shooter_sim::robot::parts::{
    ARMOR_PLATE, BATTERY, BEAM_LONG, BEAM_SHORT, CHASSIS_L, CHASSIS_M, CHASSIS_S, ENERGY_BLADE,
    ENERGY_RIFLE, GRAPPLE, HINGE, LASER, LEG, RAIL_GUN, ROCKET_LAUNCHER, ROTOR, SENSOR, SHOTGUN,
    TRACK, WHEEL,
};
use shooter_sim::robot::{
    ARMORY, AssemblyError, BETA, Candidate, Catalog, EFFICIENCY_COIL, Face, HEAVY, Kind, LOADOUTS,
    LONG_BARREL, MARKSMAN, MAX_PARTS, ModId, OVERCHARGER, Orientation, PartId, PartType,
    PartTypeId, RAPID_CYCLER, ROOT_CAPACITY, ROOT_REGEN, RawMount, RawPart, SKIRMISHER, STANDARD,
    SocketId, Tuning, loadout,
};

use shooter_sim::trig;
use shooter_sim::weapons::Delivery;
use shooter_sim::wire::{WireError, decode_input, decode_snapshot, encode_input, encode_snapshot};
use shooter_sim::{Arena, Assembly, Event, Input, Intent, Rules, Session, Spawn, Weapon};

const QUARTER: u16 = Orientation::QUARTER;

/// The root part `id`, a `kind`.
fn root(id: u8, kind: &PartType) -> RawPart {
    RawPart {
        id,
        kind: kind.id.0,
        mount: None,
        modification: None,
    }
}

/// Part `id`, a `kind`, on cell `(u, v)` of `face` of part `parent` (a `on`), unturned.
fn on(id: u8, kind: &PartType, parent: u8, on: &PartType, face: Face, u: u8, v: u8) -> RawPart {
    RawPart {
        id,
        kind: kind.id.0,
        mount: Some(RawMount {
            parent,
            socket: on.socket(face, u, v).expect("a socket").0,
            ..RawMount::default()
        }),
        modification: None,
    }
}

fn turned(mut part: RawPart, yaw: u16, pitch: u16, roll: u16) -> RawPart {
    let mount = part.mount.as_mut().unwrap();
    (mount.yaw, mount.pitch, mount.roll) = (yaw, pitch, roll);
    part
}

fn modded(mut part: RawPart, id: ModId) -> RawPart {
    part.modification = Some(id.0);
    part
}

fn build(parts: Vec<RawPart>) -> Candidate {
    Candidate { parts }
}

/// The standard loadout's raw parts, to tamper with.
fn standard() -> Vec<RawPart> {
    STANDARD.candidate().parts
}

/// The standard build with a long beam on each side of its chassis (parts 3 and 4): structure
/// the loadouts no longer carry (shooter#91).
fn armed() -> Vec<RawPart> {
    let mut parts = standard();
    parts.push(on(3, &BEAM_LONG, 0, &CHASSIS_S, Face::PosY, 3, 0));
    parts.push(on(4, &BEAM_LONG, 0, &CHASSIS_S, Face::NegY, 0, 3));
    parts
}

fn with(parts: &mut [RawPart], id: u8) -> &mut RawPart {
    parts.iter_mut().find(|part| part.id == id).unwrap()
}

#[test]
fn the_catalog_is_consistent_and_its_sockets_number_each_cell_once() {
    let mut codes: Vec<u8> = BETA.parts.iter().map(|part| part.id.0).collect();
    codes.sort_unstable();
    codes.dedup();
    assert_eq!(codes.len(), BETA.parts.len(), "unique part type codes");
    let mut keys: Vec<&str> = BETA.parts.iter().map(|part| part.key).collect();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), BETA.parts.len(), "unique part keys");
    for part in BETA.parts {
        let prefix = if matches!(part.kind, Kind::Weapon(_)) {
            "weapon."
        } else {
            "robot."
        };
        assert!(part.key.starts_with(prefix), "{}", part.key);
        assert!(part.points > 0 && part.mass > 0, "{}", part.key);
        assert!(part.cells.iter().all(|&cells| cells > 0), "{}", part.key);
        // Every socket names one cell of one face, and back.
        for socket in 0..part.socket_count() {
            let socket = SocketId(u8::try_from(socket).unwrap());
            let site = part.site(socket).unwrap();
            assert_eq!(part.socket(site.face, site.u, site.v), Some(socket));
        }
        assert_eq!(
            part.site(SocketId(u8::try_from(part.socket_count()).unwrap())),
            None
        );
    }
    // Every shipped weapon is a part.
    for weapon in Weapon::ALL {
        assert_eq!(
            BETA.parts
                .iter()
                .filter(|part| part.kind == Kind::Weapon(weapon))
                .count(),
            1,
            "{weapon:?}"
        );
    }
    // The predetermined chassis sizes: 20 × 40 × 80 cm exposes 28 sockets.
    let chassis: Vec<(&str, [u8; 3], u16)> = BETA
        .parts
        .iter()
        .filter(|part| part.kind == Kind::Chassis)
        .map(|part| (part.key, part.cells, part.socket_count()))
        .collect();
    assert_eq!(
        chassis,
        [
            ("robot.chassis.20x40x80", [1, 2, 4], 28),
            ("robot.chassis.40x40x80", [2, 2, 4], 40),
            ("robot.chassis.40x60x80", [2, 3, 4], 52),
        ]
    );
    // Faces without sockets have none; a joint has one, on its moving side.
    assert_eq!(SENSOR.socket_count(), 0);
    assert_eq!(HINGE.socket_count(), 1);
    assert_eq!(HINGE.socket(Face::NegZ, 0, 0), None);
    assert_eq!(CHASSIS_S.socket(Face::PosZ, 1, 0), None, "off the grid");
    // The control core is gone (shooter#89): its key is no part's and its code 30 stays retired.
    assert!(BETA.parts.iter().all(|part| part.key != "robot.core"));
    assert_eq!(BETA.part_type(PartTypeId(30)), None);

    let mut mods: Vec<u8> = BETA.mods.iter().map(|m| m.id.0).collect();
    mods.sort_unstable();
    mods.dedup();
    assert_eq!(mods.len(), BETA.mods.len(), "unique mod codes");
}

/// The rotation matrix of binary angles applied intrinsically yaw (Z), pitch (Y), roll (X).
fn matrix((yaw, pitch, roll): (u16, u16, u16)) -> [[f64; 3]; 3] {
    let angle = |raw: u16| f64::from(raw) / 65_536.0 * std::f64::consts::TAU;
    let (y, p, r) = (angle(yaw), angle(pitch), angle(roll));
    let (sy, cy, sp, cp, sr, cr) = (y.sin(), y.cos(), p.sin(), p.cos(), r.sin(), r.cos());
    [
        [cy * cp, cy * sp * sr - sy * cr, cy * sp * cr + sy * sr],
        [sy * cp, sy * sp * sr + cy * cr, sy * sp * cr - cy * sr],
        [-sp, cp * sr, cp * cr],
    ]
}

#[test]
fn orientations_fold_into_one_canonical_form_of_the_same_rotation() {
    let angles = [
        0,
        1,
        4_000,
        QUARTER - 1,
        QUARTER,
        QUARTER + 1,
        2 * QUARTER,
        3 * QUARTER - 1,
        3 * QUARTER,
        3 * QUARTER + 1,
        60_000,
        u16::MAX,
    ];
    for yaw in angles {
        for pitch in angles {
            for roll in angles {
                let canonical = Orientation::canonical(yaw, pitch, roll);
                // Pitch within a quarter turn, roll zero at a quarter turn of pitch...
                assert!(canonical.pitch.unsigned_abs() <= QUARTER);
                if canonical.pitch.unsigned_abs() == QUARTER {
                    assert_eq!(canonical.roll, 0);
                }
                // ...the same rotation, and already canonical.
                let (before, after) = (matrix((yaw, pitch, roll)), matrix(canonical.raw()));
                for (row, other) in before.iter().zip(after) {
                    for (a, b) in row.iter().zip(other) {
                        assert!((a - b).abs() < 1e-9, "{yaw} {pitch} {roll}: {canonical:?}");
                    }
                }
                let (y, p, r) = canonical.raw();
                assert_eq!(Orientation::canonical(y, p, r), canonical);
            }
        }
    }
    // Two spellings of one rotation are one orientation.
    assert_eq!(
        Orientation::canonical(100, 2 * QUARTER - 300, 200),
        Orientation::canonical(100 + 2 * QUARTER, 300, 200 + 2 * QUARTER)
    );
    assert_eq!(
        Orientation::canonical(5_000, QUARTER, 1_000),
        Orientation::canonical(4_000, QUARTER, 0)
    );
    // The footprint turns with the yaw's nearest quarter turn.
    let quarters = |yaw| Orientation::canonical(yaw, 0, 0).quarter_turns();
    assert_eq!(
        [0, 8_191, 8_192, QUARTER, 3 * QUARTER, u16::MAX].map(quarters),
        [0, 0, 1, 1, 3, 0]
    );
}

/// A mini-tank: the large chassis on two tracks (the left one turned a quarter so it lies along
/// the side), a rocket turret on a rotor pitched to aim ahead, armor on the front.
fn mini_tank() -> Candidate {
    build(vec![
        root(0, &CHASSIS_L),
        turned(
            on(2, &TRACK, 0, &CHASSIS_L, Face::PosY, 3, 0),
            QUARTER,
            0,
            0,
        ),
        on(3, &TRACK, 0, &CHASSIS_L, Face::NegY, 0, 0),
        on(4, &ROTOR, 0, &CHASSIS_L, Face::PosZ, 1, 1),
        turned(
            on(5, &ROCKET_LAUNCHER, 4, &ROTOR, Face::PosZ, 0, 0),
            0,
            3 * QUARTER,
            0,
        ),
        on(6, &ARMOR_PLATE, 0, &CHASSIS_L, Face::PosX, 0, 0),
        on(7, &SENSOR, 0, &CHASSIS_L, Face::PosX, 2, 3),
    ])
}

/// A robo-dog: the medium chassis on four legs, a sensor head, a shotgun on a hinged neck.
fn robo_dog() -> Candidate {
    build(vec![
        root(0, &CHASSIS_M),
        on(2, &LEG, 0, &CHASSIS_M, Face::NegZ, 0, 0),
        on(3, &LEG, 0, &CHASSIS_M, Face::NegZ, 1, 0),
        on(4, &LEG, 0, &CHASSIS_M, Face::NegZ, 0, 1),
        on(5, &LEG, 0, &CHASSIS_M, Face::NegZ, 1, 1),
        on(6, &SENSOR, 0, &CHASSIS_M, Face::PosX, 0, 3),
        on(7, &HINGE, 0, &CHASSIS_M, Face::PosX, 1, 3),
        on(8, &SHOTGUN, 7, &HINGE, Face::PosZ, 0, 0),
    ])
}

/// A humanoid with hinged shoulders: arms of long beams on hinges, a rifle and a blade at their
/// ends, two legs.
fn humanoid() -> Candidate {
    build(vec![
        root(0, &CHASSIS_S),
        on(2, &SENSOR, 0, &CHASSIS_S, Face::PosZ, 0, 0),
        on(3, &HINGE, 0, &CHASSIS_S, Face::PosY, 3, 0),
        on(4, &HINGE, 0, &CHASSIS_S, Face::NegY, 0, 3),
        on(5, &BEAM_LONG, 3, &HINGE, Face::PosZ, 0, 0),
        on(6, &BEAM_LONG, 4, &HINGE, Face::PosZ, 0, 0),
        on(7, &ENERGY_RIFLE, 5, &BEAM_LONG, Face::PosZ, 0, 0),
        on(8, &ENERGY_BLADE, 6, &BEAM_LONG, Face::PosZ, 0, 0),
        on(9, &LEG, 0, &CHASSIS_S, Face::NegZ, 0, 0),
        on(10, &LEG, 0, &CHASSIS_S, Face::NegZ, 1, 0),
    ])
}

/// A stationary laser turret: no locomotion at all.
fn stationary() -> Candidate {
    build(vec![
        root(0, &CHASSIS_M),
        on(2, &ROTOR, 0, &CHASSIS_M, Face::PosZ, 0, 0),
        on(3, &LASER, 2, &ROTOR, Face::PosZ, 0, 0),
        on(4, &GRAPPLE, 0, &CHASSIS_M, Face::PosZ, 1, 1),
        on(5, &SENSOR, 0, &CHASSIS_M, Face::PosX, 0, 3),
    ])
}

/// A grappling-only machine: a chassis and a grapple hook.
fn grappler() -> Candidate {
    build(vec![
        root(0, &CHASSIS_S),
        on(2, &GRAPPLE, 0, &CHASSIS_S, Face::PosX, 0, 0),
    ])
}

/// A lopsided wheeler: one wheel on its side, one underneath, its sensor sideways.
fn lopsided() -> Candidate {
    build(vec![
        root(0, &CHASSIS_S),
        turned(
            on(1, &SENSOR, 0, &CHASSIS_S, Face::PosY, 2, 0),
            1_234,
            40_000,
            7,
        ),
        on(2, &WHEEL, 0, &CHASSIS_S, Face::PosY, 0, 0),
        on(3, &WHEEL, 0, &CHASSIS_S, Face::NegZ, 0, 0),
    ])
}

#[test]
fn every_form_is_valid_from_the_same_parts_and_rules() {
    let forms = [
        ("mini-tank", mini_tank()),
        ("robo-dog", robo_dog()),
        ("humanoid", humanoid()),
        ("stationary", stationary()),
        ("grappler", grappler()),
        ("lopsided", lopsided()),
    ];
    for (name, candidate) in &forms {
        let assembly = BETA
            .validate(candidate)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        // Canonical: its raw form validates to itself, whatever order the parts came in.
        assert_eq!(BETA.validate(&assembly.candidate()), Ok(assembly), "{name}");
        let mut reversed = candidate.clone();
        reversed.parts.reverse();
        assert_eq!(BETA.validate(&reversed), Ok(assembly), "{name}");
        let ids: Vec<PartId> = assembly.parts().iter().map(|part| part.id).collect();
        assert!(ids.is_sorted(), "{name}");
        // It travels on the wire unchanged.
        let sent = encode_input(Intent::IDLE, 3, None, Some(&assembly));
        assert_eq!(decode_input(&sent).unwrap().build, Some(assembly), "{name}");
    }
    // The stationary, grappling-only and lopsided machines have no locomotion, or hardly any.
    let locomotion = |candidate: &Candidate| {
        let assembly = BETA.validate(candidate).unwrap();
        assembly
            .parts()
            .iter()
            .filter(|part| matches!(BETA.part_type(part.kind).unwrap().kind, Kind::Locomotion(_)))
            .count()
    };
    assert_eq!(locomotion(&stationary()), 0);
    assert_eq!(locomotion(&grappler()), 0);
    assert_eq!(locomotion(&lopsided()), 2);
    // Orientations are stored canonically: the rocket turret's pitch, the lopsided sensor's turn.
    let tank = BETA.validate(&mini_tank()).unwrap();
    let rockets = tank.part(PartId(5)).unwrap().mount.unwrap();
    assert_eq!(rockets.orientation.pitch, -i16::try_from(QUARTER).unwrap());
    let sensor = BETA.validate(&lopsided()).unwrap();
    let sensor = sensor.part(PartId(1)).unwrap().mount.unwrap();
    assert_eq!(sensor.orientation, Orientation::canonical(1_234, 40_000, 7));
    assert_ne!(sensor.orientation.raw(), (1_234, 40_000, 7));
    // The tree: the turret hangs off the rotor, the rotor off the chassis.
    assert_eq!(
        tank.children(PartId(4))
            .map(|part| part.id)
            .collect::<Vec<_>>(),
        [PartId(5)]
    );
}

#[test]
fn larger_parts_reserve_their_footprint_and_turn_it_with_their_yaw() {
    // Unturned, the left track runs across the side face; turned a quarter it runs along it.
    let mut tank = mini_tank();
    tank.parts[1] = on(2, &TRACK, 0, &CHASSIS_L, Face::PosY, 3, 0);
    tank.parts
        .push(on(8, &SENSOR, 0, &CHASSIS_L, Face::PosY, 3, 1));
    assert_eq!(
        BETA.validate(&tank),
        Err(AssemblyError::Overlap(PartId(2), PartId(8)))
    );
    let mut along = mini_tank();
    along
        .parts
        .push(on(8, &SENSOR, 0, &CHASSIS_L, Face::PosY, 3, 1));
    assert!(BETA.validate(&along).is_ok());
    along
        .parts
        .push(on(9, &LEG, 0, &CHASSIS_L, Face::PosY, 0, 0));
    assert_eq!(
        BETA.validate(&along),
        Err(AssemblyError::Overlap(PartId(2), PartId(9)))
    );
    // A 40 cm plate on the chest covers the weapons' cells.
    let mut plated = standard();
    plated.push(on(11, &ARMOR_PLATE, 0, &CHASSIS_S, Face::PosX, 0, 2));
    assert_eq!(
        BETA.validate(&build(plated)),
        Err(AssemblyError::Overlap(PartId(8), PartId(11)))
    );
    // Below them it fits.
    let mut plated = standard();
    plated.push(on(11, &ARMOR_PLATE, 0, &CHASSIS_S, Face::PosX, 0, 0));
    assert!(BETA.validate(&build(plated)).is_ok());
    // Two parts on one socket.
    let mut stacked = standard();
    stacked.push(on(11, &SENSOR, 0, &CHASSIS_S, Face::PosZ, 0, 0));
    assert_eq!(
        BETA.validate(&build(stacked)),
        Err(AssemblyError::Overlap(PartId(2), PartId(11)))
    );
    // A long barrel makes the rifle a two-cell module: below the shotgun it reaches its cell.
    let mut barrel = standard();
    assert_eq!(
        with(&mut barrel, 7).mount.unwrap().socket,
        CHASSIS_S.socket(Face::PosX, 1, 2).unwrap().0
    );
    *with(&mut barrel, 7) = modded(*with(&mut barrel, 7), LONG_BARREL);
    assert_eq!(
        BETA.validate(&build(barrel)),
        Err(AssemblyError::Overlap(PartId(7), PartId(10)))
    );
    // Overhanging the face's edge is fine: the rail gun across the back of the top, beside the
    // sensor, its second cell past the edge.
    let mut overhang = standard();
    *with(&mut overhang, 7) = on(7, &RAIL_GUN, 0, &CHASSIS_S, Face::PosZ, 0, 1);
    assert!(BETA.validate(&build(overhang)).is_ok());
}

#[test]
fn broken_structures_are_rejected() {
    let reject = |parts: Vec<RawPart>| BETA.validate(&build(parts)).unwrap_err();
    // No control core any more (shooter#89): a build without one is whole, and one naming the
    // retired core's code 30 is an unknown part, wherever it sits.
    assert!(STANDARD.parts().iter().all(|part| part.kind.0 != 30));
    let mut cored = standard();
    cored.push(on(1, &SENSOR, 0, &CHASSIS_S, Face::NegX, 2, 0));
    assert!(BETA.validate(&build(cored.clone())).is_ok());
    with(&mut cored, 1).kind = 30;
    assert_eq!(reject(cored), AssemblyError::UnknownPart(30));
    // A part on an interior socket (the mount face of an arm, which rests on the torso).
    let mut interior = armed();
    interior.push(on(1, &SENSOR, 3, &BEAM_LONG, Face::NegZ, 0, 0));
    assert_eq!(reject(interior), AssemblyError::InteriorSocket(PartId(1)));
    // The root's own underside is exterior: the legs stand there.
    // A part twice (two parents), a second root, no root, a root that is no chassis.
    let mut twice = standard();
    twice.push(on(7, &ENERGY_RIFLE, 0, &CHASSIS_S, Face::PosX, 0, 0));
    assert_eq!(reject(twice), AssemblyError::DuplicateId(PartId(7)));
    let mut roots = standard();
    with(&mut roots, 2).mount = None;
    assert_eq!(
        reject(roots),
        AssemblyError::MultipleRoots(PartId(0), PartId(2))
    );
    let mut rootless = armed();
    *with(&mut rootless, 0) = on(0, &CHASSIS_S, 3, &BEAM_LONG, Face::PosX, 0, 0);
    assert_eq!(reject(rootless), AssemblyError::NoRoot);
    assert_eq!(
        reject(vec![root(0, &SENSOR)]),
        AssemblyError::RootNotChassis(PartId(0))
    );
    assert_eq!(reject(Vec::new()), AssemblyError::NoRoot);
    // Parents and sockets that do not exist, or a part that carries nothing.
    let mut orphan = standard();
    with(&mut orphan, 9).mount.as_mut().unwrap().parent = 99;
    assert_eq!(reject(orphan), AssemblyError::UnknownParent(PartId(9)));
    let mut socketless = standard();
    with(&mut socketless, 9).mount.as_mut().unwrap().socket = 28;
    assert_eq!(reject(socketless), AssemblyError::NoSocket(PartId(9)));
    let mut on_weapon = standard();
    *with(&mut on_weapon, 9) = on(9, &GRAPPLE, 0, &CHASSIS_S, Face::PosZ, 0, 1);
    with(&mut on_weapon, 9).mount.as_mut().unwrap().parent = 7;
    assert_eq!(reject(on_weapon), AssemblyError::NoSocket(PartId(9)));
    // Cycles: a part its own parent, two arms on each other.
    let mut selfish = armed();
    *with(&mut selfish, 3) = on(3, &BEAM_LONG, 3, &BEAM_LONG, Face::PosX, 0, 0);
    assert_eq!(reject(selfish), AssemblyError::Cycle(PartId(3)));
    let mut looped = armed();
    *with(&mut looped, 3) = on(3, &BEAM_LONG, 4, &BEAM_LONG, Face::PosX, 0, 0);
    *with(&mut looped, 4) = on(4, &BEAM_LONG, 3, &BEAM_LONG, Face::PosX, 0, 0);
    assert_eq!(reject(looped), AssemblyError::Cycle(PartId(3)));
    // Unknown codes and the part ceiling.
    let mut unknown = standard();
    with(&mut unknown, 2).kind = 200;
    assert_eq!(reject(unknown), AssemblyError::UnknownPart(200));
    let mut too_many = vec![root(0, &CHASSIS_L)];
    too_many.extend((1..=u8::try_from(MAX_PARTS).unwrap()).map(|id| RawPart {
        id,
        ..root(id, &SENSOR)
    }));
    assert_eq!(reject(too_many), AssemblyError::TooManyParts(MAX_PARTS + 1));
}

#[test]
fn the_part_ceiling_holds_exactly() {
    // The large chassis' 40 side, front and back cells hold 31 one-cell parts: 32 parts.
    let mut parts = vec![root(0, &CHASSIS_L)];
    let mut id = 1;
    'faces: for face in [Face::PosX, Face::NegX, Face::PosY, Face::NegY] {
        let (columns, rows) = face.grid(CHASSIS_L.cells);
        for v in 0..rows {
            for u in 0..columns {
                if usize::from(id) == MAX_PARTS {
                    break 'faces;
                }
                parts.push(on(id, &BEAM_SHORT, 0, &CHASSIS_L, face, u, v));
                id += 1;
            }
        }
    }
    assert_eq!(parts.len(), MAX_PARTS);
    let full = BETA.validate(&build(parts.clone())).unwrap();
    assert_eq!(BETA.budget(&full).parts, u32::try_from(MAX_PARTS).unwrap());
    parts.push(on(id, &SENSOR, 0, &CHASSIS_L, Face::NegZ, 0, 0));
    assert_eq!(
        BETA.validate(&build(parts)),
        Err(AssemblyError::TooManyParts(MAX_PARTS + 1))
    );
}

#[test]
fn mods_and_weapons_follow_their_rules() {
    let reject = |parts: Vec<RawPart>| BETA.validate(&build(parts)).unwrap_err();
    let mut unknown = standard();
    with(&mut unknown, 7).modification = Some(99);
    assert_eq!(reject(unknown), AssemblyError::UnknownMod(99));
    let mut sensor = standard();
    with(&mut sensor, 2).modification = Some(EFFICIENCY_COIL.0);
    assert_eq!(reject(sensor), AssemblyError::NotAWeapon(PartId(2)));
    let mut grapple = standard();
    with(&mut grapple, 9).modification = Some(OVERCHARGER.0);
    assert_eq!(
        reject(grapple),
        AssemblyError::DoesNotFit(OVERCHARGER, Weapon::Grapple)
    );
    let mut blade = standard();
    with(&mut blade, 8).modification = Some(RAPID_CYCLER.0);
    assert_eq!(
        reject(blade),
        AssemblyError::DoesNotFit(RAPID_CYCLER, Weapon::EnergyBlade)
    );
    let mut shotgun = standard();
    with(&mut shotgun, 10).modification = Some(LONG_BARREL.0);
    assert_eq!(
        reject(shotgun),
        AssemblyError::DoesNotFit(LONG_BARREL, Weapon::Shotgun)
    );
    let mut twice = standard();
    twice.push(on(11, &ENERGY_RIFLE, 0, &CHASSIS_S, Face::PosX, 0, 0));
    assert_eq!(
        reject(twice),
        AssemblyError::DuplicateWeapon(Weapon::EnergyRifle)
    );
}

#[test]
fn every_mod_trades_something_away() {
    assert_eq!(
        BETA.mods.len(),
        4,
        "two firing mods, a mount mod and an energy mod"
    );
    // No mod is pure upkeep any more: every one changes the fire, and none is strictly better
    // (damage, refire, range and energy a shot all count).
    for modification in BETA.mods {
        let key = modification.key;
        assert!(key.starts_with("mod."), "{key}");
        assert!(!modification.fits.is_empty(), "{key} fits something");
        assert!(modification.points > 0 && modification.mass > 0, "{key}");
        let tuning = modification.tuning;
        assert_ne!(tuning, Tuning::BASE, "{key} changes nothing");
        assert!(
            tuning.damage_percent < 100
                || tuning.refire_percent > 100
                || tuning.range_percent < 100
                || tuning.energy_percent > 100,
            "{key} is strictly better"
        );
    }
    // The heat sink is gone with heat (shooter#72); its code stays retired.
    assert!(BETA.mods.iter().all(|m| m.key != "mod.heat-sink"));
    assert_eq!(BETA.modification(ModId(4)), None);
    let coil = BETA.modification(EFFICIENCY_COIL).unwrap();
    assert_eq!(coil.key, "mod.efficiency-coil");
    assert_eq!(
        (coil.tuning.energy_percent, coil.tuning.refire_percent),
        (60, 125)
    );
}

#[test]
fn the_root_chassis_and_batteries_make_the_energy_pool() {
    // The root chassis alone: 100 stored, 24 a second, whatever its size; a battery adds 60
    // stored and nothing else.
    let standard = BETA.budget(&STANDARD);
    assert_eq!((standard.capacity, standard.regen), (100, 24));
    let heavy = BETA.budget(&HEAVY);
    assert_eq!((heavy.capacity, heavy.regen), (160, 24));
    assert_eq!((ROOT_CAPACITY, ROOT_REGEN), (100, 24));
    // Another chassis section is ordinary structure: no baseline of its own.
    let stacked = BETA
        .validate(&build(vec![
            root(0, &CHASSIS_L),
            on(1, &CHASSIS_S, 0, &CHASSIS_L, Face::PosZ, 0, 0),
        ]))
        .unwrap();
    let budget = BETA.budget(&stacked);
    assert_eq!((budget.capacity, budget.regen), (100, 24));
    assert_eq!(BATTERY.key, "robot.battery");
    assert_eq!(BATTERY.kind, Kind::Battery);
    assert_eq!(
        (
            BATTERY.capacity,
            BATTERY.regen,
            BATTERY.points,
            BATTERY.mass
        ),
        (60, 0, 5, 20)
    );
    assert_eq!(BATTERY.health, 60, "a battery is a damageable component");
    assert_eq!(BATTERY.cells, [1, 2, 1], "a two-cell footprint");
    // Batteries stack; each one takes its footprint and build points.
    let mut charged = MARKSMAN.candidate();
    charged
        .parts
        .push(on(11, &BATTERY, 0, &CHASSIS_M, Face::NegX, 3, 0));
    let charged = BETA.validate(&charged).unwrap();
    assert_eq!(BETA.budget(&charged).capacity, 160);
    assert_eq!(BETA.budget(&charged).points, 104);
    let mut overlapping = charged.candidate();
    overlapping
        .parts
        .push(on(12, &BATTERY, 0, &CHASSIS_M, Face::NegX, 3, 1));
    assert_eq!(
        BETA.validate(&overlapping),
        Err(AssemblyError::Overlap(PartId(11), PartId(12)))
    );
    let mut second = charged.candidate();
    second
        .parts
        .push(on(12, &BATTERY, 0, &CHASSIS_M, Face::NegX, 0, 0));
    let second = BETA.validate(&second).unwrap();
    assert_eq!(BETA.budget(&second).capacity, 220);
    assert_eq!(BETA.budget(&second).points, 109);
    let mut third = second.candidate();
    third
        .parts
        .push(on(13, &BATTERY, 0, &CHASSIS_M, Face::NegX, 1, 0));
    assert_eq!(
        BETA.validate(&third),
        Err(AssemblyError::OverBudget {
            points: 114,
            build_points: 110
        })
    );
    let roomy = Catalog {
        build_points: 115,
        ..BETA
    };
    let third = roomy.validate(&third).unwrap();
    assert_eq!(roomy.budget(&third).capacity, 280);
    // Every part type's own capacity and regeneration: none regenerates and only the battery
    // stores (the root chassis' baseline is the root's, not its type's).
    for part in BETA.parts {
        match part.kind {
            Kind::Battery => assert_eq!(part.regen, 0),
            _ => assert_eq!((part.capacity, part.regen), (0, 0), "{}", part.key),
        }
    }
}

#[test]
fn budgets_hold_exactly_at_their_limits_and_fail_one_over() {
    // Build points: the heavy build (with its battery) costs 99.
    let heavy = BETA.budget(&HEAVY);
    assert_eq!(heavy.points, 99);
    let exact = Catalog {
        build_points: 99,
        ..BETA
    };
    assert_eq!(exact.validate(&HEAVY.candidate()), Ok(HEAVY));
    let short = Catalog {
        build_points: 98,
        ..BETA
    };
    assert_eq!(
        short.validate(&HEAVY.candidate()),
        Err(AssemblyError::OverBudget {
            points: 99,
            build_points: 98
        })
    );
    // Two plates fit the beta's 110 on the heavy build, a third passes it.
    let mut plated = HEAVY.candidate();
    plated
        .parts
        .push(on(12, &ARMOR_PLATE, 0, &CHASSIS_M, Face::NegX, 0, 0));
    plated
        .parts
        .push(on(13, &ARMOR_PLATE, 0, &CHASSIS_M, Face::PosY, 0, 0));
    assert!(BETA.validate(&plated).is_ok());
    plated
        .parts
        .push(on(14, &ARMOR_PLATE, 0, &CHASSIS_M, Face::NegY, 0, 0));
    assert_eq!(
        BETA.validate(&plated),
        Err(AssemblyError::OverBudget {
            points: 111,
            build_points: 110
        })
    );
    // No power or heat budget is left (D2.i70.3, D2.i70.4): the builds those once rejected (the
    // marksman with a second sensor, which overdrew the old core's 130 power, and the skirmisher's
    // cycler without a heat sink) are valid; energy limits them in play instead.
    let mut sensed = MARKSMAN.candidate();
    sensed
        .parts
        .push(on(11, &SENSOR, 0, &CHASSIS_M, Face::PosY, 0, 0));
    assert!(BETA.validate(&sensed).is_ok());
    let mut bare = SKIRMISHER.candidate();
    with(&mut bare.parts, 10).modification = None;
    assert!(BETA.validate(&bare).is_ok());
}

#[test]
fn the_named_loadouts_are_trees_with_their_mods_on_their_weapons() {
    for (name, build) in LOADOUTS {
        assert_eq!(BETA.validate(&build.candidate()), Ok(build), "{name}");
        assert_eq!(loadout(name), Some(build));
        let budget = BETA.budget(&build);
        assert!(budget.capacity >= 100 && budget.regen == 24, "{name}");
        assert!(budget.points <= budget.build_points);
        // The arsenal they mount, once each.
        let weapons: Vec<Weapon> = BETA.weapons(&build).map(|(_, weapon)| weapon).collect();
        assert_eq!(weapons.len(), 4, "{name}");
    }
    assert_eq!(loadout(" Marksman "), Some(MARKSMAN));
    assert_eq!(loadout("sniper"), None);
    // A chassis, a sensor, two legs and four weapons (no arms since shooter#91).
    assert_eq!(STANDARD.parts().len(), 8);
    assert_eq!(BETA.budget(&STANDARD).points, 69);
    assert_eq!(BETA.budget(&SKIRMISHER).points, 84);
    assert_eq!(BETA.budget(&MARKSMAN).points, 99);
    assert_eq!(
        BETA.mod_on(&SKIRMISHER, Weapon::EnergyRifle),
        Some(RAPID_CYCLER)
    );
    assert_eq!(
        BETA.mod_on(&SKIRMISHER, Weapon::Shotgun),
        Some(EFFICIENCY_COIL)
    );
    assert_eq!(HEAVY.parts().len(), 9, "the heavy build carries a battery");
    assert_eq!(BETA.mod_on(&MARKSMAN, Weapon::RailGun), Some(OVERCHARGER));
    assert_eq!(
        BETA.mod_on(&MARKSMAN, Weapon::EnergyRifle),
        Some(LONG_BARREL)
    );
    assert_eq!(BETA.mods(&STANDARD).count(), 0);
    assert_eq!(
        BETA.mods(&MARKSMAN).collect::<Vec<_>>(),
        [
            (PartId(7), Weapon::EnergyRifle, LONG_BARREL),
            (PartId(9), Weapon::RailGun, OVERCHARGER)
        ]
    );

    let fire = |build: &Assembly, weapon: Weapon| {
        let tuning = BETA.tuning(build, weapon);
        (tuning.refire_ticks(weapon), tuning.delivery(weapon))
    };
    // The skirmisher's rifle fires faster and lighter.
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
    // The marksman's rifle reaches 120 m but cycles slower; its rail gun kills in two hits.
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
    assert!(matches!(rail, Delivery::Hitscan { damage, .. } if damage == 106));
    for weapon in Weapon::ALL {
        assert_eq!(
            fire(&STANDARD, weapon),
            (weapon.refire_ticks(), weapon.delivery())
        );
    }
    assert_eq!(
        fire(&MARKSMAN, Weapon::Laser),
        (Weapon::Laser.refire_ticks(), Weapon::Laser.delivery())
    );
    // The skirmisher's efficiency-coiled shotgun blasts as hard, a quarter slower, for 60 % of
    // the energy.
    assert_eq!(
        fire(&SKIRMISHER, Weapon::Shotgun),
        (68, Weapon::Shotgun.delivery())
    );
    // What each activation costs (milli-units): the shipped weapons' own, the mods' tuned.
    let energy = |build: &Assembly, weapon: Weapon| BETA.tuning(build, weapon).energy(weapon);
    for weapon in Weapon::ALL {
        assert_eq!(energy(&STANDARD, weapon), weapon.energy());
    }
    assert_eq!(energy(&SKIRMISHER, Weapon::Shotgun), 7_200);
    assert_eq!(energy(&SKIRMISHER, Weapon::EnergyRifle), 2_000);
    assert_eq!(energy(&MARKSMAN, Weapon::RailGun), 45_000);
    assert_eq!(energy(&MARKSMAN, Weapon::EnergyRifle), 2_000);
}

/// The direction `quaternion` (`[x, y, z, w]`) turns a part's own +Z (its mount normal, the way
/// the weapons kit's muzzles point) to.
fn mount_normal([x, y, z, w]: [f32; 4]) -> [f32; 3] {
    [
        2.0 * (x * z + w * y),
        2.0 * (y * z - w * x),
        1.0 - 2.0 * (x * x + y * y),
    ]
}

#[test]
fn every_loadout_weapon_faces_the_aim_from_the_chassis() {
    // The arm beams are gone (shooter#91): every weapon sits on the root chassis' front, its own
    // +Z, along which the kit's muzzles and blade point, within a degree of the robot's forward
    // (+X, the aim at rest).
    let forward = 1.0_f32.to_radians().cos();
    for (name, build) in LOADOUTS {
        assert!(
            build
                .parts()
                .iter()
                .all(|part| !matches!(BETA.part_type(part.kind).unwrap().kind, Kind::Beam)),
            "{name}"
        );
        let placed = shooter_sim::mechanics::placements(&build);
        let mut weapons = 0;
        for (part, placement) in build.parts().iter().zip(&placed) {
            if !matches!(BETA.part_type(part.kind).unwrap().kind, Kind::Weapon(_)) {
                continue;
            }
            weapons += 1;
            let mount = part.mount.unwrap();
            assert_eq!(mount.parent, PartId(0), "{name}: {:?}", part.id);
            let [x, ..] = mount_normal(placement.orientation);
            assert!(x > forward, "{name}: {:?} points {x}", part.id);
            // In front of the chassis, not inside it.
            assert!(placement.centre[0] > 0.0, "{name}: {:?}", part.id);
        }
        assert_eq!(weapons, 4, "{name}");
    }
}

/// The bytes `build` travels as (after an input's build flag).
fn build_bytes(build: &Assembly) -> Vec<u8> {
    let plain = encode_input(Intent::IDLE, 3, None, None);
    encode_input(Intent::IDLE, 3, None, Some(build))[plain.len()..].to_vec()
}

#[test]
fn builds_from_untrusted_bytes_are_validated() {
    for (_, build) in LOADOUTS {
        let sent = encode_input(Intent::IDLE, 3, None, Some(&build));
        assert_eq!(decode_input(&sent).unwrap().build, Some(build));
    }
    let plain = encode_input(Intent::IDLE, 3, None, None);
    let forged = |bytes: &[u8]| {
        let mut forged = plain[..plain.len() - 1].to_vec();
        forged.push(1);
        forged.extend_from_slice(bytes);
        decode_input(&forged)
    };
    let standard = build_bytes(&STANDARD);
    assert_eq!(forged(&standard).unwrap().build, Some(STANDARD));
    // Count, then per part: ID, type, flags, [parent, socket], [angles], [mod].
    assert_eq!(&standard[..4], &[8, 0, CHASSIS_S.id.0, 0]);
    let mut unknown = standard.clone();
    unknown[2] = 99;
    assert_eq!(
        forged(&unknown),
        Err(WireError::Build(AssemblyError::UnknownPart(99)))
    );
    // The second part (the sensor, 5 bytes from the 4th on) named as the retired control core.
    let mut cored = standard.clone();
    assert_eq!(&cored[4..7], &[2, SENSOR.id.0, 1]);
    cored[5] = 30;
    assert_eq!(
        forged(&cored),
        Err(WireError::Build(AssemblyError::UnknownPart(30)))
    );
    // The sensor moved onto the grapple's socket.
    let mut stacked = standard.clone();
    stacked[8] = CHASSIS_S.socket(Face::PosX, 0, 3).unwrap().0;
    assert_eq!(
        forged(&stacked),
        Err(WireError::Build(AssemblyError::Overlap(
            PartId(2),
            PartId(9)
        )))
    );
    // Unknown flags, a turned root, truncation: malformed.
    let mut flags = standard.clone();
    flags[3] = 8;
    assert!(matches!(forged(&flags), Err(WireError::Malformed(_))));
    let mut turned_root = standard.clone();
    turned_root[3] = 2;
    assert!(matches!(forged(&turned_root), Err(WireError::Malformed(_))));
    assert!(matches!(
        forged(&standard[..standard.len() - 1]),
        Err(WireError::Malformed(_))
    ));
    let mut flag = plain.clone();
    *flag.last_mut().unwrap() = 2;
    assert!(matches!(decode_input(&flag), Err(WireError::Malformed(_))));
    // A turned part carries its canonical angles; any spelling of them arrives canonical.
    let mut raw = lopsided();
    let sent = BETA.validate(&raw).unwrap();
    assert_eq!(
        decode_input(&encode_input(Intent::IDLE, 3, None, Some(&sent)))
            .unwrap()
            .build,
        Some(sent)
    );
    let mount = raw.parts[1].mount.as_mut().unwrap();
    let canonical = Orientation::canonical(mount.yaw, mount.pitch, mount.roll);
    (mount.yaw, mount.pitch, mount.roll) = canonical.raw();
    assert_eq!(BETA.validate(&raw), Ok(sent));
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
        waypoints: Vec::new(),
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

/// The yaw and pitch from `from`'s eye to `to`'s root chassis a cell below its middle, where the
/// host places it (straight ahead while `to` is dead).
#[allow(clippy::cast_possible_truncation)]
fn at_chassis(session: &Session, from: usize, to: usize) -> (u16, i16) {
    let target = &session.fighters[to];
    if !target.alive() {
        return (if from == 0 { 0 } else { 32_768 }, 0);
    }
    let eye = session.fighters[from].player.eye(&session.world);
    let parts = target.hitboxes(target.player.center(&session.world), target.intent.yaw);
    let mut chassis = parts
        .get(PartDamage::root(&target.build).unwrap())
        .unwrap()
        .center;
    // A cell below its middle: below the weapons on a loadout's chest.
    chassis[1] -= 20.0;
    let d = [0, 1, 2].map(|axis| chassis[axis] - eye[axis]);
    let yaw = trig::yaw_from_vector(d[0].round() as i32, d[2].round() as i32).unwrap();
    let run = d[0].hypot(d[2]).round() as i32;
    let pitch = trig::yaw_from_vector(d[1].round() as i32, run).unwrap() as i16;
    (yaw, pitch)
}

/// Person 1 holds the rail gun's trigger for `ticks` ticks, aimed at person 2's root chassis
/// (through whatever is in front of it).
fn rail(session: &mut Session, ticks: u32) {
    for _ in 0..ticks {
        let (yaw, pitch) = at_chassis(session, 0, 1);
        let intent = Intent {
            fire: true,
            yaw,
            pitch,
            select: Some(Weapon::RailGun),
            ..Intent::IDLE
        };
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
    let tank = BETA.validate(&mini_tank()).unwrap();
    let mut session = duel();
    assert_eq!(session.fighters[1].build, STANDARD);
    assert!(session.configure(2, tank));
    assert!(!session.configure(3, tank));
    rail(&mut session, 1);
    // Alive: rebuilt only between lives.
    assert_eq!(session.fighters[1].build, STANDARD);
    assert_eq!(session.fighters[1].configured, tank);
    let snapshot = session.snapshot();
    let victim = snapshot.slot(1).unwrap();
    assert_eq!((victim.build, victim.configured), (STANDARD, tank));
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
    assert_eq!(session.fighters[1].build, tank);
    assert_eq!(session.fighters[0].build, STANDARD);
    // Back with the new build's energy pool full.
    assert_eq!(session.fighters[1].energy, Pool::of(&tank).capacity);
    let snapshot = session.snapshot();
    assert_eq!(snapshot.slot(1).unwrap().build, tank);
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
    let pattern = build_bytes(&STANDARD);
    let at = bytes
        .windows(pattern.len())
        .position(|window| window == pattern)
        .unwrap();
    let mut tampered = bytes.clone();
    tampered[at + 2] = 99;
    assert_eq!(
        decode_snapshot(&tampered),
        Err(WireError::Build(AssemblyError::UnknownPart(99)))
    );
}

#[test]
fn matches_with_configured_builds_replay_exactly() {
    let dog = BETA.validate(&robo_dog()).unwrap();
    let play = || {
        let mut session = duel();
        session.fighters[0].rebuild(MARKSMAN);
        session.configure(2, dog);
        let mut snapshots = Vec::new();
        for _ in 0..600 {
            rail(&mut session, 1);
            snapshots.extend(encode_snapshot(&session.snapshot()));
        }
        assert!(session.fighters[1].deaths > 1);
        assert_eq!(session.fighters[1].build, dog);
        snapshots
    };
    assert_eq!(play(), play());
}

/// The ticks person 1, built as `build`, fired the rail gun on over `ticks` ticks of the trigger
/// held at person 2, and person 2's deaths.
fn rail_record(build: Assembly, ticks: u32) -> (Vec<u32>, i32) {
    let mut session = duel();
    session.fighters[0].rebuild(build);
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
    // Shipped: 72 ticks apart, four hits a kill aimed at the root chassis from the front (270
    // on the standard build: 85 a hit), as it took before the core went (shooter#89: two for the
    // chassis, then two for the core); the heavy build's rail gun has no mod. The standard build
    // carries none, so it never fires one (shooter#92). The heavy build holds its rail gun from
    // the start (its first weapon), the marksman switches to it from its rifle.
    assert_eq!(rail_record(STANDARD, 120), (vec![], 0));
    assert_eq!(rail_record(HEAVY, 300), (vec![0, 72, 144, 216, 288], 1));
    // Overcharged (106): 94 ticks apart, three hits a kill, as before (two for the chassis, then
    // one for the core).
    assert_eq!(rail_record(MARKSMAN, 220), (vec![12, 106, 200], 1));
}

#[test]
fn a_configured_modded_build_applies_at_the_next_spawn() {
    let mut session = duel();
    assert!(session.configure(1, MARKSMAN));
    let rail_tuning = |session: &Session| BETA.tuning(&session.fighters[0].build, Weapon::RailGun);
    assert_eq!(rail_tuning(&session), Tuning::BASE);
    while session.fighters[0].deaths == 0 {
        let (yaw, pitch) = at_chassis(&session, 1, 0);
        let intent = Intent {
            yaw,
            pitch,
            fire: true,
            select: Some(Weapon::RailGun),
            ..Intent::IDLE
        };
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
fn the_armory_mounts_every_weapon_and_is_over_budget_only() {
    let mut weapons: Vec<Weapon> = BETA.weapons(&ARMORY).map(|(_, weapon)| weapon).collect();
    weapons.sort();
    assert_eq!(weapons, Weapon::ALL.to_vec());
    assert!(matches!(
        BETA.validate(&ARMORY.candidate()),
        Err(AssemblyError::OverBudget { .. })
    ));
}
