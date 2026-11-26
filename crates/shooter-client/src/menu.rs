//! The main menu (shooter#154, Atlas D2.i154.1): the first screen natively and in the browser
//! unless the command line or the page's query starts something directly (`--connect` /
//! `?connect`, `--map` / `?map`, the profile runs). Drawn with the client's UI ([`crate::ui`])
//! over a live view of a real arena and the robot the person plays, the same on either renderer
//! and on a touch screen (a tap is a click), with the same choices natively and in the browser:
//!
//! - **Play Online**: for now an in-process deathmatch against bots on the assembly works with
//!   [`PLAY_ONLINE_FIGHTERS`] fighters, labelled as the offline bot match it is (no public host
//!   exists yet; the owner's temporary choice, 2026-11-25).
//! - **Practice**: one of [`PRACTICE_MAPS`] (the small, medium and large kit-built arenas; the
//!   greybox maps stay test fixtures, reachable with `--map` / `?map`), previewed by a render of
//!   the real arena, and how many fighters (the person included) within the map's capacity from
//!   `shooter_sim::maps`, by default its most ([`MapInfo::default_fighters`]).
//! - **Robot Workshop**: the robot builder without a match ([`crate::builder::view::Stage`]).
//! - **Private Match**: shown, not available yet: nothing happens.
//! - **Settings**: the name played under online and the look sensitivity ([`crate::prefs`]), and
//!   under the advanced developer connection a hosted match's URL and certificate to join.
//!
//! The menu decides nothing about matches: it hands back a [`Choice`] the app starts with the
//! existing practice session, builder and connection.

use std::rc::Rc;
use std::sync::OnceLock;

use crate::gfx::{TextureId, World};
use crate::overlay::palette::{GRAPHITE, MUTED, ORANGE, PALE, RED, SHADOW, STEEL};
use crate::overlay::{Painter, srgb, text_width};
use crate::prefs::{PrefStore, Prefs};
use crate::ui::Item;
use render_engine::glam::{Vec2, Vec4};
use render_engine::material::{Sampler, TextureData};
use render_engine::wgpu;
use shooter_sim::MapId;
use shooter_sim::session::DEATHMATCH_SLOTS;
use winit::keyboard::KeyCode;

/// Play Online's map while it is a bot match.
pub const PLAY_ONLINE_MAP: MapId = MapId::AssemblyWorks;
/// Play Online's fighters: the person and eleven bots, the default deathmatch's slots.
pub const PLAY_ONLINE_FIGHTERS: u8 = DEATHMATCH_SLOTS;
/// The maps Practice offers: small, medium and large.
pub const PRACTICE_MAPS: [MapId; 3] = [MapId::ProvingCell, MapId::AssemblyWorks, MapId::Drydock];

/// A map's name for people.
#[must_use]
pub const fn map_name(map: MapId) -> &'static str {
    match map {
        MapId::Greybox => "Greybox",
        MapId::GreyboxYard => "Greybox Yard",
        MapId::AssemblyWorks => "Assembly Works",
        MapId::ProvingCell => "Proving Cell",
        MapId::Drydock => "Drydock",
    }
}

/// What Practice shows of a map, from the catalog.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MapInfo {
    pub map: MapId,
    /// The floor's side in metres.
    pub metres: i32,
    /// The fewest and most fighters Practice offers: the map's capacity, bounded by its spawn
    /// points.
    pub fewest: u8,
    pub most: u8,
}

impl MapInfo {
    #[must_use]
    pub fn of(map: MapId) -> Self {
        let capacity = map.capacity();
        let most = capacity.max.min(map.max_slots());
        Self {
            map,
            metres: map.arena().half_size * 2 / 100,
            fewest: capacity.min.min(most),
            most,
        }
    }

    /// The fighters Practice starts with: the map's most (6 on the proving cell, 12 on the
    /// assembly works and the drydock).
    #[must_use]
    pub const fn default_fighters(&self) -> u8 {
        self.most
    }

    const fn class(&self) -> &'static str {
        match self.map {
            MapId::ProvingCell => "Small",
            MapId::AssemblyWorks => "Medium",
            MapId::Drydock => "Large",
            MapId::Greybox | MapId::GreyboxYard => "Test",
        }
    }
}

/// [`PRACTICE_MAPS`]' [`MapInfo`], worked out once.
fn practice_maps() -> &'static [MapInfo; 3] {
    static INFOS: OnceLock<[MapInfo; 3]> = OnceLock::new();
    INFOS.get_or_init(|| PRACTICE_MAPS.map(MapInfo::of))
}

/// The maps' previews (`assets/maps/<map>.png`, rendered from the real arenas with the arena kit
/// by `--example arena_views -- <map> <dir> thumbnail`), in [`PRACTICE_MAPS`]' order.
const PREVIEWS: [&[u8]; 3] = [
    include_bytes!("../assets/maps/proving-cell.png"),
    include_bytes!("../assets/maps/assembly-works.png"),
    include_bytes!("../assets/maps/drydock.png"),
];

/// The previews as textures of the frame's world.
pub struct Previews {
    textures: [TextureId; 3],
}

impl Previews {
    /// Decodes the previews into `world`.
    ///
    /// # Errors
    ///
    /// If a preview does not decode.
    pub fn new(world: &mut World) -> Result<Self, String> {
        let [a, b, c] = PREVIEWS;
        Ok(Self {
            textures: [
                world.add_texture(&decode(a)?),
                world.add_texture(&decode(b)?),
                world.add_texture(&decode(c)?),
            ],
        })
    }
}

/// A PNG (RGB or RGBA, 8 bits) as an sRGB texture.
fn decode(bytes: &[u8]) -> Result<TextureData, String> {
    let failed = |error: png::DecodingError| format!("a map preview: {error}");
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .map_err(failed)?;
    let mut buffer = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or("a map preview is too large")?
    ];
    let info = reader.next_frame(&mut buffer).map_err(failed)?;
    let pixels = &buffer[..info.buffer_size()];
    let rgba = match (info.color_type, info.bit_depth) {
        (png::ColorType::Rgba, png::BitDepth::Eight) => pixels.to_vec(),
        (png::ColorType::Rgb, png::BitDepth::Eight) => pixels
            .chunks(3)
            .flat_map(|rgb| [rgb[0], rgb[1], rgb[2], 255])
            .collect(),
        other => return Err(format!("a map preview is {other:?}, not 8-bit RGB")),
    };
    Ok(TextureData {
        width: info.width,
        height: info.height,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        levels: vec![rgba],
        sampler: Sampler {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
        },
    })
}

