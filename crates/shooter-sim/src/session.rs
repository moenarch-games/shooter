//! The arena simulation: the world, the fighters in their slots (people and bots) with their
//! weapons, health and scores, the rockets in flight and the target dummies, advanced one fixed
//! tick at a time. The offline practice mode runs it in the client; game-server runs the same
//! state for a hosted match (`shooter-server`), and the clients draw it and play its [`Event`]s.
//!
//! A deathmatch: every fighter is its robot build with per-part health ([`crate::damage`]);
//! strikes land on the parts they meet, destroyed parts fall away with what hung from them, and
//! destroying the root chassis is the kill. A kill scores a frag for the killer and a death for the
//! victim (a suicide, by a fall, one's own rocket, one's own battery's blast or a self-destruct,
//! costs a frag),
//! the dead come back as their configured build, whole, at the spawn point farthest from
//! everyone else after [`Rules::respawn_ticks`], and a round ends at the frag limit or the time
//! limit, followed by an intermission and a fresh round. A dead fighter whose person holds Edit
//! Robot ([`Intent::edit`]) is in the builder: it does not come back, at a respawn or a round's
//! start, until the person releases it (Ready, Atlas D2.i70.11). A person who stops sending
//! inputs (a lost connection) lets go of it after [`HOLD_INPUT_TICKS`].
//!
//! **Mechanical robots** (shooter#73, Atlas D2.i70.5). Every living fighter is its robot in the
//! articulated world ([`crate::mechanics`], [`Session::mechanics`]), keyed by its slot: the
//! robot's compound bodies are its only body (no capsule), and every part's box is where its
//! body carries it ([`Fighter::hitboxes`]). A spawn adds the robot, a death or a slot changing
//! hands removes it, and a part that comes off ([`crate::damage`]) leaves the bodies, so it no
//! longer weighs, touches or drives anything ([`Mechanics::sever`]); each of those rebuilds the
//! world from its exact state, so the host, a predicting client and a replay continue alike.
//! A robot without a connected leg moves on its own wheels and tracks alone (shooter#58, Atlas
//! D2.i74.1): no drive carries or turns it, its actuators take its control layout's commands and
//! pay by them ([`mechanics::draw`]; a refused wheel or track brakes), a jump press fires the
//! thruster module it carries and nothing else, and the arena's jump pads throw it
//! ([`crate::player::Motion::act`]). A robot with a connected leg stays on the interim movement
//! drive, the bounded fallback until legs walk on their own servos: each tick the movement rules
//! ([`crate::player::Motion::drive`]: walk, sprint, slide, jumps, pads, air control, the rope)
//! give the velocity the drive sets on the whole robot ([`Mechanics::drive`]), and it also
//! balances the robot and faces it toward the aim (an interim chassis drive, Atlas D2.i70.5). Its
//! actuators take what its person's control layout resolved ([`crate::bindings`]; a bot's, its
//! build's preset from its movement keys), its motors paying first in the tick: its locomotion for
//! the walk the drive carries it on, by speed and load ([`walk_draw`]; a refused walk leaves it
//! standing), its joints as commanded ([`mechanics::draw`]); and its legs, wheels and tracks
//! slide where the drive takes them. It walks on its feet: only a connected leg,
//! wheel or track on the ground is footing, and the wished speeds scale by the share of its
//! locomotion it still has ([`crate::player::Traction`]).
//!
//! **Collisions and debris** (shooter#75, Atlas D2.i70.10). After the step, a robot that a hard
//! impact (another robot, debris, the arena, a high fall) changed fast enough takes a blow on the
//! part reaching furthest toward what it hit ([`crate::impact`]), nobody's (a collision kill
//! costs a frag, as a fall does). A large part carried away whole stays a physical body for a
//! while ([`Mechanics::break_off`], [`crate::mechanics::DebrisState`]); destroyed and small ones
//! do not. Debris retires deterministically (lifetime, rest, budget, its robot's respawn) at the
//! start of a tick and travels in snapshots, so a predicting client holds it exactly.
//!
//! Hitscan shots and blade swings are lag-compensated: a person's shot is checked against the
//! fighters' parts where that person saw them (where their bodies carried them at the snapshot
//! tick its command names, at most [`MAX_REWIND_TICKS`] back; the parts they have now), while
//! the arena's geometry blocks the ray as it is now. Rockets and shotgun pellets are projectiles:
//! they fly through the world as it is each tick and meet fighters' parts where they are then,
//! like the flame.
//!
//! Module pickups lie on the map's pads and where their carriers died; the host resolves who takes
//! them, applies, expires and clears their effects, drops a dead carrier's module, removes the
//! drops and brings the pads back ([`crate::pickups`] has the rules).
//!
//! **Arena destruction** (shooter#60). A kit-built map's marked cover, barriers and crates are
//! damageable placeables ([`Session::scenery`]): a ray, a pellet or the flame that stops on one,
//! a blade swing that strikes no robot but meets one, and a rocket's blast hurt it as they hurt
//! robot parts, and at zero health it breaks. The break lands last in the tick (after the robots'
//! changes), in the arena, the physics world and the robots' world at once, so every query of
//! the next tick meets the broken pieces; a new round restores every placeable
//! ([`crate::scenery`] has the rules).
//!
//! Every fighter has its build's electricity and fuel ([`crate::energy`], shooter#128); its
//! motors, jumps, swings, shots and repairs ask for electricity first come, first served, and
//! after them each tick its generators burn fuel to refill it; nothing refills by itself. Its
//! root chassis sets its powertrain (shooter#130): an electric chassis' locomotion draws
//! electricity, a combustion chassis' burns fuel through a connected engine, and the flamethrower
//! burns fuel on either. A lost battery or tank takes its storage and its share of what it held
//! with it; a lost generator stops the recharge, a lost engine a combustion drive. Depletion
//! stops only what needed the resource.
//!
//! A person whose build mounts a repair module may repair a damaged part in its reach while the
//! match goes on ([`crate::repair`]): the host checks the order, the repair draws electricity
//! after the tick's shots and hits, a blow on the part or the module
//! pauses it, and death and respawn end it.
//!
//! Any robot may self-destruct ([`crate::destruct`], shooter#131): a press arms a warning
//! countdown (another cancels it; death and the round's end do too), and when it ends the robot
//! dies by its own kill and its volatile contents blast every other robot's parts in reach that
//! the arena does not shelter, after the tick's shots and before the repairs.

use crate::aim;
use crate::arena::{Arena, FIRST_DUMMY_BODY, FIRST_PLAYER_BODY, Spawn};
use crate::bindings::{BindingState, Bindings, Pressed};
use crate::bots::{Bot, Sight};
use crate::damage::{BATTERY_BLAST_DAMAGE, BATTERY_BLAST_RADIUS_UNITS, Blow, Hitboxes, PartDamage};
use crate::destruct;
use crate::dummies::Dummy;
use crate::energy::{self, Flow, Pool};
use crate::grapple::Hook;
use crate::maps::MapId;
use crate::mechanics::{self, Controls, Footing as RobotFooting, Mechanics, RobotState};
use crate::melee;
use crate::physics::articulated::Pose;
use crate::physics::{BodyId, PhysicsError, RayHit, SUBUNITS_PER_UNIT, Vec3i, World};
use crate::pickups::{
    self, ArenaModule, Carried, DROP_LIFETIME_TICKS, Drop, PAD_RESPAWN_TICKS, Pickup,
};
use crate::player::{
    Footing, GRAVITY, Intent, KILL_HEIGHT_UNITS, Motion, PLAYER_HALF_EXTENTS, STEP_UP, Traction,
    eye_height,
};
use crate::repair::{self, Order, Outcome, Repair};
use crate::robot::{Assembly, BETA, Kind, Locomotion, PartId, Tuning};
use crate::scenery::{PlaceableState, Scenery};
use crate::weapons::{
    self, Arsenal, Delivery, Flight, Instance, Pellet, Rocket, Select, Travel, Weapon,
};
use std::collections::VecDeque;

/// Ticks a fighter stays dead: 2 s.
pub const RESPAWN_TICKS: u32 = 120;
/// A rocket's splash hurts its owner at this share of the damage (rocket jumps cost health).
pub const SELF_SPLASH_PERCENT: i32 = 50;
/// The furthest back a lag-compensated shot looks: 250 ms.
pub const MAX_REWIND_TICKS: u32 = 15;
/// Inputs a person may have queued; older ones are dropped beyond this.
pub const MAX_QUEUED_INPUTS: usize = 8;
/// A person's input backlog is judged over this many ticks (0.5 s)...
pub const BACKLOG_WINDOW_TICKS: u32 = 30;
/// ...and when at least this many more inputs stayed queued the whole window (a client whose
/// clock runs fast, or a burst after a stall), the oldest are dropped down to it: the queue adds
/// at most this many ticks of standing latency, and jitter below it is never trimmed.
pub const BACKLOG_KEEP: usize = 2;
/// A person with no input for this many ticks (250 ms: a dropped connection) stops moving and
/// shooting until inputs arrive again; shorter gaps hold the last input.
pub const HOLD_INPUT_TICKS: u32 = 15;
/// Slots in a deathmatch by default (Atlas D2.i46.12): people take them from bots. A host sets
/// another count per match, up to its map's [`MapId::max_slots`].
pub const DEATHMATCH_SLOTS: u8 = 12;
/// A round ends when someone has this many frags...
pub const FRAG_LIMIT: i32 = 15;
/// ...or after 5 minutes.
pub const TIME_LIMIT_TICKS: u32 = 5 * 60 * 60;
/// The scoreboard shows for 8 s between rounds.
pub const INTERMISSION_TICKS: u32 = 8 * 60;
/// The person playing the in-process practice match (and the scripted scenarios).
pub const LOCAL_PLAYER: u32 = 0;

/// How a match is played.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rules {
    pub slots: u8,
    /// Bots fill the slots no person holds.
    pub bots: bool,
    /// Slot 0 starts held by [`LOCAL_PLAYER`] (practice and scripts).
    pub local: bool,
    /// Seeds the bots.
    pub seed: u64,
    /// 0: no limit.
    pub frag_limit: i32,
    /// 0: no limit.
    pub time_limit_ticks: u32,
    pub intermission_ticks: u32,
    pub respawn_ticks: u32,
}

impl Rules {
    /// One person alone, no bots and no end: the movement and weapon scripts.
    #[must_use]
    pub const fn solo() -> Self {
        Self {
            slots: 1,
            bots: false,
            local: true,
            seed: 0,
            frag_limit: 0,
            time_limit_ticks: 0,
            intermission_ticks: 0,
            respawn_ticks: RESPAWN_TICKS,
        }
    }

    /// A hosted deathmatch: every slot a bot until people join.
    #[must_use]
    pub const fn deathmatch(seed: u64) -> Self {
        Self {
            slots: DEATHMATCH_SLOTS,
            bots: true,
            local: false,
            seed,
            frag_limit: FRAG_LIMIT,
            time_limit_ticks: TIME_LIMIT_TICKS,
            intermission_ticks: INTERMISSION_TICKS,
            respawn_ticks: RESPAWN_TICKS,
        }
    }

    /// Offline practice: the local person in slot 0 against bots.
    #[must_use]
    pub const fn practice(seed: u64) -> Self {
        Self {
            local: true,
            ..Self::deathmatch(seed)
        }
    }
}

/// Where the round is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Phase {
    /// Fighting; the round ends at `ends_at` (if there is a time limit) or at the frag limit.
    Playing { ends_at: Option<u32> },
    /// The round is over: `winner` had the most frags; the next starts at `restarts_at`.
    Intermission {
        winner: Option<u8>,
        restarts_at: u32,
    },
}

/// Who controls a slot.
#[derive(Clone, Debug, PartialEq)]
pub enum Pilot {
    /// Nobody: no body in the arena.
    Open,
    /// A person; `id` is game-server's player ID (or [`LOCAL_PLAYER`]).
    Human {
        id: u32,
    },
    Bot(Box<Bot>),
}

/// One tick of a person's controls, as their client sent it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Input {
    /// game-server's command sequence: snapshots acknowledge the last one applied.
    pub sequence: u32,
    pub intent: Intent,
    /// The snapshot tick the client showed the other fighters at: hitscan rewinds to it.
    pub view_tick: u32,
}

/// One slot of the match.
#[derive(Clone, Debug)]
pub struct Fighter {
    pub slot: u8,
    pub pilot: Pilot,
    /// The name its person picked ([`crate::names`]); cleared when the slot changes hands.
    pub name: Option<String>,
    /// Its movement state (slides, jumps) between ticks.
    pub motion: Motion,
    /// Where its mechanical robot is after the last tick, while it is alive.
    pub stance: Option<Stance>,
    pub arsenal: Arsenal,
    /// The robot it is built as this life ([`crate::robot`]).
    pub build: Assembly,
    /// The build it comes back as at its next spawn (D2.i46.7: rebuilt only between lives).
    pub configured: Assembly,
    /// The grapple's hook while it is set.
    pub hook: Option<Hook>,
    /// The module pickup it carries this life, if any ([`crate::pickups`]).
    pub carried: Option<Carried>,
    /// The electricity it holds, milli-units ([`crate::energy`]; its build's [`Fighter::pool`]
    /// bounds it).
    pub energy: u32,
    /// The fuel it holds, milli-units ([`crate::energy`]; bounded likewise).
    pub fuel: u32,
    /// What its resources did in the last tick (a report for the HUD, not state).
    pub flow: Flow,
    /// Its build's part healths and lost parts this life ([`crate::damage`]); the root
    /// chassis' health is its life.
    pub parts: PartDamage,
    /// The field repair under way this life, if any ([`crate::repair`]).
    pub repair: Option<Repair>,
    /// While its self-destruct counts down: the tick it goes off ([`crate::destruct`]).
    pub destruct: Option<u32>,
    /// While dead: the tick it comes back.
    pub respawn_at: Option<u32>,
    pub frags: i32,
    pub deaths: i32,
    /// The intent it acts on (a person's last input, held until the next).
    pub intent: Intent,
    /// The last input applied.
    pub ack: u32,
    pub view_tick: u32,
    inputs: VecDeque<Input>,
    /// The fewest inputs queued at any tick of the current backlog window.
    backlog: usize,
    backlog_ticks: u32,
    /// Ticks in a row without a queued input.
    starved: u32,
    grounded: bool,
}

impl Fighter {
    fn new(slot: u8, pilot: Pilot) -> Self {
        Self {
            slot,
            pilot,
            name: None,
            motion: Motion::default(),
            stance: None,
            arsenal: Arsenal::default(),
            build: Assembly::default(),
            configured: Assembly::default(),
            hook: None,
            carried: None,
            energy: 0,
            fuel: 0,
            flow: Flow::default(),
            parts: PartDamage::default(),
            repair: None,
            destruct: None,
            respawn_at: None,
            frags: 0,
            deaths: 0,
            intent: Intent::IDLE,
            ack: 0,
            view_tick: 0,
            inputs: VecDeque::new(),
            backlog: usize::MAX,
            backlog_ticks: 0,
            starved: 0,
            grounded: false,
        }
    }

    /// Inputs queued for this tick and the ticks after.
    #[must_use]
    pub fn queued_inputs(&self) -> usize {
        self.inputs.len()
    }

    /// Tracks the input backlog over [`BACKLOG_WINDOW_TICKS`] and, at the end of a window in
    /// which more than [`BACKLOG_KEEP`] inputs always waited, drops the oldest down to it (a
    /// dropped weapon switch or repair order carries over to the next input).
    fn trim_backlog(&mut self) {
        self.backlog = self.backlog.min(self.inputs.len());
        self.backlog_ticks += 1;
        if self.backlog_ticks < BACKLOG_WINDOW_TICKS {
            return;
        }
        for _ in BACKLOG_KEEP..self.backlog {
            let Some(dropped) = self.inputs.pop_front() else {
                break;
            };
            if let Some(next) = self.inputs.front_mut() {
                next.intent.select = next.intent.select.or(dropped.intent.select);
                next.intent.repair = next.intent.repair.or(dropped.intent.repair);
                // Presses toggle: two dropped together cancel out.
                next.intent.destruct ^= dropped.intent.destruct;
            }
        }
        self.backlog = usize::MAX;
        self.backlog_ticks = 0;
    }

