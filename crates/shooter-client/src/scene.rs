//! The arena as render-engine content: a kit-built map's modules (`crate::arena_art`), or a
//! greybox map's one mesh per piece at its true size (so normals stay
//! exact), a checkered floor for a sense of speed, glowing jump pads, the module pickups' pads, and the hall's lighting per map ([`Lighting`]);
//! and the [`Actors`] that move over it.

use crate::arena_art::{ArenaArt, Placed};
use crate::camera::{self, basis};
use crate::effects::tint;
use crate::gfx::StandardMaterial;
use crate::gfx::{Instance, InstanceId, MaterialId, MeshId, World};
use render_engine::glam::{Mat3, Mat4, Quat, Vec3, Vec4};
use render_engine::mesh::{MeshData, Shape};
use render_engine::scene::{
    Ambient, Camera, Fog, FogFalloff, PointLight, Scene, ShadowFilter, ShadowMapSizes, Sun,
    SunShadows, Tonemapping,
};
use shooter_sim::physics::RampRise;
use shooter_sim::physics::Vec3i;
use shooter_sim::pickups::REACH_UNITS;
use shooter_sim::{Arena, ArenaModule, MapId, Piece, Weapon};

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

fn material(world: &mut World, rgb: [f32; 3], roughness: f32) -> crate::gfx::MaterialId {
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

/// The greybox jump pads' and module pads' plates (metres).
const PAD_THICKNESS: f32 = 0.02;

/// Uploads the arena and retains its instances in `world`: they are drawn every frame. A
/// kit-built map (one with placements) is drawn from the arena kit when `art` is there, else
/// (the kit unavailable) and on the greybox maps as boxes; the module pads on every map.
/// Returns the kit's placed modules, whose looks follow the match's destruction
/// ([`ArenaArt::show`]); the boxes stand as the map authored them.
pub fn build(world: &mut World, arena: &Arena, art: Option<&ArenaArt>) -> DrawnArena {
    let mut instances = Vec::new();
    let placed = match art.filter(|_| !arena.placements.is_empty()) {
        Some(art) => Some(art.draw(world, arena)),
        None => {
            greybox(world, arena, &mut instances);
            None
        }
    };
    module_pads(world, arena, &mut instances);
    DrawnArena { placed, instances }
}

/// An arena in the GPU's world ([`build`]): its kit-built modules, if it has them, and every
/// other instance it added, so that it can be taken out again (the main menu's live view
/// changes arenas, and a match after it is on another, shooter#154).
pub struct DrawnArena {
    pub placed: Option<Placed>,
    instances: Vec<InstanceId>,
}

impl DrawnArena {
    /// Takes the arena's instances out of `world` (its meshes and materials stay uploaded).
    pub fn remove(self, world: &mut World) {
        let placed = self.placed.iter().flat_map(Placed::instances);
        for instance in self.instances.into_iter().chain(placed) {
            world.remove_instance(instance);
        }
    }
}

/// The arena's pieces as boxes and wedges on a checkered floor, and its jump pads as glowing
/// plates.
fn greybox(world: &mut World, arena: &Arena, instances: &mut Vec<InstanceId>) {
    let floor_material = material(world, [0.20, 0.22, 0.26], 0.8);
    let wall = material(world, [0.55, 0.57, 0.62], 0.7);
    let accent = material(world, [0.85, 0.42, 0.12], 0.5);
    let crate_material = material(world, [0.10, 0.55, 0.60], 0.6);

    #[allow(clippy::cast_precision_loss)]
    let half_size = arena.half_size as f32 * METRES_PER_UNIT;
    let floor_mesh = world.add_mesh(&floor(half_size));
    instances.push(world.add_instance(Instance {
        mesh: floor_mesh,
        material: floor_material,
        world_from_local: Mat4::IDENTITY,
        skin: None,
    }));

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
        instances.push(world.add_instance(Instance {
            mesh,
            material,
            world_from_local: Mat4::from_rotation_translation(rotation, center),
            skin: None,
        }));
    }

    // Jump pads: glowing 2 cm plates (no collider; the simulation throws from the footprint).
    let pad_material = world.add_material(&StandardMaterial {
        base_color: Vec4::new(0.1, 0.8, 0.95, 1.0),
        emissive: Vec3::new(400.0, 3_000.0, 3_600.0),
        perceptual_roughness: 0.3,
        ..StandardMaterial::default()
    });
    for pad in &arena.pads {
        #[allow(clippy::cast_precision_loss)]
        let half = Vec3::new(pad.half[0] as f32, 0.0, pad.half[1] as f32) * METRES_PER_UNIT;
        let size = Vec3::new(half.x * 2.0, PAD_THICKNESS, half.z * 2.0);
        let center = metres(pad.center) + Vec3::Y * (PAD_THICKNESS / 2.0);
        let mesh = world.add_mesh(&MeshData::shape(Shape::Cuboid(size)));
        instances.push(world.add_instance(Instance {
            mesh,
            material: pad_material,
            world_from_local: Mat4::from_translation(center),
            skin: None,
        }));
    }
}

