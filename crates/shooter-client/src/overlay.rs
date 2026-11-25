//! The drawn HUD: the weapons and the chassis and energy, as render-engine screen overlay quads
//! ([`render_engine::scene::Scene::overlay`]), the same natively and in the browser.
//!
//! render-engine's overlay draws solid rectangles only, so the text is a 5×7 bitmap font and the
//! weapon silhouettes are pixel art, both drawn here as quads (each lit row run one quad). The
//! HUD has no panels: text and silhouettes float over the scene on a drop shadow, and bars sit on
//! a dark track. Pale grey for what matters, muted grey for the rest, olive for mods and ready
//! weapons, orange for the held weapon's key, a refire and a shot's cost, blue for energy and red
//! for what is lost or short. Everything sits in the bottom corners, clear of the reticle; the
//! layout is for 720 lines and scales with the frame's height.
//!
//! The bottom right shows a row of silhouettes, one for every weapon copy the build carries (its
//! select key, the blade's V, its copy number; lost ones dim red and struck out), each with its
//! own readiness bar (ready, recharging, short of energy: every copy may fire, shooter#127), the
//! held one bright and lit up for a moment by a switch. Under it the held weapon's name, its
//! refire filling while it recharges, and a line with its mod and, when it cannot fire, why
//! (lost, low energy). The bottom left shows the root chassis' health as a large reading over its
//! bar and the energy pool as a thin bar under it, the share one shot of the held weapon takes
//! marked on it, and above them, in teal, field repair ([`repair_panel`],
//! [`crate::repair_menu`]): the quick repair menu while it is open, else the repair under way,
//! else how many parts may be repaired.
use render_engine::glam::{Vec2, Vec4};
use render_engine::scene::OverlayQuad;
use shooter_sim::energy::MILLI;
use shooter_sim::robot::{BETA, ModId};
use shooter_sim::session::Fighter;
use shooter_sim::{Assembly, Instance, TICK_HZ, Weapon};

