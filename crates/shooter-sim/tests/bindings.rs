//! Control layouts (shooter#74, Atlas D2.i70.7): bindings from keys, mouse buttons and mouse axes
//! to actuators and action groups resolve deterministically into per-actuator commands, with
//! inversion per binding and per part, hold and toggle, mouse steering that stops with the
//! mouse (relative) or stays where it was left (offset); the presets drive each locomotion as
//! it is mounted, correcting nothing, and aim a two-axis turret; layouts and the commands
//! travel exactly and bounded, and the host keeps only commands the robot can take, so a client
//! predicting with its layout's commands continues exactly like the host.

use shooter_sim::bindings::{
    Binding, BindingError, BindingState, Bindings, MAX_BINDINGS, MAX_GROUP, MAX_MOTORS, Mode,
    Motors, Output, Pressed, Source, Target, key,
};
use shooter_sim::mechanics::fixtures::{self, HINGE_PART, ROTOR_PART, WHEELS, floor, slope};
use shooter_sim::mechanics::{Command, GaitChoice, Layout, Mechanics};
use shooter_sim::physics::RigidBody;
use shooter_sim::physics::Vec3i;
use shooter_sim::robot::{Assembly, PartId, STANDARD};
use shooter_sim::wire::{
    decode_bindings, decode_input, decode_snapshot, encode_bindings, encode_input, encode_snapshot,
};
use shooter_sim::{Arena, Input, Intent, Rules, Session, Spawn};

const LEFT_LEG: PartId = PartId(5);
const RIGHT_LEG: PartId = PartId(6);

fn held(code: &str) -> Pressed {
    Pressed::default().with_key(code)
}

fn source(code: &str) -> Source {
    Source::Key(key(code).unwrap())
}

fn binding(source: Source, mode: Mode, output: Output, targets: &[(PartId, bool)]) -> Binding {
    Binding {
        source,
        mode,
        output,
        invert: false,
        strength: 100,
        targets: targets
            .iter()
            .map(|&(part, invert)| Target { part, invert })
            .collect(),
    }
}

fn layout(bindings: Vec<Binding>) -> Bindings {
    Bindings {
        gait: GaitChoice::Suggested,
        bindings,
    }
}

/// The standard robot's legs both on one key.
fn legs_on(code: &str, mode: Mode) -> Bindings {
    layout(vec![binding(
        source(code),
        mode,
        Output::Stride,
        &[(LEFT_LEG, false), (RIGHT_LEG, false)],
    )])
}

#[test]
fn the_same_inputs_resolve_to_the_same_commands() {
    let robot = Layout::of(&fixtures::paired_turret());
    let preset = Bindings::preset(&robot);
    let inputs: Vec<Pressed> = (0..240)
        .map(|tick: i32| Pressed {
            keys: if tick % 50 < 20 { held("KeyW").keys } else { 0 }
                | if tick % 70 < 10 { held("KeyD").keys } else { 0 },
            buttons: 0,
            mouse: [(tick % 13) - 6, (tick % 7) - 3],
        })
        .collect();
    let play = || {
        let mut state = BindingState::default();
        inputs
            .iter()
            .map(|pressed| preset.resolve(&robot, &mut state, *pressed))
            .collect::<Vec<Motors>>()
    };
    let first = play();
    assert_eq!(first, play());
    // The commands travel exactly in an input.
    for motors in &first {
        let intent = Intent {
            motors: *motors,
            ..Intent::IDLE
        };
        assert_eq!(
            decode_input(&encode_input(intent, 0, None, None))
                .unwrap()
                .intent,
            intent
        );
    }
}

