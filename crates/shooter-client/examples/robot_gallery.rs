//! The robots side by side in the greybox arena, drawn offscreen through the clients' own path
//! (`shooter_client::robots`) at a third-person gameplay scale, for review:
//!
//! ```text
//! scripts/fetch-robot-kit.sh
//! cargo run -p shooter-client --release --example robot_gallery -- <directory>
//! ```
//!
//! writes `loadouts.ppm` (the five named loadouts and the mechanics fixtures: the biped, the
//! robo-dog and the paired turret with its rotor and hinge turned), `poses.ppm` (the standard
//! build idle, running, strafing, airborne, grappling, sliding, firing, swinging the blade, dead,
//! and with its blade shot off), `weapons.ppm` (the weapons' states and the mods: the rifle
//! held with the blade away, the rifle firing, a blade swing, the rocket pod firing, the rail gun
//! recharging, the skirmisher's grapple hooked with its mods, the marksman's long barrel and
//! overcharger, the pyro's flamethrower firing beside its long-barrelled laser), `close.ppm` (the
//! heavy build close up), `mods.ppm` (the skirmisher and marksman close up), `locomotion.ppm`
//! (the wheeled fixtures with their wheels where the simulation turned them, the mini-tank on its
//! tracks, a car rolling at a run, a damaged wheel and a lost one, a damaged track),
//! `locomotion-close.ppm` (the car and the mini-tank close up) and `locomotion-damage.ppm` (the
//! damaged and lost wheel and the damaged track close up).

use render_engine::glam::{Mat4, Vec3};
use shooter_client::gpu::Gpu;
use shooter_client::robots::{self, PartStates, RobotState, RobotView, Robots};
use shooter_sim::mechanics::fixtures;
use shooter_sim::robot::{HEAVY, MARKSMAN, PYRO, PartId, SKIRMISHER, STANDARD};
use shooter_sim::{Assembly, MapId, Weapon};

const SIZE: (u32, u32) = (1600, 900);

fn standing(at: Vec3, forward: Vec3) -> RobotState {
    RobotState {
        center: at + Vec3::Y * robots::pose::HALF_HEIGHT,
        forward,
        velocity: Vec3::ZERO,
        sliding: false,
        hook: None,
        since_fired: f32::INFINITY,
        fired: Weapon::EnergyRifle,
        shots: 0,
        held: Weapon::EnergyRifle,
        energy: 1.0,
        swing: None,
        dead_for: None,
    }
}

fn shoot(
    gpu: &mut Gpu,
    robots: &mut Robots,
    views: &[RobotView],
    eye: Vec3,
    target: Vec3,
    path: &std::path::Path,
) -> Result<(), String> {
    // Two updates: the first lays the builds out, the second poses them (phase 0 → as given).
    robots.update(&mut gpu.world, views, 0.0)?;
    let drawn = robots.update(&mut gpu.world, views, 0.0)?;
    for robot in drawn {
        println!("slot {}: {}", robot.slot, robot.label);
    }
    let scene = shooter_client::scene::frame(eye, (target - eye).normalize(), Vec::new());
    gpu.render(&scene, 0)?;
    let (width, height, pixels) = gpu.read_frame()?;
    shooter_client::write_ppm(path, width, height, &pixels)
}