/// An sRGB colour (0-255) with alpha as the overlay's linear colour.
pub(crate) fn srgb(hex: u32, alpha: f32) -> Vec4 {
    let channel = |shift: u32| {
        #[allow(clippy::cast_precision_loss)]
        let value = ((hex >> shift) & 0xff) as f32 / 255.0;
        if value <= 0.040_45 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    Vec4::new(channel(16), channel(8), channel(0), alpha)
}

/// The palette: the robot kits' pale grey, graphite and olive, and the orange accent.
pub(crate) mod palette {
    pub const PALE: u32 = 0xE4_E6_E1;
    pub const MUTED: u32 = 0x9A_A0_A3;
    pub const GRAPHITE: u32 = 0x1F_22_25;
    pub const STEEL: u32 = 0x3A_40_45;
    pub const OLIVE: u32 = 0x9C_AA_5C;
    pub const ORANGE: u32 = 0xFF_8A_1F;
    pub const RED: u32 = 0xE0_4A_2F;
    /// Field repair: the menu, the part under repair and its glow on the robot.
    pub const TEAL: u32 = 0x4F_D1_A5;
    /// The energy pool in the HUD.
    pub const ENERGY: u32 = 0x5C_C8_FF;
    /// Behind the HUD's text and bars: a drop shadow and the empty part of a bar.
    pub const SHADOW: u32 = 0x0B_0D_0F;
}
use crate::repair_menu::{Entry, RepairView};
use palette::{ENERGY, MUTED, OLIVE, ORANGE, PALE, RED, SHADOW, TEAL};

/// The 5×7 glyphs, rows top down, the leftmost column the highest of five bits.
fn glyph(character: char) -> [u8; 7] {
    match character.to_ascii_uppercase() {
        'A' => [14, 17, 17, 31, 17, 17, 17],
        'B' => [30, 17, 17, 30, 17, 17, 30],
        'C' => [14, 17, 16, 16, 16, 17, 14],
        'D' => [30, 17, 17, 17, 17, 17, 30],
        'E' => [31, 16, 16, 30, 16, 16, 31],
        'F' => [31, 16, 16, 30, 16, 16, 16],
        'G' => [14, 17, 16, 23, 17, 17, 15],
        'H' => [17, 17, 17, 31, 17, 17, 17],
        'I' => [14, 4, 4, 4, 4, 4, 14],
        'J' => [7, 2, 2, 2, 2, 18, 12],
        'K' => [17, 18, 20, 24, 20, 18, 17],
        'L' => [16, 16, 16, 16, 16, 16, 31],
        'M' => [17, 27, 21, 21, 17, 17, 17],
        'N' => [17, 17, 25, 21, 19, 17, 17],
        'O' => [14, 17, 17, 17, 17, 17, 14],
        'P' => [30, 17, 17, 30, 16, 16, 16],
        'Q' => [14, 17, 17, 17, 21, 18, 13],
        'R' => [30, 17, 17, 30, 20, 18, 17],
        'S' => [15, 16, 16, 14, 1, 1, 30],
        'T' => [31, 4, 4, 4, 4, 4, 4],
        'U' => [17, 17, 17, 17, 17, 17, 14],
        'V' => [17, 17, 17, 17, 17, 10, 4],
        'W' => [17, 17, 17, 21, 21, 21, 10],
        'X' => [17, 17, 10, 4, 10, 17, 17],
        'Y' => [17, 17, 17, 10, 4, 4, 4],
        'Z' => [31, 1, 2, 4, 8, 16, 31],
        '0' => [14, 17, 19, 21, 25, 17, 14],
        '1' => [4, 12, 4, 4, 4, 4, 14],
        '2' => [14, 17, 1, 2, 4, 8, 31],
        '3' => [31, 2, 4, 2, 1, 17, 14],
        '4' => [2, 6, 10, 18, 31, 2, 2],
        '5' => [31, 16, 30, 1, 1, 17, 14],
        '6' => [6, 8, 16, 30, 17, 17, 14],
        '7' => [31, 1, 2, 4, 8, 8, 8],
        '8' => [14, 17, 17, 14, 17, 17, 14],
        '9' => [14, 17, 17, 15, 1, 2, 12],
        '+' => [0, 4, 4, 31, 4, 4, 0],
        '-' => [0, 0, 0, 31, 0, 0, 0],
        '_' => [0, 0, 0, 0, 0, 0, 31],
        '/' => [0, 1, 2, 4, 8, 16, 0],
        '.' => [0, 0, 0, 0, 0, 12, 12],
        ':' => [0, 12, 12, 0, 12, 12, 0],
        '%' => [24, 25, 2, 4, 8, 19, 3],
        '(' => [2, 4, 8, 8, 8, 4, 2],
        ')' => [8, 4, 2, 2, 2, 4, 8],
        '[' => [14, 8, 8, 8, 8, 8, 14],
        ']' => [14, 2, 2, 2, 2, 2, 14],
        ',' => [0, 0, 0, 0, 12, 4, 8],
        '<' => [2, 4, 8, 16, 8, 4, 2],
        '>' => [8, 4, 2, 1, 2, 4, 8],
        '=' => [0, 0, 31, 0, 31, 0, 0],
        '!' => [4, 4, 4, 4, 4, 0, 4],
        '?' => [14, 17, 1, 2, 4, 0, 4],
        '\'' => [4, 4, 8, 0, 0, 0, 0],
        _ => [0; 7],
    }
}

/// Font cells a character advances (five wide, one apart).
const ADVANCE: f32 = 6.0;

/// A weapon's silhouette, muzzle to the right, 20 cells by 8.
const fn silhouette(weapon: Weapon) -> [&'static str; 8] {
    match weapon {
        Weapon::EnergyRifle => [
            "....................",
            "......#######.......",
            "##################..",
            "####################",
            "########..####......",
            "###.....##..##......",
            "###.........##......",
            "....................",
        ],
        Weapon::RailGun => [
            "....................",
            "...#################",
            "####################",
            "...#################",
            "######..##..........",
            "####....##..........",
            "###.................",
            "....................",
        ],
        Weapon::RocketLauncher => [
            "...............###..",
            "####################",
            "####################",
            "####################",
            "####################",
            "....##....###.......",
            "....##....###.......",
            "....................",
        ],
        Weapon::Grapple => [
            "..................#.",
            ".................##.",
            "###########.######..",
            "################....",
            "###########.######..",
            ".....##..........##.",
            ".....##...........#.",
            "....................",
        ],
        Weapon::EnergyBlade => [
            "....................",
            "...#................",
            "...###############..",
            "####################",
            "...###############..",
            "...#................",
            "....................",
            "....................",
        ],
        Weapon::Shotgun => [
            "....................",
            "....................",
            "..##################",
            "####################",
            "#########...........",
            "####....##..........",
            "###.................",
            "....................",
        ],
        Weapon::Laser => [
            "....................",
            "......##.##.##......",
            "##################..",
            "####################",
            "##################..",
            "####...##...........",
            "###....##...........",
            "....................",
        ],
        Weapon::Flamethrower => [
            "....................",
            ".............###....",
            "####################",
            "####################",
            "###..######.........",
            "###.########........",
            "....######..........",
            "....................",
        ],
    }
}

/// A weapon's name on the HUD.
#[must_use]
pub const fn weapon_name(weapon: Weapon) -> &'static str {
    match weapon {
        Weapon::EnergyRifle => "Energy rifle",
        Weapon::RailGun => "Rail gun",
        Weapon::RocketLauncher => "Rockets",
        Weapon::Grapple => "Grapple",
        Weapon::EnergyBlade => "Energy blade",
        Weapon::Shotgun => "Shotgun",
        Weapon::Laser => "Laser",
        Weapon::Flamethrower => "Flamethrower",
    }
}

/// A weapon copy's name in `build`: its weapon's, numbered among its copies when `build` mounts
/// that weapon more than once ("Rail gun 2/2", shooter#125).
#[must_use]
pub fn copy_name(build: &Assembly, instance: Instance) -> String {
    let name = weapon_name(instance.weapon);
    match BETA.copy(build, instance) {
        Some((number, of)) if of > 1 => format!("{name} {number}/{of}"),
        _ => name.to_owned(),
    }
}

