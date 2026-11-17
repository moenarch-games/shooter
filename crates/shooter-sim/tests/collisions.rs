//! Collision damage and physical debris (shooter#75, Atlas D2.i70.10): controlled fixtures that
//! tell damaging impacts (a head-on ram into a robot or a wall, a high fall) from harmless ones (a
//! glancing hit, low-speed contact, a robot leaning on a wall or lying still), the part struck
//! independent of the order the physics meets its contacts, an armor plate taking the impact, a
//! parent part breaking and its branch coming off as debris that falls, collides and retires, the
//! root chassis broken by an impact as the kill, the debris budget, exact continuation and the
//! wire, the interactions with field repair (an impact pauses it like a hit), module drops (a
//! carrier killed by an impact drops its module) and breakable placeables (debris on a crate falls
//! when it breaks), and a bounded multi-robot scene measuring active debris and update cost.
//!
//! `cargo test -p shooter-sim --test collisions`; the scene:
//! `cargo test --release -p shooter-sim --test collisions -- --ignored --nocapture`

use shooter_sim::damage::{Hitboxes, PartDamage};
use shooter_sim::impact::{self, IMPACT_SPEED};
use shooter_sim::mechanics::{
    DEBRIS_LIFETIME_TICKS, DebrisState, MAX_DEBRIS, Mechanics, placements,
};
use shooter_sim::physics::Vec3i;
use shooter_sim::robot::parts::{ARMOR_PLATE, BATTERY, BEAM_LONG};
use shooter_sim::robot::{BETA, Face, PartId, RawMount, RawPart, STANDARD};
use shooter_sim::wire::{decode_mechanics, decode_snapshot, encode_mechanics, encode_snapshot};
use shooter_sim::{Arena, Assembly, Event, Input, Intent, Piece, Rules, Session};

/// Facing +Z (yaw 0).
const NORTH: u16 = 0;
/// The front-most part of the standard robot: its weapons tie, the rifle has the lowest ID.
const RIFLE: PartId = PartId(7);
/// The match's gravity in the fixtures (m/s²).
const GRAVITY: [f32; 3] = [0.0, -18.0, 0.0];

type Blows = Vec<(u8, PartId, i32)>;

/// An open floor 60 m across, with a wall across +Z when `wall` (its face at z = 500).
fn arena(wall: bool) -> Arena {
    Arena {
        half_size: 3_000,
        pieces: if wall {
            vec![Piece::Block {
                center: Vec3i::new(0, 400, 600),
                half: Vec3i::new(1_500, 400, 100),
            }]
        } else {
            Vec::new()
        },
        pads: Vec::new(),
        posts: Vec::new(),
        spawn: Vec3i::new(0, 0, -600),
        spawn_yaw: 0,
        spawns: Vec::new(),
        module_pads: Vec::new(),
        waypoints: Vec::new(),
        placements: Vec::new(),
        repair_anchors: Vec::new(),
    }
}

/// `builds.len()` people in a match on [`arena`], each built as its build.
fn match_of(builds: &[Assembly], wall: bool) -> Session {
    let rules = Rules {
        slots: u8::try_from(builds.len()).unwrap(),
        bots: false,
        local: false,
        seed: 0,
        frag_limit: 0,
        time_limit_ticks: 0,
        intermission_ticks: 60,
        respawn_ticks: 600,
    };
    let mut session = Session::with_rules(arena(wall), rules).unwrap();
    for (slot, build) in builds.iter().enumerate() {
        let id = u32::try_from(slot).unwrap() + 1;
        assert_eq!(session.join(id).unwrap(), Some(u8::try_from(slot).unwrap()));
        session.fighters[slot].configured = *build;
        session
            .rebuild(u8::try_from(slot).unwrap(), *build)
            .unwrap();
    }
    session
}

