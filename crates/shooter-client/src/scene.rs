//! The arena as render-engine content: one mesh per piece at its true size (so normals stay
//! exact), a checkered floor for a sense of speed, glowing jump pads, and the hall's lighting;
//! and the [`Actors`] that move over it.

use crate::effects::tint;
use render_engine::glam::{Mat3, Mat4, Quat, Vec3, Vec4};
use render_engine::material::StandardMaterial;
use render_engine::mesh::{MeshData, Shape};
use render_engine::scene::{
    Ambient, Camera, PointLight, Scene, ShadowFilter, ShadowMapSizes, Sun, SunShadows, Tonemapping,
};
use render_engine::world::{Instance, InstanceId, MaterialId, MeshId, World};
use shooter_sim::sim_foundation::RampRise;
use shooter_sim::sim_foundation::physics::Vec3i;
use shooter_sim::{Arena, Piece, Weapon};

/// Simulation units (cm) to render metres.
pub const METRES_PER_UNIT: f32 = 0.01;

#[allow(clippy::cast_precision_loss)]
fn metres(value: Vec3i) -> Vec3 {
    Vec3::new(value.x as f32, value.y as f32, value.z as f32) * METRES_PER_UNIT
}

/// Appends one flat triangle, wound counter-clockwise when seen from `normal`'s side.
fn triangle(mesh: &mut MeshData, corners: [Vec3; 3], normal: Vec3, color: Vec4) {
    let [a, mut b, mut c] = corners;
    if (b - a).cross(c - a).dot(normal) < 0.0 {
        std::mem::swap(&mut b, &mut c);
    }
    let base = u32::try_from(mesh.positions.len()).expect("small mesh");
    for corner in [a, b, c] {
        mesh.positions.push(corner);
        mesh.normals.push(normal);
        mesh.colors.push(color);
    }
    mesh.indices.extend([base, base + 1, base + 2]);
}

fn quad(mesh: &mut MeshData, corners: [Vec3; 4], normal: Vec3, color: Vec4) {
    let [a, b, c, d] = corners;
    triangle(mesh, [a, b, c], normal, color);
    triangle(mesh, [a, c, d], normal, color);
}

/// A ramp wedge in its box of half extents `half` (metres), rising toward +Z.
fn wedge(half: Vec3) -> MeshData {
    let mut mesh = MeshData::default();
    let white = Vec4::ONE;
    let (x, y, z) = (half.x, half.y, half.z);
    let low = |sx: f32| Vec3::new(sx * x, -y, -z);
    let foot = |sx: f32| Vec3::new(sx * x, -y, z);
    let top = |sx: f32| Vec3::new(sx * x, y, z);
    quad(
        &mut mesh,
        [low(-1.0), low(1.0), foot(1.0), foot(-1.0)],
        Vec3::NEG_Y,
        white,
    );
    quad(
        &mut mesh,
        [foot(-1.0), foot(1.0), top(1.0), top(-1.0)],
        Vec3::Z,
        white,
    );
    let slope = Vec3::new(0.0, z, -y).normalize();
    quad(
        &mut mesh,
        [low(-1.0), low(1.0), top(1.0), top(-1.0)],
        slope,
        white,
    );
    for side in [-1.0, 1.0] {
        triangle(
            &mut mesh,
            [low(side), foot(side), top(side)],
            Vec3::X * side,
            white,
        );
    }
    mesh
}

