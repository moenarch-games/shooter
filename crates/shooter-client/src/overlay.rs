//! The drawn HUD: the weapon panel and the chassis and energy bars, as render-engine screen
//! overlay quads ([`render_engine::scene::Scene::overlay`]), the same natively and in the browser.
//!
//! render-engine's overlay draws solid rectangles only, so the text is a 5×7 bitmap font and the
//! weapon silhouettes are pixel art, both drawn here as quads (each lit row run one quad). The
//! look follows the robot kits' palette: pale grey on graphite panels, olive for the mods and the
//! chassis, an orange accent for the held weapon, energy and the switch highlight. Everything
//! sits in the bottom corners, clear of the reticle; the layout is for 720 lines and scales with
//! the frame's height.
//!
//! The bottom right shows the held weapon (its silhouette, name and mod, the energy one shot
//! costs, and whether it is ready, recharging, short of energy or lost) over a row of slots for
//! every weapon copy the build carries (its select key, the blade's V; lost ones struck out red),
//! each with its own readiness bar (ready, recharging, short of energy: every copy may fire,
//! shooter#127); a switch flashes the panel and sweeps a highlight across it. The bottom left
//! shows the root chassis' health and the energy pool, the share one shot of the held weapon
//! takes marked on it, and above them, in teal, field repair ([`repair_panel`],
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
}
use crate::repair_menu::{Entry, RepairView};
use palette::{GRAPHITE, MUTED, OLIVE, ORANGE, PALE, RED, STEEL, TEAL};

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
const MARGIN: f32 = 24.0;
const PANEL_WIDTH: f32 = 364.0;
const PANEL_HEIGHT: f32 = 104.0;
const SLOT_WIDTH: f32 = 54.0;
const SLOT_HEIGHT: f32 = 38.0;
const SLOT_GAP: f32 = 6.0;
const VITALS_WIDTH: f32 = 272.0;
const VITALS_HEIGHT: f32 = 84.0;

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
        let bottom = 720.0 - MARGIN;
        self.weapon_panel(
            &mut painter,
            vitals,
            right - PANEL_WIDTH,
            bottom - PANEL_HEIGHT,
        );
        slots(
            &mut painter,
            vitals,
            right,
            bottom - PANEL_HEIGHT - SLOT_GAP - SLOT_HEIGHT,
            self.highlight(),
        );
        bars(&mut painter, vitals, MARGIN, bottom - VITALS_HEIGHT);
        painter.quads
    }

    /// The switch highlight left, 1 at the switch and 0 once it is over.
    fn highlight(&self) -> f32 {
        (1.0 - self.since_switch / SWITCH_SECONDS).clamp(0.0, 1.0)
    }

    fn weapon_panel(&self, painter: &mut Painter, vitals: &Vitals, x: f32, y: f32) {
        let flash = self.highlight();
        painter.rect(x, y, PANEL_WIDTH, PANEL_HEIGHT, srgb(GRAPHITE, 0.82));
        painter.rect(x, y, PANEL_WIDTH, 2.0, srgb(STEEL, 1.0));
        // The accent bar on its left edge widens with a switch.
        painter.rect(x, y, 4.0 + 6.0 * flash, PANEL_HEIGHT, srgb(ORANGE, 1.0));
        if flash > 0.0 {
            painter.rect(x, y, PANEL_WIDTH, PANEL_HEIGHT, srgb(ORANGE, 0.18 * flash));
            // A band sweeping across, left to right.
            let sweep = 1.0 - flash;
            painter.rect(
                x + (PANEL_WIDTH - 40.0) * sweep,
                y,
                40.0,
                PANEL_HEIGHT,
                srgb(PALE, 0.16 * flash),
            );
        }
        let armed = vitals.armed();
        let tone = if armed { PALE } else { MUTED };
        // The silhouette in a recessed bay.
        painter.rect(x + 16.0, y + 14.0, 96.0, 50.0, srgb(STEEL, 0.55));
        painter.icon(x + 24.0, y + 23.0, 4.0, vitals.held.weapon, srgb(tone, 1.0));
        let text_x = x + 126.0;
        painter.text(text_x, y + 14.0, 3.0, &vitals.held_name(), srgb(tone, 1.0));
        let modded = vitals
            .held_mod
            .map_or_else(|| "no mod".to_owned(), |id| format!("+ {}", mod_name(id)));
        let mod_color = if vitals.held_mod.is_some() {
            OLIVE
        } else {
            MUTED
        };
        painter.text(text_x, y + 43.0, 2.0, &modded, srgb(mod_color, 1.0));
        // The cost of a shot (a second, for the continuous weapons).
        #[allow(clippy::cast_precision_loss)]
        let cost = if vitals.continuous {
            format!(
                "{}/s",
                units(vitals.cost.saturating_mul(u32::from(TICK_HZ)))
            )
        } else {
            format!("{}/shot", units(vitals.cost))
        };
        painter.text(text_x, y + 62.0, 2.0, &cost, srgb(ORANGE, 1.0));
        // Ready, recharging, starved or lost: a label and the refire bar under it all.
        let short = vitals.energy < vitals.cost;
        let (label, color) = if !armed {
            ("lost", RED)
        } else if short {
            ("low energy", RED)
        } else if vitals.cooldown > 0 {
            ("recharging", MUTED)
        } else {
            ("ready", OLIVE)
        };
        painter.text_right(
            x + PANEL_WIDTH - 14.0,
            y + 62.0,
            2.0,
            label,
            srgb(color, 1.0),
        );
        let (bar_x, bar_y, bar_width) = (x + 16.0, y + PANEL_HEIGHT - 16.0, PANEL_WIDTH - 30.0);
        painter.rect(bar_x, bar_y, bar_width, 5.0, srgb(STEEL, 1.0));
        #[allow(clippy::cast_precision_loss)]
        let ready = if !armed {
            0.0
        } else if self.period == 0 {
            1.0
        } else {
            1.0 - vitals.cooldown as f32 / self.period as f32
        };
        let fill = if ready >= 1.0 && !short {
            OLIVE
        } else {
            ORANGE
        };
        painter.rect(bar_x, bar_y, bar_width * ready, 5.0, srgb(fill, 1.0));
    }
}