/// A weapon mod's name, from its catalog key.
#[must_use]
pub fn mod_name(id: ModId) -> &'static str {
    match BETA.modification(id).map(|modification| modification.key) {
        Some("mod.rapid-cycler") => "rapid cycler",
        Some("mod.overcharger") => "overcharger",
        Some("mod.long-barrel") => "long barrel",
        Some("mod.efficiency-coil") => "efficiency coil",
        _ => "mod",
    }
}

/// The key that selects `weapon`: its digit, V for the blade, or F for the grapple module (or the
/// right button).
fn key_label(weapon: Weapon) -> String {
    match weapon {
        Weapon::EnergyBlade => "V".to_owned(),
        Weapon::Grapple => "F".to_owned(),
        _ => Weapon::ROTATION
            .iter()
            .position(|held| *held == weapon)
            .map_or_else(String::new, |index| (index + 1).to_string()),
    }
}

/// Quads in layout units (720 lines high), scaled to the frame and snapped to whole pixels.
pub(crate) struct Painter {
    pub(crate) quads: Vec<OverlayQuad>,
    pub(crate) scale: f32,
}

impl Painter {
    pub(crate) fn rect(&mut self, x: f32, y: f32, width: f32, height: f32, color: Vec4) {
        if width <= 0.0 || height <= 0.0 || color.w <= 0.0 {
            return;
        }
        let snap = |value: f32| (value * self.scale).round();
        let (left, top) = (snap(x), snap(y));
        let size = Vec2::new(
            (snap(x + width) - left).max(1.0),
            (snap(y + height) - top).max(1.0),
        );
        self.quads.push(OverlayQuad {
            position: Vec2::new(left, top),
            size,
            color,
        });
    }

    /// A rectangle's outline, `line` thick.
    pub(crate) fn frame(
        &mut self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        line: f32,
        color: Vec4,
    ) {
        self.rect(x, y, width, line, color);
        self.rect(x, y + height - line, width, line, color);
        self.rect(x, y + line, line, height - 2.0 * line, color);
        self.rect(x + width - line, y + line, line, height - 2.0 * line, color);
    }

    /// Lit cells of a bitmap, `cell` units each, every row's runs one quad.
    fn cells(
        &mut self,
        x: f32,
        y: f32,
        cell: f32,
        rows: impl Iterator<Item = Vec<bool>>,
        color: Vec4,
    ) {
        for (row, lit) in rows.enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let top = y + row as f32 * cell;
            let mut column = 0;
            while column < lit.len() {
                if !lit[column] {
                    column += 1;
                    continue;
                }
                let start = column;
                while column < lit.len() && lit[column] {
                    column += 1;
                }
                #[allow(clippy::cast_precision_loss)]
                self.rect(
                    x + start as f32 * cell,
                    top,
                    (column - start) as f32 * cell,
                    cell,
                    color,
                );
            }
        }
    }

    /// `text` in capitals at `cell` units a font cell, left-aligned at `x`; its width.
    pub(crate) fn text(&mut self, x: f32, y: f32, cell: f32, text: &str, color: Vec4) -> f32 {
        let snapped = self.snap_cell(cell);
        let mut at = x;
        for character in text.chars() {
            let rows = glyph(character)
                .into_iter()
                .map(|bits| (0..5).map(|bit| bits & (16 >> bit) != 0).collect());
            self.cells(at, y, snapped, rows, color);
            at += ADVANCE * snapped;
        }
        text_width(text, snapped)
    }

    /// `text` right-aligned to `right`.
    pub(crate) fn text_right(&mut self, right: f32, y: f32, cell: f32, text: &str, color: Vec4) {
        let width = text_width(text, self.snap_cell(cell));
        self.text(right - width, y, cell, text, color);
    }

    /// `text` over a drop shadow half a font cell down and right (whole pixels), legible on any
    /// part of the scene without a panel; its width.
    pub(crate) fn label(&mut self, x: f32, y: f32, cell: f32, text: &str, color: Vec4) -> f32 {
        let offset = (cell * self.scale * 0.5).round().max(1.0) / self.scale;
        let shadow = srgb(SHADOW, 0.6 * color.w);
        self.text(x + offset, y + offset, cell, text, shadow);
        self.text(x, y, cell, text, color)
    }

    /// [`Self::label`] right-aligned to `right`; its width.
    pub(crate) fn label_right(
        &mut self,
        right: f32,
        y: f32,
        cell: f32,
        text: &str,
        color: Vec4,
    ) -> f32 {
        let width = text_width(text, self.snap_cell(cell));
        self.label(right - width, y, cell, text, color)
    }

    /// A font cell as a whole number of pixels (crisp glyphs at any height), back in units.
    pub(crate) fn snap_cell(&self, cell: f32) -> f32 {
        (cell * self.scale).round().max(1.0) / self.scale
    }

    pub(crate) fn icon(&mut self, x: f32, y: f32, cell: f32, weapon: Weapon, color: Vec4) {
        let rows = silhouette(weapon)
            .into_iter()
            .map(|row| row.bytes().map(|byte| byte == b'#').collect());
        self.cells(x, y, self.snap_cell(cell), rows, color);
    }
}

#[allow(clippy::cast_precision_loss)]
pub(crate) fn text_width(text: &str, cell: f32) -> f32 {
    let count = text.chars().count() as f32;
    if count == 0.0 {
        0.0
    } else {
        (count * ADVANCE - 1.0) * cell
    }
}

