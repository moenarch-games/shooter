//! The control layout on the bench (Atlas D2.i70.7, shooter#74): the person's bindings for the
//! robot being built, edited beside it ([`super::view`]'s controls panel) and sent with Ready.
//! Like the [`super::Workbench`], it holds no rules of its own: every edit is checked by the
//! simulation's one validation path ([`Bindings::check`]) against the build's actuators, and an
//! edit that breaks a rule is refused with its reason.
//!
//! The layout follows the build: while the person has not edited it, it is the build's preset
//! ([`Bindings::preset`]), suggested again whenever the build changes; once edited it is theirs,
//! and a change to the build only drops the targets the build no longer has ([`Bindings::pruned`]).

use super::Refusal;
use shooter_sim::bindings::{
    Binding, Bindings, KEYS, MAX_BINDINGS, MOUSE_BUTTONS, Mode, Output, Source, Target, key,
};
use shooter_sim::mechanics::{Gait, GaitChoice, Layout};
use shooter_sim::robot::{Assembly, PartId};

/// The bench's control layout and the binding being edited.
#[derive(Clone, Debug)]
pub struct ControlsBench {
    layout: Bindings,
    /// Edited by the person: no longer re-suggested when the build changes.
    custom: bool,
    /// The build the layout is for and its actuators.
    build: Assembly,
    actuators: Layout,
    /// The binding being edited.
    pub selected: Option<usize>,
    /// The next key pressed binds the selected binding.
    pub capturing: bool,
}

impl ControlsBench {
    /// The layout for `build`: `layout` if it fits it, else the build's preset.
    #[must_use]
    pub fn new(build: Assembly, layout: Option<&Bindings>) -> Self {
        let actuators = Layout::of(&build);
        let preset = Bindings::preset(&actuators);
        let (layout, custom) = match layout {
            Some(layout) if layout.check(&actuators).is_ok() => (layout.clone(), *layout != preset),
            _ => (preset, false),
        };
        Self {
            layout,
            custom,
            build,
            actuators,
            selected: None,
            capturing: false,
        }
    }

    /// The layout, valid for the build it follows.
    #[must_use]
    pub const fn layout(&self) -> &Bindings {
        &self.layout
    }

    /// Whether the person has edited it (else it is the build's preset).
    #[must_use]
    pub const fn custom(&self) -> bool {
        self.custom
    }

    /// The build's actuators.
    #[must_use]
    pub const fn actuators(&self) -> &Layout {
        &self.actuators
    }

    /// Follows the bench's build: a new preset while unedited, else the edited layout without
    /// the targets the build no longer has.
    pub fn follow(&mut self, build: &Assembly) {
        if *build == self.build {
            return;
        }
        self.build = *build;
        self.actuators = Layout::of(build);
        self.layout = if self.custom {
            self.layout.pruned(&self.actuators)
        } else {
            Bindings::preset(&self.actuators)
        };
        self.selected = self
            .selected
            .filter(|index| *index < self.layout.bindings.len());
        self.capturing = false;
    }

    /// Back to the build's preset.
    pub fn preset(&mut self) {
        self.layout = Bindings::preset(&self.actuators);
        self.custom = false;
        self.selected = None;
        self.capturing = false;
    }

    /// Applies `edit` to a copy and keeps it if the build can take it.
    fn edit(
        &mut self,
        edit: impl FnOnce(&mut Bindings) -> Result<(), Refusal>,
    ) -> Result<(), Refusal> {
        let mut layout = self.layout.clone();
        edit(&mut layout)?;
        layout.check(&self.actuators).map_err(Refusal::Binding)?;
        self.layout = layout;
        self.custom = true;
        Ok(())
    }

    fn with_selected(
        &mut self,
        edit: impl FnOnce(&mut Binding) -> Result<(), Refusal>,
    ) -> Result<(), Refusal> {
        let index = self.selected.ok_or(Refusal::Nothing)?;
        self.edit(|layout| edit(layout.bindings.get_mut(index).ok_or(Refusal::Nothing)?))
    }

