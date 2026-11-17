//! The fast moves on physics-engine: each scripted scenario does what a player expects.

use shooter_sim::player::{PLAYER_HALF_EXTENTS, SLIDE_SPEED_UNITS_PER_SECOND};
use shooter_sim::scenarios::{self, Scenario};
use shooter_sim::tour::{Sample, end_of, run_legs};
use shooter_sim::{Session, TICK_HZ};

fn run(scenario: Scenario) -> Vec<Sample> {
    let mut session = Session::new((scenario.arena)()).expect("arena builds");
    run_legs(&mut session, scenario.legs).expect("scenario runs")
}

const HALF: f32 = PLAYER_HALF_EXTENTS.y as f32;
/// How far (units) a robot standing on its legs may sit from its build's rest height: its legs
/// give a little under it.
const STAND: f32 = 3.0;

/// Horizontal units moved over the ticks of leg `name`.
fn speeds(samples: &[Sample], name: &str) -> Vec<f32> {
    samples
        .windows(2)
        .filter(|pair| pair[1].leg == name)
        .map(|pair| {
            (pair[1].center[0] - pair[0].center[0]).hypot(pair[1].center[2] - pair[0].center[2])
        })
        .collect()
}

#[test]
fn sprinting_is_faster_and_a_slide_starts_faster_still_and_decays() {
    let samples = run(scenarios::SPRINT_AND_SLIDE);
    let sprint = speeds(&samples, "sprint");
    // (Measured at the robot's frame, which sways a little about its centre of mass.)
    assert!((sprint.last().unwrap() - 17.5).abs() < 0.3, "{sprint:?}");
    let slide = speeds(&samples, "slide");
    let peak = slide.iter().copied().fold(0.0, f32::max);
    let slide_per_tick = (SLIDE_SPEED_UNITS_PER_SECOND / i32::from(TICK_HZ)) as f32;
    assert!((peak - slide_per_tick).abs() < 0.5, "{slide:?}");
    // It decays to walking speed and ends; the held sprint takes over again.
    assert!(
        slide
            .windows(2)
            .all(|pair| pair[1] <= pair[0] || pair[1] > 17.0),
        "{slide:?}"
    );
    assert!(
        slide.iter().any(|speed| (speed - 12.0).abs() < 0.5),
        "{slide:?}"
    );
    assert!((slide.last().unwrap() - 17.5).abs() < 0.3, "{slide:?}");
    assert!(samples.iter().any(|s| s.sliding));
    assert!(!end_of(&samples, "slide").unwrap().sliding);
}

#[test]
fn a_double_jump_reaches_a_ledge_one_jump_cannot() {
    let samples = run(scenarios::DOUBLE_JUMP);
    assert_eq!(samples.iter().filter(|s| s.air_jumped).count(), 1);
    let top = end_of(&samples, "on the ledge").unwrap();
    assert!(top.grounded, "{top:?}");
    assert!((top.center[1] - (160.0 + HALF)).abs() < STAND, "{top:?}");
}

#[test]
fn the_jump_pad_throws_onto_the_platform() {
    let samples = run(scenarios::JUMP_PAD);
    assert_eq!(samples.iter().filter(|s| s.launched).count(), 1);
    let peak = samples.iter().map(|s| s.center[1]).fold(f32::MIN, f32::max);
    assert!(peak > 350.0 + HALF, "apex {peak}");
    let landed = end_of(&samples, "on the platform").unwrap();
    assert!(landed.grounded, "{landed:?}");
    assert!(
        (landed.center[1] - (200.0 + HALF)).abs() < STAND,
        "{landed:?}"
    );
    assert!(
        landed.center[0].abs() < 400.0 && landed.center[2].abs() < 400.0,
        "{landed:?}"
    );
}

