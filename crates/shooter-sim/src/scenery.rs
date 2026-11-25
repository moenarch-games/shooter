//! Partial arena destruction (shooter#60, Atlas D2.i46.11): the kit-built maps' destruction
//! candidates ([`crate::arena_kit::Placement::destructible`]: their cover, barriers, crates and
//! doors) are damageable placeables the host alone breaks. A weapon's ray, a pellet, the flame, a
//! blade swing or a rocket's blast that meets one takes its health down through the session's
//! ordinary hit queries ([`crate::session`]); at zero it breaks: its placement turns to the kit's
//! broken variant, and its pieces to that variant's collision proxies.
//!
//! - **Stable IDs.** A placeable's ID is its placement's index in [`Arena::placements`]: fixed
//!   by the map, the same on the host, every client and every replay.
//! - **Stable topology.** Only placements the map marks as candidates break, never floors,
//!   decks, platforms, ramps or walls; a candidate something else rests on (the lower crate of a
//!   stack) carries it and never breaks.
//! - **Stable bodies.** Every candidate keeps a run of body IDs, as many as the larger of its
//!   intact and broken pieces, right after the fixed pieces' (whose IDs are as
//!   [`Arena::statics`] gives them): breaking one changes no other body.
//! - **One authority.** Breaking lands at the end of the tick that took the last health
//!   ([`Session::settle_scenery`](crate::session::Session)): from the next tick on the physics
//!   world, the robots' articulated world, rays, projectiles, the flame, hooks and bots (their
//!   walks, [`Arena::clear_walk`]) all meet the broken pieces, and the tick's snapshot carries it
//!   ([`PlaceableState`]). Clients draw what the snapshot says; a predicting client adopts it.
//! - **Persistence.** A broken placeable stays broken through deaths and respawns until the
//!   round restarts, which restores every placeable whole.
//!
//! Health is provisional balance data ([`max_health`], Atlas Q2.i60.1).

use crate::arena::{Arena, FLOOR_BODY, Piece};
use crate::arena_kit::{Module, Placement, Variant};
use crate::physics::{BodyId, RigidBody, Vec3i};

/// A damageable placeable's health when whole, by module (provisional, Atlas Q2.i60.1): `None`
/// for modules that do not break. Low cover takes two direct rockets or thirteen rifle bolts,
/// a barrier three rail shots, a crate one close rocket, a door two rail shots and a few bolts.
#[must_use]
pub const fn max_health(module: Module) -> Option<u16> {
    match module {
        Module::CoverLow => Some(150),
        Module::Barrier => Some(250),
        Module::Crate => Some(60),
        Module::Door => Some(200),
        _ => None,
    }
}

/// A placeable's health as the snapshot carries it: only those that have taken damage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlaceableState {
    /// Its stable ID: its placement's index in [`Arena::placements`].
    pub id: u16,
    /// What it has left; 0 is broken.
    pub health: u16,
}

/// One destruction candidate of the map.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Placeable {
    /// Its stable ID: its placement's index in [`Arena::placements`].
    pub id: u16,
    pub module: Module,
    /// Its health whole; `None` when it never breaks (something rests on it, or its module has
    /// no health, [`max_health`]).
    pub max: Option<u16>,
    /// What it has left (its whole health, or 0 when it never breaks).
    pub health: u16,
    /// Its first body's ID, and how many it keeps.
    first_body: u64,
    bodies: u64,
}

impl Placeable {
    /// Whether it is broken.
    #[must_use]
    pub const fn broken(&self) -> bool {
        self.max.is_some() && self.health == 0
    }

    /// Whether it still takes damage: it breaks and has not yet.
    #[must_use]
    pub const fn damageable(&self) -> bool {
        self.max.is_some() && self.health > 0
    }

    /// Its look: broken or intact.
    #[must_use]
    pub const fn variant(&self) -> Variant {
        if self.broken() {
            Variant::Broken
        } else {
            Variant::Intact
        }
    }
}

/// The arena's damageable placeables and their health.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Scenery {
    /// How many of [`Arena::pieces`] are fixed (fused) pieces; the candidates' follow.
    fixed: usize,
    placeables: Vec<Placeable>,
    /// Indices into `placeables` whose look changed since [`Scenery::settle`].
    changed: Vec<usize>,
}

