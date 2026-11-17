//! The robot builder (Atlas D2.i70.11, shooter#57): a blueprint workbench before the match and,
//! after a death, between lives. The person takes parts from the catalogue onto the sockets of
//! the robot on the bench, turns them, gives weapons their mods and removes parts, watching the
//! build points, parts, mass and energy and whether the build is valid; Ready sends it to the
//! match.
//!
//! The [`Workbench`] here is the editing model, free of drawing and input ([`view`] draws it and
//! turns the mouse and keys into its edits). It holds no rules of its own: every edit is a
//! candidate that the simulation's one validation path checks ([`shooter_sim::robot::Catalog::validate`]).
//! An edit that breaks a structural rule (an unknown socket, an overlap, a mod that does not fit,
//! a weapon twice, too many parts) is refused with that rule's reason, so the bench always holds
//! a build that can be laid out and drawn. Only the build points may run over while building:
//! the bench checks every rule but the build-point ceiling ([`BENCH`]), and the build's
//! validity is the beta catalog's verdict on it ([`Workbench::check`]), which the host repeats
//! on whatever arrives. Nothing warns about or forbids unconventional builds (no locomotion,
//! odd balance): only the objective rules reject one.
//!
//! A part's rotation is exact: snaps of an eighth of a turn, or fine steps of about a degree,
//! about each of the socket's axes ([`Axis`]), in the canonical form the wire carries. The bench
//! never turns a part into a conventional orientation by itself.

pub mod controls;
pub mod view;

use shooter_sim::Weapon;
use shooter_sim::robot::{
    Assembly, AssemblyError, BETA, Budget, Candidate, Catalog, Joint, Kind, Locomotion, MAX_PARTS,
    ModId, Orientation, PartId, PartType, PartTypeId, RawMount, RawPart, SocketId,
};
use std::fmt;

/// The beta catalog with no build-point ceiling: every structural rule, so the bench always holds
/// a build that lays out, while a build over budget is still being worked on.
pub const BENCH: Catalog = Catalog {
    build_points: u16::MAX,
    ..BETA
};

/// A snap: an eighth of a turn.
pub const SNAP: u16 = Orientation::QUARTER / 2;
/// A fine step: about a degree (182 of the 65 536 binary angles a turn).
pub const FINE: u16 = 182;

/// An axis a part turns about on its socket, in the order the orientation applies them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Axis {
    /// About the socket's outward normal (turns the footprint).
    Yaw,
    Pitch,
    Roll,
}

/// Why the bench refused an edit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Refusal {
    /// The edit breaks a build rule.
    Rule(AssemblyError),
    /// The root chassis stays: it is the robot (start a new frame instead).
    Root,
    /// Nothing is selected, or in hand, to act on.
    Nothing,
    /// The selected part takes no mods.
    NoMods,
    /// The control layout edit breaks a binding rule ([`controls`]).
    Binding(shooter_sim::bindings::BindingError),
    /// A key no binding may name.
    UnboundKey,
}

impl fmt::Display for Refusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rule(error) => error.fmt(formatter),
            Self::Root => formatter.write_str("the root chassis stays: start a new frame"),
            Self::Nothing => formatter.write_str("select a part first"),
            Self::NoMods => formatter.write_str("no mod fits this part"),
            Self::Binding(error) => error.fmt(formatter),
            Self::UnboundKey => formatter.write_str("that key cannot be bound"),
        }
    }
}

impl From<AssemblyError> for Refusal {
    fn from(error: AssemblyError) -> Self {
        Self::Rule(error)
    }
}

/// The bench: the build being worked on, the catalogue part in hand and the selected part.
#[derive(Clone, Debug)]
pub struct Workbench {
    draft: Assembly,
    /// The catalogue part in hand: the next socket clicked takes it.
    pub hand: Option<PartTypeId>,
    /// How the part in hand will be turned on its socket.
    pub turn: Orientation,
    pub selected: Option<PartId>,
}

