//! The robots side by side in the greybox arena, drawn offscreen through the clients' own path
//! (`shooter_client::robots`) at a third-person gameplay scale, for review:
//!
//! ```text
//! scripts/fetch-robot-kit.sh
//! cargo run -p shooter-client --release --example robot_gallery -- <directory>
//! ```
//!
//! writes `loadouts.ppm` (the five named loadouts and the mechanics fixtures: the biped, the
//! robo-dog and the paired turret with its rotor and hinge turned by the simulation),
//! `poses.ppm` (the standard build as the simulation poses it: standing, two moments of its
//! walk, walking on one leg, lying on its chassis without legs; firing, swinging the blade, dead,
//! and with its blade shot off), `weapons.ppm` (the weapons' states and the mods: the rifle
//! held with the blade away, the rifle firing, a blade swing, the rocket pod firing, the rail gun
//! recharging, the skirmisher's grapple hooked with its mods, the marksman's long barrel and
//! overcharger, the pyro's flamethrower firing beside its long-barrelled laser), `close.ppm` (the
//! heavy build close up), `mods.ppm` (the skirmisher and marksman close up), `locomotion.ppm`
//! (the wheeled fixtures and the mini-tank as the simulation drove them, a car a second into its
//! drive, a damaged wheel and a lost one, a damaged track),
//! `locomotion-close.ppm` (the car and the mini-tank close up) and `locomotion-damage.ppm` (the
//! damaged and lost wheel and the damaged track close up).

use render_engine::glam::{Mat4, Quat, Vec3};
use shooter_client::gpu::Gpu;
use shooter_client::robots::{self, Frames, PartStates, RobotState, RobotView, Robots};
use shooter_sim::mechanics::{Command, Controls, Layout, Mechanics, fixtures};
use shooter_sim::robot::{HEAVY, MARKSMAN, PYRO, PartId, SKIRMISHER, STANDARD};
use shooter_sim::{Assembly, Instance, MapId, Weapon};

const SIZE: (u32, u32) = (1600, 900);

/// Lost parts and where each one's robot frame lies.
type Detached = Vec<(PartId, Mat4)>;

/// `build` alone on the fixtures' floor (facing +X), the parts in `lost` (a bit per part index)
/// severed, stepped `ticks` under `controls`: its bodies as the simulation has them, drawn with
/// its feet at `state`'s and facing its look. Alone in the mechanics world, without the match's
/// movement drive (which holds a fighter upright), a robot leans as its legs push it.
fn simulated(
    build: Assembly,
    lost: u32,
    ticks: u32,
    controls: impl Fn(&Layout) -> Controls,
    state: &RobotState,
) -> Result<Frames, String> {
    let error = |error: shooter_sim::physics::PhysicsError| format!("{error:?}");
    let mut mechanics = Mechanics::new(&fixtures::floor()).map_err(error)?;
    mechanics.spawn(build, [0.0; 3], 0).map_err(error)?;
    if lost != 0 {
        mechanics.sever(0, lost).map_err(error)?;
    }
    for _ in 0..ticks {
        let controls = controls(&mechanics.robots()[0].layout);
        mechanics.step(&[controls]).map_err(error)?;
    }
    let frames = Frames::of_robot(&mechanics, 0, Vec3::ZERO).ok_or("no robot")?;
    let root = frames.root().w_axis.truncate();
    let pivot = Vec3::new(root.x, 0.0, root.z);
    let look = Vec3::new(state.forward.x, 0.0, state.forward.z).normalize_or(Vec3::X);
    let turn = Quat::from_rotation_arc(Vec3::X, look);
    let feet = state.center - Vec3::Y * robots::pose::HALF_HEIGHT;
    Ok(Frames {
        center: state.center,
        bodies: frames
            .bodies
            .iter()
            .map(|(at, rotation)| (feet + turn * (*at - pivot), turn * *rotation))
            .collect(),
        ..frames
    })
}

