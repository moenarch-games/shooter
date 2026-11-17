//! The energy weapons: a hitscan energy rifle, a high-damage rail gun, a rocket launcher whose
//! rockets fly as projectiles and explode with splash damage and knockback (rocket jumping), a
//! shotgun whose blast is a fixed pattern of fast pellets that slow down (and hurt less) as they
//! fly, a continuous laser that burns the first target in its beam every tick the trigger is held,
//! and a flamethrower whose flame, after a short ignition, is a volume cut short by walls; the
//! energy blade, a quick melee swing on its own key whatever gun is held ([`crate::melee`]); and
//! the grapple hook, a mounted module on its own key that is never in the rotation
//! ([`crate::grapple`], Atlas D2.i104.2).
//!
//! Everything a shot decides is integer: the aim is a Q16 direction from the yaw and pitch
//! steps (the shotgun's pellets add fixed yaw and pitch offsets, [`PELLETS`]), rockets fly in
//! rockets and pellets fly in subunits per tick (a pellet's drag and damage falloff are integer
//! fractions of its speed), splash distances are integer square roots. Rays and rocket and pellet
//! sweeps go through the physics world's `cast_ray` (the arena and the dummies) and the robots'
//! part hitboxes ([`crate::damage`]), and the flame through its `overlap_box`, so physics-engine
//! and the Rapier oracle answer them alike.

use crate::fixed::subunits_per_tick;
use crate::physics::{BodyId, PhysicsError, RayHit, SUBUNITS_PER_UNIT, World};
use crate::robot::{MAX_PARTS, PartId};
use crate::trig;

/// The guns a player carries.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Weapon {
    /// Fast hitscan bolts, low damage each.
    #[default]
    EnergyRifle,
    /// One slow, heavy hitscan beam.
    RailGun,
    /// A slow rocket that explodes on contact: splash damage and knockback.
    RocketLauncher,
    /// The grapple module (Atlas D2.i104.2): a movement tool on its own key, never held or in
    /// the rotation. It hooks arena geometry and reels the robot in while its key is held
    /// ([`crate::grapple`]).
    Grapple,
    /// The quick-melee energy blade: never held, swung on its own key ([`crate::melee`]).
    EnergyBlade,
    /// A blast of [`PELLETS`] [`Pellet`]s in a fixed spread.
    Shotgun,
    /// A continuous beam: damage every tick the trigger is held.
    Laser,
    /// A short flame that, after [`FLAME_IGNITION_TICKS`], burns every fighter and dummy in its
    /// volume each tick it is held ([`flame`]).
    Flamethrower,
}

/// How a weapon's shot reaches its target.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Delivery {
    /// An instant ray: `damage` to the first target within `range_units`.
    Hitscan { damage: i32, range_units: i32 },
    /// A [`Rocket`].
    Rocket,
    /// A [`crate::grapple::Hook`].
    Grapple,
    /// A [`crate::melee`] swing.
    Melee,
    /// [`PELLETS`] [`Pellet`]s, each `damage` at the muzzle's speed.
    Pellets { damage: i32 },
    /// A flame ([`flame`]): `damage` to everything in it.
    Flame { damage: i32 },
}

impl Weapon {
    /// Every weapon, in [`Weapon::index`] order.
    pub const ALL: [Self; 8] = [
        Self::EnergyRifle,
        Self::RailGun,
        Self::RocketLauncher,
        Self::Grapple,
        Self::EnergyBlade,
        Self::Shotgun,
        Self::Laser,
        Self::Flamethrower,
    ];
    /// The weapons a player can hold (the 1 to 6 keys); the blade is swung and the grapple
    /// module hooked on their own keys, never held.
    pub const ROTATION: [Self; 6] = [
        Self::EnergyRifle,
        Self::RailGun,
        Self::RocketLauncher,
        Self::Shotgun,
        Self::Laser,
        Self::Flamethrower,
    ];

    /// Its place in [`Weapon::ALL`] (the wire).
    #[must_use]
    pub const fn index(self) -> u8 {
        match self {
            Self::EnergyRifle => 0,
            Self::RailGun => 1,
            Self::RocketLauncher => 2,
            Self::Grapple => 3,
            Self::EnergyBlade => 4,
            Self::Shotgun => 5,
            Self::Laser => 6,
            Self::Flamethrower => 7,
        }
    }

    /// The weapon at `index` in [`Weapon::ALL`].
    #[must_use]
    pub const fn from_index(index: u8) -> Option<Self> {
        match index {
            0 => Some(Self::EnergyRifle),
            1 => Some(Self::RailGun),
            2 => Some(Self::RocketLauncher),
            3 => Some(Self::Grapple),
            4 => Some(Self::EnergyBlade),
            5 => Some(Self::Shotgun),
            6 => Some(Self::Laser),
            7 => Some(Self::Flamethrower),
            _ => None,
        }
    }

    /// Ticks between two shots.
    #[must_use]
    pub const fn refire_ticks(self) -> u32 {
        match self {
            // 10 shots a second.
            Self::EnergyRifle => 6,
            // 1.2 s.
            Self::RailGun => 72,
            // 0.8 s.
            Self::RocketLauncher => 48,
            // 0.75 s from one hook to the next.
            Self::Grapple => 45,
            Self::EnergyBlade => crate::melee::SWING_TICKS,
            // 0.9 s.
            Self::Shotgun => 54,
            // Continuous: every tick the trigger is held.
            Self::Laser | Self::Flamethrower => 1,
        }
    }

    /// What one activation costs from the robot's store of its [`Weapon::resource`], in
    /// milli-units ([`crate::energy`]): a shot, a swing, a hook, or one tick of a continuous
    /// weapon. Provisional (Atlas Q2.i123.2). With a generator giving 20 electricity a second
    /// the rifle (20 a second), rockets (25), rail gun (25) and shotgun (13) can almost or fully
    /// keep firing while its fuel lasts; the laser (36) drains the store. The flamethrower burns
    /// 15 fuel a second from the tanks.
    #[must_use]
    pub const fn energy(self) -> u32 {
        match self {
            Self::EnergyRifle => 2_000,
            Self::RailGun => 30_000,
            Self::RocketLauncher => 20_000,
            Self::Grapple => 10_000,
            Self::EnergyBlade => 5_000,
            Self::Shotgun => 12_000,
            Self::Laser => 600,
            Self::Flamethrower => 250,
        }
    }

    /// The resource it runs on (D2.i123.3, shooter#128): the flamethrower burns fuel, every other
    /// weapon and the grapple module are electric.
    #[must_use]
    pub const fn resource(self) -> crate::energy::Resource {
        match self {
            Self::Flamethrower => crate::energy::Resource::Fuel,
            _ => crate::energy::Resource::Electricity,
        }
    }