#[test]
fn an_action_group_drives_every_motor_in_it_each_its_own_way() {
    // The car's preset: W turns all four wheels, the left ones (axles out to the left) the
    // other way from the right ones, so all four roll forward.
    let car = Layout::of(&fixtures::car());
    let preset = Bindings::preset(&car);
    let motors = preset.resolve(&car, &mut BindingState::default(), held("KeyW"));
    let rates: Vec<i16> = WHEELS
        .iter()
        .map(|part| match motors.get(*part) {
            Some(Command::Rate(rate)) => rate,
            other => panic!("{part:?}: {other:?}"),
        })
        .collect();
    // Top rate 12 rad/s.
    assert_eq!(rates[0], rates[1]);
    assert_eq!(rates[2], rates[3]);
    assert_eq!(rates[0], -rates[2]);
    assert_eq!(rates[0].abs(), 12_000);
    // D (60 %) turns one side forward and the other back: every wheel the same way.
    let motors = preset.resolve(&car, &mut BindingState::default(), held("KeyD"));
    let turning: Vec<Option<Command>> = WHEELS.iter().map(|part| motors.get(*part)).collect();
    assert!(
        turning.iter().all(|command| *command == turning[0]),
        "{turning:?}"
    );
    assert_eq!(
        turning[0].map(|c| matches!(c, Command::Rate(r) if r.abs() == 7_200)),
        Some(true)
    );
    // W and D together sum per wheel: one side at full, the other at 40 %, clamped to the top.
    let both = preset.resolve(
        &car,
        &mut BindingState::default(),
        Pressed {
            keys: held("KeyW").keys | held("KeyD").keys,
            ..Pressed::default()
        },
    );
    let magnitudes: Vec<i16> = WHEELS
        .iter()
        .map(|part| match both.get(*part) {
            Some(Command::Rate(rate)) => rate.abs(),
            other => panic!("{other:?}"),
        })
        .collect();
    assert!(
        magnitudes.contains(&12_000) && magnitudes.contains(&4_800),
        "{magnitudes:?}"
    );
}

#[test]
fn inversion_flips_a_binding_and_a_part_flips_its_own_share() {
    let robot = Layout::of(&STANDARD);
    let mut flipped = legs_on("KeyW", Mode::Hold);
    flipped.bindings[0].invert = true;
    flipped.bindings[0].targets[1].invert = true;
    flipped.bindings[0].strength = 40;
    let motors = flipped.resolve(&robot, &mut BindingState::default(), held("KeyW"));
    assert_eq!(motors.get(LEFT_LEG), Some(Command::Stride(-40)));
    assert_eq!(motors.get(RIGHT_LEG), Some(Command::Stride(40)));
}

#[test]
fn hold_lasts_while_held_and_toggle_switches_on_each_fresh_press() {
    let robot = Layout::of(&STANDARD);
    let up = Pressed::default();
    let down = held("KeyG");
    let sequence = [down, down, up, up, down, up, down, down];
    let strides = |bindings: &Bindings| -> Vec<Option<Command>> {
        let mut state = BindingState::default();
        sequence
            .iter()
            .map(|pressed| bindings.resolve(&robot, &mut state, *pressed).get(LEFT_LEG))
            .collect()
    };
    let walk = Some(Command::Stride(100));
    let stand = Some(Command::Stride(0));
    assert_eq!(
        strides(&legs_on("KeyG", Mode::Hold)),
        [walk, walk, stand, stand, walk, stand, walk, walk]
    );
    // On at the first press and held on through the release and the held key; off at the
    // second press; on again at the third, and a key kept down does not switch it again.
    assert_eq!(
        strides(&legs_on("KeyG", Mode::Toggle)),
        [walk, walk, walk, walk, stand, stand, walk, walk]
    );
}

#[test]
fn relative_mouse_steering_stops_with_the_mouse_and_an_offset_stays() {
    let robot = Layout::of(&fixtures::paired_turret());
    let steer = |mode: Mode| {
        layout(vec![Binding {
            strength: 10,
            ..binding(Source::MouseX, mode, Output::Rate, &[(ROTOR_PART, false)])
        }])
    };
    let moving = Pressed {
        mouse: [30, 0],
        ..Pressed::default()
    };
    let still = Pressed::default();
    let rates = |bindings: &Bindings| -> Vec<Option<Command>> {
        let mut state = BindingState::default();
        [moving, moving, still, still]
            .iter()
            .map(|pressed| {
                bindings
                    .resolve(&robot, &mut state, *pressed)
                    .get(ROTOR_PART)
            })
            .collect()
    };
    // The rotor's top rate is 4 rad/s. Relative: 30 counts × 10 = 300 ‰ while the mouse
    // moves, nothing once it stops.
    assert_eq!(
        rates(&steer(Mode::Relative)),
        [
            Some(Command::Rate(1_200)),
            Some(Command::Rate(1_200)),
            Some(Command::Rate(0)),
            Some(Command::Rate(0))
        ]
    );
    // Offset: each tick's movement pushes the offset (30 × 10 / 10 = 30 ‰) and it stays.
    assert_eq!(
        rates(&steer(Mode::Offset)),
        [
            Some(Command::Rate(120)),
            Some(Command::Rate(240)),
            Some(Command::Rate(240)),
            Some(Command::Rate(240))
        ]
    );
}