    /// A new binding for `part` (a gun fires: a direct weapon button, shooter#127; a leg
    /// strides; anything else turns at a rate) on an unused key, selected and waiting for its
    /// key.
    ///
    /// # Errors
    ///
    /// If `part` is no actuator or gun, or the layout is full.
    pub fn add(&mut self, part: Option<PartId>) -> Result<(), Refusal> {
        let part = part.ok_or(Refusal::Nothing)?;
        let leg = self.actuators.actuators.iter().any(|actuator| {
            actuator.part == part
                && actuator.kind
                    == shooter_sim::mechanics::Actuator::Locomotion(
                        shooter_sim::robot::Locomotion::Leg,
                    )
        });
        let gun = self.actuators.guns.contains(&part);
        let used: Vec<Source> = self.layout.bindings.iter().map(|b| b.source).collect();
        let free = (0..KEYS.len())
            .filter_map(|index| u8::try_from(index).ok())
            .map(Source::Key)
            .find(|source| !used.contains(source))
            .unwrap_or(Source::Key(0));
        self.edit(|layout| {
            if layout.bindings.len() >= MAX_BINDINGS {
                return Err(Refusal::Binding(
                    shooter_sim::bindings::BindingError::TooMany,
                ));
            }
            layout.bindings.push(Binding {
                source: free,
                mode: Mode::Hold,
                output: if gun {
                    Output::Fire
                } else if leg {
                    Output::Stride
                } else {
                    Output::Rate
                },
                invert: false,
                strength: 100,
                targets: vec![Target {
                    part,
                    invert: false,
                }],
            });
            Ok(())
        })?;
        self.selected = Some(self.layout.bindings.len() - 1);
        self.capturing = true;
        Ok(())
    }

    /// Removes the selected binding.
    ///
    /// # Errors
    ///
    /// If none is selected.
    pub fn delete(&mut self) -> Result<(), Refusal> {
        let index = self.selected.ok_or(Refusal::Nothing)?;
        self.edit(|layout| {
            if index >= layout.bindings.len() {
                return Err(Refusal::Nothing);
            }
            layout.bindings.remove(index);
            Ok(())
        })?;
        self.selected = None;
        self.capturing = false;
        Ok(())
    }

    /// Binds the selected binding to the key with W3C code `code`, held (or toggled, if it
    /// was), and stops waiting for a key.
    ///
    /// # Errors
    ///
    /// If none is selected or the key is not one a binding may name.
    pub fn bind_key(&mut self, code: &str) -> Result<(), Refusal> {
        self.capturing = false;
        let index = key(code).ok_or(Refusal::UnboundKey)?;
        self.with_selected(|binding| {
            binding.source = Source::Key(index);
            if !matches!(binding.mode, Mode::Hold | Mode::Toggle) {
                binding.mode = Mode::Hold;
            }
            Ok(())
        })
    }

    /// Moves the selected binding to the next mouse input: the horizontal movement, the vertical
    /// one, then each button, then back to the horizontal movement (a fire binding: the next
    /// button).
    ///
    /// # Errors
    ///
    /// If none is selected.
    pub fn cycle_mouse(&mut self) -> Result<(), Refusal> {
        self.capturing = false;
        self.with_selected(|binding| {
            binding.source = match binding.source {
                // A fire binding takes buttons only.
                Source::Button(index) if binding.output == Output::Fire => {
                    Source::Button((index + 1) % MOUSE_BUTTONS)
                }
                _ if binding.output == Output::Fire => Source::Button(0),
                Source::MouseX => Source::MouseY,
                Source::MouseY => Source::Button(0),
                Source::Button(index) if index + 1 < MOUSE_BUTTONS => Source::Button(index + 1),
                Source::Button(_) | Source::Key(_) => Source::MouseX,
            };
            let axis = matches!(binding.source, Source::MouseX | Source::MouseY);
            binding.mode = match (axis, binding.mode) {
                (true, Mode::Hold | Mode::Toggle) => Mode::Offset,
                (false, Mode::Relative | Mode::Offset) => Mode::Hold,
                (_, mode) => mode,
            };
            Ok(())
        })
    }

    /// Hold and toggle for a key or button, relative and offset for a mouse axis.
    ///
    /// # Errors
    ///
    /// If none is selected.
    pub fn cycle_mode(&mut self) -> Result<(), Refusal> {
        self.with_selected(|binding| {
            binding.mode = match binding.mode {
                Mode::Hold => Mode::Toggle,
                Mode::Toggle => Mode::Hold,
                Mode::Relative => Mode::Offset,
                Mode::Offset => Mode::Relative,
            };
            Ok(())
        })
    }

    /// The next kind of command (stride, rate, angle, fire) the selected binding's parts all take.
    ///
    /// # Errors
    ///
    /// If none is selected, or none its parts all take.
    pub fn cycle_output(&mut self) -> Result<(), Refusal> {
        let index = self.selected.ok_or(Refusal::Nothing)?;
        let now = self
            .layout
            .bindings
            .get(index)
            .ok_or(Refusal::Nothing)?
            .output;
        let order = [Output::Stride, Output::Rate, Output::Angle, Output::Fire];
        let at = order.iter().position(|output| *output == now).unwrap_or(0);
        let mut last = Err(Refusal::Nothing);
        for step in 1..order.len() {
            let next = order[(at + step) % order.len()];
            last = self.with_selected(|binding| {
                binding.output = next;
                Ok(())
            });
            if last.is_ok() {
                return last;
            }
        }
        last
    }

