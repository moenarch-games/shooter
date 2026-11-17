//! A kit-built map drawn offscreen through the clients' own path (`shooter_client::scene` with
//! `shooter_client::arena_art`), with standard robots standing at its spawn points for scale:
//! an overview from high above, south of its middle, and views at the third-person camera's height
//! along its routes, for review:
//!
//! ```text
//! scripts/fetch-robot-kit.sh
//! cargo run -p shooter-client --release --example arena_views -- [<map> [<directory>]]
//! ```
//!
//! writes `overview.ppm` and `view-1.ppm`, `view-2.ppm`, … (the map's [`views`]) into the
//! directory (default `target/arena-views`); the map defaults to `assembly-works` (or
//! `proving-cell`).

use render_engine::glam::Vec3;
use shooter_client::arena_art::ArenaArt;
use shooter_client::gpu::Gpu;
use shooter_client::robots::{self, PartStates, RobotState, RobotView, Robots};
use shooter_sim::robot::STANDARD;
use shooter_sim::{MapId, Weapon};

const SIZE: (u32, u32) = (1600, 900);

/// Where each view looks from and at (metres), for `map`.
fn views(map: MapId) -> Vec<(Vec3, Vec3)> {
    match map {
        MapId::AssemblyWorks => vec![
            // The assembly floor from the west, under the mezzanine ramp: the bridge, the
            // cradle under it and the south mezzanine beyond.
            (Vec3::new(-14.0, 2.0, 2.0), Vec3::new(0.0, 2.5, 0.0)),
            // On the bridge, north toward the mezzanine, its parapet and the low cover.
            (Vec3::new(1.0, 5.0, 9.0), Vec3::new(0.0, 4.0, -8.0)),
            // From the north mezzanine east toward the work platforms and the crane tower.
            (Vec3::new(-6.0, 5.0, -15.0), Vec3::new(21.0, 4.0, -22.0)),
            // From the floor north up the work platforms to the crane and its mast.
            (Vec3::new(9.0, 2.0, -15.0), Vec3::new(20.0, 4.0, -22.0)),
            // Through the doorway into the north bay.
            (Vec3::new(-8.0, 2.0, -12.0), Vec3::new(-8.5, 1.5, -24.0)),
            // A charging bay in the east wall, from the floor.
            (Vec3::new(18.0, 2.0, -6.0), Vec3::new(27.0, 1.5, -11.0)),
        ],
        MapId::ProvingCell => vec![
            // The test floor from the south-east: the stage, its north ramp and the north
            // gallery beyond.
            (Vec3::new(10.0, 2.0, 1.0), Vec3::new(0.0, 2.5, -5.0)),
            // On the stage, north toward the gallery and its low cover.
            (Vec3::new(-1.0, 4.0, 1.5), Vec3::new(-3.0, 3.5, -14.0)),
            // From the north gallery's deck east toward the control booth.
            (Vec3::new(-9.0, 5.0, -14.0), Vec3::new(12.0, 2.0, -9.0)),
            // Through the booth's south doorway to its blast screen.
            (Vec3::new(12.5, 1.8, -6.2), Vec3::new(13.0, 1.5, -12.0)),
            // The passage under the north gallery, west toward its closed end.
            (Vec3::new(5.0, 1.8, -13.5), Vec3::new(-14.0, 1.5, -15.0)),
            // The floor west of the stage: the container, the jump pad and the gallery's ramp.
            (Vec3::new(-5.5, 2.2, 1.5), Vec3::new(-10.0, 2.0, -9.0)),
            // The test bay in the east wall, from the floor.
            (Vec3::new(7.0, 2.0, 1.0), Vec3::new(15.0, 1.5, -4.0)),
        ],
        _ => vec![(Vec3::new(0.0, 2.0, 10.0), Vec3::new(0.0, 2.0, 0.0))],
    }
}

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let map: MapId = args
        .next()
        .map_or(Ok(MapId::AssemblyWorks), |id| id.parse())
        .map_err(|error| error.to_string())?;
    let out = std::path::PathBuf::from(args.next().unwrap_or_else(|| "target/arena-views".into()));
    std::fs::create_dir_all(&out).map_err(|error| error.to_string())?;
    let mut gpu = Gpu::offscreen(SIZE.0, SIZE.1)?;
    let arena = map.arena();
    let art = ArenaArt::new(&mut gpu.world, robots::load::read_arena()?)?;
    shooter_client::scene::build(&mut gpu.world, &arena, Some(&art));
    let mut robots = Robots::new(&mut gpu.world, robots::load::read()?)?;
    let views_of: Vec<RobotView> = arena
        .spawn_points()
        .iter()
        .enumerate()
        .map(|(slot, spawn)| {
            #[allow(clippy::cast_precision_loss)]
            let feet = Vec3::new(
                spawn.feet.x as f32,
                spawn.feet.y as f32,
                spawn.feet.z as f32,
            ) * 0.01;
            let yaw = f32::from(spawn.yaw) / 65_536.0 * std::f32::consts::TAU;
            RobotView {
                slot: u8::try_from(slot).unwrap_or(0),
                build: STANDARD,
                state: RobotState {
                    center: feet + Vec3::Y * robots::pose::HALF_HEIGHT,
                    forward: Vec3::new(yaw.sin(), 0.0, yaw.cos()),
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
                },
                glow: 0.0,
                parts: PartStates::default(),
            }
        })
        .collect();
    robots.update(&mut gpu.world, &views_of, 0.0)?;
    robots.update(&mut gpu.world, &views_of, 0.0)?;
    #[allow(clippy::cast_precision_loss)]
    let half = arena.half_size as f32 * 0.01;
    let overview = (
        Vec3::new(0.0, 1.9 * half, 0.75 * half),
        Vec3::new(0.0, 0.0, 0.0),
    );
    let shots = std::iter::once(("overview".to_owned(), overview)).chain(
        views(map)
            .into_iter()
            .enumerate()
            .map(|(index, view)| (format!("view-{}", index + 1), view)),
    );
    for (name, (eye, target)) in shots {
        let scene = shooter_client::scene::frame(eye, (target - eye).normalize(), Vec::new());
        gpu.render(&scene, 0)?;
        let (width, height, pixels) = gpu.read_frame()?;
        let path = out.join(format!("{name}.ppm"));
        shooter_client::write_ppm(&path, width, height, &pixels)?;
        println!("{}", path.display());
    }
    Ok(())
}