/// One weapon copy the build mounts, as the HUD lists it (shooter#125), with its own readiness
/// (shooter#127: every copy may fire, not only the held one).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mounted {
    pub instance: Instance,
    /// It is still on the robot.
    pub armed: bool,
    /// Which copy of its weapon it is and how many the build mounts (`(1, 1)` for a weapon
    /// mounted once).
    pub copy: (u8, u8),
    /// Ticks until it may fire again: its own refire, its family's salvo spacing, a swing's
    /// lockout ([`shooter_sim::weapons::Arsenal::cooldown_of`]).
    pub cooldown: u32,
    /// Ticks between its shots in this build (its mod's).
    pub refire: u32,
    /// Milli-units one activation costs (a tick, for the continuous ones).
    pub cost: u32,
}

impl Mounted {
    /// Its number among its weapon's copies, `"1/2"`, or nothing for a weapon mounted once.
    #[must_use]
    pub fn number(&self) -> Option<String> {
        let (number, of) = self.copy;
        (of > 1).then(|| format!("{number}/{of}"))
    }

    /// How ready it is to fire with `energy` in the pool: lost, short of energy, recovering
    /// (with the share of its refire already over) or ready.
    #[must_use]
    pub fn readiness(&self, energy: u32) -> Readiness {
        if !self.armed {
            Readiness::Lost
        } else if energy < self.cost {
            Readiness::LowEnergy
        } else if self.cooldown > 0 {
            #[allow(clippy::cast_precision_loss)]
            let over = 1.0 - self.cooldown as f32 / self.refire.max(self.cooldown).max(1) as f32;
            Readiness::Recharging(over)
        } else {
            Readiness::Ready
        }
    }
}

/// A weapon copy's readiness ([`Mounted::readiness`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Readiness {
    Lost,
    LowEnergy,
    /// The share of its refire already over (0..1).
    Recharging(f32),
    Ready,
}

/// What the drawn HUD shows of the local fighter this frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Vitals {
    /// The weapon copy held.
    pub held: Instance,
    /// Ticks until the held weapon may fire.
    pub cooldown: u32,
    /// Every weapon copy the build mounts, in part ID order, and whether it still has it.
    pub carried: Vec<Mounted>,
    pub held_mod: Option<ModId>,
    /// Milli-units one activation of the held weapon costs (a tick, for the continuous ones).
    pub cost: u32,
    /// Whether the held weapon fires every tick the trigger is held (laser, flamethrower).
    pub continuous: bool,
    pub energy: u32,
    pub capacity: u32,
    pub chassis: u16,
    pub chassis_whole: u16,
}

impl Vitals {
    /// The local fighter's, as it stands at `tick`.
    #[must_use]
    pub fn of(fighter: &Fighter, tick: u32) -> Self {
        let build = &fighter.build;
        let held = fighter.arsenal.held;
        let tuning = fighter.tuning(held);
        let armed = fighter.instances();
        let carried = BETA
            .weapons(build)
            .map(|instance| {
                let tuning = fighter.tuning(instance);
                let arsenal = &fighter.arsenal;
                Mounted {
                    instance,
                    armed: armed.contains(&instance),
                    copy: BETA.copy(build, instance).unwrap_or((1, 1)),
                    cooldown: match instance.weapon {
                        Weapon::Grapple => arsenal.grapple_cooldown(tick),
                        Weapon::EnergyBlade => arsenal.blade_cooldown(tick),
                        _ => arsenal.cooldown_of(instance, tick),
                    },
                    refire: tuning.refire_ticks(instance.weapon),
                    cost: tuning.energy(instance.weapon),
                }
            })
            .collect();
        Self {
            held,
            cooldown: fighter.arsenal.cooldown(tick),
            carried,
            held_mod: BETA.mod_at(build, held.part),
            cost: tuning.energy(held.weapon),
            continuous: tuning.refire_ticks(held.weapon) <= 1,
            energy: fighter.energy,
            capacity: fighter.pool().capacity,
            chassis: fighter.chassis_health(),
            chassis_whole: shooter_sim::damage::PartDamage::fresh(build).chassis_health(build),
        }
    }

    /// The held copy as listed, if the build mounts it.
    fn held(&self) -> Option<&Mounted> {
        self.carried
            .iter()
            .find(|mounted| mounted.instance == self.held)
    }

    fn armed(&self) -> bool {
        self.held().is_some_and(|mounted| mounted.armed)
    }

    /// The held weapon's name, with its copy's number when the build mounts it more than once:
    /// "Rail gun 2/2".
    #[must_use]
    pub fn held_name(&self) -> String {
        let name = weapon_name(self.held.weapon);
        match self.held().and_then(Mounted::number) {
            Some(number) => format!("{name} {number}"),
            None => name.to_owned(),
        }
    }
}

/// How long a switch's highlight lasts (seconds).
const SWITCH_SECONDS: f32 = 0.4;

/// The drawn HUD's state between frames: the switch highlight and the cooldown's length.
#[derive(Debug, Default)]
pub struct Overlay {
    held: Option<Instance>,
    since_switch: f32,
    /// The longest cooldown since the held weapon was last ready: what the refire bar fills.
    period: u32,
    last_cooldown: u32,
}

