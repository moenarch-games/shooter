//! Precision aim (shooter#132) drawn offscreen through the client's own camera, reticle and
//! scope, with the pinned kits (`scripts/fetch-robot-kit.sh`), on an open floor with a tall wall
//! 15 m ahead:
//!
//! - `normal.ppm` and `precision.ppm`: the standard build aiming at the wall at its guns'
//!   height, out and fully zoomed in (the scope, the narrower view); each gun's mark sits on the
//!   reticle.
//! - `turret-tracking.ppm`: the two-axis turret fixture after two seconds with its tracking
//!   binding held, its barrel turned up at a point high on the wall, its mark on the reticle.
//! - `turret-tracking-side.ppm`: the same turret seen from its left, ahead: the barrel raised
//!   toward the point.
//! - `unreachable.ppm`: the standard build aiming high up the wall: its fixed guns cannot point
//!   there, so each gun's amber mark sits where its mount sends the shot, below the reticle.
//!
//! ```text
//! cargo run -p shooter-client --release --example precision_views -- [<directory>]
//! ```

use render_engine::glam::Vec3;
use shooter_client::camera::{self, Aiming, Follow, Precision, Reticle};
use shooter_client::gpu::Gpu;
use shooter_client::overlay::{Overlay, Vitals, scope};
use shooter_client::robots::{self, Frames, PartStates, RobotState, RobotView, Robots};
use shooter_client::scene::{self, Actors, METRES_PER_UNIT};
use shooter_sim::aim;
use shooter_sim::bindings::{AIM_BUTTON, BindingState, Bindings, Pressed};
use shooter_sim::mechanics::fixtures;
use shooter_sim::physics::Vec3i;
use shooter_sim::robot::STANDARD;
use shooter_sim::{Arena, Assembly, Instance, Intent, Piece, Session, Spawn, Weapon};

const SIZE: (u32, u32) = (1280, 720);

/// An open floor, a wall 15 m south (+Z) of the spawn, 20 m high and 50 m wide.
fn walled() -> Arena {
    Arena {
        half_size: 3_000,
        pieces: vec![Piece::Block {
            center: Vec3i::new(0, 1_000, 1_550),
            half: Vec3i::new(2_500, 1_000, 50),
        }],
        pads: Vec::new(),
        posts: Vec::new(),
        spawn: Vec3i::new(0, 0, 0),
        spawn_yaw: 0,
        spawns: vec![Spawn {
            feet: Vec3i::new(0, 0, 0),
            yaw: 0,
        }],
        module_pads: Vec::new(),
        waypoints: Vec::new(),
        placements: Vec::new(),
        repair_anchors: Vec::new(),
    }
}

/// A practice session with the local fighter built as `build`.
fn session(build: Assembly) -> Result<Session, String> {
    let mut session = Session::new(walled()).map_err(|error| error.to_string())?;
    session.fighters[0].configured = build;
    session
        .rebuild(0, build)
        .map_err(|error| error.to_string())?;
    Ok(session)
}

/// `ticks` of the local fighter aiming its reticle at `point`, its tracking binding held when
/// `track` is.
fn aim_for(
    session: &mut Session,
    point: [f32; 3],
    ticks: u32,
    track: bool,
) -> Result<(u16, i16), String> {
    let mut state = BindingState::default();
    let mut aimed = (0, 0);
    for _ in 0..ticks {
        let (yaw, pitch) = aim::at(Some(&session.world), session.local().eye(), point);
        aimed = (yaw, pitch);
        let motors = match session.mechanics.robot(0) {
            Some(robot) if track => Bindings::preset(&robot.layout).resolve(
                &robot.layout,
                &mut state,
                Pressed {
                    buttons: 1 << AIM_BUTTON,
                    ..Pressed::default()
                },
            ),
            _ => shooter_sim::bindings::Motors::REST,
        };
        session
            .tick(Intent {
                yaw,
                pitch,
                motors,
                ..Intent::IDLE
            })
            .map_err(|error| error.to_string())?;
    }
    Ok(aimed)
}

