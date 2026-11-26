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
//! A part's rotation is exact: quarter turns, eighths, or fine steps of about a degree
//! ([`Step`]), about each of the socket's axes ([`Axis`]), in the canonical form the wire
//! carries. The bench never turns a part into a conventional orientation by itself.
//!
//! **Placing** (shooter#124). A placement is first a [`Proposal`]: the part in hand on one
//! socket, turned as it is, with the footprint it would reserve and the simulation's verdict on
//! the build it would make. The preview draws the proposal and a placement commits the
//! proposal's own build, so what is committed is what was previewed and what the rules
//! accepted. A refused proposal names its reason ([`describe`]).
//!
//! **History.** Every edit to the build (placing, turning, a mod, removing, a new frame, a
//! loadout) can be undone and redone: the bench keeps the builds before and after, so undo and
//! redo restore the exact build that was there.

pub mod controls;
pub mod saved;
pub mod view;

use shooter_sim::Weapon;
use shooter_sim::robot::{
    Assembly, AssemblyError, BETA, Budget, Candidate, Catalog, Joint, Kind, Locomotion, MAX_PARTS,
    ModId, Mount, Orientation, Part, PartId, PartType, PartTypeId, Power, RawMount, RawPart,
    SocketId,
};
use std::fmt;

/// The beta catalog with no build-point ceiling: every structural rule, so the bench always holds
/// a build that lays out, while a build over budget is still being worked on.
pub const BENCH: Catalog = Catalog {
    build_points: u16::MAX,
    ..BETA
};

/// A coarse step: a quarter turn.
pub const COARSE: u16 = Orientation::QUARTER;
/// A snap: an eighth of a turn.
pub const SNAP: u16 = Orientation::QUARTER / 2;
/// A fine step: about a degree (182 of the 65 536 binary angles a turn).
pub const FINE: u16 = 182;
/// How many edits the bench can undo.
pub const HISTORY: usize = 64;

/// How far a rotation key turns a part.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Step {
    /// A quarter turn ([`COARSE`]).
    #[default]
    Quarter,
    /// An eighth of a turn ([`SNAP`]).
    Eighth,
    /// About a degree ([`FINE`]).
    Fine,
}

impl Step {
    /// The step's binary angle.
    #[must_use]
    pub const fn angle(self) -> u16 {
        match self {
            Self::Quarter => COARSE,
            Self::Eighth => SNAP,
            Self::Fine => FINE,
        }
    }

    /// The next step, coarse to fine and round again.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Quarter => Self::Eighth,
            Self::Eighth => Self::Fine,
            Self::Fine => Self::Quarter,
        }
    }
}

/// The part in hand on one socket, turned as it is: what placing it would do.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Proposal {
    /// The new part: its kind and its mount (the orientation canonical), with the ID it would get.
    pub part: Part,
    /// The parent's sockets its footprint would reserve ([`Catalog::footprint`]).
    pub footprint: Vec<SocketId>,
    /// Those of them another part already holds, with the part holding each.
    pub blocked: Vec<(SocketId, PartId)>,
    /// The build placing it makes, or the rule that refuses it.
    pub verdict: Result<Assembly, AssemblyError>,
}

impl Proposal {
    /// The parent it would go on.
    #[must_use]
    pub fn parent(&self) -> PartId {
        self.part.mount.map_or(PartId(0), |mount| mount.parent)
    }

    /// The socket it would go on.
    #[must_use]
    pub fn socket(&self) -> SocketId {
        self.part.mount.map_or(SocketId(0), |mount| mount.socket)
    }
}

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
    /// No edit to undo.
    NoUndo,
    /// No undone edit to redo.
    NoRedo,
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
            Self::NoUndo => formatter.write_str("nothing to undo"),
            Self::NoRedo => formatter.write_str("nothing to redo"),
            Self::Binding(error) => error.fmt(formatter),
            Self::UnboundKey => formatter.write_str("that key cannot be bound"),
        }
    }
}

