//! Turns a recorded match ([`crate::replay`]) into fixtures for the foundation crates, in the
//! shape each foundation already uses:
//!
//! - [`game_server`]: the log itself, in game-server's replay format, cut after tick `to`, with a
//!   `key=value` manifest beside it (the format of game-server's `tests/fixtures`). A game-server
//!   log cannot start mid-match (the shooter's snapshot is not a restorable state), so it always
//!   starts at tick 0.
//! - [`physics`]: JSON for a physics A/B scenario: the arena's boxes and ramps, the character
//!   box, its contact offset and step-up, gravity, and per tick every fighter's intent, the move
//!   its step handed the character controller, and the position, velocity and ground contact
//!   after it (physics-engine's consumer fixtures are JSON in `tests/fixtures`).
//! - [`render`]: JSON for one frame: the arena pieces and jump pads, every fighter's pose, the
//!   rockets in flight, a camera at the first person's eye, and the shots, beams and explosions
//!   of the last few ticks (the weapon lights and particles).

use crate::replay::{Error, Replay};
use shooter_sim::arena::Piece;
use shooter_sim::physics::CONTACT_OFFSET_UNITS;
use shooter_sim::physics::Vec3i;
use shooter_sim::player::{EYE_HEIGHT_UNITS, GRAVITY, PLAYER_HALF_EXTENTS, STEP_UP};
use shooter_sim::session::body_of;
use shooter_sim::{Controller, Event, SUBUNITS_PER_UNIT, Session, TICK_HZ};
use std::fmt::Write;

/// Ticks of effects a render frame carries (beams and explosions fade over about this long).
pub const RENDER_EFFECT_TICKS: u32 = 6;

/// game-server's replay bytes for ticks `0..=to` and the manifest describing them.
///
/// # Errors
///
/// If the log does not reach `to` or does not re-encode.
pub fn game_server(replay: &Replay, to: u64, source: &str) -> Result<(Vec<u8>, String), Error> {
    let bytes = replay.log.encode()?;
    let cut = checkpoint_end(&bytes, to).ok_or(Error::Format("the log does not reach `to`"))?;
    let bytes = bytes[..cut].to_vec();
    let log = game_server::ReplayLog::decode(&bytes)?;
    let records = log.records();
    let count = |kind: fn(&game_server::ReplayRecord) -> bool| {
        records.iter().filter(|record| kind(record)).count()
    };
    use game_server::ReplayRecord as R;
    let mut manifest = String::new();
    let _ = writeln!(manifest, "schema=1");
    let _ = writeln!(manifest, "source_repository=moenarch-games/shooter");
    let _ = writeln!(manifest, "source={source}");
    let _ = writeln!(
        manifest,
        "description=scripted deathmatch: seeded bots in every slot no person holds, scripted people at 100 ms \
         input latency (lag-compensated shots), one leaves and is removed after the grace"
    );
    let _ = writeln!(manifest, "tick_hz={TICK_HZ}");
    let _ = writeln!(manifest, "map={}", replay.map);
    let _ = writeln!(manifest, "slots={}", replay.slots);
    let _ = writeln!(manifest, "bot_seed={}", replay.seed);
    let _ = writeln!(manifest, "replay_format_version={}", bytes[4]);
    let _ = writeln!(manifest, "replay_bytes={}", bytes.len());
    let _ = writeln!(manifest, "expected_records={}", records.len());
    let _ = writeln!(manifest, "expected_final_tick={to}");
    let _ = writeln!(
        manifest,
        "expected_admissions={}",
        count(|r| matches!(r, R::PlayerAdmitted { .. }))
    );
    let _ = writeln!(
        manifest,
        "expected_commands={}",
        count(|r| matches!(r, R::CommandApplied { .. }))
    );
    let _ = writeln!(
        manifest,
        "expected_removals={}",
        count(|r| matches!(r, R::PlayerRemoved { .. }))
    );
    let _ = writeln!(
        manifest,
        "expected_digest_checkpoints={}",
        count(|r| matches!(r, R::DigestCheckpoint { .. }))
    );
    Ok((bytes, manifest))
}

/// The byte offset just past the checkpoint record of tick `to`, walking game-server's record
/// framing (kind u8, tick u64, body length u32, body) after the 5-byte header.
fn checkpoint_end(bytes: &[u8], to: u64) -> Option<usize> {
    const CHECKPOINT_KINDS: [u8; 2] = [4, 5];
    let mut offset = 5;
    while offset + 13 <= bytes.len() {
        let kind = bytes[offset];
        let tick = u64::from_be_bytes(bytes[offset + 1..offset + 9].try_into().ok()?);
        let len = u32::from_be_bytes(bytes[offset + 9..offset + 13].try_into().ok()?) as usize;
        offset += 13 + len;
        if tick == to && CHECKPOINT_KINDS.contains(&kind) {
            return Some(offset);
        }
    }
    None
}