fn main() -> Result<(), String> {
    let out = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "target/robot-gallery".into()),
    );
    std::fs::create_dir_all(&out).map_err(|error| error.to_string())?;
    let mut gpu = Gpu::offscreen(SIZE.0, SIZE.1)?;
    // The greybox: the gallery's line stands on its open floor.
    let arena = MapId::Greybox.arena();
    shooter_client::scene::build(&mut gpu.world, &arena, None);
    let kit = robots::load::read()?;
    let mut robots = Robots::new(&mut gpu.world, kit)?;

    // A line across the open floor, the robots facing the camera (−Z).
    let origin = Vec3::new(6.5, 0.0, -6.5);
    let toward = -Vec3::Z;
    let at = |index: usize, spacing: f32| {
        #[allow(clippy::cast_precision_loss)]
        let x = index as f32 * spacing;
        origin - Vec3::X * x
    };

    let fixtures_turret = fixtures::paired_turret();
    let lineup: [(Assembly, &[(PartId, f32)]); 8] = [
        (STANDARD, &[]),
        (HEAVY, &[]),
        (SKIRMISHER, &[]),
        (MARKSMAN, &[]),
        (PYRO, &[]),
        (fixtures::biped(), &[]),
        (fixtures::dog(false), &[]),
        (
            fixtures_turret,
            &[(fixtures::ROTOR_PART, 0.6), (fixtures::HINGE_PART, -0.5)],
        ),
    ];
    let views: Vec<RobotView> = lineup
        .iter()
        .enumerate()
        .map(|(index, (build, joints))| RobotView {
            slot: u8::try_from(index).unwrap_or(0),
            build: *build,
            state: standing(at(index, 2.0), toward),
            glow: 0.0,
            parts: PartStates {
                joints,
                ..PartStates::default()
            },
        })
        .collect();
    let middle = at(3, 2.0) - Vec3::X + Vec3::Y * 0.9;
    shoot(
        &mut gpu,
        &mut robots,
        &views,
        middle + Vec3::new(0.0, 0.9, -6.6),
        middle,
        &out.join("loadouts.ppm"),
    )?;

    let run = Vec3::new(0.0, 0.0, -6.0);
    let poses: Vec<(RobotState, f32)> = {
        let base = |index| standing(at(index, 1.45), toward);
        vec![
            (base(0), 0.0),
            (
                RobotState {
                    velocity: run,
                    ..base(1)
                },
                0.25,
            ),
            (
                RobotState {
                    velocity: Vec3::new(6.0, 0.0, 0.0),
                    ..base(2)
                },
                0.6,
            ),
            (
                RobotState {
                    velocity: Vec3::new(0.0, 5.0, 0.0),
                    center: base(3).center + Vec3::Y * 0.5,
                    ..base(3)
                },
                0.0,
            ),
            (
                RobotState {
                    hook: Some(base(4).center + Vec3::new(0.0, 3.0, -2.0)),
                    held: Weapon::Grapple,
                    center: base(4).center + Vec3::Y * 0.5,
                    ..base(4)
                },
                0.0,
            ),
            (
                RobotState {
                    sliding: true,
                    ..base(5)
                },
                0.0,
            ),
            (
                RobotState {
                    since_fired: 0.06,
                    ..base(6)
                },
                0.0,
            ),
            (
                RobotState {
                    swing: Some(0.5),
                    held: Weapon::EnergyBlade,
                    ..base(7)
                },
                0.0,
            ),
            (
                RobotState {
                    dead_for: Some(3.0),
                    ..base(8)
                },
                0.0,
            ),
            (base(9), 0.0),
        ]
    };
    // The standard build's blade shot off, lying on the floor.
    let fallen = [(
        PartId(8),
        Mat4::from_translation(at(9, 1.45) + Vec3::new(-0.5, 0.12, -0.7))
            * Mat4::from_rotation_z(-std::f32::consts::FRAC_PI_2)
            * Mat4::from_translation(Vec3::new(0.0, 0.0, -0.4)),
    )];
    let views: Vec<RobotView> = poses
        .iter()
        .enumerate()
        .map(|(index, (state, _))| RobotView {
            slot: u8::try_from(index).unwrap_or(0),
            build: STANDARD,
            state: *state,
            glow: 0.0,
            parts: if index == 9 {
                PartStates {
                    detached: &fallen,
                    ..PartStates::default()
                }
            } else {
                PartStates::default()
            },
        })
        .collect();
    // The run and strafe gaits advanced `phase` cycles (the legs cycle at their fastest).
    robots.update(&mut gpu.world, &views, 0.0)?;
    for (view, (_, phase)) in views.iter().zip(&poses) {
        if *phase > 0.0 {
            let seconds = phase / robots::pose::MAX_CYCLES_PER_SECOND;
            robots.update(&mut gpu.world, std::slice::from_ref(view), seconds)?;
        }
    }
    let middle = at(4, 1.45) + Vec3::new(-0.725, 0.9, 0.0);
    shoot(
        &mut gpu,
        &mut robots,
        &views,
        middle + Vec3::new(0.0, 0.9, -7.0),
        middle,
        &out.join("poses.ppm"),
    )?;

    // The weapons' states, three-quarter on.
    let three_quarter = Vec3::new(-1.0, 0.0, -0.45).normalize();
    let weapons: Vec<(Assembly, RobotState)> = {
        let base = |index| standing(at(index, 1.3), three_quarter);
        vec![
            (STANDARD, base(0)),
            (
                STANDARD,
                RobotState {
                    since_fired: 0.07,
                    ..base(1)
                },
            ),
            (
                STANDARD,
                RobotState {
                    swing: Some(0.35),
                    ..base(2)
                },
            ),
            (
                HEAVY,
                RobotState {
                    held: Weapon::RocketLauncher,
                    fired: Weapon::RocketLauncher,
                    since_fired: 0.12,
                    shots: 1,
                    ..base(3)
                },
            ),
            (
                HEAVY,
                RobotState {
                    held: Weapon::RailGun,
                    fired: Weapon::RailGun,
                    since_fired: 0.75,
                    energy: 0.4,
                    ..base(4)
                },
            ),
            (
                SKIRMISHER,
                RobotState {
                    held: Weapon::Shotgun,
                    hook: Some(base(5).center + Vec3::new(0.7, 1.6, -1.6)),
                    ..base(5)
                },
            ),
            (
                MARKSMAN,
                RobotState {
                    held: Weapon::RailGun,
                    fired: Weapon::RailGun,
                    since_fired: 0.1,
                    ..base(6)
                },
            ),
            (
                PYRO,
                RobotState {
                    held: Weapon::Flamethrower,
                    fired: Weapon::Flamethrower,
                    since_fired: 0.05,
                    ..base(7)
                },
            ),
        ]
    };
    let views: Vec<RobotView> = weapons
        .iter()
        .enumerate()
        .map(|(index, (build, state))| RobotView {
            slot: u8::try_from(index).unwrap_or(0),
            build: *build,
            state: *state,
            glow: 0.0,
            parts: PartStates::default(),
        })
        .collect();
    let middle = at(3, 1.3) - Vec3::X * 0.65 + Vec3::Y * 1.2;
    shoot(
        &mut gpu,
        &mut robots,
        &views,
        middle + Vec3::new(0.0, 0.6, -5.0),
        middle,
        &out.join("weapons.ppm"),
    )?;

    let views = [
        RobotView {
            slot: 0,
            build: SKIRMISHER,
            state: standing(origin, Vec3::new(-0.5, 0.0, -1.0)),
            glow: 0.0,
            parts: PartStates::default(),
        },
        RobotView {
            slot: 1,
            build: MARKSMAN,
            state: standing(origin - Vec3::X * 1.6, Vec3::new(-0.5, 0.0, -1.0)),
            glow: 0.0,
            parts: PartStates::default(),
        },
    ];
    let middle = origin - Vec3::X * 0.8 + Vec3::Y * 1.1;
    shoot(
        &mut gpu,
        &mut robots,
        &views,
        middle + Vec3::new(-0.6, 0.3, -2.6),
        middle,
        &out.join("mods.ppm"),
    )?;

    let views = [RobotView {
        slot: 0,
        build: HEAVY,
        state: standing(origin, Vec3::new(-0.6, 0.0, -1.0)),
        glow: 0.0,
        parts: PartStates::default(),
    }];
    let middle = origin + Vec3::Y * 1.1;
    shoot(
        &mut gpu,
        &mut robots,
        &views,
        middle + Vec3::new(-1.2, 0.4, -2.8),
        middle,
        &out.join("close.ppm"),
    )?;

    locomotion(&mut gpu, &mut robots, origin, &out)
}

