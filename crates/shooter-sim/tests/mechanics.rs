//! Mechanical robots (shooter#73, Atlas D2.i70.5) on physics-engine: the fixtures roll, strafe,
//! stall, aim one- and two-axis turrets, tip from where their mass is, and walk on the biped
//! and quadruped presets while badly mounted legs stay constrained; motors draw from the shared
//! pool first come, first served; part poses and per-part ray hits follow the bodies; and a
//! world restored from its encoded state continues bit for bit (host, prediction, replay).
// The platform's floats only check results here; the simulation uses libm (clippy.toml).
#![allow(clippy::disallowed_methods)]

use shooter_sim::energy::MILLI;
use shooter_sim::mechanics::fixtures::{
    self, BARREL, HINGE_PART, ROTOR_PART, WHEELS, binary, floor, walled, wheels,
};
use shooter_sim::mechanics::{Command, Controls, Gait, GaitChoice, Layout, Mechanics, State};
use shooter_sim::physics::RigidBody;
use shooter_sim::physics::articulated::Pose;
use shooter_sim::robot::{Assembly, PartId};
use shooter_sim::wire::{decode_controls, decode_mechanics, encode_controls, encode_mechanics};

/// The car's wheel speed at 6 rad/s on its 18 cm wheel spheres, in units a second.
const RATE: i16 = 6_000;
const GROUND_SPEED: f32 = 108.0;

fn world(statics: &[RigidBody], builds: &[Assembly]) -> Mechanics {
    let mut mechanics = Mechanics::new(statics).unwrap();
    for (index, build) in builds.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let at = [0.0, 0.0, 400.0 * index as f32];
        mechanics.spawn(*build, at, 0).unwrap();
    }
    mechanics
}

fn run(mechanics: &mut Mechanics, ticks: u32, controls: impl Fn(u32, &Layout) -> Controls) {
    for tick in 0..ticks {
        let all: Vec<Controls> = mechanics
            .robots()
            .iter()
            .map(|robot| controls(tick, &robot.layout))
            .collect();
        mechanics.step(&all).unwrap();
    }
}

/// The world direction of the robot's authored +Y (its up).
fn up(pose: Pose) -> [f32; 3] {
    let [x, y, z, w] = pose.orientation;
    [
        2.0 * (x * y - z * w),
        1.0 - 2.0 * (x * x + z * z),
        2.0 * (y * z + x * w),
    ]
}

/// The heading of a pose's +X about +Y (radians; positive turns +X toward −Z).
fn heading(pose: Pose) -> f32 {
    let [x, y, z, w] = pose.orientation;
    let forward = [
        1.0 - 2.0 * (y * y + z * z),
        2.0 * (x * y + z * w),
        2.0 * (x * z - y * w),
    ];
    (-forward[2]).atan2(forward[0])
}

fn root(mechanics: &Mechanics) -> Pose {
    mechanics.root(0).unwrap()
}

#[test]
fn normal_rolling_drives_along_the_wheels_and_a_mismatched_drive_spins() {
    let mut car = world(&floor(), &[fixtures::car()]);
    let start = root(&car).position;
    run(&mut car, 120, |_, _| wheels(RATE, -RATE));
    let motion = car.root_motion(0).unwrap();
    let pose = root(&car);
    assert!(
        (motion.velocity[0] - GROUND_SPEED).abs() < 0.1 * GROUND_SPEED,
        "{motion:?}"
    );
    assert!(motion.velocity[2].abs() < 5.0, "{motion:?}");
    assert!((pose.position[2] - start[2]).abs() < 5.0, "{pose:?}");
    assert!(up(pose)[1] > 0.99, "{pose:?}");
    // The wheels run at their command, below their torque.
    for actuator in &car.robots()[0].actuators {
        assert!((actuator.rate.abs() - 6.0).abs() < 0.1, "{actuator:?}");
        assert!(!actuator.stalled, "{actuator:?}");
    }

    // The same rate on every wheel turns the two sides opposite ways: it spins in place.
    let mut spinner = world(&floor(), &[fixtures::car()]);
    run(&mut spinner, 120, |_, _| wheels(RATE, RATE));
    let pose = root(&spinner);
    let moved = (pose.position[0] - start[0]).hypot(pose.position[2] - start[2]);
    assert!(moved < 15.0, "{pose:?}");
    assert!(heading(pose).abs() > 0.2, "{}", heading(pose));
    assert!(up(pose)[1] > 0.99, "{pose:?}");
}

