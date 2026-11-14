//! The robot kit as the clients draw it, against the pinned asset-tooling release in
//! `target/robot-kit` (or `$SHOOTER_ROBOT_KIT`; fetch it with `scripts/fetch-robot-kit.sh`): it
//! decodes within its bounds, every loadout maps onto kit entries and labelled placeholders, the
//! parts sit inside the cuboids the simulation places, and the poses, lost parts and joint pivots
//! follow the authoritative state.

#![cfg(not(target_arch = "wasm32"))]

use render_engine::glam::{Mat4, Vec3};
use shooter_client::robots::kit::{KitData, MAX_TOTAL_BYTES, Plan};
use shooter_client::robots::load;
use shooter_client::robots::model::{Model, Visual};
use shooter_client::robots::pose::{
    self, Action, Activity, Animation, HALF_HEIGHT, PartStates, RobotState, SIM_JOINT_ANCHOR, SOLE,
};
use shooter_sim::mechanics::{Layout, fixtures, placements};
use shooter_sim::robot::{HEAVY, LOADOUTS, MARKSMAN, PartId, SKIRMISHER, STANDARD};
use shooter_sim::weapons::Weapon;
use std::sync::OnceLock;

fn kit() -> &'static KitData {
    static KIT: OnceLock<KitData> = OnceLock::new();
    KIT.get_or_init(|| {
        load::read(&load::directory()).unwrap_or_else(|error| {
            panic!("{error}\n(the robot kit tests need scripts/fetch-robot-kit.sh first)")
        })
    })
}

fn standing() -> RobotState {
    RobotState {
        center: Vec3::new(1.0, 2.0, 3.0),
        forward: Vec3::Z,
        velocity: Vec3::ZERO,
        sliding: false,
        hooked: false,
        since_fired: f32::INFINITY,
        held: Weapon::EnergyRifle,
        swing: None,
        dead_for: None,
    }
}

#[test]
fn the_pinned_kit_decodes_within_its_bounds() {
    let manifest = std::fs::read(load::directory().join("manifest.json")).unwrap();
    let plan = Plan::of(&manifest).expect("the pinned manifest");
    let bytes: usize = plan
        .files()
        .iter()
        .map(|file| file.byte_length.unwrap_or(0))
        .sum();
    assert!(bytes < MAX_TOTAL_BYTES, "{bytes}");
    let kit = kit();
    // Twelve parts at full detail and the plate's damaged look; one material, four textures.
    assert_eq!(kit.entries.len(), 13);
    assert_eq!(kit.materials.len(), 1);
    assert_eq!(kit.textures.len(), 4);
    assert!(kit.textures.iter().all(|t| t.width == 1024));
    let leg = &kit.entries[kit.entry("robot.leg", "default").unwrap()];
    let rig = leg.rig.as_ref().expect("the leg is rigged");
    for joint in ["base", "hip", "knee", "ankle"] {
        assert!(rig.joint(joint).is_some(), "{joint}");
    }
    assert!(rig.clip("walk-biped").is_some() && rig.clip("walk-quadruped").is_some());
    for key in ["robot.joint.hinge", "robot.joint.rotor"] {
        let entry = &kit.entries[kit.entry(key, "default").unwrap()];
        assert!(entry.rig.is_some(), "{key}");
    }
    // At rest every palette is the identity: the bind pose is the modelled pose.
    for matrix in rig.palette(&rig.rest()) {
        assert!(matrix.abs_diff_eq(Mat4::IDENTITY, 1e-5), "{matrix}");
    }
}

#[test]
fn a_tampered_file_is_refused() {
    let directory = load::directory();
    let plan = Plan::of(&std::fs::read(directory.join("manifest.json")).unwrap()).unwrap();
    let mut files = load::Files::new();
    for file in plan.files() {
        files.insert(
            file.path.clone(),
            std::fs::read(directory.join(&file.path)).unwrap(),
        );
    }
    let first = plan.entries[0].scene.path.clone();
    files.get_mut(&first).unwrap().push(b' ');
    let error = shooter_client::robots::kit::decode(&plan, &files).unwrap_err();
    assert!(error.contains(&first), "{error}");
    files.remove(&first);
    let error = shooter_client::robots::kit::decode(&plan, &files).unwrap_err();
    assert!(error.contains("missing"), "{error}");
}

