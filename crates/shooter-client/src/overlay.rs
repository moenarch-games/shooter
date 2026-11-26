//! The drawn HUD: the weapons and the chassis and energy, as the client's UI items
//! ([`crate::ui`]: rounded rectangles and text in a real font, drawn as the renderer's overlay),
//! the same natively and in the browser.
//!
//! The HUD has no panels: text floats over the scene on a soft shadow, bars are pills on a dark
//! track and the weapons are rounded chips. Pale grey for what matters, muted grey for the rest,
//! olive for mods and ready weapons, orange for the held weapon's key, a refire and a shot's
//! cost, blue for energy and red for what is lost or short. Everything sits in the bottom
//! corners, clear of the reticle; the layout is for 720 lines and scales with the frame's height.
//!
//! The bottom right shows a chip for every weapon copy the build carries (its select key on a
//! badge, the blade's V, its short name and copy number; lost ones red and struck out), each
//! with its own readiness bar (ready, recharging, short of energy: every copy may fire,
//! shooter#127), the held one opaque with an orange badge and glowing for a moment after a
//! switch. Under them the held weapon's name, its refire filling while it recharges, and a line
//! with its mod and, when it cannot fire, why (lost, low energy). The bottom left shows the root
//! chassis' health as a large reading over its bar and under it the robot's resources
//! (shooter#128): its electricity ("power", blue, the share one shot of the held weapon takes
//! marked on its bar) with what it used and made a second, and its fuel (amber) with what its
//! generators burn a second; in red instead, what cannot work (a walk refused for want of power,
//! a generator without fuel) or, muted, that nothing recharges it (no generator). Above them, in
//! teal, field
//! repair ([`repair_panel`], [`crate::repair_menu`]): the quick repair menu while it is open,
//! else the repair under way, else how many parts may be repaired.

use crate::ui::{self, CAP_HEIGHT, Item};
use render_engine::glam::{Vec2, Vec4};
use shooter_sim::energy::{Flow, MILLI, Resource};
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
    /// The electricity in the HUD.
    pub const ENERGY: u32 = 0x5C_C8_FF;
    /// The fuel in the HUD.
    pub const FUEL: u32 = 0xE6_C2_4C;
    /// Behind the HUD's text and bars: a drop shadow and the empty part of a bar.
    pub const SHADOW: u32 = 0x0B_0D_0F;
}
use crate::repair_menu::{Entry, RepairView};
use palette::{ENERGY, FUEL, GRAPHITE, MUTED, OLIVE, ORANGE, PALE, RED, SHADOW, TEAL};

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

/// The key that selects `weapon`: its digit, V for the blade, or F for the grapple module.
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

/// A weapon's short name on its HUD chip.
const fn short_name(weapon: Weapon) -> &'static str {
    match weapon {
        Weapon::EnergyRifle => "Rifle",
        Weapon::RailGun => "Rail",
        Weapon::RocketLauncher => "Rockets",
        Weapon::Grapple => "Grapple",
        Weapon::EnergyBlade => "Blade",
        Weapon::Shotgun => "Shotgun",
        Weapon::Laser => "Laser",
        Weapon::Flamethrower => "Flamer",
    }
}

/// The UI's items ([`crate::ui`]) in layout units (720 lines high), scaled to the frame; shapes
/// snapped to whole pixels. Text is set in capitals, a font `cell` the seventh of its cap height
/// (the layout's unit for text sizes).
pub(crate) struct Painter {
    pub(crate) quads: Vec<Item>,
    pub(crate) scale: f32,
}

/// A text's cap height in cells.
const CAP_CELLS: f32 = 6.0;

/// The font size, in layout units, of text `cell` units a cell: six cells its cap height,
/// centred in a line box seven cells high (the layout's unit for text).
fn font_size(cell: f32) -> f32 {
    CAP_CELLS * cell / CAP_HEIGHT
}

impl Painter {
    pub(crate) fn rect(&mut self, x: f32, y: f32, width: f32, height: f32, color: Vec4) {
        self.round_rect(x, y, width, height, 0.0, color);
    }

    /// A rectangle with its corners rounded to `radius` units.
    pub(crate) fn round_rect(
        &mut self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        radius: f32,
        color: Vec4,
    ) {
        if width <= 0.0 || height <= 0.0 || color.w <= 0.0 {
            return;
        }
        let snap = |value: f32| (value * self.scale).round();
        let (left, top) = (snap(x), snap(y));
        let size = Vec2::new(
            (snap(x + width) - left).max(1.0),
            (snap(y + height) - top).max(1.0),
        );
        self.quads.push(Item::Rect {
            position: Vec2::new(left, top),
            size,
            radius: radius * self.scale,
            color,
        });
    }

