//! A person's control layout (Atlas D2.i70.7, shooter#74): direct bindings from keys, mouse
//! buttons and mouse axes to a robot's actuators, alone or in action groups of several parts,
//! each binding with its own inversion, strength (or mouse sensitivity) and hold or toggle (or,
//! for a mouse axis, relative or cursor-offset steering). Presets are ordinary layouts suggested
//! from a build's locomotion ([`Bindings::preset`]); the person edits them like any other.
//!
//! **One path.** A layout resolves the inputs held this tick ([`Pressed`]) into per-actuator
//! commands ([`Motors`]) through [`Bindings::resolve`], deterministic integer arithmetic over
//! the layout's own state ([`BindingState`]: toggles and cursor offsets). The client resolves
//! its person's layout every tick and sends the commands in its input; bots resolve their
//! build's preset from the keys they would press. The host takes the commands, not the layout:
//! [`Motors::controls`] keeps only commands the robot's connected actuators can take and puts
//! every other actuator at rest, so a tampered input moves nothing it could not move with some
//! layout. A layout itself is validated against the build it drives ([`Bindings::check`]) and
//! travels bounded ([`crate::wire::encode_bindings`]).
//!
//! **Values.** Each binding gives a value in thousandths of full scale: a held (or toggled-on)
//! key or button its strength in percent of full scale; a mouse axis in relative mode its
//! movement this tick times its sensitivity (it stops when the mouse stops), in offset mode a
//! cursor offset that the movement pushes and that stays where it was left. Inversion flips the
//! binding, and each target in its group may flip its own share (the two sides of a
//! differential drive). A part takes the kind of command of the first binding that targets it
//! (in layout order) and the clamped sum of every binding of that kind on it: a stride
//! ([`Output::Stride`], legs only) in percent, a rate in its motor's top rate, or an angle of at
//! most a quarter turn either way. Nothing is timed, sequenced or conditional: what a binding
//! does follows from what is held this tick and the layout's own toggles and offsets.
//!
//! **Weapons** (shooter#127). A fire binding ([`Output::Fire`]) names guns instead of
//! actuators: one gun is a direct weapon button, several a weapon group, and groups may overlap
//! and be held together. While held (or toggled on) its guns ask to fire
//! ([`crate::weapons::Triggers`], [`Bindings::act`]); the host fires each copy that is ready,
//! within its family's salvo limit and with the energy for it
//! ([`crate::weapons::Arsenal::trigger_tuned`]). A request is direct, never a timed sequence.
//!
//! **Tracking the aim** (shooter#132). A tracking binding ([`Output::Track`]) names joints:
//! while held (or toggled on) each is told [`Command::Track`] instead of what else binds it.
//! The input carries only that word; the host resolves it from its own state into a servo angle
//! within the joint's limits that turns the gun the joint carries toward the person's camera
//! target ([`crate::aim::track`]), so the joint turns at its servo's speed and on the pool's
//! energy like any other, and a lost or unpowered joint aims nothing.
//!
//! **No chassis forces.** A binding reaches only actuators. Until the robots walk and roll on
//! their own (see the Atlas: the interim movement drive), the match still moves a robot by the
//! movement rules ([`crate::player::Motion::drive`]); the bindings drive its joints, legs,
//! wheels and tracks.

use crate::mechanics::{Actuator, Command, Controls, GaitChoice, Layout};
use crate::robot::{Assembly, Joint, Locomotion, PartId};
use crate::weapons::Triggers;
use std::fmt;

/// The most bindings in a layout.
pub const MAX_BINDINGS: usize = 24;
/// The most parts in one binding's action group.
pub const MAX_GROUP: usize = 8;
/// The most commanded parts in one tick's [`Motors`].
pub const MAX_MOTORS: usize = 32;
/// Full scale of a binding's value.
pub const FULL: i32 = 1_000;
/// The mouse button the presets fire every gun with: the middle one (W3C 1).
pub const ALL_GUNS_BUTTON: u8 = 1;
/// The mouse button the presets track the aim point with (shooter#132): the right one (W3C 2),
/// the precision aim's default.
pub const AIM_BUTTON: u8 = 2;
/// How far the presets' A and D turn a steering motor (shooter#170): this percentage of a
/// quarter turn, about 13.5°. Provisional (Atlas Q2.i70.1): on the steer-car's 40 cm wheelbase
/// at its top speed a third of a quarter turn (30°) pulled about the match's gravity sideways,
/// so it slid and its 40 N·m servos were thrown off their angle.
pub const STEER: u8 = 15;