    #[must_use]
    pub const fn delivery(self) -> Delivery {
        match self {
            Self::EnergyRifle => Delivery::Hitscan {
                damage: 12,
                range_units: 8_000,
            },
            Self::RailGun => Delivery::Hitscan {
                damage: 85,
                range_units: 10_000,
            },
            Self::RocketLauncher => Delivery::Rocket,
            Self::Grapple => Delivery::Grapple,
            Self::EnergyBlade => Delivery::Melee,
            Self::Shotgun => Delivery::Pellets { damage: 9 },
            Self::Laser => Delivery::Hitscan {
                damage: 2,
                range_units: 3_000,
            },
            Self::Flamethrower => Delivery::Flame { damage: 3 },
        }
    }

    /// Whether it fires every tick the trigger is held (a beam or a flame, not single shots).
    #[must_use]
    pub const fn continuous(self) -> bool {
        matches!(self, Self::Laser | Self::Flamethrower)
    }

    /// Its family's shared salvo limit on one robot (shooter#127, Atlas D2.i123.5), if it has
    /// one: four rocket launchers do not launch four rockets at once. Every other gun fires
    /// alongside its copies, bound only by its own refire and energy. Provisional (Atlas
    /// Q2.i123.2).
    #[must_use]
    pub const fn salvo(self) -> Option<Salvo> {
        match self {
            Self::RocketLauncher => Some(Salvo {
                per_tick: 1,
                spacing: 6,
            }),
            _ => None,
        }
    }

    /// Whether it may be bound to fire directly, alone or in a group (shooter#127): the guns.
    /// The blade and the grapple module act on their own keys.
    #[must_use]
    pub fn gun(self) -> bool {
        Self::ROTATION.contains(&self)
    }

    /// The share of a strike's damage (percent) that passes an armor plate to the part behind
    /// it ([`crate::damage`]): the hitscan beams pierce, the rail gun most; pellets, rockets,
    /// the flame and the blade do not. Provisional (Atlas Q2.i70.1).
    #[must_use]
    pub const fn penetration(self) -> u8 {
        match self {
            Self::RailGun => 50,
            Self::EnergyRifle | Self::Laser => 20,
            _ => 0,
        }
    }
}

/// The shotgun's pellets: (yaw, pitch) offsets in `trig` angle steps (182 a degree) from the
/// aim. One pellet on the aim, three on a 2° ring and six on a 4.5° ring: a fixed pattern, so a
/// blast needs no randomness.
pub const PELLETS: [(i16, i16); 10] = [
    (0, 0),
    (0, 364),
    (315, -182),
    (-315, -182),
    (819, 0),
    (410, 709),
    (-410, 709),
    (-819, 0),
    (-410, -709),
    (410, -709),
];

/// Pellets leave the muzzle at 240 m/s (4 m a tick)...
pub const PELLET_SPEED_UNITS_PER_SECOND: i32 = 24_000;
pub const PELLET_SPEED_SUBUNITS_PER_TICK: i32 = subunits_per_tick(PELLET_SPEED_UNITS_PER_SECOND);
/// ...lose a sixteenth of their speed every tick (drag)...
pub const PELLET_DRAG_DIVISOR: i32 = 16;
/// ...and are spent once below 150 m/s...
pub const PELLET_MIN_SPEED_SUBUNITS_PER_TICK: i32 = subunits_per_tick(15_000);
/// ...or 25 m out (with this drag, both come about 8 ticks after the blast).
pub const PELLET_RANGE_UNITS: i32 = 2_500;

/// The flamethrower deals no damage for its first 15 ticks (0.25 s) of a continuous burst: it
/// ignites. Letting go of the trigger (or switching away) puts it out again.
pub const FLAME_IGNITION_TICKS: u32 = 15;
/// The flame reaches 6 m from the eye...
pub const FLAME_REACH_UNITS: i32 = 600;
/// ...and is 1.2 m across: everything within 60 units of its axis burns.
pub const FLAME_RADIUS_UNITS: i32 = 60;

/// A weapon family's shared salvo limit on one robot (shooter#127): at most `per_tick` of its
/// copies fire in one tick, and after a tick in which any did, the family waits `spacing` ticks
/// before the next. Each copy keeps its own refire on top. The numbers are provisional (Atlas
/// Q2.i123.2): rockets launch one a tick, 0.1 s apart, so a pod of four staggers over 0.3 s.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Salvo {
    pub per_tick: u8,
    pub spacing: u32,
}

/// Rocket speed: 24 m/s.
pub const ROCKET_SPEED_UNITS_PER_SECOND: i32 = 2_400;
pub const ROCKET_SPEED_SUBUNITS_PER_TICK: i32 = subunits_per_tick(ROCKET_SPEED_UNITS_PER_SECOND);
/// A rocket that has met nothing for 4 s fizzles out.
pub const ROCKET_LIFETIME_TICKS: u32 = 240;
/// Splash reaches 3 m from the blast, falling off linearly with the distance to a body's centre.
pub const SPLASH_RADIUS_UNITS: i32 = 300;
/// Splash damage at the blast's centre.
pub const SPLASH_DAMAGE: i32 = 100;
/// Knockback at the blast's centre: 15 m/s. A rocket at a standing player's feet (0.9 m from
/// its centre) throws it up at 10.5 m/s, about 3 m.
pub const SPLASH_KNOCKBACK_UNITS_PER_SECOND: i32 = 1_500;
pub const SPLASH_KNOCKBACK_SUBUNITS_PER_TICK: i32 =
    subunits_per_tick(SPLASH_KNOCKBACK_UNITS_PER_SECOND);
/// Pitch steps (`crate::trig` angle units) of straight up; straight down is the negative.
pub const MAX_PITCH: i16 = trig::YAW_QUARTER_TURN as i16;

/// One mounted weapon (shooter#125): the part that is it in its robot's build (a stable handle
/// within the build) and the weapon it is. Copies of one weapon on one robot are distinct
/// instances, each with its own mount, mod, health, cooldown and muzzle.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Instance {
    pub part: PartId,
    pub weapon: Weapon,
}

impl Instance {
    #[must_use]
    pub const fn new(part: u8, weapon: Weapon) -> Self {
        Self {
            part: PartId(part),
            weapon,
        }
    }
}

/// A weapon switch request: a weapon (the copy held if it is one, else its first holdable
/// copy, [`resolve`]) or one exact mounted copy.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Select {
    Weapon(Weapon),
    Part(PartId),
}

impl From<Weapon> for Select {
    fn from(weapon: Weapon) -> Self {
        Self::Weapon(weapon)
    }
}

impl From<Instance> for Select {
    fn from(instance: Instance) -> Self {
        Self::Part(instance.part)
    }
}

/// Fire requests for exact mounted copies, by part (shooter#127): what a person's direct weapon
/// bindings and weapon groups resolved this tick ([`crate::bindings`]), or what a bot asks. Each
/// part at most once, in ascending part ID order (the order the host grants them in), at most
/// [`MAX_PARTS`]. The host fires only the guns the robot still has; any other part is ignored.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Triggers {
    len: u8,
    parts: [PartId; MAX_PARTS],
}

impl Default for Triggers {
    fn default() -> Self {
        Self::NONE
    }
}

impl Triggers {
    /// No copy asked to fire.
    pub const NONE: Self = Self {
        len: 0,
        parts: [PartId(0); MAX_PARTS],
    };

