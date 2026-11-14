//! The arena as render-engine content: one mesh per piece at its true size (so normals stay
//! exact), a checkered floor for a sense of speed, glowing jump pads, and the hall's lighting;
//! and the [`Actors`] that move over it.

use crate::camera::{self, basis};
use crate::effects::tint;
use render_engine::glam::{Mat3, Mat4, Quat, Vec3, Vec4};
use render_engine::material::StandardMaterial;
use render_engine::mesh::{MeshData, Shape};
use render_engine::scene::{
    Ambient, Camera, PointLight, Scene, ShadowFilter, ShadowMapSizes, Sun, SunShadows, Tonemapping,
};
use render_engine::world::{Instance, InstanceId, MaterialId, MeshId, World};
use shooter_sim::physics::RampRise;
use shooter_sim::physics::Vec3i;
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

/// The energy blade's length (metres).
const BLADE_LENGTH: f32 = 0.75;
/// A swing sweeps from this far right of the view to as far left (radians).
const SWING_ARC: f32 = 1.3;
/// How long a drawn swing lasts (seconds).
pub const SWING_SECONDS: f32 = 0.18;

/// The blade `progress` (0..=1) through a swing from `eye` looking along `forward` (metres): a
/// flat slash from right to left across the view, like the swing's arc in the simulation.
#[must_use]
pub fn blade_pose(eye: Vec3, forward: Vec3, progress: f32) -> Mat4 {
    let (forward, right, up) = basis(forward);
    let t = progress.clamp(0.0, 1.0);
    let angle = SWING_ARC * (1.0 - 2.0 * t * t * (3.0 - 2.0 * t));
    let (sine, cosine) = angle.sin_cos();
    let direction = (forward * cosine + right * sine - up * 0.2).normalize_or(forward);
    let hilt = eye + forward * 0.1 + right * (0.12 * sine) - up * 0.22;
    let rotation = Quat::from_rotation_arc(Vec3::Z, direction);
    Mat4::from_rotation_translation(rotation, hilt + direction * (BLADE_LENGTH / 2.0))
}

/// How far above its body centre a fighter's eye is drawn (metres), unless it is sliding.
#[allow(clippy::cast_precision_loss)]
pub const EYE_ABOVE_CENTER: f32 = (shooter_sim::player::EYE_HEIGHT_UNITS
    - shooter_sim::player::PLAYER_HALF_EXTENTS.y) as f32
    * METRES_PER_UNIT;

/// One fighter as drawn this frame, the local one included.
#[derive(Clone, Copy, Debug)]
pub struct FighterView {
    /// Body centre in metres, or `None` while it is dead (or the camera is inside it).
    pub center: Option<Vec3>,
    /// How far above the centre its eye is (metres): where its gun and blade are held.
    pub eye_above_center: f32,
    /// Where it looks.
    pub forward: Vec3,
    /// Remaining health, 0..=1.
    pub health: f32,
    /// Hit glow, 1 just hit and fading to 0.
    pub glow: f32,
    pub held: Weapon,
    /// Where its grapple hook holds (metres), while set.
    pub hook: Option<Vec3>,
    /// How far through a blade swing it is (0..=1), while swinging.
    pub swing: Option<f32>,
}

/// A grapple cable's thickness (metres).
const CABLE_WIDTH: f32 = 0.025;