/// The keys a binding may name, by their W3C physical key codes (what winit's `KeyCode` and the
/// browser's `KeyboardEvent.code` call them); a [`Source::Key`] is an index into this table.
pub const KEYS: [&str; 48] = [
    "KeyA",
    "KeyB",
    "KeyC",
    "KeyD",
    "KeyE",
    "KeyF",
    "KeyG",
    "KeyH",
    "KeyI",
    "KeyJ",
    "KeyK",
    "KeyL",
    "KeyM",
    "KeyN",
    "KeyO",
    "KeyP",
    "KeyQ",
    "KeyR",
    "KeyS",
    "KeyT",
    "KeyU",
    "KeyV",
    "KeyW",
    "KeyX",
    "KeyY",
    "KeyZ",
    "Digit0",
    "Digit1",
    "Digit2",
    "Digit3",
    "Digit4",
    "Digit5",
    "Digit6",
    "Digit7",
    "Digit8",
    "Digit9",
    "Space",
    "ShiftLeft",
    "ShiftRight",
    "ControlLeft",
    "ControlRight",
    "AltLeft",
    "Tab",
    "CapsLock",
    "ArrowUp",
    "ArrowDown",
    "ArrowLeft",
    "ArrowRight",
];
/// Mouse buttons a binding may name: the W3C button indices 0 to 4.
pub const MOUSE_BUTTONS: u8 = 5;

/// The index in [`KEYS`] of the key with W3C code `code`.
#[must_use]
pub fn key(code: &str) -> Option<u8> {
    KEYS.iter()
        .position(|known| *known == code)
        .and_then(|index| u8::try_from(index).ok())
}

/// What a binding listens to.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Source {
    /// A key, by its index in [`KEYS`].
    Key(u8),
    /// A mouse button (W3C index, below [`MOUSE_BUTTONS`]).
    Button(u8),
    /// The mouse's horizontal movement (right positive)...
    MouseX,
    /// ...and its vertical movement (down positive).
    MouseY,
}

impl Source {
    const fn is_axis(self) -> bool {
        matches!(self, Self::MouseX | Self::MouseY)
    }
}

/// How a binding turns its source into a value.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Mode {
    /// A key or button: full while held.
    Hold,
    /// A key or button: each press switches it on or off.
    Toggle,
    /// A mouse axis: this tick's movement; the value stops when the mouse stops.
    Relative,
    /// A mouse axis: a cursor offset the movement pushes; it stays where it was left.
    Offset,
}

/// The kind of command a binding gives its parts.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Output {
    /// A leg's stride through its gait ([`Command::Stride`]); legs only.
    Stride,
    /// A motor's rate ([`Command::Rate`]), full scale its top rate.
    Rate,
    /// A servo's angle ([`Command::Angle`]), full scale a quarter turn.
    Angle,
    /// Fire (shooter#127): while held (or toggled on) every gun in its group asks to fire
    /// ([`crate::weapons::Triggers`]). One gun is a direct weapon button, several a weapon group;
    /// groups may overlap, and a gun asked twice in a tick fires once. Keys and buttons only;
    /// strength and inversion change nothing.
    Fire,
    /// Track the aim point (shooter#132): while held (or toggled on) every joint in its group
    /// is told [`Command::Track`], whatever else binds it this tick, and the host turns it
    /// toward the person's camera target within its limits, at its servo's speed and on the
    /// pool's energy ([`crate::aim::track`]). Joints only; keys and buttons only; strength and
    /// inversion change nothing.
    Track,
}

/// One part of a binding's action group, and whether it takes the binding's value reversed.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Target {
    pub part: PartId,
    pub invert: bool,
}

/// One binding: a source, how it is read, what kind of command it gives, its inversion and
/// strength (percent of full scale for a key or button; for a mouse axis its sensitivity: in
/// relative mode thousandths of full scale per count of movement, in offset mode a tenth of
/// that), and its action group.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct Binding {
    pub source: Source,
    pub mode: Mode,
    pub output: Output,
    pub invert: bool,
    pub strength: u8,
    pub targets: Vec<Target>,
}

/// A control layout: the legs' gait choice and the bindings, in order.
#[derive(Clone, Debug, Default, Eq, PartialEq, Hash)]
pub struct Bindings {
    pub gait: GaitChoice,
    pub bindings: Vec<Binding>,
}