/// Which page of the menu shows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Screen {
    Main,
    Practice,
    Settings,
}

/// A clickable item.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Target {
    PlayOnline,
    Practice,
    Workshop,
    /// Shown, not available yet.
    PrivateMatch,
    Settings,
    /// Back to the main page.
    Back,
    /// A practice map, by its index in [`PRACTICE_MAPS`].
    Map(usize),
    Fewer,
    More,
    Start,
    Name,
    Slower,
    Faster,
    /// Shows or hides the advanced developer connection.
    Advanced,
    Url,
    Certificate,
    Connect,
}

impl Target {
    /// Its name in the readout ([`Menu::readout`]).
    fn name(self) -> String {
        match self {
            Self::Map(index) => format!("Map:{}", PRACTICE_MAPS[index]),
            other => format!("{other:?}"),
        }
    }
}

/// A text the person types into (natively in place; in the browser through the page's prompt,
/// which a phone's keyboard can type into too).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Field {
    Name,
    Url,
    Certificate,
}

/// What the person chose.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Choice {
    /// An in-process deathmatch against bots on `map` with `fighters` fighters, the person one of
    /// them: Play Online's bot match (`online`) or a practice match.
    Play {
        map: MapId,
        fighters: u8,
        online: bool,
    },
    /// The robot builder, no match.
    Workshop,
    /// Join the hosted match at `url`, trusting `certificate` (a development host's: natively
    /// its PEM file, in the browser its SHA-256 in hex; empty for a host with a real one).
    Connect { url: String, certificate: String },
}

