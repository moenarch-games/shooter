//! The builder on screen, the same natively and in the browser: the robot on the bench drawn from
//! the kits ([`crate::robots`]) in the middle, its sockets and the selected part's or the
//! previewed part's box and footprint marked over it, and around it the drawn panels
//! ([`crate::overlay`]'s painter): the part catalogue and the named loadouts to start from on
//! the left, the component inspector on the right, and the budget, the validation status and
//! Ready along the bottom.
//!
//! **Mouse.** Drag a part from the catalogue onto a socket, or click it to take it in hand and
//! click sockets to place copies (the sockets it fits light up; hovering one previews the part
//! there with its real geometry and footprint). Click a part on the robot to select it. Right
//! drag (or the arrow keys) turns the view round the robot, the wheel zooms; a right click
//! drops what is in hand.
//!
//! **Keys.** R, T and F turn the selected part (or the part in hand) an eighth of a turn about
//! its socket's normal, pitch and roll axes (Shift back, Ctrl a degree instead); M changes a
//! weapon's mod, Delete removes the selected part and what hangs from it, Escape drops or
//! deselects, Enter is Ready.
//!
//! The bench stands far above the arena ([`BENCH_AT`]), the robot facing +X with its root
//! chassis' centre there, so the match goes on unseen while the person builds.

use super::controls::{ControlsBench, gait_name, mode_name, output_name, source_name};
use super::{Axis, FINE, Refusal, SNAP, Workbench, degrees, part_name, weapon_of};
use crate::camera::FOV_Y;
use crate::overlay::palette::{GRAPHITE, MUTED, OLIVE, ORANGE, PALE, RED, STEEL};
use crate::overlay::{Painter, mod_name, srgb, text_width};
use crate::robots::track::rest_frame;
use crate::robots::{RobotState, RobotView, Robots};
use crate::scene::{self, METRES_PER_UNIT};
use render_engine::glam::{Mat4, Quat, Vec2, Vec3, Vec4};
use render_engine::scene::{OverlayQuad, Scene};
use render_engine::world::World;
use shooter_sim::bindings::Bindings;
use shooter_sim::mechanics::{placements, sockets};
use shooter_sim::robot::{
    Assembly, BETA, Kind, LOADOUTS, MAX_PARTS, Orientation, PartId, PartTypeId, SocketId,
};
use winit::keyboard::KeyCode;

/// Where the bench's robot has its root chassis' centre (metres): far above every arena.
pub const BENCH_AT: Vec3 = Vec3::new(0.0, 1_000.0, 0.0);
/// The robots' slot the bench's robot is drawn in (no fighter has it).
pub const BENCH_SLOT: u8 = u8::MAX;

/// Why the builder is open.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Stage {
    /// Before the match: nothing plays until Ready.
    Setup,
    /// The robot is dead and its respawn suspended until Ready (Atlas D2.i70.11).
    BetweenLives,
}

/// What the person asked of the match.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Action {
    /// Ready with this valid build and the control layout the person edited for it (none:
    /// they kept the build's preset, [`super::controls`]).
    Ready(Assembly, Option<Bindings>),
}

/// A clickable panel item.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Target {
    Catalogue(PartTypeId),
    Loadout(usize),
    /// A new robot from the chassis in hand.
    NewFrame,
    Rotate(Axis, i32),
    Mod,
    Remove,
    Ready,
    /// Opens or closes the controls panel in the inspector's place.
    Controls,
    /// A binding's row in the controls panel.
    Binding(usize),
    /// The controls panel's buttons.
    Edit(ControlEdit),
}

/// What a controls panel button does to the layout ([`super::controls::ControlsBench`]).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlEdit {
    Preset,
    /// A new binding for the selected part.
    Add,
    Delete,
    /// Waits for the key that binds the selected binding.
    Key,
    Mouse,
    Mode,
    Output,
    Invert,
    Weaker,
    Stronger,
    /// Puts the selected part in the binding's group, or takes it out.
    Part,
    FlipPart,
    Gait,
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
}

/// Layout units.
const MARGIN: f32 = 16.0;
const CATALOGUE_WIDTH: f32 = 248.0;
const INSPECTOR_WIDTH: f32 = 272.0;
const BAR_HEIGHT: f32 = 92.0;
const ROW: f32 = 18.0;
/// A catalogue or loadout row: tighter than [`ROW`], so every catalogue part and loadout fits
/// above the bar at 720 lines.
const CATALOGUE_ROW: f32 = 16.0;
const CELL: f32 = 2.0;
/// The font cell of the smaller text: a glyph cell snaps to whole pixels, so below 2 it reads no
/// smaller at 720 lines; it stays one name for the lines that would shrink first.
const SMALL: f32 = 2.0;
/// A line of text.
const LINE: f32 = 18.0;
const READY_WIDTH: f32 = 196.0;
/// The inspector's key help at its foot: six lines.
const HELP_HEIGHT: f32 = 6.0 * LINE + 8.0;
/// The binding rows the controls panel shows.
const CONTROL_ROWS: usize = 14;

/// The panels for a frame `width` units wide.
struct Layout {
    catalogue: Rect,
    inspector: Rect,
    bar: Rect,
}

impl Layout {
    fn of(width: f32) -> Self {
        let bar = Rect::new(
            MARGIN,
            720.0 - MARGIN - BAR_HEIGHT,
            width - 2.0 * MARGIN,
            BAR_HEIGHT,
        );
        let top = MARGIN;
        let height = bar.y - 8.0 - top;
        Self {
            catalogue: Rect::new(MARGIN, top, CATALOGUE_WIDTH, height),
            inspector: Rect::new(
                width - MARGIN - INSPECTOR_WIDTH,
                top,
                INSPECTOR_WIDTH,
                height,
            ),
            bar,
        }
    }

    /// Each catalogue part's row, then each loadout's.
    fn catalogue_rows(&self) -> Vec<(Rect, Target)> {
        let x = self.catalogue.x + 6.0;
        let width = self.catalogue.width - 12.0;
        let mut rows = Vec::new();
        let mut y = self.catalogue.y + 28.0;
        for kind in BETA.parts {
            rows.push((
                Rect::new(x, y, width, CATALOGUE_ROW),
                Target::Catalogue(kind.id),
            ));
            y += CATALOGUE_ROW;
        }
        y += 30.0;
        for index in 0..LOADOUTS.len() {
            rows.push((
                Rect::new(x, y, width, CATALOGUE_ROW),
                Target::Loadout(index),
            ));
            y += CATALOGUE_ROW;
        }
        rows
    }

    /// Where the loadouts' heading is.
    fn loadouts_heading(&self) -> f32 {
        self.catalogue.y + 28.0 + CATALOGUE_ROW * BETA.parts.len() as f32 + 12.0
    }

    /// The inspector's buttons.
    fn buttons(&self) -> Vec<(Rect, Target)> {
        let x = self.inspector.x + 10.0;
        let half = (self.inspector.width - 26.0) / 2.0;
        let mut y = self.inspector.y + 262.0;
        let mut buttons = Vec::new();
        for axis in [Axis::Yaw, Axis::Pitch, Axis::Roll] {
            buttons.push((Rect::new(x, y, half, 24.0), Target::Rotate(axis, -1)));
            buttons.push((
                Rect::new(x + half + 6.0, y, half, 24.0),
                Target::Rotate(axis, 1),
            ));
            y += 30.0;
        }
        buttons.push((Rect::new(x, y, half, 24.0), Target::Mod));
        buttons.push((Rect::new(x + half + 6.0, y, half, 24.0), Target::Remove));
        y += 30.0;
        buttons.push((
            Rect::new(x, y, self.inspector.width - 20.0, 24.0),
            Target::NewFrame,
        ));
        y += 30.0;
        buttons.push((
            Rect::new(x, y, self.inspector.width - 20.0, 24.0),
            Target::Controls,
        ));
        buttons
    }