/// Milli-units as whole units, or tenths below 10.
fn units(milli: u32) -> String {
    if milli < 10 * MILLI && !milli.is_multiple_of(MILLI) {
        format!("{}.{}", milli / MILLI, milli % MILLI / (MILLI / 10))
    } else {
        (milli / MILLI).to_string()
    }
}

/// The carried weapon copies as small slots in a row ending at `right`, the held one lit; a
/// weapon mounted more than once has a slot for every copy, numbered (shooter#125).
fn slots(painter: &mut Painter, vitals: &Vitals, right: f32, y: f32, flash: f32) {
    #[allow(clippy::cast_precision_loss)]
    let count = vitals.carried.len() as f32;
    let mut x = right - count * SLOT_WIDTH - (count - 1.0).max(0.0) * SLOT_GAP;
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
    for mounted in order {
        let (weapon, armed) = (mounted.instance.weapon, mounted.armed);
        let held = mounted.instance == vitals.held;
        let background = if held {
            srgb(GRAPHITE, 0.9)
        } else {
            srgb(GRAPHITE, 0.62)
        };
        painter.rect(x, y, SLOT_WIDTH, SLOT_HEIGHT, background);
        if held {
            let line = 2.0 + 2.0 * flash;
            painter.frame(x, y, SLOT_WIDTH, SLOT_HEIGHT, line, srgb(ORANGE, 1.0));
        }
        let tone = match (armed, held) {
            (false, _) => srgb(RED, 0.75),
            (true, true) => srgb(PALE, 1.0),
            (true, false) => srgb(MUTED, 1.0),
        };
        painter.icon(x + 7.0, y + 8.0, 2.0, weapon, tone);
        painter.text(x + 7.0, y + 27.0, 1.0, &key_label(weapon), srgb(MUTED, 1.0));
        if !armed {
            // Struck out.
            painter.rect(
                x + 4.0,
                y + SLOT_HEIGHT / 2.0 - 1.0,
                SLOT_WIDTH - 8.0,
                2.0,
                srgb(RED, 1.0),
            );
            painter.text_right(x + SLOT_WIDTH - 5.0, y + 27.0, 1.0, "lost", srgb(RED, 1.0));
        } else if let Some(number) = mounted.number() {
            painter.text_right(x + SLOT_WIDTH - 5.0, y + 27.0, 1.0, &number, tone);
        }
        // Its own readiness along its foot (shooter#127: every copy may fire, held or not):
        // olive when ready, the refire filling orange, red when the pool cannot pay it.
        let (bar_x, bar_y, bar_width) = (x + 4.0, y + SLOT_HEIGHT - 5.0, SLOT_WIDTH - 8.0);
        match mounted.readiness(vitals.energy) {
            Readiness::Lost => {}
            Readiness::LowEnergy => {
                painter.rect(bar_x, bar_y, bar_width, 3.0, srgb(RED, 0.9));
            }
            Readiness::Recharging(over) => {
                painter.rect(bar_x, bar_y, bar_width, 3.0, srgb(STEEL, 1.0));
                painter.rect(bar_x, bar_y, bar_width * over, 3.0, srgb(ORANGE, 1.0));
            }
            Readiness::Ready => painter.rect(bar_x, bar_y, bar_width, 3.0, srgb(OLIVE, 1.0)),
        }
        x += SLOT_WIDTH + SLOT_GAP;
    }
}

