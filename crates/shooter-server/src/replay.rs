//! Deterministic match replays, recorded in game-server's own replay format.
//!
//! A replay is a scripted deathmatch hosted through game-server's [`MatchRuntime`] with replay
//! capture on: the seeded bots plus a few scripted people who join (the first asking for a
//! heavy robot build, the second for the modded skirmisher loadout), play with a fixed input latency (so their shots are lag-compensated) and
//! leave. game-server's [`ReplayLog`] holds every
//! admission, command and removal and a digest of every tick's snapshot; with the bot seed that is
//! the whole match on its map, and [`verify`] replays it into a fresh [`DeathmatchHost`] and checks every
//! tick. Nothing here comes from real players.
//!
//! A file is `SHRP`, a format byte, the bot seed (u64 big-endian), the slot count (a byte), the
//! map ID (a length byte and its [`MapId::as_str`]) and the game-server log. A file naming a map
//! this build does not know is rejected.
//! [`crate::promote`] turns a replay into fixtures for the foundation crates.

use crate::DeathmatchHost;
use game_server::{
    GameSimulation, MatchRuntime, RECONNECT_TOKEN_BYTES, ReconnectToken, ReplayError, ReplayLog,
    ReplayRecord, ReplayVerification, SimulationError, verify_replay,
};
use shooter_sim::scenarios::{scripted_build, scripted_intent};
use shooter_sim::session::DEATHMATCH_SLOTS;
use shooter_sim::wire::encode_input;
use shooter_sim::{MapId, Session};
use std::fmt;

const MAGIC: &[u8; 4] = b"SHRP";
/// 23: the logged commands are wire-30 inputs and the digests cover wire-30 snapshots (turned
/// parts seated on their socket's face, their footprint the cells they lie over, shooter#124).
/// 22: the logged commands are wire-29 inputs and the digests cover wire-29 snapshots (each
/// fighter's ticks until its grapple module may hook again; the rope tied at the module,
/// shooter#104).
/// 21: the logged commands are wire-28 inputs and the digests cover wire-28 snapshots (the
/// rocket pod's tubes alternate, each fighter's next tubes in the snapshots, shooter#102).
/// 20: the logged commands are wire-27 inputs and the digests cover wire-27 snapshots (the
/// combustion chassis and engine, the flamethrower and a combustion drive on fuel, shooter#130).
/// 19: the logged commands are wire-26 inputs and the digests cover wire-26 snapshots (each
/// robot's fuel beside its electricity, shooter#128).
/// 18: the logged commands are wire-25 inputs (a joint may be told to track the camera's
/// target, shooter#132) and the digests cover wire-25 snapshots.
/// 17: the logged commands are wire-24 inputs (per-copy fire requests, shooter#127: every
/// scripted person asks the parts 7 to 12 to fire as a weapon group now and then) and the
/// digests cover wire-24 snapshots.
/// 16: the logged commands are wire-23 inputs (a weapon switch names a weapon or an exact
/// mounted copy, shooter#125; the third and fourth scripted people play the twin rail and the
/// rocket pod) and the digests cover wire-23 snapshots (the copy held, the copy that fired).
/// 15: the logged commands are wire-22 inputs (each carries the actuator commands its
/// person's control layout resolved, shooter#74; the scripted people play the named loadouts'
/// preset) and the digests cover wire-22 snapshots.
/// 14: the logged commands are wire-21 inputs (the grapple module's own key, shooter#104) and
/// the digests cover wire-21 snapshots.
/// 13: the logged commands are wire-20 inputs and the digests cover wire-20 snapshots
/// (collision damage and the physical debris, shooter#75).
/// 12: the logged commands are wire-19 inputs (a repair order) and the digests cover wire-19
/// snapshots (each fighter's repair and repairable parts, shooter#61); 11 carried wire 18 (the
/// modules dead carriers dropped, shooter#62); 10
/// carried wire 17 (the arena placeables' health, shooter#60); 9: the logged commands are wire-14
/// inputs and the digests cover wire-14 snapshots (builds without the retired control core, the
/// root chassis the vital part); 8 carried wire 13 (each
/// robot's part healths, part hits and detachments); 7 carried wire 12 (shared energy charge,
/// batteries, no heat), 6 wire 11 (robot builds as socket-grid component trees), 5 wire 10
/// (module pickups), 4 wire 9 (robot builds with weapon mods).
const FORMAT: u8 = 23;
/// The magic, the format byte, the seed and the slot count; the map follows.
const HEADER_BYTES: usize = MAGIC.len() + 1 + 8 + 1;

/// How a scripted match is played.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Script {
    /// The map it is played on.
    pub map: MapId,
    /// The match's slots (bots in every one no scripted person holds).
    pub slots: u8,
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
        map: MapId::DEFAULT,
        slots: DEATHMATCH_SLOTS,
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

