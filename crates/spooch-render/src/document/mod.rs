//! An editable capture: the frozen pixels plus the annotations drawn on
//! them. Platform shells feed it input in desktop pixels and blit what it
//! renders; nothing here knows about windows or monitors.
//!
//! Everything lives in one coordinate space, the shell's desktop pixels.
//! The canvas is the part of the desktop being edited and exported; dragging
//! its edges crops it or extends it over more of the frozen desktop, without
//! moving any mark. The document starts with only the canvas's pixels and
//! asks the shell for the whole desktop the first time an edge is dragged,
//! so opening the editor stays cheap.

use std::fmt;

mod bar;
mod chrome;
mod edit;
mod frame;
mod gallery;
mod grips;
mod guides;
mod keys;
mod mark;
mod pointer;
mod remembered;
mod render;
mod saved;
mod settings_page;
mod style;
#[cfg(test)]
mod tests;
mod text;
mod theme;
mod view;
pub use bar::Action;
pub use chrome::Panel;
pub(crate) use chrome::panel;
pub use frame::{Exported, FRAME_COLOURS, Frame, SHADOWS, load_frames};
pub use gallery::{Gallery, GalleryAction, SyncBadge, thumbnail};
pub use keys::{Key, KeyOutcome, Mods, SHARE_SHORTCUT};
pub use saved::LoadError;
pub use settings_page::{SettingsEvent, SettingsItem, SettingsPage};
pub use style::{PALETTE, PALETTES, Palette, Style, Styles};
pub(crate) use theme::{SELECTION, fill, hex, ink_baseline, stroke, with_alpha};
pub use view::{WindowButton, WindowPart};

use grips::{
    Grip, LABEL_PAD, edge_cursor, grips_of, label_box, loupe_point, reshape, resized, rotate_about,
    rotate_stick,
};
use mark::{Base, Mark, Redaction, contains};
use pointer::{Gesture, HoverMark};

use skia_safe::{
    Canvas, Color4f, Data, IRect, Image, Paint, PaintCap, PaintStyle, Point, Rect, Surface,
    Typeface, surfaces,
};

use crate::{
    Arrow, Boxed, Highlight, Label, Line, Oval, Step, Stroke, blur_region, pixelate_region,
};

/// A sticker stamped with a click is this big, in screen pixels; a drag
/// shorter than `STICKER_DRAG_PX` counts as a click.
const STICKER_PX: f32 = 96.0;
const STICKER_DRAG_PX: f32 = 16.0;
/// Arrows shorter than this on release are treated as stray clicks.
const MIN_ARROW_PX: f32 = 6.0;
/// Boxes and highlights narrower or shorter than this are stray clicks too.
const MIN_BOX_PX: f32 = 4.0;
/// A box's rounding handle sits this far inside its corner (plus the
/// radius), shows on boxes at least `ROUND_MIN_SIDE` across, and snaps
/// square within `ROUND_SNAP` of its innermost-square spot.
const ROUND_INSET: f32 = 12.0;
const ROUND_MIN_SIDE: f32 = 48.0;
const ROUND_SNAP: f32 = 2.0;
/// How far down and right a duplicate lands, in screen pixels.
const DUPLICATE_OFFSET: f32 = 16.0;
/// How far above a mark its rotate handle sits, in screen pixels.
const ROTATE_STICK: f32 = 24.0;
/// A bend handle dragged within this many pixels of the straight line
/// snaps it straight.
const BEND_SNAP: f32 = 6.0;
/// How far from a mark a click still picks it, in screen pixels.
const HIT_SLOP: f32 = 6.0;
/// Half the side of a selection handle, in screen pixels.
const HANDLE_HALF: f32 = 4.0;
/// How far outside and inside the canvas edge a press grabs the edge, in
/// screen pixels. Mostly outside, so drawing from near the edge still draws.
const EDGE_OUT: f32 = 8.0;
const EDGE_IN: f32 = 4.0;
/// The pen keeps samples at least this far apart, in screen pixels.
const PEN_STEP: f32 = 2.0;
/// The canvas never gets narrower or shorter than this.
const MIN_CANVAS_PX: i32 = 8;

/// The file format for saving and dragging out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Format {
    #[default]
    Png,
    Jpeg,
}

