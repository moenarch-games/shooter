//! The map catalog: stable IDs and wire tags, unknown maps rejected, a match keeping its map
//! and spawning on it through respawns and round restarts, and the slot count left to the
//! rules.
// The platform's floats only check results here; the simulation uses libm (clippy.toml).
#![allow(clippy::disallowed_methods)]

use shooter_sim::physics::Vec3i;
use shooter_sim::player::{EYE_HEIGHT_UNITS, GRAVITY};
use shooter_sim::session::DEATHMATCH_SLOTS;
use shooter_sim::wire::{WireError, decode_snapshot, encode_snapshot};
use shooter_sim::{Arena, Event, MapId, Phase, Piece, Rules, Session, Spawn, UnknownMap};

#[test]
fn ids_and_tags_are_stable_and_unique() {
    assert_eq!(MapId::DEFAULT, MapId::GreyboxYard);
    assert_eq!(MapId::Greybox.as_str(), "greybox");
    assert_eq!(MapId::Greybox.tag(), 1);
    for (index, map) in MapId::ALL.iter().enumerate() {
        assert_eq!(map.as_str().parse::<MapId>(), Ok(*map));
        assert_eq!(MapId::from_tag(map.tag()), Some(*map));
        assert_ne!(map.tag(), 0, "0 is no catalogued map");
        for other in &MapId::ALL[index + 1..] {
            assert_ne!(map.as_str(), other.as_str());
            assert_ne!(map.tag(), other.tag());
        }
    }
}

#[test]
fn unknown_ids_are_rejected() {
    for id in ["", "nowhere", "Greybox", " greybox", "greybox2"] {
        assert_eq!(id.parse::<MapId>(), Err(UnknownMap(id.to_owned())));
    }
    assert!(
        UnknownMap("nowhere".to_owned())
            .to_string()
            .contains("greybox")
    );
    assert_eq!(MapId::from_tag(0), None);
    assert_eq!(MapId::from_tag(u8::MAX), None);
}

#[test]
fn every_map_fits_its_capacity_without_changing_the_slots() {
    for map in MapId::ALL {
        let arena = map.arena();
        let capacity = map.capacity();
        assert!(2 <= capacity.min && capacity.min <= capacity.max, "{map}");
        assert!(
            usize::from(capacity.max) <= arena.spawn_points().len(),
            "{map} recommends more players than it has spawn points"
        );
        let session = Session::on_map(map, Rules::deathmatch(1)).unwrap();
        assert_eq!(session.rules.slots, DEATHMATCH_SLOTS);
        assert_eq!(session.fighters.len(), usize::from(DEATHMATCH_SLOTS));
    }
    assert_eq!(DEATHMATCH_SLOTS, 12);
    assert!(MapId::Greybox.capacity().fits(DEATHMATCH_SLOTS));
    assert!(!MapId::Greybox.capacity().fits(1));
}

#[test]
fn every_slot_starts_on_a_spawn_point_of_its_own() {
    for map in MapId::ALL {
        assert_eq!(
            usize::from(map.max_slots()),
            map.arena().spawn_points().len()
        );
        assert!(map.max_slots() >= DEATHMATCH_SLOTS, "{map}");
        assert!(map.capacity().max <= map.max_slots(), "{map}");
        let rules = Rules {
            slots: map.max_slots(),
            ..Rules::deathmatch(3)
        };
        let session = Session::on_map(map, rules).unwrap();
        let feet: Vec<[f32; 3]> = session
            .fighters
            .iter()
            .map(|fighter| fighter.feet())
            .collect();
        for (index, a) in feet.iter().enumerate() {
            for b in &feet[index + 1..] {
                let apart = (a[0] - b[0]).hypot(a[2] - b[2]);
                assert!(apart > 200.0, "{map}: fighters start {apart} units apart");
            }
        }
    }
}

#[test]
fn the_greybox_map_is_the_greybox_fixture() {
    assert_eq!(MapId::Greybox.arena(), Arena::greybox());
    let mut on_map = Session::on_map(MapId::Greybox, Rules::deathmatch(5)).unwrap();
    let mut fixture = Session::with_rules(Arena::greybox(), Rules::deathmatch(5)).unwrap();
    assert_eq!(on_map.map, Some(MapId::Greybox));
    assert_eq!(fixture.map, None);
    for _ in 0..600 {
        on_map.step().unwrap();
        fixture.step().unwrap();
    }
    let (mut a, b) = (on_map.snapshot(), fixture.snapshot());
    assert_eq!(a.map, Some(MapId::Greybox));
    a.map = None;
    assert_eq!(a, b, "the map label changes nothing else");
}

