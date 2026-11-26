//! The match's fighters as mechanical robots (shooter#73, Atlas D2.i70.5): each slot's robot in
//! the articulated world keeps its slot through open slots, deaths, respawns and fresh rounds;
//! its exact state travels in snapshots, and a client predicting from one, or a replay, continues
//! bit for bit like the host, whole and after losing a leg; strikes meet its parts where its
//! bodies carry them; and a part shot off leaves its bodies, mass and motors.
// The platform's floats only check results here; the simulation uses libm (clippy.toml).
#![allow(clippy::disallowed_methods)]

use shooter_sim::mechanics::fixtures;
use shooter_sim::mechanics::{BODIES_PER_ROBOT, FIRST_ROBOT_BODY, Layout};
use shooter_sim::physics::Vec3i;
use shooter_sim::robot::STANDARD;
use shooter_sim::robot::parts::LEG;
use shooter_sim::session::LOCAL_PLAYER;
use shooter_sim::wire::{decode_snapshot, encode_snapshot};
use shooter_sim::{Arena, Event, Input, Intent, JumpPad, Rules, Session, Spawn};

/// An open floor with two spawn points 12 m apart, no pads.
fn floor() -> Arena {
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

fn people(slots: u8) -> Rules {
    Rules {
        slots,
        bots: false,
        local: false,
        seed: 0,
        frag_limit: 0,
        time_limit_ticks: 0,
        intermission_ticks: 60,
        respawn_ticks: 30,
    }
}

/// Every living fighter has its robot in its own slot, and only they do.
fn robots_follow_slots(session: &Session) {
    let slots: Vec<u8> = session
        .mechanics
        .robots()
        .iter()
        .map(|robot| robot.slot)
        .collect();
    let alive: Vec<u8> = session
        .fighters
        .iter()
        .filter(|fighter| fighter.alive())
        .map(|fighter| fighter.slot)
        .collect();
    assert_eq!(slots, alive);
    for robot in session.mechanics.robots() {
        // Its bodies' ids follow from its slot.
        assert_eq!(
            robot.body(0).0,
            FIRST_ROBOT_BODY + u64::from(robot.slot) * BODIES_PER_ROBOT
        );
        assert_eq!(robot.build, session.fighters[usize::from(robot.slot)].build);
    }
}

fn input(session: &mut Session, id: u32, sequence: u32, intent: Intent) {
    let view_tick = session.ticks();
    session.queue(
        id,
        Input {
            sequence,
            intent,
            view_tick,
        },
    );
}

#[test]
fn wheeled_and_tracked_robots_do_not_jump_or_turn_toward_the_camera() {
    // Even with a working wheel/track on the floor, Space is not a jump actuator and changing
    // the aim does not give a chassis without legs a free turning motor.
    for build in [fixtures::car(), fixtures::tank(), fixtures::turret()] {
        let mut session = Session::with_rules(floor(), Rules::solo()).unwrap();
        assert!(session.configure(LOCAL_PLAYER, build));
        assert_eq!(session.fighters[0].build, build);
        for _ in 0..12 {
            let report = session
                .tick(Intent {
                    yaw: 16_384,
                    jump: true,
                    ..Intent::IDLE
                })
                .unwrap();
            assert!(!report.jumped && !report.air_jumped, "{build:?}");
            let spin = session.mechanics.root_motion(0).unwrap().angular_velocity;
            assert!(spin[1].abs() < 0.1, "{build:?}: {spin:?}");
        }
    }
}

#[test]
fn a_robot_with_connected_legs_can_still_jump_and_face_its_aim() {
    let mut session = Session::with_rules(floor(), Rules::solo()).unwrap();
    let report = session
        .tick(Intent {
            yaw: 16_384,
            jump: true,
            ..Intent::IDLE
        })
        .unwrap();
    assert!(report.jumped);
    let spin = session.mechanics.root_motion(0).unwrap().angular_velocity;
    assert!(spin[1].abs() > 0.1, "{spin:?}");
}

#[test]
fn a_wheeled_robot_can_still_be_launched_by_an_arena_jump_pad() {
    let mut arena = floor();
    arena.pads.push(JumpPad {
        center: Vec3i::new(0, 0, -600),
        half: [100, 100],
        launch: Vec3i::new(0, 1_200, 0),
    });
    let mut session = Session::with_rules(arena, Rules::solo()).unwrap();
    assert!(session.configure(LOCAL_PLAYER, fixtures::car()));
    let launched = (0..30).any(|_| session.tick(Intent::IDLE).unwrap().launched);
    assert!(
        launched,
        "arena jump pads remain independent of built-in jumping"
    );
}

#[test]
fn each_slot_keeps_its_robot_through_open_slots_deaths_respawns_and_rounds() {
    let mut session = Session::with_rules(
        floor(),
        Rules {
            frag_limit: 1,
            ..people(4)
        },
    )
    .unwrap();
    robots_follow_slots(&session);
    assert!(
        session.mechanics.robots().is_empty(),
        "open slots have none"
    );
    // People take slots 0 and 2 (slot 1 opens when its person leaves).
    for id in [1, 2, 3] {
        session.join(id).unwrap();
    }
    assert!(session.leave(2).unwrap());
    robots_follow_slots(&session);
    assert_eq!(session.mechanics.robots().len(), 2);
    // Slot 2 is shot with the rifle until it dies.
    let mut sequence = 0;
    let mut died = false;
    for _ in 0..2_000 {
        sequence += 1;
        let target = session.fighters[2].hitboxes();
        let aim = target.get(0).map_or([0.0; 3], |part| part.center);
        let eye = session.fighters[0].eye();
        let d = [0, 1, 2].map(|axis| aim[axis] - eye[axis]);
        #[allow(clippy::cast_possible_truncation)]
        let yaw = shooter_sim::trig::yaw_from_vector(d[0].round() as i32, d[2].round() as i32)
            .unwrap_or(0);
        #[allow(clippy::cast_possible_truncation)]
        let pitch = shooter_sim::trig::yaw_from_vector(
            d[1].round() as i32,
            d[0].hypot(d[2]).round() as i32,
        )
        .unwrap_or(0) as i16;
        input(
            &mut session,
            1,
            sequence,
            Intent {
                yaw,
                pitch,
                fire: true,
                select: Some(shooter_sim::Weapon::EnergyRifle.into()),
                ..Intent::IDLE
            },
        );
        input(&mut session, 3, sequence, Intent::IDLE);
        session.step().unwrap();
        robots_follow_slots(&session);
        if session
            .events
            .iter()
            .any(|event| matches!(event, Event::Killed { victim: 2, .. }))
        {
            died = true;
            // The frag limit ends the round; the robot left the world at once.
            assert!(session.mechanics.robot(2).is_none());
            break;
        }
    }
    assert!(died, "slot 2 died");
    // Through the intermission and into a fresh round: everyone back in their own slot.
    for _ in 0..200 {
        sequence += 1;
        input(&mut session, 1, sequence, Intent::IDLE);
        input(&mut session, 3, sequence, Intent::IDLE);
        session.step().unwrap();
        robots_follow_slots(&session);
    }
    assert_eq!(session.mechanics.robots().len(), 2);
}

/// The standard robot's first leg's index in its build.
fn leg_index() -> usize {
    STANDARD
        .parts()
        .iter()
        .position(|part| part.kind == LEG.id)
        .unwrap()
}

/// A person's robot on a host, a client predicting it from the host's snapshot from tick 30 on,
/// both given the same inputs: the prediction follows the host bit for bit, walking, sprinting,
/// sliding, jumping and turning, whole or after losing a leg.
fn predicted_like_the_host(lose_leg: bool) {
    let mut host = Session::with_rules(floor(), people(1)).unwrap();
    host.join(7).unwrap();
    let mut predictor = Session::new(floor()).unwrap();
    let intents = [
        Intent::IDLE,
        Intent {
            forward: 1,
            yaw: 8_000,
            ..Intent::IDLE
        },
        Intent {
            forward: 1,
            sprint: true,
            yaw: 16_000,
            ..Intent::IDLE
        },
        Intent {
            forward: 1,
            sprint: true,
            crouch: true,
            yaw: 16_000,
            ..Intent::IDLE
        },
        Intent {
            strafe: -1,
            jump: true,
            yaw: 30_000,
            ..Intent::IDLE
        },
    ];
    let mut sequence = 0;
    for tick in 0..400u32 {
        let intent = intents[(tick / 40) as usize % intents.len()];
        sequence += 1;
        input(&mut host, 7, sequence, intent);
        if tick == 20 && lose_leg {
            let build = host.fighters[0].build;
            host.fighters[0].parts.hurt(&build, leg_index(), 10_000);
        }
        host.step().unwrap();
        let snapshot = decode_snapshot(&encode_snapshot(&host.snapshot())).unwrap();
        let me = snapshot.of_player(7).unwrap().clone();
        let robot = me.robot.clone().unwrap();
        if tick == 30 {
            predictor.fighters[0].parts = me.parts;
            predictor.fighters[0].energy = me.energy;
            predictor
                .place(0, &robot, me.motion, me.held, me.hook)
                .unwrap();
            if lose_leg {
                assert_ne!(robot.lost, 0, "the leg came off the host's robot");
                assert!(
                    robot.bodies.len() < Layout::of(&STANDARD).bodies.len(),
                    "and its bodies"
                );
            }
        } else if tick > 30 {
            predictor.tick(intent).unwrap();
            let predicted = predictor.mechanics.state().robots[0].clone();
            assert_eq!(predicted.bodies, robot.bodies, "tick {tick}");
            assert_eq!(predicted.lost, robot.lost);
            assert_eq!(predictor.local().motion, me.motion, "tick {tick}");
            assert_eq!(predictor.local().energy, me.energy, "tick {tick}");
        }
    }
}

#[test]
fn a_client_predicts_the_hosts_robot_exactly() {
    predicted_like_the_host(false);
}

#[test]
fn a_client_predicts_a_one_legged_robot_exactly() {
    predicted_like_the_host(true);
}

/// A replay of a match with bots (inputs and seed alone) gives the same snapshots, robot state
/// and lost parts included.
#[test]
fn bot_matches_with_robots_replay_exactly() {
    let play = || {
        let mut session = Session::with_rules(Arena::greybox(), Rules::deathmatch(5)).unwrap();
        let mut bytes = Vec::new();
        let mut lost = 0;
        for _ in 0..20 * 60 {
            session.step().unwrap();
            lost += session
                .events
                .iter()
                .filter(|event| matches!(event, Event::Detached { .. }))
                .count();
            bytes.extend(encode_snapshot(&session.snapshot()));
        }
        assert!(lost > 0, "bots shot parts off");
        bytes
    };
    assert_eq!(play(), play());
}

/// A leg shot off leaves the robot's bodies: what is left weighs less, has fewer motors (and
/// draws less), and the robot's remaining parts keep their places on its frame.
#[test]
fn a_part_shot_off_leaves_the_bodies_mass_and_motors() {
    let mut session = Session::with_rules(floor(), people(1)).unwrap();
    session.join(1).unwrap();
    for _ in 0..30 {
        session.step().unwrap();
    }
    let whole = Layout::of(&STANDARD);
    let mass = |layout: &Layout| -> f32 {
        layout
            .bodies
            .iter()
            .flat_map(|body| &body.children)
            .map(|child| child.mass)
            .sum()
    };
    let before = session.fighters[0].hitboxes();
    let chassis = before.get(0).unwrap().center;
    let build = session.fighters[0].build;
    session.fighters[0].parts.hurt(&build, leg_index(), 10_000);
    session.step().unwrap();
    let robot = session.mechanics.robot(0).unwrap();
    assert_eq!(robot.lost, session.fighters[0].parts.lost());
    assert!(robot.layout.bodies.len() < whole.bodies.len());
    assert!(robot.layout.actuators.len() < whole.actuators.len());
    assert!(mass(&robot.layout) < mass(&whole));
    assert!(
        robot
            .layout
            .actuators
            .iter()
            .all(|actuator| actuator.part != STANDARD.parts()[leg_index()].id)
    );
    // The leg has no box any more; the chassis is where it was, give or take the tick.
    let after = session.fighters[0].hitboxes();
    assert!(after.get(leg_index()).is_none());
    let moved = after.get(0).unwrap().center;
    assert!(
        (0..3).all(|axis| (moved[axis] - chassis[axis]).abs() < 5.0),
        "{chassis:?} {moved:?}"
    );
    assert_eq!(
        session.fighters[0].traction(),
        shooter_sim::player::Traction {
            connected: 1,
            mounted: 2
        }
    );
}

/// [`floor`] with a target dummy standing 4 m ahead of the first spawn point, in its way.
fn floor_with_dummy() -> Arena {
    Arena {
        posts: vec![shooter_sim::Post {
            feet: Vec3i::new(0, 0, -200),
            patrol_units: 0,
        }],
        ..floor()
    }
}

/// How far (units, along +Z) `build` gets from the first spawn point in `ticks` of W, and the
/// session after.
fn run_into(arena: Arena, build: shooter_sim::Assembly, ticks: u32) -> (f32, Session) {
    let mut session = Session::with_rules(arena, Rules::solo()).unwrap();
    assert!(session.configure(LOCAL_PLAYER, build));
    for _ in 0..ticks {
        let intent = session.with_preset(
            0,
            Intent {
                forward: 1,
                ..Intent::IDLE
            },
        );
        session.tick(intent).unwrap();
    }
    (session.local().center()[2], session)
}

/// Target dummies stand in the robots' world (shooter#73): a robot walking on its legs (the
/// movement drive) or rolling on its wheels into one stops against it, where on open floor it
/// carries on past the post; the dummy still takes the robot's shots.
#[test]
fn robots_stop_against_target_dummies() {
    for (build, ticks) in [(STANDARD, 120), (fixtures::car(), 240)] {
        let (open, _) = run_into(floor(), build, ticks);
        let (blocked, mut session) = run_into(floor_with_dummy(), build, ticks);
        println!("open {open} blocked {blocked}");
        assert!(open > 100.0, "it passes the post on open floor: {open}");
        // The dummy's near side is 30 units before its post; the robot's centre stays short of it.
        assert!(blocked < -230.0, "it stops against the dummy: {blocked}");
        assert!(blocked > -600.0, "it moved toward it: {blocked}");
        // The robot does not move it: it stands where it stood, in both worlds (so the robots'
        // world is not rebuilt for it tick after tick).
        let dummy = session.dummies[0].body;
        let standing = Vec3i::new(0, 90, -200);
        assert_eq!(session.world.body(dummy).unwrap().position(), standing);
        let boxed = session
            .mechanics
            .statics()
            .iter()
            .find(|body| body.id() == dummy);
        assert_eq!(boxed.unwrap().position(), standing);
        if build == STANDARD {
            let mut hit = false;
            for _ in 0..60 {
                let report = session
                    .tick(Intent {
                        fire: true,
                        ..Intent::IDLE
                    })
                    .unwrap();
                hit |= report.damage > 0;
            }
            assert!(hit, "its shots still hit the dummy");
            assert!(session.dummies[0].health < shooter_sim::dummies::DUMMY_HEALTH);
        }
    }
}

/// A dummy that goes down leaves the robots' world, so a robot held up by it walks on; it comes
/// back into both worlds when it stands again. A replay of the same inputs goes alike, and a
/// client predicting the robot from a snapshot follows the host bit for bit as it walks into the
/// dummy and on past it once it is down.
#[test]
fn a_downed_dummy_lets_robots_through_alike_on_host_client_and_replay() {
    let dummy = shooter_sim::physics::BodyId(shooter_sim::arena::FIRST_DUMMY_BODY);
    let walls = |session: &Session| {
        session
            .mechanics
            .statics()
            .iter()
            .any(|body| body.id() == dummy)
    };
    let play = || {
        let mut host = Session::with_rules(floor_with_dummy(), people(1)).unwrap();
        host.join(7).unwrap();
        let mut predictor = Session::with_rules(floor_with_dummy(), Rules::solo()).unwrap();
        let walk = Intent {
            forward: 1,
            ..Intent::IDLE
        };
        let mut trace = Vec::new();
        for tick in 0..240u32 {
            input(&mut host, 7, tick + 1, walk);
            if tick == 120 {
                assert!(walls(&host), "the standing dummy is in the robots' world");
                let at = host.ticks();
                assert!(host.dummies[0].hurt(&mut host.world, 10_000, at));
                // The predictor learns of it as a client would, from the host's events.
                let at = predictor.ticks();
                assert!(predictor.dummies[0].hurt(&mut predictor.world, 10_000, at));
            }
            host.step().unwrap();
            if tick == 121 {
                assert!(!walls(&host), "the downed dummy left the robots' world");
            }
            let snapshot = decode_snapshot(&encode_snapshot(&host.snapshot())).unwrap();
            let me = snapshot.of_player(7).unwrap().clone();
            let robot = me.robot.clone().unwrap();
            if tick == 30 {
                predictor.fighters[0].parts = me.parts;
                predictor.fighters[0].energy = me.energy;
                predictor
                    .place(0, &robot, me.motion, me.held, me.hook)
                    .unwrap();
            } else if tick > 30 {
                predictor.tick(walk).unwrap();
                let predicted = predictor.mechanics.state().robots[0].clone();
                assert_eq!(predicted.bodies, robot.bodies, "tick {tick}");
            }
            trace.push(host.local().center());
        }
        trace
    };
    let trace = play();
    // Held up at the dummy, then on past its post once it went down.
    assert!(trace[119][2] < -230.0, "{:?}", trace[119]);
    assert!(trace[239][2] > 0.0, "{:?}", trace[239]);
    assert_eq!(trace, play(), "a replay goes alike");
}
