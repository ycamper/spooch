//! The editor's chrome, drawn by the core so it looks the same everywhere:
//! the tool rail, the style row, the move tab, the font list and tooltips.
//! This file lays it out and handles input; `draw.rs` draws it. The shells
//! only blend the panels and forward the pointer.

mod draw;

use skia_safe::{AlphaType, Canvas, Font, IRect, Point, Rect};

use super::bar::BarItem;
use super::style::{PALETTE, Row, Slider, Style};
use super::{Document, Format, Gesture, HoverMark, Tool};
use crate::sticker::STICKER_TABS;
use crate::{Label, LabelFont};

/// A floating piece of chrome: where it goes on the desktop, and its
/// pixels as tightly packed premultiplied BGRA (transparent around the
/// rounded corners).
#[derive(Clone, Debug, PartialEq)]
pub struct Panel {
    pub rect: IRect,
    pub pixels: Vec<u8>,
}

/// Rail sizes in screen pixels, multiplied by the display scale.
const BUTTON_W: f32 = 30.0;
const BUTTON_H: f32 = 26.0;
const PAD: f32 = 4.0;
const GAP: f32 = 2.0;
/// The gap a group separator adds between buttons.
const SEPARATOR: f32 = 7.0;
/// Space between chrome and the canvas.
const MARGIN: f32 = 8.0;
/// Separators after these tools: shapes and effects, lines and pen, steps
/// and redact, then the tools without a key.
const GROUP_ENDS: [Tool; 3] = [Tool::Pixelate, Tool::Pen, Tool::Redact];
/// Style row sizes.
const ROW_H: f32 = 32.0;
const ROW_PAD: f32 = 8.0;
const SWATCH: f32 = 18.0;
const SWATCH_GAP: f32 = 6.0;
const SIZE_BUTTON: f32 = 22.0;
/// Space taken by the line between the colours and the rest.
const SEPARATOR_GAP: f32 = 13.0;
/// Space between the sliders and the edge button.
const GROUP_GAP: f32 = 10.0;
/// A slider's icon, the gap after it, its track and its knob's radius.
const SLIDER_ICON: f32 = 14.0;
const SLIDER_ICON_GAP: f32 = 5.0;
const SLIDER_W: f32 = 76.0;
const KNOB: f32 = 6.0;
/// The label font button, showing "Aa" in the font.
const FONT_BUTTON: f32 = 26.0;
/// The font dropdown under a label: its height, text size, inner padding,
/// the chevron's width, and each row of its list.
const PICKER_H: f32 = 24.0;
const PICKER_FONT: f32 = 14.0;
const PICKER_PAD: f32 = 9.0;
const CHEVRON: f32 = 12.0;
const PICKER_ROW: f32 = 28.0;
/// A sticker on the style row, the gap between them, and a cell in the
/// grid of all of them, which has this many columns.
const STICKER_BUTTON: f32 = 26.0;
const STICKER_GAP: f32 = 3.0;
const STICKER_CELL: f32 = 40.0;
const STICKER_COLUMNS: usize = 9;
/// The grid's tabs.
const STICKER_TAB_H: f32 = 28.0;
/// Between the saved looks.
const QUICK_GAP: f32 = 4.0;
/// The move tab on the canvas's bottom edge.
const TAB_W: f32 = 40.0;
const TAB_H: f32 = 12.0;
const TIP_H: f32 = 22.0;
const TIP_FONT: f32 = 12.0;

/// The open sticker grid.
pub(super) struct StickerGrid {
    pub rect: IRect,
    /// Its tabs, into `STICKER_TABS`.
    pub tabs: Vec<IRect>,
    /// The open tab's stickers: each one's index in `STICKERS`, and cell.
    pub cells: Vec<(usize, IRect)>,
}

/// Tab `tab`'s stickers laid out in the grid, as their index in
/// `STICKERS`, column, row and cells across; and how many rows they take.
fn sticker_spots(tab: usize) -> (Vec<(usize, i32, i32, i32)>, i32) {
    let mut spots = Vec::new();
    let (mut c, mut r) = (0, 0);
    for (i, (_, art)) in crate::STICKERS.iter().enumerate() {
        if art.tab() != tab {
            continue;
        }
        let span = art.cells() as i32;
        if c > 0 && c + span > STICKER_COLUMNS as i32 {
            (c, r) = (0, r + 1);
        }
        spots.push((i, c, r, span));
        c += span;
    }
    (spots, r + 1)
}