#[test]
fn a_match_keeps_its_map_and_spawns_on_it_through_rounds() {
    let map = MapId::Greybox;
    let points = map.arena().spawn_points();
    let rules = Rules {
        time_limit_ticks: 20 * 60,
        intermission_ticks: 120,
        ..Rules::deathmatch(9)
    };
    let mut session = Session::on_map(map, rules).unwrap();
    let (mut spawns, mut rounds, mut intermission) = (0, 0, false);
    for _ in 0..3 * (20 * 60 + 120) {
        session.step().unwrap();
        let snapshot = decode_snapshot(&encode_snapshot(&session.snapshot())).unwrap();
        assert_eq!(snapshot.map, Some(map));
        match session.phase {
            Phase::Intermission { .. } => intermission = true,
            Phase::Playing { .. } if intermission => {
                intermission = false;
                rounds += 1;
            }
            Phase::Playing { .. } => {}
        }
        for event in &session.events {
            let Event::Spawned { slot, yaw } = *event else {
                continue;
            };
            let fighter = &session.fighters[usize::from(slot)];
            let feet = fighter.feet();
            assert!(
                points.iter().any(|point| {
                    point.yaw == yaw
                        && (feet[0] - point.feet.x as f32).abs() < 50.0
                        && (feet[2] - point.feet.z as f32).abs() < 50.0
                }),
                "slot {slot} spawned at {feet:?} facing {yaw}, off the map's spawn points"
            );
            spawns += 1;
        }
    }
    assert!(rounds >= 2, "{rounds} round restarts");
    assert!(
        spawns > usize::from(DEATHMATCH_SLOTS) * 2,
        "{spawns} spawns"
    );
}

#[test]
fn a_snapshot_with_an_unknown_map_is_rejected() {
    let session = Session::on_map(MapId::Greybox, Rules::deathmatch(2)).unwrap();
    let mut bytes = encode_snapshot(&session.snapshot());
    // The version byte, the tick, then the map's tag.
    assert_eq!(bytes[5], MapId::Greybox.tag());
    bytes[5] = 0;
    assert_eq!(decode_snapshot(&bytes).unwrap().map, None);
    bytes[5] = 200;
    assert_eq!(
        decode_snapshot(&bytes),
        Err(WireError::Malformed("unknown map"))
    );
}

/// Pairs of spawn points (indices) from whose eyes each other's eyes are in plain sight.
fn spawns_in_sight(arena: &Arena) -> Vec<(usize, usize)> {
    let world = arena.build_world(GRAVITY).unwrap();
    let eye = |spawn: &Spawn| {
        #[allow(clippy::cast_precision_loss)]
        [spawn.feet.x, spawn.feet.y + EYE_HEIGHT_UNITS, spawn.feet.z].map(|axis| axis as f32)
    };
    let points = arena.spawn_points();
    let mut pairs = Vec::new();
    for (a, from) in points.iter().enumerate() {
        for (b, to) in points.iter().enumerate().skip(a + 1) {
            let (from, to) = (eye(from), eye(to));
            let along = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
            if world.cast_ray(from, along, |_| true).is_none() {
                pairs.push((a, b));
            }
        }
    }
    pairs
}

#[test]
fn the_yard_is_the_bigger_default_and_the_greybox_stays_the_fixture() {
    assert_eq!(MapId::DEFAULT, MapId::GreyboxYard);
    assert_eq!(MapId::GreyboxYard.as_str(), "greybox-yard");
    assert_eq!(MapId::GreyboxYard.tag(), 2);
    assert_eq!(MapId::GreyboxYard.arena(), Arena::greybox_yard());
    let (yard, greybox) = (Arena::greybox_yard(), Arena::greybox());
    assert!((3_500..=4_000).contains(&yard.half_size), "70-80 m across");
    let ramps = |arena: &Arena| {
        arena
            .pieces
            .iter()
            .filter(|piece| matches!(piece, Piece::Ramp { .. }))
            .count()
    };
    assert!(ramps(&yard) > ramps(&greybox));
    assert!(yard.pieces.len() > 2 * greybox.pieces.len());
    assert!(yard.spawn_points().len() >= 12);
    assert!(MapId::GreyboxYard.capacity().fits(DEATHMATCH_SLOTS));
}

