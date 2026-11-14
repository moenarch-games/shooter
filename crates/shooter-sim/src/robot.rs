//! Robot builds (Atlas D2.i46.4, D2.i46.5): which component sits in each socket of a fighter's
//! robot and which weapon module sits on each hardpoint, the curated component catalog, and the
//! one validation path every build passes, whether it comes from a client, a snapshot or code.
//!
//! A [`Candidate`] is untrusted raw data (socket, component, weapon and mod codes as they arrived);
//! [`Catalog::validate`] turns it into an [`Assembly`], the only form the simulation stores and
//! sends, or rejects it with an [`AssemblyError`]. A build is valid when every socket holds a
//! component of its kind (torso, head and locomotion are required, arms are optional), every
//! weapon sits on a hardpoint its parts provide and can carry, no weapon is mounted twice, and the
//! build stays within its engineering budgets (the locomotion's load, the torso's power, the
//! parts' cooling) and the catalog's build points.
//!
//! **Provisional data.** The exact budgets, attachment rules and point values are open
//! (Atlas Q2.i46.4): the numbers in [`BETA`] are tunable placeholders chosen so that builds trade
//! off against each other, not owner-approved balance. The component keys are the shooter's
//! proposal for the stable part and weapon IDs of the forthcoming robot and weapon kits
//! (moritzbrantner/asset-tooling#595, #594) and are reconciled with them when the kits ship.
//!
//! **Weapon modifications** (Atlas D2.i46.8, shooter#51): a mounted weapon may carry one
//! [`Mod`] from the catalog. A mod costs build points and weighs on the mass, power and cooling
//! budgets like any part, may make its weapon a heavy module (it then needs a heavy hardpoint),
//! fits only the weapons it names, and changes how that weapon fires ([`Tuning`]: damage, refire
//! and range). Every mod trades something away: none is strictly better. The catalog and its
//! numbers are provisional like the rest of [`BETA`].
//!
//! A build does not yet change how a fighter moves or which weapons it carries: every fighter
//! still has the full arsenal, and a weapon without a mod on it fires as shipped. A mounted
//! weapon's mod applies whenever it fires, on the host and in a client's prediction alike. Limb
//! damage (shooter#56), locomotion (shooter#58) and the builder (shooter#57) apply the rest.

use crate::weapons::{Delivery, Weapon};
use std::fmt;

/// Where a component attaches. The torso is the root; the other sockets are on it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum Socket {
    Torso,
    Head,
    LeftArm,
    RightArm,
    Locomotion,
}

impl Socket {
    pub const ALL: [Self; 5] = [
        Self::Torso,
        Self::Head,
        Self::LeftArm,
        Self::RightArm,
        Self::Locomotion,
    ];

    /// Its stable wire code.
    #[must_use]
    pub const fn index(self) -> u8 {
        self as u8
    }

    #[must_use]
    pub fn from_index(index: u8) -> Option<Self> {
        Self::ALL.get(usize::from(index)).copied()
    }

    /// The kind of component it takes.
    #[must_use]
    pub const fn kind(self) -> Kind {
        match self {
            Self::Torso => Kind::Torso,
            Self::Head => Kind::Head,
            Self::LeftArm | Self::RightArm => Kind::Arm,
            Self::Locomotion => Kind::Locomotion,
        }
    }

    /// A build without it is invalid.
    #[must_use]
    pub const fn required(self) -> bool {
        !matches!(self, Self::LeftArm | Self::RightArm)
    }
}

/// Where a weapon module mounts: a hand on each arm, a shoulder on each side of the torso.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum Hardpoint {
    LeftHand,
    RightHand,
    LeftShoulder,
    RightShoulder,
}

impl Hardpoint {
    pub const ALL: [Self; 4] = [
        Self::LeftHand,
        Self::RightHand,
        Self::LeftShoulder,
        Self::RightShoulder,
    ];

    /// Its stable wire code.
    #[must_use]
    pub const fn index(self) -> u8 {
        self as u8
    }