/// The floor: 2 m tiles in two tones.
fn floor(half_size: f32) -> MeshData {
    let mut mesh = MeshData::default();
    let tile = 2.0;
    #[allow(clippy::cast_possible_truncation)]
    let count = (2.0 * half_size / tile).round() as i32;
    for row in 0..count {
        for column in 0..count {
            #[allow(clippy::cast_precision_loss)]
            let (x0, z0) = (
                -half_size + column as f32 * tile,
                -half_size + row as f32 * tile,
            );
            let shade = if (row + column) % 2 == 0 { 1.0 } else { 0.82 };
            let color = Vec4::new(shade, shade, shade, 1.0);
            quad(
                &mut mesh,
                [
                    Vec3::new(x0, 0.0, z0),
                    Vec3::new(x0 + tile, 0.0, z0),
                    Vec3::new(x0 + tile, 0.0, z0 + tile),
                    Vec3::new(x0, 0.0, z0 + tile),
                ],
                Vec3::Y,
                color,
            );
        }
    }
    mesh
}

fn material(world: &mut World, rgb: [f32; 3], roughness: f32) -> render_engine::world::MaterialId {
    world.add_material(&StandardMaterial {
        base_color: Vec4::new(rgb[0], rgb[1], rgb[2], 1.0),
        perceptual_roughness: roughness,
        ..StandardMaterial::default()
    })
}

/// X and Z exchanged: a ramp's box seen from its own rise axis.
const fn swapped(half: Vec3) -> Vec3 {
    Vec3::new(half.z, half.y, half.x)
}

/// Uploads the arena and retains its instances in `world`: they are drawn every frame.
pub fn build(world: &mut World, arena: &Arena) {
    let floor_material = material(world, [0.20, 0.22, 0.26], 0.8);
    let wall = material(world, [0.55, 0.57, 0.62], 0.7);
    let accent = material(world, [0.85, 0.42, 0.12], 0.5);
    let crate_material = material(world, [0.10, 0.55, 0.60], 0.6);

    #[allow(clippy::cast_precision_loss)]
    let half_size = arena.half_size as f32 * METRES_PER_UNIT;
    let floor_mesh = world.add_mesh(&floor(half_size));
    world.add_instance(Instance {
        mesh: floor_mesh,
        material: floor_material,
        world_from_local: Mat4::IDENTITY,
        skin: None,
    });

    for piece in &arena.pieces {
        let center = metres(piece.center());
        let half = metres(piece.half());
        let (mesh, rotation, material) = match *piece {
            Piece::Block { .. } => {
                let material = if half.y >= 1.9 {
                    wall
                } else if half.x.max(half.z) <= 0.5 {
                    crate_material
                } else {
                    accent
                };
                (
                    world.add_mesh(&MeshData::shape(Shape::Cuboid(half * 2.0))),
                    Quat::IDENTITY,
                    material,
                )
            }
            Piece::Ramp { rises, .. } => {
                // The wedge rises toward +Z; turn it, swapping its run onto the right axis.
                let (angle, local_half) = match rises {
                    RampRise::PosZ => (0.0, half),
                    RampRise::NegZ => (std::f32::consts::PI, half),
                    RampRise::PosX => (std::f32::consts::FRAC_PI_2, swapped(half)),
                    RampRise::NegX => (-std::f32::consts::FRAC_PI_2, swapped(half)),
                };
                (
                    world.add_mesh(&wedge(local_half)),
                    Quat::from_rotation_y(angle),
                    accent,
                )
            }
        };
        world.add_instance(Instance {
            mesh,
            material,
            world_from_local: Mat4::from_rotation_translation(rotation, center),
            skin: None,
        });
    }

    // Jump pads: glowing 2 cm plates (no collider; the simulation throws from the footprint).
    let pad_material = world.add_material(&StandardMaterial {
        base_color: Vec4::new(0.1, 0.8, 0.95, 1.0),
        emissive: Vec3::new(400.0, 3_000.0, 3_600.0),
        perceptual_roughness: 0.3,
        ..StandardMaterial::default()
    });
    const PAD_THICKNESS: f32 = 0.02;
    for pad in &arena.pads {
        #[allow(clippy::cast_precision_loss)]
        let half = Vec3::new(pad.half[0] as f32, 0.0, pad.half[1] as f32) * METRES_PER_UNIT;
        let size = Vec3::new(half.x * 2.0, PAD_THICKNESS, half.z * 2.0);
        let center = metres(pad.center) + Vec3::Y * (PAD_THICKNESS / 2.0);
        let mesh = world.add_mesh(&MeshData::shape(Shape::Cuboid(size)));
        world.add_instance(Instance {
            mesh,
            material: pad_material,
            world_from_local: Mat4::from_translation(center),
            skin: None,
        });
    }
}

