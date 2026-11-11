//! Weapon effects as render-engine lights and particles: muzzle flashes, beam trails, impact
//! sparks, rocket exhaust and explosions. The simulation's [`Event`]s start them; they live on
//! their own clock (presentation only, never fed back into the simulation).

use crate::scene::{METRES_PER_UNIT, gun_pose};
use render_engine::glam::{Mat4, Quat, Vec2, Vec3, Vec4};
use render_engine::particles::{
    ColorBlend, ColorOverLife, Emitter, Gradient, InitModifier, Orient, ParticleBlend,
    ShapeDimension, Spawner, Value,
};
use render_engine::scene::PointLight;
use render_engine::world::{EmitterId, World};
use shooter_sim::wire::RocketState;
use shooter_sim::{Event, Weapon};

/// Rockets trail exhaust from this many ticks after launch (about 1.6 m out).
const EXHAUST_AFTER_TICKS: u32 = 4;

/// A weapon's energy colour (linear, roughly unit brightness).
#[must_use]
pub const fn tint(weapon: Weapon) -> Vec3 {
    match weapon {
        Weapon::EnergyRifle => Vec3::new(0.25, 0.8, 1.0),
        Weapon::RailGun => Vec3::new(0.75, 0.35, 1.0),
        Weapon::RocketLauncher => Vec3::new(1.0, 0.55, 0.15),
    }
}

fn units(value: [f32; 3]) -> Vec3 {
    Vec3::from(value) * METRES_PER_UNIT
}

/// A transform whose +Z points along `direction`, at `position`.
fn facing(position: Vec3, direction: Vec3) -> Mat4 {
    let rotation = Quat::from_rotation_arc(Vec3::Z, direction.normalize_or(Vec3::Z));
    Mat4::from_rotation_translation(rotation, position)
}

fn fade(color: Vec3, brightness: f32) -> ColorOverLife {
    let lit = (color * brightness).extend(1.0);
    ColorOverLife {
        gradient: Gradient::new(vec![(0.0, lit), (1.0, (color * brightness).extend(0.0))]),
        blend: ColorBlend::Overwrite,
    }
}

/// A burst-only emitter (no spawner of its own) with additive, round, unlit particles.
fn burst_emitter(capacity: u32, seed: u32, init: Vec<InitModifier>) -> Emitter {
    Emitter {
        capacity,
        spawner: Spawner::once(0.0),
        seed,
        init,
        accel: Vec3::ZERO,
        size_over_life: None,
        color_over_life: None,
        orient: Orient::CameraPlane,
        texture: None,
        roundness: 1.0,
        depth_fade: 0.0,
        lighting: None,
        fog: false,
        blend: ParticleBlend::Add,
        active: false,
    }
}

fn sizes(from: f32, to: f32) -> Option<Gradient<Vec2>> {
    Some(Gradient::new(vec![
        (0.0, Vec2::splat(from)),
        (1.0, Vec2::splat(to)),
    ]))
}

/// Sparks thrown along the transform's +Z.
fn sparks(color: Vec3, capacity: u32, seed: u32, spread: f32, speed: [f32; 2]) -> Emitter {
    Emitter {
        accel: Vec3::new(0.0, -9.8, 0.0),
        size_over_life: Some(Gradient::new(vec![
            (0.0, Vec2::new(0.12, 0.025)),
            (1.0, Vec2::new(0.03, 0.01)),
        ])),
        color_over_life: Some(fade(color, 6.0)),
        orient: Orient::AlongVelocity,
        ..burst_emitter(
            capacity,
            seed,
            vec![
                InitModifier::PositionSphere {
                    center: Vec3::ZERO,
                    radius: 0.02,
                    dimension: ShapeDimension::Volume,
                },
                InitModifier::VelocityCone {
                    direction: Vec3::Z,
                    spread,
                    speed: Value::Uniform {
                        min: speed[0],
                        max: speed[1],
                    },
                },
                InitModifier::Lifetime(Value::Uniform {
                    min: 0.12,
                    max: 0.35,
                }),
            ],
        )
    }
}