    /// The controls panel's binding rows (the first [`CONTROL_ROWS`] of `count`).
    fn control_rows(&self, count: usize) -> Vec<(Rect, Target)> {
        let x = self.inspector.x + 6.0;
        let width = self.inspector.width - 12.0;
        (0..count.min(CONTROL_ROWS))
            .map(|index| {
                #[allow(clippy::cast_precision_loss)]
                let y = self.inspector.y + 58.0 + ROW * index as f32;
                (Rect::new(x, y, width, ROW), Target::Binding(index))
            })
            .collect()
    }

    /// The controls panel's buttons: the gait at the top, the edits in pairs at the foot, and
    /// the way back to the parts.
    fn control_buttons(&self) -> Vec<(Rect, Target)> {
        let x = self.inspector.x + 10.0;
        let full = self.inspector.width - 20.0;
        let half = (self.inspector.width - 26.0) / 2.0;
        let mut buttons = vec![(
            Rect::new(x, self.inspector.y + 28.0, full, 22.0),
            Target::Edit(ControlEdit::Gait),
        )];
        let mut y = self.inspector.y + 58.0 + ROW * CONTROL_ROWS as f32 + 30.0;
        for (left, right) in [
            (ControlEdit::Preset, ControlEdit::Add),
            (ControlEdit::Key, ControlEdit::Mouse),
            (ControlEdit::Mode, ControlEdit::Output),
            (ControlEdit::Invert, ControlEdit::Delete),
            (ControlEdit::Weaker, ControlEdit::Stronger),
            (ControlEdit::Part, ControlEdit::FlipPart),
        ] {
            buttons.push((Rect::new(x, y, half, 24.0), Target::Edit(left)));
            buttons.push((
                Rect::new(x + half + 6.0, y, half, 24.0),
                Target::Edit(right),
            ));
            y += 28.0;
        }
        buttons.push((Rect::new(x, y, full, 24.0), Target::Controls));
        buttons
    }

    fn ready(&self) -> Rect {
        Rect::new(
            self.bar.x + self.bar.width - READY_WIDTH - 10.0,
            self.bar.y + 10.0,
            READY_WIDTH,
            self.bar.height - 20.0,
        )
    }

    /// Every clickable item: the inspector's, or with the controls panel open (`bindings` of
    /// them) the panel's.
    fn targets(&self, controls: Option<usize>) -> Vec<(Rect, Target)> {
        let mut targets = self.catalogue_rows();
        match controls {
            Some(bindings) => {
                targets.extend(self.control_rows(bindings));
                targets.extend(self.control_buttons());
            }
            None => targets.extend(self.buttons()),
        }
        targets.push((self.ready(), Target::Ready));
        targets
    }

    /// Whether `at` is on a panel (not on the blueprint).
    fn on_panel(&self, at: Vec2) -> bool {
        [self.catalogue, self.inspector, self.bar]
            .iter()
            .any(|panel| panel.contains(at))
    }
}

/// A socket as drawn this frame.
#[derive(Clone, Copy, Debug)]
struct Spot {
    part: PartId,
    socket: SocketId,
    /// Where it is on screen (layout units), and whether it faces the camera.
    at: Vec2,
    facing: bool,
    world: Vec3,
}

/// A part's box in the world (metres).
#[derive(Clone, Copy, Debug)]
struct PartBox {
    part: PartId,
    centre: Vec3,
    rotation: Quat,
    half: Vec3,
}

impl PartBox {
    fn corners(&self) -> [Vec3; 8] {
        std::array::from_fn(|index| {
            let sign = |bit: usize| if index & bit == 0 { -1.0 } else { 1.0 };
            self.centre + self.rotation * (self.half * Vec3::new(sign(1), sign(2), sign(4)))
        })
    }

    /// Whether `point` is within the box, `slack` metres beyond its faces.
    fn holds(&self, point: Vec3, slack: f32) -> bool {
        let local = self.rotation.inverse() * (point - self.centre);
        (local.abs() - self.half).max_element() <= slack
    }

    /// How far along the ray from `origin` along unit `direction` it meets the box.
    fn ray(&self, origin: Vec3, direction: Vec3) -> Option<f32> {
        let inverse = self.rotation.inverse();
        let origin = inverse * (origin - self.centre);
        let direction = inverse * direction;
        let (mut near, mut far) = (f32::MIN, f32::MAX);
        for axis in 0..3 {
            let (o, d, h) = (origin[axis], direction[axis], self.half[axis]);
            if d.abs() < 1e-6 {
                if o.abs() > h {
                    return None;
                }
                continue;
            }
            let (a, b) = ((-h - o) / d, (h - o) / d);
            near = near.max(a.min(b));
            far = far.min(a.max(b));
        }
        (near <= far && far >= 0.0).then_some(near.max(0.0))
    }
}

/// Every part's box of `build` on the bench.
fn boxes(build: &Assembly) -> Vec<PartBox> {
    placements(build)
        .into_iter()
        .map(|placement| PartBox {
            part: placement.part,
            centre: BENCH_AT + Vec3::from(placement.centre) * METRES_PER_UNIT,
            rotation: Quat::from_array(placement.orientation).normalize(),
            half: Vec3::from(placement.half_extents) * METRES_PER_UNIT,
        })
        .collect()
}

/// The camera's clip-from-world at `eye` looking at `target`: render-engine's right-handed view
/// with +Y up and its perspective ([`scene::frame`]'s field of view), with a far plane (only
/// what lands where on screen matters here).
fn clip_from_world(eye: Vec3, target: Vec3, aspect: f32) -> Mat4 {
    let back = (eye - target).normalize_or(Vec3::Z);
    let right = Vec3::Y.cross(back).normalize_or(Vec3::X);
    let up = back.cross(right);
    let world_from_view = Mat4::from_cols(
        right.extend(0.0),
        up.extend(0.0),
        back.extend(0.0),
        eye.extend(1.0),
    );
    let (near, far) = (0.05, 100.0);
    let h = 1.0 / (0.5 * FOV_Y).tan();
    let projection = Mat4::from_cols(
        Vec4::new(h / aspect, 0.0, 0.0, 0.0),
        Vec4::new(0.0, h, 0.0, 0.0),
        Vec4::new(0.0, 0.0, far / (near - far), -1.0),
        Vec4::new(0.0, 0.0, near * far / (near - far), 0.0),
    );
    projection * world_from_view.inverse()
}

/// The capsule centre that puts `build`'s root chassis at [`BENCH_AT`] facing +X.
fn bench_center(build: &Assembly) -> Vec3 {
    BENCH_AT - rest_frame(build, Vec3::ZERO, Vec3::X).w_axis.truncate()
}