/// Why a layout is not one the robot can take.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BindingError {
    /// More than [`MAX_BINDINGS`] bindings.
    TooMany,
    /// A binding (by index) with no part, or more than [`MAX_GROUP`].
    Group(usize),
    /// A binding naming a key or button outside [`KEYS`] or [`MOUSE_BUTTONS`].
    Source(usize),
    /// A key or button read relative or as an offset, or a mouse axis held or toggled.
    Mode(usize),
    /// A strength of zero or above 100.
    Strength(usize),
    /// A part named twice in one group.
    Twice(usize, PartId),
    /// A part that is no actuator of the build.
    NotAnActuator(usize, PartId),
    /// A stride for a part that is no leg.
    NotALeg(usize, PartId),
    /// A fire binding naming a part that is no gun of the build (the blade and the grapple
    /// module act on their own keys).
    NotAGun(usize, PartId),
    /// A tracking binding naming a part that is no joint.
    NotAJoint(usize, PartId),
}

impl fmt::Display for BindingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooMany => write!(formatter, "more than {MAX_BINDINGS} bindings"),
            Self::Group(at) => write!(formatter, "binding {} needs 1 to {MAX_GROUP} parts", at + 1),
            Self::Source(at) => write!(formatter, "binding {} has no such input", at + 1),
            Self::Mode(at) => write!(
                formatter,
                "binding {}: keys hold or toggle, mouse axes steer relative or by offset",
                at + 1
            ),
            Self::Strength(at) => {
                write!(formatter, "binding {}: strength is 1 to 100", at + 1)
            }
            Self::Twice(at, part) => {
                write!(formatter, "binding {} names part {} twice", at + 1, part.0)
            }
            Self::NotAnActuator(at, part) => write!(
                formatter,
                "binding {}: part {} is no joint, leg, wheel or track",
                at + 1,
                part.0
            ),
            Self::NotALeg(at, part) => write!(
                formatter,
                "binding {}: only legs stride (part {})",
                at + 1,
                part.0
            ),
            Self::NotAGun(at, part) => {
                write!(formatter, "binding {}: part {} is no gun", at + 1, part.0)
            }
            Self::NotAJoint(at, part) => write!(
                formatter,
                "binding {}: only joints track the aim (part {})",
                at + 1,
                part.0
            ),
        }
    }
}

impl std::error::Error for BindingError {}

/// What a person holds this tick: keys by their bit in [`KEYS`], mouse buttons by theirs, and
/// the mouse's movement since the last tick in counts (pixels; right and down positive).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub struct Pressed {
    pub keys: u64,
    pub buttons: u8,
    pub mouse: [i32; 2],
}

impl Pressed {
    /// With key `code` (W3C) held, if a binding may name it.
    #[must_use]
    pub fn with_key(mut self, code: &str) -> Self {
        if let Some(index) = key(code) {
            self.keys |= 1 << index;
        }
        self
    }

    /// The keys the presets drive with for a movement `forward` (+1 W, -1 S) and `strafe`
    /// (+1 D, -1 A): what a bot or a script holds.
    #[must_use]
    pub fn wasd(forward: i8, strafe: i8) -> Self {
        let mut pressed = Self::default();
        for (held, code) in [
            (forward > 0, "KeyW"),
            (forward < 0, "KeyS"),
            (strafe < 0, "KeyA"),
            (strafe > 0, "KeyD"),
        ] {
            if held {
                pressed = pressed.with_key(code);
            }
        }
        pressed
    }

    fn holds(&self, source: Source) -> bool {
        match source {
            Source::Key(index) => index < 64 && self.keys & (1 << index) != 0,
            Source::Button(index) => index < 8 && self.buttons & (1 << index) != 0,
            Source::MouseX | Source::MouseY => false,
        }
    }

    const fn movement(&self, source: Source) -> i32 {
        match source {
            Source::MouseX => self.mouse[0],
            Source::MouseY => self.mouse[1],
            Source::Key(_) | Source::Button(_) => 0,
        }
    }
}

/// A layout's own state between ticks: which toggles are on, which bindings were held last
/// tick (a toggle switches on a fresh press) and every offset binding's cursor offset.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub struct BindingState {
    toggled: u32,
    held: u32,
    offsets: [i16; MAX_BINDINGS],
}

