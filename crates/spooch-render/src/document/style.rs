//! Colours, sizes, edges, opacity and strength, from small built-in sets
//! so outputs stay consistent (opacity is a 5% slider). Each tool keeps its
//! last style; picking one restyles the selected marks too.

use skia_safe::{Color4f, Point};

use super::frame::Frame;
use super::theme::hex;
use super::{Document, Mark, Redaction, Tool};
use crate::{HighlightLook, LabelFont, Look, Plate, STICKERS};

/// Eight mark colours on Shift+1 to Shift+8, balanced in OKLCH. Every
/// preset keeps the same slots: red first, yellow third, white last.
pub struct Palette {
    pub name: &'static str,
    pub colours: [Color4f; 8],
}

/// The mark colour presets; the first is the default.
pub const PALETTES: [Palette; 3] = [
    // Bright and clean, for any screenshot.
    Palette {
        name: "Vivid",
        colours: [
            hex(0xf93534),
            hex(0xfe7802),
            hex(0xfacc08),
            hex(0x27bd53),
            hex(0x2981fb),
            hex(0x9754ed),
            hex(0x222222),
            hex(0xffffff),
        ],
    },
    // Pastels, for dark-themed screenshots and docs.
    Palette {
        name: "Soft",
        colours: [
            hex(0xf88d96),
            hex(0xfcb17e),
            hex(0xf6e193),
            hex(0x98df9a),
            hex(0x84b7f9),
            hex(0xc6a0f2),
            hex(0x2b2b33),
            hex(0xffffff),
        ],
    },
    // Calm: greys with one red, one blue and a marker yellow.
    Palette {
        name: "Ink",
        colours: [
            hex(0xf93534),
            hex(0x4b5058),
            hex(0xfacc08),
            hex(0x8b9099),
            hex(0x2981fb),
            hex(0xd4d7dc),
            hex(0x111111),
            hex(0xffffff),
        ],
    },
];

/// The default preset's colours.
pub const PALETTE: [Color4f; 8] = PALETTES[0].colours;
const WHITE: u8 = 7;

pub const SIZES: usize = 5;
/// Arrow width (and so line, box and pen thickness) per size step, in
/// screen pixels.
const WIDTHS: [f32; SIZES] = [4.0, 6.0, 8.0, 11.0, 15.0];
/// Label text size per size step; numbered steps scale with it.
const TEXT: [f32; SIZES] = [18.0, 22.0, 28.0, 36.0, 46.0];
/// A numbered step's radius, per pixel of the label size it matches.
pub(super) const STEP_RADIUS: f32 = 0.55;
/// Blur radius and pixelate block size per strength step, in screen
/// pixels.
pub(super) const STRENGTH: [f32; SIZES] = [4.0, 8.0, 12.0, 18.0, 26.0];
/// The highlighter's colour slot: yellow in every preset.
pub(super) const YELLOW: u8 = 2;
/// A colour index meaning the selected marks' colours differ.
pub(super) const MIXED: u8 = u8::MAX;

/// Edge choices: white (Skitch's), black, a darker shade of the mark's
/// colour, none, or auto (white over dark backgrounds, black over light),
/// the default. The edge button goes round them from auto.
pub const EDGES: usize = 5;
const EDGE_WHITE: u8 = 0;
const EDGE_BLACK: u8 = 1;
const EDGE_DARKER: u8 = 2;
const EDGE_NONE: u8 = 3;
const EDGE_AUTO: u8 = 4;
/// Edge thickness steps, as multiples of each mark's standard edge.
pub const EDGE_WIDTHS: [f32; 5] = [0.5, 1.0, 1.5, 2.0, 3.0];
/// The standard edge thickness.
const STANDARD_EDGE: u8 = 1;

/// Opacity runs from this up to 100%, in 5% steps.
pub const MIN_OPACITY: u8 = 10;
const OPACITY_STEP: u8 = 5;

/// A style, as indexes into the sets above.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Style {
    pub color: u8,
    /// Size step, or strength for blur and pixelate.
    pub size: u8,
    pub edge: u8,
    /// Percent, `MIN_OPACITY` to 100.
    pub opacity: u8,
    /// Index into `LabelFont::ALL`, for labels.
    pub font: u8,
    /// Edge thickness step, into `EDGE_WIDTHS`.
    pub edge_width: u8,
    /// The tool's own look: into `HighlightLook::ALL` for highlights,
    /// `Plate::ALL` for labels.
    #[serde(alias = "highlight")]
    pub variant: u8,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            color: 0,
            size: 2,
            edge: EDGE_AUTO,
            opacity: 100,
            font: 0,
            edge_width: STANDARD_EDGE,
            variant: 0,
        }
    }
}

/// Which controls the style row shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    /// Colours, sizes, edges and opacity.
    Full,
    /// Colours only: the highlighter.
    Colours,
    /// Strength only: blur and pixelate.
    Strength,
    /// The stickers to pick from.
    Stickers,
}

/// How many recently picked stickers the style row shows.
pub const RECENT_STICKERS: usize = 10;

/// How many saved looks each tool has, as chips at the end of the row.
pub const QUICK_STYLES: usize = 3;

/// The stickers the row starts with, before any are picked.
const FIRST_STICKERS: [&str; RECENT_STICKERS] = [
    "thumbs_up",
    "fire",
    "hundred_points",
    "check_mark_button",
    "cross_mark",
    "red_question_mark",
    "eyes",
    "face_with_tears_of_joy",
    "thinking_face",
    "party_popper",
];

/// Everything picked in the chrome that carries on to the next capture:
/// each tool's style, the frame, its colour, the shadow and the colour
/// preset. Kept by the shell between captures, so it stays `Copy`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Styles {
    pub(super) tools: [Style; Tool::ALL.len()],
    /// The frame round the output, its colour and shadow.
    pub(super) framing: Framing,
    /// The mark colour preset, into `PALETTES`.
    pub(super) palette: u8,
    /// Whether the editor window shows its gallery.
    pub(super) gallery: bool,
    /// The stickers picked lately, newest first, into `STICKERS`; the
    /// first is the one the tool stamps.
    pub(super) stickers: [u8; RECENT_STICKERS],
    /// The look every capture starts with ("Use for every capture" in the
    /// Frame menu). Without one, each capture starts with the last.
    pub(super) pinned: Option<Framing>,
    /// Each tool's saved looks, by `tool_index`.
    pub(super) quick: [[Option<Style>; QUICK_STYLES]; Tool::ALL.len()],
}

/// What goes round the output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Framing {
    pub frame: Frame,
    /// Into `FRAME_COLOURS`.
    pub frame_colour: u8,
    /// Into `SHADOWS`.
    pub shadow: u8,
    /// Into `BACKGROUNDS`.
    pub background: u8,
    /// Into `PADDINGS`.
    pub padding: u8,
}

impl Default for Framing {
    fn default() -> Self {
        Framing {
            frame: Frame::NONE,
            frame_colour: 0,
            shadow: 0,
            background: 0,
            padding: super::frame::DEFAULT_PADDING,
        }
    }
}

impl Default for Styles {
    fn default() -> Self {
        let mut tools = [Style::default(); Tool::ALL.len()];
        tools[tool_index(Tool::Highlight)].color = YELLOW;
        Styles {
            tools,
            framing: Framing::default(),
            palette: 0,
            gallery: true,
            stickers: FIRST_STICKERS
                .map(|name| STICKERS.iter().position(|(n, _)| *n == name).unwrap_or(0) as u8),
            pinned: None,
            quick: [[None; QUICK_STYLES]; Tool::ALL.len()],
        }
    }
}

impl Styles {
    /// The same styles with colour preset `index` (into `PALETTES`), as
    /// `settings.txt` picks it.
    pub fn with_palette(mut self, index: usize) -> Styles {
        self.palette = index.min(PALETTES.len() - 1) as u8;
        self
    }

    /// The styles the next capture starts with: these, but with the
    /// pinned framing if there is one.
    pub fn for_next_capture(mut self) -> Styles {
        if let Some(p) = self.pinned {
            self.framing = p;
        }
        self
    }
}

impl Palette {
    /// The preset called `name`, ignoring case.
    pub fn by_name(name: &str) -> Option<usize> {
        PALETTES
            .iter()
            .position(|p| p.name.eq_ignore_ascii_case(name.trim()))
    }
}

