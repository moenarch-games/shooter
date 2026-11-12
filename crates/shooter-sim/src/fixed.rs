//! The fixed tick and the fixed-point units the simulation hands physics.

/// The authoritative simulation rate.
pub const TICK_HZ: u16 = 60;

/// Subunits per unit: velocities (per tick) and gravity (per tick²) cross the physics boundary
/// in this fixed point, so 60 Hz speeds and gravity are exact. A power of two: subunit values
/// convert to `f32` units exactly. Positions are `f32` units inside physics and whole units in
/// gameplay rules and on the wire.
pub const SUBUNITS_PER_UNIT: i32 = 256;

/// Subunits per tick² of an acceleration authored in units per second². One that is not a whole
/// number of subunits per tick² fails to compile in const contexts.
#[must_use]
pub const fn subunits_per_tick_squared(units_per_second_squared: i32) -> i32 {
    let subunits = units_per_second_squared * SUBUNITS_PER_UNIT;
    let ticks_squared = TICK_HZ as i32 * TICK_HZ as i32;
    assert!(
        subunits % ticks_squared == 0,
        "acceleration is not a whole number of subunits per tick squared"
    );
    subunits / ticks_squared
}

/// Subunits per tick of a speed authored in units per second. One that is not a whole number of
/// subunits per tick fails to compile in const contexts.
#[must_use]
pub const fn subunits_per_tick(units_per_second: i32) -> i32 {
    let subunits = units_per_second * SUBUNITS_PER_UNIT;
    assert!(
        subunits % TICK_HZ as i32 == 0,
        "speed is not a whole number of subunits per tick"
    );
    subunits / TICK_HZ as i32
}
