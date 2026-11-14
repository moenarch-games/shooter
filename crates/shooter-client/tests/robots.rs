//! The robot and weapons kits as the clients draw them, against the pinned asset-tooling
//! releases in `target/robot-kit` and `target/weapon-kit` (or `$SHOOTER_ROBOT_KIT`,
//! `$SHOOTER_WEAPON_KIT`; fetch them with `scripts/fetch-robot-kit.sh`): they decode within their
//! bounds, every loadout maps onto kit entries (wheels and tracks onto labelled placeholders), the
//! parts sit inside the cuboids the simulation places, mods sit on their weapons' anchors, and
//! the poses, weapon states, muzzles, lost parts and joint pivots follow the authoritative state.

#![cfg(not(target_arch = "wasm32"))]

use render_engine::glam::{Mat4, Vec3};
use shooter_client::robots::kit::{Bundle, KitData, MAX_TOTAL_BYTES, Plan, RETIRED_KIT_PARTS};
use shooter_client::robots::load;
use shooter_client::robots::model::{Model, Visual};
use shooter_client::robots::pose::{
    self, Action, Activity, Animation, HALF_HEIGHT, PartStates, Posed, RobotState,
    SIM_JOINT_ANCHOR, SOLE,
};
use shooter_client::robots::weapon::{self, Memory};
use shooter_sim::mechanics::{Layout, fixtures, placements};
use shooter_sim::robot::{HEAVY, LOADOUTS, MARKSMAN, PartId, SKIRMISHER, STANDARD};
use shooter_sim::weapons::Weapon;
use std::sync::OnceLock;