/// `slot` stands facing `yaw` with its feet at `feet` (units): in the air when above the floor.
fn place(session: &mut Session, slot: u8, feet: [f32; 3], yaw: u16) {
    session.fighters[usize::from(slot)].intent.yaw = yaw;
    session.stand(slot, feet).unwrap();
}

/// One tick: each person keeps `intents[slot]`.
fn step(session: &mut Session, intents: &[Intent]) {
    for (slot, intent) in intents.iter().enumerate() {
        let id = u32::try_from(slot).unwrap() + 1;
        let ack = session.fighters[slot].ack;
        session.queue(
            id,
            Input {
                sequence: ack + 1,
                intent: *intent,
                view_tick: session.ticks(),
            },
        );
    }
    session.step().unwrap();
}

fn facing(yaw: u16) -> Intent {
    Intent {
        yaw,
        ..Intent::IDLE
    }
}

/// The collision blows of `events` (nobody the attacker): victim, part, damage.
fn impacts(events: &[Event]) -> Blows {
    events
        .iter()
        .filter_map(|event| match *event {
            Event::Hit {
                victim,
                attacker: None,
                part,
                damage,
                ..
            } => Some((victim, part, damage)),
            _ => None,
        })
        .collect()
}

/// Runs `ticks` ticks of `intents`, collecting every collision blow.
fn run(session: &mut Session, intents: &[Intent], ticks: u32) -> Blows {
    let mut all = Vec::new();
    for _ in 0..ticks {
        step(session, intents);
        all.extend(impacts(&session.events));
    }
    all
}

/// The standard robot without its legs: the root chassis is its lowest part.
fn legless() -> Assembly {
    let mut candidate = STANDARD.candidate();
    candidate.parts.retain(|part| part.id != 5 && part.id != 6);
    BETA.validate(&candidate).unwrap()
}

/// The first leg socket, on the chassis' underside.
fn underside() -> u8 {
    STANDARD
        .candidate()
        .parts
        .iter()
        .find(|part| part.id == 5)
        .and_then(|part| part.mount)
        .unwrap()
        .socket
}

/// [`legless`] standing on a long beam (part 3, 60 health) with a battery (part 4, 20 kg) on the
/// beam's side near the chassis: the beam's foot is the lowest point.
fn pole() -> Assembly {
    let mut candidate = legless().candidate();
    candidate.parts.push(RawPart {
        id: 3,
        kind: BEAM_LONG.id.0,
        mount: Some(RawMount {
            parent: 0,
            socket: underside(),
            ..RawMount::default()
        }),
        modification: None,
    });
    candidate.parts.push(RawPart {
        id: 4,
        kind: BATTERY.id.0,
        mount: Some(RawMount {
            parent: 3,
            socket: BEAM_LONG.socket(Face::PosX, 0, 0).unwrap().0,
            ..RawMount::default()
        }),
        modification: None,
    });
    BETA.validate(&candidate).unwrap()
}

/// [`legless`] with an armor plate (part 11) under its chassis.
fn keeled() -> Assembly {
    let mut candidate = legless().candidate();
    candidate.parts.push(RawPart {
        id: 11,
        kind: ARMOR_PLATE.id.0,
        mount: Some(RawMount {
            parent: 0,
            socket: underside(),
            ..RawMount::default()
        }),
        modification: None,
    });
    BETA.validate(&candidate).unwrap()
}

/// Two standard robots 6 m apart in the air, both facing north, driven broadside into each other
/// along X at `speed` (units a second) each; `swapped` puts slot 1 in the west one's place.
/// (Front to front their thin gun barrels meet first, which the physics resolves unreliably at
/// these speeds; their flat sides meet squarely.)
fn ram(speed: f32, swapped: bool) -> (Session, Blows) {
    let mut session = match_of(&[STANDARD, STANDARD], false);
    let (west, east) = if swapped { (1, 0) } else { (0, 1) };
    place(&mut session, west, [-300.0, 100.0, 0.0], NORTH);
    place(&mut session, east, [300.0, 100.0, 0.0], NORTH);
    session.launch(west, [speed, 0.0, 0.0]).unwrap();
    session.launch(east, [-speed, 0.0, 0.0]).unwrap();
    let blows = run(&mut session, &[facing(NORTH), facing(NORTH)], 30);
    (session, blows)
}