/// The field repair panel's width, a line's height and its padding (layout units).
const REPAIR_WIDTH: f32 = 340.0;
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
    let bottom = 720.0 - MARGIN - VITALS_HEIGHT - SLOT_GAP;
    let line = |painter: &mut Painter, y: f32, entry: &Entry, color: Vec4| match entry {
        Entry::Part {
            name,
            health,
            whole,
            ..
        } => {
            painter.text(x + REPAIR_PAD, y + 3.0, 2.0, name, color);
            painter.text_right(
                x + REPAIR_WIDTH - REPAIR_PAD,
                y + 3.0,
                2.0,
                &format!("{health}/{whole}"),
                color,
            );
        }
        Entry::Stop => {
            painter.text(x + REPAIR_PAD, y + 3.0, 2.0, "stop repair", color);
        }
    };
    if let Some((entries, cursor)) = &view.menu {
        #[allow(clippy::cast_precision_loss)]
        let panel = REPAIR_LINE * (entries.len() + 1) as f32 + REPAIR_PAD;
        let top = bottom - panel;
        painter.rect(x, top, REPAIR_WIDTH, panel, srgb(GRAPHITE, 0.88));
        painter.rect(x, top, REPAIR_WIDTH, 2.0, srgb(TEAL, 1.0));
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
                painter.rect(x, y, REPAIR_WIDTH, REPAIR_LINE, srgb(ORANGE, 0.22));
                painter.rect(x, y, 4.0, REPAIR_LINE, srgb(ORANGE, 1.0));
            }
            let color = if chosen { PALE } else { MUTED };
            line(&mut painter, y, entry, srgb(color, 1.0));
        }
    } else if let Some((repair, entry)) = &view.active {
        let panel = 2.0 * REPAIR_LINE + REPAIR_PAD;
        let top = bottom - panel;
        painter.rect(x, top, REPAIR_WIDTH, panel, srgb(GRAPHITE, 0.82));
        painter.rect(x, top, 4.0, panel, srgb(TEAL, 1.0));
        let paused = repair.paused > 0;
        painter.text(
            x + REPAIR_PAD,
            top + 8.0,
            2.0,
            if paused { "repair paused" } else { "repairing" },
            srgb(if paused { RED } else { TEAL }, 1.0),
        );
        line(
            &mut painter,
            top + REPAIR_LINE + 2.0,
            entry,
            srgb(PALE, 1.0),
        );
        if let Entry::Part { health, whole, .. } = entry {
            #[allow(clippy::cast_precision_loss)]
            let share = (f32::from(*health) / f32::from((*whole).max(1))).clamp(0.0, 1.0);
            let inner = REPAIR_WIDTH - 2.0 * REPAIR_PAD;
            let y = top + panel - 6.0;
            painter.rect(x + REPAIR_PAD, y, inner, 3.0, srgb(STEEL, 1.0));
            painter.rect(x + REPAIR_PAD, y, inner * share, 3.0, srgb(TEAL, 1.0));
        }
    } else if view.repairable > 0 {
        let top = bottom - REPAIR_LINE - 4.0;
        painter.rect(x, top, REPAIR_WIDTH, REPAIR_LINE + 4.0, srgb(GRAPHITE, 0.7));
        painter.rect(x, top, 4.0, REPAIR_LINE + 4.0, srgb(TEAL, 1.0));
        painter.text(
            x + REPAIR_PAD,
            top + 5.0,
            2.0,
            &format!("r  repair ({})", view.repairable),
            srgb(TEAL, 1.0),
        );
    }
    painter.quads
}

