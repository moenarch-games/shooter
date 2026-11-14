//! The robots in render-engine's world: the kit's meshes, textures and material uploaded once,
//! and per fighter slot the instances (and, for skinned parts, joint palettes) of its build,
//! rebuilt when its build changes and posed every frame ([`super::pose`]).

use super::kit::{KitData, KitMaterial};
use super::model::{Model, Visual};
use super::pose::{self, PartStates, RobotState};
use render_engine::glam::{Mat4, Quat, Vec3, Vec4};
use render_engine::material::{AlphaMode, StandardMaterial};
use render_engine::mesh::{MeshData, Shape};
use render_engine::world::{Instance, InstanceId, MaterialId, MeshId, SkinId, TextureId, World};
use shooter_sim::Assembly;
use shooter_sim::Weapon;
use shooter_sim::robot::{Kind, LOADOUTS};
use std::collections::HashMap;
use std::rc::Rc;

/// The kit's emissive factor 1 in render-engine's units (nits): its glows (the core, the lens,
/// the charge window) read under the arena's exposure (EV100 13).
const KIT_EMISSIVE_NITS: f32 = 3_000.0;
/// Where an unused instance is drawn: under the floor.
const HIDDEN: Mat4 = Mat4::from_cols(Vec4::X, Vec4::Y, Vec4::Z, Vec4::new(0.0, -100.0, 0.0, 1.0));
/// The missing-animation marker's edge (metres).
const MARKER_SIZE: f32 = 0.14;

/// One fighter this frame, as the match says.
#[derive(Clone, Copy, Debug)]
pub struct RobotView<'a> {
    pub slot: u8,
    pub build: Assembly,
    pub state: RobotState,
    /// Hit glow, 1 just hit and fading to 0.
    pub glow: f32,
    pub parts: PartStates<'a>,
}

/// What was drawn for a fighter.
#[derive(Clone, Debug, PartialEq)]
pub struct Drawn {
    pub slot: u8,
    /// Its animation state ([`pose::Animation::label`]).
    pub label: String,
    /// Its placeholder parts and the issues that make them.
    pub placeholders: Vec<(&'static str, &'static str)>,
    /// Where its grapple weapon is (the cable's start).
    pub grapple: Option<Vec3>,
    /// The pointed laser barrel tip, if the build has a mounted laser.
    pub laser_muzzle: Option<Vec3>,
}

struct SlotRobot {
    build: Assembly,
    model: Rc<Model>,
    /// One per model part, with its skin when skinned.
    instances: Vec<(InstanceId, Option<SkinId>)>,
    /// This fighter's copies of the kit's materials (hit glow).
    materials: Vec<MaterialId>,
    /// The quantised glow `materials` show (render-engine rebuilds a material's GPU buffer on
    /// each `set_material`, so they change only when this does).
    glow: Option<u8>,
    marker: InstanceId,
    phase: f32,
}

/// Every fighter's robot.
pub struct Robots {
    kit: KitData,
    meshes: Vec<MeshId>,
    materials: Vec<StandardMaterial>,
    placeholder_mesh: MeshId,
    placeholder: MaterialId,
    placeholder_held: MaterialId,
    placeholder_flash: MaterialId,
    placeholder_laser: MaterialId,
    battery: StandardMaterial,
    marker_mesh: MeshId,
    marker_material: MaterialId,
    models: HashMap<Assembly, Rc<Model>>,
    slots: Vec<Option<SlotRobot>>,
    clock: f32,
}

fn glowing(rgba: Vec4, nits: f32) -> StandardMaterial {
    StandardMaterial {
        base_color: rgba,
        emissive: rgba.truncate() * nits,
        perceptual_roughness: 0.5,
        alpha_mode: if rgba.w < 1.0 {
            AlphaMode::Blend
        } else {
            AlphaMode::Opaque
        },
        ..StandardMaterial::default()
    }
}

/// The kit's material with `textures` (by kit index).
fn material(kit: &KitMaterial, textures: &[TextureId]) -> StandardMaterial {
    let texture = |index: Option<usize>| index.and_then(|index| textures.get(index).copied());
    StandardMaterial {
        base_color: kit.base_color,
        base_color_texture: texture(kit.color),
        emissive: kit.emissive * KIT_EMISSIVE_NITS,
        emissive_texture: texture(kit.emissive_texture),
        metallic: kit.metallic,
        perceptual_roughness: kit.roughness,
        metallic_roughness_texture: texture(kit.metallic_roughness),
        normal_map_texture: texture(kit.normal),
        occlusion_texture: texture(kit.occlusion),
        double_sided: kit.double_sided,
        ..StandardMaterial::default()
    }
}

