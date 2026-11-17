//! Rapier A/B: the scripted tour through the greybox arena, the greybox yard's walkway (its ramp and
//! pad), the assembly works' ramp, bridge, cover, crane tower pad, work platforms and grapple
//! and every fast-move scenario
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
use shooter_sim::robot::{ARMORY, MARKSMAN, SKIRMISHER};
use shooter_sim::scenarios::{self, Scenario};
use shooter_sim::tour::{self, Leg, Sample, end_of};
use shooter_sim::{Arena, Assembly, Session, Weapon};

/// Largest per-axis distance (units) between the two backends' paths at any tick.
const PATH_TOLERANCE_UNITS: f32 = 6.0;

fn run_on(backend: Backend, arena: fn() -> Arena, legs: &[Leg]) -> Vec<Sample> {
    run_as(backend, arena, legs, ARMORY)
}

/// [`run_on`] with the local fighter built as `build` (its weapon mods).
fn run_as(backend: Backend, arena: fn() -> Arena, legs: &[Leg], build: Assembly) -> Vec<Sample> {
    using_backend(backend, || {
        let mut session = Session::new(arena()).expect("arena builds");
        assert_eq!(session.world.backend(), backend);
        session.rebuild(0, build).unwrap();
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
    compare_as(scenario, ARMORY);
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
fn the_shooting_range_matches_the_rapier_oracle() {
    compare(scenarios::SHOOTING_RANGE);
}

#[test]
fn the_close_range_matches_the_rapier_oracle() {
    compare(scenarios::CLOSE_RANGE);
}

/// The match's mechanical robots (shooter#73) on flat ground: the standard robot sprints,
/// slides and stops along the same path on both backends, whole and on one leg (the other shot
/// off: it no longer weighs or holds anything up, and the robot goes at half speed). The
/// scripted routes over ledges, steps and pads are checked on physics-engine (`tests/moves.rs`)
/// and replay exactly on each backend: robots' legs catching a lip, and Rapier's servo spring
/// under the heavier builds, part the two backends' paths there.
#[test]
fn flat_ground_moves_match_the_rapier_oracle() {
    let scenario = scenarios::SPRINT_AND_SLIDE;
    for lose_leg in [false, true] {
        let run = |backend| {
            using_backend(backend, || {
                let mut session = Session::new((scenario.arena)()).expect("arena builds");
                if lose_leg {
                    let fighter = &mut session.fighters[0];
                    let build = fighter.build;
                    let leg = build
                        .parts()
                        .iter()
                        .position(|part| part.kind == shooter_sim::robot::parts::LEG.id)
                        .expect("the standard robot has legs");
                    fighter.parts.hurt(&build, leg, 10_000);
                }
                tour::run_legs(&mut session, scenario.legs).expect("script runs")
            })
        };
        let (engine, rapier) = (run(Backend::Engine), run(Backend::Rapier));
        let worst = engine
            .iter()
            .zip(&rapier)
            .flat_map(|(a, b)| (0..3).map(move |axis| (a.center[axis] - b.center[axis]).abs()))
            .fold(0.0f32, f32::max);
        println!("one leg lost {lose_leg}: worst path gap {worst:.3} units");
        // On one leg the robot rocks on that leg's one foot: the backends part a little more.
        let tolerance = if lose_leg {
            4.0 * PATH_TOLERANCE_UNITS
        } else {
            PATH_TOLERANCE_UNITS
        };
        assert!(worst <= tolerance, "one leg lost {lose_leg}: {worst}");
        let travelled = |samples: &[Sample]| {
            let (first, last) = (samples[0].center, samples[samples.len() - 1].center);
            (last[2] - first[2]).abs()
        };
        if lose_leg {
            let whole = using_backend(Backend::Engine, || {
                let mut session = Session::new((scenario.arena)()).expect("arena builds");
                tour::run_legs(&mut session, scenario.legs).expect("script runs")
            });
            assert!(
                travelled(&engine) < 0.75 * travelled(&whole),
                "{} {}",
                travelled(&engine),
                travelled(&whole)
            );
        }
    }
}

/// The match's twelve slots, every one a bot's mechanical robot, on the default map: the mean
/// wall time a session tick takes on each backend; no threshold
/// (`--release -- --ignored --nocapture`).
#[test]
#[ignore = "a timing run, not a check"]
fn twelve_robots_in_a_match_a_tick() {
    for backend in [Backend::Engine, Backend::Rapier] {
        using_backend(backend, || {
            let mut session = Session::on_map(
                shooter_sim::MapId::DEFAULT,
                shooter_sim::Rules::deathmatch(7),
            )
            .expect("map builds");
            for _ in 0..60 {
                session.step().unwrap();
            }
            let started = std::time::Instant::now();
            for _ in 0..600 {
                session.step().unwrap();
            }
            println!(
                "{backend:?}: {} robots in the match: {:.1} µs a tick",
                session.mechanics.robots().len(),
                started.elapsed().as_secs_f64() * 1e6 / 600.0
            );
        });
    }
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
            combat(scenarios::SHOOTING_RANGE, ARMORY),
            "{build:?} changes the fire"
        );
    }
    compare_as(scenarios::CLOSE_RANGE, SKIRMISHER);
    // The efficiency coil slows the shotgun's blasts.
    assert_ne!(
        combat(scenarios::CLOSE_RANGE, SKIRMISHER),
        combat(scenarios::CLOSE_RANGE, ARMORY)
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