    /// A pill: a bar with fully rounded ends, `share` of it filled with `fill` over `track`.
    pub(crate) fn pill(
        &mut self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        share: f32,
        (track, fill): (Vec4, Vec4),
    ) {
        let radius = height / 2.0;
        self.round_rect(x, y, width, height, radius, track);
        let filled = width * share.clamp(0.0, 1.0);
        if filled > 0.0 {
            self.round_rect(x, y, filled.max(height), height, radius, fill);
        }
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

    /// `text` in capitals in a line box `7 × cell` units high from `y`, left-aligned at `x`;
    /// its width.
    pub(crate) fn text(&mut self, x: f32, y: f32, cell: f32, text: &str, color: Vec4) -> f32 {
        let cell = self.snap_cell(cell);
        let upper = text.to_uppercase();
        let baseline = y + (7.0 + CAP_CELLS) / 2.0 * cell;
        let pen = Vec2::new(x * self.scale, (baseline * self.scale).round());
        self.quads
            .extend(ui::glyphs(&upper, pen, font_size(cell) * self.scale, color));
        text_width(text, cell)
    }

    /// `text` right-aligned to `right`.
    pub(crate) fn text_right(&mut self, right: f32, y: f32, cell: f32, text: &str, color: Vec4) {
        let width = text_width(text, self.snap_cell(cell));
        self.text(right - width, y, cell, text, color);
    }

    /// `text` over a soft shadow one pixel down and right, legible on any part of the scene
    /// without a panel; its width.
    pub(crate) fn label(&mut self, x: f32, y: f32, cell: f32, text: &str, color: Vec4) -> f32 {
        let offset = 1.0 / self.scale;
        let shadow = srgb(SHADOW, 0.55 * color.w);
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

    /// A text cell as a whole number of pixels (even text sizes at any height), back in units.
    pub(crate) fn snap_cell(&self, cell: f32) -> f32 {
        (cell * self.scale * 2.0).round().max(1.0) / (2.0 * self.scale)
    }
}

/// `text`'s width in units, set in capitals at `cell` units a cell.
pub(crate) fn text_width(text: &str, cell: f32) -> f32 {
    ui::advance(&text.to_uppercase(), font_size(cell))
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

    /// How ready it is to fire with what `vitals` holds of its resource (the flamethrower's
    /// fuel, the rest's electricity, shooter#130): lost, short of it, recovering (with the share
    /// of its refire already over) or ready.
    #[must_use]
    pub fn readiness(&self, vitals: &Vitals) -> Readiness {
        if !self.armed {
            Readiness::Lost
        } else if vitals.stock(self.instance.weapon.resource()) < self.cost {
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
    /// Its electricity and storage (milli-units).
    pub energy: u32,
    pub capacity: u32,
    /// Its fuel and its tanks' storage (milli-units).
    pub fuel: u32,
    pub fuel_capacity: u32,
    /// The fuel its connected generators may burn a tick (none: nothing recharges it).
    pub burn: u32,
    /// What its locomotion runs on (shooter#130): electricity, fuel through an engine, or
    /// nothing (a combustion chassis without an engine).
    pub drive: Option<Resource>,
    /// What its resources did in the last tick: used, made, burned, a refused walk.
    pub flow: Flow,
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
            fuel: fighter.fuel,
            fuel_capacity: fighter.pool().fuel,
            burn: fighter.pool().burn,
            drive: fighter.pool().drive(),
            flow: fighter.flow,
            chassis: fighter.chassis_health(),
            chassis_whole: shooter_sim::damage::PartDamage::fresh(build).chassis_health(build),
        }
    }

    /// What it holds of `resource` (milli-units).
    #[must_use]
    pub const fn stock(&self, resource: Resource) -> u32 {
        match resource {
            Resource::Electricity => self.energy,
            Resource::Fuel => self.fuel,
        }
    }

    /// The resource the held weapon runs on.
    #[must_use]
    pub const fn resource(&self) -> Resource {
        self.held.weapon.resource()
    }

    /// Whether it holds less of the held weapon's resource than an activation costs.
    #[must_use]
    pub const fn short(&self) -> bool {
        self.stock(self.resource()) < self.cost
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
    /// The electricity used and made and the fuel burned a second, smoothed over the last
    /// half second or so (units a second): the HUD's consumption readings.
    rates: Rates,
}

/// Resource rates a second ([`Overlay::rates`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rates {
    pub used: f32,
    pub made: f32,
    pub burned: f32,
}

/// How long the consumption readings take to follow a change (seconds).
const RATE_SECONDS: f32 = 0.5;

impl Rates {
    /// The rates moved toward `flow` (a tick's amounts) over `seconds`.
    fn follow(&mut self, flow: Flow, seconds: f32) {
        #[allow(clippy::cast_precision_loss)]
        let per_second = |amount: u32| amount as f32 * f32::from(TICK_HZ) / MILLI as f32;
        let step = (seconds / RATE_SECONDS).clamp(0.0, 1.0);
        self.used += (per_second(flow.used) - self.used) * step;
        self.made += (per_second(flow.made) - self.made) * step;
        // Every fuel the tick burned: the generators', an engine's and a flamethrower's.
        self.burned += (per_second(flow.burned + flow.fueled) - self.burned) * step;
    }
}

/// Layout units (the frame is 720 high in them).
const MARGIN: f32 = 28.0;
const BOTTOM: f32 = 720.0 - MARGIN;
/// The chassis and energy bars' width.
const BAR_WIDTH: f32 = 240.0;
/// The top of the bottom left block (the chassis reading): the repair panel sits above it.
const VITALS_TOP: f32 = BOTTOM - 86.0;
/// The held weapon's name, its cap top.
const NAME_TOP: f32 = BOTTOM - 46.0;
/// The weapon chips: their height, the gap between them and their top.
const CHIP_HEIGHT: f32 = 28.0;
const CHIP_GAP: f32 = 6.0;
const CHIPS_TOP: f32 = NAME_TOP - 16.0 - CHIP_HEIGHT;
/// A chip's key badge and its padding.
const BADGE: f32 = 16.0;
const CHIP_PAD: f32 = 6.0;
/// The chips' text size (a cell, layout units), the small text's elsewhere.
const SMALL: f32 = 1.85;
/// The held weapon's name and the chassis reading (cells).
const NAME: f32 = 3.5;
const READING: f32 = 4.6;
const KEY: f32 = 1.6;
/// The refire bar under the held weapon's name.
const REFIRE_WIDTH: f32 = 180.0;
/// The track under every bar.
fn track() -> Vec4 {
    srgb(SHADOW, 0.5)
}

impl Overlay {
    /// The UI items for a `width`×`height` frame `seconds` after the last; nothing while the
    /// local fighter is not alive.
    pub fn draw(
        &mut self,
        vitals: Option<&Vitals>,
        width: u32,
        height: u32,
        seconds: f32,
    ) -> Vec<Item> {
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
        self.rates.follow(vitals.flow, seconds);
        #[allow(clippy::cast_precision_loss)]
        let (width, height) = (width as f32, height as f32);
        let scale = height / 720.0;
        let mut painter = Painter {
            quads: Vec::with_capacity(256),
            scale,
        };
        let right = width / scale - MARGIN;
        chips(&mut painter, vitals, right, self.highlight());
        self.held_weapon(&mut painter, vitals, right);
        bars(&mut painter, vitals, self.rates, MARGIN);
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
        let short = vitals.short();
        painter.label_right(
            right,
            NAME_TOP,
            NAME,
            &vitals.held_name(),
            srgb(if armed { PALE } else { MUTED }, 1.0),
        );
        let line = NAME_TOP + 7.0 * painter.snap_cell(NAME) + 6.0;
        if armed && !short && vitals.cooldown > 0 && self.period > 0 {
            #[allow(clippy::cast_precision_loss)]
            let over = 1.0 - vitals.cooldown as f32 / self.period as f32;
            painter.pill(
                right - REFIRE_WIDTH,
                line - 6.0,
                REFIRE_WIDTH,
                3.0,
                over,
                (track(), srgb(ORANGE, 1.0)),
            );
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
            at -= painter.label_right(at, line, SMALL, label, srgb(color, 1.0)) + 14.0;
        }
        if let Some(id) = vitals.held_mod {
            painter.label_right(at, line, SMALL, mod_name(id), srgb(OLIVE, 1.0));
        }
    }
}

/// A chip's text: the weapon's short name, numbered among its copies.
fn chip_name(mounted: &Mounted) -> String {
    let name = short_name(mounted.instance.weapon);
    match mounted.number() {
        Some(number) => format!("{name} {number}"),
        None => name.to_owned(),
    }
}

/// The carried weapon copies as rounded chips in a row ending at `right`: each its key on a
/// badge, its short name (numbered when the build mounts its weapon more than once, shooter#125)
/// and its own readiness along its foot (shooter#127: every copy may fire, held or not): olive
/// when ready, the refire filling orange, red when the pool cannot pay it. The held one is
/// opaque with an orange badge and glows for a moment after a switch; lost ones are red and
/// struck out.
fn chips(painter: &mut Painter, vitals: &Vitals, right: f32, flash: f32) {
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
    let cell = painter.snap_cell(SMALL);
    let widths: Vec<f32> = order
        .iter()
        .map(|mounted| {
            CHIP_PAD + BADGE + CHIP_PAD + text_width(&chip_name(mounted), cell) + CHIP_PAD + 2.0
        })
        .collect();
    #[allow(clippy::cast_precision_loss)]
    let total = widths.iter().sum::<f32>() + (widths.len().max(1) - 1) as f32 * CHIP_GAP;
    let (mut x, y) = (right - total, CHIPS_TOP);
    for (mounted, width) in order.iter().zip(widths) {
        let (weapon, armed) = (mounted.instance.weapon, mounted.armed);
        let held = mounted.instance == vitals.held;
        if held && flash > 0.0 {
            painter.round_rect(
                x - 4.0,
                y - 4.0,
                width + 8.0,
                CHIP_HEIGHT + 8.0,
                10.0,
                srgb(ORANGE, 0.4 * flash),
            );
        }
        let background = if held {
            srgb(GRAPHITE, 0.92)
        } else {
            srgb(SHADOW, 0.45)
        };
        painter.round_rect(x, y, width, CHIP_HEIGHT, 7.0, background);
        let badge_y = y + (CHIP_HEIGHT - BADGE) / 2.0 - 1.0;
        let (badge, key_color) = match (armed, held) {
            (false, _) => (srgb(RED, 0.35), srgb(PALE, 0.8)),
            (true, true) => (srgb(ORANGE, 1.0), srgb(GRAPHITE, 1.0)),
            (true, false) => (srgb(PALE, 0.14), srgb(PALE, 0.9)),
        };
        painter.round_rect(x + CHIP_PAD, badge_y, BADGE, BADGE, 4.0, badge);
        let key = key_label(weapon);
        let key_cell = painter.snap_cell(KEY);
        painter.text(
            x + CHIP_PAD + (BADGE - text_width(&key, key_cell)) / 2.0,
            badge_y + (BADGE - 7.0 * key_cell) / 2.0,
            KEY,
            &key,
            key_color,
        );
        let name_x = x + CHIP_PAD + BADGE + CHIP_PAD;
        let name_y = badge_y + (BADGE - 7.0 * cell) / 2.0;
        let tone = match (armed, held) {
            (false, _) => srgb(RED, 0.8),
            (true, true) => srgb(PALE, 1.0),
            (true, false) => srgb(MUTED, 1.0),
        };
        let name_width = painter.text(name_x, name_y, SMALL, &chip_name(mounted), tone);
        if !armed {
            painter.rect(
                name_x - 1.0,
                name_y + 3.5 * cell - 0.75,
                name_width + 2.0,
                1.5,
                srgb(RED, 0.9),
            );
        }
        let (bar_x, bar_y, bar_width) = (x + 8.0, y + CHIP_HEIGHT - 5.0, width - 16.0);
        match mounted.readiness(vitals) {
            Readiness::Lost => {}
            Readiness::LowEnergy => {
                painter.pill(bar_x, bar_y, bar_width, 2.0, 1.0, (track(), srgb(RED, 0.9)));
            }
            Readiness::Recharging(over) => {
                painter.pill(
                    bar_x,
                    bar_y,
                    bar_width,
                    2.0,
                    over,
                    (track(), srgb(ORANGE, 1.0)),
                );
            }
            Readiness::Ready => {
                painter.pill(
                    bar_x,
                    bar_y,
                    bar_width,
                    2.0,
                    1.0,
                    (track(), srgb(OLIVE, 0.9)),
                );
            }
        }
        x += width + CHIP_GAP;
    }
}

/// The field repair panel's width, a line's height and its padding (layout units).
const REPAIR_WIDTH: f32 = 300.0;
const REPAIR_LINE: f32 = 22.0;
const REPAIR_PAD: f32 = 12.0;

/// Precision aim's scope (shooter#132) for a frame `width` × `height` pixels, `zoom` of the way
/// in (0 to 1, [`crate::camera::Precision::zoom`]): the frame darkened outside a square sight
/// centred on the reticle, and fine cross hairs from the sight's edge toward its centre that
/// leave the middle clear for the reticle and the guns' marks. Nothing while out.
#[must_use]
pub fn scope(width: u32, height: u32, zoom: f32) -> Vec<Item> {
    let zoom = zoom.clamp(0.0, 1.0);
    if zoom <= 0.0 {
        return Vec::new();
    }
    #[allow(clippy::cast_precision_loss)]
    let scale = height as f32 / 720.0;
    let mut painter = Painter {
        quads: Vec::with_capacity(12),
        scale,
    };
    #[allow(clippy::cast_precision_loss)]
    let wide = width as f32 / scale;
    let (cx, cy) = (wide / 2.0, 360.0);
    // The sight: a square 600 units across, the rest of the frame shaded.
    let half = 300.0;
    let shade = Vec4::new(0.0, 0.0, 0.0, 0.55 * zoom);
    painter.rect(0.0, 0.0, wide, cy - half, shade);
    painter.rect(0.0, cy + half, wide, 720.0 - (cy + half), shade);
    painter.rect(0.0, cy - half, cx - half, 2.0 * half, shade);
    painter.rect(cx + half, cy - half, wide - (cx + half), 2.0 * half, shade);
    // Its edge and the cross hairs, leaving the middle 80 units clear.
    let line = Vec4::new(0.85, 0.95, 1.0, 0.6 * zoom);
    painter.frame(cx - half, cy - half, 2.0 * half, 2.0 * half, 1.5, line);
    let (gap, thin) = (40.0, 1.5);
    painter.rect(cx - half, cy - thin / 2.0, half - gap, thin, line);
    painter.rect(cx + gap, cy - thin / 2.0, half - gap, thin, line);
    painter.rect(cx - thin / 2.0, cy - half, thin, half - gap, line);
    painter.rect(cx - thin / 2.0, cy + gap, thin, half - gap, line);
    painter.quads
}

/// The owner's self-destruct warning (shooter#131) for a frame `width` × `height` pixels while
/// its countdown runs, `ticks` before the blast: a red banner above the reticle with the seconds
/// left, the blast's strength from what the robot carries now (`damage` at its centre,
/// [`shooter_sim::destruct::Charge`]; "empty" when it would only die) and how to cancel, over a
/// bar that drains to the blast. It blinks faster in the last second.
#[must_use]
pub fn destruct_warning(ticks: u32, damage: i32, width: u32, height: u32) -> Vec<Item> {
    #[allow(clippy::cast_precision_loss)]
    let (width, height) = (width as f32, height as f32);
    let scale = height / 720.0;
    let mut painter = Painter {
        quads: Vec::with_capacity(96),
        scale,
    };
    let centre = width / scale / 2.0;
    let (panel_width, top) = (360.0, 150.0);
    let x = centre - panel_width / 2.0;
    let period = if ticks < u32::from(TICK_HZ) { 10 } else { 30 };
    let lit = ticks % period < period / 2;
    painter.round_rect(
        x,
        top,
        panel_width,
        74.0,
        10.0,
        srgb(RED, if lit { 0.78 } else { 0.5 }),
    );
    #[allow(clippy::cast_precision_loss)]
    let seconds = ticks as f32 / f32::from(TICK_HZ);
    let line = format!("SELF-DESTRUCT {seconds:.1} S");
    let cell = painter.snap_cell(NAME);
    painter.text(
        centre - text_width(&line, cell) / 2.0,
        top + 8.0,
        NAME,
        &line,
        srgb(PALE, 1.0),
    );
    let strength = if damage > 0 {
        format!("blast {damage}   [Backspace] cancel")
    } else {
        "empty: no blast   [Backspace] cancel".to_owned()
    };
    let cell = painter.snap_cell(SMALL);
    painter.text(
        centre - text_width(&strength, cell) / 2.0,
        top + 36.0,
        SMALL,
        &strength,
        srgb(PALE, 0.9),
    );
    #[allow(clippy::cast_precision_loss)]
    let share = ticks as f32 / shooter_sim::destruct::COUNTDOWN_TICKS as f32;
    painter.pill(
        x + 14.0,
        top + 60.0,
        panel_width - 28.0,
        6.0,
        share,
        (srgb(SHADOW, 0.6), srgb(PALE, 0.95)),
    );
    painter.quads
}

/// The field repair panel (shooter#61) for a frame `height` pixels high, bottom left above the
/// chassis and energy: the open quick repair menu (only the parts the robot may repair now, the
/// cursor's line in the accent), else the repair under way (its part's health, paused by a hit
/// in red), else a hint that the repair action has something to repair. Nothing when there is
/// none of these.
#[must_use]
pub fn repair_panel(view: &RepairView, height: u32) -> Vec<Item> {
    #[allow(clippy::cast_precision_loss)]
    let scale = height as f32 / 720.0;
    let mut painter = Painter {
        quads: Vec::with_capacity(256),
        scale,
    };
    let x = MARGIN;
    let bottom = VITALS_TOP - 16.0;
    let text_y = (REPAIR_LINE - 7.0 * SMALL) / 2.0;
    if let Some((entries, cursor)) = &view.menu {
        // The menu is a list over the scene: the one block with a backdrop.
        #[allow(clippy::cast_precision_loss)]
        let panel = REPAIR_LINE * (entries.len() + 1) as f32 + 2.0 * REPAIR_PAD - 6.0;
        let top = bottom - panel;
        painter.round_rect(x, top, REPAIR_WIDTH, panel, 10.0, srgb(SHADOW, 0.62));
        let title = top + REPAIR_PAD - 4.0;
        painter.text(
            x + REPAIR_PAD,
            title + text_y,
            SMALL,
            "repair",
            srgb(TEAL, 1.0),
        );
        painter.text_right(
            x + REPAIR_WIDTH - REPAIR_PAD,
            title + text_y,
            SMALL,
            "wheel  ·  r",
            srgb(MUTED, 1.0),
        );
        for (row, entry) in entries.iter().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let y = title + REPAIR_LINE * (row + 1) as f32;
            let chosen = row == *cursor;
            if chosen {
                painter.round_rect(
                    x + 4.0,
                    y,
                    REPAIR_WIDTH - 8.0,
                    REPAIR_LINE,
                    6.0,
                    srgb(TEAL, 0.24),
                );
            }
            let color = srgb(if chosen { PALE } else { MUTED }, 1.0);
            entry_line(
                &mut painter,
                x + REPAIR_PAD,
                REPAIR_WIDTH - 2.0 * REPAIR_PAD,
                y + text_y,
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
            SMALL,
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
            let share = f32::from(*health) / f32::from((*whole).max(1));
            painter.pill(
                x,
                bottom - 3.0,
                BAR_WIDTH,
                3.0,
                share,
                (track(), srgb(TEAL, 1.0)),
            );
        }
    } else if view.repairable > 0 {
        painter.label(
            x,
            bottom - 12.0,
            SMALL,
            &format!("r  ·  repair {}", view.repairable),
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
            painter.label(x, y, SMALL, name, color);
            painter.label_right(x + width, y, SMALL, &format!("{health}/{whole}"), color);
        }
        Entry::Stop => {
            painter.label(x, y, SMALL, "stop repair", color);
        }
    }
}

/// What keeps a resource's consumers or producers from working, if anything (shooter#128).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Shortage {
    /// The locomotion was refused the electricity to walk.
    DriveStalled,
    /// A combustion drive was refused the fuel to walk (shooter#130).
    DriveNoFuel,
    /// A combustion chassis has no engine connected: it cannot walk (shooter#130).
    NoEngine,
    /// No generator is connected: nothing recharges the electricity.
    NoGenerator,
    /// The generators have no fuel left to burn.
    NoFuel,
}

impl Shortage {
    /// Its words on the HUD.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::DriveStalled => "drive: no power",
            Self::DriveNoFuel => "drive: no fuel",
            Self::NoEngine => "drive: no engine",
            Self::NoGenerator => "no generator",
            Self::NoFuel => "generator: no fuel",
        }
    }
}