    /// In the arena with a body (not dead, not an open slot).
    #[must_use]
    pub fn alive(&self) -> bool {
        self.respawn_at.is_none() && self.pilot != Pilot::Open
    }

    #[must_use]
    pub const fn human(&self) -> Option<u32> {
        match self.pilot {
            Pilot::Human { id } => Some(id),
            _ => None,
        }
    }

    /// Whether it stood on ground after the last tick.
    #[must_use]
    pub const fn grounded(&self) -> bool {
        self.grounded
    }

    /// How the weapon copy `instance` fires for it: its build's mod on that copy, then the
    /// module it carries.
    #[must_use]
    pub fn tuning(&self, instance: Instance) -> Tuning {
        tuning_of(&self.build, self.carried, instance)
    }

    /// The weapon copies it still has, in part ID order ([`PartDamage::instances`]).
    #[must_use]
    pub fn instances(&self) -> Vec<Instance> {
        self.parts.instances(&self.build).collect()
    }

    /// Its pools: its connected parts' electricity and fuel storage and generation
    /// ([`crate::energy`]).
    #[must_use]
    pub fn pool(&self) -> Pool {
        Pool::attached(&self.build, &self.parts)
    }

    /// It is `build` this life, whole: its parts and arsenal ([`Session::rebuild`] puts its
    /// robot in the world as that build).
    fn rebuild(&mut self, build: Assembly) {
        self.build = build;
        self.parts = PartDamage::fresh(&build);
        self.repair = None;
        if !self.parts.holdable(&build, self.arsenal.held) {
            self.arsenal = Arsenal::holding(self.first_weapon());
        }
        // What it holds stays, as far as the new build's storage takes it (nothing refills).
        let pool = self.pool();
        self.energy = self.energy.min(pool.capacity);
        self.fuel = self.fuel.min(pool.fuel);
    }

    /// The parts a field repair may start on now (bits by index, [`repair::eligible`]): damaged,
    /// still connected and in reach of a working repair module where its robot carries them.
    /// None while dead.
    #[must_use]
    pub fn repairable(&self) -> u32 {
        if !self.alive() {
            return 0;
        }
        repair::eligible(&self.build, &self.parts, &self.hitboxes())
    }

    /// The weapon it takes up at spawn: the first gun its build lets it hold, else the blade
    /// ([`weapons::spawn_weapon`]; never the grapple module), the energy rifle when it has none.
    fn first_weapon(&self) -> Instance {
        weapons::spawn_weapon(&self.instances()).unwrap_or_default()
    }

    /// Its root chassis' health: its life ([`crate::damage`]).
    #[must_use]
    pub fn chassis_health(&self) -> u16 {
        self.parts.chassis_health(&self.build)
    }

    /// Its connected parts where its robot's bodies carry them now (none while dead).
    #[must_use]
    pub fn hitboxes(&self) -> Hitboxes {
        self.stance
            .as_ref()
            .map(|stance| stance.hitboxes.attached(&self.parts))
            .unwrap_or_default()
    }

    /// The weapon copy `which` names on its robot: that exact part if it is a weapon of its
    /// build, or the first copy of that weapon it still has.
    #[must_use]
    pub fn copy(&self, which: impl Into<Select>) -> Option<Instance> {
        match which.into() {
            Select::Part(part) => BETA.instance(&self.build, part),
            Select::Weapon(weapon) => self.parts.first(&self.build, weapon),
        }
    }

    /// Where the weapon copy `which` names ([`Fighter::copy`]) shoots from now
    /// ([`aim::tube_muzzle`]; the rocket pod's next tube, [`Fighter::tube`]): `None` while dead,
    /// or when that copy is no longer connected or the weapon has no muzzle (the blade).
    #[must_use]
    pub fn muzzle(&self, which: impl Into<Select>) -> Option<aim::Muzzle> {
        let copy = self.copy(which)?;
        aim::tube_muzzle(&self.build, &self.hitboxes(), copy, self.tube(copy))
    }

    /// Where its grapple's rope is tied on the robot (units): the muzzle of its first grapple
    /// module still connected, where the simulation carries it now (Atlas D2.i104.2,
    /// shooter#104); `None` while dead or without a working module. [`crate::grapple::Hook`]
    /// measures the rope from here, and the clients draw it from the module's muzzle.
    #[must_use]
    pub fn rope_tie(&self) -> Option<[f32; 3]> {
        self.muzzle(Weapon::Grapple).map(|muzzle| muzzle.at)
    }

    /// The tube the weapon copy `instance` fires its next shot from (0 the first): the rocket
    /// pod's alternate launch by launch (shooter#102), every other weapon has one.
    #[must_use]
    pub fn tube(&self, instance: Instance) -> u8 {
        if instance.weapon.tubes() < 2 {
            return 0;
        }
        self.build
            .parts()
            .iter()
            .position(|part| part.id == instance.part)
            .map_or(0, |index| self.arsenal.tube(index))
    }

    /// Its feet (units): its robot's frame origin less the reach of its build standing whole
    /// ([`mechanics::stance`]); the origin while dead.
    #[must_use]
    pub fn feet(&self) -> [f32; 3] {
        self.stance.as_ref().map_or([0.0; 3], Stance::feet)
    }

    /// Its centre (units): as high above its feet as a fighter's reference box's centre
    /// ([`PLAYER_HALF_EXTENTS`]), what bots, pads and blasts measure from.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn center(&self) -> [f32; 3] {
        let [x, y, z] = self.feet();
        [x, y + PLAYER_HALF_EXTENTS.y as f32, z]
    }

    /// Where its shots leave from: the eye above its feet, lower while sliding.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn eye(&self) -> [f32; 3] {
        let [x, y, z] = self.feet();
        [x, y + eye_height(self.motion) as f32, z]
    }

    /// Its root body's velocity (units a second), zero while dead.
    #[must_use]
    pub fn velocity(&self) -> [f32; 3] {
        self.stance
            .as_ref()
            .map_or([0.0; 3], |stance| stance.velocity)
    }

    /// The share of its locomotion it still has.
    #[must_use]
    pub fn traction(&self) -> Traction {
        let mut traction = Traction {
            connected: 0,
            mounted: 0,
        };
        for (index, part) in self.build.parts().iter().enumerate() {
            if BETA
                .part_type(part.kind)
                .is_some_and(|kind| matches!(kind.kind, Kind::Locomotion(_)))
            {
                traction.mounted = traction.mounted.saturating_add(1);
                if self.parts.attached(index) {
                    traction.connected = traction.connected.saturating_add(1);
                }
            }
        }
        traction
    }

    /// Which way its chassis faces (binary yaw): its robot frame's +X, level. On wheels or tracks
    /// alone (shooter#58) the way it rolls, not where its person looks.
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub fn heading(&self) -> u16 {
        let Some(stance) = self.stance.as_ref() else {
            return 0;
        };
        let [x, y, z, w] = stance.origin.orientation;
        let ahead = [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * z - y * w)];
        let [ax, az] = ahead.map(|axis| (axis * 4_096.0).round() as i32);
        crate::trig::yaw_from_vector(ax, az).unwrap_or(0)
    }

    /// An operational leg is the interim movement drive's only source of natural jump and
    /// chassis turning. Wheels, tracks and detached legs do not grant either capability.
    #[must_use]
    fn connected_leg(&self) -> bool {
        self.build.parts().iter().enumerate().any(|(index, part)| {
            self.parts.attached(index)
                && BETA
                    .part_type(part.kind)
                    .is_some_and(|part_type| part_type.kind == Kind::Locomotion(Locomotion::Leg))
        })
    }
}

/// Where a fighter's mechanical robot is after a tick.
#[derive(Clone, Debug, PartialEq)]
pub struct Stance {
    /// Its root body's authored frame.
    pub origin: Pose,
    /// Its root body's velocity (of its centre of mass), units a second.
    pub velocity: [f32; 3],
    /// How far its feet reach below the origin ([`mechanics::stance`] of its whole build).
    pub reach: f32,
    pub footing: RobotFooting,
    /// Its connected parts where its bodies carry them.
    pub hitboxes: Hitboxes,
}

impl Stance {
    #[must_use]
    pub fn feet(&self) -> [f32; 3] {
        let [x, y, z] = self.origin.position;
        [x, y - self.reach, z]
    }
}

/// Units a second of a velocity in subunits a tick.
#[allow(clippy::cast_precision_loss)]
fn per_second(velocity: Vec3i) -> [f32; 3] {
    [velocity.x, velocity.y, velocity.z]
        .map(|axis| (axis * i32::from(crate::TICK_HZ)) as f32 / SUBUNITS_PER_UNIT as f32)
}

/// Subunits a tick of a velocity in units a second, rounded.
#[allow(clippy::cast_possible_truncation)]
fn per_tick(velocity: [f32; 3]) -> Vec3i {
    let [x, y, z] = velocity
        .map(|axis| (axis * SUBUNITS_PER_UNIT as f32 / f32::from(crate::TICK_HZ)).round() as i32);
    Vec3i::new(x, y, z)
}

/// Whole units of a position.
#[allow(clippy::cast_possible_truncation)]
fn whole(position: [f32; 3]) -> Vec3i {
    let [x, y, z] = position.map(|axis| axis.round() as i32);
    Vec3i::new(x, y, z)
}

/// How hard a robot's locomotion works to carry it as `intent` asks (thousandths,
/// [`mechanics::activity`]): `walking` (on its feet, not on a rope) toward a wished direction,
/// at its wished speed against a sprint; nothing standing, in the air or on a rope.
#[must_use]
pub fn walk_activity(intent: Intent, walking: bool) -> u32 {
    if !walking || crate::player::wish_direction(intent).is_none() {
        return 0;
    }
    let speed = crate::player::wish_speed(intent).unsigned_abs();
    let sprint = crate::player::SPRINT_SPEED_SUBUNITS_PER_TICK.unsigned_abs();
    (speed * 1_000 / sprint.max(1)).min(1_000)
}

/// The electricity (milli-units) a tick of walking at `activity` ([`walk_activity`]) takes the
/// connected locomotion of `layout` on a robot of `load_kg`: every leg's, wheel's and track's
/// [`mechanics::work`], together.
#[must_use]
pub fn walk_draw(layout: &mechanics::Layout, activity: u32, load_kg: u32) -> u32 {
    layout
        .actuators
        .iter()
        .filter(|actuator| matches!(actuator.kind, mechanics::Actuator::Locomotion(_)))
        .map(|actuator| mechanics::work(actuator.drive.draw, activity, Some(load_kg)))
        .sum()
}

/// A robot's motors ask its stores (`charge`, `fuel`) for this tick's work, its first requests
/// (shooter#128): first its locomotion for the walk `intent` asks of the movement drive
/// ([`walk_draw`], one request for all of it, `walking` on its feet) from what `pool`'s drive
/// runs on (electricity on an electric chassis, fuel through an engine on a combustion one, and
/// nothing on a combustion chassis without a connected engine, [`Pool::drive`], shooter#130),
/// then each joint for the electricity what
/// `controls` command it ([`mechanics::draw`]), in part-ID order. A refused walk does not happen:
/// the intent keeps only its aim, and its locomotion stands (free); a refused joint is idle
/// this tick. Everything else works on. The intent and controls to act on, and whether the walk
/// was refused.
fn power(
    (pool, charge, fuel): (Pool, &mut u32, &mut u32),
    load_kg: u32,
    layout: &mechanics::Layout,
    controls: Controls,
    intent: Intent,
    walking: bool,
) -> (Intent, Controls, bool) {
    let walk = walk_draw(layout, walk_activity(intent, walking), load_kg);
    let stalled = !energy::pay(pool.drive(), charge, fuel, walk);
    let intent = if stalled {
        Intent {
            yaw: intent.yaw,
            pitch: intent.pitch,
            ..Intent::IDLE
        }
    } else {
        intent
    };
    let mut controls = controls;
    for actuator in &layout.actuators {
        let at = usize::from(actuator.part.0);
        let command = &mut controls.commands[at];
        match actuator.kind {
            // A refused walk leaves the locomotion standing, which costs nothing.
            mechanics::Actuator::Locomotion(Locomotion::Leg) if stalled => {
                *command = mechanics::Command::Stride(0);
            }
            mechanics::Actuator::Locomotion(_) if stalled => {
                *command = mechanics::Command::Rate(0);
            }
            mechanics::Actuator::Locomotion(_) => {}
            mechanics::Actuator::Joint(_) => {
                if !energy::spend(charge, mechanics::draw(actuator, *command, load_kg)) {
                    *command = mechanics::Command::Idle;
                }
            }
        }
    }
    (intent, controls, stalled)
}

/// [`power`] for a robot that moves on its own wheels and tracks (shooter#58): each actuator
/// asks for what `controls` command it ([`mechanics::draw`]), in part-ID order, locomotion from
/// what `pool`'s drive runs on and joints from the electricity. A refused wheel or track brakes,
/// which costs nothing; a refused joint is idle this tick. The controls to act on, and whether
/// any locomotion was refused.
fn power_own(
    (pool, charge, fuel): (Pool, &mut u32, &mut u32),
    load_kg: u32,
    layout: &mechanics::Layout,
    controls: Controls,
) -> (Controls, bool) {
    let mut controls = controls;
    let mut stalled = false;
    for actuator in &layout.actuators {
        let at = usize::from(actuator.part.0);
        let work = mechanics::draw(actuator, controls.commands[at], load_kg);
        let paid = match actuator.kind {
            mechanics::Actuator::Locomotion(_) => energy::pay(pool.drive(), charge, fuel, work),
            mechanics::Actuator::Joint(_) => energy::spend(charge, work),
        };
        if !paid {
            controls.commands[at] = match actuator.kind {
                mechanics::Actuator::Locomotion(_) => {
                    stalled = true;
                    mechanics::Command::Rate(0)
                }
                mechanics::Actuator::Joint(_) => mechanics::Command::Idle,
            };
        }
    }
    (controls, stalled)
}

/// Below this speed up or down (units a second) a robot held up by its chassis has come to rest
/// there rather than glancing off on its way down...
const PERCHED_REST: f32 = 2.0;
/// ...and with no scenery this far (units) under its feet it is hung up on it, not leaning on
/// a wall a step above the ground.
const PERCHED_HEIGHT: f32 = 20.0;

/// A robot held up by its chassis, upright, at rest and high off the ground: perched on a
/// ledge's lip rather than tipped over.
fn perched(world: &World, stance: &Stance) -> bool {
    let feet = stance.feet();
    upright(stance.origin.orientation)
        && stance.velocity[1].abs() < PERCHED_REST
        && world
            .cast_ray(feet, [0.0, -PERCHED_HEIGHT, 0.0], |body| {
                body.0 < FIRST_PLAYER_BODY
            })
            .is_none()
}

/// A robot's frame at `orientation` leans less than 45° from upright: its up axis's height,
/// `1 - 2 (x² + z²)`, is above `cos 45°` (arithmetic only, so every platform agrees).
fn upright(orientation: [f32; 4]) -> bool {
    let [x, _, z, _] = orientation;
    1.0 - 2.0 * (x * x + z * z) > std::f32::consts::FRAC_1_SQRT_2
}