/// Standing still on its legs (a zero stride holds them straight).
fn stand(layout: &Layout) -> Controls {
    Controls::IDLE.walk(layout, 0, 0)
}

/// Walking ahead at its full stride.
fn walk(layout: &Layout) -> Controls {
    Controls::IDLE.walk(layout, 100, 0)
}

/// The part-index bits of `build`'s legs.
fn legs_of(build: &Assembly) -> Vec<u32> {
    use shooter_sim::robot::{BETA, Kind, Locomotion};
    build
        .parts()
        .iter()
        .enumerate()
        .filter(|(_, part)| {
            BETA.part_type(part.kind).map(|kind| kind.kind)
                == Some(Kind::Locomotion(Locomotion::Leg))
        })
        .map(|(index, _)| 1 << index)
        .collect()
}

/// A lost part's robot frame lying on its side on the floor `beside` a spot.
fn lying(beside: Vec3) -> Mat4 {
    Mat4::from_translation(beside + Vec3::new(-0.5, 0.12, -0.7))
        * Mat4::from_rotation_z(-std::f32::consts::FRAC_PI_2)
        * Mat4::from_translation(Vec3::new(0.0, 0.0, -0.4))
}

fn standing(at: Vec3, forward: Vec3) -> RobotState {
    RobotState {
        center: at + Vec3::Y * robots::pose::HALF_HEIGHT,
        forward,
        velocity: Vec3::ZERO,
        sliding: false,
        hook: None,
        since_fired: f32::INFINITY,
        fired: Instance::new(7, Weapon::EnergyRifle),
        shots: 0,
        held: Instance::new(7, Weapon::EnergyRifle),
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

    // Each as the simulation spawns it; the turret's rotor and hinge driven to about
    // 0.6 and −0.5 rad (binary angles).
    let lineup = [
        STANDARD,
        HEAVY,
        SKIRMISHER,
        MARKSMAN,
        PYRO,
        fixtures::biped(),
        fixtures::dog(false),
        fixtures::paired_turret(),
    ];
    let turret = |_: &Layout| {
        Controls::IDLE
            .with(fixtures::ROTOR_PART, Command::Angle(6_258))
            .with(fixtures::HINGE_PART, Command::Angle(-5_215))
    };
    let frames: Vec<Frames> = lineup
        .iter()
        .enumerate()
        .map(|(index, build)| {
            let state = standing(at(index, 2.0), toward);
            if index == 7 {
                simulated(*build, 0, 90, turret, &state)
            } else {
                simulated(*build, 0, 0, stand, &state)
            }
        })
        .collect::<Result<_, _>>()?;
    let views: Vec<RobotView> = lineup
        .iter()
        .zip(&frames)
        .enumerate()
        .map(|(index, (build, frames))| RobotView {
            slot: u8::try_from(index).unwrap_or(0),
            build: *build,
            state: standing(at(index, 2.0), toward),
            glow: 0.0,
            highlight: None,
            parts: PartStates {
                frames: Some(frames),
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

    // The standard build as the simulation poses it, then what it does on top (the weapons'
    // states, the procedural death, a part shot off).
    let legs = legs_of(&STANDARD);
    let base = |index| standing(at(index, 1.45), toward);
    let walking = |index| RobotState {
        velocity: toward * 1.0,
        ..base(index)
    };
    let poses: Vec<(RobotState, Option<Frames>, Detached)> = vec![
        (
            base(0),
            Some(simulated(STANDARD, 0, 0, stand, &base(0))?),
            vec![],
        ),
        (
            walking(1),
            Some(simulated(STANDARD, 0, 22, walk, &walking(1))?),
            vec![],
        ),
        (
            walking(2),
            Some(simulated(STANDARD, 0, 36, walk, &walking(2))?),
            vec![],
        ),
        (
            walking(3),
            Some(simulated(STANDARD, legs[0], 20, walk, &walking(3))?),
            vec![(
                STANDARD.parts()[legs[0].trailing_zeros() as usize].id,
                lying(at(3, 1.45)),
            )],
        ),
        (
            base(4),
            Some(simulated(STANDARD, legs[0] | legs[1], 90, stand, &base(4))?),
            legs.iter()
                .enumerate()
                .map(|(n, bit)| {
                    #[allow(clippy::cast_precision_loss)]
                    let along = Vec3::X * 0.4 * n as f32;
                    (
                        STANDARD.parts()[bit.trailing_zeros() as usize].id,
                        lying(at(4, 1.45) + along),
                    )
                })
                .collect(),
        ),
        (
            RobotState {
                since_fired: 0.06,
                ..base(5)
            },
            Some(simulated(STANDARD, 0, 0, stand, &base(5))?),
            vec![],
        ),
        (
            RobotState {
                swing: Some(0.5),
                held: Instance::new(8, Weapon::EnergyBlade),
                ..base(6)
            },
            Some(simulated(STANDARD, 0, 0, stand, &base(6))?),
            vec![],
        ),
        (
            RobotState {
                dead_for: Some(3.0),
                ..base(7)
            },
            None,
            vec![],
        ),
        // Its blade shot off, lying on the floor.
        (
            base(8),
            Some(simulated(STANDARD, 0, 0, stand, &base(8))?),
            vec![(PartId(8), lying(at(8, 1.45)))],
        ),
    ];
    let views: Vec<RobotView> = poses
        .iter()
        .enumerate()
        .map(|(index, (state, frames, detached))| RobotView {
            slot: u8::try_from(index).unwrap_or(0),
            build: STANDARD,
            state: *state,
            glow: 0.0,
            highlight: None,
            parts: PartStates {
                frames: frames.as_ref(),
                detached,
                ..PartStates::default()
            },
        })
        .collect();
    let middle = at(4, 1.45) + Vec3::Y * 0.9;
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
                    held: Instance::new(9, Weapon::RocketLauncher),
                    fired: Instance::new(9, Weapon::RocketLauncher),
                    since_fired: 0.12,
                    shots: 1,
                    ..base(3)
                },
            ),
            (
                HEAVY,
                RobotState {
                    held: Instance::new(7, Weapon::RailGun),
                    fired: Instance::new(7, Weapon::RailGun),
                    since_fired: 0.75,
                    energy: 0.4,
                    ..base(4)
                },
            ),
            (
                SKIRMISHER,
                RobotState {
                    held: Instance::new(10, Weapon::Shotgun),
                    hook: Some(base(5).center + Vec3::new(0.7, 1.6, -1.6)),
                    ..base(5)
                },
            ),
            (
                MARKSMAN,
                RobotState {
                    held: Instance::new(9, Weapon::RailGun),
                    fired: Instance::new(9, Weapon::RailGun),
                    since_fired: 0.1,
                    ..base(6)
                },
            ),
            (
                PYRO,
                RobotState {
                    held: Instance::new(9, Weapon::Flamethrower),
                    fired: Instance::new(9, Weapon::Flamethrower),
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
            highlight: None,
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
            highlight: None,
            parts: PartStates::default(),
        },
        RobotView {
            slot: 1,
            build: MARKSMAN,
            state: standing(origin - Vec3::X * 1.6, Vec3::new(-0.5, 0.0, -1.0)),
            glow: 0.0,
            highlight: None,
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
        highlight: None,
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

/// `locomotion.ppm`: the wheeled fixtures (the car and strafer as the simulation drove them for
/// half a second, the turret), the mini-tank driven alike, a car a second into its drive, a car
/// with a damaged wheel and one lost beside it, and the mini-tank with a damaged track;
/// `locomotion-close.ppm` the car and the mini-tank close up.
fn locomotion(
    gpu: &mut Gpu,
    robots: &mut Robots,
    origin: Vec3,
    out: &std::path::Path,
) -> Result<(), String> {
    const RATE: i16 = 6_000;
    let tank = mini_tank();
    let wheels = |_: &Layout| fixtures::wheels(RATE, -RATE);
    let tracks = |_: &Layout| {
        Controls::IDLE
            .with(PartId(1), Command::Rate(RATE))
            .with(PartId(2), Command::Rate(-RATE))
    };
    let idle = |_: &Layout| Controls::IDLE;
    let toward = Vec3::new(-0.35, 0.0, -1.0).normalize();
    let at = |index: usize| {
        #[allow(clippy::cast_precision_loss)]
        let x = index as f32 * 1.6;
        origin - Vec3::X * x
    };
    let state = |index| standing(at(index), toward);
    let lost_wheel = fixtures::WHEELS[1];
    let lost_bit = fixtures::car()
        .parts()
        .iter()
        .position(|part| part.id == lost_wheel)
        .map_or(0, |index| 1 << index);
    let damaged_wheel = [fixtures::WHEELS[0]];
    let damaged_track = [PartId(1)];
    // The lost front-left wheel lying flat beside the car: the car's rest frame, turned a
    // quarter about the robot's forward axis round the wheel's centre and lowered onto the
    // floor.
    let car_rest = robots::track::rest_frame(&fixtures::car(), state(5).center, toward);
    let wheel_centre = Vec3::new(0.2, -0.4, -0.4);
    let lying = [(
        lost_wheel,
        Mat4::from_translation(Vec3::new(0.6, -0.08, -0.7))
            * car_rest
            * Mat4::from_translation(wheel_centre)
            * Mat4::from_rotation_x(std::f32::consts::FRAC_PI_2)
            * Mat4::from_translation(-wheel_centre),
    )];
    let frames = [
        simulated(fixtures::car(), 0, 30, wheels, &state(0))?,
        simulated(fixtures::strafer(), 0, 30, wheels, &state(1))?,
        simulated(fixtures::turret(), 0, 30, idle, &state(2))?,
        simulated(tank, 0, 30, tracks, &state(3))?,
        simulated(fixtures::car(), 0, 60, wheels, &state(4))?,
        simulated(fixtures::car(), lost_bit, 30, idle, &state(5))?,
        simulated(tank, 0, 30, tracks, &state(6))?,
    ];
    let lineup: Vec<(Assembly, PartStates)> = vec![
        (fixtures::car(), PartStates::default()),
        (fixtures::strafer(), PartStates::default()),
        (fixtures::turret(), PartStates::default()),
        (tank, PartStates::default()),
        (fixtures::car(), PartStates::default()),
        (
            fixtures::car(),
            PartStates {
                damaged: &damaged_wheel,
                detached: &lying,
                ..PartStates::default()
            },
        ),
        (
            tank,
            PartStates {
                damaged: &damaged_track,
                ..PartStates::default()
            },
        ),
    ];
    let views: Vec<RobotView> = lineup
        .iter()
        .zip(&frames)
        .enumerate()
        .map(|(index, ((build, parts), frames))| RobotView {
            slot: u8::try_from(index).unwrap_or(0),
            build: *build,
            state: state(index),
            glow: 0.0,
            highlight: None,
            parts: PartStates {
                frames: Some(frames),
                ..*parts
            },
        })
        .collect();
    let middle = at(3) + Vec3::Y * 0.5;
    shoot(
        gpu,
        robots,
        &views,
        middle + Vec3::new(0.0, 1.6, -6.2),
        middle,
        &out.join("locomotion.ppm"),
    )?;

    let beside = standing(at(0) - Vec3::X * 1.4, toward);
    let tank_beside = frames[6].at(beside.center);
    let close = [
        views[0],
        RobotView {
            slot: 1,
            state: beside,
            parts: PartStates {
                frames: Some(&tank_beside),
                ..views[6].parts
            },
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
