//! The grapple module (Atlas D2.i104.1, D2.i104.2, shooter#104): an optional socket-mounted
//! robot part on its own key, never a gun in the rotation. A robot without a working module has
//! no grapple; the hook leaves the module's muzzle; it pulls toward its anchor and swings on a
//! rope that never stretches; letting go, losing the module or dying ends it; each hook costs
//! the module's energy from the shared pool and waits its interval.

use shooter_sim::grapple::{MIN_LENGTH_UNITS, RANGE_UNITS};
use shooter_sim::physics::{SUBUNITS_PER_UNIT, Vec3i};
use shooter_sim::robot::{BETA, STANDARD};
use shooter_sim::{Arena, Event, Intent, Piece, Session, Weapon, scenarios};

/// The grapple scenario's hook: facing the pillar on the ledge, the grapple key held.
fn hook_intent() -> Intent {
    let leg = scenarios::GRAPPLE
        .legs
        .iter()
        .find(|leg| leg.name == "hook and reel in")
        .unwrap();
    assert!(leg.intent.grapple && !leg.intent.fire);
    leg.intent
}

/// The grapple scenario's arena with the local robot settled on the floor as `build`, its pool
/// full.
fn settled(build: shooter_sim::robot::Assembly) -> Session {
    let mut session = Session::new((scenarios::GRAPPLE.arena)()).unwrap();
    session.rebuild(0, build).unwrap();
    let aim = Intent {
        grapple: false,
        ..hook_intent()
    };
    for _ in 0..20 {
        session.tick(aim).unwrap();
    }
    let fighter = &mut session.fighters[0];
    fighter.energy = fighter.pool().capacity;
    session
}

/// The standard build without its grapple module (part 9): still a valid robot.
fn standard_without_module() -> shooter_sim::robot::Assembly {
    let mut candidate = STANDARD.candidate();
    let before = candidate.parts.len();
    candidate.parts.retain(|part| part.id != 9);
    assert_eq!(candidate.parts.len(), before - 1);
    BETA.validate(&candidate).unwrap()
}

fn module_index(session: &Session) -> usize {
    let build = session.fighters[0].build;
    build
        .parts()
        .iter()
        .position(|part| BETA.weapon(part) == Some(Weapon::Grapple))
        .unwrap()
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3)
        .map(|axis| (a[axis] - b[axis]) * (a[axis] - b[axis]))
        .sum::<f32>()
        .sqrt()
}

/// Rope length in units.
#[allow(clippy::cast_precision_loss)]
fn units(subunits: i32) -> f32 {
    subunits as f32 / SUBUNITS_PER_UNIT as f32
}

#[test]
fn a_robot_without_the_module_has_no_grapple() {
    let mut session = settled(standard_without_module());
    let charge = session.fighters[0].energy;
    for _ in 0..90 {
        let report = session.tick(hook_intent()).unwrap();
        assert_ne!(
            report.fired.map(|fired| fired.weapon),
            Some(Weapon::Grapple)
        );
        assert!(session.local().hook.is_none());
        assert!(!session.events.iter().any(|event| matches!(
            event,
            Event::Fired {
                weapon: Weapon::Grapple,
                ..
            }
        )));
    }
    // Nothing spent: the pool stays full.
    assert_eq!(session.fighters[0].energy, charge);
    // And no select key makes it a held weapon.
    session
        .tick(Intent {
            select: Some(Weapon::Grapple.into()),
            ..Intent::IDLE
        })
        .unwrap();
    assert_eq!(session.local().arsenal.held.weapon, Weapon::EnergyRifle);
}

