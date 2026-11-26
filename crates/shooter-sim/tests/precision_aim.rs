//! Precision aim (shooter#132, Atlas D2.i123.8): joints bound to track the aim point turn the gun
//! they carry toward the camera's target only through their real limits, servo speed and the
//! pool's energy, resolved by the host from its own state; every gun's shot leaves its muzzle
//! toward the target only within its convergence cone about its mount, else along the mount, so
//! fixed, sideways and occluded guns fire only where their mounts let them. The same inputs give
//! the same match on the host, a predicting client and a replay.

use shooter_sim::aim::{self, CONVERGENCE_COS, Lands, Reach};
use shooter_sim::bindings::{
    AIM_BUTTON, ALL_GUNS_BUTTON, Binding, BindingError, BindingState, Bindings, Mode, Motors,
    Output, Pressed, Source, Target,
};
use shooter_sim::mechanics::fixtures::{self, BARREL, HINGE_PART, ROTOR_PART};
use shooter_sim::mechanics::{Command, Layout};
use shooter_sim::physics::Vec3i;
use shooter_sim::robot::{
    Assembly, BETA, Candidate, Face, PartId, PartType, RawMount, RawPart, STANDARD, parts as kit,
};
use shooter_sim::session::body_of;
use shooter_sim::wire::{decode_input, decode_snapshot, encode_input, encode_snapshot};
use shooter_sim::{Arena, Input, Intent, Piece, Rules, Select, Session, Spawn, Weapon};

/// A wall 15 m south (+Z) of the spawn, 20 m high: what the camera's target lands on.
fn walled() -> Arena {
    Arena {
        half_size: 3_000,
        pieces: vec![Piece::Block {
            center: Vec3i::new(0, 1_000, 1_550),
            half: Vec3i::new(2_500, 1_000, 50),
        }],
        pads: Vec::new(),
        posts: Vec::new(),
        spawn: Vec3i::new(0, 0, 0),
        spawn_yaw: 0,
        spawns: vec![Spawn {
            feet: Vec3i::new(0, 0, 0),
            yaw: 0,
        }],
        module_pads: Vec::new(),
        waypoints: Vec::new(),
        placements: Vec::new(),
        repair_anchors: Vec::new(),
    }
}

fn rules() -> Rules {
    Rules {
        slots: 1,
        bots: false,
        local: false,
        seed: 0,
        frag_limit: 0,
        time_limit_ticks: 0,
        intermission_ticks: 60,
        respawn_ticks: 30,
    }
}

/// A host with person 7 in slot 0 as `build`.
fn host(build: Assembly) -> Session {
    let mut host = Session::with_rules(walled(), rules()).unwrap();
    host.join(7).unwrap();
    assert!(host.configure(7, build));
    host
}

fn input(host: &mut Session, sequence: u32, intent: Intent) {
    let view_tick = host.ticks();
    host.queue(
        7,
        Input {
            sequence,
            intent,
            view_tick,
        },
    );
}

/// The yaw and pitch that put slot 0's reticle on `point`.
fn aiming_at(session: &Session, point: [f32; 3]) -> (u16, i16) {
    aim::at(Some(&session.world), session.fighters[0].eye(), point)
}

/// The right mouse button held: the preset's tracking binding.
fn aim_held() -> Pressed {
    Pressed {
        buttons: 1 << AIM_BUTTON,
        ..Pressed::default()
    }
}

/// The angle (radians) between the barrel's mount direction and the line from its muzzle to
/// `point`.
fn off_target(session: &Session, gun: PartId, point: [f32; 3]) -> f32 {
    let muzzle = session.fighters[0].muzzle(Select::Part(gun)).unwrap();
    let axis = muzzle.axis.unwrap();
    let toward = [0, 1, 2].map(|i| point[i] - muzzle.at[i]);
    let length = toward.iter().map(|v| v * v).sum::<f32>().sqrt();
    let cos = (0..3).map(|i| axis[i] * toward[i]).sum::<f32>() / length;
    libm::acosf(cos.clamp(-1.0, 1.0))
}

fn joint_angle(session: &Session, part: PartId) -> f32 {
    session.mechanics.robots()[0]
        .actuators
        .iter()
        .find(|actuator| actuator.part == part)
        .unwrap()
        .angle
}