/// Size step `size` one up (`bigger`) or down, stopping at the ends.
fn stepped(size: u8, bigger: bool) -> u8 {
    if bigger {
        (size + 1).min(SIZES as u8 - 1)
    } else {
        size.saturating_sub(1)
    }
}

pub(super) fn tool_index(tool: Tool) -> usize {
    Tool::ALL.iter().position(|t| *t == tool).unwrap_or(0)
}

impl Tool {
    /// The style row for the tool, if it has one.
    pub fn row(self) -> Option<Row> {
        match self {
            Tool::Redact => None,
            Tool::Blur | Tool::Pixelate => Some(Row::Strength),
            Tool::Highlight => Some(Row::Colours),
            Tool::Sticker => Some(Row::Stickers),
            _ => Some(Row::Full),
        }
    }
}

/// The tool that draws marks like `m`, if it has a style of its own.
fn tool_of(m: &Mark) -> Option<Tool> {
    Some(match m {
        Mark::Arrow(_) => Tool::Arrow,
        Mark::Line(_) => Tool::Line,
        Mark::Box(_) => Tool::Box,
        Mark::Oval(_) => Tool::Ellipse,
        Mark::Stroke(_) => Tool::Pen,
        Mark::Highlight(_) => Tool::Highlight,
        Mark::Step(_) => Tool::Steps,
        Mark::Sticker(_) => Tool::Sticker,
        Mark::Redact(_, Redaction::Blur(_)) => Tool::Blur,
        Mark::Redact(_, Redaction::Pixelate(_)) => Tool::Pixelate,
        Mark::Label(_) => Tool::Text,
        Mark::Redact(_, Redaction::Solid) => return None,
    })
}

/// The controls a mark's style has, if any.
fn row_of(m: &Mark) -> Option<Row> {
    tool_of(m).and_then(Tool::row)
}

/// The index below `n` with the smallest `dist`, or `fallback` when `n`
/// is 0.
fn nearest(n: usize, fallback: u8, dist: impl Fn(usize) -> f32) -> u8 {
    (0..n)
        .min_by(|&a, &b| dist(a).total_cmp(&dist(b)))
        .map_or(fallback, |i| i as u8)
}

/// The edge thickness step nearest the multiple `k`.
fn nearest_edge_width(k: f32) -> u8 {
    nearest(EDGE_WIDTHS.len(), 1, |i| (EDGE_WIDTHS[i] - k).abs())
}

/// The tool that draws marks like `m`.
pub(super) fn mark_tool(m: &Mark) -> Tool {
    tool_of(m).unwrap_or(Tool::Redact)
}

/// The colour index whose entry in `palette` is nearest `c`.
fn nearest_colour(palette: &[Color4f; 8], c: Color4f) -> u8 {
    nearest(palette.len(), 0, |i| {
        let p = palette[i];
        (p.r - c.r).powi(2) + (p.g - c.g).powi(2) + (p.b - c.b).powi(2)
    })
}

/// The size step whose value in `table` (times `scale`) is nearest `v`.
fn nearest_size(v: f32, table: &[f32; SIZES], scale: f32) -> u8 {
    nearest(SIZES, 2, |i| (table[i] * scale - v).abs())
}

/// `c` darkened, for the "darker shade" edge.
fn darker(c: Color4f) -> Color4f {
    Color4f::new(c.r * 0.55, c.g * 0.55, c.b * 0.55, 1.0)
}

/// `opacity` (0 to 1) as a percent on the slider's steps.
fn opacity_percent(opacity: f32) -> u8 {
    let steps = (opacity * 100.0 / f32::from(OPACITY_STEP)).round() as u8;
    (steps * OPACITY_STEP).clamp(MIN_OPACITY, 100)
}

/// The style indexes for a mark's look.
fn look_indexes(look: &Look, color: Color4f) -> (u8, u8) {
    let edge = match look.edge {
        _ if look.auto_edge => EDGE_AUTO,
        None => EDGE_NONE,
        Some(e) if e.r > 0.95 && e.g > 0.95 && e.b > 0.95 => EDGE_WHITE,
        Some(e) if e == darker(color) => EDGE_DARKER,
        Some(e) if e.r < 0.15 && e.g < 0.15 && e.b < 0.15 => EDGE_BLACK,
        Some(_) => EDGE_DARKER,
    };
    (edge, opacity_percent(look.opacity))
}

impl Document {
    pub fn styles(&self) -> Styles {
        self.styles
    }

    /// Restore the styles the shell kept from earlier captures.
    pub fn set_styles(&mut self, styles: Styles) {
        self.styles = styles;
        self.chrome_key = None;
    }

    /// For a reopened capture: the picks carried from other captures, but
    /// its own frame, frame colour and shadow.
    pub fn carry_styles(&mut self, styles: Styles) {
        self.set_styles(Styles {
            framing: self.styles.framing,
            ..styles
        });
    }

    /// The current tool's style.
    pub(crate) fn tool_style(&self) -> Style {
        self.styles.tools[tool_index(self.tool)]
    }

