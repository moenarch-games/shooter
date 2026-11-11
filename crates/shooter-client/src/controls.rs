//! Movement controls over the shared `input-bindings` runtime: the shooter's action catalog
//! (keyboard and gamepad defaults), physical strokes in, held semantic actions out. Binding
//! resolution and held-state release belong to `input_bindings_core`.
//!
//! Look is analog and stays outside the runtime (input-bindings' Rust core has no analog axis
//! runtime yet): mouse deltas and the right stick turn the camera directly.

use input_bindings_core::{
    ActionDefinition, ActionRegistry, AxisDirection, Binding, DeviceClass, DeviceStroke,
    InputRuntime, InputRuntimeOptions, InputStroke, KeyMatch, KeyStroke, Modifiers, Provenance,
    RepeatPolicy, RuntimeDecision, SemanticControlState, WhenExpr,
};
use std::collections::BTreeMap;
use winit::keyboard::KeyCode;

pub const MOVE_FORWARD: &str = "move.forward";
pub const MOVE_BACK: &str = "move.back";
pub const MOVE_LEFT: &str = "move.strafeLeft";
pub const MOVE_RIGHT: &str = "move.strafeRight";
pub const JUMP: &str = "move.jump";
pub const SPRINT: &str = "move.sprint";
pub const CROUCH: &str = "move.crouch";

/// W3C standard gamepad layout indices.
pub mod standard {
    pub const SOUTH: u16 = 0;
    pub const BUTTONS: usize = 17;
    pub const LEFT_X: u8 = 0;
    pub const LEFT_Y: u8 = 1;
    pub const RIGHT_X: usize = 2;
    pub const RIGHT_Y: usize = 3;
    pub const AXES: usize = 4;
}

/// The runtime needs no clock: the catalog has no chords.
const NO_CLOCK_MS: u64 = 0;

fn key(code: &str) -> InputStroke {
    InputStroke::Keyboard(KeyStroke {
        key: KeyMatch::Physical {
            value: code.to_owned(),
        },
        modifiers: Modifiers::default(),
    })
}

fn pad_button(button: u16) -> InputStroke {
    InputStroke::Device(DeviceStroke::GamepadButton {
        button,
        threshold: 50,
        gamepad: None,
    })
}

fn stick(axis: u8, direction: AxisDirection) -> InputStroke {
    InputStroke::Device(DeviceStroke::GamepadAxis {
        axis,
        direction,
        threshold: 50,
        deadzone: 20,
        gamepad: None,
    })
}

/// The shooter's movement catalog: id, title and default strokes.
#[must_use]
pub fn action_registry() -> ActionRegistry {
    use AxisDirection::{Negative, Positive};
    use standard::{LEFT_X, LEFT_Y, SOUTH};
    let actions: [(&str, &str, Vec<InputStroke>); 7] = [
        (
            MOVE_FORWARD,
            "Move forward",
            vec![key("KeyW"), key("ArrowUp"), stick(LEFT_Y, Negative)],
        ),
        (
            MOVE_BACK,
            "Move backward",
            vec![key("KeyS"), key("ArrowDown"), stick(LEFT_Y, Positive)],
        ),
        (
            MOVE_LEFT,
            "Strafe left",
            vec![key("KeyA"), key("ArrowLeft"), stick(LEFT_X, Negative)],
        ),
        (
            MOVE_RIGHT,
            "Strafe right",
            vec![key("KeyD"), key("ArrowRight"), stick(LEFT_X, Positive)],
        ),
        (JUMP, "Jump", vec![key("Space"), pad_button(SOUTH)]),
        // Mouse and keyboard first (Atlas D3.2): no pad defaults for the new moves yet.
        (SPRINT, "Sprint", vec![key("ShiftLeft")]),
        // C, not Control: Control+W closes the browser tab and pages cannot stop it.
        (CROUCH, "Crouch / slide", vec![key("KeyC")]),
    ];
    ActionRegistry {
        actions: actions
            .into_iter()
            .map(|(id, title, strokes)| ActionDefinition {
                id: id.to_owned(),
                title: title.to_owned(),
                description: None,
                category_path: vec!["Movement".to_owned()],
                repeat_policy: RepeatPolicy::Never,
                allowed_devices: vec![DeviceClass::Keyboard, DeviceClass::Gamepad],
                defaults: strokes
                    .into_iter()
                    .enumerate()
                    .map(|(index, stroke)| Binding {
                        id: format!("{id}.{index}"),
                        action: id.to_owned(),
                        sequence: vec![stroke],
                        when: WhenExpr::Always,
                        priority: 0,
                    })
                    .collect(),
                provenance: Some(Provenance {
                    source: "shooter".to_owned(),
                    version: Some("1".to_owned()),
                }),
            })
            .collect(),
    }
}

