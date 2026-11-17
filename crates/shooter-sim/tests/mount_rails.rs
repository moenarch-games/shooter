//! Mount rails, adapters and structural extensions (shooter#126, Atlas D2.i123.11): ordinary
//! parts of the single-parent socket tree that expose further hardpoints. The one validator
//! accepts builds on them and refuses overlaps, cycles, disconnected children and builds over
//! budget; what they carry is placed, weighed and struck where they carry it; losing a support
//! loses everything on it, in the damage model, the mechanics and a match; and the builds travel
//! the wire unchanged.
// The platform's floats only check results here; the simulation uses libm (clippy.toml).
#![allow(clippy::disallowed_methods)]

use shooter_sim::aim;
use shooter_sim::damage::PartDamage;
use shooter_sim::mechanics::{BodyKey, Layout, placements};
use shooter_sim::physics::Vec3i;
use shooter_sim::robot::parts::{
    ADAPTER, CHASSIS_M, ENERGY_RIFLE, LASER, MOUNT_RAIL_LONG, MOUNT_RAIL_SHORT, RAIL_GUN, SHOTGUN,
};
use shooter_sim::robot::{
    ADAPTER_MAST, AssemblyError, BETA, CROSS_RAIL, Candidate, Face, Kind, Orientation, PartId,
    PartType, RAIL_RACK, RawMount, RawPart, SocketId,
};
use shooter_sim::wire::{decode_input, decode_snapshot, encode_input, encode_snapshot};
use shooter_sim::{Arena, Assembly, Event, Input, Intent, Rules, Session, Spawn, Weapon};

const QUARTER: u16 = Orientation::QUARTER;

/// Every fixture on extensions, by name.
const FIXTURES: [(&str, Assembly); 3] = [
    ("rail-rack", RAIL_RACK),
    ("cross-rail", CROSS_RAIL),
    ("adapter-mast", ADAPTER_MAST),
];

/// Part `id`, a `kind`, on cell `(u, v)` of `face` of part `parent` (a `on`), turned `yaw`.
fn on(
    id: u8,
    kind: &PartType,
    parent: u8,
    on: &PartType,
    (face, u, v): (Face, u8, u8),
    yaw: u16,
) -> RawPart {
    RawPart {
        id,
        kind: kind.id.0,
        mount: Some(RawMount {
            parent,
            socket: on.socket(face, u, v).expect("a socket").0,
            yaw,
            ..RawMount::default()
        }),
        modification: None,
    }
}

/// `build`'s raw parts with `extra` added.
fn plus(build: &Assembly, extra: impl IntoIterator<Item = RawPart>) -> Candidate {
    let mut candidate = build.candidate();
    candidate.parts.extend(extra);
    candidate
}

fn index_of(build: &Assembly, part: PartId) -> usize {
    build.parts().iter().position(|p| p.id == part).unwrap()
}

/// The bits of the parts `ids` of `build`.
fn bits(build: &Assembly, ids: &[u8]) -> u32 {
    ids.iter().map(|&id| 1 << index_of(build, PartId(id))).sum()
}