/// The angular velocity (radians a second) that turns a robot's frame from `orientation`
/// halfway to upright and facing `yaw` in a tick: the movement drive's balance and aim.
/// Arithmetic only (no transcendental functions), so every platform agrees.
fn steer(orientation: [f32; 4], yaw: u16) -> [f32; 3] {
    let target = mechanics::facing(yaw);
    let [qx, qy, qz, qw] = orientation;
    let (cx, cy, cz, cw) = (-qx, -qy, -qz, qw);
    let [tx, ty, tz, tw] = target;
    // The rotation from here to there: target × conjugate(orientation).
    let error = [
        tw * cx + tx * cw + ty * cz - tz * cy,
        tw * cy - tx * cz + ty * cw + tz * cx,
        tw * cz + tx * cy - ty * cx + tz * cw,
        tw * cw - tx * cx - ty * cy - tz * cz,
    ];
    let sign = if error[3] < 0.0 { -1.0 } else { 1.0 };
    let rate = sign * f32::from(crate::TICK_HZ);
    let spin = [error[0] * rate, error[1] * rate, error[2] * rate];
    let speed = (spin[0] * spin[0] + spin[1] * spin[1] + spin[2] * spin[2]).sqrt();
    if speed > MAX_TURN_RATE {
        spin.map(|axis| axis * (MAX_TURN_RATE / speed))
    } else {
        spin
    }
}

/// The fastest the movement drive turns a robot (radians a second): a half turn in a fifth of
/// a second.
const MAX_TURN_RATE: f32 = 16.0;

/// How far (units) below its feet a robot still counts as standing, and the fastest it may be
/// rising then (units a second): its legs' give lifts it off the ground for a tick now and
/// then, and it does not lose its footing for that.
const GROUND_PROBE_UNITS: f32 = 4.0;
const GROUND_RISE: f32 = 60.0;

/// The scenery is right under a robot's feet and it is not rising off it.
fn on_ground(world: &World, stance: &Stance) -> bool {
    let feet = stance.feet();
    stance.velocity[1] <= GROUND_RISE
        && world
            .cast_ray(
                [feet[0], feet[1] + GROUND_PROBE_UNITS, feet[2]],
                [0.0, -2.0 * GROUND_PROBE_UNITS, 0.0],
                |body| body.0 < FIRST_PLAYER_BODY,
            )
            .is_some()
}

/// How far ahead of its feet (units) a robot feels for a ledge to step onto...
const STEP_PROBE_UNITS: f32 = 35.0;
/// ...at these heights above its feet (units): the lowest blocked, the first clear one above it
/// the ledge's top, at most [`STEP_UP`]'s height.
const STEP_PROBES: [i32; 6] = [3, 10, 17, 24, 31, STEP_UP.max_height + 3];

/// Step-up for robots (what the capsule's step-up was, [`STEP_UP`]): a robot moving toward
/// scenery that blocks its feet but not [`STEP_UP`]'s height above them hops just high enough
/// to land on it, on the ground or in the air (a ledge caught at a jump's top). Its feet's
/// boxes would otherwise catch on every kerb, stair and ledge lip. Integer square root, so
/// every platform agrees.
#[allow(clippy::cast_precision_loss)]
fn step_up(world: &World, intent: Intent, feet: [f32; 3], velocity: Vec3i) -> Vec3i {
    let Some((x, z)) = crate::player::wish_direction(intent) else {
        return velocity;
    };
    let one = crate::trig::TRIG_ONE as f32;
    let ahead = [
        x as f32 / one * STEP_PROBE_UNITS,
        0.0,
        z as f32 / one * STEP_PROBE_UNITS,
    ];
    let scenery = |body: BodyId| body.0 < FIRST_PLAYER_BODY;
    let blocked = |height: i32| {
        world
            .cast_ray([feet[0], feet[1] + height as f32, feet[2]], ahead, scenery)
            .is_some()
    };
    if !blocked(STEP_PROBES[0]) {
        return velocity;
    }
    let Some(top) = STEP_PROBES[1..]
        .iter()
        .copied()
        .find(|height| !blocked(*height))
    else {
        return velocity;
    };
    // Rising `top` units under gravity: v² = 2 g h, in subunits.
    let lift = u64::from(top.unsigned_abs())
        * 2
        * u64::from(GRAVITY.y.unsigned_abs())
        * u64::from(SUBUNITS_PER_UNIT.unsigned_abs());
    let hop = i32::try_from(crate::trig::isqrt(lift)).unwrap_or(i32::MAX);
    Vec3i::new(velocity.x, velocity.y.max(hop), velocity.z)
}

/// The commands the preset layout of the robot in `slot` ([`Bindings::preset`], from its
/// connected actuators) gives for the keys `intent`'s movement presses ([`Pressed::wasd`]): what
/// a bot sends, and a script or test that plays the default layout. Rest without a robot.
fn preset_motors(mechanics: &Mechanics, slot: u8, intent: Intent) -> crate::bindings::Motors {
    mechanics
        .robot(slot)
        .map_or(crate::bindings::Motors::REST, |robot| {
            Bindings::preset(&robot.layout).resolve(
                &robot.layout,
                &mut BindingState::default(),
                Pressed::wasd(intent.forward, intent.strafe),
            )
        })
}

fn tuning_of(build: &Assembly, carried: Option<Carried>, instance: Instance) -> Tuning {
    let tuning = BETA.tuning(build, instance);
    carried.map_or(tuning, |carried| {
        carried.module.tune(instance.weapon, tuning)
    })
}

/// One use of `fighter`'s module spent; the last one spends the module.
fn spend_module(fighter: &mut Fighter, events: &mut Vec<Event>) {
    let Some(carried) = fighter.carried else {
        return;
    };
    fighter.carried = carried.spend();
    if fighter.carried.is_none() {
        events.push(Event::ModuleSpent {
            slot: fighter.slot,
            module: carried.module,
        });
    }
}

/// The physics body of a slot's fighter.
#[must_use]
pub const fn body_of(slot: u8) -> BodyId {
    BodyId(FIRST_PLAYER_BODY + slot as u64)
}

/// The first thing a projectile of `owner` meets on its path from `origin` along `translation`:
/// the scenery and dummies in `world` (the fighters' capsules ignored), or a fighter's part
/// (`boxes`, by slot; the owner's own robot excepted), whichever is nearer. A part hit is
/// reported on its fighter's body, its normal against the path.
fn projectile_hit(
    world: &World,
    boxes: &[Option<Hitboxes>],
    owner: BodyId,
    origin: [f32; 3],
    translation: [f32; 3],
) -> Option<RayHit> {
    let scenery = world.cast_ray(origin, translation, |body| {
        body != owner && slot_of(body).is_none()
    });
    let mut best = scenery;
    for (slot, parts) in boxes.iter().enumerate() {
        let Some(parts) = parts else { continue };
        let body = body_of(u8::try_from(slot).unwrap_or(u8::MAX));
        if body == owner {
            continue;
        }
        if let Some(fraction) = parts.first(origin, translation)
            && best.is_none_or(|hit| fraction < hit.fraction)
        {
            let length = translation
                .iter()
                .map(|axis| axis * axis)
                .sum::<f32>()
                .sqrt()
                .max(f32::EPSILON);
            best = Some(RayHit {
                body,
                fraction,
                normal: translation.map(|axis| -axis / length),
            });
        }
    }
    best
}

/// The slot whose fighter owns `body`, if it is a fighter's.
#[must_use]
pub fn slot_of(body: BodyId) -> Option<u8> {
    (FIRST_PLAYER_BODY..FIRST_DUMMY_BODY)
        .contains(&body.0)
        .then(|| u8::try_from(body.0 - FIRST_PLAYER_BODY).ok())
        .flatten()
}

/// What a tick did to the local fighter (slot 0), for presentation (footsteps, landing,
/// camera) and checks.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StepReport {
    pub grounded: bool,
    pub jumped: bool,
    /// The jump was the air jump.
    pub air_jumped: bool,
    /// A jump pad threw the player.
    pub launched: bool,
    pub sliding: bool,
    /// It touched down this tick after being airborne.
    pub landed: bool,
    /// The last weapon copy that fired this tick (the last of [`StepReport::volley`]).
    pub fired: Option<Instance>,
    /// Every copy that fired this tick, in the order granted: the grapple's hook, the swing,
    /// then the guns by part (shooter#127).
    pub volley: weapons::Volley,
    /// Damage dealt to others (fighters and dummies) this tick.
    pub damage: i32,
    /// A rocket exploded this tick.
    pub exploded: bool,
}

/// Something a tick did that the clients show or sound.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    /// A shot (or the grapple module's hook) left `from` (units: its muzzle, the eye for the blade,
    /// [`aim`]) along `direction` (unit vector). `part` is the weapon copy that fired
    /// (shooter#125), `tube` the tube it left (0 the first: the rocket pod's alternate,
    /// shooter#102, [`aim::tube_muzzle`]).
    Fired {
        slot: u8,
        weapon: Weapon,
        part: PartId,
        tube: u8,
        from: [f32; 3],
        direction: [f32; 3],
    },
    /// A hitscan beam (or the grapple's line) from `from` to `end`; `struck` when it met a body there.
    Beam {
        weapon: Weapon,
        from: [f32; 3],
        end: [f32; 3],
        normal: [f32; 3],
        struck: bool,
    },
    /// A rocket blew up at `at`.
    Exploded { at: [f32; 3] },
    /// A shotgun pellet's last stretch, from `from` to where it struck a body at `at`;
    /// `launched` when it was fired this tick (`from` is the shooter's muzzle).
    Pellet {
        from: [f32; 3],
        at: [f32; 3],
        normal: [f32; 3],
        launched: bool,
    },
    /// A dummy took damage.
    Damaged {
        dummy: usize,
        at: [f32; 3],
        damage: i32,
        downed: bool,
    },
    /// A fighter's `part` was struck for `damage` (its health stops at zero) at `at` by
    /// `attacker` (itself for its own rocket; `None`: the arena) ([`crate::damage`]).
    Hit {
        victim: u8,
        attacker: Option<u8>,
        part: PartId,
        damage: i32,
        at: [f32; 3],
    },
    /// `slot`'s `part` came off its robot for the rest of the life: `destroyed` at zero
    /// health, or carried away with a destroyed part it hung from.
    Detached {
        slot: u8,
        part: PartId,
        destroyed: bool,
    },
    /// `slot` armed its self-destruct (`armed`), or cancelled its countdown, standing at `at`
    /// (units, its centre; [`crate::destruct`]).
    Destruct { slot: u8, armed: bool, at: [f32; 3] },
    /// `slot`'s self-destruct went off at `at` (units, its centre): `damage` there, falling off
    /// to nothing `radius` units away (both 0: it carried nothing volatile and only died).
    Detonated {
        slot: u8,
        at: [f32; 3],
        damage: i32,
        radius: i32,
    },
    /// A fighter died: `killer` scored (`None` or the victim itself: a suicide; `weapon` `None`
    /// with a killer other than the victim: that killer's self-destruct).
    Killed {
        victim: u8,
        killer: Option<u8>,
        weapon: Option<Weapon>,
    },
    /// A fighter appeared at a spawn point facing `yaw`.
    Spawned { slot: u8, yaw: u16 },
    /// The round is over.
    RoundOver { winner: Option<u8> },
    /// `slot` took `module` from the map's pad `pad` (its index in [`Arena::module_pads`]).
    ModuleTaken {
        slot: u8,
        pad: u8,
        module: ArenaModule,
    },
    /// `slot` spent its module's last use.
    ModuleSpent { slot: u8, module: ArenaModule },
    /// `slot` died carrying `carried`, and left it where its feet were.
    ModuleDropped { slot: u8, carried: Carried },
    /// `slot` took `carried` (the uses it had left) from where `from` dropped it.
    DropTaken {
        slot: u8,
        from: u8,
        carried: Carried,
    },
}

/// What a hitscan ray struck.
#[derive(Clone, Debug)]
enum Target {
    /// A fighter's parts, each with its share of the damage.
    Fighter(usize, Vec<Blow>),
    Dummy(usize),
    /// A damageable placeable of the arena ([`Scenery::of_body`]).
    Placeable(usize),
    Nothing,
}

/// One resolved hitscan ray: its beam event, what it struck and where.
#[derive(Clone, Debug)]
struct Shot {
    beam: Event,
    target: Target,
    at: [f32; 3],
}

/// The fighters' centres and part boxes after a tick (`None` while dead), for lag-compensated
/// shots.
#[derive(Clone, Debug)]
struct Frame {
    tick: u32,
    poses: Vec<Option<([f32; 3], Hitboxes)>>,
}

/// A destroyed battery's blast waiting to go off this tick: where, and who and what destroyed
/// the battery (nobody and nothing: a collision, [`crate::impact`]).
#[derive(Clone, Copy, Debug)]
struct Blast {
    at: [f32; 3],
    attacker: Option<usize>,
    weapon: Option<Weapon>,
}

#[derive(Debug)]
pub struct Session {
    /// The catalogued map `arena` is ([`Session::on_map`]); `None` for a scripted arena.
    pub map: Option<MapId>,
    /// The arena as it stands: a broken placeable's placement in its broken variant, and its
    /// broken pieces among [`Arena::pieces`] ([`crate::scenery`]).
    pub arena: Arena,
    /// The arena's damageable placeables and their health.
    pub scenery: Scenery,
    /// The arena's scenery and the target dummies: what rays, hooks and flames meet besides
    /// the robots.
    pub world: World,
    /// The fighters' robots, by slot, on the arena's statics.
    pub mechanics: Mechanics,
    pub rules: Rules,
    pub phase: Phase,
    /// One per slot, in slot order.
    pub fighters: Vec<Fighter>,
    pub dummies: Vec<Dummy>,
    pub rockets: Vec<Rocket>,
    /// Shotgun pellets in flight.
    pub pellets: Vec<Pellet>,
    /// The module pads, in the map's order ([`Arena::module_pads`]).
    pub pickups: Vec<Pickup>,
    /// The modules dead carriers dropped, oldest first.
    pub drops: Vec<Drop>,
    /// What the last tick did, in order.
    pub events: Vec<Event>,
    tick: u32,
    history: VecDeque<Frame>,
    /// Damage each slot dealt to others this tick.
    dealt: Vec<i32>,
    /// Battery blasts set off and not yet resolved.
    blasts: VecDeque<Blast>,
}

/// The reference fighter box's half extents (units): a robot's centre is its feet plus its height.
#[allow(clippy::cast_precision_loss)]
pub const HALF: [f32; 3] = [
    PLAYER_HALF_EXTENTS.x as f32,
    PLAYER_HALF_EXTENTS.y as f32,
    PLAYER_HALF_EXTENTS.z as f32,
];

impl Session {
    /// One person alone in `arena` ([`Rules::solo`]): the movement and weapon scripts.
    pub fn new(arena: Arena) -> Result<Self, PhysicsError> {
        Self::with_rules(arena, Rules::solo())
    }

    /// A match on the catalogued `map`: its arena for every round, respawn and snapshot.
    pub fn on_map(map: MapId, rules: Rules) -> Result<Self, PhysicsError> {
        let mut session = Self::with_rules(map.arena(), rules)?;
        session.map = Some(map);
        Ok(session)
    }