impl Workbench {
    /// A bench holding `build`, nothing in hand or selected.
    #[must_use]
    pub const fn new(build: Assembly) -> Self {
        Self {
            draft: build,
            hand: None,
            turn: Orientation::IDENTITY,
            selected: None,
        }
    }

    /// The build on the bench (it lays out; it may be over budget).
    #[must_use]
    pub const fn draft(&self) -> &Assembly {
        &self.draft
    }

    /// The beta catalog's verdict on the build: the build to send, or the first rule it breaks.
    ///
    /// # Errors
    ///
    /// The rule the build breaks (on the bench, only the build points can).
    pub fn check(&self) -> Result<Assembly, AssemblyError> {
        BETA.validate(&self.draft.candidate())
    }

    /// The build's totals against the beta's limits.
    #[must_use]
    pub fn budget(&self) -> Budget {
        BETA.budget(&self.draft)
    }

    /// Starts over from `build` (a named loadout), keeping what is in hand.
    pub const fn load(&mut self, build: Assembly) {
        self.draft = build;
        self.selected = None;
    }

    /// Starts a new robot: `chassis` alone, as its root.
    ///
    /// # Errors
    ///
    /// If `chassis` is no chassis.
    pub fn new_frame(&mut self, chassis: PartTypeId) -> Result<(), Refusal> {
        let candidate = Candidate {
            parts: vec![RawPart {
                id: 0,
                kind: chassis.0,
                mount: None,
                modification: None,
            }],
        };
        self.draft = BENCH.validate(&candidate)?;
        self.selected = Some(PartId(0));
        Ok(())
    }

    /// The build with `kind` turned `turn` on `socket` of `parent`, as the bench would hold it,
    /// and the new part's ID: what a placement would make (the bench's preview).
    ///
    /// # Errors
    ///
    /// The rule the placement would break.
    pub fn placed(
        &self,
        kind: PartTypeId,
        parent: PartId,
        socket: SocketId,
        turn: Orientation,
    ) -> Result<(Assembly, PartId), AssemblyError> {
        let mut candidate = self.draft.candidate();
        let id = (0..=u8::MAX)
            .find(|id| candidate.parts.iter().all(|part| part.id != *id))
            .ok_or(AssemblyError::TooManyParts(candidate.parts.len() + 1))?;
        let (yaw, pitch, roll) = turn.raw();
        candidate.parts.push(RawPart {
            id,
            kind: kind.0,
            mount: Some(RawMount {
                parent: parent.0,
                socket: socket.0,
                yaw,
                pitch,
                roll,
            }),
            modification: None,
        });
        Ok((BENCH.validate(&candidate)?, PartId(id)))
    }

    /// Whether the part in hand, turned as it is, fits `socket` of `parent`.
    #[must_use]
    pub fn fits(&self, parent: PartId, socket: SocketId) -> bool {
        self.hand
            .is_some_and(|kind| self.placed(kind, parent, socket, self.turn).is_ok())
    }

    /// Puts the part in hand on `socket` of `parent` and selects it.
    ///
    /// # Errors
    ///
    /// If nothing is in hand, or the placement breaks a rule.
    pub fn place(&mut self, parent: PartId, socket: SocketId) -> Result<PartId, Refusal> {
        let kind = self.hand.ok_or(Refusal::Nothing)?;
        let (draft, id) = self.placed(kind, parent, socket, self.turn)?;
        self.draft = draft;
        self.selected = Some(id);
        Ok(id)
    }

    /// Removes the selected part and everything mounted on it.
    ///
    /// # Errors
    ///
    /// If nothing is selected or it is the root chassis.
    pub fn remove(&mut self) -> Result<(), Refusal> {
        let id = self.selected.ok_or(Refusal::Nothing)?;
        let part = self.draft.part(id).ok_or(Refusal::Nothing)?;
        if part.mount.is_none() {
            return Err(Refusal::Root);
        }
        let gone = subtree(&self.draft, id);
        let mut candidate = self.draft.candidate();
        candidate
            .parts
            .retain(|part| !gone.contains(&PartId(part.id)));
        self.draft = BENCH.validate(&candidate)?;
        self.selected = None;
        Ok(())
    }

