//! Contested module pickups on the host (`shooter_sim::pickups`): who takes a pad when several
//! reach it, the pad's respawn, a module lasting the life until its uses are spent and its end at
//! death, the round and map
//! transitions, the rail capacitor's lag-compensated hits and frags, thrusters, fighters of
//! different builds, and the wire.

use shooter_sim::physics::Vec3i;
use shooter_sim::pickups::{
    PAD_RESPAWN_TICKS, RAIL_CAPACITOR_DAMAGE_PERCENT, RAIL_CAPACITOR_SHOTS,
    THRUSTER_JUMP_SUBUNITS_PER_TICK, THRUSTER_JUMPS,
};
use shooter_sim::player::{JUMP_VELOCITY_SUBUNITS_PER_TICK, Motion, PLAYER_HALF_EXTENTS};
use shooter_sim::robot::{BETA, HEAVY, MARKSMAN, SKIRMISHER, STANDARD};
use shooter_sim::scenarios;
use shooter_sim::tour::{end_of, run_legs};
use shooter_sim::wire::{decode_snapshot, encode_snapshot};
use shooter_sim::{
    Arena, ArenaModule, Assembly, Carried, Event, Input, Intent, MapId, ModulePad, Phase, Rules,
    Session, Spawn, Weapon,
};

const A: u32 = 1;
const B: u32 = 2;

fn pad(at: [i32; 3], module: ArenaModule) -> ModulePad {
    ModulePad {
        center: Vec3i::new(at[0], at[1], at[2]),
        module,
    }
}

/// A faces B 12 m apart on an open floor with `pads`; both people, no bots.
fn duel(pads: Vec<ModulePad>) -> Session {
    let arena = Arena {
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
        module_pads: pads,
    };
    let rules = Rules {
        slots: 2,
        bots: false,
        local: false,
        seed: 0,
        frag_limit: 2,
        time_limit_ticks: 0,
        intermission_ticks: 60,
        respawn_ticks: 30,
    };
    let mut session = Session::with_rules(arena, rules).unwrap();
    assert_eq!(session.join(A).unwrap(), Some(0));
    assert_eq!(session.join(B).unwrap(), Some(1));
    session
}

/// Puts `slot` standing still with its feet at `feet` (units).
fn stand_at(session: &mut Session, slot: u8, feet: [f32; 3]) {
    #[allow(clippy::cast_precision_loss)]
    let center = [feet[0], feet[1] + PLAYER_HALF_EXTENTS.y as f32, feet[2]];
    session
        .place(
            slot,
            center,
            [0.0; 3],
            Motion::default(),
            Weapon::EnergyRifle,
            None,
        )
        .unwrap();
}

