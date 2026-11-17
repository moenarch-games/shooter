//! Movement and weapon controls over the shared `input-bindings` runtime: the shooter's action
//! catalog (keyboard, mouse and gamepad defaults), physical strokes in, held semantic actions
//! out. Binding
//! resolution and held-state release belong to `input_bindings_core`.
//!
//! Look is analog and stays outside the runtime (input-bindings' Rust core has no analog axis
//! runtime yet): mouse deltas and the right stick turn the camera directly.

use input_bindings_core::{
    ActionDefinition, ActionRegistry, AxisDirection, Binding, DeviceClass, DeviceStroke,
    InputRuntime, InputRuntimeOptions, InputStroke, KeyMatch, KeyStroke, Modifiers, Provenance,
    RepeatPolicy, RuntimeActionPhase, RuntimeDecision, SemanticControlState, WheelDirection,
    WhenExpr,
};
use shooter_sim::Weapon;
use std::collections::BTreeMap;
use winit::keyboard::KeyCode;

pub const MOVE_FORWARD: &str = "move.forward";
pub const MOVE_BACK: &str = "move.back";
pub const MOVE_LEFT: &str = "move.strafeLeft";
pub const MOVE_RIGHT: &str = "move.strafeRight";
pub const JUMP: &str = "move.jump";
pub const SPRINT: &str = "move.sprint";
pub const CROUCH: &str = "move.crouch";
pub const FIRE: &str = "weapon.fire";
pub const MELEE: &str = "weapon.melee";
/// One step through the carried weapons ([`shooter_sim::weapons::cycled`]): wheel down or E...
pub const NEXT: &str = "weapon.next";
/// ...and back: wheel up or Q.
pub const PREVIOUS: &str = "weapon.previous";
/// Field repair (shooter#61): R opens the quick repair menu and, pressed again, repairs the
/// part chosen there (the wheel and Q/E move through it while it is open).
pub const REPAIR: &str = "robot.repair";
/// The grapple module (shooter#104, Atlas D2.i104.2): its own key, not a weapon in the cycle.
/// Held, the robot's mounted grapple hooks and reels; let go, the rope goes.
pub const GRAPPLE: &str = "robot.grapple";
/// Select actions, in [`Weapon::ROTATION`] order.
pub const SELECT: [&str; Weapon::ROTATION.len()] = [
    "weapon.energyRifle",
    "weapon.railGun",
    "weapon.rocketLauncher",
    "weapon.shotgun",
    "weapon.laser",
    "weapon.flamethrower",
];

/// W3C mouse button index of the primary (left) button...
pub const PRIMARY_BUTTON: u16 = 0;
/// ...of the secondary (right) button...
pub const SECONDARY_BUTTON: u16 = 2;
/// ...and of the thumb's back button.
pub const BACK_BUTTON: u16 = 3;

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

fn wheel(direction: WheelDirection) -> InputStroke {
    InputStroke::Device(DeviceStroke::Wheel {
        direction,
        modifiers: Modifiers::default(),
    })
}

fn mouse_button(button: u16) -> InputStroke {
    InputStroke::Device(DeviceStroke::MouseButton {
        button,
        modifiers: Modifiers::default(),
    })
}

