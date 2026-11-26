//! The kit-built arenas drawn from asset-tooling's arena kit (release `shooter-arena`, pinned in
//! `assets/arena-kit.pin.json`): every module's intact and broken mesh and the kit's one
//! material uploaded once ([`ArenaArt::new`]), then an arena's placements
//! (`shooter_sim::Arena::placements`, from which the simulation derives its collision) drawn as
//! instances of them where the simulation puts them, and its floor laid with the kit's 2 m floor
//! plates ([`ArenaArt::draw`]); each destruction candidate in the look the match's snapshot gives
//! it, intact or the kit's broken variant ([`ArenaArt::show`]: the snapshot is the only word on it,
//! shooter#60). The greybox maps have no placements and keep their boxes
//! (`crate::scene`).

use crate::gfx::{Instance, InstanceId, MaterialId, MeshId, TextureId, World};
use crate::robots::draw::material;
use crate::robots::kit::KitData;
use crate::scene::METRES_PER_UNIT;
use render_engine::glam::{Mat4, Quat, Vec3};
use shooter_sim::Arena;
use shooter_sim::arena_kit::{Module, Placement, Variant};
use shooter_sim::scenery::PlaceableState;
use std::collections::HashMap;

/// The floor plates' size (metres): the kit's 2 m grid.
const FLOOR_TILE: f32 = 2.0;

/// The arena kit in the GPU's world.
pub struct ArenaArt {
    /// Each module's mesh and material, by module and look.
    meshes: HashMap<(Module, Variant), (MeshId, MaterialId)>,
}

/// Where `placement`'s module is drawn: its origin (metres) and its yaw about +Y, a quarter
/// turn taking +Z to +X as in the kit and the simulation.
#[must_use]
pub fn transform(placement: &Placement) -> Mat4 {
    #[allow(clippy::cast_precision_loss)]
    let at = Vec3::new(
        placement.at.x as f32,
        placement.at.y as f32,
        placement.at.z as f32,
    ) * METRES_PER_UNIT;
    let yaw = f32::from(placement.turn.degrees()).to_radians();
    Mat4::from_rotation_translation(Quat::from_rotation_y(yaw), at)
}

impl ArenaArt {
    /// Uploads the decoded arena kit into `world`.
    ///
    /// # Errors
    ///
    /// If a module of the simulation's table (or its broken look) is not in the kit.
    pub fn new(world: &mut World, mut kit: KitData) -> Result<Self, String> {
        let textures: Vec<TextureId> = kit
            .textures
            .drain(..)
            .map(|texture| world.add_texture(&texture))
            .collect();
        let materials: Vec<MaterialId> = kit
            .materials
            .iter()
            .map(|kit| world.add_material(&material(kit, &textures)))
            .collect();
        let mut meshes = HashMap::new();
        for module in Module::ALL {
            let looks = std::iter::once(Variant::Intact)
                .chain(module.broken_proxies().map(|_| Variant::Broken));
            for variant in looks {
                let index = kit.entry(module.key(), variant.bundle()).ok_or_else(|| {
                    format!("arena kit: no {} {}", module.key(), variant.bundle())
                })?;
                let entry = &mut kit.entries[index];
                let mesh = world.add_mesh(&std::mem::take(&mut entry.mesh));
                meshes.insert((module, variant), (mesh, materials[entry.material]));
            }
        }
        Ok(Self { meshes })
    }

    /// The mesh and material `placement` is drawn with.
    fn look(&self, placement: &Placement) -> (MeshId, MaterialId) {
        self.meshes
            .get(&(placement.module, placement.variant))
            .or_else(|| self.meshes.get(&(placement.module, Variant::Intact)))
            .copied()
            .expect("every module is uploaded")
    }

    /// Retains `arena`'s floor plates and placed modules in `world`: they are drawn every frame.
    /// Returns the placed modules, whose looks [`ArenaArt::show`] follows.
    pub fn draw(&self, world: &mut World, arena: &Arena) -> Placed {
        let (plate, plate_material) = self.meshes[&(Module::Floor, Variant::Intact)];
        #[allow(clippy::cast_precision_loss)]
        let half = arena.half_size as f32 * METRES_PER_UNIT;
        #[allow(clippy::cast_possible_truncation)]
        let tiles = (2.0 * half / FLOOR_TILE).round() as i32;
        let mut plates = Vec::new();
        for row in 0..tiles {
            for column in 0..tiles {
                #[allow(clippy::cast_precision_loss)]
                let center = Vec3::new(
                    -half + (column as f32 + 0.5) * FLOOR_TILE,
                    0.0,
                    -half + (row as f32 + 0.5) * FLOOR_TILE,
                );
                plates.push(world.add_instance(Instance {
                    mesh: plate,
                    material: plate_material,
                    world_from_local: Mat4::from_translation(center),
                    skin: None,
                }));
            }
        }
        let placements = arena
            .placements
            .iter()
            .map(|placement| {
                let (mesh, material) = self.look(placement);
                let id = world.add_instance(Instance {
                    mesh,
                    material,
                    world_from_local: transform(placement),
                    skin: None,
                });
                (id, *placement)
            })
            .collect();
        Placed {
            plates,
            authored: placements,
        }
    }

    /// Brings the placed modules' looks to the match's word (a snapshot's
    /// `shooter_sim::Snapshot::scenery`): each placeable broken there drawn as the kit's broken
    /// variant, every other one as its map authored it. Only the changed ones are touched.
    pub fn show(&self, world: &mut World, placed: &Placed, scenery: &[PlaceableState]) {
        for (index, (id, authored)) in placed.authored.iter().enumerate() {
            // Only the map's destruction candidates ever change.
            if !authored.destructible {
                continue;
            }
            let broken = scenery
                .iter()
                .any(|placeable| usize::from(placeable.id) == index && placeable.health == 0);
            let placement = if broken { authored.broken() } else { *authored };
            let (mesh, material) = self.look(&placement);
            world.set_instance(
                *id,
                Instance {
                    mesh,
                    material,
                    world_from_local: transform(&placement),
                    skin: None,
                },
            );
        }
    }
}

/// A kit-built arena's modules in the GPU's world ([`ArenaArt::draw`]): each placement's
/// instance and the placement as its map authored it, in placement order (a placeable's stable
/// ID is its index).
pub struct Placed {
    /// The floor plates' instances.
    plates: Vec<InstanceId>,
    authored: Vec<(InstanceId, Placement)>,
}

impl Placed {
    /// Every instance the arena's modules and floor plates have in the world.
    pub fn instances(&self) -> impl Iterator<Item = InstanceId> + '_ {
        self.plates
            .iter()
            .copied()
            .chain(self.authored.iter().map(|(id, _)| *id))
    }
}