/// Every connected pad merged: the strongest reading of each standard button and axis.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PadState {
    pub buttons: [f32; standard::BUTTONS],
    /// Standard axes: +X right, +Y down.
    pub axes: [f32; standard::AXES],
}

impl PadState {
    fn holds(&self, stroke: &InputStroke) -> bool {
        match stroke {
            InputStroke::Device(DeviceStroke::GamepadButton {
                button, threshold, ..
            }) => self
                .buttons
                .get(usize::from(*button))
                .is_some_and(|value| *value >= f32::from(*threshold) / 100.0),
            InputStroke::Device(DeviceStroke::GamepadAxis {
                axis,
                direction,
                threshold,
                ..
            }) => self.axes.get(usize::from(*axis)).is_some_and(|value| {
                let signed = match direction {
                    AxisDirection::Positive => *value,
                    AxisDirection::Negative => -*value,
                };
                signed >= f32::from(*threshold) / 100.0
            }),
            _ => false,
        }
    }
}

/// Held movement actions from keyboard and gamepad.
#[derive(Clone, Debug)]
pub struct Controls {
    runtime: InputRuntime,
    state: SemanticControlState,
    /// Each pad stroke the catalog binds, and whether it is down.
    pad_strokes: Vec<(InputStroke, bool)>,
    keys: BTreeMap<KeyCode, InputStroke>,
}

impl Default for Controls {
    fn default() -> Self {
        Self::new()
    }
}

impl Controls {
    /// # Panics
    ///
    /// If the catalog does not validate.
    #[must_use]
    pub fn new() -> Self {
        let registry = action_registry();
        let mut pad_strokes: Vec<(InputStroke, bool)> = Vec::new();
        for binding in registry.actions.iter().flat_map(|action| &action.defaults) {
            for stroke in &binding.sequence {
                if matches!(stroke, InputStroke::Device(_))
                    && !pad_strokes.iter().any(|(known, _)| known == stroke)
                {
                    pad_strokes.push((stroke.clone(), false));
                }
            }
        }
        let runtime = InputRuntime::new(registry, None, InputRuntimeOptions::default());
        let report = runtime.validation_report();
        assert!(
            report.valid && report.diagnostics.is_empty(),
            "the shooter control catalog is invalid: {:?}",
            report.diagnostics
        );
        Self {
            runtime,
            state: SemanticControlState::new(),
            pad_strokes,
            keys: BTreeMap::new(),
        }
    }

    fn apply(&mut self, decisions: &[RuntimeDecision]) {
        self.state.apply_decisions(decisions);
    }

    /// winit's `KeyCode` names are the W3C physical key codes.
    pub fn key_down(&mut self, code: KeyCode, repeat: bool) {
        let stroke = self
            .keys
            .entry(code)
            .or_insert_with(|| key(&format!("{code:?}")))
            .clone();
        let decisions = self.runtime.input_down(stroke, repeat, NO_CLOCK_MS);
        self.apply(&decisions);
    }

    pub fn key_up(&mut self, code: KeyCode) {
        let stroke = self
            .keys
            .remove(&code)
            .unwrap_or_else(|| key(&format!("{code:?}")));
        let decisions = self.runtime.input_up(&stroke, NO_CLOCK_MS);
        self.apply(&decisions);
    }

    /// Presses and releases the pad strokes whose state changed since the last poll.
    pub fn poll_pads(&mut self, pads: &PadState) {
        for index in 0..self.pad_strokes.len() {
            let (stroke, was_down) = self.pad_strokes[index].clone();
            let down = pads.holds(&stroke);
            if down == was_down {
                continue;
            }
            self.pad_strokes[index].1 = down;
            let decisions = if down {
                self.runtime.input_down(stroke, false, NO_CLOCK_MS)
            } else {
                self.runtime.input_up(&stroke, NO_CLOCK_MS)
            };
            self.apply(&decisions);
        }
    }

    /// Focus left the game: every held action lets go.
    pub fn release_all(&mut self) {
        self.keys.clear();
        for (_, down) in &mut self.pad_strokes {
            *down = false;
        }
        let decision = self.runtime.reset("focusLost");
        self.apply(&[decision]);
    }

    /// (forward, strafe) in -1..=1.
    #[must_use]
    pub fn movement(&self) -> (i8, i8) {
        (
            self.state.axis(MOVE_BACK, MOVE_FORWARD),
            self.state.axis(MOVE_LEFT, MOVE_RIGHT),
        )
    }

