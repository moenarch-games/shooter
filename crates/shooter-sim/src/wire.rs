//! The match on the wire, inside game-server's command and snapshot frames: a person's input
//! (one per tick) and the shared snapshot every client gets each tick (the fighters with the
//! exact physics state a client resumes prediction from, the rockets and the tick's events).
//! Little-endian; positions and velocities travel as exact `f32` bits.
//!
//! The in-process practice match builds the same [`Snapshot`], so both modes draw through one
//! path.

use crate::grapple::Hook;
use crate::player::{Intent, Motion};
use crate::session::{Event, Fighter, Phase, Pilot, Session};
use crate::weapons::Weapon;
use std::fmt;

pub const WIRE_VERSION: u8 = 3;
const NONE: u8 = u8::MAX;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WireError(&'static str);

impl fmt::Display for WireError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "malformed shooter message: {}", self.0)
    }
}

impl std::error::Error for WireError {}

/// Who controls a slot, as clients see it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Controller {
    Human(u32),
    Bot,
}

/// One occupied slot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FighterState {
    pub slot: u8,
    pub controller: Controller,
    pub alive: bool,
    pub health: i32,
    pub frags: i32,
    pub deaths: i32,
    pub held: Weapon,
    pub yaw: u16,
    pub pitch: i16,
    /// Exact body centre, units (zero while dead).
    pub position: [f32; 3],
    /// Exact velocity, units per tick.
    pub velocity: [f32; 3],
    pub motion: Motion,
    /// Its grapple hook while set (exact, for prediction and the cable).
    pub hook: Option<Hook>,
    /// Ticks until it respawns, while dead.
    pub respawn_in: u32,
    /// The last input sequence applied (people only).
    pub ack: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RocketState {
    pub position: [f32; 3],
    pub age: u32,
}

/// The whole match after a tick.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub tick: u32,
    pub phase: Phase,
    pub frag_limit: i32,
    pub fighters: Vec<FighterState>,
    pub rockets: Vec<RocketState>,
    pub events: Vec<Event>,
}

impl Snapshot {
    /// The fighter person `id` plays.
    #[must_use]
    pub fn of_player(&self, id: u32) -> Option<&FighterState> {
        self.fighters
            .iter()
            .find(|fighter| fighter.controller == Controller::Human(id))
    }

    #[must_use]
    pub fn slot(&self, slot: u8) -> Option<&FighterState> {
        self.fighters.iter().find(|fighter| fighter.slot == slot)
    }
}

fn state_of(session: &Session, fighter: &Fighter) -> Option<FighterState> {
    let controller = match fighter.pilot {
        Pilot::Open => return None,
        Pilot::Human { id } => Controller::Human(id),
        Pilot::Bot(_) => Controller::Bot,
    };
    let body = session
        .world
        .body(fighter.player.body)
        .filter(|_| fighter.alive());
    Some(FighterState {
        slot: fighter.slot,
        controller,
        alive: fighter.alive(),
        health: fighter.health,
        frags: fighter.frags,
        deaths: fighter.deaths,
        held: fighter.arsenal.held,
        yaw: fighter.intent.yaw,
        pitch: fighter.intent.pitch,
        position: body.map_or([0.0; 3], |body| body.exact_position()),
        velocity: body.map_or([0.0; 3], |body| body.exact_velocity()),
        motion: fighter.player.motion,
        hook: fighter.hook,
        respawn_in: fighter
            .respawn_at
            .map_or(0, |at| at.saturating_sub(session.ticks())),
        ack: fighter.ack,
    })
}

impl Session {
    /// The match as clients see it after the last tick.
    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            tick: self.ticks(),
            phase: self.phase,
            frag_limit: self.rules.frag_limit,
            fighters: self
                .fighters
                .iter()
                .filter_map(|fighter| state_of(self, fighter))
                .collect(),
            rockets: self
                .rockets
                .iter()
                .map(|rocket| RocketState {
                    position: rocket.position_units(),
                    age: rocket.age,
                })
                .collect(),
            events: self.events.clone(),
        }
    }
}