fn taken(session: &Session) -> Vec<(u8, u8, ArenaModule)> {
    session
        .events
        .iter()
        .filter_map(|event| match *event {
            Event::ModuleTaken { slot, pad, module } => Some((slot, pad, module)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_contested_pad_goes_to_the_nearest_fighter_then_the_lowest_slot() {
    // Equally near: the lowest slot.
    let mut session = duel(vec![pad([0, 0, 0], ArenaModule::RailCapacitor)]);
    stand_at(&mut session, 0, [40.0, 0.0, 0.0]);
    stand_at(&mut session, 1, [-40.0, 0.0, 0.0]);
    session.step().unwrap();
    assert_eq!(taken(&session), [(0, 0, ArenaModule::RailCapacitor)]);
    assert_eq!(
        session.fighters[0].carried,
        Some(Carried::fresh(ArenaModule::RailCapacitor))
    );
    assert_eq!(session.fighters[1].carried, None);
    assert_eq!(session.pickups[0].ready_in, PAD_RESPAWN_TICKS);

    // Nearer wins whatever the slot.
    let mut session = duel(vec![pad([0, 0, 0], ArenaModule::Thrusters)]);
    stand_at(&mut session, 0, [55.0, 0.0, 0.0]);
    stand_at(&mut session, 1, [-20.0, 0.0, 30.0]);
    session.step().unwrap();
    assert_eq!(taken(&session), [(1, 0, ArenaModule::Thrusters)]);
    assert_eq!(session.fighters[0].carried, None);

    // Two pads in one tick: each to its own nearest fighter.
    let mut session = duel(vec![
        pad([0, 0, 0], ArenaModule::RailCapacitor),
        pad([500, 0, 0], ArenaModule::Thrusters),
    ]);
    stand_at(&mut session, 0, [500.0, 0.0, 0.0]);
    stand_at(&mut session, 1, [0.0, 0.0, 0.0]);
    session.step().unwrap();
    assert_eq!(
        taken(&session),
        [
            (1, 0, ArenaModule::RailCapacitor),
            (0, 1, ArenaModule::Thrusters)
        ]
    );
}

#[test]
fn a_taken_pad_comes_back_after_its_respawn_time() {
    let mut session = duel(vec![pad([0, 0, -600], ArenaModule::RailCapacitor)]);
    // A spawned on the pad and took it on the first tick.
    session.step().unwrap();
    assert_eq!(taken(&session), [(0, 0, ArenaModule::RailCapacitor)]);
    // B waits on the empty pad.
    stand_at(&mut session, 0, [0.0, 0.0, 600.0]);
    stand_at(&mut session, 1, [0.0, 0.0, -600.0]);
    let mut waited = 1;
    loop {
        session.step().unwrap();
        waited += 1;
        if !taken(&session).is_empty() {
            break;
        }
        assert!(waited <= PAD_RESPAWN_TICKS, "the pad never came back");
    }
    assert_eq!(waited, PAD_RESPAWN_TICKS + 1);
    assert_eq!(taken(&session), [(1, 0, ArenaModule::RailCapacitor)]);
}

/// One input for `id` holding `intent`, seen now.
fn send(session: &mut Session, id: u32, sequence: u32, intent: Intent) {
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

/// A module lasts the whole life, however long, until its uses run out: each thruster jump
/// spends one and the last one spends the module; the next jump is a plain one.
#[test]
fn a_module_lasts_the_life_until_its_uses_are_spent() {
    let mut session = duel(vec![pad([0, 0, -600], ArenaModule::Thrusters)]);
    session.step().unwrap();
    // Off the pad, so it is not taken again when it comes back.
    stand_at(&mut session, 0, [300.0, 0.0, -600.0]);
    // Longer than the pad's respawn: no timer runs it down.
    for _ in 0..PAD_RESPAWN_TICKS + 600 {
        session.step().unwrap();
    }
    assert_eq!(
        session.fighters[0].carried,
        Some(Carried::fresh(ArenaModule::Thrusters))
    );
    let mut sequence = 0;
    let mut jump = |session: &mut Session| {
        sequence += 1;
        let jump = Intent {
            jump: true,
            ..Intent::IDLE
        };
        send(session, A, sequence, jump);
        session.step().unwrap();
        let spent = session.events.contains(&Event::ModuleSpent {
            slot: 0,
            module: ArenaModule::Thrusters,
        });
        sequence += 1;
        send(session, A, sequence, Intent::IDLE);
        let mut peak = 0.0f32;
        for _ in 0..90 {
            session.step().unwrap();
            peak = peak.max(session.fighters[0].player.feet(&session.world)[1]);
        }
        assert!(session.fighters[0].grounded());
        (peak, spent)
    };
    let thruster = {
        let (peak, spent) = jump(&mut session);
        assert!(!spent);
        peak
    };
    assert_eq!(
        session.fighters[0].carried.map(|c| c.uses_left),
        Some(THRUSTER_JUMPS - 1)
    );
    for left in (0..THRUSTER_JUMPS - 1).rev() {
        let (peak, spent) = jump(&mut session);
        assert!((peak - thruster).abs() < 1.0, "{peak} {thruster}");
        assert_eq!(spent, left == 0);
        assert_eq!(
            session.fighters[0].carried.map(|c| c.uses_left),
            (left > 0).then_some(left)
        );
    }
    let (plain, spent) = jump(&mut session);
    assert!(!spent);
    assert!(plain < thruster - 100.0, "{plain} {thruster}");
}

/// Death ends a module at once and drops nothing: the pad keeps its own clock.
#[test]
fn a_module_ends_with_its_carrier() {
    let mut session = duel(vec![pad([0, 0, -600], ArenaModule::Thrusters)]);
    session.step().unwrap();
    // B's rifle bolt finishes A.
    session.fighters[0].health = 1;
    session.queue(
        B,
        Input {
            sequence: 1,
            intent: Intent {
                yaw: 32_768,
                fire: true,
                select: Some(Weapon::EnergyRifle),
                ..Intent::IDLE
            },
            view_tick: session.ticks(),
        },
    );
    session.step().unwrap();
    assert!(session.events.contains(&Event::Killed {
        victim: 0,
        killer: Some(1),
        weapon: Some(Weapon::EnergyRifle),
    }));
    assert!(!session.fighters[0].alive());
    assert_eq!(session.fighters[0].carried, None);
    assert_eq!(session.pickups.len(), 1);
    assert_eq!(session.pickups[0].ready_in, PAD_RESPAWN_TICKS - 1);
    // Respawned, it carries nothing until it takes a pad again.
    for _ in 0..40 {
        session.step().unwrap();
    }
    assert!(session.fighters[0].alive());
    assert_eq!(session.fighters[0].carried, None);
}

#[test]
fn a_fresh_round_clears_the_modules_and_readies_every_pad() {
    let mut session = duel(vec![
        pad([0, 0, -600], ArenaModule::RailCapacitor),
        pad([0, 0, 600], ArenaModule::Thrusters),
    ]);
    session.step().unwrap();
    assert!(session.fighters.iter().all(|f| f.carried.is_some()));
    assert!(session.pickups.iter().all(|p| !p.ready()));
    // Off the pads; the round ends now.
    stand_at(&mut session, 0, [400.0, 0.0, 0.0]);
    stand_at(&mut session, 1, [-400.0, 0.0, 0.0]);
    session.phase = Phase::Intermission {
        winner: None,
        restarts_at: session.ticks() + 5,
    };
    let carried: Vec<_> = session.fighters.iter().map(|f| f.carried).collect();
    for _ in 0..5 {
        session.step().unwrap();
    }
    // Nothing runs down or comes back between rounds.
    assert_eq!(
        session
            .fighters
            .iter()
            .map(|f| f.carried)
            .collect::<Vec<_>>(),
        carried
    );
    session.step().unwrap();
    assert!(matches!(session.phase, Phase::Playing { .. }));
    // Everyone is back on their spawn point, on the pads, and takes them again at once.
    assert_eq!(
        taken(&session),
        [
            (0, 0, ArenaModule::RailCapacitor),
            (1, 1, ArenaModule::Thrusters)
        ]
    );
    assert!(
        session
            .fighters
            .iter()
            .all(|f| f.carried.is_some_and(|c| c.uses_left == c.module.uses()))
    );
}

#[test]
fn every_map_starts_with_its_own_pads_ready() {
    for map in MapId::ALL {
        let arena = map.arena();
        let session = Session::on_map(map, Rules::deathmatch(1)).unwrap();
        let pads: Vec<ModulePad> = session.pickups.iter().map(|p| p.pad).collect();
        assert_eq!(pads, arena.module_pads, "{map}");
        assert!(session.pickups.iter().all(|p| p.ready()), "{map}");
        assert!(
            session.fighters.iter().all(|f| f.carried.is_none()),
            "{map}"
        );
        let snapshot = session.snapshot();
        assert_eq!(snapshot.pickups.len(), arena.module_pads.len(), "{map}");
        for (state, pad) in snapshot.pickups.iter().zip(&arena.module_pads) {
            assert_eq!((state.center, state.module), (pad.center, pad.module));
            assert_eq!(state.ready_in, 0);
        }
    }
}

/// The greybox has a weapon and a movement module, both on open, standable ground away from the
/// spawn points and the jump pads.
#[test]
fn the_greybox_places_a_weapon_and_a_movement_module_at_contested_points() {
    use shooter_sim::pickups::{Class, REACH_UNITS};
    let arena = Arena::greybox();
    let classes: Vec<Class> = arena.module_pads.iter().map(|p| p.module.class()).collect();
    assert!(classes.contains(&Class::Weapon) && classes.contains(&Class::Movement));
    for pad in &arena.module_pads {
        for spawn in arena.spawn_points() {
            let d = (spawn.feet.x - pad.center.x).abs() + (spawn.feet.z - pad.center.z).abs();
            assert!(d > 4 * REACH_UNITS, "{pad:?} next to {spawn:?}");
        }
        assert!(
            arena.pad_under(pad.center).is_none(),
            "{pad:?} on a jump pad"
        );
        // A fighter walks onto it (or along the platform's top to it).
        let beside = Vec3i::new(pad.center.x - 200, pad.center.y, pad.center.z);
        assert!(arena.clear_walk(beside, pad.center), "{pad:?}");
    }
}

/// A (slot 0) faces B 12 m away, B strafes out of the line of fire for 12 ticks, then A fires the
/// rail gun having seen B `seen_ticks_ago`; A is built as `build` and, with `capacitor`, took the
/// rail capacitor from a pad on its spawn point. The damage B took, if hit.
fn rail_duel(seen_ticks_ago: u32, build: Assembly, capacitor: bool) -> Option<i32> {
    let pads = if capacitor {
        vec![pad([0, 0, -600], ArenaModule::RailCapacitor)]
    } else {
        Vec::new()
    };
    let mut session = duel(pads);
    session.fighters[0].build = build;
    let mut sequence = 0;
    let mut step = |session: &mut Session, a: Intent, b: Intent, view_tick: u32| {
        sequence += 1;
        for (id, intent) in [(A, a), (B, Intent { yaw: 32_768, ..b })] {
            session.queue(
                id,
                Input {
                    sequence,
                    intent,
                    view_tick,
                },
            );
        }
        session.step().unwrap();
    };
    let take = Intent {
        select: Some(Weapon::RailGun),
        ..Intent::IDLE
    };
    for _ in 0..10 {
        let now = session.ticks();
        step(&mut session, take, Intent::IDLE, now);
    }
    assert_eq!(
        session.fighters[0].carried.map(|c| c.module),
        capacitor.then_some(ArenaModule::RailCapacitor)
    );
    let strafe = Intent {
        strafe: 1,
        ..Intent::IDLE
    };
    for _ in 0..12 {
        let now = session.ticks();
        step(&mut session, Intent::IDLE, strafe, now);
    }
    let fire = Intent {
        fire: true,
        ..Intent::IDLE
    };
    let seen = session.ticks() - seen_ticks_ago;
    step(&mut session, fire, strafe, seen);
    session.events.iter().find_map(|event| match event {
        Event::Hit {
            victim: 1, damage, ..
        } => Some(*damage),
        _ => None,
    })
}

/// The rail capacitor changes the host's lag-compensated hits, not whether they land, and it
/// stacks on a build's rail mod.
#[test]
fn the_rail_capacitor_hits_harder_through_lag_compensation_on_any_build() {
    let shot = |build, capacitor| {
        (
            rail_duel(0, build, capacitor),
            rail_duel(12, build, capacitor),
        )
    };
    assert_eq!(shot(STANDARD, false), (None, Some(85)));
    let boosted = 85 * i32::from(RAIL_CAPACITOR_DAMAGE_PERCENT) / 100;
    assert_eq!(shot(STANDARD, true), (None, Some(boosted)));
    // The marksman's overcharger (125 %) and the capacitor (150 %) multiply: 187 %.
    assert_eq!(shot(MARKSMAN, false), (None, Some(106)));
    assert_eq!(shot(MARKSMAN, true), (None, Some(85 * 187 / 100)));
    // Builds without a rail mod get the same capacitor.
    assert_eq!(shot(HEAVY, true), (None, Some(boosted)));
    assert_eq!(shot(SKIRMISHER, true), (None, Some(boosted)));
}

/// With the capacitor two rail hits kill and score, recharging in three quarters of the time;
/// the carrier's frags count like any other.
#[test]
fn rail_capacitor_kills_score() {
    let mut session = duel(vec![pad([0, 0, -600], ArenaModule::RailCapacitor)]);
    let rail = Intent {
        fire: true,
        select: Some(Weapon::RailGun),
        ..Intent::IDLE
    };
    let mut shots = Vec::new();
    let mut kills = 0;
    for sequence in 1..=200 {
        let now = session.ticks();
        // The rail gun comes up while A takes the capacitor, then fires.
        let intent = Intent {
            fire: sequence > 10,
            ..rail
        };
        session.queue(
            A,
            Input {
                sequence,
                intent,
                view_tick: now,
            },
        );
        session.step().unwrap();
        for event in &session.events {
            match *event {
                Event::Fired {
                    slot: 0,
                    weapon: Weapon::RailGun,
                    ..
                } => shots.push(session.ticks()),
                Event::Killed {
                    victim: 1,
                    killer: Some(0),
                    weapon: Some(Weapon::RailGun),
                } => kills += 1,
                _ => {}
            }
        }
        if kills > 0 {
            break;
        }
    }
    assert_eq!(kills, 1);
    assert_eq!(shots.len(), 2, "{shots:?}");
    // 72 ticks shipped, 54 with the capacitor.
    assert_eq!(shots[1] - shots[0], 54);
    assert_eq!(session.fighters[0].frags, 1);
    assert_eq!(session.fighters[1].deaths, 1);
    assert_eq!(
        session.fighters[0].carried.map(|c| c.uses_left),
        Some(RAIL_CAPACITOR_SHOTS - 2)
    );
}

/// The capacitor boosts its rail shots and no more: the last one spends it, and the gun is back
/// to the build's own pace and damage. Other weapons' shots spend nothing.
#[test]
fn the_rail_capacitor_is_spent_by_its_rail_shots() {
    let mut session = duel(vec![pad([0, 0, -600], ArenaModule::RailCapacitor)]);
    // Facing away from B, at the wall.
    let shoot = |weapon| Intent {
        yaw: 32_768,
        fire: true,
        select: Some(weapon),
        ..Intent::IDLE
    };
    let mut sequence = 0;
    // The rifle fires a while first: it spends nothing.
    for _ in 0..60 {
        sequence += 1;
        send(&mut session, A, sequence, shoot(Weapon::EnergyRifle));
        session.step().unwrap();
    }
    assert_eq!(
        session.fighters[0].carried,
        Some(Carried::fresh(ArenaModule::RailCapacitor))
    );
    let mut shots = Vec::new();
    let mut spent_at = None;
    for _ in 0..600 {
        sequence += 1;
        send(&mut session, A, sequence, shoot(Weapon::RailGun));
        session.step().unwrap();
        for event in &session.events {
            match *event {
                Event::Fired {
                    slot: 0,
                    weapon: Weapon::RailGun,
                    ..
                } => shots.push(session.ticks()),
                Event::ModuleSpent {
                    slot: 0,
                    module: ArenaModule::RailCapacitor,
                } => spent_at = Some(session.ticks()),
                _ => {}
            }
        }
    }
    let boosted = usize::from(RAIL_CAPACITOR_SHOTS);
    assert!(shots.len() > boosted + 2, "{shots:?}");
    // Spent by the last boosted shot, as it fires.
    assert_eq!(spent_at, Some(shots[boosted - 1]));
    assert_eq!(session.fighters[0].carried, None);
    let gaps: Vec<u32> = shots.windows(2).map(|pair| pair[1] - pair[0]).collect();
    // The boosted shots recharge in 54 ticks (the last one too: it went out boosted), then 72.
    assert!(gaps[..boosted].iter().all(|gap| *gap == 54), "{gaps:?}");
    assert!(gaps[boosted..].iter().all(|gap| *gap == 72), "{gaps:?}");
}

/// Any build takes any module, keeps its build (still valid), and gets the same effect: the
/// heavy robot jumps as high with thrusters as anyone, the skirmisher's rail gun is boosted.
#[test]
fn fighters_of_different_builds_take_and_use_any_module() {
    let mut session = duel(vec![
        pad([0, 0, -600], ArenaModule::Thrusters),
        pad([0, 0, 600], ArenaModule::RailCapacitor),
    ]);
    session.fighters[0].build = HEAVY;
    session.fighters[1].build = SKIRMISHER;
    session.step().unwrap();
    assert_eq!(
        taken(&session),
        [
            (0, 0, ArenaModule::Thrusters),
            (1, 1, ArenaModule::RailCapacitor)
        ]
    );
    assert_eq!(session.fighters[0].build, HEAVY);
    assert_eq!(session.fighters[1].build, SKIRMISHER);
    for fighter in &session.fighters {
        assert_eq!(BETA.validate(&fighter.build.candidate()), Ok(fighter.build));
    }
    assert_eq!(
        session.fighters[1].tuning(Weapon::RailGun).damage_percent,
        RAIL_CAPACITOR_DAMAGE_PERCENT
    );
    // Both jump: the thrusters leave faster.
    let jump = Intent {
        jump: true,
        ..Intent::IDLE
    };
    for (id, sequence) in [(A, 1), (B, 1)] {
        session.queue(
            id,
            Input {
                sequence,
                intent: jump,
                view_tick: 0,
            },
        );
    }
    let mut peaks = [0.0f32; 2];
    for _ in 0..40 {
        session.step().unwrap();
        for (slot, peak) in peaks.iter_mut().enumerate() {
            let y = session.fighters[slot].player.feet(&session.world)[1];
            *peak = peak.max(y);
        }
    }
    let expected = |v: i32| {
        // v²/2g with g = 18 m/s², from subunits per tick.
        let units_per_second = v as f32 * 60.0 / 256.0;
        units_per_second * units_per_second / 3_600.0
    };
    assert!(
        (peaks[0] - expected(THRUSTER_JUMP_SUBUNITS_PER_TICK)).abs() < 8.0,
        "{peaks:?}"
    );
    assert!(
        (peaks[1] - expected(JUMP_VELOCITY_SUBUNITS_PER_TICK)).abs() < 8.0,
        "{peaks:?}"
    );
}

#[test]
fn thrusters_climb_a_ledge_a_plain_jump_cannot() {
    let scenario = scenarios::THRUSTER_JUMP;
    let run = |arena: Arena| {
        let mut session = Session::new(arena).unwrap();
        run_legs(&mut session, scenario.legs).unwrap()
    };
    let half = PLAYER_HALF_EXTENTS.y as f32;
    let samples = run((scenario.arena)());
    let top = end_of(&samples, "on the ledge").unwrap();
    assert!(top.grounded, "{top:?}");
    assert!((top.center[1] - (200.0 + half)).abs() < 2.0, "{top:?}");
    let plain = run(Arena {
        module_pads: Vec::new(),
        ..(scenario.arena)()
    });
    let bottom = end_of(&plain, "on the ledge").unwrap();
    assert!((bottom.center[1] - half).abs() < 2.0, "{bottom:?}");
}

#[test]
fn modules_and_pads_round_trip_on_the_wire() {
    let mut session = duel(vec![
        pad([0, 0, -600], ArenaModule::RailCapacitor),
        pad([300, 0, 0], ArenaModule::Thrusters),
    ]);
    let mut saw = (false, false);
    for _ in 0..3 {
        session.step().unwrap();
        let snapshot = session.snapshot();
        saw.0 |= snapshot
            .events
            .iter()
            .any(|e| matches!(e, Event::ModuleTaken { .. }));
        assert_eq!(
            decode_snapshot(&encode_snapshot(&snapshot)).unwrap(),
            snapshot
        );
    }
    let snapshot = session.snapshot();
    assert_eq!(
        snapshot.slot(0).unwrap().carried,
        Some(Carried {
            module: ArenaModule::RailCapacitor,
            uses_left: RAIL_CAPACITOR_SHOTS
        })
    );
    assert_eq!(snapshot.slot(1).unwrap().carried, None);
    assert_eq!(snapshot.pickups[0].ready_in, PAD_RESPAWN_TICKS - 2);
    assert_eq!(snapshot.pickups[1].ready_in, 0);
    // B jumps on its last thruster use.
    session.fighters[1].carried = Some(Carried {
        module: ArenaModule::Thrusters,
        uses_left: 1,
    });
    stand_at(&mut session, 1, [0.0, 0.0, 600.0]);
    send(
        &mut session,
        B,
        1,
        Intent {
            jump: true,
            ..Intent::IDLE
        },
    );
    session.step().unwrap();
    let snapshot = session.snapshot();
    saw.1 |= snapshot.events.contains(&Event::ModuleSpent {
        slot: 1,
        module: ArenaModule::Thrusters,
    });
    assert_eq!(
        decode_snapshot(&encode_snapshot(&snapshot)).unwrap(),
        snapshot
    );
    assert_eq!(saw, (true, true));
}