fn units(v: Vec3i) -> String {
    format!("[{},{},{}]", v.x, v.y, v.z)
}

fn floats(v: [f32; 3]) -> String {
    format!("[{},{},{}]", v[0], v[1], v[2])
}

fn pieces(session: &Session) -> String {
    let pieces: Vec<String> = session
        .arena
        .pieces
        .iter()
        .map(|piece| match piece {
            Piece::Block { center, half } => format!(
                "{{\"kind\":\"block\",\"center\":{},\"half\":{}}}",
                units(*center),
                units(*half)
            ),
            Piece::Ramp {
                center,
                half,
                rises,
            } => format!(
                "{{\"kind\":\"ramp\",\"center\":{},\"half\":{},\"rises\":\"{rises:?}\"}}",
                units(*center),
                units(*half)
            ),
        })
        .collect();
    format!(
        "{{\"half_size\":{},\"floor_top\":0,\"pieces\":[{}],\"jump_pads\":[{}]}}",
        session.arena.half_size,
        pieces.join(","),
        session
            .arena
            .pads
            .iter()
            .map(|pad| format!(
                "{{\"center\":{},\"half\":[{},{}],\"launch\":{}}}",
                units(pad.center),
                pad.half[0],
                pad.half[1],
                units(pad.launch)
            ))
            .collect::<Vec<_>>()
            .join(",")
    )
}

/// The physics A/B scenario JSON for ticks `from..=to`.
///
/// # Errors
///
/// If the replay does not play.
pub fn physics(replay: &Replay, from: u64, to: u64, source: &str) -> Result<String, Error> {
    let mut arena = None;
    let mut ticks = Vec::new();
    replay.play(to, |session| {
        if arena.is_none() {
            arena = Some(pieces(session));
        }
        let tick = u64::from(session.ticks());
        if tick < from {
            return Ok(());
        }
        let snapshot = session.snapshot();
        let moves = session.world.moves();
        let fighters: Vec<String> = session
            .fighters
            .iter()
            .filter_map(|fighter| {
                let state = snapshot.slot(fighter.slot)?;
                let intent = fighter.intent;
                // The fighter's move this step (`null`: it had no body to move).
                let body = body_of(fighter.slot);
                let request = moves
                    .iter()
                    .find(|request| request.body == body)
                    .map_or_else(|| "null".to_owned(), |request| floats(request.desired));
                Some(format!(
                    "{{\"slot\":{},\"alive\":{},\"intent\":{{\"forward\":{},\"strafe\":{},\"yaw\":{},\"jump\":{},\"sprint\":{},\"crouch\":{}}},\"move\":{request},\"position\":{},\"velocity\":{},\"grounded\":{}}}",
                    fighter.slot,
                    state.alive,
                    intent.forward,
                    intent.strafe,
                    intent.yaw,
                    intent.jump,
                    intent.sprint,
                    intent.crouch,
                    floats(state.position),
                    floats(state.velocity),
                    fighter.grounded(),
                ))
            })
            .collect();
        ticks.push(format!(
            "{{\"tick\":{tick},\"fighters\":[{}]}}",
            fighters.join(",")
        ));
        Ok(())
    })?;
    Ok(format!(
        "{{\"schema\":\"shooter/physics-window/v1\",\"source\":\"{source}\",\"bot_seed\":{},\"tick_hz\":{TICK_HZ},\"subunits_per_unit\":{SUBUNITS_PER_UNIT},\"units\":\"centimetres, y up; positions are body centres after the tick; velocities units per tick; move is the displacement this tick's step handed the character controller (gravity, ground settle and sliding around other fighters included), in units\",\"gravity_subunits_per_tick2\":{},\"character_half_extents\":{},\"character\":{{\"shape\":\"upright capsule inside the box\",\"contact_offset\":{CONTACT_OFFSET_UNITS},\"step_up\":{{\"max_height\":{},\"min_width\":{}}},\"max_slope_degrees\":45}},\"arena\":{},\"ticks\":[\n{}\n]}}\n",
        replay.seed,
        units(GRAVITY),
        units(PLAYER_HALF_EXTENTS),
        STEP_UP.max_height,
        STEP_UP.min_width,
        arena.unwrap_or_default(),
        ticks.join(",\n")
    ))
}

fn effect(event: &Event) -> Option<String> {
    match event {
        Event::Fired {
            slot,
            weapon,
            eye,
            direction,
        } => Some(format!(
            "{{\"kind\":\"fired\",\"slot\":{slot},\"weapon\":\"{weapon:?}\",\"from\":{},\"direction\":{}}}",
            floats(*eye),
            floats(*direction)
        )),
        Event::Beam {
            weapon,
            from,
            end,
            struck,
            ..
        } => Some(format!(
            "{{\"kind\":\"beam\",\"weapon\":\"{weapon:?}\",\"from\":{},\"to\":{},\"struck\":{struck}}}",
            floats(*from),
            floats(*end)
        )),
        Event::Pellet { from, at, .. } => Some(format!(
            "{{\"kind\":\"beam\",\"weapon\":\"Shotgun\",\"from\":{},\"to\":{},\"struck\":true}}",
            floats(*from),
            floats(*at)
        )),
        Event::Exploded { at } => {
            Some(format!("{{\"kind\":\"explosion\",\"at\":{}}}", floats(*at)))
        }
        Event::Hit { at, damage, .. } => Some(format!(
            "{{\"kind\":\"hit\",\"at\":{},\"damage\":{damage}}}",
            floats(*at)
        )),
        _ => None,
    }
}