/// Where something moving is drawn hidden: under the floor.
const HIDDEN: Mat4 = Mat4::from_cols(Vec4::X, Vec4::Y, Vec4::Z, Vec4::new(0.0, -100.0, 0.0, 1.0));

/// The first-person camera's basis: forward, right (yaw 0 faces +Z, its right is -X) and up.
#[must_use]
pub fn basis(forward: Vec3) -> (Vec3, Vec3, Vec3) {
    let forward = forward.normalize_or(Vec3::Z);
    let right = forward.cross(Vec3::Y).normalize_or(Vec3::NEG_X);
    (forward, right, right.cross(forward))
}

/// The placeholder gun's length (metres) and where it sits relative to the eye.
const GUN_LENGTH: f32 = 0.3;

/// The held gun's transform and its barrel's end, for an eye and view direction (metres).
#[must_use]
pub fn gun_pose(eye: Vec3, forward: Vec3) -> (Mat4, Vec3) {
    let (forward, right, up) = basis(forward);
    let center = eye + forward * 0.42 + right * 0.2 - up * 0.2;
    let rotation = Quat::from_mat3(&Mat3::from_cols(up.cross(forward), up, forward));
    (
        Mat4::from_rotation_translation(rotation, center),
        center + forward * (GUN_LENGTH / 2.0),
    )
}

/// One other fighter as drawn this frame.
#[derive(Clone, Copy, Debug)]
pub struct FighterView {
    /// Body centre in metres, or `None` while it is dead.
    pub center: Option<Vec3>,
    /// Where it looks.
    pub forward: Vec3,
    /// Remaining health, 0..=1.
    pub health: f32,
    /// Hit glow, 1 just hit and fading to 0.
    pub glow: f32,
    pub held: Weapon,
}

fn fighter_material(health: f32, glow: f32) -> StandardMaterial {
    let healthy = Vec3::new(0.15, 0.55, 0.5);
    let hurt = Vec3::new(0.75, 0.12, 0.1);
    let color = hurt.lerp(healthy, health.clamp(0.0, 1.0));
    StandardMaterial {
        base_color: color.extend(1.0),
        emissive: Vec3::new(1.0, 0.85, 0.7) * (glow * 4_000.0),
        perceptual_roughness: 0.6,
        ..StandardMaterial::default()
    }
}

/// One other fighter's instances: its body (coloured by health) and its gun.
struct FighterInstances {
    body: InstanceId,
    material: MaterialId,
    gun: InstanceId,
}

/// The moving things drawn over the arena: the other fighters, rockets, the held gun and
/// (natively; the page draws its own) a crosshair and a health bar. Placeholder primitives until
/// asset-tooling delivers the weapon and character models.
pub struct Actors {
    fighters: Vec<FighterInstances>,
    fighter_mesh: MeshId,
    rocket_mesh: MeshId,
    rocket_material: MaterialId,
    rockets: Vec<InstanceId>,
    gun: InstanceId,
    gun_mesh: MeshId,
    gun_materials: [MaterialId; 3],
    crosshair: Option<InstanceId>,
    /// Natively: the health bar's mesh, material and instance (HUD stopgap).
    health_bar: Option<(MeshId, MaterialId, InstanceId)>,
}

fn glowing(rgb: Vec3, nits: f32) -> StandardMaterial {
    StandardMaterial {
        base_color: rgb.extend(1.0),
        emissive: rgb * nits,
        perceptual_roughness: 0.4,
        ..StandardMaterial::default()
    }
}