#[test]
fn a_head_on_ram_damages_both_robots_and_a_slow_one_nothing() {
    let (session, blows) = ram(2_500.0, false);
    assert_eq!(blows.len(), 2, "{blows:?}");
    for (slot, (victim, _, damage)) in blows.iter().enumerate() {
        assert_eq!(usize::from(*victim), slot);
        assert!(*damage > 0);
    }
    // Equal masses meeting at equal speeds each lose their own speed: the same damage, on the
    // mirrored part (each one's side facing the other; a leg, which its stance splays furthest).
    assert_eq!(blows[0].2, blows[1].2);
    assert_eq!([blows[0].1, blows[1].1], [PartId(6), PartId(5)]);
    for fighter in &session.fighters {
        assert!(fighter.alive());
    }
    // A harder ram does more.
    let (_, harder) = ram(3_000.0, false);
    assert!(harder[0].2 > blows[0].2, "{harder:?}");
    // At a sprint (10.5 m/s each) the robots only bump.
    let (session, blows) = ram(1_050.0, false);
    assert!(blows.is_empty(), "{blows:?}");
    assert!(
        session
            .fighters
            .iter()
            .all(|fighter| fighter.parts == PartDamage::fresh(&STANDARD))
    );
}

#[test]
fn the_struck_part_does_not_depend_on_the_order_contacts_are_met() {
    // The same ram with the slots (and so the bodies' order in the physics) swapped: the robot
    // coming from the west loses the same part, for the same damage within a point (the solver
    // meeting the contacts in another order rounds the impulse differently).
    let (_, ahead) = ram(2_500.0, false);
    let (_, swapped) = ram(2_500.0, true);
    let of = |blows: &Blows, slot: u8| {
        *blows
            .iter()
            .find(|(victim, ..)| *victim == slot)
            .expect("both robots are struck")
    };
    for (here, there) in [(0, 1), (1, 0)] {
        let (a, b) = (of(&ahead, here), of(&swapped, there));
        assert_eq!(a.1, b.1, "{ahead:?} {swapped:?}");
        assert!((a.2 - b.2).abs() <= 1, "{ahead:?} {swapped:?}");
    }
    // The struck part follows from the geometry alone (ties by part ID).
    let boxes = Hitboxes::place(
        &STANDARD,
        &PartDamage::fresh(&STANDARD),
        [0.0, 90.0, 0.0],
        NORTH,
    );
    let ahead = impact::strike(&boxes, [0.0, 0.0, -1.0], 30);
    assert_eq!(ahead.len(), 1);
    assert_eq!(STANDARD.parts()[ahead[0].index].id, RIFLE);
    let below = impact::strike(&boxes, [0.0, 1.0, 0.0], 30);
    assert_eq!(
        STANDARD.parts()[below[0].index].id,
        PartId(5),
        "the left leg"
    );
}