    /// These requests with `part` asked to fire too; false (and unchanged) if it already is
    /// or there is no room.
    pub fn press(&mut self, part: PartId) -> bool {
        let len = usize::from(self.len);
        if len >= MAX_PARTS {
            return false;
        }
        match self.parts[..len].binary_search(&part) {
            Ok(_) => false,
            Err(at) => {
                self.parts.copy_within(at..len, at + 1);
                self.parts[at] = part;
                self.len += 1;
                true
            }
        }
    }

    /// The parts asked to fire, ascending.
    #[must_use]
    pub fn parts(&self) -> &[PartId] {
        &self.parts[..usize::from(self.len)]
    }

    #[must_use]
    pub fn contains(&self, part: PartId) -> bool {
        self.parts().binary_search(&part).is_ok()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl FromIterator<PartId> for Triggers {
    fn from_iter<I: IntoIterator<Item = PartId>>(parts: I) -> Self {
        let mut triggers = Self::NONE;
        for part in parts {
            triggers.press(part);
        }
        triggers
    }
}

/// The copies that fired in one tick, in the order they were granted (the hook, the swing, then
/// the guns by part ID): at most one entry per mounted part.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Volley {
    len: u8,
    shots: [Instance; MAX_PARTS],
}

impl Default for Volley {
    fn default() -> Self {
        Self {
            len: 0,
            shots: [Instance::new(0, Weapon::EnergyRifle); MAX_PARTS],
        }
    }
}

impl Volley {
    /// This volley with `instance` too, if there is room.
    pub fn push(&mut self, instance: Instance) {
        if let Some(slot) = self.shots.get_mut(usize::from(self.len)) {
            *slot = instance;
            self.len += 1;
        }
    }

    /// The copies that fired, in order.
    #[must_use]
    pub fn shots(&self) -> &[Instance] {
        &self.shots[..usize::from(self.len)]
    }

    /// The first copy that fired, if any.
    #[must_use]
    pub fn first(&self) -> Option<Instance> {
        self.shots().first().copied()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// The weapon copy a player holds (what the primary trigger fires, a convenience: shooter#127)
/// and when each mounted gun, its salvo-limited families, the blade and the grapple module may
/// fire next: part of the simulation state.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Arsenal {
    pub held: Instance,
    /// A swing keeps every gun down until then.
    guns_ready_at: u32,
    /// When each mounted gun may fire again after its own last shot, by part (an entry with no
    /// shot is ready at once, [`Arsenal::refire_at`]).
    refire: [(PartId, u32); MAX_PARTS],
    /// When each weapon family with a salvo limit ([`Weapon::salvo`]) may fire again, by
    /// [`Weapon::index`].
    family_ready_at: [u32; Weapon::ALL.len()],
    blade_ready_at: u32,
    /// When the grapple module may hook again ([`Arsenal::hook`]).
    grapple_ready_at: u32,
    /// Ticks in a row a flamethrower has fired.
    flaming: u32,
}

/// The blade [`Arsenal::trigger`] swings: one on part 8, as the named loadouts mount it.
const TEST_BLADE: Instance = Instance::new(8, Weapon::EnergyBlade);

impl Arsenal {
    /// Applies a switch request, the melee key and the primary trigger (the held copy) at
    /// `tick`: what fired, with every rotation weapon to hold (the blade is never held) and the
    /// blade on part 8. A swing goes first and keeps the guns down for
    /// [`crate::melee::GUN_LOCKOUT_TICKS`].
    pub fn trigger(
        &mut self,
        select: Option<Instance>,
        melee: bool,
        fire: bool,
        tick: u32,
    ) -> Option<Instance> {
        let select = select.filter(|held| held.weapon.gun());
        if let Some(held) = select {
            self.held = held;
        }
        let requests = if fire { vec![self.held] } else { Vec::new() };
        self.trigger_tuned(
            None,
            melee.then_some(TEST_BLADE),
            &requests,
            tick,
            |held| held.weapon.refire_ticks(),
            |_| true,
        )
        .first()
    }

    /// The tick's switch (`select`: a copy to hold, the caller has checked it may), swing and
    /// gun requests (shooter#127): the blade `blade` swings if the melee key is down and the
    /// robot still has one, then every copy in `requests` (any order, repeats count once) fires
    /// if it is ready, its family's salvo limit allows ([`Weapon::salvo`]) and `spend` grants
    /// its energy ([`crate::energy`]). The requests are granted first come, first served in
    /// ascending part ID order: the swing first, then the guns by part. A refused activation
    /// does not happen and spends no cooldown, and refuses only itself: each copy keeps its own
    /// refire from `refire` (its mod's, [`crate::robot::Tuning::refire_ticks`]), so two rail guns
    /// fire together and a rifle fires beside a rocket launcher. Holding a copy is a convenience
    /// for the primary trigger: a switch delays nothing.
    pub fn trigger_tuned(
        &mut self,
        select: Option<Instance>,
        blade: Option<Instance>,
        requests: &[Instance],
        tick: u32,
        refire: impl Fn(Instance) -> u32,
        mut spend: impl FnMut(Instance) -> bool,
    ) -> Volley {
        // Which weapons this fighter may hold is the caller's to check ([`holdable`]); the blade
        // is held only when it is the last weapon.
        if let Some(instance) = select {
            self.held = instance;
        }
        let mut volley = Volley::default();
        if let Some(blade) = blade.filter(|blade| tick >= self.blade_ready_at && spend(*blade)) {
            self.blade_ready_at = tick + refire(blade);
            self.guns_ready_at = self
                .guns_ready_at
                .max(tick + crate::melee::GUN_LOCKOUT_TICKS);
            volley.push(blade);
        }
        let mut requests = requests.to_vec();
        requests.sort_by_key(|instance| instance.part);
        requests.dedup_by_key(|instance| instance.part);
        let mut launched = [0u8; Weapon::ALL.len()];
        for instance in requests {
            let family = usize::from(instance.weapon.index());
            let salvo = instance.weapon.salvo();
            let family_open = salvo.is_none_or(|salvo| {
                tick >= self.family_ready_at[family] && launched[family] < salvo.per_tick
            });
            if tick >= self.guns_ready_at
                && tick >= self.refire_at(instance.part)
                && family_open
                && spend(instance)
            {
                self.set_refire(instance.part, tick + refire(instance));
                launched[family] += 1;
                volley.push(instance);
            }
        }
        for weapon in Weapon::ALL {
            let family = usize::from(weapon.index());
            if let Some(salvo) = weapon.salvo()
                && launched[family] > 0
            {
                self.family_ready_at[family] = tick + salvo.spacing;
            }
        }
        self.flaming = if volley
            .shots()
            .iter()
            .any(|fired| fired.weapon == Weapon::Flamethrower)
        {
            self.flaming.saturating_add(1)
        } else {
            0
        };
        volley
    }

    /// When the gun on `part` may fire again after its own last shot (0: it has not fired).
    #[must_use]
    pub fn refire_at(&self, part: PartId) -> u32 {
        self.refire
            .iter()
            .find(|(at, _)| *at == part)
            .map_or(0, |(_, ready)| *ready)
    }