    /// Inverts the selected binding.
    ///
    /// # Errors
    ///
    /// If none is selected.
    pub fn invert(&mut self) -> Result<(), Refusal> {
        self.with_selected(|binding| {
            binding.invert = !binding.invert;
            Ok(())
        })
    }

    /// Changes the selected binding's strength (or sensitivity) by `step` percent, within 1 to
    /// 100.
    ///
    /// # Errors
    ///
    /// If none is selected.
    pub fn strength(&mut self, step: i32) -> Result<(), Refusal> {
        self.with_selected(|binding| {
            binding.strength =
                u8::try_from((i32::from(binding.strength) + step).clamp(1, 100)).unwrap_or(100);
            Ok(())
        })
    }

    /// Puts `part` in the selected binding's group, or takes it out if it is there.
    ///
    /// # Errors
    ///
    /// If nothing is selected, the part is no actuator (or does not take the binding's kind of
    /// command), the group is full, or it would leave the group empty.
    pub fn toggle_part(&mut self, part: Option<PartId>) -> Result<(), Refusal> {
        let part = part.ok_or(Refusal::Nothing)?;
        self.with_selected(|binding| {
            if let Some(at) = binding.targets.iter().position(|t| t.part == part) {
                binding.targets.remove(at);
            } else {
                binding.targets.push(Target {
                    part,
                    invert: false,
                });
            }
            Ok(())
        })
    }

    /// Reverses `part`'s share of the selected binding (one side of a differential drive).
    ///
    /// # Errors
    ///
    /// If nothing is selected or the part is not in its group.
    pub fn flip_part(&mut self, part: Option<PartId>) -> Result<(), Refusal> {
        let part = part.ok_or(Refusal::Nothing)?;
        self.with_selected(|binding| {
            let target = binding
                .targets
                .iter_mut()
                .find(|target| target.part == part)
                .ok_or(Refusal::Nothing)?;
            target.invert = !target.invert;
            Ok(())
        })
    }

    /// The legs' next gait: suggested, biped, quadruped, all at once.
    pub fn cycle_gait(&mut self) {
        let next = match self.layout.gait {
            GaitChoice::Suggested => GaitChoice::Preset(Gait::Biped),
            GaitChoice::Preset(Gait::Biped) => GaitChoice::Preset(Gait::Quadruped),
            GaitChoice::Preset(Gait::Quadruped) => GaitChoice::Unison,
            GaitChoice::Unison => GaitChoice::Suggested,
        };
        let _ = self.edit(|layout| {
            layout.gait = next;
            Ok(())
        });
    }
}

/// A binding's input as the panel names it.
#[must_use]
pub fn source_name(source: Source) -> String {
    match source {
        Source::Key(index) => KEYS.get(usize::from(index)).map_or_else(
            || "?".to_owned(),
            |code| {
                code.strip_prefix("Key")
                    .or_else(|| code.strip_prefix("Digit"))
                    .or_else(|| code.strip_prefix("Arrow"))
                    .unwrap_or(code)
                    .to_uppercase()
            },
        ),
        Source::Button(index) => format!("MOUSE {}", index + 1),
        Source::MouseX => "MOUSE X".to_owned(),
        Source::MouseY => "MOUSE Y".to_owned(),
    }
}

/// A binding's mode and output as the panel names them.
#[must_use]
pub const fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Hold => "HOLD",
        Mode::Toggle => "TOGGLE",
        Mode::Relative => "RELATIVE",
        Mode::Offset => "OFFSET",
    }
}

#[must_use]
pub const fn output_name(output: Output) -> &'static str {
    match output {
        Output::Stride => "STRIDE",
        Output::Rate => "RATE",
        Output::Angle => "ANGLE",
        Output::Fire => "FIRE",
    }
}