/// A rectangle in layout units (720 high).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    fn contains(self, at: Vec2) -> bool {
        at.x >= self.x
            && at.x < self.x + self.width
            && at.y >= self.y
            && at.y < self.y + self.height
    }

    /// Whether `at` is inside (both in the frame's pixels: [`leave_button`]).
    #[must_use]
    pub fn contains_pixels(self, at: Vec2) -> bool {
        self.contains(at)
    }

    #[must_use]
    pub fn centre(self) -> Vec2 {
        Vec2::new(self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    fn scaled(self, scale: f32) -> Self {
        Self::new(
            self.x * scale,
            self.y * scale,
            self.width * scale,
            self.height * scale,
        )
    }
}

/// The Leave button's size in layout units.
const LEAVE: Vec2 = Vec2::new(200.0, 44.0);

/// A match's Leave button (back to the main menu, shown while the pointer is free), top
/// centre of a `width` × `height` frame, in its pixels.
#[must_use]
pub fn leave_button(width: u32, height: u32) -> Rect {
    #[allow(clippy::cast_precision_loss)]
    let (width, height) = (width as f32, height as f32);
    let scale = height.max(1.0) / 720.0;
    let centre = width / scale / 2.0;
    Rect::new(centre - LEAVE.x / 2.0, 18.0, LEAVE.x, LEAVE.y).scaled(scale)
}

/// What a match draws of the menu on a `width` × `height` frame: the Leave button (`leave`:
/// where, in the frame's pixels, and whether the pointer is on it) and under it the match's
/// `label` (what it is: Play Online's bot match says it is offline).
#[must_use]
pub fn match_items(
    width: u32,
    height: u32,
    leave: Option<(Rect, bool)>,
    label: Option<&str>,
) -> Vec<Item> {
    #[allow(clippy::cast_precision_loss)]
    let (frame_width, frame_height) = (width as f32, height as f32);
    let scale = frame_height.max(1.0) / 720.0;
    let mut painter = Painter {
        quads: Vec::new(),
        scale,
    };
    let centre = frame_width / scale / 2.0;
    if let Some((rect, lit)) = leave {
        button(
            &mut painter,
            rect.scaled(1.0 / scale),
            "Leave match",
            lit,
            false,
        );
    }
    if let Some(label) = label {
        let cell = painter.snap_cell(2.0);
        let width = text_width(label, cell);
        let y = if leave.is_some() { 72.0 } else { 22.0 };
        painter.round_rect(
            centre - width / 2.0 - 12.0,
            y,
            width + 24.0,
            28.0,
            6.0,
            srgb(GRAPHITE, 0.8),
        );
        painter.text(centre - width / 2.0, y + 7.0, 2.0, label, srgb(PALE, 1.0));
    }
    painter.quads
}

/// Below this many layout units across, the menu stacks for a narrow (portrait) screen.
const NARROW: f32 = 760.0;
/// Below this many, Practice lists its maps as rows instead of cards side by side.
const CARDS: f32 = 1_040.0;

const ENTRY_HEIGHT: f32 = 62.0;
const ENTRY_GAP: f32 = 12.0;
const MAIN_TOP: f32 = 196.0;
const CARD_GAP: f32 = 20.0;
const ROW_HEIGHT: f32 = 86.0;
const FIELD_HEIGHT: f32 = 44.0;

/// The main page's entries, top to bottom.
const ENTRIES: [Target; 5] = [
    Target::PlayOnline,
    Target::Practice,
    Target::Workshop,
    Target::PrivateMatch,
    Target::Settings,
];

/// The main menu: its page, what Practice has chosen, the preferences and the pointer.
pub struct Menu {
    pub screen: Screen,
    /// The practice map, by its index in [`PRACTICE_MAPS`], and the fighters chosen for each.
    pub selected: usize,
    fighters: [u8; 3],
    pub prefs: Prefs,
    store: Rc<dyn PrefStore>,
    /// The advanced developer connection is shown, and what it holds.
    advanced: bool,
    url: String,
    certificate: String,
    /// The field typed into (natively) and what it holds so far.
    editing: Option<Field>,
    draft: String,
    /// The last thing refused or gone wrong (a connection that ended, a field left empty).
    pub notice: Option<String>,
    /// The robot the next match is played with, for the main page.
    robot: String,
    /// The pointer (the window's pixels), the window's size (the pointer's space, once known)
    /// and the frame's size (the drawn surface's pixels).
    cursor: Vec2,
    window: Option<(u32, u32)>,
    size: (u32, u32),
    /// The item the pointer went down on: it acts when the pointer comes up on it too (a tap).
    pressed: Option<Target>,
}

impl Menu {
    /// The main page, the preferences read from `store`.
    #[must_use]
    pub fn new(store: Rc<dyn PrefStore>) -> Self {
        let prefs = Prefs::load(store.as_ref());
        let infos = practice_maps();
        Self {
            screen: Screen::Main,
            selected: PRACTICE_MAPS
                .iter()
                .position(|map| *map == PLAY_ONLINE_MAP)
                .unwrap_or(0),
            fighters: infos.map(|info| info.default_fighters()),
            prefs,
            store,
            advanced: false,
            url: String::new(),
            certificate: String::new(),
            editing: None,
            draft: String::new(),
            notice: None,
            robot: String::new(),
            cursor: Vec2::ZERO,
            window: None,
            size: (1280, 720),
            pressed: None,
        }
    }

    /// [`Self::new`] showing that the next match plays `build`.
    #[must_use]
    pub fn with_robot(store: Rc<dyn PrefStore>, build: &shooter_sim::Assembly) -> Self {
        let mut menu = Self::new(store);
        menu.set_robot(robot_line(build));
        menu
    }

    /// The arena the live view behind the menu shows: Practice's chosen map, else Play Online's.
    #[must_use]
    pub fn backdrop(&self) -> MapId {
        match self.screen {
            Screen::Practice => PRACTICE_MAPS[self.selected],
            Screen::Main | Screen::Settings => PLAY_ONLINE_MAP,
        }
    }

    /// The robot line on the main page: what the next match is played with.
    pub fn set_robot(&mut self, line: String) {
        self.robot = line;
    }

    /// The fighters Practice has chosen for its map.
    #[must_use]
    pub fn fighters(&self) -> u8 {
        self.fighters[self.selected]
    }

    /// The field being typed into, if any.
    #[must_use]
    pub const fn editing(&self) -> Option<Field> {
        self.editing
    }

    /// Back on the main page, keeping what was chosen (after a match or the Workshop).
    pub fn reopen(&mut self) {
        self.screen = Screen::Main;
        self.editing = None;
        self.pressed = None;
    }

    /// The window is `width` × `height` pixels: the space the pointer arrives in.
    pub const fn resize(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.window = Some((width, height));
        }
    }

    /// The pointer moved to `x`, `y` (the window's pixels).
    pub const fn cursor_moved(&mut self, x: f32, y: f32) {
        self.cursor = Vec2::new(x, y);
    }

    fn scale(&self) -> f32 {
        #[allow(clippy::cast_precision_loss)]
        let height = self.size.1.max(1) as f32;
        height / 720.0
    }

    /// The frame's width in layout units.
    fn width(&self) -> f32 {
        #[allow(clippy::cast_precision_loss)]
        let width = self.size.0.max(1) as f32;
        width / self.scale()
    }

    /// The pointer in layout units.
    #[allow(clippy::cast_precision_loss)]
    fn pointer(&self) -> Vec2 {
        let pixels = match self.window {
            Some((width, height)) => {
                self.cursor
                    * Vec2::new(
                        self.size.0 as f32 / width.max(1) as f32,
                        self.size.1 as f32 / height.max(1) as f32,
                    )
            }
            None => self.cursor,
        };
        pixels / self.scale()
    }

    /// Where `at` (layout units) is in the window's pixels (the pointer's space).
    #[allow(clippy::cast_precision_loss)]
    fn window_pixels(&self, at: Vec2) -> Vec2 {
        let pixels = at * self.scale();
        match self.window {
            Some((width, height)) => {
                pixels
                    * Vec2::new(
                        width as f32 / self.size.0.max(1) as f32,
                        height as f32 / self.size.1.max(1) as f32,
                    )
            }
            None => pixels,
        }
    }

    /// The left margin.
    fn left(&self) -> f32 {
        if self.width() < NARROW { 20.0 } else { 64.0 }
    }

    /// Every clickable item on the page showing.
    fn targets(&self) -> Vec<(Rect, Target)> {
        let left = self.left();
        let width = self.width();
        match self.screen {
            Screen::Main => {
                let column = (width - 2.0 * left).min(420.0);
                ENTRIES
                    .iter()
                    .enumerate()
                    .map(|(index, target)| {
                        #[allow(clippy::cast_precision_loss)]
                        let y = MAIN_TOP + index as f32 * (ENTRY_HEIGHT + ENTRY_GAP);
                        (Rect::new(left, y, column, ENTRY_HEIGHT), *target)
                    })
                    .collect()
            }
            Screen::Practice => {
                let mut targets = vec![(self.back(), Target::Back)];
                let cards = self.cards();
                let bottom = cards
                    .iter()
                    .map(|card| card.y + card.height)
                    .fold(0.0, f32::max);
                targets.extend(
                    cards
                        .into_iter()
                        .enumerate()
                        .map(|(index, rect)| (rect, Target::Map(index))),
                );
                let y = bottom + 54.0;
                targets.push((Rect::new(left, y, 48.0, 48.0), Target::Fewer));
                targets.push((Rect::new(left + 150.0, y, 48.0, 48.0), Target::More));
                let start = if width >= CARDS {
                    Rect::new(left + 240.0, y - 4.0, 360.0, 56.0)
                } else {
                    Rect::new(left, y + 76.0, (width - 2.0 * left).min(360.0), 56.0)
                };
                targets.push((start, Target::Start));
                targets
            }
            Screen::Settings => {
                let mut targets = vec![(self.back(), Target::Back)];
                let rows = self.settings_rows();
                targets.push((rows.control(0, 340.0), Target::Name));
                targets.push((rows.control(1, 48.0), Target::Slower));
                let faster = rows.control(1, 48.0);
                targets.push((
                    Rect::new(faster.x + 170.0, faster.y, 48.0, 48.0),
                    Target::Faster,
                ));
                targets.push((rows.control(2, 340.0), Target::Advanced));
                if self.advanced {
                    let wide = rows.room.min(520.0);
                    targets.push((rows.control(3, wide), Target::Url));
                    targets.push((rows.control(4, wide), Target::Certificate));
                    targets.push((rows.control(5, 220.0), Target::Connect));
                }
                targets
            }
        }
    }

    /// The Back button of Practice and Settings.
    fn back(&self) -> Rect {
        Rect::new(self.left(), 28.0, 112.0, 40.0)
    }

    /// Practice's map cards (side by side) or rows (stacked), in [`PRACTICE_MAPS`]' order.
    fn cards(&self) -> Vec<Rect> {
        let left = self.left();
        let width = self.width();
        if width >= CARDS {
            let card = ((width - 2.0 * left - 2.0 * CARD_GAP) / 3.0).min(340.0);
            let height = (card - 16.0) * 9.0 / 16.0 + 112.0;
            (0..3)
                .map(|index| {
                    #[allow(clippy::cast_precision_loss)]
                    let x = left + index as f32 * (card + CARD_GAP);
                    Rect::new(x, 96.0, card, height)
                })
                .collect()
        } else {
            let row = (width - 2.0 * left).min(560.0);
            (0..3)
                .map(|index| {
                    #[allow(clippy::cast_precision_loss)]
                    let y = 92.0 + index as f32 * (ROW_HEIGHT + 12.0);
                    Rect::new(left, y, row, ROW_HEIGHT)
                })
                .collect()
        }
    }

    fn settings_rows(&self) -> SettingsRows {
        let left = self.left();
        if self.width() < NARROW {
            SettingsRows {
                label_x: left,
                control_x: left,
                top: 120.0,
                step: 86.0,
                label_above: true,
                room: self.width() - 2.0 * left,
            }
        } else {
            SettingsRows {
                label_x: left,
                control_x: left + 260.0,
                top: 112.0,
                step: 70.0,
                label_above: false,
                room: self.width() - left - 260.0 - left,
            }
        }
    }

    /// The item under the pointer.
    #[must_use]
    pub fn target(&self) -> Option<Target> {
        let at = self.pointer();
        self.targets()
            .into_iter()
            .find(|(rect, _)| rect.contains(at))
            .map(|(_, target)| target)
    }

    /// The primary button (a finger on a touch screen) went down or came up: an item acts when
    /// the button comes up on the one it went down on.
    pub fn mouse(&mut self, pressed: bool) -> Option<Choice> {
        let target = self.target();
        if pressed {
            self.pressed = target;
            // A click elsewhere ends typing, keeping what was typed.
            if self.editing.is_some()
                && !matches!(
                    target,
                    Some(Target::Name | Target::Url | Target::Certificate)
                )
            {
                self.commit();
            }
            return None;
        }
        let down = self.pressed.take();
        match target {
            Some(target) if down == Some(target) => self.act(target),
            _ => None,
        }
    }

    /// A key went down, typing `text` (natively): typing into a field, else Escape goes back a
    /// page, and Enter starts what the page offers.
    pub fn key(&mut self, code: KeyCode, text: Option<&str>) -> Option<Choice> {
        if self.editing.is_some() {
            match code {
                KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Tab => self.commit(),
                KeyCode::Escape => self.editing = None,
                KeyCode::Backspace => {
                    self.draft.pop();
                }
                _ => {
                    let typed = text.unwrap_or_default();
                    if self.draft.chars().count() < 512 {
                        self.draft
                            .extend(typed.chars().filter(|character| !character.is_control()));
                    }
                }
            }
            return None;
        }
        match code {
            KeyCode::Escape | KeyCode::Backspace if self.screen != Screen::Main => {
                self.act(Target::Back)
            }
            KeyCode::Enter | KeyCode::NumpadEnter => match self.screen {
                Screen::Main => self.act(Target::PlayOnline),
                Screen::Practice => self.act(Target::Start),
                Screen::Settings => None,
            },
            KeyCode::ArrowLeft if self.screen == Screen::Practice => {
                self.act(Target::Map(self.selected.saturating_sub(1)))
            }
            KeyCode::ArrowRight if self.screen == Screen::Practice => self.act(Target::Map(
                (self.selected + 1).min(PRACTICE_MAPS.len() - 1),
            )),
            KeyCode::ArrowDown | KeyCode::Minus if self.screen == Screen::Practice => {
                self.act(Target::Fewer)
            }
            KeyCode::ArrowUp | KeyCode::Equal if self.screen == Screen::Practice => {
                self.act(Target::More)
            }
            _ => None,
        }
    }

    /// Does what `target` does.
    pub fn act(&mut self, target: Target) -> Option<Choice> {
        let infos = practice_maps();
        match target {
            Target::PlayOnline => {
                return Some(Choice::Play {
                    map: PLAY_ONLINE_MAP,
                    fighters: PLAY_ONLINE_FIGHTERS,
                    online: true,
                });
            }
            Target::Practice => self.open(Screen::Practice),
            Target::Workshop => return Some(Choice::Workshop),
            // Not available yet: nothing to do.
            Target::PrivateMatch => {}
            Target::Settings => self.open(Screen::Settings),
            Target::Back => self.open(Screen::Main),
            Target::Map(index) => self.selected = index.min(PRACTICE_MAPS.len() - 1),
            Target::Fewer | Target::More => {
                let info = infos[self.selected];
                let fighters = &mut self.fighters[self.selected];
                *fighters = if target == Target::More {
                    fighters.saturating_add(1)
                } else {
                    fighters.saturating_sub(1)
                }
                .clamp(info.fewest, info.most);
            }
            Target::Start => {
                return Some(Choice::Play {
                    map: PRACTICE_MAPS[self.selected],
                    fighters: self.fighters(),
                    online: false,
                });
            }
            Target::Name => self.edit(Field::Name),
            Target::Url => self.edit(Field::Url),
            Target::Certificate => self.edit(Field::Certificate),
            Target::Slower | Target::Faster => {
                self.prefs
                    .step_sensitivity(if target == Target::Faster { 1 } else { -1 });
                self.save();
            }
            Target::Advanced => self.advanced = !self.advanced,
            Target::Connect => return self.connect(),
        }
        None
    }

    fn open(&mut self, screen: Screen) {
        self.commit();
        self.screen = screen;
        self.notice = None;
    }

    fn save(&mut self) {
        if let Err(error) = self.prefs.save(self.store.as_ref()) {
            crate::log(&format!("could not keep the settings: {error}"));
        }
    }

    /// What `field` holds.
    fn value(&self, field: Field) -> String {
        match field {
            Field::Name => self.prefs.name.clone().unwrap_or_default(),
            Field::Url => self.url.clone(),
            Field::Certificate => self.certificate.clone(),
        }
    }

    /// Starts typing into `field`: natively in place, in the browser in the page's prompt.
    fn edit(&mut self, field: Field) {
        #[cfg(target_arch = "wasm32")]
        {
            let question = match field {
                Field::Name => "Your name (online)",
                Field::Url => "Hosted match URL (https://host:4433/shooter/matches/dm)",
                Field::Certificate => "Certificate SHA-256 of a development host (empty: none)",
            };
            if let Some(typed) = crate::web::prompt(question, &self.value(field)) {
                self.set(field, &typed);
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.commit();
            self.draft = self.value(field);
            self.editing = Some(field);
        }
    }

    /// Keeps what was typed into the field being edited.
    fn commit(&mut self) {
        if let Some(field) = self.editing.take() {
            let typed = std::mem::take(&mut self.draft);
            self.set(field, &typed);
        }
    }

    fn set(&mut self, field: Field, typed: &str) {
        match field {
            Field::Name => {
                self.prefs.set_name(typed);
                self.save();
            }
            Field::Url => typed.trim().clone_into(&mut self.url),
            Field::Certificate => typed.trim().clone_into(&mut self.certificate),
        }
    }

    /// The advanced connection's Connect: the URL (and, in the browser, a certificate hash that
    /// is one) checked before anything connects.
    fn connect(&mut self) -> Option<Choice> {
        self.commit();
        if self.url.is_empty() {
            self.notice = Some("Enter the hosted match's URL first".into());
            return None;
        }
        if cfg!(target_arch = "wasm32")
            && !self.certificate.is_empty()
            && crate::parse_hash(&self.certificate).is_none()
        {
            self.notice = Some("The certificate is not a SHA-256 in hex".into());
            return None;
        }
        Some(Choice::Connect {
            url: self.url.clone(),
            certificate: self.certificate.clone(),
        })
    }

    /// What the menu shows, for scripted checks of the page (`window.shooterMenu`): the page,
    /// where every item is (the window's pixels), Practice's choice and the preferences.
    #[must_use]
    pub fn readout(&self) -> String {
        let info = practice_maps()[self.selected];
        let targets: Vec<_> = self
            .targets()
            .into_iter()
            .map(|(rect, target)| {
                let at = self.window_pixels(rect.centre());
                serde_json::json!({
                    "name": target.name(),
                    "at": [at.x.round(), at.y.round()],
                })
            })
            .collect();
        serde_json::json!({
            "screen": format!("{:?}", self.screen),
            "map": info.map.as_str(),
            "fighters": self.fighters(),
            "range": [info.fewest, info.most],
            "name": self.prefs.name,
            "sensitivity": self.prefs.sensitivity,
            "advanced": self.advanced,
            "notice": self.notice,
            "targets": targets,
        })
        .to_string()
    }

    /// The menu's items for a `width` × `height` frame, over the live view; `previews` the maps'
    /// (once the GPU is up), `note` what is still loading.
    pub fn frame(
        &mut self,
        width: u32,
        height: u32,
        previews: Option<&Previews>,
        note: Option<&str>,
    ) -> Vec<Item> {
        self.size = (width.max(1), height.max(1));
        let mut painter = Painter {
            quads: Vec::with_capacity(2_048),
            scale: self.scale(),
        };
        let hovered = self.target();
        match self.screen {
            Screen::Main => self.draw_main(&mut painter, hovered),
            Screen::Practice => self.draw_practice(&mut painter, hovered, previews),
            Screen::Settings => self.draw_settings(&mut painter, hovered),
        }
        let left = self.left();
        let bottom = 720.0 - 40.0;
        if let Some(notice) = &self.notice {
            painter.label(left, bottom - 26.0, 2.0, notice, srgb(RED, 1.0));
        }
        if let Some(note) = note {
            painter.label_right(self.width() - 24.0, bottom, 1.6, note, srgb(MUTED, 1.0));
        }
        painter.quads
    }

    /// The shade behind the page's column, fading out over the live view.
    fn shade(&self, painter: &mut Painter, column: f32) {
        let width = (self.left() + column + 48.0).min(self.width());
        painter.rect(0.0, 0.0, width, 720.0, srgb(SHADOW, 0.74));
        for step in 0..24 {
            #[allow(clippy::cast_precision_loss)]
            let step = step as f32;
            painter.rect(
                width + step * 4.0,
                0.0,
                4.0,
                720.0,
                srgb(SHADOW, 0.74 * (1.0 - (step + 1.0) / 25.0)),
            );
        }
    }

    fn draw_main(&self, painter: &mut Painter, hovered: Option<Target>) {
        let left = self.left();
        let targets = self.targets();
        let column = targets.first().map_or(420.0, |(rect, _)| rect.width);
        self.shade(painter, column);
        painter.text(left, 64.0, 6.0, "Shooter", srgb(PALE, 1.0));
        painter.rect(left, 118.0, 56.0, 4.0, srgb(ORANGE, 1.0));
        painter.text(left, 132.0, 2.0, "Robot arena deathmatch", srgb(MUTED, 1.0));
        let online = format!(
            "Bot match · {} · {PLAY_ONLINE_FIGHTERS} fighters · offline for now",
            map_name(PLAY_ONLINE_MAP)
        );
        for (rect, target) in targets {
            let (title, detail, look) = match target {
                Target::PlayOnline => ("Play Online", online.as_str(), Look::Strong),
                Target::Practice => (
                    "Practice",
                    "Proving Cell, Assembly Works or Drydock vs bots",
                    Look::Normal,
                ),
                Target::Workshop => ("Robot Workshop", "Build and tune your robot", Look::Normal),
                Target::PrivateMatch => ("Private Match", "Not available yet", Look::Disabled),
                _ => (
                    "Settings",
                    "Name, mouse, developer connection",
                    Look::Normal,
                ),
            };
            entry(painter, rect, title, detail, hovered == Some(target), look);
        }
        if !self.robot.is_empty() {
            painter.label(left, 720.0 - 66.0, 2.0, &self.robot, srgb(PALE, 0.9));
        }
    }

    /// A page's Back button and title.
    fn header(&self, painter: &mut Painter, hovered: Option<Target>, title: &str) {
        let back = self.back();
        button(painter, back, "Back", hovered == Some(Target::Back), false);
        painter.text(
            back.x + back.width + 24.0,
            32.0,
            4.0,
            title,
            srgb(PALE, 1.0),
        );
    }

    fn draw_practice(
        &self,
        painter: &mut Painter,
        hovered: Option<Target>,
        previews: Option<&Previews>,
    ) {
        let left = self.left();
        painter.rect(0.0, 0.0, self.width(), 720.0, srgb(SHADOW, 0.45));
        self.header(painter, hovered, "Practice");
        let targets = self.targets();
        let side_by_side = self.width() >= CARDS;
        for (rect, target) in &targets {
            let Target::Map(index) = *target else {
                continue;
            };
            let info = practice_maps()[index];
            let chosen = index == self.selected;
            let lit = hovered == Some(*target);
            painter.round_rect(
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                10.0,
                srgb(GRAPHITE, if lit { 0.96 } else { 0.88 }),
            );
            if chosen {
                painter.frame(
                    rect.x,
                    rect.y,
                    rect.width,
                    rect.height,
                    2.0,
                    srgb(ORANGE, 1.0),
                );
            }
            let picture = if side_by_side {
                Rect::new(
                    rect.x + 8.0,
                    rect.y + 8.0,
                    rect.width - 16.0,
                    (rect.width - 16.0) * 9.0 / 16.0,
                )
            } else {
                Rect::new(rect.x + 8.0, rect.y + 8.0, 124.0, 70.0)
            };
            if let Some(previews) = previews {
                image(painter, picture, previews.textures[index]);
            }
            let (text_x, text_y) = if side_by_side {
                (rect.x + 16.0, picture.y + picture.height + 12.0)
            } else {
                (picture.x + picture.width + 16.0, rect.y + 10.0)
            };
            painter.text(text_x, text_y, 3.0, map_name(info.map), srgb(PALE, 1.0));
            painter.text(
                text_x,
                text_y + 32.0,
                1.8,
                &format!("{} · {} m arena", info.class(), info.metres),
                srgb(MUTED, 1.0),
            );
            painter.text(
                text_x,
                text_y + 52.0,
                1.8,
                &format!("{} to {} fighters", info.fewest, info.most),
                srgb(if chosen { ORANGE } else { MUTED }, 1.0),
            );
        }
        // How many fight, the person included, and Start.
        let info = practice_maps()[self.selected];
        let fighters = self.fighters();
        for (rect, target) in &targets {
            match target {
                Target::Fewer => {
                    painter.text(left, rect.y - 26.0, 2.0, "Fighters", srgb(MUTED, 1.0));
                    button(painter, *rect, "-", hovered == Some(*target), false);
                    let value = fighters.to_string();
                    let cell = painter.snap_cell(4.0);
                    let centre = rect.x + 48.0 + 51.0;
                    painter.text(
                        centre - text_width(&value, cell) / 2.0,
                        rect.y + 6.0,
                        4.0,
                        &value,
                        srgb(PALE, 1.0),
                    );
                    let bots = fighters.saturating_sub(1);
                    painter.text(
                        left,
                        rect.y + 58.0,
                        1.8,
                        &format!(
                            "You + {bots} {} · {} to {}",
                            if bots == 1 { "bot" } else { "bots" },
                            info.fewest,
                            info.most
                        ),
                        srgb(MUTED, 1.0),
                    );
                }
                Target::More => button(painter, *rect, "+", hovered == Some(*target), false),
                Target::Start => button(
                    painter,
                    *rect,
                    &format!("Start · {}", map_name(info.map)),
                    hovered == Some(*target),
                    true,
                ),
                _ => {}
            }
        }
    }

    fn draw_settings(&self, painter: &mut Painter, hovered: Option<Target>) {
        let rows = self.settings_rows();
        let column = if rows.label_above {
            self.width()
        } else {
            860.0
        };
        self.shade(painter, column);
        self.header(painter, hovered, "Settings");
        let name = self.prefs.name.clone();
        let sensitivity = format!("{} %", self.prefs.sensitivity);
        for (rect, target) in self.targets() {
            let lit = hovered == Some(target);
            match target {
                Target::Name => {
                    rows.label(painter, 0, "Name (online)");
                    self.field(
                        painter,
                        rect,
                        Field::Name,
                        name.as_deref(),
                        "Not set: hosts show Player N",
                        lit,
                    );
                }
                Target::Slower => {
                    rows.label(painter, 1, "Look sensitivity");
                    button(painter, rect, "-", lit, false);
                    let cell = painter.snap_cell(3.0);
                    let centre = rect.x + 48.0 + 61.0;
                    painter.text(
                        centre - text_width(&sensitivity, cell) / 2.0,
                        rect.y + 12.0,
                        3.0,
                        &sensitivity,
                        srgb(PALE, 1.0),
                    );
                }
                Target::Faster => button(painter, rect, "+", lit, false),
                Target::Advanced => {
                    rows.label(painter, 2, "Advanced");
                    let label = if self.advanced {
                        "Hide developer connection"
                    } else {
                        "Developer connection"
                    };
                    button(painter, rect, label, lit, false);
                }
                Target::Url => {
                    rows.label(painter, 3, "Match URL");
                    let url = (!self.url.is_empty()).then_some(self.url.as_str());
                    self.field(
                        painter,
                        rect,
                        Field::Url,
                        url,
                        "https://host:4433/shooter/matches/dm",
                        lit,
                    );
                }
                Target::Certificate => {
                    rows.label(painter, 4, "Certificate");
                    let placeholder = if cfg!(target_arch = "wasm32") {
                        "SHA-256 of a development host's (optional)"
                    } else {
                        "PEM file of a development host's (optional)"
                    };
                    let certificate =
                        (!self.certificate.is_empty()).then_some(self.certificate.as_str());
                    self.field(
                        painter,
                        rect,
                        Field::Certificate,
                        certificate,
                        placeholder,
                        lit,
                    );
                }
                Target::Connect => {
                    button(painter, rect, "Connect", lit, !self.url.is_empty());
                    painter.text(
                        rect.x + rect.width + 16.0,
                        rect.y + 16.0,
                        1.8,
                        "Joins a match on a host you run (scripts/dev.sh)",
                        srgb(MUTED, 1.0),
                    );
                }
                _ => {}
            }
        }
    }

    /// A text field: what it holds (or, muted, `placeholder`), the draft with a caret while it
    /// is typed into.
    fn field(
        &self,
        painter: &mut Painter,
        rect: Rect,
        field: Field,
        value: Option<&str>,
        placeholder: &str,
        lit: bool,
    ) {
        let editing = self.editing == Some(field);
        painter.round_rect(
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            6.0,
            srgb(GRAPHITE, 0.92),
        );
        let edge = if editing {
            srgb(ORANGE, 1.0)
        } else if lit {
            srgb(PALE, 0.8)
        } else {
            srgb(STEEL, 1.0)
        };
        painter.frame(rect.x, rect.y, rect.width, rect.height, 1.5, edge);
        let cell = 2.0;
        let room = rect.width - 24.0;
        let (text, colour) = if editing {
            (format!("{}_", self.draft), srgb(PALE, 1.0))
        } else {
            match value {
                Some(value) => (value.to_owned(), srgb(PALE, 1.0)),
                None => (placeholder.to_owned(), srgb(MUTED, 1.0)),
            }
        };
        // The end of a long text, as much as fits.
        let snapped = painter.snap_cell(cell);
        let mut shown: &str = &text;
        while text_width(shown, snapped) > room && !shown.is_empty() {
            let mut characters = shown.chars();
            characters.next();
            shown = characters.as_str();
        }
        painter.text(rect.x + 12.0, rect.y + 15.0, cell, shown, colour);
    }
}

/// Where Settings' rows are.
struct SettingsRows {
    label_x: f32,
    control_x: f32,
    top: f32,
    step: f32,
    /// Narrow: each row's label above its control.
    label_above: bool,
    /// The width left for a control.
    room: f32,
}

impl SettingsRows {
    #[allow(clippy::cast_precision_loss)]
    fn control(&self, row: usize, width: f32) -> Rect {
        let y = self.top + row as f32 * self.step + if self.label_above { 24.0 } else { 0.0 };
        let height = if width <= 48.0 { 48.0 } else { FIELD_HEIGHT };
        Rect::new(self.control_x, y, width.min(self.room), height)
    }

    #[allow(clippy::cast_precision_loss)]
    fn label(&self, painter: &mut Painter, row: usize, text: &str) {
        let y = self.top + row as f32 * self.step + if self.label_above { 0.0 } else { 14.0 };
        painter.text(self.label_x, y, 2.0, text, srgb(MUTED, 1.0));
    }
}

/// How a main page entry looks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Look {
    /// The primary action: filled orange.
    Strong,
    Normal,
    /// Shown, not available.
    Disabled,
}