impl Format {
    pub fn name(self) -> &'static str {
        match self {
            Format::Png => "PNG",
            Format::Jpeg => "JPEG",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Format::Png => "png",
            Format::Jpeg => "jpg",
        }
    }

    /// The MIME type, for drag-and-drop.
    pub fn mime(self) -> &'static str {
        match self {
            Format::Png => "image/png",
            Format::Jpeg => "image/jpeg",
        }
    }

    /// Every format, for reading one back by name.
    pub const ALL: [Format; 2] = [Format::Png, Format::Jpeg];

    pub fn next(self) -> Format {
        match self {
            Format::Png => Format::Jpeg,
            Format::Jpeg => Format::Png,
        }
    }
}

/// The pointer shape the shell should show, for what a press would do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cursor {
    /// Outside the canvas, where a press does nothing.
    Arrow,
    /// A press draws with the current tool.
    Draw,
    /// A press moves the selected mark.
    Move,
    /// A press drags a left or right edge.
    ResizeHorizontal,
    /// A press drags a top or bottom edge.
    ResizeVertical,
    /// A press drags the top-left or bottom-right corner.
    ResizeDiagonalDown,
    /// A press drags the top-right or bottom-left corner.
    ResizeDiagonalUp,
    /// A press places a text caret.
    Text,
    /// A press clicks a button.
    Hand,
}

/// Keys that move the caret or delete in a label being typed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TextKey {
    Left,
    Right,
    /// The line above or below, as near the same place across as it goes.
    Up,
    Down,
    /// The start and end of the caret's line.
    Home,
    End,
    Backspace,
    Delete,
}

/// What a drag on the canvas draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Arrow,
    /// Click to place a label. Typing anywhere starts one with any tool.
    Text,
    Box,
    Highlight,
    Blur,
    Pixelate,
    Line,
    Ellipse,
    Pen,
    /// Click to place numbered steps: 1, 2, 3...
    Steps,
    /// Click (or drag, to size it) to stamp the sticker picked on the
    /// style row.
    Sticker,
    /// A solid black box that really removes what is under it.
    Redact,
}

impl Tool {
    /// Every tool, in rail order.
    pub const ALL: [Tool; 12] = [
        Tool::Arrow,
        Tool::Text,
        Tool::Box,
        Tool::Highlight,
        Tool::Blur,
        Tool::Pixelate,
        Tool::Line,
        Tool::Ellipse,
        Tool::Pen,
        Tool::Steps,
        Tool::Sticker,
        Tool::Redact,
    ];

    /// The tools on digit keys 1 to 9, then 0. Text has none: typing
    /// letters starts a label anyway.
    pub const KEYED: [Tool; 10] = [
        Tool::Arrow,
        Tool::Box,
        Tool::Highlight,
        Tool::Blur,
        Tool::Pixelate,
        Tool::Line,
        Tool::Ellipse,
        Tool::Pen,
        Tool::Steps,
        Tool::Redact,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Tool::Arrow => "Arrow",
            Tool::Text => "Text",
            Tool::Box => "Box",
            Tool::Highlight => "Highlight",
            Tool::Blur => "Blur",
            Tool::Pixelate => "Pixelate",
            Tool::Line => "Line",
            Tool::Ellipse => "Ellipse",
            Tool::Pen => "Pen",
            Tool::Steps => "Steps",
            Tool::Sticker => "Sticker",
            Tool::Redact => "Redact",
        }
    }

    /// The tool on a digit key, if any. 0 is the tenth.
    pub fn from_digit(c: char) -> Option<Tool> {
        let n = match c.to_digit(10)? {
            0 => 10,
            n => n as usize,
        };
        Tool::KEYED.get(n - 1).copied()
    }

    /// The tool's digit key, if it has one.
    pub fn digit(self) -> Option<char> {
        let i = Tool::KEYED.iter().position(|t| *t == self)?;
        char::from_digit(((i + 1) % 10) as u32, 10)
    }
}

/// How many of each annotation a document holds, for logs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub arrows: usize,
    pub lines: usize,
    pub boxes: usize,
    pub ellipses: usize,
    pub highlights: usize,
    pub labels: usize,
    pub strokes: usize,
    pub steps: usize,
    pub stickers: usize,
    pub blurs: usize,
    pub pixelates: usize,
    pub redactions: usize,
}