    /// The mark colour preset in use.
    pub fn palette(&self) -> &'static Palette {
        &PALETTES[usize::from(self.styles.palette).min(PALETTES.len() - 1)]
    }

    /// Switch to the next colour preset. Marks already drawn keep their
    /// colours; new ones, and the swatches, use the new preset.
    pub fn cycle_palette(&mut self) -> bool {
        self.styles.palette = ((usize::from(self.styles.palette) + 1) % PALETTES.len()) as u8;
        self.chrome_key = None;
        true
    }

    pub(super) fn colour_of(&self, style: Style) -> Color4f {
        let colours = &self.palette().colours;
        colours[usize::from(style.color).min(colours.len() - 1)]
    }

    /// The arrow width for a style, in image pixels.
    pub(super) fn width_of(&self, style: Style) -> f32 {
        WIDTHS[usize::from(style.size).min(SIZES - 1)] * self.scale
    }

    pub(super) fn text_size_of(&self, style: Style) -> f32 {
        TEXT[usize::from(style.size).min(SIZES - 1)] * self.scale
    }

    /// The edge and opacity a style gives a mark.
    pub(super) fn look_of(&self, style: Style) -> Look {
        let edge = match style.edge {
            EDGE_WHITE | EDGE_AUTO => Some(Color4f::new(1.0, 1.0, 1.0, 1.0)),
            EDGE_BLACK => Some(Color4f::new(0.0, 0.0, 0.0, 1.0)),
            EDGE_DARKER => Some(darker(self.colour_of(style))),
            _ => None,
        };
        Look {
            edge,
            opacity: f32::from(style.opacity.clamp(MIN_OPACITY, 100)) / 100.0,
            edge_width: EDGE_WIDTHS[usize::from(style.edge_width).min(EDGE_WIDTHS.len() - 1)],
            auto_edge: style.edge == EDGE_AUTO,
        }
    }

    pub(super) fn style_for(&self, tool: Tool) -> Style {
        self.styles.tools[tool_index(tool)]
    }

    /// What the style row shows: the selected mark's style if it has one,
    /// else the label being typed, else the mark just drawn, else the
    /// current tool's. `None` hides the row (solid redact has no style).
    pub(crate) fn shown_style(&self) -> Option<(Style, Row)> {
        if !self.also.is_empty() {
            return self.shared_style();
        }
        if let Some(m) = self.styled_mark().and_then(|i| self.marks.get(i))
            && let Some(s) = self.style_of_mark(m)
        {
            let row = row_of(m).unwrap_or(Row::Full);
            return Some((s, row));
        }
        if let Some(l) = &self.typing {
            return self
                .style_of_mark(&Mark::Label(l.clone()))
                .map(|s| (s, Row::Full));
        }
        self.tool.row().map(|row| (self.tool_style(), row))
    }

    /// For several selected marks: the main one's style, with the colour
    /// shown as `MIXED` when they differ, and the row of controls they all
    /// have. `None` when they have none in common.
    fn shared_style(&self) -> Option<(Style, Row)> {
        let mut row: Option<Row> = None;
        let mut style: Option<Style> = None;
        for i in self.selection() {
            let m = self.marks.get(i)?;
            let r = row_of(m)?;
            let s = self.style_of_mark(m)?;
            row = Some(match row {
                None => r,
                Some(prev) if prev == r => r,
                Some(Row::Full | Row::Colours) if matches!(r, Row::Full | Row::Colours) => {
                    Row::Colours
                }
                Some(_) => return None,
            });
            style = Some(match style {
                None => s,
                Some(mut first) => {
                    if first.color != s.color {
                        first.color = MIXED;
                    }
                    first
                }
            });
        }
        Some((style?, row?))
    }

    /// The one mark the style row shows and changes: the selected one, or
    /// else the one just drawn unless a label is being typed.
    fn styled_mark(&self) -> Option<usize> {
        self.selected
            .or_else(|| self.just_drawn().filter(|_| self.typing.is_none()))
    }

    /// The mark just drawn, while it's still the current tool's and
    /// nothing is selected.
    fn just_drawn(&self) -> Option<usize> {
        if self.selected.is_some() || !self.also.is_empty() {
            return None;
        }
        self.last_drawn
            .filter(|&i| self.marks.get(i).and_then(tool_of) == Some(self.tool))
    }

    fn style_of_mark(&self, m: &Mark) -> Option<Style> {
        let s = self.scale;
        let full = |c: Color4f, size: u8, look: &Look| {
            let (edge, opacity) = look_indexes(look, c);
            Style {
                color: nearest_colour(&self.palette().colours, c),
                size,
                edge,
                opacity,
                font: 0,
                edge_width: nearest_edge_width(look.edge_width),
                variant: 0,
            }
        };
        Some(match m {
            Mark::Arrow(a) => full(a.color, nearest_size(a.width, &WIDTHS, s), &a.look),
            Mark::Line(l) => full(l.color, nearest_size(l.width, &WIDTHS, s), &l.look),
            Mark::Box(b) => full(b.color, nearest_size(b.width, &WIDTHS, s), &b.look),
            Mark::Oval(o) => full(o.color, nearest_size(o.width, &WIDTHS, s), &o.look),
            Mark::Stroke(p) => full(p.color, nearest_size(p.width, &WIDTHS, s), &p.look),
            Mark::Label(l) => Style {
                font: l.font.index(),
                variant: l.plate.index(),
                ..full(l.color, nearest_size(l.size, &TEXT, s), &l.look)
            },
            Mark::Step(p) => full(
                p.color,
                nearest_size(p.radius / STEP_RADIUS, &TEXT, s),
                &p.look,
            ),
            Mark::Highlight(h) => Style {
                color: nearest_colour(&self.palette().colours, h.color),
                variant: h.look.index(),
                ..Style::default()
            },
            Mark::Redact(_, Redaction::Blur(level) | Redaction::Pixelate(level)) => Style {
                size: *level,
                ..Style::default()
            },
            Mark::Sticker(_) => Style::default(),
            Mark::Redact(_, Redaction::Solid) => return None,
        })
    }

    /// Give `m` the colour, size, edge, opacity or strength of `style`.
    fn restyle(&self, m: &mut Mark, style: Style) {
        let (c, mut w, mut t, look) = (
            self.colour_of(style),
            self.width_of(style),
            self.text_size_of(style),
            self.look_of(style),
        );
        // A change that isn't to the size keeps the mark's own size, so a
        // label resized by its corners isn't snapped to a size step.
        if self
            .style_of_mark(m)
            .is_some_and(|before| before.size == style.size)
        {
            match m {
                Mark::Arrow(a) => w = a.width,
                Mark::Line(l) => w = l.width,
                Mark::Box(b) => w = b.width,
                Mark::Oval(o) => w = o.width,
                Mark::Stroke(p) => w = p.width,
                Mark::Label(l) => t = l.size,
                Mark::Step(p) => t = p.radius / STEP_RADIUS,
                _ => {}
            }
        }
        match m {
            Mark::Arrow(a) => (a.color, a.width, a.look) = (c, w, look),
            Mark::Line(l) => (l.color, l.width, l.look) = (c, w, look),
            Mark::Box(b) => (b.color, b.width, b.look) = (c, w, look),
            Mark::Oval(o) => (o.color, o.width, o.look) = (c, w, look),
            Mark::Stroke(p) => (p.color, p.width, p.look) = (c, w, look),
            Mark::Highlight(h) => (h.color, h.look) = (c, HighlightLook::nth(style.variant)),
            Mark::Label(l) => {
                // Keep the text centred where it was as it grows or shrinks.
                l.origin += Point::new(0.0, (t - l.size) * 0.35);
                // The same curve at the new size.
                l.bend *= l.size / t;
                (l.color, l.size, l.look) = (c, t, look);
                (l.font, l.plate) = (LabelFont::nth(style.font), Plate::nth(style.variant));
            }
            Mark::Step(p) => (p.color, p.radius, p.look) = (c, t * STEP_RADIUS, look),
            Mark::Redact(_, Redaction::Blur(level) | Redaction::Pixelate(level)) => {
                *level = style.size.min(SIZES as u8 - 1);
            }
            Mark::Redact(_, Redaction::Solid) | Mark::Sticker(_) => {}
        }
    }

    /// Apply a change to the style on show: to the selected mark or the one
    /// just drawn (one undo step), the label being typed, or the current
    /// tool's next marks.
    fn change_style(&mut self, change: impl Fn(&mut Style)) -> bool {
        if self.is_dragging() {
            return false;
        }
        self.apply_style(change, true)
    }

    /// `change_style`, with the undo step left to the caller when
    /// `checkpoint` is false: a slider drag takes one for the whole drag.
    pub(super) fn apply_style(&mut self, change: impl Fn(&mut Style), checkpoint: bool) -> bool {
        // Several selected: each gets the change on its own style, so a
        // colour pick doesn't also unify their sizes.
        if !self.also.is_empty() {
            let mut marks = self.marks.clone();
            let mut changed = false;
            for i in self.selection() {
                let Some(mut style) = marks.get(i).and_then(|m| self.style_of_mark(m)) else {
                    continue;
                };
                let before = style;
                change(&mut style);
                if style != before {
                    self.restyle(&mut marks[i], style);
                    changed = true;
                }
            }
            if !changed || marks == self.marks {
                return false;
            }
            if checkpoint {
                self.checkpoint();
            }
            self.marks = marks;
            self.fresh = false;
            self.chrome_key = None;
            return true;
        }
        let Some((mut style, _)) = self.shown_style() else {
            return false;
        };
        let before = style;
        change(&mut style);
        if style == before {
            return false;
        }
        // New marks of the same kind follow the last pick too.
        let remember = if let Some(i) = self.styled_mark().filter(|&i| i < self.marks.len()) {
            if checkpoint {
                self.checkpoint();
            }
            let mut m = self.marks[i].clone();
            self.restyle(&mut m, style);
            let tool = tool_of(&m);
            self.marks[i] = m;
            tool
        } else if let Some(mut l) = self.typing.take() {
            let mut m = Mark::Label(l.clone());
            self.restyle(&mut m, style);
            if let Mark::Label(styled) = m {
                l = styled;
            }
            self.typing = Some(l);
            Some(Tool::Text)
        } else {
            self.tool.row().is_some().then_some(self.tool)
        };
        if let Some(t) = remember {
            self.styles.tools[tool_index(t)] = style;
        }
        self.fresh = false;
        self.chrome_key = None;
        true
    }

    /// Pick palette colour `index` (0 to 7). Returns whether anything
    /// changed. White marks get a black edge, since a white one wouldn't
    /// show.
    pub fn set_colour(&mut self, index: usize) -> bool {
        if index >= PALETTE.len() || self.shown_style().is_some_and(|(_, r)| r == Row::Strength) {
            return false;
        }
        self.change_style(|s| {
            s.color = index as u8;
            if s.color == WHITE && s.edge == EDGE_WHITE {
                s.edge = EDGE_BLACK;
            }
        })
    }

    /// Pick size (or strength) step `step` (0 to 4).
    #[cfg(test)]
    pub(super) fn set_size(&mut self, step: usize) -> bool {
        if step >= SIZES {
            return false;
        }
        self.change_style(|s| s.size = step as u8)
    }

    /// One size or strength step up (`[`/`]`), stopping at the ends.
    pub(crate) fn step_size(&mut self, bigger: bool) -> bool {
        self.change_style(|s| s.size = stepped(s.size, bigger))
    }

    /// Turn the wheel `notches` over the capture. Each whole notch steps
    /// the size like `[` and `]`; while drawing, it sizes the shape being
    /// drawn.
    pub(super) fn wheel_size(&mut self, notches: f32) -> bool {
        if !notches.is_finite() {
            return false;
        }
        // Reversing drops any leftover fraction. The cap bounds the loop;
        // more steps than there are sizes couldn't change anything.
        if self.wheel_rest * notches < 0.0 {
            self.wheel_rest = 0.0;
        }
        let most = SIZES as f32;
        self.wheel_rest = (self.wheel_rest + notches).clamp(-most, most);
        let mut changed = false;
        while self.wheel_rest.abs() >= 1.0 {
            let bigger = self.wheel_rest > 0.0;
            self.wheel_rest -= self.wheel_rest.signum();
            changed |= match self.drawing() {
                Some(tool) if tool.row().is_some() => {
                    let s = &mut self.styles.tools[tool_index(tool)];
                    let before = s.size;
                    s.size = stepped(s.size, bigger);
                    s.size != before
                }
                Some(_) => false,
                None => self.step_size(bigger),
            };
        }
        if changed {
            self.chrome_key = None;
        }
        changed
    }

    /// The tool whose saved looks the row shows: the one the row is about,
    /// if it has colours or sizes. `None` with several marks selected.
    pub(super) fn quick_tool(&self) -> Option<Tool> {
        if !self.also.is_empty() {
            return None;
        }
        let tool = match self.styled_mark().and_then(|i| self.marks.get(i)) {
            Some(m) => tool_of(m)?,
            None if self.typing.is_some() => Tool::Text,
            None => self.tool,
        };
        matches!(tool.row(), Some(Row::Full | Row::Colours | Row::Strength)).then_some(tool)
    }

    /// The saved looks the row shows.
    pub(super) fn quick_styles(&self) -> Option<[Option<Style>; QUICK_STYLES]> {
        self.quick_tool().map(|t| self.styles.quick[tool_index(t)])
    }

    /// A click on saved look `i`: use it, or keep the current look there
    /// when it's empty or Shift is held.
    pub(super) fn press_quick(&mut self, i: usize) -> bool {
        let (Some(tool), Some((current, _))) = (self.quick_tool(), self.shown_style()) else {
            return false;
        };
        let Some(slot) = self.styles.quick[tool_index(tool)].get_mut(i) else {
            return false;
        };
        match *slot {
            Some(saved) if !self.shift => self.change_style(|s| *s = saved),
            _ => {
                *slot = Some(current);
                self.chrome_key = None;
                true
            }
        }
    }

    /// The sticker the tool stamps: the one picked last.
    pub(super) fn sticker(&self) -> &'static str {
        self.recent_stickers()[0]
    }

    /// The stickers picked lately, newest first.
    pub(super) fn recent_stickers(&self) -> [&'static str; RECENT_STICKERS] {
        self.styles
            .stickers
            .map(|i| STICKERS[usize::from(i) % STICKERS.len()].0)
    }

    /// Pick sticker `index` (into `STICKERS`): the tool stamps it next, it
    /// moves to the front of the row, and selected stickers turn into it
    /// (one undo step). Returns whether anything changed.
    pub fn pick_sticker(&mut self, index: usize) -> bool {
        let Some(&(name, _)) = STICKERS.get(index) else {
            return false;
        };
        let picked = index as u8;
        let before = self.styles.stickers;
        let mut recent = vec![picked];
        recent.extend(before.iter().copied().filter(|i| *i != picked));
        for (slot, i) in self.styles.stickers.iter_mut().zip(recent) {
            *slot = i;
        }
        let swap: Vec<usize> = self
            .selection()
            .into_iter()
            .filter(|&i| matches!(self.marks.get(i), Some(Mark::Sticker(s)) if s.name != name))
            .collect();
        if !swap.is_empty() {
            self.checkpoint();
            for i in swap {
                if let Some(Mark::Sticker(s)) = self.marks.get_mut(i) {
                    s.name = name;
                }
            }
        }
        let changed = before != self.styles.stickers || self.fresh;
        self.fresh = false;
        self.chrome_key = None;
        changed
    }

    /// How many looks the look button steps through: highlight looks for
    /// highlights, plates for labels, `None` for anything else.
    pub(super) fn looks(&self) -> Option<usize> {
        if self.shows_highlight() {
            Some(HighlightLook::ALL.len())
        } else if self.shows_font() {
            Some(Plate::ALL.len())
        } else {
            None
        }
    }

    /// Whether the style row is about highlights: the highlighter, or only
    /// highlights selected.
    pub(super) fn shows_highlight(&self) -> bool {
        if self.typing.is_some() {
            return false;
        }
        let selected = self.selection();
        if selected.is_empty() {
            return self.tool == Tool::Highlight;
        }
        selected
            .iter()
            .all(|&i| matches!(self.marks.get(i), Some(Mark::Highlight(_))))
    }

    /// The next highlight look or plate, round and round.
    pub fn cycle_look(&mut self) -> bool {
        let Some(n) = self.looks() else {
            return false;
        };
        let n = n as u8;
        self.change_style(|s| s.variant = (s.variant + 1) % n)
    }

    /// Whether the style row is about text: the Text tool, a label being
    /// typed, or only labels selected. It then shows the font button.
    pub(super) fn shows_font(&self) -> bool {
        if self.typing.is_some() {
            return true;
        }
        let selected = self.selection();
        if selected.is_empty() {
            return self.tool == Tool::Text;
        }
        selected
            .iter()
            .all(|&i| matches!(self.marks.get(i), Some(Mark::Label(_))))
    }

    /// Use font `index` (into `LabelFont::ALL`) for labels.
    pub(crate) fn set_font(&mut self, index: usize) -> bool {
        if index >= LabelFont::ALL.len() || !self.shows_font() {
            return false;
        }
        self.change_style(|s| s.font = index as u8)
    }

    /// The next font for labels, round and round.
    pub(crate) fn cycle_font(&mut self) -> bool {
        if !self.shows_font() {
            return false;
        }
        let n = LabelFont::ALL.len() as u8;
        self.change_style(|s| s.font = (s.font + 1) % n)
    }

    fn has_looks(&self) -> bool {
        self.shown_style().is_some_and(|(_, r)| r == Row::Full)
    }

    /// Set the edge thickness step (0 thinnest to 4 thickest).
    #[cfg(test)]
    pub(super) fn set_edge_width(&mut self, step: usize) -> bool {
        if step >= EDGE_WIDTHS.len() || !self.has_looks() {
            return false;
        }
        self.change_style(|s| s.edge_width = step as u8)
    }

    /// Pick an edge: 0 white, 1 black, 2 a darker shade, 3 none, 4 auto.
    #[cfg(test)]
    pub(super) fn set_edge(&mut self, edge: usize) -> bool {
        if edge >= EDGES || !self.has_looks() {
            return false;
        }
        self.change_style(|s| s.edge = edge as u8)
    }

    /// The next edge: auto, white, black, darker, none, and round again.
    pub fn cycle_edge(&mut self) -> bool {
        if !self.has_looks() {
            return false;
        }
        self.change_style(|s| s.edge = (s.edge + 1) % EDGES as u8)
    }

    /// Set opacity in percent, rounded to the slider's 5% steps and kept
    /// between `MIN_OPACITY` and 100.
    #[cfg(test)]
    pub(super) fn set_opacity(&mut self, percent: u8) -> bool {
        if !self.has_looks() {
            return false;
        }
        let o = opacity_percent(f32::from(percent) / 100.0);
        self.change_style(|s| s.opacity = o)
    }

    /// Set a slider from a position `t`, 0 at its left end to 1 at its
    /// right, without an undo step.
    pub(super) fn slide(&mut self, slider: Slider, t: f32) -> bool {
        let t = t.clamp(0.0, 1.0);
        match slider {
            Slider::Size => {
                let step = (t * (SIZES - 1) as f32).round() as u8;
                self.apply_style(|s| s.size = step, false)
            }
            Slider::Opacity if self.has_looks() => {
                let range = f32::from(100 - MIN_OPACITY);
                let o = opacity_percent((f32::from(MIN_OPACITY) + t * range) / 100.0);
                self.apply_style(|s| s.opacity = o, false)
            }
            Slider::Opacity => false,
            Slider::Outline if self.has_looks() => {
                let step = (t * (EDGE_WIDTHS.len() - 1) as f32).round() as u8;
                self.apply_style(|s| s.edge_width = step, false)
            }
            Slider::Outline => false,
        }
    }

    /// Where a slider's knob sits, 0 to 1, for the style on show.
    pub(super) fn slider_value(&self, slider: Slider) -> f32 {
        let Some((style, _)) = self.shown_style() else {
            return 0.0;
        };
        match slider {
            Slider::Size => f32::from(style.size) / (SIZES - 1) as f32,
            Slider::Opacity => {
                f32::from(style.opacity.saturating_sub(MIN_OPACITY)) / f32::from(100 - MIN_OPACITY)
            }
            Slider::Outline => f32::from(style.edge_width) / (EDGE_WIDTHS.len() - 1) as f32,
        }
    }
}

