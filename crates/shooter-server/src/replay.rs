//! Deterministic match replays, recorded in game-server's own replay format.
//!
//! A replay is a scripted deathmatch hosted through game-server's [`MatchRuntime`] with replay
//! capture on: the seeded bots plus a few scripted people who join (the first asking for a
//! heavy robot build), play with a fixed input latency (so their shots are lag-compensated) and
//! leave. game-server's [`ReplayLog`] holds every
//! admission, command and removal and a digest of every tick's snapshot; with the bot seed that is
//! the whole match, and [`verify`] replays it into a fresh [`DeathmatchHost`] and checks every
//! tick. Nothing here comes from real players.
//!
//! A file is `SHRP`, a format byte, the bot seed (u64 big-endian) and the game-server log.
//! [`crate::promote`] turns a replay into fixtures for the foundation crates.

use crate::DeathmatchHost;
use game_server::{
    GameSimulation, MatchRuntime, RECONNECT_TOKEN_BYTES, ReconnectToken, ReplayError, ReplayLog,
    ReplayRecord, ReplayVerification, SimulationError, verify_replay,
};
use shooter_sim::Session;
use shooter_sim::scenarios::{scripted_build, scripted_intent};
use shooter_sim::wire::encode_input;
use std::fmt;

const MAGIC: &[u8; 4] = b"SHRP";
const FORMAT: u8 = 1;
const HEADER_BYTES: usize = MAGIC.len() + 1 + 8;

/// How a scripted match is played.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Script {
    /// Seeds the bots.
    pub seed: u64,
    /// Ticks to play (60 per second).
    pub ticks: u32,
    /// Scripted people; they join one second apart from tick 60.
    pub people: u8,
    /// How many ticks behind the host the scripted clients see the match: their shots rewind
    /// this far (lag compensation).
    pub latency_ticks: u32,
}

impl Script {
    /// 30 s of the `replay` match with two scripted people at 100 ms.
    pub const DEFAULT: Self = Self {
        seed: 0x7265_706c_6179,
        ticks: 30 * 60,
        people: 2,
        latency_ticks: 6,
    };

    /// The tick person `index` joins.
    const fn joins_at(index: u8) -> u32 {
        60 + 60 * index as u32
    }

    /// The tick person `index` disconnects, if they leave before the end: every second person
    /// leaves three quarters in (game-server removes them after the reconnect grace).
    const fn leaves_at(&self, index: u8) -> Option<u32> {
        if index % 2 == 1 {
            Some(self.ticks / 4 * 3)
        } else {
            None
        }
    }
}

/// Ticks a disconnected scripted person keeps their slot before game-server removes them.
const RECONNECT_GRACE_TICKS: u64 = 30;

/// A recorded match: the bot seed and game-server's log of everything else.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Replay {
    pub seed: u64,
    pub log: ReplayLog,
}

#[derive(Debug)]
pub enum Error {
    Simulation(SimulationError),
    Runtime(String),
    Replay(ReplayError),
    Format(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Simulation(error) => error.fmt(formatter),
            Self::Runtime(error) => formatter.write_str(error),
            Self::Replay(error) => error.fmt(formatter),
            Self::Format(error) => write!(formatter, "not a shooter replay: {error}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<SimulationError> for Error {
    fn from(error: SimulationError) -> Self {
        Self::Simulation(error)
    }
}

impl From<ReplayError> for Error {
    fn from(error: ReplayError) -> Self {
        Self::Replay(error)
    }
}

fn rejected(error: impl fmt::Display) -> Error {
    Error::Runtime(error.to_string())
}

/// Plays `script` through game-server with replay capture on.
///
/// # Errors
///
/// If the arena does not build or game-server rejects a step.
pub fn record(script: Script) -> Result<Replay, Error> {
    let host = DeathmatchHost::new(script.seed)?;
    let mut runtime = MatchRuntime::new_with_replay_capture(host, RECONNECT_GRACE_TICKS);
    let mut leases = vec![None; usize::from(script.people)];
    let mut sequences = vec![0_u32; usize::from(script.people)];
    for tick in 0..script.ticks {
        for index in 0..script.people {
            let slot = usize::from(index);
            if tick == Script::joins_at(index) {
                let token = ReconnectToken([index + 1; RECONNECT_TOKEN_BYTES]);
                leases[slot] = Some(runtime.admit(token).map_err(rejected)?);
            }
            if script.leaves_at(index) == Some(tick)
                && let Some(lease) = leases[slot].take()
            {
                runtime.disconnect(lease.player_id, lease.connection_epoch);
            }
            if let Some(lease) = &leases[slot] {
                sequences[slot] += 1;
                let view_tick = tick.saturating_sub(script.latency_ticks);
                let build = scripted_build(index).filter(|_| sequences[slot] == 1);
                let payload = encode_input(
                    scripted_intent(index, tick),
                    view_tick,
                    None,
                    build.as_ref(),
                );
                runtime
                    .submit_command(
                        lease.player_id,
                        lease.connection_epoch,
                        sequences[slot],
                        &payload,
                    )
                    .map_err(rejected)?;
            }
        }
        runtime.advance_tick().map_err(rejected)?;
    }
    let log = runtime
        .replay_log()
        .cloned()
        .ok_or(Error::Runtime("replay capture is off".to_owned()))?;
    Ok(Replay {
        seed: script.seed,
        log,
    })
}

impl Replay {
    /// The file bytes.
    ///
    /// # Errors
    ///
    /// If game-server cannot encode the log.
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        let mut bytes = Vec::with_capacity(HEADER_BYTES);
        bytes.extend_from_slice(MAGIC);
        bytes.push(FORMAT);
        bytes.extend_from_slice(&self.seed.to_be_bytes());
        bytes.extend_from_slice(&self.log.encode()?);
        Ok(bytes)
    }

    /// # Errors
    ///
    /// If `bytes` is not a replay this version writes.
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() < HEADER_BYTES || &bytes[..MAGIC.len()] != MAGIC {
            return Err(Error::Format("no SHRP header"));
        }
        if bytes[MAGIC.len()] != FORMAT {
            return Err(Error::Format("unknown format"));
        }
        let mut seed = [0; 8];
        seed.copy_from_slice(&bytes[MAGIC.len() + 1..HEADER_BYTES]);
        Ok(Self {
            seed: u64::from_be_bytes(seed),
            log: ReplayLog::decode(&bytes[HEADER_BYTES..])?,
        })
    }