#[test]
fn two_joint_axes_aim_a_turret_from_the_mouse_offset_and_hold_it() {
    let build = fixtures::paired_turret();
    let mut world = Mechanics::new(&floor()).unwrap();
    world.spawn(build, [0.0, 0.0, 0.0], 0).unwrap();
    let robot = world.robots()[0].layout.clone();
    let preset = Bindings::preset(&robot);
    let mut state = BindingState::default();
    // The mouse moves right and up for half a second, then stops: the rotor and the hinge go
    // to the offset it left and hold there.
    for tick in 0..180 {
        let mouse = if tick < 30 { [-10, -8] } else { [0, 0] };
        let motors = preset.resolve(
            &robot,
            &mut state,
            Pressed {
                mouse,
                ..Pressed::default()
            },
        );
        world.step(&[motors.controls(&robot)]).unwrap();
    }
    let actuator = |part: PartId| {
        *world.robots()[0]
            .actuators
            .iter()
            .find(|actuator| actuator.part == part)
            .unwrap()
    };
    // Offsets: 30 × 10 × 20 / 10 = 600 ‰ and 30 × 8 × 20 / 10 = 480 ‰ of a quarter turn.
    let quarter = std::f32::consts::FRAC_PI_2;
    let (rotor, hinge) = (actuator(ROTOR_PART), actuator(HINGE_PART));
    assert!((rotor.angle - 0.6 * quarter).abs() < 0.03, "{rotor:?}");
    assert!((hinge.angle - 0.48 * quarter).abs() < 0.03, "{hinge:?}");
    assert!(rotor.rate.abs() < 0.05 && hinge.rate.abs() < 0.05);
}

/// The robot in `world` after `ticks` of its preset with `pressed` held: its root's position
/// and the lowest its up axis came during the drive.
fn drive(
    statics: &[RigidBody],
    build: Assembly,
    pressed: Pressed,
    ticks: u32,
) -> ([f32; 3], [f32; 3], f32) {
    let mut world = Mechanics::new(statics).unwrap();
    world.spawn(build, [0.0, 0.0, 0.0], 0).unwrap();
    let robot = world.robots()[0].layout.clone();
    let preset = Bindings::preset(&robot);
    let mut state = BindingState::default();
    let start = world.root(0).unwrap().position;
    let mut lowest_up = 1.0f32;
    for _ in 0..ticks {
        // The fixtures carry no generator (shooter#128): kept charged, so only the locomotion
        // is on trial.
        if let Some(robot) = world.robot_mut(0) {
            robot.charge = robot.pool.capacity;
        }
        let motors = preset.resolve(&robot, &mut state, pressed);
        world.step(&[motors.controls(&robot)]).unwrap();
        let [x, _, z, _] = world.root(0).unwrap().orientation;
        lowest_up = lowest_up.min(1.0 - 2.0 * (x * x + z * z));
    }
    let end = world.root(0).unwrap().position;
    (start, end, lowest_up)
}

#[test]
fn the_presets_drive_wheels_and_legs_as_mounted_on_flat_ground_and_a_slope() {
    let w = held("KeyW");
    // Wheels: about 2 m/s ahead, upright; D turns it on the spot.
    let (start, end, up) = drive(&floor(), fixtures::car(), w, 300);
    assert!(
        end[0] - start[0] > 900.0 && (end[2] - start[2]).abs() < 10.0,
        "{end:?}"
    );
    assert!(up > 0.99);
    let (start, end, _) = drive(&floor(), fixtures::car(), held("KeyD"), 300);
    assert!(
        libm::hypotf(end[0] - start[0], end[2] - start[2]) < 5.0,
        "{end:?}"
    );
    // Up a 1:10 slope that starts 40 cm ahead.
    let (start, end, up) = drive(&slope(40, 1_600, 160), fixtures::car(), w, 400);
    assert!(end[1] - start[1] > 80.0, "{end:?}");
    assert!(up > 0.95, "{up}");

    // A quadruped (the robo-dog, its suggested crawl) walks ahead on W and stays up, and climbs
    // a 1:20 slope; a 1:10 one it reaches and slides back down (physics-engine 302416a).
    let (start, end, up) = drive(&floor(), fixtures::dog(false), w, 300);
    assert!(end[0] - start[0] > 120.0, "{end:?}");
    assert!(up > 0.95);
    let (start, end, up) = drive(&slope(40, 2_400, 120), fixtures::dog(false), w, 900);
    assert!(
        end[0] - start[0] > 150.0 && end[1] - start[1] > 5.0,
        "{end:?}"
    );
    assert!(up > 0.95);

    // A biped (the fixture: its legs under its rear half) steps ahead on W and stays up.
    let (start, end, up) = drive(&floor(), fixtures::biped(), w, 300);
    assert!(end[0] - start[0] > 80.0, "{end:?}");
    assert!(up > 0.95);
}