#[test]
fn a_wall_hit_head_on_damages_and_a_glancing_one_does_not() {
    // The blows, and every tick's velocity while it flies (before it lands and stops).
    let into_wall = |velocity: [f32; 3]| {
        let mut session = match_of(&[STANDARD], true);
        place(&mut session, 0, [0.0, 150.0, 420.0], NORTH);
        session.launch(0, velocity).unwrap();
        let mut flight = Vec::new();
        let mut blows = Vec::new();
        for _ in 0..15 {
            step(&mut session, &[facing(NORTH)]);
            blows.extend(impacts(&session.events));
            flight.push(session.fighters[0].velocity());
        }
        (flight, blows)
    };
    // Straight into the wall at 30 m/s: one blow, on the part reaching furthest toward it (its
    // forward-swung leg, past the guns on its chest).
    let (_, blows) = into_wall([0.0, 0.0, 3_000.0]);
    assert_eq!(blows.len(), 1, "{blows:?}");
    assert!(blows[0].2 > 0);
    // As fast along the wall but closing at 4 m/s: it touches (its speed into the wall is gone)
    // and slides on, unhurt.
    let (flight, blows) = into_wall([3_000.0, 0.0, 400.0]);
    assert!(blows.is_empty(), "{blows:?}");
    let last = flight.last().unwrap();
    assert!(last[2].abs() < 50.0, "{flight:?}");
    assert!(last[0] > 2_000.0, "{flight:?}");
    // Low speed straight on: a bump.
    let (flight, blows) = into_wall([0.0, 0.0, 800.0]);
    assert!(blows.is_empty(), "{blows:?}");
    assert!(flight.last().unwrap()[2].abs() < 100.0, "{flight:?}");
}

#[test]
fn walking_leaning_and_lying_still_never_hurt() {
    // Sprinting into the wall and then leaning on it for five seconds, while another robot
    // sprints into the first one's back and stays pressed there.
    let mut session = match_of(&[STANDARD, STANDARD], true);
    place(&mut session, 0, [0.0, 0.0, 0.0], NORTH);
    place(&mut session, 1, [0.0, 0.0, -400.0], NORTH);
    let sprint = Intent {
        forward: 1,
        sprint: true,
        ..facing(NORTH)
    };
    let blows = run(&mut session, &[sprint, sprint], 300);
    assert!(blows.is_empty(), "{blows:?}");
    assert!(
        session.fighters[0].center()[2] > 400.0,
        "it reached the wall"
    );
    // Lying still on the floor (legless), ten seconds.
    let mut session = match_of(&[legless()], false);
    place(&mut session, 0, [0.0, 0.0, 0.0], NORTH);
    let blows = run(&mut session, &[facing(NORTH)], 600);
    assert!(blows.is_empty(), "{blows:?}");
    // Jumping and landing on the spot for five seconds.
    let mut session = match_of(&[STANDARD], false);
    place(&mut session, 0, [0.0, 0.0, 0.0], NORTH);
    let jump = Intent {
        jump: true,
        ..facing(NORTH)
    };
    let blows = run(&mut session, &[jump], 300);
    assert!(blows.is_empty(), "{blows:?}");
}

/// Drops `build` with its feet `height` units above the floor; the collision blows over `ticks`.
fn drop(build: Assembly, height: f32, ticks: u32) -> (Session, Blows) {
    let mut session = match_of(&[build], false);
    place(&mut session, 0, [0.0, height, 0.0], NORTH);
    let blows = run(&mut session, &[facing(NORTH)], ticks);
    (session, blows)
}

#[test]
fn a_high_fall_breaks_the_lowest_part_and_a_low_one_nothing() {
    // Two metres, eight: nothing (a fall starts to hurt at about 17 m).
    for height in [200.0, 800.0] {
        let (session, blows) = drop(STANDARD, height, 120);
        assert!(blows.is_empty(), "{height}: {blows:?}");
        assert!(session.fighters[0].alive());
    }
    // Thirty metres: the feet meet the floor first; the left leg (lowest ID of the two) breaks
    // and comes off, shattered (a destroyed part is no debris).
    let (session, blows) = drop(STANDARD, 3_000.0, 150);
    assert_eq!(blows.len(), 1, "{blows:?}");
    assert_eq!(blows[0].1, PartId(5));
    assert!(blows[0].2 >= 70, "{blows:?}");
    assert!(session.fighters[0].alive());
    assert!(!session.fighters[0].parts.attached(2));
    assert!(session.mechanics.debris().is_empty());
}

#[test]
fn an_armor_plate_under_the_chassis_takes_the_whole_impact() {
    let (session, blows) = drop(keeled(), 3_000.0, 150);
    assert_eq!(blows.len(), 1, "{blows:?}");
    assert_eq!(blows[0].1, PartId(11));
    assert_eq!(session.fighters[0].chassis_health(), 270);
}

