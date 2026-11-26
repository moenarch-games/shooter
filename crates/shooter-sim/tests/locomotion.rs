//! Robots on wheels or tracks alone move on their own actuators in the match (shooter#58, Atlas
//! D2.i74.1): no drive carries or turns them. W and S roll their wheels and tracks through the
//! build's preset and A and D turn them differentially, the camera turns nothing, their tracks
//! hold their hulls off the ground, Space jumps only with a dedicated jump mechanism (the
//! thruster module) and never on wheels or tracks alone, depletion stops their locomotion, a
//! client predicts them exactly, and bots on wheels play and replay exactly. Robots with a
//! connected leg stay on the interim movement drive (the bounded fallback) and still jump.
// The platform's floats only check results here; the simulation uses libm (clippy.toml).
#![allow(clippy::disallowed_methods)]

use shooter_sim::bots::steer;
use shooter_sim::maps::MapId;
use shooter_sim::mechanics::fixtures;
use shooter_sim::physics::Vec3i;
use shooter_sim::session::LOCAL_PLAYER;
use shooter_sim::wire::{decode_snapshot, encode_snapshot};
use shooter_sim::{
    Arena, ArenaModule, Assembly, Carried, Event, Input, Intent, Rules, Session, Spawn, StepReport,
};

/// An open floor with two spawn points 12 m apart, no pads.
fn floor() -> Arena {
    Arena {
        half_size: 3_000,
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

fn solo(build: Assembly) -> Session {
    let mut session = Session::with_rules(floor(), Rules::solo()).unwrap();
    assert!(session.configure(LOCAL_PLAYER, build));
    session
}

/// One tick of `intent` through the local robot's preset layout (W and S roll, A and D turn).
fn drive(session: &mut Session, intent: Intent) -> StepReport {
    let intent = session.with_preset(0, intent);
    session.tick(intent).unwrap()
}

fn settle(session: &mut Session) {
    for _ in 0..30 {
        drive(session, Intent::IDLE);
    }
}

fn across(from: [f32; 3], to: [f32; 3]) -> f32 {
    (to[0] - from[0]).hypot(to[2] - from[2])
}

/// The signed turn from `from` to `to` (binary yaw), shortest way round.
fn turn(from: u16, to: u16) -> i32 {
    i32::from(to.wrapping_sub(from).cast_signed())
}

#[test]
fn wheels_roll_a_car_along_its_heading_and_turn_it_only_differentially() {
    let mut session = solo(fixtures::car());
    settle(&mut session);
    let (start, heading) = (session.local().feet(), session.local().heading());
    // W for 2 s with the camera sweeping round: it rolls straight ahead along its heading; the
    // camera turns nothing.
    for tick in 0..120u16 {
        drive(
            &mut session,
            Intent {
                forward: 1,
                yaw: tick.wrapping_mul(500),
                ..Intent::IDLE
            },
        );
    }
    let rolled = session.local().feet();
    assert!(across(start, rolled) > 400.0, "{start:?} {rolled:?}");
    assert!(
        turn(heading, session.local().heading()).abs() < 1_000,
        "{heading} {}",
        session.local().heading()
    );
    let ahead = shooter_sim::trig::yaw_from_vector(
        (rolled[0] - start[0]).round() as i32,
        (rolled[2] - start[2]).round() as i32,
    )
    .unwrap();
    assert!(turn(heading, ahead).abs() < 1_000, "{heading} {ahead}");
    // Stopped, then D for 1.5 s: it turns right (its yaw falls) on the spot, its sides rolling
    // opposite ways and its wheels skidding sideways (a skid steer turns slowly in place).
    for _ in 0..150 {
        drive(&mut session, Intent::IDLE);
    }
    let (at, before) = (session.local().feet(), session.local().heading());
    for _ in 0..90 {
        drive(
            &mut session,
            Intent {
                strafe: 1,
                ..Intent::IDLE
            },
        );
    }
    assert!(
        turn(before, session.local().heading()) < -1_000,
        "{before} {}",
        session.local().heading()
    );
    assert!(
        across(at, session.local().feet()) < 60.0,
        "{at:?} {:?}",
        session.local().feet()
    );
}

#[test]
fn tracks_hold_their_hull_off_the_ground_and_drive_it() {
    let mut session = solo(fixtures::flat_tank());
    settle(&mut session);
    // Only its road wheels touch the floor: the hull rides clear on them.
    let robot = session.mechanics.robot(0).unwrap();
    assert!(!session.mechanics.world().supported(robot.body(0)));
    assert!(session.local().stance.as_ref().unwrap().footing.feet);
    let start = session.local().feet();
    for _ in 0..90 {
        drive(
            &mut session,
            Intent {
                forward: 1,
                ..Intent::IDLE
            },
        );
    }
    assert!(across(start, session.local().feet()) > 150.0);
}

#[test]
fn space_jumps_on_legs_or_thrusters_and_does_nothing_on_wheels_alone() {
    let jump = Intent {
        jump: true,
        ..Intent::IDLE
    };
    // On wheels alone Space does nothing: no jump, no lift.
    let mut car = solo(fixtures::car());
    settle(&mut car);
    let floor_height = car.local().feet()[1];
    for _ in 0..20 {
        let report = drive(&mut car, jump);
        assert!(!report.jumped && !report.air_jumped);
    }
    assert!((car.local().feet()[1] - floor_height).abs() < 5.0);
    // The thruster module is a jump mechanism of its own: it lifts the car, and spends a use.
    let mut thrusted = solo(fixtures::car());
    settle(&mut thrusted);
    thrusted.fighters[0].carried = Some(Carried::fresh(ArenaModule::Thrusters));
    let report = drive(&mut thrusted, jump);
    assert!(report.jumped);
    let mut highest = floor_height;
    for _ in 0..20 {
        drive(&mut thrusted, Intent::IDLE);
        highest = highest.max(thrusted.local().feet()[1]);
    }
    assert!(highest > floor_height + 100.0, "{highest}");
    assert!(thrusted.fighters[0].carried.is_some_and(|carried| {
        carried.uses_left < Carried::fresh(ArenaModule::Thrusters).uses_left
    }));
    // A robot with a connected leg jumps (the interim movement drive's natural jump).
    let mut legged = Session::with_rules(floor(), Rules::solo()).unwrap();
    settle(&mut legged);
    assert!(drive(&mut legged, jump).jumped);
}

#[test]
fn without_electricity_the_wheels_stop() {
    // The car has no generator: with its electricity gone its wheels cannot roll it.
    let mut session = solo(fixtures::car());
    settle(&mut session);
    session.fighters[0].energy = 0;
    let start = session.local().feet();
    for _ in 0..90 {
        drive(
            &mut session,
            Intent {
                forward: 1,
                ..Intent::IDLE
            },
        );
        assert!(session.fighters[0].flow.stalled);
    }
    assert!(across(start, session.local().feet()) < 10.0);
    assert_eq!(session.fighters[0].energy, 0);
}

/// A person's car on a host, a client predicting it from the host's snapshot from tick 30 on,
/// both given the same inputs: the prediction follows the host bit for bit, rolling, turning
/// and backing up.
#[test]
fn a_client_predicts_a_wheeled_robot_exactly() {
    let mut host = Session::with_rules(
        floor(),
        Rules {
            slots: 1,
            bots: false,
            local: false,
            seed: 0,
            frag_limit: 0,
            time_limit_ticks: 0,
            intermission_ticks: 60,
            respawn_ticks: 30,
        },
    )
    .unwrap();
    host.join(7).unwrap();
    assert!(host.configure(7, fixtures::car()));
    let mut predictor = Session::new(floor()).unwrap();
    assert!(predictor.configure(LOCAL_PLAYER, fixtures::car()));
    let keys = [(0, 0), (1, 0), (1, 1), (0, -1), (-1, 0), (1, -1)];
    for tick in 0..360u32 {
        let sequence = tick + 1;
        let (forward, strafe) = keys[(tick / 40) as usize % keys.len()];
        let intent = predictor.with_preset(
            0,
            Intent {
                forward,
                strafe,
                yaw: (tick * 300) as u16,
                ..Intent::IDLE
            },
        );
        let view_tick = host.ticks();
        host.queue(
            7,
            Input {
                sequence,
                intent,
                view_tick,
            },
        );
        host.step().unwrap();
        let snapshot = decode_snapshot(&encode_snapshot(&host.snapshot())).unwrap();
        let me = snapshot.of_player(7).unwrap().clone();
        let robot = me.robot.clone().unwrap();
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
            assert_eq!(predictor.local().energy, me.energy, "tick {tick}");
        }
    }
}

/// Every fighter on `map` an armed car: the bots steer their cars themselves, cover ground and
/// shoot, and the match replays exactly from its seed.
fn wheeled_bots_play_on(map: MapId) {
    let play = || {
        let rules = Rules {
            frag_limit: 0,
            ..Rules::deathmatch(3)
        };
        let mut session = Session::on_map(map, rules).unwrap();
        for slot in 0..session.fighters.len() {
            let slot = u8::try_from(slot).unwrap();
            session.rebuild(slot, fixtures::armed_car()).unwrap();
        }
        let starts: Vec<[f32; 3]> = session.fighters.iter().map(|f| f.feet()).collect();
        let mut farthest = vec![0.0f32; starts.len()];
        let (mut shots, mut bytes) = (0, Vec::new());
        for _ in 0..45 * 60 {
            session.step().unwrap();
            shots += session
                .events
                .iter()
                .filter(|event| matches!(event, Event::Fired { .. }))
                .count();
            for (slot, fighter) in session.fighters.iter().enumerate() {
                if fighter.alive() {
                    farthest[slot] = farthest[slot].max(across(starts[slot], fighter.feet()));
                }
            }
            bytes.extend(encode_snapshot(&session.snapshot()));
        }
        (shots, farthest, bytes)
    };
    let (shots, farthest, bytes) = play();
    println!("{map}: {shots} shots; farthest {farthest:?}");
    assert!(shots > 20, "{map}: {shots} shots");
    let roamed = farthest.iter().filter(|far| **far > 500.0).count();
    assert!(roamed * 2 >= farthest.len(), "{map}: {farthest:?}");
    assert_eq!(play().2, bytes, "{map} replays exactly");
}

#[test]
fn wheeled_bots_play_and_replay_on_the_yard() {
    wheeled_bots_play_on(MapId::GreyboxYard);
}

#[test]
fn wheeled_bots_play_and_replay_on_the_assembly_works() {
    wheeled_bots_play_on(MapId::AssemblyWorks);
}

#[test]
fn wheeled_bots_play_and_replay_on_the_proving_cell() {
    wheeled_bots_play_on(MapId::ProvingCell);
}

#[test]
fn wheeled_bots_play_and_replay_on_the_drydock() {
    wheeled_bots_play_on(MapId::Drydock);
}

#[test]
fn steering_turns_toward_and_rolls_only_when_roughly_facing() {
    // Facing the target: roll, no turn.
    assert_eq!(steer(0, 500, 1), (1, 0));
    // A quarter turn to the left (yaw grows to the left): turn left on the spot.
    assert_eq!(steer(0, 16_384, 1), (0, -1));
    // 30° to the right: turn right while rolling.
    assert_eq!(steer(0, 0u16.wrapping_sub(5_461), 1), (1, 1));
    // Backing off still turns toward the target.
    assert_eq!(steer(0, 3_000, -1), (-1, -1));
}