#[test]
fn loadouts_draw_their_own_parts_with_labelled_placeholders() {
    let kit = kit();
    for (name, build) in LOADOUTS {
        let model = Model::of(&build, kit).expect(name);
        assert_eq!(model.parts.len(), build.parts().len(), "{name}");
        for (part, drawn) in build.parts().iter().zip(&model.parts) {
            assert_eq!(part.id, drawn.id);
            let kit_part = matches!(drawn.visual, Visual::Kit { entry, .. } if kit.entries[entry].key == drawn.key);
            let placeholder = matches!(
                drawn.visual,
                Visual::Placeholder {
                    owner: "asset-tooling#594"
                }
            ) && drawn.key.starts_with("weapon.");
            assert!(kit_part || placeholder, "{name}: {}", drawn.key);
        }
        assert_eq!(model.gait, Some("walk-biped"), "{name}");
    }
    let keys = |model: &Model| -> Vec<&str> { model.parts.iter().map(|p| p.key).collect() };
    let standard = Model::of(&STANDARD, kit).unwrap();
    let heavy = Model::of(&HEAVY, kit).unwrap();
    assert_ne!(keys(&standard), keys(&heavy));
    assert!(keys(&standard).contains(&"robot.chassis.20x40x80"));
    assert!(keys(&heavy).contains(&"robot.chassis.40x40x80"));
    assert!(keys(&heavy).contains(&"robot.battery"));
    assert_eq!(
        standard.placeholders(),
        [
            ("weapon.energy-rifle", "asset-tooling#594"),
            ("weapon.energy-blade", "asset-tooling#594"),
            ("weapon.grapple-hook", "asset-tooling#594"),
            ("weapon.shotgun", "asset-tooling#594"),
        ]
    );
    // A heavy mod lengthens its weapon's placeholder like its footprint.
    let marksman = Model::of(&MARKSMAN, kit).unwrap();
    let skirmisher = Model::of(&SKIRMISHER, kit).unwrap();
    let rifle = |model: &Model| {
        let part = model
            .parts
            .iter()
            .find(|p| p.key == "weapon.energy-rifle")
            .unwrap();
        part.robot_from_local
            .to_scale_rotation_translation()
            .0
            .element_product()
    };
    assert!(rifle(&marksman) > rifle(&skirmisher) * 1.9);
}

/// The robot-frame bounds of a kit entry's mesh placed by `robot_from_local`.
fn bounds(kit: &KitData, entry: usize, robot_from_local: Mat4) -> (Vec3, Vec3) {
    kit.entries[entry]
        .mesh
        .positions
        .iter()
        .map(|p| robot_from_local.transform_point3(*p))
        .fold((Vec3::MAX, Vec3::MIN), |(lo, hi), p| (lo.min(p), hi.max(p)))
}