/// Runs the paired turret on the host for `ticks`, its reticle on `point` and the tracking
/// binding held (from tick `from` on), with `each` run before every step.
fn track(
    host: &mut Session,
    point: [f32; 3],
    ticks: u32,
    mut each: impl FnMut(&mut Session, u32),
) -> Vec<Intent> {
    let layout = Layout::of(&host.fighters[0].build);
    let preset = Bindings::preset(&layout);
    let mut state = BindingState::default();
    let mut sent = Vec::new();
    for tick in 0..ticks {
        let (yaw, pitch) = aiming_at(host, point);
        let actions = preset.act(&host.mechanics.robots()[0].layout, &mut state, aim_held());
        let intent = Intent {
            yaw,
            pitch,
            motors: actions.motors,
            triggers: actions.triggers,
            ..Intent::IDLE
        };
        input(host, tick + 1, intent);
        each(host, tick);
        host.step().unwrap();
        sent.push(intent);
    }
    sent
}

/// Above the turret, ahead and to its right, on the wall: within the hinge's reach.
const REACHABLE: [f32; 3] = [-400.0, 1_500.0, 1_500.0];
/// On the floor ahead: below the turret's horizon, past the hinge's quarter turn.
const BELOW: [f32; 3] = [0.0, 0.0, 700.0];

#[test]
fn the_preset_tracks_with_every_joint_on_the_right_button_beside_the_weapon_groups() {
    let layout = Layout::of(&fixtures::paired_turret());
    let preset = Bindings::preset(&layout);
    assert_eq!(preset.check(&layout), Ok(()));
    let tracking = preset
        .bindings
        .iter()
        .find(|binding| binding.output == Output::Track)
        .expect("a tracking binding");
    assert_eq!(tracking.source, Source::Button(AIM_BUTTON));
    assert_eq!(tracking.mode, Mode::Hold);
    let parts: Vec<PartId> = tracking.targets.iter().map(|target| target.part).collect();
    assert_eq!(parts, [ROTOR_PART, HINGE_PART]);

    // Held with the all-guns group: the joints track, the gun asks to fire, the mouse's offset
    // waits underneath.
    let mut state = BindingState::default();
    let both = Pressed {
        buttons: (1 << AIM_BUTTON) | (1 << ALL_GUNS_BUTTON),
        mouse: [-10, -8],
        ..Pressed::default()
    };
    let actions = preset.act(&layout, &mut state, both);
    assert_eq!(actions.motors.get(ROTOR_PART), Some(Command::Track));
    assert_eq!(actions.motors.get(HINGE_PART), Some(Command::Track));
    assert!(actions.motors.tracking());
    assert_eq!(actions.triggers.parts(), [BARREL]);
    // Let go: the offset the mouse left steers them again.
    let actions = preset.act(&layout, &mut state, Pressed::default());
    assert!(matches!(
        actions.motors.get(ROTOR_PART),
        Some(Command::Angle(angle)) if angle != 0
    ));
    assert!(!actions.motors.tracking());

    // Only joints track, and never from a mouse axis.
    let track = |source, mode, part| Bindings {
        bindings: vec![Binding {
            source,
            mode,
            output: Output::Track,
            invert: false,
            strength: 100,
            targets: vec![Target {
                part,
                invert: false,
            }],
        }],
        ..Bindings::default()
    };
    assert_eq!(
        track(Source::Button(2), Mode::Hold, fixtures::WHEELS[0]).check(&layout),
        Err(BindingError::NotAJoint(0, fixtures::WHEELS[0]))
    );
    assert_eq!(
        track(Source::MouseX, Mode::Offset, ROTOR_PART).check(&layout),
        Err(BindingError::Mode(0))
    );
    assert_eq!(
        track(Source::Button(2), Mode::Toggle, ROTOR_PART).check(&layout),
        Ok(())
    );
    // Pruned with the joints gone.
    let car = Layout::of(&fixtures::car());
    assert!(
        preset
            .pruned(&car)
            .bindings
            .iter()
            .all(|binding| binding.output != Output::Track)
    );
}