/// No two of the yard's spawn points see each other, and every place a bot walks to (its spawn
/// points and waypoints) links to the rest by straight walks it can take both ways, every spawn
/// point to at least two of them: no bot is left without somewhere to go.
#[test]
fn yard_spawns_are_out_of_sight_and_linked_by_walks() {
    let arena = Arena::greybox_yard();
    assert_eq!(spawns_in_sight(&arena), Vec::new());
    assert_walks_link_up(&arena);
}

/// Every bot point of `arena` links to the rest by walks both ways, every spawn point to two.
fn assert_walks_link_up(arena: &Arena) {
    let points = arena.bot_points();
    let walks = |from: usize| -> Vec<usize> {
        (0..points.len())
            .filter(|to| {
                *to != from
                    && arena.clear_walk(points[from], points[*to])
                    && arena.clear_walk(points[*to], points[from])
            })
            .collect()
    };
    for spawn in 0..arena.spawn_points().len() {
        assert!(walks(spawn).len() >= 2, "spawn {spawn}: {:?}", walks(spawn));
    }
    let mut linked = vec![false; points.len()];
    let mut queue = vec![0];
    linked[0] = true;
    while let Some(from) = queue.pop() {
        for to in walks(from) {
            if !linked[to] {
                linked[to] = true;
                queue.push(to);
            }
        }
    }
    let cut_off: Vec<Vec3i> = (0..points.len())
        .filter(|point| !linked[*point])
        .map(|point| points[point])
        .collect();
    assert!(cut_off.is_empty(), "no walk reaches {cut_off:?}");
}

/// The assembly works (shooter#52) is a distinct, kit-built medium map: a stable ID and tag,
/// between the greybox hall and the yard in size, every piece derived from its kit placements,
/// inside the body IDs arena geometry may take, and the yard stays the default.
#[test]
fn the_assembly_works_is_the_kit_built_medium_map() {
    use shooter_sim::arena::{FIRST_PLAYER_BODY, FLOOR_BODY};
    use shooter_sim::arena_kit;
    assert_eq!(MapId::AssemblyWorks.as_str(), "assembly-works");
    assert_eq!(MapId::AssemblyWorks.tag(), 3);
    assert_eq!(MapId::DEFAULT, MapId::GreyboxYard);
    let works = Arena::assembly_works();
    assert_eq!(MapId::AssemblyWorks.arena(), works);
    let (greybox, yard) = (Arena::greybox(), Arena::greybox_yard());
    assert!(greybox.half_size < works.half_size && works.half_size < yard.half_size);
    assert!(MapId::AssemblyWorks.capacity().fits(DEATHMATCH_SLOTS));
    assert!(works.spawn_points().len() >= 14);
    assert_eq!(works.pieces, arena_kit::pieces(&works.placements));
    assert!(FLOOR_BODY.0 + works.pieces.len() as u64 + 1 < FIRST_PLAYER_BODY);
    // Every module the clients draw stays inside the floor and the walls round it.
    let reach = works.half_size + 60;
    for placement in &works.placements {
        let [min, max] = placement.bounds();
        assert!(
            min.x >= -reach && max.x <= reach && min.z >= -reach && max.z <= reach,
            "{placement:?}"
        );
        assert!(min.y >= -10, "{placement:?}");
    }
    // A jump pad module lies at each jump pad.
    for pad in &works.pads {
        assert!(works.placements.iter().any(|placement| {
            placement.module == arena_kit::Module::JumpPad && placement.at == pad.center
        }));
    }
    // Destruction candidates (shooter#60) are cover with a broken variant, and some of each.
    let destructible: Vec<_> = works.placements.iter().filter(|p| p.destructible).collect();
    assert!(destructible.len() >= 10);
    assert!(
        destructible
            .iter()
            .all(|p| p.module.broken_proxies().is_some())
    );
    assert!(!works.repair_anchors.is_empty());
    assert!(greybox.placements.is_empty() && yard.placements.is_empty());
}

/// No two of the assembly works' spawn points see each other, and its bot points link up by
/// walks, every spawn point to at least two: as on the yard.
#[test]
fn works_spawns_are_out_of_sight_and_linked_by_walks() {
    let arena = Arena::assembly_works();
    assert_eq!(spawns_in_sight(&arena), Vec::new());
    assert_walks_link_up(&arena);
}