/// A button on the chrome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Hit {
    Tool(Tool),
    Colour(usize),
    /// The colour preset button.
    Palette,
    /// The look button: highlight looks, or label plates.
    Look,
    /// A saved look at the end of the row.
    Quick(usize),
    Slider(Slider),
    Edge,
    /// The label font button.
    Font,
    /// A recently picked sticker on the style row, the button that opens
    /// every sticker, one in that grid (into `STICKERS`), and one of the
    /// grid's tabs (into `STICKER_TABS`).
    Sticker(usize),
    StickerMore,
    StickerOption(usize),
    StickerTab(usize),
    /// The font dropdown under a label, and one of its fonts.
    FontPicker,
    FontOption(usize),
    /// A row of the bar's open menu (frames or frame colours).
    MenuOption(usize),
    /// The tab that moves the whole selection.
    MoveTab,
    Bar(BarItem),
    /// The window's top row.
    Top(super::view::TopItem),
    /// The window's gallery.
    Gallery(super::gallery::GalleryHit),
}

/// Where the rail and each of its buttons sit, in desktop pixels.
pub(super) struct RailLayout {
    pub rect: IRect,
    pub buttons: Vec<(Tool, IRect)>,
    /// The lines between groups, across the rail.
    pub separators: Vec<f32>,
}

/// The rail's buttons in `cols` columns, at 100% and from inside its
/// padding. Each group starts a row.
struct RailGrid {
    /// Each tool's top-left corner.
    at: Vec<(Tool, f32, f32)>,
    /// The lines between groups.
    separators: Vec<f32>,
    size: (f32, f32),
}

impl RailGrid {
    fn new(cols: usize) -> RailGrid {
        let mut at = Vec::with_capacity(Tool::ALL.len());
        let mut separators = Vec::new();
        let (mut col, mut y) = (0, 0.0);
        for (i, tool) in Tool::ALL.into_iter().enumerate() {
            at.push((tool, col as f32 * (BUTTON_W + GAP), y));
            col += 1;
            let last = i + 1 == Tool::ALL.len();
            let end = GROUP_ENDS.contains(&tool) && !last;
            if col == cols || end || last {
                col = 0;
                y += BUTTON_H + GAP;
            }
            if end {
                separators.push(y + (SEPARATOR - GAP) / 2.0);
                y += SEPARATOR;
            }
        }
        let w = cols as f32 * BUTTON_W + (cols - 1) as f32 * GAP;
        RailGrid {
            at,
            separators,
            size: (w, y - GAP),
        }
    }
}

/// Where the style row and its controls sit.
pub(super) struct RowLayout {
    pub rect: IRect,
    pub colours: Vec<IRect>,
    /// The button that switches the colour preset.
    pub palette: Option<IRect>,
    /// The look button, for highlights and labels.
    pub look: Option<IRect>,
    /// The size slider's track; strength for blur and pixelate.
    pub size: Option<IRect>,
    pub opacity: Option<IRect>,
    pub edge: Option<IRect>,
    /// The edge thickness slider's track.
    pub outline: Option<IRect>,
    /// The font button, for text.
    pub font: Option<IRect>,
    /// The recently picked stickers, and the button for all of them.
    pub stickers: Vec<IRect>,
    /// The saved looks, last on the row.
    pub quick: Vec<IRect>,
    pub more: Option<IRect>,
    /// Whether the size slider is strength.
    pub strength: bool,
}

impl RowLayout {
    /// Every control on the row with what a press on it is: the one list
    /// hit testing, slider tracks and tooltips all read.
    pub(super) fn controls(&self) -> Vec<(Hit, IRect)> {
        let mut out: Vec<(Hit, IRect)> = self
            .colours
            .iter()
            .enumerate()
            .map(|(i, r)| (Hit::Colour(i), *r))
            .chain(
                self.stickers
                    .iter()
                    .enumerate()
                    .map(|(i, r)| (Hit::Sticker(i), *r)),
            )
            .chain(
                self.quick
                    .iter()
                    .enumerate()
                    .map(|(i, r)| (Hit::Quick(i), *r)),
            )
            .collect();
        let optional = [
            (Hit::Palette, self.palette),
            (Hit::Look, self.look),
            (Hit::Slider(Slider::Size), self.size),
            (Hit::Slider(Slider::Opacity), self.opacity),
            (Hit::Edge, self.edge),
            (Hit::Slider(Slider::Outline), self.outline),
            (Hit::Font, self.font),
            (Hit::StickerMore, self.more),
        ];
        out.extend(optional.into_iter().filter_map(|(h, r)| Some((h, r?))));
        out
    }

    /// Where control `hit` is, if it's on the row.
    pub(super) fn place_of(&self, hit: Hit) -> Option<IRect> {
        self.controls()
            .into_iter()
            .find(|(h, _)| *h == hit)
            .map(|(_, r)| r)
    }
}

/// The font dropdown under a single label: the pill naming its font, and
/// the list of fonts while it's open.
pub(super) struct PickerLayout {
    pub pill: IRect,
    pub list: Option<IRect>,
    pub options: Vec<IRect>,
}

