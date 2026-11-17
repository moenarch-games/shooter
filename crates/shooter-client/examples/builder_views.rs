//! The robot builder (shooter#57) drawn offscreen through the client's own path, with the pinned
//! kits (`scripts/fetch-robot-kit.sh`): the bench before the match, a part in hand previewed on
//! a socket it fits, a selected weapon, the rail rack with its long rail selected (shooter#126),
//! an over-budget build and a build between lives. Writes
//! PPM images to the directory given (default `target/builder-views`).

use shooter_client::builder::view::{Builder, Stage};
use shooter_client::gpu::Gpu;
use shooter_client::robots::{self, Robots};
use shooter_sim::robot::parts::{BATTERY, CHASSIS_M, LEG, RAIL_GUN, ROCKET_LAUNCHER, TRACK};
use shooter_sim::robot::{ARMORY, Face, HEAVY, PartId, STANDARD};

const SIZE: (u32, u32) = (1280, 720);

/// Two frames of `builder` (the first lays the robot out, the second poses it), the second
/// written to `path`.
fn shoot(
    gpu: &mut Gpu,
    robots: &mut Robots,
    builder: &mut Builder,
    path: &std::path::Path,
) -> Result<(), String> {
    for _ in 0..2 {
        let scene = builder.frame(Some((&mut gpu.world, robots)), SIZE.0, SIZE.1, 0.0);
        gpu.render(&scene, 0)?;
    }
    let (width, height, pixels) = gpu.read_frame()?;
    shooter_client::write_ppm(path, width, height, &pixels)?;
    println!("{}", path.display());
    Ok(())
}

fn main() -> Result<(), String> {
    let out = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "target/builder-views".into()),
    );
    std::fs::create_dir_all(&out).map_err(|error| error.to_string())?;
    let mut gpu = Gpu::offscreen(SIZE.0, SIZE.1)?;
    let mut robots = Robots::new(&mut gpu.world, robots::load::read()?)?;

    // Before the match, on the standard build.
    let mut builder = Builder::new(STANDARD, Stage::Setup);
    shoot(&mut gpu, &mut robots, &mut builder, &out.join("setup.ppm"))?;

    // A battery in hand from behind: the sockets it fits lit, one previewed with its footprint.
    builder.bench.hand = Some(BATTERY.id);
    builder.look_from(2.6, 0.35);
    builder.frame(None, SIZE.0, SIZE.1, 0.0);
    if !builder.point_at_a_fitting_socket() {
        return Err("no socket fits the battery".into());
    }
    shoot(
        &mut gpu,
        &mut robots,
        &mut builder,
        &out.join("placing.ppm"),
    )?;

    // The heavy build, its rail gun selected.
    let mut builder = Builder::new(HEAVY, Stage::Setup);
    builder.bench.selected = Some(PartId(7));
    shoot(
        &mut gpu,
        &mut robots,
        &mut builder,
        &out.join("selected.ppm"),
    )?;

    // The rail rack (shooter#126) from its front right, its long rail selected (with the four
    // weapons it carries).
    let mut builder = Builder::new(shooter_sim::robot::RAIL_RACK, Stage::Setup);
    builder.bench.selected = Some(PartId(3));
    builder.look_from(0.6, 0.25);
    shoot(&mut gpu, &mut robots, &mut builder, &out.join("rails.ppm"))?;

    // The controls panel (shooter#74) on the standard build's preset, its W binding selected.
    let mut builder = Builder::new(STANDARD, Stage::Setup);
    builder.controls_open = true;
    builder.controls.selected = Some(0);
    shoot(
        &mut gpu,
        &mut robots,
        &mut builder,
        &out.join("controls.ppm"),
    )?;

    // Over the build points: invalid, Ready off.
    let mut builder = Builder::new(ARMORY, Stage::Setup);
    shoot(
        &mut gpu,
        &mut robots,
        &mut builder,
        &out.join("invalid.ppm"),
    )?;

    // Between lives: a tracked medium chassis with the rail gun and rockets, built on the bench.
    let mut builder = Builder::new(STANDARD, Stage::BetweenLives);
    builder
        .bench
        .new_frame(CHASSIS_M.id)
        .map_err(|e| e.to_string())?;
    let socket = |face, u, v| CHASSIS_M.socket(face, u, v).ok_or("no socket");
    for (kind, face, u, v) in [
        (&TRACK, Face::PosY, 0, 0),
        (&TRACK, Face::NegY, 0, 0),
        (&RAIL_GUN, Face::PosX, 0, 2),
        (&ROCKET_LAUNCHER, Face::PosX, 1, 2),
        (&LEG, Face::NegZ, 0, 0),
    ] {
        builder.bench.hand = Some(kind.id);
        builder
            .bench
            .place(PartId(0), socket(face, u, v)?)
            .map_err(|refusal| format!("{}: {refusal}", kind.key))?;
    }
    builder.bench.hand = None;
    builder.look_from(0.9, 0.25);
    shoot(
        &mut gpu,
        &mut robots,
        &mut builder,
        &out.join("between-lives.ppm"),
    )
}