fn kit() -> &'static KitData {
    static KIT: OnceLock<KitData> = OnceLock::new();
    KIT.get_or_init(|| {
        load::read().unwrap_or_else(|error| {
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
        hook: None,
        since_fired: f32::INFINITY,
        fired: Weapon::EnergyRifle,
        shots: 0,
        held: Weapon::EnergyRifle,
        energy: 1.0,
        swing: None,
        dead_for: None,
    }
}

/// `model` posed for `state` with its weapons' transitions long done.
fn posed(model: &Model, state: &RobotState, phase: f32, parts: &PartStates) -> Posed {
    pose::pose(model, kit(), state, phase, parts, &Memory::settled(state))
}

/// Both pinned manifests' plans and the files they read.
fn plans_and_files() -> (Vec<Plan>, load::Files) {
    let mut plans = Vec::new();
    let mut files = load::Files::new();
    for bundle in Bundle::ALL {
        let directory = load::directory(bundle);
        let plan = Plan::of(
            bundle,
            &std::fs::read(directory.join("manifest.json")).unwrap(),
        )
        .expect("the pinned manifest");
        for file in plan.files() {
            files.insert(
                file.path.clone(),
                std::fs::read(directory.join(&file.path)).unwrap(),
            );
        }
        plans.push(plan);
    }
    (plans, files)
}

#[test]
fn the_pinned_kits_decode_within_their_bounds() {
    let (plans, files) = plans_and_files();
    let bytes: usize = files.values().map(Vec::len).sum();
    assert!(bytes < MAX_TOTAL_BYTES, "{bytes}");
    let kit = kit();
    // Eleven robot parts at full detail and the plate's damaged look, eight weapons and four
    // mods; a material and four textures for each kit.
    assert_eq!(plans[0].entries.len(), 12);
    assert_eq!(plans[1].entries.len(), 12);
    assert_eq!(kit.entries.len(), 24);
    // The pinned robot kit still ships the retired control core (shooter#89): the load skips it
    // by name. Once a kit release drops it, this fails, and `RETIRED_KIT_PARTS` empties.
    let manifest =
        std::fs::read_to_string(load::directory(Bundle::Robot).join("manifest.json")).unwrap();
    for (key, _) in RETIRED_KIT_PARTS {
        assert!(manifest.contains(&format!("\"{key}\"")), "{key}");
        assert!(kit.entry(key, "default").is_none(), "{key}");
    }
    assert_eq!(kit.materials.len(), 2);
    assert_eq!(kit.textures.len(), 8);
    assert!(kit.textures[..4].iter().all(|t| t.width == 1024));
    assert!(kit.textures[4..].iter().all(|t| t.width == 512));
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
    let (plans, mut files) = plans_and_files();
    let decode = shooter_client::robots::kit::decode;
    for plan in &plans {
        let first = plan.entries[0].scene.path.clone();
        let original = files[&first].clone();
        files.get_mut(&first).unwrap().push(b' ');
        let error = decode(&plans, &files).unwrap_err();
        assert!(error.contains(&first), "{error}");
        files.remove(&first);
        let error = decode(&plans, &files).unwrap_err();
        assert!(error.contains("missing"), "{error}");
        files.insert(first, original);
    }
    // The weapons kit's descriptor is pinned like any other file.
    let descriptor = plans[1].descriptor.clone().unwrap().path;
    files.get_mut(&descriptor).unwrap()[0] ^= 1;
    let error = decode(&plans, &files).unwrap_err();
    assert!(error.contains(&descriptor), "{error}");
    // Without the weapons kit, the catalog's weapons and mods are unmapped.
    let error = decode(&plans[..1], &files).unwrap_err();
    assert!(error.contains("weapon."), "{error}");
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
            assert!(kit_part, "{name}: {}", drawn.key);
        }
        assert!(model.placeholders().is_empty(), "{name}");
        assert_eq!(model.gait, Some("walk-biped"), "{name}");
    }
    let keys = |model: &Model| -> Vec<&str> { model.parts.iter().map(|p| p.key).collect() };
    let standard = Model::of(&STANDARD, kit).unwrap();
    let heavy = Model::of(&HEAVY, kit).unwrap();
    assert_ne!(keys(&standard), keys(&heavy));
    assert!(keys(&standard).contains(&"robot.chassis.20x40x80"));
    assert!(keys(&heavy).contains(&"robot.chassis.40x40x80"));
    assert!(keys(&heavy).contains(&"robot.battery"));
    // The mods sit on the weapons they are in the build on.
    let mods = |build| -> Vec<String> {
        let model = Model::of(build, kit).unwrap();
        model
            .mods
            .iter()
            .map(|m| {
                format!(
                    "{} on {} at {}",
                    m.key, model.parts[m.part].key, m.anchor.name
                )
            })
            .collect()
    };
    assert!(mods(&STANDARD).is_empty());
    assert_eq!(
        mods(&SKIRMISHER),
        [
            "mod.rapid-cycler on weapon.energy-rifle at mod-side",
            "mod.efficiency-coil on weapon.shotgun at mod-side",
        ]
    );
    assert_eq!(
        mods(&MARKSMAN),
        [
            "mod.long-barrel on weapon.energy-rifle at mod-muzzle",
            "mod.overcharger on weapon.rail-gun at mod-side",
        ]
    );
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
            // And fills it: no part is drawn turned onto another axis (a weapon reaches along its
            // barrel, the part's +z; a heavy mod's second cell stays empty).
            let fills = if drawn.key.starts_with("weapon.") {
                let barrel = drawn.robot_from_local.transform_vector3(Vec3::Y).abs();
                (hi - lo).dot(barrel) >= (cuboid_hi - cuboid_lo).dot(barrel) * 0.6
            } else {
                (hi - lo).cmpge((cuboid_hi - cuboid_lo) * 0.6).all()
            };
            assert!(
                fills,
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
    let posed = posed(&model, &state, 0.0, &PartStates::default());
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
    let rest = posed(&model, &standing(), 0.0, &PartStates::default());
    let running_pose = posed(&model, &running, 0.3, &PartStates::default());
    assert_eq!(running_pose.animation.activity, Activity::Run);
    assert!(running_pose.animation.missing().is_none());
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
    let (a, b) = (sole(&running_pose, legs[0]), sole(&running_pose, legs[1]));
    assert!(sole(&rest, legs[0]).distance(a) > 0.05);
    let spread = (0..10).map(|step| {
        #[allow(clippy::cast_precision_loss)]
        let at = posed(&model, &running, step as f32 / 10.0, &PartStates::default());
        (sole(&at, legs[0]).z - sole(&at, legs[1]).z).abs()
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
                hook: Some(Vec3::new(0.0, 8.0, 9.0)),
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
            "idle + blade (kit swing)",
            None,
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
        let posed = posed(&model, &state, 0.0, &PartStates::default());
        assert_eq!(posed.animation.missing(), missing, "{label}");
        assert!(posed.animation.label(&model).starts_with(label), "{label}");
        if let Some(missing) = missing {
            assert!(posed.animation.label(&model).contains(missing));
        }
    }
    // A slide crouches: the body drops while the feet stay down.
    let slide = posed(
        &model,
        &RobotState {
            sliding: true,
            ..standing()
        },
        0.0,
        &PartStates::default(),
    );
    let idle = posed(&model, &standing(), 0.0, &PartStates::default());
    let height = |posed: &pose::Posed| posed.world_from_robot.w_axis.y;
    assert!(height(&slide) < height(&idle) - 0.1);
    // A death topples it onto its back.
    let dead = posed(
        &model,
        &RobotState {
            dead_for: Some(5.0),
            ..standing()
        },
        0.0,
        &PartStates::default(),
    );
    let up = dead.world_from_robot.transform_vector3(Vec3::Y);
    assert!(up.dot(-Vec3::Z) > 0.99, "{up}");
    // Firing plays the rifle's fire clip: its barrel kicks back toward the weapon's mount.
    let firing = posed(
        &model,
        &RobotState {
            since_fired: 0.1,
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
    let muzzle = |posed: &Posed| {
        posed
            .muzzles
            .iter()
            .find(|m| m.weapon == Weapon::EnergyRifle)
            .unwrap()
            .position
    };
    let kick = muzzle(&firing) - muzzle(&idle);
    let barrel = idle.parts[rifle]
        .world_from_local
        .transform_vector3(Vec3::Y);
    assert!(kick.dot(barrel) < -0.01, "{kick}");
}

#[test]
fn a_detached_blade_falls_away_while_the_legs_keep_walking() {
    let kit = kit();
    let model = Model::of(&STANDARD, kit).unwrap();
    // The authoritative state (`shooter_sim::damage`; a fixture here) has lost the blade on the
    // chest, lying on the floor beside the robot.
    let blade = PartId(8);
    let fallen = Mat4::from_translation(Vec3::new(3.0, 0.5, 3.0))
        * Mat4::from_rotation_z(std::f32::consts::FRAC_PI_2);
    let detached = [(blade, fallen)];
    let running = RobotState {
        velocity: Vec3::new(0.0, 0.0, 6.0),
        ..standing()
    };
    let states = PartStates {
        detached: &detached,
        ..PartStates::default()
    };
    let whole = posed(&model, &running, 0.3, &PartStates::default());
    let lost = posed(&model, &running, 0.3, &states);
    for (index, part) in model.parts.iter().enumerate() {
        let off = part.id == blade;
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
    let rest = posed(&model, &standing(), 0.0, &PartStates::default());
    let turned = posed(&model, &standing(), 0.0, &states);
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
    // The barrel is a kit weapon: its part's +z (out of its socket) is its glTF +Y.
    let barrel_up = turned.parts[barrel]
        .world_from_local
        .transform_vector3(Vec3::Y)
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
        .transform_vector3(Vec3::Y)
        .normalize();
    assert!((rest_up.angle_between(barrel_up) - 0.6).abs() < 1e-3);
}

/// The model part of `build` that is `key`.
fn part_of(model: &Model, key: &str) -> usize {
    model.parts.iter().position(|p| p.key == key).unwrap()
}

#[test]
fn every_weapon_has_its_states_channels_and_muzzle_in_the_kit() {
    let kit = kit();
    for weapon in Weapon::ALL {
        let key = shooter_sim::robot::BETA
            .parts
            .iter()
            .find(|p| p.kind == shooter_sim::robot::Kind::Weapon(weapon))
            .unwrap()
            .key;
        let entry = &kit.entries[kit.entry(key, "default").unwrap()];
        let (rig, gear) = (entry.rig.as_ref().unwrap(), entry.gear.as_ref().unwrap());
        assert!(gear.charge.is_some(), "{key}");
        for node in gear.charge.iter().chain(&gear.fire) {
            assert!(rig.joint(node).is_some(), "{key}: {node}");
        }
        for anchor in &gear.anchors {
            let node = anchor.node.as_deref().unwrap();
            assert!(rig.joint(node).is_some(), "{key}: {}", anchor.name);
        }
        // The clips this client plays are the kit's states.
        let clips: &[&str] = match weapon {
            Weapon::EnergyRifle | Weapon::Shotgun | Weapon::Flamethrower => &["fire"],
            Weapon::Laser => &["beam"],
            Weapon::RailGun => &["fire", "charge"],
            Weapon::RocketLauncher => &["fire-a", "fire-b", "reload"],
            Weapon::Grapple => &["launch", "retract"],
            Weapon::EnergyBlade => &["extend", "retract", "swing"],
        };
        for clip in clips {
            assert!(
                gear.states.iter().any(|state| state == clip),
                "{key}: {clip}"
            );
        }
        let muzzle = match weapon {
            Weapon::EnergyBlade => "blade-tip",
            Weapon::RocketLauncher => "muzzle-a",
            _ => "muzzle",
        };
        assert!(gear.anchor(muzzle).is_some(), "{key}: {muzzle}");
    }
}

#[test]
fn shots_start_at_the_muzzles_and_mods_ride_their_weapons() {
    let kit = kit();
    let model = Model::of(&STANDARD, kit).unwrap();
    let idle = posed(&model, &standing(), 0.0, &PartStates::default());
    // The rifle, grapple and shotgun have muzzles (the blade none), each at the far end of its
    // weapon, pointing out of its socket.
    let weapons: Vec<Weapon> = idle.muzzles.iter().map(|m| m.weapon).collect();
    assert_eq!(
        weapons,
        [Weapon::EnergyRifle, Weapon::Grapple, Weapon::Shotgun]
    );
    for muzzle in &idle.muzzles {
        let key = match muzzle.weapon {
            Weapon::EnergyRifle => "weapon.energy-rifle",
            Weapon::Grapple => "weapon.grapple-hook",
            _ => "weapon.shotgun",
        };
        let frame = idle.parts[part_of(&model, key)].world_from_local;
        let out = frame.transform_vector3(Vec3::Y).normalize();
        assert!(muzzle.direction.dot(out) > 0.99, "{key}");
        let along = (muzzle.position - frame.transform_point3(Vec3::ZERO)).dot(out);
        assert!(along > 0.25 && along < 0.41, "{key}: {along}");
    }
    assert_eq!(idle.grapple, Some(idle.muzzles[1].position));
    // The long barrel's muzzle replaces the marksman rifle's, 9 cm further out.
    let marksman = Model::of(&MARKSMAN, kit).unwrap();
    let modded = posed(&marksman, &standing(), 0.0, &PartStates::default());
    let rifle = modded.parts[part_of(&marksman, "weapon.energy-rifle")].world_from_local;
    let muzzle = modded
        .muzzles
        .iter()
        .find(|m| m.weapon == Weapon::EnergyRifle)
        .unwrap();
    let out = rifle.transform_vector3(Vec3::Y).normalize();
    let along = (muzzle.position - rifle.transform_point3(Vec3::ZERO)).dot(out);
    assert!((along - 0.40).abs() < 0.005, "{along}");
    // The mods are drawn on their anchors, and stay on them when a shot kicks the barrel.
    assert_eq!(modded.mods.len(), 2);
    let barrel = modded.mods[0].transform_point3(Vec3::ZERO);
    let firing = posed(
        &marksman,
        &RobotState {
            since_fired: 0.1,
            ..standing()
        },
        0.0,
        &PartStates::default(),
    );
    let kicked = firing.mods[0].transform_point3(Vec3::ZERO);
    assert!((kicked - barrel).dot(out) < -0.01, "{barrel} {kicked}");
    // A lost weapon takes its mod along.
    let fallen = Mat4::from_translation(Vec3::new(5.0, 0.0, 5.0));
    let detached = [(
        marksman.parts[part_of(&marksman, "weapon.energy-rifle")].id,
        fallen,
    )];
    let lost = posed(
        &marksman,
        &standing(),
        0.0,
        &PartStates {
            detached: &detached,
            ..PartStates::default()
        },
    );
    assert!(lost.mods[0].w_axis.x > 4.0, "{}", lost.mods[0].w_axis);
    assert!(lost.muzzles.iter().all(|m| m.weapon != Weapon::EnergyRifle));
}

#[test]
fn every_loadout_weapon_points_its_kit_muzzle_along_the_aim() {
    // Mounted on the chassis' front (shooter#91), every weapon's kit muzzle (and the blade's
    // tip) points within a degree of where the fighter faces, with no aim offset in the drawing.
    let kit = kit();
    let forward = standing().forward;
    let within = 1.0_f32.to_radians().cos();
    for (name, build) in LOADOUTS {
        let model = Model::of(&build, kit).unwrap();
        let idle = posed(&model, &standing(), 0.0, &PartStates::default());
        assert_eq!(idle.muzzles.len(), 3, "{name}: every weapon but the blade");
        for muzzle in &idle.muzzles {
            let along = muzzle.direction.normalize().dot(forward);
            assert!(along > within, "{name}: {:?} {along}", muzzle.weapon);
        }
        let blade = &model.parts[part_of(&model, "weapon.energy-blade")];
        let Visual::Kit { entry, .. } = blade.visual else {
            panic!("{name}: the blade is a kit part")
        };
        let gear = kit.entries[entry]
            .gear
            .as_ref()
            .expect("the blade's descriptor");
        let tip = gear.anchor("blade-tip").expect("the blade's tip");
        let world = idle.parts[part_of(&model, "weapon.energy-blade")].world_from_local;
        let along = world.transform_vector3(tip.normal).normalize().dot(forward);
        assert!(along > within, "{name}: blade {along}");
    }
}

#[test]
fn weapon_states_follow_the_shots_hook_and_blade() {
    let kit = kit();
    let heavy = Model::of(&HEAVY, kit).unwrap();
    let entry = |model: &Model, key| {
        let Visual::Kit { entry, .. } = model.parts[part_of(model, key)].visual else {
            panic!()
        };
        &kit.entries[entry]
    };
    let locals = |model: &Model, key, state: &RobotState, memory: &Memory| {
        let part = &model.parts[part_of(model, key)];
        let entry = entry(model, key);
        let shooter_sim::robot::Kind::Weapon(held) = part.kind else {
            panic!()
        };
        weapon::locals(
            entry.rig.as_ref().unwrap(),
            entry.gear.as_ref().unwrap(),
            held,
            part.refire,
            state,
            memory,
        )
        .0
    };
    let rig = |model: &Model, key| entry(model, key).rig.as_ref().unwrap();
    // The charge glow shows the energy pool.
    let low = RobotState {
        energy: 0.25,
        ..standing()
    };
    let rail = locals(&heavy, "weapon.rail-gun", &low, &Memory::settled(&low));
    let charge = rig(&heavy, "weapon.rail-gun").joint("glow-charge").unwrap();
    assert!((rail[charge].scale.y - 0.25).abs() < 1e-6);
    // The rocket pod's tubes alternate; then both reload over its refire.
    let rocket = |shots, since| {
        let state = RobotState {
            fired: Weapon::RocketLauncher,
            since_fired: since,
            shots,
            ..standing()
        };
        let rig = rig(&heavy, "weapon.rocket-launcher");
        let locals = locals(
            &heavy,
            "weapon.rocket-launcher",
            &state,
            &Memory::settled(&state),
        );
        let (a, b) = (
            rig.joint("rocket-a").unwrap(),
            rig.joint("rocket-b").unwrap(),
        );
        (locals[a].translation.y, locals[b].translation.y)
    };
    let (a, b) = rocket(1, 0.1);
    assert!(a > 0.31 && (b - 0.3).abs() < 1e-4, "{a} {b}");
    let (a, b) = rocket(2, 0.1);
    assert!(b > 0.31 && (a - 0.3).abs() < 1e-4, "{a} {b}");
    let (a, b) = rocket(2, 0.5);
    assert!(a < 0.29 && b < 0.29, "reloading {a} {b}");
    let (a, b) = rocket(2, 5.0);
    assert!((a - 0.3).abs() < 1e-4 && (b - 0.3).abs() < 1e-4, "{a} {b}");
    // The rail gun recharges over its refire (the overcharger's is longer).
    let marksman = Model::of(&MARKSMAN, kit).unwrap();
    let rail_refire = |model: &Model| model.parts[part_of(model, "weapon.rail-gun")].refire;
    assert!((rail_refire(&heavy) - 1.2).abs() < 1e-4);
    assert!(rail_refire(&marksman) > 1.5);
    let recharging = RobotState {
        fired: Weapon::RailGun,
        since_fired: 0.75,
        ..standing()
    };
    let rail = locals(
        &heavy,
        "weapon.rail-gun",
        &recharging,
        &Memory::settled(&recharging),
    );
    assert!(rail[charge].scale.y > 0.3 && rail[charge].scale.y < 0.7);

    // The grapple's hook flies to where the host set it: its cable end reaches the target.
    let standard = Model::of(&STANDARD, kit).unwrap();
    let target = standing().center + Vec3::new(1.0, 3.0, 4.0);
    let hooked = RobotState {
        hook: Some(target),
        ..standing()
    };
    let posed_hooked = posed(&standard, &hooked, 0.0, &PartStates::default());
    let grapple = part_of(&standard, "weapon.grapple-hook");
    let grapple_entry = entry(&standard, "weapon.grapple-hook");
    let hook = grapple_entry.rig.as_ref().unwrap().joint("hook").unwrap();
    let cable = grapple_entry
        .gear
        .as_ref()
        .unwrap()
        .anchor("cable")
        .unwrap();
    let end = (posed_hooked.parts[grapple].world_from_local
        * posed_hooked.parts[grapple].palette.as_ref().unwrap()[hook])
        .transform_point3(cable.position);
    assert!(end.distance(target) < 1e-3, "{end} {target}");
    // Just set, it is still on its way; released, it flies back.
    let mut memory = Memory::settled(&standing());
    memory.advance(&hooked, 0.05);
    let flying = pose::pose(
        &standard,
        kit,
        &hooked,
        0.0,
        &PartStates::default(),
        &memory,
    );
    let end = (flying.parts[grapple].world_from_local
        * flying.parts[grapple].palette.as_ref().unwrap()[hook])
        .transform_point3(cable.position);
    assert!(end.distance(target) > 1.0);
    memory.advance(&standing(), 0.1);
    assert!(memory.hook_for < 0.2 && memory.last_hook == Some(target));

    // The blade is away unless held or swinging, and a swing turns it about its wrist.
    let blade = rig(&standard, "weapon.energy-blade");
    let (edge, wrist) = (blade.joint("blade").unwrap(), blade.joint("wrist").unwrap());
    let away = locals(
        &standard,
        "weapon.energy-blade",
        &standing(),
        &Memory::settled(&standing()),
    );
    assert!(away[edge].scale.y < 0.1);
    let swinging = RobotState {
        swing: Some(0.5),
        ..standing()
    };
    let swung = locals(
        &standard,
        "weapon.energy-blade",
        &swinging,
        &Memory::settled(&swinging),
    );
    assert!((swung[edge].scale.y - 1.0).abs() < 1e-4);
    assert!(swung[wrist].rotation.angle_between(away[wrist].rotation) > 0.5);
}

#[test]
fn the_browser_reads_each_kit_from_its_pinned_tar() {
    // What the browser does with `robot-kit/kit.tar` and `weapon-kit/kit.tar`, natively.
    let mut total = 0;
    let mut files = load::Files::new();
    let mut plans = Vec::new();
    for bundle in Bundle::ALL {
        let directory = load::directory(bundle);
        let tar = std::fs::read(directory.join("kit.tar")).expect("fetched with its tar");
        assert!(tar.len() < load::MAX_TAR_BYTES);
        assert_eq!(
            shooter_client::robots::kit::sha256(&tar),
            shooter_client::robots::kit::pinned_tar(bundle)
        );
        let mut entries = load::untar(&tar).unwrap();
        let plan = Plan::of(bundle, &entries.remove("manifest.json").unwrap()).unwrap();
        load::take(&plan, &mut entries, &mut total, &mut files).unwrap();
        plans.push(plan);
    }
    let (_, on_disk) = plans_and_files();
    assert_eq!(files, on_disk);
    let kit = load::finish(&plans, &files).unwrap();
    assert_eq!(kit.entries.len(), 24);
    // Anything but a plain archive of regular files is refused.
    assert!(load::untar(&[7; 1024]).is_err());
    let mut tar = std::fs::read(load::directory(Bundle::Weapons).join("kit.tar")).unwrap();
    tar[156] = b'2';
    assert!(load::untar(&tar).unwrap_err().contains("regular file"));
}
