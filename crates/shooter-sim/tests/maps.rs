//! The map catalog: stable IDs and wire tags, unknown maps rejected, a match keeping its map
//! and spawning on it through respawns and round restarts, and the slot count left to the
//! rules.

use shooter_sim::session::DEATHMATCH_SLOTS;
use shooter_sim::wire::{WireError, decode_snapshot, encode_snapshot};
use shooter_sim::{Arena, Event, MapId, Phase, Rules, Session, UnknownMap};

#[test]
fn ids_and_tags_are_stable_and_unique() {
    assert_eq!(MapId::DEFAULT, MapId::Greybox);
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
            .map(|fighter| fighter.player.feet(&session.world))
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
            let feet = fighter.player.feet(&session.world);
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