/// The greybox yard's second level: its ramp climbs onto the walkway, the deck carries a fighter
/// along it, its edge drops them back to the ground, and its pad throws them up onto it.
#[test]
fn the_yards_walkway_is_reached_by_ramp_and_pad() {
    let samples = run(scenarios::YARD_WALKWAY);
    let deck = end_of(&samples, "on the walkway").unwrap();
    assert!(
        deck.grounded && (deck.center[1] - (300.0 + HALF)).abs() < STAND,
        "{deck:?}"
    );
    assert!(
        deck.center[0] > -1_400.0 && deck.center[2].abs() < 50.0,
        "{deck:?}"
    );
    let below = end_of(&samples, "below").unwrap();
    assert!(
        below.grounded && (below.center[1] - HALF).abs() < STAND && below.center[2] > 200.0,
        "{below:?}"
    );

    let samples = run(scenarios::YARD_JUMP_PAD);
    assert_eq!(samples.iter().filter(|s| s.launched).count(), 1);
    let landed = end_of(&samples, "on the walkway").unwrap();
    assert!(
        landed.grounded && (landed.center[1] - (300.0 + HALF)).abs() < STAND,
        "{landed:?}"
    );
    assert!(landed.center[2].abs() < 200.0, "{landed:?}");
}

/// The assembly works' vertical routes: its ramp climbs onto the mezzanine and the deck carries a
/// fighter along it and the bridge to the middle, and its edge drops them to the floor; the pad,
/// the steps and the grapple each put a fighter on the crane tower's 5 m top.
#[test]
fn the_assembly_works_routes_reach_its_high_ground() {
    use shooter_sim::assembly_works::{DECK_TOP, TOWER_TOP};
    let deck = DECK_TOP as f32 + HALF;
    let samples = run(scenarios::WORKS_RAMP_AND_BRIDGE);
    let mezzanine = end_of(&samples, "along the mezzanine").unwrap();
    assert!(
        mezzanine.grounded && (mezzanine.center[1] - deck).abs() < STAND,
        "{mezzanine:?}"
    );
    let bridge = end_of(&samples, "on the bridge").unwrap();
    assert!(
        bridge.grounded
            && (bridge.center[1] - deck).abs() < STAND
            && bridge.center[2].abs() < 100.0
            && bridge.center[0].abs() < 200.0,
        "{bridge:?}"
    );
    let below = end_of(&samples, "below").unwrap();
    assert!(
        below.grounded && (below.center[1] - HALF).abs() < STAND && below.center[0] > 230.0,
        "{below:?}"
    );

    let on_tower = |sample: &Sample| {
        sample.grounded
            && sample.center[1] - HALF >= TOWER_TOP as f32 - 1.0
            && (2_000.0..=2_400.0).contains(&sample.center[0])
            && (-2_400.0..=-2_000.0).contains(&sample.center[2])
    };
    let samples = run(scenarios::WORKS_TOWER_PAD);
    assert_eq!(samples.iter().filter(|s| s.launched).count(), 1);
    let landed = end_of(&samples, "on the tower").unwrap();
    assert!(on_tower(landed), "{landed:?}");

    // A double jump still climbs onto the 2 m step; from there the standard robot's guns meet the
    // 4 m step's face before a double jump clears its lip (the capsule's route: a level change
    // for robots, the pad and the grapple reach the tower).
    let samples = run(scenarios::WORKS_STEPS);
    let step = end_of(&samples, "onto the 2 m step").unwrap();
    assert!(
        step.grounded && (step.center[1] - HALF - 200.0).abs() < STAND,
        "{step:?}"
    );

    let samples = run(scenarios::WORKS_GRAPPLE);
    assert!(samples.iter().any(|s| s.hooked));
    let hung = end_of(&samples, "on the tower").unwrap();
    assert!(on_tower(hung), "{hung:?}");
}

/// The assembly works' cover collides as the kit's proxies say: the 1.2 m low cover stops a walk
/// and a double jump clears it; the 2 m barrier stops a walk at its face.
#[test]
fn the_assembly_works_cover_blocks_and_is_jumped() {
    let samples = run(scenarios::WORKS_COVER);
    let stopped = end_of(&samples, "into the cover").unwrap();
    // The cover's north face is at z -1130: the robot's legs stop against it (its guns, 1.3 m
    // up, reach over the 1.2 m cover).
    assert!(
        (-1_150.0..-1_130.0).contains(&stopped.center[2]),
        "{stopped:?}"
    );
    // A jump, then a second press (an air jump, or a jump off the lip its legs caught), clears it.
    assert!(
        samples
            .iter()
            .any(|s| s.leg == "air jump" && s.center[1] > 120.0)
    );
    let over = end_of(&samples, "land").unwrap();
    assert!(over.center[2] > -1_070.0, "{over:?}");
    // Where a robot lands after the cover depends on how its legs caught the lip; the 2 m
    // barrier is checked from straight west of it. Its west face is at x -1030: the robot's
    // guns, ahead of its legs, stop against it.
    let mut session = Session::new(shooter_sim::Arena {
        spawn: shooter_sim::physics::Vec3i::new(-1_400, 0, -600),
        spawn_yaw: shooter_sim::tour::EAST,
        ..shooter_sim::Arena::assembly_works()
    })
    .expect("arena builds");
    let legs = [
        shooter_sim::tour::stand("settle", 10),
        shooter_sim::tour::walk("into the barrier", shooter_sim::tour::EAST, 50),
        shooter_sim::tour::stand("against the barrier", 10),
    ];
    let samples = run_legs(&mut session, &legs).expect("scenario runs");
    let against = end_of(&samples, "against the barrier").unwrap();
    assert!(
        against.grounded && (-1_080.0..-1_040.0).contains(&against.center[0]),
        "{against:?}"
    );
}