impl Refusal {
    /// What to tell the person, naming the parts `build` (the bench's build) holds; `subject` is
    /// the part being edited, if it is in `build`.
    #[must_use]
    pub fn describe(self, build: &Assembly, subject: Option<PartId>) -> String {
        match self {
            Self::Rule(error) => describe(build, error, subject),
            other => other.to_string(),
        }
    }
}

/// A part of `build` by name (its type's), or by ID if it is not there.
fn named(build: &Assembly, id: PartId) -> String {
    if let Some(instance) = BETA.instance(build, id) {
        // A weapon mounted more than once: which copy (shooter#125).
        return crate::overlay::copy_name(build, instance);
    }
    build
        .part(id)
        .and_then(|part| BETA.part_type(part.kind))
        .map_or_else(|| format!("part {}", id.0), part_name)
}

/// The simulation's reason for refusing an edit of `build` (the build before it), said so the
/// person can act on it: the parts it names by their names, and what would help. `subject` is the
/// part being edited when `build` holds it (a part turned), not when it is new (a placement).
#[must_use]
pub fn describe(build: &Assembly, error: AssemblyError, subject: Option<PartId>) -> String {
    match error {
        // The rule names the two parts in the order it met them: the other one is in the way.
        AssemblyError::Overlap(first, second) => {
            let holder = [first, second]
                .into_iter()
                .find(|id| Some(*id) != subject && build.part(*id).is_some())
                .unwrap_or(first);
            format!(
                "the footprint meets the {}'s cells: turn it or pick a free socket",
                named(build, holder)
            )
        }
        AssemblyError::TooManyParts(_) => {
            format!("the robot has its {MAX_PARTS} parts: remove one first")
        }
        AssemblyError::InteriorSocket(_) => {
            "that face rests on the part's parent: pick an outer face".to_owned()
        }
        AssemblyError::DoesNotFit(..) | AssemblyError::NotAWeapon(_) => {
            format!("{error}: pick another mod")
        }
        other => other.to_string(),
    }
}

impl From<AssemblyError> for Refusal {
    fn from(error: AssemblyError) -> Self {
        Self::Rule(error)
    }
}