/// What the chrome was last drawn from, so unchanged chrome isn't redrawn.
#[derive(Clone, PartialEq)]
pub(super) struct ChromeKey {
    rail: IRect,
    row: Option<IRect>,
    bar: IRect,
    bar_text: Vec<String>,
    format: Format,
    tool: Tool,
    style: Option<(Style, Row)>,
    hovered: Option<Hit>,
    sliding: bool,
    canvas: IRect,
    moving: bool,
    picker: Option<(IRect, Option<IRect>)>,
    stickers: [u8; super::style::RECENT_STICKERS],
    sticker_menu: Option<usize>,
    bar_menu: Option<super::bar::BarMenu>,
    loupe: Option<(i32, i32)>,
    maximized: bool,
    view: Option<ViewKey>,
    gallery: Option<(u64, i32)>,
}

/// The window view as the chrome cache sees it: size, zoom in thousandths,
/// fitted, and centre.
type ViewKey = ((i32, i32), i32, bool, (i32, i32));

pub(super) fn contains(r: &IRect, p: Point) -> bool {
    let (x, y) = (p.x.floor() as i32, p.y.floor() as i32);
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

impl Document {
    /// The monitors, in desktop pixels, so chrome stays on the one the
    /// canvas is on. Without them the whole desktop counts as one screen.
    pub fn set_screens(&mut self, screens: Vec<IRect>) {
        self.screens = screens;
    }

    /// The monitor the canvas's top-left corner is on; in the editor
    /// window, the window.
    pub(super) fn screen(&self) -> IRect {
        if let Some(v) = &self.view {
            return IRect::from_wh(v.size.0, v.size.1);
        }
        self.screen_at((self.canvas.left, self.canvas.top))
    }

    /// The rail's columns: two when one won't fit the height it has.
    fn rail_columns(&self) -> usize {
        let room = self.rail_room().unwrap_or_else(|| self.screen().height());
        let h = RailGrid::new(1).size.1;
        if ((h + 2.0 * PAD) * self.scale).round() as i32 <= room {
            1
        } else {
            2
        }
    }

    /// The rail's width.
    pub(super) fn rail_width(&self) -> i32 {
        let w = RailGrid::new(self.rail_columns()).size.0;
        ((w + 2.0 * PAD) * self.scale).round() as i32
    }

    pub(super) fn rail_layout(&self) -> RailLayout {
        let s = self.scale;
        let grid = RailGrid::new(self.rail_columns());
        let (gw, gh) = grid.size;
        let w = ((gw + 2.0 * PAD) * s).round() as i32;
        let h = ((gh + 2.0 * PAD) * s).round() as i32;
        let margin = (MARGIN * s).round() as i32;
        let screen = self.screen();
        let c = self.canvas;
        // Left of the canvas; else right of it; else just inside it.
        let x = if c.left - margin - w >= screen.left {
            c.left - margin - w
        } else if c.right + margin + w <= screen.right {
            c.right + margin
        } else {
            c.left + margin
        };
        let y = c.top.min(screen.bottom - h).max(screen.top);
        let mut rect = IRect::from_xywh(x, y, w, h);
        // In the window, docked down the left edge.
        let (x, y) = match self.dock() {
            Some(d) => {
                rect = d.rail;
                (
                    d.rail.left,
                    d.rail.top + (super::view::RAIL_INSET * s).round() as i32,
                )
            }
            None => (x, y),
        };
        let (x, y) = (x as f32 + PAD * s, y as f32 + PAD * s);
        let (bw, bh) = ((BUTTON_W * s).round() as i32, (BUTTON_H * s).round() as i32);
        let buttons = grid
            .at
            .into_iter()
            .map(|(tool, bx, by)| {
                let left = (x + bx * s).round() as i32;
                (
                    tool,
                    IRect::from_xywh(left, (y + by * s).round() as i32, bw, bh),
                )
            })
            .collect();
        let separators = grid.separators.into_iter().map(|sy| y + sy * s).collect();
        RailLayout {
            rect,
            buttons,
            separators,
        }
    }

    /// How far across a floating panel in rows `band` (top, bottom) can
    /// go: the screen, less the rail's side when they'd share rows.
    pub(super) fn panel_span(&self, band: (i32, i32)) -> (i32, i32) {
        let screen = self.screen();
        let rail = self.rail_layout().rect;
        let m = (MARGIN * self.scale).round() as i32;
        let (mut lo, mut hi) = (screen.left, screen.right);
        if rail.top < band.1 && rail.bottom > band.0 {
            if rail.left + rail.right <= self.canvas.left + self.canvas.right {
                lo = lo.max(rail.right + m);
            } else {
                hi = hi.min(rail.left - m);
            }
        }
        (lo, hi)
    }

    /// Where a floating panel `w` wide in rows `band` goes across: lined up
    /// with the canvas's left edge, slid to stay on the screen, and clear
    /// of the rail when there's room. Panels keep their order and shape
    /// wherever the selection is, so they can be learned.
    pub(super) fn panel_x(&self, w: i32, band: (i32, i32)) -> i32 {
        let (mut lo, mut hi) = self.panel_span(band);
        if hi - lo < w {
            let screen = self.screen();
            (lo, hi) = (screen.left, screen.right);
        }
        self.canvas.left.min(hi - w).max(lo)
    }

    /// The style row above the canvas, or `None` when what's on show has
    /// no style (solid redact). Most marks get colours, a size slider, an
    /// opacity slider and an edge button; the highlighter colours only;
    /// blur and pixelate a strength slider.
    pub(super) fn row_layout(&self) -> Option<RowLayout> {
        let (_, row) = self.shown_style()?;
        let s = self.scale;
        let has_colours = matches!(row, Row::Full | Row::Colours);
        let has_size = matches!(row, Row::Full | Row::Strength);
        let has_looks = row == Row::Full;
        let has_stickers = row == Row::Stickers;
        // The swatches, then the colour preset button.
        let colours_w = (PALETTE.len() + 1) as f32 * SWATCH + PALETTE.len() as f32 * SWATCH_GAP;
        let slider_w = SLIDER_ICON + SLIDER_ICON_GAP + SLIDER_W;
        // Widths of the groups after the colours, each preceded by a gap.
        let mut rest = Vec::new();
        let has_look = self.looks().is_some();
        if has_look {
            rest.push(SIZE_BUTTON);
        }
        if has_size {
            rest.push(slider_w);
        }
        let has_font = has_looks && self.shows_font();
        if has_looks {
            rest.push(slider_w);
            rest.push(SIZE_BUTTON);
            rest.push(slider_w);
        }
        if has_font {
            rest.push(FONT_BUTTON);
        }
        let has_quick = self.quick_tool().is_some();
        let n = super::style::QUICK_STYLES as f32;
        if has_quick {
            rest.push(n * SIZE_BUTTON + (n - 1.0) * QUICK_GAP);
        }
        let stickers_w = if has_stickers {
            let n = super::style::RECENT_STICKERS as f32;
            n * STICKER_BUTTON + (n - 1.0) * STICKER_GAP + SEPARATOR_GAP + SIZE_BUTTON
        } else {
            0.0
        };
        let mut inner = if has_colours { colours_w } else { stickers_w };
        for (i, w) in rest.iter().enumerate() {
            let gap = match (i, has_colours) {
                (0, true) => SEPARATOR_GAP,
                (0, false) => 0.0,
                _ => GROUP_GAP,
            };
            inner += gap + w;
        }
        let w = ((2.0 * ROW_PAD + inner) * s).round() as i32;
        let h = (ROW_H * s).round() as i32;
        let margin = (MARGIN * s).round() as i32;
        let screen = self.screen();
        let c = self.canvas;
        // Above the canvas, else just inside its top.
        let y = if c.top - margin - h >= screen.top {
            c.top - margin - h
        } else {
            c.top + margin
        };
        let x = self.panel_x(w, (y, y + h));
        let mut rect = IRect::from_xywh(x, y, w, h);
        // In the window, docked under the top row.
        let (x, y) = match self.dock() {
            Some(d) => {
                rect = d.row;
                (
                    d.row.left + (4.0 * s).round() as i32,
                    d.row.top + (d.row.height() - h) / 2,
                )
            }
            None => (x, y),
        };

        let mid = y as f32 + h as f32 / 2.0;
        let mut left = x as f32 + ROW_PAD * s;
        let square = |left: f32, size: f32| {
            IRect::from_xywh(
                left.round() as i32,
                (mid - size * s / 2.0).round() as i32,
                (size * s).round() as i32,
                (size * s).round() as i32,
            )
        };
        let mut colours = Vec::new();
        let mut palette = None;
        let mut stickers = Vec::new();
        let mut more = None;
        if has_stickers {
            for _ in 0..super::style::RECENT_STICKERS {
                stickers.push(square(left, STICKER_BUTTON));
                left += (STICKER_BUTTON + STICKER_GAP) * s;
            }
            left += (SEPARATOR_GAP - STICKER_GAP) * s;
            more = Some(square(left, SIZE_BUTTON));
        }
        if has_colours {
            for _ in 0..PALETTE.len() {
                colours.push(square(left, SWATCH));
                left += (SWATCH + SWATCH_GAP) * s;
            }
            palette = Some(square(left, SWATCH));
            left += (SWATCH + SEPARATOR_GAP) * s;
        }
        // A slider's track, after its icon, as tall as a button so it's
        // easy to grab.
        let track = |left: &mut f32| {
            *left += (SLIDER_ICON + SLIDER_ICON_GAP) * s;
            let r = IRect::from_xywh(
                left.round() as i32,
                (mid - SIZE_BUTTON * s / 2.0).round() as i32,
                (SLIDER_W * s).round() as i32,
                (SIZE_BUTTON * s).round() as i32,
            );
            *left += (SLIDER_W + GROUP_GAP) * s;
            r
        };
        let look = has_look.then(|| {
            let r = square(left, SIZE_BUTTON);
            left += (SIZE_BUTTON + GROUP_GAP) * s;
            r
        });
        let size = has_size.then(|| track(&mut left));
        let opacity = has_looks.then(|| track(&mut left));
        let edge = has_looks.then(|| {
            let r = square(left, SIZE_BUTTON);
            left += (SIZE_BUTTON + GROUP_GAP) * s;
            r
        });
        let outline = has_looks.then(|| track(&mut left));
        let font = has_font.then(|| {
            let r = square(left, FONT_BUTTON);
            left += (FONT_BUTTON + GROUP_GAP) * s;
            r
        });
        let mut quick = Vec::new();
        if has_quick {
            for _ in 0..super::style::QUICK_STYLES {
                quick.push(square(left, SIZE_BUTTON));
                left += (SIZE_BUTTON + QUICK_GAP) * s;
            }
        }
        Some(RowLayout {
            rect,
            colours,
            palette,
            look,
            size,
            opacity,
            edge,
            outline,
            font,
            stickers,
            more,
            quick,
            strength: row == Row::Strength,
        })
    }

    /// The move tab, centred on the canvas's bottom edge and kept on its
    /// screen.
    pub(super) fn tab_rect(&self) -> IRect {
        // The window has no need to move the capture about.
        if self.in_window() {
            return IRect::new_empty();
        }
        let s = self.scale;
        let (w, h) = ((TAB_W * s).round() as i32, (TAB_H * s).round() as i32);
        let c = self.canvas;
        let screen = self.screen();
        let x = (c.left + c.right) / 2 - w / 2;
        let y = (c.bottom - h / 2).min(screen.bottom - h);
        IRect::from_xywh(x, y, w, h)
    }

    /// The grid of every sticker, open under the style row (above it when
    /// there's no room below): tabs across the top, then the open tab's
    /// stickers. It's as tall as the fullest tab, so it doesn't jump when
    /// the tab changes.
    pub(super) fn sticker_grid(&self) -> Option<StickerGrid> {
        let tab = self.sticker_menu?;
        let row = self.row_layout().filter(|r| r.more.is_some())?;
        let s = self.scale;
        let (spots, _) = sticker_spots(tab);
        let rows = (0..STICKER_TABS.len())
            .map(|t| sticker_spots(t).1)
            .max()
            .unwrap_or(1);
        let (cell, pad) = ((STICKER_CELL * s).round() as i32, (6.0 * s).round() as i32);
        let tab_h = (STICKER_TAB_H * s).round() as i32;
        let w = STICKER_COLUMNS as i32 * cell + 2 * pad;
        let h = tab_h + rows * cell + 3 * pad;
        let screen = self.screen();
        let gap = (4.0 * s).round() as i32;
        let below = row.rect.bottom + gap;
        let top = if below + h <= screen.bottom {
            below
        } else {
            row.rect.top - gap - h
        };
        let left = row.rect.left.min(screen.right - w).max(screen.left);
        let tab_w = (w - 2 * pad) / STICKER_TABS.len() as i32;
        let tabs = (0..STICKER_TABS.len() as i32)
            .map(|t| IRect::from_xywh(left + pad + t * tab_w, top + pad, tab_w, tab_h))
            .collect();
        let cells_top = top + 2 * pad + tab_h;
        let cells = spots
            .into_iter()
            .map(|(i, c, r, span)| {
                let x = left + pad + c * cell;
                (
                    i,
                    IRect::from_xywh(x, cells_top + r * cell, span * cell, cell),
                )
            })
            .collect();
        Some(StickerGrid {
            rect: IRect::from_xywh(left, top, w, h),
            tabs,
            cells,
        })
    }

    /// The one label the font dropdown is for: the label being typed, or
    /// the only mark selected when it's a label.
    fn picker_label(&self) -> Option<&Label> {
        if let Some(l) = &self.typing {
            return Some(l);
        }
        if !self.also.is_empty() {
            return None;
        }
        match self.selected.and_then(|i| self.marks.get(i)) {
            Some(super::Mark::Label(l)) => Some(l),
            _ => None,
        }
    }

    /// Where the font dropdown sits: just under the label (its turned box,
    /// when it's turned), or above it near the bottom of the screen.
    pub(super) fn picker_layout(&self) -> Option<PickerLayout> {
        if self.is_dragging() {
            return None;
        }
        let l = self.picker_label()?;
        let tf = self.typeface.as_ref()?;
        let s = self.scale;
        let mut b = super::label_box(l, tf, s);
        let window = self.in_window();
        if l.angle != 0.0 {
            let c = b.center();
            let corners = [
                Point::new(b.left, b.top),
                Point::new(b.right, b.top),
                Point::new(b.left, b.bottom),
                Point::new(b.right, b.bottom),
            ]
            .map(|p| super::rotate_about(p, l.angle, c));
            b = Rect::from_ltrb(
                corners.iter().map(|p| p.x).fold(f32::MAX, f32::min),
                corners.iter().map(|p| p.y).fold(f32::MAX, f32::min),
                corners.iter().map(|p| p.x).fold(f32::MIN, f32::max),
                corners.iter().map(|p| p.y).fold(f32::MIN, f32::max),
            );
        }
        // In the window the label is shown zoomed.
        if window {
            b = self.screen_rect(b);
        }
        let width = |f: LabelFont| {
            f.typeface()
                .map(|t| {
                    Font::from_typeface(t, PICKER_FONT * s)
                        .measure_str(f.name(), None)
                        .0
                })
                .unwrap_or(90.0 * s)
        };
        let pill_w = (width(l.font) + (2.0 * PICKER_PAD + CHEVRON) * s).ceil() as i32;
        let h = (PICKER_H * s).round() as i32;
        let gap = (6.0 * s).round() as i32;
        let row_h = (PICKER_ROW * s).round() as i32;
        let n = LabelFont::ALL.len() as i32;
        let list_h = if self.font_menu {
            n * row_h + 2 * gap
        } else {
            0
        };
        let screen = self.screen();
        // Below the label, unless that runs off the screen.
        let below = b.bottom.round() as i32 + gap;
        let up = below + h + list_h > screen.bottom;
        let y = if up {
            b.top.round() as i32 - gap - h
        } else {
            below
        };
        let x = (b.left.round() as i32).clamp(screen.left, screen.right - pill_w);
        let pill = IRect::from_xywh(x, y, pill_w, h);
        if !self.font_menu {
            return Some(PickerLayout {
                pill,
                list: None,
                options: Vec::new(),
            });
        }
        let list_w = LabelFont::ALL
            .into_iter()
            .map(|f| (width(f) + 2.0 * PICKER_PAD * s).ceil() as i32)
            .max()
            .unwrap_or(0)
            .max(pill_w);
        let top = if up { y - gap - list_h } else { y + h + gap };
        let list = IRect::from_xywh(x, top, list_w, list_h);
        let options = (0..n)
            .map(|i| IRect::from_xywh(x, top + gap + i * row_h, list_w, row_h))
            .collect();
        Some(PickerLayout {
            pill,
            list: Some(list),
            options,
        })
    }

    /// The chrome button under `p`, if any.
    pub(super) fn chrome_hit(&self, p: Point) -> Option<Hit> {
        if let Some(item) = self.top_hit(p) {
            return Some(Hit::Top(item));
        }
        if let Some(g) = self.gallery_hit(p) {
            return Some(Hit::Gallery(g));
        }
        if let Some((_, _, rows)) = self.bar_menu_layout(&self.bar_layout())
            && let Some(i) = rows.iter().position(|r| contains(r, p))
        {
            return Some(Hit::MenuOption(i));
        }
        if let Some(g) = self.sticker_grid() {
            if let Some(t) = g.tabs.iter().position(|r| contains(r, p)) {
                return Some(Hit::StickerTab(t));
            }
            if let Some((i, _)) = g.cells.iter().find(|(_, r)| contains(r, p)) {
                return Some(Hit::StickerOption(*i));
            }
        }
        if let Some(picker) = self.picker_layout() {
            if let Some(i) = picker.options.iter().position(|r| contains(r, p)) {
                return Some(Hit::FontOption(i));
            }
            if contains(&picker.pill, p) {
                return Some(Hit::FontPicker);
            }
        }
        if contains(&self.tab_rect(), p) {
            return Some(Hit::MoveTab);
        }
        if let Some((tool, _)) = self
            .rail_layout()
            .buttons
            .into_iter()
            .find(|(_, r)| contains(r, p))
        {
            return Some(Hit::Tool(tool));
        }
        if let Some(row) = self.row_layout()
            && let Some((hit, _)) = row.controls().into_iter().find(|(_, r)| contains(r, p))
        {
            return Some(hit);
        }
        self.bar_hit(p).map(Hit::Bar)
    }

    /// Whether `p` is anywhere on the chrome, buttons or not.
    pub fn on_chrome(&self, p: Point) -> bool {
        // In the window, everything but the capture's area.
        if let Some(d) = self.dock()
            && !contains(&d.area, p)
        {
            return true;
        }
        self.bar_menu_layout(&self.bar_layout())
            .is_some_and(|(_, list, _)| contains(&list, p))
            || self.sticker_grid().is_some_and(|g| contains(&g.rect, p))
            || self
                .picker_layout()
                .is_some_and(|k| contains(&k.pill, p) || k.list.is_some_and(|l| contains(&l, p)))
            || contains(&self.rail_layout().rect, p)
            || contains(&self.tab_rect(), p)
            || self.row_layout().is_some_and(|r| contains(&r.rect, p))
            || self.on_bar(p)
    }

    /// A press on the chrome: pick the tool or colour under it, cycle the
    /// edge, or start dragging a slider. Returns whether it was on the
    /// chrome at all.
    pub(super) fn press_chrome(&mut self, p: Point) -> bool {
        // An open font list closes on any press but one on it, and typing
        // the frame's title ends on any press but one on the title.
        let hit = self.chrome_hit(p);
        if self.editing_title() && hit != Some(Hit::Bar(BarItem::FrameTitle)) {
            self.set_editing_title(false);
        }
        // An open menu closes on any press but one on it or its button.
        if !matches!(
            hit,
            Some(Hit::MenuOption(_) | Hit::Bar(BarItem::Frame | BarItem::FrameColour))
        ) {
            self.close_bar_menu();
        }
        if !matches!(
            hit,
            Some(Hit::Gallery(super::gallery::GalleryHit::Delete(_)))
        ) {
            self.disarm_gallery();
        }
        if self.font_menu && !matches!(hit, Some(Hit::FontPicker | Hit::FontOption(_))) {
            self.font_menu = false;
            self.chrome_key = None;
        }
        if self.sticker_menu.is_some()
            && !matches!(
                hit,
                Some(Hit::StickerMore | Hit::StickerOption(_) | Hit::StickerTab(_))
            )
        {
            self.sticker_menu = None;
            self.chrome_key = None;
        }
        if !self.on_chrome(p) {
            return false;
        }
        match hit {
            Some(Hit::MenuOption(i)) => self.pick_menu(i),
            Some(Hit::FontPicker) => {
                self.font_menu = !self.font_menu;
                self.chrome_key = None;
            }
            Some(Hit::Sticker(i)) => {
                let name = self.recent_stickers()[i];
                if let Some(index) = crate::STICKERS.iter().position(|(n, _)| *n == name) {
                    self.pick_sticker(index);
                }
            }
            // The grid opens on the tab of the sticker the tool stamps.
            Some(Hit::StickerMore) => {
                self.sticker_menu = match self.sticker_menu {
                    Some(_) => None,
                    None => Some(crate::sticker::art_of(self.sticker()).map_or(0, |a| a.tab())),
                };
                self.chrome_key = None;
            }
            Some(Hit::StickerTab(t)) => {
                self.sticker_menu = Some(t);
                self.chrome_key = None;
            }
            Some(Hit::StickerOption(i)) => {
                self.pick_sticker(i);
                self.sticker_menu = None;
                self.chrome_key = None;
            }
            Some(Hit::FontOption(i)) => {
                self.set_font(i);
                self.font_menu = false;
                self.chrome_key = None;
            }
            Some(Hit::Tool(t)) => {
                self.commit_text();
                self.set_tool(t);
                self.fresh = false;
            }
            Some(Hit::Colour(i)) => {
                self.set_colour(i);
            }
            Some(Hit::Palette) => {
                self.cycle_palette();
            }
            Some(Hit::Quick(i)) => {
                self.press_quick(i);
            }
            Some(Hit::Look) => {
                self.cycle_look();
            }
            Some(Hit::Slider(slider)) => {
                if let Some(track) = self.slider_track(slider) {
                    self.gesture = Some(Gesture::Slide {
                        slider,
                        track,
                        before: self.state(),
                    });
                    self.hovered = None;
                    self.slide(slider, slider_t(track, p, self.scale));
                }
            }
            Some(Hit::Edge) => {
                self.cycle_edge();
            }
            Some(Hit::Font) => {
                self.cycle_font();
            }
            Some(Hit::MoveTab) => {
                self.commit_text();
                self.want_desktop = self.base_rect() != self.desktop;
                self.hovered = None;
                let carry = if self.shift {
                    self.marks_on_canvas()
                } else {
                    Vec::new()
                };
                self.gesture = Some(Gesture::Canvas {
                    grip: super::Grip::Whole,
                    from: p,
                    original: self.canvas,
                    carry,
                });
            }
            Some(Hit::Bar(item)) => self.press_bar(item),
            Some(Hit::Top(item)) => self.press_top(item),
            Some(Hit::Gallery(g)) => self.press_gallery(g),
            None => {}
        }
        true
    }

    /// A slider's track, where it is now.
    fn slider_track(&self, slider: Slider) -> Option<IRect> {
        self.row_layout()?.place_of(Hit::Slider(slider))
    }

    /// The pointer moved to `p` without a button down. Returns whether the
    /// chrome changed (a button's hover highlight or tooltip) and needs
    /// repainting.
    pub fn pointer_moved(&mut self, p: Point) -> bool {
        let d = self.to_doc(p);
        // The loupe after a nudge stays until the pointer really moves.
        let away = (d.x - self.pointer.x).abs() >= 1.0 || (d.y - self.pointer.y).abs() >= 1.0;
        let loupe_gone = away && !self.is_dragging() && self.loupe_at.take().is_some();
        self.pointer = d;
        let hovered = if self.is_dragging() {
            None
        } else {
            self.chrome_hit(p)
        };
        let changed = hovered != self.hovered;
        self.hovered = hovered;
        // On the canvas: outline what a press would grab.
        let mark = if self.is_dragging() || self.is_typing() || self.on_chrome(p) {
            None
        } else if let Some((_, Some(g))) = self.grab_selected(d) {
            Some(HoverMark::Grip(g))
        } else if self.grab_selected(d).is_some() {
            None
        } else {
            self.mark_at(d)
                .filter(|_| self.in_canvas(d))
                .map(HoverMark::Mark)
        };
        let mark_changed = mark != self.hover_mark;
        if mark_changed {
            self.hover_mark = mark;
            self.fresh = false;
        }
        // On a canvas edge: the crop handles show in the window.
        let edge = !self.on_chrome(p) && self.canvas_grip(d).is_some();
        let edge_changed = edge != self.edge_hover;
        self.edge_hover = edge;
        changed || mark_changed || loupe_gone || (edge_changed && self.in_window())
    }

    /// Everything the chrome can cover, for repainting: the rail and the
    /// style row, with room for their tooltips.
    pub fn chrome_bounds(&self) -> IRect {
        // The Alt preview covers the whole screen; the window is all its
        // own.
        if self.preview || self.in_window() {
            return self.screen();
        }
        let s = self.scale;
        let r = self.rail_layout().rect;
        let tip = (220.0 * s) as i32;
        let mut all = IRect::from_ltrb(r.left, r.top, r.right + tip, r.bottom);
        let above = ((TIP_H + 6.0) * s) as i32;
        let mut add = |r: IRect| {
            all = IRect::from_ltrb(
                all.left.min(r.left),
                all.top.min(r.top),
                all.right.max(r.right),
                all.bottom.max(r.bottom),
            );
        };
        if let Some(row) = self.row_layout() {
            add(IRect::from_ltrb(
                row.rect.left,
                row.rect.top - above,
                row.rect.right + tip / 2,
                row.rect.bottom,
            ));
        }
        // The bar, and tooltips above its buttons.
        let bar = self.bar_layout();
        add(IRect::from_ltrb(
            bar.rect.left - tip,
            bar.rect.top - above,
            bar.rect.right + tip,
            bar.rect.bottom,
        ));
        add(self.tab_rect());
        if let Some((_, list, _)) = self.bar_menu_layout(&bar) {
            add(list);
        }
        if let Some(k) = self.picker_layout() {
            add(k.pill);
            if let Some(l) = k.list {
                add(l);
            }
        }
        if let Some(at) = self.loupe_at {
            add(crate::select::loupe_rect(
                at,
                self.screen_at(at),
                self.scale,
            ));
        }
        all
    }

    /// The monitor `p` is on; without monitors, the whole desktop.
    fn screen_at(&self, (x, y): (i32, i32)) -> IRect {
        if self.in_window() {
            return self.screen();
        }
        self.screens
            .iter()
            .copied()
            .find(|s| x >= s.left && x < s.right && y >= s.top && y < s.bottom)
            .unwrap_or(self.desktop)
    }
}

/// Where `p` falls along a slider's `track`, 0 at the knob's leftmost spot
/// to 1 at its rightmost.
pub(super) fn slider_t(track: IRect, p: Point, scale: f32) -> f32 {
    let knob = KNOB * scale;
    let (x0, x1) = (track.left as f32 + knob, track.right as f32 - knob);
    if x1 <= x0 {
        return 0.0;
    }
    ((p.x - x0) / (x1 - x0)).clamp(0.0, 1.0)
}

/// Render a panel of `rect`'s size with `draw`, on a transparent surface.
pub(crate) fn panel(rect: IRect, draw: impl FnOnce(&Canvas)) -> Panel {
    let size = (rect.width().max(1), rect.height().max(1));
    Panel {
        rect,
        pixels: crate::draw_bgra(size, AlphaType::Premul, draw),
    }
}

#[cfg(test)]
mod tests;