#[test]
fn side_mounted_wheels_strafe() {
    let mut strafer = world(&floor(), &[fixtures::strafer()]);
    let start = root(&strafer).position;
    run(&mut strafer, 120, |_, _| wheels(RATE, -RATE));
    let motion = strafer.root_motion(0).unwrap();
    let pose = root(&strafer);
    assert!(
        (motion.velocity[2] - GROUND_SPEED).abs() < 0.1 * GROUND_SPEED,
        "{motion:?}"
    );
    assert!(motion.velocity[0].abs() < 5.0, "{motion:?}");
    assert!((pose.position[0] - start[0]).abs() < 5.0, "{pose:?}");
    assert!(heading(pose).abs() < 0.05, "{pose:?}");
}

#[test]
fn a_torque_limited_drive_stalls_against_a_wall() {
    let mut car = world(&walled(80), &[fixtures::car()]);
    run(&mut car, 180, |_, _| wheels(RATE, -RATE));
    let motion = car.root_motion(0).unwrap();
    let speed = motion.velocity.iter().map(|v| v * v).sum::<f32>().sqrt();
    assert!(speed < 2.0, "{motion:?}");
    for actuator in &car.robots()[0].actuators {
        assert!(actuator.saturated && actuator.stalled, "{actuator:?}");
        assert!(actuator.rate.abs() < 1.0, "{actuator:?}");
        // At its 30 N·m.
        assert!((actuator.torque.abs() - 30.0).abs() < 1.0, "{actuator:?}");
        assert!(actuator.powered, "a stalled motor still draws");
    }
}

fn actuator(mechanics: &Mechanics, part: PartId) -> shooter_sim::mechanics::ActuatorState {
    mechanics.robots()[0]
        .actuators
        .iter()
        .find(|actuator| actuator.part == part)
        .copied()
        .unwrap()
}