/// Layout units (the frame is 720 high in them).
const MARGIN: f32 = 28.0;
const BOTTOM: f32 = 720.0 - MARGIN;
/// The chassis and energy bars' width.
const BAR_WIDTH: f32 = 240.0;
/// The top of the bottom left block (the chassis reading): the repair panel sits above it.
const VITALS_TOP: f32 = BOTTOM - 52.0;
/// The weapon slots: a silhouette at three units a cell (60 wide), one every `SLOT_PITCH`.
const SLOT_PITCH: f32 = 72.0;
const SLOT_ICON: f32 = 60.0;
const SLOTS_TOP: f32 = BOTTOM - 108.0;
/// The refire bar under the held weapon's name.
const REFIRE_WIDTH: f32 = 200.0;

impl Overlay {
    /// The quads for a `width`×`height` frame `seconds` after the last; nothing while the local
    /// fighter is not alive.
    pub fn draw(
        &mut self,
        vitals: Option<&Vitals>,
        width: u32,
        height: u32,
        seconds: f32,
    ) -> Vec<OverlayQuad> {
        let Some(vitals) = vitals else {
            self.held = None;
            return Vec::new();
        };
        if self.held != Some(vitals.held) {
            // The first frame of a life is no switch.
            self.since_switch = if self.held.is_some() {
                0.0
            } else {
                SWITCH_SECONDS
            };
            self.held = Some(vitals.held);
        } else {
            self.since_switch += seconds;
        }
        if vitals.cooldown == 0 {
            self.period = 0;
        } else if vitals.cooldown > self.last_cooldown {
            self.period = self.period.max(vitals.cooldown);
        }
        self.last_cooldown = vitals.cooldown;
        #[allow(clippy::cast_precision_loss)]
        let (width, height) = (width as f32, height as f32);
        let scale = height / 720.0;
        let mut painter = Painter {
            quads: Vec::with_capacity(512),
            scale,
        };
        let right = width / scale - MARGIN;
        slots(&mut painter, vitals, right, self.highlight());
        self.held_weapon(&mut painter, vitals, right);
        bars(&mut painter, vitals, MARGIN);
        painter.quads
    }

    /// The switch highlight left, 1 at the switch and 0 once it is over.
    fn highlight(&self) -> f32 {
        (1.0 - self.since_switch / SWITCH_SECONDS).clamp(0.0, 1.0)
    }

    /// The held weapon, right-aligned to `right`: its name, the refire bar while it recharges,
    /// and a line with its mod and what keeps it from firing (nothing when it is ready).
    fn held_weapon(&self, painter: &mut Painter, vitals: &Vitals, right: f32) {
        let armed = vitals.armed();
        let short = vitals.energy < vitals.cost;
        let name = vitals.held_name();
        painter.label_right(
            right,
            BOTTOM - 48.0,
            3.0,
            &name,
            srgb(if armed { PALE } else { MUTED }, 1.0),
        );
        if armed && !short && vitals.cooldown > 0 && self.period > 0 {
            #[allow(clippy::cast_precision_loss)]
            let over = 1.0 - vitals.cooldown as f32 / self.period as f32;
            let (x, y) = (right - REFIRE_WIDTH, BOTTOM - 21.0);
            painter.rect(x, y, REFIRE_WIDTH, 3.0, srgb(SHADOW, 0.5));
            painter.rect(x, y, REFIRE_WIDTH * over, 3.0, srgb(ORANGE, 1.0));
        }
        let state = if !armed {
            Some(("lost", RED))
        } else if short {
            Some(("low energy", RED))
        } else {
            None
        };
        let mut at = right;
        if let Some((label, color)) = state {
            at -= painter.label_right(at, BOTTOM - 12.0, 2.0, label, srgb(color, 1.0)) + 16.0;
        }
        if let Some(id) = vitals.held_mod {
            painter.label_right(at, BOTTOM - 12.0, 2.0, mod_name(id), srgb(OLIVE, 1.0));
        }
    }
}

