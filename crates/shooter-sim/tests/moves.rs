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
    assert!((sprint.last().unwrap() - 17.5).abs() < 0.1, "{sprint:?}");
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
    assert!((slide.last().unwrap() - 17.5).abs() < 0.1, "{slide:?}");
    assert!(samples.iter().any(|s| s.sliding));
    assert!(!end_of(&samples, "slide").unwrap().sliding);
}

#[test]
fn a_double_jump_reaches_a_ledge_one_jump_cannot() {
    let samples = run(scenarios::DOUBLE_JUMP);
    assert_eq!(samples.iter().filter(|s| s.air_jumped).count(), 1);
    let top = end_of(&samples, "on the ledge").unwrap();
    assert!(top.grounded, "{top:?}");
    assert!((top.center[1] - (160.0 + HALF)).abs() < 2.0, "{top:?}");
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
        (landed.center[1] - (200.0 + HALF)).abs() < 2.0,
        "{landed:?}"
    );
    assert!(
        landed.center[0].abs() < 400.0 && landed.center[2].abs() < 400.0,
        "{landed:?}"
    );
}

#[test]
fn a_slide_climbs_the_ramp_and_runs_down_it() {
    let samples = run(scenarios::RAMP_SLIDE);
    let deck = end_of(&samples, "on the deck").unwrap();
    assert!(
        deck.grounded && (deck.center[1] - (200.0 + HALF)).abs() < 2.0,
        "{deck:?}"
    );
    let foot = end_of(&samples, "at the foot").unwrap();
    assert!(
        foot.grounded && (foot.center[1] - HALF).abs() < 2.0,
        "{foot:?}"
    );
    assert!(foot.center[2] > 400.0, "{foot:?}");
}

#[test]
fn a_slide_does_not_pass_a_thin_wall() {
    let samples = run(scenarios::THIN_WALL);
    let stopped = end_of(&samples, "against the wall").unwrap();
    // The wall's north face is at z = -2; the capsule's radius is 30.
    assert!((stopped.center[2] + 32.0).abs() < 3.0, "{stopped:?}");
}

#[test]
fn a_sprint_stops_at_a_step_without_step_up() {
    let samples = run(scenarios::STEP);
    let stopped = end_of(&samples, "sprint at the step").unwrap();
    assert!((stopped.center[2] + 230.0).abs() < 3.0, "{stopped:?}");
    assert!((stopped.center[1] - HALF).abs() < 2.0, "{stopped:?}");
}

#[test]
fn every_scenario_replays_exactly() {
    for scenario in scenarios::ALL {
        assert_eq!(run(*scenario), run(*scenario), "{}", scenario.name);
    }
}