#[test]
fn a_one_axis_turret_servo_turns_to_its_angle_and_holds() {
    for target in [1.0, -2.0] {
        let mut tank = world(&floor(), &[fixtures::turret()]);
        let start = root(&tank).position;
        run(&mut tank, 120, |_, _| {
            wheels(0, 0).with(ROTOR_PART, Command::Angle(binary(target)))
        });
        let rotor = actuator(&tank, ROTOR_PART);
        assert!((f64::from(rotor.angle) - target).abs() < 0.02, "{rotor:?}");
        assert!(rotor.rate.abs() < 0.05, "{rotor:?}");
        let pose = root(&tank);
        let moved = (pose.position[0] - start[0]).hypot(pose.position[2] - start[2]);
        assert!(moved < 2.0, "the braked chassis moved: {pose:?}");
        // The barrel turned with the rotor about the robot's up.
        let barrel = tank
            .part_poses(0)
            .into_iter()
            .find(|pose| pose.segment.part == BARREL)
            .unwrap();
        let turned = heading(barrel.pose) - heading(pose);
        let wrapped = (turned + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
            - std::f32::consts::PI;
        #[allow(clippy::cast_possible_truncation)]
        let expected = target as f32;
        assert!(
            (wrapped.abs() - expected.abs()).abs() < 0.03,
            "{wrapped} {target}"
        );
    }
}

#[test]
fn paired_turret_axes_turn_together_within_the_hinge_limits() {
    // A rotor turning continuously while the hinge raises the barrel.
    let mut tank = world(&floor(), &[fixtures::paired_turret()]);
    run(&mut tank, 120, |_, _| {
        wheels(0, 0)
            .with(ROTOR_PART, Command::Rate(1_500))
            .with(HINGE_PART, Command::Angle(binary(0.5)))
    });
    let (rotor, hinge) = (actuator(&tank, ROTOR_PART), actuator(&tank, HINGE_PART));
    assert!((rotor.rate - 1.5).abs() < 0.15, "{rotor:?}");
    assert!((hinge.angle - 0.5).abs() < 0.02, "{hinge:?}");
    assert!(up(root(&tank))[1] > 0.99);

    // Commanded past its limit, the hinge stops at a quarter turn.
    let mut tank = world(&floor(), &[fixtures::paired_turret()]);
    let mut highest = 0.0f32;
    for _ in 0..120 {
        tank.step(&[wheels(0, 0).with(HINGE_PART, Command::Angle(binary(2.5)))])
            .unwrap();
        highest = highest.max(actuator(&tank, HINGE_PART).angle);
    }
    let hinge = actuator(&tank, HINGE_PART);
    assert!(
        (hinge.angle - std::f32::consts::FRAC_PI_2).abs() < 0.03,
        "{hinge:?}"
    );
    assert!(highest < std::f32::consts::FRAC_PI_2 + 0.05, "{highest}");
}

#[test]
fn mass_placement_tips_a_lopsided_robot_and_not_a_balanced_one() {
    let mut lopsided = world(&floor(), &[fixtures::lopsided()]);
    run(&mut lopsided, 120, |_, _| Controls::IDLE);
    let up_lopsided = up(root(&lopsided));
    // Its left is −Z: it lies over on that side.
    assert!(up_lopsided[1] < 0.9, "{up_lopsided:?}");
    assert!(up_lopsided[2] < -0.4, "{up_lopsided:?}");

    let mut balanced = world(&floor(), &[fixtures::balanced()]);
    run(&mut balanced, 120, |_, _| Controls::IDLE);
    assert!(up(root(&balanced))[1] > 0.999, "{:?}", root(&balanced));

    // The centre of mass is the mass-weighted one: the lopsided robot's single rigid body has
    // its centre off the chassis toward the battery.
    let fresh = world(&floor(), &[fixtures::lopsided()]);
    let centre = fresh.root_motion(0).unwrap().pose.position;
    let chassis = fresh
        .part_poses(0)
        .into_iter()
        .find(|pose| pose.segment.part == PartId(0))
        .unwrap();
    assert!(centre[2] < chassis.pose.position[2] - 20.0, "{centre:?}");
}

#[test]
fn the_gait_presets_walk_correctly_mounted_legs() {
    let dog = Layout::of(&fixtures::dog(false));
    assert_eq!(dog.suggested_gait(), Some(Gait::Quadruped));
    assert_eq!(
        Layout::of(&fixtures::biped()).suggested_gait(),
        Some(Gait::Biped)
    );
    assert_eq!(Layout::of(&fixtures::car()).suggested_gait(), None);

    // The robo-dog crawls forward on the suggested quadruped preset and stays up.
    let mut walker = world(&floor(), &[fixtures::dog(false)]);
    let start = root(&walker).position;
    run(&mut walker, 270, |_, layout| {
        Controls::IDLE.walk(layout, 100, 0)
    });
    let pose = root(&walker);
    assert!(pose.position[0] - start[0] > 150.0, "{pose:?}");
    assert!(up(pose)[1] > 0.95, "{pose:?}");

    // The biped steps forward on the suggested biped preset and stays up.
    let mut biped = world(&floor(), &[fixtures::biped()]);
    let start = root(&biped).position;
    run(&mut biped, 570, |_, layout| {
        Controls::IDLE.walk(layout, 100, 0)
    });
    let pose = root(&biped);
    assert!(pose.position[0] - start[0] > 120.0, "{pose:?}");
    assert!(up(pose)[1] > 0.95, "{pose:?}");

    // Standing (a zero stride) holds the legs straight.
    let mut stander = world(&floor(), &[fixtures::dog(false)]);
    let start = root(&stander).position;
    run(&mut stander, 120, |_, layout| {
        Controls::IDLE.walk(layout, 0, 0)
    });
    let pose = root(&stander);
    assert!((pose.position[0] - start[0]).abs() < 10.0, "{pose:?}");
    assert!(up(pose)[1] > 0.99, "{pose:?}");
}

#[test]
fn badly_configured_legs_stay_physically_constrained() {
    // The dog's legs a quarter turn round swing sideways: no preset corrects them.
    let mut crooked = world(&floor(), &[fixtures::dog(true)]);
    let start = root(&crooked).position;
    let mut last = start;
    let mut fastest = 0.0f32;
    for _ in 0..270 {
        let controls = Controls::IDLE.walk(&crooked.robots()[0].layout, 100, 0);
        crooked.step(&[controls]).unwrap();
        let now = root(&crooked).position;
        fastest = fastest.max((now[0] - last[0]).hypot(now[2] - last[2]));
        last = now;
    }
    assert!(last[0] - start[0] < 40.0, "{last:?}");
    // Nothing teleports: no tick moves the robot faster than 2 m/s.
    assert!(fastest < 200.0 / 60.0, "{fastest}");

    // Legs on the roof step in the air and carry it nowhere.
    let mut flailing = world(&floor(), &[fixtures::upside_down_legs()]);
    let start = root(&flailing).position;
    run(&mut flailing, 180, |_, layout| {
        Controls::IDLE.walk(layout, 100, 0)
    });
    let pose = root(&flailing);
    assert!(
        (pose.position[0] - start[0]).hypot(pose.position[2] - start[2]) < 10.0,
        "{pose:?}"
    );

    // An override applies whatever the legs are: the dog on the biped preset moves only as its
    // contacts let it.
    let mut overridden = world(&floor(), &[fixtures::dog(false)]);
    run(&mut overridden, 120, |_, layout| Controls {
        gait: GaitChoice::Preset(Gait::Biped),
        ..Controls::IDLE.walk(layout, 100, 0)
    });
    let pose = root(&overridden);
    assert!(
        pose.position.iter().all(|value| value.is_finite()),
        "{pose:?}"
    );
}

#[test]
fn motors_draw_by_their_work_first_come_first_served() {
    use shooter_sim::mechanics::{draw, load};
    let car = world(&floor(), &[fixtures::car()]);
    let robot = &car.robots()[0];
    let capacity = robot.pool.capacity;
    assert_eq!(robot.pool.burn, 0, "the car has no generator");
    let mass = load(&robot.build, |_| true);
    let wheel = &robot.layout.actuators[0];
    // A wheel draws 2 a second at its top rate (20 rad/s since shooter#58) under 200 kg: at
    // 6 rad/s, three tenths of its top rate, under the car's 240 kg, 2 × 0.3 × 240 / 200 a
    // second, 12 milli-units a tick.
    assert_eq!(mass, 240);
    let half = draw(wheel, Command::Rate(RATE), mass);
    assert_eq!(half, 12);
    // Faster or heavier costs more; braked, idle or a stride (no leg) costs nothing.
    assert!(draw(wheel, Command::Rate(2 * RATE), mass) > half);
    assert!(draw(wheel, Command::Rate(RATE), 2 * mass) > half);
    for rest in [Command::Rate(0), Command::Idle, Command::Stride(100)] {
        assert_eq!(draw(wheel, rest, mass), 0, "{rest:?}");
    }
    let with_charge = |charge: u32| {
        let mut state = car.state();
        state.robots[0].charge = charge;
        Mechanics::restore(&floor(), &state).unwrap()
    };
    // Idle motors draw nothing, and nothing recharges a robot without a generator.
    let mut idle = with_charge(capacity - MILLI);
    for _ in 0..60 {
        idle.step(&[Controls::IDLE]).unwrap();
    }
    assert_eq!(idle.robots()[0].charge, capacity - MILLI);
    let mut driven = with_charge(capacity - MILLI);
    driven.step(&[wheels(RATE, -RATE)]).unwrap();
    assert_eq!(driven.robots()[0].charge, capacity - MILLI - 4 * half);

    // With enough for one wheel, the first in part-ID order is powered and the rest are off
    // that tick.
    let mut short = with_charge(half + 5);
    short.step(&[wheels(RATE, -RATE)]).unwrap();
    let powered: Vec<bool> = short.robots()[0]
        .actuators
        .iter()
        .map(|actuator| actuator.powered)
        .collect();
    assert_eq!(powered, [true, false, false, false]);
    assert_eq!(short.robots()[0].charge, 5);
    // An empty pool powers nothing, and with no generator it stays empty; braking still holds.
    let mut drained = with_charge(0);
    for _ in 0..2 {
        drained.step(&[wheels(RATE, -RATE)]).unwrap();
        assert!(drained.robots()[0].actuators.iter().all(|a| !a.powered));
    }
    drained.step(&[wheels(0, 0)]).unwrap();
    assert!(drained.robots()[0].actuators.iter().all(|a| a.powered));
    assert_eq!(drained.robots()[0].charge, 0);
}

#[test]
fn part_poses_and_ray_hits_name_the_part() {
    let mut tank = world(&floor(), &[fixtures::turret()]);
    run(&mut tank, 30, |_, _| wheels(0, 0));
    let layout = &tank.robots()[0].layout;
    let poses = tank.part_poses(0);
    assert_eq!(poses.len(), layout.segments.len());
    assert!(
        poses
            .windows(2)
            .all(|pair| pair[0].segment.part <= pair[1].segment.part)
    );
    let pose_of = |part: PartId| {
        poses
            .iter()
            .find(|pose| pose.segment.part == part)
            .unwrap()
            .pose
            .position
    };
    // A ray along +X at the barrel's height hits the barrel; one across a wheel's centre hits
    // that wheel; one beside the robot hits nothing of it.
    let barrel = pose_of(BARREL);
    let hit = tank
        .cast_ray([barrel[0] - 300.0, barrel[1], barrel[2]], [600.0, 0.0, 0.0])
        .unwrap();
    assert_eq!((hit.robot, hit.part), (0, BARREL));
    let wheel = pose_of(WHEELS[0]);
    let hit = tank
        .cast_ray([wheel[0], wheel[1], wheel[2] - 200.0], [0.0, 0.0, 200.0])
        .unwrap();
    assert_eq!((hit.robot, hit.part), (0, WHEELS[0]));
    assert_eq!(
        tank.cast_ray([barrel[0] + 300.0, 400.0, barrel[2]], [0.0, -350.0, 0.0]),
        None
    );
    // From above, the barrel is the first part.
    let hit = tank
        .cast_ray([barrel[0], 400.0, barrel[2]], [0.0, -500.0, 0.0])
        .unwrap();
    assert_eq!(hit.part, BARREL);
}

fn busy_controls(tick: u32, layout: &Layout) -> Controls {
    if layout.suggested_gait().is_some() {
        return Controls::IDLE.walk(layout, 100, if tick > 60 { 30 } else { 0 });
    }
    let rate = i16::try_from((tick % 90) * 60).unwrap();
    wheels(rate, -rate / 2)
        .with(ROTOR_PART, Command::Rate(1_000))
        .with(HINGE_PART, Command::Angle(binary(0.3)))
}

fn all(mechanics: &Mechanics, tick: u32) -> Vec<Controls> {
    mechanics
        .robots()
        .iter()
        .map(|robot| busy_controls(tick, &robot.layout))
        .collect()
}

#[test]
fn a_restored_state_continues_bit_for_bit() {
    // Three robots doing everything at once: the dog walking and turning, the paired turret
    // aiming while it rolls, the strafer drifting.
    let builds = [
        fixtures::dog(false),
        fixtures::paired_turret(),
        fixtures::strafer(),
    ];
    let mut host = world(&floor(), &builds);
    for tick in 0..40 {
        host.step(&all(&host, tick)).unwrap();
    }
    // The client receives the encoded state and resumes from it (prediction's rollback).
    let state = decode_mechanics(&encode_mechanics(&host.state())).unwrap();
    assert_eq!(state, host.state());
    let mut client = Mechanics::restore(&floor(), &state).unwrap();
    for tick in 40..140 {
        // The controls travel encoded too.
        let sent: Vec<Controls> = all(&host, tick)
            .iter()
            .map(|controls| decode_controls(&encode_controls(controls)).unwrap())
            .collect();
        host.step(&all(&host, tick)).unwrap();
        client.step(&sent).unwrap();
        assert_eq!(
            encode_mechanics(&client.state()),
            encode_mechanics(&host.state()),
            "tick {tick}"
        );
    }

    // A replay from the spawn with the same inputs is the same run.
    let mut replay = world(&floor(), &builds);
    for tick in 0..140 {
        replay.step(&all(&replay, tick)).unwrap();
    }
    assert_eq!(
        encode_mechanics(&replay.state()),
        encode_mechanics(&host.state())
    );
}

#[test]
fn mechanics_on_the_wire_are_validated() {
    let mechanics = world(&floor(), &[fixtures::car()]);
    let state = mechanics.state();
    let mut bytes = encode_mechanics(&state);
    bytes.push(0);
    assert!(decode_mechanics(&bytes).is_err(), "trailing bytes");
    // A robot whose bodies do not match its build.
    let mut wrong = state.clone();
    wrong.robots[0].bodies.pop();
    assert!(decode_mechanics(&encode_mechanics(&wrong)).is_err());
    // A charge above its pool.
    let mut rich = state.clone();
    rich.robots[0].charge = u32::MAX;
    assert!(decode_mechanics(&encode_mechanics(&rich)).is_err());
    let empty = State::default();
    assert_eq!(decode_mechanics(&encode_mechanics(&empty)).unwrap(), empty);

    let controls = Controls {
        gait: GaitChoice::Unison,
        ..wheels(-RATE, RATE).with(PartId(31), Command::Stride(-100))
    };
    assert_eq!(
        decode_controls(&encode_controls(&controls)).unwrap(),
        controls
    );
    let every = Controls {
        commands: [Command::Angle(-7); shooter_sim::mechanics::PART_IDS],
        ..Controls::IDLE
    };
    assert_eq!(decode_controls(&encode_controls(&every)).unwrap(), every);
    // A stride past 100 %, an unknown command, a part twice and an unknown gait are malformed;
    // any part ID may be commanded.
    assert!(decode_controls(&[0, 1, 0, 2, 3, 101, 0]).is_err());
    assert!(decode_controls(&[0, 1, 0, 2, 5, 0, 0]).is_err());
    // A joint told to track carries no value (shooter#132).
    assert_eq!(
        decode_controls(&[0, 1, 0, 2, 4, 0, 0]).unwrap().commands[2],
        Command::Track
    );
    assert!(decode_controls(&[0, 1, 0, 2, 4, 1, 0]).is_err());
    assert_eq!(
        decode_controls(&[0, 1, 0, 200, 1, 0, 1]).unwrap().commands[200],
        Command::Rate(256)
    );
    assert!(decode_controls(&[0, 2, 0, 2, 1, 0, 0, 2, 1, 0, 0]).is_err());
    assert!(decode_controls(&[4, 0, 0]).is_err());
}

/// The builder's sockets: every socket of every part where the part places it, a mounted part's
/// own mount face left out (it rests on its parent).
#[test]
fn sockets_sit_on_their_faces_in_the_robot_frame() {
    use shooter_sim::mechanics::sockets;
    use shooter_sim::robot::parts::{BEAM_SHORT, CHASSIS_S, FUEL_TANK};
    use shooter_sim::robot::{BETA, Candidate, Face, RawMount, RawPart, STANDARD};
    // The standard build: only its small chassis (28) and its fuel tank (the one on top, where
    // the generator sits) have sockets, the chassis' front's 8 face +X, a cell's half (10 units)
    // ahead of its centre.
    let spots = sockets(&STANDARD);
    assert_eq!(
        spots.len(),
        usize::from(CHASSIS_S.socket_count() + FUEL_TANK.socket_count())
    );
    let front: Vec<_> = spots.iter().filter(|spot| spot.normal[0] > 0.99).collect();
    assert_eq!(front.len(), 8);
    assert!(
        front
            .iter()
            .all(|spot| (spot.centre[0] - 10.0).abs() < 1e-3)
    );
    let top = CHASSIS_S.socket(Face::PosZ, 0, 0).unwrap();
    let up = spots.iter().find(|spot| spot.socket == top).unwrap();
    assert!(
        up.normal[1] > 0.99 && (up.centre[1] - 40.0).abs() < 1e-3,
        "{up:?}"
    );
    // A short beam on the chassis' top: its 6 non-mount faces' sockets (10), not its mount
    // face's, its far end 40 units above the chassis' top.
    let socket = |face, u, v| CHASSIS_S.socket(face, u, v).unwrap().0;
    let build = BETA
        .validate(&Candidate {
            parts: vec![
                RawPart {
                    id: 0,
                    kind: CHASSIS_S.id.0,
                    ..RawPart::default()
                },
                RawPart {
                    id: 1,
                    kind: BEAM_SHORT.id.0,
                    mount: Some(RawMount {
                        parent: 0,
                        socket: socket(Face::PosZ, 0, 0),
                        ..RawMount::default()
                    }),
                    modification: None,
                },
            ],
        })
        .unwrap();
    let spots = sockets(&build);
    let beam: Vec<_> = spots.iter().filter(|spot| spot.part == PartId(1)).collect();
    assert_eq!(
        beam.len(),
        usize::from(BEAM_SHORT.socket_count()) - 1,
        "every face but the mount face's one cell"
    );
    assert!(
        beam.iter()
            .any(|spot| spot.normal[1] > 0.99 && (spot.centre[1] - 80.0).abs() < 1e-3)
    );
}