    /// A match in `arena`, outside the catalog (the scripted moves and test arenas).
    pub fn with_rules(arena: Arena, rules: Rules) -> Result<Self, PhysicsError> {
        let scenery = Scenery::of(&arena);
        let statics = scenery.statics(&arena);
        let mut world = World::new(GRAVITY);
        for body in &statics {
            world.add_body(body.clone())?;
        }
        world.set_step_up(Some(STEP_UP));
        #[allow(clippy::cast_precision_loss)]
        let gravity = per_second(GRAVITY).map(|axis| axis * f32::from(crate::TICK_HZ) / 100.0);
        let mechanics = Mechanics::for_match(&statics, gravity)?;
        let fighters = (0..rules.slots)
            .map(|slot| {
                let pilot = if rules.local && slot == 0 {
                    Pilot::Human { id: LOCAL_PLAYER }
                } else if rules.bots {
                    Pilot::Bot(Box::new(Bot::new(rules.seed, slot, 0)))
                } else {
                    Pilot::Open
                };
                Fighter::new(slot, pilot)
            })
            .collect();
        let mut session = Self {
            map: None,
            arena,
            scenery,
            world,
            mechanics,
            rules,
            phase: Phase::Playing {
                ends_at: (rules.time_limit_ticks > 0).then_some(rules.time_limit_ticks),
            },
            fighters,
            dummies: Vec::new(),
            rockets: Vec::new(),
            pellets: Vec::new(),
            pickups: Vec::new(),
            drops: Vec::new(),
            events: Vec::new(),
            tick: 0,
            history: VecDeque::new(),
            dealt: vec![0; usize::from(rules.slots)],
            blasts: VecDeque::new(),
        };
        session.reset_pickups();
        session.spawn_everyone()?;
        session.dummies = Dummy::spawn_all(&mut session.world, &session.arena.posts)?;
        session.record();
        Ok(session)
    }

    /// Ticks run so far.
    #[must_use]
    pub const fn ticks(&self) -> u32 {
        self.tick
    }

    /// The fighter in slot 0: the local person in practice and scripts.
    #[must_use]
    pub fn local(&self) -> &Fighter {
        &self.fighters[0]
    }

    /// The slot person `id` holds.
    #[must_use]
    pub fn slot_of_player(&self, id: u32) -> Option<u8> {
        self.fighters
            .iter()
            .find(|fighter| fighter.human() == Some(id))
            .map(|fighter| fighter.slot)
    }

    /// Every fighter at a spawn point of its own (slot order), at the start of a round; one whose
    /// person is in the builder waits until Ready (D2.i70.11).
    fn spawn_everyone(&mut self) -> Result<(), PhysicsError> {
        let points = self.arena.spawn_points();
        for slot in 0..self.fighters.len() {
            let fighter = &mut self.fighters[slot];
            if fighter.pilot == Pilot::Open {
                continue;
            }
            if fighter.intent.edit {
                fighter.respawn_at = Some(self.tick);
                continue;
            }
            self.spawn(slot, points[slot % points.len()])?;
        }
        Ok(())
    }

    /// Puts `slot`'s fighter's robot in the world as its build, whole, standing at `feet`
    /// (units) facing `yaw`, in place of any robot it had.
    fn spawn_robot(&mut self, slot: usize, feet: [f32; 3], yaw: u16) -> Result<(), PhysicsError> {
        let id = u8::try_from(slot).unwrap_or(u8::MAX);
        self.mechanics.despawn(id)?;
        let build = self.fighters[slot].build;
        self.mechanics
            .spawn_at(id, build, feet, mechanics::facing(yaw))?;
        self.refresh(slot)
    }

    /// Reads `slot`'s fighter's stance back from its robot (none without one).
    fn refresh(&mut self, slot: usize) -> Result<(), PhysicsError> {
        let id = u8::try_from(slot).unwrap_or(u8::MAX);
        let fighter = &mut self.fighters[slot];
        let (Some(origin), Some(velocity)) =
            (self.mechanics.root(slot), self.mechanics.velocity(id))
        else {
            fighter.stance = None;
            return Ok(());
        };
        let frames = self.mechanics.part_frames(id)?;
        fighter.stance = Some(Stance {
            origin,
            velocity,
            reach: mechanics::stance(&fighter.build),
            footing: self.mechanics.footing(id),
            hitboxes: Hitboxes::posed(&fighter.build, &fighter.parts, &frames),
        });
        Ok(())
    }

    /// Adds `push` (subunits per tick) to `slot`'s robot's velocity: a blast's or a blade's
    /// knockback, on the whole robot.
    fn push(&mut self, slot: usize, push: [i32; 3]) -> Result<(), PhysicsError> {
        let id = u8::try_from(slot).unwrap_or(u8::MAX);
        let (Some(root), Some(moving)) = (
            self.mechanics.root_motion(slot),
            self.mechanics.velocity(id),
        ) else {
            return Ok(());
        };
        let added = per_second(Vec3i::new(push[0], push[1], push[2]));
        let velocity = std::array::from_fn(|axis| moving[axis] + added[axis]);
        self.mechanics.drive(id, velocity, root.angular_velocity)
    }

    /// The robots catch up with the fighters: the dead leave the world and the parts each has
    /// lost come off its bodies; the large ones carried away whole stay physical debris for a
    /// while, destroyed ones shatter ([`Mechanics::break_off`], shooter#75).
    fn settle_robots(&mut self) -> Result<(), PhysicsError> {
        for slot in 0..self.fighters.len() {
            let id = u8::try_from(slot).unwrap_or(u8::MAX);
            let fighter = &self.fighters[slot];
            if !fighter.alive() {
                if self.mechanics.despawn(id)? {
                    self.fighters[slot].stance = None;
                }
                continue;
            }
            let lost = fighter.parts.lost();
            let intact = fighter
                .parts
                .health()
                .iter()
                .enumerate()
                .filter(|(_, health)| **health > 0)
                .fold(0u32, |mask, (index, _)| mask | (1 << index));
            if self
                .mechanics
                .robot(id)
                .is_some_and(|robot| robot.lost != lost)
                && self.mechanics.break_off(id, lost, intact)?
            {
                self.refresh(slot)?;
            }
        }
        Ok(())
    }

    /// The arena catches up with its placeables ([`crate::scenery`]): each one that broke (or
    /// came back whole) this tick takes its new pieces in the arena, the physics world and the
    /// robots' world, and a hook set on one that broke lets go. Last in the tick, like the
    /// robots' changes: from the next tick on every query meets the new pieces.
    fn settle_scenery(&mut self) -> Result<(), PhysicsError> {
        let changed = self.scenery.settle(&mut self.arena);
        if changed.is_empty() {
            return Ok(());
        }
        for &index in &changed {
            for id in self.scenery.body_ids(index) {
                self.world.remove_body(id);
            }
            for body in self.scenery.bodies(&self.arena, index) {
                self.world.add_body(body)?;
            }
            if self.scenery.placeables()[index].broken() {
                for fighter in &mut self.fighters {
                    if fighter.hook.is_some_and(|hook| {
                        self.scenery.holds(&self.arena, index, hook.anchor_units())
                    }) {
                        fighter.hook = None;
                    }
                }
            }
        }
        // The robots stand as they stood: only the scenery under them changes.
        self.mechanics.reshape(&self.scenery.statics(&self.arena))
    }

    /// The host's word on the arena's placeables (a snapshot's [`crate::Snapshot::scenery`]),
    /// for a predicting client: they stand as the host has them from here on.
    ///
    /// # Errors
    ///
    /// If the state names placeables this map does not break, or the physics world refuses the
    /// new pieces.
    pub fn adopt_scenery(&mut self, state: &[PlaceableState]) -> Result<(), PhysicsError> {
        self.scenery.adopt(state).map_err(crate::physics::error)?;
        self.settle_scenery()
    }

    fn spawn(&mut self, slot: usize, at: Spawn) -> Result<(), PhysicsError> {
        let fighter = &mut self.fighters[slot];
        fighter.motion = Motion::default();
        fighter.build = fighter.configured;
        fighter.hook = None;
        fighter.carried = None;
        fighter.destruct = None;
        // Respawn restores the configured build whole (D2.i70.8), with no repair under way.
        fighter.parts = PartDamage::fresh(&fighter.build);
        fighter.repair = None;
        fighter.arsenal = Arsenal::holding(fighter.first_weapon());
        fighter.energy = fighter.pool().capacity;
        fighter.fuel = fighter.pool().fuel;
        fighter.respawn_at = None;
        fighter.intent = Intent {
            yaw: at.yaw,
            ..Intent::IDLE
        };
        if let Pilot::Bot(bot) = &mut fighter.pilot {
            bot.spawned(at.yaw);
        }
        self.events.push(Event::Spawned {
            slot: fighter.slot,
            yaw: at.yaw,
        });
        // What its last life left lying about retires with it (shooter#75).
        self.mechanics
            .clear_debris(u8::try_from(slot).unwrap_or(u8::MAX))?;
        #[allow(clippy::cast_precision_loss)]
        let feet = [at.feet.x, at.feet.y, at.feet.z].map(|axis| axis as f32);
        self.spawn_robot(slot, feet, at.yaw)
    }

    /// Puts `slot`'s living fighter's robot standing at rest at `feet` (units), facing its yaw:
    /// scripts and tests placing a fighter.
    ///
    /// # Errors
    ///
    /// If the physics world refuses the robot.
    pub fn stand(&mut self, slot: u8, feet: [f32; 3]) -> Result<(), PhysicsError> {
        let slot = usize::from(slot);
        self.fighters[slot].respawn_at = None;
        let yaw = self.fighters[slot].intent.yaw;
        self.spawn_robot(slot, feet, yaw)
    }

    /// `slot`'s fighter is `build` this life, whole, its robot rebuilt where it stands (practice
    /// and scripts; a hosted fighter changes build only at a spawn, [`Session::configure`]).
    ///
    /// # Errors
    ///
    /// If the physics world refuses the robot.
    pub fn rebuild(&mut self, slot: u8, build: Assembly) -> Result<(), PhysicsError> {
        let slot = usize::from(slot);
        let fighter = &mut self.fighters[slot];
        fighter.rebuild(build);
        if !fighter.alive() {
            return Ok(());
        }
        let (feet, yaw) = (fighter.feet(), fighter.intent.yaw);
        self.spawn_robot(slot, feet, yaw)
    }

    /// The spawn point farthest from every other living fighter (the first one when alone).
    fn spawn_point_for(&self, slot: usize) -> Spawn {
        let points = self.arena.spawn_points();
        let others: Vec<Vec3i> = self
            .fighters
            .iter()
            .filter(|fighter| usize::from(fighter.slot) != slot && fighter.alive())
            .map(|fighter| whole(fighter.center()))
            .collect();
        let room = |point: &Spawn| {
            others
                .iter()
                .map(|other| {
                    let d = [
                        i64::from(other.x - point.feet.x),
                        i64::from(other.y - point.feet.y),
                        i64::from(other.z - point.feet.z),
                    ];
                    d.iter().map(|axis| axis * axis).sum::<i64>()
                })
                .min()
                .unwrap_or(i64::MAX)
        };
        let mut best = points[0];
        let mut best_room = room(&best);
        for point in &points[1..] {
            let room = room(point);
            if room > best_room {
                best = *point;
                best_room = room;
            }
        }
        best
    }

    /// Person `id` takes a slot: an open one, else the first bot's. Returns the slot, or `None`
    /// when people hold every slot.
    pub fn join(&mut self, id: u32) -> Result<Option<u8>, PhysicsError> {
        if let Some(slot) = self.slot_of_player(id) {
            return Ok(Some(slot));
        }
        let Some(slot) = self
            .fighters
            .iter()
            .position(|fighter| fighter.pilot == Pilot::Open)
            .or_else(|| {
                self.fighters
                    .iter()
                    .position(|fighter| matches!(fighter.pilot, Pilot::Bot(_)))
            })
        else {
            return Ok(None);
        };
        self.replace_pilot(slot, Pilot::Human { id })?;
        Ok(u8::try_from(slot).ok())
    }

    /// Person `id` leaves: a bot takes the slot (or it opens). Whether `id` held one.
    pub fn leave(&mut self, id: u32) -> Result<bool, PhysicsError> {
        let Some(slot) = self.slot_of_player(id) else {
            return Ok(false);
        };
        let slot = usize::from(slot);
        let pilot = if self.rules.bots {
            Pilot::Bot(Box::new(Bot::new(
                self.rules.seed,
                self.fighters[slot].slot,
                self.tick,
            )))
        } else {
            Pilot::Open
        };
        self.replace_pilot(slot, pilot)?;
        Ok(true)
    }

    fn replace_pilot(&mut self, slot: usize, pilot: Pilot) -> Result<(), PhysicsError> {
        self.mechanics
            .despawn(u8::try_from(slot).unwrap_or(u8::MAX))?;
        let fighter = &mut self.fighters[slot];
        let open = pilot == Pilot::Open;
        *fighter = Fighter::new(fighter.slot, pilot);
        if !open {
            let at = self.spawn_point_for(slot);
            self.spawn(slot, at)?;
        }
        Ok(())
    }

    /// Puts `slot`'s fighter alive with its robot in the exact state `robot` (its build and
    /// lost parts included, whatever slot it was in): a client resuming its prediction from a
    /// snapshot (its part damage is the caller's to set first, matching the robot's lost
    /// parts).
    ///
    /// # Errors
    ///
    /// If the robot's bodies do not match its build or the world refuses one.
    pub fn place(
        &mut self,
        slot: u8,
        robot: &RobotState,
        motion: Motion,
        held: Instance,
        hook: Option<Hook>,
    ) -> Result<(), PhysicsError> {
        self.mechanics.place(RobotState {
            slot,
            ..robot.clone()
        })?;
        let fighter = &mut self.fighters[usize::from(slot)];
        fighter.build = robot.build;
        fighter.motion = motion;
        fighter.arsenal.held = held;
        fighter.hook = hook;
        fighter.respawn_at = None;
        self.refresh(usize::from(slot))
    }

    /// Puts the match's physical debris in the exact state `debris` (a snapshot's): a client
    /// resuming its prediction among the pieces the host has.
    ///
    /// # Errors
    ///
    /// If there are more pieces than [`mechanics::MAX_DEBRIS`] or the world refuses one.
    pub fn place_debris(&mut self, debris: &[mechanics::DebrisState]) -> Result<(), PhysicsError> {
        self.mechanics.place_debris(debris)?;
        for slot in 0..self.fighters.len() {
            self.refresh(slot)?;
        }
        Ok(())
    }

    /// Sets `slot`'s living robot moving at `velocity` (units a second) as one whole, as a
    /// launch would: scripts and tests staging a collision.
    ///
    /// # Errors
    ///
    /// If the physics world refuses the velocity.
    pub fn launch(&mut self, slot: u8, velocity: [f32; 3]) -> Result<(), PhysicsError> {
        let Some(root) = self.mechanics.root_motion(usize::from(slot)) else {
            return Ok(());
        };
        self.mechanics
            .drive(slot, velocity, root.angular_velocity)?;
        self.refresh(usize::from(slot))
    }

    /// Queues one tick of person `id`'s controls; the fighter applies one queued input per tick.
    /// Whether `id` holds a slot.
    pub fn queue(&mut self, id: u32, input: Input) -> bool {
        let Some(slot) = self.slot_of_player(id) else {
            return false;
        };
        let inputs = &mut self.fighters[usize::from(slot)].inputs;
        if inputs.len() >= MAX_QUEUED_INPUTS {
            inputs.pop_front();
        }
        inputs.push_back(input);
        true
    }

    /// Names person `id`'s fighter `raw`, sanitized ([`crate::names::sanitize`]; nothing left
    /// keeps the old name). Whether `id` holds a slot.
    pub fn rename(&mut self, id: u32, raw: &str) -> bool {
        let Some(slot) = self.slot_of_player(id) else {
            return false;
        };
        if let Some(name) = crate::names::sanitize(raw) {
            self.fighters[usize::from(slot)].name = Some(name);
        }
        true
    }

