//! Arenas built from asset-tooling's arena kit (release `shooter-arena`, pinned in
//! `assets/arena-kit.pin.json`; Atlas D5.i46.1): a map lists kit module [`Placement`]s (a module,
//! where its origin sits, a quarter-turn yaw and its intact or broken variant), and the
//! simulation derives its authoritative [`Piece`]s from the modules' collision proxies.
//!
//! The proxies come from the kit's descriptor through a checked-in table ([`modules`], written by
//! `scripts/gen-arena-kit.py`; `shooter-client`'s `arena` test checks it against the pinned kit),
//! so the simulation reads no files and stays deterministic. The clients draw each placement's
//! kit mesh at the same place, so what a fighter sees is what it collides with.
//!
//! Frames: the kit's are the arena's (+Y up, a module's front +Z), in centimetres. A placement's
//! yaw turns counter-clockwise seen from above, +90° turning +Z to +X (the same sense as
//! `crate::trig`'s yaw). Proxies at or below the floor (the floor plates) are the arena's floor
//! already and add no piece; the jump pad's trigger is a [`crate::arena::JumpPad`] the map
//! authors with its launch, not a piece.
//!
//! Neighbouring solid pieces that meet face to face over a whole face become one piece
//! ([`fuse`]): a wall of fifteen modules is one box to collide with, with no seams for a sliding
//! fighter to catch on, and a map stays inside the body IDs arena geometry may take. Placements
//! marked [`Placement::destructible`] keep pieces of their own (shooter#60 swaps them).

pub mod modules;

pub use modules::Module;

use crate::arena::Piece;
use crate::physics::{RampRise, Vec3i};

/// What a collision proxy is to Shooter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Shape {
    /// A box with a top to stand on: a [`Piece::Block`].
    Walkable,
    /// A solid box: a [`Piece::Block`].
    Blocked,
    /// A wedge filling the lower half of its box, rising toward this side: a [`Piece::Ramp`].
    Ramp(RampRise),
    /// A jump pad's footprint: no collider.
    Trigger,
}

/// One axis-aligned collision proxy in its module's frame (centimetres).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Proxy {
    pub name: &'static str,
    pub center: [i32; 3],
    pub half: [i32; 3],
    pub shape: Shape,
}

/// A placement's yaw in quarter turns, counter-clockwise seen from above (+Z toward +X).
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Turn {
    #[default]
    None,
    /// 90°: +Z turns to +X.
    Quarter,
    /// 180°.
    Half,
    /// 270°: +Z turns to -X.
    ThreeQuarter,
}

impl Turn {
    /// Every turn, in order.
    pub const ALL: [Self; 4] = [Self::None, Self::Quarter, Self::Half, Self::ThreeQuarter];

    /// The yaw in degrees (the kit's `yawDegrees`).
    #[must_use]
    pub const fn degrees(self) -> u16 {
        match self {
            Self::None => 0,
            Self::Quarter => 90,
            Self::Half => 180,
            Self::ThreeQuarter => 270,
        }
    }

    /// The same yaw in `crate::trig` steps (0 faces +Z, a quarter turn is 16384).
    #[must_use]
    pub const fn yaw(self) -> u16 {
        match self {
            Self::None => 0,
            Self::Quarter => 16_384,
            Self::Half => 32_768,
            Self::ThreeQuarter => 49_152,
        }
    }

    /// This turn followed by `other`.
    #[must_use]
    pub const fn then(self, other: Self) -> Self {
        Self::ALL[(self as usize + other as usize) % 4]
    }

    /// `[x, z]` turned.
    #[must_use]
    pub const fn apply(self, [x, z]: [i32; 2]) -> [i32; 2] {
        match self {
            Self::None => [x, z],
            Self::Quarter => [z, -x],
            Self::Half => [-x, -z],
            Self::ThreeQuarter => [-z, x],
        }
    }

    /// Half extents `[x, z]` turned: swapped at a quarter and three quarters.
    #[must_use]
    pub const fn extents(self, [x, z]: [i32; 2]) -> [i32; 2] {
        match self {
            Self::None | Self::Half => [x, z],
            Self::Quarter | Self::ThreeQuarter => [z, x],
        }
    }

    /// The side a ramp rises toward, turned.
    #[must_use]
    pub const fn rise(self, rises: RampRise) -> RampRise {
        let [x, z] = match rises {
            RampRise::PosX => [1, 0],
            RampRise::NegX => [-1, 0],
            RampRise::PosZ => [0, 1],
            RampRise::NegZ => [0, -1],
        };
        match self.apply([x, z]) {
            [1, _] => RampRise::PosX,
            [-1, _] => RampRise::NegX,
            [_, 1] => RampRise::PosZ,
            _ => RampRise::NegZ,
        }
    }
}