    #[must_use]
    pub fn from_index(index: u8) -> Option<Self> {
        Self::ALL.get(usize::from(index)).copied()
    }

    /// The socket whose component provides it.
    #[must_use]
    pub const fn provider(self) -> Socket {
        match self {
            Self::LeftHand => Socket::LeftArm,
            Self::RightHand => Socket::RightArm,
            Self::LeftShoulder | Self::RightShoulder => Socket::Torso,
        }
    }

    #[must_use]
    pub const fn hand(self) -> bool {
        matches!(self, Self::LeftHand | Self::RightHand)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Kind {
    Torso,
    Head,
    Arm,
    Locomotion,
}

/// How heavy a weapon module is, and how heavy a module a hardpoint carries.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum Size {
    Light,
    Heavy,
}

/// A component's stable code in the catalog (on the wire).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct ComponentId(pub u8);

/// A catalog component: a torso, head, arm or locomotion module. Masses are kilograms; power,
/// heat and cooling share one unit; all values are provisional (Q2.i46.4).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Component {
    pub id: ComponentId,
    /// Its stable data ID, shared with the robot kit's part ID.
    pub key: &'static str,
    pub kind: Kind,
    pub mass: u16,
    /// Power it supplies (the torso's reactor).
    pub power: u16,
    /// Power it draws.
    pub draw: u16,
    /// Heat it can shed.
    pub cooling: u16,
    /// Mass it carries (locomotion): everything but itself.
    pub load: u16,
    pub points: u16,
    /// The hardpoints it provides (an arm's hand, the torso's two shoulders) and the heaviest
    /// module each carries.
    pub mounts: Option<Size>,
}

/// A weapon module: one of the shipped weapons (D2.i46.8) as a mountable part.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Module {
    pub weapon: Weapon,
    /// Its stable data ID, shared with the weapon kit's weapon ID.
    pub key: &'static str,
    pub size: Size,
    /// It mounts only on a hand (the blade).
    pub hand_only: bool,
    pub mass: u16,
    pub draw: u16,
    /// Heat it makes.
    pub heat: u16,
    pub points: u16,
}

/// A weapon modification's stable code in the catalog (on the wire).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct ModId(pub u8);

/// How a mod changes its weapon's fire, in percent of the shipped weapon's values (100 changes
/// nothing). Damage and range round down (to at least 1), refire rounds up; a continuous weapon
/// keeps firing every tick.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Tuning {
    pub damage_percent: u16,
    pub refire_percent: u16,
    pub range_percent: u16,
}

impl Tuning {
    /// The shipped weapon, unchanged.
    pub const BASE: Self = Self {
        damage_percent: 100,
        refire_percent: 100,
        range_percent: 100,
    };

    /// Ticks between two of `weapon`'s shots.
    #[must_use]
    pub fn refire_ticks(self, weapon: Weapon) -> u32 {
        let base = weapon.refire_ticks();
        if base <= 1 {
            return base;
        }
        (base * u32::from(self.refire_percent)).div_ceil(100).max(1)
    }

    /// How `weapon`'s shot reaches its target, tuned.
    #[must_use]
    pub fn delivery(self, weapon: Weapon) -> Delivery {
        let scale = |value: i32, percent: u16| (value * i32::from(percent) / 100).max(1);
        match weapon.delivery() {
            Delivery::Hitscan {
                damage,
                range_units,
            } => Delivery::Hitscan {
                damage: scale(damage, self.damage_percent),
                range_units: scale(range_units, self.range_percent),
            },
            Delivery::Pellets { damage } => Delivery::Pellets {
                damage: scale(damage, self.damage_percent),
            },
            Delivery::Flame { damage } => Delivery::Flame {
                damage: scale(damage, self.damage_percent),
            },
            other => other,
        }
    }
}