#[test]
fn a_two_axis_turret_tracks_a_reachable_target_and_its_shots_converge() {
    let mut host = host(fixtures::paired_turret());
    let before = off_target(&host, BARREL, REACHABLE);
    track(&mut host, REACHABLE, 150, |_, _| {});
    let after = off_target(&host, BARREL, REACHABLE);
    assert!(before > 0.5, "the barrel starts upright: {before}");
    assert!(after < 0.05, "it points at the target: {after} rad");
    let hinge = joint_angle(&host, HINGE_PART);
    assert!(hinge.abs() < std::f32::consts::FRAC_PI_2 + 0.02, "{hinge}");
    // Its shot converges on the target.
    let (yaw, pitch) = aiming_at(&host, REACHABLE);
    let me = &host.fighters[0];
    let shot = aim::aimed(
        &host.world,
        me.eye(),
        me.muzzle(Select::Part(BARREL)),
        yaw,
        pitch,
        body_of(0),
        |_| true,
        [],
    );
    assert_eq!(shot.reach, Reach::Target);
    let actual = aim::sight(
        &host.world,
        shot.from,
        shot.direction(),
        aim::RETICLE_RANGE_UNITS,
        body_of(0),
        |_| true,
        [],
    );
    assert_eq!(shot.lands(&actual), Lands::Target);
    let miss = (0..3)
        .map(|i| (actual.end[i] - REACHABLE[i]).powi(2))
        .sum::<f32>()
        .sqrt();
    assert!(miss < 15.0, "{actual:?}");
}

#[test]
fn a_turret_turns_at_its_servos_speed_not_at_once() {
    let mut host = host(fixtures::paired_turret());
    let mut hinge = Vec::new();
    track(&mut host, REACHABLE, 30, |host, _| {
        hinge.push(joint_angle(host, HINGE_PART));
    });
    // The hinge's top rate (3 rad/s): never more than a twentieth of a radian a tick, so the
    // half a radian it has to turn takes it a dozen ticks and more.
    for pair in hinge.windows(2) {
        assert!((pair[1] - pair[0]).abs() < 3.0 / 60.0 + 0.01, "{hinge:?}");
    }
    assert!(hinge[10].abs() < 0.5, "not there at once: {hinge:?}");
    assert!(hinge.last().unwrap().abs() > 0.5, "it turned: {hinge:?}");
}

#[test]
fn a_turret_refuses_an_angle_past_its_limits() {
    let mut host = host(fixtures::paired_turret());
    track(&mut host, BELOW, 150, |_, _| {});
    let hinge = joint_angle(&host, HINGE_PART);
    // Held at the hinge's quarter-turn limit, the barrel level, never past it.
    assert!(
        (hinge.abs() - std::f32::consts::FRAC_PI_2).abs() < 0.08,
        "{hinge}"
    );
    assert!(off_target(&host, BARREL, BELOW) > 0.2);
    // Its shot cannot reach the target: it flies along the barrel and the reticle says so.
    let (yaw, pitch) = aiming_at(&host, BELOW);
    let me = &host.fighters[0];
    let shot = aim::aimed(
        &host.world,
        me.eye(),
        me.muzzle(Select::Part(BARREL)),
        yaw,
        pitch,
        body_of(0),
        |_| true,
        [],
    );
    assert_eq!(shot.reach, Reach::Mount);
    let actual = aim::sight(
        &host.world,
        shot.from,
        shot.direction(),
        aim::RETICLE_RANGE_UNITS,
        body_of(0),
        |_| true,
        [],
    );
    assert!(matches!(shot.lands(&actual), Lands::Unreachable(_)));
}