/// The local fighter as the client draws it: the camera's view, the reticle with every gun's
/// mark, the scope when `zoom` is in, written to `path`.
#[allow(clippy::too_many_arguments)]
fn shoot(
    gpu: &mut Gpu,
    robots: &mut Robots,
    actors: &mut Actors,
    session: &Session,
    (yaw, pitch): (u16, i16),
    zoomed: bool,
    aside: Option<(Vec3, Vec3)>,
    path: &std::path::Path,
) -> Result<(), String> {
    let me = session.local();
    let center = Vec3::from(me.center()) * METRES_PER_UNIT;
    let eye = Vec3::from(me.eye()) * METRES_PER_UNIT;
    let forward = Vec3::from(aim::unit(shooter_sim::weapons::aim(yaw, pitch)));
    let camera = Follow::default().place(Some(&session.world), eye, forward, 1.0 / 60.0);
    let mut precision = Precision::default();
    precision.update(zoomed, 1.0);
    let reticle = Reticle::aimed(&Aiming {
        world: &session.world,
        fighter: Some(me),
        eye: me.eye(),
        held: me.arsenal.held,
        yaw,
        pitch,
        others: &[],
        camera,
        forward,
    });
    println!(
        "{}: {:?}",
        path.display(),
        reticle
            .guns
            .iter()
            .map(|(_, mark)| *mark)
            .collect::<Vec<camera::Mark>>()
    );
    let frames = Frames::of_robot(&session.mechanics, 0, center);
    let view = RobotView {
        slot: 0,
        build: me.build,
        state: RobotState {
            center,
            forward,
            velocity: Vec3::ZERO,
            sliding: false,
            hook: None,
            since_fired: f32::INFINITY,
            fired: Instance::new(7, Weapon::EnergyRifle),
            shots: 0,
            recent: robots::pose::Recent::NONE,
            held: me.arsenal.held,
            energy: 1.0,
            swing: None,
            dead_for: None,
        },
        glow: 0.0,
        parts: PartStates {
            frames: frames.as_ref(),
            detached: &[],
            damaged: &[],
        },
        highlight: None,
    };
    let mut overlay = Overlay::default();
    for _ in 0..2 {
        robots.update(&mut gpu.world, std::slice::from_ref(&view), 0.0)?;
        // From beside the robot (`aside`): no reticle, no scope, the view's own field.
        let shown = aside.is_none().then(|| reticle.clone());
        actors.update(&mut gpu.world, &[], &[], &[], shown);
        let (from, toward) = aside.unwrap_or((camera, forward));
        let mut frame = scene::frame(from, toward, Vec::new());
        let mut ui = Vec::new();
        if aside.is_none() {
            frame.camera.fov_y = precision.fov_y();
            ui = scope(SIZE.0, SIZE.1, precision.zoom());
        }
        ui.extend(overlay.draw(
            Some(&Vitals::of(me, session.ticks())),
            SIZE.0,
            SIZE.1,
            0.016,
        ));
        gpu.render(&mut frame, &ui, 0)?;
    }
    let (width, height, pixels) = gpu.read_frame()?;
    shooter_client::write_ppm(path, width, height, &pixels)?;
    Ok(())
}

fn main() -> Result<(), String> {
    let out = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "target/precision-views".into()),
    );
    std::fs::create_dir_all(&out).map_err(|error| error.to_string())?;
    let mut gpu = Gpu::offscreen(SIZE.0, SIZE.1)?;
    scene::build(&mut gpu.world, &walled(), None);
    let mut robots = Robots::new(&mut gpu.world, robots::load::read()?)?;
    let mut actors = Actors::new(&mut gpu.world);

    // The standard build aiming at the wall at its guns' height: out, then zoomed in.
    let mut standard = session(STANDARD)?;
    let level = aim_for(&mut standard, [0.0, 120.0, 1_500.0], 60, false)?;
    for (zoomed, name) in [(false, "normal.ppm"), (true, "precision.ppm")] {
        shoot(
            &mut gpu,
            &mut robots,
            &mut actors,
            &standard,
            level,
            zoomed,
            None,
            &out.join(name),
        )?;
    }

    // The same build aiming high up the wall: its fixed guns cannot point there.
    let high = aim_for(&mut standard, [0.0, 900.0, 1_500.0], 30, false)?;
    shoot(
        &mut gpu,
        &mut robots,
        &mut actors,
        &standard,
        high,
        false,
        None,
        &out.join("unreachable.ppm"),
    )?;

    // The two-axis turret tracking a point high on the wall, its binding held for two seconds.
    let mut turret = session(fixtures::paired_turret())?;
    let up = aim_for(&mut turret, [-400.0, 1_500.0, 1_500.0], 120, true)?;
    shoot(
        &mut gpu,
        &mut robots,
        &mut actors,
        &turret,
        up,
        true,
        None,
        &out.join("turret-tracking.ppm"),
    )?;
    // And from its left, ahead: the barrel raised toward the point, the hinge short of its
    // limit.
    let center = Vec3::from(turret.local().center()) * METRES_PER_UNIT;
    let from = center + Vec3::new(3.2, 1.4, 2.4);
    let toward = (center + Vec3::new(0.0, 0.8, 0.0) - from).normalize();
    shoot(
        &mut gpu,
        &mut robots,
        &mut actors,
        &turret,
        up,
        false,
        Some((from, toward)),
        &out.join("turret-tracking-side.ppm"),
    )?;
    Ok(())
}