/// The render scene JSON for tick `at`.
///
/// # Errors
///
/// If the replay does not play to `at`.
pub fn render(replay: &Replay, at: u64, source: &str) -> Result<String, Error> {
    let mut effects = Vec::new();
    let mut frame = None;
    let mut arena = None;
    let first = at.saturating_sub(u64::from(RENDER_EFFECT_TICKS) - 1);
    replay.play(at, |session| {
        if arena.is_none() {
            arena = Some(pieces(session));
        }
        let tick = u64::from(session.ticks());
        if tick < first {
            return Ok(());
        }
        effects.extend(session.events.iter().filter_map(effect).map(|effect| {
            format!(
                "{{\"age_ticks\":{},{}",
                at - tick,
                effect.trim_start_matches('{')
            )
        }));
        if tick == at {
            let snapshot = session.snapshot();
            let viewer = snapshot
                .fighters
                .iter()
                .find(|f| f.alive && matches!(f.controller, Controller::Human(_)))
                .or_else(|| snapshot.fighters.iter().find(|f| f.alive));
            #[allow(clippy::cast_precision_loss)]
            let camera = viewer.map_or_else(
                || "null".to_owned(),
                |f| {
                    let eye = [
                        f.position[0],
                        f.position[1] - PLAYER_HALF_EXTENTS.y as f32 + EYE_HEIGHT_UNITS as f32,
                        f.position[2],
                    ];
                    format!(
                        "{{\"slot\":{},\"eye\":{},\"yaw\":{},\"pitch\":{},\"fov_degrees\":90}}",
                        f.slot,
                        floats(eye),
                        f.yaw,
                        f.pitch
                    )
                },
            );
            let fighters: Vec<String> = snapshot
                .fighters
                .iter()
                .filter(|f| f.alive)
                .map(|f| {
                    format!(
                        "{{\"slot\":{},\"center\":{},\"yaw\":{},\"pitch\":{},\"held\":\"{:?}\",\"health\":{}}}",
                        f.slot,
                        floats(f.position),
                        f.yaw,
                        f.pitch,
                        f.held,
                        f.health
                    )
                })
                .collect();
            let rockets: Vec<String> = snapshot
                .rockets
                .iter()
                .map(|r| format!("{{\"at\":{},\"age_ticks\":{}}}", floats(r.position), r.age))
                .collect();
            frame = Some((camera, fighters.join(","), rockets.join(",")));
        }
        Ok(())
    })?;
    let (camera, fighters, rockets) =
        frame.ok_or(Error::Format("the replay does not reach `at`"))?;
    let arena = arena.unwrap_or_default();
    Ok(format!(
        "{{\"schema\":\"shooter/render-frame/v1\",\"source\":\"{source}\",\"tick\":{at},\"units\":\"centimetres, y up; yaw is integer trig (65536 per turn, 0 faces +Z, toward +X)\",\"character_half_extents\":{},\"camera\":{camera},\"arena\":{arena},\"fighters\":[{fighters}],\"rockets\":[{rockets}],\"effects\":[{}]}}\n",
        units(PLAYER_HALF_EXTENTS),
        effects.join(",")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::replay::{Script, record};

    #[test]
    fn promoted_fixtures_cover_their_window() {
        let replay = record(Script {
            ticks: 240,
            ..Script::DEFAULT
        })
        .unwrap();
        let (bytes, manifest) = game_server(&replay, 180, "test").unwrap();
        let cut = game_server::ReplayLog::decode(&bytes).unwrap();
        assert_eq!(cut.records().last().unwrap().tick(), 180);
        assert_eq!(cut.encode().unwrap(), bytes);
        assert!(manifest.contains("expected_final_tick=180"));
        assert!(manifest.contains("expected_admissions=2"));

        let physics = physics(&replay, 100, 110, "test").unwrap();
        assert_eq!(physics.matches("\"tick\":").count(), 11);
        // Every live fighter carries the move its step handed the controller.
        assert!(physics.contains("\"move\":["));
        assert_eq!(
            physics.matches("\"move\":").count(),
            physics.matches("\"position\":").count()
        );
        let render = render(&replay, 200, "test").unwrap();
        assert!(render.contains("\"tick\":200"));
        assert!(render.contains("\"camera\":{\"slot\":"));
    }
}
