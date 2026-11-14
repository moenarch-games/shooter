//! The arena simulation: the world, the fighters in their slots (people and bots) with their
//! weapons, health and scores, the rockets in flight and the target dummies, advanced one fixed
//! tick at a time. The offline practice mode runs it in the client; game-server runs the same
//! state for a hosted match (`shooter-server`), and the clients draw it and play its [`Event`]s.
//!
//! A deathmatch: every fighter has [`MAX_HEALTH`], a kill scores a frag for the killer and a
//! death for the victim (a suicide, by a fall or one's own rocket, costs a frag), the dead come
//! back at the spawn point farthest from everyone else after [`Rules::respawn_ticks`], and a
//! round ends at the frag limit or the time limit, followed by an intermission and a fresh round.
//!
//! Hitscan shots and blade swings are lag-compensated: a person's shot is checked against the
//! fighters where that person saw them (the snapshot tick its command names, at most
//! [`MAX_REWIND_TICKS`] back), while the arena's geometry blocks the ray as it is now. Rockets
//! and shotgun pellets are projectiles: they fly through the world as it is each tick and meet
//! fighters where they are then, like the flame.

use crate::aim;
use crate::arena::{Arena, FIRST_DUMMY_BODY, FIRST_PLAYER_BODY, Spawn};
use crate::bots::{Bot, Sight};
use crate::dummies::Dummy;
use crate::grapple::Hook;
use crate::maps::MapId;
use crate::melee;
use crate::physics::{BodyId, PhysicsError, Vec3i, World};
use crate::player::{
    EYE_HEIGHT_UNITS, GRAVITY, Intent, KILL_HEIGHT_UNITS, PLAYER_HALF_EXTENTS, Player, STEP_UP,
};
use crate::robot::{Assembly, BETA};
use crate::weapons::{self, Arsenal, Delivery, Flight, Pellet, Rocket, Travel, Weapon};
use std::collections::VecDeque;

/// A fighter's health pool (large, Atlas D2.3).
pub const MAX_HEALTH: i32 = 200;
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
    pub player: Player,
    pub arsenal: Arsenal,
    /// The robot it is built as this life ([`crate::robot`]).
    pub build: Assembly,
    /// The build it comes back as at its next spawn (D2.i46.7: rebuilt only between lives).
    pub configured: Assembly,
    /// The grapple's hook while it is set.
    pub hook: Option<Hook>,
    pub health: i32,
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
            player: Player {
                body: body_of(slot),
                motion: crate::player::Motion::default(),
            },
            arsenal: Arsenal::default(),
            build: Assembly::default(),
            configured: Assembly::default(),
            hook: None,
            health: 0,
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
    /// dropped weapon switch carries over to the next input).
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
}

/// The physics body of a slot's fighter.
#[must_use]
pub const fn body_of(slot: u8) -> BodyId {
    BodyId(FIRST_PLAYER_BODY + slot as u64)
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
    /// The weapon that fired this tick.
    pub fired: Option<Weapon>,
    /// Damage dealt to others (fighters and dummies) this tick.
    pub damage: i32,
    /// A rocket exploded this tick.
    pub exploded: bool,
}

/// Something a tick did that the clients show or sound.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    /// A shot left `slot`'s eye (units) along `direction` (unit vector).
    Fired {
        slot: u8,
        weapon: Weapon,
        eye: [f32; 3],
        direction: [f32; 3],
    },
    /// A hitscan beam from the eye to `end`; `struck` when it met a body there.
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
    /// `launched` when it was fired this tick (`from` is the shooter's eye).
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
    /// A fighter took damage from `attacker` (itself for its own rocket; `None`: the arena).
    Hit {
        victim: u8,
        attacker: Option<u8>,
        damage: i32,
        at: [f32; 3],
    },
    /// A fighter died: `killer` scored (`None` or the victim itself: a suicide).
    Killed {
        victim: u8,
        killer: Option<u8>,
        weapon: Option<Weapon>,
    },
    /// A fighter appeared at a spawn point facing `yaw`.
    Spawned { slot: u8, yaw: u16 },
    /// The round is over.
    RoundOver { winner: Option<u8> },
}

/// What a hitscan ray struck.
#[derive(Clone, Copy, Debug)]
enum Target {
    Fighter(usize),
    Dummy(usize),
    Nothing,
}

/// One resolved hitscan ray: its beam event, what it struck and where.
#[derive(Clone, Copy, Debug)]
struct Shot {
    beam: Event,
    target: Target,
    at: [f32; 3],
}