    /// Person `id`'s robot comes back as `build` at its next spawn; the robot alive now keeps
    /// its build (D2.i46.7), unless it has not played yet: a robot that has applied no input
    /// since its person joined is still being set up (the builder before the match,
    /// D2.i70.11) and is `build` at once, whole. `build` is validated
    /// ([`crate::robot::Catalog::validate`]). Whether `id` holds a slot.
    pub fn configure(&mut self, id: u32, build: Assembly) -> bool {
        let Some(slot) = self.slot_of_player(id) else {
            return false;
        };
        let fighter = &mut self.fighters[usize::from(slot)];
        fighter.configured = build;
        if fighter.alive() && fighter.ack == 0 && fighter.build != build {
            // A robot the world refuses (never, for a validated build) stays as it was.
            if self.rebuild(slot, build).is_err() {
                return true;
            }
            let fighter = &mut self.fighters[usize::from(slot)];
            fighter.arsenal = Arsenal::holding(fighter.first_weapon());
            fighter.energy = fighter.pool().capacity;
            fighter.fuel = fighter.pool().fuel;
        }
        true
    }

    /// One tick for the local person (slot 0) holding `intent`, seeing the others as they are now.
    /// `intent` as the default layout would send it for the robot in `slot`: unless it already
    /// commands motors, its movement keys through the robot's preset layout
    /// ([`crate::bindings::Bindings::preset`]), as bots do. For scripts and tests that play
    /// the default layout.
    #[must_use]
    pub fn with_preset(&self, slot: u8, intent: Intent) -> Intent {
        if intent.motors != crate::bindings::Motors::REST {
            return intent;
        }
        Intent {
            motors: preset_motors(&self.mechanics, slot, intent),
            ..intent
        }
    }

    pub fn tick(&mut self, intent: Intent) -> Result<StepReport, PhysicsError> {
        let input = Input {
            sequence: self.fighters[0].ack.wrapping_add(1),
            intent,
            view_tick: self.tick,
        };
        self.queue(LOCAL_PLAYER, input);
        self.step()
    }

    /// One tick at [`crate::TICK_HZ`]: people apply their next input and bots think;
    /// everyone moves, the dummies walk, weapons fire, rockets fly and explode, the world
    /// steps, the fallen die, and the round may end. Reports on slot 0.
    pub fn step(&mut self) -> Result<StepReport, PhysicsError> {
        self.events.clear();
        self.dealt.iter_mut().for_each(|dealt| *dealt = 0);
        // Debris that retired last tick leaves before anything moves a body.
        self.mechanics.retire_debris()?;
        if let Phase::Intermission { restarts_at, .. } = self.phase
            && self.tick >= restarts_at
        {
            self.restart()?;
        }
        let playing = matches!(self.phase, Phase::Playing { .. });

        self.decide();
        for slot in 0..self.fighters.len() {
            let fighter = &self.fighters[slot];
            // In the builder (Edit Robot held), it waits until Ready, however long (D2.i70.11).
            if let Some(at) = fighter.respawn_at
                && playing
                && self.tick >= at
                && !fighter.intent.edit
            {
                let point = self.spawn_point_for(slot);
                self.spawn(slot, point)?;
            }
        }
        // Self-destruct presses arm or cancel a countdown ([`crate::destruct`]).
        self.arm_destructs(playing);

        // How each robot moved after the last step, before this tick's drives: what collision
        // damage measures against ([`crate::impact`]).
        let before: Vec<Option<[f32; 3]>> = self
            .fighters
            .iter()
            .map(|fighter| fighter.stance.as_ref().map(|stance| stance.velocity))
            .collect();
        let mut moves = crate::player::Moves::default();
        // Each robot's electricity before its first request: what the tick used of it.
        let starts: Vec<(u32, u32)> = self
            .fighters
            .iter()
            .map(|fighter| (fighter.energy, fighter.fuel))
            .collect();
        // Where each person whose joints track the aim this tick (shooter#132) has the camera's
        // target, among the others as they saw them: what those joints turn toward.
        let aim_points: Vec<Option<[f32; 3]>> = (0..self.fighters.len())
            .map(|slot| {
                let fighter = &self.fighters[slot];
                (playing && fighter.alive() && fighter.intent.motors.tracking())
                    .then(|| self.aim_point(slot))
            })
            .collect();
        // What each robot's motors are told this tick, by slot.
        let mut motors: Vec<Option<Controls>> = vec![None; self.fighters.len()];
        for (slot, fighter) in self.fighters.iter_mut().enumerate() {
            if !fighter.alive() {
                continue;
            }
            let Some(stance) = fighter.stance.as_ref() else {
                continue;
            };
            // A robot without a connected leg moves on its own wheels and tracks alone
            // (shooter#58, Atlas D2.i74.1): no drive carries or turns it, its locomotion pays by
            // its commands, a jump press fires the thrusters it carries and nothing else, and the
            // arena's jump pads throw it. The interim movement drive below carries only robots
            // with a connected leg (the bounded fallback until legs balance and walk on their
            // own servos in the match).
            if !fighter.connected_leg() {
                let id = u8::try_from(slot).unwrap_or(u8::MAX);
                let intent = if playing {
                    fighter.intent
                } else {
                    Intent {
                        yaw: fighter.intent.yaw,
                        pitch: fighter.intent.pitch,
                        ..Intent::IDLE
                    }
                };
                let footing = Footing {
                    velocity: per_tick(stance.velocity),
                    // Right after a rebuild (a client placing its robot) its contacts are not
                    // known until it steps: the scenery under its feet stands for them, as for the
                    // drive's robots.
                    grounded: stance.footing.feet || on_ground(&self.world, stance),
                    feet: whole(stance.feet()),
                };
                let (thrust, boosted) = pickups::jump(fighter.carried, fighter.energy);
                let launch = fighter.motion.act(
                    &self.arena,
                    intent,
                    footing,
                    fighter.hook.is_some(),
                    boosted.then_some(thrust),
                );
                if let Some(robot) = self.mechanics.robot(id) {
                    let commanded = if playing {
                        fighter.intent.motors
                    } else {
                        crate::bindings::Motors::REST
                    };
                    let commanded = match aim_points[slot] {
                        Some(point) => {
                            let hitboxes = fighter.hitboxes();
                            commanded.resolved(|part| {
                                let (angle, limits) = self.mechanics.joint(id, part)?;
                                aim::track(&fighter.build, &hitboxes, part, angle, limits, point)
                                    .map(crate::mechanics::Command::Angle)
                            })
                        }
                        None => commanded,
                    };
                    let parts = &fighter.parts;
                    let carried =
                        crate::mechanics::load(&fighter.build, |index| parts.attached(index));
                    let pool = fighter.pool();
                    let (controls, stalled) = power_own(
                        (pool, &mut fighter.energy, &mut fighter.fuel),
                        carried,
                        &robot.layout,
                        commanded.controls(&robot.layout),
                    );
                    fighter.flow.stalled = stalled;
                    motors[slot] = Some(controls);
                }
                // A thruster jump is the tick's next request; a pad's throw costs nothing.
                let mut moves_now = launch.moves;
                if let Some(velocity) = launch.velocity {
                    let thrusted = !launch.moves.launched;
                    if !thrusted
                        || energy::spend(&mut fighter.energy, pickups::THRUSTER_JUMP_ENERGY)
                    {
                        let spin = self
                            .mechanics
                            .root_motion(slot)
                            .map_or([0.0; 3], |root| root.angular_velocity);
                        self.mechanics.drive(id, per_second(velocity), spin)?;
                        if thrusted {
                            spend_module(fighter, &mut self.events);
                        }
                    } else {
                        // The motors left too little for the thrusters: no jump.
                        moves_now = crate::player::Moves::default();
                    }
                }
                if fighter.slot == 0 {
                    moves = moves_now;
                }
                continue;
            }
            // A robot with a connected leg: the interim movement drive carries it, jumps it and
            // faces it toward the aim.
            let intent = if playing && fighter.parts.mobile(&fighter.build) {
                fighter.intent
            } else {
                Intent {
                    yaw: fighter.intent.yaw,
                    pitch: fighter.intent.pitch,
                    ..Intent::IDLE
                }
            };
            let hooked = fighter.hook.is_some();
            let footing = Footing {
                velocity: per_tick(stance.velocity),
                grounded: stance.footing.feet
                    || (fighter.traction().connected > 0 && on_ground(&self.world, stance)),
                feet: whole(stance.feet()),
            };
            let id = u8::try_from(slot).unwrap_or(u8::MAX);
            // The motors are the tick's first requests on its electricity ([`crate::energy`],
            // shooter#128): the walk its locomotion does as the movement drive carries it, then
            // each joint as its control layout commands it ([`crate::mechanics::draw`]). Its
            // actuators take what its layout resolved (shooter#74), as far as they can: the
            // host's word on the input ([`crate::bindings::Motors::controls`]). A robot whose
            // locomotion is gone still turns its joints; between rounds everything rests.
            let (intent, controls) = match self.mechanics.robot(id) {
                Some(robot) => {
                    let commanded = if playing {
                        fighter.intent.motors
                    } else {
                        crate::bindings::Motors::REST
                    };
                    // A joint told to track the aim turns toward the angle that points the gun
                    // it carries at the camera's target, within its limits ([`aim::track`]); the
                    // host works it out from its own state, never from the input.
                    let commanded = match aim_points[slot] {
                        Some(point) => {
                            let hitboxes = fighter.hitboxes();
                            commanded.resolved(|part| {
                                let (angle, limits) = self.mechanics.joint(id, part)?;
                                aim::track(&fighter.build, &hitboxes, part, angle, limits, point)
                                    .map(crate::mechanics::Command::Angle)
                            })
                        }
                        None => commanded,
                    };
                    let parts = &fighter.parts;
                    let carried =
                        crate::mechanics::load(&fighter.build, |index| parts.attached(index));
                    let pool = fighter.pool();
                    let (powered, controls, stalled) = power(
                        (pool, &mut fighter.energy, &mut fighter.fuel),
                        carried,
                        &robot.layout,
                        commanded.controls(&robot.layout),
                        intent,
                        footing.grounded && !hooked,
                    );
                    fighter.flow.stalled = stalled;
                    (powered, Some(controls))
                }
                None => (intent, None),
            };
            motors[slot] = controls;
            // A thruster jump is the tick's next request.
            let (jump_velocity, boosted) = pickups::jump(fighter.carried, fighter.energy);
            let origin = stance.origin;
            let traction = fighter.traction();
            let (velocity, moved) = fighter.motion.drive(
                &self.arena,
                intent,
                footing,
                hooked,
                jump_velocity,
                traction,
            );
            // Resting on its body rather than its feet (its legs shot off, tipped over), it lies
            // where it is: the movement drive is all that slides a robot over the ground. One
            // still upright and hung up on a ledge by its chassis, its feet in the air
            // ([`perched`]), steps off it under the drive's air control while it means to move
            // (else it hangs there for good).
            let perched = fighter.parts.mobile(&fighter.build)
                && crate::player::wish_direction(intent).is_some()
                && perched(&self.world, stance);
            let velocity = if stance.footing.any && !stance.footing.feet && !hooked && !perched {
                Vec3i::new(0, velocity.y, 0)
            } else {
                velocity
            };
            let velocity = step_up(&self.world, intent, stance.feet(), velocity);
            let velocity = per_second(velocity);
            let angular_velocity = steer(origin.orientation, intent.yaw);
            self.mechanics.drive(id, velocity, angular_velocity)?;
            if moved.jumped
                && boosted
                && energy::spend(&mut fighter.energy, pickups::THRUSTER_JUMP_ENERGY)
            {
                spend_module(fighter, &mut self.events);
            }
            if fighter.slot == 0 {
                moves = moved;
            }
        }
        for dummy in &mut self.dummies {
            dummy.drive(&mut self.world, self.tick)?;
        }

        let mut local_volley = weapons::Volley::default();
        for slot in 0..self.fighters.len() {
            let fighter = &mut self.fighters[slot];
            if !fighter.alive() {
                continue;
            }
            let intent = fighter.intent;
            let (build, carried, parts) = (fighter.build, fighter.carried, fighter.parts);
            // The grapple module (Atlas D2.i104.2) on its own key, whatever gun is held: a set
            // hook holds while the key stays down and the module is still connected
            // ([`crate::damage`]); let go, or with the module lost or cut off, the rope goes. A
            // robot without a working module has no grapple: the host refuses the key. No
            // second hook until the first is let go. With several grapple modules, the first
            // still connected hooks, and a set hook holds while any is.
            let armed: Vec<Instance> = parts.instances(&build).collect();
            let grapple = parts.first(&build, Weapon::Grapple);
            let grappling = intent.grapple && playing && grapple.is_some();
            if !grappling {
                fighter.hook = None;
            }
            let hook = match grapple.filter(|_| grappling && fighter.hook.is_none()) {
                Some(grapple) => fighter
                    .arsenal
                    .hook(
                        self.tick,
                        tuning_of(&build, carried, grapple).refire_ticks(Weapon::Grapple),
                        || {
                            energy::spend(
                                &mut fighter.energy,
                                tuning_of(&build, carried, grapple).energy(Weapon::Grapple),
                            )
                        },
                    )
                    .then_some(grapple),
                None => None,
            };
            // The grapple's hook, the swing, then the guns ask the shared pool for their energy,
            // first come, first served in that order and the guns by part ID ([`crate::energy`],
            // shooter#127). The guns asked are the held copy while the primary trigger is down
            // and every copy the direct weapon bindings and groups name; each fires if it is
            // ready, its family's salvo limit allows ([`Weapon::salvo`]) and the pool grants it,
            // and a refusal refuses only that copy. A weapon part the robot has lost cannot ask
            // ([`crate::damage`]), so it neither fires nor spends a cooldown; nor can a weapon
            // the build does not carry, nor a part that is no gun. A switch to a weapon copy it
            // may not hold is refused ([`weapons::resolve`]): the host decides, the prediction
            // and replays alike. Every copy of a weapon is its own instance (shooter#125): its
            // own mod, refire and muzzle; the melee key swings the first blade still connected.
            // Each asks its own resource ([`Weapon::resource`], shooter#130): the flamethrower
            // the fuel, the rest the electricity.
            let charge = &mut fighter.energy;
            let fuel = &mut fighter.fuel;
            let held = fighter.arsenal.held;
            let select = intent
                .select
                .and_then(|select| weapons::resolve(select, held, &armed));
            let mut requests: Vec<Instance> = Vec::new();
            if playing {
                if intent.fire {
                    requests.push(select.unwrap_or(held));
                }
                requests.extend(
                    intent
                        .triggers
                        .parts()
                        .iter()
                        .filter_map(|part| armed.iter().find(|armed| armed.part == *part))
                        .filter(|instance| instance.weapon.gun()),
                );
            }
            let volley = fighter.arsenal.trigger_tuned(
                select,
                parts
                    .first(&build, Weapon::EnergyBlade)
                    .filter(|_| intent.melee && playing),
                &requests,
                self.tick,
                |instance| tuning_of(&build, carried, instance).refire_ticks(instance.weapon),
                |instance| {
                    armed.contains(&instance)
                        && energy::pay(
                            Some(instance.weapon.resource()),
                            charge,
                            fuel,
                            tuning_of(&build, carried, instance).energy(instance.weapon),
                        )
                },
            );
            if let Some(grapple) = hook {
                if slot == 0 {
                    local_volley.push(grapple);
                }
                self.fire(slot, grapple, intent)?;
            }
            for &instance in volley.shots() {
                if slot == 0 {
                    local_volley.push(instance);
                }
                self.fire(slot, instance, intent)?;
                // The shot went out boosted; it spends a use of the module that boosted it.
                let fighter = &mut self.fighters[slot];
                if fighter
                    .carried
                    .is_some_and(|carried| carried.module.spent_by_shot(instance.weapon))
                {
                    spend_module(fighter, &mut self.events);
                }
            }
        }
        for slot in 0..self.fighters.len() {
            // The rope is tied at the grapple module (shooter#104): a hook is set only while the
            // robot has one connected, so it has its muzzle.
            let Some(tie) = self.fighters[slot].rope_tie() else {
                continue;
            };
            let Some(hook) = &mut self.fighters[slot].hook else {
                continue;
            };
            let id = u8::try_from(slot).unwrap_or(u8::MAX);
            let (Some(root), Some(moving)) = (
                self.mechanics.root_motion(slot),
                self.mechanics.velocity(id),
            ) else {
                continue;
            };
            let pulled = hook.pull(per_tick(moving), tie);
            self.mechanics
                .drive(id, per_second(pulled), root.angular_velocity)?;
        }

        let mut exploded = false;
        let mut index = 0;
        let mut boxes = self.hitboxes_now();
        while index < self.rockets.len() {
            let owner = self.rockets[index].owner;
            let world = &self.world;
            let flight = self.rockets[index].fly(|origin, translation| {
                projectile_hit(world, &boxes, owner, origin, translation)
            });
            match flight {
                Flight::Flying => index += 1,
                Flight::Fizzled => {
                    self.rockets.remove(index);
                }
                Flight::Exploded { at, .. } => {
                    let owner = self.rockets.remove(index).owner;
                    exploded = true;
                    self.explode(at, owner)?;
                    boxes = self.hitboxes_now();
                }
            }
        }
        self.fly_pellets();
        // Every shot of the tick is in: the self-destructs whose countdown ends go off.
        self.detonate();
        // Every hit of the tick is in: the repairs take their orders and work.
        self.run_repairs(playing);

        // Every request of the tick is in: the generators run ([`crate::energy`]).
        for (fighter, (start, start_fuel)) in self.fighters.iter_mut().zip(starts) {
            if fighter.alive() {
                let used = start.saturating_sub(fighter.energy);
                let fueled = start_fuel.saturating_sub(fighter.fuel);
                let (burned, made) = fighter
                    .pool()
                    .generate(&mut fighter.energy, &mut fighter.fuel);
                fighter.flow = Flow {
                    used,
                    fueled,
                    made,
                    burned,
                    ..fighter.flow
                };
            } else {
                fighter.flow = Flow::default();
            }
        }

        // The robots step on the motors' paid commands. A robot that died this tick, or lost
        // parts, still steps as it was; it leaves, or they come off, right after
        // ([`Session::settle_robots`]), so no change to the world lands between the drive and the
        // step.
        let mut controls = Vec::with_capacity(self.mechanics.robots().len());
        for robot in self.mechanics.robots() {
            let slot = usize::from(robot.slot);
            controls.push(
                motors
                    .get(slot)
                    .copied()
                    .flatten()
                    .unwrap_or(Controls::IDLE),
            );
        }
        for slot in 0..self.fighters.len() {
            let (energy, fuel) = (self.fighters[slot].energy, self.fighters[slot].fuel);
            if let Some(robot) = self
                .mechanics
                .robot_mut(u8::try_from(slot).unwrap_or(u8::MAX))
            {
                robot.charge = energy;
                robot.fuel = fuel;
            }
        }
        // How the tick's drives, pulls and pushes set each robot moving.
        let set: Vec<Option<[f32; 3]>> = (0..self.fighters.len())
            .map(|slot| {
                self.mechanics
                    .velocity(u8::try_from(slot).unwrap_or(u8::MAX))
            })
            .collect();
        self.mechanics.step(&controls)?;
        self.world.step()?;
        self.tick += 1;
        for slot in 0..self.fighters.len() {
            self.refresh(slot)?;
        }
        self.collide(&before, &set);
        #[allow(clippy::cast_precision_loss)]
        for slot in 0..self.fighters.len() {
            let fighter = &self.fighters[slot];
            if fighter.alive() && fighter.feet()[1] < KILL_HEIGHT_UNITS as f32 {
                self.kill(slot, None, None);
            }
        }
        self.settle_robots()?;
        self.settle_scenery()?;
        if playing {
            self.run_pickups();
        }
        let mut report = StepReport {
            fired: local_volley.shots().last().copied(),
            volley: local_volley,
            damage: self.dealt.first().copied().unwrap_or(0),
            exploded,
            jumped: moves.jumped,
            air_jumped: moves.air_jumped,
            launched: moves.launched,
            ..StepReport::default()
        };
        for fighter in &mut self.fighters {
            let grounded = fighter.alive()
                && fighter
                    .stance
                    .as_ref()
                    .is_some_and(|stance| stance.footing.any);
            if fighter.slot == 0 {
                let left_ground = moves.jumped || moves.launched;
                report.grounded = grounded;
                report.landed = grounded && !fighter.grounded && !left_ground;
                report.sliding = fighter.alive() && fighter.motion.sliding;
            }
            fighter.grounded = grounded;
        }
        self.record();
        self.check_round_end();
        Ok(report)
    }