#[test]
fn every_kit_part_sits_inside_the_cuboid_the_simulation_places() {
    // The meshes are uploaded (and dropped) by the GPU path only; this kit keeps them.
    let kit = kit();
    for (name, build) in LOADOUTS {
        let model = Model::of(&build, kit).unwrap();
        for (drawn, placement) in model.parts.iter().zip(placements(&build)) {
            let Visual::Kit { entry, .. } = drawn.visual else {
                continue;
            };
            let (lo, hi) = bounds(kit, entry, drawn.robot_from_local);
            let rotation = render_engine::glam::Quat::from_array(placement.orientation);
            let half = Vec3::from(placement.half_extents) * 0.01;
            let axes = render_engine::glam::Mat3::from_quat(rotation);
            let reach = axes.x_axis.abs() * half.x
                + axes.y_axis.abs() * half.y
                + axes.z_axis.abs() * half.z;
            let centre = Vec3::from(placement.centre) * 0.01;
            let (cuboid_lo, cuboid_hi) = (centre - reach, centre + reach);
            assert!(
                lo.cmpge(cuboid_lo - 0.002).all() && hi.cmple(cuboid_hi + 0.002).all(),
                "{name}: {} {lo}..{hi} outside {cuboid_lo}..{cuboid_hi}",
                drawn.key
            );
            // And fills it: no part is drawn turned onto another axis.
            assert!(
                (hi - lo).cmpge((cuboid_hi - cuboid_lo) * 0.6).all(),
                "{name}: {} {lo}..{hi} does not fill {cuboid_lo}..{cuboid_hi}",
                drawn.key
            );
        }
    }
    // The legs' toes point forward (+X) and the soles are the robot's floor.
    let model = Model::of(&STANDARD, kit).unwrap();
    for drawn in model.parts.iter().filter(|p| p.key == "robot.leg") {
        let Visual::Kit { entry, .. } = drawn.visual else {
            panic!()
        };
        let (lo, hi) = bounds(kit, entry, drawn.robot_from_local);
        let sole = drawn.robot_from_local.transform_point3(SOLE);
        assert!(
            (sole.y - model.floor).abs() < 0.01,
            "{sole} {}",
            model.floor
        );
        let toe: Vec<Vec3> = kit.entries[entry]
            .mesh
            .positions
            .iter()
            .map(|p| drawn.robot_from_local.transform_point3(*p))
            .filter(|p| p.y < lo.y + 0.03)
            .collect();
        #[allow(clippy::cast_precision_loss)]
        let toe_x = toe.iter().map(|p| p.x).sum::<f32>() / toe.len() as f32;
        assert!(toe_x > (lo.x + hi.x) / 2.0, "toe {toe_x} in {lo}..{hi}");
    }
}

#[test]
fn a_standing_robot_stands_on_the_capsule_feet_and_faces_its_look() {
    let kit = kit();
    let model = Model::of(&STANDARD, kit).unwrap();
    let state = standing();
    let posed = pose::pose(&model, kit, &state, 0.0, &PartStates::default());
    assert_eq!(
        posed.animation,
        Animation {
            activity: Activity::Idle,
            action: Action::None
        }
    );
    let feet = state.center.y - HALF_HEIGHT;
    let lowest = posed
        .world_from_robot
        .transform_point3(Vec3::Y * model.floor)
        .y;
    assert!((lowest - feet).abs() < 1e-4, "{lowest} {feet}");
    // Robot +X is the look.
    let forward = posed.world_from_robot.transform_vector3(Vec3::X);
    assert!(forward.abs_diff_eq(Vec3::Z, 1e-5), "{forward}");
    // About as tall as the capsule.
    assert!((model.top - model.floor - 2.0 * HALF_HEIGHT).abs() < 0.25);
}

#[test]
fn running_walks_the_gait_clip_with_the_feet_on_the_floor() {
    let kit = kit();
    let model = Model::of(&STANDARD, kit).unwrap();
    let running = RobotState {
        velocity: Vec3::new(0.0, 0.0, 6.0),
        ..standing()
    };
    let phase = pose::advance_phase(0.0, &model, &running, 0.1);
    assert!(
        (phase - pose::MAX_CYCLES_PER_SECOND * 0.1).abs() < 1e-5,
        "{phase}"
    );
    let backing = RobotState {
        velocity: Vec3::new(0.0, 0.0, -6.0),
        ..standing()
    };
    assert!(pose::advance_phase(0.5, &model, &backing, 0.1) < 0.5);
    let rest = pose::pose(&model, kit, &standing(), 0.0, &PartStates::default());
    let posed = pose::pose(&model, kit, &running, 0.3, &PartStates::default());
    assert_eq!(posed.animation.activity, Activity::Run);
    assert!(posed.animation.missing().is_none());
    let legs: Vec<usize> = (0..model.parts.len())
        .filter(|&i| model.parts[i].key == "robot.leg")
        .collect();
    assert_eq!(legs.len(), 2);
    // The legs move, half a cycle apart, and the lowest sole stays on the capsule's feet.
    let sole = |posed: &pose::Posed, leg: usize| {
        let rig = kit.entries[kit.entry("robot.leg", "default").unwrap()]
            .rig
            .as_ref()
            .unwrap();
        let palette = posed.parts[leg].palette.as_ref().unwrap();
        (posed.parts[leg].world_from_local * palette[rig.joint("ankle").unwrap()])
            .transform_point3(SOLE)
    };
    let (a, b) = (sole(&posed, legs[0]), sole(&posed, legs[1]));
    assert!(sole(&rest, legs[0]).distance(a) > 0.05);
    let spread = (0..10).map(|step| {
        #[allow(clippy::cast_precision_loss)]
        let posed = pose::pose(
            &model,
            kit,
            &running,
            step as f32 / 10.0,
            &PartStates::default(),
        );
        (sole(&posed, legs[0]).z - sole(&posed, legs[1]).z).abs()
    });
    assert!(spread.fold(0.0, f32::max) > 0.2);
    let feet = running.center.y - HALF_HEIGHT;
    assert!((a.y.min(b.y) - feet).abs() < 1e-3, "{a} {b} {feet}");
}