/// The style row's sliders. Size is also strength for blur and pixelate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Slider {
    Size,
    Opacity,
    /// Edge thickness.
    Outline,
}

#[cfg(test)]
mod tests {
    use super::super::grips_of;
    use super::super::tests::{click, drag, placed};
    use super::*;

    fn style(color: u8, size: u8) -> Style {
        Style {
            color,
            size,
            ..Style::default()
        }
    }

    #[test]
    fn tools_remember_their_own_style() {
        let (mut d, _) = placed();
        assert_eq!(d.tool_style(), style(0, 2));
        d.set_tool(Tool::Highlight);
        assert_eq!(d.tool_style().color, YELLOW);
        d.set_tool(Tool::Box);
        assert!(d.set_colour(4));
        assert!(d.set_size(4));
        assert!(drag(&mut d, (150.0, 120.0), (250.0, 180.0)));
        let Mark::Box(b) = &d.marks[0] else {
            panic!("not a box")
        };
        assert_eq!(b.color, PALETTE[4]);
        assert_eq!(b.width, 15.0);
        // The arrow still has its own.
        d.set_tool(Tool::Arrow);
        assert_eq!(d.tool_style(), style(0, 2));
        // And the styles carry to the next capture through the shell.
        let kept = d.styles();
        let (mut next, _) = placed();
        next.set_styles(kept);
        next.set_tool(Tool::Box);
        assert_eq!(next.tool_style(), style(4, 4));
        // A reopened capture takes them too, but keeps its own frame.
        let frame = super::super::Frame::builtin("terminal-modern");
        let (mut reopened, _) = placed();
        reopened.set_frame(frame);
        reopened.set_shadow(3);
        reopened.carry_styles(kept);
        reopened.set_tool(Tool::Box);
        assert_eq!(reopened.tool_style(), style(4, 4));
        assert_eq!((reopened.frame(), reopened.shadow()), (frame, 3));
    }