#[test]
fn a_turret_without_energy_or_with_its_hinge_shot_off_aims_nothing() {
    // No energy (no electricity, and no fuel for its generator to make more, shooter#128): its
    // servos are off, the joints turn freely and the barrel never comes onto the target.
    let mut host = host(fixtures::paired_turret());
    let mut closest = f32::MAX;
    track(&mut host, REACHABLE, 90, |host, _| {
        host.fighters[0].energy = 0;
        host.fighters[0].fuel = 0;
        closest = closest.min(off_target(host, BARREL, REACHABLE));
    });
    assert!(closest > 0.3, "{closest}");
    // The turret's servos (standing legs draw nothing, shooter#128, so they may hold).
    assert!(
        host.mechanics.robots()[0]
            .actuators
            .iter()
            .filter(|actuator| actuator.part == ROTOR_PART || actuator.part == HINGE_PART)
            .all(|actuator| !actuator.powered)
    );

    // The barrel shot off: the hinge carries nothing and no longer turns; the rotor still turns
    // but carries no gun, so it holds where it is.
    let mut host = host_turret_tracking(60);
    lose(&mut host, BARREL);
    let mut rotor = Vec::new();
    track(&mut host, [1_500.0, 1_500.0, 1_500.0], 60, |host, _| {
        rotor.push(joint_angle(host, ROTOR_PART));
    });
    assert_eq!(host.fighters[0].muzzle(Select::Part(BARREL)), None);
    let actuators: Vec<PartId> = host.mechanics.robots()[0]
        .actuators
        .iter()
        .filter(|actuator| actuator.part == ROTOR_PART || actuator.part == HINGE_PART)
        .map(|actuator| actuator.part)
        .collect();
    assert_eq!(actuators, [ROTOR_PART]);
    let settled = &rotor[20..];
    let spread = settled.iter().copied().fold(f32::MIN, f32::max)
        - settled.iter().copied().fold(f32::MAX, f32::min);
    assert!(spread < 0.05, "the rotor holds: {rotor:?}");

    // The hinge shot off: the barrel goes with it and neither joint turns any more; the track
    // words for them change nothing.
    let mut host = host_turret_tracking(60);
    lose(&mut host, HINGE_PART);
    track(&mut host, REACHABLE, 30, |_, _| {});
    assert_eq!(host.fighters[0].muzzle(Select::Part(BARREL)), None);
    assert!(
        host.mechanics.robots()[0]
            .actuators
            .iter()
            .all(|actuator| actuator.part != ROTOR_PART && actuator.part != HINGE_PART)
    );
}

/// Part `part` of slot 0's robot shot off.
fn lose(host: &mut Session, part: PartId) {
    let build = host.fighters[0].build;
    let index = build
        .parts()
        .iter()
        .position(|built| built.id == part)
        .unwrap();
    host.fighters[0].parts.hurt(&build, index, 10_000).unwrap();
}

fn host_turret_tracking(ticks: u32) -> Session {
    let mut host = host(fixtures::paired_turret());
    track(&mut host, REACHABLE, ticks, |_, _| {});
    host
}

/// A tampered input cannot point a joint anywhere the host would not: the track word carries no
/// angle, and the host works the angle out from its own state.
#[test]
fn the_track_word_travels_without_an_angle() {
    let mut motors = Motors::REST;
    motors.push(ROTOR_PART, Command::Track);
    let intent = Intent {
        motors,
        ..Intent::IDLE
    };
    let bytes = encode_input(intent, 1, None, None);
    assert_eq!(decode_input(&bytes).unwrap().intent, intent);
    // A track with a value is malformed.
    let mut tampered = bytes.clone();
    let at = bytes
        .windows(4)
        .position(|window| window == [ROTOR_PART.0, 4, 0, 0])
        .expect("the track command");
    tampered[at + 2] = 9;
    assert!(decode_input(&tampered).is_err());
    // A track for a part that is no joint rests.
    let layout = Layout::of(&fixtures::paired_turret());
    let mut wheel = Motors::REST;
    wheel.push(fixtures::WHEELS[0], Command::Track);
    assert_eq!(
        wheel.controls(&layout).commands[usize::from(fixtures::WHEELS[0].0)],
        Command::Rate(0)
    );
}