/// Whether `top` stands on `bottom`: its placement box starts where `bottom`'s ends and they
/// overlap seen from above.
fn rests_on(top: &Placement, bottom: &Placement) -> bool {
    let [top_min, top_max] = top.bounds();
    let [min, max] = bottom.bounds();
    top_min.y == max.y
        && top_min.x < max.x
        && min.x < top_max.x
        && top_min.z < max.z
        && min.z < top_max.z
}

impl Scenery {
    /// The destruction candidates of `arena` (whole), in placement order. An arena whose pieces
    /// are not derived from its placements ([`crate::arena_kit::pieces`]: the greybox and
    /// scripted arenas) has none.
    #[must_use]
    pub fn of(arena: &Arena) -> Self {
        let candidates: Vec<(usize, &Placement)> = arena
            .placements
            .iter()
            .enumerate()
            .filter(|(_, placement)| placement.destructible)
            .collect();
        let intact: Vec<Piece> = candidates
            .iter()
            .flat_map(|(_, placement)| placement.pieces())
            .collect();
        let Some(fixed) = arena.pieces.len().checked_sub(intact.len()) else {
            return Self::whole(arena);
        };
        if candidates.is_empty() || arena.pieces[fixed..] != intact[..] {
            return Self::whole(arena);
        }
        let mut first_body = FLOOR_BODY.0 + 1 + fixed as u64;
        let placeables = candidates
            .iter()
            .map(|&(index, placement)| {
                let carries = arena
                    .placements
                    .iter()
                    .enumerate()
                    .any(|(other, top)| other != index && rests_on(top, placement));
                let max = max_health(placement.module).filter(|_| !carries);
                let bodies = placement
                    .pieces()
                    .len()
                    .max(placement.broken().pieces().len()) as u64;
                let placeable = Placeable {
                    id: u16::try_from(index).expect("a map has fewer than 65 536 placements"),
                    module: placement.module,
                    max,
                    health: max.unwrap_or(0),
                    first_body,
                    bodies,
                };
                first_body += bodies;
                placeable
            })
            .collect();
        Self {
            fixed,
            placeables,
            changed: Vec::new(),
        }
    }

    /// No placeables: every piece is fixed.
    fn whole(arena: &Arena) -> Self {
        Self {
            fixed: arena.pieces.len(),
            ..Self::default()
        }
    }

    /// The candidates, in placement order.
    #[must_use]
    pub fn placeables(&self) -> &[Placeable] {
        &self.placeables
    }

    /// The candidate with stable ID `id`.
    #[must_use]
    pub fn placeable(&self, id: u16) -> Option<&Placeable> {
        self.placeables.iter().find(|placeable| placeable.id == id)
    }

    /// The highest body ID the arena may take (the floor, the fixed pieces and every
    /// candidate's run).
    #[must_use]
    pub fn last_body(&self) -> u64 {
        self.placeables
            .last()
            .map_or(FLOOR_BODY.0 + self.fixed as u64, |placeable| {
                placeable.first_body + placeable.bodies - 1
            })
    }

    /// The damageable placeable whose piece `body` is, if any.
    #[must_use]
    pub fn of_body(&self, body: BodyId) -> Option<usize> {
        self.placeables.iter().position(|placeable| {
            placeable.damageable()
                && (placeable.first_body..placeable.first_body + placeable.bodies).contains(&body.0)
        })
    }

    /// `damage` to placeable `index` (none to one that is broken or never breaks): true when
    /// this breaks it. Its pieces change at [`Scenery::settle`].
    pub fn hurt(&mut self, index: usize, damage: i32) -> bool {
        let Some(placeable) = self.placeables.get_mut(index) else {
            return false;
        };
        if !placeable.damageable() || damage <= 0 {
            return false;
        }
        let damage = u16::try_from(damage).unwrap_or(u16::MAX);
        placeable.health = placeable.health.saturating_sub(damage);
        if placeable.health == 0 {
            self.changed.push(index);
            return true;
        }
        false
    }

    /// Every placeable whole again (a new round).
    pub fn restore(&mut self) {
        for (index, placeable) in self.placeables.iter_mut().enumerate() {
            let Some(max) = placeable.max else { continue };
            if placeable.health == 0 {
                self.changed.push(index);
            }
            placeable.health = max;
        }
    }

    /// The healths of the damaged placeables, by ID: what a snapshot carries.
    #[must_use]
    pub fn state(&self) -> Vec<PlaceableState> {
        self.placeables
            .iter()
            .filter(|placeable| placeable.max.is_some_and(|max| placeable.health < max))
            .map(|placeable| PlaceableState {
                id: placeable.id,
                health: placeable.health,
            })
            .collect()
    }