#[test]
fn the_hook_leaves_the_module_muzzle_on_its_own_key_whatever_gun_is_held() {
    let mut session = settled(STANDARD);
    // The trigger alone never hooks (the rifle fires), and the grapple is never held.
    let report = session
        .tick(Intent {
            fire: true,
            ..hook_intent()
        })
        .unwrap();
    assert_eq!(session.local().arsenal.held.weapon, Weapon::EnergyRifle);
    // The rifle and the hook both went out this tick: separate keys, separate cooldowns.
    let fired: Vec<(Weapon, [f32; 3])> = session
        .events
        .iter()
        .filter_map(|event| match event {
            Event::Fired { weapon, from, .. } => Some((*weapon, *from)),
            _ => None,
        })
        .collect();
    assert!(
        fired
            .iter()
            .any(|(weapon, _)| *weapon == Weapon::EnergyRifle)
    );
    assert_eq!(
        report.fired.map(|fired| fired.weapon),
        Some(Weapon::EnergyRifle)
    );
    let hook = session.local().hook.expect("the hook set");

    // From its own muzzle, not the eye: where the module's anchor sits on the robot.
    let mut session = settled(STANDARD);
    let muzzle = session.local().muzzle(Weapon::Grapple).unwrap();
    let eye = session.local().eye();
    let report = session.tick(hook_intent()).unwrap();
    assert_eq!(
        report.fired.map(|fired| fired.weapon),
        Some(Weapon::Grapple)
    );
    let from = session
        .events
        .iter()
        .find_map(|event| match event {
            Event::Fired {
                weapon: Weapon::Grapple,
                from,
                ..
            } => Some(*from),
            _ => None,
        })
        .unwrap();
    assert!(distance(from, muzzle.at) < 0.01, "{from:?} {muzzle:?}");
    assert!(distance(from, eye) > 20.0, "{from:?} {eye:?}");
    let line = session.events.iter().find_map(|event| match event {
        Event::Beam {
            weapon: Weapon::Grapple,
            from,
            struck,
            ..
        } => Some((*from, *struck)),
        _ => None,
    });
    assert_eq!(line, Some((from, true)));
    // It set on the pillar's face (arena geometry), within reach, with the rope tied at the
    // eye.
    let set = session.local().hook.unwrap();
    assert_eq!(set.anchor, hook.anchor);
    let anchor = set.anchor_units();
    assert!((anchor[2] - 650.0).abs() < 1.0, "{anchor:?}");
    assert!(distance(anchor, from) < RANGE_UNITS as f32);
    assert!((units(set.length) - distance(anchor, eye)).abs() < 25.0);

    // Turned a quarter round, the mount turns with the robot: the hook leaves it still.
    let mut session = settled(STANDARD);
    let turned = Intent {
        yaw: 16_384,
        ..Intent::IDLE
    };
    for _ in 0..60 {
        session.tick(turned).unwrap();
    }
    let muzzle = session.local().muzzle(Weapon::Grapple).unwrap();
    assert!(muzzle.at[0] - session.local().eye()[0] > 20.0, "{muzzle:?}");
    session
        .tick(Intent {
            grapple: true,
            ..turned
        })
        .unwrap();
    let from = session.events.iter().find_map(|event| match event {
        Event::Fired {
            weapon: Weapon::Grapple,
            from,
            ..
        } => Some(*from),
        _ => None,
    });
    assert!(
        distance(from.unwrap(), muzzle.at) < 0.01,
        "{from:?} {muzzle:?}"
    );
}

#[test]
fn the_hook_pulls_toward_its_anchor_and_the_rope_only_shortens() {
    let mut session = settled(STANDARD);
    let start = session.local().center();
    session.tick(hook_intent()).unwrap();
    let anchor = session.local().hook.unwrap().anchor_units();
    let mut length = session.local().hook.unwrap().length;
    let first = distance(session.local().eye(), anchor);
    for _ in 0..60 {
        session.tick(hook_intent()).unwrap();
        let hook = session.local().hook.expect("held while the key is");
        assert_eq!(hook.anchor_units(), anchor);
        assert!(hook.length <= length, "the rope never pays out");
        length = hook.length;
    }
    let now = distance(session.local().eye(), anchor);
    assert!(now < first - 500.0, "pulled in: {first} -> {now}");
    assert!(session.local().center()[1] > start[1] + 50.0, "lifted");
    assert!(units(length) >= MIN_LENGTH_UNITS as f32);
}

/// A ceiling slab 12 m up and 8 m ahead: hooked under it from the floor, the robot is reeled up
/// and its momentum carries it past under the anchor, where the rope swings it back.
fn under_a_ceiling() -> Arena {
    Arena {
        half_size: 3_000,
        pieces: vec![Piece::Block {
            center: Vec3i::new(0, 1_220, 800),
            half: Vec3i::new(800, 20, 400),
        }],
        pads: Vec::new(),
        posts: Vec::new(),
        spawn: Vec3i::new(0, 0, -600),
        spawn_yaw: 0,
        spawns: Vec::new(),
        module_pads: Vec::new(),
        waypoints: Vec::new(),
        placements: Vec::new(),
        repair_anchors: Vec::new(),
    }
}

#[test]
fn the_rope_swings_the_robot_round_its_anchor_without_stretching() {
    let mut session = Session::new(under_a_ceiling()).unwrap();
    for _ in 0..20 {
        session.tick(Intent::IDLE).unwrap();
    }
    // Up at the ceiling's near edge, about 41° above level.
    let swing = Intent {
        pitch: 7_500,
        grapple: true,
        ..Intent::IDLE
    };
    session.tick(swing).unwrap();
    let hook = session.local().hook.expect("hooked the ceiling");
    let anchor = hook.anchor_units();
    assert!((anchor[1] - 1_200.0).abs() < 1.0, "{anchor:?}");
    let mut beyond = false;
    let mut back = false;
    let mut track = Vec::new();
    for _ in 0..240 {
        session.tick(swing).unwrap();
        let fighter = session.local();
        let hook = fighter.hook.expect("held");
        // The rope holds the eye within its length (the give of a tick's gravity and swing).
        let span = distance(fighter.eye(), anchor);
        assert!(
            span <= units(hook.length) + 15.0,
            "{span} > {}",
            units(hook.length)
        );
        let velocity = fighter.velocity();
        track.push((fighter.eye(), velocity));
        // Past under the anchor, still moving on, then drawn back the other way: a pendulum.
        beyond |= fighter.eye()[2] > anchor[2] + 50.0 && velocity[2] > 0.0;
        back |= beyond && velocity[2] < -50.0;
    }
    assert!(beyond && back, "{track:?}");
}

