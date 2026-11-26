//! The robot, weapons and locomotion kits as the clients draw them, against the pinned
//! asset-tooling releases in `target/robot-kit`, `target/weapon-kit` and `target/locomotion-kit`
//! (or `$SHOOTER_ROBOT_KIT`, `$SHOOTER_WEAPON_KIT`, `$SHOOTER_LOCOMOTION_KIT`; fetch them with
//! `scripts/fetch-robot-kit.sh`): they decode within their bounds, every loadout and mechanics
//! fixture maps onto kit entries, the parts sit inside the cuboids the simulation places, mods sit
//! on their weapons' anchors, wheels and tracks on the simulation's axles rolling as it turns
//! them, every part on the body the simulation's mechanics have it on (legs striding, joints
//! turning, a legless robot lying on its chassis), and the weapon states, muzzles and lost parts
//! follow the authoritative state.

#![cfg(not(target_arch = "wasm32"))]

use render_engine::glam::{Mat4, Vec3};
use shooter_client::robots::Frames;
use shooter_client::robots::kit::{Bundle, KitData, MAX_TOTAL_BYTES, Plan, drawn_key};
use shooter_client::robots::load;
use shooter_client::robots::model::Model;
use shooter_client::robots::pose::{
    self, Action, Activity, Animation, HALF_HEIGHT, PartStates, Posed, RobotState, SOLE,
};
use shooter_client::robots::weapon::{self, Memory};
use shooter_sim::Assembly;
use shooter_sim::mechanics::{Controls, Layout, Mechanics, fixtures, placements};
use shooter_sim::robot::{HEAVY, LOADOUTS, MARKSMAN, PartId, SKIRMISHER, STANDARD};
use shooter_sim::weapons::{Instance, Weapon};
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
        fired: Instance::new(7, Weapon::EnergyRifle),
        tube: 0,
        recent: shooter_client::robots::pose::Recent::NONE,
        held: Instance::new(7, Weapon::EnergyRifle),
        energy: 1.0,
        swing: None,
        dead_for: None,
    }
}

/// `model` posed for `state` with its weapons' transitions long done.
fn posed(model: &Model, state: &RobotState, parts: &PartStates) -> Posed {
    pose::pose(model, kit(), state, parts, &Memory::settled(state))
}

/// `build` alone on the fixtures' floor, stepped `ticks` under `controls` (given its layout);
/// with any parts in `lost` (bit per part index) severed first.
fn mechanics(
    build: Assembly,
    lost: u32,
    ticks: u32,
    controls: impl Fn(&Layout) -> Controls,
) -> Mechanics {
    let mut mechanics = Mechanics::new(&fixtures::floor()).unwrap();
    mechanics.spawn(build, [0.0, 0.0, 0.0], 0).unwrap();
    if lost != 0 {
        mechanics.sever(0, lost).unwrap();
    }
    for _ in 0..ticks {
        let controls = controls(&mechanics.robots()[0].layout);
        mechanics.step(&[controls]).unwrap();
    }
    mechanics
}

/// The frames of the robot in `mechanics`, drawn around the origin.
fn frames_of(mechanics: &Mechanics) -> Frames {
    Frames::of_robot(mechanics, 0, Vec3::ZERO).unwrap()
}

/// Every connected rigid part (and every joint's box, every weapon) is drawn on the body the
/// simulation has it on: its kit mesh where `part_frames` puts its cuboid.
fn assert_on_the_simulations_bodies(model: &Model, posed: &Posed, mechanics: &Mechanics) {
    use shooter_sim::robot::Kind;
    let frames = mechanics.part_frames(0).unwrap();
    for (index, part) in model.parts.iter().enumerate() {
        if matches!(part.kind, Kind::Locomotion(_)) || posed.parts[index].detached {
            continue;
        }
        let Some((_, pose)) = frames.iter().find(|(id, _)| *id == part.id) else {
            panic!("{}: not connected in the simulation", part.key);
        };
        let body = Mat4::from_rotation_translation(
            render_engine::glam::Quat::from_array(pose.orientation),
            Vec3::from(pose.position) * 0.01,
        );
        let expected = body * part.robot_from_local;
        assert!(
            posed.parts[index]
                .world_from_local
                .abs_diff_eq(expected, 1e-3),
            "{}: {} vs {expected}",
            part.key,
            posed.parts[index].world_from_local
        );
    }
}

/// The part-index bits of `build`'s legs.
fn legs_of(build: &Assembly) -> Vec<u32> {
    build
        .parts()
        .iter()
        .enumerate()
        .filter(|(_, part)| {
            shooter_sim::robot::BETA
                .part_type(part.kind)
                .map(|t| t.kind)
                == Some(shooter_sim::robot::Kind::Locomotion(
                    shooter_sim::robot::Locomotion::Leg,
                ))
        })
        .map(|(index, _)| 1 << index)
        .collect()
}