/// A tracked build (not a simulation fixture: where tracks should sit is shooter#58): the large
/// chassis on two tracks along its lower sides, the belts' bottoms flush with the hull's, a
/// rotor on the roof carrying the rocket pod pitched to aim ahead.
fn mini_tank() -> Assembly {
    use shooter_sim::robot::parts::{CHASSIS_L, ROCKET_LAUNCHER, ROTOR, TRACK};
    use shooter_sim::robot::{BETA, Candidate, Face, Orientation, PartType, RawMount, RawPart};
    let on = |id: u8, kind: &PartType, parent: u8, to: &PartType, face, yaw, pitch| RawPart {
        id,
        kind: kind.id.0,
        mount: Some(RawMount {
            parent,
            socket: to.socket(face, 0, 0).expect("a socket").0,
            yaw,
            pitch,
            roll: 0,
        }),
        modification: None,
    };
    let quarter = Orientation::QUARTER;
    let parts = vec![
        RawPart {
            id: 0,
            kind: CHASSIS_L.id.0,
            mount: None,
            modification: None,
        },
        on(1, &TRACK, 0, &CHASSIS_L, Face::PosY, 0, 0),
        on(2, &TRACK, 0, &CHASSIS_L, Face::NegY, 3 * quarter, 0),
        on(3, &ROTOR, 0, &CHASSIS_L, Face::PosZ, 0, 0),
        on(4, &ROCKET_LAUNCHER, 3, &ROTOR, Face::PosZ, 0, 3 * quarter),
    ];
    BETA.validate(&Candidate { parts })
        .expect("the mini-tank is a valid build")
}