/// The carried weapon copies as silhouettes in a row ending at `right`, the held one bright with
/// its key in the accent; a weapon mounted more than once has one for every copy, numbered
/// (shooter#125). Under each, its key and its own readiness (shooter#127: every copy may fire,
/// held or not): olive when ready, the refire filling orange, red when the pool cannot pay it;
/// lost ones dim red and struck out.
fn slots(painter: &mut Painter, vitals: &Vitals, right: f32, flash: f32) {
    #[allow(clippy::cast_precision_loss)]
    let count = vitals.carried.len() as f32;
    let mut x = right - SLOT_ICON - (count - 1.0).max(0.0) * SLOT_PITCH;
    // The guns in rotation order (copies by part), then the blade and the grapple module (on
    // their own keys).
    let mut order: Vec<Mounted> = vitals.carried.clone();
    order.sort_by_key(|mounted| {
        let weapon = mounted.instance.weapon;
        let place = Weapon::ROTATION
            .iter()
            .position(|held| *held == weapon)
            .unwrap_or(match weapon {
                Weapon::EnergyBlade => usize::MAX - 1,
                _ => usize::MAX,
            });
        (place, mounted.instance.part)
    });
    let y = SLOTS_TOP;
    for mounted in order {
        let (weapon, armed) = (mounted.instance.weapon, mounted.armed);
        let held = mounted.instance == vitals.held;
        if held && flash > 0.0 {
            painter.rect(
                x - 6.0,
                y - 5.0,
                SLOT_ICON + 12.0,
                34.0,
                srgb(ORANGE, 0.35 * flash),
            );
        }
        let tone = match (armed, held) {
            (false, _) => srgb(RED, 0.55),
            (true, true) => srgb(PALE, 1.0),
            (true, false) => srgb(MUTED, 0.8),
        };
        painter.icon(x + 1.5, y + 1.5, 3.0, weapon, srgb(SHADOW, 0.5 * tone.w));
        painter.icon(x, y, 3.0, weapon, tone);
        if !armed {
            painter.rect(x - 2.0, y + 11.0, SLOT_ICON + 4.0, 2.0, srgb(RED, 0.9));
        }
        let bar = (x, y + 28.0, SLOT_ICON);
        match mounted.readiness(vitals.energy) {
            Readiness::Lost => {}
            Readiness::LowEnergy => painter.rect(bar.0, bar.1, bar.2, 2.0, srgb(RED, 0.9)),
            Readiness::Recharging(over) => {
                painter.rect(bar.0, bar.1, bar.2, 2.0, srgb(SHADOW, 0.5));
                painter.rect(bar.0, bar.1, bar.2 * over, 2.0, srgb(ORANGE, 1.0));
            }
            Readiness::Ready => painter.rect(bar.0, bar.1, bar.2, 2.0, srgb(OLIVE, 0.9)),
        }
        let key = if held { ORANGE } else { MUTED };
        painter.label(x, y + 34.0, 2.0, &key_label(weapon), srgb(key, 1.0));
        if let Some(number) = mounted.number() {
            painter.label_right(x + SLOT_ICON, y + 34.0, 2.0, &number, srgb(MUTED, 1.0));
        }
        x += SLOT_PITCH;
    }
}

/// The field repair panel's width, a line's height and its padding (layout units).
const REPAIR_WIDTH: f32 = 320.0;
const REPAIR_LINE: f32 = 20.0;
const REPAIR_PAD: f32 = 10.0;

/// The field repair panel (shooter#61) for a frame `height` pixels high, bottom left above the
/// chassis and energy: the open quick repair menu (only the parts the robot may repair now, the
/// cursor's line in the accent), else the repair under way (its part's health, paused by a hit
/// in red), else a hint that the repair action has something to repair. Nothing when there is
/// none of these.
#[must_use]
pub fn repair_panel(view: &RepairView, height: u32) -> Vec<OverlayQuad> {
    #[allow(clippy::cast_precision_loss)]
    let scale = height as f32 / 720.0;
    let mut painter = Painter {
        quads: Vec::with_capacity(256),
        scale,
    };
    let x = MARGIN;
    let bottom = VITALS_TOP - 12.0;
    if let Some((entries, cursor)) = &view.menu {
        // The menu is a list over the scene: the one block with a backdrop.
        #[allow(clippy::cast_precision_loss)]
        let panel = REPAIR_LINE * (entries.len() + 1) as f32 + REPAIR_PAD;
        let top = bottom - panel;
        painter.rect(x, top, REPAIR_WIDTH, panel, srgb(SHADOW, 0.6));
        painter.text(x + REPAIR_PAD, top + 8.0, 2.0, "repair", srgb(TEAL, 1.0));
        painter.text_right(
            x + REPAIR_WIDTH - REPAIR_PAD,
            top + 8.0,
            2.0,
            "wheel  r",
            srgb(MUTED, 1.0),
        );
        for (row, entry) in entries.iter().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let y = top + REPAIR_LINE * (row + 1) as f32 + 4.0;
            let chosen = row == *cursor;
            if chosen {
                painter.rect(x, y, REPAIR_WIDTH, REPAIR_LINE, srgb(TEAL, 0.22));
                painter.rect(x, y, 3.0, REPAIR_LINE, srgb(TEAL, 1.0));
            }
            let color = srgb(if chosen { PALE } else { MUTED }, 1.0);
            entry_line(
                &mut painter,
                x + REPAIR_PAD,
                REPAIR_WIDTH - 2.0 * REPAIR_PAD,
                y + 3.0,
                entry,
                color,
            );
        }
    } else if let Some((repair, entry)) = &view.active {
        let paused = repair.paused > 0;
        let top = bottom - 2.0 * REPAIR_LINE - 4.0;
        painter.label(
            x,
            top,
            2.0,
            if paused { "repair paused" } else { "repairing" },
            srgb(if paused { RED } else { TEAL }, 1.0),
        );
        entry_line(
            &mut painter,
            x,
            BAR_WIDTH,
            top + REPAIR_LINE,
            entry,
            srgb(PALE, 1.0),
        );
        if let Entry::Part { health, whole, .. } = entry {
            #[allow(clippy::cast_precision_loss)]
            let share = (f32::from(*health) / f32::from((*whole).max(1))).clamp(0.0, 1.0);
            let y = bottom - 3.0;
            painter.rect(x, y, BAR_WIDTH, 3.0, srgb(SHADOW, 0.5));
            painter.rect(x, y, BAR_WIDTH * share, 3.0, srgb(TEAL, 1.0));
        }
    } else if view.repairable > 0 {
        painter.label(
            x,
            bottom - 14.0,
            2.0,
            &format!("r  repair ({})", view.repairable),
            srgb(TEAL, 1.0),
        );
    }
    painter.quads
}

