//! Rapier A/B: the scripted tour through the greybox arena and every fast-move scenario
//! (sprint, slide, double jump, a thruster-module jump, jump pad, slopes, a thin wall, a step, a rocket jump, a grapple,
//! a shooting range, a close range for the shotgun, laser and flamethrower) on physics-engine and on the Rapier oracle
//! (`shooter_sim::physics::using_backend`).
//! Both must follow the same path within a few centimetres, agree on support at the end of every
//! leg, and fire, hit and explode on the same ticks for the same damage (rays and the rocket and
//! pellet sweeps are the backends' `cast_ray`, the flame their `overlap_box`).
//!
//! `cargo test -p shooter-sim --features physics-ab --test physics_ab`

use shooter_sim::physics::{Backend, using_backend};
use shooter_sim::robot::{MARKSMAN, SKIRMISHER, STANDARD};
use shooter_sim::scenarios::{self, Scenario};
use shooter_sim::tour::{self, Leg, Sample, end_of};
use shooter_sim::{Arena, Assembly, Session, Weapon};

/// Largest per-axis distance (units) between the two backends' paths at any tick.
const PATH_TOLERANCE_UNITS: f32 = 6.0;

fn run_on(backend: Backend, arena: fn() -> Arena, legs: &[Leg]) -> Vec<Sample> {
    run_as(backend, arena, legs, STANDARD)
}

/// [`run_on`] with the local fighter built as `build` (its weapon mods).
fn run_as(backend: Backend, arena: fn() -> Arena, legs: &[Leg], build: Assembly) -> Vec<Sample> {
    using_backend(backend, || {
        let mut session = Session::new(arena()).expect("arena builds");
        assert_eq!(session.world.backend(), backend);
        session.fighters[0].build = build;
        tour::run_legs(&mut session, legs).expect("script runs")
    })
}

fn tour_scenario() -> Scenario {
    Scenario {
        name: "tour",
        arena: Arena::greybox,
        legs: tour::LEGS,
    }
}

fn compare(scenario: Scenario) {
    compare_as(scenario, STANDARD);
}

fn compare_as(scenario: Scenario, build: Assembly) {
    let engine = run_as(Backend::Engine, scenario.arena, scenario.legs, build);
    let rapier = run_as(Backend::Rapier, scenario.arena, scenario.legs, build);
    assert_eq!(engine.len(), rapier.len());
    let name = scenario.name;

    let mut worst = (0.0f32, 0u32);
    for (a, b) in engine.iter().zip(&rapier) {
        for axis in 0..3 {
            let gap = (a.center[axis] - b.center[axis]).abs();
            if gap > worst.0 {
                worst = (gap, a.tick);
            }
        }
    }
    println!(
        "{name}: worst path gap {:.3} units at tick {}",
        worst.0, worst.1
    );
    assert!(
        worst.0 <= PATH_TOLERANCE_UNITS,
        "{name}: paths diverge by {:.3} units at tick {}: engine {:?} rapier {:?}",
        worst.0,
        worst.1,
        engine[worst.1 as usize - 1],
        rapier[worst.1 as usize - 1],
    );

    for leg in scenario.legs {
        let (a, b) = (
            end_of(&engine, leg.name).unwrap(),
            end_of(&rapier, leg.name).unwrap(),
        );
        println!(
            "  {:<24} engine {:>8.2?} rapier {:>8.2?} grounded {}",
            leg.name, a.center, b.center, a.grounded
        );
        assert_eq!(
            a.grounded, b.grounded,
            "{name}: support after {:?}",
            leg.name
        );
        assert_eq!(a.sliding, b.sliding, "{name}: sliding after {:?}", leg.name);
    }
    let events = |samples: &[Sample]| {
        samples
            .iter()
            .filter(|s| s.launched || s.air_jumped)
            .map(|s| s.tick)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        events(&engine),
        events(&rapier),
        "{name}: pad and air-jump ticks"
    );
    // Shots, hits and blasts: the same weapon fires on the same tick and every ray or rocket
    // deals the same damage on the same tick.
    let combat = |samples: &[Sample]| {
        samples
            .iter()
            .filter(|s| s.fired.is_some() || s.damage > 0 || s.exploded)
            .map(|s| (s.tick, s.fired, s.damage, s.exploded))
            .collect::<Vec<_>>()
    };
    assert_eq!(combat(&engine), combat(&rapier), "{name}: shots and hits");
    // The grapple's hook sets and lets go on the same ticks.
    let hooked = |samples: &[Sample]| samples.iter().map(|s| s.hooked).collect::<Vec<_>>();
    assert_eq!(hooked(&engine), hooked(&rapier), "{name}: hook");
}

