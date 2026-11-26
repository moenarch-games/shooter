//! The builder on screen, the same natively and in the browser: the robot on the bench drawn from
//! the kits ([`crate::robots`]) in the middle, its sockets and the selected part's or the
//! previewed part's box and footprint marked over it, and around it the drawn panels
//! ([`crate::overlay`]'s painter): the part catalogue and the named loadouts to start from on
//! the left, the component inspector on the right, and the budget, the validation status and
//! Ready along the bottom.
//!
//! **Mouse.** Drag a part from the catalogue onto a socket, or click it to take it in hand and
//! click sockets to place copies. The sockets it fits light up. The socket under the pointer is
//! found by a ray from the camera through the pointer ([`Builder::pick`]): the first part box it
//! meets, and the socket cell of that box's face it meets (so a crowded face, a cell beside a
//! taken footprint, a large part and a turned parent all resolve to the cell actually under the
//! pointer, and sockets behind other parts are never taken); where the ray meets no socket face
//! (over a part with no sockets, or just off the robot), the nearest lit socket within a few
//! pixels. The socket under the pointer previews the part there turned as it is in hand: its
//! real geometry, its box and the cells its footprint reserves, or, if the rules refuse it, its
//! box and footprint in red, the cells another part holds marked, and the reason. Click a part
//! on the robot to select it. Right drag (or the arrow keys) turns the view round the robot, the
//! wheel zooms; a right click drops what is in hand.
//!
//! **Keys.** R, T and F turn the part in hand (or, with nothing in hand, the selected part) about
//! its socket's normal, pitch and roll axes by the step (G changes it: a quarter turn, an eighth,
//! about a degree; Shift turns back, Ctrl a degree whatever the step); M changes a weapon's mod,
//! Delete removes the selected part and what hangs from it, Z undoes and Shift+Z or Y redoes,
//! Escape cancels a drag, drops what is in hand or deselects, Enter is Ready.
//!
//! **Saved builds** (shooter#140). B (or BUILDS) opens the builds panel in the inspector's place
//! ([`super::saved`]): letters, digits, `-` and Space type the name, Backspace erases, Enter or
//! SAVE saves the build and its control layout (a name already saved waits for a second press
//! to overwrite), a click on a listed save picks it, LOAD puts it on the bench (one edit undo
//! takes back) and DELETE removes it on a second press. Escape or BACK closes the panel.
//!
//! **Pointer geometry.** The pointer arrives in the window's pixels and the frame is drawn in
//! the surface's ([`Builder::resize`] tells the builder the window's size): in the browser both
//! are device pixels (CSS pixels times the device pixel ratio), and a surface the GPU limits to a
//! smaller size than the canvas still maps the pointer onto what is drawn under it.
//!
//! The bench stands far above the arena ([`BENCH_AT`]), the robot facing +X with its root
//! chassis' centre there, so the match goes on unseen while the person builds.

use super::controls::{ControlsBench, gait_name, mode_name, output_name, source_name};
use super::saved::{self, Confirm, SavedBuilds, Shelf};
use super::{
    Axis, BENCH, FINE, Proposal, Refusal, Step, Workbench, degrees, describe, part_name, weapon_of,
    with_power,
};
use crate::camera::FOV_Y;
use crate::gfx::World;
use crate::overlay::palette::{GRAPHITE, MUTED, OLIVE, ORANGE, PALE, RED, STEEL};
use crate::overlay::{Painter, mod_name, srgb, text_width};
use crate::robots::track::rest_frame;
use crate::robots::{RobotState, RobotView, Robots};
use crate::scene::{self, METRES_PER_UNIT};
use crate::ui::Item;
use render_engine::glam::{Quat, Vec2, Vec3, Vec4};
use render_engine::scene::Scene;
use shooter_sim::bindings::Bindings;
use shooter_sim::energy::{Pool, Resource};
use shooter_sim::mechanics::{Placement, placement_of, placements, sockets};
use shooter_sim::robot::{
    Assembly, BETA, CELL_MM, Kind, LOADOUTS, MAX_PARTS, Orientation, PartId, PartType, PartTypeId,
    Power, SocketId,
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
    /// The main menu's Robot Workshop (shooter#154): no match. Ready (Done) keeps the build for
    /// the next match, Back returns to the menu without it.
    Workshop,
}

/// What the person asked of the match.
// Returned once per press and matched at once: boxing the build would buy nothing.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Action {
    /// Ready with this valid build and the control layout the person edited for it (none:
    /// they kept the build's preset, [`super::controls`]).
    Ready(Assembly, Option<Bindings>),
    /// Back to the main menu, the bench's build left as it was before (the Workshop).
    Back,
}

/// A clickable panel item.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Target {
    Catalogue(PartTypeId),
    /// Switches the robot's power, electric or combustion (shooter#130): its root chassis and
    /// the chassis the catalogue gives.
    Power,
    Loadout(usize),
    /// A new robot from the chassis in hand.
    NewFrame,
    Rotate(Axis, i32),
    /// The next rotation step.
    Step,
    Mod,
    Remove,
    Undo,
    Redo,
    Ready,
    /// Back to the main menu (the Workshop's).
    Back,
    /// Opens or closes the controls panel in the inspector's place.
    Controls,
    /// A binding's row in the controls panel.
    Binding(usize),
    /// The controls panel's buttons.
    Edit(ControlEdit),
    /// Opens or closes the builds panel in the inspector's place.
    Builds,
    /// A saved build's row in the builds panel.
    Saved(usize),
    /// The builds panel's buttons.
    Shelf(ShelfEdit),
}

/// What a builds panel button does ([`super::saved::Shelf`]).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShelfEdit {
    Save,
    Load,
    Delete,
}

/// What the inspector's place shows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Side {
    Parts,
    /// The controls panel, with this many bindings.
    Controls(usize),
    /// The builds panel, with this many saves.
    Builds(usize),
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
const BACK_WIDTH: f32 = 120.0;
/// The inspector's key help at its foot.
const HELP: [&str; 7] = [
    "SHIFT+KEY: TURN BACK",
    "CTRL+KEY: 1 DEGREE",
    "Z UNDO, Y REDO",
    "RIGHT DRAG: VIEW",
    "WHEEL: ZOOM",
    "ESC: CANCEL, DROP",
    "ENTER: READY",
];
#[allow(clippy::cast_precision_loss)]
const HELP_HEIGHT: f32 = HELP.len() as f32 * LINE + 8.0;
/// The binding rows the controls panel shows.
const CONTROL_ROWS: usize = 14;
/// The saves the builds panel lists, from this far down it.
const SHELF_ROWS: usize = 20;
const SHELF_TOP: f32 = 146.0;