/// The assembly works' high ground: the bridge's middle (the rail capacitor) and the mezzanines
/// are bot points a walk reaches up the ramps, the two crane towers carry the thrusters and
/// each has a jump pad at its foot; every fighter on the floor is under 35 m (well inside the
/// grapple's 30 m reach of some high point) from the bridge.
#[test]
fn works_high_ground_has_routes_up() {
    use shooter_sim::assembly_works::{DECK_TOP, TOWER_TOP};
    use shooter_sim::pickups::Class;
    let arena = Arena::assembly_works();
    let high: Vec<Vec3i> = arena
        .bot_points()
        .into_iter()
        .filter(|point| point.y == DECK_TOP)
        .collect();
    assert!(high.len() >= 4, "{high:?}");
    let capacitor = arena
        .module_pads
        .iter()
        .find(|pad| pad.module.class() == Class::Weapon)
        .unwrap();
    assert_eq!(capacitor.center, Vec3i::new(0, DECK_TOP, 0));
    let towers: Vec<_> = arena
        .module_pads
        .iter()
        .filter(|pad| pad.module.class() == Class::Movement)
        .collect();
    assert_eq!(towers.len(), 2);
    for tower in towers {
        assert_eq!(tower.center.y, TOWER_TOP);
        assert!(
            arena.pads.iter().any(|pad| {
                (pad.center.x - tower.center.x).abs() + (pad.center.z - tower.center.z).abs()
                    < 1_000
                    && pad.launch.y >= 1_500
            }),
            "{tower:?}"
        );
    }
}

/// The proving cell (shooter#53) is a distinct, kit-built small map: a stable ID and tag, smaller
/// than the greybox hall, laid out for a duel to a 6-player skirmish yet with a spawn point for
/// each default slot, every piece derived from its kit placements, inside the body IDs arena
/// geometry may take, and the yard stays the default.
#[test]
fn the_proving_cell_is_the_kit_built_small_map() {
    use shooter_sim::arena::{FIRST_PLAYER_BODY, FLOOR_BODY};
    use shooter_sim::arena_kit;
    assert_eq!(MapId::ProvingCell.as_str(), "proving-cell");
    assert_eq!(MapId::ProvingCell.tag(), 4);
    assert_eq!(MapId::DEFAULT, MapId::GreyboxYard);
    let cell = Arena::proving_cell();
    assert_eq!(MapId::ProvingCell.arena(), cell);
    let (greybox, works) = (Arena::greybox(), Arena::assembly_works());
    assert!(cell.half_size < greybox.half_size && cell.half_size < works.half_size);
    assert!(MapId::ProvingCell.capacity().fits(2));
    assert!(MapId::ProvingCell.capacity().max <= 6);
    assert_eq!(
        cell.spawn_points().len(),
        usize::from(DEATHMATCH_SLOTS),
        "a spawn point per default slot"
    );
    assert_eq!(cell.pieces, arena_kit::pieces(&cell.placements));
    assert!(FLOOR_BODY.0 + cell.pieces.len() as u64 + 1 < FIRST_PLAYER_BODY);
    let reach = cell.half_size + 60;
    for placement in &cell.placements {
        let [min, max] = placement.bounds();
        assert!(
            min.x >= -reach && max.x <= reach && min.z >= -reach && max.z <= reach,
            "{placement:?}"
        );
        assert!(min.y >= -10, "{placement:?}");
    }
    for pad in &cell.pads {
        assert!(cell.placements.iter().any(|placement| {
            placement.module == arena_kit::Module::JumpPad && placement.at == pad.center
        }));
    }
    let destructible: Vec<_> = cell.placements.iter().filter(|p| p.destructible).collect();
    assert!(destructible.len() >= 8);
    assert!(
        destructible
            .iter()
            .all(|p| p.module.broken_proxies().is_some())
    );
    assert!(!cell.repair_anchors.is_empty());
    // Its landmark (shooter#151): the test rig over the stage, a pillar at each corner carrying
    // a canopy over the whole stage.
    let over_stage = |module| {
        cell.placements
            .iter()
            .filter(|p| p.module == module && p.at.x.abs() <= 300 && p.at.z.abs() <= 300)
            .count()
    };
    assert_eq!(over_stage(arena_kit::Module::Pillar), 4);
    assert_eq!(over_stage(arena_kit::Module::Ceiling), 9);
    // Its own layout, not a copy of the works.
    assert_ne!(cell.placements, works.placements);
}

