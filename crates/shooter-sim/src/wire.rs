//! The match on the wire, inside game-server's command and snapshot frames: a person's input
//! (one per tick) and the shared snapshot every client gets each tick (the fighters with the
//! exact physics state a client resumes prediction from and their robot builds, the rockets, the
//! shotgun pellets and the tick's events). Robot builds travel as raw codes and are decoded
//! through [`crate::robot::Catalog::validate`], so a tampered build is rejected, not stored.
//! Little-endian; positions and velocities travel as exact `f32` bits.
//!
//! The in-process practice match builds the same [`Snapshot`], so both modes draw through one
//! path.

use crate::grapple::Hook;
use crate::player::{Intent, Motion};
use crate::robot::{Assembly, AssemblyError, BETA, Candidate};
use crate::session::{Event, Fighter, Phase, Pilot, Session};
use crate::weapons::Weapon;
use std::fmt;

pub const WIRE_VERSION: u8 = 7;
const NONE: u8 = u8::MAX;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WireError {
    Malformed(&'static str),
    /// A well-formed robot build that breaks a build rule.
    Build(AssemblyError),
}

impl fmt::Display for WireError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(reason) => write!(formatter, "malformed shooter message: {reason}"),
            Self::Build(error) => write!(formatter, "rejected robot build: {error}"),
        }
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
    /// The robot it is built as this life.
    pub build: Assembly,
    /// The build it comes back as at its next spawn.
    pub configured: Assembly,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RocketState {
    pub position: [f32; 3],
    pub age: u32,
}

/// A shotgun pellet in flight: this tick's stretch of its path, for drawing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PelletState {
    pub from: [f32; 3],
    pub to: [f32; 3],
    /// It was fired this tick (`from` is the shooter's eye).
    pub launched: bool,
}

/// The whole match after a tick.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub tick: u32,
    pub phase: Phase,
    pub frag_limit: i32,
    pub fighters: Vec<FighterState>,
    pub rockets: Vec<RocketState>,
    pub pellets: Vec<PelletState>,
    pub events: Vec<Event>,
    /// The names people picked, by slot (slots without one show as "Player N").
    pub names: Vec<(u8, String)>,
}

impl Snapshot {
    /// The name `slot`'s person picked.
    #[must_use]
    pub fn name(&self, slot: u8) -> Option<&str> {
        self.names
            .iter()
            .find(|(named, _)| *named == slot)
            .map(|(_, name)| name.as_str())
    }

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
        build: fighter.build,
        configured: fighter.configured,
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
            pellets: self
                .pellets
                .iter()
                .map(|pellet| PelletState {
                    from: pellet.from_units(),
                    to: pellet.position_units(),
                    launched: pellet.age == 1,
                })
                .collect(),
            events: self.events.clone(),
            names: self
                .fighters
                .iter()
                .filter(|fighter| fighter.human().is_some())
                .filter_map(|fighter| Some((fighter.slot, fighter.name.clone()?)))
                .collect(),
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
    /// A robot build in canonical order: a count and `(socket, component)` codes, then a count
    /// and `(hardpoint, weapon)` codes.
    fn build(&mut self, build: &Assembly) {
        let candidate = build.candidate();
        for pairs in [&candidate.parts, &candidate.mounts] {
            self.u8(count(pairs.len()));
            for &(at, code) in pairs {
                self.u8(at);
                self.u8(code);
            }
        }
    }
    /// A length byte, then up to 255 bytes of UTF-8 (cut at a character boundary).
    fn text(&mut self, text: &str) {
        let mut end = text.len().min(usize::from(u8::MAX));
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        self.u8(count(end));
        self.0.extend_from_slice(&text.as_bytes()[..end]);
    }
}

struct In<'a>(&'a [u8]);

impl In<'_> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], WireError> {
        let (head, rest) = self
            .0
            .split_first_chunk::<N>()
            .ok_or(WireError::Malformed("truncated"))?;
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
        Weapon::from_index(self.u8()?).ok_or(WireError::Malformed("unknown weapon"))
    }
    fn text(&mut self) -> Result<String, WireError> {
        let length = usize::from(self.u8()?);
        if self.0.len() < length {
            return Err(WireError::Malformed("truncated"));
        }
        let (text, rest) = self.0.split_at(length);
        self.0 = rest;
        String::from_utf8(text.to_vec()).map_err(|_| WireError::Malformed("text is not UTF-8"))
    }
    /// Inverse of `Out::build`, validated against [`BETA`].
    fn build(&mut self) -> Result<Assembly, WireError> {
        let mut pairs = || {
            (0..self.u8()?)
                .map(|_| Ok((self.u8()?, self.u8()?)))
                .collect::<Result<Vec<_>, WireError>>()
        };
        let parts = pairs()?;
        let mounts = pairs()?;
        BETA.validate(&Candidate { parts, mounts })
            .map_err(WireError::Build)
    }
    fn finish(self) -> Result<(), WireError> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(WireError::Malformed("trailing bytes"))
        }
    }
}

const JUMP: u8 = 1;
const SPRINT: u8 = 2;
const CROUCH: u8 = 4;
const FIRE: u8 = 8;
const MELEE: u8 = 16;

/// A person's command as the host decoded it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Command {
    pub intent: Intent,
    /// The snapshot tick it was made against.
    pub view_tick: u32,
    /// The name as sent (the host sanitizes it).
    pub name: Option<String>,
    /// The robot build the person asks to come back as, already validated.
    pub build: Option<Assembly>,
}