    /// The last tick the log reaches.
    #[must_use]
    pub fn ticks(&self) -> u64 {
        self.log.records().last().map_or(0, ReplayRecord::tick)
    }

    /// Replays the log into a fresh match and calls `observe` after every tick; stops after
    /// tick `until` (or the end). Use [`verify`] to check the ticks against the log.
    ///
    /// # Errors
    ///
    /// If the arena does not build or the match rejects a recorded step.
    pub fn play(
        &self,
        until: u64,
        mut observe: impl FnMut(&Session) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let mut host = DeathmatchHost::new(self.seed)?;
        for record in self.log.records() {
            match record {
                ReplayRecord::PlayerAdmitted { player_id, .. } => host.add_player(*player_id)?,
                ReplayRecord::CommandApplied {
                    player_id,
                    sequence,
                    payload,
                    ..
                } => host.apply_command(*player_id, *sequence, payload)?,
                ReplayRecord::PlayerRemoved { player_id, .. } => {
                    host.try_remove_player(*player_id)?;
                }
                ReplayRecord::Checkpoint { .. } | ReplayRecord::DigestCheckpoint { .. } => {
                    host.advance_tick()?;
                    observe(host.session())?;
                    if host.current_tick() >= until {
                        break;
                    }
                }
                ReplayRecord::HostInputApplied { .. } | ReplayRecord::Baseline { .. } => {
                    return Err(Error::Format("host inputs and baselines are not recorded"));
                }
            }
        }
        Ok(())
    }
}

/// Replays `replay` into a fresh match through game-server's verifier: every tick's snapshot
/// must match the recorded digest.
///
/// # Errors
///
/// If the arena does not build or any tick diverges.
pub fn verify(replay: &Replay) -> Result<ReplayVerification, Error> {
    Ok(verify_replay(
        DeathmatchHost::new(replay.seed)?,
        &replay.log,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shooter_sim::Event;
    use shooter_sim::robot::HEAVY;

    fn short() -> Script {
        Script {
            ticks: 600,
            ..Script::DEFAULT
        }
    }

    #[test]
    fn a_replay_reproduces_every_tick_and_round_trips() {
        let replay = record(short()).unwrap();
        assert_eq!(replay.ticks(), 600);
        let bytes = replay.encode().unwrap();
        assert_eq!(Replay::decode(&bytes).unwrap(), replay);
        assert_eq!(record(short()).unwrap().encode().unwrap(), bytes);
        let verified = verify(&replay).unwrap();
        assert_eq!(verified.checkpoints_verified, 600);
        assert_eq!(verified.final_snapshot.tick, 600);
    }

    #[test]
    fn a_diverging_match_fails_verification() {
        let mut replay = record(short()).unwrap();
        replay.seed ^= 1;
        assert!(verify(&replay).is_err());
    }

    #[test]
    fn scripted_people_join_fight_and_leave() {
        let replay = record(Script::DEFAULT).unwrap();
        let records = replay.log.records();
        let count = |kind: fn(&ReplayRecord) -> bool| records.iter().filter(|r| kind(r)).count();
        assert_eq!(
            count(|r| matches!(r, ReplayRecord::PlayerAdmitted { .. })),
            2
        );
        assert_eq!(
            count(|r| matches!(r, ReplayRecord::PlayerRemoved { .. })),
            1
        );
        let (mut shots, mut ticks, mut heavy) = (0, 0, false);
        replay
            .play(u64::MAX, |session| {
                ticks += 1;
                heavy |= session
                    .fighters
                    .iter()
                    .any(|fighter| fighter.human().is_some() && fighter.configured == HEAVY);
                shots += session
                    .events
                    .iter()
                    .filter(|event| matches!(event, Event::Fired { slot: 0 | 1, .. }))
                    .count();
                Ok(())
            })
            .unwrap();
        assert_eq!(ticks, Script::DEFAULT.ticks);
        assert!(shots > 20, "{shots} shots from slots 0 and 1");
        assert!(heavy, "the first person's build request is in the replay");
    }
}