impl Robots {
    /// Uploads `kit` into `world` and checks that every named loadout draws.
    ///
    /// # Errors
    ///
    /// If a loadout uses a part the kit cannot draw.
    pub fn new(world: &mut World, mut kit: KitData) -> Result<Self, String> {
        let mut models = HashMap::new();
        for (name, build) in LOADOUTS {
            let model = Model::of(&build, &kit).map_err(|error| format!("{name}: {error}"))?;
            models.insert(build, Rc::new(model));
        }
        let textures: Vec<TextureId> = kit
            .textures
            .drain(..)
            .map(|texture| world.add_texture(&texture))
            .collect();
        let meshes = kit
            .entries
            .iter_mut()
            .map(|entry| world.add_mesh(&std::mem::take(&mut entry.mesh)))
            .collect();
        let materials = kit
            .materials
            .iter()
            .map(|kit| material(kit, &textures))
            .collect();
        let purple = Vec4::new(0.62, 0.2, 1.0, 0.55);
        Ok(Self {
            meshes,
            materials,
            placeholder_mesh: world.add_mesh(&MeshData::shape(Shape::Cuboid(Vec3::ONE))),
            placeholder: world.add_material(&glowing(purple, 600.0)),
            placeholder_held: world.add_material(&glowing(purple.with_w(0.75), 3_000.0)),
            placeholder_flash: world
                .add_material(&glowing(Vec4::new(1.0, 0.85, 1.0, 0.9), 30_000.0)),
            // Laser remains a declared weapon-kit placeholder, but its green housing makes the
            // aim-tracking turret distinguishable from the other unmodelled weapon modules.
            placeholder_laser: world
                .add_material(&glowing(Vec4::new(0.14, 0.8, 0.34, 0.8), 2_000.0)),
            // Recolour the real asset-tooling battery mesh, retaining its shape and stable ID.
            battery: glowing(Vec4::new(1.0, 0.38, 0.04, 1.0), 650.0),
            marker_mesh: world.add_mesh(&MeshData::shape(Shape::Cuboid(Vec3::splat(MARKER_SIZE)))),
            marker_material: world.add_material(&glowing(Vec4::new(1.0, 0.0, 0.8, 1.0), 20_000.0)),
            kit,
            models,
            slots: Vec::new(),
            clock: 0.0,
        })
    }

    /// The model of `build`, laid out once.
    fn model(&mut self, build: &Assembly) -> Result<Rc<Model>, String> {
        if let Some(model) = self.models.get(build) {
            return Ok(model.clone());
        }
        let model = Rc::new(Model::of(build, &self.kit)?);
        self.models.insert(*build, model.clone());
        Ok(model)
    }

    fn remove(world: &mut World, robot: SlotRobot) {
        for (instance, skin) in robot.instances {
            world.remove_instance(instance);
            if let Some(skin) = skin {
                world.remove_skin(skin);
            }
        }
        world.remove_instance(robot.marker);
        for material in robot.materials {
            world.remove_material(material);
        }
    }

    fn spawn(&self, world: &mut World, build: Assembly, model: Rc<Model>) -> SlotRobot {
        let materials: Vec<MaterialId> = self
            .materials
            .iter()
            .chain(std::iter::once(&self.battery))
            .map(|material| world.add_material(material))
            .collect();
        let instances = model
            .parts
            .iter()
            .map(|part| match part.visual {
                Visual::Kit { entry, .. } => {
                    let skin = self.kit.entries[entry]
                        .rig
                        .as_ref()
                        .map(|rig| world.add_skin(&rig.palette(&rig.rest())));
                    let instance = world.add_instance(Instance {
                        mesh: self.meshes[entry],
                        material: materials[self.kit.entries[entry].material],
                        world_from_local: HIDDEN,
                        skin,
                    });
                    (instance, skin)
                }
                Visual::Placeholder { .. } => (
                    world.add_instance(Instance {
                        mesh: self.placeholder_mesh,
                        material: self.placeholder,
                        world_from_local: HIDDEN,
                        skin: None,
                    }),
                    None,
                ),
            })
            .collect();
        let marker = world.add_instance(Instance {
            mesh: self.marker_mesh,
            material: self.marker_material,
            world_from_local: HIDDEN,
            skin: None,
        });
        SlotRobot {
            build,
            model,
            instances,
            materials,
            glow: None,
            marker,
            phase: 0.0,
        }
    }