impl BindingState {
    /// Whether binding `index` is toggled on.
    #[must_use]
    pub const fn toggled(&self, index: usize) -> bool {
        index < MAX_BINDINGS && self.toggled & (1 << index) != 0
    }

    /// Binding `index`'s cursor offset, in thousandths of full scale.
    #[must_use]
    pub fn offset(&self, index: usize) -> i32 {
        self.offsets.get(index).copied().map_or(0, i32::from)
    }
}

/// One tick's commands for a robot's actuators: the legs' gait and at most [`MAX_MOTORS`]
/// commanded parts, each once, in the order given. What an input carries.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Motors {
    pub gait: GaitChoice,
    len: u8,
    commands: [(PartId, Command); MAX_MOTORS],
}

impl Default for Motors {
    fn default() -> Self {
        Self::REST
    }
}

impl Motors {
    /// Nothing commanded: every actuator at rest ([`Motors::controls`]).
    pub const REST: Self = Self {
        gait: GaitChoice::Suggested,
        len: 0,
        commands: [(PartId(0), Command::Idle); MAX_MOTORS],
    };

    /// Only joint `part` told to track the aim point ([`Command::Track`]): a script's tracking
    /// binding held.
    #[must_use]
    pub const fn track(part: PartId) -> Self {
        let mut commands = Self::REST.commands;
        commands[0] = (part, Command::Track);
        Self {
            gait: GaitChoice::Suggested,
            len: 1,
            commands,
        }
    }

    /// These motors with `part` told `command`; false (and unchanged) if `part` is already
    /// commanded or there is no room.
    pub fn push(&mut self, part: PartId, command: Command) -> bool {
        if self.get(part).is_some() || usize::from(self.len) >= MAX_MOTORS {
            return false;
        }
        self.commands[usize::from(self.len)] = (part, command);
        self.len += 1;
        true
    }

    /// The commanded parts, in order.
    #[must_use]
    pub fn commands(&self) -> &[(PartId, Command)] {
        &self.commands[..usize::from(self.len)]
    }

    /// What `part` is told, if anything.
    #[must_use]
    pub fn get(&self, part: PartId) -> Option<Command> {
        self.commands()
            .iter()
            .find(|(commanded, _)| *commanded == part)
            .map(|(_, command)| *command)
    }

    /// The robot's controls for a tick (the host's word on an input): every connected actuator
    /// of `layout` at rest (a leg standing, a wheel, track or joint braked where it is
    /// centred: a stride of zero, a rate of zero, an angle of zero) unless these motors command
    /// it a command it takes. A command for a part that is no connected actuator, a stride for
    /// one that is no leg, or a track for one that is no joint, is ignored. A joint's track stays
    /// [`Command::Track`] for the match to resolve ([`Motors::resolved`]).
    #[must_use]
    pub fn controls(&self, layout: &Layout) -> Controls {
        let mut controls = Controls {
            gait: self.gait,
            ..Controls::IDLE
        };
        for actuator in &layout.actuators {
            let leg = actuator.kind == Actuator::Locomotion(Locomotion::Leg);
            let rest = match actuator.kind {
                Actuator::Locomotion(Locomotion::Leg) => Command::Stride(0),
                Actuator::Locomotion(_) => Command::Rate(0),
                Actuator::Joint(_) => Command::Angle(0),
            };
            let joint = matches!(actuator.kind, Actuator::Joint(_));
            let command = match self.get(actuator.part) {
                Some(Command::Stride(_)) if !leg => rest,
                // An aim the host did not resolve ([`crate::aim::track`]) rests; only a joint
                // tracks.
                Some(Command::Track) if !joint => rest,
                Some(command) => command,
                None => rest,
            };
            controls = controls.with(actuator.part, command);
        }
        controls
    }

    /// Whether any part is told to track the aim point.
    #[must_use]
    pub fn tracking(&self) -> bool {
        self.commands()
            .iter()
            .any(|(_, command)| *command == Command::Track)
    }

    /// These motors with every track replaced by what `resolve` makes of it (the host's angle
    /// for that joint, [`crate::aim::track`]), or left out where it makes nothing (the part is
    /// no connected joint: it rests).
    #[must_use]
    pub fn resolved(&self, mut resolve: impl FnMut(PartId) -> Option<Command>) -> Self {
        let mut resolved = Self {
            gait: self.gait,
            ..Self::REST
        };
        for &(part, command) in self.commands() {
            let command = match command {
                Command::Track => match resolve(part) {
                    Some(command) => command,
                    None => continue,
                },
                command => command,
            };
            resolved.push(part, command);
        }
        resolved
    }
}