#[test]
fn letting_go_releases_the_rope_and_keeps_the_momentum() {
    let mut session = settled(STANDARD);
    for _ in 0..50 {
        session.tick(hook_intent()).unwrap();
    }
    assert!(session.local().hook.is_some());
    let moving = session.local().velocity();
    let let_go = Intent {
        grapple: false,
        ..hook_intent()
    };
    session.tick(let_go).unwrap();
    assert!(session.local().hook.is_none());
    let after = session.local().velocity();
    // Still flying up the face: no more than the reel's and a tick of gravity's pull lost.
    assert!(
        after[1] > 0.0 && after[1] > moving[1] - 40.0,
        "{moving:?} -> {after:?}"
    );
    // Its interval passed while it held: pressed again, it hooks again at once.
    session.tick(hook_intent()).unwrap();
    assert!(session.local().hook.is_some());
}

#[test]
fn losing_the_module_mid_pull_ends_the_grapple() {
    let mut session = settled(STANDARD);
    for _ in 0..10 {
        session.tick(hook_intent()).unwrap();
    }
    assert!(session.local().hook.is_some());
    let index = module_index(&session);
    let fighter = &mut session.fighters[0];
    let build = fighter.build;
    fighter.parts.hurt(&build, index, 1_000);
    assert!(!fighter.parts.armed(&build, Weapon::Grapple));
    let charge = fighter.energy;
    session.tick(hook_intent()).unwrap();
    assert!(
        session.local().hook.is_none(),
        "the rope goes with the module"
    );
    // Held on, it never hooks again this life and spends nothing.
    for _ in 0..90 {
        let report = session.tick(hook_intent()).unwrap();
        assert!(session.local().hook.is_none());
        assert_ne!(
            report.fired.map(|fired| fired.weapon),
            Some(Weapon::Grapple)
        );
    }
    assert!(session.fighters[0].energy >= charge);
}

#[test]
fn each_hook_pays_its_energy_from_the_shared_pool_and_waits_its_interval() {
    let cost = Weapon::Grapple.energy();
    // Too little charge: refused, nothing spent and no cooldown.
    let mut session = settled(STANDARD);
    session.fighters[0].energy = cost - 1;
    session.tick(hook_intent()).unwrap();
    assert!(session.local().hook.is_none());
    assert!(session.fighters[0].energy >= cost - 1);
    assert_eq!(session.local().arsenal.grapple_cooldown(session.ticks()), 0);

    // Enough: the hook takes its cost (less a tick's regeneration).
    let mut session = settled(STANDARD);
    let full = session.fighters[0].energy;
    session.tick(hook_intent()).unwrap();
    assert!(session.local().hook.is_some());
    let spent = full - session.fighters[0].energy;
    assert!(
        (cost - 1_000..=cost).contains(&spent),
        "spent {spent} of {full}"
    );

    // Let go and pressed again at once: refused until its interval has passed.
    let let_go = Intent {
        grapple: false,
        ..hook_intent()
    };
    session.tick(let_go).unwrap();
    let mut rehooked = None;
    for tick in 0..Weapon::Grapple.refire_ticks() {
        session.tick(hook_intent()).unwrap();
        if session.local().hook.is_some() {
            rehooked = Some(tick);
            break;
        }
    }
    let waited = rehooked.expect("hooks again once ready");
    assert!(waited + 2 >= Weapon::Grapple.refire_ticks() - 1, "{waited}");
}

#[test]
fn bots_never_grapple() {
    let mut session =
        Session::with_rules(Arena::greybox(), shooter_sim::Rules::deathmatch(7)).unwrap();
    for _ in 0..600 {
        session.step().unwrap();
        assert!(
            session
                .fighters
                .iter()
                .all(|fighter| fighter.hook.is_none())
        );
    }
}

/// The scripted people of the replays (`shooter-replay`) hold the grapple key now and then: a
/// recorded match logs the module's activations, and replaying it checks them tick by tick.
#[test]
fn the_scripted_people_grapple() {
    let mut session =
        Session::with_rules(Arena::greybox(), shooter_sim::Rules::deathmatch(7)).unwrap();
    let slot = session.join(1).unwrap().unwrap();
    let mut hooks = 0;
    let mut set = 0;
    for tick in 0..1_200 {
        session.queue(
            1,
            shooter_sim::Input {
                sequence: tick + 1,
                intent: scenarios::scripted_intent(0, tick),
                view_tick: session.ticks(),
            },
        );
        session.step().unwrap();
        hooks += session
            .events
            .iter()
            .filter(|event| {
                matches!(event, Event::Fired { slot: from, weapon: Weapon::Grapple, .. } if *from == slot)
            })
            .count();
        set += usize::from(session.fighters[usize::from(slot)].hook.is_some());
    }
    assert!(hooks > 0 && set > 0, "{hooks} hooks, {set} ticks hooked");
}