/// The fighters' centres after a tick (`None` while dead), for lag-compensated shots.
#[derive(Clone, Debug)]
struct Frame {
    tick: u32,
    centers: Vec<Option<[f32; 3]>>,
}

#[derive(Debug)]
pub struct Session {
    /// The catalogued map `arena` is ([`Session::on_map`]); `None` for a scripted arena.
    pub map: Option<MapId>,
    pub arena: Arena,
    pub world: World,
    pub rules: Rules,
    pub phase: Phase,
    /// One per slot, in slot order.
    pub fighters: Vec<Fighter>,
    pub dummies: Vec<Dummy>,
    pub rockets: Vec<Rocket>,
    /// Shotgun pellets in flight.
    pub pellets: Vec<Pellet>,
    /// What the last tick did, in order.
    pub events: Vec<Event>,
    tick: u32,
    history: VecDeque<Frame>,
    /// Damage each slot dealt to others this tick.
    dealt: Vec<i32>,
}

/// A fighter's half extents (units) as lag-compensated shots see it ([`weapons::ray_box`]).
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
        let mut world = arena.build_world(GRAVITY)?;
        world.set_step_up(Some(STEP_UP));
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
            world,
            rules,
            phase: Phase::Playing {
                ends_at: (rules.time_limit_ticks > 0).then_some(rules.time_limit_ticks),
            },
            fighters,
            dummies: Vec::new(),
            rockets: Vec::new(),
            pellets: Vec::new(),
            events: Vec::new(),
            tick: 0,
            history: VecDeque::new(),
            dealt: vec![0; usize::from(rules.slots)],
        };
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

    /// Every fighter at a spawn point of its own (slot order), at the start of a round.
    fn spawn_everyone(&mut self) -> Result<(), PhysicsError> {
        let points = self.arena.spawn_points();
        for slot in 0..self.fighters.len() {
            if self.fighters[slot].pilot != Pilot::Open {
                self.spawn(slot, points[slot % points.len()])?;
            }
        }
        Ok(())
    }

    fn spawn(&mut self, slot: usize, at: Spawn) -> Result<(), PhysicsError> {
        let fighter = &mut self.fighters[slot];
        fighter.player = Player::spawn(&mut self.world, fighter.player.body, at.feet)?;
        fighter.arsenal = Arsenal::default();
        fighter.build = fighter.configured;
        fighter.hook = None;
        fighter.health = MAX_HEALTH;
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
        Ok(())
    }

    /// The spawn point farthest from every other living fighter (the first one when alone).
    fn spawn_point_for(&self, slot: usize) -> Spawn {
        let points = self.arena.spawn_points();
        let others: Vec<Vec3i> = self
            .fighters
            .iter()
            .filter(|fighter| usize::from(fighter.slot) != slot && fighter.alive())
            .filter_map(|fighter| self.world.body(fighter.player.body))
            .map(crate::physics::RigidBody::position)
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
        let fighter = &mut self.fighters[slot];
        if fighter.alive() {
            self.world.remove_body(fighter.player.body);
        }
        let open = pilot == Pilot::Open;
        *fighter = Fighter::new(fighter.slot, pilot);
        if !open {
            let at = self.spawn_point_for(slot);
            self.spawn(slot, at)?;
        }
        Ok(())
    }

    /// Puts `slot`'s fighter alive at an exact physics state: a client resuming its prediction
    /// from a snapshot.
    pub fn place(
        &mut self,
        slot: u8,
        position: [f32; 3],
        velocity: [f32; 3],
        motion: crate::player::Motion,
        held: Weapon,
        hook: Option<Hook>,
    ) -> Result<(), PhysicsError> {
        let fighter = &mut self.fighters[usize::from(slot)];
        self.world.remove_body(fighter.player.body);
        self.world.add_body(crate::physics::RigidBody::moving(
            fighter.player.body,
            position,
            velocity,
            PLAYER_HALF_EXTENTS,
        ))?;
        fighter.player.motion = motion;
        fighter.arsenal.held = held;
        fighter.hook = hook;
        fighter.respawn_at = None;
        fighter.health = MAX_HEALTH;
        Ok(())
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
    /// its build (D2.i46.7). `build` is validated ([`crate::robot::Catalog::validate`]).
    /// Whether `id` holds a slot.
    pub fn configure(&mut self, id: u32, build: Assembly) -> bool {
        let Some(slot) = self.slot_of_player(id) else {
            return false;
        };
        self.fighters[usize::from(slot)].configured = build;
        true
    }

    /// One tick for the local person (slot 0) holding `intent`, seeing the others as they are now.
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
        if let Phase::Intermission { restarts_at, .. } = self.phase
            && self.tick >= restarts_at
        {
            self.restart()?;
        }
        let playing = matches!(self.phase, Phase::Playing { .. });

        self.decide();
        for slot in 0..self.fighters.len() {
            let fighter = &self.fighters[slot];
            if let Some(at) = fighter.respawn_at
                && playing
                && self.tick >= at
            {
                let point = self.spawn_point_for(slot);
                self.spawn(slot, point)?;
            }
        }

        let mut moves = crate::player::Moves::default();
        for fighter in &mut self.fighters {
            if !fighter.alive() {
                continue;
            }
            let intent = if playing {
                fighter.intent
            } else {
                Intent {
                    yaw: fighter.intent.yaw,
                    pitch: fighter.intent.pitch,
                    ..Intent::IDLE
                }
            };
            let hooked = fighter.hook.is_some();
            let moved = fighter
                .player
                .apply(&mut self.world, &self.arena, intent, hooked)?;
            if fighter.slot == 0 {
                moves = moved;
            }
        }
        for dummy in &mut self.dummies {
            dummy.drive(&mut self.world, self.tick)?;
        }

        let mut local_fired = None;
        for slot in 0..self.fighters.len() {
            let fighter = &mut self.fighters[slot];
            if !fighter.alive() {
                continue;
            }
            let intent = fighter.intent;
            // A set hook holds while the trigger stays down: no second hook until it is let go.
            let hooked = fighter.hook.is_some();
            let build = fighter.build;
            let fired = fighter.arsenal.trigger_tuned(
                intent.select,
                intent.melee && playing,
                intent.fire && playing && !hooked,
                self.tick,
                |weapon| BETA.tuning(&build, weapon).refire_ticks(weapon),
            );
            if hooked && !(intent.fire && playing && fighter.arsenal.held == Weapon::Grapple) {
                fighter.hook = None;
            }
            if let Some(weapon) = fired {
                if slot == 0 {
                    local_fired = Some(weapon);
                }
                self.fire(slot, weapon, intent)?;
            }
        }
        for fighter in &mut self.fighters {
            if let Some(hook) = &mut fighter.hook {
                let eye = fighter.player.eye(&self.world);
                hook.pull(&mut self.world, fighter.player.body, eye)?;
            }
        }

        let mut exploded = false;
        let mut index = 0;
        while index < self.rockets.len() {
            match self.rockets[index].fly(&self.world) {
                Flight::Flying => index += 1,
                Flight::Fizzled => {
                    self.rockets.remove(index);
                }
                Flight::Exploded { at, .. } => {
                    let owner = self.rockets.remove(index).owner;
                    exploded = true;
                    self.explode(at, owner)?;
                }
            }
        }
        self.fly_pellets();

        self.world.step()?;
        self.tick += 1;
        for slot in 0..self.fighters.len() {
            let fighter = &self.fighters[slot];
            if fighter.alive() && fighter.player.feet(&self.world)[1] < KILL_HEIGHT_UNITS as f32 {
                self.kill(slot, None, None);
            }
        }
        let mut report = StepReport {
            fired: local_fired,
            damage: self.dealt.first().copied().unwrap_or(0),
            exploded,
            jumped: moves.jumped,
            air_jumped: moves.air_jumped,
            launched: moves.launched,
            ..StepReport::default()
        };
        for fighter in &mut self.fighters {
            let grounded = fighter.alive() && self.world.grounded(fighter.player.body);
            if fighter.slot == 0 {
                let left_ground = moves.jumped || moves.launched;
                report.grounded = grounded;
                report.landed = grounded && !fighter.grounded && !left_ground;
                report.sliding = fighter.alive() && fighter.player.motion.sliding;
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
                let center = fighter
                    .alive()
                    .then(|| self.world.body(fighter.player.body))
                    .flatten()
                    .map(crate::physics::RigidBody::position);
                Sight {
                    center,
                    eye: center.map(|center| {
                        Vec3i::new(
                            center.x,
                            center.y - PLAYER_HALF_EXTENTS.y + EYE_HEIGHT_UNITS,
                            center.z,
                        )
                    }),
                    health: fighter.health,
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
                        // never repeat a weapon switch; after a while (a lost
                        // connection), stand still looking the same way.
                        fighter.intent.select = None;
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
                        fighter.intent = bot.think(
                            slot,
                            &sights,
                            &self.world,
                            &self.arena,
                            fighter.arsenal.held,
                            tick,
                        );
                        fighter.view_tick = tick;
                    }
                }
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
        self.events.push(Event::RoundOver { winner });
    }

    /// A fresh round: scores cleared, everyone back at their spawn point, no rockets.
    fn restart(&mut self) -> Result<(), PhysicsError> {
        for fighter in &mut self.fighters {
            if fighter.alive() {
                self.world.remove_body(fighter.player.body);
            }
            fighter.respawn_at = None;
            fighter.frags = 0;
            fighter.deaths = 0;
        }
        self.rockets.clear();
        self.pellets.clear();
        self.phase = Phase::Playing {
            ends_at: (self.rules.time_limit_ticks > 0)
                .then_some(self.tick + self.rules.time_limit_ticks),
        };
        self.spawn_everyone()
    }

    fn record(&mut self) {
        let centers = self
            .fighters
            .iter()
            .map(|fighter| fighter.alive().then(|| fighter.player.center(&self.world)))
            .collect();
        if self.history.len() > MAX_REWIND_TICKS as usize {
            self.history.pop_front();
        }
        self.history.push_back(Frame {
            tick: self.tick,
            centers,
        });
    }

    /// The fighters' centres at `tick`, or at the oldest (newest) kept tick before (after) it.
    fn centers_at(&self, tick: u32) -> &[Option<[f32; 3]>] {
        let frame = self
            .history
            .iter()
            .find(|frame| frame.tick >= tick)
            .or_else(|| self.history.back());
        frame.map_or(&[], |frame| frame.centers.as_slice())
    }

    fn dummy_of(&self, body: BodyId) -> Option<usize> {
        self.dummies.iter().position(|dummy| dummy.body == body)
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

    fn hurt(&mut self, victim: usize, attacker: usize, damage: i32, weapon: Weapon, at: [f32; 3]) {
        let fighter = &mut self.fighters[victim];
        if !fighter.alive() || damage <= 0 {
            return;
        }
        fighter.health -= damage;
        if attacker != victim {
            self.dealt[attacker] += damage;
        }
        #[allow(clippy::cast_possible_truncation)]
        self.events.push(Event::Hit {
            victim: victim as u8,
            attacker: Some(attacker as u8),
            damage,
            at,
        });
        if self.fighters[victim].health <= 0 {
            self.kill(victim, Some(attacker), Some(weapon));
        }
    }

    /// `victim` dies; `killer` scores unless it is the victim (or nobody): then the victim
    /// loses a frag.
    #[allow(clippy::cast_possible_truncation)]
    fn kill(&mut self, victim: usize, killer: Option<usize>, weapon: Option<Weapon>) {
        let fighter = &mut self.fighters[victim];
        self.world.remove_body(fighter.player.body);
        fighter.health = 0;
        fighter.hook = None;
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

    /// Fires `weapon` from `slot`'s eye: a hitscan beam resolves now (lag-compensated), a rocket
    /// or a shotgun blast's pellets start flying, a flame burns what it touches now once it has
    /// ignited.
    fn fire(&mut self, slot: usize, weapon: Weapon, intent: Intent) -> Result<(), PhysicsError> {
        let shooter = &self.fighters[slot];
        let body = shooter.player.body;
        let eye = shooter.player.eye(&self.world);
        let direction = weapons::aim(intent.yaw, intent.pitch);
        #[allow(clippy::cast_precision_loss)]
        let unit = direction.map(|axis| axis as f32 / crate::trig::TRIG_ONE as f32);
        #[allow(clippy::cast_possible_truncation)]
        self.events.push(Event::Fired {
            slot: slot as u8,
            weapon,
            eye,
            direction: unit,
        });
        // The weapon's mod in the shooter's build, if any, decides its damage and range.
        match BETA.tuning(&shooter.build, weapon).delivery(weapon) {
            Delivery::Rocket => {
                self.rockets.push(Rocket::launch(body, eye, direction));
            }
            Delivery::Grapple => {
                let (hook, trace) = Hook::fire(&self.world, eye, direction, body);
                self.fighters[slot].hook = hook;
                self.events.push(Event::Beam {
                    weapon,
                    from: eye,
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
                let shot = self.hitscan(slot, weapon, eye, direction, range_units);
                self.land(slot, weapon, damage, &[shot]);
            }
            Delivery::Pellets { damage } => {
                self.pellets.extend(
                    weapons::pellets(intent.yaw, intent.pitch)
                        .map(|pellet| Pellet::launch(body, eye, pellet, damage)),
                );
            }
            // Still igniting: the muzzle glows, nothing burns yet.
            Delivery::Flame { .. } if !self.fighters[slot].arsenal.ignited() => {}
            Delivery::Flame { damage } => self.burn(slot, weapon, eye, direction, damage)?,
        }
        Ok(())
    }

    /// Where one hitscan ray from `slot`'s `eye` along the Q16 `direction` ends and what it
    /// strikes: the arena (and the dummies) as they are now, fighters where the shooter saw them.
    fn hitscan(
        &self,
        slot: usize,
        weapon: Weapon,
        eye: [f32; 3],
        direction: [i32; 3],
        range_units: i32,
    ) -> Shot {
        let shooter = &self.fighters[slot];
        let rewind = self
            .tick
            .saturating_sub(shooter.view_tick)
            .min(MAX_REWIND_TICKS);
        let seen = self.centers_at(self.tick - rewind);
        let fighters = seen.iter().enumerate().filter_map(|(other, center)| {
            let alive = self.fighters.get(other).is_some_and(Fighter::alive);
            (other != slot && alive).then_some((other, (*center)?))
        });
        let sight = aim::sight(
            &self.world,
            eye,
            direction,
            range_units,
            shooter.player.body,
            |hit| slot_of(hit).is_none(),
            fighters,
        );
        if let Some((victim, _)) = sight.fighter {
            #[allow(clippy::cast_precision_loss)]
            let unit = direction.map(|axis| axis as f32 / crate::trig::TRIG_ONE as f32);
            return Shot {
                beam: Event::Beam {
                    weapon,
                    from: eye,
                    end: sight.end,
                    normal: unit.map(|axis| -axis),
                    struck: true,
                },
                target: Target::Fighter(victim),
                at: sight.end,
            };
        }
        let scenery = sight.scenery;
        Shot {
            beam: Event::Beam {
                weapon,
                from: eye,
                end: scenery.end,
                normal: scenery.hit.map_or([0.0; 3], |hit| hit.normal),
                struck: scenery.hit.is_some(),
            },
            target: scenery
                .hit
                .and_then(|hit| self.dummy_of(hit.body))
                .map_or(Target::Nothing, Target::Dummy),
            at: scenery.end,
        }
    }

    /// One tick of every pellet's flight, in launch order: a pellet that meets a body ends there
    /// ([`Event::Pellet`]) and hurts it if it is a fighter or a dummy.
    fn fly_pellets(&mut self) {
        let mut index = 0;
        while index < self.pellets.len() {
            match self.pellets[index].fly(&self.world) {
                Travel::Flying => index += 1,
                Travel::Spent => {
                    self.pellets.remove(index);
                }
                Travel::Struck {
                    at,
                    normal,
                    body,
                    damage,
                } => {
                    let pellet = self.pellets.remove(index);
                    self.events.push(Event::Pellet {
                        from: pellet.from_units(),
                        at,
                        normal,
                        launched: pellet.age == 0,
                    });
                    let Some(attacker) = slot_of(pellet.owner).map(usize::from) else {
                        continue;
                    };
                    if let Some(victim) = slot_of(body) {
                        self.hurt(usize::from(victim), attacker, damage, Weapon::Shotgun, at);
                    } else if let Some(dummy) = self.dummy_of(body) {
                        self.hurt_dummy(dummy, attacker, damage, at);
                    }
                }
            }
        }
    }

    /// Shows every shot's beam, then deals `damage` for each one that struck.
    fn land(&mut self, slot: usize, weapon: Weapon, damage: i32, shots: &[Shot]) {
        self.events.extend(shots.iter().map(|shot| shot.beam));
        for shot in shots {
            match shot.target {
                Target::Fighter(victim) => self.hurt(victim, slot, damage, weapon, shot.at),
                Target::Dummy(index) => self.hurt_dummy(index, slot, damage, shot.at),
                Target::Nothing => {}
            }
        }
    }

    /// One tick of `slot`'s flame from `eye` along the Q16 `direction` ([`weapons::flame`]):
    /// `damage` to every fighter and dummy in it as they are now, the owner excepted.
    fn burn(
        &mut self,
        slot: usize,
        weapon: Weapon,
        eye: [f32; 3],
        direction: [i32; 3],
        damage: i32,
    ) -> Result<(), PhysicsError> {
        let owner = self.fighters[slot].player.body;
        let (flame, burning) = weapons::flame(&self.world, eye, direction, owner, |body| {
            slot_of(body).is_some() || self.dummy_of(body).is_some()
        })?;
        self.events.push(Event::Beam {
            weapon,
            from: eye,
            end: flame.end,
            normal: flame.hit.map_or([0.0; 3], |hit| hit.normal),
            struck: flame.hit.is_some(),
        });
        let targets: Vec<(Target, [f32; 3])> = burning
            .into_iter()
            .filter_map(|body| {
                let center = self.world.body(body)?.exact_position();
                let target = match slot_of(body) {
                    Some(victim) => Target::Fighter(usize::from(victim)),
                    None => Target::Dummy(self.dummy_of(body)?),
                };
                Some((target, melee::contact(eye, center)))
            })
            .collect();
        for (target, at) in targets {
            match target {
                Target::Fighter(victim) => self.hurt(victim, slot, damage, weapon, at),
                Target::Dummy(index) => self.hurt_dummy(index, slot, damage, at),
                Target::Nothing => {}
            }
        }
        Ok(())
    }

    /// `slot`'s energy blade swung toward `yaw` from `eye` (units): every fighter (where the
    /// swinger saw it, like hitscan) and dummy it covers ([`melee::covers`]) and sees past the
    /// arena is struck, shoved and hurt; a swing that strikes nobody may still spark on a wall
    /// within reach.
    fn swing(
        &mut self,
        slot: usize,
        yaw: u16,
        eye: [f32; 3],
        direction: [i32; 3],
    ) -> Result<(), PhysicsError> {
        let swinger = &self.fighters[slot];
        let body = swinger.player.body;
        let feet = swinger.player.feet(&self.world);
        let rewind = self
            .tick
            .saturating_sub(swinger.view_tick)
            .min(MAX_REWIND_TICKS);
        let in_sight = |world: &World, center: [f32; 3]| {
            let along = [0, 1, 2].map(|axis| center[axis] - eye[axis]);
            world
                .cast_ray(eye, along, |hit| hit.0 < FIRST_PLAYER_BODY)
                .is_none()
        };
        let covered = |center: [f32; 3]| melee::covers(feet, yaw, center);
        let fighters: Vec<(usize, [f32; 3])> = self
            .centers_at(self.tick - rewind)
            .iter()
            .enumerate()
            .filter(|(other, _)| {
                *other != slot && self.fighters.get(*other).is_some_and(Fighter::alive)
            })
            .filter_map(|(other, center)| Some((other, (*center)?)))
            .filter(|(_, center)| covered(*center) && in_sight(&self.world, *center))
            .collect();
        let dummies: Vec<(usize, [f32; 3])> = (0..self.dummies.len())
            .filter_map(|index| Some((index, self.dummies[index].center(&self.world)?)))
            .filter(|(_, center)| covered(*center) && in_sight(&self.world, *center))
            .collect();
        let strike = |at: [f32; 3]| {
            let normal = [eye[0] - at[0], 0.0, eye[2] - at[2]];
            let length = normal[0].hypot(normal[2]).max(f32::EPSILON);
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
            }
            return Ok(());
        }
        for (victim, center) in fighters {
            let at = melee::contact(eye, center);
            self.events.push(strike(at));
            self.fighters[victim]
                .player
                .push(&mut self.world, melee::knockback(yaw))?;
            self.hurt(victim, slot, melee::DAMAGE, Weapon::EnergyBlade, at);
        }
        for (index, center) in dummies {
            let at = melee::contact(eye, center);
            self.events.push(strike(at));
            self.hurt_dummy(index, slot, melee::DAMAGE, at);
        }
        Ok(())
    }

    /// A blast at `at` from `owner`'s rocket: splash damage and knockback to every standing
    /// dummy and fighter in reach. The owner takes [`SELF_SPLASH_PERCENT`] of its share (and the
    /// full push: rocket jumps).
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
        for slot in 0..self.fighters.len() {
            if !self.fighters[slot].alive() {
                continue;
            }
            let player = self.fighters[slot].player;
            let center = player.center(&self.world);
            let Some((damage, push)) = weapons::splash(at, center) else {
                continue;
            };
            player.push(&mut self.world, push)?;
            let damage = if slot == attacker {
                damage * SELF_SPLASH_PERCENT / 100
            } else {
                damage
            };
            self.hurt(slot, attacker, damage, Weapon::RocketLauncher, center);
        }
        Ok(())
    }
}