fn gun_material(materials: &[MaterialId; 3], held: Weapon) -> MaterialId {
    materials[usize::from(held.index())]
}

impl Actors {
    pub fn new(world: &mut World) -> Self {
        let fighter_mesh =
            world.add_mesh(&MeshData::shape(Shape::Cuboid(Vec3::new(0.6, 1.8, 0.6))));
        let rocket_mesh = world.add_mesh(&MeshData::shape(Shape::Sphere {
            radius: 0.09,
            sectors: 12,
            stacks: 8,
        }));
        let rocket_material = world.add_material(&glowing(Vec3::new(1.0, 0.6, 0.2), 60_000.0));
        let gun_mesh = world.add_mesh(&MeshData::shape(Shape::Cuboid(Vec3::new(
            0.045, 0.055, GUN_LENGTH,
        ))));
        let gun_materials =
            Weapon::ALL.map(|weapon| world.add_material(&glowing(tint(weapon), 1_200.0)));
        let gun = world.add_instance(Instance {
            mesh: gun_mesh,
            material: gun_materials[0],
            world_from_local: HIDDEN,
            skin: None,
        });
        let native = !cfg!(target_arch = "wasm32");
        let crosshair = native.then(|| {
            let mesh = world.add_mesh(&MeshData::shape(Shape::Sphere {
                radius: 0.0015,
                sectors: 8,
                stacks: 6,
            }));
            let material = world.add_material(&glowing(Vec3::ONE, 40_000.0));
            world.add_instance(Instance {
                mesh,
                material,
                world_from_local: HIDDEN,
                skin: None,
            })
        });
        let health_bar = native.then(|| {
            let mesh = world.add_mesh(&MeshData::shape(Shape::Cuboid(Vec3::new(
                HEALTH_BAR_LENGTH,
                0.004,
                0.001,
            ))));
            let material = world.add_material(&glowing(Vec3::new(0.3, 1.0, 0.5), 20_000.0));
            let instance = world.add_instance(Instance {
                mesh,
                material,
                world_from_local: HIDDEN,
                skin: None,
            });
            (mesh, material, instance)
        });
        Self {
            fighters: Vec::new(),
            fighter_mesh,
            rocket_mesh,
            rocket_material,
            rockets: Vec::new(),
            gun,
            gun_mesh,
            gun_materials,
            crosshair,
            health_bar,
        }
    }

    fn place(
        world: &mut World,
        instance: InstanceId,
        mesh: MeshId,
        material: MaterialId,
        at: Mat4,
    ) {
        world.set_instance(
            instance,
            Instance {
                mesh,
                material,
                world_from_local: at,
                skin: None,
            },
        );
    }