impl fmt::Display for Counts {
    /// Only what is there, for example "2 arrows, 1 label"; "no marks"
    /// when empty.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parts: Vec<String> = [
            (self.arrows, "arrow", "arrows"),
            (self.lines, "line", "lines"),
            (self.boxes, "box", "boxes"),
            (self.ellipses, "ellipse", "ellipses"),
            (self.highlights, "highlight", "highlights"),
            (self.labels, "label", "labels"),
            (self.strokes, "pen stroke", "pen strokes"),
            (self.steps, "step", "steps"),
            (self.stickers, "sticker", "stickers"),
            (self.blurs, "blur", "blurs"),
            (self.pixelates, "pixelate", "pixelates"),
            (self.redactions, "redaction", "redactions"),
        ]
        .into_iter()
        .filter(|(n, ..)| *n > 0)
        .map(|(n, one, many)| format!("{n} {}", if n == 1 { one } else { many }))
        .collect();
        if parts.is_empty() {
            f.write_str("no marks")
        } else {
            f.write_str(&parts.join(", "))
        }
    }
}

/// The redactions a backdrop was built from.
type RedactionKey = Vec<(Rect, Redaction)>;

/// Everything undo restores.
#[derive(Clone, PartialEq)]
struct State {
    marks: Vec<Mark>,
    canvas: IRect,
}

pub struct Document {
    base: Base,
    /// The capture with every redaction applied, and the redactions it was
    /// built from, positioned like `base`. Rebuilt only when those change,
    /// since blurring is the one costly thing to draw.
    backdrop: Option<(RedactionKey, Image)>,
    marks: Vec<Mark>,
    /// The part of the desktop being edited, which is what gets exported.
    /// Always inside `base`.
    canvas: IRect,
    /// The whole desktop, which the canvas can grow to once the shell has
    /// supplied its pixels.
    desktop: IRect,
    /// Set when an edge drag starts and `base` is not yet the whole desktop.
    want_desktop: bool,
    /// A nudge past `base`, waiting for the desktop's pixels.
    pending_canvas: Option<IRect>,
    /// Whether the last key was an arrow nudge, so a run of them is one
    /// undo step.
    nudged: bool,
    /// Where the pointer was last seen, for which edge arrows nudge.
    pointer: Point,
    /// How the capture is shown in the editor window; `None` in the
    /// overlay.
    view: Option<view::View>,
    /// The editor window as last drawn, and the capture shrunk for it.
    window_surface: Option<Surface>,
    window_image: Option<Image>,
    /// Whether `window_image` has a frame or shadow round the capture.
    window_framed: bool,
    /// The frame, colour, shadow and title `window_image` was drawn with.
    window_frame_key: Option<(Frame, u8, u8, String)>,
    /// Whether the pointer is on a canvas edge, which shows the crop
    /// handles in the editor window.
    edge_hover: bool,
    /// Whether the editor window is maximised.
    window_maximized: bool,
    /// What lines up with the mark being dragged, to draw.
    guides: Vec<guides::Guide>,
    /// Whether Alt is held, which turns snapping off while dragging.
    alt: bool,
    /// The desktop pixel the loupe looks at, while an edge or the whole
    /// canvas is dragged or just after an edge nudge.
    loupe_at: Option<(i32, i32)>,
    /// Earlier states, most recent last. Every committed edit pushes one,
    /// so undo covers moves, deletes and crops as well as new marks.
    undo: Vec<State>,
    /// States undone since the last edit, most recently undone last.
    redo: Vec<State>,
    /// Index into `marks` of the mark showing handles.
    selected: Option<usize>,
    /// More selected marks, added with Shift+click. `selected` is the one
    /// the style row and handles follow.
    also: Vec<usize>,
    /// Index into `marks` of the mark just drawn or typed. Until the next
    /// press on the canvas, the style row shows it and restyles it, as if
    /// it were selected but without handles in the way of the next drag.
    last_drawn: Option<usize>,
    tool: Tool,
    format: Format,
    /// What the bar's Share button says while a share is under way or
    /// after it; "Share" when `None`.
    share_label: Option<String>,
    /// What the Save button says instead of "Save": "Show" once saved.
    save_label: Option<String>,
    gesture: Option<Gesture>,
    /// The label being typed.
    typing: Option<Label>,
    /// The caret in `typing`, and the other end of any selected text.
    caret: crate::Caret,
    /// When `typing` is an existing label being edited: where it sat in
    /// `marks`, and how it was, for undo or to put it back unchanged.
    editing: Option<(usize, Label)>,
    /// `None` on a system without fonts; labels are then not drawn.
    typeface: Option<Typeface>,
    /// The chrome's text: the bar, tooltips and the top row.
    ui_typeface: Option<Typeface>,
    /// Canvas-sized; recreated when the canvas changes size.
    surface: Surface,
    /// Whether `surface` matches the current state.
    fresh: bool,
    /// Everything picked in the chrome that carries on to the next capture.
    styles: Styles,
    /// The file name without extension, shown in the output bar.
    name: String,
    /// What a click on the output bar asked the shell to do.
    action: Option<Action>,
    /// Image pixels per screen pixel of editor chrome (handles, hit slop).
    scale: f32,
    /// The monitors, for keeping chrome on the canvas's screen.
    screens: Vec<IRect>,
    /// The chrome button under the pointer, for its highlight and tooltip.
    hovered: Option<chrome::Hit>,
    /// What a press on the canvas would grab, outlined before the press.
    hover_mark: Option<HoverMark>,
    /// Whether Shift was down at the last press, as the shell said.
    shift: bool,
    /// Whether Space is held while drawing, moving the shape instead of
    /// stretching it.
    space: bool,
    /// Wheel notches not yet turned into a size step (touchpads send
    /// fractions).
    wheel_rest: f32,
    /// Whether the font dropdown under a label is open.
    font_menu: bool,
    /// Whether the grid of every sticker is open under the style row.
    sticker_menu: bool,
    /// Whether Alt is held, showing the framed result instead of the
    /// editor's chrome.
    preview: bool,
    /// The window frames' title, and whether it's being typed.
    frame_title: String,
    editing_title: bool,
    /// The bar's open list (frames or frame colours), if one is.
    bar_menu: Option<bar::BarMenu>,
    /// The chrome as last drawn, and what it was drawn from.
    chrome_key: Option<chrome::ChromeKey>,
    chrome_panels: Vec<Panel>,
    /// The editor window's history gallery.
    gallery: Gallery,
}