/// Module pickup pads, on every map.
fn module_pads(world: &mut World, arena: &Arena, instances: &mut Vec<InstanceId>) {
    // Module pickup pads: dark plates as wide as their reach, tinted by the module they hold;
    // the module itself floats over the plate while it is there ([`Actors`]).
    #[allow(clippy::cast_precision_loss)]
    let reach = REACH_UNITS as f32 * METRES_PER_UNIT;
    let plate = world.add_mesh(&MeshData::shape(Shape::Cuboid(Vec3::new(
        reach * 2.0,
        PAD_THICKNESS,
        reach * 2.0,
    ))));
    for pad in &arena.module_pads {
        let rgb = module_tint(pad.module);
        let material = material(world, (rgb * 0.3).to_array(), 0.4);
        instances.push(world.add_instance(Instance {
            mesh: plate,
            material,
            world_from_local: Mat4::from_translation(
                metres(pad.center) + Vec3::Y * (PAD_THICKNESS / 2.0),
            ),
            skin: None,
        }));
    }
}

/// A module pickup's colour: the weapon it boosts, or the grapple's green for movement.
const fn module_tint(module: ArenaModule) -> Vec3 {
    match module {
        ArenaModule::RailCapacitor => tint(Weapon::RailGun),
        ArenaModule::Thrusters => tint(Weapon::Grapple),
    }
}

