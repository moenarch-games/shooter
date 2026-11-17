//! Partial arena destruction on the host (`shooter_sim::scenery`, shooter#60): the kit-built
//! maps' candidates and their stable IDs, a rail gun's hitscan and a rocket's blast breaking cover,
//! overlapping cover, an obstruction that blocks a shot and a jump before it breaks and not
//! after, the world, the robots' world and the snapshot changing at the same tick, a round's
//! restart restoring every placeable, the wire, a predictor adopting the host's word, and
//! matches that replay exactly.

use shooter_sim::aim;
use shooter_sim::arena::FIRST_PLAYER_BODY;
use shooter_sim::arena_kit::{self, Module, Placement, Turn, Variant};
use shooter_sim::physics::Vec3i;
use shooter_sim::robot::ARMORY;
use shooter_sim::scenery::{PlaceableState, Scenery, max_health};
use shooter_sim::wire::{WireError, decode_snapshot, encode_snapshot};
use shooter_sim::{Arena, Event, Input, Intent, MapId, Phase, Rules, Session, Spawn, Weapon};

const A: u32 = 1;
const B: u32 = 2;

/// An open floor with `placements`: A (slot 0) spawns 6 m south of the middle facing north, B
/// (slot 1) 6 m north facing south.
fn floor(placements: Vec<Placement>) -> Arena {
    Arena {
        half_size: 2_000,
        pieces: arena_kit::pieces(&placements),
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
        placements,
        repair_anchors: Vec::new(),
    }
}

fn rules(time_limit_ticks: u32) -> Rules {
    Rules {
        slots: 2,
        bots: false,
        local: false,
        seed: 0,
        frag_limit: 0,
        time_limit_ticks,
        intermission_ticks: 60,
        respawn_ticks: 30,
    }
}

/// A and B across `placements`; A carries every weapon.
fn duel_with(placements: Vec<Placement>, time_limit_ticks: u32) -> Session {
    let mut session = Session::with_rules(floor(placements), rules(time_limit_ticks)).unwrap();
    assert_eq!(session.join(A).unwrap(), Some(0));
    assert_eq!(session.join(B).unwrap(), Some(1));
    session.fighters[0].configured = ARMORY;
    session.rebuild(0, ARMORY).unwrap();
    session
}

fn duel(placements: Vec<Placement>) -> Session {
    duel_with(placements, 0)
}