/// One tick of a layout ([`Bindings::act`]): its actuators' commands and the guns it asks to
/// fire, what an input carries ([`crate::player::Intent::motors`],
/// [`crate::player::Intent::triggers`]).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub struct Actions {
    pub motors: Motors,
    pub triggers: Triggers,
}

/// What kind of actuator `part` is in `layout`, and its top rate (tenths of a radian a second).
fn actuator_of(layout: &Layout, part: PartId) -> Option<(Actuator, u16)> {
    layout
        .actuators
        .iter()
        .find(|actuator| actuator.part == part)
        .map(|actuator| (actuator.kind, actuator.drive.rate))
}

impl Bindings {
    /// Whether the layout fits within its bounds and every binding's group drives actuators
    /// of `layout` (the build's, [`Layout::of`]) as they can be driven.
    ///
    /// # Errors
    ///
    /// The first rule it breaks.
    pub fn check(&self, layout: &Layout) -> Result<(), BindingError> {
        self.check_bounds()?;
        for (at, binding) in self.bindings.iter().enumerate() {
            for target in &binding.targets {
                if binding.output == Output::Fire {
                    if !layout.guns.contains(&target.part) {
                        return Err(BindingError::NotAGun(at, target.part));
                    }
                    continue;
                }
                let Some((kind, _)) = actuator_of(layout, target.part) else {
                    return Err(BindingError::NotAnActuator(at, target.part));
                };
                if binding.output == Output::Stride && kind != Actuator::Locomotion(Locomotion::Leg)
                {
                    return Err(BindingError::NotALeg(at, target.part));
                }
                if binding.output == Output::Track && !matches!(kind, Actuator::Joint(_)) {
                    return Err(BindingError::NotAJoint(at, target.part));
                }
            }
        }
        Ok(())
    }

    /// [`Self::check`] against `build`'s whole layout.
    ///
    /// # Errors
    ///
    /// The first rule it breaks.
    pub fn validate(&self, build: &Assembly) -> Result<(), BindingError> {
        self.check(&Layout::of(build))
    }

    /// The rules that need no build: counts, sources, modes, strengths, no part twice.
    ///
    /// # Errors
    ///
    /// The first rule it breaks.
    pub fn check_bounds(&self) -> Result<(), BindingError> {
        if self.bindings.len() > MAX_BINDINGS {
            return Err(BindingError::TooMany);
        }
        for (at, binding) in self.bindings.iter().enumerate() {
            if binding.targets.is_empty() || binding.targets.len() > MAX_GROUP {
                return Err(BindingError::Group(at));
            }
            let known = match binding.source {
                Source::Key(index) => usize::from(index) < KEYS.len(),
                Source::Button(index) => index < MOUSE_BUTTONS,
                Source::MouseX | Source::MouseY => true,
            };
            if !known {
                return Err(BindingError::Source(at));
            }
            let axis_mode = matches!(binding.mode, Mode::Relative | Mode::Offset);
            if binding.source.is_axis() != axis_mode
                || (matches!(binding.output, Output::Fire | Output::Track)
                    && binding.source.is_axis())
            {
                return Err(BindingError::Mode(at));
            }
            if !(1..=100).contains(&binding.strength) {
                return Err(BindingError::Strength(at));
            }
            for (index, target) in binding.targets.iter().enumerate() {
                if binding.targets[..index]
                    .iter()
                    .any(|earlier| earlier.part == target.part)
                {
                    return Err(BindingError::Twice(at, target.part));
                }
            }
        }
        Ok(())
    }

    /// This layout without the targets `layout` no longer has as actuators it can drive so
    /// (a part removed in the builder), and without the bindings left with none.
    #[must_use]
    pub fn pruned(&self, layout: &Layout) -> Self {
        let bindings = self
            .bindings
            .iter()
            .filter_map(|binding| {
                let targets: Vec<Target> = binding
                    .targets
                    .iter()
                    .copied()
                    .filter(|target| {
                        if binding.output == Output::Fire {
                            return layout.guns.contains(&target.part);
                        }
                        actuator_of(layout, target.part).is_some_and(|(kind, _)| {
                            match binding.output {
                                Output::Stride => kind == Actuator::Locomotion(Locomotion::Leg),
                                Output::Track => matches!(kind, Actuator::Joint(_)),
                                _ => true,
                            }
                        })
                    })
                    .collect();
                (!targets.is_empty()).then(|| Binding {
                    targets,
                    ..binding.clone()
                })
            })
            .collect();
        Self {
            gait: self.gait,
            bindings,
        }
    }