/// The builder: the bench, the view round it and the pointer.
#[derive(Clone, Debug)]
pub struct Builder {
    pub bench: Workbench,
    /// The control layout for the robot on the bench ([`super::controls`]).
    pub controls: ControlsBench,
    /// The controls panel is open in the inspector's place.
    pub controls_open: bool,
    pub stage: Stage,
    /// The build Ready sent, until the match shows it configured (the host accepted it).
    pub ready: Option<Assembly>,
    /// The last edit refused, or why the robot is not drawn.
    pub notice: Option<String>,
    /// The pointer (pixels) and the frame size.
    cursor: Vec2,
    size: (u32, u32),
    /// The view round the robot: radians about +Y from +X, radians up, and a zoom factor.
    azimuth: f32,
    elevation: f32,
    zoom: f32,
    /// The right button held: the pointer where the orbit last was, and whether it moved.
    orbit: Option<(Vec2, bool)>,
    /// A catalogue part being dragged (the left button held since it was pressed on its row).
    dragging: bool,
    shift: bool,
    ctrl: bool,
    /// The sockets and part boxes of the last frame.
    spots: Vec<Spot>,
    parts: Vec<PartBox>,
    eye: Vec3,
    view_proj: Mat4,
    /// Which sockets the part in hand fits: for the bench state it was worked out for.
    fits: Vec<bool>,
    fits_for: Option<(Assembly, Option<PartTypeId>, Orientation)>,
}

impl Builder {
    #[must_use]
    pub fn new(build: Assembly, stage: Stage) -> Self {
        Self {
            bench: Workbench::new(build),
            controls: ControlsBench::new(build, None),
            controls_open: false,
            stage,
            ready: None,
            notice: None,
            cursor: Vec2::ZERO,
            size: (1280, 720),
            azimuth: 0.55,
            elevation: 0.3,
            zoom: 1.0,
            orbit: None,
            dragging: false,
            shift: false,
            ctrl: false,
            spots: Vec::new(),
            parts: Vec::new(),
            eye: Vec3::ZERO,
            view_proj: Mat4::IDENTITY,
            fits: Vec::new(),
            fits_for: None,
        }
    }

    /// The builder with the person's control layout `layout` (none, or one that does not fit
    /// the build: the build's preset).
    #[must_use]
    pub fn with_controls(mut self, layout: Option<&Bindings>) -> Self {
        self.controls = ControlsBench::new(*self.bench.draft(), layout);
        self
    }

    fn scale(&self) -> f32 {
        #[allow(clippy::cast_precision_loss)]
        let height = self.size.1.max(1) as f32;
        height / 720.0
    }

    fn layout(&self) -> Layout {
        #[allow(clippy::cast_precision_loss)]
        let width = self.size.0 as f32 / self.scale();
        Layout::of(width)
    }

    /// The pointer in layout units.
    fn pointer(&self) -> Vec2 {
        self.cursor / self.scale()
    }

    /// The panel item under the pointer.
    #[must_use]
    pub fn target(&self) -> Option<Target> {
        let at = self.pointer();
        self.layout()
            .targets(
                self.controls_open
                    .then(|| self.controls.layout().bindings.len()),
            )
            .into_iter()
            .find(|(rect, _)| rect.contains(at))
            .map(|(_, target)| target)
    }

    /// Whether Ready may be pressed: the build is valid and nothing is waiting on the host.
    #[must_use]
    pub fn can_ready(&self) -> bool {
        self.ready.is_none() && self.bench.check().is_ok()
    }

    /// The modifier keys.
    pub const fn modifiers(&mut self, shift: bool, ctrl: bool) {
        self.shift = shift;
        self.ctrl = ctrl;
    }

    pub fn cursor_moved(&mut self, x: f32, y: f32) {
        let at = Vec2::new(x, y);
        if let Some((last, _)) = self.orbit {
            let moved = at - last;
            self.azimuth -= moved.x * 0.008;
            self.elevation = (self.elevation + moved.y * 0.006).clamp(-0.2, 0.75);
            self.orbit = Some((at, moved.length() > 0.0 || self.orbit.is_some_and(|o| o.1)));
        }
        self.cursor = at;
    }

    /// Turns the view to `azimuth` radians round the robot from its front (toward its right)
    /// and `elevation` radians up.
    pub const fn look_from(&mut self, azimuth: f32, elevation: f32) {
        self.azimuth = azimuth;
        self.elevation = elevation;
    }

    /// Puts the pointer on the first socket, facing the camera in the last frame, that the
    /// part in hand fits; whether there was one (scripted views and tests).
    pub fn point_at_a_fitting_socket(&mut self) -> bool {
        let Some(at) = self
            .spots
            .iter()
            .zip(&self.fits)
            .find(|(spot, fits)| **fits && spot.facing)
            .map(|(spot, _)| spot.at)
        else {
            return false;
        };
        self.cursor = at * self.scale();
        true
    }

    /// The wheel turned `lines` (positive away from the person: closer).
    pub fn wheel(&mut self, lines: f32) {
        self.zoom = (self.zoom * 0.9_f32.powf(lines)).clamp(0.4, 2.5);
    }

    fn refused(&mut self, result: Result<impl Sized, Refusal>) {
        self.notice = result.err().map(|refusal| refusal.to_string());
    }

    /// The socket under the pointer the part in hand fits.
    fn hovered_spot(&self) -> Option<Spot> {
        if self.bench.hand.is_none() || self.layout().on_panel(self.pointer()) {
            return None;
        }
        let at = self.pointer();
        self.spots
            .iter()
            .zip(&self.fits)
            .filter(|(spot, fits)| **fits && spot.facing)
            .map(|(spot, _)| (spot, spot.at.distance(at)))
            .filter(|(_, distance)| *distance < 14.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(spot, _)| *spot)
    }