/// A weapon modification (D2.i46.8): what it costs a build, which weapons it fits and how it
/// changes their fire. Masses are kilograms; power, heat and cooling share the components' unit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Mod {
    pub id: ModId,
    /// Its stable data ID.
    pub key: &'static str,
    /// The weapons it fits.
    pub fits: &'static [Weapon],
    /// It makes its weapon a heavy module (a heavy hardpoint only).
    pub heavy: bool,
    pub mass: u16,
    pub draw: u16,
    /// Heat it adds...
    pub heat: u16,
    /// ...or sheds.
    pub cooling: u16,
    pub points: u16,
    pub tuning: Tuning,
}

/// The components, modules and weapon mods builds are made of, and the build-point budget.
#[derive(Clone, Copy, Debug)]
pub struct Catalog {
    pub components: &'static [Component],
    pub modules: &'static [Module],
    pub mods: &'static [Mod],
    pub build_points: u16,
}

const fn part(
    id: u8,
    key: &'static str,
    kind: Kind,
    [mass, power, draw, cooling, load, points]: [u16; 6],
    mounts: Option<Size>,
) -> Component {
    Component {
        id: ComponentId(id),
        key,
        kind,
        mass,
        power,
        draw,
        cooling,
        load,
        points,
        mounts,
    }
}

const fn module(
    weapon: Weapon,
    key: &'static str,
    size: Size,
    [mass, draw, heat, points]: [u16; 4],
) -> Module {
    Module {
        weapon,
        key,
        size,
        hand_only: matches!(weapon, Weapon::EnergyBlade),
        mass,
        draw,
        heat,
        points,
    }
}

const fn modification(
    id: u8,
    key: &'static str,
    fits: &'static [Weapon],
    heavy: bool,
    [mass, draw, heat, cooling, points]: [u16; 5],
    [damage_percent, refire_percent, range_percent]: [u16; 3],
) -> Mod {
    Mod {
        id: ModId(id),
        key,
        fits,
        heavy,
        mass,
        draw,
        heat,
        cooling,
        points,
        tuning: Tuning {
            damage_percent,
            refire_percent,
            range_percent,
        },
    }
}

/// The weapons the firing mods fit: the single-shot guns whose damage is worth tuning.
const SINGLE_SHOT: &[Weapon] = &[Weapon::EnergyRifle, Weapon::RailGun, Weapon::Shotgun];
/// The hitscan beams a longer barrel reaches further with.
const BEAMS: &[Weapon] = &[Weapon::EnergyRifle, Weapon::RailGun, Weapon::Laser];