/// Wrap BGRA pixels as an opaque image.
fn opaque_image(width: u32, height: u32, stride: usize, data: &[u8]) -> Option<Image> {
    let info = crate::bgra_info((width as i32, height as i32), skia_safe::AlphaType::Opaque);
    // Skia trusts "opaque" and copies the alpha byte as is, but X11 and
    // some Windows paths leave it at 0, which breaks blend modes like the
    // highlighter's multiply. Make it really opaque.
    let mut pixels = data.to_vec();
    let row_bytes = width as usize * 4;
    for row in pixels.chunks_mut(stride) {
        let used = row_bytes.min(row.len());
        for px in row[..used].as_chunks_mut::<4>().0 {
            px[3] = 255;
        }
    }
    skia_safe::images::raster_from_data(&info, Data::new_copy(&pixels), stride)
}

pub(super) fn canvas_surface(size: (i32, i32)) -> Option<Surface> {
    surfaces::raster(
        &crate::bgra_info(size, skia_safe::AlphaType::Premul),
        None,
        None,
    )
}

impl Document {
    /// Wrap tightly or loosely packed BGRA pixels (alpha ignored, treated as
    /// opaque) as a document whose canvas is the whole image, at the desktop
    /// origin. `scale` is the display scale, so arrows keep the same visual
    /// weight on high-DPI screens.
    pub fn from_bgra(
        width: u32,
        height: u32,
        stride: usize,
        data: &[u8],
        scale: f32,
    ) -> Option<Self> {
        let image = opaque_image(width, height, stride, data)?;
        let surface = canvas_surface((width as i32, height as i32))?;
        let rect = IRect::from_wh(width as i32, height as i32);
        let scale = scale.max(1.0);
        Some(Self {
            base: Base {
                image,
                at: Point::new(0.0, 0.0),
            },
            backdrop: None,
            marks: Vec::new(),
            canvas: rect,
            desktop: rect,
            want_desktop: false,
            pending_canvas: None,
            nudged: false,
            pointer: Point::new(width as f32, height as f32),
            loupe_at: None,
            view: None,
            window_surface: None,
            window_image: None,
            window_framed: false,
            window_frame_key: None,
            edge_hover: false,
            window_maximized: false,
            guides: Vec::new(),
            alt: false,
            undo: Vec::new(),
            redo: Vec::new(),
            selected: None,
            also: Vec::new(),
            last_drawn: None,
            tool: Tool::Arrow,
            format: Format::Png,
            share_label: None,
            save_label: None,
            gesture: None,
            typing: None,
            caret: crate::Caret::default(),
            editing: None,
            hover_mark: None,
            shift: false,
            space: false,
            wheel_rest: 0.0,
            font_menu: false,
            sticker_menu: false,
            preview: false,
            frame_title: frame::last_title(),
            editing_title: false,
            bar_menu: None,
            typeface: Label::typeface(),
            ui_typeface: Label::ui_typeface(),
            surface,
            fresh: false,
            styles: Styles::default(),
            name: String::from("Spooch"),
            action: None,
            scale,
            screens: Vec::new(),
            hovered: None,
            chrome_key: None,
            chrome_panels: Vec::new(),
            gallery: Gallery::default(),
        })
    }