/// Glowing motes left along a beam, one per burst.
fn trail(color: Vec3, capacity: u32, seed: u32, size: f32, life: f32) -> Emitter {
    Emitter {
        size_over_life: sizes(size, size * 0.3),
        color_over_life: Some(fade(color, 4.0)),
        ..burst_emitter(
            capacity,
            seed,
            vec![
                InitModifier::Velocity(Value::Uniform {
                    min: Vec3::splat(-0.08),
                    max: Vec3::splat(0.08),
                }),
                InitModifier::Lifetime(Value::Const(life)),
            ],
        )
    }
}

fn fireball(seed: u32) -> Emitter {
    Emitter {
        accel: Vec3::new(0.0, 1.5, 0.0),
        depth_fade: 0.3,
        size_over_life: sizes(0.7, 0.25),
        color_over_life: Some(ColorOverLife {
            gradient: Gradient::new(vec![
                (0.0, Vec4::new(8.0, 4.0, 1.2, 1.0)),
                (0.4, Vec4::new(4.0, 1.2, 0.2, 0.8)),
                (1.0, Vec4::new(0.6, 0.1, 0.0, 0.0)),
            ]),
            blend: ColorBlend::Overwrite,
        }),
        ..burst_emitter(
            1_024,
            seed,
            vec![
                InitModifier::PositionSphere {
                    center: Vec3::ZERO,
                    radius: 0.35,
                    dimension: ShapeDimension::Volume,
                },
                InitModifier::VelocitySphere {
                    center: Vec3::ZERO,
                    speed: Value::Uniform { min: 2.0, max: 7.0 },
                },
                InitModifier::Lifetime(Value::Uniform { min: 0.3, max: 0.6 }),
            ],
        )
    }
}

fn exhaust(seed: u32) -> Emitter {
    Emitter {
        accel: Vec3::new(0.0, 0.6, 0.0),
        size_over_life: sizes(0.12, 0.45),
        color_over_life: Some(ColorOverLife {
            gradient: Gradient::new(vec![
                (0.0, Vec4::new(3.0, 1.6, 0.5, 0.8)),
                (0.15, Vec4::new(0.5, 0.5, 0.55, 0.5)),
                (1.0, Vec4::new(0.4, 0.4, 0.45, 0.0)),
            ]),
            blend: ColorBlend::Overwrite,
        }),
        blend: ParticleBlend::Alpha,
        depth_fade: 0.25,
        ..burst_emitter(
            1_024,
            seed,
            vec![
                InitModifier::Velocity(Value::Uniform {
                    min: Vec3::splat(-0.3),
                    max: Vec3::splat(0.3),
                }),
                InitModifier::Lifetime(Value::Uniform { min: 0.5, max: 0.8 }),
            ],
        )
    }
}

/// A short-lived point light.
#[derive(Clone, Copy, Debug)]
struct Flash {
    position: Vec3,
    color: Vec3,
    lumens: f32,
    range: f32,
    age: f32,
    life: f32,
}

pub struct Effects {
    muzzle: [EmitterId; 3],
    trail: [EmitterId; 3],
    impact: EmitterId,
    fireball: EmitterId,
    exhaust: EmitterId,
    flashes: Vec<Flash>,
}

fn index(weapon: Weapon) -> usize {
    match weapon {
        Weapon::EnergyRifle => 0,
        Weapon::RailGun => 1,
        Weapon::RocketLauncher => 2,
    }
}

/// Spacing of trail motes along a beam (metres) and their size and life, per weapon (only
/// hitscan weapons draw beams; the rocket's entry is unused).
const TRAIL: [(f32, f32, f32); 3] = [(0.5, 0.05, 0.12), (0.12, 0.07, 0.7), (1.0, 0.05, 0.1)];

impl Effects {
    /// Creates the emitters in `world`.
    pub fn new(world: &mut World) -> Self {
        let muzzle = Weapon::ALL.map(|weapon| {
            world.add_emitter(&sparks(
                tint(weapon),
                256,
                11 + index(weapon) as u32,
                0.35,
                [2.0, 6.0],
            ))
        });
        let trail = Weapon::ALL.map(|weapon| {
            let (_, size, life) = TRAIL[index(weapon)];
            world.add_emitter(&trail(
                tint(weapon),
                2_048,
                21 + index(weapon) as u32,
                size,
                life,
            ))
        });
        Self {
            muzzle,
            trail,
            impact: world.add_emitter(&sparks(
                Vec3::new(1.0, 0.8, 0.5),
                1_024,
                31,
                1.2,
                [2.0, 7.0],
            )),
            fireball: world.add_emitter(&fireball(41)),
            exhaust: world.add_emitter(&exhaust(51)),
            flashes: Vec::new(),
        }
    }

