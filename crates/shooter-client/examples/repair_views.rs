//! Field repair (shooter#61) drawn offscreen through the client's own path, with the pinned kits
//! (`scripts/fetch-robot-kit.sh`): the practice match's mechanic (a repair module on its back)
//! with its leg and rifle worn, first with the quick repair menu open (the leg under the cursor,
//! lit on the robot), then while it repairs the leg, then with the repair paused by a hit.
//! Writes PPM images to the directory given (default `target/repair-views`):
//!
//! ```text
//! cargo run -p shooter-client --release --example repair_views -- [<directory>]
//! ```

use render_engine::glam::Vec3;
use shooter_client::gpu::Gpu;
use shooter_client::overlay::{Overlay, Vitals, repair_panel};
use shooter_client::repair_menu::RepairMenu;
use shooter_client::robots::{self, PartStates, RobotState, RobotView, Robots};
use shooter_client::scene::{self, METRES_PER_UNIT};
use shooter_sim::repair::Order;
use shooter_sim::robot::{MECHANIC, PartId};
use shooter_sim::{Arena, Instance, Intent, Session, Weapon};

const SIZE: (u32, u32) = (1280, 720);

/// The local fighter of `session` drawn with `menu`'s highlight and HUD, written to `path`.
fn shoot(
    gpu: &mut Gpu,
    robots: &mut Robots,
    session: &Session,
    menu: &RepairMenu,
    path: &std::path::Path,
) -> Result<(), String> {
    let me = session.local();
    let center = Vec3::from(me.center()) * METRES_PER_UNIT;
    let forward = Vec3::Z;
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
        parts: PartStates::default(),
        highlight: menu.highlight(me),
    };
    // From in front, a little to the robot's right and above: its legs, chest and the module's
    // edge behind it.
    let eye = center + Vec3::new(-1.3, 0.55, 2.1);
    let mut overlay = Overlay::default();
    for _ in 0..2 {
        robots.update(&mut gpu.world, std::slice::from_ref(&view), 0.0)?;
        let frame = scene::frame(eye, (center - eye).normalize(), Vec::new());
        let mut ui = overlay.draw(
            Some(&Vitals::of(me, session.ticks())),
            SIZE.0,
            SIZE.1,
            0.016,
        );
        ui.extend(repair_panel(&menu.view(me), SIZE.1));
        gpu.render(&frame, &ui, 0)?;
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
            .unwrap_or_else(|| "target/repair-views".into()),
    );
    std::fs::create_dir_all(&out).map_err(|error| error.to_string())?;
    let mut gpu = Gpu::offscreen(SIZE.0, SIZE.1)?;
    let arena = Arena::greybox();
    scene::build(&mut gpu.world, &arena, None);
    let mut robots = Robots::new(&mut gpu.world, robots::load::read()?)?;

    let mut session = Session::new(arena).map_err(|error| error.to_string())?;
    session
        .rebuild(0, MECHANIC)
        .map_err(|error| error.to_string())?;
    for _ in 0..30 {
        session.tick(Intent::IDLE).map_err(|e| e.to_string())?;
    }
    let build = session.fighters[0].build;
    // The left leg (5) and the rifle (7) worn.
    session.fighters[0].parts.hurt(&build, 2, 28);
    session.fighters[0].parts.hurt(&build, 4, 14);

    // R: the menu opens on the leg, lit on the robot.
    let mut menu = RepairMenu::default();
    menu.tick(Some(session.local()), 1, &mut 0);
    shoot(
        &mut gpu,
        &mut robots,
        &session,
        &menu,
        &out.join("menu.ppm"),
    )?;

    // R again: the leg under repair, the match running on.
    let order = menu.tick(Some(session.local()), 1, &mut 0);
    if order != Some(Order::Start(PartId(5))) {
        return Err(format!("the menu ordered {order:?}"));
    }
    session
        .tick(Intent {
            repair: order,
            ..Intent::IDLE
        })
        .map_err(|e| e.to_string())?;
    for _ in 0..60 {
        session.tick(Intent::IDLE).map_err(|e| e.to_string())?;
    }
    shoot(
        &mut gpu,
        &mut robots,
        &session,
        &menu,
        &out.join("repairing.ppm"),
    )?;

    // A direct hit on the leg pauses it (as the host's hit would).
    if let Some(repair) = session.fighters[0].repair.as_mut() {
        repair.struck(PartId(5));
    }
    shoot(
        &mut gpu,
        &mut robots,
        &session,
        &menu,
        &out.join("paused.ppm"),
    )
}