/// The beta's components, every one available from the start (no unlocks, D2.i46.5): two of
/// each kind, light and heavy, and the eight shipped weapons. Provisional values (Q2.i46.4).
#[rustfmt::skip]
pub const BETA: Catalog = Catalog {
    components: &[
        //    code key                    kind               mass power draw cool load points   mounts
        part(1,  "robot.torso.striker", Kind::Torso,      [60,  100, 0,  40, 0,   15], Some(Size::Light)),
        part(2,  "robot.torso.bulwark", Kind::Torso,      [110, 160, 0,  60, 0,   25], Some(Size::Heavy)),
        part(10, "robot.head.sensor",   Kind::Head,       [10,  0,   10, 10, 0,   5],  None),
        part(11, "robot.head.optic",    Kind::Head,       [15,  0,   20, 5,  0,   10], None),
        part(20, "robot.arm.light",     Kind::Arm,        [20,  0,   5,  0,  0,   5],  Some(Size::Light)),
        part(21, "robot.arm.heavy",     Kind::Arm,        [35,  0,   10, 0,  0,   10], Some(Size::Heavy)),
        part(30, "robot.legs.runner",   Kind::Locomotion, [50,  0,   25, 0,  260, 15], None),
        part(31, "robot.legs.strider",  Kind::Locomotion, [80,  0,   35, 0,  400, 20], None),
    ],
    modules: &[
        //     weapon                  key                       size          mass draw heat points
        module(Weapon::EnergyRifle,    "weapon.energy-rifle",    Size::Light, [12, 15, 10, 10]),
        module(Weapon::RailGun,        "weapon.rail-gun",        Size::Heavy, [25, 35, 25, 20]),
        module(Weapon::RocketLauncher, "weapon.rocket-launcher", Size::Heavy, [30, 10, 20, 20]),
        module(Weapon::Grapple,        "weapon.grapple-hook",    Size::Light, [10, 10, 5,  10]),
        module(Weapon::EnergyBlade,    "weapon.energy-blade",    Size::Light, [8,  10, 5,  5]),
        module(Weapon::Shotgun,        "weapon.shotgun",         Size::Light, [15, 10, 15, 15]),
        module(Weapon::Laser,          "weapon.laser",           Size::Light, [14, 30, 30, 15]),
        module(Weapon::Flamethrower,   "weapon.flamethrower",    Size::Light, [18, 20, 35, 15]),
    ],
    mods: &[
        //           code key                fits          heavy  mass draw heat cool points  damage% refire% range%
        // Faster fire for less damage a shot; hot and power-hungry.
        modification(1, "mod.rapid-cycler", SINGLE_SHOT,  false, [4,   10,  20,  0,   10], [75,  65,  100]),
        // More damage a shot for slower fire; the most power.
        modification(2, "mod.overcharger",  SINGLE_SHOT,  false, [6,   20,  15,  0,   10], [125, 130, 100]),
        // Half again the range, a little slower fire, and too heavy for a light mount.
        modification(3, "mod.long-barrel",  BEAMS,        true,  [10,  0,   0,   0,   5],  [100, 110, 150]),
        // Sheds heat for the rest of the build; mass and points, no change to the fire.
        modification(4, "mod.heat-sink",    &Weapon::ALL, false, [10,  0,   0,   20,  5],  [100, 100, 100]),
    ],
    build_points: 120,
};

/// Untrusted build data as it arrived: `(socket, component)` and `(hardpoint, weapon)` codes,
/// in any order. Only [`Catalog::validate`] turns it into an [`Assembly`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Candidate {
    pub parts: Vec<(u8, u8)>,
    pub mounts: Vec<(u8, u8)>,
    /// `(hardpoint, mod)` codes: the mod on the weapon at that hardpoint.
    pub mods: Vec<(u8, u8)>,
}

/// A validated robot build, in canonical (socket and hardpoint) order.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Assembly {
    parts: [Option<ComponentId>; Socket::ALL.len()],
    mounts: [Option<Weapon>; Hardpoint::ALL.len()],
    mods: [Option<ModId>; Hardpoint::ALL.len()],
}

/// The build every fighter starts with: a light torso, the sensor head, two light arms and the
/// runner legs, with the energy rifle and the blade in hand and the grapple and the shotgun on
/// the shoulders. Valid in [`BETA`].
pub const STANDARD: Assembly = Assembly {
    parts: [
        Some(ComponentId(1)),
        Some(ComponentId(10)),
        Some(ComponentId(20)),
        Some(ComponentId(20)),
        Some(ComponentId(30)),
    ],
    mounts: [
        Some(Weapon::EnergyRifle),
        Some(Weapon::EnergyBlade),
        Some(Weapon::Grapple),
        Some(Weapon::Shotgun),
    ],
    mods: [None; Hardpoint::ALL.len()],
};

/// A heavy build that spends the whole build-point budget: the heavy torso, the sensor head, a
/// heavy left arm with the rail gun, a light right arm with the blade, the strider legs, and the
/// rocket launcher and the grapple on the shoulders. Valid in [`BETA`].
pub const HEAVY: Assembly = Assembly {
    parts: [
        Some(ComponentId(2)),
        Some(ComponentId(10)),
        Some(ComponentId(21)),
        Some(ComponentId(20)),
        Some(ComponentId(31)),
    ],
    mounts: [
        Some(Weapon::RailGun),
        Some(Weapon::EnergyBlade),
        Some(Weapon::RocketLauncher),
        Some(Weapon::Grapple),
    ],
    mods: [None; Hardpoint::ALL.len()],
};