impl Vitals {
    /// What stops the electricity's consumers: an electric drive's refused walk.
    #[must_use]
    pub const fn power_shortage(&self) -> Option<Shortage> {
        if self.flow.stalled && matches!(self.drive, Some(Resource::Electricity)) {
            Some(Shortage::DriveStalled)
        } else {
            None
        }
    }

    /// What stops the fuel's consumers or the recharge: a combustion drive's refused walk (no
    /// engine, or no fuel), else no generator connected, or no fuel for it.
    #[must_use]
    pub const fn fuel_shortage(&self) -> Option<Shortage> {
        if self.flow.stalled && self.drive.is_none() {
            Some(Shortage::NoEngine)
        } else if self.flow.stalled && matches!(self.drive, Some(Resource::Fuel)) {
            Some(Shortage::DriveNoFuel)
        } else if self.burn == 0 {
            Some(Shortage::NoGenerator)
        } else if self.fuel == 0 {
            Some(Shortage::NoFuel)
        } else {
            None
        }
    }
}

/// A rate a second as the HUD reads it: whole units, with its sign.
fn rate(sign: char, value: f32) -> String {
    format!("{sign}{value:.0}")
}

/// One resource's row from `y`: its name and amount on the left in `color`, on the right what
/// keeps it from working (red) or its rates a second (muted), and its thin bar under them.
#[allow(clippy::too_many_arguments)]
fn resource(
    painter: &mut Painter,
    x: f32,
    y: f32,
    (name, color): (&str, u32),
    (amount, whole): (u32, u32),
    shortage: Option<Shortage>,
    rates: &str,
    track: Vec4,
) -> f32 {
    painter.label(
        x,
        y,
        SMALL,
        &format!("{name} {}", amount / MILLI),
        srgb(color, 1.0),
    );
    match shortage {
        Some(shortage) => {
            painter.label_right(x + BAR_WIDTH, y, SMALL, shortage.label(), srgb(RED, 1.0));
        }
        None if !rates.is_empty() => {
            painter.label_right(x + BAR_WIDTH, y, SMALL, rates, srgb(MUTED, 1.0));
        }
        None => {}
    }
    let bar_y = y + 7.0 * painter.snap_cell(SMALL) + 1.0;
    #[allow(clippy::cast_precision_loss)]
    let share = (amount as f32 / whole.max(1) as f32).clamp(0.0, 1.0);
    painter.pill(x, bar_y, BAR_WIDTH, 4.0, share, (track, srgb(color, 1.0)));
    bar_y
}