/// A unit-length cable stretched from `from` to `to` (metres).
fn cable_pose(from: Vec3, to: Vec3) -> Mat4 {
    let span = to - from;
    let rotation = Quat::from_rotation_arc(Vec3::Z, span.normalize_or(Vec3::Z));
    Mat4::from_scale_rotation_translation(
        Vec3::new(1.0, 1.0, span.length().max(0.001)),
        rotation,
        from + span / 2.0,
    )
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

/// One fighter's instances: its body (coloured by health), its gun and its blade.
struct FighterInstances {
    body: InstanceId,
    material: MaterialId,
    /// The health and hit glow `material` shows. render-engine rebuilds a material's GPU buffer
    /// on each `set_material` (in WebGPU those pile up until garbage collection), so it is only
    /// set when they change.
    shown: Option<(f32, f32)>,
    gun: InstanceId,
    blade: InstanceId,
}

/// The moving things drawn over the arena: every fighter (the local one too, seen from the
/// third-person camera) with its gun, swung blade and grapple cable, the rockets, the reticle
/// and (natively; the page draws its own) a health bar. Placeholder primitives until
/// asset-tooling delivers the weapon and robot models.
pub struct Actors {
    fighters: Vec<FighterInstances>,
    fighter_mesh: MeshId,
    rocket_mesh: MeshId,
    rocket_material: MaterialId,
    rockets: Vec<InstanceId>,
    gun_mesh: MeshId,
    gun_materials: [MaterialId; Weapon::ALL.len()],
    blade_mesh: MeshId,
    blade_material: MaterialId,
    cable_mesh: MeshId,
    cable_material: MaterialId,
    /// Grapple cables, reused frame to frame (hidden when unused).
    cables: Vec<InstanceId>,
    reticle: InstanceId,
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

fn gun_material(materials: &[MaterialId; Weapon::ALL.len()], held: Weapon) -> MaterialId {
    materials[usize::from(held.index())]
}

/// The reticle's radius (metres, [`camera::RETICLE_DISTANCE`] ahead of the camera).
const RETICLE_RADIUS: f32 = 0.0009;

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
        let blade_mesh = world.add_mesh(&MeshData::shape(Shape::Cuboid(Vec3::new(
            0.012,
            0.05,
            BLADE_LENGTH,
        ))));
        let blade_material = world.add_material(&glowing(tint(Weapon::EnergyBlade), 8_000.0));
        let reticle = {
            let mesh = world.add_mesh(&MeshData::shape(Shape::Sphere {
                radius: RETICLE_RADIUS,
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
        };
        let health_bar = (!cfg!(target_arch = "wasm32")).then(|| {
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
            gun_mesh,
            gun_materials,
            blade_mesh,
            blade_material,
            cable_mesh: world.add_mesh(&MeshData::shape(Shape::Cuboid(Vec3::new(
                CABLE_WIDTH,
                CABLE_WIDTH,
                1.0,
            )))),
            cable_material: world.add_material(&glowing(tint(Weapon::Grapple), 6_000.0)),
            cables: Vec::new(),
            reticle,
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

    /// Places everything for this frame. `camera`/`forward` is the camera; `reticle` where the
    /// reticle is drawn ([`camera::reticle`]), while the local fighter is alive; `health` is its
    /// health (0..=1).
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        world: &mut World,
        fighters: &[FighterView],
        rockets: &[Vec3],
        health: Option<f32>,
        camera: Vec3,
        forward: Vec3,
        reticle: Option<Vec3>,
    ) {
        let cables: Vec<(Vec3, Vec3)> = fighters
            .iter()
            .filter_map(|view| {
                let fighter_eye = view.center? + Vec3::Y * view.eye_above_center;
                Some((gun_pose(fighter_eye, view.forward).1, view.hook?))
            })
            .collect();
        while self.cables.len() < cables.len() {
            let instance = world.add_instance(Instance {
                mesh: self.cable_mesh,
                material: self.cable_material,
                world_from_local: HIDDEN,
                skin: None,
            });
            self.cables.push(instance);
        }
        for (index, instance) in self.cables.iter().enumerate() {
            let at = cables
                .get(index)
                .map_or(HIDDEN, |(from, to)| cable_pose(*from, *to));
            Self::place(world, *instance, self.cable_mesh, self.cable_material, at);
        }
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
            let blade = world.add_instance(Instance {
                mesh: self.blade_mesh,
                material: self.blade_material,
                world_from_local: HIDDEN,
                skin: None,
            });
            self.fighters.push(FighterInstances {
                body,
                material,
                shown: None,
                gun,
                blade,
            });
        }
        for (index, instances) in self.fighters.iter_mut().enumerate() {
            let view = fighters.get(index).filter(|view| view.center.is_some());
            let (body_at, gun_at, gun_material) = match view {
                Some(view) => {
                    let look = (view.health, view.glow);
                    if instances.shown != Some(look) {
                        world.set_material(
                            instances.material,
                            &fighter_material(view.health, view.glow),
                        );
                        instances.shown = Some(look);
                    }
                    let center = view.center.unwrap_or_default();
                    let fighter_eye = center + Vec3::Y * view.eye_above_center;
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
            // Through a swing the blade sweeps across the fighter's front and the gun is put
            // away.
            let swinging = view.and_then(|view| Some((view, view.swing?)));
            let blade_at = swinging.map_or(HIDDEN, |(view, swing)| {
                let fighter_eye = view.center.unwrap_or_default() + Vec3::Y * view.eye_above_center;
                blade_pose(fighter_eye, view.forward, swing)
            });
            let gun_at = if swinging.is_some() { HIDDEN } else { gun_at };
            Self::place(world, instances.gun, self.gun_mesh, gun_material, gun_at);
            Self::place(
                world,
                instances.blade,
                self.blade_mesh,
                self.blade_material,
                blade_at,
            );
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
        let mut instance = world.instance(self.reticle);
        instance.world_from_local = reticle.map_or(HIDDEN, Mat4::from_translation);
        world.set_instance(self.reticle, instance);
        if let Some((mesh, material, instance)) = self.health_bar {
            let at = health.map_or(HIDDEN, |health| health_bar_pose(camera, forward, health));
            Self::place(world, instance, mesh, material, at);
        }
    }
}

/// The native health bar's full length (metres, half a metre ahead of the camera).
const HEALTH_BAR_LENGTH: f32 = 0.12;

/// The health bar low and right of the view's centre, clear of the fighter over whose shoulder
/// the camera looks, shrunk to `health` from its left end.
fn health_bar_pose(camera: Vec3, forward: Vec3, health: f32) -> Mat4 {
    let (forward, right, up) = basis(forward);
    let health = health.clamp(0.0, 1.0);
    let left = camera + forward * 0.5 - up * 0.25 + right * 0.12;
    let center = left + right * (HEALTH_BAR_LENGTH * health / 2.0);
    let rotation = Quat::from_mat3(&Mat3::from_cols(-right, up, forward));
    Mat4::from_scale_rotation_translation(Vec3::new(health.max(0.001), 1.0, 1.0), rotation, center)
}

/// The frame's camera ([`camera`]) and the hall's light: a high sun through an open roof and
/// soft ambient, plus the weapons' flashes.
#[must_use]
pub fn frame(camera: Vec3, forward: Vec3, point_lights: Vec<PointLight>) -> Scene {
    Scene {
        camera: Camera {
            eye: camera,
            target: camera + forward,
            fov_y: camera::FOV_Y,
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
        overlay: Vec::new(),
    }
}