/// One tick of a person's controls and the snapshot tick it was made against, and the name the
/// person picked and the build they configured while the host has not shown them yet (datagrams
/// are unreliable, so the client repeats them until a snapshot does).
#[must_use]
pub fn encode_input(
    intent: Intent,
    view_tick: u32,
    name: Option<&str>,
    build: Option<&Assembly>,
) -> Vec<u8> {
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
    out.text(name.unwrap_or_default());
    match build {
        Some(build) => {
            out.u8(1);
            out.build(build);
        }
        None => out.u8(0),
    }
    out.0
}

/// Inverse of [`encode_input`]. The build goes through [`crate::robot::Catalog::validate`]: a
/// client's build is untrusted.
///
/// # Errors
///
/// [`WireError::Build`] for a well-formed build that breaks a build rule, else
/// [`WireError::Malformed`].
pub fn decode_input(bytes: &[u8]) -> Result<Command, WireError> {
    let mut input = In(bytes);
    if input.u8()? != WIRE_VERSION {
        return Err(WireError::Malformed("unknown version"));
    }
    let forward = i8::from_le_bytes(input.take()?);
    let strafe = i8::from_le_bytes(input.take()?);
    let yaw = input.u16()?;
    let pitch = i16::from_le_bytes(input.take()?);
    let flags = input.u8()?;
    let select = match input.option()? {
        Some(index) => {
            Some(Weapon::from_index(index).ok_or(WireError::Malformed("unknown weapon"))?)
        }
        None => None,
    };
    let view_tick = input.u32()?;
    let name = Some(input.text()?).filter(|name| !name.is_empty());
    let build = match input.u8()? {
        0 => None,
        1 => Some(input.build()?),
        _ => return Err(WireError::Malformed("unknown build flag")),
    };
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
    Ok(Command {
        intent,
        view_tick,
        name,
        build,
    })
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
        Event::Pellet {
            from,
            at,
            normal,
            launched,
        } => {
            out.u8(8);
            out.vec3(from);
            out.vec3(at);
            out.vec3(normal);
            out.u8(u8::from(launched));
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
                Some(index) => {
                    Some(Weapon::from_index(index).ok_or(WireError::Malformed("unknown weapon"))?)
                }
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
        8 => Event::Pellet {
            from: input.vec3()?,
            at: input.vec3()?,
            normal: input.vec3()?,
            launched: input.u8()? != 0,
        },
        _ => return Err(WireError::Malformed("unknown event")),
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
        out.build(&fighter.build);
        if fighter.configured == fighter.build {
            out.u8(0);
        } else {
            out.u8(1);
            out.build(&fighter.configured);
        }
    }
    out.u8(count(snapshot.rockets.len()));
    for rocket in snapshot.rockets.iter().take(usize::from(u8::MAX)) {
        out.vec3(rocket.position);
        out.u32(rocket.age);
    }
    out.u8(count(snapshot.pellets.len()));
    for pellet in snapshot.pellets.iter().take(usize::from(u8::MAX)) {
        out.vec3(pellet.from);
        out.vec3(pellet.to);
        out.u8(u8::from(pellet.launched));
    }
    out.u8(count(snapshot.events.len()));
    for event in snapshot.events.iter().take(usize::from(u8::MAX)) {
        encode_event(&mut out, event);
    }
    out.u8(count(snapshot.names.len()));
    for (slot, name) in snapshot.names.iter().take(usize::from(u8::MAX)) {
        out.u8(*slot);
        out.text(name);
    }
    out.0
}

/// Inverse of [`encode_snapshot`].
pub fn decode_snapshot(bytes: &[u8]) -> Result<Snapshot, WireError> {
    let mut input = In(bytes);
    if input.u8()? != WIRE_VERSION {
        return Err(WireError::Malformed("unknown version"));
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
        _ => return Err(WireError::Malformed("unknown phase")),
    };
    let frag_limit = input.i32()?;
    let fighters = (0..input.u8()?)
        .map(|_| {
            let mut fighter = FighterState {
                slot: input.u8()?,
                controller: match input.u8()? {
                    1 => Controller::Human(input.u32()?),
                    2 => Controller::Bot,
                    _ => return Err(WireError::Malformed("unknown controller")),
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
                    _ => return Err(WireError::Malformed("unknown hook state")),
                },
                respawn_in: input.u32()?,
                ack: input.u32()?,
                build: input.build()?,
                configured: Assembly::default(),
            };
            fighter.configured = match input.u8()? {
                0 => fighter.build,
                1 => input.build()?,
                _ => return Err(WireError::Malformed("unknown build state")),
            };
            Ok(fighter)
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
    let pellets = (0..input.u8()?)
        .map(|_| {
            Ok(PelletState {
                from: input.vec3()?,
                to: input.vec3()?,
                launched: input.u8()? != 0,
            })
        })
        .collect::<Result<Vec<_>, WireError>>()?;
    let events = (0..input.u8()?)
        .map(|_| decode_event(&mut input))
        .collect::<Result<Vec<_>, _>>()?;
    let names = (0..input.u8()?)
        .map(|_| Ok((input.u8()?, input.text()?)))
        .collect::<Result<Vec<_>, WireError>>()?;
    input.finish()?;
    Ok(Snapshot {
        tick,
        phase,
        frag_limit,
        fighters,
        rockets,
        pellets,
        events,
        names,
    })
}