/// How high over its pad a module floats (metres, its centre) and its cube's edge.
const MODULE_HEIGHT: f32 = 0.6;
const MODULE_SIZE: f32 = 0.35;

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
    pub slot: u8,
    /// Drawn as its robot ([`crate::robots`]): no box body and no held gun box here.
    pub robot: bool,
    /// Where its robot's grapple weapon is: the cable's start (else the held gun's barrel).
    pub grapple_from: Option<Vec3>,
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
/// third-person camera) with its gun, swung blade and grapple cable, the rockets, the modules lying on their pads, the reticle
/// and its blocked marker
/// (the HUD is screen overlay, [`crate::overlay`]). Placeholder primitives: a fighter drawn as
/// its robot ([`crate::robots`]) shows no box body or held gun box here (its weapons are the
/// weapons kit's); the swung blade's sweep still shows the simulation's arc for every fighter.
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
    module_mesh: MeshId,
    /// By module wire tag.
    module_materials: [MaterialId; ArenaModule::ALL.len()],
    /// Modules on their pads, reused frame to frame (hidden when unused).
    modules: Vec<InstanceId>,
    reticle: InstanceId,
    /// Where the muzzle's shot really stops when cover beside the robot blocks it.
    blocked: InstanceId,
    /// Each gun's mark ([`camera::Reticle::guns`], shooter#132), reused frame to frame (hidden
    /// when unused), and its mesh and materials: on the target, blocked, unreachable.
    gun_marks: Vec<InstanceId>,
    gun_mark_mesh: MeshId,
    gun_mark_materials: [MaterialId; 3],
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
/// A gun's mark's radius (metres, likewise): smaller than the reticle.
const GUN_MARK_RADIUS: f32 = 0.0007;

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
        let marker_mesh = world.add_mesh(&MeshData::shape(Shape::Sphere {
            radius: RETICLE_RADIUS,
            sectors: 8,
            stacks: 6,
        }));
        let mut marker = |rgb: Vec3| {
            let material = world.add_material(&glowing(rgb, 40_000.0));
            world.add_instance(Instance {
                mesh: marker_mesh,
                material,
                world_from_local: HIDDEN,
                skin: None,
            })
        };
        // White on the camera's target, red where cover beside the robot stops the shot.
        let reticle = marker(Vec3::ONE);
        let blocked = marker(Vec3::new(1.0, 0.1, 0.05));
        // Each gun's mark, smaller: green on the target, red blocked, amber where a mount that
        // cannot point at the target sends the shot.
        let gun_mark_mesh = world.add_mesh(&MeshData::shape(Shape::Sphere {
            radius: GUN_MARK_RADIUS,
            sectors: 8,
            stacks: 6,
        }));
        let gun_mark_materials = [
            Vec3::new(0.2, 1.0, 0.35),
            Vec3::new(1.0, 0.1, 0.05),
            Vec3::new(1.0, 0.6, 0.0),
        ]
        .map(|rgb| world.add_material(&glowing(rgb, 40_000.0)));
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
            module_mesh: world.add_mesh(&MeshData::shape(Shape::Cuboid(Vec3::splat(MODULE_SIZE)))),
            module_materials: ArenaModule::ALL
                .map(|module| world.add_material(&glowing(module_tint(module), 4_000.0))),
            modules: Vec::new(),
            reticle,
            blocked,
            gun_marks: Vec::new(),
            gun_mark_mesh,
            gun_mark_materials,
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

    /// Places everything for this frame. `reticle` is the reticle and its blocked marker
    /// ([`camera::Reticle`]), while the local fighter is alive; `modules` the modules lying on
    /// their pads (each pad's centre, metres).
    pub fn update(
        &mut self,
        world: &mut World,
        fighters: &[FighterView],
        rockets: &[Vec3],
        modules: &[(Vec3, ArenaModule)],
        reticle: Option<camera::Reticle>,
    ) {
        let cables: Vec<(Vec3, Vec3)> = fighters
            .iter()
            .filter_map(|view| {
                let fighter_eye = view.center? + Vec3::Y * view.eye_above_center;
                let from = view
                    .grapple_from
                    .unwrap_or_else(|| gun_pose(fighter_eye, view.forward).1);
                Some((from, view.hook?))
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
                    if view.robot {
                        // Its robot and the weapons on it are drawn by `crate::robots`.
                        (HIDDEN, HIDDEN, gun_material(&self.gun_materials, view.held))
                    } else {
                        (
                            Mat4::from_translation(center),
                            gun_pose(fighter_eye, view.forward).0,
                            gun_material(&self.gun_materials, view.held),
                        )
                    }
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
        while self.modules.len() < modules.len() {
            let instance = world.add_instance(Instance {
                mesh: self.module_mesh,
                material: self.module_materials[0],
                world_from_local: HIDDEN,
                skin: None,
            });
            self.modules.push(instance);
        }
        for (index, instance) in self.modules.iter().enumerate() {
            let (at, material) =
                modules
                    .get(index)
                    .map_or((HIDDEN, self.module_materials[0]), |(pad, module)| {
                        (
                            Mat4::from_translation(*pad + Vec3::Y * MODULE_HEIGHT),
                            self.module_materials[usize::from(module.tag())],
                        )
                    });
            Self::place(world, *instance, self.module_mesh, material, at);
        }
        for (instance, at) in [
            (self.reticle, reticle.as_ref().map(|reticle| reticle.at)),
            (
                self.blocked,
                reticle.as_ref().and_then(|reticle| reticle.blocked),
            ),
        ] {
            let mut placed = world.instance(instance);
            placed.world_from_local = at.map_or(HIDDEN, Mat4::from_translation);
            world.set_instance(instance, placed);
        }
        let guns = reticle.map(|reticle| reticle.guns).unwrap_or_default();
        while self.gun_marks.len() < guns.len() {
            let instance = world.add_instance(Instance {
                mesh: self.gun_mark_mesh,
                material: self.gun_mark_materials[0],
                world_from_local: HIDDEN,
                skin: None,
            });
            self.gun_marks.push(instance);
        }
        for (index, instance) in self.gun_marks.iter().enumerate() {
            let (at, material) =
                guns.get(index)
                    .map_or((HIDDEN, self.gun_mark_materials[0]), |(at, mark)| {
                        let material = match mark {
                            camera::Mark::Target => self.gun_mark_materials[0],
                            camera::Mark::Blocked => self.gun_mark_materials[1],
                            camera::Mark::Unreachable => self.gun_mark_materials[2],
                        };
                        (Mat4::from_translation(*at), material)
                    });
            Self::place(world, *instance, self.gun_mark_mesh, material, at);
        }
    }
}

/// Distance haze ([`Lighting::haze`]): linear from `start` to `end` metres from the camera, where
/// it hides `density` of what is behind it in `color` (exposed, like the clear colour).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Haze {
    pub color: Vec3,
    pub start: f32,
    pub end: f32,
    pub density: f32,
}