/// A fast, light loadout on the standard frame: the rapid-cycler rifle in the left hand, the
/// blade, the grapple, and the shotgun with a heat sink (without it the cycler overheats the
/// standard frame). Draws exactly the light torso's power. Valid in [`BETA`].
pub const SKIRMISHER: Assembly = Assembly {
    mods: [Some(ModId(1)), None, None, Some(ModId(4))],
    ..STANDARD
};

/// A heavy, long-range loadout that spends the whole build-point budget: the heavy torso with an
/// overcharged rail gun (two hits a kill instead of three) on the left shoulder and the grapple on the
/// right, a heavy left arm for the long-barrel rifle, the blade in the light right hand, the
/// runner legs. Valid in [`BETA`].
pub const MARKSMAN: Assembly = Assembly {
    parts: [
        Some(ComponentId(2)),
        Some(ComponentId(10)),
        Some(ComponentId(21)),
        Some(ComponentId(20)),
        Some(ComponentId(30)),
    ],
    mounts: [
        Some(Weapon::EnergyRifle),
        Some(Weapon::EnergyBlade),
        Some(Weapon::RailGun),
        Some(Weapon::Grapple),
    ],
    mods: [Some(ModId(3)), None, Some(ModId(2)), None],
};

/// The named loadouts a person picks from until the builder (shooter#57): the unmodified
/// standard and heavy builds and the two contrasting modded ones.
pub const LOADOUTS: [(&str, Assembly); 4] = [
    ("standard", STANDARD),
    ("heavy", HEAVY),
    ("skirmisher", SKIRMISHER),
    ("marksman", MARKSMAN),
];

/// The loadout called `name` in [`LOADOUTS`] (any case).
#[must_use]
pub fn loadout(name: &str) -> Option<Assembly> {
    LOADOUTS
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name.trim()))
        .map(|(_, build)| *build)
}

impl Default for Assembly {
    fn default() -> Self {
        STANDARD
    }
}

impl Assembly {
    /// The component in `socket`.
    #[must_use]
    pub const fn part(&self, socket: Socket) -> Option<ComponentId> {
        self.parts[socket as usize]
    }

    /// The weapon on `hardpoint`.
    #[must_use]
    pub const fn mount(&self, hardpoint: Hardpoint) -> Option<Weapon> {
        self.mounts[hardpoint as usize]
    }

    /// The mod on the weapon at `hardpoint`.
    #[must_use]
    pub const fn modification(&self, hardpoint: Hardpoint) -> Option<ModId> {
        self.mods[hardpoint as usize]
    }

    /// The mod on `weapon`, wherever it is mounted.
    #[must_use]
    pub fn mod_on(&self, weapon: Weapon) -> Option<ModId> {
        self.mounts()
            .find(|(_, mounted)| *mounted == weapon)
            .and_then(|(hardpoint, _)| self.modification(hardpoint))
    }

    /// The mods with the weapon each is on, in hardpoint order.
    pub fn mods(&self) -> impl Iterator<Item = (Hardpoint, Weapon, ModId)> + '_ {
        self.mounts().filter_map(|(hardpoint, weapon)| {
            Some((hardpoint, weapon, self.modification(hardpoint)?))
        })
    }

    /// The installed components, in socket order.
    pub fn parts(&self) -> impl Iterator<Item = (Socket, ComponentId)> + '_ {
        Socket::ALL
            .into_iter()
            .filter_map(|socket| Some((socket, self.part(socket)?)))
    }

    /// The mounted weapons, in hardpoint order.
    pub fn mounts(&self) -> impl Iterator<Item = (Hardpoint, Weapon)> + '_ {
        Hardpoint::ALL
            .into_iter()
            .filter_map(|hardpoint| Some((hardpoint, self.mount(hardpoint)?)))
    }

    /// Its canonical raw form (what the wire carries).
    #[must_use]
    pub fn candidate(&self) -> Candidate {
        Candidate {
            parts: self
                .parts()
                .map(|(socket, id)| (socket.index(), id.0))
                .collect(),
            mounts: self
                .mounts()
                .map(|(hardpoint, weapon)| (hardpoint.index(), weapon.index()))
                .collect(),
            mods: self
                .mods()
                .map(|(hardpoint, _, id)| (hardpoint.index(), id.0))
                .collect(),
        }
    }
}

