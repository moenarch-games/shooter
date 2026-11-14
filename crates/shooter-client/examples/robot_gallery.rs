//! The robots side by side in the greybox arena, drawn offscreen through the clients' own path
//! (`shooter_client::robots`) at a third-person gameplay scale, for review:
//!
//! ```text
//! scripts/fetch-robot-kit.sh
//! cargo run -p shooter-client --release --example robot_gallery -- <directory>
//! ```
//!
//! writes `loadouts.ppm` (the four named loadouts and the mechanics fixtures: the biped, the
//! robo-dog and the paired turret with its rotor and hinge turned), `poses.ppm` (the standard
//! build idle, running, strafing, airborne, grappling, sliding, firing, swinging the blade, dead,
//! and with its right arm shot off) and `close.ppm` (the heavy build close up).

use render_engine::glam::{Mat4, Vec3};
use shooter_client::gpu::Gpu;
use shooter_client::robots::{self, PartStates, RobotState, RobotView, Robots};
use shooter_sim::mechanics::fixtures;
use shooter_sim::robot::{HEAVY, MARKSMAN, PartId, SKIRMISHER, STANDARD};
use shooter_sim::{Assembly, MapId, Weapon};

const SIZE: (u32, u32) = (1600, 900);

fn standing(at: Vec3, forward: Vec3) -> RobotState {
    RobotState {
        center: at + Vec3::Y * robots::pose::HALF_HEIGHT,
        forward,
        velocity: Vec3::ZERO,
        sliding: false,
        hooked: false,
        since_fired: f32::INFINITY,
        held: Weapon::EnergyRifle,
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
    shooter_client::scene::build(&mut gpu.world, &arena);
    let kit = robots::load::read(&robots::load::directory())?;
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
    let lineup: [(Assembly, &[(PartId, f32)]); 7] = [
        (STANDARD, &[]),
        (HEAVY, &[]),
        (SKIRMISHER, &[]),
        (MARKSMAN, &[]),
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
    let middle = at(3, 2.0) + Vec3::Y * 0.9;
    shoot(
        &mut gpu,
        &mut robots,
        &views,
        middle + Vec3::new(0.0, 0.9, -6.0),
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
                    hooked: true,
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
                    since_fired: 0.02,
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
    // The standard build's right arm (and the blade on it) shot off, lying on the floor.
    let fallen = [(
        PartId(4),
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
    )
}