    /// Put the pixels at `(x, y)` on a desktop covering `desktop`. Call
    /// right after `from_bgra`, before any input; from then on every point
    /// is in desktop pixels.
    pub fn place(&mut self, x: i32, y: i32, desktop: IRect) {
        self.base.at = Point::new(x as f32, y as f32);
        self.canvas = IRect::from_xywh(x, y, self.base.image.width(), self.base.image.height());
        self.desktop = desktop;
        // Until the shell says otherwise, the pointer is where a drag
        // from the top left let go.
        self.pointer = Point::new(self.canvas.right as f32, self.canvas.bottom as f32);
        self.backdrop = None;
        self.fresh = false;
    }

    /// The desktop area whose pixels the document wants, once an edge drag
    /// has started and it only holds part of the desktop. Hand them over
    /// with `supply_desktop`; until then the canvas can only shrink.
    pub fn wants_desktop(&self) -> Option<IRect> {
        self.want_desktop.then_some(self.desktop)
    }

    /// The whole desktop's pixels, as asked for by `wants_desktop`.
    /// Returns whether they were taken.
    pub fn supply_desktop(&mut self, width: u32, height: u32, stride: usize, data: &[u8]) -> bool {
        self.want_desktop = false;
        if width as i32 != self.desktop.width() || height as i32 != self.desktop.height() {
            return false;
        }
        let Some(image) = opaque_image(width, height, stride, data) else {
            return false;
        };
        self.base = Base {
            image,
            at: Point::new(self.desktop.left as f32, self.desktop.top as f32),
        };
        if let Some(c) = self.pending_canvas.take() {
            self.canvas = c;
        }
        self.backdrop = None;
        self.fresh = false;
        true
    }

    /// Save for undo before the first of a run of nudges, so the run undoes
    /// as one step.
    fn nudge_checkpoint(&mut self) {
        if !self.nudged {
            self.checkpoint();
        }
        self.nudged = true;
    }

    /// Move the selected marks by `(dx, dy)`, or with nothing selected the
    /// canvas edge on the pointer's side: the left or right edge for a
    /// sideways nudge, the top or bottom for an up or down one. A run of
    /// nudges is one undo step. Returns whether anything moved.
    pub(super) fn nudge(&mut self, (dx, dy): (i32, i32)) -> bool {
        if self.gesture.is_some() {
            return false;
        }
        let d = Point::new(dx as f32, dy as f32);
        let selection = self.selection();
        if !selection.is_empty() {
            self.nudge_checkpoint();
            for i in selection {
                self.marks[i].offset(d);
            }
            self.fresh = false;
            return true;
        }
        let c = self.canvas;
        let side = |v: f32, lo: i32, hi: i32| if (v as i32) < (lo + hi) / 2 { 0 } else { 2 };
        let x = if dx == 0 {
            1
        } else {
            side(self.pointer.x, c.left, c.right)
        };
        let y = if dy == 0 {
            1
        } else {
            side(self.pointer.y, c.top, c.bottom)
        };
        let Some(to) = resized(c, Grip::Edge { x, y }, d, self.desktop) else {
            return false;
        };
        if to == c {
            return false;
        }
        self.nudge_checkpoint();
        self.loupe_at = Some(loupe_point(to, Grip::Edge { x, y }, self.pointer));
        if IRect::intersect(&self.base_rect(), &to) == Some(to) {
            self.canvas = to;
            self.fresh = false;
        } else {
            // Past the pixels held: the shell hands over the desktop next,
            // and the nudge lands then.
            self.want_desktop = true;
            self.pending_canvas = Some(to);
        }
        true
    }

