//! The scripted tour on physics-engine: the player climbs the ramp, jumps, descends and stops at
//! the wall.

use shooter_sim::player::{PLAYER_HALF_EXTENTS, WALK_SPEED_SUBUNITS_PER_TICK, walk_velocity};
use shooter_sim::tour::{self, end_of};
use shooter_sim::{Arena, Intent, Session};

fn tour() -> Vec<tour::Sample> {
    let mut session = Session::new(Arena::greybox()).expect("arena builds");
    tour::run(&mut session).expect("tour runs")
}

#[test]
fn walking_reaches_the_platform_and_the_wall() {
    let samples = tour();
    let half = PLAYER_HALF_EXTENTS.y as f32;

    let on_platform = end_of(&samples, "on the platform").unwrap();
    assert!(on_platform.grounded, "{on_platform:?}");
    assert!(
        (on_platform.center[1] - (200.0 + half)).abs() < 2.0,
        "{on_platform:?}"
    );

    let peak = samples
        .iter()
        .filter(|s| s.leg == "airborne and landing")
        .map(|s| s.center[1])
        .fold(f32::MIN, f32::max);
    assert!(peak > 200.0 + half + 80.0, "jump peak {peak}");
    let landed = end_of(&samples, "airborne and landing").unwrap();
    assert!(landed.grounded, "{landed:?}");

    let wall = end_of(&samples, "into the west wall").unwrap();
    assert!(
        wall.grounded && (wall.center[1] - half).abs() < 2.0,
        "{wall:?}"
    );
    // The wall's face is at x = -2000; the capsule's radius is 30.
    assert!((wall.center[0] + 1970.0).abs() < 3.0, "{wall:?}");

    // The 80 cm crate at (-12 m, 12 m) blocks walking and a running jump clears it.
    let blocked = end_of(&samples, "against the crate").unwrap();
    assert!((blocked.center[2] - 1130.0).abs() < 3.0, "{blocked:?}");
    let crate_top = end_of(&samples, "on the crate").unwrap();
    assert!(crate_top.grounded, "{crate_top:?}");
    assert!(
        (crate_top.center[1] - (80.0 + half)).abs() < 2.0,
        "{crate_top:?}"
    );
}

#[test]
fn the_tour_replays_exactly() {
    assert_eq!(tour(), tour());
}

#[test]
fn diagonal_walking_is_no_faster() {
    let straight = walk_velocity(Intent {
        forward: 1,
        ..Intent::default()
    });
    let diagonal = walk_velocity(Intent {
        forward: 1,
        strafe: 1,
        ..Intent::default()
    });
    let length = |[x, z]: [i32; 2]| f64::from(x).hypot(f64::from(z));
    assert_eq!(straight, [0, WALK_SPEED_SUBUNITS_PER_TICK]);
    assert!((length(diagonal) - length(straight)).abs() <= 1.0);
    // Right of yaw 0 (+Z) is -X.
    let right = walk_velocity(Intent {
        strafe: 1,
        ..Intent::default()
    });
    assert_eq!(right, [-WALK_SPEED_SUBUNITS_PER_TICK, 0]);
}