#[test]
fn a_broken_parent_takes_its_branch_offline_and_the_branch_falls_as_debris() {
    let (mut session, _) = drop(pole(), 3_000.0, 0);
    let mut landed = Vec::new();
    let mut events = Vec::new();
    for _ in 0..150 {
        step(&mut session, &[facing(NORTH)]);
        landed.extend(impacts(&session.events));
        events.extend(session.events.iter().copied());
        if !landed.is_empty() {
            break;
        }
    }
    // The beam's foot hits the floor: the beam breaks, the battery hanging from it comes off
    // whole and stays a physical body.
    assert_eq!(landed.len(), 1, "{landed:?}");
    assert_eq!(landed[0].1, PartId(3));
    assert!(events.contains(&Event::Detached {
        slot: 0,
        part: PartId(3),
        destroyed: true,
    }));
    assert!(events.contains(&Event::Detached {
        slot: 0,
        part: PartId(4),
        destroyed: false,
    }));
    let fighter = &session.fighters[0];
    assert!(fighter.alive());
    let battery = fighter
        .build
        .parts()
        .iter()
        .position(|part| part.id == PartId(4))
        .unwrap();
    assert!(!fighter.parts.attached(battery), "offline with its parent");
    assert!(fighter.parts.health()[battery] > 0, "but not destroyed");
    let debris = session.mechanics.debris();
    assert_eq!(debris.len(), 1);
    assert_eq!((debris[0].slot, debris[0].part), (0, PartId(4)));
    // It falls, collides with the floor (and the robot) and comes to rest on it, then retires
    // deterministically: after lying still a second, at the latest at the end of its lifetime.
    let half = placements(&pole())
        .iter()
        .find(|placement| placement.part == PartId(4))
        .unwrap()
        .half_extents;
    let thinnest = half.iter().copied().fold(f32::MAX, f32::min);
    let mut lowest = f32::MAX;
    let mut ticks = 0;
    while let Some(piece) = session.mechanics.debris().first().copied() {
        lowest = lowest.min(piece.motion.pose.position[1]);
        step(&mut session, &[facing(NORTH)]);
        ticks += 1;
        assert!(ticks <= u32::from(DEBRIS_LIFETIME_TICKS) + 1, "it retires");
    }
    assert!(lowest >= thinnest - 2.0, "it lies on the floor: {lowest}");
    assert!(lowest < 40.0, "it fell to the floor: {lowest}");
    println!("the battery's debris retired after {ticks} ticks");
    assert!(session.fighters[0].alive());
}

#[test]
fn an_impact_on_the_root_chassis_is_the_kill() {
    // No legs: the chassis meets the floor first. A 60 m fall breaks it.
    let (mut session, _) = drop(legless(), 6_000.0, 0);
    let mut killed = None;
    for _ in 0..240 {
        step(&mut session, &[facing(NORTH)]);
        if let Some(event) = session
            .events
            .iter()
            .find(|event| matches!(event, Event::Killed { .. }))
        {
            killed = Some(*event);
            break;
        }
    }
    assert_eq!(
        killed,
        Some(Event::Killed {
            victim: 0,
            killer: None,
            weapon: None,
        })
    );
    assert!(!session.fighters[0].alive());
    assert_eq!(
        session.fighters[0].frags, -1,
        "as for a fall out of the arena"
    );
}