/// Both pinned manifests' plans and the files they read.
fn plans_and_files() -> (Vec<Plan>, load::Files) {
    let mut plans = Vec::new();
    let mut files = load::Files::new();
    for bundle in Bundle::FIGHTERS {
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
    // mods, the wheel and the track and their damaged looks; a material and four textures for
    // each kit.
    assert_eq!(plans[0].entries.len(), 12);
    assert_eq!(plans[1].entries.len(), 12);
    assert_eq!(plans[2].entries.len(), 4);
    assert_eq!(kit.entries.len(), 28);
    // The pinned robot kit no longer ships the retired control core (shooter#89).
    let manifest =
        std::fs::read_to_string(load::directory(Bundle::Robot).join("manifest.json")).unwrap();
    assert!(!manifest.contains("\"robot.core\""));
    assert_eq!(kit.materials.len(), 3);
    assert_eq!(kit.textures.len(), 12);
    // The robot kit's four textures (one republished at half size), then the others' at 512.
    let widths: Vec<u32> = kit.textures.iter().map(|t| t.width).collect();
    assert_eq!(widths[..4], [1024, 512, 1024, 1024]);
    assert!(widths[4..].iter().all(|width| *width == 512));
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
    // Without the weapons kit, the catalog's weapons and mods are unmapped; without the
    // locomotion kit, the wheel and the track.
    let error = decode(&[plans[0].clone(), plans[2].clone()], &files).unwrap_err();
    assert!(error.contains("weapon."), "{error}");
    let error = decode(&plans[..2], &files).unwrap_err();
    assert!(error.contains("robot.wheel"), "{error}");
}

#[test]
fn loadouts_and_fixtures_draw_every_part_from_the_kits() {
    let kit = kit();
    for (name, build) in LOADOUTS {
        let model = Model::of(&build, kit).expect(name);
        assert_eq!(model.parts.len(), build.parts().len(), "{name}");
        for (part, drawn) in build.parts().iter().zip(&model.parts) {
            assert_eq!(part.id, drawn.id);
            let kit_part = kit.entries[drawn.entry].key == drawn_key(drawn.key);
            assert!(kit_part, "{name}: {}", drawn.key);
        }
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

/// A tracked build for review and tests (not a simulation fixture: where tracks should sit is
/// shooter#58): the large chassis on two tracks along its lower sides, the belts' bottoms flush
/// with the hull's, a rotor on the roof carrying the rocket pod pitched to aim ahead.
fn mini_tank() -> Assembly {
    use shooter_sim::robot::parts::{CHASSIS_L, ROCKET_LAUNCHER, ROTOR, TRACK};
    use shooter_sim::robot::{BETA, Candidate, Face, Orientation, PartType, RawMount, RawPart};
    let on =
        |id: u8, kind: &PartType, parent: u8, to: &PartType, face: Face, yaw: u16, pitch: u16| {
            RawPart {
                id,
                kind: kind.id.0,
                mount: Some(RawMount {
                    parent,
                    socket: to.socket(face, 0, 0).unwrap().0,
                    yaw,
                    pitch,
                    roll: 0,
                }),
                modification: None,
            }
        };
    let quarter = Orientation::QUARTER;
    let parts = vec![
        RawPart {
            id: 0,
            kind: CHASSIS_L.id.0,
            mount: None,
            modification: None,
        },
        on(1, &TRACK, 0, &CHASSIS_L, Face::PosY, 0, 0),
        on(2, &TRACK, 0, &CHASSIS_L, Face::NegY, 3 * quarter, 0),
        on(3, &ROTOR, 0, &CHASSIS_L, Face::PosZ, 0, 0),
        on(4, &ROCKET_LAUNCHER, 3, &ROTOR, Face::PosZ, 0, 3 * quarter),
    ];
    BETA.validate(&Candidate { parts })
        .expect("the mini-tank is a valid build")
}

/// The named loadouts, the mechanics fixtures, the mini-tank and the mount-rail fixtures.
fn builds() -> Vec<(&'static str, Assembly)> {
    let mut builds = LOADOUTS.to_vec();
    builds.extend([
        ("car", fixtures::car()),
        ("strafer", fixtures::strafer()),
        ("turret", fixtures::turret()),
        ("paired-turret", fixtures::paired_turret()),
        ("lopsided", fixtures::lopsided()),
        ("balanced", fixtures::balanced()),
        ("biped", fixtures::biped()),
        ("dog", fixtures::dog(false)),
        ("upside-down-legs", fixtures::upside_down_legs()),
        ("mini-tank", mini_tank()),
        ("rail-rack", shooter_sim::robot::RAIL_RACK),
        ("cross-rail", shooter_sim::robot::CROSS_RAIL),
        ("adapter-mast", shooter_sim::robot::ADAPTER_MAST),
    ]);
    builds
}

#[test]
fn every_kit_part_sits_inside_the_cuboid_the_simulation_places() {
    // The meshes are uploaded (and dropped) by the GPU path only; this kit keeps them.
    let kit = kit();
    for (name, build) in builds() {
        let model = Model::of(&build, kit).unwrap();
        for (drawn, placement) in model.parts.iter().zip(placements(&build)) {
            let entry = drawn.entry;
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
        let entry = drawn.entry;
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
    let posed = posed(&model, &state, &PartStates::default());
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
fn walking_strides_the_legs_where_the_simulation_has_them() {
    let kit = kit();
    let model = Model::of(&STANDARD, kit).unwrap();
    let legs: Vec<usize> = (0..model.parts.len())
        .filter(|&i| model.parts[i].key == "robot.leg")
        .collect();
    assert_eq!(legs.len(), 2);
    let sole = |posed: &Posed, leg: usize| {
        let rig = kit.entries[kit.entry("robot.leg", "default").unwrap()]
            .rig
            .as_ref()
            .unwrap();
        let palette = posed.parts[leg].palette.as_ref().unwrap();
        (posed.parts[leg].world_from_local * palette[rig.joint("ankle").unwrap()])
            .transform_point3(SOLE)
    };
    let mut spread = 0.0f32;
    let mut stepped = false;
    for ticks in [10, 20, 30, 40, 50] {
        let walking = mechanics(STANDARD, 0, ticks, |layout| {
            Controls::IDLE.walk(layout, 100, 0)
        });
        let frames = frames_of(&walking);
        let posed = posed(
            &model,
            &standing(),
            &PartStates {
                frames: Some(&frames),
                ..PartStates::default()
            },
        );
        // The robot is where its root body is, every rigid part on its body.
        assert!(posed.world_from_robot.abs_diff_eq(frames.root(), 1e-6));
        assert_on_the_simulations_bodies(&model, &posed, &walking);
        // Each leg's hip turns as the simulation's hip hinge has turned.
        for &leg in &legs {
            let part = &model.parts[leg];
            let [hip, knee] = pose::leg_angles(part, &frames).unwrap();
            let actuator = walking.robots()[0]
                .actuators
                .iter()
                .find(|actuator| actuator.part == part.id)
                .unwrap();
            assert!(
                (hip - actuator.angle).abs() < 1e-3,
                "{ticks}: hip {hip} vs the simulation's {}",
                actuator.angle
            );
            assert!((-0.05..=0.85).contains(&knee), "{ticks}: knee {knee}");
            stepped |= hip.abs() > 0.05;
        }
        // The soles stay near the floor.
        let (a, b) = (sole(&posed, legs[0]), sole(&posed, legs[1]));
        assert!(a.y.min(b.y).abs() < 0.12, "{ticks}: {a} {b}");
        spread = spread.max(a.distance(b));
    }
    assert!(stepped, "the gait swings the hips");
    // Without the simulation's frames (the builder's bench) it stands at rest.
    let rest = posed(&model, &standing(), &PartStates::default());
    let (a, b) = (sole(&rest, legs[0]), sole(&rest, legs[1]));
    assert!(spread > a.distance(b) + 0.02, "{spread}");
}

#[test]
fn a_robot_without_legs_lies_on_its_chassis_where_the_simulation_drops_it() {
    let kit = kit();
    let model = Model::of(&STANDARD, kit).unwrap();
    let legs = legs_of(&STANDARD);
    assert_eq!(legs.len(), 2);
    let standing_up = mechanics(STANDARD, 0, 90, |layout| Controls::IDLE.walk(layout, 0, 0));
    let legless = mechanics(STANDARD, legs[0] | legs[1], 90, |_| Controls::IDLE);
    let one_legged = mechanics(STANDARD, legs[0], 90, |layout| {
        Controls::IDLE.walk(layout, 100, 0)
    });
    let height = |mechanics: &Mechanics| frames_of(mechanics).root().w_axis.y;
    assert!(
        height(&legless) < height(&standing_up) - 0.3,
        "{} {}",
        height(&legless),
        height(&standing_up)
    );
    for (name, mechanics, lost) in [
        ("legless", &legless, legs[0] | legs[1]),
        ("one-legged", &one_legged, legs[0]),
    ] {
        let frames = frames_of(mechanics);
        // The lost legs are not on the robot: the match lays them where they fell.
        let detached: Vec<(PartId, Mat4)> = STANDARD
            .parts()
            .iter()
            .enumerate()
            .filter(|(index, _)| lost & (1 << index) != 0)
            .map(|(_, part)| (part.id, Mat4::from_translation(Vec3::new(3.0, 0.0, 0.0))))
            .collect();
        let posed = posed(
            &model,
            &standing(),
            &PartStates {
                frames: Some(&frames),
                detached: &detached,
                ..PartStates::default()
            },
        );
        assert!(
            posed.world_from_robot.abs_diff_eq(frames.root(), 1e-6),
            "{name}"
        );
        assert_on_the_simulations_bodies(&model, &posed, mechanics);
        for (index, part) in model.parts.iter().enumerate() {
            let lost_here = detached.iter().any(|(id, _)| *id == part.id);
            assert_eq!(
                posed.parts[index].detached, lost_here,
                "{name}: {}",
                part.key
            );
        }
    }
}

#[test]
fn frames_blend_between_snapshots_and_move_with_the_drawn_centre() {
    let early = mechanics(STANDARD, 0, 20, |layout| {
        Controls::IDLE.walk(layout, 100, 0)
    });
    let late = mechanics(STANDARD, 0, 30, |layout| {
        Controls::IDLE.walk(layout, 100, 0)
    });
    let (a, b) = (frames_of(&early), frames_of(&late));
    let start = a.blend(&b, 0.0);
    for ((at, turn), (from, expected)) in start.bodies.iter().zip(&a.bodies) {
        assert!(at.abs_diff_eq(*from, 1e-6) && turn.abs_diff_eq(*expected, 1e-6));
    }
    let end = a.blend(&b, 1.0);
    for ((at, turn), (to, expected)) in end.bodies.iter().zip(&b.bodies) {
        assert!(at.abs_diff_eq(*to, 1e-5) && turn.abs_diff_eq(*expected, 1e-5));
    }
    let middle = a.blend(&b, 0.5);
    assert!(
        middle
            .root()
            .w_axis
            .distance((a.root().w_axis + b.root().w_axis) / 2.0)
            < 1e-4
    );
    // A robot that lost a part between the two snapshots is drawn as the nearer one.
    let one_legged = mechanics(STANDARD, legs_of(&STANDARD)[0], 30, |_| Controls::IDLE);
    let c = frames_of(&one_legged);
    assert_eq!(a.blend(&c, 0.4), a);
    assert_eq!(a.blend(&c, 0.6), c);
    // Re-anchored on a drawn centre, the whole robot moves with it.
    let moved = a.at(Vec3::new(1.0, 2.0, 3.0));
    assert!(
        moved
            .root()
            .w_axis
            .truncate()
            .abs_diff_eq(a.root().w_axis.truncate() + Vec3::new(1.0, 2.0, 3.0), 1e-5)
    );
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
            "strafe",
            Some("strafe"),
        ),
        (
            RobotState {
                velocity: Vec3::new(0.0, 4.0, 0.0),
                ..standing()
            },
            "jump/fall",
            None,
        ),
        (
            RobotState {
                hook: Some(Vec3::new(0.0, 8.0, 9.0)),
                ..standing()
            },
            "grapple",
            None,
        ),
        (
            RobotState {
                sliding: true,
                ..standing()
            },
            "slide",
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
        let posed = posed(&model, &state, &PartStates::default());
        assert_eq!(posed.animation.missing(), missing, "{label}");
        assert!(posed.animation.label().starts_with(label), "{label}");
        if let Some(missing) = missing {
            assert!(posed.animation.label().contains(missing));
        }
    }
    let idle = posed(&model, &standing(), &PartStates::default());
    // A death topples it onto its back.
    let dead = posed(
        &model,
        &RobotState {
            dead_for: Some(5.0),
            ..standing()
        },
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
    let walking = mechanics(STANDARD, 0, 30, |layout| {
        Controls::IDLE.walk(layout, 100, 0)
    });
    let frames = frames_of(&walking);
    let states = PartStates {
        frames: Some(&frames),
        detached: &detached,
        ..PartStates::default()
    };
    let whole = posed(
        &model,
        &standing(),
        &PartStates {
            frames: Some(&frames),
            ..PartStates::default()
        },
    );
    let lost = posed(&model, &standing(), &states);
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
fn joints_turn_as_the_simulation_turns_them_and_carry_their_children() {
    use shooter_sim::mechanics::Command;
    let kit = kit();
    let build = fixtures::paired_turret();
    let model = Model::of(&build, kit).unwrap();
    let index = |id| model.parts.iter().position(|p| p.id == id).unwrap();
    let (hinge, barrel) = (index(fixtures::HINGE_PART), index(fixtures::BARREL));
    // The hinge driven to about 0.6 rad (binary angle), the rotor to about −0.4.
    let turned = mechanics(build, 0, 90, |_| {
        Controls::IDLE
            .with(fixtures::HINGE_PART, Command::Angle(6_258))
            .with(fixtures::ROTOR_PART, Command::Angle(-4_172))
    });
    let at_rest = mechanics(build, 0, 0, |_| Controls::IDLE);
    let pose_of = |mechanics: &Mechanics, frames: &Frames| {
        let posed = posed(
            &model,
            &standing(),
            &PartStates {
                frames: Some(frames),
                ..PartStates::default()
            },
        );
        assert_on_the_simulations_bodies(&model, &posed, mechanics);
        posed
    };
    let (rest_frames, turned_frames) = (frames_of(&at_rest), frames_of(&turned));
    let rest = pose_of(&at_rest, &rest_frames);
    let posed = pose_of(&turned, &turned_frames);
    let angle = turned.robots()[0]
        .actuators
        .iter()
        .find(|actuator| actuator.part == fixtures::HINGE_PART)
        .unwrap()
        .angle;
    assert!(angle.abs() > 0.3, "the hinge turned ({angle})");
    // The hinge's moving plate and the barrel on it turn together: the plate's top normal stays
    // the barrel's mount normal, turned from rest by the simulation's hinge angle.
    let rig = kit.entries[kit.entry("robot.joint.hinge", "default").unwrap()]
        .rig
        .as_ref()
        .unwrap();
    let moving = rig.joint("hinge").unwrap();
    let up = |posed: &Posed| {
        let plate = posed.parts[hinge].world_from_local
            * posed.parts[hinge].palette.as_ref().unwrap()[moving];
        let plate_up = plate.transform_vector3(Vec3::Y).normalize();
        // The barrel is a kit weapon: its part's +z (out of its socket) is its glTF +Y.
        let barrel_up = posed.parts[barrel]
            .world_from_local
            .transform_vector3(Vec3::Y)
            .normalize();
        assert!(plate_up.dot(barrel_up) > 0.999, "{plate_up} {barrel_up}");
        (plate_up, posed.parts[hinge].world_from_local)
    };
    let (rest_up, rest_box) = up(&rest);
    let (turned_up, turned_box) = up(&posed);
    // The plate's turn against the hinge's own box is the simulation's angle.
    let relative = |box_frame: Mat4, up: Vec3| box_frame.inverse().transform_vector3(up);
    let between = relative(rest_box, rest_up).angle_between(relative(turned_box, turned_up));
    assert!((between - angle.abs()).abs() < 2e-3, "{between} vs {angle}");
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

/// The simulation's muzzle table ([`shooter_sim::aim::MUZZLE_ANCHORS_MM`]) is the pinned weapons
/// kit's: each weapon's muzzle anchor (the rocket pod's first tube), and the long barrel's
/// muzzle on the weapon's `mod-muzzle` anchor.
#[test]
fn the_simulations_muzzles_are_the_kits() {
    let kit = kit();
    let gear = |key: &str| {
        kit.entries[kit.entry(key, "default").unwrap()]
            .gear
            .clone()
            .unwrap()
    };
    let mm = |at: Vec3| (at * 1_000.0).round().to_array().map(|axis| axis as i32);
    let barrel = gear("mod.long-barrel");
    let barrel_muzzle = barrel.anchor("muzzle").unwrap();
    for (weapon, modification, table) in shooter_sim::aim::MUZZLE_ANCHORS_MM {
        let key = shooter_sim::robot::BETA
            .parts
            .iter()
            .find(|p| p.kind == shooter_sim::robot::Kind::Weapon(weapon))
            .unwrap()
            .key;
        let weapon_gear = gear(key);
        let at = match modification {
            None => {
                let name = if weapon == Weapon::RocketLauncher {
                    "muzzle-a"
                } else {
                    "muzzle"
                };
                weapon_gear.anchor(name).unwrap().position
            }
            Some(modification) => {
                assert_eq!(modification, shooter_sim::robot::LONG_BARREL);
                let mount = weapon_gear.anchor("mod-muzzle").unwrap();
                mount.frame().transform_point3(barrel_muzzle.position)
            }
        };
        assert_eq!(mm(at), table, "{key} {modification:?}");
    }
    // The rocket pod's second tube (shooter#102).
    let pod = gear("weapon.rocket-launcher");
    assert_eq!(
        mm(pod.anchor("muzzle-b").unwrap().position),
        shooter_sim::aim::SECOND_TUBE_MM
    );
    // Every weapon but the blade has one, every long-barrel weapon its own.
    for weapon in Weapon::ALL {
        assert_eq!(
            shooter_sim::aim::muzzle_anchor_mm(weapon, None).is_none(),
            weapon == Weapon::EnergyBlade
        );
    }
}

/// Where the simulation fires from is where the client draws the muzzle at rest: every
/// weapon of the armory, the marksman's long barrel and every copy of the twin rail's rail gun
/// (shooter#125), and every weapon on a mount rail or an adapter (shooter#126); a rocket
/// launcher's two tubes each where the simulation fires that tube from (shooter#102), every
/// launcher of the rocket pod at its own.
#[test]
fn the_simulations_muzzles_are_where_the_robot_draws_them() {
    use shooter_sim::damage::{Hitboxes, PartDamage};
    use shooter_sim::physics::articulated::Pose;
    use shooter_sim::robot::{ADAPTER_MAST, ARMORY, CROSS_RAIL, RAIL_RACK, ROCKET_POD, TWIN_RAIL};
    let kit = kit();
    for build in [
        ARMORY,
        MARKSMAN,
        TWIN_RAIL,
        ROCKET_POD,
        RAIL_RACK,
        CROSS_RAIL,
        ADAPTER_MAST,
    ] {
        let model = Model::of(&build, kit).unwrap();
        // The tube each drawn after a shot from it (0 the first).
        let tubes = [0, 1].map(|tube| {
            posed(
                &model,
                &RobotState { tube, ..standing() },
                &PartStates::default(),
            )
        });
        let idle = &tubes[0];
        // The parts in the robot's own frame.
        let frames: Vec<(PartId, Pose)> = build
            .parts()
            .iter()
            .map(|part| (part.id, Pose::IDENTITY))
            .collect();
        let boxes = Hitboxes::posed(&build, &PartDamage::fresh(&build), &frames);
        // A muzzle drawn for every mounted copy that has one (the blade has none).
        let copies = shooter_sim::robot::BETA
            .weapons(&build)
            .filter(|copy| copy.weapon != Weapon::EnergyBlade)
            .count();
        assert_eq!(idle.muzzles.len(), copies);
        for drawn in &idle.muzzles {
            if drawn.weapon == Weapon::Grapple {
                continue;
            }
            let copy = Instance {
                part: drawn.part,
                weapon: drawn.weapon,
            };
            for tube in 0..drawn.weapon.tubes() {
                let muzzle = shooter_sim::aim::tube_muzzle(&build, &boxes, copy, tube).unwrap();
                let at = idle
                    .world_from_robot
                    .transform_point3(Vec3::from(muzzle.at) * 0.01);
                let drawn = tubes[usize::from(tube)]
                    .muzzles
                    .iter()
                    .find(|muzzle| muzzle.part == drawn.part)
                    .unwrap();
                assert!(
                    (at - drawn.position).length() < 0.01,
                    "{copy:?} tube {tube}: simulation {at} drawn {}",
                    drawn.position
                );
            }
        }
        // The rocket pod's two tubes are apart.
        if let Some(launcher) = idle
            .muzzles
            .iter()
            .find(|muzzle| muzzle.weapon == Weapon::RocketLauncher)
        {
            let other = tubes[1]
                .muzzles
                .iter()
                .find(|muzzle| muzzle.part == launcher.part)
                .unwrap();
            assert!((launcher.position - other.position).length() > 0.1);
        }
    }
}

#[test]
fn shots_start_at_the_muzzles_and_mods_ride_their_weapons() {
    let kit = kit();
    let model = Model::of(&STANDARD, kit).unwrap();
    let idle = posed(&model, &standing(), &PartStates::default());
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
    let modded = posed(&marksman, &standing(), &PartStates::default());
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
        let idle = posed(&model, &standing(), &PartStates::default());
        assert_eq!(idle.muzzles.len(), 3, "{name}: every weapon but the blade");
        for muzzle in &idle.muzzles {
            let along = muzzle.direction.normalize().dot(forward);
            assert!(along > within, "{name}: {:?} {along}", muzzle.weapon);
        }
        let blade = &model.parts[part_of(&model, "weapon.energy-blade")];
        let entry = blade.entry;
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
        let entry = model.parts[part_of(model, key)].entry;
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
            shooter_sim::Instance {
                part: part.id,
                weapon: held,
            },
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
    // The tube the simulation fired kicks; then both reload over its refire.
    let rocket = |tube, since| {
        let state = RobotState {
            fired: Instance::new(9, Weapon::RocketLauncher),
            since_fired: since,
            tube,
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
    let (a, b) = rocket(0, 0.1);
    assert!(a > 0.31 && (b - 0.3).abs() < 1e-4, "{a} {b}");
    let (a, b) = rocket(1, 0.1);
    assert!(b > 0.31 && (a - 0.3).abs() < 1e-4, "{a} {b}");
    let (a, b) = rocket(1, 0.5);
    assert!(a < 0.29 && b < 0.29, "reloading {a} {b}");
    let (a, b) = rocket(1, 5.0);
    assert!((a - 0.3).abs() < 1e-4 && (b - 0.3).abs() < 1e-4, "{a} {b}");
    // The rail gun recharges over its refire (the overcharger's is longer).
    let marksman = Model::of(&MARKSMAN, kit).unwrap();
    let rail_refire = |model: &Model| model.parts[part_of(model, "weapon.rail-gun")].refire;
    assert!((rail_refire(&heavy) - 1.2).abs() < 1e-4);
    assert!(rail_refire(&marksman) > 1.5);
    let recharging = RobotState {
        fired: Instance::new(7, Weapon::RailGun),
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
    let posed_hooked = posed(&standard, &hooked, &PartStates::default());
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
    let flying = pose::pose(&standard, kit, &hooked, &PartStates::default(), &memory);
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
    for bundle in Bundle::FIGHTERS {
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
    assert_eq!(kit.entries.len(), 28);
    // Anything but a plain archive of regular files is refused.
    assert!(load::untar(&[7; 1024]).is_err());
    let mut tar = std::fs::read(load::directory(Bundle::Weapons).join("kit.tar")).unwrap();
    tar[156] = b'2';
    assert!(load::untar(&tar).unwrap_err().contains("regular file"));
}

/// A rotation of `angle` about `axis` through `pivot`.
fn about(pivot: Vec3, axis: Vec3, angle: f32) -> Mat4 {
    Mat4::from_translation(pivot)
        * Mat4::from_axis_angle(axis, angle)
        * Mat4::from_translation(-pivot)
}

#[test]
fn every_build_draws_its_wheels_and_tracks_from_the_locomotion_kit() {
    let kit = kit();
    for (name, build) in builds() {
        let model = Model::of(&build, kit).expect(name);
        for drawn in &model.parts {
            assert_eq!(kit.entries[drawn.entry].key, drawn_key(drawn.key), "{name}");
            assert_eq!(kit.entries[drawn.entry].variant, "default", "{name}");
            let rolls = matches!(drawn.key, "robot.wheel" | "robot.track");
            assert_eq!(drawn.roll.is_some(), rolls, "{name}: {}", drawn.key);
            if rolls {
                let damaged = drawn
                    .damaged
                    .expect("a wheel and a track have a damaged look");
                assert_eq!(kit.entries[damaged].variant, "damaged");
            }
        }
    }
    // Only the plate, the wheel and the track have a damaged look.
    let damaged: Vec<&str> = kit
        .entries
        .iter()
        .filter(|entry| entry.variant == "damaged")
        .map(|entry| entry.key.as_str())
        .collect();
    assert_eq!(damaged, ["robot.armor.plate", "robot.track", "robot.wheel"]);
}

#[test]
fn the_roll_clip_turns_the_wheels_about_the_simulations_axles() {
    let kit = kit();
    for (key, nodes) in [
        ("robot.wheel", &["wheel"][..]),
        ("robot.track", &["sprocket", "idler"][..]),
    ] {
        let entry = &kit.entries[kit.entry(key, "default").unwrap()];
        let damaged = &kit.entries[kit.entry(key, "damaged").unwrap()];
        let rig = entry.rig.as_ref().expect("rigged");
        let rolling = entry
            .rolling
            .as_ref()
            .expect("its descriptor's locomotion block");
        // The damaged look has the same nodes and clip, so it swaps in under the same skin.
        let names = |rig: &shooter_client::robots::kit::Rig| -> Vec<String> {
            rig.joints.iter().map(|joint| joint.name.clone()).collect()
        };
        assert_eq!(names(rig), names(damaged.rig.as_ref().unwrap()), "{key}");
        assert_eq!(damaged.rolling.as_ref(), Some(rolling), "{key}");
        assert_eq!(rolling.axis, Vec3::Y, "{key}");
        assert_eq!(rolling.centres.len(), nodes.len(), "{key}");
        let clip = rig.clip(&rolling.clip).expect("the roll clip");
        for angle in [0.0_f32, 1.0, -2.5, 7.0] {
            let mut locals = rig.rest_locals();
            clip.pose(rolling.time(angle), &mut locals);
            let palette = rig.palette_of(&locals);
            // A positive angle turns right-handed about the axle, as the simulation's joint.
            for (node, centre) in nodes.iter().zip(&rolling.centres) {
                let joint = rig.joint(node).unwrap();
                let expected = about(*centre, rolling.axis, angle);
                assert!(
                    palette[joint].abs_diff_eq(expected, 2e-3),
                    "{key} {node} at {angle}: {} vs {expected}",
                    palette[joint]
                );
            }
        }
    }
    // The tyre is the simulation's wheel radius, the belt rolls on its road wheels.
    let radius = |key| {
        kit.entries[kit.entry(key, "default").unwrap()]
            .rolling
            .as_ref()
            .unwrap()
            .radius
    };
    assert!((radius("robot.wheel") - 0.18).abs() < 1e-6);
    assert!((radius("robot.track") - 0.1).abs() < 1e-6);
}

#[test]
fn wheels_and_tracks_sit_on_the_simulations_axles_and_stand_the_robot_on_them() {
    use shooter_sim::physics::articulated::Shape;
    let kit = kit();
    for (name, build) in builds() {
        let model = Model::of(&build, kit).unwrap();
        let layout = Layout::of(&build);
        let mut lowest = f32::MAX;
        for drawn in &model.parts {
            let Some(roll) = &drawn.roll else {
                continue;
            };
            // Each rolling body of the part, in the simulation's frame (metres).
            let bodies: Vec<(Vec3, f32, Vec3)> = layout
                .segments
                .iter()
                .filter(|segment| segment.part == drawn.id)
                .map(|segment| {
                    let child = &layout.bodies[segment.body].children[segment.child];
                    let Shape::Sphere(radius) = child.shape else {
                        panic!("{name}: a wheel or road wheel is a sphere")
                    };
                    let hinge = layout
                        .hinges
                        .iter()
                        .find(|hinge| hinge.b == segment.body)
                        .expect("on a hinge");
                    (
                        Vec3::from(child.position) * 0.01,
                        radius * 0.01,
                        Vec3::from(hinge.axis),
                    )
                })
                .collect();
            assert_eq!(bodies.len(), roll.centres.len(), "{name}: {}", drawn.key);
            for ((centre, radius, axis), drawn_centre) in bodies.iter().zip(&roll.centres) {
                assert!(
                    centre.abs_diff_eq(*drawn_centre, 1e-3),
                    "{name}: {} {centre} vs {drawn_centre}",
                    drawn.key
                );
                assert!(
                    axis.abs_diff_eq(roll.axis, 1e-4),
                    "{name}: {axis} vs {}",
                    roll.axis
                );
                assert!((radius - roll.radius).abs() < 1e-3, "{name}: {radius}");
                lowest = lowest.min(centre.y - radius);
            }
        }
        // A wheeled or tracked robot stands on its rolling bodies where they are lowest, as the
        // simulation stands it.
        if lowest < f32::MAX {
            assert!(
                model.floor <= lowest + 1e-3,
                "{name}: floor {} above its wheels {lowest}",
                model.floor
            );
        }
    }
    let car = Model::of(&fixtures::car(), kit).unwrap();
    let wheels = car.parts.iter().filter_map(|part| part.roll.as_ref());
    for roll in wheels {
        assert!(
            (roll.bottom() - car.floor).abs() < 1e-3,
            "{}",
            roll.bottom()
        );
    }
}

#[test]
fn wheels_and_tracks_roll_where_the_simulation_turns_them() {
    use shooter_sim::mechanics::Command;
    const RATE: i16 = 6_000;
    let kit = kit();
    let tank = mini_tank();
    for (name, build, controls) in [
        ("car", fixtures::car(), fixtures::wheels(RATE, -RATE)),
        (
            "strafer",
            fixtures::strafer(),
            fixtures::wheels(RATE, -RATE),
        ),
        (
            "mini-tank",
            tank,
            Controls::IDLE
                .with(PartId(1), Command::Rate(RATE))
                .with(PartId(2), Command::Rate(-RATE)),
        ),
    ] {
        let model = Model::of(&build, kit).unwrap();
        let driven = mechanics(build, 0, 20, |_| controls);
        let frames = frames_of(&driven);
        let posed = posed(
            &model,
            &standing(),
            &PartStates {
                frames: Some(&frames),
                ..PartStates::default()
            },
        );
        assert_on_the_simulations_bodies(&model, &posed, &driven);
        let mut rolling = 0;
        for actuator in &driven.robots()[0].actuators {
            let Some(index) = model
                .parts
                .iter()
                .position(|part| part.id == actuator.part && part.roll.is_some())
            else {
                continue;
            };
            let part = &model.parts[index];
            let angle = pose::roll_angle(part, &frames).unwrap();
            // The same turn about the axle, whole turns aside.
            let off = (angle - actuator.angle).rem_euclid(std::f32::consts::TAU);
            let off = off.min(std::f32::consts::TAU - off);
            assert!(
                off < 1e-3,
                "{name}: part {} drawn at {angle}, the simulation's {}",
                part.id.0,
                actuator.angle
            );
            rolling += usize::from(actuator.angle.abs() > 0.1);
            // The drawn wheel (a track's sprocket) turns by that angle about its axle.
            let entry = &kit.entries[part.entry];
            let rig = entry.rig.as_ref().unwrap();
            let rolls = entry.rolling.as_ref().unwrap();
            let node = rig
                .joint("wheel")
                .or_else(|| rig.joint("sprocket"))
                .unwrap();
            let expected = about(
                rolls.centres[0],
                rolls.axis,
                angle.rem_euclid(std::f32::consts::TAU),
            );
            assert!(
                posed.parts[index].palette.as_ref().unwrap()[node].abs_diff_eq(expected, 2e-3),
                "{name}: part {}",
                part.id.0
            );
        }
        assert!(rolling > 0, "{name}: its wheels turned");
    }
}

#[test]
fn a_lost_wheel_lies_at_rest_and_a_damaged_one_draws_its_damaged_look() {
    let kit = kit();
    let car = Model::of(&fixtures::car(), kit).unwrap();
    let wheel = car
        .parts
        .iter()
        .position(|part| part.roll.is_some())
        .unwrap();
    let id = car.parts[wheel].id;
    let lying = Mat4::from_translation(Vec3::new(4.0, 0.0, 4.0));
    let detached = [(id, lying)];
    let driven = mechanics(fixtures::car(), 0, 20, |_| fixtures::wheels(6_000, -6_000));
    let frames = frames_of(&driven);
    let damaged = [id];
    let posed = posed(
        &car,
        &standing(),
        &PartStates {
            detached: &detached,
            damaged: &damaged,
            frames: Some(&frames),
        },
    );
    let part = &posed.parts[wheel];
    assert!(part.detached && part.damaged);
    assert!(
        part.world_from_local
            .abs_diff_eq(lying * car.parts[wheel].robot_from_local, 1e-5)
    );
    for matrix in part.palette.as_ref().unwrap() {
        assert!(matrix.abs_diff_eq(Mat4::IDENTITY, 1e-4), "{matrix}");
    }
    // The other wheels stay on and whole.
    for (index, other) in posed.parts.iter().enumerate() {
        if index != wheel {
            assert!(!other.detached && !other.damaged);
        }
    }
}

/// Several copies of one weapon (shooter#125): the twin rail draws both rail guns at their own
/// mounts with their own muzzles, only the copy that fired plays its shot, and a lost copy comes
/// off while the other stays on, muzzle and all.
#[test]
fn every_copy_of_a_weapon_is_drawn_at_its_mount_and_on_its_own() {
    use shooter_sim::robot::TWIN_RAIL;
    let kit = kit();
    let model = Model::of(&TWIN_RAIL, kit).unwrap();
    let index = |id: u8| {
        model
            .parts
            .iter()
            .position(|part| part.id == PartId(id))
            .unwrap()
    };
    let (left, right) = (index(7), index(9));
    let idle = posed(&model, &standing(), &PartStates::default());
    let rails: Vec<_> = idle
        .muzzles
        .iter()
        .filter(|muzzle| muzzle.weapon == Weapon::RailGun)
        .collect();
    assert_eq!(
        rails.iter().map(|muzzle| muzzle.part).collect::<Vec<_>>(),
        [PartId(7), PartId(9)]
    );
    assert!((rails[0].position - rails[1].position).length() > 0.15);
    let mount = |posed: &Posed, at: usize| posed.parts[at].world_from_local.w_axis.truncate();
    assert!((mount(&idle, left) - mount(&idle, right)).length() > 0.15);
    // The right one fired: its rig plays the shot, the left one stays at rest.
    let fired = RobotState {
        held: Instance::new(9, Weapon::RailGun),
        fired: Instance::new(9, Weapon::RailGun),
        since_fired: 0.05,
        ..standing()
    };
    let shot = posed(&model, &fired, &PartStates::default());
    assert_eq!(shot.parts[left].palette, idle.parts[left].palette);
    assert_ne!(shot.parts[right].palette, idle.parts[right].palette);
    // Both fired together (shooter#127): each plays its own shot.
    let mut recent = shooter_client::robots::pose::Recent::NONE;
    recent.fired(PartId(7));
    recent.fired(PartId(9));
    recent.advance(0.05);
    let together = posed(
        &model,
        &RobotState { recent, ..fired },
        &PartStates::default(),
    );
    assert_eq!(together.parts[right].palette, shot.parts[right].palette);
    assert_ne!(together.parts[left].palette, idle.parts[left].palette);
    // The left one lost: it comes off, the right one stays on with its muzzle.
    let away = Mat4::from_translation(Vec3::new(0.0, -1.0, 2.0));
    let detached = [(PartId(7), away)];
    let lost = posed(
        &model,
        &standing(),
        &PartStates {
            detached: &detached,
            ..PartStates::default()
        },
    );
    assert!(lost.parts[left].detached);
    assert!(!lost.parts[right].detached);
    assert_eq!(
        lost.parts[right].world_from_local,
        idle.parts[right].world_from_local
    );
    let rails: Vec<PartId> = lost
        .muzzles
        .iter()
        .filter(|muzzle| muzzle.weapon == Weapon::RailGun)
        .map(|muzzle| muzzle.part)
        .collect();
    assert_eq!(rails, [PartId(9)]);
}

/// Mount rails and adapters (shooter#126): every fixture's parts, the rails, the adapter and
/// what they carry, are drawn on the simulation's bodies where it holds them; with the rack's
/// long rail shot off, the rail and its four weapons come off and the short rail's stay drawn
/// where the simulation keeps them.
#[test]
fn what_rails_and_adapters_carry_is_drawn_where_the_simulation_holds_it() {
    use shooter_sim::robot::{ADAPTER_MAST, CROSS_RAIL, RAIL_RACK};
    let kit = kit();
    for (name, build) in [
        ("rail-rack", RAIL_RACK),
        ("cross-rail", CROSS_RAIL),
        ("adapter-mast", ADAPTER_MAST),
    ] {
        let model = Model::of(&build, kit).expect(name);
        let settled = mechanics(build, 0, 30, |_| Controls::IDLE);
        let frames = frames_of(&settled);
        let posed = posed(
            &model,
            &standing(),
            &PartStates {
                frames: Some(&frames),
                ..PartStates::default()
            },
        );
        assert_on_the_simulations_bodies(&model, &posed, &settled);
    }
    let index = |id: u8| {
        RAIL_RACK
            .parts()
            .iter()
            .position(|part| part.id == PartId(id))
            .unwrap()
    };
    let lost: u32 = [3, 4, 5, 6, 7].iter().map(|&id| 1 << index(id)).sum();
    let model = Model::of(&RAIL_RACK, kit).unwrap();
    let severed = mechanics(RAIL_RACK, lost, 30, |_| Controls::IDLE);
    let frames = frames_of(&severed);
    let detached: Vec<(PartId, Mat4)> = [3, 4, 5, 6, 7]
        .iter()
        .map(|&id| (PartId(id), Mat4::from_translation(Vec3::new(3.0, 0.0, 0.0))))
        .collect();
    let posed = posed(
        &model,
        &standing(),
        &PartStates {
            frames: Some(&frames),
            detached: &detached,
            ..PartStates::default()
        },
    );
    assert_on_the_simulations_bodies(&model, &posed, &severed);
    for (at, part) in model.parts.iter().enumerate() {
        let lost_here = lost & (1 << at) != 0;
        assert_eq!(posed.parts[at].detached, lost_here, "{}", part.key);
    }
    // The muzzles left are the short rail's: the laser and the flamethrower.
    let muzzles: Vec<PartId> = posed.muzzles.iter().map(|muzzle| muzzle.part).collect();
    assert_eq!(muzzles, [PartId(9), PartId(10)]);
}