/// The root chassis' health and the energy pool, at `x`, `y`.
fn bars(painter: &mut Painter, vitals: &Vitals, x: f32, y: f32) {
    painter.rect(x, y, VITALS_WIDTH, VITALS_HEIGHT, srgb(GRAPHITE, 0.82));
    painter.rect(x, y, VITALS_WIDTH, 2.0, srgb(STEEL, 1.0));
    let inner = VITALS_WIDTH - 28.0;
    let share = |value: u32, whole: u32| {
        #[allow(clippy::cast_precision_loss)]
        let share = value as f32 / whole.max(1) as f32;
        share.clamp(0.0, 1.0)
    };
    // The chassis: olive, orange once under a third.
    let health = share(u32::from(vitals.chassis), u32::from(vitals.chassis_whole));
    painter.text(x + 14.0, y + 12.0, 2.0, "chassis", srgb(MUTED, 1.0));
    painter.text_right(
        x + VITALS_WIDTH - 14.0,
        y + 12.0,
        2.0,
        &format!("{}/{}", vitals.chassis, vitals.chassis_whole),
        srgb(PALE, 1.0),
    );
    painter.rect(x + 14.0, y + 29.0, inner, 7.0, srgb(STEEL, 1.0));
    let tone = if health < 1.0 / 3.0 { ORANGE } else { OLIVE };
    painter.rect(x + 14.0, y + 29.0, inner * health, 7.0, srgb(tone, 1.0));
    // The energy, with the share one shot of the held weapon takes marked at its end.
    let charge = share(vitals.energy, vitals.capacity);
    let shot = if vitals.continuous {
        vitals.cost.saturating_mul(u32::from(TICK_HZ))
    } else {
        vitals.cost
    };
    let cost = share(shot.min(vitals.energy), vitals.capacity);
    painter.text(x + 14.0, y + 46.0, 2.0, "energy", srgb(MUTED, 1.0));
    painter.text_right(
        x + VITALS_WIDTH - 14.0,
        y + 46.0,
        2.0,
        &format!("{}/{}", vitals.energy / MILLI, vitals.capacity / MILLI),
        srgb(PALE, 1.0),
    );
    painter.rect(x + 14.0, y + 63.0, inner, 7.0, srgb(STEEL, 1.0));
    painter.rect(x + 14.0, y + 63.0, inner * charge, 7.0, srgb(PALE, 0.85));
    painter.rect(
        x + 14.0 + inner * (charge - cost),
        y + 63.0,
        inner * cost,
        7.0,
        srgb(ORANGE, 1.0),
    );
    if vitals.energy < vitals.cost {
        painter.frame(x + 13.0, y + 62.0, inner + 2.0, 9.0, 1.0, srgb(RED, 1.0));
    }
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
        assert_eq!(units(2_000), "2");
        assert_eq!(units(600), "0.6");
        assert_eq!(units(36_000), "36");
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
        // The flash over the panel and the sweeping band, then neither.
        assert_eq!(switched.len(), settled.len() + 2);
    }
}