    fn flash(&mut self, position: Vec3, color: Vec3, lumens: f32, range: f32, life: f32) {
        self.flashes.push(Flash {
            position,
            color,
            lumens,
            range,
            age: 0.0,
            life,
        });
    }

    /// Starts the effects of a tick's events. Shots leave the eye in the simulation but are drawn
    /// from the held gun's barrel.
    pub fn play(&mut self, world: &mut World, events: &[Event]) {
        for event in events {
            match *event {
                Event::Fired {
                    weapon,
                    eye,
                    direction,
                    ..
                } => {
                    let direction = Vec3::from(direction);
                    let muzzle = gun_pose(units(eye), direction).1;
                    let count = if weapon == Weapon::RailGun { 24 } else { 8 };
                    world.burst(self.muzzle[index(weapon)], count, facing(muzzle, direction));
                    let lumens = if weapon == Weapon::RailGun {
                        400_000.0
                    } else {
                        150_000.0
                    };
                    self.flash(muzzle, tint(weapon), lumens, 6.0, 0.07);
                }
                Event::Beam {
                    weapon,
                    from,
                    end,
                    normal,
                    struck,
                } => {
                    let (from, end) = (units(from), units(end));
                    let muzzle = gun_pose(from, end - from).1;
                    let (spacing, _, _) = TRAIL[index(weapon)];
                    let span = end - muzzle;
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let steps = (span.length() / spacing).ceil() as u32;
                    for step in 0..steps {
                        #[allow(clippy::cast_precision_loss)]
                        let at = muzzle + span * (step as f32 / steps as f32);
                        world.burst(self.trail[index(weapon)], 1, Mat4::from_translation(at));
                    }
                    if struck {
                        let normal = Vec3::from(normal);
                        let count = if weapon == Weapon::RailGun { 40 } else { 10 };
                        world.burst(self.impact, count, facing(end + normal * 0.02, normal));
                        self.flash(end + normal * 0.1, tint(weapon), 80_000.0, 4.0, 0.1);
                    }
                }
                Event::Exploded { at } => {
                    let at = units(at);
                    world.burst(self.fireball, 140, Mat4::from_translation(at));
                    world.burst(self.impact, 60, facing(at, Vec3::Y));
                    self.flash(at, Vec3::new(1.0, 0.55, 0.2), 3_000_000.0, 14.0, 0.4);
                }
                Event::Damaged { .. }
                | Event::Hit { .. }
                | Event::Killed { .. }
                | Event::Spawned { .. }
                | Event::RoundOver { .. } => {}
            }
        }
    }

    /// Exhaust behind each rocket, once per simulation tick, from a few ticks out (so it never
    /// fills the shooter's view).
    pub fn rockets(&mut self, world: &mut World, rockets: &[RocketState]) {
        for rocket in rockets
            .iter()
            .filter(|rocket| rocket.age >= EXHAUST_AFTER_TICKS)
        {
            let at = units(rocket.position);
            world.burst(self.exhaust, 3, Mat4::from_translation(at));
        }
    }

    /// Ages the flashes by `seconds` and returns the lights to draw this frame, with one light
    /// riding on each rocket (positions in metres).
    pub fn lights(&mut self, seconds: f32, rockets: &[Vec3]) -> Vec<PointLight> {
        for flash in &mut self.flashes {
            flash.age += seconds;
        }
        self.flashes.retain(|flash| flash.age < flash.life);
        let rocket_color = tint(Weapon::RocketLauncher);
        self.flashes
            .iter()
            .map(|flash| {
                let left = 1.0 - flash.age / flash.life;
                (
                    flash.position,
                    flash.color,
                    flash.lumens * left * left,
                    flash.range,
                )
            })
            .chain(
                rockets
                    .iter()
                    .map(|position| (*position, rocket_color, 120_000.0, 5.0)),
            )
            .map(|(position, color, intensity, range)| PointLight {
                position,
                color,
                intensity,
                range,
                radius: 0.05,
                shadows: None,
            })
            .collect()
    }
}
