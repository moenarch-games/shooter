//! The shooter's deathmatch on game-server: [`DeathmatchHost`] is the `GameSimulation` game-server
//! runs for a match. People join a slot (taking it from a bot), every command is one tick of
//! their controls ([`shooter_sim::wire::decode_input`]), and every tick goes out to all clients
//! as one shared snapshot ([`shooter_sim::wire::encode_snapshot`]). [`replay`] records scripted
//! matches in game-server's replay format; [`promote`] turns them into foundation fixtures.

pub mod promote;
pub mod replay;

use game_server::{
    GameSimulation, HostError, MatchHost, MatchId, MatchIdError, MatchRuntime, PlayerId,
    SimulationError, SimulationSnapshot,
};
use shooter_sim::session::DEATHMATCH_SLOTS;
use shooter_sim::wire::{decode_input, encode_snapshot};
use shooter_sim::{Arena, Input, Rules, Session, TICK_HZ};

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
    /// A greybox deathmatch full of bots, seeded `seed`.
    ///
    /// # Errors
    ///
    /// If the arena does not build.
    pub fn new(seed: u64) -> Result<Self, SimulationError> {
        Session::with_rules(Arena::greybox(), Rules::deathmatch(seed))
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
        usize::from(DEATHMATCH_SLOTS)
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
        let (intent, view_tick, name) = decode_input(payload).map_err(failed)?;
        let input = Input {
            sequence,
            intent,
            view_tick,
        };
        if let Some(name) = name {
            self.session.rename(player_id, &name);
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

/// A host serving one deathmatch per ID in `match_ids`, each seeded from its ID.
///
/// # Errors
///
/// If an ID is not a valid game-server match ID, repeats, or the arena does not build.
pub fn build_host(
    match_ids: &[String],
    reconnect_grace_ticks: u64,
) -> Result<MatchHost<DeathmatchHost>, BuildError> {
    let mut host = MatchHost::new(match_ids.len()).map_err(BuildError::Host)?;
    for id in match_ids {
        let route = MatchId::new(id.clone()).map_err(BuildError::MatchId)?;
        let simulation = DeathmatchHost::new(seed_of(id)).map_err(BuildError::Simulation)?;
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
        let mut runtime = MatchRuntime::new(DeathmatchHost::new(seed_of("dm")).unwrap(), 300);
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
            let payload = encode_input(forward, 0, Some("Ripley"));
            runtime
                .submit_command(first.player_id, first.connection_epoch, sequence, &payload)
                .unwrap();
            runtime.advance_tick().unwrap();
        }
        let snapshot = decode_snapshot(&runtime.snapshot().unwrap().payload).unwrap();
        assert_eq!(snapshot.tick, 60);
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

    #[test]
    fn a_host_serves_each_match_id() {
        let host = build_host(&["dm".to_owned(), "dm-2".to_owned()], 120).unwrap();
        assert_eq!(host.len(), 2);
        assert!(build_host(&["bad id!".to_owned()], 120).is_err());
        assert_ne!(seed_of("dm"), seed_of("dm-2"));
    }
}
