//! The match on the wire, inside game-server's command and snapshot frames: a person's input
//! (one per tick) and the shared snapshot every client gets each tick (the match's map, the
//! fighters with the
//! exact physics state a client resumes prediction from, their robot builds, the charge in each
//! one's energy pool, every part's health (what it has lost follows from them,
//! [`crate::damage`]) and the module each carries, the rockets, the shotgun pellets, the module
//! pads and the tick's events). Robot builds travel as raw codes and are decoded
//! through [`crate::robot::Catalog::validate`], so a tampered build is rejected, not stored; a
//! part health above its catalog health, or a charge above the capacity of the parts still
//! connected, is malformed.
//! Little-endian; positions and velocities travel as exact `f32` bits.
//!
//! The in-process practice match builds the same [`Snapshot`], so both modes draw through one
//! path.

use crate::damage::PartDamage;
use crate::energy::Pool;
use crate::grapple::Hook;
use crate::maps::MapId;
use crate::mechanics::{
    Command as MechCommand, Controls, Gait, GaitChoice, Layout, RobotState, State as MechState,
};
use crate::physics::Vec3i;
use crate::physics::articulated::{Motion as BodyMotion, Pose};
use crate::pickups::{ArenaModule, Carried};
use crate::player::{Intent, Motion};
use crate::robot::{Assembly, AssemblyError, BETA, Candidate, PartId, RawMount, RawPart};
use crate::session::{Event, Fighter, Phase, Pilot, Session};
use crate::weapons::Weapon;
use std::fmt;

pub const WIRE_VERSION: u8 = 13;
const NONE: u8 = u8::MAX;

/// A build part's flags: it has a parent, it is turned, it carries a mod.
const MOUNTED: u8 = 1;
const TURNED: u8 = 2;
const MODDED: u8 = 4;

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
    /// The module pickup it carries and the uses it has left.
    pub carried: Option<Carried>,
    /// The charge in its shared energy pool, milli-units ([`crate::energy`]); the pool of its
    /// build's connected parts ([`crate::energy::Pool::attached`]) has the capacity.
    pub energy: u32,
    /// Its part healths this life and, following from them, the parts it has lost
    /// ([`crate::damage`]); the core's health is its life.
    pub parts: PartDamage,
}

impl FighterState {
    /// Its core's health.
    #[must_use]
    pub fn core_health(&self) -> u16 {
        self.parts.core_health(&self.build)
    }
}

/// A module pad: where it lies (units, the map's anchor), its module and the ticks until the
/// module is back (0: ready to take).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PickupState {
    pub center: Vec3i,
    pub module: ArenaModule,
    pub ready_in: u32,
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
    /// The catalogued map the match is played on (`None`: a scripted arena outside the
    /// catalog); a client builds its arena from it ([`MapId::arena`]).
    pub map: Option<MapId>,
    pub phase: Phase,
    pub frag_limit: i32,
    pub fighters: Vec<FighterState>,
    pub rockets: Vec<RocketState>,
    pub pellets: Vec<PelletState>,
    /// The module pads, in the map's order.
    pub pickups: Vec<PickupState>,
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
        carried: fighter.carried,
        energy: fighter.energy,
        parts: fighter.parts,
    })
}