/// The proving cell's vertical routes: its ramp climbs onto the gallery and the deck carries a
/// fighter along it, and its edge drops them to the floor; the stage's ramps climb onto it and
/// down again; the pad and a grapple from the stage each put a fighter on the gallery.
#[test]
fn the_proving_cell_routes_reach_its_high_ground() {
    use shooter_sim::proving_cell::{DECK_TOP, STAGE_TOP};
    let deck = DECK_TOP as f32 + HALF;
    let on_gallery = |sample: &Sample| {
        sample.grounded
            && (sample.center[1] - deck).abs() < STAND
            && (-1_600.0..=-1_200.0).contains(&sample.center[2])
    };
    let samples = run(scenarios::CELL_GALLERY);
    let up = end_of(&samples, "up the ramp").unwrap();
    assert!(on_gallery(up), "{up:?}");
    let along = end_of(&samples, "on the gallery").unwrap();
    assert!(on_gallery(along) && along.center[0] > -500.0, "{along:?}");
    let below = end_of(&samples, "below").unwrap();
    assert!(
        below.grounded && (below.center[1] - HALF).abs() < STAND && below.center[2] > -1_150.0,
        "{below:?}"
    );

    let samples = run(scenarios::CELL_STAGE);
    let stage = end_of(&samples, "on the stage").unwrap();
    assert!(
        stage.grounded
            && (stage.center[1] - (STAGE_TOP as f32 + HALF)).abs() < STAND
            && stage.center[2].abs() < 300.0,
        "{stage:?}"
    );
    let foot = end_of(&samples, "at its foot").unwrap();
    assert!(
        foot.grounded && (foot.center[1] - HALF).abs() < STAND && foot.center[2] > 1_100.0,
        "{foot:?}"
    );

    let samples = run(scenarios::CELL_PAD);
    assert_eq!(samples.iter().filter(|s| s.launched).count(), 1);
    let landed = end_of(&samples, "on the gallery").unwrap();
    assert!(on_gallery(landed), "{landed:?}");

    let samples = run(scenarios::CELL_GRAPPLE);
    assert!(samples.iter().any(|s| s.hooked));
    let hung = end_of(&samples, "on the gallery").unwrap();
    assert!(on_gallery(hung) && !hung.hooked, "{hung:?}");
}