#[derive(Default)]
struct Out(Vec<u8>);

impl Out {
    fn u8(&mut self, value: u8) {
        self.0.push(value);
    }
    fn u16(&mut self, value: u16) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }
    fn u32(&mut self, value: u32) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }
    fn i32(&mut self, value: i32) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }
    fn vec3(&mut self, value: [f32; 3]) {
        for axis in value {
            self.u32(axis.to_bits());
        }
    }
    fn option(&mut self, value: Option<u8>) {
        self.u8(value.unwrap_or(NONE));
    }
    fn weapon(&mut self, weapon: Weapon) {
        self.u8(weapon.index());
    }
}

struct In<'a>(&'a [u8]);

impl In<'_> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], WireError> {
        let (head, rest) = self
            .0
            .split_first_chunk::<N>()
            .ok_or(WireError("truncated"))?;
        self.0 = rest;
        Ok(*head)
    }
    fn u8(&mut self) -> Result<u8, WireError> {
        Ok(self.take::<1>()?[0])
    }
    fn u16(&mut self) -> Result<u16, WireError> {
        Ok(u16::from_le_bytes(self.take()?))
    }
    fn u32(&mut self) -> Result<u32, WireError> {
        Ok(u32::from_le_bytes(self.take()?))
    }
    fn i32(&mut self) -> Result<i32, WireError> {
        Ok(i32::from_le_bytes(self.take()?))
    }
    fn vec3(&mut self) -> Result<[f32; 3], WireError> {
        Ok([
            f32::from_bits(self.u32()?),
            f32::from_bits(self.u32()?),
            f32::from_bits(self.u32()?),
        ])
    }
    fn option(&mut self) -> Result<Option<u8>, WireError> {
        Ok(Some(self.u8()?).filter(|value| *value != NONE))
    }
    fn weapon(&mut self) -> Result<Weapon, WireError> {
        Weapon::from_index(self.u8()?).ok_or(WireError("unknown weapon"))
    }
    fn finish(self) -> Result<(), WireError> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(WireError("trailing bytes"))
        }
    }
}

const JUMP: u8 = 1;
const SPRINT: u8 = 2;
const CROUCH: u8 = 4;
const FIRE: u8 = 8;
const MELEE: u8 = 16;

/// One tick of a person's controls and the snapshot tick it was made against.
#[must_use]
pub fn encode_input(intent: Intent, view_tick: u32) -> Vec<u8> {
    let mut out = Out::default();
    out.u8(WIRE_VERSION);
    out.0.push(intent.forward.to_le_bytes()[0]);
    out.0.push(intent.strafe.to_le_bytes()[0]);
    out.u16(intent.yaw);
    out.0.extend_from_slice(&intent.pitch.to_le_bytes());
    out.u8([
        (intent.jump, JUMP),
        (intent.sprint, SPRINT),
        (intent.crouch, CROUCH),
        (intent.fire, FIRE),
        (intent.melee, MELEE),
    ]
    .iter()
    .filter(|(held, _)| *held)
    .fold(0, |flags, (_, flag)| flags | flag));
    out.option(intent.select.map(Weapon::index));
    out.u32(view_tick);
    out.0
}

/// Inverse of [`encode_input`].
pub fn decode_input(bytes: &[u8]) -> Result<(Intent, u32), WireError> {
    let mut input = In(bytes);
    if input.u8()? != WIRE_VERSION {
        return Err(WireError("unknown version"));
    }
    let forward = i8::from_le_bytes(input.take()?);
    let strafe = i8::from_le_bytes(input.take()?);
    let yaw = input.u16()?;
    let pitch = i16::from_le_bytes(input.take()?);
    let flags = input.u8()?;
    let select = match input.option()? {
        Some(index) => Some(Weapon::from_index(index).ok_or(WireError("unknown weapon"))?),
        None => None,
    };
    let view_tick = input.u32()?;
    input.finish()?;
    let intent = Intent {
        forward: forward.clamp(-1, 1),
        strafe: strafe.clamp(-1, 1),
        yaw,
        pitch: pitch.clamp(-crate::weapons::MAX_PITCH, crate::weapons::MAX_PITCH),
        jump: flags & JUMP != 0,
        sprint: flags & SPRINT != 0,
        crouch: flags & CROUCH != 0,
        fire: flags & FIRE != 0,
        melee: flags & MELEE != 0,
        select,
    };
    Ok((intent, view_tick))
}

