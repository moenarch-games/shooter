//! Touch controls for phones and tablets: the page draws a move stick, a look pad and buttons
//! over the canvas (`web/index.html`) and keeps what they hold in `window.shooterTouch`; each
//! frame the client reads it ([`crate::web::touch`]) and presses and releases the same keys and
//! mouse buttons a keyboard and mouse would ([`Touch::update`]), so the control layout, the
//! weapon cycle and the host's checks see nothing new. The look pad's drag turns the camera like
//! pointer-locked mouse movement.

use winit::keyboard::KeyCode;

/// The page's button bits in `window.shooterTouch.held`.
pub mod bit {
    pub const FIRE: u32 = 1;
    pub const JUMP: u32 = 1 << 1;
    pub const SPRINT: u32 = 1 << 2;
    pub const CROUCH: u32 = 1 << 3;
    pub const MELEE: u32 = 1 << 4;
    pub const GRAPPLE: u32 = 1 << 5;
    pub const NEXT: u32 = 1 << 6;
    pub const PREVIOUS: u32 = 1 << 7;
    pub const REPAIR: u32 = 1 << 8;
    pub const BOARD: u32 = 1 << 9;
    pub const EDIT: u32 = 1 << 10;
}

/// The keys the buttons hold, by bit.
const KEYS: [(u32, KeyCode); 8] = [
    (bit::JUMP, KeyCode::Space),
    (bit::SPRINT, KeyCode::ShiftLeft),
    (bit::CROUCH, KeyCode::KeyC),
    (bit::MELEE, KeyCode::KeyV),
    (bit::GRAPPLE, KeyCode::KeyF),
    (bit::NEXT, KeyCode::KeyE),
    (bit::PREVIOUS, KeyCode::KeyQ),
    (bit::REPAIR, KeyCode::KeyR),
];

/// How far the move stick leans (of its radius) before it holds a direction's key.
const STICK_THRESHOLD: f32 = 0.35;

/// Mouse counts per CSS pixel the look pad is dragged: a phone's width turns the camera about
/// half round.
pub const LOOK_COUNTS_PER_PIXEL: f64 = 3.5;

/// One frame of the page's touch state.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Frame {
    /// The touch controls are playing (the page shows them and the player tapped to play).
    pub on: bool,
    /// The move stick, right and down positive, each within -1..=1.
    pub stick: [f32; 2],
    /// CSS pixels the look pad was dragged since the last frame.
    pub look: [f64; 2],
    /// The held buttons' [`bit`]s.
    pub held: u32,
}

impl Frame {
    /// The keys this frame holds: the stick's directions and the buttons'.
    #[must_use]
    pub fn keys(&self) -> Vec<KeyCode> {
        let [x, y] = self.stick;
        let mut keys: Vec<KeyCode> = [
            (y < -STICK_THRESHOLD, KeyCode::KeyW),
            (y > STICK_THRESHOLD, KeyCode::KeyS),
            (x < -STICK_THRESHOLD, KeyCode::KeyA),
            (x > STICK_THRESHOLD, KeyCode::KeyD),
        ]
        .into_iter()
        .filter_map(|(held, key)| held.then_some(key))
        .collect();
        keys.extend(
            KEYS.iter()
                .filter(|(bit, _)| self.held & bit != 0)
                .map(|(_, key)| *key),
        );
        keys
    }
}

/// What changed since the last frame, for the client to press and release.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Changes {
    pub pressed: Vec<KeyCode>,
    pub released: Vec<KeyCode>,
    /// The fire button went down (`Some(true)`) or up (`Some(false)`): the primary mouse button.
    pub fire: Option<bool>,
    /// The edit button was tapped: Edit Robot, when dead.
    pub edit: bool,
}

/// The touch state already applied: the keys and buttons the client holds for the page.
#[derive(Debug, Default)]
pub struct Touch {
    on: bool,
    keys: Vec<KeyCode>,
    held: u32,
}

impl Touch {
    /// Whether the touch controls were playing at the last frame.
    #[must_use]
    pub const fn on(&self) -> bool {
        self.on
    }

    /// The presses and releases from the last frame applied to `frame`.
    pub fn update(&mut self, frame: &Frame) -> Changes {
        let keys = if frame.on { frame.keys() } else { Vec::new() };
        let held = if frame.on { frame.held } else { 0 };
        let changes = Changes {
            pressed: keys
                .iter()
                .filter(|key| !self.keys.contains(key))
                .copied()
                .collect(),
            released: self
                .keys
                .iter()
                .filter(|key| !keys.contains(key))
                .copied()
                .collect(),
            fire: ((held ^ self.held) & bit::FIRE != 0).then_some(held & bit::FIRE != 0),
            edit: held & !self.held & bit::EDIT != 0,
        };
        self.on = frame.on;
        self.keys = keys;
        self.held = held;
        changes
    }

    /// Whether the scoreboard button is held.
    #[must_use]
    pub const fn board(&self) -> bool {
        self.held & bit::BOARD != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stick_holds_its_directions_past_the_threshold() {
        let frame = Frame {
            on: true,
            stick: [0.8, -0.5],
            ..Frame::default()
        };
        assert_eq!(frame.keys(), vec![KeyCode::KeyW, KeyCode::KeyD]);
        let frame = Frame {
            stick: [0.2, 0.3],
            ..frame
        };
        assert!(frame.keys().is_empty());
    }

    #[test]
    fn buttons_press_once_and_release_when_let_go_or_off() {
        let mut touch = Touch::default();
        let jump = Frame {
            on: true,
            held: bit::JUMP | bit::FIRE,
            ..Frame::default()
        };
        let changes = touch.update(&jump);
        assert_eq!(changes.pressed, vec![KeyCode::Space]);
        assert_eq!(changes.fire, Some(true));
        assert_eq!(touch.update(&jump), Changes::default());
        let changes = touch.update(&Frame::default());
        assert_eq!(changes.released, vec![KeyCode::Space]);
        assert_eq!(changes.fire, Some(false));
    }

    #[test]
    fn edit_is_a_tap() {
        let mut touch = Touch::default();
        let edit = Frame {
            on: true,
            held: bit::EDIT,
            ..Frame::default()
        };
        assert!(touch.update(&edit).edit);
        assert!(!touch.update(&edit).edit);
    }
}