/// The shooter's movement and weapon catalog: id, title and default strokes.
#[must_use]
pub fn action_registry() -> ActionRegistry {
    use AxisDirection::{Negative, Positive};
    use standard::{LEFT_X, LEFT_Y, SOUTH};
    let actions: [(&str, &str, Vec<InputStroke>); 19] = [
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
        (FIRE, "Fire", vec![mouse_button(PRIMARY_BUTTON)]),
        (
            MELEE,
            "Energy blade (quick melee)",
            vec![key("KeyV"), mouse_button(BACK_BUTTON)],
        ),
        (
            NEXT,
            "Next weapon",
            vec![wheel(WheelDirection::Down), key("KeyE")],
        ),
        (
            PREVIOUS,
            "Previous weapon",
            vec![wheel(WheelDirection::Up), key("KeyQ")],
        ),
        (SELECT[0], "Energy rifle", vec![key("Digit1")]),
        (SELECT[1], "Rail gun", vec![key("Digit2")]),
        (SELECT[2], "Rocket launcher", vec![key("Digit3")]),
        (SELECT[3], "Shotgun", vec![key("Digit4")]),
        (SELECT[4], "Laser", vec![key("Digit5")]),
        (SELECT[5], "Flamethrower", vec![key("Digit6")]),
        (
            GRAPPLE,
            "Grapple hook (mounted module)",
            vec![mouse_button(SECONDARY_BUTTON), key("KeyF")],
        ),
        (
            REPAIR,
            "Repair menu / repair the chosen part",
            vec![key("KeyR")],
        ),
    ];
    ActionRegistry {
        actions: actions
            .into_iter()
            .map(|(id, title, strokes)| ActionDefinition {
                id: id.to_owned(),
                title: title.to_owned(),
                description: None,
                category_path: vec![
                    if id.starts_with("weapon.") {
                        "Weapons"
                    } else if id.starts_with("robot.") {
                        "Robot"
                    } else {
                        "Movement"
                    }
                    .to_owned(),
                ],
                repeat_policy: RepeatPolicy::Never,
                allowed_devices: vec![
                    DeviceClass::Keyboard,
                    DeviceClass::Mouse,
                    DeviceClass::Gamepad,
                ],
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
    /// Weapon steps pressed since the last tick took them: + next, - previous.
    steps: i32,
    /// Repair presses since the last tick took them.
    repairs: u32,
    wheel: Notches,
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
                if matches!(
                    stroke,
                    InputStroke::Device(
                        DeviceStroke::GamepadButton { .. } | DeviceStroke::GamepadAxis { .. }
                    )
                ) && !pad_strokes.iter().any(|(known, _)| known == stroke)
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
            steps: 0,
            repairs: 0,
            wheel: Notches::default(),
        }
    }

    fn apply(&mut self, decisions: &[RuntimeDecision]) {
        self.state.apply_decisions(decisions);
        // Next and previous are presses, not holds: each one is a step.
        for dispatch in decisions.iter().flat_map(|decision| &decision.dispatches) {
            if dispatch.phase == RuntimeActionPhase::Press && !dispatch.repeat {
                match dispatch.action.as_str() {
                    NEXT => self.steps += 1,
                    PREVIOUS => self.steps -= 1,
                    REPAIR => self.repairs += 1,
                    _ => {}
                }
            }
        }
    }

    /// The mouse wheel turned by `delta` (winit's: positive y scrolls up, in lines or pixels)
    /// at `now_ms`: each notch ([`Notches`]) is a wheel stroke, pressed and released, so it
    /// resolves through the catalog like any other input.
    pub fn wheel(&mut self, delta: WheelDelta, now_ms: f64) {
        let Some(direction) = self.wheel.notch(delta, now_ms) else {
            return;
        };
        let stroke = wheel(direction);
        let mut decisions = self.runtime.input_down(stroke.clone(), false, NO_CLOCK_MS);
        decisions.extend(self.runtime.input_up(&stroke, NO_CLOCK_MS));
        self.apply(&decisions);
    }

    /// The weapon steps pressed since the last call: + next, - previous.
    pub fn take_steps(&mut self) -> i32 {
        std::mem::take(&mut self.steps)
    }

    /// The repair presses since the last call.
    pub fn take_repairs(&mut self) -> u32 {
        std::mem::take(&mut self.repairs)
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
        self.steps = 0;
        self.repairs = 0;
        self.wheel = Notches::default();
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

    /// A W3C mouse button went down (0 primary).
    pub fn mouse_down(&mut self, button: u16) {
        let decisions = self
            .runtime
            .input_down(mouse_button(button), false, NO_CLOCK_MS);
        self.apply(&decisions);
    }

    pub fn mouse_up(&mut self, button: u16) {
        let decisions = self.runtime.input_up(&mouse_button(button), NO_CLOCK_MS);
        self.apply(&decisions);
    }

    #[must_use]
    pub fn fire_held(&self) -> bool {
        self.state.is_held(FIRE)
    }

    #[must_use]
    pub fn melee_held(&self) -> bool {
        self.state.is_held(MELEE)
    }

    #[must_use]
    pub fn grapple_held(&self) -> bool {
        self.state.is_held(GRAPPLE)
    }

    /// The weapon whose select key is held, if any.
    #[must_use]
    pub fn selected(&self) -> Option<Weapon> {
        SELECT
            .iter()
            .zip(Weapon::ROTATION)
            .find(|(action, _)| self.state.is_held(action))
            .map(|(_, weapon)| weapon)
    }
}

/// One mouse wheel event's vertical turn, as winit reports it natively and in the browser:
/// positive scrolls up (away from the person).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WheelDelta {
    /// In lines: a classic wheel's notch is one (Firefox reports three).
    Lines(f32),
    /// In physical pixels: browsers and touchpads (a Chromium notch is 100 CSS pixels on
    /// Windows, about 53 on Linux).
    Pixels(f64),
}

/// The wheel travel a notch takes, in lines...
pub const LINES_PER_NOTCH: f32 = 1.0;
/// ...and in pixels.
pub const PIXELS_PER_NOTCH: f64 = 50.0;
/// After a notch, further turning the same way within this long is the same notch (a burst:
/// several events for one detent, a touchpad's or a smooth-scrolling wheel's stream).
pub const NOTCH_GUARD_MS: f64 = 60.0;
/// Wheel travel that rests this long without making a notch is forgotten.
const NOTCH_RESET_MS: f64 = 250.0;

/// Wheel events in, notches out: one step per notch, whatever the device's units or burst.
/// Travel adds up until it makes a notch (at most one an event); then, for
/// [`NOTCH_GUARD_MS`], turning the same way makes nothing more.
#[derive(Clone, Debug, Default)]
struct Notches {
    /// Travel since the last notch, in notches (positive up).
    travel: f64,
    last_event_ms: f64,
    last_notch: Option<(WheelDirection, f64)>,
}

impl Notches {
    fn notch(&mut self, delta: WheelDelta, now_ms: f64) -> Option<WheelDirection> {
        let turn = match delta {
            WheelDelta::Lines(lines) => f64::from(lines / LINES_PER_NOTCH),
            WheelDelta::Pixels(pixels) => pixels / PIXELS_PER_NOTCH,
        };
        if turn == 0.0 || !turn.is_finite() {
            return None;
        }
        let direction = if turn > 0.0 {
            WheelDirection::Up
        } else {
            WheelDirection::Down
        };
        let stale = now_ms - self.last_event_ms > NOTCH_RESET_MS;
        self.last_event_ms = now_ms;
        if stale || self.travel.signum() != turn.signum() {
            self.travel = 0.0;
        }
        if let Some((last, at)) = &self.last_notch
            && *last == direction
            && now_ms - at < NOTCH_GUARD_MS
        {
            self.travel = 0.0;
            return None;
        }
        self.travel += turn;
        if self.travel.abs() < 1.0 - 1e-6 {
            return None;
        }
        self.travel = 0.0;
        self.last_notch = Some((direction.clone(), now_ms));
        Some(direction)
    }
}

/// The weapon the local fighter asks to hold: a select key held, or where the next and previous
/// steps lead through the weapons it carries and still has ([`shooter_sim::weapons::cycled`]).
/// A step's target is asked for every tick for [`Selection::ASK_TICKS`] (inputs are unreliable
/// datagrams; asking for the weapon already held changes nothing), and steps taken meanwhile
/// count on from it, so quick notches add up however late the host's word comes. The host
/// decides: it refuses a weapon the robot does not have.
#[derive(Clone, Copy, Debug, Default)]
pub struct Selection {
    /// The step's target and the ticks it has been asked for.
    wanted: Option<(Weapon, u32)>,
}

impl Selection {
    /// Half a second: well over a round trip.
    pub const ASK_TICKS: u32 = 30;

    /// This tick's `select`: `direct` (a select key) if held, else the cycle's target `steps`
    /// on from the last target (or from `held`, the weapon held as predicted), with `armed`
    /// saying which weapons the build still has.
    pub fn intent(
        &mut self,
        direct: Option<Weapon>,
        steps: i32,
        held: Weapon,
        armed: impl Fn(Weapon) -> bool,
    ) -> Option<Weapon> {
        if direct.is_some() {
            self.wanted = None;
            return direct;
        }
        if steps != 0 {
            let from = self.wanted.map_or(held, |(weapon, _)| weapon);
            if let Some(target) = shooter_sim::weapons::cycled(from, steps, armed) {
                self.wanted = Some((target, 0));
            }
        }
        let (weapon, asked) = self.wanted.as_mut()?;
        *asked += 1;
        let weapon = *weapon;
        if *asked >= Self::ASK_TICKS {
            self.wanted = None;
        }
        Some(weapon)
    }
}

/// The aim: where the fighter looks and the third-person camera with it, turned by mouse and
/// right stick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    /// Radians; 0 faces +Z and growing yaw turns toward +X (as `shooter_sim::trig`).
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

    /// The pitch as the simulation's angle steps (a quarter turn is 16 384).
    #[must_use]
    pub fn sim_pitch(&self) -> i16 {
        #[allow(clippy::cast_possible_truncation)]
        let steps = (self.pitch / std::f32::consts::TAU * 65_536.0).round() as i16;
        steps
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
    fn the_mouse_fires_and_digits_select_weapons() {
        let mut controls = Controls::new();
        controls.mouse_down(PRIMARY_BUTTON);
        assert!(controls.fire_held());
        controls.mouse_up(PRIMARY_BUTTON);
        assert!(!controls.fire_held());
        assert_eq!(controls.selected(), None);
        controls.key_down(KeyCode::Digit3, false);
        assert_eq!(controls.selected(), Some(Weapon::RocketLauncher));
        controls.key_up(KeyCode::Digit3);
        controls.key_down(KeyCode::Digit2, false);
        assert_eq!(controls.selected(), Some(Weapon::RailGun));
    }

    #[test]
    fn v_and_the_thumb_button_swing_the_blade() {
        let mut controls = Controls::new();
        controls.key_down(KeyCode::KeyV, false);
        assert!(controls.melee_held());
        controls.key_up(KeyCode::KeyV);
        assert!(!controls.melee_held());
        controls.mouse_down(BACK_BUTTON);
        assert!(controls.melee_held());
        assert_eq!(controls.selected(), None);
    }

    #[test]
    fn the_right_button_and_f_hold_the_grapple_module() {
        let mut controls = Controls::new();
        controls.mouse_down(SECONDARY_BUTTON);
        assert!(controls.grapple_held() && !controls.fire_held());
        controls.mouse_up(SECONDARY_BUTTON);
        assert!(!controls.grapple_held());
        controls.key_down(KeyCode::KeyF, false);
        assert!(controls.grapple_held());
        // No select key makes it a held weapon.
        assert_eq!(controls.selected(), None);
        controls.key_up(KeyCode::KeyF);
        assert!(!controls.grapple_held());
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
    fn e_and_q_step_through_the_weapons_once_a_press() {
        let mut controls = Controls::new();
        controls.key_down(KeyCode::KeyE, false);
        // Held down, the key repeats: still one step.
        controls.key_down(KeyCode::KeyE, true);
        controls.key_up(KeyCode::KeyE);
        assert_eq!(controls.take_steps(), 1);
        assert_eq!(controls.take_steps(), 0);
        controls.key_down(KeyCode::KeyQ, false);
        controls.key_up(KeyCode::KeyQ);
        controls.key_down(KeyCode::KeyQ, false);
        controls.key_up(KeyCode::KeyQ);
        controls.key_down(KeyCode::KeyE, false);
        assert_eq!(controls.take_steps(), -1);
        assert_eq!(controls.selected(), None);
    }

    #[test]
    fn a_wheel_notch_is_one_step_in_lines_or_pixels_and_bursts_do_not_add_up() {
        let mut controls = Controls::new();
        // A classic wheel, notch by notch: down is next, up is previous.
        controls.wheel(WheelDelta::Lines(-1.0), 0.0);
        controls.wheel(WheelDelta::Lines(-1.0), 100.0);
        assert_eq!(controls.take_steps(), 2);
        controls.wheel(WheelDelta::Lines(1.0), 200.0);
        assert_eq!(controls.take_steps(), -1);
        // Firefox's three lines a notch, and a high-resolution wheel's fractions.
        controls.wheel(WheelDelta::Lines(-3.0), 400.0);
        assert_eq!(controls.take_steps(), 1);
        for (index, at) in [500.0, 510.0, 520.0].into_iter().enumerate() {
            controls.wheel(WheelDelta::Lines(-0.34), at);
            assert_eq!(controls.take_steps(), i32::from(index == 2));
        }
        // Pixels: a browser notch (100 or 53 CSS pixels) is one step...
        controls.wheel(WheelDelta::Pixels(-100.0), 1_000.0);
        controls.wheel(WheelDelta::Pixels(-53.0), 1_200.0);
        assert_eq!(controls.take_steps(), 2);
        // ...a burst of events for one notch (a smooth-scrolling wheel) is one...
        for at in 0..8 {
            controls.wheel(WheelDelta::Pixels(30.0), 1_400.0 + f64::from(at) * 5.0);
        }
        assert_eq!(controls.take_steps(), -1);
        // ...and a little touchpad drift that never makes a notch is forgotten.
        controls.wheel(WheelDelta::Pixels(-30.0), 2_000.0);
        controls.wheel(WheelDelta::Pixels(-30.0), 2_400.0);
        assert_eq!(controls.take_steps(), 0);
        controls.release_all();
        assert_eq!(controls.take_steps(), 0);
    }

    #[test]
    fn the_selection_cycles_the_carried_weapons_and_keeps_asking_for_a_while() {
        use Weapon::{EnergyBlade, EnergyRifle, Grapple, Laser, RailGun, Shotgun};
        // The grapple module is carried but never in the cycle.
        let standard = |weapon| {
            matches!(
                weapon,
                EnergyRifle | Grapple | Laser | Shotgun | EnergyBlade
            )
        };
        let mut selection = Selection::default();
        assert_eq!(selection.intent(None, 0, EnergyRifle, standard), None);
        assert_eq!(
            selection.intent(None, 1, EnergyRifle, standard),
            Some(Shotgun)
        );
        // A second notch before the switch shows counts on from the first one's target.
        assert_eq!(
            selection.intent(None, 1, EnergyRifle, standard),
            Some(Laser)
        );
        // Asked for until the host surely has it, then nothing.
        let mut asked = 1;
        while selection.intent(None, 0, Laser, standard).is_some() {
            asked += 1;
        }
        assert_eq!(asked, Selection::ASK_TICKS);
        // Wraps round; a select key wins over the cycle and ends it.
        assert_eq!(
            selection.intent(None, 1, Laser, standard),
            Some(EnergyRifle)
        );
        assert_eq!(
            selection.intent(Some(RailGun), 0, Shotgun, standard),
            Some(RailGun)
        );
        assert_eq!(selection.intent(None, 0, Shotgun, standard), None);
        // Nothing to cycle through: nothing asked.
        assert_eq!(selection.intent(None, 1, EnergyRifle, |_| false), None);
    }

    #[test]
    fn look_yaw_matches_the_simulation() {
        let look = Look::new(std::f32::consts::PI);
        assert_eq!(look.sim_yaw(), 32_768);
        let mut down = look;
        down.pitch = -std::f32::consts::FRAC_PI_4;
        assert_eq!(down.sim_pitch(), -8_192);
        let [x, _, z] = Look::new(std::f32::consts::FRAC_PI_2).forward();
        assert!((x - 1.0).abs() < 1e-6 && z.abs() < 1e-6);
    }
}