/// A build on the bench and the part selected in it: what undo and redo restore.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Snapshot {
    draft: Assembly,
    selected: Option<PartId>,
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
    /// The builds before each edit (the latest last), and those undone (the latest undone last).
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
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
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            draft: self.draft,
            selected: self.selected,
        }
    }

    /// Makes `draft` the bench's build, `selected` selected, as one edit that can be undone (no
    /// edit if nothing changes).
    fn commit(&mut self, draft: Assembly, selected: Option<PartId>) {
        if draft == self.draft {
            self.selected = selected;
            return;
        }
        if self.undo.len() == HISTORY {
            self.undo.remove(0);
        }
        self.undo.push(self.snapshot());
        self.redo.clear();
        self.draft = draft;
        self.selected = selected;
    }

    /// Whether there is an edit to undo, and an undone edit to redo.
    #[must_use]
    pub fn history(&self) -> (bool, bool) {
        (!self.undo.is_empty(), !self.redo.is_empty())
    }

    /// Takes the last edit back: the build before it, exactly, with what was selected then.
    ///
    /// # Errors
    ///
    /// If there is no edit to undo.
    pub fn undo(&mut self) -> Result<(), Refusal> {
        let before = self.undo.pop().ok_or(Refusal::NoUndo)?;
        self.redo.push(self.snapshot());
        self.restore(before);
        Ok(())
    }

    /// Makes the last undone edit again: the build after it, exactly.
    ///
    /// # Errors
    ///
    /// If there is no undone edit.
    pub fn redo(&mut self) -> Result<(), Refusal> {
        let after = self.redo.pop().ok_or(Refusal::NoRedo)?;
        self.undo.push(self.snapshot());
        self.restore(after);
        Ok(())
    }

    fn restore(&mut self, snapshot: Snapshot) {
        self.draft = snapshot.draft;
        self.selected = snapshot
            .selected
            .filter(|id| self.draft.part(*id).is_some());
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
    pub fn load(&mut self, build: Assembly) {
        self.commit(build, None);
    }

    /// The root chassis' native power (shooter#130).
    #[must_use]
    pub fn power(&self) -> Power {
        BETA.budget(&self.draft).power
    }

    /// The build on a root chassis of `power` (the same size; every part stays where it is),
    /// and a chassis in hand swapped likewise: one edit, undone as one.
    ///
    /// # Errors
    ///
    /// If the build has no chassis root (it always has).
    pub fn set_power(&mut self, power: Power) -> Result<(), Refusal> {
        self.hand = self.hand.map(|hand| with_power(hand, power));
        if self.power() == power {
            return Ok(());
        }
        let mut candidate = self.draft.candidate();
        let root = candidate
            .parts
            .iter_mut()
            .find(|part| part.mount.is_none())
            .ok_or(Refusal::Nothing)?;
        root.kind = with_power(PartTypeId(root.kind), power).0;
        let draft = BENCH.validate(&candidate)?;
        self.commit(draft, self.selected);
        Ok(())
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
        let draft = BENCH.validate(&candidate)?;
        self.commit(draft, Some(PartId(0)));
        Ok(())
    }

    /// `kind` turned `turn` on `socket` of `parent`: the new part (with the lowest free ID), the
    /// footprint it would reserve and the build the bench would hold, or the rule it breaks (the
    /// bench's preview, valid or not).
    #[must_use]
    pub fn propose(
        &self,
        kind: PartTypeId,
        parent: PartId,
        socket: SocketId,
        turn: Orientation,
    ) -> Proposal {
        let mut candidate = self.draft.candidate();
        let free = (0..=u8::MAX).find(|id| candidate.parts.iter().all(|part| part.id != *id));
        let (yaw, pitch, roll) = turn.raw();
        let part = Part {
            id: PartId(free.unwrap_or(u8::MAX)),
            kind,
            mount: Some(Mount {
                parent,
                socket,
                orientation: Orientation::canonical(yaw, pitch, roll),
            }),
            modification: None,
        };
        let footprint = BENCH.footprint(&self.draft, parent, socket, kind, None, turn);
        let claims = BENCH.claims(&self.draft, parent);
        let blocked = footprint
            .iter()
            .filter_map(|cell| claims.iter().find(|(held, _)| held == cell).copied())
            .collect();
        let verdict = match free {
            Some(id) => {
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
                BENCH.validate(&candidate)
            }
            None => Err(AssemblyError::TooManyParts(candidate.parts.len() + 1)),
        };
        Proposal {
            part,
            footprint,
            blocked,
            verdict,
        }
    }

    /// The build with `kind` turned `turn` on `socket` of `parent`, as the bench would hold it,
    /// and the new part's ID: what a placement would make.
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
        let proposal = self.propose(kind, parent, socket, turn);
        Ok((proposal.verdict?, proposal.part.id))
    }

    /// Whether the part in hand, turned as it is, fits `socket` of `parent`.
    #[must_use]
    pub fn fits(&self, parent: PartId, socket: SocketId) -> bool {
        self.hand
            .is_some_and(|kind| self.placed(kind, parent, socket, self.turn).is_ok())
    }

    /// Commits `proposal` (made on this bench as it is) and selects the new part: exactly the
    /// build it previewed.
    ///
    /// # Errors
    ///
    /// The rule the proposal breaks.
    pub fn commit_proposal(&mut self, proposal: &Proposal) -> Result<PartId, Refusal> {
        let draft = proposal.verdict?;
        self.commit(draft, Some(proposal.part.id));
        Ok(proposal.part.id)
    }

    /// Puts the part in hand on `socket` of `parent` and selects it.
    ///
    /// # Errors
    ///
    /// If nothing is in hand, or the placement breaks a rule.
    pub fn place(&mut self, parent: PartId, socket: SocketId) -> Result<PartId, Refusal> {
        let kind = self.hand.ok_or(Refusal::Nothing)?;
        let proposal = self.propose(kind, parent, socket, self.turn);
        self.commit_proposal(&proposal)
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
        let draft = BENCH.validate(&candidate)?;
        self.commit(draft, None);
        Ok(())
    }

    /// Turns the part in hand `steps` binary angles about `axis` of the socket it will go on
    /// (negative turns back), or, with nothing in hand, the selected part on its socket.
    ///
    /// # Errors
    ///
    /// If nothing is in hand or selected, the selected part is the root, or the turn breaks a
    /// rule (its footprint would overlap another's): it stays as it was.
    pub fn rotate(&mut self, axis: Axis, steps: i32) -> Result<(), Refusal> {
        if self.hand.is_some() {
            self.turn = turned(self.turn, axis, steps);
            return Ok(());
        }
        let id = self.selected.ok_or(Refusal::Nothing)?;
        let mut candidate = self.draft.candidate();
        let raw = candidate
            .parts
            .iter_mut()
            .find(|part| part.id == id.0)
            .ok_or(Refusal::Nothing)?;
        let mount = raw.mount.as_mut().ok_or(Refusal::Root)?;
        let now = Orientation::canonical(mount.yaw, mount.pitch, mount.roll);
        (mount.yaw, mount.pitch, mount.roll) = turned(now, axis, steps).raw();
        let draft = BENCH.validate(&candidate)?;
        self.commit(draft, Some(id));
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
                self.commit(draft, Some(id));
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

/// The chassis of `power` the size of chassis `kind` (shooter#130); any other part type stays
/// itself.
#[must_use]
pub fn with_power(kind: PartTypeId, power: Power) -> PartTypeId {
    let Some(chassis) = BETA.part_type(kind) else {
        return kind;
    };
    if !matches!(chassis.kind, Kind::Chassis(_)) {
        return kind;
    }
    BETA.parts
        .iter()
        .find(|part| part.kind == Kind::Chassis(power) && part.cells == chassis.cells)
        .map_or(kind, |part| part.id)
}

/// A part type's name in the catalogue.
#[must_use]
pub fn part_name(kind: &PartType) -> String {
    match kind.kind {
        // A combustion chassis (shooter#130) is the fuel chassis, short enough for a row.
        Kind::Chassis(power) => format!(
            "{} {}x{}x{}",
            match power {
                Power::Electric => "Chassis",
                Power::Combustion => "Fuel chassis",
            },
            u16::from(kind.cells[0]) * 20,
            u16::from(kind.cells[1]) * 20,
            u16::from(kind.cells[2]) * 20
        ),
        Kind::Beam => format!("Beam {} cm", u16::from(kind.cells[2]) * 20),
        Kind::Rail => format!("Mount rail {} cm", u16::from(kind.cells[1]) * 20),
        Kind::Adapter => "Adapter".to_owned(),
        Kind::Joint(Joint::Hinge) => "Hinge".to_owned(),
        Kind::Joint(Joint::Rotor) => "Rotor".to_owned(),
        Kind::Battery => "Battery".to_owned(),
        Kind::FuelTank => "Fuel tank".to_owned(),
        Kind::Generator => "Generator".to_owned(),
        Kind::Engine => "Engine".to_owned(),
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

    /// The extension fixtures (shooter#126) built on the bench a part at a time, the turned rails
    /// turned with the bench's own yaw: the same build as authored, socket for socket, and its
    /// preview of each placement is what the placement makes.
    #[test]
    fn the_bench_builds_rails_and_adapters_socket_for_socket() {
        use shooter_sim::mechanics::sockets;
        use shooter_sim::robot::{ADAPTER_MAST, CROSS_RAIL, RAIL_RACK};
        for build in [RAIL_RACK, CROSS_RAIL, ADAPTER_MAST] {
            let mut bench = Workbench::new(STANDARD);
            bench.new_frame(build.parts()[0].kind).unwrap();
            for part in &build.parts()[1..] {
                let mount = part.mount.unwrap();
                bench.selected = None;
                bench.hand = Some(part.kind);
                bench.turn = Orientation::IDENTITY;
                let (yaw, _, _) = mount.orientation.raw();
                bench.rotate(Axis::Yaw, i32::from(yaw)).unwrap();
                assert_eq!(bench.turn, mount.orientation);
                let preview = bench.placed(part.kind, mount.parent, mount.socket, bench.turn);
                let id = bench.place(mount.parent, mount.socket).unwrap();
                assert_eq!(id, part.id);
                assert_eq!(preview, Ok((*bench.draft(), id)));
            }
            assert_eq!(*bench.draft(), build);
            assert_eq!(bench.check(), Ok(build));
            assert_eq!(sockets(bench.draft()), sockets(&build));
            // Removing a rail takes what it carries.
            bench.selected = Some(PartId(3));
            bench.remove().unwrap();
            assert!(bench.draft().part(PartId(3)).is_none());
            assert_eq!(
                bench.draft().parts().len(),
                build.parts().len() - subtree(&build, PartId(3)).len()
            );
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
    fn a_taken_socket_is_refused_and_a_second_copy_of_a_weapon_goes_on() {
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
        // A second copy of a weapon is no rule broken (shooter#125): it goes on, its own part.
        let copy = bench.place(PartId(0), back).unwrap();
        assert_eq!(bench.check().map(|build| build == *bench.draft()), Ok(true));
        let shotguns: Vec<PartId> = BETA
            .weapons(bench.draft())
            .filter(|instance| instance.weapon == Weapon::Shotgun)
            .map(|instance| instance.part)
            .collect();
        // The lowest free ID (1), so it is the first copy by part ID.
        assert_eq!(shotguns, [copy, PartId(10)]);
        assert_eq!(
            crate::overlay::copy_name(
                bench.draft(),
                shooter_sim::Instance {
                    part: copy,
                    weapon: Weapon::Shotgun
                }
            ),
            "Shotgun 1/2"
        );
    }

    /// The rocket pod on the bench (shooter#125): four launchers, each listed by its own number,
    /// one removed leaving the other three numbered anew; it stays a valid build.
    #[test]
    fn the_bench_holds_and_names_every_copy_of_a_weapon() {
        use shooter_sim::robot::ROCKET_POD;
        let mut bench = Workbench::new(ROCKET_POD);
        assert_eq!(bench.check(), Ok(ROCKET_POD));
        let names = |bench: &Workbench| -> Vec<String> {
            BETA.weapons(bench.draft())
                .map(|copy| crate::overlay::copy_name(bench.draft(), copy))
                .collect()
        };
        assert_eq!(
            names(&bench),
            ["Rockets 1/4", "Rockets 2/4", "Rockets 3/4", "Rockets 4/4"]
        );
        bench.selected = Some(PartId(8));
        bench.remove().unwrap();
        assert_eq!(names(&bench), ["Rockets 1/3", "Rockets 2/3", "Rockets 3/3"]);
        assert!(bench.check().is_ok());
    }

    #[test]
    fn going_over_the_budget_is_allowed_on_the_bench_but_not_valid() {
        let mut bench = holding(&RAIL_GUN);
        let back = |u, v| CHASSIS_S.socket(Face::NegX, u, v).unwrap();
        bench.place(PartId(0), back(0, 0)).unwrap();
        bench.hand = Some(ROCKET_LAUNCHER.id);
        bench.place(PartId(0), back(2, 0)).unwrap();
        assert!(bench.check().is_ok(), "117 of 118 points");
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