/// A module's look: intact, or its kit's broken variant (same key, box and placement).
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum Variant {
    #[default]
    Intact,
    Broken,
}

impl Variant {
    /// The kit's bundle variant name.
    #[must_use]
    pub const fn bundle(self) -> &'static str {
        match self {
            Self::Intact => "default",
            Self::Broken => "broken",
        }
    }
}

/// One kit module in a map.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Placement {
    pub module: Module,
    /// Where its origin sits (centimetres).
    pub at: Vec3i,
    pub turn: Turn,
    pub variant: Variant,
    /// A candidate for shooter#60's destruction (a module with a broken variant): its pieces are
    /// its own, never fused with a neighbour's. Nothing breaks it yet.
    pub destructible: bool,
}

impl Placement {
    /// `module` intact at `at`, turned by `turn`.
    #[must_use]
    pub const fn new(module: Module, at: [i32; 3], turn: Turn) -> Self {
        Self {
            module,
            at: Vec3i::new(at[0], at[1], at[2]),
            turn,
            variant: Variant::Intact,
            destructible: false,
        }
    }

    /// The same placement marked as a destruction candidate (shooter#60).
    ///
    /// # Panics
    ///
    /// If the kit has no broken variant of its module (a const-evaluated map fails to build).
    #[must_use]
    pub const fn destructible(self) -> Self {
        assert!(
            self.module.broken_proxies().is_some(),
            "only modules with a broken variant are destructible"
        );
        Self {
            destructible: true,
            ..self
        }
    }

    /// The same placement in its broken variant.
    #[must_use]
    pub const fn broken(self) -> Self {
        Self {
            variant: Variant::Broken,
            ..self
        }
    }

    /// The same placement turned half a turn round the arena's centre.
    #[must_use]
    pub const fn turned_half(self) -> Self {
        Self {
            at: Vec3i::new(-self.at.x, self.at.y, -self.at.z),
            turn: self.turn.then(Turn::Half),
            ..self
        }
    }

    /// Its proxies in its variant (the intact ones where the kit has no broken variant).
    #[must_use]
    pub const fn proxies(&self) -> &'static [Proxy] {
        match (self.variant, self.module.broken_proxies()) {
            (Variant::Broken, Some(broken)) => broken,
            _ => self.module.proxies(),
        }
    }

    /// The arena pieces of its colliding proxies, in the kit's order: none for the floor plates
    /// (wholly at or below the floor) and the jump pad's trigger.
    #[must_use]
    pub fn pieces(&self) -> Vec<Piece> {
        self.proxies()
            .iter()
            .filter(|proxy| {
                proxy.shape != Shape::Trigger && self.at.y + proxy.center[1] + proxy.half[1] > 0
            })
            .map(|proxy| {
                let [x, z] = self.turn.apply([proxy.center[0], proxy.center[2]]);
                let center = Vec3i::new(self.at.x + x, self.at.y + proxy.center[1], self.at.z + z);
                let [hx, hz] = self.turn.extents([proxy.half[0], proxy.half[2]]);
                let half = Vec3i::new(hx, proxy.half[1], hz);
                match proxy.shape {
                    Shape::Ramp(rises) => Piece::Ramp {
                        center,
                        half,
                        rises: self.turn.rise(rises),
                    },
                    _ => Piece::Block { center, half },
                }
            })
            .collect()
    }

    /// Its placement box in the arena: `[min, max]` corners (centimetres).
    #[must_use]
    pub const fn bounds(&self) -> [Vec3i; 2] {
        let [min, max] = self.module.placement_box();
        let [ax, az] = self.turn.apply([min[0], min[2]]);
        let [bx, bz] = self.turn.apply([max[0], max[2]]);
        let (x0, x1) = if ax < bx { (ax, bx) } else { (bx, ax) };
        let (z0, z1) = if az < bz { (az, bz) } else { (bz, az) };
        [
            Vec3i::new(self.at.x + x0, self.at.y + min[1], self.at.z + z0),
            Vec3i::new(self.at.x + x1, self.at.y + max[1], self.at.z + z1),
        ]
    }
}

/// Whether `a` and `b` (centre, half) agree on the two axes other than `along` and meet face to
/// face on it.
fn joins(a: (Vec3i, Vec3i), b: (Vec3i, Vec3i), along: usize) -> bool {
    let axis = |v: Vec3i| [v.x, v.y, v.z];
    let (ac, ah, bc, bh) = (axis(a.0), axis(a.1), axis(b.0), axis(b.1));
    (0..3).all(|i| i == along || (ac[i] == bc[i] && ah[i] == bh[i]))
        && (ac[along] - bc[along]).abs() == ah[along] + bh[along]
}