/// `vector` turned by the unit quaternion `[x, y, z, w]`.
fn rotate([x, y, z, w]: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let u = [x, y, z];
    let cross = |a: [f32; 3], b: [f32; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let t = cross(u, v).map(|c| 2.0 * c);
    let ut = cross(u, t);
    [0, 1, 2].map(|i| v[i] + w * t[i] + ut[i])
}

fn close(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(b).all(|(a, b)| (a - b).abs() < 0.01)
}

#[test]
fn rails_and_adapters_are_catalog_parts_with_hardpoints() {
    // A rail's hardpoints are its top's cells; the adapter has one socket on each face but its
    // mount face.
    assert_eq!(MOUNT_RAIL_SHORT.kind, Kind::Rail);
    assert_eq!(MOUNT_RAIL_LONG.kind, Kind::Rail);
    assert_eq!(ADAPTER.kind, Kind::Adapter);
    assert_eq!(MOUNT_RAIL_SHORT.socket_count(), 2);
    assert_eq!(MOUNT_RAIL_LONG.socket_count(), 4);
    assert_eq!(ADAPTER.socket_count(), 5);
    assert_eq!(MOUNT_RAIL_LONG.socket(Face::PosZ, 0, 3), Some(SocketId(3)));
    assert_eq!(MOUNT_RAIL_LONG.socket(Face::PosX, 0, 0), None);
    assert_eq!(ADAPTER.socket(Face::NegZ, 0, 0), None);
    // Mass, health and build points like any part.
    for kind in [MOUNT_RAIL_SHORT, MOUNT_RAIL_LONG, ADAPTER] {
        assert!(
            kind.mass > 0 && kind.health > 0 && kind.points > 0,
            "{}",
            kind.key
        );
        assert_eq!(BETA.part_type(kind.id), Some(&kind));
    }
    // Every fixture is valid as authored, canonical, and costs its parts.
    for (name, build) in FIXTURES {
        assert_eq!(BETA.validate(&build.candidate()), Ok(build), "{name}");
        let budget = BETA.budget(&build);
        assert!(budget.points <= budget.build_points, "{name}");
    }
    assert_eq!(BETA.budget(&RAIL_RACK).points, 107);
    assert_eq!(BETA.budget(&CROSS_RAIL).points, 96);
    assert_eq!(BETA.budget(&ADAPTER_MAST).points, 61);
    // Different weapon counts: six on two rails, four (two of them on a turned rail), three on
    // an adapter.
    let count = |build: &Assembly| BETA.weapons(build).count();
    assert_eq!(
        [count(&RAIL_RACK), count(&CROSS_RAIL), count(&ADAPTER_MAST)],
        [6, 4, 3]
    );
    let on_rails = |build: &Assembly| {
        BETA.weapons(build)
            .filter(|weapon| {
                let parent = build.part(weapon.part).unwrap().mount.unwrap().parent;
                BETA.part_type(build.part(parent).unwrap().kind)
                    .unwrap()
                    .kind
                    == Kind::Rail
            })
            .count()
    };
    assert_eq!(
        [
            on_rails(&RAIL_RACK),
            on_rails(&CROSS_RAIL),
            on_rails(&ADAPTER_MAST)
        ],
        [6, 2, 2]
    );
}

#[test]
fn the_validator_refuses_overlaps_cycles_disconnected_children_and_overspending_on_rails() {
    // Two weapons on one hardpoint.
    let crowded = plus(
        &RAIL_RACK,
        [on(11, &LASER, 8, &MOUNT_RAIL_SHORT, (Face::PosZ, 0, 1), 0)],
    );
    assert_eq!(
        BETA.validate(&crowded),
        Err(AssemblyError::Overlap(PartId(10), PartId(11)))
    );
    // A heavy weapon's two-cell footprint across a rail's taken hardpoint.
    let mut heavy = RAIL_RACK.candidate();
    heavy.parts.retain(|part| part.id != 5);
    heavy.parts.push(on(
        11,
        &RAIL_GUN,
        3,
        &MOUNT_RAIL_LONG,
        (Face::PosZ, 0, 1),
        0,
    ));
    assert_eq!(
        BETA.validate(&heavy),
        Err(AssemblyError::Overlap(PartId(6), PartId(11)))
    );
    // The turned rail lies across the cell to its right on the chassis' front: nothing else goes
    // there, and turned the other way it lies across a taken one.
    let beside = plus(
        &CROSS_RAIL,
        [on(9, &ENERGY_RIFLE, 0, &CHASSIS_M, (Face::PosX, 0, 3), 0)],
    );
    assert_eq!(
        BETA.validate(&beside),
        Err(AssemblyError::Overlap(PartId(3), PartId(9)))
    );
    let mut lower = CROSS_RAIL.candidate();
    let rail = lower.parts.iter_mut().find(|part| part.id == 3).unwrap();
    let mount = rail.mount.as_mut().unwrap();
    (mount.socket, mount.yaw) = (CHASSIS_M.socket(Face::PosX, 0, 0).unwrap().0, 3 * QUARTER);
    assert_eq!(
        BETA.validate(&lower),
        Err(AssemblyError::Overlap(PartId(3), PartId(6)))
    );
    // A hardpoint the rail does not have, and the rail's underside, which rests on its parent.
    let off_the_end = plus(
        &CROSS_RAIL,
        [RawPart {
            id: 9,
            kind: SHOTGUN.id.0,
            mount: Some(RawMount {
                parent: 3,
                socket: 4,
                ..RawMount::default()
            }),
            modification: None,
        }],
    );
    assert_eq!(
        BETA.validate(&off_the_end),
        Err(AssemblyError::NoSocket(PartId(9)))
    );
    // A weapon is no support: it has no sockets.
    let on_a_gun = plus(
        &RAIL_RACK,
        [RawPart {
            id: 11,
            kind: ADAPTER.id.0,
            mount: Some(RawMount {
                parent: 4,
                ..RawMount::default()
            }),
            modification: None,
        }],
    );
    assert_eq!(
        BETA.validate(&on_a_gun),
        Err(AssemblyError::NoSocket(PartId(11)))
    );
    // Two rails on each other: neither reaches the root.
    let looped = plus(
        &ADAPTER_MAST,
        [
            on(
                8,
                &MOUNT_RAIL_SHORT,
                9,
                &MOUNT_RAIL_SHORT,
                (Face::PosZ, 0, 0),
                0,
            ),
            on(
                9,
                &MOUNT_RAIL_SHORT,
                8,
                &MOUNT_RAIL_SHORT,
                (Face::PosZ, 0, 0),
                0,
            ),
        ],
    );
    assert_eq!(BETA.validate(&looped), Err(AssemblyError::Cycle(PartId(8))));
    // A weapon on a rail that is not in the build, and a rail's children without the rail.
    let orphan = plus(
        &ADAPTER_MAST,
        [on(8, &LASER, 20, &MOUNT_RAIL_SHORT, (Face::PosZ, 0, 0), 0)],
    );
    assert_eq!(
        BETA.validate(&orphan),
        Err(AssemblyError::UnknownParent(PartId(8)))
    );
    let mut unsupported = ADAPTER_MAST.candidate();
    unsupported.parts.retain(|part| part.id != 4);
    assert_eq!(
        BETA.validate(&unsupported),
        Err(AssemblyError::UnknownParent(PartId(5)))
    );
    // The rails cost build points: the rack with one more gun on an adapter is over the budget.
    let rich = plus(
        &RAIL_RACK,
        [
            on(11, &ADAPTER, 0, &CHASSIS_M, (Face::PosZ, 0, 0), 0),
            on(12, &ENERGY_RIFLE, 11, &ADAPTER, (Face::PosZ, 0, 0), 0),
        ],
    );
    assert_eq!(
        BETA.validate(&rich),
        Err(AssemblyError::OverBudget {
            points: 118,
            build_points: 110
        })
    );
}

#[test]
fn what_a_rail_or_adapter_carries_is_placed_where_its_support_holds_it() {
    let placed = |build: &Assembly, id: u8| placements(build)[index_of(build, PartId(id))];
    let forward = |build: &Assembly, id: u8| rotate(placed(build, id).orientation, [0.0, 0.0, 1.0]);
    // The rack: the long rail stands on the front's right column, its four weapons in front of
    // it stacked 20 cm apart, all facing the aim (+X).
    let rail = placed(&RAIL_RACK, 3);
    for (step, id) in [4, 5, 6, 7].into_iter().enumerate() {
        let weapon = placed(&RAIL_RACK, id);
        assert!(close(forward(&RAIL_RACK, id), [1.0, 0.0, 0.0]), "{id}");
        assert!(weapon.centre[0] > rail.centre[0] + 20.0, "{id} in front");
        assert!(
            (weapon.centre[2] - rail.centre[2]).abs() < 0.5,
            "{id} on the rail's column"
        );
        let below = placed(&RAIL_RACK, 4).centre[1];
        #[allow(clippy::cast_precision_loss)]
        let height = 20.0 * step as f32;
        assert!(
            (weapon.centre[1] - below - height).abs() < 0.5,
            "{id} stacked"
        );
    }
    // The short rail's two on the left column (−Z), level with the long rail's lowest two.
    for (id, beside) in [(9, 4), (10, 5)] {
        let (weapon, other) = (placed(&RAIL_RACK, id), placed(&RAIL_RACK, beside));
        assert!((weapon.centre[1] - other.centre[1]).abs() < 0.5);
        assert!((other.centre[2] - weapon.centre[2] - 20.0).abs() < 0.5);
    }
    // The turned rail lies across the front's top row: its two rail guns side by side, the
    // second out past the chassis' right side (+Z beyond 20 cm), both facing the aim.
    let (first, second) = (placed(&CROSS_RAIL, 4), placed(&CROSS_RAIL, 5));
    assert!((first.centre[1] - second.centre[1]).abs() < 0.5);
    assert!((second.centre[2] - first.centre[2] - 40.0).abs() < 0.5);
    assert!(second.centre[2] - second.half_extents[1] >= 20.0 - 0.5);
    for id in [4, 5] {
        assert!(close(forward(&CROSS_RAIL, id), [1.0, 0.0, 0.0]), "{id}");
    }
    // On the adapter: the rail across its front, the rifle and the shotgun above the roof and
    // facing the aim, the grapple straight up.
    let roof = placed(&ADAPTER_MAST, 0).half_extents[2];
    for id in [5, 6] {
        assert!(close(forward(&ADAPTER_MAST, id), [1.0, 0.0, 0.0]), "{id}");
        assert!(
            placed(&ADAPTER_MAST, id).centre[1] > roof,
            "{id} above the roof"
        );
    }
    assert!(close(forward(&ADAPTER_MAST, 7), [0.0, 1.0, 0.0]));
}

#[test]
fn rails_carry_their_children_in_the_supports_body_and_take_them_when_they_go() {
    for (name, build) in FIXTURES {
        let layout = Layout::of(&build);
        let placed = placements(&build);
        // No joint: every rigid part, rails and what they carry, is one body with the root,
        // each a box where it is mounted.
        for segment in &layout.segments {
            let part = build.part(segment.part).unwrap();
            if matches!(BETA.part_type(part.kind).unwrap().kind, Kind::Locomotion(_)) {
                continue;
            }
            assert_eq!(
                layout.bodies[segment.body].key,
                BodyKey::Rigid(None),
                "{name}"
            );
            let child = layout.bodies[segment.body].children[segment.child];
            let at = placed[index_of(&build, segment.part)].centre;
            assert!(close(child.position, at), "{name}: {:?}", segment.part);
        }
    }
    // The rack's long rail off: it and its four weapons leave the body; the short rail stays.
    let lost = bits(&RAIL_RACK, &[3, 4, 5, 6, 7]);
    let layout = Layout::connected(&RAIL_RACK, lost);
    let left: Vec<u8> = layout.segments.iter().map(|s| s.part.0).collect();
    assert!(
        left.iter().all(|id| ![3, 4, 5, 6, 7].contains(id)),
        "{left:?}"
    );
    assert!([0, 8, 9, 10].iter().all(|id| left.contains(id)), "{left:?}");
}

#[test]
fn losing_a_support_loses_everything_it_carries() {
    // The rack's long rail destroyed: its four weapons come off whole; the other rail's stay.
    let mut damage = PartDamage::fresh(&RAIL_RACK);
    let change = damage
        .hurt(&RAIL_RACK, index_of(&RAIL_RACK, PartId(3)), 1_000)
        .unwrap();
    assert!(change.destroyed);
    assert_eq!(change.dealt, MOUNT_RAIL_LONG.health);
    assert_eq!(change.lost, bits(&RAIL_RACK, &[3, 4, 5, 6, 7]));
    for weapon in [
        Weapon::EnergyRifle,
        Weapon::Shotgun,
        Weapon::EnergyBlade,
        Weapon::Grapple,
    ] {
        assert!(damage.weapon_lost(&RAIL_RACK, weapon), "{weapon:?}");
    }
    assert!(damage.armed(&RAIL_RACK, Weapon::Laser));
    assert!(damage.armed(&RAIL_RACK, Weapon::Flamethrower));
    assert_eq!(damage.health()[index_of(&RAIL_RACK, PartId(4))], 40);
    // The turned rail: both rail guns go, the grapple and the blade on the chassis stay.
    let mut damage = PartDamage::fresh(&CROSS_RAIL);
    let change = damage
        .hurt(&CROSS_RAIL, index_of(&CROSS_RAIL, PartId(3)), 1_000)
        .unwrap();
    assert_eq!(change.lost, bits(&CROSS_RAIL, &[3, 4, 5]));
    assert!(damage.weapon_lost(&CROSS_RAIL, Weapon::RailGun));
    assert!(damage.armed(&CROSS_RAIL, Weapon::Grapple));
    // Two levels: the rail on the adapter takes its guns; the adapter takes the rail, its guns
    // and the grapple.
    let mut damage = PartDamage::fresh(&ADAPTER_MAST);
    let change = damage
        .hurt(&ADAPTER_MAST, index_of(&ADAPTER_MAST, PartId(4)), 1_000)
        .unwrap();
    assert_eq!(change.lost, bits(&ADAPTER_MAST, &[4, 5, 6]));
    assert!(damage.armed(&ADAPTER_MAST, Weapon::Grapple));
    let change = damage
        .hurt(&ADAPTER_MAST, index_of(&ADAPTER_MAST, PartId(3)), 1_000)
        .unwrap();
    assert_eq!(change.lost, bits(&ADAPTER_MAST, &[3, 7]));
    assert!(damage.weapon_lost(&ADAPTER_MAST, Weapon::Grapple));
    // A support only damaged keeps everything.
    let mut damage = PartDamage::fresh(&ADAPTER_MAST);
    let change = damage
        .hurt(&ADAPTER_MAST, index_of(&ADAPTER_MAST, PartId(3)), 10)
        .unwrap();
    assert_eq!((change.destroyed, change.lost), (false, 0));
}

#[test]
fn rail_builds_travel_the_wire_unchanged() {
    for (name, build) in FIXTURES {
        let bytes = encode_input(Intent::IDLE, 7, None, Some(&build));
        assert_eq!(decode_input(&bytes).unwrap().build, Some(build), "{name}");
    }
    let mut session = Session::with_rules(Arena::greybox(), Rules::deathmatch(2)).unwrap();
    session.join(1).unwrap();
    assert!(session.configure(1, RAIL_RACK));
    let snapshot = session.snapshot();
    assert_eq!(
        decode_snapshot(&encode_snapshot(&snapshot)).unwrap(),
        snapshot
    );
}

const A: u32 = 1;
const B: u32 = 2;

/// A (slot 0, the cross rail) faces B (slot 1, the rail rack) 12 m apart on an open floor.
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
    for (slot, build) in [(0, CROSS_RAIL), (1, RAIL_RACK)] {
        session.fighters[slot].configured = build;
        session.rebuild(u8::try_from(slot).unwrap(), build).unwrap();
        let fighter = &mut session.fighters[slot];
        fighter.energy = fighter.pool().capacity;
    }
    session
}

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