#[test]
fn badly_mounted_locomotion_is_not_corrected() {
    let w = held("KeyW");
    // Wheels on the front and back roll the robot sideways for W: their mounting says so.
    let (start, end, _) = drive(&floor(), fixtures::strafer(), w, 300);
    assert!(
        end[2] - start[2] > 900.0 && (end[0] - start[0]).abs() < 10.0,
        "{end:?}"
    );
    // The dog's legs turned a quarter swing it sideways and round: it does not walk ahead.
    let (start, end, up) = drive(&floor(), fixtures::dog(true), w, 300);
    assert!(end[0] - start[0] < 40.0, "{end:?}");
    assert!(up > 0.9);
    // Legs on the roof step in the air.
    let (start, end, _) = drive(&floor(), fixtures::upside_down_legs(), w, 180);
    assert!(
        libm::hypotf(end[0] - start[0], end[2] - start[2]) < 10.0,
        "{end:?}"
    );
}

#[test]
fn layouts_that_do_not_fit_their_robot_or_their_bounds_are_refused() {
    let robot = Layout::of(&STANDARD);
    assert_eq!(Bindings::preset(&robot).check(&robot), Ok(()));
    let refused = |bindings: Bindings| bindings.check(&robot).unwrap_err();
    // The chassis (part 0) is no actuator; a stride needs a leg.
    assert_eq!(
        refused(layout(vec![binding(
            source("KeyW"),
            Mode::Hold,
            Output::Rate,
            &[(PartId(0), false)]
        )])),
        BindingError::NotAnActuator(0, PartId(0))
    );
    let car = Layout::of(&fixtures::car());
    assert_eq!(
        layout(vec![binding(
            source("KeyW"),
            Mode::Hold,
            Output::Stride,
            &[(WHEELS[0], false)]
        )])
        .check(&car),
        Err(BindingError::NotALeg(0, WHEELS[0]))
    );
    assert_eq!(
        refused(layout(vec![binding(
            Source::Key(200),
            Mode::Hold,
            Output::Stride,
            &[(LEFT_LEG, false)]
        )])),
        BindingError::Source(0)
    );
    assert_eq!(
        refused(layout(vec![binding(
            Source::MouseX,
            Mode::Hold,
            Output::Stride,
            &[(LEFT_LEG, false)]
        )])),
        BindingError::Mode(0)
    );
    assert_eq!(
        refused(layout(vec![binding(
            source("KeyW"),
            Mode::Offset,
            Output::Stride,
            &[(LEFT_LEG, false)]
        )])),
        BindingError::Mode(0)
    );
    assert_eq!(
        refused(layout(vec![Binding {
            strength: 0,
            ..binding(
                source("KeyW"),
                Mode::Hold,
                Output::Stride,
                &[(LEFT_LEG, false)]
            )
        }])),
        BindingError::Strength(0)
    );
    assert_eq!(
        refused(layout(vec![binding(
            source("KeyW"),
            Mode::Hold,
            Output::Stride,
            &[(LEFT_LEG, false), (LEFT_LEG, true)]
        )])),
        BindingError::Twice(0, LEFT_LEG)
    );
    assert_eq!(
        refused(layout(vec![binding(
            source("KeyW"),
            Mode::Hold,
            Output::Stride,
            &[]
        )])),
        BindingError::Group(0)
    );
    let crowd: Vec<(PartId, bool)> = (0..=MAX_GROUP as u8)
        .map(|id| (PartId(id), false))
        .collect();
    assert_eq!(
        refused(layout(vec![binding(
            source("KeyW"),
            Mode::Hold,
            Output::Rate,
            &crowd
        )])),
        BindingError::Group(0)
    );
    let many = layout(vec![
        legs_on("KeyW", Mode::Hold).bindings[0].clone();
        MAX_BINDINGS + 1
    ]);
    assert_eq!(refused(many), BindingError::TooMany);
    // A layout made for another build is pruned to what this one can take, not broken: of the
    // car's wheel bindings only part 5 is left, the standard robot's left leg (which takes a
    // rate at its hip).
    let pruned = Bindings::preset(&Layout::of(&fixtures::car())).pruned(&robot);
    assert!(!pruned.bindings.is_empty());
    assert!(
        pruned
            .bindings
            .iter()
            .all(|binding| binding.targets.iter().all(|target| target.part == LEFT_LEG))
    );
    assert_eq!(pruned.check(&robot), Ok(()));
}