    #[must_use]
    pub fn jump_held(&self) -> bool {
        self.state.is_held(JUMP)
    }

    #[must_use]
    pub fn sprint_held(&self) -> bool {
        self.state.is_held(SPRINT)
    }

    #[must_use]
    pub fn crouch_held(&self) -> bool {
        self.state.is_held(CROUCH)
    }
}

/// The first-person view direction, turned by mouse and right stick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    /// Radians; 0 faces +Z and growing yaw turns toward +X (as `sim_foundation::trig`).
    pub yaw: f32,
    /// Radians, up positive, within ±89°.
    pub pitch: f32,
}

/// Radians per mouse count.
pub const MOUSE_SENSITIVITY: f32 = 0.0022;
/// Radians per second at full right-stick deflection.
pub const STICK_TURN_RATE: f32 = 3.2;
const STICK_DEADZONE: f32 = 0.15;
const PITCH_LIMIT: f32 = 89.0 * std::f32::consts::PI / 180.0;

impl Look {
    #[must_use]
    pub const fn new(yaw: f32) -> Self {
        Self { yaw, pitch: 0.0 }
    }

    /// Mouse motion in counts: right turns right (yaw falls), up looks up.
    pub fn mouse(&mut self, dx: f64, dy: f64) {
        #[allow(clippy::cast_possible_truncation)]
        self.turn(dx as f32 * MOUSE_SENSITIVITY, dy as f32 * MOUSE_SENSITIVITY);
    }

    /// The right stick held for `seconds`.
    pub fn stick(&mut self, pads: &PadState, seconds: f32) {
        let shape = |value: f32| {
            if value.abs() < STICK_DEADZONE {
                0.0
            } else {
                value.signum() * ((value.abs() - STICK_DEADZONE) / (1.0 - STICK_DEADZONE)).powi(2)
            }
        };
        let rate = STICK_TURN_RATE * seconds;
        self.turn(
            shape(pads.axes[standard::RIGHT_X]) * rate,
            shape(pads.axes[standard::RIGHT_Y]) * rate,
        );
    }

    fn turn(&mut self, right: f32, down: f32) {
        self.yaw = (self.yaw - right).rem_euclid(std::f32::consts::TAU);
        self.pitch = (self.pitch - down).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    }

    /// The yaw as the simulation's `u16` steps.
    #[must_use]
    pub fn sim_yaw(&self) -> u16 {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let steps = (self.yaw / std::f32::consts::TAU * 65_536.0).round() as i64;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let wrapped = steps.rem_euclid(65_536) as u16;
        wrapped
    }

    /// Unit view direction (x, y, z).
    #[must_use]
    pub fn forward(&self) -> [f32; 3] {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        [sy * cp, sp, cy * cp]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalog_validates_and_keys_move() {
        let mut controls = Controls::new();
        controls.key_down(KeyCode::KeyW, false);
        controls.key_down(KeyCode::KeyD, false);
        assert_eq!(controls.movement(), (1, 1));
        controls.key_up(KeyCode::KeyW);
        assert_eq!(controls.movement(), (0, 1));
        controls.release_all();
        assert_eq!(controls.movement(), (0, 0));
    }

    #[test]
    fn shift_sprints_and_c_crouches() {
        let mut controls = Controls::new();
        controls.key_down(KeyCode::ShiftLeft, false);
        controls.key_down(KeyCode::KeyC, false);
        assert!(controls.sprint_held() && controls.crouch_held());
        controls.key_up(KeyCode::KeyC);
        assert!(!controls.crouch_held());
        assert!(controls.sprint_held());
    }

    #[test]
    fn the_pad_moves_and_jumps() {
        let mut controls = Controls::new();
        let mut pads = PadState::default();
        pads.axes[usize::from(standard::LEFT_Y)] = -0.9;
        pads.buttons[usize::from(standard::SOUTH)] = 1.0;
        controls.poll_pads(&pads);
        assert_eq!(controls.movement(), (1, 0));
        assert!(controls.jump_held());
        controls.poll_pads(&PadState::default());
        assert_eq!(controls.movement(), (0, 0));
        assert!(!controls.jump_held());
    }

    #[test]
    fn look_yaw_matches_the_simulation() {
        let look = Look::new(std::f32::consts::PI);
        assert_eq!(look.sim_yaw(), 32_768);
        let [x, _, z] = Look::new(std::f32::consts::FRAC_PI_2).forward();
        assert!((x - 1.0).abs() < 1e-6 && z.abs() < 1e-6);
    }
}