    #[test]
    fn presets_change_new_marks_and_leave_drawn_ones() {
        let (mut d, _) = placed();
        assert_eq!(d.palette().name, "Vivid");
        assert!(drag(&mut d, (120.0, 150.0), (280.0, 150.0)));
        let first = d.marks[0].clone();
        assert!(d.cycle_palette());
        assert_eq!(d.palette().name, "Soft");
        assert!(d.marks[0] == first, "drawn marks keep their colours");
        assert!(drag(&mut d, (120.0, 180.0), (280.0, 180.0)));
        let Mark::Arrow(a) = &d.marks[1] else {
            panic!("not an arrow")
        };
        assert_eq!(a.color, PALETTES[1].colours[0]);
        // Every preset keeps the highlighter's yellow third and white last.
        for p in &PALETTES {
            assert!(p.colours[2].r > 0.9 && p.colours[2].g > 0.75, "{}", p.name);
            assert_eq!(p.colours[7], Color4f::new(1.0, 1.0, 1.0, 1.0));
        }
        assert_eq!(Palette::by_name(" INK "), Some(2));
        assert_eq!(Styles::default().with_palette(9).palette, 2);
    }

    #[test]
    fn picking_restyles_the_selected_mark_as_one_undo_step() {
        let (mut d, _) = placed();
        assert!(drag(&mut d, (120.0, 150.0), (280.0, 150.0)));
        click(&mut d, 200.0, 150.0);
        assert!(d.has_selection());
        assert_eq!(d.shown_style(), Some((style(0, 2), Row::Full)));
        assert!(d.set_colour(3));
        assert!(d.step_size(true));
        let Mark::Arrow(a) = &d.marks[0] else {
            panic!("not an arrow")
        };
        assert_eq!((a.color, a.width), (PALETTE[3], 11.0));
        assert_eq!(d.shown_style(), Some((style(3, 3), Row::Full)));
        assert!(d.undo());
        let Mark::Arrow(a) = &d.marks[0] else {
            panic!("not an arrow")
        };
        assert_eq!(a.color, PALETTE[3], "undo takes back the size first");
        assert!(d.undo());
        let Mark::Arrow(a) = &d.marks[0] else {
            panic!("not an arrow")
        };
        assert_eq!((a.color, a.width), (PALETTE[0], 8.0));
        // Nothing to step past at the ends.
        d.set_tool(Tool::Line);
        assert!(d.set_size(4));
        assert!(!d.step_size(true));
    }

    #[test]
    fn picking_restyles_the_mark_just_drawn() {
        let (mut d, _) = placed();
        assert!(drag(&mut d, (120.0, 150.0), (280.0, 150.0)));
        assert!(!d.has_selection(), "no handles in the way of the next drag");
        assert!(d.set_colour(3));
        assert!(d.step_size(true));
        let Mark::Arrow(a) = &d.marks[0] else {
            panic!("not an arrow")
        };
        assert_eq!((a.color, a.width), (PALETTE[3], 11.0));
        // The next arrow follows the pick too.
        assert_eq!(d.tool_style(), style(3, 3));
        // One undo step each, then the arrow itself.
        assert!(d.undo());
        assert!(d.undo());
        assert_eq!(d.marks.len(), 1);
        assert!(d.undo());
        assert!(d.marks.is_empty());
        // Once the next drag starts, a pick is for the new mark only.
        assert!(drag(&mut d, (120.0, 150.0), (280.0, 150.0)));
        click(&mut d, 20.0, 20.0);
        assert!(d.set_colour(5));
        let Mark::Arrow(a) = &d.marks[0] else {
            panic!("not an arrow")
        };
        assert_eq!(a.color, PALETTE[3], "undo leaves the tool's last pick");
        // Another tool shows its own style, and leaves the arrow be.
        assert!(drag(&mut d, (120.0, 180.0), (280.0, 180.0)));
        d.set_tool(Tool::Box);
        assert!(d.set_colour(1));
        let Mark::Arrow(a) = &d.marks[1] else {
            panic!("not an arrow")
        };
        assert_eq!(a.color, PALETTE[5]);
    }

    #[test]
    fn picking_restyles_a_label_just_finished() {
        let (mut d, _) = placed();
        if d.typeface.is_none() {
            eprintln!("no system fonts; skipping");
            return;
        }
        d.set_tool(Tool::Text);
        d.begin_text(Point::new(150.0, 150.0));
        assert!(d.type_text("Hi"));
        assert!(d.commit_text());
        assert!(d.set_colour(4));
        let Mark::Label(l) = &d.marks[0] else {
            panic!("not a label")
        };
        assert_eq!(l.color, PALETTE[4]);
    }

    #[test]
    fn effects_have_strength_not_colour() {
        let (mut d, _) = placed();
        d.set_tool(Tool::Blur);
        assert_eq!(d.shown_style(), Some((style(0, 2), Row::Strength)));
        assert!(!d.set_colour(1));
        assert!(!d.set_edge(1));
        assert!(d.set_size(4));
        assert!(drag(&mut d, (120.0, 120.0), (200.0, 180.0)));
        assert!(matches!(d.marks[0], Mark::Redact(_, Redaction::Blur(4))));
        // Selected, its strength can change, as one undo step.
        click(&mut d, 160.0, 150.0);
        assert!(d.step_size(false));
        assert!(matches!(d.marks[0], Mark::Redact(_, Redaction::Blur(3))));
        assert!(d.undo());
        assert!(matches!(d.marks[0], Mark::Redact(_, Redaction::Blur(4))));
        // Solid redact has no style at all.
        d.set_tool(Tool::Redact);
        click(&mut d, 20.0, 20.0);
        assert_eq!(d.shown_style(), None);
        d.set_tool(Tool::Highlight);
        assert_eq!(
            d.shown_style(),
            Some((
                Style {
                    color: YELLOW,
                    ..Style::default()
                },
                Row::Colours
            ))
        );
    }