/// A build's totals against its limits.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Budget {
    /// Mass on the locomotion (everything but the locomotion itself)...
    pub carried: u32,
    /// ...and the mass it carries.
    pub load: u32,
    pub draw: u32,
    pub power: u32,
    pub heat: u32,
    pub cooling: u32,
    pub points: u32,
    pub build_points: u32,
}

/// Why a candidate is not a valid build.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssemblyError {
    UnknownSocket(u8),
    UnknownComponent(u8),
    UnknownHardpoint(u8),
    UnknownWeapon(u8),
    /// The weapon is not in the catalog.
    UnavailableWeapon(Weapon),
    UnknownMod(u8),
    /// The hardpoint carries two mods.
    DuplicateMod(Hardpoint),
    /// A mod on a hardpoint with no weapon.
    Unmounted(Hardpoint),
    /// The mod does not fit the weapon.
    DoesNotFit(ModId, Weapon),
    DuplicateSocket(Socket),
    DuplicateHardpoint(Hardpoint),
    DuplicateWeapon(Weapon),
    /// The component is not the kind the socket takes.
    WrongKind(Socket, ComponentId),
    Missing(Socket),
    /// No installed component provides the hardpoint.
    NoHardpoint(Hardpoint),
    /// The hardpoint cannot carry a module that heavy.
    TooHeavyForMount(Hardpoint, Weapon),
    /// The weapon mounts only on a hand.
    HandOnly(Hardpoint, Weapon),
    Overweight {
        carried: u32,
        load: u32,
    },
    Underpowered {
        draw: u32,
        power: u32,
    },
    Overheated {
        heat: u32,
        cooling: u32,
    },
    OverBudget {
        points: u32,
        build_points: u32,
    },
}

impl fmt::Display for AssemblyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownSocket(code) => write!(formatter, "unknown socket {code}"),
            Self::UnknownComponent(code) => write!(formatter, "unknown component {code}"),
            Self::UnknownHardpoint(code) => write!(formatter, "unknown hardpoint {code}"),
            Self::UnknownWeapon(code) => write!(formatter, "unknown weapon {code}"),
            Self::UnavailableWeapon(weapon) => write!(formatter, "{weapon:?} is not available"),
            Self::UnknownMod(code) => write!(formatter, "unknown weapon mod {code}"),
            Self::DuplicateMod(hardpoint) => write!(formatter, "{hardpoint:?} has two mods"),
            Self::Unmounted(hardpoint) => {
                write!(formatter, "a mod on {hardpoint:?}, which holds no weapon")
            }
            Self::DoesNotFit(id, weapon) => {
                write!(formatter, "mod {} does not fit {weapon:?}", id.0)
            }
            Self::DuplicateSocket(socket) => write!(formatter, "{socket:?} is filled twice"),
            Self::DuplicateHardpoint(hardpoint) => {
                write!(formatter, "{hardpoint:?} is filled twice")
            }
            Self::DuplicateWeapon(weapon) => write!(formatter, "{weapon:?} is mounted twice"),
            Self::WrongKind(socket, id) => {
                write!(formatter, "component {} does not fit {socket:?}", id.0)
            }
            Self::Missing(socket) => write!(formatter, "no component in {socket:?}"),
            Self::NoHardpoint(hardpoint) => write!(formatter, "nothing provides {hardpoint:?}"),
            Self::TooHeavyForMount(hardpoint, weapon) => {
                write!(formatter, "{hardpoint:?} cannot carry {weapon:?}")
            }
            Self::HandOnly(hardpoint, weapon) => {
                write!(formatter, "{weapon:?} mounts on a hand, not {hardpoint:?}")
            }
            Self::Overweight { carried, load } => {
                write!(
                    formatter,
                    "carries {carried} kg, the locomotion takes {load} kg"
                )
            }
            Self::Underpowered { draw, power } => {
                write!(formatter, "draws {draw} power, the torso supplies {power}")
            }
            Self::Overheated { heat, cooling } => {
                write!(formatter, "makes {heat} heat, sheds {cooling}")
            }
            Self::OverBudget {
                points,
                build_points,
            } => write!(
                formatter,
                "costs {points} build points, the budget is {build_points}"
            ),
        }
    }
}