/// One tick: A holds `a`, B stands still facing A.
fn step(session: &mut Session, a: Intent) {
    let b = Intent {
        yaw: 32_768,
        ..Intent::IDLE
    };
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

/// The yaw and pitch that put A's reticle on `point` ([`aim::at`]).
fn aim_at(session: &Session, point: [f32; 3]) -> (u16, i16) {
    aim::at(Some(&session.world), session.fighters[0].eye(), point)
}

/// A takes up `weapon`, waits for a full pool, then fires it once at `point`: the tick's
/// events.
fn fire_at(session: &mut Session, weapon: Weapon, point: [f32; 3]) -> Vec<Event> {
    let take = Intent {
        select: Some(weapon),
        ..Intent::IDLE
    };
    for _ in 0..240 {
        step(session, take);
        let fighter = &session.fighters[0];
        if fighter.arsenal.held == weapon && fighter.energy == fighter.pool().capacity {
            break;
        }
    }
    for _ in 0..240 {
        let (yaw, pitch) = aim_at(session, point);
        step(
            session,
            Intent {
                yaw,
                pitch,
                fire: true,
                select: Some(weapon),
                ..Intent::IDLE
            },
        );
        if session
            .events
            .iter()
            .any(|event| matches!(event, Event::Fired { slot: 0, .. }))
        {
            return session.events.clone();
        }
    }
    panic!("A never fired");
}

/// B's parts struck in `events`.
fn b_struck(events: &[Event]) -> bool {
    events
        .iter()
        .any(|event| matches!(event, Event::Hit { victim: 1, .. }))
}

/// Whether a ray from A's eye to `point` meets the arena (what bots and the camera look
/// through, and where hitscan stops).
fn blocked(session: &Session, point: [f32; 3]) -> bool {
    let eye = session.fighters[0].eye();
    let along = [0, 1, 2].map(|axis| point[axis] - eye[axis]);
    session
        .world
        .cast_ray(eye, along, |body| body.0 < FIRST_PLAYER_BODY)
        .is_some()
}

/// Every kit-built map marks its cover, barriers and crates as candidates, each with its stable
/// ID (its placement index) and a body run inside the arena's IDs; nothing load-bearing breaks:
/// no floor, deck, platform, ramp or wall is a candidate, and a crate under another never breaks.
/// The greybox maps have none.
#[test]
fn kit_maps_mark_damageable_cover_with_stable_ids() {
    for map in MapId::ALL {
        let arena = map.arena();
        let scenery = Scenery::of(&arena);
        assert!(scenery.last_body() < FIRST_PLAYER_BODY, "{map}");
        if arena.placements.is_empty() {
            assert!(scenery.placeables().is_empty(), "{map}");
            continue;
        }
        let damageable = scenery
            .placeables()
            .iter()
            .filter(|p| p.damageable())
            .count();
        assert!(damageable >= 8, "{map}: {damageable} damageable placeables");
        for placeable in scenery.placeables() {
            let placement = arena.placements[usize::from(placeable.id)];
            assert!(placement.destructible, "{map}: {placeable:?}");
            assert!(
                matches!(
                    placeable.module,
                    Module::CoverLow | Module::Barrier | Module::Crate | Module::Door
                ),
                "{map}: {placeable:?}"
            );
            assert_eq!(placeable.module, placement.module);
            // A carrier never breaks; everything else breaks at its module's health.
            let carries = arena.placements.iter().any(|top| {
                let ([low, high], [top_low, top_high]) = (placement.bounds(), top.bounds());
                top_low.y == high.y
                    && top_low.x < high.x
                    && low.x < top_high.x
                    && top_low.z < high.z
                    && low.z < top_high.z
            });
            let expected = max_health(placement.module).filter(|_| !carries);
            assert_eq!(placeable.max, expected, "{map}: {placeable:?}");
        }
        // The same IDs every time the map is built.
        assert_eq!(Scenery::of(&map.arena()), scenery);
    }
}

/// A rail gun at B through a 2 m barrier: the first two shots stop on the barrier (B unhurt)
/// and the third breaks it (250 health, 85 a shot); the snapshot of that tick, the physics world
/// and the arena all have it broken, and the fourth shot passes over its 1 m remains into B.
#[test]
fn hitscan_breaks_a_barrier_and_opens_the_line() {
    let mut session = duel(vec![
        Placement::new(Module::Barrier, [0, 0, 0], Turn::None).destructible(),
    ]);
    let target = |session: &Session| {
        let mut center = session.fighters[1].center();
        center[1] += 20.0;
        center
    };
    assert!(blocked(&session, target(&session)));
    let mut healths = Vec::new();
    for _ in 0..3 {
        let point = target(&session);
        let events = fire_at(&mut session, Weapon::RailGun, point);
        assert!(!b_struck(&events), "the barrier takes the shot");
        healths.push(session.snapshot().scenery);
    }
    assert_eq!(
        healths,
        [
            vec![PlaceableState { id: 0, health: 165 }],
            vec![PlaceableState { id: 0, health: 80 }],
            vec![PlaceableState { id: 0, health: 0 }],
        ]
    );
    assert_eq!(session.arena.placements[0].variant, Variant::Broken);
    assert_eq!(session.arena.pieces, session.arena.placements[0].pieces());
    // Its stub and base in the world, under its own body IDs.
    let statics = session.scenery.statics(&session.arena);
    for body in &statics[1..] {
        assert_eq!(session.world.body(body.id()), Some(body));
    }
    assert!(!blocked(&session, target(&session)));
    let point = target(&session);
    let events = fire_at(&mut session, Weapon::RailGun, point);
    assert!(b_struck(&events), "the line is open");
    // Broken, it takes no more damage.
    assert_eq!(
        session.snapshot().scenery,
        [PlaceableState { id: 0, health: 0 }]
    );
}

/// Overlapping cover: low cover with a barrier right behind it. A rocket at the low cover's foot
/// hurts both through its blast (the nearer more); rail shots stop on the nearer one first, and
/// once it breaks the barrier behind it takes them.
#[test]
fn blasts_and_shots_break_overlapping_cover_front_first() {
    let mut session = duel(vec![
        Placement::new(Module::CoverLow, [0, 0, -100], Turn::None).destructible(),
        Placement::new(Module::Barrier, [0, 0, 0], Turn::None).destructible(),
    ]);
    let events = fire_at(&mut session, Weapon::RocketLauncher, [0.0, 10.0, -140.0]);
    assert!(events.iter().any(|e| matches!(e, Event::Fired { .. })));
    for _ in 0..60 {
        if session
            .events
            .iter()
            .any(|e| matches!(e, Event::Exploded { .. }))
        {
            break;
        }
        step(&mut session, Intent::IDLE);
    }
    let after_blast = session.snapshot().scenery;
    assert_eq!(after_blast.len(), 2, "{after_blast:?}");
    let (cover, barrier) = (after_blast[0], after_blast[1]);
    assert_eq!((cover.id, barrier.id), (0, 1));
    let (cover_lost, barrier_lost) = (150 - cover.health, 250 - barrier.health);
    assert!(
        cover_lost > barrier_lost && barrier_lost > 0,
        "{after_blast:?}"
    );
    // Rail shots at the cover's top: the cover takes them until it breaks, then the barrier.
    let point = [0.0, 100.0, -100.0];
    let mut cover_health = cover.health;
    while cover_health > 0 {
        fire_at(&mut session, Weapon::RailGun, point);
        let state = session.snapshot().scenery;
        assert_eq!(state[1], barrier, "the barrier behind is untouched");
        cover_health = state[0].health;
    }
    assert_eq!(session.arena.placements[0].variant, Variant::Broken);
    // Low cover breaks to a 56 cm base and a stub at its -x end: a shot at 1 m passes on.
    fire_at(&mut session, Weapon::RailGun, point);
    let state = session.snapshot().scenery;
    assert_eq!(state[1].health, barrier.health - 85);
}

/// A rocket square on a crate breaks it in one blast; a crate carrying another never breaks.
#[test]
fn a_blast_breaks_a_crate_but_not_the_one_under_a_stack() {
    let mut session = duel(vec![
        Placement::new(Module::Crate, [-200, 0, 0], Turn::None).destructible(),
        Placement::new(Module::Crate, [200, 0, 0], Turn::None).destructible(),
        Placement::new(Module::Crate, [200, 80, 0], Turn::None).destructible(),
    ]);
    for x in [-200.0, 200.0] {
        fire_at(&mut session, Weapon::RocketLauncher, [x, 40.0, -40.0]);
        for _ in 0..60 {
            if session
                .events
                .iter()
                .any(|e| matches!(e, Event::Exploded { .. }))
            {
                break;
            }
            step(&mut session, Intent::IDLE);
        }
    }
    let variants: Vec<Variant> = session.arena.placements.iter().map(|p| p.variant).collect();
    assert_eq!(variants[0], Variant::Broken);
    assert_eq!(
        variants[1],
        Variant::Intact,
        "the lower crate of the stack carries"
    );
    assert!(session.scenery.placeables()[1].max.is_none());
}

/// A 2 m barrier stops a running jump; broken to its 1 m base, the same jump clears it. The
/// robots' world changes with the scenery's at the tick the barrier breaks.
#[test]
fn a_broken_barrier_opens_a_jump_route() {
    // Across the arena's middle, east of the line between A and B.
    let placements = vec![Placement::new(Module::Barrier, [-25, 0, 0], Turn::None).destructible()];
    let run = |break_first: bool| -> f32 {
        let mut session = duel(placements.clone());
        session.stand(0, [0.0, 0.0, -300.0]).unwrap();
        if break_first {
            session
                .adopt_scenery(&[PlaceableState { id: 0, health: 0 }])
                .unwrap();
        }
        let toward = Intent {
            forward: 1,
            sprint: true,
            ..Intent::IDLE
        };
        for tick in 0..120 {
            let jump = (12..30).contains(&tick);
            step(&mut session, Intent { jump, ..toward });
        }
        session.fighters[0].feet()[2]
    };
    let (intact, broken) = (run(false), run(true));
    assert!(intact < 0.0, "the barrier holds A south: {intact}");
    assert!(broken > 100.0, "A jumps the remains: {broken}");
}

/// A round restart brings every placeable back whole: the arena, the world's bodies and the
/// snapshot as at the start; within the round a broken placeable stays broken through a death
/// and respawn.
#[test]
fn a_new_round_restores_every_placeable() {
    let mut session = duel_with(
        vec![Placement::new(Module::Crate, [0, 0, 0], Turn::None).destructible()],
        600,
    );
    let whole_arena = session.arena.clone();
    let whole_statics = session.scenery.statics(&session.arena);
    fire_at(&mut session, Weapon::RocketLauncher, [0.0, 40.0, -40.0]);
    for _ in 0..60 {
        step(&mut session, Intent::IDLE);
    }
    assert_eq!(
        session.snapshot().scenery,
        [PlaceableState { id: 0, health: 0 }]
    );
    let broken_statics = session.scenery.statics(&session.arena);
    assert_ne!(broken_statics, whole_statics);
    // B falls out of the arena and comes back: the crate stays broken.
    session.stand(1, [0.0, -2_100.0, 600.0]).unwrap();
    for _ in 0..60 {
        step(&mut session, Intent::IDLE);
    }
    assert!(session.fighters[1].alive());
    assert_eq!(session.arena.placements[0].variant, Variant::Broken);
    let mut restarted = false;
    for _ in 0..1_200 {
        let before = session.phase;
        step(&mut session, Intent::IDLE);
        if matches!(before, Phase::Intermission { .. })
            && matches!(session.phase, Phase::Playing { .. })
        {
            restarted = true;
            break;
        }
        assert_eq!(session.arena.placements[0].variant, Variant::Broken);
    }
    assert!(restarted);
    assert!(session.snapshot().scenery.is_empty());
    assert_eq!(session.arena, whole_arena);
    for body in &whole_statics {
        assert_eq!(session.world.body(body.id()), Some(body));
    }
    let gone = broken_statics.iter().filter(|b| !whole_statics.contains(b));
    for body in gone {
        assert_ne!(session.world.body(body.id()), Some(body));
    }
}

/// The placeables' health rides the snapshot by stable ID: it round-trips, out-of-order IDs are
/// malformed, and a predictor on the same map that adopts it stands its arena, world and robots'
/// statics as the host's; one naming a placeable the map does not break is refused.
#[test]
fn the_wire_and_a_predictor_follow_the_hosts_scenery() {
    let mut host = Session::on_map(MapId::ProvingCell, Rules::practice(3)).unwrap();
    let damageable: Vec<usize> = host
        .scenery
        .placeables()
        .iter()
        .enumerate()
        .filter(|(_, p)| p.damageable())
        .map(|(index, _)| index)
        .take(3)
        .collect();
    let ids: Vec<u16> = damageable
        .iter()
        .map(|&index| host.scenery.placeables()[index].id)
        .collect();
    let state = vec![
        PlaceableState {
            id: ids[0],
            health: 0,
        },
        PlaceableState {
            id: ids[1],
            health: 10,
        },
    ];
    host.adopt_scenery(&state).unwrap();
    let snapshot = host.snapshot();
    assert_eq!(snapshot.scenery, state);
    let bytes = encode_snapshot(&snapshot);
    assert_eq!(decode_snapshot(&bytes).unwrap(), snapshot);
    let mut reversed = snapshot.clone();
    reversed.scenery.reverse();
    assert_eq!(
        decode_snapshot(&encode_snapshot(&reversed)),
        Err(WireError::Malformed("placeables out of ID order"))
    );

    let mut predictor = Session::on_map(MapId::ProvingCell, Rules::solo()).unwrap();
    predictor.adopt_scenery(&snapshot.scenery).unwrap();
    assert_eq!(predictor.arena, host.arena);
    let statics = host.scenery.statics(&host.arena);
    assert_eq!(predictor.scenery.statics(&predictor.arena), statics);
    for body in &statics {
        assert_eq!(predictor.world.body(body.id()), Some(body));
    }
    // A placeable that never breaks (or none at all) is refused, and nothing changes.
    let fixed = (0..u16::MAX)
        .find(|id| host.scenery.placeable(*id).is_none())
        .unwrap();
    assert!(
        predictor
            .adopt_scenery(&[PlaceableState {
                id: fixed,
                health: 0
            }])
            .is_err()
    );
    assert_eq!(predictor.arena, host.arena);
    // The host's next word: the broken one whole again (a new round), the other untouched.
    predictor.adopt_scenery(&[]).unwrap();
    assert_eq!(predictor.arena, MapId::ProvingCell.arena());
}

/// Two runs of the same bot match on each kit-built map produce the same snapshots tick for
/// tick, placeables included, and their bots' fire breaks some of them.
#[test]
fn bot_matches_break_cover_and_replay_exactly() {
    for map in [MapId::ProvingCell, MapId::AssemblyWorks] {
        let run = || {
            let mut session = Session::on_map(map, Rules::deathmatch(7)).unwrap();
            let mut snapshots = Vec::new();
            for _ in 0..3_600 {
                session.step().unwrap();
                snapshots.push(encode_snapshot(&session.snapshot()));
            }
            (session.snapshot().scenery, snapshots)
        };
        let (scenery, first) = run();
        let (_, second) = run();
        assert!(first == second, "{map}: a run diverged");
        println!("{map}: {scenery:?}");
        assert!(!scenery.is_empty(), "{map}: the bots damaged nothing");
    }
}