    /// Turns the selected part `steps` binary angles about `axis` on its socket (negative turns
    /// back), or the part in hand when nothing is selected.
    ///
    /// # Errors
    ///
    /// If nothing is selected or in hand, the selected part is the root, or the turn breaks a
    /// rule (its footprint would overlap another's): it stays as it was.
    pub fn rotate(&mut self, axis: Axis, steps: i32) -> Result<(), Refusal> {
        let Some(id) = self.selected else {
            if self.hand.is_none() {
                return Err(Refusal::Nothing);
            }
            self.turn = turned(self.turn, axis, steps);
            return Ok(());
        };
        let mut candidate = self.draft.candidate();
        let raw = candidate
            .parts
            .iter_mut()
            .find(|part| part.id == id.0)
            .ok_or(Refusal::Nothing)?;
        let mount = raw.mount.as_mut().ok_or(Refusal::Root)?;
        let now = Orientation::canonical(mount.yaw, mount.pitch, mount.roll);
        (mount.yaw, mount.pitch, mount.roll) = turned(now, axis, steps).raw();
        self.draft = BENCH.validate(&candidate)?;
        Ok(())
    }

    /// Gives the selected weapon the next mod that fits it (in catalog order, then none again),
    /// skipping one that would break a rule (a heavy mod with no room).
    ///
    /// # Errors
    ///
    /// If nothing is selected, it is no weapon, or no mod fits it.
    pub fn cycle_mod(&mut self) -> Result<Option<ModId>, Refusal> {
        let id = self.selected.ok_or(Refusal::Nothing)?;
        let part = *self.draft.part(id).ok_or(Refusal::Nothing)?;
        let weapon = BETA.weapon(&part).ok_or(Refusal::NoMods)?;
        let mut choices: Vec<Option<ModId>> = std::iter::once(None)
            .chain(
                BETA.mods
                    .iter()
                    .filter(|modification| modification.fits.contains(&weapon))
                    .map(|modification| Some(modification.id)),
            )
            .collect();
        if choices.len() == 1 {
            return Err(Refusal::NoMods);
        }
        let now = choices
            .iter()
            .position(|choice| *choice == part.modification)
            .unwrap_or(0);
        choices.rotate_left(now + 1);
        for choice in choices {
            let mut candidate = self.draft.candidate();
            if let Some(raw) = candidate.parts.iter_mut().find(|raw| raw.id == id.0) {
                raw.modification = choice.map(|id| id.0);
            }
            if let Ok(draft) = BENCH.validate(&candidate) {
                self.draft = draft;
                return Ok(choice);
            }
        }
        Err(Refusal::NoMods)
    }
}

/// `orientation` turned `steps` binary angles about `axis`, canonical.
#[must_use]
pub fn turned(orientation: Orientation, axis: Axis, steps: i32) -> Orientation {
    let (mut yaw, mut pitch, mut roll) = orientation.raw();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let step = steps as u16;
    match axis {
        Axis::Yaw => yaw = yaw.wrapping_add(step),
        Axis::Pitch => pitch = pitch.wrapping_add(step),
        Axis::Roll => roll = roll.wrapping_add(step),
    }
    Orientation::canonical(yaw, pitch, roll)
}

/// The IDs of `id` and everything mounted on it, directly or not.
#[must_use]
pub fn subtree(build: &Assembly, id: PartId) -> Vec<PartId> {
    let mut found = vec![id];
    let mut at = 0;
    while at < found.len() {
        let parent = found[at];
        found.extend(build.children(parent).map(|part| part.id));
        at += 1;
    }
    found
}