/// A main page entry: its title and, smaller, what it does.
fn entry(painter: &mut Painter, rect: Rect, title: &str, detail: &str, lit: bool, look: Look) {
    let (fill, title_colour, detail_colour) = match look {
        Look::Strong => (
            srgb(ORANGE, if lit { 1.0 } else { 0.9 }),
            srgb(GRAPHITE, 1.0),
            srgb(GRAPHITE, 0.8),
        ),
        Look::Normal => (
            srgb(if lit { STEEL } else { GRAPHITE }, 0.92),
            srgb(PALE, 1.0),
            srgb(MUTED, 1.0),
        ),
        Look::Disabled => (srgb(GRAPHITE, 0.6), srgb(MUTED, 0.8), srgb(MUTED, 0.8)),
    };
    painter.round_rect(rect.x, rect.y, rect.width, rect.height, 8.0, fill);
    if look == Look::Normal && lit {
        painter.round_rect(
            rect.x,
            rect.y + 10.0,
            4.0,
            rect.height - 20.0,
            2.0,
            srgb(ORANGE, 1.0),
        );
    }
    painter.text(rect.x + 20.0, rect.y + 10.0, 3.0, title, title_colour);
    painter.text(rect.x + 20.0, rect.y + 38.0, 1.6, detail, detail_colour);
    if look == Look::Disabled {
        painter.text_right(
            rect.x + rect.width - 16.0,
            rect.y + 14.0,
            1.6,
            "Later",
            srgb(ORANGE, 0.9),
        );
    }
}