/// The panels for a frame `width` units wide.
struct Layout {
    catalogue: Rect,
    inspector: Rect,
    bar: Rect,
    /// The Workshop's Back button, left of Ready.
    back: bool,
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
            back: false,
        }
    }

    /// The Workshop's Back button: left of Ready, as tall.
    fn back(&self) -> Rect {
        let ready = self.ready();
        Rect::new(
            ready.x - BACK_WIDTH - 10.0,
            ready.y,
            BACK_WIDTH,
            ready.height,
        )
    }

    /// The power switch's row, each catalogue part's (a chassis once a size: the switch picks
    /// its power, shooter#130), then the loadouts', two to a row.
    fn catalogue_rows(&self) -> Vec<(Rect, Target)> {
        let x = self.catalogue.x + 6.0;
        let width = self.catalogue.width - 12.0;
        let mut rows = Vec::new();
        let mut y = self.catalogue.y + 28.0;
        rows.push((Rect::new(x, y, width, CATALOGUE_ROW), Target::Power));
        y += CATALOGUE_ROW;
        for kind in catalogue_parts() {
            rows.push((
                Rect::new(x, y, width, CATALOGUE_ROW),
                Target::Catalogue(kind.id),
            ));
            y += CATALOGUE_ROW;
        }
        y += 30.0;
        let half = width / 2.0;
        for index in 0..LOADOUTS.len() {
            #[allow(clippy::cast_precision_loss)]
            let column = (index % 2) as f32;
            rows.push((
                Rect::new(x + column * half, y, half, CATALOGUE_ROW),
                Target::Loadout(index),
            ));
            if index % 2 == 1 {
                y += CATALOGUE_ROW;
            }
        }
        rows
    }

    /// Where the loadouts' heading is.
    fn loadouts_heading(&self) -> f32 {
        self.catalogue.y + 28.0 + CATALOGUE_ROW * (catalogue_parts().count() + 1) as f32 + 12.0
    }

    /// The inspector's buttons.
    fn buttons(&self) -> Vec<(Rect, Target)> {
        let x = self.inspector.x + 10.0;
        let half = (self.inspector.width - 26.0) / 2.0;
        let mut y = self.inspector.y + 250.0;
        let mut buttons = Vec::new();
        let rows = [
            (Target::Rotate(Axis::Yaw, -1), Target::Rotate(Axis::Yaw, 1)),
            (
                Target::Rotate(Axis::Pitch, -1),
                Target::Rotate(Axis::Pitch, 1),
            ),
            (
                Target::Rotate(Axis::Roll, -1),
                Target::Rotate(Axis::Roll, 1),
            ),
            (Target::Step, Target::NewFrame),
            (Target::Mod, Target::Remove),
            (Target::Undo, Target::Redo),
        ];
        for (left, right) in rows {
            buttons.push((Rect::new(x, y, half, 24.0), left));
            buttons.push((Rect::new(x + half + 6.0, y, half, 24.0), right));
            y += 28.0;
        }
        buttons.push((Rect::new(x, y, half, 24.0), Target::Controls));
        buttons.push((Rect::new(x + half + 6.0, y, half, 24.0), Target::Builds));
        buttons
    }

    /// The builds panel's name field.
    fn name_field(&self) -> Rect {
        Rect::new(
            self.inspector.x + 10.0,
            self.inspector.y + 30.0,
            self.inspector.width - 20.0,
            24.0,
        )
    }

    /// The builds panel's buttons, in pairs under the name field.
    fn builds_buttons(&self) -> Vec<(Rect, Target)> {
        let x = self.inspector.x + 10.0;
        let half = (self.inspector.width - 26.0) / 2.0;
        let y = self.inspector.y + 62.0;
        vec![
            (Rect::new(x, y, half, 24.0), Target::Shelf(ShelfEdit::Save)),
            (
                Rect::new(x + half + 6.0, y, half, 24.0),
                Target::Shelf(ShelfEdit::Load),
            ),
            (
                Rect::new(x, y + 28.0, half, 24.0),
                Target::Shelf(ShelfEdit::Delete),
            ),
            (
                Rect::new(x + half + 6.0, y + 28.0, half, 24.0),
                Target::Builds,
            ),
        ]
    }

    /// The builds panel's rows (the first [`SHELF_ROWS`] of `count` saves).
    fn builds_rows(&self, count: usize) -> Vec<(Rect, Target)> {
        let x = self.inspector.x + 6.0;
        let width = self.inspector.width - 12.0;
        (0..count.min(SHELF_ROWS))
            .map(|index| {
                #[allow(clippy::cast_precision_loss)]
                let y = self.inspector.y + SHELF_TOP + ROW * index as f32;
                (Rect::new(x, y, width, ROW), Target::Saved(index))
            })
            .collect()
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

    /// Every clickable item: the catalogue's, what the inspector's place shows, and Ready.
    fn targets(&self, side: Side) -> Vec<(Rect, Target)> {
        let mut targets = self.catalogue_rows();
        match side {
            Side::Controls(bindings) => {
                targets.extend(self.control_rows(bindings));
                targets.extend(self.control_buttons());
            }
            Side::Builds(saves) => {
                targets.extend(self.builds_rows(saves));
                targets.extend(self.builds_buttons());
            }
            Side::Parts => targets.extend(self.buttons()),
        }
        targets.push((self.ready(), Target::Ready));
        if self.back {
            targets.push((self.back(), Target::Back));
        }
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
    /// Whether the camera sees it: it faces the camera and no part is in front of it.
    visible: bool,
    world: Vec3,
    normal: Vec3,
}

/// A socket cell's half width (metres): a ray meets a socket's cell within this of its centre.
#[allow(clippy::cast_precision_loss)]
const HALF_CELL: f32 = CELL_MM as f32 / 2_000.0;
/// How far (layout units) from a lit socket the pointer still snaps to it when its ray meets no
/// socket face.
const SNAP_REACH: f32 = 14.0;

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

    /// The outward normal of the face `point` (on the box) is on: the face it is nearest.
    fn normal_at(&self, point: Vec3) -> Vec3 {
        let local = self.rotation.inverse() * (point - self.centre);
        let gap = local.abs() - self.half;
        let axis = if gap.x >= gap.y && gap.x >= gap.z {
            Vec3::X
        } else if gap.y >= gap.z {
            Vec3::Y
        } else {
            Vec3::Z
        };
        self.rotation * (axis * local.signum())
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
    placements(build).into_iter().map(box_of).collect()
}

/// A part's placement as its box on the bench.
fn box_of(placement: Placement) -> PartBox {
    PartBox {
        part: placement.part,
        centre: BENCH_AT + Vec3::from(placement.centre) * METRES_PER_UNIT,
        rotation: Quat::from_array(placement.orientation).normalize(),
        half: Vec3::from(placement.half_extents) * METRES_PER_UNIT,
    }
}

/// The bench's camera: render-engine's right-handed view at `eye` with +Y up and its perspective
/// ([`scene::frame`]'s field of view) on a frame of `size` pixels. Points go on screen, and the
/// pointer's pixels back into rays, relative to the eye, so the bench's height above the arena
/// costs no precision.
#[derive(Clone, Copy, Debug)]
struct Camera {
    eye: Vec3,
    right: Vec3,
    up: Vec3,
    back: Vec3,
    /// The projection's focal length (1 / tan of half the vertical field of view).
    focal: f32,
    /// The frame in pixels, and pixels a layout unit.
    size: Vec2,
    scale: f32,
}

impl Camera {
    /// At `eye` looking at `target`, on a `size` frame.
    #[allow(clippy::cast_precision_loss)]
    fn new(eye: Vec3, target: Vec3, size: (u32, u32)) -> Self {
        let back = (eye - target).normalize_or(Vec3::Z);
        let right = Vec3::Y.cross(back).normalize_or(Vec3::X);
        let size = Vec2::new(size.0.max(1) as f32, size.1.max(1) as f32);
        Self {
            eye,
            right,
            up: back.cross(right),
            back,
            focal: 1.0 / (0.5 * FOV_Y).tan(),
            size,
            scale: size.y / 720.0,
        }
    }

    /// Where `point` is drawn (layout units), if it is in front of the camera.
    fn screen(&self, point: Vec3) -> Option<Vec2> {
        let offset = point - self.eye;
        let depth = -offset.dot(self.back);
        (depth > 1e-4).then(|| {
            let aspect = self.size.x / self.size.y;
            let ndc = Vec2::new(
                offset.dot(self.right) * self.focal / aspect,
                offset.dot(self.up) * self.focal,
            ) / depth;
            Vec2::new((ndc.x + 1.0) / 2.0, (1.0 - ndc.y) / 2.0) * self.size / self.scale
        })
    }

    /// The unit direction from the eye through `pixels` on the frame.
    fn ray(&self, pixels: Vec2) -> Vec3 {
        let aspect = self.size.x / self.size.y;
        let ndc = Vec2::new(
            pixels.x / self.size.x * 2.0 - 1.0,
            1.0 - pixels.y / self.size.y * 2.0,
        );
        (self.right * (ndc.x * aspect / self.focal) + self.up * (ndc.y / self.focal) - self.back)
            .normalize_or_zero()
    }
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
    /// The saved builds ([`super::saved`]) and whether their panel is open in the inspector's
    /// place.
    pub shelf: Shelf,
    pub builds_open: bool,
    pub stage: Stage,
    /// The build Ready sent, until the match shows it configured (the host accepted it).
    pub ready: Option<Assembly>,
    /// The last edit refused, or why the robot is not drawn.
    pub notice: Option<String>,
    /// How far a rotation key turns.
    pub step: Step,
    /// The pointer (the window's pixels), the window's size (the pointer's space, once known)
    /// and the frame's size (the drawn surface's pixels).
    cursor: Vec2,
    window: Option<(u32, u32)>,
    size: (u32, u32),
    /// The placement under the pointer in the last frame.
    proposal: Option<Proposal>,
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
    camera: Camera,
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
            shelf: Shelf::new(saved::platform()),
            builds_open: false,
            stage,
            ready: None,
            notice: None,
            step: Step::default(),
            cursor: Vec2::ZERO,
            window: None,
            size: (1280, 720),
            proposal: None,
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
            camera: Camera::new(Vec3::X, Vec3::ZERO, (1280, 720)),
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

    /// The builder keeping its saved builds in `store` (by default this client's,
    /// [`saved::platform`]).
    #[must_use]
    pub fn with_store(mut self, store: std::rc::Rc<dyn SavedBuilds>) -> Self {
        self.shelf = Shelf::new(store);
        self
    }

    /// What the inspector's place shows.
    fn side(&self) -> Side {
        if self.builds_open {
            Side::Builds(self.shelf.names.len())
        } else if self.controls_open {
            Side::Controls(self.controls.layout().bindings.len())
        } else {
            Side::Parts
        }
    }

    fn scale(&self) -> f32 {
        #[allow(clippy::cast_precision_loss)]
        let height = self.size.1.max(1) as f32;
        height / 720.0
    }

    fn layout(&self) -> Layout {
        #[allow(clippy::cast_precision_loss)]
        let width = self.size.0 as f32 / self.scale();
        Layout {
            back: self.stage == Stage::Workshop,
            ..Layout::of(width)
        }
    }

    /// The pointer in the frame's pixels: the window's pixels scaled onto the drawn surface.
    #[allow(clippy::cast_precision_loss)]
    fn pointer_pixels(&self) -> Vec2 {
        let Some((width, height)) = self.window else {
            return self.cursor;
        };
        self.cursor
            * Vec2::new(
                self.size.0 as f32 / width.max(1) as f32,
                self.size.1 as f32 / height.max(1) as f32,
            )
    }

    /// The pointer in layout units.
    fn pointer(&self) -> Vec2 {
        self.pointer_pixels() / self.scale()
    }

    /// Puts the pointer at `at` (layout units).
    fn put_pointer(&mut self, at: Vec2) {
        self.cursor = self.window_pixels(at);
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

    /// The window is `width` × `height` pixels: the space the pointer arrives in (in the browser,
    /// the canvas' device pixels), which may differ from the drawn surface's.
    pub const fn resize(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.window = Some((width, height));
        }
    }

    /// The panel item under the pointer.
    #[must_use]
    pub fn target(&self) -> Option<Target> {
        let at = self.pointer();
        self.layout()
            .targets(self.side())
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

    /// Puts the pointer on the first socket, seen in the last frame, that the part in hand fits;
    /// whether there was one (scripted views and tests).
    pub fn point_at_a_fitting_socket(&mut self) -> bool {
        let Some(at) = self
            .spots
            .iter()
            .zip(&self.fits)
            .find(|(spot, fits)| **fits && spot.visible)
            .map(|(spot, _)| spot.at)
        else {
            return false;
        };
        self.put_pointer(at);
        true
    }

    /// Puts the pointer on `socket` of `part` where the last frame drew it; whether the camera
    /// saw it (scripted views and tests).
    pub fn point_at(&mut self, part: PartId, socket: SocketId) -> bool {
        let Some(spot) = self
            .spots
            .iter()
            .find(|spot| spot.part == part && spot.socket == socket)
            .copied()
        else {
            return false;
        };
        self.put_pointer(spot.at);
        spot.visible
    }

    /// The placement under the pointer in the last frame: the part in hand on the socket picked,
    /// its footprint and the simulation's verdict.
    #[must_use]
    pub const fn proposal(&self) -> Option<&Proposal> {
        self.proposal.as_ref()
    }

    /// What the builder shows after the last frame, as JSON, for scripted checks of a real
    /// client (the browser page sets it on `window.shooterBuilder`; `scripts/builder-web.mjs`,
    /// shooter#124): the build on the bench (its wire bytes in hex, so an undo's build compares
    /// exactly), whether it is valid and why not, the part in hand and its turn, the selected
    /// part and its turn, the step in degrees, the history, the notice, the placement under the
    /// pointer and its verdict, whether Ready was sent, and where the sockets the camera sees
    /// and the panels' items are, in the window's pixels (the pointer's space).
    #[must_use]
    pub fn readout(&self) -> String {
        let draft = self.bench.draft();
        let key = |kind: PartTypeId| BENCH.part_type(kind).map(|kind| kind.key);
        let turn = |orientation: Orientation| {
            let (yaw, pitch, roll) = orientation.raw();
            [yaw, pitch, roll]
        };
        let at = |layout: Vec2| {
            let pixels = self.window_pixels(layout);
            serde_json::json!([pixels.x.round(), pixels.y.round()])
        };
        let (undo, redo) = self.bench.history();
        let selected = self.bench.selected.and_then(|id| draft.part(id));
        let proposal = self.proposal.as_ref().map(|proposal| {
            serde_json::json!({
                "parent": proposal.parent().0,
                "socket": proposal.socket().0,
                "turn": proposal.part.mount.map(|mount| turn(mount.orientation)),
                "footprint": proposal.footprint.iter().map(|socket| socket.0).collect::<Vec<_>>(),
                "refused": proposal.verdict.as_ref().err().map(|error| {
                    describe(draft, *error, Some(proposal.part.id))
                }),
            })
        });
        let sockets: Vec<_> = self
            .spots
            .iter()
            .zip(self.fits.iter().chain(std::iter::repeat(&false)))
            .filter(|(spot, _)| spot.visible)
            .map(|(spot, fits)| {
                serde_json::json!({
                    "part": spot.part.0,
                    "socket": spot.socket.0,
                    "at": at(spot.at),
                    "fits": fits,
                })
            })
            .collect();
        let targets: Vec<_> = self
            .layout()
            .targets(self.side())
            .into_iter()
            .map(|(rect, target)| {
                let name = match target {
                    Target::Catalogue(kind) => key(kind).unwrap_or("?").to_owned(),
                    other => format!("{other:?}"),
                };
                serde_json::json!({
                    "name": name,
                    "at": at(Vec2::new(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0)),
                })
            })
            .collect();
        serde_json::json!({
            "build": draft_hex(draft),
            "parts": draft.parts().len(),
            "valid": self.bench.check().is_ok(),
            "error": self.bench.check().err().map(|error| describe(draft, error, None)),
            "hand": self.bench.hand.and_then(key),
            "turn": turn(self.bench.turn),
            "selected": selected.map(|part| part.id.0),
            "selectedTurn": selected.and_then(|part| part.mount).map(|mount| turn(mount.orientation)),
            "step": degrees(self.step.angle()),
            "undo": undo,
            "redo": redo,
            "notice": self.notice,
            "proposal": proposal,
            "ready": self.ready.is_some(),
            "sockets": sockets,
            "targets": targets,
        })
        .to_string()
    }

    /// The wheel turned `lines` (positive away from the person: closer).
    pub fn wheel(&mut self, lines: f32) {
        self.zoom = (self.zoom * 0.9_f32.powf(lines)).clamp(0.4, 2.5);
    }

    fn refused(&mut self, result: Result<impl Sized, Refusal>) {
        let draft = *self.bench.draft();
        let subject = self.bench.selected.filter(|_| self.bench.hand.is_none());
        self.notice = result
            .err()
            .map(|refusal| refusal.describe(&draft, subject));
    }

    /// The unit direction of the ray from the eye through the pointer.
    fn pointer_ray(&self) -> Vec3 {
        self.camera.ray(self.pointer_pixels())
    }

    /// The nearest part box along the ray from the eye along `direction`, and how far.
    fn first_hit(&self, direction: Vec3) -> Option<(&PartBox, f32)> {
        self.parts
            .iter()
            .filter_map(|part| Some((part, part.ray(self.eye, direction)?)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
    }

    /// The socket under the pointer, while a part is in hand, whether the part fits it or not:
    /// the socket cell the pointer's ray meets on the first part box it meets; where that is no
    /// socket face (a part without sockets there, or nothing), the nearest lit socket within
    /// [`SNAP_REACH`] of the pointer.
    fn pick(&self) -> Option<Spot> {
        if self.bench.hand.is_none() || self.layout().on_panel(self.pointer()) {
            return None;
        }
        let direction = self.pointer_ray();
        if let Some((part, distance)) = self.first_hit(direction) {
            let point = self.eye + direction * distance;
            let normal = part.normal_at(point);
            let cell = self
                .spots
                .iter()
                .filter(|spot| spot.part == part.part && spot.normal.dot(normal) > 0.9)
                .map(|spot| {
                    let off = part.rotation.inverse() * (spot.world - point);
                    (spot, off.abs().max_element())
                })
                .filter(|(_, off)| *off <= HALF_CELL * 1.01)
                .min_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((spot, _)) = cell {
                return Some(*spot);
            }
        }
        let at = self.pointer();
        self.spots
            .iter()
            .zip(&self.fits)
            .filter(|(spot, fits)| **fits && spot.visible)
            .map(|(spot, _)| (spot, spot.at.distance(at)))
            .filter(|(_, distance)| *distance < SNAP_REACH)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(spot, _)| *spot)
    }

    /// The part under the pointer, the nearest along the view.
    fn hovered_part(&self) -> Option<PartId> {
        if self.layout().on_panel(self.pointer()) {
            return None;
        }
        self.first_hit(self.pointer_ray())
            .map(|(part, _)| part.part)
    }

    /// Places the part in hand on `spot` as the preview shows it; whether it was placed (else
    /// the notice says why).
    fn place_at(&mut self, spot: Spot) -> bool {
        let Some(kind) = self.bench.hand else {
            return false;
        };
        let proposal = self
            .bench
            .propose(kind, spot.part, spot.socket, self.bench.turn);
        let placed = self.bench.commit_proposal(&proposal);
        let done = placed.is_ok();
        self.refused(placed);
        done
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
            // A drop places one copy; a refused drop keeps the part in hand (the notice says
            // why) to turn it or try another socket; a drop off the robot puts it back.
            if std::mem::take(&mut self.dragging) {
                if let Some(spot) = self.pick() {
                    if self.place_at(spot) {
                        self.bench.hand = None;
                    }
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
        if let Some(spot) = self.pick() {
            self.place_at(spot);
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
            Target::Power => {
                let next = match self.bench.power() {
                    Power::Electric => Power::Combustion,
                    Power::Combustion => Power::Electric,
                };
                let switched = self.bench.set_power(next);
                self.refused(switched);
            }
            Target::Catalogue(kind) => {
                // A chassis row gives the chassis of the robot's power (shooter#130).
                self.bench.hand = Some(with_power(kind, self.bench.power()));
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
                            .is_some_and(|kind| matches!(kind.kind, Kind::Chassis(_)))
                    })
                    .ok_or(Refusal::Nothing);
                let made = chassis.and_then(|chassis| self.bench.new_frame(chassis));
                let missing = made == Err(Refusal::Nothing);
                self.refused(made);
                if missing {
                    self.notice = Some("take a chassis from the catalogue first".to_owned());
                }
            }
            Target::Rotate(axis, sign) => {
                let step = if self.ctrl { FINE } else { self.step.angle() };
                let turned = self.bench.rotate(axis, sign * i32::from(step));
                self.refused(turned);
            }
            Target::Step => {
                self.step = self.step.next();
            }
            Target::Undo => {
                let undone = self.bench.undo();
                self.refused(undone);
            }
            Target::Redo => {
                let redone = self.bench.redo();
                self.refused(redone);
            }
            Target::Mod => {
                let changed = self.bench.cycle_mod();
                self.refused(changed);
            }
            Target::Remove => {
                let removed = self.bench.remove();
                self.refused(removed);
            }
            Target::Back => return Some(Action::Back),
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
                self.builds_open = false;
                self.controls.capturing = false;
                self.notice = None;
            }
            Target::Builds => {
                self.builds_open = !self.builds_open;
                self.controls.capturing = false;
                self.shelf.confirm = None;
                self.notice = None;
                if self.builds_open {
                    self.controls_open = false;
                    self.notice = self.shelf.refresh().err();
                }
            }
            Target::Saved(index) => {
                self.shelf.select(index);
                self.notice = None;
            }
            Target::Shelf(edit) => self.edit_shelf(edit),
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

    /// A builds panel button: saves the bench's build with its layout (none while it is the
    /// build's preset), loads a save onto the bench as one edit (the bench keeps its build if
    /// the save is refused) or deletes one; the notice says what happened, or why not.
    fn edit_shelf(&mut self, edit: ShelfEdit) {
        self.notice = Some(match edit {
            ShelfEdit::Save => {
                let draft = *self.bench.draft();
                self.controls.follow(&draft);
                let layout = self
                    .controls
                    .custom()
                    .then(|| self.controls.layout().clone());
                self.shelf
                    .save(draft, layout)
                    .unwrap_or_else(|error| format!("can't save: {error}"))
            }
            ShelfEdit::Load => match self.shelf.load() {
                Ok(saved) => {
                    self.bench.hand = None;
                    self.bench.load(saved.build);
                    self.controls = ControlsBench::new(saved.build, saved.layout.as_ref());
                    format!("loaded {}", saved.name)
                }
                Err(error) => format!("can't load: {error}"),
            },
            ShelfEdit::Delete => self
                .shelf
                .delete()
                .unwrap_or_else(|error| format!("can't delete: {error}")),
        });
    }

    /// A key while the builds panel is open: the name's letters, Backspace, Enter saves,
    /// Escape closes the panel.
    fn shelf_key(&mut self, code: KeyCode) -> Option<Action> {
        let typed = match code {
            KeyCode::Escape => return self.act(Target::Builds),
            KeyCode::Enter | KeyCode::NumpadEnter => {
                return self.act(Target::Shelf(ShelfEdit::Save));
            }
            KeyCode::Backspace => {
                self.shelf.erase();
                return None;
            }
            KeyCode::Space => Some('_'),
            KeyCode::Minus | KeyCode::NumpadSubtract => Some('-'),
            code => {
                let name = format!("{code:?}");
                name.strip_prefix("Key")
                    .or_else(|| name.strip_prefix("Digit"))
                    .or_else(|| name.strip_prefix("Numpad"))
                    .filter(|rest| rest.len() == 1)
                    .and_then(|rest| rest.chars().next())
            }
        };
        if let Some(c) = typed {
            self.shelf.type_char(c);
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
        // The builds panel takes the keys for the name.
        if self.builds_open {
            return self.shelf_key(code);
        }
        let sign = if self.shift { -1 } else { 1 };
        match code {
            KeyCode::KeyK => self.act(Target::Controls),
            KeyCode::KeyB => self.act(Target::Builds),
            KeyCode::KeyR => self.act(Target::Rotate(Axis::Yaw, sign)),
            KeyCode::KeyT => self.act(Target::Rotate(Axis::Pitch, sign)),
            KeyCode::KeyF => self.act(Target::Rotate(Axis::Roll, sign)),
            KeyCode::KeyG => self.act(Target::Step),
            KeyCode::KeyZ if self.shift => self.act(Target::Redo),
            KeyCode::KeyZ => self.act(Target::Undo),
            KeyCode::KeyY => self.act(Target::Redo),
            KeyCode::KeyM => self.act(Target::Mod),
            KeyCode::Delete | KeyCode::Backspace | KeyCode::KeyX => self.act(Target::Remove),
            KeyCode::Enter | KeyCode::NumpadEnter => self.act(Target::Ready),
            // Cancels a drag (nothing is placed when the button comes up), else drops what is in
            // hand, else deselects.
            KeyCode::Escape => {
                let idle =
                    !self.dragging && self.bench.hand.is_none() && self.bench.selected.is_none();
                self.dragging = false;
                if self.bench.hand.take().is_none() {
                    self.bench.selected = None;
                }
                self.notice = None;
                // With nothing to let go of, the Workshop's Escape is Back.
                (idle && self.stage == Stage::Workshop).then_some(Action::Back)
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
    /// it and the panels for a `width` × `height` frame: the scene and the UI over it.
    pub fn frame(
        &mut self,
        robots: Option<(&mut World, &mut Robots)>,
        width: u32,
        height: u32,
        seconds: f32,
    ) -> (Scene, Vec<Item>) {
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
        self.camera = Camera::new(self.eye, target, self.size);
        let scale = self.scale();
        let camera = self.camera;
        let project = move |point: Vec3| camera.screen(point);
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
                    visible: false,
                    world,
                    normal,
                })
            })
            .collect();
        self.parts = parts;
        // A socket is seen when it faces the camera and the first box on the way to it is its
        // own (a part on it, or in front of it, hides it).
        for index in 0..self.spots.len() {
            let spot = self.spots[index];
            let offset = spot.world - self.eye;
            let length = offset.length();
            self.spots[index].visible = spot.facing
                && self
                    .first_hit(offset / length)
                    .is_none_or(|(_, distance)| distance >= length - 0.25 * METRES_PER_UNIT);
        }

        // What is shown: the placement under the pointer if the rules take it, else the bench's
        // build.
        self.proposal = self.pick().and_then(|spot| {
            let kind = self.bench.hand?;
            Some(
                self.bench
                    .propose(kind, spot.part, spot.socket, self.bench.turn),
            )
        });
        let shown = self
            .proposal
            .as_ref()
            .and_then(|proposal| proposal.verdict.ok())
            .unwrap_or(draft);
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
                    tube: 0,
                    recent: crate::robots::pose::Recent::NONE,
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
        self.blueprint(&mut painter, &project);
        self.catalogue(&mut painter, &layout);
        self.inspector(&mut painter, &layout);
        self.status(&mut painter, &layout, kit_missing.as_deref());

        let mut scene = scene::frame(self.eye, (target - self.eye).normalize(), Vec::new());
        scene.clear_color = Vec3::new(0.05, 0.09, 0.16);
        (scene, painter.quads)
    }

    /// The sockets the camera sees (those the part in hand fits lit), the placement under the
    /// pointer (its box and footprint; refused, in red with the cells another part holds and
    /// that part's box marked) and the selected part's box and footprint.
    fn blueprint(&self, painter: &mut Painter, project: &dyn Fn(Vec3) -> Option<Vec2>) {
        let holding = self.bench.hand.is_some();
        for (spot, fits) in self.spots.iter().zip(&self.fits) {
            if !spot.visible {
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
        // The placement under the pointer: where its box would be (the simulation's placement,
        // the same whether the rules take it or not) and the cells its footprint reserves.
        if let Some(proposal) = &self.proposal {
            let colour = if proposal.verdict.is_ok() {
                ORANGE
            } else {
                RED
            };
            let parent = proposal.parent();
            self.footprint(painter, parent, &proposal.footprint, srgb(colour, 0.9));
            for (cell, holder) in &proposal.blocked {
                if let Some(spot) = self.spot(parent, *cell)
                    && spot.facing
                {
                    painter.rect(spot.at.x - 4.0, spot.at.y - 4.0, 8.0, 8.0, srgb(RED, 0.9));
                }
                if let Some(part) = self.parts.iter().find(|part| part.part == *holder) {
                    dotted_box(painter, project, part, srgb(RED, 0.8));
                }
            }
            if let Some(placement) = placement_of(self.bench.draft(), &proposal.part) {
                dotted_box(painter, project, &box_of(placement), srgb(colour, 1.0));
            }
            if let Some(spot) = self.spot(parent, proposal.socket()) {
                painter.frame(
                    spot.at.x - 7.0,
                    spot.at.y - 7.0,
                    14.0,
                    14.0,
                    1.5,
                    srgb(colour, 1.0),
                );
            }
        }
        if let Some(selected) = self.bench.selected
            && let Some(part) = self.parts.iter().find(|part| part.part == selected)
        {
            let draft = self.bench.draft();
            if let Some(placed) = draft.part(selected)
                && let Some(mount) = placed.mount
            {
                let cells = BENCH.footprint(
                    draft,
                    mount.parent,
                    mount.socket,
                    placed.kind,
                    placed.modification,
                    mount.orientation,
                );
                self.footprint(painter, mount.parent, &cells, srgb(PALE, 0.8));
            }
            dotted_box(painter, project, part, srgb(PALE, 1.0));
        }
    }

    /// `socket` of `part` as the last frame drew it.
    fn spot(&self, part: PartId, socket: SocketId) -> Option<Spot> {
        self.spots
            .iter()
            .find(|spot| spot.part == part && spot.socket == socket)
            .copied()
    }

    /// The `cells` (sockets) of `parent` facing the camera, framed: a footprint.
    fn footprint(&self, painter: &mut Painter, parent: PartId, cells: &[SocketId], color: Vec4) {
        for spot in &self.spots {
            if spot.part == parent && spot.facing && cells.contains(&spot.socket) {
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
                Target::Power => (
                    format!("Power: {}", self.bench.power().name()),
                    "SWAP".to_owned(),
                    OLIVE,
                    false,
                ),
                Target::Catalogue(id) => {
                    let id = with_power(id, self.bench.power());
                    let Some(kind) = BETA.part_type(id) else {
                        continue;
                    };
                    let colour = match kind.kind {
                        Kind::Weapon(_) => ORANGE,
                        Kind::Chassis(_) => OLIVE,
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
        if self.builds_open {
            self.builds_panel(painter, layout);
            return;
        }
        if self.controls_open {
            self.controls_panel(painter, layout);
            return;
        }
        let panel = layout.inspector;
        panel_back(painter, panel);
        let x = panel.x + 10.0;
        let mut y = panel.y + 10.0;
        let draft = self.bench.draft();
        // What the rotation keys turn: the part in hand, else the selected part.
        let selected = self
            .bench
            .selected
            .and_then(|id| draft.part(id))
            .filter(|_| self.bench.hand.is_none());
        let (heading, kind) = match (self.bench.hand, selected) {
            (Some(hand), _) => ("IN HAND", BETA.part_type(hand)),
            (None, Some(part)) => ("SELECTED", BETA.part_type(part.kind)),
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
            if kind.capacity > 0 || matches!(kind.kind, Kind::Chassis(_)) {
                let root = selected.is_some_and(|part| part.mount.is_none());
                let capacity = selected.map_or(kind.capacity, |part| {
                    shooter_sim::robot::capacity(part, kind)
                });
                let root = if root { " (ROOT)" } else { "" };
                rows.push(("POWER", format!("{capacity} STORED{root}")));
            }
            if kind.fuel > 0 {
                rows.push(("FUEL", format!("{} STORED", kind.fuel)));
            }
            if let Kind::Chassis(power) = kind.kind {
                rows.push(("DRIVE", drive_text(power).to_owned()));
            }
            if kind.kind == Kind::Engine {
                rows.push(("DRIVES", "COMBUSTION LEGS/WHEELS".to_owned()));
            }
            if kind.burn > 0 {
                // At the rate of the chassis on the bench (shooter#130).
                let rate = Pool::of(draft).electricity_per_fuel();
                rows.push((
                    "GENERATES",
                    format!(
                        "{} POWER/S FROM {} FUEL/S",
                        u32::from(kind.burn) * rate,
                        kind.burn
                    ),
                ));
            }
            if kind.drive.torque > 0 {
                rows.push((
                    "MOTOR",
                    format!("{} NM, {}/S", kind.drive.torque, kind.drive.draw),
                ));
            }
            if let Some(weapon) = weapon_of(kind) {
                #[allow(clippy::cast_precision_loss)]
                let cost = weapon.energy() as f32 / shooter_sim::energy::MILLI as f32;
                rows.push((
                    "SHOT",
                    format!("{cost:.1} {}", resource_text(weapon.resource())),
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
        let (can_undo, can_redo) = self.bench.history();
        for (rect, target) in layout.buttons() {
            let label = match target {
                Target::Rotate(Axis::Yaw, -1) => "YAW-",
                Target::Rotate(Axis::Yaw, _) => "YAW+ (R)",
                Target::Rotate(Axis::Pitch, -1) => "PITCH-",
                Target::Rotate(Axis::Pitch, _) => "PITCH+ (T)",
                Target::Rotate(Axis::Roll, -1) => "ROLL-",
                Target::Rotate(Axis::Roll, _) => "ROLL+ (F)",
                Target::Step => match self.step {
                    Step::Quarter => "90 DEG (G)",
                    Step::Eighth => "45 DEG (G)",
                    Step::Fine => "1 DEG (G)",
                },
                Target::Mod => "MOD (M)",
                Target::Remove => "REMOVE (X)",
                Target::NewFrame => "NEW FRAME",
                Target::Undo if can_undo => "UNDO (Z)",
                Target::Redo if can_redo => "REDO (Y)",
                Target::Undo => "NO UNDO",
                Target::Redo => "NO REDO",
                Target::Controls => "CONTROLS (K)",
                Target::Builds => "BUILDS (B)",
                _ => continue,
            };
            button(painter, rect, label, hovered == Some(target), false);
        }
        let help_y = panel.y + panel.height - HELP_HEIGHT;
        for (index, help) in HELP.iter().enumerate() {
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

    /// The builds panel in the inspector's place: the name being typed, the buttons, and a row
    /// per save (the picked one marked).
    fn builds_panel(&self, painter: &mut Painter, layout: &Layout) {
        let panel = layout.inspector;
        panel_back(painter, panel);
        let x = panel.x + 10.0;
        painter.text(x, panel.y + 10.0, CELL, "SAVED BUILDS", srgb(MUTED, 1.0));
        let field = layout.name_field();
        painter.rect(
            field.x,
            field.y,
            field.width,
            field.height,
            srgb(GRAPHITE, 1.0),
        );
        painter.frame(
            field.x,
            field.y,
            field.width,
            field.height,
            1.0,
            srgb(ORANGE, 1.0),
        );
        let (name, tone) = if self.shelf.name.is_empty() {
            ("TYPE A NAME".to_owned(), MUTED)
        } else {
            (format!("{}_", self.shelf.name), PALE)
        };
        painter.text(field.x + 6.0, field.y + 6.0, SMALL, &name, srgb(tone, 1.0));
        let hovered = self.target();
        for (rect, target) in layout.builds_buttons() {
            let confirm = self.shelf.confirm.as_ref();
            let (label, strong) = match target {
                Target::Shelf(ShelfEdit::Save) => match confirm {
                    Some(Confirm::Overwrite(_)) => ("OVERWRITE?", true),
                    _ => ("SAVE", false),
                },
                Target::Shelf(ShelfEdit::Load) => ("LOAD", false),
                Target::Shelf(ShelfEdit::Delete) => match confirm {
                    Some(Confirm::Delete(_)) => ("DELETE?", true),
                    _ => ("DELETE", false),
                },
                Target::Builds => ("BACK (ESC)", false),
                _ => continue,
            };
            button(painter, rect, label, hovered == Some(target), strong);
        }
        let names = &self.shelf.names;
        let hint = if names.is_empty() {
            "NO SAVED BUILDS YET".to_owned()
        } else {
            format!("{} SAVED: CLICK ONE", names.len())
        };
        painter.text(
            x,
            panel.y + SHELF_TOP - 22.0,
            SMALL,
            &hint,
            srgb(MUTED, 1.0),
        );
        for (rect, target) in layout.builds_rows(names.len()) {
            let Target::Saved(index) = target else {
                continue;
            };
            let selected = self.shelf.selected == Some(index);
            if selected || hovered == Some(target) {
                painter.rect(
                    rect.x,
                    rect.y,
                    rect.width,
                    rect.height,
                    srgb(STEEL, if selected { 0.9 } else { 0.5 }),
                );
            }
            painter.text(
                rect.x + 4.0,
                rect.y + 2.0,
                SMALL,
                &names[index],
                srgb(PALE, 1.0),
            );
        }
        if names.len() > SHELF_ROWS {
            #[allow(clippy::cast_precision_loss)]
            painter.text(
                x,
                panel.y + SHELF_TOP + ROW * SHELF_ROWS as f32 + 4.0,
                SMALL,
                &format!("+{} MORE", names.len() - SHELF_ROWS),
                srgb(MUTED, 1.0),
            );
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
                "{}   MASS {} KG   POWER {}   FUEL {}   +{}/S",
                budget.power.name().to_uppercase(),
                budget.mass,
                budget.capacity,
                budget.fuel,
                budget.burn * Pool::of(self.bench.draft()).electricity_per_fuel()
            ),
            srgb(PALE, 1.0),
        );
        let text_x = x + 250.0;
        let stage = match self.stage {
            Stage::Setup => "BEFORE THE MATCH",
            Stage::BetweenLives => "DESTROYED: RESPAWN WAITS FOR READY",
            Stage::Workshop => "ROBOT WORKSHOP: DONE PLAYS THIS BUILD NEXT",
        };
        painter.text(text_x, bar.y + 10.0, SMALL, stage, srgb(MUTED, 1.0));
        let (verdict, colour) = match self.bench.check() {
            Ok(_) => ("VALID BUILD".to_owned(), OLIVE),
            Err(error) => (format!("INVALID: {error}"), RED),
        };
        painter.text(text_x, bar.y + 30.0, CELL, &verdict, srgb(colour, 1.0));
        // What the powertrain cannot supply (shooter#130): advice, the build stays valid.
        if let Some(supply) = supply_line(self.bench.draft()) {
            painter.text(text_x, bar.y + 54.0, SMALL, &supply, srgb(ORANGE, 1.0));
        }
        // Why the placement under the pointer is refused, else the last refused edit.
        let refused = self.proposal.as_ref().and_then(|proposal| {
            let error = proposal.verdict.err()?;
            Some(format!(
                "CAN'T PLACE: {}",
                describe(self.bench.draft(), error, None)
            ))
        });
        let ready = layout.ready();
        let (note, tone) = match (refused, self.notice.as_deref().or(kit_missing)) {
            (Some(reason), _) => (Some(reason), RED),
            (None, note) => (note.map(str::to_owned), ORANGE),
        };
        // Over the blueprint, just above the bar between the panels.
        if let Some(note) = note {
            let left = layout.catalogue.x + layout.catalogue.width + 12.0;
            let room = layout.inspector.x - 12.0 - left;
            let cell = painter.snap_cell(SMALL);
            let lines = wrap(&note.to_uppercase(), room - 16.0, cell);
            let lines = &lines[..lines.len().min(3)];
            #[allow(clippy::cast_precision_loss)]
            let height = lines.len() as f32 * LINE + 10.0;
            let width = lines
                .iter()
                .map(|line| text_width(line, cell))
                .fold(0.0, f32::max)
                + 16.0;
            let x = left + (room - width) / 2.0;
            let y = bar.y - 10.0 - height;
            painter.rect(x, y, width, height, srgb(GRAPHITE, 0.92));
            painter.frame(x, y, width, height, 1.0, srgb(tone, 1.0));
            for (index, line) in lines.iter().enumerate() {
                #[allow(clippy::cast_precision_loss)]
                let at = y + 6.0 + index as f32 * LINE;
                painter.text(x + 8.0, at, SMALL, line, srgb(tone, 1.0));
            }
        }
        let label = match (self.ready.is_some(), self.stage) {
            (true, _) => "WAITING FOR HOST",
            (false, Stage::Workshop) => "DONE",
            (false, _) => "READY",
        };
        button(
            painter,
            ready,
            label,
            self.target() == Some(Target::Ready),
            self.can_ready(),
        );
        if layout.back {
            button(
                painter,
                layout.back(),
                "BACK",
                self.target() == Some(Target::Back),
                false,
            );
        }
    }
}

/// The part types the catalogue lists: every one but the combustion chassis, which the
/// electric chassis' rows give while the power switch says combustion ([`Target::Power`]).
fn catalogue_parts() -> impl Iterator<Item = &'static PartType> {
    BETA.parts
        .iter()
        .filter(|kind| kind.kind != Kind::Chassis(Power::Combustion))
}

/// What a chassis of `power` drives on, in the inspector.
const fn drive_text(power: Power) -> &'static str {
    match power {
        Power::Electric => "ELECTRIC: BATTERIES",
        Power::Combustion => "COMBUSTION: ENGINE + FUEL",
    }
}

/// A resource's name in the inspector.
const fn resource_text(resource: Resource) -> &'static str {
    match resource {
        Resource::Electricity => "ENERGY",
        Resource::Fuel => "FUEL",
    }
}

/// The builder's supply line: the first thing `build`'s powertrain cannot supply
/// ([`shooter_sim::robot::Catalog::notices`]) and how many more there are, if any.
#[must_use]
pub fn supply_line(build: &Assembly) -> Option<String> {
    let notices = BETA.notices(build);
    let first = notices.first()?;
    let more = match notices.len() - 1 {
        0 => String::new(),
        more => format!(" (+{more} MORE)"),
    };
    Some(format!("SUPPLY: {first}{more}").to_uppercase())
}

/// `text` broken at spaces into lines at most `width` layout units wide in a font of `cell`.
fn wrap(text: &str, width: f32, cell: f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        match lines.last_mut() {
            Some(line) if text_width(&format!("{line} {word}"), cell) <= width => {
                line.push(' ');
                line.push_str(word);
            }
            _ => lines.push(word.to_owned()),
        }
    }
    lines
}

/// A panel's background: a rounded, translucent graphite card.
fn panel_back(painter: &mut Painter, rect: Rect) {
    painter.round_rect(
        rect.x,
        rect.y,
        rect.width,
        rect.height,
        10.0,
        srgb(GRAPHITE, 0.86),
    );
}

/// `build`'s wire bytes ([`shooter_sim::wire::encode_build`]) in lowercase hex.
fn draft_hex(build: &Assembly) -> String {
    use std::fmt::Write;
    shooter_sim::wire::encode_build(build)
        .iter()
        .fold(String::new(), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

/// A button: `strong` filled orange (Ready while it may be pressed).
fn button(painter: &mut Painter, rect: Rect, label: &str, hovered: bool, strong: bool) {
    let fill = match (strong, hovered) {
        (true, true) => srgb(ORANGE, 1.0),
        (true, false) => srgb(ORANGE, 0.85),
        (false, true) => srgb(STEEL, 1.0),
        (false, false) => srgb(STEEL, 0.6),
    };
    painter.round_rect(rect.x, rect.y, rect.width, rect.height, 6.0, fill);
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
pub fn death_prompt(respawn_in: f32, width: u32, height: u32) -> Vec<Item> {
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

    /// The builder names each chassis' powertrain and says what a build's powertrain cannot
    /// supply, while the build stays valid (shooter#130).
    #[test]
    fn the_builder_shows_the_powertrain_and_what_it_cannot_supply() {
        use shooter_sim::robot::STOKER;
        use shooter_sim::robot::parts::{COMBUSTION_M, ENGINE};
        assert_eq!(
            super::super::part_name(&COMBUSTION_M),
            "Fuel chassis 40x40x80"
        );
        assert_eq!(super::super::part_name(&CHASSIS_S), "Chassis 20x40x80");
        assert_eq!(super::super::part_name(&ENGINE), "Engine");
        assert_eq!(supply_line(&STANDARD), None);
        assert_eq!(supply_line(&STOKER), None);
        let mut candidate = STOKER.candidate();
        candidate
            .parts
            .retain(|part| part.id != 22 && part.id != 11);
        let hybrid = BETA.validate(&candidate).unwrap();
        assert_eq!(
            supply_line(&hybrid).as_deref(),
            Some("SUPPLY: DRIVE: COMBUSTION NEEDS AN ENGINE (+3 MORE)")
        );
        // The catalogue lists a chassis once a size and the engine; the power switch makes the
        // robot combustion (the root swapped in place, one undoable edit) and the chassis rows
        // give combustion chassis.
        let mut builder = drawn(Stage::Setup);
        let rows = builder.layout().catalogue_rows();
        let listed = |id| {
            rows.iter()
                .any(|(_, target)| *target == Target::Catalogue(id))
        };
        assert!(listed(ENGINE.id) && listed(CHASSIS_S.id) && !listed(COMBUSTION_M.id));
        assert!(rows.iter().any(|(_, target)| *target == Target::Power));
        assert_eq!(builder.bench.power(), Power::Electric);
        builder.act(Target::Power);
        assert_eq!(builder.bench.power(), Power::Combustion);
        let root = builder.bench.draft().parts()[0];
        assert_eq!(BETA.part_type(root.kind).unwrap().cells, CHASSIS_S.cells);
        assert_eq!(builder.bench.draft().parts().len(), STANDARD.parts().len());
        assert_eq!(
            supply_line(builder.bench.draft()).as_deref(),
            Some("SUPPLY: DRIVE: COMBUSTION NEEDS AN ENGINE (+4 MORE)")
        );
        builder.act(Target::Catalogue(CHASSIS_S.id));
        assert_ne!(
            builder.bench.hand,
            Some(CHASSIS_S.id),
            "a combustion chassis"
        );
        builder.bench.undo().unwrap();
        assert_eq!(*builder.bench.draft(), STANDARD);
        // The loadouts, two a row, end above the bar.
        let last = rows.last().unwrap().0;
        assert!(last.y + last.height <= builder.layout().bar.y - 8.0);
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
    fn the_workshop_goes_back_to_the_menu_or_keeps_its_build_with_done() {
        let mut builder = drawn(Stage::Workshop);
        let back = builder.layout().back();
        builder.cursor_moved(back.x + 5.0, back.y + 5.0);
        assert_eq!(builder.target(), Some(Target::Back));
        assert_eq!(builder.mouse(false, true), Some(Action::Back));
        // Escape lets go of what is in hand first, then goes back.
        builder.bench.hand = Some(BATTERY.id);
        assert_eq!(builder.key(KeyCode::Escape), None);
        assert_eq!(builder.key(KeyCode::Escape), Some(Action::Back));
        assert_eq!(
            builder.key(KeyCode::Enter),
            Some(Action::Ready(STANDARD, None))
        );
        // Neither before the match nor between lives has Back.
        let mut setup = drawn(Stage::Setup);
        assert!(!setup.layout().back);
        assert_eq!(setup.key(KeyCode::Escape), None);
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
        let at = builder.camera.screen(BENCH_AT).unwrap();
        builder.cursor_moved(at.x, at.y);
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
        assert_eq!(turned.orientation.yaw, crate::builder::COARSE);
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
            .targets(builder.side())
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

    fn typed(builder: &mut Builder, codes: &[KeyCode]) {
        for code in codes {
            builder.key(*code);
        }
    }

    #[test]
    fn a_saved_build_and_its_layout_come_back_after_a_change_in_either_stage() {
        for stage in [Stage::Setup, Stage::BetweenLives] {
            let store = std::rc::Rc::new(saved::MemoryBuilds::default());
            let mut builder = drawn(stage).with_store(store.clone());
            // An edited layout to save with the build.
            builder.key(KeyCode::KeyK);
            click(&mut builder, Target::Binding(0));
            click(&mut builder, Target::Edit(ControlEdit::Weaker));
            let layout = builder.controls.layout().clone();
            // B opens the panel; the name is typed (B and the builder's shortcuts type too).
            builder.key(KeyCode::KeyB);
            assert!(builder.builds_open && !builder.controls_open);
            typed(
                &mut builder,
                &[
                    KeyCode::KeyB,
                    KeyCode::KeyO,
                    KeyCode::KeyT,
                    KeyCode::KeyX,
                    KeyCode::Backspace,
                    KeyCode::Space,
                    KeyCode::Digit1,
                    KeyCode::Period,
                ],
            );
            assert_eq!(builder.shelf.name, "BOT_1");
            builder.key(KeyCode::Enter);
            assert_eq!(builder.notice.as_deref(), Some("saved BOT_1"));
            assert_eq!(store.names(), Ok(vec!["BOT_1".to_owned()]));
            assert_eq!(builder.bench.draft(), &STANDARD, "typing edits nothing");

            // Change the build and the layout.
            builder.key(KeyCode::Escape);
            assert!(!builder.builds_open);
            click(&mut builder, Target::Loadout(1));
            builder.frame(None, 1280, 720, 0.0);
            assert_eq!(builder.bench.draft(), &HEAVY);
            builder.controls.preset();
            assert_ne!(builder.controls.layout(), &layout);

            // Load restores both, as one edit.
            click(&mut builder, Target::Builds);
            builder.frame(None, 1280, 720, 0.0);
            click(&mut builder, Target::Saved(0));
            click(&mut builder, Target::Shelf(ShelfEdit::Load));
            assert_eq!(builder.notice.as_deref(), Some("loaded BOT_1"));
            assert_eq!(builder.bench.draft(), &STANDARD);
            assert_eq!(builder.controls.layout(), &layout);
            assert!(builder.controls.custom());
            builder.key(KeyCode::Escape);
            builder.key(KeyCode::KeyZ);
            assert_eq!(builder.bench.draft(), &HEAVY, "undo takes the load back");
            builder.key(KeyCode::KeyY);

            // Saving over it waits for a second press.
            click(&mut builder, Target::Builds);
            click(&mut builder, Target::Saved(0));
            builder.bench.load(HEAVY);
            click(&mut builder, Target::Shelf(ShelfEdit::Save));
            assert_eq!(
                builder.shelf.confirm,
                Some(Confirm::Overwrite("BOT_1".to_owned()))
            );
            click(&mut builder, Target::Shelf(ShelfEdit::Save));
            assert_eq!(builder.notice.as_deref(), Some("overwrote BOT_1"));

            // A damaged save is refused with its reason, and the bench keeps its build.
            store.write("BAD", "53484244ff00").unwrap();
            builder.shelf.refresh().unwrap();
            builder.shelf.name = "BAD".to_owned();
            builder.bench.load(STANDARD);
            click(&mut builder, Target::Shelf(ShelfEdit::Load));
            let notice = builder.notice.clone().unwrap();
            assert!(notice.starts_with("can't load:"), "{notice}");
            assert_eq!(builder.bench.draft(), &STANDARD);

            // Delete on a second press.
            builder.frame(None, 1280, 720, 0.0);
            click(&mut builder, Target::Saved(1));
            click(&mut builder, Target::Shelf(ShelfEdit::Delete));
            click(&mut builder, Target::Shelf(ShelfEdit::Delete));
            assert_eq!(builder.notice.as_deref(), Some("deleted BOT_1"));
            assert_eq!(store.names(), Ok(vec!["BAD".to_owned()]));
        }
    }
}

/// Snapping and turning on the bench (shooter#124): the socket under the pointer, the preview of
/// the turned part and its footprint, what is committed, refusals, undo and redo, cancelling and
/// selecting again, through the builder's own pointer and keys.
#[cfg(test)]
mod snapping {
    use super::*;
    use crate::builder::{COARSE, Step};
    use shooter_sim::robot::parts::{
        BATTERY, BEAM_LONG, CHASSIS_L, CHASSIS_M, CHASSIS_S, SENSOR, TRACK,
    };
    use shooter_sim::robot::{AssemblyError, Face, HEAVY, STANDARD};

    const SIZE: (u32, u32) = (1280, 720);

    fn draw(builder: &mut Builder) {
        builder.frame(None, SIZE.0, SIZE.1, 0.0);
    }

    /// A builder on `build` holding `kind`, seen from `azimuth` and `elevation`, drawn once.
    fn holding(build: Assembly, kind: PartTypeId, azimuth: f32, elevation: f32) -> Builder {
        let mut builder = Builder::new(build, Stage::Setup);
        builder.bench.hand = Some(kind);
        builder.look_from(azimuth, elevation);
        draw(&mut builder);
        builder
    }

    /// Where `world` is drawn, in layout units.
    fn screen(builder: &Builder, world: Vec3) -> Vec2 {
        builder.camera.screen(world).unwrap()
    }

    fn picked(builder: &Builder) -> Option<(PartId, SocketId)> {
        builder.pick().map(|spot| (spot.part, spot.socket))
    }

    /// Every socket the camera sees is picked wherever the pointer is on its cell (its centre
    /// and four points a quarter cell out, where the ray meets its own part first): the cell
    /// under the pointer, never a neighbour's. Returns how many sockets were checked.
    fn every_seen_socket_is_picked(builder: &mut Builder) -> usize {
        let spots: Vec<Spot> = builder
            .spots
            .iter()
            .filter(|spot| spot.visible)
            .copied()
            .collect();
        for spot in &spots {
            let across = if spot.normal.y.abs() < 0.9 {
                spot.normal.cross(Vec3::Y)
            } else {
                spot.normal.cross(Vec3::X)
            }
            .normalize();
            let up = spot.normal.cross(across);
            let quarter = HALF_CELL / 2.0;
            for offset in [
                Vec3::ZERO,
                across * quarter,
                -across * quarter,
                up * quarter,
                -up * quarter,
            ] {
                // Only where its own part is first along the ray, by a clear margin (a millimetre
                // beside another part's edge is that part's).
                let point = spot.world + offset;
                let direction = (point - builder.eye).normalize();
                let own = (point - builder.eye).length();
                if builder.parts.iter().any(|part| {
                    part.part != spot.part
                        && part
                            .ray(builder.eye, direction)
                            .is_some_and(|distance| distance < own + 0.005)
                }) {
                    continue;
                }
                builder.put_pointer(screen(builder, point));
                assert_eq!(
                    picked(builder),
                    Some((spot.part, spot.socket)),
                    "{spot:?} at {offset:?}"
                );
            }
        }
        spots.len()
    }

    #[test]
    fn crowded_faces_and_cells_beside_footprints_pick_the_cell_under_the_pointer() {
        // The standard build's front holds four weapons; its free cells sit beside them.
        for (build, azimuth, elevation) in [
            (STANDARD, 0.0, 0.0),
            (STANDARD, 0.55, 0.3),
            (STANDARD, 2.6, 0.35),
            (STANDARD, -1.4, -0.15),
            (HEAVY, 0.3, 0.2),
            (HEAVY, 3.6, 0.5),
        ] {
            let mut builder = holding(build, BATTERY.id, azimuth, elevation);
            let seen = every_seen_socket_is_picked(&mut builder);
            assert!(seen >= 4, "{azimuth} {elevation}: {seen}");
        }
        // A socket a weapon covers is hidden and never picked: the rifle holds the front's (1, 2).
        let builder = holding(STANDARD, BATTERY.id, 0.0, 0.0);
        let covered = CHASSIS_S.socket(Face::PosX, 1, 2).unwrap();
        let spot = builder.spot(PartId(0), covered).unwrap();
        assert!(spot.facing && !spot.visible);
    }

    #[test]
    fn large_parts_and_turned_parents_pick_the_cell_under_the_pointer() {
        // A large chassis with a long beam turned an eighth and a few degrees on its side, a
        // medium chassis on top turned an eighth, and the sockets on both turned parts.
        let mut bench = Workbench::new(STANDARD);
        bench.new_frame(CHASSIS_L.id).unwrap();
        bench.hand = Some(BEAM_LONG.id);
        bench.turn = Orientation::canonical(COARSE / 2, 3_000, 1_000);
        let beam = bench
            .place(PartId(0), CHASSIS_L.socket(Face::PosY, 1, 1).unwrap())
            .unwrap();
        bench.hand = Some(CHASSIS_M.id);
        bench.turn = Orientation::canonical(COARSE / 2, 0, 0);
        let top = bench
            .place(PartId(0), CHASSIS_L.socket(Face::PosZ, 0, 0).unwrap())
            .unwrap();
        let build = *bench.draft();
        let mut checked = 0;
        let mut seen = [0; 2];
        for azimuth in [0.4, 1.2, 2.0, 2.8, 3.8, 5.0] {
            let mut builder = holding(build, SENSOR.id, azimuth, 0.45);
            checked += every_seen_socket_is_picked(&mut builder);
            for (count, turned) in seen.iter_mut().zip([beam, top]) {
                *count += builder
                    .spots
                    .iter()
                    .filter(|spot| spot.part == turned && spot.visible)
                    .count();
            }
        }
        assert!(checked > 60, "{checked}");
        assert!(seen.iter().all(|count| *count > 6), "{seen:?}");
        // A large part in hand: a track's footprint is its four cells, as the rules count them.
        let mut builder = holding(STANDARD, TRACK.id, 1.4, 0.1);
        let side = CHASSIS_S.socket(Face::NegY, 0, 0).unwrap();
        assert!(builder.point_at(PartId(0), side));
        draw(&mut builder);
        let proposal = builder.proposal().unwrap();
        assert_eq!(proposal.socket(), side);
        assert_eq!(
            proposal.footprint,
            BENCH.footprint(
                &STANDARD,
                PartId(0),
                side,
                TRACK.id,
                None,
                Orientation::IDENTITY
            )
        );
        assert_eq!(proposal.footprint.len(), 4);
    }

    /// The battery in hand over the back's free cell (0, 1) of the standard build.
    fn over_the_back() -> (Builder, SocketId) {
        let mut builder = holding(STANDARD, BATTERY.id, std::f32::consts::PI, 0.1);
        let back = CHASSIS_S.socket(Face::NegX, 0, 1).unwrap();
        assert!(builder.point_at(PartId(0), back));
        draw(&mut builder);
        (builder, back)
    }

    #[test]
    fn each_axis_turns_the_preview_and_the_commit_is_the_preview() {
        let (mut builder, back) = over_the_back();
        let mut expected = Orientation::IDENTITY;
        for (key, axis) in [
            (KeyCode::KeyR, Axis::Yaw),
            (KeyCode::KeyT, Axis::Pitch),
            (KeyCode::KeyF, Axis::Roll),
        ] {
            let before = builder.proposal().unwrap().part.mount.unwrap().orientation;
            builder.key(key);
            expected = crate::builder::turned(expected, axis, i32::from(COARSE));
            draw(&mut builder);
            let proposal = builder.proposal().unwrap();
            let after = proposal.part.mount.unwrap().orientation;
            assert_ne!(after, before, "{axis:?} turns the part");
            assert_eq!(after, expected);
            assert!(proposal.verdict.is_ok(), "{axis:?}: {:?}", proposal.verdict);
        }
        // Shift turns back; G steps to an eighth, then a degree; Ctrl is a degree.
        builder.modifiers(true, false);
        builder.key(KeyCode::KeyF);
        builder.modifiers(false, false);
        expected = crate::builder::turned(expected, Axis::Roll, -i32::from(COARSE));
        builder.key(KeyCode::KeyG);
        assert_eq!(builder.step, Step::Eighth);
        builder.key(KeyCode::KeyR);
        expected = crate::builder::turned(expected, Axis::Yaw, i32::from(COARSE / 2));
        builder.modifiers(false, true);
        builder.key(KeyCode::KeyT);
        builder.modifiers(false, false);
        expected = crate::builder::turned(expected, Axis::Pitch, i32::from(FINE));
        assert_eq!(builder.bench.turn, expected);
        draw(&mut builder);
        let proposal = builder.proposal().unwrap().clone();
        assert_eq!(proposal.socket(), back);
        assert_eq!(proposal.part.mount.unwrap().orientation, expected);
        // What the preview drew is what is committed: the same build, the same part, the same
        // placement, and the simulation takes it.
        let before = *builder.bench.draft();
        let previewed = placement_of(&before, &proposal.part).unwrap();
        builder.mouse(false, true);
        builder.mouse(false, false);
        assert_eq!(builder.notice, None);
        let draft = *builder.bench.draft();
        assert_eq!(Ok(draft), proposal.verdict);
        assert_eq!(draft.part(proposal.part.id), Some(&proposal.part));
        let committed = placements(&draft)
            .into_iter()
            .find(|placement| placement.part == proposal.part.id)
            .unwrap();
        assert_eq!(committed, previewed);
        assert_eq!(BENCH.validate(&draft.candidate()), Ok(draft));
        assert_eq!(builder.bench.check(), Ok(draft));
    }

    #[test]
    fn a_blocked_footprint_is_refused_with_its_reason_and_nothing_changes() {
        // The battery is two cells long: on the front's free (1, 1) it would reach the rifle's
        // (1, 2).
        let mut builder = holding(STANDARD, BATTERY.id, 0.0, 0.0);
        let below = CHASSIS_S.socket(Face::PosX, 1, 1).unwrap();
        let rifle_cell = CHASSIS_S.socket(Face::PosX, 1, 2).unwrap();
        assert!(builder.point_at(PartId(0), below));
        draw(&mut builder);
        let proposal = builder.proposal().unwrap().clone();
        assert_eq!(proposal.socket(), below);
        assert!(
            matches!(
                proposal.verdict,
                Err(AssemblyError::Overlap(first, second)) if [first, second].contains(&PartId(7))
            ),
            "{:?}",
            proposal.verdict
        );
        assert_eq!(proposal.blocked, [(rifle_cell, PartId(7))]);
        builder.mouse(false, true);
        assert_eq!(*builder.bench.draft(), STANDARD);
        assert_eq!(builder.bench.hand, Some(BATTERY.id), "still in hand");
        let notice = builder.notice.clone().unwrap();
        assert!(notice.to_lowercase().contains("energy rifle"), "{notice}");
        assert!(notice.contains("turn it"), "{notice}");
        assert_eq!(builder.bench.history(), (false, false));
        // Turned a half turn, it reaches down to the free (1, 0) instead: placed.
        builder.key(KeyCode::KeyR);
        builder.key(KeyCode::KeyR);
        draw(&mut builder);
        assert!(builder.proposal().unwrap().verdict.is_ok());
        builder.mouse(false, true);
        assert_eq!(builder.notice, None);
        assert_eq!(
            builder.bench.draft().parts().len(),
            STANDARD.parts().len() + 1
        );
    }

    #[test]
    fn undo_and_redo_restore_the_exact_builds() {
        let (mut builder, _) = over_the_back();
        let mut builds = vec![*builder.bench.draft()];
        // Place, drop the hand, turn the new part, give the rifle a mod, remove the sensor,
        // load a loadout.
        builder.mouse(false, true);
        builds.push(*builder.bench.draft());
        let placed = builder.bench.selected.unwrap();
        builder.key(KeyCode::Escape);
        builder.bench.selected = Some(placed);
        builder.key(KeyCode::KeyT);
        builds.push(*builder.bench.draft());
        builder.bench.selected = Some(PartId(7));
        builder.key(KeyCode::KeyM);
        builds.push(*builder.bench.draft());
        builder.bench.selected = Some(PartId(2));
        builder.key(KeyCode::Delete);
        builds.push(*builder.bench.draft());
        builder.act(Target::Loadout(1));
        builds.push(*builder.bench.draft());
        for pair in builds.windows(2) {
            assert_ne!(pair[0], pair[1], "every step is an edit");
        }
        for build in builds.iter().rev().skip(1) {
            builder.key(KeyCode::KeyZ);
            assert_eq!(builder.bench.draft(), build);
        }
        builder.key(KeyCode::KeyZ);
        assert_eq!(builder.notice.as_deref(), Some("nothing to undo"));
        for build in builds.iter().skip(1) {
            builder.key(KeyCode::KeyY);
            assert_eq!(builder.bench.draft(), build);
        }
        // Shift+Z redoes too; a new edit after an undo drops what was undone.
        builder.key(KeyCode::KeyZ);
        builder.modifiers(true, false);
        builder.key(KeyCode::KeyZ);
        builder.modifiers(false, false);
        assert_eq!(builder.bench.draft(), builds.last().unwrap());
        builder.key(KeyCode::KeyZ);
        builder.key(KeyCode::KeyZ);
        builder.bench.selected = Some(PartId(7));
        builder.key(KeyCode::KeyM);
        assert_eq!(builder.bench.history(), (true, false));
        // Every build undo and redo restored is one the simulation validates to itself.
        for build in &builds {
            assert_eq!(BENCH.validate(&build.candidate()).as_ref(), Ok(build));
        }
    }

    #[test]
    fn escape_cancels_a_drag_and_a_refused_drop_keeps_the_part() {
        let mut builder = Builder::new(STANDARD, Stage::Setup);
        builder.look_from(std::f32::consts::PI, 0.1);
        draw(&mut builder);
        let (row, _) = builder
            .layout()
            .catalogue_rows()
            .into_iter()
            .find(|(_, target)| *target == Target::Catalogue(BATTERY.id))
            .unwrap();
        // Drag, turn it, Escape: dropped on a socket, nothing is placed.
        builder.put_pointer(Vec2::new(row.x + 20.0, row.y + 5.0));
        builder.mouse(false, true);
        builder.key(KeyCode::KeyR);
        draw(&mut builder);
        let back = CHASSIS_S.socket(Face::NegX, 0, 1).unwrap();
        assert!(builder.point_at(PartId(0), back));
        draw(&mut builder);
        assert!(builder.proposal().is_some());
        builder.key(KeyCode::Escape);
        builder.mouse(false, false);
        assert_eq!(*builder.bench.draft(), STANDARD);
        assert_eq!(builder.bench.hand, None);
        draw(&mut builder);
        assert!(builder.proposal().is_none(), "no preview after cancelling");
        // A right click drops what is in hand too.
        builder.bench.hand = Some(BATTERY.id);
        builder.mouse(true, true);
        builder.mouse(true, false);
        assert_eq!(builder.bench.hand, None);
        // A drop the rules refuse keeps the part in hand, with the reason: a battery on the
        // back's (0, 0) holds (0, 1), which a second battery turned a quarter on (1, 1) reaches.
        builder.bench.hand = Some(BATTERY.id);
        builder.bench.turn = Orientation::IDENTITY;
        builder
            .bench
            .place(PartId(0), CHASSIS_S.socket(Face::NegX, 0, 0).unwrap())
            .unwrap();
        builder.bench.selected = None;
        builder.bench.turn = Orientation::canonical(COARSE, 0, 0);
        let before = *builder.bench.draft();
        builder.dragging = true;
        draw(&mut builder);
        let beside = CHASSIS_S.socket(Face::NegX, 1, 1).unwrap();
        assert!(builder.point_at(PartId(0), beside));
        draw(&mut builder);
        builder.mouse(false, false);
        assert_eq!(*builder.bench.draft(), before);
        assert_eq!(builder.bench.hand, Some(BATTERY.id));
        assert!(
            builder
                .notice
                .as_deref()
                .is_some_and(|notice| notice.contains("the footprint meets the Battery")),
            "{:?}",
            builder.notice
        );
    }

    #[test]
    fn parts_are_selected_again_and_edited_where_they_are() {
        let (mut builder, _) = over_the_back();
        builder.mouse(false, true);
        let battery = builder.bench.selected.unwrap();
        // With the battery still in hand the keys turn the next copy, not the placed one.
        builder.key(KeyCode::KeyR);
        assert_eq!(builder.bench.turn.yaw, COARSE);
        let placed = |builder: &Builder, id| {
            builder
                .bench
                .draft()
                .part(id)
                .unwrap()
                .mount
                .unwrap()
                .orientation
        };
        assert_eq!(placed(&builder, battery), Orientation::IDENTITY);
        builder.key(KeyCode::Escape);
        // Click the sensor on the top: selected; turn it.
        builder.look_from(std::f32::consts::PI, 0.7);
        draw(&mut builder);
        let sensor = builder
            .parts
            .iter()
            .find(|part| part.part == PartId(2))
            .unwrap()
            .centre;
        builder.put_pointer(screen(&builder, sensor));
        builder.mouse(false, true);
        assert_eq!(builder.bench.selected, Some(PartId(2)));
        builder.key(KeyCode::KeyF);
        assert_eq!(placed(&builder, PartId(2)).roll, COARSE);
        // Back to the battery: selected again, turned on its own socket.
        let at = builder
            .parts
            .iter()
            .find(|part| part.part == battery)
            .unwrap()
            .centre;
        builder.put_pointer(screen(&builder, at));
        builder.mouse(false, true);
        assert_eq!(builder.bench.selected, Some(battery));
        builder.key(KeyCode::KeyR);
        assert_eq!(placed(&builder, battery).yaw, COARSE);
        assert_eq!(placed(&builder, PartId(2)).roll, COARSE, "the sensor stays");
        // Undo takes back the battery's turn, then the sensor's.
        builder.key(KeyCode::KeyZ);
        assert_eq!(placed(&builder, battery), Orientation::IDENTITY);
        builder.key(KeyCode::KeyZ);
        assert_eq!(placed(&builder, PartId(2)), Orientation::IDENTITY);
    }

    #[test]
    fn the_pointer_maps_from_the_window_onto_the_drawn_surface() {
        // The browser's canvas at a device pixel ratio of 2 drawn on a surface the GPU limits to
        // half its size, and a window smaller than its surface: the same pointer in the window's
        // pixels picks the same socket.
        let (builder, back) = over_the_back();
        let at = builder.spot(PartId(0), back).unwrap().at * builder.scale();
        for window in [(2560, 1440), (640, 360), (1280, 720)] {
            let mut builder = builder.clone();
            builder.resize(window.0, window.1);
            #[allow(clippy::cast_precision_loss)]
            let factor = window.0 as f32 / SIZE.0 as f32;
            builder.cursor_moved(at.x * factor, at.y * factor);
            assert_eq!(picked(&builder), Some((PartId(0), back)), "{window:?}");
        }
        // Without the window's size the pointer is the surface's.
        let mut builder = builder;
        builder.cursor_moved(at.x, at.y);
        assert_eq!(picked(&builder), Some((PartId(0), back)));
    }

    /// While the controls panel waits for a binding's key, the builder's shortcuts (turning,
    /// the step, undo and redo) do not fire: the key binds.
    #[test]
    fn a_key_the_controls_panel_captures_is_no_shortcut() {
        let (mut builder, _) = over_the_back();
        builder.mouse(false, true);
        let placed = *builder.bench.draft();
        builder.key(KeyCode::KeyK);
        assert!(builder.controls_open);
        builder.controls.selected = Some(0);
        builder.controls.capturing = true;
        builder.key(KeyCode::KeyR);
        assert_eq!(
            builder.bench.turn,
            Orientation::IDENTITY,
            "R bound, not turned"
        );
        for key in [KeyCode::KeyG, KeyCode::KeyZ, KeyCode::KeyY, KeyCode::KeyT] {
            builder.controls.capturing = true;
            builder.key(key);
        }
        assert_eq!(builder.step, Step::Quarter);
        assert_eq!(*builder.bench.draft(), placed, "nothing undone");
        assert_eq!(builder.bench.turn, Orientation::IDENTITY);
        // Not capturing, the same keys are shortcuts again.
        builder.key(KeyCode::KeyZ);
        assert_eq!(*builder.bench.draft(), STANDARD);
    }

    /// The readout a page publishes for scripted checks says what the builder shows: the build's
    /// exact bytes, the part in hand and the placement under the pointer, and where a socket is
    /// in the window's pixels (moving the pointer there picks it).
    #[test]
    fn the_readout_says_what_the_builder_shows_and_where() {
        let (mut builder, back) = over_the_back();
        builder.resize(2560, 1440);
        let readout: serde_json::Value = serde_json::from_str(&builder.readout()).unwrap();
        assert_eq!(readout["build"], draft_hex(&STANDARD));
        assert_eq!(readout["hand"], BATTERY.key);
        assert_eq!(readout["valid"], true);
        assert_eq!(readout["proposal"]["parent"], 0);
        assert_eq!(readout["proposal"]["socket"], back.0);
        assert!(readout["proposal"]["refused"].is_null());
        let spot = readout["sockets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|spot| spot["part"] == 0 && spot["socket"] == back.0)
            .expect("the back's socket is seen");
        assert_eq!(spot["fits"], true);
        #[allow(clippy::cast_possible_truncation)]
        let [x, y] = [0, 1].map(|axis| spot["at"][axis].as_f64().unwrap() as f32);
        builder.cursor_moved(x, y);
        assert_eq!(picked(&builder), Some((PartId(0), back)));
        assert!(
            readout["targets"]
                .as_array()
                .unwrap()
                .iter()
                .any(|target| target["name"] == "Ready")
        );
        // Placed there: one more part, and the history has the edit.
        draw(&mut builder);
        builder.mouse(false, true);
        draw(&mut builder);
        let readout: serde_json::Value = serde_json::from_str(&builder.readout()).unwrap();
        assert_eq!(readout["build"], draft_hex(builder.bench.draft()));
        assert_eq!(readout["parts"], STANDARD.parts().len() + 1);
        assert_eq!(readout["undo"], true);
    }
}