    /// Records the gun on `part` ready again at `ready`, in its own entry or the one that
    /// constrains least (a build has fewer weapon parts than entries, so no live entry goes).
    fn set_refire(&mut self, part: PartId, ready: u32) {
        let entry = self
            .refire
            .iter()
            .position(|(at, _)| *at == part)
            .or_else(|| (0..self.refire.len()).min_by_key(|index| self.refire[*index].1))
            .unwrap_or(0);
        self.refire[entry] = (part, ready);
    }

    /// The grapple module's key at `tick` (Atlas D2.i104.2): it hooks when ready and `spend`
    /// pays its activation from the shared pool ([`crate::energy`]), then waits `refire` ticks.
    /// Independent of the guns and the blade; a refused activation spends no cooldown.
    pub fn hook(&mut self, tick: u32, refire: u32, spend: impl FnOnce() -> bool) -> bool {
        if tick >= self.grapple_ready_at && spend() {
            self.grapple_ready_at = tick + refire;
            true
        } else {
            false
        }
    }

    /// Ticks from `tick` until the grapple module may hook again: 0 when it is ready.
    #[must_use]
    pub const fn grapple_cooldown(&self, tick: u32) -> u32 {
        self.grapple_ready_at.saturating_sub(tick)
    }

    /// Ticks from `tick` until the blade may swing again: 0 when it is ready.
    #[must_use]
    pub const fn blade_cooldown(&self, tick: u32) -> u32 {
        self.blade_ready_at.saturating_sub(tick)
    }

    /// An arsenal holding `held`, ready at once (a fighter's at spawn).
    #[must_use]
    pub fn holding(held: Instance) -> Self {
        Self {
            held,
            ..Self::default()
        }
    }

    /// Ticks from `tick` until the held copy may fire again ([`Arsenal::cooldown_of`]): 0 when
    /// it is ready.
    #[must_use]
    pub fn cooldown(&self, tick: u32) -> u32 {
        self.cooldown_of(self.held, tick)
    }

    /// Ticks from `tick` until the copy `instance` may fire again: its own refire, its family's
    /// salvo spacing ([`Weapon::salvo`]) and a swing's lockout. 0 when it is ready (energy
    /// permitting).
    #[must_use]
    pub fn cooldown_of(&self, instance: Instance, tick: u32) -> u32 {
        let family = instance.weapon.salvo().map_or(0, |_| {
            self.family_ready_at[usize::from(instance.weapon.index())]
        });
        self.guns_ready_at
            .max(self.refire_at(instance.part))
            .max(family)
            .saturating_sub(tick)
    }

    /// Ticks from `tick` until the gun on `part` may fire again after its own last shot: 0 when
    /// it is ready.
    #[must_use]
    pub fn refire_left(&self, part: PartId, tick: u32) -> u32 {
        self.refire_at(part).saturating_sub(tick)
    }

