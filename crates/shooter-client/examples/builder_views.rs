//! The robot builder (shooter#57, shooter#124) drawn offscreen through the client's own path,
//! with the pinned kits (`scripts/fetch-robot-kit.sh`): the bench before the match, a part in
//! hand previewed on a socket it fits, snapping on a crowded face, a part in hand turned about
//! each axis, a placement the rules refuse for a taken cell, a placement on a turned parent, a
//! part selected again after placing, a selected weapon, an over-budget build and a build
//! between lives. Writes PPM images to the directory given (default `target/builder-views`).

use shooter_client::builder::view::{Builder, Stage};
use shooter_client::gpu::Gpu;
use shooter_client::robots::{self, Robots};
use shooter_sim::robot::parts::{
    BATTERY, BEAM_LONG, CHASSIS_L, CHASSIS_M, CHASSIS_S, LEG, RAIL_GUN, ROCKET_LAUNCHER, SENSOR,
    TRACK,
};
use shooter_sim::robot::{ARMORY, Face, HEAVY, Orientation, PartId, STANDARD};
use winit::keyboard::KeyCode;

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

/// A builder on `build` holding `kind`, seen from `azimuth` and `elevation`, with the pointer on
/// `socket` of `part` (the frame before lays the sockets out).
fn pointing(
    build: shooter_sim::Assembly,
    kind: &shooter_sim::robot::PartType,
    (azimuth, elevation): (f32, f32),
    part: PartId,
    socket: Option<shooter_sim::robot::SocketId>,
) -> Result<Builder, String> {
    let mut builder = Builder::new(build, Stage::Setup);
    builder.bench.hand = Some(kind.id);
    builder.look_from(azimuth, elevation);
    builder.frame(None, SIZE.0, SIZE.1, 0.0);
    let socket = socket.ok_or("no such socket")?;
    if !builder.point_at(part, socket) {
        return Err(format!("{part:?} {socket:?} is not seen"));
    }
    Ok(builder)
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
    let mut shoot =
        |builder: &mut Builder, name: &str| shoot(&mut gpu, &mut robots, builder, &out.join(name));

    // Before the match, on the standard build.
    let mut builder = Builder::new(STANDARD, Stage::Setup);
    shoot(&mut builder, "setup.ppm")?;

    // A battery in hand from behind: the sockets it fits lit, one previewed with its footprint.
    builder.bench.hand = Some(BATTERY.id);
    builder.look_from(2.6, 0.35);
    builder.frame(None, SIZE.0, SIZE.1, 0.0);
    if !builder.point_at_a_fitting_socket() {
        return Err("no socket fits the battery".into());
    }
    shoot(&mut builder, "placing.ppm")?;

    // Snapping on the crowded front: the battery on the free cell low on the right, beside the
    // blade's, previewed with its two-cell footprint.
    let front = |u, v| CHASSIS_S.socket(Face::PosX, u, v);
    let mut builder = pointing(STANDARD, &BATTERY, (0.35, 0.15), PartId(0), front(0, 0))?;
    shoot(&mut builder, "snapping.ppm")?;

    // The rail gun in hand on the back, turned a quarter about each of the socket's axes in
    // turn (R yaw, T pitch, F roll): the preview is the part as it will be placed.
    let back = CHASSIS_S.socket(Face::NegX, 0, 1);
    for (key, name) in [
        (KeyCode::KeyR, "turn-yaw.ppm"),
        (KeyCode::KeyT, "turn-pitch.ppm"),
        (KeyCode::KeyF, "turn-roll.ppm"),
    ] {
        let mut builder = pointing(STANDARD, &RAIL_GUN, (2.5, 0.3), PartId(0), back)?;
        builder.key(key);
        shoot(&mut builder, name)?;
    }

    // Refused: the battery's two cells from the front's (1, 1) reach the rifle's: in red, the
    // taken cell and the rifle marked, and the reason.
    let mut builder = pointing(STANDARD, &BATTERY, (0.35, 0.15), PartId(0), front(1, 1))?;
    shoot(&mut builder, "blocked.ppm")?;

    // A turned parent: a long beam on a large chassis' side, turned an eighth and tipped; the
    // sensor in hand snaps to the beam's turned faces.
    let mut builder = Builder::new(STANDARD, Stage::Setup);
    builder
        .bench
        .new_frame(CHASSIS_L.id)
        .map_err(|e| e.to_string())?;
    builder.bench.hand = Some(BEAM_LONG.id);
    builder.bench.turn = Orientation::canonical(Orientation::QUARTER / 2, 3_000, 0);
    let side = CHASSIS_L.socket(Face::PosY, 1, 1).ok_or("no socket")?;
    let beam = builder
        .bench
        .place(PartId(0), side)
        .map_err(|e| e.to_string())?;
    let build = *builder.bench.draft();
    let end = BEAM_LONG.socket(Face::PosX, 0, 3);
    let mut builder = pointing(build, &SENSOR, (-1.2, 0.35), beam, end)?;
    shoot(&mut builder, "turned-parent.ppm")?;

    // Selected again: a battery placed on the back, the hand dropped, the sensor turned, then
    // the battery clicked again (its box and footprint marked) and turned on its socket.
    let mut builder = pointing(STANDARD, &BATTERY, (2.6, 0.35), PartId(0), back)?;
    builder.mouse(false, true);
    let battery = builder.bench.selected.ok_or("not placed")?;
    builder.key(KeyCode::Escape);
    builder.bench.selected = Some(PartId(2));
    builder.key(KeyCode::KeyF);
    builder.bench.selected = Some(battery);
    builder.key(KeyCode::KeyR);
    shoot(&mut builder, "reselected.ppm")?;

    // The heavy build, its rail gun selected.
    let mut builder = Builder::new(HEAVY, Stage::Setup);
    builder.bench.selected = Some(PartId(7));
    shoot(&mut builder, "selected.ppm")?;

    // Over the build points: invalid, Ready off.
    let mut builder = Builder::new(ARMORY, Stage::Setup);
    shoot(&mut builder, "invalid.ppm")?;

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
    shoot(&mut builder, "between-lives.ppm")
}