fn encode_event(out: &mut Out, event: &Event) {
    match *event {
        Event::Fired {
            slot,
            weapon,
            eye,
            direction,
        } => {
            out.u8(0);
            out.u8(slot);
            out.weapon(weapon);
            out.vec3(eye);
            out.vec3(direction);
        }
        Event::Beam {
            weapon,
            from,
            end,
            normal,
            struck,
        } => {
            out.u8(1);
            out.weapon(weapon);
            out.vec3(from);
            out.vec3(end);
            out.vec3(normal);
            out.u8(u8::from(struck));
        }
        Event::Exploded { at } => {
            out.u8(2);
            out.vec3(at);
        }
        Event::Damaged {
            dummy,
            at,
            damage,
            downed,
        } => {
            out.u8(3);
            out.u32(u32::try_from(dummy).unwrap_or(u32::MAX));
            out.vec3(at);
            out.i32(damage);
            out.u8(u8::from(downed));
        }
        Event::Hit {
            victim,
            attacker,
            damage,
            at,
        } => {
            out.u8(4);
            out.u8(victim);
            out.option(attacker);
            out.i32(damage);
            out.vec3(at);
        }
        Event::Killed {
            victim,
            killer,
            weapon,
        } => {
            out.u8(5);
            out.u8(victim);
            out.option(killer);
            out.option(weapon.map(Weapon::index));
        }
        Event::Spawned { slot, yaw } => {
            out.u8(6);
            out.u8(slot);
            out.u16(yaw);
        }
        Event::RoundOver { winner } => {
            out.u8(7);
            out.option(winner);
        }
    }
}

fn decode_event(input: &mut In<'_>) -> Result<Event, WireError> {
    Ok(match input.u8()? {
        0 => Event::Fired {
            slot: input.u8()?,
            weapon: input.weapon()?,
            eye: input.vec3()?,
            direction: input.vec3()?,
        },
        1 => Event::Beam {
            weapon: input.weapon()?,
            from: input.vec3()?,
            end: input.vec3()?,
            normal: input.vec3()?,
            struck: input.u8()? != 0,
        },
        2 => Event::Exploded { at: input.vec3()? },
        3 => Event::Damaged {
            dummy: input.u32()? as usize,
            at: input.vec3()?,
            damage: input.i32()?,
            downed: input.u8()? != 0,
        },
        4 => Event::Hit {
            victim: input.u8()?,
            attacker: input.option()?,
            damage: input.i32()?,
            at: input.vec3()?,
        },
        5 => Event::Killed {
            victim: input.u8()?,
            killer: input.option()?,
            weapon: match input.option()? {
                Some(index) => Some(Weapon::from_index(index).ok_or(WireError("unknown weapon"))?),
                None => None,
            },
        },
        6 => Event::Spawned {
            slot: input.u8()?,
            yaw: input.u16()?,
        },
        7 => Event::RoundOver {
            winner: input.option()?,
        },
        _ => return Err(WireError("unknown event")),
    })
}

fn count(value: usize) -> u8 {
    u8::try_from(value).unwrap_or(u8::MAX)
}