    /// Each fighter's intent for this tick: a person's next queued input (or the last one
    /// again), a bot's decision.
    fn decide(&mut self) {
        let sights: Vec<Sight> = self
            .fighters
            .iter()
            .map(|fighter| {
                let alive = fighter.alive() && fighter.stance.is_some();
                let center = alive.then(|| whole(fighter.center()));
                // Where its root chassis is now: what a bot aims at, the one part whose loss
                // kills.
                let chassis = center.and_then(|_| {
                    let boxes = fighter.hitboxes();
                    Some(whole(boxes.get(PartDamage::root(&fighter.build)?)?.center))
                });
                Sight {
                    center,
                    eye: alive.then(|| whole(fighter.eye())),
                    chassis,
                    heading: (alive && !fighter.connected_leg()).then(|| fighter.heading()),
                    armed: Weapon::ALL.map(|weapon| fighter.parts.armed(&fighter.build, weapon)),
                }
            })
            .collect();
        let tick = self.tick;
        for (slot, fighter) in self.fighters.iter_mut().enumerate() {
            match &mut fighter.pilot {
                Pilot::Open => {}
                Pilot::Human { .. } => {
                    fighter.trim_backlog();
                    if let Some(input) = fighter.inputs.pop_front() {
                        fighter.intent = input.intent;
                        fighter.ack = input.sequence;
                        fighter.view_tick = input.view_tick;
                        fighter.starved = 0;
                    } else {
                        // No input this tick (a late packet): hold the last one, but
                        // never repeat a weapon switch, repair order or self-destruct
                        // press; after a while (a lost connection), stand still looking
                        // the same way.
                        fighter.intent.select = None;
                        fighter.intent.repair = None;
                        fighter.intent.destruct = false;
                        fighter.starved = fighter.starved.saturating_add(1);
                        if fighter.starved >= HOLD_INPUT_TICKS {
                            fighter.intent = Intent {
                                yaw: fighter.intent.yaw,
                                pitch: fighter.intent.pitch,
                                ..Intent::IDLE
                            };
                        }
                    }
                }
                Pilot::Bot(bot) => {
                    if fighter.respawn_at.is_none() {
                        // The copies it still has: what its weapon request may name.
                        let copies: Vec<Instance> =
                            fighter.parts.instances(&fighter.build).collect();
                        fighter.intent =
                            bot.think(slot, &sights, &self.world, &self.arena, &copies, tick);
                        // Its robot's actuators through its build's preset layout, from the
                        // keys its movement would press: the same commands a person's layout
                        // sends ([`crate::bindings`]).
                        fighter.intent.motors =
                            preset_motors(&self.mechanics, fighter.slot, fighter.intent);
                        fighter.view_tick = tick;
                    }
                }
            }
        }
    }

    /// Each living fighter's repair order, then a tick of its repair ([`crate::repair`]), in slot
    /// order: after its jump, swing and shot have asked its pool and every hit of the tick has
    /// landed (a blow on the part or the module has paused it); its motors asked first. An order
    /// the host refuses (the part not eligible now) changes nothing; asking again for the part
    /// under repair keeps the work done. Repairs work only while the round is played.
    fn run_repairs(&mut self, playing: bool) {
        for fighter in &mut self.fighters {
            if !fighter.alive() {
                continue;
            }
            let build = fighter.build;
            let boxes = fighter.hitboxes();
            match fighter.intent.repair.filter(|_| playing) {
                Some(Order::Stop) => fighter.repair = None,
                Some(Order::Start(part))
                    if fighter.repair.is_none_or(|repair| repair.target != part) =>
                {
                    if let Some(started) = repair::start(&build, &fighter.parts, &boxes, part) {
                        fighter.repair = Some(started);
                    }
                }
                _ => {}
            }
            if !playing {
                continue;
            }
            let Some(work) = fighter.repair.as_mut() else {
                continue;
            };
            match work.tick(&build, &mut fighter.parts, &boxes, &mut fighter.energy) {
                Outcome::Done | Outcome::Stopped => fighter.repair = None,
                Outcome::Worked | Outcome::Paused | Outcome::Starved => {}
            }
        }
    }

    fn check_round_end(&mut self) {
        let Phase::Playing { ends_at } = self.phase else {
            return;
        };
        let limit = self.rules.frag_limit;
        let fragged = limit > 0 && self.fighters.iter().any(|f| f.frags >= limit);
        let timed_out = ends_at.is_some_and(|at| self.tick >= at);
        if !(fragged || timed_out) {
            return;
        }
        let winner = self
            .fighters
            .iter()
            .filter(|fighter| fighter.pilot != Pilot::Open)
            .fold(None::<&Fighter>, |best, fighter| match best {
                Some(best) if best.frags >= fighter.frags => Some(best),
                _ => Some(fighter),
            })
            .map(|fighter| fighter.slot);
        self.phase = Phase::Intermission {
            winner,
            restarts_at: self.tick + self.rules.intermission_ticks,
        };
        // The round's drops end with it.
        self.drops.clear();
        // And every self-destruct still counting down.
        for fighter in &mut self.fighters {
            fighter.destruct = None;
        }
        self.events.push(Event::RoundOver { winner });
    }

    /// A fresh round: scores cleared, everyone back at their spawn point, no rockets.
    fn restart(&mut self) -> Result<(), PhysicsError> {
        for fighter in &mut self.fighters {
            self.mechanics.despawn(fighter.slot)?;
            fighter.stance = None;
            fighter.respawn_at = None;
            fighter.frags = 0;
            fighter.deaths = 0;
        }
        self.rockets.clear();
        self.pellets.clear();
        self.reset_pickups();
        // Every placeable comes back whole for the new round.
        self.scenery.restore();
        self.settle_scenery()?;
        self.phase = Phase::Playing {
            ends_at: (self.rules.time_limit_ticks > 0)
                .then_some(self.tick + self.rules.time_limit_ticks),
        };
        self.spawn_everyone()
    }

    /// Every pad of the map ready again and nothing dropped, as at the start of a round.
    fn reset_pickups(&mut self) {
        self.drops.clear();
        self.pickups = self
            .arena
            .module_pads
            .iter()
            .map(|&pad| Pickup { pad, ready_in: 0 })
            .collect();
    }

    /// One tick of the module pads, after the world stepped: empty pads come back, and each ready
    /// pad goes to the living fighter nearest its centre within its reach (the lowest slot on a
    /// tie), in pad order.
    #[allow(clippy::cast_possible_truncation)]
    fn run_pickups(&mut self) {
        for index in 0..self.pickups.len() {
            let pickup = &mut self.pickups[index];
            pickup.ready_in = pickup.ready_in.saturating_sub(1);
            if !pickup.ready() {
                continue;
            }
            let pad = pickup.pad;
            let taker = self
                .fighters
                .iter()
                .filter(|fighter| fighter.alive())
                .filter_map(|fighter| Some((pad.reach(whole(fighter.feet()))?, fighter.slot)))
                .min();
            let Some((_, slot)) = taker else {
                continue;
            };
            self.pickups[index].ready_in = PAD_RESPAWN_TICKS;
            self.fighters[usize::from(slot)].carried = Some(Carried::fresh(pad.module));
            self.events.push(Event::ModuleTaken {
                slot,
                pad: index as u8,
                module: pad.module,
            });
        }
        // Then the drops, oldest first, by the same rule; one not taken lies a tick less.
        let mut index = 0;
        while index < self.drops.len() {
            let drop = self.drops[index];
            let taker = self
                .fighters
                .iter()
                .filter(|fighter| fighter.alive())
                .filter_map(|fighter| Some((drop.reach(whole(fighter.feet()))?, fighter.slot)))
                .min();
            if let Some((_, slot)) = taker {
                self.drops.remove(index);
                self.fighters[usize::from(slot)].carried = Some(drop.carried);
                self.events.push(Event::DropTaken {
                    slot,
                    from: drop.from,
                    carried: drop.carried,
                });
                continue;
            }
            self.drops[index].expires_in -= 1;
            if self.drops[index].expires_in == 0 {
                self.drops.remove(index);
            } else {
                index += 1;
            }
        }
    }

    fn record(&mut self) {
        let poses = self
            .fighters
            .iter()
            .map(|fighter| {
                fighter
                    .stance
                    .as_ref()
                    .filter(|_| fighter.alive())
                    .map(|stance| (fighter.center(), stance.hitboxes.clone()))
            })
            .collect();
        if self.history.len() > MAX_REWIND_TICKS as usize {
            self.history.pop_front();
        }
        self.history.push_back(Frame {
            tick: self.tick,
            poses,
        });
    }

    /// The fighters' centres and part boxes at `tick`, or at the oldest (newest) kept tick
    /// before (after) it.
    fn poses_at(&self, tick: u32) -> &[Option<([f32; 3], Hitboxes)>] {
        let frame = self
            .history
            .iter()
            .find(|frame| frame.tick >= tick)
            .or_else(|| self.history.back());
        frame.map_or(&[], |frame| frame.poses.as_slice())
    }

    /// Every living fighter's connected parts where it is now, by slot (`None`: dead or open).
    fn hitboxes_now(&self) -> Vec<Option<Hitboxes>> {
        self.fighters
            .iter()
            .map(|fighter| fighter.alive().then(|| fighter.hitboxes()))
            .collect()
    }

    /// The other living fighters as `slot`'s person saw them (lag compensation: where their
    /// bodies carried their parts at the snapshot tick of its input, at most
    /// [`MAX_REWIND_TICKS`] back), each with its centre then and the parts it has now.
    fn seen_by(&self, slot: usize) -> Vec<(usize, [f32; 3], Hitboxes)> {
        let rewind = self
            .tick
            .saturating_sub(self.fighters[slot].view_tick)
            .min(MAX_REWIND_TICKS);
        self.poses_at(self.tick - rewind)
            .iter()
            .enumerate()
            .filter(|(other, _)| {
                *other != slot && self.fighters.get(*other).is_some_and(Fighter::alive)
            })
            .filter_map(|(other, pose)| {
                let (center, boxes) = pose.as_ref()?;
                Some((other, *center, boxes.attached(&self.fighters[other].parts)))
            })
            .collect()
    }

    fn dummy_of(&self, body: BodyId) -> Option<usize> {
        self.dummies.iter().position(|dummy| dummy.body == body)
    }

    /// What a ray that stopped on `body` (not a fighter's) strikes: a dummy, a damageable
    /// placeable or nothing that takes damage.
    fn target_of(&self, body: BodyId) -> Target {
        if let Some(dummy) = self.dummy_of(body) {
            Target::Dummy(dummy)
        } else if let Some(placeable) = self.scenery.of_body(body) {
            Target::Placeable(placeable)
        } else {
            Target::Nothing
        }
    }

    fn hurt_dummy(&mut self, index: usize, attacker: usize, damage: i32, at: [f32; 3]) {
        let dummy = &mut self.dummies[index];
        if !dummy.alive() || damage <= 0 {
            return;
        }
        let downed = dummy.hurt(&mut self.world, damage, self.tick);
        self.dealt[attacker] += damage;
        self.events.push(Event::Damaged {
            dummy: index,
            at,
            damage,
            downed,
        });
    }

