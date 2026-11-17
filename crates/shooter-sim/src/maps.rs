//! The map catalog: every arena a match can be played on, by a stable [`MapId`].
//!
//! The host picks a map for a match; the clients learn its ID from the snapshots
//! ([`crate::wire`]) and build the same [`Arena`] from this catalog, so the native client, the
//! browser client and the host share one definition of its geometry (collision and drawing),
//! spawn points, jump pads and walkable traversal. Replays carry the ID too. Authored art for
//! a map comes from asset-tooling's arena kit (moritzbrantner/asset-tooling#593): a kit-built map
//! lists the kit's module placements ([`crate::arena_kit`]); its layout, placement and
//! collision data live here.
//!
//! A map recommends a player-capacity range ([`Capacity`]) and bounds a match's slot count by
//! its spawn points ([`MapId::max_slots`]); the slot count itself stays with [`crate::Rules`]
//! (12 by default, the host may set another per match).

use crate::arena::Arena;
use std::fmt;
use std::str::FromStr;

/// A catalogued map. Its string ([`MapId::as_str`]) and wire tag ([`MapId::tag`]) are stable:
/// a new map takes new ones, and neither is ever reused.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum MapId {
    /// The first greybox hall ([`Arena::greybox`]), kept as the test and A/B fixture.
    Greybox,
    /// The greybox yard ([`Arena::greybox_yard`]), a 76 m interim playtest map with a raised
    /// walkway, wall lines and grapple towers (shooter#82): the default until the beta arenas.
    GreyboxYard,
    /// The assembly works ([`Arena::assembly_works`]), the beta's medium arena (shooter#52): a
    /// 56 m multilevel robotic assembly factory built from the arena kit, with mezzanines, a
    /// gantry bridge and crane towers.
    AssemblyWorks,
    /// The proving cell ([`Arena::proving_cell`]), the beta's small close-quarters arena
    /// (shooter#53): a 32 m purpose-built combat test cell built from the arena kit, with a 2 m
    /// test stage, 3 m observation galleries and corner control booths.
    ProvingCell,
}

/// The players a map is laid out for, inclusive. A recommendation for whoever sets up a
/// match; it does not change the default slot count.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Capacity {
    pub min: u8,
    pub max: u8,
}

impl Capacity {
    /// Whether `players` is inside the range.
    #[must_use]
    pub const fn fits(self, players: u8) -> bool {
        self.min <= players && players <= self.max
    }
}

impl fmt::Display for Capacity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}-{}", self.min, self.max)
    }
}

/// A map ID that is not in the catalog.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnknownMap(pub String);

impl fmt::Display for UnknownMap {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let known: Vec<&str> = MapId::ALL.iter().map(|map| map.as_str()).collect();
        write!(
            formatter,
            "unknown map {:?} (maps: {})",
            self.0,
            known.join(", ")
        )
    }
}

impl std::error::Error for UnknownMap {}

impl MapId {
    /// Every catalogued map.
    pub const ALL: [Self; 4] = [
        Self::Greybox,
        Self::GreyboxYard,
        Self::AssemblyWorks,
        Self::ProvingCell,
    ];
    /// The map a match is played on when none is chosen.
    pub const DEFAULT: Self = Self::GreyboxYard;

    /// The stable ID: command lines, the host's configuration, page queries and replays.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Greybox => "greybox",
            Self::GreyboxYard => "greybox-yard",
            Self::AssemblyWorks => "assembly-works",
            Self::ProvingCell => "proving-cell",
        }
    }

    /// The stable wire tag in snapshots (0 is no catalogued map).
    #[must_use]
    pub const fn tag(self) -> u8 {
        match self {
            Self::Greybox => 1,
            Self::GreyboxYard => 2,
            Self::AssemblyWorks => 3,
            Self::ProvingCell => 4,
        }
    }

    /// The map with wire tag `tag`.
    #[must_use]
    pub fn from_tag(tag: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|map| map.tag() == tag)
    }

    /// The players it is laid out for. Provisional until the arenas' layouts are decided
    /// (Atlas Q2.i46.5).
    #[must_use]
    pub const fn capacity(self) -> Capacity {
        match self {
            // The default 12 slots, one spawn point to spare.
            Self::Greybox => Capacity { min: 2, max: 12 },
            // Too big for a duel: 6 to the default 12, two spawn points to spare.
            Self::GreyboxYard => Capacity { min: 6, max: 12 },
            // Medium: from a 4-player skirmish to the default 12, two spawn points to spare.
            Self::AssemblyWorks => Capacity { min: 4, max: 12 },
            // Small: a duel to a 6-player skirmish. Its 12 spawn points still take the default
            // 12 slots (practice, a host that sets none), only crowded.
            Self::ProvingCell => Capacity { min: 2, max: 6 },
        }
    }

    /// The most slots a match on it may have: one per spawn point, so nobody starts a round on
    /// top of someone else.
    #[must_use]
    pub fn max_slots(self) -> u8 {
        u8::try_from(self.arena().spawn_points().len()).unwrap_or(u8::MAX)
    }

    /// Its arena: geometry, jump pads, spawn points.
    #[must_use]
    pub fn arena(self) -> Arena {
        match self {
            Self::Greybox => Arena::greybox(),
            Self::GreyboxYard => Arena::greybox_yard(),
            Self::AssemblyWorks => Arena::assembly_works(),
            Self::ProvingCell => Arena::proving_cell(),
        }
    }
}

impl fmt::Display for MapId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for MapId {
    type Err = UnknownMap;

    fn from_str(id: &str) -> Result<Self, UnknownMap> {
        Self::ALL
            .into_iter()
            .find(|map| map.as_str() == id)
            .ok_or_else(|| UnknownMap(id.to_owned()))
    }
}