    #[test]
    fn edges_and_opacity() {
        let (mut d, _) = placed();
        assert!(d.set_edge(1));
        assert!(d.set_opacity(50));
        assert!(drag(&mut d, (120.0, 150.0), (280.0, 150.0)));
        let Mark::Arrow(a) = &d.marks[0] else {
            panic!("not an arrow")
        };
        assert_eq!(a.look.edge, Some(Color4f::new(0.0, 0.0, 0.0, 1.0)));
        assert_eq!(a.look.opacity, 0.5);
        // No edge, and the darker shade follows the colour.
        click(&mut d, 200.0, 150.0);
        assert!(d.set_edge(3));
        let Mark::Arrow(a) = &d.marks[0] else {
            panic!("not an arrow")
        };
        assert_eq!(a.look.edge, None);
        assert!(d.set_edge(2));
        assert!(d.set_colour(4));
        let Mark::Arrow(a) = &d.marks[0] else {
            panic!("not an arrow")
        };
        assert_eq!(a.look.edge, Some(darker(PALETTE[4])));
        assert_eq!(
            d.shown_style().map(|(s, _)| (s.edge, s.opacity)),
            Some((2, 50))
        );
        // The edge button cycles round; opacity snaps to 5% and stays
        // visible.
        assert!(d.cycle_edge());
        assert!(d.cycle_edge());
        assert_eq!(d.shown_style().map(|(s, _)| s.edge), Some(EDGE_AUTO));
        assert!(d.cycle_edge());
        assert_eq!(d.shown_style().map(|(s, _)| s.edge), Some(EDGE_WHITE));
        assert!(d.set_opacity(63));
        assert_eq!(d.shown_style().map(|(s, _)| s.opacity), Some(65));
        assert!(d.set_opacity(0));
        assert_eq!(d.shown_style().map(|(s, _)| s.opacity), Some(MIN_OPACITY));
    }

    #[test]
    fn auto_edges_follow_the_background() {
        for (shade, edge) in [(20u8, 1.0), (235u8, 0.0)] {
            let px: Vec<u8> = [shade, shade, shade, 255].repeat(300 * 200);
            let mut d = Document::from_bgra(300, 200, 300 * 4, &px, 1.0).unwrap();
            assert_eq!(d.tool_style().edge, EDGE_AUTO);
            assert!(drag(&mut d, (80.0, 100.0), (220.0, 100.0)));
            let m = &d.marks[0];
            assert!(m.look().is_some_and(|l| l.auto_edge));
            let c = m.auto_edge(&d.base, None);
            assert_eq!((c.r, c.g, c.b), (edge, edge, edge), "over shade {shade}");
        }
    }

    #[test]
    fn white_marks_get_a_black_edge() {
        let (mut d, _) = placed();
        // Auto already picks black over light backgrounds.
        assert!(d.set_colour(usize::from(WHITE)));
        assert_eq!(d.tool_style().edge, EDGE_AUTO);
        assert!(d.cycle_edge());
        assert!(d.set_colour(0));
        assert!(d.set_colour(usize::from(WHITE)));
        assert_eq!(d.tool_style().edge, EDGE_BLACK);
        // An edge picked by hand stays.
        assert!(d.set_edge(3));
        assert!(d.set_colour(0));
        assert!(d.set_colour(usize::from(WHITE)));
        assert_eq!(d.tool_style().edge, EDGE_NONE);
    }

    #[test]
    fn half_opacity_lets_the_background_through() {
        let (mut d, _) = placed();
        let before = d.export();
        d.set_tool(Tool::Box);
        d.set_edge(3);
        d.set_opacity(50);
        drag(&mut d, (110.0, 110.0), (290.0, 190.0));
        let out = d.export();
        // On the box's left stroke the red is mixed half and half with
        // what was there.
        let i = (50 * 200 + 10) * 4;
        let (was, now) = (&before[i..i + 3], &out[i..i + 3]);
        assert_ne!(was, now);
        let red = PALETTE[0];
        let want = |bg: u8, c: f32| (f32::from(bg) * 0.5 + c * 255.0 * 0.5) as i32;
        assert!(
            (i32::from(now[2]) - want(was[2], red.r)).abs() <= 3,
            "{was:?} -> {now:?}"
        );
    }

    #[test]
    fn labels_and_steps_follow_the_size() {
        let (mut d, _) = placed();
        d.set_tool(Tool::Steps);
        d.set_size(4);
        assert!(click(&mut d, 200.0, 150.0));
        let Mark::Step(s) = &d.marks[0] else {
            panic!("not a step")
        };
        assert!((s.radius - 46.0 * STEP_RADIUS).abs() < 0.01);
        // Labels have the Text tool's style, not the current tool's.
        d.begin_text(Point::new(150.0, 120.0));
        d.type_text("Hi");
        assert_eq!(d.typing.as_ref().map(|l| l.size), Some(28.0));
        assert!(d.set_colour(5));
        assert!(d.set_size(3));
        d.commit_text();
        let Mark::Label(l) = &d.marks[1] else {
            panic!("not a label")
        };
        assert_eq!((l.color, l.size), (PALETTE[5], 36.0));
        d.set_tool(Tool::Text);
        assert_eq!(
            d.tool_style(),
            Style {
                color: 5,
                size: 3,
                ..Style::default()
            }
        );
        assert_eq!(d.style_for(Tool::Steps).size, 4);
    }

    #[test]
    fn selecting_a_mark_picks_its_tool_and_picking_a_tool_lets_go() {
        let (mut d, _) = placed();
        d.set_tool(Tool::Box);
        d.set_colour(4);
        d.set_tool(Tool::Arrow);
        assert!(drag(&mut d, (120.0, 150.0), (280.0, 150.0)));
        d.set_tool(Tool::Box);
        // Clicking the arrow selects it and switches to the arrow tool.
        click(&mut d, 200.0, 150.0);
        assert!(d.has_selection());
        assert_eq!(d.tool(), Tool::Arrow);
        // The same tool keeps the selection; another one lets it go and
        // shows its own style.
        d.set_tool(Tool::Arrow);
        assert!(d.has_selection());
        d.set_tool(Tool::Box);
        assert!(!d.has_selection());
        assert_eq!(d.shown_style().map(|(s, _)| s.color), Some(4));
    }

    #[test]
    fn double_clicking_a_label_edits_its_text() {
        let (mut d, _) = placed();
        d.set_tool(Tool::Arrow);
        assert!(drag(&mut d, (120.0, 190.0), (280.0, 190.0)));
        d.begin_text(Point::new(150.0, 150.0));
        d.type_text("Helo");
        d.commit_text();
        // First click selects, the second (no move) edits.
        click(&mut d, 165.0, 150.0);
        assert!(d.has_selection() && !d.is_typing());
        assert!(click(&mut d, 165.0, 150.0));
        assert!(d.is_typing());
        assert_eq!(d.marks.len(), 1, "out of the list while it's edited");
        d.text_key(crate::document::TextKey::End, false, false);
        d.backspace();
        d.type_text("lo");
        d.commit_text();
        let Mark::Label(l) = &d.marks[1] else {
            panic!("not back in its place")
        };
        assert_eq!(l.text, "Hello");
        // One undo puts the old text back.
        assert!(d.undo());
        let Mark::Label(l) = &d.marks[1] else {
            panic!("not a label")
        };
        assert_eq!(l.text, "Helo");
        // Undo mid-edit, or no change at all, leaves it as it was.
        click(&mut d, 165.0, 150.0);
        click(&mut d, 165.0, 150.0);
        d.type_text("!!");
        assert!(d.undo());
        assert_eq!(d.marks.len(), 2);
        click(&mut d, 165.0, 150.0);
        click(&mut d, 165.0, 150.0);
        let undo_depth = d.undo.len();
        assert!(!d.commit_text());
        assert_eq!((d.marks.len(), d.undo.len()), (2, undo_depth));
    }