/// No two of the proving cell's spawn points see each other, and its bot points link up by
/// walks, every spawn point to at least two.
#[test]
fn cell_spawns_are_out_of_sight_and_linked_by_walks() {
    let arena = Arena::proving_cell();
    assert_eq!(spawns_in_sight(&arena), Vec::new());
    assert_walks_link_up(&arena);
}

/// The proving cell's high ground: the stage carries the rail capacitor and the galleries the
/// thrusters, both are bot points a walk reaches up their ramps, a jump pad throws onto each
/// gallery, and no bot point on the floor is more than 20 m from the stage: a close-quarters
/// map.
#[test]
fn cell_high_ground_has_routes_up() {
    use shooter_sim::pickups::Class;
    use shooter_sim::proving_cell::{DECK_TOP, STAGE_TOP};
    let arena = Arena::proving_cell();
    let points = arena.bot_points();
    for top in [STAGE_TOP, DECK_TOP] {
        assert!(
            points.iter().filter(|point| point.y == top).count() >= 2,
            "{top}: {points:?}"
        );
    }
    let capacitor = arena
        .module_pads
        .iter()
        .find(|pad| pad.module.class() == Class::Weapon)
        .unwrap();
    assert_eq!(capacitor.center, Vec3i::new(0, STAGE_TOP, 0));
    let thrusters: Vec<_> = arena
        .module_pads
        .iter()
        .filter(|pad| pad.module.class() == Class::Movement)
        .collect();
    assert_eq!(thrusters.len(), 2);
    for gallery in thrusters {
        assert_eq!(gallery.center.y, DECK_TOP);
        assert!(
            arena.pads.iter().any(|pad| {
                (pad.center.z - gallery.center.z).abs() < 800 && pad.launch.y >= 1_200
            }),
            "{gallery:?}"
        );
    }
    for point in points.iter().filter(|point| point.y == 0) {
        assert!(point.x.abs().max(point.z.abs()) < 2_000, "{point:?}");
    }
}

/// The drydock (shooter#55) is a distinct, kit-built large map: a stable ID and tag, bigger than
/// the yard, laid out for 8 to the default 12 players with spawn points to spare, every piece
/// derived from its kit placements (its hulls from the kit's hull modules), inside the body IDs
/// arena geometry may take, and the yard stays the default.
#[test]
fn the_drydock_is_the_kit_built_large_map() {
    use shooter_sim::arena::{FIRST_PLAYER_BODY, FLOOR_BODY};
    use shooter_sim::arena_kit::{self, Module};
    assert_eq!(MapId::Drydock.as_str(), "drydock");
    assert_eq!(MapId::Drydock.tag(), 5);
    assert_eq!(MapId::DEFAULT, MapId::GreyboxYard);
    let dock = Arena::drydock();
    assert_eq!(MapId::Drydock.arena(), dock);
    for other in [
        Arena::greybox(),
        Arena::greybox_yard(),
        Arena::assembly_works(),
        Arena::proving_cell(),
    ] {
        assert!(dock.half_size > other.half_size);
    }
    assert!(MapId::Drydock.capacity().fits(DEATHMATCH_SLOTS));
    assert!(MapId::Drydock.capacity().min >= 8);
    assert!(dock.spawn_points().len() > usize::from(DEATHMATCH_SLOTS));
    assert_eq!(dock.pieces, arena_kit::pieces(&dock.placements));
    assert!(FLOOR_BODY.0 + dock.pieces.len() as u64 + 1 < FIRST_PLAYER_BODY);
    let reach = dock.half_size + 60;
    for placement in &dock.placements {
        let [min, max] = placement.bounds();
        assert!(
            min.x >= -reach && max.x <= reach && min.z >= -reach && max.z <= reach,
            "{placement:?}"
        );
        assert!(min.y >= -10, "{placement:?}");
    }
    for pad in &dock.pads {
        assert!(dock.placements.iter().any(|placement| {
            placement.module == Module::JumpPad && placement.at == pad.center
        }));
    }
    // The ships are the kit's hull modules: decks, shoulders, sides, ribs and masts.
    for module in [
        Module::HullDeck,
        Module::HullShoulder,
        Module::HullSide,
        Module::HullRib,
        Module::HullMast,
    ] {
        assert!(
            dock.placements.iter().any(|p| p.module == module),
            "{module:?}"
        );
    }
    let destructible: Vec<_> = dock.placements.iter().filter(|p| p.destructible).collect();
    assert!(destructible.len() >= 10);
    assert!(
        destructible
            .iter()
            .all(|p| p.module.broken_proxies().is_some())
    );
    assert!(!dock.repair_anchors.is_empty());
}