/// A button: `strong` filled orange.
fn button(painter: &mut Painter, rect: Rect, label: &str, lit: bool, strong: bool) {
    let fill = match (strong, lit) {
        (true, true) => srgb(ORANGE, 1.0),
        (true, false) => srgb(ORANGE, 0.88),
        (false, true) => srgb(STEEL, 1.0),
        (false, false) => srgb(GRAPHITE, 0.9),
    };
    painter.round_rect(rect.x, rect.y, rect.width, rect.height, 6.0, fill);
    if !strong {
        painter.frame(
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            1.0,
            srgb(STEEL, 1.0),
        );
    }
    let cell = if rect.height >= 48.0 { 3.0 } else { 2.4 };
    let width = text_width(label, painter.snap_cell(cell));
    let colour = if strong { GRAPHITE } else { PALE };
    painter.text(
        rect.x + (rect.width - width) / 2.0,
        rect.y + (rect.height - 7.0 * painter.snap_cell(cell)) / 2.0,
        cell,
        label,
        srgb(colour, 1.0),
    );
}

/// `texture` stretched over `rect` (layout units), snapped to whole pixels.
fn image(painter: &mut Painter, rect: Rect, texture: TextureId) {
    let snap = |value: f32| (value * painter.scale).round();
    let (left, top) = (snap(rect.x), snap(rect.y));
    painter.quads.push(Item::Image {
        position: Vec2::new(left, top),
        size: Vec2::new(
            snap(rect.x + rect.width) - left,
            snap(rect.y + rect.height) - top,
        ),
        radius: 6.0 * painter.scale,
        texture,
        color: Vec4::ONE,
    });
}