    fn shift_click(d: &mut Document, x: f32, y: f32) {
        d.set_modifiers(true, true);
        d.begin_drag(Point::new(x, y));
        d.end_drag();
        d.set_modifiers(false, true);
    }

    #[test]
    fn several_marks_restyle_move_and_delete_together() {
        let (mut d, _) = placed();
        d.set_tool(Tool::Arrow);
        assert!(drag(&mut d, (120.0, 120.0), (280.0, 120.0)));
        d.set_tool(Tool::Box);
        d.set_size(4);
        assert!(drag(&mut d, (130.0, 140.0), (200.0, 180.0)));
        d.set_tool(Tool::Highlight);
        assert!(drag(&mut d, (220.0, 140.0), (290.0, 180.0)));
        // Arrow and box: colours, sizes and the rest.
        click(&mut d, 200.0, 120.0);
        shift_click(&mut d, 130.0, 160.0);
        assert_eq!(d.selection().len(), 2);
        assert_eq!(d.shown_style().map(|(_, r)| r), Some(Row::Full));
        // A colour pick changes only the colour: each keeps its own size.
        assert!(d.set_colour(4));
        let (Mark::Arrow(a), Mark::Box(b)) = (&d.marks[0], &d.marks[1]) else {
            panic!("not an arrow and a box")
        };
        assert_eq!((a.color, b.color), (PALETTE[4], PALETTE[4]));
        assert_eq!((a.width, b.width), (8.0, 15.0));
        // One undo for the lot.
        assert!(d.undo());
        let Mark::Arrow(a) = &d.marks[0] else {
            panic!("not an arrow")
        };
        assert_eq!(a.color, PALETTE[0]);
        // With the highlighter too, colours only; differing colours show
        // none chosen.
        click(&mut d, 200.0, 120.0);
        shift_click(&mut d, 130.0, 160.0);
        shift_click(&mut d, 255.0, 160.0);
        assert_eq!(
            d.shown_style().map(|(s, r)| (s.color, r)),
            Some((MIXED, Row::Colours))
        );
        assert!(!d.set_edge(1));
        // Dragging one moves them all, as one undo step.
        assert!(d.begin_drag(Point::new(200.0, 120.0)));
        d.drag_to(Point::new(205.0, 110.0));
        assert!(d.end_drag());
        let Mark::Box(b) = &d.marks[1] else {
            panic!("not a box")
        };
        assert_eq!(b.rect.left, 135.0);
        assert_eq!(d.selection().len(), 3, "still selected after the move");
        // Shift+click again takes one out; Del deletes the rest.
        shift_click(&mut d, 135.0, 150.0);
        assert_eq!(d.selection().len(), 2);
        assert!(d.delete_selected());
        assert_eq!(d.marks.len(), 1);
        assert!(matches!(d.marks[0], Mark::Box(_)));
        assert!(d.undo());
        assert_eq!(d.marks.len(), 3);
    }

    #[test]
    fn enter_starts_a_new_line() {
        use crate::document::{Key, Mods, TextKey::*};
        let (mut d, _) = placed();
        d.begin_text(Point::new(120.0, 150.0));
        d.type_text("one");
        let one = d.typing.as_ref().unwrap().clone();
        d.key(Key::Enter, Mods::default());
        d.type_text("three");
        let l = d.typing.as_ref().unwrap().clone();
        assert_eq!(l.text, "one\nthree");
        assert_eq!(l.lines(), vec![(0, 3), (4, 9)]);
        // The label grows downwards; the first line stays put.
        let tf = d.typeface.clone().unwrap();
        assert!(l.bounds(&tf).bottom > one.bounds(&tf).bottom + l.size);
        assert_eq!(l.bounds(&tf).top, one.bounds(&tf).top);
        // Home and End keep to the caret's line; Up and Down change line.
        d.text_key(Home, false, false);
        assert_eq!(d.caret.at, 4);
        d.text_key(Up, false, false);
        assert_eq!(d.caret.at, 0);
        d.text_key(End, false, false);
        assert_eq!(d.caret.at, 3);
        d.text_key(Down, false, false);
        assert_eq!(l.line_of(d.caret.at), 1);
        // A click on the second line puts the caret there.
        let p = Point::new(l.origin.x + l.advance_to(&tf, 9) + 1.0, l.baseline(1) - 5.0);
        assert_eq!(l.index_at(&tf, p), 9);
        // Esc finishes it, keeping both lines.
        d.key(Key::Escape, Mods::default());
        let Mark::Label(kept) = d.marks.last().unwrap() else {
            panic!("not a label")
        };
        assert_eq!(kept.text, "one\nthree");
    }

    #[test]
    fn the_caret_moves_inserts_deletes_and_selects() {
        use crate::document::TextKey::*;
        let (mut d, _) = placed();
        d.begin_text(Point::new(120.0, 150.0));
        d.type_text("hello world");
        let text = |d: &Document| d.typing.as_ref().unwrap().text.clone();
        // Home, then type: inserts at the front.
        d.text_key(Home, false, false);
        d.type_text("> ");
        assert_eq!(text(&d), "> hello world");
        // Ctrl+Right jumps a word; Delete removes forwards.
        d.text_key(Right, true, false);
        assert_eq!(d.caret.at, 7);
        d.text_key(Delete, false, false);
        assert_eq!(text(&d), "> helloworld");
        // Shift+End selects to the end; typing replaces it.
        d.text_key(End, false, true);
        d.type_text(" there");
        assert_eq!(text(&d), "> hello there");
        // Ctrl+Backspace deletes a word; Ctrl+A then Backspace clears.
        d.text_key(Backspace, true, false);
        assert_eq!(text(&d), "> hello ");
        assert!(d.select_all_text());
        d.backspace();
        assert_eq!(text(&d), "");
    }

    #[test]
    fn clicks_place_the_caret_and_corners_resize_labels() {
        let (mut d, _) = placed();
        let Some(tf) = d.typeface.clone() else {
            return;
        };
        d.begin_text(Point::new(120.0, 150.0));
        d.type_text("abcdef");
        // A click inside the label moves the caret rather than finishing it.
        let l = d.typing.clone().unwrap();
        let mid = l.origin.x + l.advance_to(&tf, 3);
        assert!(d.begin_drag(Point::new(mid, 150.0)));
        // Dragging selects.
        let end = l.origin.x + l.advance_to(&tf, 6);
        d.drag_to(Point::new(end, 150.0));
        d.end_drag();
        assert!(d.is_typing());
        assert_eq!((d.caret.anchor, d.caret.at), (3, 6));
        d.type_text("x");
        assert_eq!(d.typing.as_ref().unwrap().text, "abcx");
        d.commit_text();
        // A double-click puts the caret where it landed.
        let Mark::Label(l) = d.marks[0].clone() else {
            panic!("not a label")
        };
        let at = Point::new(l.origin.x + l.advance_to(&tf, 1), 150.0);
        click(&mut d, at.x, at.y);
        click(&mut d, at.x, at.y);
        assert!(d.is_typing());
        assert_eq!(d.caret.at, 1);
        d.commit_text();
        // Dragging the bottom-right corner down makes it bigger, the
        // top-left corner staying put.
        click(&mut d, at.x, at.y);
        let grips = grips_of(&d.marks[0], Some(&tf), d.scale);
        let (_, corner) = grips[3];
        let before = d.marks[0].outline(Some(&tf)).unwrap();
        assert!(d.begin_drag(corner));
        d.drag_to(corner + Point::new(0.0, 30.0));
        assert!(d.end_drag());
        let Mark::Label(big) = &d.marks[0] else {
            panic!("not a label")
        };
        assert!(big.size > l.size * 1.8, "{} -> {}", l.size, big.size);
        let after = d.marks[0].outline(Some(&tf)).unwrap();
        assert!(
            (after.left - before.left).abs() < 0.5,
            "{before:?} {after:?}"
        );
        assert!((after.top - before.top).abs() < 0.5, "{before:?} {after:?}");
        assert!(d.undo());
        let Mark::Label(back) = &d.marks[0] else {
            panic!("not a label")
        };
        assert_eq!(back.size, l.size);
    }