/// A binary angle as whole degrees, signed (-180..180).
#[must_use]
pub fn degrees(angle: u16) -> i32 {
    #[allow(clippy::cast_possible_wrap)]
    let signed = i32::from(angle as i16);
    (signed * 360 + signed.signum() * 32_768) / 65_536
}

/// A part type's name in the catalogue.
#[must_use]
pub fn part_name(kind: &PartType) -> String {
    match kind.kind {
        Kind::Chassis => format!(
            "Chassis {}x{}x{}",
            u16::from(kind.cells[0]) * 20,
            u16::from(kind.cells[1]) * 20,
            u16::from(kind.cells[2]) * 20
        ),
        Kind::Beam => format!("Beam {} cm", u16::from(kind.cells[2]) * 20),
        Kind::Joint(Joint::Hinge) => "Hinge".to_owned(),
        Kind::Joint(Joint::Rotor) => "Rotor".to_owned(),
        Kind::Battery => "Battery".to_owned(),
        Kind::Sensor => "Sensor".to_owned(),
        Kind::Repair => "Repair module".to_owned(),
        Kind::Locomotion(Locomotion::Leg) => "Leg".to_owned(),
        Kind::Locomotion(Locomotion::Wheel) => "Wheel".to_owned(),
        Kind::Locomotion(Locomotion::Track) => "Track".to_owned(),
        Kind::Armor => "Armor plate".to_owned(),
        Kind::Weapon(weapon) => crate::overlay::weapon_name(weapon).to_owned(),
    }
}

/// The weapon a part type is, if it is one.
#[must_use]
pub const fn weapon_of(kind: &PartType) -> Option<Weapon> {
    match kind.kind {
        Kind::Weapon(weapon) => Some(weapon),
        _ => None,
    }
}

/// Whether one more part fits under the part ceiling.
#[must_use]
pub fn room_for_a_part(build: &Assembly) -> bool {
    build.parts().len() < MAX_PARTS
}

#[cfg(test)]
mod tests {
    use super::*;
    use shooter_sim::robot::parts::{
        BATTERY, BEAM_LONG, CHASSIS_L, CHASSIS_M, CHASSIS_S, ENERGY_RIFLE, LASER, RAIL_GUN,
        ROCKET_LAUNCHER, SHOTGUN, WHEEL,
    };
    use shooter_sim::robot::{
        EFFICIENCY_COIL, Face, HEAVY, LONG_BARREL, OVERCHARGER, RAPID_CYCLER, STANDARD,
    };

    fn holding(kind: &PartType) -> Workbench {
        Workbench {
            hand: Some(kind.id),
            ..Workbench::new(STANDARD)
        }
    }

    #[test]
    fn a_part_from_the_catalogue_goes_on_a_free_socket_and_is_selected() {
        let mut bench = holding(&BATTERY);
        let back = CHASSIS_S.socket(Face::NegX, 0, 0).unwrap();
        assert!(bench.fits(PartId(0), back));
        let id = bench.place(PartId(0), back).unwrap();
        assert_eq!(bench.selected, Some(id));
        assert_eq!(bench.draft().parts().len(), STANDARD.parts().len() + 1);
        // The lowest free ID: 1 (the retired core's).
        assert_eq!(id, PartId(1));
        assert_eq!(bench.check().map(|build| build == *bench.draft()), Ok(true));
        assert!(bench.budget().capacity > BETA.budget(&STANDARD).capacity);
    }

    #[test]
    fn a_taken_socket_or_a_second_copy_of_a_weapon_is_refused() {
        let mut bench = holding(&BATTERY);
        // The rifle holds the front's cell (1, 2).
        let rifle = CHASSIS_S.socket(Face::PosX, 1, 2).unwrap();
        assert!(!bench.fits(PartId(0), rifle));
        assert!(matches!(
            bench.place(PartId(0), rifle),
            Err(Refusal::Rule(AssemblyError::Overlap(..)))
        ));
        assert_eq!(*bench.draft(), STANDARD);
        bench.hand = Some(SHOTGUN.id);
        let back = CHASSIS_S.socket(Face::NegX, 0, 0).unwrap();
        assert_eq!(
            bench.place(PartId(0), back),
            Err(Refusal::Rule(AssemblyError::DuplicateWeapon(
                Weapon::Shotgun
            )))
        );
    }