    /// Where the base pixels sit on the desktop.
    fn base_rect(&self) -> IRect {
        IRect::from_xywh(
            self.base.at.x as i32,
            self.base.at.y as i32,
            self.base.image.width(),
            self.base.image.height(),
        )
    }

    /// The part of the desktop being edited, in desktop pixels.
    pub fn canvas(&self) -> IRect {
        self.canvas
    }

    /// Width of the canvas, which is the exported image's width.
    pub fn width(&self) -> u32 {
        self.canvas.width() as u32
    }

    pub fn height(&self) -> u32 {
        self.canvas.height() as u32
    }

    pub fn format(&self) -> Format {
        self.format
    }

    pub fn set_format(&mut self, format: Format) {
        self.format = format;
    }

    pub fn counts(&self) -> Counts {
        let mut c = Counts::default();
        for m in &self.marks {
            match m {
                Mark::Arrow(_) => c.arrows += 1,
                Mark::Line(_) => c.lines += 1,
                Mark::Box(_) => c.boxes += 1,
                Mark::Oval(_) => c.ellipses += 1,
                Mark::Highlight(_) => c.highlights += 1,
                Mark::Stroke(_) => c.strokes += 1,
                Mark::Step(_) => c.steps += 1,
                Mark::Sticker(_) => c.stickers += 1,
                Mark::Redact(_, Redaction::Blur(_)) => c.blurs += 1,
                Mark::Redact(_, Redaction::Pixelate(_)) => c.pixelates += 1,
                Mark::Redact(_, Redaction::Solid) => c.redactions += 1,
                Mark::Label(_) => c.labels += 1,
            }
        }
        // A label still being typed is kept by `export`, so count it too.
        if self
            .typing
            .as_ref()
            .is_some_and(|l| !l.text.trim().is_empty())
        {
            c.labels += 1;
        }
        c
    }

    pub fn tool(&self) -> Tool {
        self.tool
    }

    /// Shift and Alt as they are now, during a drag too: Shift keeps a
    /// move straight (and a corner's proportions), Alt places freely,
    /// without snapping. Shift+drag on the move tab carries the marks
    /// along.
    pub fn set_modifiers(&mut self, shift: bool, alt: bool) {
        self.shift = shift;
        self.alt = alt;
    }

    /// The marks that touch the canvas, as they are, to carry along.
    fn marks_on_canvas(&self) -> Vec<(usize, Mark)> {
        let c = Rect::from(self.canvas);
        let tf = self.typeface.as_ref();
        self.marks
            .iter()
            .enumerate()
            // Inclusive: a straight arrow's bounds have no height, which
            // `Rect::intersects` would count as touching nothing.
            .filter(|(_, m)| {
                m.bounds(tf).is_some_and(|b| {
                    b.left <= c.right && b.right >= c.left && b.top <= c.bottom && b.bottom >= c.top
                })
            })
            .map(|(i, m)| (i, m.clone()))
            .collect()
    }

    /// Pick what the next drag draws. Returns whether it changed. A
    /// selected mark of another kind is let go, so the style row shows the
    /// new tool's style.
    pub fn set_tool(&mut self, tool: Tool) -> bool {
        let changed = self.tool != tool;
        self.tool = tool;
        if self
            .selection()
            .iter()
            .any(|&i| self.marks.get(i).map(style::mark_tool) != Some(tool))
        {
            self.deselect();
        }
        changed
    }

    /// Every selected mark, the main one first.
    pub(super) fn selection(&self) -> Vec<usize> {
        self.selected
            .into_iter()
            .chain(self.also.iter().copied())
            .collect()
    }

    fn deselect(&mut self) {
        self.selected = None;
        self.also.clear();
    }

    /// Shift+click on mark `i`: add it to the selection, or take it out.
    fn toggle_selected(&mut self, i: usize) {
        if self.selected == Some(i) {
            self.selected = (!self.also.is_empty()).then(|| self.also.remove(0));
        } else if let Some(k) = self.also.iter().position(|&j| j == i) {
            self.also.remove(k);
        } else if self.selected.is_none() {
            self.select(Some(i));
        } else {
            self.also.push(i);
        }
        self.fresh = false;
        self.chrome_key = None;
    }