    /// Places everything for this frame. `eye`/`forward` is the local camera; `held` is `None`
    /// while the local fighter is dead (no gun in view); `health` is its health (0..=1).
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        world: &mut World,
        fighters: &[FighterView],
        rockets: &[Vec3],
        held: Option<Weapon>,
        health: Option<f32>,
        eye: Vec3,
        forward: Vec3,
    ) {
        while self.fighters.len() < fighters.len() {
            let material = world.add_material(&fighter_material(1.0, 0.0));
            let body = world.add_instance(Instance {
                mesh: self.fighter_mesh,
                material,
                world_from_local: HIDDEN,
                skin: None,
            });
            let gun = world.add_instance(Instance {
                mesh: self.gun_mesh,
                material: self.gun_materials[0],
                world_from_local: HIDDEN,
                skin: None,
            });
            self.fighters.push(FighterInstances {
                body,
                material,
                gun,
            });
        }
        for (index, instances) in self.fighters.iter().enumerate() {
            let view = fighters.get(index).filter(|view| view.center.is_some());
            let (body_at, gun_at, gun_material) = match view {
                Some(view) => {
                    world.set_material(
                        instances.material,
                        &fighter_material(view.health, view.glow),
                    );
                    let center = view.center.unwrap_or_default();
                    let fighter_eye = center + Vec3::Y * 0.75;
                    (
                        Mat4::from_translation(center),
                        gun_pose(fighter_eye, view.forward).0,
                        gun_material(&self.gun_materials, view.held),
                    )
                }
                None => (HIDDEN, HIDDEN, self.gun_materials[0]),
            };
            Self::place(
                world,
                instances.body,
                self.fighter_mesh,
                instances.material,
                body_at,
            );
            Self::place(world, instances.gun, self.gun_mesh, gun_material, gun_at);
        }
        while self.rockets.len() < rockets.len() {
            let instance = world.add_instance(Instance {
                mesh: self.rocket_mesh,
                material: self.rocket_material,
                world_from_local: HIDDEN,
                skin: None,
            });
            self.rockets.push(instance);
        }
        for (index, instance) in self.rockets.iter().enumerate() {
            let at = rockets
                .get(index)
                .map_or(HIDDEN, |at| Mat4::from_translation(*at));
            Self::place(world, *instance, self.rocket_mesh, self.rocket_material, at);
        }
        let material = gun_material(&self.gun_materials, held.unwrap_or_default());
        let at = if held.is_some() {
            gun_pose(eye, forward).0
        } else {
            HIDDEN
        };
        Self::place(world, self.gun, self.gun_mesh, material, at);
        if let Some(crosshair) = self.crosshair {
            let mut instance = world.instance(crosshair);
            instance.world_from_local = Mat4::from_translation(eye + forward * 0.5);
            world.set_instance(crosshair, instance);
        }
        if let Some((mesh, material, instance)) = self.health_bar {
            let at = health.map_or(HIDDEN, |health| health_bar_pose(eye, forward, health));
            Self::place(world, instance, mesh, material, at);
        }
    }
}

/// The native health bar's full length (metres, half a metre ahead of the eye).
const HEALTH_BAR_LENGTH: f32 = 0.12;

/// The health bar low in the view, shrunk to `health` from its left end.
fn health_bar_pose(eye: Vec3, forward: Vec3, health: f32) -> Mat4 {
    let (forward, right, up) = basis(forward);
    let health = health.clamp(0.0, 1.0);
    let left = eye + forward * 0.5 - up * 0.2 - right * (HEALTH_BAR_LENGTH / 2.0);
    let center = left + right * (HEALTH_BAR_LENGTH * health / 2.0);
    let rotation = Quat::from_mat3(&Mat3::from_cols(-right, up, forward));
    Mat4::from_scale_rotation_translation(Vec3::new(health.max(0.001), 1.0, 1.0), rotation, center)
}

/// The frame's camera and the hall's light: a high sun through an open roof and soft ambient,
/// plus the weapons' flashes.
#[must_use]
pub fn frame(eye: Vec3, forward: Vec3, point_lights: Vec<PointLight>) -> Scene {
    Scene {
        camera: Camera {
            eye,
            target: eye + forward,
            fov_y: 1.2,
            near: 0.05,
            ev100: 13.0,
            shadow_filter: ShadowFilter::Gaussian,
            tonemapping: Tonemapping::TonyMcMapface,
        },
        clear_color: Vec3::new(0.35, 0.45, 0.6),
        sun: Some(Sun {
            direction: Vec3::new(-0.35, -1.0, -0.55).normalize(),
            color: Vec3::new(1.0, 0.96, 0.9),
            illuminance: 20_000.0,
            shadows: Some(SunShadows {
                max_distance: 60.0,
                ..SunShadows::default()
            }),
        }),
        moon: None,
        sky: None,
        point_lights,
        spot_lights: Vec::new(),
        shadow_maps: ShadowMapSizes::default(),
        ambient: Ambient {
            color: Vec3::new(0.85, 0.9, 1.0),
            brightness: 2_500.0,
        },
        environment: None,
        fog: None,
    }
}