#[test]
fn debris_stays_within_its_budget_oldest_first_and_continues_exactly() {
    // 20 standard robots in a row, each losing its four weapons whole (each at least 12 kg
    // becomes debris) a tick apart.
    let statics = arena(false).statics();
    let mut mechanics = Mechanics::for_match(&statics, GRAVITY).unwrap();
    for slot in 0..20u8 {
        let x = f32::from(slot) * 200.0 - 2_000.0;
        mechanics
            .spawn_at(slot, STANDARD, [x, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0])
            .unwrap();
    }
    let weapons: u32 = (4..8).map(|index| 1 << index).sum();
    let mut most = 0;
    for slot in 0..20u8 {
        mechanics.break_off(slot, weapons, weapons).unwrap();
        mechanics.step(&[]).unwrap();
        most = most.max(mechanics.debris().len());
        assert!(mechanics.debris().len() <= MAX_DEBRIS);
    }
    assert_eq!(most, MAX_DEBRIS);
    // The newest are kept: the last robot's pieces are there, the first's retired.
    let slots: Vec<u8> = mechanics.debris().iter().map(|piece| piece.slot).collect();
    assert!(slots.contains(&19) && !slots.contains(&0), "{slots:?}");
    // A world restored from the state (a client's, a replay's) continues bit for bit, debris and
    // all, through the pieces' landing and retirement.
    let state = decode_mechanics(&encode_mechanics(&mechanics.state())).unwrap();
    assert_eq!(state, mechanics.state());
    let mut copy = Mechanics::restore_for_match(&statics, GRAVITY, &state).unwrap();
    for _ in 0..400 {
        for world in [&mut mechanics, &mut copy] {
            world.retire_debris().unwrap();
            world.step(&[]).unwrap();
        }
        assert_eq!(copy.state(), mechanics.state());
    }
    assert!(mechanics.debris().is_empty(), "all retired");
}

#[test]
fn collisions_and_debris_travel_in_snapshots_and_replay_exactly() {
    let play = || {
        let (mut session, _) = drop(pole(), 3_000.0, 0);
        let mut snapshots = Vec::new();
        for _ in 0..200 {
            step(&mut session, &[facing(NORTH)]);
            snapshots.push(encode_snapshot(&session.snapshot()));
        }
        snapshots
    };
    let first = play();
    assert_eq!(first, play(), "the same match replays bit for bit");
    let with_debris = first
        .iter()
        .map(|bytes| decode_snapshot(bytes).unwrap())
        .find(|snapshot| !snapshot.debris.is_empty())
        .expect("a snapshot carries the debris");
    assert_eq!(
        decode_snapshot(&encode_snapshot(&with_debris)).unwrap(),
        with_debris
    );
    // A predictor put among the host's pieces holds them exactly.
    let mut predictor = match_of(&[pole()], false);
    predictor.place_debris(&with_debris.debris).unwrap();
    assert_eq!(predictor.mechanics.debris(), &with_debris.debris[..]);
    // Malformed debris is refused: past the budget, not finite, without mass.
    let mut many = with_debris.clone();
    many.debris = vec![with_debris.debris[0]; MAX_DEBRIS + 1];
    assert!(decode_snapshot(&encode_snapshot(&many)).is_err());
    let mut broken = with_debris.clone();
    broken.debris[0].motion.velocity[0] = f32::NAN;
    assert!(decode_snapshot(&encode_snapshot(&broken)).is_err());
    let mut massless: DebrisState = with_debris.debris[0];
    massless.piece.mass = 0.0;
    broken.debris = vec![massless];
    assert!(decode_snapshot(&encode_snapshot(&broken)).is_err());
}

#[test]
fn the_threshold_sits_above_walking_sprinting_and_sliding() {
    // A slide (15 m/s) is the fastest move on foot; the grapple pulls at up to 24 m/s.
    const { assert!(IMPACT_SPEED > 1_500.0) };
    assert_eq!(impact::damage(1_500.0), 0);
    assert_eq!(impact::damage(IMPACT_SPEED), 0);
    assert!(impact::damage(2_400.0) > 0);
}