    /// Select mark `i` (or nothing), switching to the tool that draws it,
    /// so the rail and the style row match what's selected.
    fn select(&mut self, i: Option<usize>) {
        self.also.clear();
        self.last_drawn = None;
        self.selected = i.filter(|&i| i < self.marks.len());
        if let Some(i) = self.selected {
            self.tool = style::mark_tool(&self.marks[i]);
        }
    }

    pub fn has_selection(&self) -> bool {
        self.selected.is_some()
    }

    /// Whether `p` (a window point in the editor window) is on the canvas.
    pub fn on_canvas(&self, p: Point) -> bool {
        self.in_canvas(self.to_doc(p))
    }

    /// Image pixels per screen pixel for what the editor draws on the
    /// canvas for itself (handles, outlines, hit slop): the display scale
    /// over the zoom, so they stay the same size on screen.
    pub(super) fn ui_px(&self) -> f32 {
        self.scale / self.zoom()
    }

    /// Whether document point `p` is on the canvas.
    fn in_canvas(&self, p: Point) -> bool {
        chrome::contains(&self.canvas, p)
    }

    /// The selected mark under `p`, and the handle if it is on one.
    fn grab_selected(&self, p: Point) -> Option<(usize, Option<Grip>)> {
        // Handles work wherever they are, even off the canvas (a rotate
        // handle above a box near the top); the marks themselves only on it.
        let on_canvas = self.in_canvas(p);
        let i = self.selected?;
        let m = self.marks.get(i)?;
        let slop = HIT_SLOP * self.ui_px();
        // Several selected: no handles, any of them grabs them all.
        if !self.also.is_empty() {
            if !on_canvas {
                return None;
            }
            return self
                .selection()
                .into_iter()
                .find(|&j| {
                    self.marks
                        .get(j)
                        .is_some_and(|m| m.hit(p, slop, self.typeface.as_ref()))
                })
                .map(|j| (j, None));
        }
        let reach = HANDLE_HALF * self.ui_px() + slop;
        if let Some((grip, _)) = grips_of(m, self.typeface.as_ref(), self.ui_px())
            .into_iter()
            .find(|(_, at)| (*at - p).length() <= reach)
        {
            return Some((i, Some(grip)));
        }
        (on_canvas && m.hit(p, slop, self.typeface.as_ref())).then_some((i, None))
    }

    /// The canvas edge or corner a press at `p` would drag.
    fn canvas_grip(&self, p: Point) -> Option<Grip> {
        let (out, inside) = (EDGE_OUT * self.ui_px(), EDGE_IN * self.ui_px());
        let c = Rect::from(self.canvas);
        // `outward` is -1 for the left and top edges, 1 for the others.
        let near = |v: f32, edge: f32, outward: f32| {
            let d = (v - edge) * outward;
            (-inside..=out).contains(&d)
        };
        let x = if near(p.x, c.left, -1.0) {
            0
        } else if near(p.x, c.right, 1.0) {
            2
        } else {
            1
        };
        let y = if near(p.y, c.top, -1.0) {
            0
        } else if near(p.y, c.bottom, 1.0) {
            2
        } else {
            1
        };
        let band = c.with_outset((out, out));
        ((x, y) != (1, 1) && contains(band, p)).then_some(Grip::Edge { x, y })
    }

    /// Each mark's step number: steps count up in the order they were made,
    /// so deleting one renumbers the rest. Other marks get 0.
    fn step_numbers(&self) -> Vec<u32> {
        let mut n = 0;
        self.marks
            .iter()
            .map(|m| {
                if matches!(m, Mark::Step(_)) {
                    n += 1;
                    n
                } else {
                    0
                }
            })
            .collect()
    }

    /// Indices of `marks` in the order they are drawn.
    fn draw_order(&self) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.marks.len()).collect();
        order.sort_by_key(|&i| self.marks[i].layer());
        order
    }

    /// The topmost mark a click at `p` picks.
    fn mark_at(&self, p: Point) -> Option<usize> {
        let slop = HIT_SLOP * self.ui_px();
        self.draw_order()
            .into_iter()
            .rev()
            .find(|&i| self.marks[i].hit(p, slop, self.typeface.as_ref()))
    }
}