    #[test]
    fn going_over_the_budget_is_allowed_on_the_bench_but_not_valid() {
        let mut bench = holding(&RAIL_GUN);
        let back = |u, v| CHASSIS_S.socket(Face::NegX, u, v).unwrap();
        bench.place(PartId(0), back(0, 0)).unwrap();
        bench.hand = Some(ROCKET_LAUNCHER.id);
        bench.place(PartId(0), back(2, 0)).unwrap();
        assert!(bench.check().is_ok(), "109 of 110 points");
        bench.hand = Some(BATTERY.id);
        bench.place(PartId(0), back(3, 0)).unwrap();
        let budget = bench.budget();
        assert!(budget.points > budget.build_points, "{budget:?}");
        assert!(matches!(
            bench.check(),
            Err(AssemblyError::OverBudget { .. })
        ));
        // Taking the battery off makes it valid again.
        bench.remove().unwrap();
        assert!(bench.check().is_ok());
    }

    #[test]
    fn removing_a_part_takes_what_hangs_from_it_but_never_the_root() {
        let mut bench = holding(&BEAM_LONG);
        let side = CHASSIS_S.socket(Face::PosY, 2, 0).unwrap();
        let beam = bench.place(PartId(0), side).unwrap();
        bench.hand = Some(WHEEL.id);
        let end = BEAM_LONG.socket(Face::PosZ, 0, 0).unwrap();
        bench.place(beam, end).unwrap();
        assert_eq!(bench.draft().parts().len(), STANDARD.parts().len() + 2);
        bench.selected = Some(beam);
        bench.remove().unwrap();
        assert_eq!(*bench.draft(), STANDARD);
        bench.selected = Some(PartId(0));
        assert_eq!(bench.remove(), Err(Refusal::Root));
        bench.selected = None;
        assert_eq!(bench.remove(), Err(Refusal::Nothing));
    }

    #[test]
    fn parts_turn_exactly_and_a_turn_into_another_part_is_refused() {
        let mut bench = Workbench::new(STANDARD);
        // The sensor alone on the top: it turns freely, in snaps and fine steps.
        bench.selected = Some(PartId(2));
        bench.rotate(Axis::Yaw, i32::from(SNAP)).unwrap();
        bench.rotate(Axis::Pitch, -i32::from(FINE)).unwrap();
        let mount = bench.draft().part(PartId(2)).unwrap().mount.unwrap();
        assert_eq!(mount.orientation.yaw, SNAP);
        assert_eq!(mount.orientation.pitch, -i16::try_from(FINE).unwrap());
        assert_eq!(degrees(mount.orientation.yaw), 45);
        assert!(bench.check().is_ok());
        // A long-barrel rifle (a 1 x 2 footprint) low on the left of the front, the shotgun
        // beside it: a quarter turn back swings the footprint onto the shotgun's cell.
        let mut bench = front_pair();
        bench.selected = Some(PartId(1));
        bench.cycle_mod().unwrap();
        bench.cycle_mod().unwrap();
        assert_eq!(bench.cycle_mod(), Ok(Some(LONG_BARREL)));
        let before = *bench.draft();
        let refused = bench.rotate(Axis::Yaw, -i32::from(Orientation::QUARTER));
        assert!(
            matches!(refused, Err(Refusal::Rule(AssemblyError::Overlap(..)))),
            "{refused:?}"
        );
        assert_eq!(*bench.draft(), before);
        // A quarter turn the other way overhangs the face's edge: fine.
        bench
            .rotate(Axis::Yaw, i32::from(Orientation::QUARTER))
            .unwrap();
        // The root has no socket to turn on.
        bench.selected = Some(PartId(0));
        assert_eq!(bench.rotate(Axis::Roll, 1), Err(Refusal::Root));
        // Nothing selected: the part in hand turns.
        bench.selected = None;
        assert_eq!(bench.rotate(Axis::Yaw, 1), Err(Refusal::Nothing));
        bench.hand = Some(BATTERY.id);
        bench.rotate(Axis::Roll, i32::from(SNAP)).unwrap();
        assert_eq!(bench.turn.roll, SNAP);
    }