/// 12 robots in a ring on the greybox floor lose their four weapons (whole) once a second and
/// are rebuilt whole half a second later, with the lost weapons left as debris or not; every
/// tick is timed whole (lifecycle rebuilds, retirements and the step). Prints the active debris
/// and the cost; asserts only the budget.
#[test]
#[ignore = "a measured scene: run with --release --ignored --nocapture"]
fn twelve_robots_shedding_debris_a_tick() {
    use std::time::Instant;
    let statics = Arena::greybox().statics();
    let ring = |slot: u8| {
        let angle = f32::from(slot) * std::f32::consts::TAU / 12.0;
        [angle.cos() * 800.0, 0.0, angle.sin() * 800.0]
    };
    let weapons: u32 = (4..8).map(|index| 1 << index).sum();
    let run = |debris: bool| {
        let mut mechanics = Mechanics::for_match(&statics, GRAVITY).unwrap();
        for slot in 0..12u8 {
            mechanics
                .spawn_at(slot, STANDARD, ring(slot), [0.0, 0.0, 0.0, 1.0])
                .unwrap();
        }
        let (mut total, mut worst, mut active, mut peak) = (0.0f64, 0.0f64, 0usize, 0usize);
        let ticks = 1_200u32;
        for tick in 0..ticks {
            let started = Instant::now();
            let phase = u8::try_from(tick % 60).unwrap();
            if phase < 12 {
                let intact = if debris { weapons } else { 0 };
                mechanics.break_off(phase, weapons, intact).unwrap();
            } else if (30..42).contains(&phase) {
                let slot = phase - 30;
                mechanics.despawn(slot).unwrap();
                mechanics
                    .spawn_at(slot, STANDARD, ring(slot), [0.0, 0.0, 0.0, 1.0])
                    .unwrap();
            }
            mechanics.retire_debris().unwrap();
            mechanics.step(&[]).unwrap();
            let spent = started.elapsed().as_secs_f64() * 1_000.0;
            total += spent;
            worst = worst.max(spent);
            active += mechanics.debris().len();
            peak = peak.max(mechanics.debris().len());
            assert!(mechanics.debris().len() <= MAX_DEBRIS);
        }
        #[allow(clippy::cast_precision_loss)]
        let mean = active as f64 / f64::from(ticks);
        (total / f64::from(ticks), worst, mean, peak)
    };
    let (bare, bare_worst, _, _) = run(false);
    let (shed, shed_worst, mean, peak) = run(true);
    println!(
        "12 robots shedding 4 weapons a second each: {bare:.3} ms/tick (worst {bare_worst:.3}) \
         with the weapons shattered; {shed:.3} ms/tick (worst {shed_worst:.3}) with them as \
         debris, {mean:.1} pieces active on average, {peak} at most (budget {MAX_DEBRIS})"
    );
}

/// The interactions with field repair (shooter#61), module drops (shooter#62) and breakable
/// placeables (shooter#60).
mod interactions {
    use super::*;
    use shooter_sim::arena_kit::{self, Module, Placement, Turn};
    use shooter_sim::repair::{Order, PAUSE_TICKS};
    use shooter_sim::robot::MECHANIC;
    use shooter_sim::scenery::PlaceableState;
    use shooter_sim::{ArenaModule, Carried};

    const LEG: PartId = PartId(5);