#[test]
fn layouts_travel_exactly_and_tampered_bytes_are_refused() {
    for build in [
        STANDARD,
        fixtures::car(),
        fixtures::tank(),
        fixtures::dog(false),
        fixtures::paired_turret(),
    ] {
        let mut preset = Bindings::preset_of(&build);
        preset.gait = GaitChoice::Unison;
        if let Some(first) = preset.bindings.first_mut() {
            first.mode = Mode::Toggle;
            first.invert = true;
            first.strength = 37;
        }
        let bytes = encode_bindings(&preset);
        assert_eq!(decode_bindings(&bytes).unwrap(), preset);
        assert!(decode_bindings(&bytes[..bytes.len() - 1]).is_err());
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(decode_bindings(&trailing).is_err());
    }
    let bytes = encode_bindings(&legs_on("KeyW", Mode::Hold));
    // gait, count, then source kind, index, mode, output, flags, strength, group size...
    let tamper = |at: usize, value: u8| {
        let mut bytes = bytes.clone();
        bytes[at] = value;
        decode_bindings(&bytes)
    };
    assert!(tamper(0, 9).is_err(), "unknown gait");
    assert!(tamper(1, 200).is_err(), "too many bindings");
    assert!(tamper(2, 7).is_err(), "unknown source");
    assert!(tamper(3, 200).is_err(), "no such key");
    assert!(tamper(4, 2).is_err(), "a key read as a mouse offset");
    assert!(tamper(5, 4).is_err(), "unknown output");
    assert!(tamper(6, 2).is_err(), "unknown flag");
    assert!(tamper(7, 0).is_err(), "no strength");
    assert!(tamper(8, 9).is_err(), "too many parts");

    // An input's commands: past the bound, a part twice, a stride past 100 or an unknown kind
    // refuse the input whole.
    let intent = Intent {
        motors: legs_on("KeyW", Mode::Hold).resolve(
            &Layout::of(&STANDARD),
            &mut BindingState::default(),
            held("KeyW"),
        ),
        ..Intent::IDLE
    };
    let bytes = encode_input(intent, 3, None, None);
    // After the commands: the fire requests (none), the view tick, the name and the build flag.
    let motors_at = bytes.len() - (1 + 4 + 1 + 1) - (2 + 2 * 4);
    let tamper = |at: usize, value: u8| {
        let mut bytes = bytes.clone();
        bytes[motors_at + at] = value;
        decode_input(&bytes)
    };
    assert_eq!(decode_input(&bytes).unwrap().intent, intent);
    assert!(tamper(1, (MAX_MOTORS + 1) as u8).is_err(), "too many");
    assert!(tamper(6, 5).is_err(), "the same leg twice");
    assert!(tamper(3, 0).is_err(), "idle with a value");
    assert!(tamper(4, 101).is_err(), "stride 101");
    assert!(tamper(3, 9).is_err(), "unknown command");
}

