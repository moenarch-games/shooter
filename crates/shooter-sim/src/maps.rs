//! The map catalog: every arena a match can be played on, by a stable [`MapId`].
//!
//! The host picks a map for a match; the clients learn its ID from the snapshots
//! ([`crate::wire`]) and build the same [`Arena`] from this catalog, so the native client, the
//! browser client and the host share one definition of its geometry (collision and drawing),
//! spawn points, jump pads and walkable traversal. Replays carry the ID too. Authored art for
//! a map comes from asset-tooling (moritzbrantner/asset-tooling#593); its layout, placement and
//! collision data live here.
//!
//! A map recommends a player-capacity range ([`Capacity`]); it never sets a match's slot count,
//! which stays with [`crate::Rules`].

use crate::arena::Arena;
use std::fmt;
use std::str::FromStr;

/// A catalogued map. Its string ([`MapId::as_str`]) and wire tag ([`MapId::tag`]) are stable:
/// a new map takes new ones, and neither is ever reused.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum MapId {
    /// The first greybox hall ([`Arena::greybox`]), kept as the test and A/B fixture.
    Greybox,
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
    pub const ALL: [Self; 1] = [Self::Greybox];
    /// The map a match is played on when none is chosen.
    pub const DEFAULT: Self = Self::Greybox;

    /// The stable ID: command lines, the host's configuration, page queries and replays.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Greybox => "greybox",
        }
    }

    /// The stable wire tag in snapshots (0 is no catalogued map).
    #[must_use]
    pub const fn tag(self) -> u8 {
        match self {
            Self::Greybox => 1,
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
            // Up to one fighter per spawn point.
            Self::Greybox => Capacity { min: 2, max: 8 },
        }
    }

    /// Its arena: geometry, jump pads, spawn points.
    #[must_use]
    pub fn arena(self) -> Arena {
        match self {
            Self::Greybox => Arena::greybox(),
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