/// The turret tracking on the host, predicted by a client from a snapshot, and replayed from
/// the same inputs on a fresh host: bit for bit alike.
#[test]
fn host_client_and_replay_track_alike() {
    let mut host = host(fixtures::paired_turret());
    let mut predictor = Session::new(walled()).unwrap();
    predictor.fighters[0].configured = fixtures::paired_turret();
    predictor.rebuild(0, fixtures::paired_turret()).unwrap();
    let mut joints = Vec::new();
    let point = |tick: u32| {
        if tick < 60 {
            REACHABLE
        } else {
            [800.0, 300.0, 1_500.0]
        }
    };
    let layout = Layout::of(&fixtures::paired_turret());
    let preset = Bindings::preset(&layout);
    let mut state = BindingState::default();
    let mut sent = Vec::new();
    for tick in 0..150u32 {
        let (yaw, pitch) = aiming_at(&host, point(tick));
        let pressed = if tick % 50 < 40 {
            aim_held()
        } else {
            Pressed::default()
        };
        let intent = Intent {
            yaw,
            pitch,
            motors: preset.resolve(&layout, &mut state, pressed),
            ..Intent::IDLE
        };
        input(&mut host, tick + 1, intent);
        host.step().unwrap();
        sent.push(intent);
        let snapshot = decode_snapshot(&encode_snapshot(&host.snapshot())).unwrap();
        let me = snapshot.of_player(7).unwrap().clone();
        let robot = me.robot.clone().unwrap();
        if tick == 20 {
            predictor.fighters[0].energy = me.energy;
            predictor
                .place(0, &robot, me.motion, me.held, me.hook)
                .unwrap();
        } else if tick > 20 {
            predictor.tick(intent).unwrap();
            let predicted = predictor.mechanics.state().robots[0].clone();
            assert_eq!(predicted.bodies, robot.bodies, "tick {tick}");
        }
        joints.push(
            host.mechanics.robots()[0]
                .actuators
                .iter()
                .map(|actuator| actuator.angle.to_bits())
                .collect::<Vec<u32>>(),
        );
    }
    // A replay: the same inputs on a fresh host.
    let mut replay = self::host(fixtures::paired_turret());
    let mut replayed = Vec::new();
    for (tick, intent) in sent.into_iter().enumerate() {
        input(&mut replay, u32::try_from(tick).unwrap() + 1, intent);
        replay.step().unwrap();
        replayed.push(
            replay.mechanics.robots()[0]
                .actuators
                .iter()
                .map(|actuator| actuator.angle.to_bits())
                .collect::<Vec<u32>>(),
        );
    }
    assert_eq!(joints, replayed);
}

/// The standard frame (the small chassis, its sensor and legs) with the energy rifle (ID 7) on
/// chassis `face` at its first cell: a fixed gun pointing out of that face.
fn rifle_on(face: Face) -> Assembly {
    let raw = |id: u8, kind: &PartType, mount: Option<(&PartType, Face, u8)>| RawPart {
        id,
        kind: kind.id.0,
        mount: mount.map(|(on, face, v)| RawMount {
            parent: 0,
            socket: (0..4)
                .flat_map(|u| [(u, v), (v, u)])
                .find_map(|(u, v)| on.socket(face, u, v))
                .expect("a real socket")
                .0,
            ..RawMount::default()
        }),
        modification: None,
    };
    let chassis = &kit::CHASSIS_S;
    BETA.validate(&Candidate {
        parts: vec![
            raw(0, chassis, None),
            raw(2, &kit::SENSOR, Some((chassis, Face::PosZ, 0))),
            raw(5, &kit::LEG, Some((chassis, Face::NegZ, 0))),
            RawPart {
                mount: Some(RawMount {
                    parent: 0,
                    socket: chassis.socket(Face::NegZ, 1, 0).unwrap().0,
                    ..RawMount::default()
                }),
                ..raw(6, &kit::LEG, None)
            },
            raw(7, &kit::ENERGY_RIFLE, Some((chassis, face, 1))),
        ],
    })
    .expect("a valid build")
}

/// One host tick with slot 0 aiming at `point` and firing the rifle (part 7); its `Fired` event
/// and its beam's end.
fn fire_at(host: &mut Session, sequence: u32, point: [f32; 3]) -> Option<([f32; 3], [f32; 3])> {
    let (yaw, pitch) = aiming_at(host, point);
    input(
        host,
        sequence,
        Intent {
            yaw,
            pitch,
            fire: true,
            select: Some(Select::Part(PartId(7))),
            ..Intent::IDLE
        },
    );
    host.step().unwrap();
    let direction = host.events.iter().find_map(|event| match *event {
        shooter_sim::Event::Fired {
            weapon: Weapon::EnergyRifle,
            direction,
            ..
        } => Some(direction),
        _ => None,
    })?;
    let end = host.events.iter().find_map(|event| match *event {
        shooter_sim::Event::Beam { end, .. } => Some(end),
        _ => None,
    })?;
    Some((direction, end))
}