    /// The preset layout for `layout`'s actuators, suggested from what is mounted where and
    /// correcting nothing (Atlas D2.i70.5, D2.i70.7):
    ///
    /// - **Legs**: W strides every leg forward and S back on the suggested gait; D strides the
    ///   left legs forward and the right ones back (turning right) at half strength, A the
    ///   other way.
    /// - **Wheels and tracks**: W turns each at its top rate the way that rolls it toward the
    ///   robot's front as mounted (its axle across the robot), S the other way; D turns the
    ///   left side forward and the right side back at 60 %, A the other way. A part whose axle
    ///   points along the robot rolls it sideways for W: its mounting says so.
    /// - **Steering motors** ([`Layout::steering_joints`], shooter#170): when joints turning
    ///   about the robot's vertical carry wheels, A and D hold those joints at a third of a
    ///   quarter turn (about 30°) the way that turns the rolling robot left or right (wheels
    ///   ahead of the robot's other axles point into the turn, wheels behind them away from it),
    ///   back to straight ahead on release, and no longer turn the wheels differentially
    ///   (tracks still do). The mouse and the precision aim leave them alone.
    /// - **Other joints**: every rotor follows the mouse's horizontal offset and every hinge its
    ///   vertical offset (up raises), so two joint axes aim a turret; the offset stays when the
    ///   mouse stops. Held, the right mouse button (the precision aim's, shooter#132) makes every
    ///   joint track the camera's target instead ([`Output::Track`]).
    ///
    /// - **Guns** (shooter#127): the middle mouse button fires every gun at once, one weapon
    ///   group; the left button's primary trigger fires the held copy as before.
    ///
    /// Groups larger than [`MAX_GROUP`] are split over several bindings on the same input.
    #[must_use]
    pub fn preset(layout: &Layout) -> Self {
        let mut bindings = Vec::new();
        let w = Source::Key(key("KeyW").unwrap_or(0));
        let s = Source::Key(key("KeyS").unwrap_or(0));
        let a = Source::Key(key("KeyA").unwrap_or(0));
        let d = Source::Key(key("KeyD").unwrap_or(0));
        let mut add = |source: Source,
                       mode: Mode,
                       output: Output,
                       invert: bool,
                       strength: u8,
                       targets: &[Target]| {
            for group in targets.chunks(MAX_GROUP) {
                bindings.push(Binding {
                    source,
                    mode,
                    output,
                    invert,
                    strength,
                    targets: group.to_vec(),
                });
            }
        };
        // Legs, and which side each hip is on (left is −Z).
        let legs: Vec<(PartId, bool)> = layout
            .actuators
            .iter()
            .filter(|actuator| actuator.kind == Actuator::Locomotion(Locomotion::Leg))
            .map(|actuator| (actuator.part, actuator.hip[2] < 0.0))
            .collect();
        if !legs.is_empty() {
            let along: Vec<Target> = legs
                .iter()
                .map(|&(part, _)| Target {
                    part,
                    invert: false,
                })
                .collect();
            let right: Vec<Target> = legs
                .iter()
                .map(|&(part, left)| Target {
                    part,
                    invert: !left,
                })
                .collect();
            add(w, Mode::Hold, Output::Stride, false, 100, &along);
            add(s, Mode::Hold, Output::Stride, true, 100, &along);
            add(d, Mode::Hold, Output::Stride, false, 50, &right);
            add(a, Mode::Hold, Output::Stride, true, 50, &right);
        }
        // Steering motors (shooter#170): a positive angle about +Y points the wheels left, so D
        // (right) is the negative one for wheels ahead of the line they steer about and the
        // positive one for wheels behind it (rear steering turns the robot the other way).
        let steering: Vec<Target> = layout
            .steering_joints()
            .into_iter()
            .map(|steering| Target {
                part: steering.actuator.part,
                invert: (steering.hinge.axis[1] > 0.0) != steering.rear,
            })
            .collect();
        // Wheels and tracks: the spin that rolls each forward is along axle × up, and which
        // side its axle is on.
        let rolling: Vec<(PartId, bool, bool, bool)> = layout
            .actuators
            .iter()
            .filter(|actuator| {
                matches!(
                    actuator.kind,
                    Actuator::Locomotion(Locomotion::Wheel | Locomotion::Track)
                )
            })
            .filter_map(|actuator| {
                let hinge = layout.hinges.get(*actuator.hinges.first()?)?;
                // A positive spin rolls it along axle × up = (−axle.z, 0, axle.x): toward the
                // front when −axle.z > 0. An axle along the robot rolls it sideways; then W is
                // the spin that rolls it toward the robot's right (+Z).
                let along = if hinge.axis[2].abs() >= 0.5 {
                    -hinge.axis[2]
                } else {
                    hinge.axis[0]
                };
                let backward = along < 0.0;
                Some((
                    actuator.part,
                    backward,
                    hinge.anchor[2] < 0.0,
                    actuator.kind == Actuator::Locomotion(Locomotion::Track),
                ))
            })
            .collect();
        if !rolling.is_empty() {
            let along: Vec<Target> = rolling
                .iter()
                .map(|&(part, backward, _, _)| Target {
                    part,
                    invert: backward,
                })
                .collect();
            let right: Vec<Target> = rolling
                .iter()
                .filter(|&&(_, _, _, track)| track || steering.is_empty())
                .map(|&(part, backward, left, _)| Target {
                    part,
                    invert: backward != !left,
                })
                .collect();
            add(w, Mode::Hold, Output::Rate, false, 100, &along);
            add(s, Mode::Hold, Output::Rate, true, 100, &along);
            add(d, Mode::Hold, Output::Rate, false, 60, &right);
            add(a, Mode::Hold, Output::Rate, true, 60, &right);
        }
        // A/D steer the actual wheel mounts; W/S keep rolling both sides together.
        // Releasing A/D returns the motors to their authored straight-ahead angles.
        add(d, Mode::Hold, Output::Angle, false, STEER, &steering);
        add(a, Mode::Hold, Output::Angle, true, STEER, &steering);
        let joints = |joint: Joint| -> Vec<Target> {
            layout
                .actuators
                .iter()
                .filter(|actuator| actuator.kind == Actuator::Joint(joint))
                .filter(|actuator| !steering.iter().any(|target| target.part == actuator.part))
                .map(|actuator| Target {
                    part: actuator.part,
                    invert: false,
                })
                .collect()
        };
        let rotors = joints(Joint::Rotor);
        if !rotors.is_empty() {
            add(
                Source::MouseX,
                Mode::Offset,
                Output::Angle,
                true,
                20,
                &rotors,
            );
        }
        let hinges = joints(Joint::Hinge);
        if !hinges.is_empty() {
            add(
                Source::MouseY,
                Mode::Offset,
                Output::Angle,
                true,
                20,
                &hinges,
            );
        }
        // Held, the precision aim's button tracks the camera's target with every joint
        // (shooter#132).
        let tracking: Vec<Target> = rotors.iter().chain(&hinges).copied().collect();
        if !tracking.is_empty() {
            add(
                Source::Button(AIM_BUTTON),
                Mode::Hold,
                Output::Track,
                false,
                100,
                &tracking,
            );
        }
        // Every gun together on the middle mouse button (shooter#127); the primary trigger (the
        // left button) stays the held copy's.
        let guns: Vec<Target> = layout
            .guns
            .iter()
            .map(|&part| Target {
                part,
                invert: false,
            })
            .collect();
        add(
            Source::Button(ALL_GUNS_BUTTON),
            Mode::Hold,
            Output::Fire,
            false,
            100,
            &guns,
        );
        bindings.truncate(MAX_BINDINGS);
        Self {
            gait: GaitChoice::Suggested,
            bindings,
        }
    }