/// A repair entry as a line from `x`, `width` wide: the part's name and its health, or "stop
/// repair".
fn entry_line(painter: &mut Painter, x: f32, width: f32, y: f32, entry: &Entry, color: Vec4) {
    match entry {
        Entry::Part {
            name,
            health,
            whole,
            ..
        } => {
            painter.label(x, y, 2.0, name, color);
            painter.label_right(x + width, y, 2.0, &format!("{health}/{whole}"), color);
        }
        Entry::Stop => {
            painter.label(x, y, 2.0, "stop repair", color);
        }
    }
}

/// The root chassis' health (a large reading over its bar) and the energy pool (a thin bar under
/// it, the share one shot of the held weapon takes marked at its end), from `x`.
fn bars(painter: &mut Painter, vitals: &Vitals, x: f32) {
    let share = |value: u32, whole: u32| {
        #[allow(clippy::cast_precision_loss)]
        let share = value as f32 / whole.max(1) as f32;
        share.clamp(0.0, 1.0)
    };
    let health = share(u32::from(vitals.chassis), u32::from(vitals.chassis_whole));
    let tone = if health < 1.0 / 3.0 { RED } else { PALE };
    let reading = painter.label(
        x,
        VITALS_TOP,
        4.0,
        &vitals.chassis.to_string(),
        srgb(tone, 1.0),
    );
    painter.label(
        x + reading + 8.0,
        VITALS_TOP + 14.0,
        2.0,
        &format!("/{}", vitals.chassis_whole),
        srgb(MUTED, 1.0),
    );
    painter.label_right(
        x + BAR_WIDTH,
        VITALS_TOP + 14.0,
        2.0,
        &(vitals.energy / MILLI).to_string(),
        srgb(ENERGY, 1.0),
    );
    let chassis_y = VITALS_TOP + 36.0;
    painter.rect(x, chassis_y, BAR_WIDTH, 6.0, srgb(SHADOW, 0.5));
    painter.rect(x, chassis_y, BAR_WIDTH * health, 6.0, srgb(tone, 1.0));
    let charge = share(vitals.energy, vitals.capacity);
    let shot = if vitals.continuous {
        vitals.cost.saturating_mul(u32::from(TICK_HZ))
    } else {
        vitals.cost
    };
    let cost = share(shot.min(vitals.energy), vitals.capacity);
    let energy_y = chassis_y + 10.0;
    let track = if vitals.energy < vitals.cost {
        srgb(RED, 0.6)
    } else {
        srgb(SHADOW, 0.5)
    };
    painter.rect(x, energy_y, BAR_WIDTH, 4.0, track);
    painter.rect(x, energy_y, BAR_WIDTH * charge, 4.0, srgb(ENERGY, 1.0));
    painter.rect(
        x + BAR_WIDTH * (charge - cost),
        energy_y,
        BAR_WIDTH * cost,
        4.0,
        srgb(ORANGE, 1.0),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use shooter_sim::robot::STANDARD;
    use shooter_sim::{Rules, Session};

    fn standard() -> Vitals {
        let session = Session::new(shooter_sim::Arena::greybox()).unwrap();
        let _ = Rules::solo();
        let fighter = session.local();
        assert_eq!(fighter.build, STANDARD);
        Vitals::of(fighter, session.ticks())
    }

    #[test]
    fn the_vitals_list_the_carried_weapons_and_the_held_ones_cost() {
        let vitals = standard();
        assert_eq!(vitals.held, Instance::new(7, Weapon::EnergyRifle));
        let listed: Vec<(Instance, bool, Option<String>)> = vitals
            .carried
            .iter()
            .map(|mounted| (mounted.instance, mounted.armed, mounted.number()))
            .collect();
        assert_eq!(
            listed,
            [
                (Instance::new(7, Weapon::EnergyRifle), true, None),
                (Instance::new(8, Weapon::EnergyBlade), true, None),
                (Instance::new(9, Weapon::Grapple), true, None),
                (Instance::new(10, Weapon::Shotgun), true, None),
            ]
        );
        assert_eq!(vitals.held_name(), "Energy rifle");
        assert_eq!(vitals.cost, 2_000);
        assert!(!vitals.continuous);
    }

    /// The rocket pod (shooter#125): a slot for each of its four launchers, numbered, the held
    /// one named with its number; a lost copy struck out, the others still armed.
    #[test]
    fn the_vitals_list_every_copy_of_a_weapon() {
        use shooter_sim::robot::ROCKET_POD;
        let mut session = Session::new(shooter_sim::Arena::greybox()).unwrap();
        session.rebuild(0, ROCKET_POD).unwrap();
        let vitals = Vitals::of(session.local(), session.ticks());
        assert_eq!(vitals.held, Instance::new(7, Weapon::RocketLauncher));
        assert_eq!(vitals.held_name(), "Rockets 1/4");
        let numbers: Vec<Option<String>> = vitals.carried.iter().map(Mounted::number).collect();
        assert_eq!(
            numbers,
            ["1/4", "2/4", "3/4", "4/4"].map(|number| Some(number.to_owned()))
        );
        let fighter = &mut session.fighters[0];
        let build = fighter.build;
        fighter.parts.hurt(&build, 5, 1_000);
        let vitals = Vitals::of(session.local(), session.ticks());
        let armed: Vec<bool> = vitals.carried.iter().map(|mounted| mounted.armed).collect();
        assert_eq!(armed, [true, false, true, true]);
        assert!(vitals.armed(), "the held launcher is still on");
        let quads = Overlay::default().draw(Some(&vitals), 1280, 720, 0.016);
        assert!(!quads.is_empty());
    }

    /// Every mounted copy shows its own readiness (shooter#127): the two rail guns fired
    /// together both recharge, a launcher waits out its family's salvo spacing, and a copy the
    /// pool cannot pay shows short of energy, whichever is held.
    #[test]
    fn every_copy_shows_its_own_readiness() {
        use shooter_sim::robot::{PartId, ROCKET_POD, TWIN_RAIL};
        use shooter_sim::weapons::Triggers;
        let fire = |session: &mut Session, parts: &[u8]| {
            let triggers: Triggers = parts.iter().copied().map(PartId).collect();
            session
                .tick(shooter_sim::Intent {
                    triggers,
                    ..shooter_sim::Intent::IDLE
                })
                .unwrap();
        };
        let mut session = Session::new(shooter_sim::Arena::greybox()).unwrap();
        session.rebuild(0, TWIN_RAIL).unwrap();
        let fighter = &mut session.fighters[0];
        fighter.energy = fighter.pool().capacity;
        fire(&mut session, &[7, 9]);
        let vitals = Vitals::of(session.local(), session.ticks());
        let rails: Vec<Readiness> = vitals
            .carried
            .iter()
            .filter(|mounted| mounted.instance.weapon == Weapon::RailGun)
            .map(|mounted| mounted.readiness(vitals.energy))
            .collect();
        assert!(
            rails
                .iter()
                .all(|ready| matches!(ready, Readiness::Recharging(over) if *over < 0.1)),
            "{rails:?}"
        );
        // The pool drained: both short of energy.
        session.fighters[0].energy = 1_000;
        let vitals = Vitals::of(session.local(), session.ticks());
        assert!(
            vitals
                .carried
                .iter()
                .filter(|mounted| mounted.instance.weapon == Weapon::RailGun)
                .all(|mounted| mounted.readiness(vitals.energy) == Readiness::LowEnergy)
        );

        let mut session = Session::new(shooter_sim::Arena::greybox()).unwrap();
        session.rebuild(0, ROCKET_POD).unwrap();
        let fighter = &mut session.fighters[0];
        fighter.energy = fighter.pool().capacity;
        fire(&mut session, &[7, 8, 9, 10]);
        let vitals = Vitals::of(session.local(), session.ticks());
        let pods: Vec<(u32, Readiness)> = vitals
            .carried
            .iter()
            .map(|mounted| (mounted.cooldown, mounted.readiness(vitals.energy)))
            .collect();
        let spacing = Weapon::RocketLauncher.salvo().unwrap().spacing;
        // The first launched; the others wait for the family, not their own refire.
        assert_eq!(pods[0].0, 47);
        assert!(
            pods[1..]
                .iter()
                .all(|(cooldown, _)| *cooldown == spacing - 1)
        );
        let quads = Overlay::default().draw(Some(&vitals), 1280, 720, 0.016);
        assert!(!quads.is_empty());
    }

    #[test]
    fn the_hud_stays_in_the_bottom_corners_clear_of_the_reticle() {
        let vitals = standard();
        let mut overlay = Overlay::default();
        for (width, height) in [(1280, 720), (1920, 1080), (1024, 768)] {
            let quads = overlay.draw(Some(&vitals), width, height, 0.016);
            assert!(!quads.is_empty());
            #[allow(clippy::cast_precision_loss)]
            let (w, h) = (width as f32, height as f32);
            for quad in &quads {
                let (min, max) = (quad.position, quad.position + quad.size);
                assert!(
                    min.x >= 0.0 && min.y >= 0.0 && max.x <= w && max.y <= h,
                    "{quad:?}"
                );
                // Nothing in the middle half of the frame, where the reticle moves.
                assert!(min.y >= h * 0.6, "{quad:?} at {width}×{height}");
            }
        }
        assert!(overlay.draw(None, 1280, 720, 0.016).is_empty());
    }

    #[test]
    fn a_switch_lights_the_panel_and_fades() {
        let mut vitals = standard();
        let mut overlay = Overlay::default();
        overlay.draw(Some(&vitals), 1280, 720, 0.016);
        vitals.held = Instance::new(10, Weapon::Shotgun);
        let switched = overlay.draw(Some(&vitals), 1280, 720, 0.016);
        let settled = overlay.draw(Some(&vitals), 1280, 720, 1.0);
        // The glow behind the held silhouette, then none.
        assert_eq!(switched.len(), settled.len() + 1);
    }
}