/// No two of the drydock's spawn points see each other, all are on the ground, and its bot
/// points link up by walks, every spawn point to at least two.
#[test]
fn drydock_spawns_are_out_of_sight_and_linked_by_walks() {
    let arena = Arena::drydock();
    assert_eq!(spawns_in_sight(&arena), Vec::new());
    assert!(arena.spawn_points().iter().all(|spawn| spawn.feet.y == 0));
    assert_walks_link_up(&arena);
}

/// The drydock's hull tops: each ship's foredeck and stern deck are bot points a walk reaches
/// (up the gantry's ramps, over its gangways, up the stern ramp), a jump pad in each outer lane
/// throws onto a foredeck and each stern deck carries thrusters. No hull top is an unanswered
/// perch: from every stern deck point some point on the other ship's hull and some point on a
/// gantry is in plain sight, and the crane pylon cuts the line between the two stern decks.
#[test]
fn drydock_hulls_are_reached_and_answered() {
    use shooter_sim::drydock::{FOREDECK_TOP, GANTRY_TOP, SHIP_X, STERN_TOP};
    use shooter_sim::pickups::Class;
    let arena = Arena::drydock();
    let points = arena.bot_points();
    let on = |west: bool, top: i32| -> Vec<Vec3i> {
        points
            .iter()
            .copied()
            .filter(|point| point.y == top && (point.x < 0) == west && point.x.abs() > 1_600)
            .collect()
    };
    for west in [true, false] {
        assert!(on(west, FOREDECK_TOP).len() >= 3, "{points:?}");
        assert!(on(west, STERN_TOP).len() >= 2, "{points:?}");
    }
    let thrusters: Vec<_> = arena
        .module_pads
        .iter()
        .filter(|pad| pad.module.class() == Class::Movement)
        .collect();
    assert_eq!(thrusters.len(), 2);
    assert!(thrusters.iter().all(|pad| pad.center.y == STERN_TOP));
    assert!(
        arena
            .module_pads
            .iter()
            .any(|pad| pad.module.class() == Class::Weapon && pad.center.y == 0)
    );
    for ship in [SHIP_X, -SHIP_X] {
        assert!(
            arena.pads.iter().any(|pad| {
                (pad.center.x - ship).abs() > 1_000
                    && (pad.center.x - ship).abs() < 1_500
                    && pad.launch.y >= 1_350
                    && pad.launch.x.signum() == (ship - pad.center.x).signum()
            }),
            "no pad onto the ship at {ship}"
        );
    }
    let world = arena.build_world(GRAVITY).unwrap();
    let sees = |a: Vec3i, b: Vec3i| {
        #[allow(clippy::cast_precision_loss)]
        let eye = |p: Vec3i| [p.x, p.y + EYE_HEIGHT_UNITS, p.z].map(|axis| axis as f32);
        let (from, to) = (eye(a), eye(b));
        world
            .cast_ray(
                from,
                [to[0] - from[0], to[1] - from[1], to[2] - from[2]],
                |_| true,
            )
            .is_none()
    };
    let gantries: Vec<Vec3i> = points
        .iter()
        .copied()
        .filter(|point| point.y == GANTRY_TOP && point.x.abs() < 1_600)
        .collect();
    for west in [true, false] {
        let other_hull: Vec<Vec3i> = on(!west, FOREDECK_TOP)
            .into_iter()
            .chain(on(!west, STERN_TOP))
            .collect();
        for perch in on(west, STERN_TOP) {
            assert!(
                other_hull.iter().any(|&point| sees(perch, point)),
                "{perch:?} sees no other hull"
            );
            assert!(
                gantries.iter().any(|&point| sees(perch, point)),
                "{perch:?} sees no gantry"
            );
        }
    }
    let (west_stern, east_stern) = (on(true, STERN_TOP), on(false, STERN_TOP));
    assert!(!sees(west_stern[0], east_stern[0]));
}
