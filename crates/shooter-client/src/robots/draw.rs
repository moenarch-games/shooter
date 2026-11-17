//! The robots in render-engine's world: the kits' meshes, textures and materials uploaded once,
//! and per fighter slot the instances (and, for skinned parts, joint palettes) of its build and
//! its weapons' mods, rebuilt when its build changes and posed every frame ([`super::pose`]).

use super::kit::{KitData, KitMaterial};
use super::model::Model;
use super::pose::{self, Muzzle, PartStates, RobotState};
use super::weapon::Memory;
use render_engine::glam::{Mat4, Quat, Vec3, Vec4};
use render_engine::material::{AlphaMode, StandardMaterial};
use render_engine::mesh::{MeshData, Shape};
use render_engine::world::{Instance, InstanceId, MaterialId, MeshId, SkinId, TextureId, World};
use shooter_sim::Assembly;
use shooter_sim::robot::LOADOUTS;
use std::collections::HashMap;
use std::rc::Rc;

/// The kit's emissive factor 1 in render-engine's units (nits): its glows (the lens,
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
    /// Where its grapple's cable starts (the launcher's muzzle).
    pub grapple: Option<Vec3>,
    /// Its attached weapons' muzzles, where their shots are drawn from.
    pub muzzles: Vec<Muzzle>,
}

struct SlotRobot {
    build: Assembly,
    model: Rc<Model>,
    /// One per model part, with its skin when skinned.
    instances: Vec<(InstanceId, Option<SkinId>)>,
    /// One per mod ([`Model::mods`]).
    mods: Vec<InstanceId>,
    /// This fighter's copies of the kit's materials (hit glow).
    materials: Vec<MaterialId>,
    /// The quantised glow `materials` show (render-engine rebuilds a material's GPU buffer on
    /// each `set_material`, so they change only when this does).
    glow: Option<u8>,
    marker: InstanceId,
    /// Its weapons' transitions, from its first frame.
    memory: Option<Memory>,
}

/// Every fighter's robot.
pub struct Robots {
    kit: KitData,
    meshes: Vec<MeshId>,
    materials: Vec<StandardMaterial>,
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
pub(crate) fn material(kit: &KitMaterial, textures: &[TextureId]) -> StandardMaterial {
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
    /// Uploads `kit` (both kits) into `world` and checks that every named loadout draws.
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
        Ok(Self {
            meshes,
            materials,
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
        for instance in robot.mods {
            world.remove_instance(instance);
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
            .map(|material| world.add_material(material))
            .collect();
        let instances = model
            .parts
            .iter()
            .map(|part| {
                let entry = part.entry;
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
            })
            .collect();
        let mods = model
            .mods
            .iter()
            .map(|modification| {
                world.add_instance(Instance {
                    mesh: self.meshes[modification.entry],
                    material: materials[self.kit.entries[modification.entry].material],
                    world_from_local: HIDDEN,
                    skin: None,
                })
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
            mods,
            materials,
            glow: None,
            marker,
            memory: None,
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
            let memory = robot
                .memory
                .get_or_insert_with(|| Memory::settled(&view.state));
            memory.advance(&view.state, seconds);
            let posed = pose::pose(&robot.model, &self.kit, &view.state, &view.parts, memory);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let glow = (view.glow.clamp(0.0, 1.0) * 8.0).round() as u8;
            if robot.glow != Some(glow) {
                // The copies are in kit order: tinted toward red and glowing brighter when hit.
                robot.glow = Some(glow);
                let glow = f32::from(glow) / 8.0;
                for (base, id) in self.materials.iter().zip(&robot.materials) {
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
                let entry = match (posed_part.damaged, part.damaged) {
                    (true, Some(damaged)) => damaged,
                    _ => part.entry,
                };
                let (mesh, material) = (
                    self.meshes[entry],
                    robot.materials[self.kit.entries[entry].material],
                );
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
            for (instance, world_from_local) in robot.mods.iter().zip(&posed.mods) {
                let mut placed = world.instance(*instance);
                placed.world_from_local = *world_from_local;
                world.set_instance(*instance, placed);
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
                label: posed.animation.label(),
                grapple: posed.grapple,
                muzzles: posed.muzzles,
            });
        }
        for (slot, robot) in self.slots.iter().enumerate() {
            let Some(robot) = robot.as_ref().filter(|_| !shown[slot]) else {
                continue;
            };
            let instances = robot.instances.iter().map(|(instance, _)| *instance);
            for instance in instances
                .chain(robot.mods.iter().copied())
                .chain([robot.marker])
            {
                let mut hidden = world.instance(instance);
                hidden.world_from_local = HIDDEN;
                world.set_instance(instance, hidden);
            }
        }
        Ok(drawn)
    }
}