#[test]
fn every_pose_is_named_and_missing_animation_is_explicit() {
    let kit = kit();
    let model = Model::of(&STANDARD, kit).unwrap();
    let cases = [
        (standing(), "idle", None),
        (
            RobotState {
                velocity: Vec3::new(5.0, 0.0, 0.0),
                ..standing()
            },
            "strafe (walk-biped)",
            Some("strafe"),
        ),
        (
            RobotState {
                velocity: Vec3::new(0.0, 4.0, 0.0),
                ..standing()
            },
            "jump/fall (procedural tuck)",
            None,
        ),
        (
            RobotState {
                hooked: true,
                ..standing()
            },
            "grapple (procedural tuck)",
            None,
        ),
        (
            RobotState {
                sliding: true,
                ..standing()
            },
            "slide (procedural crouch)",
            None,
        ),
        (
            RobotState {
                since_fired: 0.0,
                ..standing()
            },
            "idle + fire",
            None,
        ),
        (
            RobotState {
                swing: Some(0.5),
                ..standing()
            },
            "idle + blade",
            Some("blade swing"),
        ),
        (
            RobotState {
                dead_for: Some(0.2),
                ..standing()
            },
            "death (procedural topple)",
            None,
        ),
    ];
    for (state, label, missing) in cases {
        let posed = pose::pose(&model, kit, &state, 0.0, &PartStates::default());
        assert_eq!(posed.animation.missing(), missing, "{label}");
        assert!(posed.animation.label(&model).starts_with(label), "{label}");
        if let Some(missing) = missing {
            assert!(posed.animation.label(&model).contains(missing));
        }
    }
    // A slide crouches: the body drops while the feet stay down.
    let slide = pose::pose(
        &model,
        kit,
        &RobotState {
            sliding: true,
            ..standing()
        },
        0.0,
        &PartStates::default(),
    );
    let idle = pose::pose(&model, kit, &standing(), 0.0, &PartStates::default());
    let height = |posed: &pose::Posed| posed.world_from_robot.w_axis.y;
    assert!(height(&slide) < height(&idle) - 0.1);
    // A death topples it onto its back.
    let dead = pose::pose(
        &model,
        kit,
        &RobotState {
            dead_for: Some(5.0),
            ..standing()
        },
        0.0,
        &PartStates::default(),
    );
    let up = dead.world_from_robot.transform_vector3(Vec3::Y);
    assert!(up.dot(-Vec3::Z) > 0.99, "{up}");
    // Firing kicks the held weapon (a highlighted placeholder) back and flashes it.
    let firing = pose::pose(
        &model,
        kit,
        &RobotState {
            since_fired: 0.0,
            ..standing()
        },
        0.0,
        &PartStates::default(),
    );
    let rifle = model
        .parts
        .iter()
        .position(|p| p.key == "weapon.energy-rifle")
        .unwrap();
    assert!(firing.parts[rifle].held && firing.parts[rifle].flash);
    let kick =
        firing.parts[rifle].world_from_local.w_axis - idle.parts[rifle].world_from_local.w_axis;
    assert!(kick.z < -0.01, "{kick}");
}