    /// Whether the flame burns this tick: a flamethrower has fired for more than
    /// [`FLAME_IGNITION_TICKS`] ticks in a row. Before that it is still igniting.
    #[must_use]
    pub const fn ignited(&self) -> bool {
        self.flaming > FLAME_IGNITION_TICKS
    }
}

/// Its place in the cycle: guns in [`Weapon::ROTATION`] order, copies of one gun by part; the
/// blade (never in the rotation) first.
fn place(instance: Instance) -> (i32, PartId) {
    let rotation = Weapon::ROTATION
        .iter()
        .position(|weapon| *weapon == instance.weapon)
        .map_or(-1, |index| i32::try_from(index).unwrap_or(0));
    (rotation, instance.part)
}

/// The weapons a fighter may hold among `armed` (the instances its build mounts and still has,
/// [`crate::damage::PartDamage::instances`]): its guns in [`Weapon::ROTATION`] order, copies of
/// one gun by part. The blade is swung on its own key and is not among them, unless it is the
/// only weapon left: then its first copy is held and the trigger swings it.
#[must_use]
pub fn holdable(armed: &[Instance]) -> Vec<Instance> {
    let mut guns: Vec<Instance> = armed
        .iter()
        .copied()
        .filter(|instance| Weapon::ROTATION.contains(&instance.weapon))
        .collect();
    if guns.is_empty() {
        return armed
            .iter()
            .copied()
            .filter(|instance| instance.weapon == Weapon::EnergyBlade)
            .min_by_key(|instance| instance.part)
            .into_iter()
            .collect();
    }
    guns.sort_by_key(|instance| place(*instance));
    guns
}

/// The weapon a fighter takes up at spawn: the first [`holdable`] one, else the blade. `None`
/// if nothing is holdable (the grapple module never is).
#[must_use]
pub fn spawn_weapon(armed: &[Instance]) -> Option<Instance> {
    holdable(armed).first().copied()
}

/// The instance `select` asks for among `armed`, if it may be held: an exact copy, or for a
/// weapon the copy `held` if it is one (and still holdable), else that weapon's first holdable
/// copy. `None`: the switch is refused.
#[must_use]
pub fn resolve(select: Select, held: Instance, armed: &[Instance]) -> Option<Instance> {
    let holdable = holdable(armed);
    match select {
        Select::Part(part) => holdable.into_iter().find(|instance| instance.part == part),
        Select::Weapon(weapon) if held.weapon == weapon && holdable.contains(&held) => Some(held),
        Select::Weapon(weapon) => holdable
            .into_iter()
            .find(|instance| instance.weapon == weapon),
    }
}

/// The instance `steps` places on from `from` among the [`holdable`] ones (next for positive
/// steps, previous for negative), wrapping round: every copy of a gun is its own place. A `from`
/// no longer holdable (lost, say) counts from where it stands in the cycle, so one step goes to
/// its nearest holdable neighbour that way. `None` if nothing is holdable or `steps` is 0.
#[must_use]
pub fn cycled(from: Instance, steps: i32, armed: &[Instance]) -> Option<Instance> {
    let cycle = holdable(armed);
    let len = i32::try_from(cycle.len()).ok().filter(|len| *len > 0)?;
    if steps == 0 {
        return None;
    }
    let start = if let Some(index) = cycle.iter().position(|instance| *instance == from) {
        i32::try_from(index).unwrap_or(0)
    } else {
        // Between two holdable weapons: the first step lands on the neighbour that way.
        let after = cycle
            .iter()
            .position(|instance| place(*instance) > place(from))
            .map_or(len, |index| i32::try_from(index).unwrap_or(0));
        if steps > 0 { after - 1 } else { after }
    };
    let index = usize::try_from((start + steps).rem_euclid(len)).unwrap_or(0);
    cycle.get(index).copied()
}

/// The Q16 directions of a shotgun blast's [`PELLETS`] aimed at `yaw` and `pitch`.
#[must_use]
pub fn pellets(yaw: u16, pitch: i16) -> [[i32; 3]; PELLETS.len()] {
    PELLETS.map(|(turn, rise)| {
        #[allow(clippy::cast_sign_loss)]
        let yaw = yaw.wrapping_add(turn as u16);
        aim(yaw, pitch.saturating_add(rise))
    })
}

/// The Q16 unit aim direction `(x, y, z)` of a yaw and a pitch (up positive).
#[must_use]
pub fn aim(yaw: u16, pitch: i16) -> [i32; 3] {
    // A negative pitch wraps to the same angle below the horizon.
    #[allow(clippy::cast_sign_loss)]
    let pitch = pitch.clamp(-MAX_PITCH, MAX_PITCH) as u16;
    let (sine, cosine) = (trig::sin(pitch), trig::cos(pitch));
    let (x, z) = trig::direction(yaw);
    let scale = |unit| trig::checked_scale(cosine, unit).unwrap_or(0);
    [scale(x), sine, scale(z)]
}

#[allow(clippy::cast_precision_loss)]
fn unit_f32(direction: [i32; 3]) -> [f32; 3] {
    direction.map(|axis| axis as f32 / trig::TRIG_ONE as f32)
}

fn along(origin: [f32; 3], translation: [f32; 3], fraction: f32) -> [f32; 3] {
    [0, 1, 2].map(|axis| origin[axis] + translation[axis] * fraction)
}

/// What a hitscan shot met.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trace {
    /// Where the ray ended: the hit point, or the end of its range.
    pub end: [f32; 3],
    pub hit: Option<RayHit>,
}

/// A hitscan ray from `eye` (units) along the Q16 `direction` for `range_units`, ignoring
/// `owner` and every body `accept` refuses.
#[must_use]
pub fn trace(
    world: &World,
    eye: [f32; 3],
    direction: [i32; 3],
    range_units: i32,
    owner: BodyId,
    accept: impl Fn(BodyId) -> bool,
) -> Trace {
    #[allow(clippy::cast_precision_loss)]
    let translation = unit_f32(direction).map(|axis| axis * range_units as f32);
    let hit = world.cast_ray(eye, translation, |body| body != owner && accept(body));
    Trace {
        end: along(eye, translation, hit.map_or(1.0, |hit| hit.fraction)),
        hit,
    }
}

/// A flame from `eye` (units) along the Q16 `direction`, from `owner`: where it ends (the
/// first body `burns` refuses, the arena's geometry, cuts it short within [`FLAME_REACH_UNITS`])
/// and every body `burns` accepts in its volume, in ID order. The volume is the physics world's
/// `overlap_box` round the flame, narrowed to bodies whose box comes within
/// [`FLAME_RADIUS_UNITS`] of the flame's axis.
pub fn flame(
    world: &World,
    eye: [f32; 3],
    direction: [i32; 3],
    owner: BodyId,
    burns: impl Fn(BodyId) -> bool,
) -> Result<(Trace, Vec<BodyId>), PhysicsError> {
    let wall = trace(world, eye, direction, FLAME_REACH_UNITS, owner, |body| {
        !burns(body)
    });
    let axis = [0, 1, 2].map(|axis| wall.end[axis] - eye[axis]);
    #[allow(clippy::cast_precision_loss)]
    let radius = FLAME_RADIUS_UNITS as f32;
    let centre = [0, 1, 2].map(|index| eye[index] + axis[index] * 0.5);
    let half = axis.map(|length| length.abs() * 0.5 + radius);
    let burning = world
        .overlap_box(centre, half)?
        .into_iter()
        .filter(|body| *body != owner && burns(*body))
        .filter(|body| {
            world.body(*body).is_some_and(|rigid| {
                let extents = rigid.half_extents();
                #[allow(clippy::cast_precision_loss)]
                let inflated = [extents.x, extents.y, extents.z].map(|e| e as f32 + radius);
                ray_box(eye, axis, rigid.exact_position(), inflated).is_some()
            })
        })
        .collect();
    Ok((wall, burning))
}

/// Where along `translation` (0..=1) a ray from `origin` enters the box at `center` with
/// `half` extents, if it does (0 when it starts inside). A slab test: fighters are boxes to
/// lag-compensated shots, wherever they stood at the rewound tick.
#[must_use]
pub fn ray_box(
    origin: [f32; 3],
    translation: [f32; 3],
    center: [f32; 3],
    half: [f32; 3],
) -> Option<f32> {
    let (mut enter, mut exit) = (0.0f32, 1.0f32);
    for axis in 0..3 {
        let (low, high) = (center[axis] - half[axis], center[axis] + half[axis]);
        let (start, step) = (origin[axis], translation[axis]);
        if step == 0.0 {
            if start < low || start > high {
                return None;
            }
            continue;
        }
        let (a, b) = ((low - start) / step, (high - start) / step);
        enter = enter.max(a.min(b));
        exit = exit.min(a.max(b));
        if enter > exit {
            return None;
        }
    }
    Some(enter)
}

/// A rocket in flight, in subunits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rocket {
    pub owner: BodyId,
    pub position: [i32; 3],
    /// Subunits per tick.
    pub velocity: [i32; 3],
    pub age: u32,
}

/// Where a rocket went this tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Flight {
    Flying,
    /// It met a body: the blast point (units), pulled back off the surface by a unit.
    Exploded {
        at: [f32; 3],
        body: BodyId,
    },
    Fizzled,
}

#[allow(clippy::cast_precision_loss)]
fn units(subunits: [i32; 3]) -> [f32; 3] {
    subunits.map(|axis| axis as f32 / SUBUNITS_PER_UNIT as f32)
}

/// Units to subunits, rounded.
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn subunits(units: [f32; 3]) -> [i32; 3] {
    #[allow(clippy::cast_precision_loss)]
    units.map(|axis| (axis * SUBUNITS_PER_UNIT as f32).round() as i32)
}

impl Rocket {
    /// A rocket leaving `eye` (units) along the Q16 `direction`.
    #[must_use]
    pub fn launch(owner: BodyId, eye: [f32; 3], direction: [i32; 3]) -> Self {
        Self {
            owner,
            position: subunits(eye),
            velocity: direction
                .map(|unit| trig::checked_scale(ROCKET_SPEED_SUBUNITS_PER_TICK, unit).unwrap_or(0)),
            age: 0,
        }
    }

    /// The rocket's position in units.
    #[must_use]
    pub fn position_units(&self) -> [f32; 3] {
        units(self.position)
    }

    /// owner that it meets: in a match, the scenery or a robot's part hitbox,
    /// owner that it meets: the scenery and, the session's, the robots' part hitboxes,
    /// [`crate::damage`]).
    pub fn fly(&mut self, cast: impl Fn([f32; 3], [f32; 3]) -> Option<RayHit>) -> Flight {
        let origin = self.position_units();
        let translation = units(self.velocity);
        if let Some(hit) = cast(origin, translation) {
            let point = along(origin, translation, hit.fraction);
            let at = [0, 1, 2].map(|axis| point[axis] + hit.normal[axis]);
            return Flight::Exploded { at, body: hit.body };
        }
        self.position = [0, 1, 2].map(|axis| self.position[axis] + self.velocity[axis]);
        self.age += 1;
        if self.age >= ROCKET_LIFETIME_TICKS {
            Flight::Fizzled
        } else {
            Flight::Flying
        }
    }
}