fn settle_facing(host: &mut Session, point: [f32; 3]) -> u32 {
    let mut sequence = 0;
    for _ in 0..40 {
        sequence += 1;
        let (yaw, pitch) = aiming_at(host, point);
        input(
            host,
            sequence,
            Intent {
                yaw,
                pitch,
                ..Intent::IDLE
            },
        );
        host.step().unwrap();
    }
    sequence
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[test]
fn a_fixed_gun_converges_only_within_its_cone_about_its_mount() {
    // Ahead on the wall, near the muzzle's height: within the cone, the bolt meets the target.
    let ahead = [0.0, 120.0, 1_500.0];
    let mut host = host(STANDARD);
    let sequence = settle_facing(&mut host, ahead);
    let axis = host.fighters[0]
        .muzzle(Select::Part(PartId(7)))
        .unwrap()
        .axis
        .unwrap();
    let (direction, end) = fire_at(&mut host, sequence + 1, ahead).expect("it fired");
    assert!(dot(direction, axis) >= CONVERGENCE_COS);
    let miss = (0..3)
        .map(|i| (end[i] - ahead[i]).powi(2))
        .sum::<f32>()
        .sqrt();
    assert!(miss < 5.0, "{end:?}");

    // High up the wall, 30° above the mount: the bolt flies along the mount instead, and lands
    // on the wall level with the muzzle, not on the reticle.
    let high = [0.0, 900.0, 1_500.0];
    let mut host = self::host(STANDARD);
    let sequence = settle_facing(&mut host, high);
    let muzzle = host.fighters[0].muzzle(Select::Part(PartId(7))).unwrap();
    let axis = muzzle.axis.unwrap();
    let (direction, end) = fire_at(&mut host, sequence + 1, high).expect("it fired");
    assert!(dot(direction, axis) > 0.9999, "{direction:?} {axis:?}");
    assert!((end[1] - muzzle.at[1]).abs() < 20.0, "{end:?} {muzzle:?}");
    assert!((end[2] - 1_500.0).abs() < 1.0, "{end:?}");
}

#[test]
fn a_sideways_gun_fires_only_out_of_its_side() {
    let build = rifle_on(Face::PosY);
    let ahead = [0.0, 120.0, 1_500.0];
    let mut host = host(build);
    let sequence = settle_facing(&mut host, ahead);
    let axis = host.fighters[0]
        .muzzle(Select::Part(PartId(7)))
        .unwrap()
        .axis
        .unwrap();
    // The robot faces the aim (+Z); the rifle points out of its side, square to it.
    assert!(axis[2].abs() < 0.1 && axis[0].abs() > 0.9, "{axis:?}");
    let (direction, _) = fire_at(&mut host, sequence + 1, ahead).expect("it fired");
    assert!(dot(direction, axis) > 0.9999, "{direction:?}");
    assert!(
        direction[2].abs() < 0.1,
        "not toward the reticle: {direction:?}"
    );
}

#[test]
fn an_occluded_gun_is_stopped_by_the_cover_in_front_of_its_muzzle() {
    // A post a metre ahead, right in front of the rifle's muzzle and below the camera's line.
    let mut arena = walled();
    let ahead = [0.0, 120.0, 1_500.0];
    let mut host = host(STANDARD);
    let sequence = settle_facing(&mut host, ahead);
    let muzzle = host.fighters[0].muzzle(Select::Part(PartId(7))).unwrap();
    let post = Vec3i::new(
        muzzle.at[0].round() as i32,
        muzzle.at[1].round() as i32,
        100,
    );
    arena.pieces.push(Piece::Block {
        center: post,
        half: Vec3i::new(15, 15, 15),
    });
    let mut covered = Session::with_rules(arena, rules()).unwrap();
    covered.join(7).unwrap();
    assert!(covered.configure(7, STANDARD));
    let sequence = settle_facing(&mut covered, ahead).max(sequence);
    let (yaw, pitch) = aiming_at(&covered, ahead);
    let me = &covered.fighters[0];
    let shot = aim::aimed(
        &covered.world,
        me.eye(),
        me.muzzle(Select::Part(PartId(7))),
        yaw,
        pitch,
        body_of(0),
        |_| true,
        [],
    );
    let actual = aim::sight(
        &covered.world,
        shot.from,
        shot.direction(),
        aim::RETICLE_RANGE_UNITS,
        body_of(0),
        |_| true,
        [],
    );
    let Lands::Blocked(at) = shot.lands(&actual) else {
        panic!("blocked: {:?}", shot.lands(&actual));
    };
    let (_, end) = fire_at(&mut covered, sequence + 1, ahead).expect("it fired");
    assert!(end[2] < 100.0 && at[2] < 100.0, "{end:?} {at:?}");
}