/// Every wheel's and track's angle after `ticks` of the simulation driving `build` alone on a
/// floor under `controls`.
fn driven(
    build: Assembly,
    controls: shooter_sim::mechanics::Controls,
    ticks: u32,
) -> Result<Vec<(PartId, f32)>, String> {
    let mut mechanics = shooter_sim::mechanics::Mechanics::new(&fixtures::floor())
        .map_err(|error| format!("{error:?}"))?;
    mechanics
        .spawn(build, [0.0, 0.0, 0.0], 0)
        .map_err(|error| format!("{error:?}"))?;
    for _ in 0..ticks {
        mechanics
            .step(&[controls])
            .map_err(|error| format!("{error:?}"))?;
    }
    Ok(mechanics.robots()[0]
        .actuators
        .iter()
        .map(|actuator| (actuator.part, actuator.angle))
        .collect())
}

/// `locomotion.ppm`: the wheeled fixtures (the car and strafer with their wheels where the
/// simulation turned them after half a second's drive, the turret), the mini-tank, a car rolling
/// along at a run (its wheels turned by its ground motion, as in a match), a car with a damaged
/// wheel and one lost beside it, and the mini-tank with a damaged track; `locomotion-close.ppm`
/// the car and the mini-tank close up.
fn locomotion(
    gpu: &mut Gpu,
    robots: &mut Robots,
    origin: Vec3,
    out: &std::path::Path,
) -> Result<(), String> {
    use shooter_sim::mechanics::{Command, Controls};
    const RATE: i16 = 6_000;
    let tank = mini_tank();
    let car_angles = driven(fixtures::car(), fixtures::wheels(RATE, -RATE), 30)?;
    let strafer_angles = driven(fixtures::strafer(), fixtures::wheels(RATE, -RATE), 30)?;
    let tank_angles = driven(
        tank,
        Controls::IDLE
            .with(PartId(1), Command::Rate(RATE))
            .with(PartId(2), Command::Rate(-RATE)),
        30,
    )?;
    let toward = Vec3::new(-0.35, 0.0, -1.0).normalize();
    let at = |index: usize| {
        #[allow(clippy::cast_precision_loss)]
        let x = index as f32 * 1.6;
        origin - Vec3::X * x
    };
    let lost_wheel = fixtures::WHEELS[1];
    let damaged_wheel = [fixtures::WHEELS[0]];
    let damaged_track = [PartId(1)];
    // The lost front-left wheel lying flat beside the car: the car's rest frame, turned a
    // quarter about the robot's forward axis round the wheel's centre and lowered onto the
    // floor.
    let car_rest = robots::track::rest_frame(
        &fixtures::car(),
        at(5) + Vec3::Y * robots::pose::HALF_HEIGHT,
        toward,
    );
    let wheel_centre = Vec3::new(0.2, -0.4, -0.4);
    let lying = [(
        lost_wheel,
        Mat4::from_translation(Vec3::new(0.6, -0.08, -0.7))
            * car_rest
            * Mat4::from_translation(wheel_centre)
            * Mat4::from_rotation_x(std::f32::consts::FRAC_PI_2)
            * Mat4::from_translation(-wheel_centre),
    )];
    let lineup: Vec<(Assembly, RobotState, PartStates)> = vec![
        (
            fixtures::car(),
            standing(at(0), toward),
            PartStates {
                joints: &car_angles,
                ..PartStates::default()
            },
        ),
        (
            fixtures::strafer(),
            standing(at(1), toward),
            PartStates {
                joints: &strafer_angles,
                ..PartStates::default()
            },
        ),
        (
            fixtures::turret(),
            standing(at(2), toward),
            PartStates::default(),
        ),
        (
            tank,
            standing(at(3), toward),
            PartStates {
                joints: &tank_angles,
                ..PartStates::default()
            },
        ),
        (
            fixtures::car(),
            RobotState {
                velocity: toward * 6.0,
                ..standing(at(4), toward)
            },
            PartStates::default(),
        ),
        (
            fixtures::car(),
            standing(at(5), toward),
            PartStates {
                damaged: &damaged_wheel,
                detached: &lying,
                ..PartStates::default()
            },
        ),
        (
            tank,
            standing(at(6), toward),
            PartStates {
                damaged: &damaged_track,
                joints: &tank_angles,
                ..PartStates::default()
            },
        ),
    ];
    let views: Vec<RobotView> = lineup
        .iter()
        .enumerate()
        .map(|(index, (build, state, parts))| RobotView {
            slot: u8::try_from(index).unwrap_or(0),
            build: *build,
            state: *state,
            glow: 0.0,
            parts: *parts,
        })
        .collect();
    // The rolling car runs for a fifth of a second first: its wheels turn by the ground it
    // covers.
    robots.update(&mut gpu.world, &views, 0.0)?;
    robots.update(&mut gpu.world, &views[4..5], 0.2)?;
    let middle = at(3) + Vec3::Y * 0.5;
    shoot(
        gpu,
        robots,
        &views,
        middle + Vec3::new(0.0, 1.6, -6.2),
        middle,
        &out.join("locomotion.ppm"),
    )?;

    let close = [
        views[0],
        RobotView {
            slot: 1,
            state: standing(at(0) - Vec3::X * 1.4, toward),
            ..views[6]
        },
    ];
    let middle = at(0) - Vec3::X * 0.7 + Vec3::Y * 0.4;
    shoot(
        gpu,
        robots,
        &close,
        middle + Vec3::new(-0.9, 0.9, -2.6),
        middle,
        &out.join("locomotion-close.ppm"),
    )?;

    // The damaged wheel and the lost one, the damaged track, from low on their left.
    let damage = [views[5], views[6]];
    let middle = (at(5) + at(6)) / 2.0 + Vec3::Y * 0.3;
    shoot(
        gpu,
        robots,
        &damage,
        middle + Vec3::new(1.2, 0.6, -2.2),
        middle,
        &out.join("locomotion-damage.ppm"),
    )
}
