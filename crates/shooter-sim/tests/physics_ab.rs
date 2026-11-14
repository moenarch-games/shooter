//! Rapier A/B: the scripted tour through the greybox arena and every fast-move scenario
//! (sprint, slide, double jump, a thruster-module jump, jump pad, slopes, a thin wall, a step, a rocket jump, a grapple,
//! a shooting range, a close range for the shotgun, laser and flamethrower) on physics-engine and on the Rapier oracle
//! (`shooter_sim::physics::using_backend`), and the mechanics fixtures (shooter#73: rolling,
//! strafing, a stall, turrets, tipping, per-part hits, a walking robo-dog) on both.
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
        session.fighters[0].rebuild(build);
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

mod mechanics {
    //! The mechanics fixtures (shooter#73) on physics-engine and on the Rapier oracle (one
    //! dynamic body with a collider a part, impulse revolute joints with motors): the same
    //! rolling, strafing and stall speeds and stops, turret angles, tipping and per-part hits,
    //! within the tolerances physics-engine's own `revolute-robots` oracle holds.

    use shooter_sim::mechanics::fixtures::{
        self, BARREL, HINGE_PART, ROTOR_PART, binary, floor, walled, wheels,
    };
    use shooter_sim::mechanics::{Command, Controls, Mechanics};
    use shooter_sim::physics::articulated::{Motion, Pose};
    use shooter_sim::physics::{Backend, RigidBody, using_backend};
    use shooter_sim::robot::{Assembly, PartId};

    struct Outcome {
        root: Pose,
        motion: Motion,
        /// Each actuator's angle and rate.
        actuators: Vec<(PartId, f32, f32)>,
        mechanics: Mechanics,
    }

    fn run(
        backend: Backend,
        statics: &[RigidBody],
        build: Assembly,
        ticks: u32,
        controls: impl Fn(&Mechanics) -> Controls,
    ) -> Outcome {
        using_backend(backend, || {
            let mut mechanics = Mechanics::new(statics).unwrap();
            assert_eq!(mechanics.world().backend(), backend);
            mechanics.spawn(build, [0.0; 3], 0).unwrap();
            for _ in 0..ticks {
                let controls = controls(&mechanics);
                mechanics.step(&[controls]).unwrap();
            }
            Outcome {
                root: mechanics.root(0).unwrap(),
                motion: mechanics.root_motion(0).unwrap(),
                actuators: mechanics.robots()[0]
                    .actuators
                    .iter()
                    .map(|actuator| (actuator.part, actuator.angle, actuator.rate))
                    .collect(),
                mechanics,
            }
        })
    }

    fn both(
        statics: &[RigidBody],
        build: Assembly,
        ticks: u32,
        controls: impl Fn(&Mechanics) -> Controls + Copy,
    ) -> (Outcome, Outcome) {
        (
            run(Backend::Engine, statics, build, ticks, controls),
            run(Backend::Rapier, statics, build, ticks, controls),
        )
    }

    fn up(pose: Pose) -> [f32; 3] {
        let [x, y, z, w] = pose.orientation;
        [
            2.0 * (x * y - z * w),
            1.0 - 2.0 * (x * x + z * z),
            2.0 * (y * z + x * w),
        ]
    }

    fn gap(a: [f32; 3], b: [f32; 3]) -> f32 {
        a.iter()
            .zip(&b)
            .map(|(a, b)| (a - b) * (a - b))
            .sum::<f32>()
            .sqrt()
    }

    fn angle(outcome: &Outcome, part: PartId) -> (f32, f32) {
        let &(_, angle, rate) = outcome
            .actuators
            .iter()
            .find(|(id, ..)| *id == part)
            .unwrap();
        (angle, rate)
    }

    #[test]
    fn rolling_and_strafing_match_the_rapier_oracle() {
        for (build, along, across) in [(fixtures::car(), 0, 2), (fixtures::strafer(), 2, 0)] {
            let (engine, rapier) = both(&floor(), build, 120, |_| wheels(6_000, -6_000));
            let (ev, rv) = (engine.motion.velocity[along], rapier.motion.velocity[along]);
            assert!(
                ev * rv > 0.0 && (ev - rv).abs() < 0.1 * rv.abs(),
                "{ev} {rv}"
            );
            assert!(rapier.motion.velocity[across].abs() < 5.0);
            assert!(gap(engine.root.position, rapier.root.position) < 25.0);
            assert!(up(engine.root)[1] > 0.99 && up(rapier.root)[1] > 0.99);
        }
    }

    #[test]
    fn the_stall_matches_the_rapier_oracle() {
        let (engine, rapier) = both(&walled(80), fixtures::car(), 180, |_| wheels(6_000, -6_000));
        for outcome in [&engine, &rapier] {
            let speed = gap(outcome.motion.velocity, [0.0; 3]);
            assert!(speed < 5.0, "{speed}");
            for &(_, _, rate) in &outcome.actuators {
                assert!(rate.abs() < 1.0, "{rate}");
            }
        }
        assert!((engine.root.position[0] - rapier.root.position[0]).abs() < 5.0);
    }