/// How a map's hall is lit: the sky's clear colour, a sun through the open roof, the ambient
/// light and an optional distance haze. Presentation only: each kit-built map has its own
/// profile (shooter#151, #152, #153), its sun high (43° and more) and from the north or the
/// south, so the east and west halves sit in the same light, and its robots stay as readable
/// as in the greybox's. The lite renderer draws the haze too (`gfx::lite`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lighting {
    pub clear_color: Vec3,
    /// The direction the sunlight travels (normalised when drawn).
    pub sun_direction: Vec3,
    pub sun_color: Vec3,
    /// Lux.
    pub sun_illuminance: f32,
    pub ambient_color: Vec3,
    /// cd/m².
    pub ambient_brightness: f32,
    pub haze: Option<Haze>,
}

impl Lighting {
    /// The greybox maps' light (and the builder's and the examples' benches): a high sun and
    /// a pale blue sky.
    pub const GREYBOX: Self = Self {
        clear_color: Vec3::new(0.35, 0.45, 0.6),
        sun_direction: Vec3::new(-0.35, -1.0, -0.55),
        sun_color: Vec3::new(1.0, 0.96, 0.9),
        sun_illuminance: 20_000.0,
        ambient_color: Vec3::new(0.85, 0.9, 1.0),
        ambient_brightness: 2_500.0,
        haze: None,
    };

    /// The proving cell (shooter#151): a clean test cell under a white overhead key, almost
    /// straight down so the 32 m cell keeps few shadows, a warm-white fill and an amber glow
    /// over the open roof; no haze at its short ranges.
    pub const PROVING_CELL: Self = Self {
        clear_color: Vec3::new(0.9, 0.62, 0.3),
        sun_direction: Vec3::new(0.0, -1.0, -0.22),
        sun_color: Vec3::new(1.0, 1.0, 0.98),
        sun_illuminance: 26_000.0,
        ambient_color: Vec3::new(1.0, 0.92, 0.8),
        ambient_brightness: 3_400.0,
        haze: None,
    };

    /// The assembly works (shooter#152): cool, clean industrial light, a blue-white sun along
    /// the works' north-south axis, a steel-teal fill and sky, and a thin cool haze over the
    /// far bays.
    pub const ASSEMBLY_WORKS: Self = Self {
        clear_color: Vec3::new(0.16, 0.25, 0.33),
        sun_direction: Vec3::new(0.0, -1.0, 0.42),
        sun_color: Vec3::new(0.84, 0.93, 1.0),
        sun_illuminance: 21_000.0,
        ambient_color: Vec3::new(0.62, 0.84, 1.0),
        ambient_brightness: 2_900.0,
        haze: Some(Haze {
            color: Vec3::new(0.16, 0.25, 0.33),
            start: 20.0,
            end: 110.0,
            density: 0.22,
        }),
    };