    /// Draws `views` (one per fighter shown, by slot) `seconds` after the last frame; every
    /// other slot's robot is hidden.
    ///
    /// # Errors
    ///
    /// If a fighter's build uses a part the kit cannot draw.
    pub fn update(
        &mut self,
        world: &mut World,
        views: &[RobotView],
        seconds: f32,
    ) -> Result<Vec<Drawn>, String> {
        self.clock += seconds;
        let mut drawn = Vec::with_capacity(views.len());
        let mut shown = vec![false; self.slots.len()];
        for view in views {
            let slot = usize::from(view.slot);
            if self.slots.len() <= slot {
                self.slots.resize_with(slot + 1, || None);
                shown.resize(slot + 1, false);
            }
            shown[slot] = true;
            if self.slots[slot]
                .as_ref()
                .is_none_or(|robot| robot.build != view.build)
            {
                let model = self.model(&view.build)?;
                if let Some(old) = self.slots[slot].take() {
                    Self::remove(world, old);
                }
                self.slots[slot] = Some(self.spawn(world, view.build, model));
            }
            let Some(robot) = &mut self.slots[slot] else {
                continue;
            };
            robot.phase = pose::advance_phase(robot.phase, &robot.model, &view.state, seconds);
            let posed = pose::pose(
                &robot.model,
                &self.kit,
                &view.state,
                robot.phase,
                &view.parts,
            );
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let glow = (view.glow.clamp(0.0, 1.0) * 8.0).round() as u8;
            if robot.glow != Some(glow) {
                // The copies are in kit order: tinted toward red and glowing brighter when hit.
                robot.glow = Some(glow);
                let glow = f32::from(glow) / 8.0;
                for (base, id) in self
                    .materials
                    .iter()
                    .chain(std::iter::once(&self.battery))
                    .zip(&robot.materials)
                {
                    let mut lit = *base;
                    lit.base_color *= Vec4::ONE.lerp(Vec4::new(1.0, 0.45, 0.35, 1.0), glow);
                    lit.emissive *= 1.0 + 3.0 * glow;
                    world.set_material(*id, &lit);
                }
            }
            for ((part, posed_part), (instance, skin)) in robot
                .model
                .parts
                .iter()
                .zip(&posed.parts)
                .zip(&robot.instances)
            {
                let (mesh, material) = match part.visual {
                    Visual::Kit { entry, damaged } => {
                        let entry = if posed_part.damaged {
                            damaged.unwrap_or(entry)
                        } else {
                            entry
                        };
                        (
                            self.meshes[entry],
                            if part.kind == Kind::Battery {
                                robot.materials[self.materials.len()]
                            } else {
                                robot.materials[self.kit.entries[entry].material]
                            },
                        )
                    }
                    Visual::Placeholder { .. } => (
                        self.placeholder_mesh,
                        if posed_part.flash {
                            self.placeholder_flash
                        } else if part.kind == Kind::Weapon(Weapon::Laser) {
                            self.placeholder_laser
                        } else if posed_part.held {
                            self.placeholder_held
                        } else {
                            self.placeholder
                        },
                    ),
                };
                if let (Some(skin), Some(palette)) = (skin, &posed_part.palette) {
                    world.set_skin(*skin, palette);
                }
                world.set_instance(
                    *instance,
                    Instance {
                        mesh,
                        material,
                        world_from_local: posed_part.world_from_local,
                        skin: *skin,
                    },
                );
            }
            let marker = if posed.animation.missing().is_some() {
                Mat4::from_rotation_translation(
                    Quat::from_rotation_y(self.clock * 3.0)
                        * Quat::from_rotation_x(std::f32::consts::FRAC_PI_4)
                        * Quat::from_rotation_z(std::f32::consts::FRAC_PI_4),
                    posed.marker,
                )
            } else {
                HIDDEN
            };
            let mut instance = world.instance(robot.marker);
            instance.world_from_local = marker;
            world.set_instance(robot.marker, instance);
            drawn.push(Drawn {
                slot: view.slot,
                label: posed.animation.label(&robot.model),
                placeholders: robot.model.placeholders(),
                grapple: posed.grapple,
                laser_muzzle: posed.laser_muzzle,
            });
        }
        for (slot, robot) in self.slots.iter().enumerate() {
            let Some(robot) = robot.as_ref().filter(|_| !shown[slot]) else {
                continue;
            };
            let instances = robot.instances.iter().map(|(instance, _)| *instance);
            for instance in instances.chain([robot.marker]) {
                let mut hidden = world.instance(instance);
                hidden.world_from_local = HIDDEN;
                world.set_instance(instance, hidden);
            }
        }
        Ok(drawn)
    }
}