/// The box of `a` and `b` joined along `along` (they [`joins`]).
fn joined(a: (Vec3i, Vec3i), b: (Vec3i, Vec3i), along: usize) -> (Vec3i, Vec3i) {
    let get = |v: Vec3i| [v.x, v.y, v.z];
    let (mut center, mut half) = (get(a.0), get(a.1));
    let low = (get(a.0)[along] - get(a.1)[along]).min(get(b.0)[along] - get(b.1)[along]);
    let high = (get(a.0)[along] + get(a.1)[along]).max(get(b.0)[along] + get(b.1)[along]);
    center[along] = (low + high) / 2;
    half[along] = (high - low) / 2;
    (
        Vec3i::new(center[0], center[1], center[2]),
        Vec3i::new(half[0], half[1], half[2]),
    )
}

/// One fusion of `a` and `b`, if they make one piece: two boxes meeting over a whole face on any
/// axis, or two ramps rising the same way side by side across their rise (or end to end along
/// it is never fused: that would change the slope).
fn fusion(a: &Piece, b: &Piece) -> Option<Piece> {
    match (*a, *b) {
        (
            Piece::Block {
                center: ac,
                half: ah,
            },
            Piece::Block {
                center: bc,
                half: bh,
            },
        ) => (0..3).find_map(|along| {
            let ends = |c: Vec3i, h: Vec3i| {
                let (c, h) = ([c.x, c.y, c.z][along], [h.x, h.y, h.z][along]);
                (c - h, c + h)
            };
            // A joined box keeps whole-centimetre ends; its centre must be whole too.
            let (alo, ahi) = ends(ac, ah);
            let (blo, bhi) = ends(bc, bh);
            let even = (alo.min(blo) + ahi.max(bhi)) % 2 == 0;
            (even && joins((ac, ah), (bc, bh), along)).then(|| {
                let (center, half) = joined((ac, ah), (bc, bh), along);
                Piece::Block { center, half }
            })
        }),
        (
            Piece::Ramp {
                center: ac,
                half: ah,
                rises: ar,
            },
            Piece::Ramp {
                center: bc,
                half: bh,
                rises: br,
            },
        ) if ar == br => {
            let across = match ar {
                RampRise::PosX | RampRise::NegX => 2,
                RampRise::PosZ | RampRise::NegZ => 0,
            };
            let lo = ([ac.x, ac.y, ac.z][across] - [ah.x, ah.y, ah.z][across])
                .min([bc.x, bc.y, bc.z][across] - [bh.x, bh.y, bh.z][across]);
            let hi = ([ac.x, ac.y, ac.z][across] + [ah.x, ah.y, ah.z][across])
                .max([bc.x, bc.y, bc.z][across] + [bh.x, bh.y, bh.z][across]);
            ((lo + hi) % 2 == 0 && joins((ac, ah), (bc, bh), across)).then(|| {
                let (center, half) = joined((ac, ah), (bc, bh), across);
                Piece::Ramp {
                    center,
                    half,
                    rises: ar,
                }
            })
        }
        _ => None,
    }
}

/// `pieces` with every pair that makes one piece ([`fusion`]) fused, until none does: each
/// fusion keeps the earlier piece's index and drops the later one, so the result depends only on
/// the order of `pieces`.
#[must_use]
pub fn fuse(mut pieces: Vec<Piece>) -> Vec<Piece> {
    loop {
        let mut fused = false;
        let mut index = 0;
        while index < pieces.len() {
            let mut other = index + 1;
            while other < pieces.len() {
                if let Some(piece) = fusion(&pieces[index], &pieces[other]) {
                    pieces[index] = piece;
                    pieces.remove(other);
                    fused = true;
                    other = index + 1;
                } else {
                    other += 1;
                }
            }
            index += 1;
        }
        if !fused {
            return pieces;
        }
    }
}

