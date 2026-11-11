//! Rapier A/B: the scripted tour through the greybox arena on physics-engine and on the Rapier
//! oracle (`sim_foundation::physics::using_backend`). Both must follow the same path within a few
//! centimetres and agree on support at the end of every leg.
//!
//! `cargo test -p shooter-sim --features physics-ab --test physics_ab`

use shooter_sim::sim_foundation::physics::{Backend, using_backend};
use shooter_sim::tour::{self, LEGS, Sample, end_of};
use shooter_sim::{Arena, Session};

/// Largest per-axis distance (units) between the two backends' paths at any tick.
const PATH_TOLERANCE_UNITS: f32 = 6.0;

fn tour_on(backend: Backend) -> Vec<Sample> {
    using_backend(backend, || {
        let mut session = Session::new(Arena::greybox()).expect("arena builds");
        assert_eq!(session.world.backend(), backend);
        tour::run(&mut session).expect("tour runs")
    })
}

#[test]
fn the_tour_matches_the_rapier_oracle() {
    let engine = tour_on(Backend::Engine);
    let rapier = tour_on(Backend::Rapier);
    assert_eq!(engine.len(), rapier.len());

    let mut worst = (0.0f32, 0u32);
    for (a, b) in engine.iter().zip(&rapier) {
        for axis in 0..3 {
            let gap = (a.center[axis] - b.center[axis]).abs();
            if gap > worst.0 {
                worst = (gap, a.tick);
            }
        }
    }
    println!("worst path gap {:.3} units at tick {}", worst.0, worst.1);
    assert!(
        worst.0 <= PATH_TOLERANCE_UNITS,
        "paths diverge by {:.3} units at tick {}: engine {:?} rapier {:?}",
        worst.0,
        worst.1,
        engine[worst.1 as usize - 1],
        rapier[worst.1 as usize - 1],
    );

    for leg in LEGS {
        let (a, b) = (
            end_of(&engine, leg.name).unwrap(),
            end_of(&rapier, leg.name).unwrap(),
        );
        println!(
            "{:<24} engine {:>8.2?} rapier {:>8.2?} grounded {}",
            leg.name, a.center, b.center, a.grounded
        );
        assert_eq!(a.grounded, b.grounded, "support after {:?}", leg.name);
    }
}

#[test]
fn each_backend_replays_exactly() {
    for backend in [Backend::Engine, Backend::Rapier] {
        assert_eq!(tour_on(backend), tour_on(backend), "{backend:?}");
    }
}
