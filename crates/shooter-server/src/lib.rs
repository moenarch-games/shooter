//! The shooter's deathmatch on game-server: [`DeathmatchHost`] is the `GameSimulation` game-server
//! runs for a match, on the map the host chose for it ([`MatchSpec`]). People join a slot (taking it from a bot), every command is one tick of
//! their controls ([`shooter_sim::wire::decode_input`], rejected whole when it carries an invalid
//! robot build), and every tick goes out to all clients as one shared snapshot
//! ([`shooter_sim::wire::encode_snapshot`]). [`replay`] records scripted
//! matches in game-server's replay format; [`promote`] turns them into foundation fixtures.

pub mod promote;
pub mod replay;

use game_server::{
    GameSimulation, HostError, MatchHost, MatchId, MatchIdError, MatchRuntime, PlayerId,
    SimulationError, SimulationSnapshot,
};
use shooter_sim::session::DEATHMATCH_SLOTS;
use shooter_sim::wire::{decode_input, encode_snapshot};
use shooter_sim::{Input, MapId, Rules, Session, TICK_HZ, UnknownMap};

/// One hosted deathmatch.
#[derive(Debug)]
pub struct DeathmatchHost {
    session: Session,
}

/// FNV-1a: a match's bot seed from its ID, so a match ID always plays the same bots.
#[must_use]
pub fn seed_of(match_id: &str) -> u64 {
    match_id
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325, |hash: u64, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        })
}

impl DeathmatchHost {
    /// A deathmatch on `map` with `slots` slots, all bots, seeded `seed`.
    ///
    /// # Errors
    ///
    /// If `slots` is 0 or more than the map's [`MapId::max_slots`], or the arena does not build.
    pub fn new(map: MapId, slots: u8, seed: u64) -> Result<Self, SimulationError> {
        if slots == 0 || slots > map.max_slots() {
            return Err(SimulationError::new(format!(
                "{slots} slots on {map} (1 to {})",
                map.max_slots()
            )));
        }
        let rules = Rules {
            slots,
            ..Rules::deathmatch(seed)
        };
        Session::on_map(map, rules)
            .map(|session| Self { session })
            .map_err(failed)
    }

    #[must_use]
    pub const fn session(&self) -> &Session {
        &self.session
    }
}

fn failed(error: impl std::fmt::Display) -> SimulationError {
    SimulationError::new(error.to_string())
}

impl GameSimulation for DeathmatchHost {
    fn tick_hz(&self) -> u16 {
        TICK_HZ
    }

    fn max_players(&self) -> usize {
        usize::from(self.session.rules.slots)
    }

    fn current_tick(&self) -> u64 {
        u64::from(self.session.ticks())
    }

    fn add_player(&mut self, player_id: PlayerId) -> Result<(), SimulationError> {
        match self.session.join(player_id).map_err(failed)? {
            Some(_) => Ok(()),
            None => Err(SimulationError::new("every slot is taken")),
        }
    }

    fn remove_player(&mut self, player_id: PlayerId) -> bool {
        self.try_remove_player(player_id).unwrap_or(false)
    }

    fn try_remove_player(&mut self, player_id: PlayerId) -> Result<bool, SimulationError> {
        self.session.leave(player_id).map_err(failed)
    }

    fn apply_command(
        &mut self,
        player_id: PlayerId,
        sequence: u32,
        payload: &[u8],
    ) -> Result<(), SimulationError> {
        let command = decode_input(payload).map_err(failed)?;
        let input = Input {
            sequence,
            intent: command.intent,
            view_tick: command.view_tick,
        };
        if let Some(name) = command.name {
            self.session.rename(player_id, &name);
        }
        if let Some(build) = command.build {
            self.session.configure(player_id, build);
        }
        if self.session.queue(player_id, input) {
            Ok(())
        } else {
            Err(SimulationError::new("the player holds no slot"))
        }
    }

    fn advance_tick(&mut self) -> Result<(), SimulationError> {
        self.session.step().map(|_| ()).map_err(failed)
    }

    fn snapshot(&self) -> Result<SimulationSnapshot, SimulationError> {
        Ok(SimulationSnapshot::new(
            self.current_tick(),
            encode_snapshot(&self.session.snapshot()),
        ))
    }
}

/// One match a host serves: its route ID, its map and its slot count. Written `<id>` (on
/// [`MapId::DEFAULT`]) or `<id>=<map>`, either followed by `:<slots>` (default
/// [`DEATHMATCH_SLOTS`]; [`build_host`] refuses more than the map's [`MapId::max_slots`]).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatchSpec {
    pub id: String,
    pub map: MapId,
    pub slots: u8,
}

/// A match spec that does not parse.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SpecError {
    Map(UnknownMap),
    /// The text after `:` is not a slot count.
    Slots(String),
}

impl std::fmt::Display for SpecError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Map(error) => error.fmt(formatter),
            Self::Slots(text) => write!(formatter, "{text:?} is no slot count"),
        }
    }
}

impl std::error::Error for SpecError {}

impl std::str::FromStr for MatchSpec {
    type Err = SpecError;

    fn from_str(spec: &str) -> Result<Self, SpecError> {
        let (spec, slots) = match spec.rsplit_once(':') {
            Some((spec, slots)) => (
                spec,
                slots
                    .trim()
                    .parse()
                    .map_err(|_| SpecError::Slots(slots.trim().to_owned()))?,
            ),
            None => (spec, DEATHMATCH_SLOTS),
        };
        let (id, map) = match spec.split_once('=') {
            Some((id, map)) => (id.trim(), map.trim().parse().map_err(SpecError::Map)?),
            None => (spec.trim(), MapId::DEFAULT),
        };
        Ok(Self {
            id: id.to_owned(),
            map,
            slots,
        })
    }
}