    /// Takes the host's word ([`Scenery::state`]): every placeable it names at that health,
    /// every other one whole.
    ///
    /// # Errors
    ///
    /// If it names a placeable that does not break, or more health than it has whole; nothing
    /// changes then.
    pub fn adopt(&mut self, state: &[PlaceableState]) -> Result<(), &'static str> {
        let mut health: Vec<Option<u16>> = self.placeables.iter().map(|p| p.max).collect();
        for named in state {
            let index = self
                .placeables
                .iter()
                .position(|placeable| placeable.id == named.id)
                .ok_or("a placeable the map does not have")?;
            let max = self.placeables[index]
                .max
                .ok_or("a placeable that does not break")?;
            if named.health > max {
                return Err("a placeable above its health");
            }
            health[index] = Some(named.health);
        }
        for (index, (placeable, health)) in self.placeables.iter_mut().zip(health).enumerate() {
            let Some(health) = health else { continue };
            if (placeable.health == 0) != (health == 0) {
                self.changed.push(index);
            }
            placeable.health = health;
        }
        Ok(())
    }

    /// Brings `arena` (the session's own) to the placeables' looks: each changed one's
    /// placement in its variant and the candidates' pieces after the fixed ones. Returns the
    /// changed placeables (none: nothing to do).
    pub fn settle(&mut self, arena: &mut Arena) -> Vec<usize> {
        let mut changed = std::mem::take(&mut self.changed);
        changed.sort_unstable();
        changed.dedup();
        changed.retain(|&index| {
            let placeable = &self.placeables[index];
            let placement = &mut arena.placements[usize::from(placeable.id)];
            let variant = placeable.variant();
            let differs = placement.variant != variant;
            placement.variant = variant;
            differs
        });
        if !changed.is_empty() {
            arena.pieces.truncate(self.fixed);
            for placeable in &self.placeables {
                arena
                    .pieces
                    .extend(arena.placements[usize::from(placeable.id)].pieces());
            }
        }
        changed
    }

    /// Placeable `index`'s bodies as `arena` stands (after [`Scenery::settle`]).
    #[must_use]
    pub fn bodies(&self, arena: &Arena, index: usize) -> Vec<RigidBody> {
        let placeable = &self.placeables[index];
        arena.placements[usize::from(placeable.id)]
            .pieces()
            .into_iter()
            .zip(placeable.first_body..)
            .map(|(piece, id)| body(BodyId(id), piece))
            .collect()
    }

    /// Every body ID placeable `index` may take.
    pub fn body_ids(&self, index: usize) -> impl Iterator<Item = BodyId> {
        let placeable = &self.placeables[index];
        (placeable.first_body..placeable.first_body + placeable.bodies).map(BodyId)
    }

    /// The floor, the fixed pieces and every candidate's pieces as `arena` stands, as static
    /// bodies in body-ID order: the physics world's and the robots' statics.
    #[must_use]
    pub fn statics(&self, arena: &Arena) -> Vec<RigidBody> {
        let mut statics = std::iter::once(RigidBody::floor(FLOOR_BODY, 0))
            .chain(
                arena.pieces[..self.fixed]
                    .iter()
                    .zip(FLOOR_BODY.0 + 1..)
                    .map(|(piece, id)| body(BodyId(id), *piece)),
            )
            .collect::<Vec<_>>();
        for index in 0..self.placeables.len() {
            statics.extend(self.bodies(arena, index));
        }
        statics
    }

    /// The point of placeable `index`'s pieces (as `arena` stands) nearest `point` (units):
    /// where a blast at `point` meets it.
    #[must_use]
    pub fn nearest(&self, arena: &Arena, index: usize, point: [f32; 3]) -> Option<[f32; 3]> {
        let placeable = &self.placeables[index];
        #[allow(clippy::cast_precision_loss)]
        arena.placements[usize::from(placeable.id)]
            .pieces()
            .iter()
            .map(|piece| {
                let (center, half) = (piece.center(), piece.half());
                let c = [center.x, center.y, center.z].map(|v| v as f32);
                let h = [half.x, half.y, half.z].map(|v| v as f32);
                [0, 1, 2].map(|axis| point[axis].clamp(c[axis] - h[axis], c[axis] + h[axis]))
            })
            .min_by(|a, b| {
                let d = |p: &[f32; 3]| {
                    (0..3)
                        .map(|i| (p[i] - point[i]) * (p[i] - point[i]))
                        .sum::<f32>()
                };
                d(a).total_cmp(&d(b))
            })
    }

    /// Whether `point` (units) lies in placeable `index`'s placement box: a hook set there lets
    /// go when it breaks.
    #[must_use]
    pub fn holds(&self, arena: &Arena, index: usize, point: [f32; 3]) -> bool {
        let [min, max] = arena.placements[usize::from(self.placeables[index].id)].bounds();
        let to = |v: Vec3i| [v.x, v.y, v.z];
        let (min, max) = (to(min), to(max));
        #[allow(clippy::cast_precision_loss)]
        (0..3).all(|axis| (min[axis] as f32 - 1.0..=max[axis] as f32 + 1.0).contains(&point[axis]))
    }
}