impl Session {
    /// The match as clients see it after the last tick.
    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            tick: self.ticks(),
            map: self.map,
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
            pickups: self
                .pickups
                .iter()
                .map(|pickup| PickupState {
                    center: pickup.pad.center,
                    module: pickup.pad.module,
                    ready_in: pickup.ready_in,
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
    fn module(&mut self, module: ArenaModule) {
        self.u8(module.tag());
    }
    /// A robot build's component tree in canonical (part ID) order: a part count, then per part
    /// its ID, its part type code and a flags byte, then its parent's ID and socket if it is
    /// mounted, its orientation's three angles (u16 each) if it is turned, and its mod's code if
    /// it carries one.
    fn build(&mut self, build: &Assembly) {
        let candidate = build.candidate();
        self.u8(count(candidate.parts.len()));
        for part in &candidate.parts {
            self.u8(part.id);
            self.u8(part.kind);
            let turned = part
                .mount
                .filter(|mount| (mount.yaw, mount.pitch, mount.roll) != (0, 0, 0));
            self.u8([
                (part.mount.is_some(), MOUNTED),
                (turned.is_some(), TURNED),
                (part.modification.is_some(), MODDED),
            ]
            .iter()
            .filter(|(set, _)| *set)
            .fold(0, |flags, (_, flag)| flags | flag));
            if let Some(mount) = part.mount {
                self.u8(mount.parent);
                self.u8(mount.socket);
            }
            if let Some(mount) = turned {
                self.u16(mount.yaw);
                self.u16(mount.pitch);
                self.u16(mount.roll);
            }
            if let Some(code) = part.modification {
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
    fn module(&mut self) -> Result<ArenaModule, WireError> {
        ArenaModule::from_tag(self.u8()?).ok_or(WireError::Malformed("unknown module"))
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
        let parts = (0..self.u8()?)
            .map(|_| {
                let id = self.u8()?;
                let kind = self.u8()?;
                let flags = self.u8()?;
                if flags & !(MOUNTED | TURNED | MODDED) != 0 {
                    return Err(WireError::Malformed("unknown part flags"));
                }
                let mut mount = if flags & MOUNTED == 0 {
                    None
                } else {
                    Some(RawMount {
                        parent: self.u8()?,
                        socket: self.u8()?,
                        ..RawMount::default()
                    })
                };
                if flags & TURNED != 0 {
                    let mount = mount
                        .as_mut()
                        .ok_or(WireError::Malformed("a root part turned"))?;
                    mount.yaw = self.u16()?;
                    mount.pitch = self.u16()?;
                    mount.roll = self.u16()?;
                }
                let modification = if flags & MODDED == 0 {
                    None
                } else {
                    Some(self.u8()?)
                };
                Ok(RawPart {
                    id,
                    kind,
                    mount,
                    modification,
                })
            })
            .collect::<Result<Vec<_>, WireError>>()?;
        BETA.validate(&Candidate { parts })
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
            part,
            damage,
            at,
        } => {
            out.u8(4);
            out.u8(victim);
            out.option(attacker);
            out.u8(part.0);
            out.i32(damage);
            out.vec3(at);
        }
        Event::Detached {
            slot,
            part,
            destroyed,
        } => {
            out.u8(11);
            out.u8(slot);
            out.u8(part.0);
            out.u8(u8::from(destroyed));
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
        Event::ModuleTaken { slot, pad, module } => {
            out.u8(9);
            out.u8(slot);
            out.u8(pad);
            out.module(module);
        }
        Event::ModuleSpent { slot, module } => {
            out.u8(10);
            out.u8(slot);
            out.module(module);
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
            part: PartId(input.u8()?),
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
        9 => Event::ModuleTaken {
            slot: input.u8()?,
            pad: input.u8()?,
            module: input.module()?,
        },
        10 => Event::ModuleSpent {
            slot: input.u8()?,
            module: input.module()?,
        },
        11 => Event::Detached {
            slot: input.u8()?,
            part: PartId(input.u8()?),
            destroyed: input.u8()? != 0,
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
    out.u8(snapshot.map.map_or(0, MapId::tag));
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
        match fighter.carried {
            Some(carried) => {
                out.u8(1);
                out.module(carried.module);
                out.u8(carried.uses_left);
            }
            None => out.u8(0),
        }
        out.u32(fighter.energy);
        for health in fighter.parts.health() {
            out.u16(*health);
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
    out.u8(count(snapshot.pickups.len()));
    for pickup in snapshot.pickups.iter().take(usize::from(u8::MAX)) {
        for axis in [pickup.center.x, pickup.center.y, pickup.center.z] {
            out.i32(axis);
        }
        out.module(pickup.module);
        out.u32(pickup.ready_in);
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
    let map = match input.u8()? {
        0 => None,
        tag => Some(MapId::from_tag(tag).ok_or(WireError::Malformed("unknown map"))?),
    };
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
                carried: None,
                energy: 0,
                parts: PartDamage::default(),
            };
            fighter.configured = match input.u8()? {
                0 => fighter.build,
                1 => input.build()?,
                _ => return Err(WireError::Malformed("unknown build state")),
            };
            fighter.carried = match input.u8()? {
                0 => None,
                1 => Some(Carried {
                    module: input.module()?,
                    uses_left: input.u8()?,
                }),
                _ => return Err(WireError::Malformed("unknown module state")),
            };
            fighter.energy = input.u32()?;
            let health = (0..fighter.build.parts().len())
                .map(|_| input.u16())
                .collect::<Result<Vec<_>, _>>()?;
            fighter.parts =
                PartDamage::from_health(&fighter.build, &health).map_err(WireError::Malformed)?;
            if fighter.energy > Pool::attached(&fighter.build, &fighter.parts).capacity {
                return Err(WireError::Malformed("energy above the build's capacity"));
            }
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
    let pickups = (0..input.u8()?)
        .map(|_| {
            Ok(PickupState {
                center: Vec3i::new(input.i32()?, input.i32()?, input.i32()?),
                module: input.module()?,
                ready_in: input.u32()?,
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
        map,
        phase,
        frag_limit,
        fighters,
        rockets,
        pellets,
        pickups,
        events,
        names,
    })
}

const GAIT_SUGGESTED: u8 = 0;
const GAIT_BIPED: u8 = 1;
const GAIT_QUADRUPED: u8 = 2;
const GAIT_UNISON: u8 = 3;
const COMMAND_RATE: u8 = 1;
const COMMAND_ANGLE: u8 = 2;
const COMMAND_STRIDE: u8 = 3;

/// A robot's mechanical controls for one tick ([`crate::mechanics::Controls`]): the gait
/// choice, then the count of commanded parts (a `u16`) and, for each in part-ID order, its ID, the
/// command's kind and its value (an `i16`).
#[must_use]
pub fn encode_controls(controls: &Controls) -> Vec<u8> {
    let mut out = Out(Vec::new());
    out.u8(match controls.gait {
        GaitChoice::Suggested => GAIT_SUGGESTED,
        GaitChoice::Preset(Gait::Biped) => GAIT_BIPED,
        GaitChoice::Preset(Gait::Quadruped) => GAIT_QUADRUPED,
        GaitChoice::Unison => GAIT_UNISON,
    });
    let commanded: Vec<(u8, u8, i16)> = (0..=u8::MAX)
        .zip(&controls.commands)
        .filter_map(|(part, command)| match *command {
            MechCommand::Idle => None,
            MechCommand::Rate(rate) => Some((part, COMMAND_RATE, rate)),
            MechCommand::Angle(angle) => Some((part, COMMAND_ANGLE, angle)),
            MechCommand::Stride(stride) => Some((part, COMMAND_STRIDE, i16::from(stride))),
        })
        .collect();
    out.u16(u16::try_from(commanded.len()).expect("at most 256 parts"));
    for (part, kind, value) in commanded {
        out.u8(part);
        out.u8(kind);
        out.0.extend_from_slice(&value.to_le_bytes());
    }
    out.0
}

/// Inverse of [`encode_controls`].
///
/// # Errors
///
/// If the bytes are truncated, trail, name an unknown gait or command or one part
/// twice, or a stride past ±100.
pub fn decode_controls(bytes: &[u8]) -> Result<Controls, WireError> {
    let mut input = In(bytes);
    let mut controls = Controls::IDLE;
    controls.gait = match input.u8()? {
        GAIT_SUGGESTED => GaitChoice::Suggested,
        GAIT_BIPED => GaitChoice::Preset(Gait::Biped),
        GAIT_QUADRUPED => GaitChoice::Preset(Gait::Quadruped),
        GAIT_UNISON => GaitChoice::Unison,
        _ => return Err(WireError::Malformed("unknown gait")),
    };
    for _ in 0..input.u16()? {
        let part = usize::from(input.u8()?);
        let kind = input.u8()?;
        let value = i16::from_le_bytes(input.take()?);
        let slot = controls
            .commands
            .get_mut(part)
            .ok_or(WireError::Malformed("no such part"))?;
        if *slot != MechCommand::Idle {
            return Err(WireError::Malformed("a part commanded twice"));
        }
        *slot = match kind {
            COMMAND_RATE => MechCommand::Rate(value),
            COMMAND_ANGLE => MechCommand::Angle(value),
            COMMAND_STRIDE => MechCommand::Stride(
                i8::try_from(value)
                    .ok()
                    .filter(|stride| (-100..=100).contains(stride))
                    .ok_or(WireError::Malformed("stride out of range"))?,
            ),
            _ => return Err(WireError::Malformed("unknown command")),
        };
    }
    input.finish()?;
    Ok(controls)
}

/// The robots' exact mechanical state ([`crate::mechanics::State`]): a robot count, then per
/// robot its build (as in a snapshot), its charge and gait clock, and each of its bodies'
/// position, orientation, velocity and angular velocity as exact `f32` bits.
#[must_use]
pub fn encode_mechanics(state: &MechState) -> Vec<u8> {
    let mut out = Out(Vec::new());
    out.u8(count(state.robots.len()));
    for robot in &state.robots {
        out.build(&robot.build);
        out.u32(robot.charge);
        out.u32(robot.clock);
        out.u8(count(robot.bodies.len()));
        for body in &robot.bodies {
            out.vec3(body.pose.position);
            for value in body.pose.orientation {
                out.u32(value.to_bits());
            }
            out.vec3(body.velocity);
            out.vec3(body.angular_velocity);
        }
    }
    out.0
}

/// Inverse of [`encode_mechanics`]; builds are validated, and each robot must have its build's
/// bodies and a charge within its pool.
///
/// # Errors
///
/// If the bytes are malformed, a build is invalid or a robot's bodies or charge do not fit its
/// build.
pub fn decode_mechanics(bytes: &[u8]) -> Result<MechState, WireError> {
    let mut input = In(bytes);
    let mut robots = Vec::new();
    for _ in 0..input.u8()? {
        let build = input.build()?;
        let charge = input.u32()?;
        let clock = input.u32()?;
        if charge > Pool::of(&build).capacity {
            return Err(WireError::Malformed("charge above capacity"));
        }
        let count = input.u8()?;
        if usize::from(count) != Layout::of(&build).bodies.len() {
            return Err(WireError::Malformed("bodies do not match the build"));
        }
        let bodies = (0..count)
            .map(|_| {
                let position = input.vec3()?;
                let orientation = [
                    f32::from_bits(input.u32()?),
                    f32::from_bits(input.u32()?),
                    f32::from_bits(input.u32()?),
                    f32::from_bits(input.u32()?),
                ];
                let velocity = input.vec3()?;
                let angular_velocity = input.vec3()?;
                let motion = BodyMotion {
                    pose: Pose {
                        position,
                        orientation,
                    },
                    velocity,
                    angular_velocity,
                };
                if [position, velocity, angular_velocity]
                    .iter()
                    .flatten()
                    .chain(&orientation)
                    .all(|value| value.is_finite())
                {
                    Ok(motion)
                } else {
                    Err(WireError::Malformed("body state not finite"))
                }
            })
            .collect::<Result<Vec<_>, WireError>>()?;
        robots.push(RobotState {
            build,
            charge,
            clock,
            bodies,
        });
    }
    input.finish()?;
    Ok(MechState { robots })
}
