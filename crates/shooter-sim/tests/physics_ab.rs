//! Rapier A/B: the scripted tour through the greybox arena, the greybox yard's walkway (its ramp and
//! pad), the assembly works' ramp, bridge, cover, crane tower pad, work platforms and grapple
//! and every fast-move scenario
//! (sprint, slide, double jump, a thruster-module jump, jump pad, slopes, a thin wall, a step, a rocket jump, a grapple,
//! a shooting range, a close range for the shotgun, laser and flamethrower) on physics-engine and on the Rapier oracle
//! (`shooter_sim::physics::using_backend`), and the mechanics fixtures (shooter#73: rolling,
//! strafing, a stall, turrets, tipping, per-part hits, a walking robo-dog), broken cover
//! (shooter#60: rays across it, a robot on its remains) and a fall's collision damage and debris
//! (shooter#75) on both.
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
        // On one leg the robot rocks on that leg's one foot: the backends part a little more,
        // and more since shooter#74 (the default layout strides that leg on through the sprint
        // and the slide while W is held: about 34 units at worst, against 24 before).
        let tolerance = if lose_leg {
            6.0 * PATH_TOLERANCE_UNITS
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

/// Broken collision (shooter#60): a barrier and low cover broken by the host on both backends.
/// Rays across them meet the same pieces at the same heights (over the remains, through them and
/// at the stubs), and the standard robot set down on each one's remains stands at the same level
/// on both (within 30 units, as on the proving cell's ledges): the broken variant's pieces, not
/// the intact ones, in the scenery's world and the robots' alike.
#[test]
fn broken_cover_collides_alike_on_the_rapier_oracle() {
    use shooter_sim::arena_kit::{self, Module, Placement, Turn};
    use shooter_sim::robot::STANDARD;
    use shooter_sim::scenery::PlaceableState;
    fn arena() -> Arena {
        let placements = vec![
            Placement::new(Module::Barrier, [0, 0, 0], Turn::None).destructible(),
            Placement::new(Module::CoverLow, [600, 0, 0], Turn::Quarter).destructible(),
        ];
        Arena {
            pieces: arena_kit::pieces(&placements),
            placements,
            ..Arena::greybox()
        }
    }
    // Over the barrier's base and through its stub; over the cover's base and through its stub
    // (a quarter turn puts its -x end at +z).
    let rays: [([f32; 3], [f32; 3]); 4] = [
        ([0.0, 150.0, -300.0], [0.0, 0.0, 600.0]),
        ([75.0, 150.0, -300.0], [0.0, 0.0, 600.0]),
        ([300.0, 90.0, -40.0], [600.0, 0.0, 0.0]),
        ([300.0, 90.0, 70.0], [600.0, 0.0, 0.0]),
    ];
    let run = |backend: Backend| {
        using_backend(backend, || {
            let mut session = Session::new(arena()).expect("arena builds");
            session.rebuild(0, STANDARD).unwrap();
            let before: Vec<_> = rays
                .iter()
                .map(|(from, along)| {
                    session
                        .world
                        .cast_ray(*from, *along, |_| true)
                        .map(|hit| hit.body)
                })
                .collect();
            session
                .adopt_scenery(&[
                    PlaceableState { id: 0, health: 0 },
                    PlaceableState { id: 1, health: 0 },
                ])
                .unwrap();
            let after: Vec<_> = rays
                .iter()
                .map(|(from, along)| {
                    session
                        .world
                        .cast_ray(*from, *along, |_| true)
                        .map(|hit| hit.body)
                })
                .collect();
            let mut levels = Vec::new();
            for feet in [[-25.0, 110.0, 0.0], [630.0, 66.0, 30.0]] {
                session.stand(0, feet).unwrap();
                for _ in 0..90 {
                    session.tick(shooter_sim::Intent::IDLE).unwrap();
                }
                levels.push(session.local().feet()[1]);
            }
            (before, after, levels)
        })
    };
    let (engine, rapier) = (run(Backend::Engine), run(Backend::Rapier));
    println!("engine {engine:?}\nrapier {rapier:?}");
    assert_eq!((&engine.0, &engine.1), (&rapier.0, &rapier.1));
    // Intact, every ray meets cover; broken, only the stubs stop them.
    assert!(engine.0.iter().all(Option::is_some));
    assert_eq!(
        engine.1.iter().map(Option::is_some).collect::<Vec<_>>(),
        [false, true, false, true]
    );
    for (a, b) in engine.2.iter().zip(&rapier.2) {
        assert!((a - b).abs() < 30.0, "levels {:?} {:?}", engine.2, rapier.2);
    }
    // On the remains: the barrier's 1 m base and the cover's 56 cm one, not their intact tops.
    assert!((90.0..130.0).contains(&engine.2[0]), "{:?}", engine.2);
    assert!((46.0..80.0).contains(&engine.2[1]), "{:?}", engine.2);
}

/// The proving cell (shooter#53): its ramp, gallery deck and the drop off the gallery's edge; up
/// the stage and down its other ramp; the jump pad and the grapple from the stage onto the
/// gallery. The standard robot stands on the same level on both backends after every route's
/// resting legs
/// (within 30 units: Rapier's servo spring lets its legs give more; the paths themselves part
/// over the ledges, as above), and the pad and the
/// hook act alike.
#[test]
fn the_proving_cell_routes_land_alike_on_the_rapier_oracle() {
    use shooter_sim::robot::STANDARD;
    for scenario in [
        scenarios::CELL_GALLERY,
        scenarios::CELL_STAGE,
        scenarios::CELL_PAD,
        scenarios::CELL_GRAPPLE,
    ] {
        let engine = run_as(Backend::Engine, scenario.arena, scenario.legs, STANDARD);
        let rapier = run_as(Backend::Rapier, scenario.arena, scenario.legs, STANDARD);
        // Where it stands still: the legs that end at rest (a moving leg's end can catch a
        // launch or a landing a tick apart).
        let resting = scenario.legs.iter().filter(|leg| {
            let intent = leg.intent;
            intent.forward == 0
                && intent.strafe == 0
                && !intent.jump
                && !intent.fire
                && !intent.grapple
        });
        for leg in resting {
            let (a, b) = (
                end_of(&engine, leg.name).unwrap(),
                end_of(&rapier, leg.name).unwrap(),
            );
            println!(
                "{} {:<24} engine {:>8.2?} rapier {:>8.2?}",
                scenario.name, leg.name, a.center, b.center
            );
            assert!(
                (a.center[1] - b.center[1]).abs() < 30.0,
                "{}: level after {:?}: {a:?} {b:?}",
                scenario.name,
                leg.name
            );
        }
        let acts = |samples: &[Sample]| {
            (
                samples.iter().filter(|s| s.launched).count(),
                samples.iter().any(|s| s.hooked),
            )
        };
        assert_eq!(acts(&engine), acts(&rapier), "{}", scenario.name);
    }
}

/// The drydock (shooter#55): up the gantry's ramp and over a gangway onto the foredeck, up the
/// stern ramp, the outer lane's pad and a grapple on the bow mast onto the foredeck. As on the
/// proving cell, the standard robot's frame stands on the same level on both backends after
/// every route's resting legs, and the pad and the hook act alike. The frame is the standard
/// build without its roof's tank and generator (shooter#128): on the stern ramp's landing the
/// Rapier oracle tips the robot carrying them where physics-engine lands it upright (which
/// `tests/moves.rs` checks on the shipped build), so the oracle compares the frame both landed
/// alike before them.
#[test]
fn the_drydock_routes_land_alike_on_the_rapier_oracle() {
    use shooter_sim::robot::{BETA, STANDARD};
    let mut frame = STANDARD.candidate();
    frame.parts.retain(|part| part.id < 20);
    let frame = BETA.validate(&frame).unwrap();
    for scenario in [
        scenarios::DOCK_GANTRY,
        scenarios::DOCK_STERN,
        scenarios::DOCK_PAD,
        scenarios::DOCK_GRAPPLE,
    ] {
        let engine = run_as(Backend::Engine, scenario.arena, scenario.legs, frame);
        let rapier = run_as(Backend::Rapier, scenario.arena, scenario.legs, frame);
        let resting = scenario.legs.iter().filter(|leg| {
            let intent = leg.intent;
            intent.forward == 0
                && intent.strafe == 0
                && !intent.jump
                && !intent.fire
                && !intent.grapple
        });
        for leg in resting {
            let (a, b) = (
                end_of(&engine, leg.name).unwrap(),
                end_of(&rapier, leg.name).unwrap(),
            );
            println!(
                "{} {:<24} engine {:>8.2?} rapier {:>8.2?}",
                scenario.name, leg.name, a.center, b.center
            );
            assert!(
                (a.center[1] - b.center[1]).abs() < 30.0,
                "{}: level after {:?}: {a:?} {b:?}",
                scenario.name,
                leg.name
            );
        }
        let acts = |samples: &[Sample]| {
            (
                samples.iter().filter(|s| s.launched).count(),
                samples.iter().any(|s| s.hooked),
            )
        };
        assert_eq!(acts(&engine), acts(&rapier), "{}", scenario.name);
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

    /// The preset layouts (shooter#74) through the same commands on both backends, narrowed
    /// like the routes: W rolls the car ahead and the strafer sideways at speeds within a
    /// tenth, D turns the car the same way on the spot (narrowed to the direction: the wheels
    /// skid round about ten times faster on Rapier, 1.3 rad/s against 0.13), and the mouse's
    /// offset aims the paired turret to the same angles within 0.05 rad.
    #[test]
    fn the_presets_drive_alike_on_the_rapier_oracle() {
        use shooter_sim::bindings::{BindingState, Bindings, Pressed};
        let preset = |pressed: Pressed| {
            move |mechanics: &Mechanics| {
                let layout = &mechanics.robots()[0].layout;
                Bindings::preset(layout)
                    .resolve(layout, &mut BindingState::default(), pressed)
                    .controls(layout)
            }
        };
        let w = Pressed::default().with_key("KeyW");
        for (build, along) in [(fixtures::car(), 0), (fixtures::strafer(), 2)] {
            let (engine, rapier) = both(&floor(), build, 120, preset(w));
            let (ev, rv) = (engine.motion.velocity[along], rapier.motion.velocity[along]);
            assert!(ev > 100.0 && (ev - rv).abs() < 0.1 * rv.abs(), "{ev} {rv}");
        }
        let (engine, rapier) = both(
            &floor(),
            fixtures::car(),
            120,
            preset(Pressed::default().with_key("KeyD")),
        );
        let (ew, rw) = (
            engine.motion.angular_velocity[1],
            rapier.motion.angular_velocity[1],
        );
        assert!(ew * rw > 0.0 && ew.abs() > 0.05, "{ew} {rw}");
        // The offset is the layout's own state: one pushed for 30 ticks, then left.
        let aimed = |mechanics: &Mechanics| {
            let layout = &mechanics.robots()[0].layout;
            let preset = Bindings::preset(layout);
            let mut state = BindingState::default();
            for _ in 0..30 {
                let _ = preset.resolve(
                    layout,
                    &mut state,
                    Pressed {
                        mouse: [-10, -8],
                        ..Pressed::default()
                    },
                );
            }
            preset
                .resolve(layout, &mut state, Pressed::default())
                .controls(layout)
        };
        let (engine, rapier) = both(&floor(), fixtures::paired_turret(), 120, aimed);
        for part in [ROTOR_PART, HINGE_PART] {
            let (e, r) = (angle(&engine, part).0, angle(&rapier, part).0);
            assert!((e - r).abs() < 0.05 && e.abs() > 0.5, "{part:?}: {e} {r}");
        }
    }
}

/// Collision damage and debris (shooter#75), narrowed as the routes above are: a robot standing
/// on a beam with a battery on it falls 30 m onto the floor on each backend; both measure an
/// impact on the same tick (within one), strike the same part (the beam's foot) for damage
/// within a fifth of each other, break it and leave the battery as debris that comes to rest on
/// the floor and retires; and a fall from 8 m does nothing on either. Narrowed: the battery
/// tumbles to rest on another face on each backend (engine about 19 units up, Rapier about 7.5:
/// a 10-unit half extent a little into the floor), so only "on the floor" is compared; it retires
/// after a different time (still at rest a second, or its 5 s lifetime).
#[test]
fn a_fall_and_its_debris_land_alike_on_the_rapier_oracle() {
    use shooter_sim::robot::parts::{BATTERY, BEAM_LONG};
    use shooter_sim::robot::{BETA, Face, PartId, RawMount, RawPart, STANDARD};
    use shooter_sim::{Event, Intent};
    let underside = STANDARD
        .candidate()
        .parts
        .iter()
        .find(|part| part.id == 5)
        .and_then(|part| part.mount)
        .unwrap()
        .socket;
    let mut candidate = STANDARD.candidate();
    candidate.parts.retain(|part| part.id != 5 && part.id != 6);
    candidate.parts.push(RawPart {
        id: 3,
        kind: BEAM_LONG.id.0,
        mount: Some(RawMount {
            parent: 0,
            socket: underside,
            ..RawMount::default()
        }),
        modification: None,
    });
    candidate.parts.push(RawPart {
        id: 4,
        kind: BATTERY.id.0,
        mount: Some(RawMount {
            parent: 3,
            socket: BEAM_LONG.socket(Face::PosX, 0, 0).unwrap().0,
            ..RawMount::default()
        }),
        modification: None,
    });
    let pole = BETA.validate(&candidate).unwrap();
    // (tick of the impact, part, damage), the debris' lowest height, ticks until it retired.
    let fall = |backend, height: f32| {
        using_backend(backend, || {
            let mut session = Session::new(Arena::greybox()).expect("arena builds");
            session.rebuild(0, pole).unwrap();
            session.stand(0, [0.0, height, -800.0]).unwrap();
            let mut struck = Vec::new();
            let mut lowest = f32::MAX;
            let mut retired = None;
            let mut seen = false;
            for tick in 0..600u32 {
                session.tick(Intent::IDLE).unwrap();
                for event in &session.events {
                    if let Event::Hit {
                        attacker: None,
                        part,
                        damage,
                        ..
                    } = *event
                    {
                        struck.push((tick, part, damage));
                    }
                }
                match session.mechanics.debris().first() {
                    Some(piece) => {
                        assert_eq!(piece.part, PartId(4));
                        seen = true;
                        lowest = lowest.min(piece.motion.pose.position[1]);
                    }
                    None if seen && retired.is_none() => retired = Some(tick),
                    None => {}
                }
            }
            (struck, lowest, retired)
        })
    };
    for backend in [Backend::Engine, Backend::Rapier] {
        let (struck, _, _) = fall(backend, 800.0);
        assert!(struck.is_empty(), "{backend:?}: {struck:?}");
    }
    let engine = fall(Backend::Engine, 3_000.0);
    let rapier = fall(Backend::Rapier, 3_000.0);
    println!("engine {engine:?}\nrapier {rapier:?}");
    for (struck, lowest, retired) in [&engine, &rapier] {
        assert_eq!(struck.len(), 1, "{struck:?}");
        assert_eq!(struck[0].1, PartId(3));
        assert!(*lowest > 0.0 && *lowest < 40.0, "{lowest}");
        assert!(retired.is_some());
    }
    let (a, b) = (engine.0[0], rapier.0[0]);
    assert!(a.0.abs_diff(b.0) <= 1, "{a:?} {b:?}");
    assert!((a.2 - b.2).abs() * 5 <= a.2.max(b.2), "{a:?} {b:?}");
}