/// The gait choice as the panel names it.
#[must_use]
pub const fn gait_name(gait: GaitChoice) -> &'static str {
    match gait {
        GaitChoice::Suggested => "SUGGESTED",
        GaitChoice::Preset(Gait::Biped) => "BIPED",
        GaitChoice::Preset(Gait::Quadruped) => "QUADRUPED",
        GaitChoice::Unison => "ALL AT ONCE",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shooter_sim::bindings::{BindingError, BindingState, Pressed};
    use shooter_sim::mechanics::fixtures;
    use shooter_sim::robot::{HEAVY, STANDARD};

    #[test]
    fn the_preset_follows_the_build_until_the_person_edits_it() {
        let mut bench = ControlsBench::new(STANDARD, None);
        assert!(!bench.custom());
        assert_eq!(bench.layout(), &Bindings::preset_of(&STANDARD));
        bench.follow(&fixtures::car());
        assert_eq!(bench.layout(), &Bindings::preset_of(&fixtures::car()));
        // An edit makes it the person's: a new build only prunes it.
        bench.selected = Some(0);
        bench.invert().unwrap();
        assert!(bench.custom());
        bench.follow(&STANDARD);
        assert!(bench.layout().check(&Layout::of(&STANDARD)).is_ok());
        assert_ne!(bench.layout(), &Bindings::preset_of(&STANDARD));
        bench.preset();
        assert_eq!(bench.layout(), &Bindings::preset_of(&STANDARD));
        // A layout brought in that fits is kept, one that does not is replaced by the preset.
        let car = Bindings::preset_of(&fixtures::car());
        assert_eq!(
            ControlsBench::new(fixtures::car(), Some(&car)).layout(),
            &car
        );
        assert_eq!(
            ControlsBench::new(HEAVY, Some(&car)).layout(),
            &Bindings::preset_of(&HEAVY)
        );
    }

    #[test]
    fn a_new_binding_drives_the_chosen_part_on_the_key_pressed() {
        let mut bench = ControlsBench::new(STANDARD, None);
        // The chassis is no actuator.
        assert_eq!(
            bench.add(Some(PartId(0))),
            Err(Refusal::Binding(BindingError::NotAnActuator(5, PartId(0))))
        );
        bench.add(Some(PartId(5))).unwrap();
        assert!(bench.capturing);
        bench.bind_key("KeyJ").unwrap();
        assert!(!bench.capturing);
        // Both legs on J, the right one reversed, toggled: a turn on the spot that stays on.
        bench.toggle_part(Some(PartId(6))).unwrap();
        bench.flip_part(Some(PartId(6))).unwrap();
        bench.cycle_mode().unwrap();
        bench.strength(-50).unwrap();
        let actuators = Layout::of(&STANDARD);
        let mut state = BindingState::default();
        let pressed = Pressed::default().with_key("KeyJ");
        let motors = bench.layout().resolve(&actuators, &mut state, pressed);
        assert_eq!(
            motors.get(PartId(5)),
            Some(shooter_sim::mechanics::Command::Stride(50))
        );
        assert_eq!(
            motors.get(PartId(6)),
            Some(shooter_sim::mechanics::Command::Stride(-50))
        );
        // The output cycles to what legs take; a mouse axis steers by offset.
        bench.cycle_output().unwrap();
        assert_eq!(bench.layout().bindings[5].output, Output::Rate);
        bench.cycle_mouse().unwrap();
        assert_eq!(bench.layout().bindings[5].source, Source::MouseX);
        assert_eq!(bench.layout().bindings[5].mode, Mode::Offset);
        assert_eq!(bench.bind_key("Escape"), Err(Refusal::UnboundKey));
        bench.delete().unwrap();
        assert_eq!(bench.layout().bindings.len(), 5);
    }

    /// A gun clicked takes a direct weapon button; a second gun added makes it a weapon group
    /// that overlaps the preset's every-gun group (shooter#127).
    #[test]
    fn a_gun_gets_a_direct_button_and_groups_with_others() {
        let mut bench = ControlsBench::new(shooter_sim::robot::TWIN_RAIL, None);
        let preset = bench.layout().bindings.len();
        bench.add(Some(PartId(7))).unwrap();
        bench.bind_key("KeyZ").unwrap();
        let binding = &bench.layout().bindings[preset];
        assert_eq!(binding.output, Output::Fire);
        // No other command fits a gun, and no actuator joins a fire binding.
        assert!(bench.cycle_output().is_err());
        assert!(bench.toggle_part(Some(PartId(0))).is_err());
        // Only buttons for the mouse.
        bench.cycle_mouse().unwrap();
        assert_eq!(bench.layout().bindings[preset].source, Source::Button(0));
        bench.bind_key("KeyZ").unwrap();
        bench.toggle_part(Some(PartId(9))).unwrap();
        let actuators = Layout::of(&shooter_sim::robot::TWIN_RAIL);
        let mut state = BindingState::default();
        let pressed = Pressed {
            buttons: 1 << shooter_sim::bindings::ALL_GUNS_BUTTON,
            ..Pressed::default().with_key("KeyZ")
        };
        let fire = bench.layout().act(&actuators, &mut state, pressed).triggers;
        assert_eq!(fire.parts(), [PartId(7), PartId(9)]);
        // The blade is no gun.
        assert!(bench.toggle_part(Some(PartId(8))).is_err());
    }

    #[test]
    fn the_output_cycles_through_the_kinds_every_part_takes() {
        // The car's W binding turns its wheels at a rate; a wheel takes no stride, so the next
        // kinds are an angle, then (past the stride) a rate again.
        let mut bench = ControlsBench::new(fixtures::car(), None);
        bench.selected = Some(0);
        assert_eq!(bench.layout().bindings[0].output, Output::Rate);
        bench.cycle_output().unwrap();
        assert_eq!(bench.layout().bindings[0].output, Output::Angle);
        bench.cycle_output().unwrap();
        assert_eq!(bench.layout().bindings[0].output, Output::Rate);
    }
}