    /// The part under the pointer, the nearest along the view.
    fn hovered_part(&self) -> Option<PartId> {
        if self.layout().on_panel(self.pointer()) {
            return None;
        }
        let ndc = Vec2::new(
            self.cursor.x / self.size.0.max(1) as f32 * 2.0 - 1.0,
            1.0 - self.cursor.y / self.size.1.max(1) as f32 * 2.0,
        );
        let inverse = self.view_proj.inverse();
        let far = inverse.project_point3(ndc.extend(0.5));
        let direction = (far - self.eye).normalize_or_zero();
        self.parts
            .iter()
            .filter_map(|part| Some((part.part, part.ray(self.eye, direction)?)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(part, _)| part)
    }

    /// The left or right button `pressed` or released; Ready when it was pressed.
    pub fn mouse(&mut self, right: bool, pressed: bool) -> Option<Action> {
        if right {
            if pressed {
                self.orbit = Some((self.cursor, false));
            } else if let Some((_, moved)) = self.orbit.take()
                && !moved
            {
                self.bench.hand = None;
            }
            return None;
        }
        if !pressed {
            if std::mem::take(&mut self.dragging) {
                if let Some(spot) = self.hovered_spot() {
                    let placed = self.bench.place(spot.part, spot.socket);
                    self.refused(placed);
                    self.bench.hand = None;
                } else if !self.layout().on_panel(self.pointer()) {
                    self.bench.hand = None;
                }
            }
            return None;
        }
        if let Some(target) = self.target() {
            return self.act(target);
        }
        if self.layout().on_panel(self.pointer()) {
            return None;
        }
        if let Some(spot) = self.hovered_spot() {
            let placed = self.bench.place(spot.part, spot.socket);
            self.refused(placed);
        } else if let Some(part) = self.hovered_part() {
            self.bench.hand = None;
            self.bench.selected = Some(part);
            self.notice = None;
        } else {
            self.bench.selected = None;
        }
        None
    }

    /// Does what a panel item says.
    pub fn act(&mut self, target: Target) -> Option<Action> {
        match target {
            Target::Catalogue(kind) => {
                self.bench.hand = Some(kind);
                self.bench.turn = Orientation::IDENTITY;
                self.bench.selected = None;
                self.dragging = true;
                self.notice = None;
            }
            Target::Loadout(index) => {
                self.bench.load(LOADOUTS[index].1);
                self.notice = None;
            }
            Target::NewFrame => {
                let chassis = self
                    .bench
                    .hand
                    .filter(|kind| {
                        BETA.part_type(*kind)
                            .is_some_and(|kind| kind.kind == Kind::Chassis)
                    })
                    .ok_or(Refusal::Nothing);
                let made = chassis.and_then(|chassis| self.bench.new_frame(chassis));
                self.refused(made);
                if self.notice.is_some() {
                    self.notice = Some("take a chassis from the catalogue first".to_owned());
                }
            }
            Target::Rotate(axis, sign) => {
                let step = if self.ctrl { FINE } else { SNAP };
                let turned = self.bench.rotate(axis, sign * i32::from(step));
                self.refused(turned);
            }
            Target::Mod => {
                let changed = self.bench.cycle_mod();
                self.refused(changed);
            }
            Target::Remove => {
                let removed = self.bench.remove();
                self.refused(removed);
            }
            Target::Ready => {
                if !self.can_ready() {
                    return None;
                }
                let build = self.bench.check().ok()?;
                self.ready = Some(build);
                self.controls.follow(&build);
                let layout = self
                    .controls
                    .custom()
                    .then(|| self.controls.layout().clone());
                return Some(Action::Ready(build, layout));
            }
            Target::Controls => {
                self.controls_open = !self.controls_open;
                self.controls.capturing = false;
                self.notice = None;
            }
            Target::Binding(index) => {
                self.controls.selected = Some(index);
                self.controls.capturing = false;
                self.notice = None;
            }
            Target::Edit(edit) => {
                let edited = self.edit_controls(edit);
                self.refused(edited);
            }
        }
        None
    }

    /// A controls panel edit, on the layout for the build on the bench now.
    fn edit_controls(&mut self, edit: ControlEdit) -> Result<(), Refusal> {
        self.controls.follow(self.bench.draft());
        let part = self.bench.selected;
        match edit {
            ControlEdit::Preset => {
                self.controls.preset();
                Ok(())
            }
            ControlEdit::Add => self.controls.add(part),
            ControlEdit::Delete => self.controls.delete(),
            ControlEdit::Key => {
                self.controls.selected.ok_or(Refusal::Nothing)?;
                self.controls.capturing = true;
                Ok(())
            }
            ControlEdit::Mouse => self.controls.cycle_mouse(),
            ControlEdit::Mode => self.controls.cycle_mode(),
            ControlEdit::Output => self.controls.cycle_output(),
            ControlEdit::Invert => self.controls.invert(),
            ControlEdit::Weaker => self.controls.strength(-10),
            ControlEdit::Stronger => self.controls.strength(10),
            ControlEdit::Part => self.controls.toggle_part(part),
            ControlEdit::FlipPart => self.controls.flip_part(part),
            ControlEdit::Gait => {
                self.controls.cycle_gait();
                Ok(())
            }
        }
    }

    /// A key pressed; Ready on Enter.
    pub fn key(&mut self, code: KeyCode) -> Option<Action> {
        // Waiting for a binding's key: this one binds it (Escape gives up).
        if self.controls.capturing {
            if code == KeyCode::Escape {
                self.controls.capturing = false;
            } else {
                self.controls.follow(self.bench.draft());
                let bound = self.controls.bind_key(&format!("{code:?}"));
                self.refused(bound);
            }
            return None;
        }
        let sign = if self.shift { -1 } else { 1 };
        match code {
            KeyCode::KeyK => self.act(Target::Controls),
            KeyCode::KeyR => self.act(Target::Rotate(Axis::Yaw, sign)),
            KeyCode::KeyT => self.act(Target::Rotate(Axis::Pitch, sign)),
            KeyCode::KeyF => self.act(Target::Rotate(Axis::Roll, sign)),
            KeyCode::KeyM => self.act(Target::Mod),
            KeyCode::Delete | KeyCode::Backspace | KeyCode::KeyX => self.act(Target::Remove),
            KeyCode::Enter | KeyCode::NumpadEnter => self.act(Target::Ready),
            KeyCode::Escape => {
                if self.bench.hand.take().is_none() {
                    self.bench.selected = None;
                }
                self.notice = None;
                None
            }
            KeyCode::ArrowLeft | KeyCode::KeyA => {
                self.azimuth += 0.15;
                None
            }
            KeyCode::ArrowRight | KeyCode::KeyD => {
                self.azimuth -= 0.15;
                None
            }
            KeyCode::ArrowUp | KeyCode::KeyW => {
                self.elevation = (self.elevation + 0.1).min(0.75);
                None
            }
            KeyCode::ArrowDown | KeyCode::KeyS => {
                self.elevation = (self.elevation - 0.1).max(-0.2);
                None
            }
            _ => None,
        }
    }

    /// Works out which sockets the part in hand fits, when the bench changed.
    fn refresh_fits(&mut self, spots: &[shooter_sim::mechanics::SocketSpot]) {
        let key = (*self.bench.draft(), self.bench.hand, self.bench.turn);
        if self.fits_for == Some(key) && self.fits.len() == spots.len() {
            return;
        }
        self.fits = spots
            .iter()
            .map(|spot| self.bench.fits(spot.part, spot.socket))
            .collect();
        self.fits_for = Some(key);
    }

    /// The frame: the robot on the bench (or what a placement would make, while the pointer is
    /// on a socket it fits) drawn by `robots` in their world, if the kit is in, the camera round
    /// it and the panels for a `width` × `height` frame.
    pub fn frame(
        &mut self,
        robots: Option<(&mut World, &mut Robots)>,
        width: u32,
        height: u32,
        seconds: f32,
    ) -> Scene {
        self.size = (width.max(1), height.max(1));
        let draft = *self.bench.draft();
        self.controls.follow(&draft);
        let socket_spots = sockets(&draft);
        self.refresh_fits(&socket_spots);

        // The camera round the robot's middle.
        let parts = boxes(&draft);
        let (low, high) = parts.iter().flat_map(PartBox::corners).fold(
            (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
            |(low, high), corner| (low.min(corner), high.max(corner)),
        );
        let target = (low + high) / 2.0;
        let reach = ((high - low).length() / 2.0).max(0.3);
        let distance = (reach * 1.9 + 0.35) * self.zoom;
        let direction = Vec3::new(
            self.elevation.cos() * self.azimuth.cos(),
            self.elevation.sin(),
            self.elevation.cos() * self.azimuth.sin(),
        );
        self.eye = target + direction * distance;
        #[allow(clippy::cast_precision_loss)]
        let aspect = self.size.0 as f32 / self.size.1 as f32;
        self.view_proj = clip_from_world(self.eye, target, aspect);
        let scale = self.scale();
        let project = |point: Vec3| -> Option<Vec2> {
            let clip = self.view_proj * point.extend(1.0);
            (clip.w > 0.0).then(|| {
                let ndc = clip.truncate() / clip.w;
                Vec2::new(
                    (ndc.x + 1.0) / 2.0 * self.size.0 as f32,
                    (1.0 - ndc.y) / 2.0 * self.size.1 as f32,
                ) / scale
            })
        };
        self.spots = socket_spots
            .iter()
            .filter_map(|spot| {
                let world = BENCH_AT + Vec3::from(spot.centre) * METRES_PER_UNIT;
                let normal = Vec3::from(spot.normal);
                Some(Spot {
                    part: spot.part,
                    socket: spot.socket,
                    at: project(world)?,
                    facing: normal.dot(self.eye - world) > 0.0,
                    world,
                })
            })
            .collect();
        self.parts = parts;

        // What is shown: the placement under the pointer, else the bench's build.
        let hovered = self.hovered_spot();
        let preview = hovered.and_then(|spot| {
            let kind = self.bench.hand?;
            self.bench
                .placed(kind, spot.part, spot.socket, self.bench.turn)
                .ok()
        });
        let shown = preview.map_or(draft, |(build, _)| build);
        let drawn = robots.map(|(world, robots)| {
            let view = RobotView {
                slot: BENCH_SLOT,
                build: shown,
                state: RobotState {
                    center: bench_center(&shown),
                    forward: Vec3::X,
                    velocity: Vec3::ZERO,
                    sliding: false,
                    hook: None,
                    since_fired: f32::INFINITY,
                    fired: shooter_sim::Instance::default(),
                    shots: 0,
                    held: BETA.weapons(&shown).next().unwrap_or_default(),
                    energy: 1.0,
                    swing: None,
                    dead_for: None,
                },
                glow: 0.0,
                parts: crate::robots::PartStates::default(),
                highlight: None,
            };
            robots.update(world, &[view], seconds)
        });
        let kit_missing = match drawn {
            Some(Ok(_)) => None,
            Some(Err(error)) => Some(error),
            None => Some("robot kit loading".to_owned()),
        };

        let mut painter = Painter {
            quads: Vec::with_capacity(4_096),
            scale,
        };
        let layout = self.layout();
        // Without the kit the parts are drawn as their boxes.
        if kit_missing.is_some() {
            for part in &self.parts {
                dotted_box(&mut painter, &project, part, srgb(MUTED, 0.6));
            }
        }
        self.blueprint(&mut painter, &project, hovered, preview);
        self.catalogue(&mut painter, &layout);
        self.inspector(&mut painter, &layout);
        self.status(&mut painter, &layout, kit_missing.as_deref());

        let mut scene = scene::frame(self.eye, (target - self.eye).normalize(), Vec::new());
        scene.clear_color = Vec3::new(0.05, 0.09, 0.16);
        scene.overlay = painter.quads;
        scene
    }

    /// The sockets, and the selected or previewed part's box and footprint.
    fn blueprint(
        &self,
        painter: &mut Painter,
        project: &dyn Fn(Vec3) -> Option<Vec2>,
        hovered: Option<Spot>,
        preview: Option<(Assembly, PartId)>,
    ) {
        let holding = self.bench.hand.is_some();
        for (spot, fits) in self.spots.iter().zip(&self.fits) {
            if !spot.facing {
                continue;
            }
            let (size, color) = if holding && *fits {
                (7.0, srgb(OLIVE, 0.95))
            } else {
                (3.0, srgb(PALE, 0.35))
            };
            painter.rect(
                spot.at.x - size / 2.0,
                spot.at.y - size / 2.0,
                size,
                size,
                color,
            );
        }
        // The previewed part: its box and the cells its footprint takes on its parent's face.
        if let (Some(spot), Some((build, id))) = (hovered, preview)
            && let Some(part) = boxes(&build).into_iter().find(|part| part.part == id)
        {
            self.footprint(painter, &part, spot.part, srgb(ORANGE, 0.9));
            dotted_box(painter, project, &part, srgb(ORANGE, 1.0));
            painter.frame(
                spot.at.x - 7.0,
                spot.at.y - 7.0,
                14.0,
                14.0,
                1.5,
                srgb(ORANGE, 1.0),
            );
        }
        if let Some(selected) = self.bench.selected
            && let Some(part) = self.parts.iter().find(|part| part.part == selected)
        {
            if let Some(parent) = self
                .bench
                .draft()
                .part(selected)
                .and_then(|part| part.mount)
            {
                self.footprint(painter, part, parent.parent, srgb(PALE, 0.8));
            }
            dotted_box(painter, project, part, srgb(PALE, 1.0));
        }
    }

    /// The sockets of `parent` under `part`'s base: its footprint.
    fn footprint(&self, painter: &mut Painter, part: &PartBox, parent: PartId, color: Vec4) {
        let slack = 0.3 * METRES_PER_UNIT;
        for spot in &self.spots {
            if spot.part == parent && spot.facing && part.holds(spot.world, slack) {
                painter.frame(spot.at.x - 5.0, spot.at.y - 5.0, 10.0, 10.0, 2.0, color);
            }
        }
    }

    fn catalogue(&self, painter: &mut Painter, layout: &Layout) {
        let panel = layout.catalogue;
        panel_back(painter, panel);
        painter.text(
            panel.x + 10.0,
            panel.y + 10.0,
            CELL,
            "PARTS",
            srgb(MUTED, 1.0),
        );
        painter.text_right(
            panel.x + panel.width - 10.0,
            panel.y + 10.0,
            CELL,
            "PTS",
            srgb(MUTED, 1.0),
        );
        let hovered = self.target();
        for (rect, target) in layout.catalogue_rows() {
            let (label, points, kind_colour, active) = match target {
                Target::Catalogue(id) => {
                    let Some(kind) = BETA.part_type(id) else {
                        continue;
                    };
                    let colour = match kind.kind {
                        Kind::Weapon(_) => ORANGE,
                        Kind::Chassis => OLIVE,
                        _ => STEEL,
                    };
                    (
                        part_name(kind),
                        kind.points.to_string(),
                        colour,
                        self.bench.hand == Some(id),
                    )
                }
                Target::Loadout(index) => (
                    LOADOUTS[index].0.to_owned(),
                    BETA.budget(&LOADOUTS[index].1).points.to_string(),
                    MUTED,
                    *self.bench.draft() == LOADOUTS[index].1,
                ),
                _ => continue,
            };
            if active {
                painter.rect(rect.x, rect.y, rect.width, rect.height, srgb(ORANGE, 0.28));
            } else if hovered == Some(target) {
                painter.rect(rect.x, rect.y, rect.width, rect.height, srgb(PALE, 0.08));
            }
            painter.rect(rect.x + 2.0, rect.y + 4.5, 3.0, 7.0, srgb(kind_colour, 1.0));
            painter.text(rect.x + 10.0, rect.y + 1.0, CELL, &label, srgb(PALE, 1.0));
            painter.text_right(
                rect.x + rect.width - 4.0,
                rect.y + 1.0,
                CELL,
                &points,
                srgb(MUTED, 1.0),
            );
        }
        painter.text(
            panel.x + 10.0,
            layout.loadouts_heading(),
            CELL,
            "START FROM",
            srgb(MUTED, 1.0),
        );
    }

    fn inspector(&self, painter: &mut Painter, layout: &Layout) {
        if self.controls_open {
            self.controls_panel(painter, layout);
            return;
        }
        let panel = layout.inspector;
        panel_back(painter, panel);
        let x = panel.x + 10.0;
        let mut y = panel.y + 10.0;
        let draft = self.bench.draft();
        let selected = self.bench.selected.and_then(|id| draft.part(id));
        let (heading, kind) = match (selected, self.bench.hand) {
            (Some(part), _) => ("SELECTED", BETA.part_type(part.kind)),
            (None, Some(hand)) => ("IN HAND", BETA.part_type(hand)),
            (None, None) => ("NOTHING SELECTED", None),
        };
        painter.text(x, y, CELL, heading, srgb(MUTED, 1.0));
        y += 22.0;
        if let Some(kind) = kind {
            // A weapon mounted more than once: which copy this one is (shooter#125).
            let name = selected
                .and_then(|part| BETA.instance(draft, part.id))
                .map_or_else(
                    || part_name(kind),
                    |instance| crate::overlay::copy_name(draft, instance),
                );
            painter.text(x, y, CELL, &name, srgb(ORANGE, 1.0));
            y += 22.0;
            let [cx, cy, cz] = kind.cells;
            let mut rows = vec![
                ("CELLS", format!("{cx} X {cy} X {cz}")),
                ("MASS", format!("{} KG", kind.mass)),
                ("HEALTH", kind.health.to_string()),
                ("POINTS", kind.points.to_string()),
            ];
            if kind.capacity > 0 || kind.kind == Kind::Chassis {
                let root = selected.is_some_and(|part| part.mount.is_none());
                let (capacity, regen) = selected.map_or((kind.capacity, kind.regen), |part| {
                    shooter_sim::robot::energy(part, kind)
                });
                let root = if root { " (ROOT)" } else { "" };
                rows.push(("ENERGY", format!("{capacity} +{regen}/S{root}")));
            }
            if kind.drive.torque > 0 {
                rows.push((
                    "MOTOR",
                    format!("{} NM, {}/S", kind.drive.torque, kind.drive.draw),
                ));
            }
            if let Some(weapon) = weapon_of(kind) {
                rows.push((
                    "SHOT",
                    format!("{} ENERGY", weapon.energy() / shooter_sim::energy::MILLI),
                ));
            }
            let turn = selected
                .and_then(|part| part.mount)
                .map_or(self.bench.turn, |mount| mount.orientation);
            if selected.is_none_or(|part| part.mount.is_some()) {
                let (yaw, pitch, roll) = turn.raw();
                rows.push((
                    "TURN",
                    format!("{} {} {}", degrees(yaw), degrees(pitch), degrees(roll)),
                ));
            } else {
                rows.push(("ROOT", "THE VITAL PART".to_owned()));
            }
            if let Some(part) = selected
                && weapon_of(kind).is_some()
            {
                rows.push(("MOD", part.modification.map_or("NONE", mod_name).to_owned()));
            }
            for (label, value) in rows {
                painter.text(x, y, SMALL, label, srgb(MUTED, 1.0));
                painter.text(x + 84.0, y, SMALL, &value, srgb(PALE, 1.0));
                y += LINE;
            }
        } else {
            for help in [
                "DRAG A PART FROM THE",
                "LIST ONTO A SOCKET,",
                "OR CLICK IT, THEN",
                "SOCKETS. CLICK A",
                "PART TO SELECT IT.",
            ] {
                painter.text(x, y, SMALL, help, srgb(PALE, 0.8));
                y += LINE;
            }
        }
        let hovered = self.target();
        for (rect, target) in layout.buttons() {
            let label = match target {
                Target::Rotate(Axis::Yaw, -1) => "YAW- (R)",
                Target::Rotate(Axis::Yaw, _) => "YAW+",
                Target::Rotate(Axis::Pitch, -1) => "PITCH- (T)",
                Target::Rotate(Axis::Pitch, _) => "PITCH+",
                Target::Rotate(Axis::Roll, -1) => "ROLL- (F)",
                Target::Rotate(Axis::Roll, _) => "ROLL+",
                Target::Mod => "MOD (M)",
                Target::Remove => "REMOVE (X)",
                Target::NewFrame => "NEW FRAME (CHASSIS)",
                Target::Controls => "CONTROLS (K)",
                _ => continue,
            };
            button(painter, rect, label, hovered == Some(target), false);
        }
        let help_y = panel.y + panel.height - HELP_HEIGHT;
        for (index, help) in [
            "SHIFT+KEY: TURN BACK",
            "CTRL+KEY: 1 DEGREE",
            "RIGHT DRAG: VIEW",
            "WHEEL: ZOOM",
            "RIGHT CLICK: DROP",
            "ENTER: READY",
        ]
        .iter()
        .enumerate()
        {
            #[allow(clippy::cast_precision_loss)]
            painter.text(
                x,
                help_y + index as f32 * LINE,
                SMALL,
                help,
                srgb(MUTED, 0.9),
            );
        }
    }

    /// The controls panel in the inspector's place: the gait, a row per binding (its input,
    /// command, mode, strength and how many parts it drives), the selected binding's parts, and
    /// the edits. Parts are chosen by clicking them on the robot.
    fn controls_panel(&self, painter: &mut Painter, layout: &Layout) {
        let panel = layout.inspector;
        panel_back(painter, panel);
        let x = panel.x + 10.0;
        let heading = if self.controls.custom() {
            "CONTROLS"
        } else {
            "CONTROLS: PRESET"
        };
        painter.text(x, panel.y + 10.0, CELL, heading, srgb(MUTED, 1.0));
        let hovered = self.target();
        let bindings = &self.controls.layout().bindings;
        for (rect, target) in layout.control_rows(bindings.len()) {
            let Target::Binding(index) = target else {
                continue;
            };
            let binding = &bindings[index];
            let selected = self.controls.selected == Some(index);
            if selected || hovered == Some(target) {
                painter.rect(
                    rect.x,
                    rect.y,
                    rect.width,
                    rect.height,
                    srgb(STEEL, if selected { 0.9 } else { 0.5 }),
                );
            }
            let source = if selected && self.controls.capturing {
                "PRESS".to_owned()
            } else {
                source_name(binding.source)
            };
            let sign = if binding.invert { '-' } else { '+' };
            let row = format!(
                "{source:<8}{} {}{sign}{}",
                &output_name(binding.output)[..3],
                &mode_name(binding.mode)[..1],
                binding.strength,
            );
            painter.text(rect.x + 4.0, rect.y + 2.0, SMALL, &row, srgb(PALE, 1.0));
            let parts = format!("{}P", binding.targets.len());
            let width = text_width(&parts, painter.snap_cell(SMALL));
            painter.text(
                rect.x + rect.width - width - 4.0,
                rect.y + 2.0,
                SMALL,
                &parts,
                srgb(MUTED, 1.0),
            );
        }
        #[allow(clippy::cast_precision_loss)]
        let detail_y = panel.y + 58.0 + ROW * CONTROL_ROWS as f32 + 6.0;
        let detail = self
            .controls
            .selected
            .and_then(|index| bindings.get(index))
            .map_or_else(
                || {
                    if bindings.len() > CONTROL_ROWS {
                        format!("+{} MORE", bindings.len() - CONTROL_ROWS)
                    } else {
                        "CLICK A ROW, THEN A PART".to_owned()
                    }
                },
                |binding| {
                    let parts: Vec<String> = binding
                        .targets
                        .iter()
                        .map(|target| {
                            format!("{}{}", target.part.0, if target.invert { "-" } else { "" })
                        })
                        .collect();
                    format!("PARTS {}", parts.join(" "))
                },
            );
        painter.text(x, detail_y, SMALL, &detail, srgb(ORANGE, 1.0));
        for (rect, target) in layout.control_buttons() {
            let gait;
            let label = match target {
                Target::Edit(ControlEdit::Gait) => {
                    gait = format!("GAIT: {}", gait_name(self.controls.layout().gait));
                    gait.as_str()
                }
                Target::Edit(ControlEdit::Preset) => "PRESET",
                Target::Edit(ControlEdit::Add) => "NEW (PART)",
                Target::Edit(ControlEdit::Key) => "KEY...",
                Target::Edit(ControlEdit::Mouse) => "MOUSE",
                Target::Edit(ControlEdit::Mode) => "MODE",
                Target::Edit(ControlEdit::Output) => "COMMAND",
                Target::Edit(ControlEdit::Invert) => "INVERT",
                Target::Edit(ControlEdit::Delete) => "DELETE",
                Target::Edit(ControlEdit::Weaker) => "STRENGTH-",
                Target::Edit(ControlEdit::Stronger) => "STRENGTH+",
                Target::Edit(ControlEdit::Part) => "PART +/-",
                Target::Edit(ControlEdit::FlipPart) => "FLIP PART",
                Target::Controls => "PARTS (K)",
                _ => continue,
            };
            button(painter, rect, label, hovered == Some(target), false);
        }
    }

    fn status(&self, painter: &mut Painter, layout: &Layout, kit_missing: Option<&str>) {
        let bar = layout.bar;
        panel_back(painter, bar);
        let budget = self.bench.budget();
        let x = bar.x + 12.0;
        let meter = |painter: &mut Painter, y: f32, label: &str, have: u32, limit: u32| {
            let over = have > limit;
            painter.text(x, y, SMALL, label, srgb(MUTED, 1.0));
            painter.text_right(
                x + 230.0,
                y,
                SMALL,
                &format!("{have}/{limit}"),
                srgb(if over { RED } else { PALE }, 1.0),
            );
            painter.rect(x, y + 16.0, 230.0, 5.0, srgb(STEEL, 1.0));
            #[allow(clippy::cast_precision_loss)]
            let share = (have as f32 / limit.max(1) as f32).min(1.0);
            painter.rect(
                x,
                y + 16.0,
                230.0 * share,
                5.0,
                srgb(if over { RED } else { OLIVE }, 1.0),
            );
        };
        meter(
            painter,
            bar.y + 8.0,
            "POINTS",
            budget.points,
            budget.build_points,
        );
        #[allow(clippy::cast_possible_truncation)]
        meter(
            painter,
            bar.y + 36.0,
            "PARTS",
            budget.parts,
            MAX_PARTS as u32,
        );
        painter.text(
            x,
            bar.y + 66.0,
            SMALL,
            &format!(
                "MASS {} KG   ENERGY {} +{}/S",
                budget.mass, budget.capacity, budget.regen
            ),
            srgb(PALE, 1.0),
        );
        let text_x = x + 250.0;
        let stage = match self.stage {
            Stage::Setup => "BEFORE THE MATCH",
            Stage::BetweenLives => "DESTROYED: RESPAWN WAITS FOR READY",
        };
        painter.text(text_x, bar.y + 10.0, SMALL, stage, srgb(MUTED, 1.0));
        let (verdict, colour) = match self.bench.check() {
            Ok(_) => ("VALID BUILD".to_owned(), OLIVE),
            Err(error) => (format!("INVALID: {error}"), RED),
        };
        painter.text(text_x, bar.y + 30.0, CELL, &verdict, srgb(colour, 1.0));
        if let Some(note) = self.notice.as_deref().or(kit_missing) {
            painter.text(text_x, bar.y + 58.0, SMALL, note, srgb(ORANGE, 1.0));
        }
        let ready = layout.ready();
        let label = if self.ready.is_some() {
            "WAITING FOR HOST"
        } else {
            "READY"
        };
        button(
            painter,
            ready,
            label,
            self.target() == Some(Target::Ready),
            self.can_ready(),
        );
    }
}

/// A panel's background and edge.
fn panel_back(painter: &mut Painter, rect: Rect) {
    painter.rect(
        rect.x,
        rect.y,
        rect.width,
        rect.height,
        srgb(GRAPHITE, 0.88),
    );
    painter.frame(
        rect.x,
        rect.y,
        rect.width,
        rect.height,
        1.0,
        srgb(STEEL, 1.0),
    );
}

/// A button: `strong` filled orange (Ready while it may be pressed).
fn button(painter: &mut Painter, rect: Rect, label: &str, hovered: bool, strong: bool) {
    let fill = match (strong, hovered) {
        (true, true) => srgb(ORANGE, 1.0),
        (true, false) => srgb(ORANGE, 0.85),
        (false, true) => srgb(STEEL, 1.0),
        (false, false) => srgb(STEEL, 0.6),
    };
    painter.rect(rect.x, rect.y, rect.width, rect.height, fill);
    let cell = if rect.height > 40.0 { 3.0 } else { SMALL };
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

/// `part`'s box as dotted edges.
fn dotted_box(
    painter: &mut Painter,
    project: &dyn Fn(Vec3) -> Option<Vec2>,
    part: &PartBox,
    color: Vec4,
) {
    let corners = part.corners();
    for (a, b) in [
        (0, 1),
        (2, 3),
        (4, 5),
        (6, 7),
        (0, 2),
        (1, 3),
        (4, 6),
        (5, 7),
        (0, 4),
        (1, 5),
        (2, 6),
        (3, 7),
    ] {
        let (Some(from), Some(to)) = (project(corners[a]), project(corners[b])) else {
            continue;
        };
        let length = from.distance(to);
        let dots = (length / 4.0).ceil().clamp(1.0, 400.0);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        for step in 0..=dots as u32 {
            #[allow(clippy::cast_precision_loss)]
            let at = from.lerp(to, step as f32 / dots);
            painter.rect(at.x - 0.75, at.y - 0.75, 1.5, 1.5, color);
        }
    }
}

/// The prompt while the local robot is dead and the builder closed: the respawn countdown and
/// Edit Robot (B), centred low on a `width` × `height` frame.
#[must_use]
pub fn death_prompt(respawn_in: f32, width: u32, height: u32) -> Vec<OverlayQuad> {
    #[allow(clippy::cast_precision_loss)]
    let (width, height) = (width as f32, height as f32);
    let scale = height / 720.0;
    let mut painter = Painter {
        quads: Vec::new(),
        scale,
    };
    let centre = width / scale / 2.0;
    let panel = Rect::new(centre - 170.0, 470.0, 340.0, 78.0);
    panel_back(&mut painter, panel);
    let line = format!("RESPAWN IN {respawn_in:.1} S");
    let cell = painter.snap_cell(CELL);
    painter.text(
        centre - text_width(&line, cell) / 2.0,
        panel.y + 12.0,
        CELL,
        &line,
        srgb(PALE, 1.0),
    );
    let edit = "[B] EDIT ROBOT";
    painter.text(
        centre - text_width(edit, cell) / 2.0,
        panel.y + 44.0,
        CELL,
        edit,
        srgb(ORANGE, 1.0),
    );
    painter.quads
}

/// The world's up-facing rotation for `forward` (the robots' frame), for tests.
#[cfg(test)]
fn facing(forward: Vec3) -> render_engine::glam::Mat3 {
    render_engine::glam::Mat3::from_cols(forward, Vec3::Y, forward.cross(Vec3::Y))
}

#[cfg(test)]
mod tests {
    use super::*;
    use shooter_sim::robot::parts::{BATTERY, CHASSIS_S};
    use shooter_sim::robot::{Face, HEAVY, STANDARD};

    /// A builder after one frame without the kit, on a 1280 × 720 frame.
    fn drawn(stage: Stage) -> Builder {
        let mut builder = Builder::new(STANDARD, stage);
        builder.frame(None, 1280, 720, 0.0);
        builder
    }

    #[test]
    fn the_robot_s_root_sits_on_the_bench_facing_forward() {
        assert_eq!(facing(Vec3::X), render_engine::glam::Mat3::IDENTITY);
        for build in [STANDARD, HEAVY] {
            let frame = rest_frame(&build, bench_center(&build), Vec3::X);
            assert!(frame.w_axis.truncate().distance(BENCH_AT) < 1e-3);
        }
    }

    #[test]
    fn a_catalogue_part_is_dragged_onto_a_socket_it_fits() {
        let mut builder = drawn(Stage::Setup);
        let layout = builder.layout();
        let (row, _) = layout
            .catalogue_rows()
            .into_iter()
            .find(|(_, target)| *target == Target::Catalogue(BATTERY.id))
            .unwrap();
        builder.cursor_moved(row.x + 20.0, row.y + 5.0);
        assert_eq!(builder.mouse(false, true), None);
        assert_eq!(builder.bench.hand, Some(BATTERY.id));
        // The next frame lights up the sockets the battery fits; drop it on the back's first.
        builder.frame(None, 1280, 720, 0.0);
        let back = CHASSIS_S.socket(Face::NegX, 0, 0).unwrap();
        let (spot, fits) = builder
            .spots
            .iter()
            .zip(&builder.fits)
            .find(|(spot, _)| spot.part == PartId(0) && spot.socket == back)
            .unwrap();
        assert!(*fits);
        // Turn the view to face the back first.
        let at = if spot.facing {
            spot.at
        } else {
            builder.azimuth += std::f32::consts::PI;
            builder.frame(None, 1280, 720, 0.0);
            builder
                .spots
                .iter()
                .find(|spot| spot.part == PartId(0) && spot.socket == back)
                .unwrap()
                .at
        };
        builder.cursor_moved(at.x, at.y);
        builder.mouse(false, false);
        assert_eq!(builder.notice, None);
        assert_eq!(builder.bench.hand, None, "a drop is one placement");
        assert_eq!(
            builder.bench.draft().parts().len(),
            STANDARD.parts().len() + 1
        );
    }

    #[test]
    fn ready_needs_a_valid_build_and_waits_for_the_host_once() {
        let mut builder = drawn(Stage::BetweenLives);
        let ready = builder.layout().ready();
        builder.cursor_moved(ready.x + 5.0, ready.y + 5.0);
        assert_eq!(builder.target(), Some(Target::Ready));
        assert_eq!(
            builder.mouse(false, true),
            Some(Action::Ready(STANDARD, None))
        );
        assert_eq!(builder.ready, Some(STANDARD));
        assert!(!builder.can_ready());
        assert_eq!(builder.key(KeyCode::Enter), None, "already sent");
        // An over-budget build cannot be readied.
        let mut builder = drawn(Stage::BetweenLives);
        builder.bench.load(shooter_sim::robot::ARMORY);
        assert!(!builder.can_ready());
        assert_eq!(builder.key(KeyCode::Enter), None);
    }

    #[test]
    fn clicking_a_part_selects_it_and_keys_edit_it() {
        let mut builder = drawn(Stage::Setup);
        // From behind (nothing is on the standard build's back), the root chassis' centre is the
        // bench's.
        builder.azimuth += std::f32::consts::PI;
        builder.frame(None, 1280, 720, 0.0);
        let clip = builder.view_proj * BENCH_AT.extend(1.0);
        let ndc = clip.truncate() / clip.w;
        builder.cursor_moved((ndc.x + 1.0) / 2.0 * 1280.0, (1.0 - ndc.y) / 2.0 * 720.0);
        builder.mouse(false, true);
        assert_eq!(builder.bench.selected, Some(PartId(0)));
        builder.key(KeyCode::Delete);
        assert!(builder.notice.is_some(), "the root stays");
        builder.bench.selected = Some(PartId(2));
        builder.key(KeyCode::KeyR);
        let turned = builder
            .bench
            .draft()
            .part(PartId(2))
            .unwrap()
            .mount
            .unwrap();
        assert_eq!(turned.orientation.yaw, SNAP);
        builder.key(KeyCode::Delete);
        assert!(builder.bench.draft().part(PartId(2)).is_none());
    }

    #[test]
    fn the_panels_stay_clear_of_the_middle_at_four_by_three() {
        let layout = Layout::of(960.0);
        assert!(layout.catalogue.x + layout.catalogue.width < layout.inspector.x - 200.0);
        let rows = layout.catalogue_rows();
        let last = rows.last().unwrap().0;
        assert!(last.y + last.height <= layout.bar.y - 8.0, "{last:?}");
        let buttons = layout.buttons();
        let lowest = buttons.last().unwrap().0;
        assert!(
            lowest.y + lowest.height < layout.inspector.y + layout.inspector.height - HELP_HEIGHT
        );
        let controls = layout.control_buttons();
        let lowest = controls.last().unwrap().0;
        assert!(lowest.y + lowest.height <= layout.inspector.y + layout.inspector.height);
        let rows = layout.control_rows(CONTROL_ROWS);
        assert!(rows.last().unwrap().0.y + ROW < controls[1].0.y);
    }

    /// Clicks the panel item `target` (its rectangle's middle).
    fn click(builder: &mut Builder, target: Target) -> Option<Action> {
        let (rect, _) = builder
            .layout()
            .targets(
                builder
                    .controls_open
                    .then(|| builder.controls.layout().bindings.len()),
            )
            .into_iter()
            .find(|(_, item)| *item == target)
            .unwrap_or_else(|| panic!("{target:?} is on screen"));
        let scale = builder.scale();
        builder.cursor_moved(
            (rect.x + rect.width / 2.0) * scale,
            (rect.y + rect.height / 2.0) * scale,
        );
        builder.mouse(false, true)
    }

    #[test]
    fn the_controls_panel_edits_the_layout_and_ready_sends_it() {
        let mut builder = drawn(Stage::Setup);
        // K opens the panel in the inspector's place, on the build's preset.
        builder.key(KeyCode::KeyK);
        assert!(builder.controls_open);
        builder.frame(None, 1280, 720, 0.0);
        assert!(!builder.controls.custom());
        // Pick the W binding, wait for a key, press I: the legs stride on I.
        click(&mut builder, Target::Binding(0));
        click(&mut builder, Target::Edit(ControlEdit::Key));
        assert!(builder.controls.capturing);
        builder.key(KeyCode::KeyI);
        assert_eq!(
            builder.controls.layout().bindings[0].source,
            shooter_sim::bindings::Source::Key(shooter_sim::bindings::key("KeyI").unwrap())
        );
        // Toggle, and weaker.
        click(&mut builder, Target::Edit(ControlEdit::Mode));
        click(&mut builder, Target::Edit(ControlEdit::Weaker));
        let binding = &builder.controls.layout().bindings[0];
        assert_eq!(binding.mode, shooter_sim::bindings::Mode::Toggle);
        assert_eq!(binding.strength, 90);
        // The chassis selected on the bench is no actuator: the panel says why.
        builder.bench.selected = Some(PartId(0));
        click(&mut builder, Target::Edit(ControlEdit::Part));
        assert!(builder.notice.is_some());
        // Ready sends the edited layout with the build; back to the preset sends none.
        let Some(Action::Ready(build, Some(layout))) = click(&mut builder, Target::Ready) else {
            panic!("Ready with the edited layout");
        };
        assert_eq!(build, STANDARD);
        assert_eq!(layout.bindings[0].strength, 90);
        let mut builder = drawn(Stage::Setup).with_controls(Some(&layout));
        builder.key(KeyCode::KeyK);
        assert!(builder.controls.custom());
        click(&mut builder, Target::Edit(ControlEdit::Preset));
        assert_eq!(
            click(&mut builder, Target::Ready),
            Some(Action::Ready(STANDARD, None))
        );
    }
}