#[test]
fn the_tour_matches_the_rapier_oracle() {
    compare(tour_scenario());
}

#[test]
fn sprint_and_slide_match_the_rapier_oracle() {
    compare(scenarios::SPRINT_AND_SLIDE);
}

#[test]
fn the_double_jump_matches_the_rapier_oracle() {
    compare(scenarios::DOUBLE_JUMP);
}

/// The thrusters module (`shooter_sim::pickups`), taken from a pad, then a 2.25 m jump.
#[test]
fn the_thruster_jump_matches_the_rapier_oracle() {
    compare(scenarios::THRUSTER_JUMP);
}

#[test]
fn the_jump_pad_matches_the_rapier_oracle() {
    compare(scenarios::JUMP_PAD);
}

#[test]
fn ramp_slides_match_the_rapier_oracle() {
    compare(scenarios::RAMP_SLIDE);
}

#[test]
fn the_thin_wall_matches_the_rapier_oracle() {
    compare(scenarios::THIN_WALL);
}

#[test]
fn the_step_matches_the_rapier_oracle() {
    compare(scenarios::STEP);
}

#[test]
fn the_rocket_jump_matches_the_rapier_oracle() {
    compare(scenarios::ROCKET_JUMP);
}

#[test]
fn the_shooting_range_matches_the_rapier_oracle() {
    compare(scenarios::SHOOTING_RANGE);
}

#[test]
fn the_grapple_matches_the_rapier_oracle() {
    compare(scenarios::GRAPPLE);
}

#[test]
fn the_close_range_matches_the_rapier_oracle() {
    compare(scenarios::CLOSE_RANGE);
}

/// The shots and hits of `scenario` on physics-engine as `build`.
fn combat(scenario: Scenario, build: Assembly) -> Vec<(u32, Option<Weapon>, i32)> {
    run_as(Backend::Engine, scenario.arena, scenario.legs, build)
        .iter()
        .filter(|s| s.fired.is_some() || s.damage > 0)
        .map(|s| (s.tick, s.fired, s.damage))
        .collect()
}

/// Weapon mods on both backends: the skirmisher's rapid-cycler rifle and the marksman's
/// long-barrel rifle and overcharged rail gun on the shooting range, the skirmisher's
/// efficiency-coiled shotgun on the close range. They fire and hit alike on both, and differently
/// from the shipped weapons.
#[test]
fn modded_loadouts_match_the_rapier_oracle() {
    for build in [SKIRMISHER, MARKSMAN] {
        compare_as(scenarios::SHOOTING_RANGE, build);
        assert_ne!(
            combat(scenarios::SHOOTING_RANGE, build),
            combat(scenarios::SHOOTING_RANGE, STANDARD),
            "{build:?} changes the fire"
        );
    }
    compare_as(scenarios::CLOSE_RANGE, SKIRMISHER);
    // The efficiency coil slows the shotgun's blasts.
    assert_ne!(
        combat(scenarios::CLOSE_RANGE, SKIRMISHER),
        combat(scenarios::CLOSE_RANGE, STANDARD)
    );
}

#[test]
fn each_backend_replays_exactly() {
    let all = std::iter::once(tour_scenario()).chain(scenarios::ALL.iter().copied());
    for scenario in all {
        for backend in [Backend::Engine, Backend::Rapier] {
            assert_eq!(
                run_on(backend, scenario.arena, scenario.legs),
                run_on(backend, scenario.arena, scenario.legs),
                "{} on {backend:?}",
                scenario.name
            );
        }
    }
}