    /// One strike's `blows` on `victim`'s parts from `attacker`'s `weapon`, in order, then
    /// every battery blast they set off.
    fn strike(&mut self, victim: usize, attacker: usize, blows: &[Blow], weapon: Weapon) {
        for blow in blows {
            self.hurt_part(victim, Some(attacker), *blow, Some(weapon));
        }
        self.settle_blasts();
    }

    /// One blow on one of `victim`'s parts ([`crate::damage`]) by `attacker`'s `weapon`
    /// (nobody's and none: a collision, [`crate::impact`]): its health goes down; at zero
    /// it comes off with what hung from it, a lost battery takes its capacity and its share of
    /// the charge, a destroyed one queues its blast, and a destroyed root chassis
    /// kills.
    #[allow(clippy::cast_possible_truncation)]
    fn hurt_part(
        &mut self,
        victim: usize,
        attacker: Option<usize>,
        blow: Blow,
        weapon: Option<Weapon>,
    ) {
        if !self.fighters[victim].alive() {
            return;
        }
        // Where its parts are before this blow (a destroyed battery blasts where it was).
        let placed = self.fighters[victim]
            .stance
            .as_ref()
            .map(|stance| stance.hitboxes.clone())
            .unwrap_or_default();
        let fighter = &mut self.fighters[victim];
        let build = fighter.build;
        let before = fighter.pool();
        let Some(change) = fighter.parts.hurt(&build, blow.index, blow.damage) else {
            return;
        };
        fighter.energy = before.shrink(fighter.pool(), fighter.energy);
        fighter.fuel = before.shrink_fuel(fighter.pool(), fighter.fuel);
        // A direct hit on the part under repair, or on the module repairing it, pauses it.
        if let Some(repair) = fighter.repair.as_mut() {
            repair.struck(build.parts()[blow.index].id);
        }
        let vital = fighter.chassis_health() == 0;
        if let Some(attacker) = attacker.filter(|attacker| *attacker != victim) {
            self.dealt[attacker] += blow.damage;
        }
        self.events.push(Event::Hit {
            victim: victim as u8,
            attacker: attacker.map(|attacker| attacker as u8),
            part: build.parts()[blow.index].id,
            damage: blow.damage,
            at: blow.at,
        });
        if vital {
            self.kill(victim, attacker, weapon);
            return;
        }
        let destroyed = change.destroyed.then_some(blow.index);
        for (index, part) in build.parts().iter().enumerate() {
            if change.lost & (1 << index) == 0 {
                continue;
            }
            self.events.push(Event::Detached {
                slot: victim as u8,
                part: part.id,
                destroyed: destroyed == Some(index),
            });
        }
        let battery = destroyed.filter(|&index| {
            BETA.part_type(build.parts()[index].kind)
                .is_some_and(|part| part.kind == Kind::Battery)
        });
        if let Some(index) = battery {
            // Where the battery was: as its robot carried it before the blow.
            if let Some(part) = placed.get(index) {
                self.blasts.push_back(Blast {
                    at: part.center,
                    attacker,
                    weapon,
                });
            }
        }
    }

    /// Collision damage ([`crate::impact`]), in slot order: each living robot's impact this
    /// step, measured from how it moved `before` the tick's drives, how they `set` it moving and
    /// how it moves now, strikes its part reaching furthest toward what it hit. Nobody is the
    /// attacker: a robot that dies of it loses a frag, as for a fall.
    fn collide(&mut self, before: &[Option<[f32; 3]>], set: &[Option<[f32; 3]>]) {
        let gravity = self
            .mechanics
            .gravity()
            // m/s² to units (centimetres) a second, one step's worth.
            .map(|axis| axis * 100.0 / f32::from(crate::TICK_HZ));
        for slot in 0..self.fighters.len() {
            let fighter = &self.fighters[slot];
            let (Some(before), Some(set), Some(stance)) = (
                before.get(slot).copied().flatten(),
                set.get(slot).copied().flatten(),
                fighter.stance.as_ref().filter(|_| fighter.alive()),
            ) else {
                continue;
            };
            let Some(impact) = crate::impact::measure(before, set, stance.velocity, gravity) else {
                continue;
            };
            let damage = crate::impact::damage(impact.speed);
            let blows = crate::impact::strike(&stance.hitboxes, impact.direction, damage);
            for blow in blows {
                self.hurt_part(slot, None, blow, None);
            }
            self.settle_blasts();
        }
    }

    /// The queued battery blasts, in order: each damages every connected part within its
    /// small radius through the same resolution as a rocket's splash, and a battery it
    /// destroys queues its own (each battery blasts at most once a life).
    fn settle_blasts(&mut self) {
        while let Some(blast) = self.blasts.pop_front() {
            self.events.push(Event::Exploded { at: blast.at });
            for (victim, parts) in self.hitboxes_now().into_iter().enumerate() {
                let Some(parts) = parts else {
                    continue;
                };
                let blows = parts.blast(blast.at, BATTERY_BLAST_RADIUS_UNITS, BATTERY_BLAST_DAMAGE);
                for blow in blows {
                    self.hurt_part(victim, blast.attacker, blow, blast.weapon);
                }
            }
        }
    }

    /// Each living fighter's self-destruct press, in slot order ([`crate::destruct`]): it arms
    /// a countdown of [`destruct::COUNTDOWN_TICKS`], or cancels the one under way. Only while
    /// the round is played; nothing else is asked of the robot (stranded, empty or airborne, it
    /// may still arm).
    fn arm_destructs(&mut self, playing: bool) {
        if !playing {
            return;
        }
        for fighter in &mut self.fighters {
            if !fighter.alive() || !fighter.intent.destruct {
                continue;
            }
            fighter.destruct = match fighter.destruct {
                Some(_) => None,
                None => Some(self.tick + destruct::COUNTDOWN_TICKS),
            };
            self.events.push(Event::Destruct {
                slot: fighter.slot,
                armed: fighter.destruct.is_some(),
                at: fighter.center(),
            });
        }
    }

    /// The self-destructs whose countdown ends this tick ([`crate::destruct`]): every one of
    /// them dies first (each its own kill), so simultaneous blasts all go off and none is
    /// cancelled by another's; then each blast, in slot order, from the robot's centre with the
    /// volatile contents it held ([`destruct::Charge`]). A blast strikes every other living
    /// robot's connected parts within reach that the arena's geometry does not shelter from it
    /// (by the falloff to each part's nearest point), the damageable placeables in reach and the
    /// dummies in sight; batteries it destroys blast in turn.
    #[allow(clippy::cast_possible_truncation)]
    fn detonate(&mut self) {
        let tick = self.tick;
        let going: Vec<(usize, [f32; 3], destruct::Charge)> = self
            .fighters
            .iter()
            .enumerate()
            .filter(|(_, fighter)| fighter.alive() && fighter.destruct.is_some_and(|at| tick >= at))
            .map(|(slot, fighter)| {
                (
                    slot,
                    fighter.center(),
                    destruct::Charge::of(
                        &fighter.build,
                        &fighter.parts,
                        fighter.energy,
                        fighter.fuel,
                    ),
                )
            })
            .collect();
        for &(slot, _, _) in &going {
            self.kill(slot, Some(slot), None);
        }
        let sheltered = |world: &World, from: [f32; 3], to: [f32; 3]| {
            let along = [0, 1, 2].map(|axis| to[axis] - from[axis]);
            world
                .cast_ray(from, along, |hit| hit.0 < FIRST_PLAYER_BODY)
                .is_some()
        };
        for (slot, at, charge) in going {
            let (damage, radius) = (charge.damage(), charge.radius_units());
            self.events.push(Event::Detonated {
                slot: slot as u8,
                at,
                damage,
                radius,
            });
            if damage <= 0 {
                continue;
            }
            for index in 0..self.scenery.placeables().len() {
                if !self.scenery.placeables()[index].damageable() {
                    continue;
                }
                let Some(point) = self.scenery.nearest(&self.arena, index, at) else {
                    continue;
                };
                if let Some(dealt) = weapons::falloff(at, point, radius, damage) {
                    self.scenery.hurt(index, dealt);
                }
            }
            for index in 0..self.dummies.len() {
                let Some(center) = self.dummies[index].center(&self.world) else {
                    continue;
                };
                if sheltered(&self.world, at, center) {
                    continue;
                }
                if let Some(dealt) = weapons::falloff(at, center, radius, damage) {
                    self.hurt_dummy(index, slot, dealt, center);
                }
            }
            for (victim, parts) in self.hitboxes_now().into_iter().enumerate() {
                let Some(parts) = parts else {
                    continue;
                };
                let blows: Vec<Blow> = parts
                    .blast(at, radius, damage)
                    .into_iter()
                    .filter(|blow| !sheltered(&self.world, at, blow.at))
                    .collect();
                for blow in blows {
                    self.hurt_part(victim, Some(slot), blow, None);
                }
                self.settle_blasts();
            }
        }
    }

    /// `victim` dies; `killer` scores unless it is the victim (or nobody): then the victim
    /// loses a frag.
    #[allow(clippy::cast_possible_truncation)]
    fn kill(&mut self, victim: usize, killer: Option<usize>, weapon: Option<Weapon>) {
        // Its robot leaves the world once this tick's step is done ([`Session::settle_robots`]).
        let fighter = &mut self.fighters[victim];
        fighter.hook = None;
        fighter.repair = None;
        // Death cancels a self-destruct countdown.
        fighter.destruct = None;
        // A carried module's effect ends with its carrier, and the module (its uses left) lies
        // where its feet were, while a round is played and unless it fell out of the arena.
        #[allow(clippy::cast_precision_loss)]
        if let Some(carried) = fighter.carried.take()
            && matches!(self.phase, Phase::Playing { .. })
            && fighter.feet()[1] >= KILL_HEIGHT_UNITS as f32
        {
            self.drops.push(Drop {
                center: whole(fighter.feet()),
                carried,
                from: fighter.slot,
                expires_in: DROP_LIFETIME_TICKS,
            });
            self.events.push(Event::ModuleDropped {
                slot: fighter.slot,
                carried,
            });
        }
        let fighter = &mut self.fighters[victim];
        fighter.respawn_at = Some(self.tick + self.rules.respawn_ticks);
        fighter.deaths += 1;
        match killer.filter(|killer| *killer != victim) {
            Some(killer) => self.fighters[killer].frags += 1,
            None => self.fighters[victim].frags -= 1,
        }
        self.events.push(Event::Killed {
            victim: victim as u8,
            killer: killer.map(|killer| killer as u8),
            weapon,
        });
    }

    /// This session's last tick's own shots as a client predicting its local fighter (slot
    /// `slot` on the host) shows them at once (shooter#102): each ranged shot aimed again as
    /// the host aims it ([`aim::aimed`]), the camera picking its target among `others` (the
    /// other fighters' connected parts where the client draws them) as well as the arena, from
    /// the predicted muzzle (the tube the prediction fired). A rocket's and a shotgun blast's
    /// `Fired` turn toward that target (their projectiles fly on the host, drawn from its
    /// snapshots); a hitscan beam is traced again among `others`, a burning flame's again along
    /// the new aim. The host's hits decide the damage. Blade swings are left to the host's
    /// events; the grapple's hook keeps the prediction's own (its state is reconciled).
    #[must_use]
    pub fn predicted_shots(&self, slot: u8, others: &[Hitboxes]) -> Vec<Event> {
        let me = self.local();
        let owner = body_of(slot);
        let scenery = |body: BodyId| slot_of(body).is_none();
        let fighters = || others.iter().enumerate();
        // The beam or flame of the shot just aimed again, traced again.
        let mut again: Option<Event> = None;
        let mut shots = Vec::new();
        for event in &self.events {
            match *event {
                Event::Fired { weapon, .. } | Event::Beam { weapon, .. }
                    if weapon == Weapon::EnergyBlade => {}
                Event::Fired {
                    weapon,
                    part,
                    tube,
                    from,
                    mut direction,
                    ..
                } => {
                    let instance = Instance { part, weapon };
                    let delivery = me.tuning(instance).delivery(weapon);
                    again = None;
                    if !matches!(delivery, Delivery::Grapple | Delivery::Melee) {
                        // From where it left (a muzzle pushed into a wall already fires from
                        // the wall's face), its barrel as the predicted robot carries it: it
                        // converges only within its cone (shooter#132).
                        let muzzle = aim::Muzzle {
                            mount: from,
                            at: from,
                            axis: me.muzzle(instance).and_then(|muzzle| muzzle.axis),
                        };
                        let shot = aim::aimed(
                            &self.world,
                            me.eye(),
                            Some(muzzle),
                            me.intent.yaw,
                            me.intent.pitch,
                            owner,
                            scenery,
                            fighters(),
                        );
                        let aimed = shot.direction();
                        direction = aim::unit(aimed);
                        again = match delivery {
                            Delivery::Hitscan { range_units, .. } => {
                                let sight = aim::sight(
                                    &self.world,
                                    from,
                                    aimed,
                                    range_units,
                                    owner,
                                    scenery,
                                    fighters(),
                                );
                                Some(Event::Beam {
                                    weapon,
                                    from,
                                    end: sight.end,
                                    normal: match sight.fighter {
                                        Some(_) => direction.map(|axis| -axis),
                                        None => {
                                            sight.scenery.hit.map_or([0.0; 3], |hit| hit.normal)
                                        }
                                    },
                                    struck: sight.met(),
                                })
                            }
                            Delivery::Flame { .. } => {
                                let flame = weapons::trace(
                                    &self.world,
                                    from,
                                    aimed,
                                    weapons::FLAME_REACH_UNITS,
                                    owner,
                                    |body| self.dummy_of(body).is_none(),
                                );
                                Some(Event::Beam {
                                    weapon,
                                    from,
                                    end: flame.end,
                                    normal: flame.hit.map_or([0.0; 3], |hit| hit.normal),
                                    struck: flame.hit.is_some(),
                                })
                            }
                            _ => None,
                        };
                    }
                    shots.push(Event::Fired {
                        slot,
                        weapon,
                        part,
                        tube,
                        from,
                        direction,
                    });
                }
                Event::Beam { weapon, .. } if matches!(again, Some(Event::Beam { weapon: fired, .. }) if fired == weapon) =>
                {
                    shots.extend(again.take());
                }
                Event::Beam { .. } => shots.push(*event),
                _ => {}
            }
        }
        shots
    }

    /// Where `slot`'s camera's centre ray meets the arena or another fighter's part as its person
    /// saw them ([`aim::target`]): its intended target, what its tracking joints aim at
    /// (shooter#132).
    fn aim_point(&self, slot: usize) -> [f32; 3] {
        let fighter = &self.fighters[slot];
        let seen = self.seen_by(slot);
        aim::aimed(
            &self.world,
            fighter.eye(),
            None,
            fighter.intent.yaw,
            fighter.intent.pitch,
            body_of(fighter.slot),
            |hit| slot_of(hit).is_none(),
            seen.iter().map(|(other, _, parts)| (*other, parts)),
        )
        .target
        .end
    }