#[test]
fn a_detached_arm_falls_away_with_its_blade_while_the_legs_keep_walking() {
    let kit = kit();
    let model = Model::of(&STANDARD, kit).unwrap();
    // The authoritative state (per-part damage is shooter#56; a fixture here) puts the right
    // arm's subtree on the floor beside the robot.
    let arm = PartId(4);
    let blade = PartId(8);
    let fallen = Mat4::from_translation(Vec3::new(3.0, 0.5, 3.0))
        * Mat4::from_rotation_z(std::f32::consts::FRAC_PI_2);
    let detached = [(arm, fallen)];
    let running = RobotState {
        velocity: Vec3::new(0.0, 0.0, 6.0),
        ..standing()
    };
    let states = PartStates {
        detached: &detached,
        ..PartStates::default()
    };
    let whole = pose::pose(&model, kit, &running, 0.3, &PartStates::default());
    let lost = pose::pose(&model, kit, &running, 0.3, &states);
    for (index, part) in model.parts.iter().enumerate() {
        let off = part.id == arm || part.id == blade;
        assert_eq!(lost.parts[index].detached, off, "{}", part.key);
        if off {
            let expected = fallen * part.robot_from_local;
            assert!(
                lost.parts[index]
                    .world_from_local
                    .abs_diff_eq(expected, 1e-5)
            );
        } else {
            assert_eq!(lost.parts[index], whole.parts[index], "{}", part.key);
        }
    }
    // A damaged plate draws its damaged variant (the plate is in the kit, none in a loadout).
    assert!(model.parts.iter().all(|p| p.key != "robot.armor.plate"));
}

#[test]
fn joints_turn_about_the_simulations_anchor_and_carry_their_children() {
    let kit = kit();
    let build = fixtures::paired_turret();
    let model = Model::of(&build, kit).unwrap();
    let layout = Layout::of(&build);
    for joint in [fixtures::ROTOR_PART, fixtures::HINGE_PART] {
        let index = model.parts.iter().position(|p| p.id == joint).unwrap();
        let actuator = layout.actuators.iter().find(|a| a.part == joint).unwrap();
        let hinge = &layout.hinges[actuator.hinges[0]];
        // The simulation's anchor and axis, robot frame (units to metres).
        let anchor = Vec3::from(hinge.anchor) * shooter_client::scene::METRES_PER_UNIT;
        let drawn = model.parts[index]
            .robot_from_local
            .transform_point3(SIM_JOINT_ANCHOR);
        assert!(drawn.distance(anchor) < 1e-4, "{drawn} {anchor}");
    }
    let hinge = model
        .parts
        .iter()
        .position(|p| p.id == fixtures::HINGE_PART)
        .unwrap();
    let barrel = model
        .parts
        .iter()
        .position(|p| p.id == fixtures::BARREL)
        .unwrap();
    let angles = [(fixtures::HINGE_PART, 0.6)];
    let states = PartStates {
        joints: &angles,
        ..PartStates::default()
    };
    let rest = pose::pose(&model, kit, &standing(), 0.0, &PartStates::default());
    let turned = pose::pose(&model, kit, &standing(), 0.0, &states);
    // The hinge's moving plate and the barrel on it turn together: the plate's top normal stays
    // the barrel's mount normal, and the point where they meet stays put.
    let rig = kit.entries[kit.entry("robot.joint.hinge", "default").unwrap()]
        .rig
        .as_ref()
        .unwrap();
    let moving = rig.joint("hinge").unwrap();
    let plate = turned.parts[hinge].world_from_local
        * turned.parts[hinge].palette.as_ref().unwrap()[moving];
    let plate_up = plate.transform_vector3(Vec3::Y).normalize();
    let barrel_up = turned.parts[barrel]
        .world_from_local
        .transform_vector3(Vec3::Z)
        .normalize();
    assert!(plate_up.dot(barrel_up) > 0.999, "{plate_up} {barrel_up}");
    let meet = |posed: &pose::Posed| {
        posed.parts[hinge]
            .world_from_local
            .transform_point3(SIM_JOINT_ANCHOR)
    };
    assert!(meet(&rest).distance(meet(&turned)) < 1e-4);
    let rest_up = rest.parts[barrel]
        .world_from_local
        .transform_vector3(Vec3::Z)
        .normalize();
    assert!((rest_up.angle_between(barrel_up) - 0.6).abs() < 1e-3);
}