    /// A small chassis with the rifle (ID 1) and the shotgun (ID 2) low on its front, side by
    /// side.
    fn front_pair() -> Workbench {
        let mut bench = Workbench::new(STANDARD);
        bench.new_frame(CHASSIS_S.id).unwrap();
        for (kind, u) in [(&ENERGY_RIFLE, 0), (&SHOTGUN, 1)] {
            bench.hand = Some(kind.id);
            let front = CHASSIS_S.socket(Face::PosX, u, 0).unwrap();
            bench.place(PartId(0), front).unwrap();
        }
        bench.hand = None;
        bench
    }

    #[test]
    fn mods_cycle_through_the_ones_that_fit_and_back_to_none() {
        let mut bench = front_pair();
        // The rifle: rapid cycler, overcharger, long barrel, efficiency coil, none.
        bench.selected = Some(PartId(1));
        let seen: Vec<_> = (0..5).map(|_| bench.cycle_mod().unwrap()).collect();
        assert_eq!(
            seen,
            [
                Some(RAPID_CYCLER),
                Some(OVERCHARGER),
                Some(LONG_BARREL),
                Some(EFFICIENCY_COIL),
                None
            ]
        );
        // On the standard build the shotgun sits above the rifle: the long barrel has no room
        // and is skipped.
        let mut bench = Workbench::new(STANDARD);
        bench.selected = Some(PartId(7));
        let seen: Vec<_> = (0..4).map(|_| bench.cycle_mod().unwrap()).collect();
        assert!(!seen.contains(&Some(LONG_BARREL)), "{seen:?}");
        // The blade takes none; the sensor is no weapon.
        bench.selected = Some(PartId(8));
        assert_eq!(bench.cycle_mod(), Err(Refusal::NoMods));
        bench.selected = Some(PartId(2));
        assert_eq!(bench.cycle_mod(), Err(Refusal::NoMods));
    }

    #[test]
    fn a_new_frame_is_a_lone_chassis_and_loadouts_load() {
        let mut bench = Workbench::new(STANDARD);
        bench.new_frame(CHASSIS_L.id).unwrap();
        assert_eq!(bench.draft().parts().len(), 1);
        assert!(bench.check().is_ok(), "a lone chassis is a valid robot");
        assert!(bench.new_frame(LASER.id).is_err());
        bench.load(HEAVY);
        assert_eq!(*bench.draft(), HEAVY);
        // Another chassis mounts on a socket like any part: frames of several cuboids.
        bench.new_frame(CHASSIS_M.id).unwrap();
        bench.hand = Some(CHASSIS_S.id);
        let top = CHASSIS_M.socket(Face::PosZ, 0, 0).unwrap();
        bench.place(PartId(0), top).unwrap();
        bench.hand = Some(ENERGY_RIFLE.id);
        let front = CHASSIS_S.socket(Face::PosX, 0, 0).unwrap();
        bench.place(PartId(1), front).unwrap();
        assert!(bench.check().is_ok());
    }

    #[test]
    fn angles_read_as_signed_degrees() {
        assert_eq!(degrees(0), 0);
        assert_eq!(degrees(SNAP), 45);
        assert_eq!(degrees(Orientation::QUARTER.wrapping_neg()), -90);
        assert_eq!(degrees(FINE), 1);
        assert_eq!(degrees(32_768), -180);
    }
}
