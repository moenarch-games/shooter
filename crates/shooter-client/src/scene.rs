//! The arena as render-engine content: one mesh per piece at its true size (so normals stay
//! exact), a checkered floor for a sense of speed, and the hall's lighting.

use render_engine::glam::{Mat4, Quat, Vec3, Vec4};
use render_engine::material::StandardMaterial;
use render_engine::mesh::{MeshData, Shape};
use render_engine::scene::{
    Ambient, Camera, Scene, ShadowFilter, ShadowMapSizes, Sun, SunShadows, Tonemapping,
};
use render_engine::world::{Instance, World};
use shooter_sim::sim_foundation::RampRise;
use shooter_sim::sim_foundation::physics::Vec3i;
use shooter_sim::{Arena, Piece};

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
}

/// The frame's camera and the hall's light: a high sun through an open roof and soft ambient.
#[must_use]
pub fn frame(eye: Vec3, forward: Vec3) -> Scene {
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
        point_lights: Vec::new(),
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