    #[test]
    fn an_impact_on_the_part_under_repair_pauses_it_like_a_hit() {
        // The mechanic repairs its worn left leg, then falls far enough that the leg, its
        // lowest part, takes a blow it survives: the repair pauses, as for a weapon's hit.
        let mut paused = None;
        for height in (900u16..=1_400).step_by(25) {
            let mut session = match_of(&[MECHANIC], false);
            place(&mut session, 0, [0.0, 0.0, 0.0], NORTH);
            for _ in 0..10 {
                step(&mut session, &[facing(NORTH)]);
            }
            let leg = 2;
            assert_eq!(MECHANIC.parts()[leg].id, LEG);
            session.fighters[0].parts.hurt(&MECHANIC, leg, 40);
            let repair = Intent {
                repair: Some(Order::Start(LEG)),
                ..facing(NORTH)
            };
            step(&mut session, &[repair]);
            assert_eq!(session.fighters[0].repair.map(|r| r.target), Some(LEG));
            // Up into the air with the repair under way, and down.
            let feet = session.fighters[0].feet();
            place(
                &mut session,
                0,
                [feet[0], f32::from(height), feet[2]],
                NORTH,
            );
            let mut blows = Vec::new();
            for _ in 0..90 {
                step(&mut session, &[facing(NORTH)]);
                blows.extend(impacts(&session.events));
                if !blows.is_empty() {
                    break;
                }
            }
            match blows.first() {
                Some(&(0, part, damage))
                    if part == LEG && session.fighters[0].parts.attached(leg) =>
                {
                    paused = Some((height, damage, session.fighters[0].repair));
                    break;
                }
                _ => {}
            }
        }
        let (height, damage, repair) = paused.expect("a fall that strikes the leg and spares it");
        println!("{height} units: the leg took {damage}");
        let repair = repair.expect("the repair goes on, paused");
        assert_eq!(repair.target, LEG);
        assert_eq!(repair.paused, PAUSE_TICKS);
    }

    #[test]
    fn a_carrier_killed_by_an_impact_drops_its_module() {
        let mut session = match_of(&[legless()], false);
        place(&mut session, 0, [0.0, 6_000.0, 0.0], NORTH);
        session.fighters[0].carried = Some(Carried {
            module: ArenaModule::Thrusters,
            uses_left: 3,
        });
        for _ in 0..240 {
            step(&mut session, &[facing(NORTH)]);
            if !session.fighters[0].alive() {
                break;
            }
        }
        assert!(!session.fighters[0].alive(), "the impact killed it");
        assert_eq!(session.drops.len(), 1, "{:?}", session.drops);
        assert_eq!(session.drops[0].carried.module, ArenaModule::Thrusters);
        assert_eq!(session.drops[0].from, 0);
    }

    #[test]
    fn debris_on_a_crate_falls_when_the_crate_breaks() {
        let placements = vec![Placement::new(Module::Crate, [0, 0, 0], Turn::None).destructible()];
        let mut arena = arena(false);
        arena.pieces = arena_kit::pieces(&placements);
        arena.placements = placements;
        let rules = Rules {
            slots: 1,
            bots: false,
            local: false,
            seed: 0,
            frag_limit: 0,
            time_limit_ticks: 0,
            intermission_ticks: 60,
            respawn_ticks: 600,
        };
        let mut session = Session::with_rules(arena, rules).unwrap();
        // A battery's worth of debris dropped onto the crate (80 units tall).
        let mut host = match_of(&[pole()], false);
        place(&mut host, 0, [0.0, 3_000.0, 0.0], NORTH);
        while host.mechanics.debris().is_empty() {
            step(&mut host, &[facing(NORTH)]);
        }
        let mut piece = host.mechanics.debris()[0];
        piece.motion.pose.position = [0.0, 200.0, 0.0];
        piece.motion.velocity = [0.0; 3];
        piece.motion.angular_velocity = [0.0; 3];
        session.place_debris(&[piece]).unwrap();
        for _ in 0..40 {
            session.step().unwrap();
        }
        let on_crate = session.mechanics.debris()[0].motion.pose.position[1];
        assert!(on_crate > 80.0, "it lies on the crate: {on_crate}");
        // The crate breaks: the world is rebuilt on the broken pieces with the debris kept.
        session
            .adopt_scenery(&[PlaceableState { id: 0, health: 0 }])
            .unwrap();
        assert_eq!(session.mechanics.debris().len(), 1);
        let mut lowest = on_crate;
        for _ in 0..40 {
            session.step().unwrap();
            if let Some(piece) = session.mechanics.debris().first() {
                lowest = lowest.min(piece.motion.pose.position[1]);
            }
        }
        assert!(lowest < on_crate - 20.0, "{on_crate} -> {lowest}");
    }
}