/// The arena pieces of `placements`: every fixed placement's pieces fused ([`fuse`]), then each
/// destructible placement's own pieces in placement order.
#[must_use]
pub fn pieces(placements: &[Placement]) -> Vec<Piece> {
    let fixed = placements
        .iter()
        .filter(|placement| !placement.destructible)
        .flat_map(Placement::pieces)
        .collect();
    let mut pieces = fuse(fixed);
    pieces.extend(
        placements
            .iter()
            .filter(|placement| placement.destructible)
            .flat_map(Placement::pieces),
    );
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(center: [i32; 3], half: [i32; 3]) -> Piece {
        Piece::Block {
            center: Vec3i::new(center[0], center[1], center[2]),
            half: Vec3i::new(half[0], half[1], half[2]),
        }
    }

    /// The kit's documented turns: `-Z` rises `NegZ` at 0°, `NegX` at 90°, `PosZ` at 180° and
    /// `PosX` at 270°; a wall's 2 m runs along X unturned and along Z at a quarter turn.
    #[test]
    fn turns_follow_the_kits_convention() {
        let rises: Vec<RampRise> = Turn::ALL
            .iter()
            .map(|turn| turn.rise(RampRise::NegZ))
            .collect();
        assert_eq!(
            rises,
            [
                RampRise::NegZ,
                RampRise::NegX,
                RampRise::PosZ,
                RampRise::PosX
            ]
        );
        assert_eq!(Turn::Quarter.apply([0, 100]), [100, 0]);
        let wall = |turn| Placement::new(Module::Wall, [500, 0, 700], turn).pieces();
        assert_eq!(
            wall(Turn::None),
            vec![block([500, 200, 700], [100, 200, 30])]
        );
        assert_eq!(
            wall(Turn::Quarter),
            vec![block([500, 200, 700], [30, 200, 100])]
        );
        // The doorway's left jamb (-x) is at +z after a quarter turn.
        let doorway = Placement::new(Module::Doorway, [0, 0, 0], Turn::Quarter).pieces();
        assert_eq!(doorway[0], block([0, 200, 150], [30, 200, 50]));
        assert_eq!(Turn::ThreeQuarter.then(Turn::Half), Turn::Quarter);
    }

    /// The kit's contract test rebuilds the greybox yard's 16 m wall line from eight tall wall
    /// modules and its 12 m ramp from one tall ramp: so do these.
    #[test]
    fn modules_rebuild_the_yards_pieces() {
        let line: Vec<Placement> = (0..8)
            .map(|i| Placement::new(Module::WallTall, [-2_300 + 200 * i, 0, -1_200], Turn::None))
            .collect();
        assert_eq!(
            pieces(&line),
            vec![block([-1_600, 250, -1_200], [800, 250, 30])]
        );
        let ramp = Placement::new(Module::RampTall, [-2_200, 0, 0], Turn::ThreeQuarter);
        assert_eq!(
            ramp.pieces(),
            vec![Piece::Ramp {
                center: Vec3i::new(-2_200, 150, 0),
                half: Vec3i::new(600, 150, 100),
                rises: RampRise::PosX,
            }]
        );
    }

    #[test]
    fn floors_and_pads_add_no_piece_and_destructibles_keep_theirs() {
        assert!(
            Placement::new(Module::Floor, [0, 0, 0], Turn::None)
                .pieces()
                .is_empty()
        );
        assert!(
            Placement::new(Module::JumpPad, [0, 0, 0], Turn::None)
                .pieces()
                .is_empty()
        );
        // A floor plate raised onto a deck collides.
        assert_eq!(
            Placement::new(Module::Floor, [0, 300, 0], Turn::None).pieces(),
            vec![block([0, 295, 0], [100, 5, 100])]
        );
        let cover = [
            Placement::new(Module::CoverLow, [0, 0, 0], Turn::None).destructible(),
            Placement::new(Module::CoverLow, [200, 0, 0], Turn::None).destructible(),
        ];
        assert_eq!(pieces(&cover).len(), 2);
        let broken = Placement::new(Module::CoverLow, [0, 0, 0], Turn::None).broken();
        assert_eq!(broken.pieces().len(), 2);
    }

    #[test]
    fn side_by_side_ramps_fuse_but_end_to_end_ones_do_not() {
        let side = [
            Placement::new(Module::Ramp, [0, 0, 0], Turn::None),
            Placement::new(Module::Ramp, [200, 0, 0], Turn::None),
        ];
        assert_eq!(
            pieces(&side),
            vec![Piece::Ramp {
                center: Vec3i::new(100, 100, 0),
                half: Vec3i::new(200, 100, 400),
                rises: RampRise::NegZ,
            }]
        );
        let ends = [
            Placement::new(Module::Ramp, [0, 0, 0], Turn::None),
            Placement::new(Module::Ramp, [0, 0, 800], Turn::None),
        ];
        assert_eq!(pieces(&ends).len(), 2);
    }

    #[test]
    fn stacked_platforms_fuse_into_one_tower() {
        let tower = [
            Placement::new(Module::Platform, [0, 0, 0], Turn::None),
            Placement::new(Module::Platform, [0, 200, 0], Turn::None),
            Placement::new(Module::PlatformHalf, [0, 400, 0], Turn::None),
            Placement::new(Module::Platform, [200, 0, 0], Turn::None),
        ];
        // The two lower blocks fuse upward first, then nothing else meets a whole face.
        let fused = pieces(&tower);
        assert!(
            fused.contains(&block([0, 250, 0], [100, 250, 100])),
            "{fused:?}"
        );
        assert!(
            fused.contains(&block([200, 100, 0], [100, 100, 100])),
            "{fused:?}"
        );
    }
}