/// The snapshot payload game-server sends every client each tick.
#[must_use]
pub fn encode_snapshot(snapshot: &Snapshot) -> Vec<u8> {
    let mut out = Out::default();
    out.u8(WIRE_VERSION);
    out.u32(snapshot.tick);
    match snapshot.phase {
        Phase::Playing { ends_at } => {
            out.u8(0);
            out.u32(ends_at.unwrap_or(u32::MAX));
        }
        Phase::Intermission {
            winner,
            restarts_at,
        } => {
            out.u8(1);
            out.option(winner);
            out.u32(restarts_at);
        }
    }
    out.i32(snapshot.frag_limit);
    out.u8(count(snapshot.fighters.len()));
    for fighter in snapshot.fighters.iter().take(usize::from(u8::MAX)) {
        out.u8(fighter.slot);
        match fighter.controller {
            Controller::Human(id) => {
                out.u8(1);
                out.u32(id);
            }
            Controller::Bot => out.u8(2),
        }
        out.u8(u8::from(fighter.alive));
        out.i32(fighter.health);
        out.i32(fighter.frags);
        out.i32(fighter.deaths);
        out.weapon(fighter.held);
        out.u16(fighter.yaw);
        out.0.extend_from_slice(&fighter.pitch.to_le_bytes());
        out.vec3(fighter.position);
        out.vec3(fighter.velocity);
        out.u8(fighter.motion.to_bits());
        match fighter.hook {
            Some(hook) => {
                out.u8(1);
                hook.anchor.into_iter().for_each(|axis| out.i32(axis));
                out.i32(hook.length);
            }
            None => out.u8(0),
        }
        out.u32(fighter.respawn_in);
        out.u32(fighter.ack);
    }
    out.u8(count(snapshot.rockets.len()));
    for rocket in snapshot.rockets.iter().take(usize::from(u8::MAX)) {
        out.vec3(rocket.position);
        out.u32(rocket.age);
    }
    out.u8(count(snapshot.events.len()));
    for event in snapshot.events.iter().take(usize::from(u8::MAX)) {
        encode_event(&mut out, event);
    }
    out.0
}

/// Inverse of [`encode_snapshot`].
pub fn decode_snapshot(bytes: &[u8]) -> Result<Snapshot, WireError> {
    let mut input = In(bytes);
    if input.u8()? != WIRE_VERSION {
        return Err(WireError("unknown version"));
    }
    let tick = input.u32()?;
    let phase = match input.u8()? {
        0 => Phase::Playing {
            ends_at: Some(input.u32()?).filter(|at| *at != u32::MAX),
        },
        1 => Phase::Intermission {
            winner: input.option()?,
            restarts_at: input.u32()?,
        },
        _ => return Err(WireError("unknown phase")),
    };
    let frag_limit = input.i32()?;
    let fighters = (0..input.u8()?)
        .map(|_| {
            Ok(FighterState {
                slot: input.u8()?,
                controller: match input.u8()? {
                    1 => Controller::Human(input.u32()?),
                    2 => Controller::Bot,
                    _ => return Err(WireError("unknown controller")),
                },
                alive: input.u8()? != 0,
                health: input.i32()?,
                frags: input.i32()?,
                deaths: input.i32()?,
                held: input.weapon()?,
                yaw: input.u16()?,
                pitch: i16::from_le_bytes(input.take()?),
                position: input.vec3()?,
                velocity: input.vec3()?,
                motion: Motion::from_bits(input.u8()?),
                hook: match input.u8()? {
                    0 => None,
                    1 => Some(Hook {
                        anchor: [input.i32()?, input.i32()?, input.i32()?],
                        length: input.i32()?,
                    }),
                    _ => return Err(WireError("unknown hook state")),
                },
                respawn_in: input.u32()?,
                ack: input.u32()?,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let rockets = (0..input.u8()?)
        .map(|_| {
            Ok(RocketState {
                position: input.vec3()?,
                age: input.u32()?,
            })
        })
        .collect::<Result<Vec<_>, WireError>>()?;
    let events = (0..input.u8()?)
        .map(|_| decode_event(&mut input))
        .collect::<Result<Vec<_>, _>>()?;
    input.finish()?;
    Ok(Snapshot {
        tick,
        phase,
        frag_limit,
        fighters,
        rockets,
        events,
    })
}