    #[test]
    fn text_has_a_font_button_and_labels_keep_their_font() {
        let (mut d, _) = placed();
        // Arrows have no font button; the Text tool does.
        assert!(d.row_layout().unwrap().font.is_none());
        d.set_tool(Tool::Text);
        let button = d
            .row_layout()
            .unwrap()
            .font
            .expect("text has a font button");
        assert!(!d.begin_drag(skia_safe::Rect::from(button).center()));
        assert_eq!(LabelFont::nth(d.tool_style().font), LabelFont::Pixel);
        // New labels use it, and it's kept when saved and loaded.
        d.begin_text(Point::new(140.0, 150.0));
        d.type_text("8-bit");
        d.commit_text();
        let Mark::Label(l) = &d.marks[0] else {
            panic!("not a label")
        };
        assert_eq!(l.font, LabelFont::Pixel);
        let json = d.to_json("fonts");
        assert!(json.contains("\"font\": \"pixel\"") || json.contains("\"font\":\"pixel\""));
        let (w, h, pixels) = d.base_pixels();
        let (back, _) = Document::from_saved(&json, w, h, &pixels).unwrap();
        let Mark::Label(l) = &back.marks[0] else {
            panic!("not a label")
        };
        assert_eq!(l.font, LabelFont::Pixel);
        // A selected label changes font from the button too, round to the
        // rounded one after the last.
        click(&mut d, 150.0, 150.0);
        assert!(d.has_selection() && d.row_layout().unwrap().font.is_some());
        assert!(d.cycle_font());
        assert!(d.cycle_font());
        assert!(d.cycle_font());
        let Mark::Label(l) = &d.marks[0] else {
            panic!("not a label")
        };
        assert_eq!(l.font, LabelFont::Rounded);
        // Every bundled font loads, and they really differ.
        let widths: Vec<f32> = LabelFont::ALL
            .iter()
            .map(|f| {
                let tf = f.typeface().expect("bundled font loads");
                skia_safe::Font::from_typeface(tf, 40.0)
                    .measure_str("Spooch", None)
                    .0
            })
            .collect();
        for (i, a) in widths.iter().enumerate() {
            for b in &widths[i + 1..] {
                assert!((a - b).abs() > 0.5, "{widths:?}");
            }
        }
    }

    #[test]
    fn a_font_dropdown_sits_under_the_label() {
        let (mut d, _) = placed();
        if d.typeface.is_none() {
            return;
        }
        assert!(d.picker_layout().is_none(), "no label, no dropdown");
        d.begin_text(Point::new(130.0, 130.0));
        d.type_text("pick me");
        let k = d
            .picker_layout()
            .expect("a dropdown under the label being typed");
        let bounds = d
            .typing
            .as_ref()
            .unwrap()
            .bounds(d.typeface.as_ref().unwrap());
        assert!(k.pill.top as f32 > bounds.bottom && k.list.is_none());
        // Clicking the pill opens the list, without finishing the label.
        assert!(!d.begin_drag(skia_safe::Rect::from(k.pill).center()));
        assert!(d.is_typing());
        let k = d.picker_layout().unwrap();
        assert_eq!(k.options.len(), LabelFont::ALL.len());
        // Picking a font sets it and closes the list.
        d.begin_drag(skia_safe::Rect::from(k.options[3]).center());
        assert_eq!(d.typing.as_ref().unwrap().font, LabelFont::Impact);
        assert!(d.picker_layout().unwrap().list.is_none());
        // A press anywhere else closes an open list.
        let k = d.picker_layout().unwrap();
        d.begin_drag(skia_safe::Rect::from(k.pill).center());
        assert!(d.picker_layout().unwrap().list.is_some());
        d.commit_text();
        d.begin_drag(Point::new(280.0, 190.0));
        d.end_drag();
        assert!(!d.font_menu);
    }

    #[test]
    fn a_style_change_keeps_a_freely_resized_label_size() {
        let (mut d, _) = placed();
        d.begin_text(Point::new(130.0, 150.0));
        d.type_text("Big");
        d.commit_text();
        if let Mark::Label(l) = &mut d.marks[0] {
            l.size = 61.0;
        }
        click(&mut d, 140.0, 150.0);
        assert!(d.set_colour(4));
        assert!(d.set_font(1));
        let Mark::Label(l) = &d.marks[0] else {
            panic!("not a label")
        };
        assert_eq!(
            (l.size, l.color, l.font),
            (61.0, PALETTE[4], LabelFont::Pixel)
        );
        // Changing the size itself still uses the steps.
        assert!(d.set_size(0));
        let Mark::Label(l) = &d.marks[0] else {
            panic!("not a label")
        };
        assert_eq!(l.size, 18.0);
    }

    #[test]
    fn outlines_get_thicker_and_thinner() {
        let (mut d, _) = placed();
        // The outline slider sits after the edge button.
        let row = d.row_layout().unwrap();
        let track = row.outline.expect("arrows have an outline slider");
        assert!(track.left > row.edge.unwrap().right);
        // Dragged to the right end: the thickest step.
        let right = Point::new(
            track.right as f32 - 1.0,
            skia_safe::Rect::from(track).center_y(),
        );
        assert!(d.begin_drag(right));
        d.end_drag();
        assert_eq!(d.tool_style().edge_width, 4);
        assert!(drag(&mut d, (120.0, 150.0), (280.0, 150.0)));
        let Mark::Arrow(a) = &d.marks[0] else {
            panic!("not an arrow")
        };
        assert_eq!(a.look.edge_width, 3.0);
        // A selected mark gets thinner, keeping its own width; one undo.
        click(&mut d, 200.0, 150.0);
        assert!(d.set_edge_width(0));
        let Mark::Arrow(a) = &d.marks[0] else {
            panic!("not an arrow")
        };
        assert_eq!((a.look.edge_width, a.width), (0.5, 8.0));
        assert!(d.undo());
        // Saved and loaded, it keeps its thickness.
        let json = d.to_json("outline");
        let (w, h, pixels) = d.base_pixels();
        let (back, _) = Document::from_saved(&json, w, h, &pixels).unwrap();
        let Mark::Arrow(b) = &back.marks[0] else {
            panic!("not an arrow")
        };
        assert_eq!(b.look.edge_width, 3.0);
        // Effects have no outline.
        d.set_tool(Tool::Blur);
        assert!(d.row_layout().unwrap().outline.is_none());
    }

    #[test]
    fn the_text_tool_places_a_caret_on_click() {
        let (mut d, _) = placed();
        d.set_tool(Tool::Text);
        let p = Point::new(150.0, 150.0);
        assert_eq!(d.cursor_at(p), crate::Cursor::Text);
        assert!(d.begin_drag(p));
        assert!(d.is_typing());
        // Holding the button down drags the caret along.
        assert!(d.drag_to(Point::new(170.0, 160.0)));
        assert!(!d.end_drag());
        let origin = d.typing.as_ref().unwrap().origin;
        assert_eq!(origin.x, 170.0);
        assert!(d.is_typing(), "still typing after letting go");
        d.type_text("Hello");
        // A second click finishes the first label and starts another.
        d.begin_drag(Point::new(150.0, 220.0));
        d.end_drag();
        assert_eq!(d.counts().labels, 1);
        assert!(d.is_typing());
        // A click on a finished label selects it.
        d.commit_text();
        d.begin_drag(Point::new(180.0, 155.0));
        assert!(!d.is_typing() && d.has_selection());
        // So does a click on any other mark, switching to its tool.
        d.set_tool(Tool::Arrow);
        assert!(drag(&mut d, (120.0, 190.0), (280.0, 190.0)));
        d.set_tool(Tool::Text);
        assert!(!d.begin_drag(Point::new(200.0, 190.0)));
        assert!(!d.is_typing() && d.has_selection());
        assert_eq!(d.tool(), Tool::Arrow);
        // Text has no digit key; the others keep theirs.
        assert_eq!(Tool::Text.digit(), None);
        assert_eq!(Tool::from_digit('2'), Some(Tool::Box));
        assert_eq!(Tool::Redact.digit(), Some('0'));
    }
}