/// A recorded match: its map, its slot count, the bot seed and game-server's log of everything
/// else.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Replay {
    pub map: MapId,
    pub slots: u8,
    pub seed: u64,
    pub log: ReplayLog,
}

#[derive(Debug)]
pub enum Error {
    Simulation(SimulationError),
    Runtime(String),
    Replay(ReplayError),
    Format(&'static str),
    Map(shooter_sim::UnknownMap),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Simulation(error) => error.fmt(formatter),
            Self::Runtime(error) => formatter.write_str(error),
            Self::Replay(error) => error.fmt(formatter),
            Self::Format(error) => write!(formatter, "not a shooter replay: {error}"),
            Self::Map(error) => write!(formatter, "replay of an {error}"),
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
    let host = DeathmatchHost::new(script.map, script.slots, script.seed)?;
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
        map: script.map,
        slots: script.slots,
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
        bytes.push(self.slots);
        let map = self.map.as_str().as_bytes();
        bytes.push(u8::try_from(map.len()).map_err(|_| Error::Format("map ID too long"))?);
        bytes.extend_from_slice(map);
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
        seed.copy_from_slice(&bytes[MAGIC.len() + 1..HEADER_BYTES - 1]);
        let slots = bytes[HEADER_BYTES - 1];
        let rest = &bytes[HEADER_BYTES..];
        let (&length, rest) = rest.split_first().ok_or(Error::Format("no map ID"))?;
        let (map, log) = rest
            .split_at_checked(usize::from(length))
            .ok_or(Error::Format("no map ID"))?;
        let map = std::str::from_utf8(map)
            .map_err(|_| Error::Format("map ID is not UTF-8"))?
            .parse()
            .map_err(Error::Map)?;
        Ok(Self {
            map,
            slots,
            seed: u64::from_be_bytes(seed),
            log: ReplayLog::decode(log)?,
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
        let mut host = DeathmatchHost::new(self.map, self.slots, self.seed)?;
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
        DeathmatchHost::new(replay.map, replay.slots, replay.seed)?,
        &replay.log,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shooter_sim::Event;
    use shooter_sim::robot::{HEAVY, SKIRMISHER};

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

    /// A scripted match on each kit-built map (the assembly works, shooter#52; the proving cell,
    /// shooter#53; the drydock, shooter#55) records, round-trips, replays tick for tick and
    /// restores its map.
    #[test]
    fn a_replay_on_a_kit_built_map_reproduces_every_tick() {
        for map in [MapId::AssemblyWorks, MapId::ProvingCell, MapId::Drydock] {
            let script = Script { map, ..short() };
            let replay = record(script).unwrap();
            let bytes = replay.encode().unwrap();
            assert_eq!(Replay::decode(&bytes).unwrap().map, map);
            assert_eq!(record(script).unwrap().encode().unwrap(), bytes);
            let verified = verify(&replay).unwrap();
            assert_eq!(verified.checkpoints_verified, 600);
            assert_eq!(verified.final_snapshot.tick, 600);
            replay
                .play(600, |session| {
                    assert_eq!(session.map, Some(map));
                    Ok(())
                })
                .unwrap();
        }
    }

    #[test]
    fn a_replay_restores_its_map_and_rejects_unknown_ones() {
        let replay = record(short()).unwrap();
        let bytes = replay.encode().unwrap();
        assert_eq!(Replay::decode(&bytes).unwrap().map, MapId::DEFAULT);
        let mut maps = Vec::new();
        replay
            .play(10, |session| {
                maps.push(session.map);
                Ok(())
            })
            .unwrap();
        assert_eq!(maps, vec![Some(MapId::DEFAULT); 10]);
        let named = b"greybox";
        let at = bytes
            .windows(named.len())
            .position(|window| window == named)
            .unwrap();
        let mut unknown = bytes.clone();
        unknown[at..at + named.len()].copy_from_slice(b"nowhere");
        assert!(matches!(Replay::decode(&unknown), Err(Error::Map(_))));
        assert!(Replay::decode(&bytes[..HEADER_BYTES + 3]).is_err());
    }

    /// Every robot's electricity and fuel are part of the replayed match: two replays of the
    /// same log spend and generate them alike, the robots do spend electricity and burn fuel for
    /// it, and the heavy build's battery shows in its capacity.
    #[test]
    fn a_replay_reproduces_every_energy_pool() {
        let replay = record(Script::DEFAULT).unwrap();
        let pools = || {
            let mut charges = Vec::new();
            let (mut spent, mut burned, mut battery) = (false, false, false);
            replay
                .play(u64::MAX, |session| {
                    for fighter in session.fighters.iter().filter(|f| f.alive()) {
                        let pool = fighter.pool();
                        assert!(fighter.energy <= pool.capacity);
                        assert!(fighter.fuel <= pool.fuel);
                        spent |= fighter.energy < pool.capacity;
                        burned |= fighter.fuel < pool.fuel;
                        battery |= fighter.build == HEAVY && pool.capacity == 160_000;
                    }
                    charges.push(
                        session
                            .fighters
                            .iter()
                            .map(|f| (f.energy, f.fuel))
                            .collect::<Vec<_>>(),
                    );
                    Ok(())
                })
                .unwrap();
            assert!(spent, "the match spends energy");
            assert!(burned, "and the generators burn fuel for it");
            assert!(battery, "the heavy person comes back with its battery");
            charges
        };
        assert_eq!(pools(), pools());
    }

    /// The scripted people's grapples replay (shooter#104): they play six ticks behind the host,
    /// hold the grapple module's key now and then and hook the arena; every tick verifies, and
    /// a replay sets, pulls and lets go of the same hooks on the same ticks, each rope tied at
    /// its robot's module.
    #[test]
    fn a_replay_reproduces_every_grapple() {
        let replay = record(Script::DEFAULT).unwrap();
        assert_eq!(verify(&replay).unwrap().checkpoints_verified, 1_800);
        let ropes = || {
            let mut ropes = Vec::new();
            replay
                .play(u64::MAX, |session| {
                    for fighter in &session.fighters {
                        if let Some(hook) = fighter.hook {
                            let tie = fighter.rope_tie().expect("hooked by its module");
                            ropes.push((session.ticks(), fighter.slot, hook, tie));
                        }
                    }
                    Ok(())
                })
                .unwrap();
            ropes
        };
        let first = ropes();
        let hooks = first
            .windows(2)
            .filter(|pair| pair[0].1 != pair[1].1 || pair[0].2.anchor != pair[1].2.anchor)
            .count();
        assert!(
            first.len() > 60 && hooks > 1,
            "{} ticks hooked",
            first.len()
        );
        assert_eq!(first, ropes());
    }

    /// Arena destruction replays (shooter#60): a minute on the proving cell breaks some of its
    /// cover, every tick (the placeables' health in each snapshot digest) verifies, and a replay
    /// breaks the same placeables on the same ticks.
    #[test]
    fn a_replay_reproduces_every_broken_placeable() {
        let script = Script {
            map: MapId::ProvingCell,
            ticks: 3_600,
            ..Script::DEFAULT
        };
        let replay = record(script).unwrap();
        assert_eq!(verify(&replay).unwrap().checkpoints_verified, 3_600);
        let breaks = || {
            let mut states = Vec::new();
            replay
                .play(u64::MAX, |session| {
                    let state = session.scenery.state();
                    if states.last().is_none_or(|(_, last)| *last != state) {
                        states.push((session.ticks(), state));
                    }
                    Ok(())
                })
                .unwrap();
            states
        };
        let states = breaks();
        let (_, last) = states.last().unwrap();
        assert!(
            last.iter().any(|placeable| placeable.health == 0),
            "nothing broke: {last:?}"
        );
        assert_eq!(states, breaks());
    }

    #[test]
    fn a_diverging_match_fails_verification() {
        let mut replay = record(short()).unwrap();
        replay.seed ^= 1;
        assert!(verify(&replay).is_err());
        let mut replay = record(short()).unwrap();
        replay.slots -= 1;
        assert!(
            verify(&replay).is_err(),
            "the slot count is part of the match"
        );
    }

    #[test]
    fn a_replay_restores_its_slot_count() {
        assert_eq!(Script::DEFAULT.slots, 12);
        for slots in [4, DEATHMATCH_SLOTS] {
            let replay = record(Script { slots, ..short() }).unwrap();
            let decoded = Replay::decode(&replay.encode().unwrap()).unwrap();
            assert_eq!(decoded.slots, slots);
            assert_eq!(verify(&decoded).unwrap().checkpoints_verified, 600);
            let mut fighters = 0;
            decoded
                .play(1, |session| {
                    fighters = session.fighters.len();
                    Ok(())
                })
                .unwrap();
            assert_eq!(fighters, usize::from(slots));
        }
    }

    /// Four scripted people, the third on the twin rail and the fourth on the rocket pod
    /// (shooter#125): the match replays tick for tick, and both fire more than one copy of their
    /// weapon: the twin rail both at once, the pod never two rockets in one tick (shooter#127).
    #[test]
    fn a_replay_with_weapon_copies_reproduces_every_tick() {
        use shooter_sim::Assembly;
        use shooter_sim::robot::{BETA, PartId, ROCKET_POD, TWIN_RAIL};
        use shooter_sim::weapons::Weapon;
        let script = Script {
            ticks: 900,
            people: 4,
            ..Script::DEFAULT
        };
        let replay = record(script).unwrap();
        let bytes = replay.encode().unwrap();
        assert_eq!(Replay::decode(&bytes).unwrap(), replay);
        assert_eq!(verify(&replay).unwrap().checkpoints_verified, 900);
        let mut copies: Vec<(Assembly, PartId)> = Vec::new();
        // Both rail guns in one tick (shooter#127, the scripted weapon group).
        let mut together = false;
        replay
            .play(u64::MAX, |session| {
                let mut this_tick: Vec<(u8, Weapon)> = Vec::new();
                for event in &session.events {
                    if let Event::Fired {
                        slot, weapon, part, ..
                    } = *event
                    {
                        this_tick.push((slot, weapon));
                        let fighter = &session.fighters[usize::from(slot)];
                        let build = fighter.build;
                        if fighter.human().is_some()
                            && [TWIN_RAIL, ROCKET_POD].contains(&build)
                            && BETA.instance(&build, part).map(|copy| copy.weapon) == Some(weapon)
                            && !copies.contains(&(build, part))
                        {
                            copies.push((build, part));
                        }
                    }
                }
                for (slot, fighter) in session.fighters.iter().enumerate() {
                    let fired = |of: Weapon| {
                        this_tick
                            .iter()
                            .filter(|(shooter, weapon)| {
                                usize::from(*shooter) == slot && *weapon == of
                            })
                            .count()
                    };
                    // Never two rockets from one robot in one tick: the salvo limit.
                    assert!(fired(Weapon::RocketLauncher) <= 1);
                    together |= fighter.build == TWIN_RAIL && fired(Weapon::RailGun) == 2;
                }
                Ok(())
            })
            .unwrap();
        assert!(together, "the twin rail fires both rail guns in one tick");
        for build in [TWIN_RAIL, ROCKET_POD] {
            let fired: Vec<PartId> = copies
                .iter()
                .filter(|(of, _)| *of == build)
                .map(|(_, part)| *part)
                .collect();
            assert!(
                fired.contains(&PartId(7)) && fired.contains(&PartId(9)),
                "{fired:?}"
            );
        }
    }

    /// Five scripted people, the fifth asking for the rail rack (shooter#126): the replay
    /// decodes and verifies tick for tick, and the rack it carries and plays has the authored
    /// socket structure, part for part and socket for socket, and fires from its rails.
    #[test]
    fn a_replay_with_mount_rails_reproduces_their_socket_structure() {
        use shooter_sim::mechanics::sockets;
        use shooter_sim::robot::{BETA, Kind, RAIL_RACK};
        let script = Script {
            ticks: 900,
            people: 5,
            ..Script::DEFAULT
        };
        let replay = record(script).unwrap();
        let bytes = replay.encode().unwrap();
        let decoded = Replay::decode(&bytes).unwrap();
        assert_eq!(decoded, replay);
        assert_eq!(verify(&decoded).unwrap().checkpoints_verified, 900);
        let (mut configured, mut played, mut from_a_rail) = (false, false, false);
        decoded
            .play(u64::MAX, |session| {
                for fighter in session.fighters.iter().filter(|f| f.human().is_some()) {
                    if fighter.configured == RAIL_RACK {
                        configured = true;
                        assert_eq!(fighter.configured.candidate(), RAIL_RACK.candidate());
                    }
                    if fighter.build == RAIL_RACK {
                        played = true;
                        assert_eq!(sockets(&fighter.build), sockets(&RAIL_RACK));
                    }
                }
                for event in &session.events {
                    if let Event::Fired { slot, part, .. } = *event {
                        let build = session.fighters[usize::from(slot)].build;
                        let support = build
                            .part(part)
                            .and_then(|part| part.mount)
                            .and_then(|mount| build.part(mount.parent))
                            .and_then(|parent| BETA.part_type(parent.kind));
                        from_a_rail |= build == RAIL_RACK
                            && support.is_some_and(|support| support.kind == Kind::Rail);
                    }
                }
                Ok(())
            })
            .unwrap();
        assert!(configured, "the fifth person's rail rack is in the replay");
        assert!(played, "the rail rack plays after a respawn");
        assert!(from_a_rail, "a weapon on a rail fires");
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
        let (mut shots, mut ticks, mut heavy, mut modded) = (0, 0, false, false);
        replay
            .play(u64::MAX, |session| {
                ticks += 1;
                let configured = |build| {
                    session
                        .fighters
                        .iter()
                        .any(|fighter| fighter.human().is_some() && fighter.configured == build)
                };
                heavy |= configured(HEAVY);
                modded |= configured(SKIRMISHER);
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
        assert!(
            modded,
            "the second person's modded build request is in the replay"
        );
    }
}