/// The main page's robot line for `build`: the named loadout it is, else a custom build.
#[must_use]
pub fn robot_line(build: &shooter_sim::Assembly) -> String {
    let parts = build.parts().len();
    shooter_sim::robot::LOADOUTS
        .iter()
        .find(|(_, loadout)| loadout == build)
        .map_or_else(
            || format!("Your robot: custom build, {parts} parts"),
            |(name, _)| format!("Your robot: {name} loadout, {parts} parts"),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prefs::MemoryPrefs;

    fn menu() -> Menu {
        let mut menu = Menu::new(Rc::new(MemoryPrefs::default()));
        menu.frame(1280, 720, None, None);
        menu
    }

    /// Clicks `target` (down and up on its middle).
    fn click(menu: &mut Menu, target: Target) -> Option<Choice> {
        let (rect, _) = menu
            .targets()
            .into_iter()
            .find(|(_, at)| *at == target)
            .unwrap_or_else(|| panic!("{target:?} is not on {:?}", menu.screen));
        let at = menu.window_pixels(rect.centre());
        menu.cursor_moved(at.x, at.y);
        assert_eq!(menu.target(), Some(target));
        assert_eq!(menu.mouse(true), None);
        let choice = menu.mouse(false);
        menu.frame(1280, 720, None, None);
        choice
    }

    #[test]
    fn play_online_is_a_twelve_fighter_bot_match_on_the_assembly_works() {
        let mut menu = menu();
        assert_eq!(
            click(&mut menu, Target::PlayOnline),
            Some(Choice::Play {
                map: MapId::AssemblyWorks,
                fighters: 12,
                online: true,
            })
        );
        // The same from the keyboard.
        assert_eq!(
            menu.key(KeyCode::Enter, None),
            Some(Choice::Play {
                map: MapId::AssemblyWorks,
                fighters: 12,
                online: true,
            })
        );
    }

    #[test]
    fn practice_offers_the_three_arenas_within_their_capacity() {
        let infos = practice_maps();
        assert_eq!(
            infos.map(|info| (info.map, info.fewest, info.most, info.default_fighters())),
            [
                (MapId::ProvingCell, 2, 6, 6),
                (MapId::AssemblyWorks, 4, 12, 12),
                (MapId::Drydock, 8, 12, 12),
            ]
        );
        let mut menu = menu();
        assert_eq!(click(&mut menu, Target::Practice), None);
        assert_eq!(menu.screen, Screen::Practice);
        assert_eq!(menu.backdrop(), MapId::AssemblyWorks);
        for (index, info) in infos.iter().enumerate() {
            click(&mut menu, Target::Map(index));
            assert_eq!(
                menu.backdrop(),
                info.map,
                "the live view follows the choice"
            );
            // Never fewer or more than the map takes.
            for _ in 0..20 {
                click(&mut menu, Target::Fewer);
            }
            assert_eq!(menu.fighters(), info.fewest);
            for _ in 0..20 {
                click(&mut menu, Target::More);
            }
            assert_eq!(menu.fighters(), info.most);
            click(&mut menu, Target::Fewer);
            assert_eq!(
                click(&mut menu, Target::Start),
                Some(Choice::Play {
                    map: info.map,
                    fighters: info.most - 1,
                    online: false,
                })
            );
        }
        // Each map keeps its own count; Escape goes back.
        click(&mut menu, Target::Map(0));
        assert_eq!(menu.fighters(), 5);
        assert_eq!(menu.key(KeyCode::Escape, None), None);
        assert_eq!(menu.screen, Screen::Main);
        // Greybox maps are not offered.
        assert!(!PRACTICE_MAPS.contains(&MapId::GreyboxYard));
        assert!(!PRACTICE_MAPS.contains(&MapId::Greybox));
    }

    #[test]
    fn private_match_does_nothing_and_the_workshop_is_a_choice() {
        let mut menu = menu();
        assert_eq!(click(&mut menu, Target::PrivateMatch), None);
        assert_eq!(menu.screen, Screen::Main);
        assert_eq!(click(&mut menu, Target::Workshop), Some(Choice::Workshop));
    }

    #[test]
    fn a_tap_acts_only_where_it_went_down() {
        let mut menu = menu();
        let (practice, _) = menu.targets()[1];
        let at = menu.window_pixels(practice.centre());
        menu.cursor_moved(at.x, at.y);
        menu.mouse(true);
        // Dragged off before lifting: nothing.
        menu.cursor_moved(5.0, 5.0);
        assert_eq!(menu.mouse(false), None);
        assert_eq!(menu.screen, Screen::Main);
    }

    #[test]
    fn settings_keep_the_name_and_sensitivity_and_hide_the_connection() {
        let store = Rc::new(MemoryPrefs::default());
        let mut menu = Menu::new(store.clone());
        menu.frame(1280, 720, None, None);
        click(&mut menu, Target::Settings);
        assert_eq!(menu.screen, Screen::Settings);
        // The developer connection is behind Advanced.
        assert!(
            !menu
                .targets()
                .iter()
                .any(|(_, target)| *target == Target::Url)
        );
        click(&mut menu, Target::Name);
        assert_eq!(menu.editing(), Some(Field::Name));
        for (code, text) in [
            (KeyCode::KeyA, "A"),
            (KeyCode::KeyD, "d"),
            (KeyCode::KeyA, "a"),
        ] {
            menu.key(code, Some(text));
        }
        menu.key(KeyCode::Enter, None);
        click(&mut menu, Target::Faster);
        click(&mut menu, Target::Faster);
        assert_eq!(Prefs::load(store.as_ref()).name.as_deref(), Some("Ada"));
        assert_eq!(Prefs::load(store.as_ref()).sensitivity, 120);
        // A new menu (back from a match) has them.
        let again = Menu::new(store);
        assert_eq!(again.prefs.name.as_deref(), Some("Ada"));
        assert_eq!(again.prefs.sensitivity, 120);

        click(&mut menu, Target::Advanced);
        assert_eq!(click(&mut menu, Target::Connect), None, "no URL yet");
        assert!(menu.notice.is_some());
        click(&mut menu, Target::Url);
        for character in "https://localhost:4433/shooter/matches/dm".chars() {
            menu.key(KeyCode::KeyA, Some(&character.to_string()));
        }
        assert_eq!(
            click(&mut menu, Target::Connect),
            Some(Choice::Connect {
                url: "https://localhost:4433/shooter/matches/dm".into(),
                certificate: String::new(),
            })
        );
    }

    #[test]
    fn a_narrow_screen_stacks_the_menu_inside_it() {
        for (width, height) in [(390, 844), (844, 390), (1024, 768), (1920, 1080)] {
            let mut menu = Menu::new(Rc::new(MemoryPrefs::default()));
            for screen in [Target::Practice, Target::Settings] {
                menu.reopen();
                menu.frame(width, height, None, None);
                menu.act(screen);
                menu.act(Target::Advanced);
                menu.frame(width, height, None, None);
                let room = menu.width();
                for (rect, target) in menu.targets() {
                    assert!(
                        rect.x >= 0.0
                            && rect.x + rect.width <= room + 0.5
                            && rect.y + rect.height <= 720.0,
                        "{target:?} {rect:?} outside {width}x{height}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_previews_decode() {
        for bytes in PREVIEWS {
            let texture = decode(bytes).unwrap();
            assert_eq!((texture.width, texture.height), (400, 225));
            assert_eq!(texture.levels[0].len(), 400 * 225 * 4);
        }
    }

    #[test]
    fn the_robot_line_names_a_loadout_or_a_custom_build() {
        use shooter_sim::robot::{HEAVY, STANDARD};
        assert!(robot_line(&STANDARD).contains("standard loadout"));
        assert!(robot_line(&HEAVY).contains("heavy loadout"));
    }
}
