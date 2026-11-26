//! The main menu (shooter#154) driven through the client's own app offscreen, with the kits, as a
//! person clicks it: the main page, Practice on each of its three arenas, Settings with the
//! developer connection, the Robot Workshop and back, Play Online's bot match and Leave back to
//! the menu, a practice match with a chosen count, and the Workshop's active build (shooter#155):
//! a loadout put on the bench, saved under a name and made active with Done, an edit dropped with
//! Back, the client restarted on the same stores still playing it, a practice match with it and
//! nothing connecting, then the robo-dog, rover and mini-tank presets each made active from the
//! Workshop and played in a practice match (shooter#169). Every step checks what the app then plays (map, fighters and build) and
//! writes the frame:
//!
//! ```text
//! scripts/fetch-robot-kit.sh
//! cargo run -p shooter-client --release --example menu_views -- [<directory>]
//! ```
//!
//! writes `01-menu.ppm`, `02-practice-proving-cell.ppm`, … into the directory (default
//! `target/menu-views`). It exits non-zero on the first step that does not do what it should.

use shooter_client::app::App;
use shooter_client::builder::saved::{self, MemoryBuilds, SavedBuilds};
use shooter_client::gpu::Gpu;
use shooter_client::prefs::{MemoryPrefs, PrefStore};
use shooter_sim::MapId;
use shooter_sim::robot::{HEAVY, LOADOUTS, MARKSMAN, MINI_TANK, ROBO_DOG, ROVER};
use std::rc::Rc;
use winit::keyboard::KeyCode;

/// The frame's size.
const SIZE: (u32, u32) = (1280, 720);

struct Run {
    app: App,
    out: std::path::PathBuf,
    shots: u32,
}

impl Run {
    /// `frames` frames of a 60th of a second.
    fn frames(&mut self, frames: u32) -> Result<(), String> {
        for _ in 0..frames {
            self.app.offscreen_frame(1.0 / 60.0)?;
        }
        Ok(())
    }

    fn shoot(&mut self, name: &str) -> Result<(), String> {
        self.shots += 1;
        let gpu = self.app.gpu().ok_or("no GPU")?;
        let (width, height, pixels) = gpu.read_frame()?;
        let path = self.out.join(format!("{:02}-{name}.ppm", self.shots));
        shooter_client::write_ppm(&path, width, height, &pixels)?;
        println!("{}", path.display());
        Ok(())
    }

    /// The menu's readout.
    fn menu(&self) -> Result<serde_json::Value, String> {
        let readout = self.app.menu_readout().ok_or("the menu is not open")?;
        serde_json::from_str(&readout).map_err(|error| error.to_string())
    }

    /// Clicks the menu item called `name`.
    fn click(&mut self, name: &str) -> Result<(), String> {
        let menu = self.menu()?;
        let at = menu["targets"]
            .as_array()
            .and_then(|targets| targets.iter().find(|target| target["name"] == name))
            .map(|target| &target["at"])
            .ok_or_else(|| format!("no {name} on the {} page", menu["screen"]))?;
        #[allow(clippy::cast_possible_truncation)]
        let (x, y) = (
            at[0].as_f64().unwrap_or_default() as f32,
            at[1].as_f64().unwrap_or_default() as f32,
        );
        self.app.click(x, y);
        self.frames(2)
    }

    /// The open builder's readout.
    fn bench(&self) -> Result<serde_json::Value, String> {
        let readout = self
            .app
            .workbench_readout()
            .ok_or("the builder is not open")?;
        serde_json::from_str(&readout).map_err(|error| error.to_string())
    }

    /// Clicks the builder's item called `name`.
    fn click_bench(&mut self, name: &str) -> Result<(), String> {
        let bench = self.bench()?;
        let at = bench["targets"]
            .as_array()
            .and_then(|targets| targets.iter().find(|target| target["name"] == name))
            .map(|target| &target["at"])
            .ok_or_else(|| format!("no {name} on the builder"))?;
        #[allow(clippy::cast_possible_truncation)]
        let (x, y) = (
            at[0].as_f64().unwrap_or_default() as f32,
            at[1].as_f64().unwrap_or_default() as f32,
        );
        self.app.click(x, y);
        self.frames(2)
    }

    /// Presses each key in turn.
    fn keys(&mut self, codes: &[KeyCode]) -> Result<(), String> {
        for code in codes {
            self.app.press(*code);
        }
        self.frames(2)
    }

    fn check(&self, condition: bool, what: &str) -> Result<(), String> {
        if condition {
            println!("ok: {what}");
            Ok(())
        } else {
            Err(format!("failed: {what}"))
        }
    }
}