/// A shotgun pellet in flight, in subunits: a fast projectile that slows down with drag and
/// hurts less the slower it goes ([`Pellet::damage`]).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Pellet {
    pub owner: BodyId,
    pub position: [i32; 3],
    /// Q16 unit direction.
    pub direction: [i32; 3],
    /// Subunits per tick.
    pub speed: i32,
    /// Subunits flown so far.
    pub travelled: i32,
    /// Its full damage, at the muzzle's speed.
    pub damage: i32,
    /// Where this tick's flight began (subunits): the eye on the tick it is fired.
    pub from: [i32; 3],
    /// Ticks flown.
    pub age: u32,
}

/// Where a pellet went this tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Travel {
    Flying,
    /// It met a body at `at` (units) on this tick's stretch from `origin` along `translation`,
    /// and deals `damage` there.
    Struck {
        at: [f32; 3],
        normal: [f32; 3],
        body: BodyId,
        damage: i32,
        origin: [f32; 3],
        translation: [f32; 3],
    },
    /// Out of range or too slow: gone.
    Spent,
}

impl Pellet {
    /// A pellet of `damage` leaving `eye` (units) along the Q16 `direction`.
    #[must_use]
    pub fn launch(owner: BodyId, eye: [f32; 3], direction: [i32; 3], damage: i32) -> Self {
        let position = subunits(eye);
        Self {
            owner,
            position,
            direction,
            speed: PELLET_SPEED_SUBUNITS_PER_TICK,
            travelled: 0,
            damage,
            from: position,
            age: 0,
        }
    }

    #[must_use]
    pub fn position_units(&self) -> [f32; 3] {
        units(self.position)
    }

    #[must_use]
    pub fn from_units(&self) -> [f32; 3] {
        units(self.from)
    }

    /// What it deals at its current speed: its full damage scaled by the speed's share of the
    /// muzzle's, rounded up (so a point-blank pellet deals all of it).
    #[must_use]
    pub fn damage(&self) -> i32 {
        let scaled = i64::from(self.damage) * i64::from(self.speed);
        let muzzle = i64::from(PELLET_SPEED_SUBUNITS_PER_TICK);
        i32::try_from((scaled + muzzle - 1) / muzzle).unwrap_or(0)
    }

    /// One tick of flight: sweeps this tick's path (cut at [`PELLET_RANGE_UNITS`]) with `cast`
    /// (the first body other than the owner that it meets, like [`Rocket::fly`]), then slows
    /// down by drag.
    pub fn fly(&mut self, cast: impl Fn([f32; 3], [f32; 3]) -> Option<RayHit>) -> Travel {
        let range = PELLET_RANGE_UNITS * SUBUNITS_PER_UNIT;
        let step = self.speed.min(range - self.travelled).max(0);
        let velocity = self
            .direction
            .map(|unit| trig::checked_scale(step, unit).unwrap_or(0));
        let origin = self.position_units();
        let translation = units(velocity);
        self.from = self.position;
        if let Some(hit) = cast(origin, translation) {
            return Travel::Struck {
                at: along(origin, translation, hit.fraction),
                normal: hit.normal,
                body: hit.body,
                damage: self.damage(),
                origin,
                translation,
            };
        }
        self.position = [0, 1, 2].map(|axis| self.position[axis] + velocity[axis]);
        self.travelled += step;
        self.speed -= self.speed / PELLET_DRAG_DIVISOR;
        self.age += 1;
        if self.travelled >= range || self.speed < PELLET_MIN_SPEED_SUBUNITS_PER_TICK {
            Travel::Spent
        } else {
            Travel::Flying
        }
    }
}

/// What a blast does to a body whose centre is at `center` (units): damage and the knockback
/// velocity (subunits per tick), or `None` out of reach.
#[must_use]
pub fn splash(blast: [f32; 3], center: [f32; 3]) -> Option<(i32, [i32; 3])> {
    let blast = subunits(blast);
    let center = subunits(center);
    let offset: [i64; 3] = [0, 1, 2].map(|axis| i64::from(center[axis] - blast[axis]));
    #[allow(clippy::cast_sign_loss)]
    let squared = offset.iter().map(|axis| axis * axis).sum::<i64>() as u64;
    #[allow(clippy::cast_possible_wrap)]
    let distance = trig::isqrt(squared) as i64;
    let radius = i64::from(SPLASH_RADIUS_UNITS * SUBUNITS_PER_UNIT);
    if distance >= radius {
        return None;
    }
    let reach = radius - distance;
    let damage = i64::from(SPLASH_DAMAGE) * reach / radius;
    let push = i64::from(SPLASH_KNOCKBACK_SUBUNITS_PER_TICK) * reach / radius;
    let knockback = if distance == 0 {
        [0, push, 0]
    } else {
        offset.map(|axis| axis * push / distance)
    };
    let narrow = |value: i64| i32::try_from(value).unwrap_or(0);
    Some((narrow(damage), knockback.map(narrow)))
}