fn body(id: BodyId, piece: Piece) -> RigidBody {
    match piece {
        Piece::Block { center, half } => RigidBody::fixed(id, center, half),
        Piece::Ramp {
            center,
            half,
            rises,
        } => RigidBody::ramp(id, center, half, rises),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena_kit::{Turn, pieces};

    fn arena(placements: Vec<Placement>) -> Arena {
        let mut arena = Arena::greybox();
        arena.pieces = pieces(&placements);
        arena.placements = placements;
        arena
    }

    #[test]
    fn candidates_keep_stable_ids_and_body_runs_and_stacks_carry() {
        let arena = arena(vec![
            Placement::new(Module::Wall, [0, 0, -800], Turn::None),
            Placement::new(Module::Crate, [0, 0, 0], Turn::None).destructible(),
            Placement::new(Module::Crate, [0, 80, 0], Turn::None).destructible(),
            Placement::new(Module::CoverLow, [400, 0, 0], Turn::None).destructible(),
        ]);
        let scenery = Scenery::of(&arena);
        let ids: Vec<(u16, Option<u16>)> =
            scenery.placeables().iter().map(|p| (p.id, p.max)).collect();
        // The lower crate carries the upper one: it never breaks.
        assert_eq!(ids, [(1, None), (2, Some(60)), (3, Some(150))]);
        // The wall (2), then a run per candidate: a crate 1, the cover 2 (broken: stub and base).
        let statics = scenery.statics(&arena);
        let ids: Vec<u64> = statics.iter().map(|body| body.id().0).collect();
        assert_eq!(ids, [1, 2, 3, 4, 5]);
        assert_eq!(scenery.last_body(), 6);
        assert_eq!(scenery.of_body(BodyId(3)), None);
        assert_eq!(scenery.of_body(BodyId(5)), Some(2));
    }

    #[test]
    fn breaking_swaps_pieces_and_restoring_brings_them_back() {
        let mut arena = arena(vec![
            Placement::new(Module::CoverLow, [0, 0, 0], Turn::None).destructible(),
            Placement::new(Module::Barrier, [400, 0, 0], Turn::Quarter).destructible(),
        ]);
        let whole = arena.clone();
        let mut scenery = Scenery::of(&arena);
        assert!(!scenery.hurt(1, 249));
        assert!(scenery.settle(&mut arena).is_empty());
        assert!(scenery.hurt(1, 12));
        assert!(!scenery.hurt(1, 12), "broken: no more damage");
        assert_eq!(scenery.settle(&mut arena), [1]);
        assert_eq!(arena.placements[1].variant, Variant::Broken);
        assert_eq!(arena.pieces.len(), 3);
        assert_eq!(scenery.state(), [PlaceableState { id: 1, health: 0 }]);
        // The cover keeps 2 and 3 (one used), the barrier's stub and base take 4 and 5.
        let ids: Vec<u64> = scenery.statics(&arena).iter().map(|b| b.id().0).collect();
        assert_eq!(ids, [1, 2, 4, 5]);
        let mut client = Scenery::of(&whole);
        let mut drawn = whole.clone();
        client.adopt(&scenery.state()).unwrap();
        client.settle(&mut drawn);
        assert_eq!(drawn, arena);
        assert!(
            client
                .adopt(&[PlaceableState { id: 7, health: 0 }])
                .is_err()
        );
        assert!(
            client
                .adopt(&[PlaceableState { id: 0, health: 151 }])
                .is_err()
        );
        scenery.restore();
        assert_eq!(scenery.settle(&mut arena), [1]);
        assert_eq!(arena, whole);
        assert!(scenery.state().is_empty());
    }
}