#[test]
fn the_host_keeps_only_commands_the_robot_takes() {
    let robot = Layout::of(&STANDARD);
    let mut motors = Motors::REST;
    motors.push(PartId(0), Command::Rate(9_000)); // the chassis: no actuator
    motors.push(PartId(7), Command::Angle(4_000)); // a gun: no actuator
    motors.push(LEFT_LEG, Command::Angle(3_000)); // a hip angle: a leg takes it
    let controls = motors.controls(&robot);
    assert_eq!(controls.commands[0], Command::Idle);
    assert_eq!(controls.commands[7], Command::Idle);
    assert_eq!(
        controls.commands[usize::from(LEFT_LEG.0)],
        Command::Angle(3_000)
    );
    // Untold, the other leg stands.
    assert_eq!(
        controls.commands[usize::from(RIGHT_LEG.0)],
        Command::Stride(0)
    );
    // A stride for a wheel is no command a wheel takes: it rests.
    let car = Layout::of(&fixtures::car());
    let mut striding = Motors::REST;
    striding.push(WHEELS[0], Command::Stride(80));
    assert_eq!(
        striding.controls(&car).commands[usize::from(WHEELS[0].0)],
        Command::Rate(0)
    );
}

/// An open floor with one spawn point, no pads.
fn open_floor() -> Arena {
    Arena {
        half_size: 2_000,
        pieces: Vec::new(),
        pads: Vec::new(),
        posts: Vec::new(),
        spawn: Vec3i::new(0, 0, -600),
        spawn_yaw: 0,
        spawns: vec![Spawn {
            feet: Vec3i::new(0, 0, 600),
            yaw: 32_768,
        }],
        module_pads: Vec::new(),
        waypoints: Vec::new(),
        placements: Vec::new(),
        repair_anchors: Vec::new(),
    }
}

/// A person's turret, aimed through the preset by the mouse offset, predicted from the host's
/// snapshot with the same inputs: bit for bit, the joints' turn included; a tampered command for
/// a part that is no actuator changes nothing.
#[test]
fn a_client_predicts_its_turret_exactly_and_tampered_commands_change_nothing() {
    let rules = Rules {
        slots: 1,
        bots: false,
        local: false,
        seed: 0,
        frag_limit: 0,
        time_limit_ticks: 0,
        intermission_ticks: 60,
        respawn_ticks: 30,
    };
    let play = |tampered: bool| {
        let mut host = Session::with_rules(open_floor(), rules).unwrap();
        host.join(7).unwrap();
        assert!(host.configure(7, fixtures::paired_turret()));
        let mut predictor = Session::new(open_floor()).unwrap();
        let robot = Layout::of(&fixtures::paired_turret());
        let preset = Bindings::preset(&robot);
        let mut state = BindingState::default();
        let mut joints = Vec::new();
        for tick in 0..240u32 {
            let pressed = Pressed {
                keys: if tick % 80 < 40 { held("KeyW").keys } else { 0 },
                buttons: 0,
                mouse: if tick % 60 < 15 { [-6, -4] } else { [0, 0] },
            };
            let mut motors = preset.resolve(&robot, &mut state, pressed);
            if tampered {
                motors.push(PartId(0), Command::Rate(30_000));
                motors.push(PartId(8), Command::Angle(12_000));
            }
            let intent = Intent {
                yaw: 8_000,
                motors,
                ..Intent::IDLE
            };
            host.queue(
                7,
                Input {
                    sequence: tick + 1,
                    intent,
                    view_tick: host.ticks(),
                },
            );
            host.step().unwrap();
            let snapshot = decode_snapshot(&encode_snapshot(&host.snapshot())).unwrap();
            let me = snapshot.of_player(7).unwrap().clone();
            let robot_state = me.robot.clone().unwrap();
            if tick == 20 {
                predictor.fighters[0].energy = me.energy;
                predictor
                    .place(0, &robot_state, me.motion, me.held, me.hook)
                    .unwrap();
            } else if tick > 20 {
                predictor.tick(intent).unwrap();
                let predicted = predictor.mechanics.state().robots[0].clone();
                assert_eq!(predicted.bodies, robot_state.bodies, "tick {tick}");
                assert_eq!(predictor.local().energy, me.energy, "tick {tick}");
            }
            joints.push(
                host.mechanics.robots()[0]
                    .actuators
                    .iter()
                    .map(|actuator| actuator.angle.to_bits())
                    .collect::<Vec<u32>>(),
            );
        }
        let rotor = host.mechanics.robots()[0]
            .actuators
            .iter()
            .find(|actuator| actuator.part == ROTOR_PART)
            .unwrap()
            .angle;
        assert!(rotor.abs() > 0.2, "the mouse turned the turret: {rotor}");
        joints
    };
    assert_eq!(play(false), play(true));
}