    /// [`Self::preset`] for `build` whole.
    #[must_use]
    pub fn preset_of(build: &Assembly) -> Self {
        Self::preset(&Layout::of(build))
    }

    /// This tick's commands for `layout`'s actuators from what is `pressed`, moving `state`
    /// on (toggles, offsets). Targets `layout` does not have (a part lost this life) are
    /// skipped; a binding whose kind a part does not take adds nothing to it.
    #[must_use]
    pub fn resolve(&self, layout: &Layout, state: &mut BindingState, pressed: Pressed) -> Motors {
        self.act(layout, state, pressed).motors
    }

    /// [`Self::resolve`] with the guns its fire bindings ask to fire this tick too
    /// (shooter#127): every gun of `layout` in the group of a fire binding that is held (or
    /// toggled on), each once however many groups name it.
    #[must_use]
    pub fn act(&self, layout: &Layout, state: &mut BindingState, pressed: Pressed) -> Actions {
        // Each part's command kind and summed value, in the order parts are first named.
        let mut sums: Vec<(PartId, Output, i32)> = Vec::new();
        let mut triggers = Triggers::NONE;
        // The joints told to track the aim point this tick: that overrides whatever else binds
        // them.
        let mut tracked: Vec<PartId> = Vec::new();
        let mut held = 0u32;
        for (at, binding) in self.bindings.iter().take(MAX_BINDINGS).enumerate() {
            let bit = 1u32 << at;
            let strength = i32::from(binding.strength.min(100));
            let value = match binding.mode {
                Mode::Hold | Mode::Toggle => {
                    let down = pressed.holds(binding.source);
                    if down {
                        held |= bit;
                    }
                    if binding.mode == Mode::Toggle && down && state.held & bit == 0 {
                        state.toggled ^= bit;
                    }
                    let on = match binding.mode {
                        Mode::Toggle => state.toggled & bit != 0,
                        _ => down,
                    };
                    if on { FULL * strength / 100 } else { 0 }
                }
                Mode::Relative => pressed
                    .movement(binding.source)
                    .saturating_mul(strength)
                    .clamp(-FULL, FULL),
                Mode::Offset => {
                    let pushed = pressed.movement(binding.source).saturating_mul(strength) / 10;
                    let offset = (i32::from(state.offsets[at]) + pushed).clamp(-FULL, FULL);
                    state.offsets[at] = i16::try_from(offset).unwrap_or(0);
                    offset
                }
            };
            if binding.output == Output::Fire {
                if value != 0 {
                    for target in &binding.targets {
                        if layout.guns.contains(&target.part) {
                            triggers.press(target.part);
                        }
                    }
                }
                continue;
            }
            if binding.output == Output::Track {
                if value != 0 {
                    for target in &binding.targets {
                        if actuator_of(layout, target.part)
                            .is_some_and(|(kind, _)| matches!(kind, Actuator::Joint(_)))
                            && !tracked.contains(&target.part)
                        {
                            tracked.push(target.part);
                        }
                    }
                }
                continue;
            }
            let value = if binding.invert { -value } else { value };
            for target in &binding.targets {
                let Some((kind, _)) = actuator_of(layout, target.part) else {
                    continue;
                };
                if binding.output == Output::Stride && kind != Actuator::Locomotion(Locomotion::Leg)
                {
                    continue;
                }
                let share = if target.invert { -value } else { value };
                match sums.iter_mut().find(|(part, _, _)| *part == target.part) {
                    Some((_, output, sum)) if *output == binding.output => {
                        *sum = sum.saturating_add(share);
                    }
                    Some(_) => {}
                    None => sums.push((target.part, binding.output, share)),
                }
            }
        }
        state.held = held;
        let mut motors = Motors {
            gait: self.gait,
            ..Motors::REST
        };
        for (part, output, sum) in sums {
            let sum = sum.clamp(-FULL, FULL);
            if tracked.contains(&part) {
                motors.push(part, Command::Track);
                continue;
            }
            let command = match output {
                Output::Stride => Command::Stride(i8::try_from(sum / 10).unwrap_or(0)),
                Output::Rate => {
                    let top = actuator_of(layout, part).map_or(0, |(_, rate)| i32::from(rate));
                    Command::Rate(
                        i16::try_from((sum * top / 10).clamp(-32_767, 32_767)).unwrap_or(0),
                    )
                }
                Output::Angle => {
                    let quarter = i32::from(crate::robot::Orientation::QUARTER);
                    Command::Angle(
                        i16::try_from((sum * quarter / FULL).clamp(-32_767, 32_767)).unwrap_or(0),
                    )
                }
                // Fire and tracking bindings give no summed command.
                Output::Fire | Output::Track => continue,
            };
            motors.push(part, command);
        }
        for part in tracked {
            // Already told to track when another binding named it first.
            motors.push(part, Command::Track);
        }
        Actions { motors, triggers }
    }
}