/// The drydock's routes onto the ships' hulls: the gantry's ramp climbs to the 4 m gantry and a
/// gangway carries a fighter over the hull's shoulder onto the foredeck; the foredeck's ramp
/// climbs onto the 6 m stern deck; the outer lane's pad and a grapple on the bow mast each put a
/// fighter on the foredeck.
#[test]
fn the_drydock_routes_reach_the_hulls() {
    use shooter_sim::drydock::{FOREDECK_TOP, GANTRY_TOP, SHIP_X, STERN_DECK_Z, STERN_TOP};
    let on_foredeck = |sample: &Sample| {
        sample.grounded
            && (sample.center[1] - (FOREDECK_TOP as f32 + HALF)).abs() < STAND
            && (sample.center[0] - SHIP_X as f32).abs() < 400.0
    };
    let samples = run(scenarios::DOCK_GANTRY);
    let gantry = end_of(&samples, "on the gantry").unwrap();
    assert!(
        gantry.grounded
            && (gantry.center[1] - (GANTRY_TOP as f32 + HALF)).abs() < STAND
            && (-1_200.0..=-800.0).contains(&gantry.center[2]),
        "{gantry:?}"
    );
    let deck = end_of(&samples, "on the foredeck").unwrap();
    assert!(on_foredeck(deck), "{deck:?}");

    let samples = run(scenarios::DOCK_STERN);
    let stern = end_of(&samples, "on the stern deck").unwrap();
    assert!(
        stern.grounded
            && (stern.center[1] - (STERN_TOP as f32 + HALF)).abs() < STAND
            && stern.center[2] > STERN_DECK_Z as f32,
        "{stern:?}"
    );

    let samples = run(scenarios::DOCK_PAD);
    assert_eq!(samples.iter().filter(|s| s.launched).count(), 1);
    let landed = end_of(&samples, "on the foredeck").unwrap();
    assert!(on_foredeck(landed), "{landed:?}");

    let samples = run(scenarios::DOCK_GRAPPLE);
    assert!(samples.iter().any(|s| s.hooked));
    let hung = end_of(&samples, "on the foredeck").unwrap();
    assert!(on_foredeck(hung) && !hung.hooked, "{hung:?}");
}

#[test]
fn a_slide_climbs_the_ramp_and_runs_down_it() {
    let samples = run(scenarios::RAMP_SLIDE);
    let deck = end_of(&samples, "on the deck").unwrap();
    assert!(
        deck.grounded && (deck.center[1] - (200.0 + HALF)).abs() < STAND,
        "{deck:?}"
    );
    let foot = end_of(&samples, "at the foot").unwrap();
    assert!(
        foot.grounded && (foot.center[1] - HALF).abs() < STAND,
        "{foot:?}"
    );
    assert!(foot.center[2] > 400.0, "{foot:?}");
}

#[test]
fn a_slide_does_not_pass_a_thin_wall() {
    let samples = run(scenarios::THIN_WALL);
    let stopped = end_of(&samples, "against the wall").unwrap();
    // The wall's north face is at z = -2; the standard robot's guns reach about 35 units ahead
    // of its centre.
    assert!(
        (-70.0..-30.0).contains(&stopped.center[2]) && (stopped.center[1] - HALF).abs() < STAND,
        "{stopped:?}"
    );
}

#[test]
fn a_sprint_climbs_a_step() {
    let samples = run(scenarios::STEP);
    let on_top = end_of(&samples, "on the step").unwrap();
    // Past the step's north face (z = -200) and standing on its 20 cm top.
    assert!(on_top.center[2] > -170.0, "{on_top:?}");
    assert!(
        (on_top.center[1] - (HALF + 20.0)).abs() < STAND,
        "{on_top:?}"
    );
    assert!(on_top.grounded, "{on_top:?}");
}

#[test]
fn the_grapple_reels_onto_a_ledge_no_jump_reaches() {
    let samples = run(scenarios::GRAPPLE);
    let reel = samples
        .iter()
        .filter(|sample| sample.leg == "hook and reel in")
        .collect::<Vec<_>>();
    assert_eq!(
        samples
            .iter()
            .find(|sample| sample.fired.is_some())
            .unwrap()
            .leg,
        "hook and reel in"
    );
    assert!(reel.iter().all(|sample| sample.hooked), "the hook holds");
    // The reel swings the robot up and over the lip (its legs trail over it); it lets go above
    // the ledge's top.
    let over = |sample: &&Sample| sample.center[2] > 300.0;
    let peak = samples
        .iter()
        .filter(over)
        .map(|sample| sample.center[1])
        .fold(0.0, f32::max);
    assert!(peak > 300.0 + HALF, "reeled above the ledge's top: {peak}");
    assert!(reel.iter().any(over), "reeled over the lip");
    let landed = end_of(&samples, "on the ledge").unwrap();
    assert!(!landed.hooked, "{landed:?}");
    assert!(landed.grounded, "{landed:?}");
    assert!(
        (landed.center[1] - (300.0 + HALF)).abs() < STAND,
        "{landed:?}"
    );
    assert!(landed.center[2] > 300.0, "{landed:?}");
}

#[test]
fn every_scenario_replays_exactly() {
    for scenario in scenarios::ALL {
        assert_eq!(run(*scenario), run(*scenario), "{}", scenario.name);
    }
}