    /// Fires `weapon` from `slot` ([`aim`]): the camera picks the intended target among the
    /// arena and the fighters where the shooter saw them, and a mounted ranged weapon's shot
    /// leaves its muzzle toward it. A hitscan beam resolves now (lag-compensated), a rocket or a
    /// shotgun blast's pellets start flying, a flame burns what it touches now once it has
    /// ignited. The grapple module's hook leaves its mount's muzzle toward the target too and
    /// sets where its line first meets arena geometry ([`Hook::fire`]). The blade leaves the eye
    /// along the aim.
    fn fire(
        &mut self,
        slot: usize,
        instance: Instance,
        intent: Intent,
    ) -> Result<(), PhysicsError> {
        let weapon = instance.weapon;
        let shooter = &self.fighters[slot];
        // The robot is no body in the scenery's world: rays leave its muzzle and meet the other
        // robots' part boxes.
        let body = body_of(shooter.slot);
        let eye = shooter.eye();
        let delivery = shooter.tuning(instance).delivery(weapon);
        let muzzle = match delivery {
            Delivery::Melee => None,
            _ => shooter.muzzle(instance),
        };
        let seen = self.seen_by(slot);
        let shot = aim::aimed(
            &self.world,
            eye,
            muzzle,
            intent.yaw,
            intent.pitch,
            body,
            |hit| slot_of(hit).is_none(),
            seen.iter().map(|(other, _, parts)| (*other, parts)),
        );
        let (from, direction) = (shot.from, shot.direction());
        let tube = shooter.tube(instance);
        #[allow(clippy::cast_possible_truncation)]
        self.events.push(Event::Fired {
            slot: slot as u8,
            weapon,
            part: instance.part,
            tube,
            from,
            direction: aim::unit(direction),
        });
        // A launcher's next shot leaves its other tube.
        if weapon.tubes() > 1
            && let Some(index) = shooter
                .build
                .parts()
                .iter()
                .position(|part| part.id == instance.part)
        {
            self.fighters[slot].arsenal.launched(index);
        }
        // The weapon's mod in the shooter's build, if any, and the module it carries decide its
        // damage and range.
        match delivery {
            Delivery::Rocket => {
                self.rockets.push(Rocket::launch(body, from, direction));
            }
            Delivery::Grapple => {
                let (hook, trace) = Hook::fire(&self.world, from, direction, body);
                self.fighters[slot].hook = hook;
                self.events.push(Event::Beam {
                    weapon,
                    from,
                    end: trace.end,
                    normal: trace.hit.map_or([0.0; 3], |hit| hit.normal),
                    struck: hook.is_some(),
                });
            }
            Delivery::Melee => return self.swing(slot, intent.yaw, eye, direction),
            Delivery::Hitscan {
                damage,
                range_units,
            } => {
                let shot =
                    self.hitscan(slot, &seen, weapon, (from, direction), range_units, damage);
                self.land(slot, weapon, damage, shot);
            }
            Delivery::Pellets { damage } => {
                self.pellets.extend(
                    weapons::pellets(shot.yaw, shot.pitch)
                        .map(|pellet| Pellet::launch(body, from, pellet, damage)),
                );
            }
            // Still igniting: the muzzle glows, nothing burns yet.
            Delivery::Flame { .. } if !self.fighters[slot].arsenal.ignited() => {}
            Delivery::Flame { damage } => self.burn(slot, weapon, from, direction, damage)?,
        }
        Ok(())
    }

    /// Where one hitscan ray from `from` (the muzzle) along the Q16 `direction` ends and what it
    /// strikes: the arena (and the dummies) as they are now, fighters' parts where the shooter
    /// saw them (`seen`, [`Session::seen_by`]), `damage` shared among the parts it meets
    /// ([`Hitboxes::pierce`]).
    fn hitscan(
        &self,
        slot: usize,
        seen: &[(usize, [f32; 3], Hitboxes)],
        weapon: Weapon,
        (from, direction): ([f32; 3], [i32; 3]),
        range_units: i32,
        damage: i32,
    ) -> Shot {
        let sight = aim::sight(
            &self.world,
            from,
            direction,
            range_units,
            body_of(self.fighters[slot].slot),
            |hit| slot_of(hit).is_none(),
            seen.iter().map(|(other, _, parts)| (*other, parts)),
        );
        let unit = aim::unit(direction);
        if let Some((victim, _)) = sight.fighter
            && let Some((_, _, parts)) = seen.iter().find(|(other, _, _)| *other == victim)
        {
            // Up to the scenery: a plate passes damage on only to parts in front of the wall.
            let reach = sight.scenery.hit.map_or(1.0, |hit| hit.fraction);
            #[allow(clippy::cast_precision_loss)]
            let translation = unit.map(|axis| axis * range_units as f32 * reach);
            let blows = parts.pierce(from, translation, damage, weapon.penetration());
            return Shot {
                beam: Event::Beam {
                    weapon,
                    from,
                    end: sight.end,
                    normal: unit.map(|axis| -axis),
                    struck: true,
                },
                target: Target::Fighter(victim, blows),
                at: sight.end,
            };
        }
        let scenery = sight.scenery;
        Shot {
            beam: Event::Beam {
                weapon,
                from,
                end: scenery.end,
                normal: scenery.hit.map_or([0.0; 3], |hit| hit.normal),
                struck: scenery.hit.is_some(),
            },
            target: scenery
                .hit
                .map_or(Target::Nothing, |hit| self.target_of(hit.body)),
            at: scenery.end,
        }
    }

    /// One tick of every pellet's flight, in launch order: a pellet that meets a robot's part,
    /// a dummy or the arena ends there ([`Event::Pellet`]) and hurts the part or the dummy.
    fn fly_pellets(&mut self) {
        let mut boxes = self.hitboxes_now();
        let mut index = 0;
        while index < self.pellets.len() {
            let owner = self.pellets[index].owner;
            let world = &self.world;
            let travel = self.pellets[index].fly(|origin, translation| {
                projectile_hit(world, &boxes, owner, origin, translation)
            });
            match travel {
                Travel::Flying => index += 1,
                Travel::Spent => {
                    self.pellets.remove(index);
                }
                Travel::Struck {
                    at,
                    normal,
                    body,
                    damage,
                    origin,
                    translation,
                } => {
                    let pellet = self.pellets.remove(index);
                    let victim = slot_of(body).map(usize::from);
                    let blows = victim
                        .and_then(|victim| boxes.get(victim)?.as_ref())
                        .map(|parts| {
                            parts.pierce(origin, translation, damage, Weapon::Shotgun.penetration())
                        })
                        .unwrap_or_default();
                    let at = blows.first().map_or(at, |blow| blow.at);
                    self.events.push(Event::Pellet {
                        from: pellet.from_units(),
                        at,
                        normal,
                        launched: pellet.age == 0,
                    });
                    let Some(attacker) = slot_of(pellet.owner).map(usize::from) else {
                        continue;
                    };
                    if let Some(victim) = victim {
                        self.strike(victim, attacker, &blows, Weapon::Shotgun);
                        boxes = self.hitboxes_now();
                    } else {
                        match self.target_of(body) {
                            Target::Dummy(dummy) => self.hurt_dummy(dummy, attacker, damage, at),
                            Target::Placeable(index) => {
                                self.scenery.hurt(index, damage);
                            }
                            Target::Fighter(..) | Target::Nothing => {}
                        }
                    }
                }
            }
        }
    }

    /// Shows the shot's beam, then deals its damage to what it struck.
    fn land(&mut self, slot: usize, weapon: Weapon, damage: i32, shot: Shot) {
        self.events.push(shot.beam);
        match shot.target {
            Target::Fighter(victim, blows) => self.strike(victim, slot, &blows, weapon),
            Target::Dummy(index) => self.hurt_dummy(index, slot, damage, shot.at),
            Target::Placeable(index) => {
                self.scenery.hurt(index, damage);
            }
            Target::Nothing => {}
        }
    }

    /// One tick of `slot`'s flame from `eye` along the Q16 `direction` ([`weapons::flame`]):
    /// `damage` to every dummy in it and to the part of every fighter in it nearest the eye, as
    /// they are now, the owner excepted.
    fn burn(
        &mut self,
        slot: usize,
        weapon: Weapon,
        eye: [f32; 3],
        direction: [i32; 3],
        damage: i32,
    ) -> Result<(), PhysicsError> {
        let owner = body_of(self.fighters[slot].slot);
        let (flame, burning) = weapons::flame(&self.world, eye, direction, owner, |body| {
            self.dummy_of(body).is_some()
        })?;
        self.events.push(Event::Beam {
            weapon,
            from: eye,
            end: flame.end,
            normal: flame.hit.map_or([0.0; 3], |hit| hit.normal),
            struck: flame.hit.is_some(),
        });
        // The other robots whose parts come within the flame's radius of its path, in slot
        // order, then the dummies in it.
        let axis = [0, 1, 2].map(|index| flame.end[index] - eye[index]);
        #[allow(clippy::cast_precision_loss)]
        let radius = weapons::FLAME_RADIUS_UNITS as f32;
        let mut targets: Vec<(Target, [f32; 3])> = self
            .fighters
            .iter()
            .enumerate()
            .filter(|(victim, fighter)| *victim != slot && fighter.alive())
            .filter_map(|(victim, fighter)| {
                let parts = fighter.hitboxes();
                parts.grown(radius).first(eye, axis)?;
                let (index, at) = parts.nearest(eye)?;
                let blow = Blow { index, damage, at };
                Some((Target::Fighter(victim, vec![blow]), at))
            })
            .collect();
        targets.extend(burning.into_iter().filter_map(|body| {
            let center = self.world.body(body)?.exact_position();
            Some((
                Target::Dummy(self.dummy_of(body)?),
                melee::contact(eye, center),
            ))
        }));
        // The placeable the flame stops on burns too.
        if let Some(hit) = flame.hit
            && let Some(index) = self.scenery.of_body(hit.body)
        {
            targets.push((Target::Placeable(index), flame.end));
        }
        for (target, at) in targets {
            match target {
                Target::Fighter(victim, blows) => self.strike(victim, slot, &blows, weapon),
                Target::Dummy(index) => self.hurt_dummy(index, slot, damage, at),
                Target::Placeable(index) => {
                    self.scenery.hurt(index, damage);
                }
                Target::Nothing => {}
            }
        }
        Ok(())
    }

    /// `slot`'s energy blade swung toward `yaw` from `eye` (units): every fighter (where the
    /// swinger saw it, like hitscan) and dummy it covers ([`melee::covers`]) and sees past the
    /// arena is struck (a fighter on its part nearest the eye), shoved and hurt; a swing that
    /// strikes nobody may still spark on a wall within reach.
    fn swing(
        &mut self,
        slot: usize,
        yaw: u16,
        eye: [f32; 3],
        direction: [i32; 3],
    ) -> Result<(), PhysicsError> {
        let swinger = &self.fighters[slot];
        let body = body_of(swinger.slot);
        let feet = swinger.feet();
        let in_sight = |world: &World, center: [f32; 3]| {
            let along = [0, 1, 2].map(|axis| center[axis] - eye[axis]);
            world
                .cast_ray(eye, along, |hit| hit.0 < FIRST_PLAYER_BODY)
                .is_none()
        };
        let covered = |center: [f32; 3]| melee::covers(feet, yaw, center);
        let fighters: Vec<(usize, Blow)> = self
            .seen_by(slot)
            .into_iter()
            .filter(|(_, center, _)| covered(*center) && in_sight(&self.world, *center))
            .filter_map(|(other, _, parts)| {
                let (index, at) = parts.nearest(eye)?;
                Some((
                    other,
                    Blow {
                        index,
                        damage: melee::DAMAGE,
                        at,
                    },
                ))
            })
            .collect();
        let dummies: Vec<(usize, [f32; 3])> = (0..self.dummies.len())
            .filter_map(|index| Some((index, self.dummies[index].center(&self.world)?)))
            .filter(|(_, center)| covered(*center) && in_sight(&self.world, *center))
            .collect();
        let strike = |at: [f32; 3]| {
            let normal = [eye[0] - at[0], 0.0, eye[2] - at[2]];
            let length = libm::hypotf(normal[0], normal[2]).max(f32::EPSILON);
            Event::Beam {
                weapon: Weapon::EnergyBlade,
                from: eye,
                end: at,
                normal: normal.map(|axis| axis / length),
                struck: true,
            }
        };
        if fighters.is_empty() && dummies.is_empty() {
            let wall = weapons::trace(
                &self.world,
                eye,
                direction,
                melee::REACH_UNITS,
                body,
                |hit| hit.0 < FIRST_PLAYER_BODY,
            );
            if let Some(hit) = wall.hit {
                self.events.push(Event::Beam {
                    weapon: Weapon::EnergyBlade,
                    from: eye,
                    end: wall.end,
                    normal: hit.normal,
                    struck: true,
                });
                // A placeable within reach takes the blade.
                if let Some(index) = self.scenery.of_body(hit.body) {
                    self.scenery.hurt(index, melee::DAMAGE);
                }
            }
            return Ok(());
        }
        for (victim, blow) in fighters {
            self.events.push(strike(blow.at));
            self.push(victim, melee::knockback(yaw))?;
            self.strike(victim, slot, &[blow], Weapon::EnergyBlade);
        }
        for (index, center) in dummies {
            let at = melee::contact(eye, center);
            self.events.push(strike(at));
            self.hurt_dummy(index, slot, melee::DAMAGE, at);
        }
        Ok(())
    }

    /// A blast at `at` from `owner`'s rocket: splash damage and knockback to every standing
    /// dummy, and to every fighter in reach knockback (by its centre) and splash damage on each
    /// of its parts within reach ([`Hitboxes::blast`]). The owner takes [`SELF_SPLASH_PERCENT`]
    /// of its share (and the full push: rocket jumps).
    fn explode(&mut self, at: [f32; 3], owner: BodyId) -> Result<(), PhysicsError> {
        self.events.push(Event::Exploded { at });
        let attacker = slot_of(owner).map_or(0, usize::from);
        for index in 0..self.dummies.len() {
            let Some(center) = self.dummies[index].center(&self.world) else {
                continue;
            };
            if let Some((damage, _)) = weapons::splash(at, center) {
                self.hurt_dummy(index, attacker, damage, center);
            }
        }
        // Every damageable placeable in reach, by the point of its pieces nearest the blast.
        for index in 0..self.scenery.placeables().len() {
            if !self.scenery.placeables()[index].damageable() {
                continue;
            }
            let Some(point) = self.scenery.nearest(&self.arena, index, at) else {
                continue;
            };
            if let Some(damage) = weapons::falloff(
                at,
                point,
                weapons::SPLASH_RADIUS_UNITS,
                weapons::SPLASH_DAMAGE,
            ) {
                self.scenery.hurt(index, damage);
            }
        }
        for slot in 0..self.fighters.len() {
            if !self.fighters[slot].alive() {
                continue;
            }
            let center = self.fighters[slot].center();
            if let Some((_, push)) = weapons::splash(at, center) {
                self.push(slot, push)?;
            }
            let fighter = &self.fighters[slot];
            let blows: Vec<Blow> = fighter
                .hitboxes()
                .blast(at, weapons::SPLASH_RADIUS_UNITS, weapons::SPLASH_DAMAGE)
                .into_iter()
                .map(|blow| Blow {
                    damage: if slot == attacker {
                        blow.damage * SELF_SPLASH_PERCENT / 100
                    } else {
                        blow.damage
                    },
                    ..blow
                })
                .collect();
            self.strike(slot, attacker, &blows, Weapon::RocketLauncher);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::upright;

    /// Only a robot leaning less than 45° may step off a ledge it is hung on: one tipped over
    /// lies where it is (`tests/collisions.rs` steps an upright one off a block's lip).
    #[test]
    fn only_a_robot_leaning_less_than_45_degrees_is_upright() {
        // Upright, then turned half round about the vertical: upright either way.
        assert!(upright([0.0, 0.0, 0.0, 1.0]));
        assert!(upright([0.0, 1.0, 0.0, 0.0]));
        // Leaning 30° forward (about X) and 40° sideways (about Z): upright.
        assert!(upright([0.258_819, 0.0, 0.0, 0.965_926]));
        assert!(upright([0.0, 0.0, 0.342_020, 0.939_693]));
        // Leaning 50°, on its side and upside down: tipped over.
        assert!(!upright([0.422_618, 0.0, 0.0, 0.906_308]));
        let half = std::f32::consts::FRAC_1_SQRT_2;
        assert!(!upright([0.0, 0.0, half, half]));
        assert!(!upright([1.0, 0.0, 0.0, 0.0]));
    }
}