    #[test]
    fn the_turrets_match_the_rapier_oracle() {
        for target in [1.0, -2.0] {
            let (engine, rapier) = both(&floor(), fixtures::turret(), 120, |_| {
                wheels(0, 0).with(ROTOR_PART, Command::Angle(binary(target)))
            });
            for outcome in [&engine, &rapier] {
                let (angle, _) = angle(outcome, ROTOR_PART);
                assert!((f64::from(angle) - target).abs() < 0.02, "{angle} {target}");
            }
        }
        let (engine, rapier) = both(&floor(), fixtures::paired_turret(), 120, |_| {
            wheels(0, 0)
                .with(ROTOR_PART, Command::Rate(1_500))
                .with(HINGE_PART, Command::Angle(binary(0.5)))
        });
        for outcome in [&engine, &rapier] {
            let (_, rate) = angle(outcome, ROTOR_PART);
            let (pitch, _) = angle(outcome, HINGE_PART);
            assert!((rate - 1.5).abs() < 0.15, "{rate}");
            assert!((pitch - 0.5).abs() < 0.02, "{pitch}");
        }
    }

    #[test]
    fn tipping_and_part_hits_match_the_rapier_oracle() {
        let (engine, rapier) = both(&floor(), fixtures::lopsided(), 120, |_| Controls::IDLE);
        for outcome in [&engine, &rapier] {
            let up = up(outcome.root);
            assert!(up[1] < 0.9 && up[2] < -0.4, "{up:?}");
        }
        let (engine, rapier) = both(&floor(), fixtures::balanced(), 120, |_| Controls::IDLE);
        assert!(up(engine.root)[1] > 0.999 && up(rapier.root)[1] > 0.999);

        let (engine, rapier) = both(&floor(), fixtures::turret(), 30, |_| wheels(0, 0));
        for (from, along) in [
            ([-300.0, 140.0, 10.0], [600.0, 0.0, 0.0]),
            ([10.0, 400.0, 0.0], [0.0, -500.0, 0.0]),
            ([-20.0, 18.0, -200.0], [0.0, 0.0, 400.0]),
        ] {
            let parts = [&engine, &rapier]
                .map(|outcome| outcome.mechanics.cast_ray(from, along).map(|hit| hit.part));
            assert_eq!(parts[0], parts[1], "{from:?}");
            assert!(parts[0].is_some(), "{from:?}");
        }
        let barrel = engine
            .mechanics
            .cast_ray([10.0, 400.0, 0.0], [0.0, -500.0, 0.0])
            .unwrap();
        assert_eq!(barrel.part, BARREL);
    }

    /// Twelve robots (the match size) at once: three walking robo-dogs, three rolling paired
    /// turrets aiming, three strafers and three bipeds, 600 ticks on each backend. Prints the
    /// mean wall time a tick; no threshold (`--release -- --ignored --nocapture`).
    #[test]
    #[ignore = "a timing run, not a check"]
    fn twelve_robots_a_tick() {
        let builds = [
            fixtures::dog(false),
            fixtures::paired_turret(),
            fixtures::strafer(),
            fixtures::biped(),
        ];
        for backend in [Backend::Engine, Backend::Rapier] {
            using_backend(backend, || {
                let mut mechanics = Mechanics::new(&floor()).unwrap();
                for index in 0..12u8 {
                    let at = [
                        f32::from(index % 4) * 400.0,
                        0.0,
                        f32::from(index / 4) * 400.0,
                    ];
                    mechanics
                        .spawn(builds[usize::from(index % 4)], at, 0)
                        .unwrap();
                }
                let started = std::time::Instant::now();
                for tick in 0..600u32 {
                    let rate = i16::try_from((tick % 120) * 50).unwrap();
                    let controls: Vec<Controls> = mechanics
                        .robots()
                        .iter()
                        .map(|robot| {
                            if robot.layout.suggested_gait().is_some() {
                                Controls::IDLE.walk(&robot.layout, 100, 0)
                            } else {
                                wheels(rate, -rate)
                                    .with(ROTOR_PART, Command::Rate(1_000))
                                    .with(HINGE_PART, Command::Angle(binary(0.4)))
                            }
                        })
                        .collect();
                    mechanics.step(&controls).unwrap();
                }
                let joints: usize = mechanics
                    .robots()
                    .iter()
                    .map(|robot| robot.layout.hinges.len())
                    .sum();
                println!(
                    "{backend:?}: 12 robots, {joints} joints: {:.1} µs a tick",
                    started.elapsed().as_secs_f64() * 1e6 / 600.0
                );
            });
        }
    }

    #[test]
    fn the_quadruped_walks_forward_on_both() {
        let (engine, rapier) = both(&floor(), fixtures::dog(false), 270, |mechanics| {
            Controls::IDLE.walk(&mechanics.robots()[0].layout, 100, 0)
        });
        for outcome in [&engine, &rapier] {
            assert!(outcome.root.position[0] > 100.0, "{:?}", outcome.root);
            assert!(up(outcome.root)[1] > 0.8, "{:?}", outcome.root);
        }
    }
}