/// The damage a blast of `damage` at its centre and `radius_units` reach deals at `point`
/// (units): falling off linearly with the integer distance, `None` out of reach. A rocket's
/// splash on a robot part is `falloff(blast, point, SPLASH_RADIUS_UNITS, SPLASH_DAMAGE)`, like
/// [`splash`] for a body's centre.
#[must_use]
pub fn falloff(blast: [f32; 3], point: [f32; 3], radius_units: i32, damage: i32) -> Option<i32> {
    let blast = subunits(blast);
    let point = subunits(point);
    let offset: [i64; 3] = [0, 1, 2].map(|axis| i64::from(point[axis] - blast[axis]));
    #[allow(clippy::cast_sign_loss)]
    let squared = offset.iter().map(|axis| axis * axis).sum::<i64>() as u64;
    #[allow(clippy::cast_possible_wrap)]
    let distance = trig::isqrt(squared) as i64;
    let radius = i64::from(radius_units * SUBUNITS_PER_UNIT);
    if distance >= radius {
        return None;
    }
    i32::try_from(i64::from(damage) * (radius - distance) / radius).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cycle_runs_through_the_armed_guns_in_rotation_order_and_wraps() {
        use Weapon::{EnergyBlade, EnergyRifle, Grapple, Laser, RailGun, RocketLauncher, Shotgun};
        let at = Instance::new;
        let (rifle, blade, grapple, shotgun, laser) = (
            at(7, EnergyRifle),
            at(8, EnergyBlade),
            at(9, Grapple),
            at(10, Shotgun),
            at(11, Laser),
        );
        // The blade and the grapple module stay on their own keys.
        let standard = [rifle, blade, grapple, shotgun, laser];
        assert_eq!(holdable(&standard), [rifle, shotgun, laser]);
        assert_eq!(cycled(rifle, 1, &standard), Some(shotgun));
        assert_eq!(cycled(shotgun, 1, &standard), Some(laser));
        assert_eq!(cycled(laser, 1, &standard), Some(rifle));
        assert_eq!(cycled(rifle, -1, &standard), Some(laser));
        assert_eq!(cycled(laser, -1, &standard), Some(shotgun));
        // Several notches at once, either way.
        assert_eq!(cycled(rifle, 2, &standard), Some(laser));
        assert_eq!(cycled(rifle, -4, &standard), Some(laser));
        assert_eq!(cycled(rifle, 0, &standard), None);
        // The shotgun lost: skipped, and from it (still held) one step reaches a neighbour.
        let lost = [rifle, blade, grapple, laser];
        assert_eq!(cycled(rifle, 1, &lost), Some(laser));
        assert_eq!(cycled(shotgun, 1, &lost), Some(laser));
        assert_eq!(cycled(shotgun, -1, &lost), Some(rifle));
        // A held weapon past the last armed one wraps to the first.
        assert_eq!(cycled(laser, 1, &[rifle, shotgun]), Some(rifle));
        let rockets = at(3, RocketLauncher);
        assert_eq!(cycled(at(2, RailGun), -1, &[rockets]), Some(rockets));
        // Only the blade left: it is the cycle; nothing at all: no cycle.
        assert_eq!(holdable(&[blade]), [blade]);
        assert_eq!(cycled(rifle, 1, &[blade]), Some(blade));
        assert_eq!(cycled(blade, -1, &[blade]), Some(blade));
        assert_eq!(cycled(rifle, 1, &[]), None);
        // The grapple module is never held (Atlas D2.i104.2).
        assert_eq!(cycled(rifle, 1, &[grapple]), None);
    }

    #[test]
    fn every_copy_of_a_gun_is_its_own_place_in_the_cycle() {
        use Weapon::{EnergyBlade, RailGun, RocketLauncher};
        let at = Instance::new;
        // Two rail guns, four rocket launchers and two blades, mounted in any part order.
        let (rail_a, rail_b) = (at(12, RailGun), at(7, RailGun));
        let pods = [9, 3, 14, 10].map(|part| at(part, RocketLauncher));
        let mut armed = vec![rail_a, at(8, EnergyBlade), rail_b, at(2, EnergyBlade)];
        armed.extend(pods);
        let cycle = holdable(&armed);
        assert_eq!(
            cycle,
            [
                rail_b,
                rail_a,
                at(3, RocketLauncher),
                at(9, RocketLauncher),
                at(10, RocketLauncher),
                at(14, RocketLauncher),
            ]
        );
        assert_eq!(spawn_weapon(&armed), Some(rail_b));
        assert_eq!(cycled(rail_b, 1, &armed), Some(rail_a));
        assert_eq!(cycled(rail_b, -1, &armed), Some(at(14, RocketLauncher)));
        // One rail gun lost: the other stays in the cycle, and from the lost one a step reaches
        // its neighbours.
        let lost: Vec<Instance> = armed.iter().copied().filter(|i| *i != rail_b).collect();
        assert_eq!(cycled(rail_b, 1, &lost), Some(rail_a));
        assert_eq!(cycled(rail_b, -1, &lost), Some(at(14, RocketLauncher)));
        // A weapon selects the copy held if it is one, else its first holdable copy; a part
        // selects exactly that copy; anything not holdable is refused.
        let rockets = Select::Weapon(RocketLauncher);
        assert_eq!(
            resolve(rockets, rail_b, &armed),
            Some(at(3, RocketLauncher))
        );
        assert_eq!(
            resolve(rockets, at(10, RocketLauncher), &armed),
            Some(at(10, RocketLauncher))
        );
        assert_eq!(resolve(RailGun.into(), rail_b, &lost), Some(rail_a));
        assert_eq!(resolve(rail_a.into(), rail_b, &armed), Some(rail_a));
        assert_eq!(resolve(Select::Part(PartId(8)), rail_b, &armed), None);
        assert_eq!(resolve(rail_b.into(), rail_a, &lost), None);
        // Nothing but blades: the first is held.
        let blades = [at(8, EnergyBlade), at(2, EnergyBlade)];
        assert_eq!(spawn_weapon(&blades), Some(at(2, EnergyBlade)));
    }

    #[test]
    fn a_fighter_spawns_holding_its_first_gun_never_the_grapple() {
        use Weapon::{EnergyBlade, EnergyRifle, Flamethrower, Grapple, Laser};
        let at = Instance::new;
        let pyro = [
            at(7, Laser),
            at(8, EnergyBlade),
            at(9, Flamethrower),
            at(10, Grapple),
        ];
        assert_eq!(spawn_weapon(&pyro), Some(at(7, Laser)));
        let standard = [at(7, EnergyRifle), at(8, EnergyBlade), at(9, Grapple)];
        assert_eq!(spawn_weapon(&standard), Some(at(7, EnergyRifle)));
        // Nothing but the grapple and the blade: the blade; the grapple alone: nothing.
        assert_eq!(
            spawn_weapon(&[at(8, EnergyBlade), at(9, Grapple)]),
            Some(at(8, EnergyBlade))
        );
        assert_eq!(spawn_weapon(&[at(9, Grapple)]), None);
        assert_eq!(spawn_weapon(&[]), None);
    }

    #[test]
    fn the_grapple_module_has_its_own_cooldown_and_pays_first() {
        let mut arsenal = Arsenal::default();
        // Refused energy: no hook and no cooldown.
        assert!(!arsenal.hook(0, 45, || false));
        assert_eq!(arsenal.grapple_cooldown(0), 0);
        assert!(arsenal.hook(0, 45, || true));
        assert_eq!(arsenal.grapple_cooldown(10), 35);
        assert!(!arsenal.hook(44, 45, || true));
        // The gun fires meanwhile: the two do not share a cooldown.
        assert_eq!(
            arsenal.trigger(None, false, true, 1),
            Some(Instance::default())
        );
        assert!(arsenal.hook(45, 45, || true));
    }

    #[test]
    fn aim_points_where_the_yaw_and_pitch_say() {
        assert_eq!(aim(0, 0), [0, 0, trig::TRIG_ONE]);
        assert_eq!(aim(0, -MAX_PITCH), [0, -trig::TRIG_ONE, 0]);
        let [x, y, z] = aim(trig::YAW_QUARTER_TURN, 0);
        assert_eq!((x, y, z), (trig::TRIG_ONE, 0, 0));
    }

    #[test]
    fn the_trigger_respects_refire_and_a_switch_delays_nothing() {
        let mut arsenal = Arsenal::default();
        let rifle = Some(Instance::default());
        assert_eq!(arsenal.trigger(None, false, true, 0), rifle);
        assert_eq!(arsenal.trigger(None, false, true, 5), None);
        assert_eq!(arsenal.trigger(None, false, true, 6), rifle);
        // Every mounted gun is out: the primary trigger fires the newly held one at once.
        let rail = Some(Instance::new(13, Weapon::RailGun));
        assert_eq!(arsenal.trigger(rail, false, true, 7), rail);
        assert_eq!(arsenal.trigger(None, false, true, 8), None);
    }

    /// Fires `requests` at `tick` with every activation granted: the copies that fired.
    fn fire(arsenal: &mut Arsenal, requests: &[Instance], tick: u32) -> Vec<Instance> {
        arsenal
            .trigger_tuned(
                None,
                None,
                requests,
                tick,
                |instance| instance.weapon.refire_ticks(),
                |_| true,
            )
            .shots()
            .to_vec()
    }

    #[test]
    fn each_copy_of_a_gun_keeps_its_own_refire_and_they_fire_together() {
        let (first, second) = (
            Instance::new(7, Weapon::RailGun),
            Instance::new(9, Weapon::RailGun),
        );
        let mut arsenal = Arsenal::holding(first);
        // Asked in any order, granted by part.
        assert_eq!(fire(&mut arsenal, &[second, first], 0), [first, second]);
        assert_eq!(arsenal.refire_left(first.part, 0), 72);
        assert_eq!(arsenal.refire_left(second.part, 0), 72);
        assert_eq!(fire(&mut arsenal, &[first, second], 71), []);
        // A copy that is ready fires alone while the other recovers.
        assert_eq!(fire(&mut arsenal, &[first], 72), [first]);
        assert_eq!(fire(&mut arsenal, &[first, second], 100), [second]);
        assert_eq!(arsenal.cooldown_of(first, 100), 44);
        assert_eq!(arsenal.cooldown_of(second, 100), 72);
        // Repeats are one request.
        assert_eq!(fire(&mut arsenal, &[first, first, first], 144), [first]);
    }

    #[test]
    fn a_refused_copy_refuses_only_itself() {
        let rail = Instance::new(7, Weapon::RailGun);
        let rifle = Instance::new(9, Weapon::EnergyRifle);
        let mut arsenal = Arsenal::default();
        let volley = arsenal.trigger_tuned(
            None,
            None,
            &[rail, rifle],
            0,
            |instance| instance.weapon.refire_ticks(),
            |instance| instance.weapon != Weapon::RailGun,
        );
        assert_eq!(volley.shots(), [rifle]);
        // The refused rail gun spent no cooldown.
        assert_eq!(arsenal.cooldown_of(rail, 0), 0);
        assert_eq!(arsenal.cooldown_of(rifle, 0), 6);
    }

    #[test]
    fn rocket_launchers_share_a_salvo_limit_and_other_guns_do_not() {
        let pods = [7, 8, 9, 10].map(|part| Instance::new(part, Weapon::RocketLauncher));
        let rifle = Instance::new(11, Weapon::EnergyRifle);
        let salvo = Weapon::RocketLauncher.salvo().unwrap();
        let mut arsenal = Arsenal::default();
        let mut all = pods.to_vec();
        all.push(rifle);
        let mut launches = Vec::new();
        for tick in 0..120 {
            let fired = fire(&mut arsenal, &all, tick);
            let rockets: Vec<Instance> = fired
                .iter()
                .copied()
                .filter(|fired| fired.weapon == Weapon::RocketLauncher)
                .collect();
            assert!(
                rockets.len() <= usize::from(salvo.per_tick),
                "{tick}: {fired:?}"
            );
            launches.extend(rockets.iter().map(|rocket| (tick, rocket.part.0)));
            // The rifle is no rocket: it keeps its own cadence beside them.
            assert_eq!(fired.contains(&rifle), tick % 6 == 0, "{tick}");
        }
        // Staggered `spacing` apart, by part, each launcher then on its own refire.
        let spacing = salvo.spacing;
        assert_eq!(
            launches[..5],
            [
                (0, 7),
                (spacing, 8),
                (2 * spacing, 9),
                (3 * spacing, 10),
                (48, 7)
            ]
        );
        // A launcher that has not fired waits out its family's spacing.
        let mut fresh = Arsenal::default();
        assert_eq!(fire(&mut fresh, &pods, 0), [pods[0]]);
        assert_eq!(fresh.cooldown_of(pods[1], 1), spacing - 1);
        assert_eq!(fresh.refire_left(pods[1].part, 1), 0);
    }

    #[test]
    fn the_blade_swings_over_any_gun_and_is_never_held() {
        let mut arsenal = Arsenal::default();
        let blade = Some(TEST_BLADE);
        assert_eq!(arsenal.trigger(blade, true, true, 0), blade);
        assert_eq!(arsenal.held, Instance::default());
        // The gun waits out the lockout; the blade its own refire.
        assert_eq!(arsenal.trigger(None, false, true, 17), None);
        assert_eq!(
            arsenal.trigger(None, true, true, 18),
            Some(Instance::default())
        );
        assert_eq!(arsenal.trigger(None, true, false, 29), None);
        assert_eq!(arsenal.trigger(None, true, false, 30), blade);
    }

    #[test]
    fn the_flame_ignites_while_the_trigger_stays_down() {
        let mut arsenal = Arsenal::default();
        let flame = Some(Instance::new(12, Weapon::Flamethrower));
        arsenal.trigger(flame, false, false, 0);
        let mut tick = 0;
        for _ in 0..FLAME_IGNITION_TICKS {
            assert_eq!(arsenal.trigger(None, false, true, tick), flame);
            assert!(!arsenal.ignited(), "tick {tick}");
            tick += 1;
        }
        assert_eq!(arsenal.trigger(None, false, true, tick), flame);
        assert!(arsenal.ignited());
        // Letting go puts it out: the next burst ignites from the start.
        arsenal.trigger(None, false, false, tick + 1);
        assert_eq!(arsenal.trigger(None, false, true, tick + 2), flame);
        assert!(!arsenal.ignited());
    }

    #[test]
    fn pellets_hurt_less_as_they_slow_down() {
        let mut pellet = Pellet::launch(BodyId(0), [0.0; 3], [0, 0, trig::TRIG_ONE], 9);
        assert_eq!(pellet.damage(), 9);
        let mut damages = vec![pellet.damage()];
        let mut ticks = 0;
        let world = World::new(crate::physics::Vec3i::new(0, 0, 0));
        while pellet.fly(|origin, translation| world.cast_ray(origin, translation, |_| true))
            == Travel::Flying
        {
            damages.push(pellet.damage());
            ticks += 1;
        }
        // Never more on the way, and still two thirds at the end of its range.
        assert!(
            damages.windows(2).all(|pair| pair[1] <= pair[0]),
            "{damages:?}"
        );
        assert_eq!(damages.last(), Some(&6), "{damages:?}");
        assert_eq!(ticks, 7);
        assert_eq!(pellet.travelled, PELLET_RANGE_UNITS * SUBUNITS_PER_UNIT);
    }

    #[test]
    fn splash_falls_off_and_pushes_away() {
        let (damage, push) = splash([0.0; 3], [0.0, 90.0, 0.0]).unwrap();
        assert_eq!(damage, 70);
        assert_eq!(push, [0, SPLASH_KNOCKBACK_SUBUNITS_PER_TICK * 7 / 10, 0]);
        assert!(splash([0.0; 3], [300.0, 0.0, 0.0]).is_none());
    }
}