/// The root chassis' health (a large reading over its bar), then the robot's resources
/// (shooter#128): its electricity (a thin bar, the share one shot of the held weapon takes marked
/// at its end) and its fuel, each with its amount, its rates a second (`rates`: electricity used
/// and made, fuel burned) and what keeps it from working, from `x`.
fn bars(painter: &mut Painter, vitals: &Vitals, rates: Rates, x: f32) {
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
        READING,
        &vitals.chassis.to_string(),
        srgb(tone, 1.0),
    );
    // The small text's cap bottom on the reading's.
    let small = VITALS_TOP + 6.5 * (painter.snap_cell(READING) - painter.snap_cell(SMALL));
    painter.label(
        x + reading + 6.0,
        small,
        SMALL,
        &format!("/ {}", vitals.chassis_whole),
        srgb(MUTED, 1.0),
    );
    let chassis_y = VITALS_TOP + 37.0;
    painter.pill(
        x,
        chassis_y,
        BAR_WIDTH,
        7.0,
        health,
        (track(), srgb(tone, 1.0)),
    );
    let charge = share(vitals.energy, vitals.capacity);
    let shot = if vitals.continuous {
        vitals.cost.saturating_mul(u32::from(TICK_HZ))
    } else {
        vitals.cost
    };
    let cost = share(shot.min(vitals.energy), vitals.capacity);
    let energy_track = if vitals.short() && vitals.resource() == Resource::Electricity {
        srgb(RED, 0.55)
    } else {
        track()
    };
    let energy_y = resource(
        painter,
        x,
        chassis_y + 11.0,
        ("power", ENERGY),
        (vitals.energy, vitals.capacity),
        vitals.power_shortage(),
        &format!("{}  {} /s", rate('-', rates.used), rate('+', rates.made)),
        energy_track,
    );
    let burning = if rates.burned >= 0.5 {
        format!("{} /s", rate('-', rates.burned))
    } else {
        String::new()
    };
    resource(
        painter,
        x,
        energy_y + 7.0,
        ("fuel", FUEL),
        (vitals.fuel, vitals.fuel_capacity),
        vitals.fuel_shortage(),
        &burning,
        track(),
    );
    if cost > 0.0 {
        painter.round_rect(
            x + BAR_WIDTH * (charge - cost),
            energy_y,
            (BAR_WIDTH * cost).max(4.0),
            4.0,
            2.0,
            srgb(ORANGE, 1.0),
        );
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
            .map(|mounted| mounted.readiness(&vitals))
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
                .all(|mounted| mounted.readiness(&vitals) == Readiness::LowEnergy)
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
            .map(|mounted| (mounted.cooldown, mounted.readiness(&vitals)))
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
                let (min, max) = quad.bounds();
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

    /// The resources on the HUD (shooter#128): electricity and fuel with their storage, the rates
    /// a second following the ticks' flow, and what cannot work: a walk refused for want of
    /// power, a generator out of fuel, a robot without one.
    #[test]
    fn the_hud_shows_both_pools_their_rates_and_what_cannot_work() {
        use shooter_sim::robot::{BETA, STANDARD};
        let vitals = standard();
        assert_eq!((vitals.energy, vitals.capacity), (100_000, 100_000));
        assert_eq!((vitals.fuel, vitals.fuel_capacity), (1_000_000, 1_000_000));
        assert_eq!(vitals.burn, 200);
        assert_eq!(
            (vitals.power_shortage(), vitals.fuel_shortage()),
            (None, None)
        );

        // A tick's flow held for 10 s: 92 used, 400 made from 200 burned, at 60 ticks a second.
        let mut rates = Rates::default();
        let flow = Flow {
            used: 92,
            fueled: 0,
            made: 400,
            burned: 200,
            stalled: false,
        };
        for _ in 0..600 {
            rates.follow(flow, 1.0 / 60.0);
        }
        assert!((rates.used - 5.52).abs() < 0.05, "{rates:?}");
        assert!((rates.made - 24.0).abs() < 0.1, "{rates:?}");
        assert!((rates.burned - 12.0).abs() < 0.1, "{rates:?}");

        // Drained of both: the walk refused, the generator without fuel.
        let mut session = Session::new(shooter_sim::Arena::greybox()).unwrap();
        // On its feet first.
        for _ in 0..30 {
            session.tick(shooter_sim::Intent::IDLE).unwrap();
        }
        session.fighters[0].energy = 0;
        session.fighters[0].fuel = 0;
        session
            .tick(shooter_sim::Intent {
                forward: 1,
                ..shooter_sim::Intent::IDLE
            })
            .unwrap();
        let drained = Vitals::of(session.local(), session.ticks());
        assert_eq!(drained.power_shortage(), Some(Shortage::DriveStalled));
        assert_eq!(drained.fuel_shortage(), Some(Shortage::NoFuel));
        assert_eq!(Shortage::DriveStalled.label(), "drive: no power");
        let quads = Overlay::default().draw(Some(&drained), 1280, 720, 0.016);
        assert!(!quads.is_empty());

        // No generator at all: the standard build without its roof's tank and generator.
        let mut bare = STANDARD.candidate();
        bare.parts.retain(|part| part.id < 20);
        session.rebuild(0, BETA.validate(&bare).unwrap()).unwrap();
        let bare = Vitals::of(session.local(), session.ticks());
        assert_eq!(bare.fuel_shortage(), Some(Shortage::NoGenerator));
    }

    /// A combustion robot on the HUD (shooter#130): its drive's refusal on the fuel row (no
    /// fuel, or no engine), its flamethrower's readiness by the fuel it holds and its rifle's by
    /// the electricity.
    #[test]
    fn the_hud_reads_a_combustion_drive_and_each_weapon_by_its_resource() {
        use shooter_sim::energy::Resource;
        use shooter_sim::robot::{BETA, STOKER};
        let walk = shooter_sim::Intent {
            forward: 1,
            ..shooter_sim::Intent::IDLE
        };
        let mut session = Session::new(shooter_sim::Arena::greybox()).unwrap();
        session.rebuild(0, STOKER).unwrap();
        for _ in 0..30 {
            session.tick(shooter_sim::Intent::IDLE).unwrap();
        }
        let fighter = &mut session.fighters[0];
        fighter.fuel = 0;
        fighter.energy = fighter.pool().capacity;
        session.tick(walk).unwrap();
        let dry = Vitals::of(session.local(), session.ticks());
        assert_eq!(dry.drive, Some(Resource::Fuel));
        assert_eq!(
            (dry.power_shortage(), dry.fuel_shortage()),
            (None, Some(Shortage::DriveNoFuel))
        );
        let readiness = |vitals: &Vitals, weapon: Weapon| {
            vitals
                .carried
                .iter()
                .find(|mounted| mounted.instance.weapon == weapon)
                .unwrap()
                .readiness(vitals)
        };
        assert_eq!(
            readiness(&dry, Weapon::Flamethrower),
            Readiness::LowEnergy,
            "no fuel, no flame"
        );
        assert_ne!(readiness(&dry, Weapon::EnergyRifle), Readiness::LowEnergy);
        assert!(
            !Overlay::default()
                .draw(Some(&dry), 1280, 720, 0.016)
                .is_empty()
        );

        // Without its engine it never walks.
        let mut candidate = STOKER.candidate();
        candidate.parts.retain(|part| part.id != 22);
        session
            .rebuild(0, BETA.validate(&candidate).unwrap())
            .unwrap();
        session.fighters[0].fuel = session.fighters[0].pool().fuel;
        session.tick(walk).unwrap();
        let stuck = Vitals::of(session.local(), session.ticks());
        assert_eq!(stuck.drive, None);
        assert_eq!(stuck.fuel_shortage(), Some(Shortage::NoEngine));
        assert_eq!(Shortage::NoEngine.label(), "drive: no engine");
    }

    /// Precision aim's scope (shooter#132): nothing while out; in, it shades round a square
    /// sight and draws cross hairs that keep the reticle's middle clear, within the frame at
    /// any size.
    #[test]
    fn the_scope_frames_the_reticle_and_leaves_its_middle_clear() {
        assert!(scope(1280, 720, 0.0).is_empty());
        for (width, height) in [(1280, 720), (1920, 1080), (1024, 768)] {
            let quads = scope(width, height, 1.0);
            assert!(quads.len() >= 8);
            #[allow(clippy::cast_precision_loss)]
            let (w, h) = (width as f32, height as f32);
            let centre = Vec2::new(w / 2.0, h / 2.0);
            for quad in &quads {
                let Item::Rect { position, size, .. } = *quad else {
                    panic!("the scope is rectangles only: {quad:?}");
                };
                let (min, max) = (position, position + size);
                assert!(
                    min.x >= -1.0 && min.y >= -1.0 && max.x <= w + 1.0 && max.y <= h + 1.0,
                    "{quad:?}"
                );
                // Nothing within 5 % of the frame's height of its centre.
                let near = Vec2::new(centre.x.clamp(min.x, max.x), centre.y.clamp(min.y, max.y));
                assert!(
                    near.distance(centre) > h * 0.05,
                    "{quad:?} at {width}×{height}"
                );
            }
        }
        // Half in, half as dark.
        let (full, half) = (scope(1280, 720, 1.0), scope(1280, 720, 0.5));
        let alpha = |item: &Item| match *item {
            Item::Rect { color, .. } => color.w,
            Item::Glyph { .. } | Item::Image { .. } => panic!("the scope is rectangles only"),
        };
        assert!((alpha(&half[0]) * 2.0 - alpha(&full[0])).abs() < 1e-6);
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