fn main() -> Result<(), String> {
    let out = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "target/menu-views".into()),
    );
    std::fs::create_dir_all(&out).map_err(|error| error.to_string())?;
    // The person's stores, in memory: the active build and saved builds outlive the app made
    // again on them (a restart), and nothing touches the machine's own.
    let prefs: Rc<dyn PrefStore> = Rc::new(MemoryPrefs::default());
    let builds: Rc<dyn SavedBuilds> = Rc::new(MemoryBuilds::default());
    let gpu = Gpu::offscreen(SIZE.0, SIZE.1)?;
    let mut run = Run {
        app: App::offscreen_with(None, gpu, prefs.clone(), builds.clone()),
        out,
        shots: 0,
    };
    run.frames(90)?;
    run.check(
        run.app.playing().is_none(),
        "the app opens on the menu, nothing playing",
    )?;
    run.check(run.menu()?["screen"] == "Main", "the main page shows")?;
    run.shoot("menu")?;

    // Practice: each arena previewed, the live view following the choice.
    run.click("Practice")?;
    for map in ["proving-cell", "assembly-works", "drydock"] {
        run.click(&format!("Map:{map}"))?;
        run.frames(60)?;
        let menu = run.menu()?;
        run.check(menu["map"] == map, &format!("Practice has {map} chosen"))?;
        run.shoot(&format!("practice-{map}"))?;
    }
    run.click("Back")?;

    // Settings, the developer connection open.
    run.click("Settings")?;
    run.click("Advanced")?;
    run.frames(10)?;
    run.check(
        run.menu()?["targets"]
            .as_array()
            .is_some_and(|targets| targets.iter().any(|target| target["name"] == "Url")),
        "the developer connection is under Advanced",
    )?;
    run.shoot("settings")?;
    run.click("Back")?;

    // The Workshop opens the builder with no match, and Escape goes back to the menu.
    run.click("Workshop")?;
    run.frames(30)?;
    run.check(
        run.app.playing().is_none() && run.app.menu_readout().is_none(),
        "the Workshop is open without a match",
    )?;
    run.shoot("workshop")?;
    run.app.press(KeyCode::Escape);
    run.frames(2)?;
    run.check(
        run.app.menu_readout().is_some(),
        "Back from the Workshop is the menu",
    )?;

    // Play Online: at once a 12-fighter bot match on the assembly works.
    run.click("PlayOnline")?;
    run.frames(240)?;
    run.check(
        run.app.playing() == Some((Some(MapId::AssemblyWorks), Some(12))),
        "Play Online is a 12-fighter match on the assembly works",
    )?;
    run.shoot("play-online")?;
    let leave = run.app.leave_at().ok_or("no Leave button")?;
    run.app.click(leave.x, leave.y);
    run.frames(30)?;
    run.check(
        run.app.playing().is_none() && run.menu()?["screen"] == "Main",
        "Leave goes back to the menu",
    )?;
    run.shoot("menu-again")?;

    // A practice match with a chosen count: the proving cell with four.
    run.click("Practice")?;
    run.click("Map:proving-cell")?;
    run.click("Fewer")?;
    run.click("Fewer")?;
    run.check(run.menu()?["fighters"] == 4, "four fighters chosen")?;
    run.click("Start")?;
    run.frames(240)?;
    run.check(
        run.app.playing() == Some((Some(MapId::ProvingCell), Some(4))),
        "Practice plays the proving cell with four fighters",
    )?;
    run.shoot("practice-match")?;

    // The Workshop's active build (shooter#155): the marksman loadout on the bench, saved as
    // SCOUT and made active with Done, all without a match or a connection.
    let leave = run.app.leave_at().ok_or("no Leave button")?;
    run.app.click(leave.x, leave.y);
    run.frames(2)?;
    run.click("Workshop")?;
    run.frames(30)?;
    run.check(
        run.app.played().is_none() && run.app.hosted().is_none(),
        "the Workshop opens with no match and nothing connecting",
    )?;
    run.click_bench("Loadout(3)")?;
    run.keys(&[KeyCode::KeyB])?;
    run.keys(&[
        KeyCode::KeyS,
        KeyCode::KeyC,
        KeyCode::KeyO,
        KeyCode::KeyU,
        KeyCode::KeyT,
        KeyCode::Enter,
    ])?;
    let bench = run.bench()?;
    run.check(
        bench["notice"] == "saved SCOUT" && bench["valid"] == true,
        "the marksman build is saved as SCOUT",
    )?;
    run.frames(20)?;
    run.shoot("workshop-saved")?;
    run.keys(&[KeyCode::Escape])?;
    run.click_bench("Ready")?;
    run.check(
        run.app.menu_readout().is_some() && run.app.active() == (MARKSMAN, "SCOUT"),
        "Done makes SCOUT the active build and returns to the menu",
    )?;
    run.check(
        run.menu()?["robot"]
            .as_str()
            .is_some_and(|line| line.starts_with("Your robot: SCOUT")),
        "the menu names the active build",
    )?;
    run.check(
        saved::read_active(prefs.as_ref()).is_some_and(|active| {
            active.is_ok_and(|active| active.name == "SCOUT" && active.build == MARKSMAN)
        }),
        "the active build is kept in the preferences",
    )?;

    // An edit left with Back is dropped: the active build stays SCOUT.
    run.click("Workshop")?;
    run.click_bench("Loadout(1)")?;
    run.check(
        run.bench()?["build"] == hex(&HEAVY),
        "the heavy loadout is on the bench",
    )?;
    run.click_bench("Back")?;
    run.check(
        run.app.active() == (MARKSMAN, "SCOUT"),
        "Back drops the edit and keeps the active build",
    )?;

    // The client restarted on the same stores: the menu still plays SCOUT, and a practice
    // match plays it, in this process.
    drop(run.app);
    let gpu = Gpu::offscreen(SIZE.0, SIZE.1)?;
    run.app = App::offscreen_with(None, gpu, prefs.clone(), builds.clone());
    run.frames(90)?;
    run.check(
        run.app.active() == (MARKSMAN, "SCOUT")
            && run.menu()?["robot"]
                .as_str()
                .is_some_and(|line| line.starts_with("Your robot: SCOUT")),
        "after a restart the active build is still SCOUT",
    )?;
    run.shoot("menu-active-build")?;
    run.click("Practice")?;
    run.click("Map:proving-cell")?;
    run.click("Start")?;
    run.frames(240)?;
    run.check(
        run.app.played() == Some((Some(MARKSMAN), true)) && run.app.hosted().is_none(),
        "Practice plays SCOUT in this process, nothing connecting",
    )?;
    run.shoot("practice-active-build")?;

    // The robo-dog, rover and mini-tank presets (shooter#169): each put on the bench from the
    // Workshop's START FROM list, made active with Done and played in a practice match.
    for (name, build) in [
        ("robo-dog", ROBO_DOG),
        ("rover", ROVER),
        ("mini-tank", MINI_TANK),
    ] {
        let index = LOADOUTS
            .iter()
            .position(|(key, _)| *key == name)
            .ok_or("no such loadout")?;
        let leave = run.app.leave_at().ok_or("no Leave button")?;
        run.app.click(leave.x, leave.y);
        run.frames(2)?;
        run.click("Workshop")?;
        run.frames(30)?;
        run.click_bench(&format!("Loadout({index})"))?;
        let bench = run.bench()?;
        run.check(
            bench["build"] == hex(&build) && bench["valid"] == true,
            &format!("the {name} preset is on the bench and valid"),
        )?;
        run.frames(20)?;
        run.shoot(&format!("workshop-{name}"))?;
        run.click_bench("Ready")?;
        run.check(
            run.app.active() == (build, "")
                && run.menu()?["robot"]
                    .as_str()
                    .is_some_and(|line| line.contains(&format!("{name} loadout"))),
            &format!("Done makes the {name} the active build"),
        )?;
        run.click("Practice")?;
        run.click("Map:proving-cell")?;
        run.click("Start")?;
        run.frames(240)?;
        run.check(
            run.app.played() == Some((Some(build), true)),
            &format!("Practice plays the {name}"),
        )?;
        run.shoot(&format!("practice-{name}"))?;
    }

    // With `SHOOTER_MENU_CONNECT=<match url>` and `SHOOTER_MENU_CERT=<pem>` (a host running as
    // `scripts/dev.sh` starts it): Settings' developer connection joins it.
    if let Ok(url) = std::env::var("SHOOTER_MENU_CONNECT") {
        let leave = run.app.leave_at().ok_or("no Leave button")?;
        run.app.click(leave.x, leave.y);
        run.frames(2)?;
        run.click("Settings")?;
        // Still open from before, or opened now.
        let open = run.menu()?["advanced"] == true;
        if !open {
            run.click("Advanced")?;
        }
        run.click("Url")?;
        run.app.type_text(&url);
        run.click("Certificate")?;
        run.app
            .type_text(&std::env::var("SHOOTER_MENU_CERT").unwrap_or_default());
        run.app.press(KeyCode::Enter);
        run.click("Connect")?;
        let mut admitted = false;
        for _ in 0..600 {
            run.frames(1)?;
            if run.app.hosted().is_some_and(|(admitted, _)| admitted) {
                admitted = true;
                break;
            }
        }
        let status = run.app.hosted().and_then(|(_, status)| status);
        run.check(
            admitted,
            &format!("the developer connection joins the host ({status:?})"),
        )?;
        run.frames(120)?;
        run.shoot("hosted-match")?;
    }
    Ok(())
}

/// `build`'s wire bytes in hex, as the builder's readout shows its build.
fn hex(build: &shooter_sim::Assembly) -> String {
    shooter_sim::wire::encode_build(build)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