impl std::error::Error for AssemblyError {}

impl Catalog {
    #[must_use]
    pub fn component(&self, id: ComponentId) -> Option<&Component> {
        self.components.iter().find(|component| component.id == id)
    }

    #[must_use]
    pub fn module(&self, weapon: Weapon) -> Option<&Module> {
        self.modules.iter().find(|module| module.weapon == weapon)
    }

    #[must_use]
    pub fn modification(&self, id: ModId) -> Option<&Mod> {
        self.mods.iter().find(|modification| modification.id == id)
    }

    /// How `weapon` fires in `build`: its mod's tuning, else the shipped weapon's.
    #[must_use]
    pub fn tuning(&self, build: &Assembly, weapon: Weapon) -> Tuning {
        build
            .mod_on(weapon)
            .and_then(|id| self.modification(id))
            .map_or(Tuning::BASE, |modification| modification.tuning)
    }

    /// The one validation path for a build: structure first (known, unique codes), then
    /// attachment (kinds, required sockets, hardpoints, which mods fit where), then the budgets
    /// (mass, power, cooling, build points), each in a fixed order so the same candidate always
    /// fails the same way.
    ///
    /// # Errors
    ///
    /// The first rule the candidate breaks.
    pub fn validate(&self, candidate: &Candidate) -> Result<Assembly, AssemblyError> {
        let mut assembly = Assembly {
            parts: [None; Socket::ALL.len()],
            mounts: [None; Hardpoint::ALL.len()],
            mods: [None; Hardpoint::ALL.len()],
        };
        for &(socket, id) in &candidate.parts {
            let socket = Socket::from_index(socket).ok_or(AssemblyError::UnknownSocket(socket))?;
            let component = self
                .component(ComponentId(id))
                .ok_or(AssemblyError::UnknownComponent(id))?;
            let slot = &mut assembly.parts[usize::from(socket.index())];
            if slot.is_some() {
                return Err(AssemblyError::DuplicateSocket(socket));
            }
            if component.kind != socket.kind() {
                return Err(AssemblyError::WrongKind(socket, component.id));
            }
            *slot = Some(component.id);
        }
        for &(hardpoint, weapon) in &candidate.mounts {
            let hardpoint = Hardpoint::from_index(hardpoint)
                .ok_or(AssemblyError::UnknownHardpoint(hardpoint))?;
            let weapon = Weapon::from_index(weapon).ok_or(AssemblyError::UnknownWeapon(weapon))?;
            let slot = &mut assembly.mounts[usize::from(hardpoint.index())];
            if slot.is_some() {
                return Err(AssemblyError::DuplicateHardpoint(hardpoint));
            }
            *slot = Some(weapon);
        }
        for &(hardpoint, id) in &candidate.mods {
            let hardpoint = Hardpoint::from_index(hardpoint)
                .ok_or(AssemblyError::UnknownHardpoint(hardpoint))?;
            let modification = self
                .modification(ModId(id))
                .ok_or(AssemblyError::UnknownMod(id))?;
            let slot = &mut assembly.mods[usize::from(hardpoint.index())];
            if slot.is_some() {
                return Err(AssemblyError::DuplicateMod(hardpoint));
            }
            *slot = Some(modification.id);
        }
        for socket in Socket::ALL {
            if socket.required() && assembly.part(socket).is_none() {
                return Err(AssemblyError::Missing(socket));
            }
        }
        let mut seen = Vec::with_capacity(Hardpoint::ALL.len());
        for (hardpoint, weapon) in assembly.mounts() {
            if seen.contains(&weapon) {
                return Err(AssemblyError::DuplicateWeapon(weapon));
            }
            seen.push(weapon);
            let module = self
                .module(weapon)
                .ok_or(AssemblyError::UnavailableWeapon(weapon))?;
            let carries = assembly
                .part(hardpoint.provider())
                .and_then(|id| self.component(id))
                .and_then(|component| component.mounts)
                .ok_or(AssemblyError::NoHardpoint(hardpoint))?;
            let modification = assembly
                .modification(hardpoint)
                .and_then(|id| self.modification(id));
            if let Some(modification) = modification
                && !modification.fits.contains(&weapon)
            {
                return Err(AssemblyError::DoesNotFit(modification.id, weapon));
            }
            let size = if modification.is_some_and(|modification| modification.heavy) {
                Size::Heavy
            } else {
                module.size
            };
            if size > carries {
                return Err(AssemblyError::TooHeavyForMount(hardpoint, weapon));
            }
            if module.hand_only && !hardpoint.hand() {
                return Err(AssemblyError::HandOnly(hardpoint, weapon));
            }
        }
        for hardpoint in Hardpoint::ALL {
            if assembly.modification(hardpoint).is_some() && assembly.mount(hardpoint).is_none() {
                return Err(AssemblyError::Unmounted(hardpoint));
            }
        }
        let budget = self.budget(&assembly);
        if budget.carried > budget.load {
            return Err(AssemblyError::Overweight {
                carried: budget.carried,
                load: budget.load,
            });
        }
        if budget.draw > budget.power {
            return Err(AssemblyError::Underpowered {
                draw: budget.draw,
                power: budget.power,
            });
        }
        if budget.heat > budget.cooling {
            return Err(AssemblyError::Overheated {
                heat: budget.heat,
                cooling: budget.cooling,
            });
        }
        if budget.points > budget.build_points {
            return Err(AssemblyError::OverBudget {
                points: budget.points,
                build_points: budget.build_points,
            });
        }
        Ok(assembly)
    }

