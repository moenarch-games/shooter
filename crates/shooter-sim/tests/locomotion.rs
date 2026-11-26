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
    predicts_exactly(fixtures::car());
}

/// [`a_client_predicts_a_wheeled_robot_exactly`] for the steer-car (shooter#170): its servos
/// steering as well.
#[test]
fn a_client_predicts_a_steered_car_exactly() {
    predicts_exactly(shooter_sim::robot::STEER_CAR);
}

fn predicts_exactly(build: Assembly) {
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
    assert!(host.configure(7, build));
    let mut predictor = Session::new(floor()).unwrap();
    assert!(predictor.configure(LOCAL_PLAYER, build));
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
    bots_play_as(map, fixtures::armed_car(), 45);
}

/// Every fighter on `map` playing `build` for `seconds`: the bots move it themselves, cover
/// ground and shoot, and the match replays exactly from its seed.
fn bots_play_as(map: MapId, build: Assembly, seconds: usize) {
    let play = || {
        let rules = Rules {
            frag_limit: 0,
            ..Rules::deathmatch(3)
        };
        let mut session = Session::on_map(map, rules).unwrap();
        for slot in 0..session.fighters.len() {
            let slot = u8::try_from(slot).unwrap();
            session.rebuild(slot, build).unwrap();
        }
        let starts: Vec<[f32; 3]> = session.fighters.iter().map(|f| f.feet()).collect();
        let mut farthest = vec![0.0f32; starts.len()];
        let (mut shots, mut bytes) = (0, Vec::new());
        for _ in 0..seconds * 60 {
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

/// The Workshop's robo-dog, rover and mini-tank (shooter#169) play as built: each settles on
/// its own locomotion (the dog's legs on the interim drive, the rover's wheels and the tank's
/// tracks on their own actuators), W carries it forward, and its one gun fires on the primary
/// trigger without a weapon being chosen first.
#[test]
fn the_workshop_presets_drive_and_fire() {
    use shooter_sim::Weapon;
    use shooter_sim::robot::loadout;

    // The rover is the armed car every map's wheeled playtests run.
    assert_eq!(loadout("rover"), Some(fixtures::armed_car()));
    for (name, gun) in [
        ("robo-dog", Weapon::Shotgun),
        ("rover", Weapon::EnergyRifle),
        ("mini-tank", Weapon::RailGun),
    ] {
        let mut session = solo(loadout(name).unwrap());
        settle(&mut session);
        assert!(session.local().alive(), "{name}");
        let (start, heading) = (session.local().feet(), session.local().heading());
        for _ in 0..120 {
            drive(
                &mut session,
                Intent {
                    forward: 1,
                    yaw: heading,
                    ..Intent::IDLE
                },
            );
        }
        let moved = across(start, session.local().feet());
        assert!(moved > 300.0, "{name}: {moved}");
        let mut shots = 0;
        for _ in 0..180 {
            let yaw = session.local().heading();
            drive(
                &mut session,
                Intent {
                    fire: true,
                    yaw,
                    ..Intent::IDLE
                },
            );
            shots += session
                .events
                .iter()
                .filter(
                    |event| matches!(event, Event::Fired { slot: 0, weapon, .. } if *weapon == gun),
                )
                .count();
        }
        assert!(shots >= 2, "{name}: {shots} shots of {gun:?}");
    }
}

/// Bots play the robo-dog and the mini-tank as they play the armed car (the rover) on every
/// map: they cover ground and shoot, and the match replays exactly.
#[test]
fn bots_play_the_dog_and_the_mini_tank_on_the_yard() {
    for name in ["robo-dog", "mini-tank"] {
        let build = shooter_sim::robot::loadout(name).unwrap();
        bots_play_as(MapId::GreyboxYard, build, 30);
    }
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
    // On steering servos (shooter#170) a turn rolls ahead, and backing up turns the other way.
    assert_eq!(shooter_sim::bots::steered(0, -1), (1, -1));
    assert_eq!(shooter_sim::bots::steered(1, 1), (1, 1));
    assert_eq!(shooter_sim::bots::steered(-1, -1), (-1, 1));
    assert_eq!(shooter_sim::bots::steered(-1, 0), (-1, 0));
}

/// The steer-car's two steering servos (shooter#170).
const STEERING: [shooter_sim::robot::PartId; 2] =
    [shooter_sim::robot::PartId(3), shooter_sim::robot::PartId(5)];

fn steer_car() -> Session {
    let mut session = solo(shooter_sim::robot::STEER_CAR);
    settle(&mut session);
    session
}

/// Each steering servo's angle (radians) now.
fn servos(session: &Session) -> [f32; 2] {
    STEERING.map(|part| session.mechanics.joint(0, part).unwrap().0)
}

/// `ticks` of `intent`: how far the robot turned (binary yaw, unwrapped) and moved.
fn hold(session: &mut Session, intent: Intent, ticks: usize) -> (i32, f32) {
    let start = session.local().feet();
    let mut heading = session.local().heading();
    let mut turned = 0;
    for _ in 0..ticks {
        drive(session, intent);
        let now = session.local().heading();
        turned += turn(heading, now);
        heading = now;
    }
    (turned, across(start, session.local().feet()))
}

fn keys(forward: i8, strafe: i8) -> Intent {
    Intent {
        forward,
        strafe,
        ..Intent::IDLE
    }
}

/// The steer-car rolls on all four wheels and turns on its steering servos alone (shooter#170):
/// W with the camera sweeping round rolls it straight with its servos centred; W with D turns
/// its rear wheels out of the turn and the rolling car right, with A left; let go of D and the
/// servos come back to straight ahead and it rolls straight again. Its preset never turns a
/// wheel faster on one side than the other ([`bindings`](shooter_sim::bindings) tests that).
#[test]
fn a_steered_car_turns_on_its_steering_servos() {
    let mut session = steer_car();
    let start = session.local().heading();
    let (mut turned, mut heading) = (0, start);
    for tick in 0..90u16 {
        drive(
            &mut session,
            Intent {
                forward: 1,
                yaw: tick.wrapping_mul(700),
                ..Intent::IDLE
            },
        );
        let now = session.local().heading();
        turned += turn(heading, now);
        heading = now;
    }
    assert!(turned.abs() < 1_000, "the camera steered it: {turned}");
    assert!(
        servos(&session).iter().all(|angle| angle.abs() < 0.1),
        "{:?}",
        servos(&session)
    );
    let (right, moved) = hold(&mut session, keys(1, 1), 60);
    assert!(right < -3_000 && moved > 150.0, "{right} {moved}");
    let [left_servo, right_servo] = servos(&session);
    assert!(
        left_servo > 0.1 && right_servo > 0.1,
        "{left_servo} {right_servo}"
    );
    // Let go: centred again, rolling (almost) straight.
    hold(&mut session, keys(1, 0), 45);
    assert!(
        servos(&session).iter().all(|angle| angle.abs() < 0.1),
        "{:?}",
        servos(&session)
    );
    let (straight, _) = hold(&mut session, keys(1, 0), 30);
    assert!(straight.abs() < 600, "{straight}");
    let (left, _) = hold(&mut session, keys(1, -1), 60);
    assert!(left > 3_000, "{left}");
    assert!(
        servos(&session).iter().all(|angle| *angle < -0.1),
        "{:?}",
        servos(&session)
    );
}

/// Backing up the steer-car rolls straight with its servos centred. Starting back with D, it
/// holds its servos as for a right turn, so the car's nose swings left as a car's does in
/// reverse. (Backing up at full speed its rear wheels lead their kingpins, and their tyres turn
/// one servo back toward straight: it hardly turns then, shooter#170.) Standing, D turns its
/// servos and not the car (no chassis yaw, no free torque).
#[test]
fn a_steered_car_backs_up_and_stands_as_a_car() {
    let mut session = steer_car();
    let (turned, moved) = hold(&mut session, keys(-1, 0), 90);
    assert!(moved > 300.0 && turned.abs() < 1_000, "{turned} {moved}");
    assert!(
        servos(&session).iter().all(|angle| angle.abs() < 0.15),
        "{:?}",
        servos(&session)
    );
    let mut session = steer_car();
    let (turned, moved) = hold(&mut session, keys(-1, 1), 60);
    assert!(turned > 2_000 && moved > 150.0, "{turned} {moved}");

    let mut session = steer_car();
    let (turned, moved) = hold(&mut session, keys(0, 1), 60);
    assert!(moved < 20.0 && turned.abs() < 1_000, "{turned} {moved}");
    assert!(
        servos(&session).iter().all(|angle| *angle > 0.15),
        "{:?}",
        servos(&session)
    );
}

/// Without electricity (and no fuel for its generator) the steer-car's servos and wheels stop:
/// D turns nothing, W rolls nothing.
#[test]
fn without_electricity_a_steered_car_neither_steers_nor_rolls() {
    let mut session = steer_car();
    // Its generator has no fuel to make more.
    session.fighters[0].energy = 0;
    session.fighters[0].fuel = 0;
    let (turned, moved) = hold(&mut session, keys(1, 1), 90);
    assert!(session.fighters[0].flow.stalled);
    assert!(moved < 10.0 && turned.abs() < 500, "{turned} {moved}");
    assert!(
        servos(&session).iter().all(|angle| angle.abs() < 0.1),
        "{:?}",
        servos(&session)
    );
}

/// Bots drive the steer-car (rolling to turn, its reverse turning the other way) as they drive
/// the rover: they cover ground and shoot, and the match replays exactly.
#[test]
fn bots_play_the_steer_car_on_the_yard() {
    bots_play_as(MapId::GreyboxYard, shooter_sim::robot::STEER_CAR, 30);
}