    /// The drydock (shooter#153): an expansive open yard, a strong warm sun from the
    /// south along the axis between the two ships (both hulls lit alike, long shadows on the
    /// dock floor), a high bright sky and an atmospheric haze that gives the 80 m yard depth.
    pub const DRYDOCK: Self = Self {
        clear_color: Vec3::new(0.42, 0.6, 0.9),
        sun_direction: Vec3::new(0.0, -1.0, -1.05),
        sun_color: Vec3::new(1.0, 0.87, 0.68),
        sun_illuminance: 26_000.0,
        ambient_color: Vec3::new(0.68, 0.8, 1.0),
        ambient_brightness: 3_000.0,
        haze: Some(Haze {
            color: Vec3::new(0.5, 0.64, 0.86),
            start: 40.0,
            end: 200.0,
            density: 0.25,
        }),
    };

    /// `map`'s light: its own for a kit-built map, [`Lighting::GREYBOX`] for the greybox ones.
    #[must_use]
    pub const fn of(map: MapId) -> Self {
        match map {
            MapId::Greybox | MapId::GreyboxYard => Self::GREYBOX,
            MapId::ProvingCell => Self::PROVING_CELL,
            MapId::AssemblyWorks => Self::ASSEMBLY_WORKS,
            MapId::Drydock => Self::DRYDOCK,
        }
    }
}

/// The frame's camera ([`camera`]) and the hall's light ([`Lighting`]), plus the weapons'
/// flashes.
#[must_use]
pub fn frame(
    camera: Vec3,
    forward: Vec3,
    point_lights: Vec<PointLight>,
    lighting: &Lighting,
) -> Scene {
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
        clear_color: lighting.clear_color,
        sun: Some(Sun {
            direction: lighting.sun_direction.normalize(),
            color: lighting.sun_color,
            illuminance: lighting.sun_illuminance,
            shadows: Some(SunShadows {
                // Far enough to cover the greybox yard's 76 m from corner to corner, and most
                // of the drydock's 80 m from a camera on its floor.
                max_distance: 110.0,
                ..SunShadows::default()
            }),
        }),
        moon: None,
        sky: None,
        point_lights,
        spot_lights: Vec::new(),
        shadow_maps: ShadowMapSizes::default(),
        ambient: Ambient {
            color: lighting.ambient_color,
            brightness: lighting.ambient_brightness,
        },
        environment: None,
        fog: lighting.haze.map(|haze| Fog {
            color: haze.color.extend(haze.density),
            falloff: FogFalloff::Linear {
                start: haze.start,
                end: haze.end,
            },
            ..Fog::default()
        }),
        overlay: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::{Lighting, frame};
    use render_engine::glam::Vec3;
    use shooter_sim::MapId;

    #[test]
    fn each_kit_map_has_its_own_light_and_the_greyboxes_keep_theirs() {
        assert_eq!(Lighting::of(MapId::Greybox), Lighting::GREYBOX);
        assert_eq!(Lighting::of(MapId::GreyboxYard), Lighting::GREYBOX);
        let kit = [MapId::ProvingCell, MapId::AssemblyWorks, MapId::Drydock].map(Lighting::of);
        for (index, lighting) in kit.iter().enumerate() {
            assert_ne!(*lighting, Lighting::GREYBOX);
            assert!(kit[index + 1..].iter().all(|other| other != lighting));
            // East and west alike, and a high sun.
            let sun = lighting.sun_direction.normalize();
            assert!(sun.x.abs() < 1e-6, "{sun}");
            assert!(-sun.y > 0.68, "{sun}");
        }
    }

    #[test]
    fn the_frame_carries_the_light() {
        let lighting = Lighting::DRYDOCK;
        let scene = frame(Vec3::ZERO, Vec3::NEG_Z, Vec::new(), &lighting);
        assert_eq!(scene.clear_color, lighting.clear_color);
        assert!(scene.fog.is_some());
        let scene = frame(Vec3::ZERO, Vec3::NEG_Z, Vec::new(), &Lighting::GREYBOX);
        assert!(scene.fog.is_none());
    }
}