    /// `assembly`'s totals against its limits (components or modules this catalog lacks count
    /// nothing).
    #[must_use]
    pub fn budget(&self, assembly: &Assembly) -> Budget {
        let mut budget = Budget {
            build_points: u32::from(self.build_points),
            ..Budget::default()
        };
        for (socket, id) in assembly.parts() {
            let Some(component) = self.component(id) else {
                continue;
            };
            if socket == Socket::Locomotion {
                budget.load += u32::from(component.load);
            } else {
                budget.carried += u32::from(component.mass);
            }
            budget.power += u32::from(component.power);
            budget.draw += u32::from(component.draw);
            budget.cooling += u32::from(component.cooling);
            budget.points += u32::from(component.points);
        }
        for (_, weapon) in assembly.mounts() {
            let Some(module) = self.module(weapon) else {
                continue;
            };
            budget.carried += u32::from(module.mass);
            budget.draw += u32::from(module.draw);
            budget.heat += u32::from(module.heat);
            budget.points += u32::from(module.points);
        }
        for (_, _, id) in assembly.mods() {
            let Some(modification) = self.modification(id) else {
                continue;
            };
            budget.carried += u32::from(modification.mass);
            budget.draw += u32::from(modification.draw);
            budget.heat += u32::from(modification.heat);
            budget.cooling += u32::from(modification.cooling);
            budget.points += u32::from(modification.points);
        }
        budget
    }
}