fn part_center(session: &Session, slot: usize, part: PartId) -> [f32; 3] {
    let fighter = &session.fighters[slot];
    fighter
        .hitboxes()
        .get(index_of(&fighter.build, part))
        .expect("a connected part")
        .center
}

#[test]
fn a_rail_shot_off_in_a_match_takes_its_weapons_and_the_rest_still_fire() {
    let mut session = duel();
    // Each of B's rail-mounted weapons fires from its own muzzle, where the rail holds it.
    for (part, weapon) in [(4, Weapon::EnergyRifle), (9, Weapon::Laser)] {
        let b = &session.fighters[1];
        let instance = BETA.instance(&b.build, PartId(part)).unwrap();
        assert_eq!(instance.weapon, weapon);
        let muzzle = aim::muzzle(&b.build, &b.hitboxes(), instance).expect("a muzzle");
        let centre = part_center(&session, 1, PartId(part));
        assert!(close(muzzle.mount, centre), "{weapon:?}");
        let reach: f32 = muzzle
            .at
            .iter()
            .zip(centre)
            .map(|(a, b)| (a - b) * (a - b))
            .sum::<f32>()
            .sqrt();
        assert!(reach < 40.0, "{weapon:?}: {reach}");
    }
    // B turns its right side (where the long rail stands) to A; A's rail gun (85) breaks the
    // long rail (55): the rail and the four weapons on it come off.
    let side = Intent {
        yaw: 16_384,
        ..Intent::IDLE
    };
    let mut events = Vec::new();
    for tick in 0..240 {
        let (yaw, pitch) = aim::at(
            Some(&session.world),
            session.fighters[0].eye(),
            part_center(&session, 1, PartId(3)),
        );
        // Both settle first: B turned, A's rail gun up and a shot paid for (the fixture has no
        // generation, shooter#128: its battery only drains).
        let ready = tick >= 90 && session.fighters[0].energy >= Weapon::RailGun.energy();
        let fire = Intent {
            yaw,
            pitch,
            fire: ready,
            select: Some(Weapon::RailGun.into()),
            ..Intent::IDLE
        };
        step(&mut session, fire, side);
        if session
            .events
            .iter()
            .any(|event| matches!(event, Event::Fired { slot: 0, .. }))
        {
            events = session.events.clone();
            break;
        }
    }
    let detached: Vec<(u8, bool)> = events
        .iter()
        .filter_map(|event| match *event {
            Event::Detached {
                slot: 1,
                part,
                destroyed,
            } => Some((part.0, destroyed)),
            _ => None,
        })
        .collect();
    assert_eq!(
        detached,
        [(3, true), (4, false), (5, false), (6, false), (7, false)]
    );
    let b = &session.fighters[1];
    assert!(b.alive());
    assert!(b.hitboxes().get(index_of(&b.build, PartId(4))).is_none());
    // B fires only what its short rail still carries.
    let mut fired = Vec::new();
    for weapon in [Weapon::EnergyRifle, Weapon::Shotgun, Weapon::Laser] {
        for _ in 0..60 {
            let intent = Intent {
                yaw: 32_768,
                fire: true,
                select: Some(weapon.into()),
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
    assert_eq!(fired, [Weapon::Laser]);
}