#[derive(Debug)]
pub enum BuildError {
    MatchId(MatchIdError),
    Host(HostError),
    Simulation(SimulationError),
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MatchId(error) => error.fmt(formatter),
            Self::Host(error) => error.fmt(formatter),
            Self::Simulation(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for BuildError {}

/// A host serving one deathmatch per spec in `matches`, each on its map and seeded from its ID.
///
/// # Errors
///
/// If an ID is not a valid game-server match ID, repeats, or the arena does not build.
pub fn build_host(
    matches: &[MatchSpec],
    reconnect_grace_ticks: u64,
) -> Result<MatchHost<DeathmatchHost>, BuildError> {
    let mut host = MatchHost::new(matches.len()).map_err(BuildError::Host)?;
    for spec in matches {
        let route = MatchId::new(spec.id.clone()).map_err(BuildError::MatchId)?;
        let simulation = DeathmatchHost::new(spec.map, spec.slots, seed_of(&spec.id))
            .map_err(BuildError::Simulation)?;
        host.insert(route, MatchRuntime::new(simulation, reconnect_grace_ticks))
            .map_err(|failure| BuildError::Host(failure.into_parts().0))?;
    }
    Ok(host)
}

#[cfg(test)]
mod tests {
    use super::*;
    use game_server::{RECONNECT_TOKEN_BYTES, ReconnectToken};
    use shooter_sim::wire::{decode_snapshot, encode_input};
    use shooter_sim::{Controller, Intent};

    #[test]
    fn two_people_join_a_bot_match_and_move() {
        let mut runtime = MatchRuntime::new(
            DeathmatchHost::new(MapId::Greybox, DEATHMATCH_SLOTS, seed_of("dm")).unwrap(),
            300,
        );
        let first = runtime
            .admit(ReconnectToken([1; RECONNECT_TOKEN_BYTES]))
            .unwrap();
        let second = runtime
            .admit(ReconnectToken([2; RECONNECT_TOKEN_BYTES]))
            .unwrap();
        let forward = Intent {
            forward: 1,
            ..Intent::IDLE
        };
        for sequence in 1..=60 {
            let payload = encode_input(forward, 0, Some("Ripley"), None);
            runtime
                .submit_command(first.player_id, first.connection_epoch, sequence, &payload)
                .unwrap();
            runtime.advance_tick().unwrap();
        }
        let snapshot = decode_snapshot(&runtime.snapshot().unwrap().payload).unwrap();
        assert_eq!(snapshot.tick, 60);
        assert_eq!(snapshot.map, Some(MapId::Greybox));
        assert_eq!(snapshot.fighters.len(), usize::from(DEATHMATCH_SLOTS));
        let me = snapshot.of_player(first.player_id).unwrap();
        assert_eq!(me.ack, 60);
        assert_eq!(snapshot.name(me.slot), Some("Ripley"));
        let other = snapshot.of_player(second.player_id).unwrap();
        assert_eq!(snapshot.name(other.slot), None);
        let bots = snapshot
            .fighters
            .iter()
            .filter(|fighter| fighter.controller == Controller::Bot)
            .count();
        assert_eq!(bots, usize::from(DEATHMATCH_SLOTS) - 2);
        assert!(runtime.disconnect(first.player_id, first.connection_epoch));
    }

    fn specs(specs: &[&str]) -> Vec<MatchSpec> {
        specs.iter().map(|spec| spec.parse().unwrap()).collect()
    }

    #[test]
    fn a_host_serves_each_match_id() {
        let host = build_host(&specs(&["dm", "dm-2=greybox"]), 120).unwrap();
        assert_eq!(host.len(), 2);
        assert!(build_host(&specs(&["bad id!"]), 120).is_err());
        assert_ne!(seed_of("dm"), seed_of("dm-2"));
        let max = MapId::Greybox.max_slots();
        assert!(build_host(&specs(&["duel:2", &format!("big:{max}")]), 120).is_ok());
        assert!(build_host(&specs(&["none:0"]), 120).is_err());
        assert!(build_host(&specs(&[&format!("crowd:{}", max + 1)]), 120).is_err());
    }

    #[test]
    fn a_match_takes_its_slot_count() {
        for slots in [2, 6, DEATHMATCH_SLOTS] {
            let host = DeathmatchHost::new(MapId::Greybox, slots, seed_of("dm")).unwrap();
            assert_eq!(host.max_players(), usize::from(slots));
            assert_eq!(host.session().fighters.len(), usize::from(slots));
        }
    }

    #[test]
    fn a_match_spec_names_its_map_and_slots() {
        let spec = |text: &str| text.parse::<MatchSpec>();
        assert_eq!(
            spec("dm"),
            Ok(MatchSpec {
                id: "dm".to_owned(),
                map: MapId::DEFAULT,
                slots: DEATHMATCH_SLOTS,
            })
        );
        assert_eq!(spec(" arena = greybox ").unwrap().map, MapId::Greybox);
        assert_eq!(spec("dm=greybox:8").unwrap().slots, 8);
        assert_eq!(
            spec("dm:4"),
            Ok(MatchSpec {
                id: "dm".to_owned(),
                map: MapId::DEFAULT,
                slots: 4,
            })
        );
        assert_eq!(
            spec("dm=nowhere"),
            Err(SpecError::Map(UnknownMap("nowhere".to_owned())))
        );
        assert_eq!(spec("dm:many"), Err(SpecError::Slots("many".to_owned())));
        assert!(spec("dm=").is_err());
        assert!(spec("dm:").is_err());
    }
}
