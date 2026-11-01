//! Spooch's editor, drawn with Skia and the same on every platform: the
//! `Document` (marks, tools, undo, the chrome round it, frames and the
//! editor window's view), the marks' own drawing, and the loupe and frame
//! shown while an area is picked. The shells hand it input and show the
//! pixels it draws.

pub use skia_safe;

mod choices;
mod curve;
mod document;
mod select;
mod sticker;
pub use document::{
    Action, Counts, Cursor, Document, Exported, Format, Frame, Gallery, GalleryAction, Key,
    KeyOutcome, LoadError, Mods, PALETTES, Palette, Panel, SHARE_SHORTCUT, SettingsEvent,
    SettingsItem, SettingsPage, Style, Styles, SyncBadge, Tool, WindowButton, WindowPart,
    load_frames, thumbnail,
};
pub use select::{OUTSIDE_BRIGHTNESS, nudge_pointer, selecting_loupe, selection_frame};
pub use sticker::{Art, PointerShape, STICKERS, Sticker};

use crate::document::{fill, stroke};
use std::cell::RefCell;
use std::collections::HashMap;

use skia_safe::{
    AlphaType, BlendMode, BlurStyle, Canvas, Color4f, Font, FontMgr, FontStyle, Image, ImageFilter,
    ImageInfo, MaskFilter, Matrix, Paint, PaintJoin, PaintStyle, Path, PathBuilder, Point, RRect,
    Rect, SamplingOptions, TileMode, Typeface, image_filters, surfaces,
};

/// How a mark is finished: the edge drawn around it so it reads on any
/// background (white by default, as in Skitch), and its opacity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    /// `None` for no edge at all.
    pub edge: Option<Color4f>,
    /// 1.0 is opaque. Applied to the whole mark (fill, edge and shadow
    /// together) by whoever draws it.
    pub opacity: f32,
    /// How thick the edge is, as a multiple of each mark's standard edge.
    pub edge_width: f32,
    /// Pick the edge from what's under the mark: white over dark, black
    /// over light. `edge` is then only a fallback.
    pub auto_edge: bool,
}

impl Default for Look {
    fn default() -> Self {
        Look {
            edge: Some(Color4f::new(1.0, 1.0, 1.0, 1.0)),
            opacity: 1.0,
            edge_width: 1.0,
            auto_edge: false,
        }
    }
}

/// A Skitch-style arrow: a shaft that widens from a fine tail towards a
/// broad, swept-back head, with a white outline and a soft drop shadow so it
/// reads on any background.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Arrow {
    pub from: Point,
    pub to: Point,
    pub color: Color4f,
    /// Nominal thickness in pixels; every proportion derives from it.
    pub width: f32,
    pub look: Look,
    /// How far the middle of the arrow bows out from the straight line:
    /// to the right of the way it points (on screen) when positive, to the
    /// left when negative. 0 is straight.
    pub bend: f32,
}

/// The point a curve from `from` to `to` with `bend` passes through
/// halfway: the middle of the chord, pushed `bend` to its side.
pub fn bend_point(from: Point, to: Point, bend: f32) -> Point {
    let (mid, n) = chord(from, to);
    mid + n * bend
}

/// The `bend` that puts the curve's middle nearest `p`.
pub fn bend_toward(from: Point, to: Point, p: Point) -> f32 {
    let (mid, n) = chord(from, to);
    (p.x - mid.x) * n.x + (p.y - mid.y) * n.y
}

/// The middle of the chord from `from` to `to`, and its unit normal (to
/// the right of the way it points, on screen).
fn chord(from: Point, to: Point) -> (Point, Point) {
    let d = to - from;
    let len = d.length().max(1e-3);
    let mid = Point::new((from.x + to.x) / 2.0, (from.y + to.y) / 2.0);
    (mid, Point::new(-d.y / len, d.x / len))
}

/// An arrow's proportions for width `w` and length `len`, tuned by eye
/// against Skitch. The head never takes more than half the arrow, so short
/// arrows stay arrows.
struct Head {
    len: f32,
    half: f32,
    barb_back: f32,
    tail_half: f32,
    neck_half: f32,
}

impl Head {
    fn new(w: f32, len: f32) -> Head {
        let head_len = (w * 3.6).min(len * 0.5);
        Head {
            len: head_len,
            half: (w * 2.0).min(head_len * 0.75),
            barb_back: head_len * 0.2,
            tail_half: w * 0.2,
            neck_half: w * 0.6,
        }
    }
}

/// The quadratic curve's control point: twice as far out as the point the
/// curve passes through.
fn bend_control(from: Point, to: Point, bend: f32) -> Point {
    bend_point(from, to, bend * 2.0)
}

/// `n + 1` points along the curve, from `from` to `to`.
pub fn curve_points(from: Point, to: Point, bend: f32, n: usize) -> Vec<Point> {
    let c = bend_control(from, to, bend);
    (0..=n)
        .map(|i| {
            let t = i as f32 / n as f32;
            let (a, b, e) = ((1.0 - t) * (1.0 - t), 2.0 * (1.0 - t) * t, t * t);
            Point::new(
                a * from.x + b * c.x + e * to.x,
                a * from.y + b * c.y + e * to.y,
            )
        })
        .collect()
}

/// The centre line of a straight or bent arrow or line.
pub fn curve_path(from: Point, to: Point, bend: f32) -> Path {
    let mut pb = PathBuilder::new();
    pb.move_to(from);
    if bend == 0.0 {
        pb.line_to(to);
    } else {
        pb.quad_to(bend_control(from, to, bend), to);
    }
    pb.detach()
}

impl Arrow {
    /// The filled outline of the arrow, in canvas coordinates.
    pub fn path(&self) -> Path {
        if self.bend != 0.0 {
            return self.curved_path();
        }
        let d = self.to - self.from;
        let len = d.length();
        let mut pb = PathBuilder::new();
        if len < 1.0 {
            return pb.detach();
        }
        let dir = Point::new(d.x / len, d.y / len);
        let normal = Point::new(-dir.y, dir.x);
        let Head {
            len: head_len,
            half: head_half,
            barb_back,
            tail_half,
            neck_half,
        } = Head::new(self.width, len);
        let neck = len - head_len;

        let at = |along: f32, across: f32| {
            Point::new(
                self.from.x + dir.x * along + normal.x * across,
                self.from.y + dir.y * along + normal.y * across,
            )
        };
        pb.move_to(at(0.0, -tail_half))
            .line_to(at(neck, -neck_half))
            .line_to(at(neck - barb_back, -head_half))
            .line_to(at(len, 0.0))
            .line_to(at(neck - barb_back, head_half))
            .line_to(at(neck, neck_half))
            .line_to(at(0.0, tail_half))
            .close();
        pb.detach()
    }

    /// The outline of a bent arrow: the same tapering shaft along the
    /// curve, and the head pointing the way the curve arrives.
    fn curved_path(&self) -> Path {
        let pts = curve_points(self.from, self.to, self.bend, 48);
        let mut cum = vec![0.0f32];
        for w in pts.windows(2) {
            cum.push(cum[cum.len() - 1] + (w[1] - w[0]).length());
        }
        let len = cum[cum.len() - 1];
        let mut pb = PathBuilder::new();
        if len < 1.0 {
            return pb.detach();
        }
        let Head {
            len: head_len,
            half: head_half,
            barb_back,
            tail_half,
            neck_half,
        } = Head::new(self.width, len);
        let neck = len - head_len;
        // The point and direction at `s` along the curve.
        let at = |s: f32| -> (Point, Point) {
            let i = cum
                .windows(2)
                .position(|c| c[1] >= s)
                .unwrap_or(pts.len() - 2);
            let seg = (cum[i + 1] - cum[i]).max(1e-6);
            let t = ((s - cum[i]) / seg).clamp(0.0, 1.0);
            let d = pts[i + 1] - pts[i];
            let dl = d.length().max(1e-6);
            (
                Point::new(pts[i].x + d.x * t, pts[i].y + d.y * t),
                Point::new(d.x / dl, d.y / dl),
            )
        };
        let mut stops: Vec<f32> = cum.iter().copied().filter(|&s| s < neck).collect();
        stops.push(neck);
        let (mut left, mut right) = (Vec::new(), Vec::new());
        for &s in &stops {
            let (p, d) = at(s);
            let n = Point::new(-d.y, d.x);
            let half = tail_half + (neck_half - tail_half) * (s / neck.max(1e-6));
            left.push(Point::new(p.x - n.x * half, p.y - n.y * half));
            right.push(Point::new(p.x + n.x * half, p.y + n.y * half));
        }
        let (np, _) = at(neck);
        let hd = self.to - np;
        let hl = hd.length().max(1e-6);
        let (hd, hn) = (
            Point::new(hd.x / hl, hd.y / hl),
            Point::new(-hd.y / hl, hd.x / hl),
        );
        let barb = |side: f32| {
            Point::new(
                np.x - hd.x * barb_back + hn.x * head_half * side,
                np.y - hd.y * barb_back + hn.y * head_half * side,
            )
        };
        pb.move_to(left[0]);
        for p in &left[1..] {
            pb.line_to(*p);
        }
        pb.line_to(barb(-1.0)).line_to(self.to).line_to(barb(1.0));
        for p in right.iter().rev() {
            pb.line_to(*p);
        }
        pb.close();
        pb.detach()
    }

    pub fn draw(&self, canvas: &Canvas) {
        let w = self.width;
        let edge = (w * 0.25).max(2.0) * self.look.edge_width;
        draw_filled(
            canvas,
            &self.path(),
            self.color,
            self.look.edge,
            edge,
            (w * 0.4, w * 0.25),
        );
    }
}

/// A filled mark the Skitch way: a soft shadow (`blur`, dropped `drop`),
/// an edge `edge_width` wide unless there's none, then the fill.
fn draw_filled(
    canvas: &Canvas,
    path: &Path,
    color: Color4f,
    edge: Option<Color4f>,
    edge_width: f32,
    (blur, drop): (f32, f32),
) {
    canvas.save();
    canvas.translate((0.0, drop));
    canvas.draw_path(path, &shadow_paint(blur));
    canvas.restore();
    if let Some(e) = edge {
        let mut outline = stroke(e, edge_width);
        outline.set_stroke_join(PaintJoin::Round);
        canvas.draw_path(path, &outline);
    }
    canvas.draw_path(path, &fill(color));
}

/// Where the caret is in a label being typed, in characters, and the
/// other end of the selection (the same when nothing is selected).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Caret {
    pub at: usize,
    pub anchor: usize,
}

/// A Skitch-style text label: bold, in the annotation color, with the same
/// white outline and soft shadow as the arrow so it reads on any background.
#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    /// Left end of the baseline, in canvas coordinates.
    pub origin: Point,
    pub text: String,
    pub color: Color4f,
    /// Font size in pixels.
    pub size: f32,
    pub look: Look,
    /// Rotation about the middle of its box, in radians, clockwise.
    pub angle: f32,
    pub font: LabelFont,
    /// How far the baseline curves, as one over the arc's radius: 0 is
    /// straight, positive bends the middle down (a smile), negative up (a
    /// rainbow). Bent lines are centred on the first line's middle.
    pub bend: f32,
    /// What the text sits on. Bent labels are drawn without it.
    pub plate: Plate,
}

choices::choices! {
    /// What a label sits on.
    pub enum Plate {
        /// Nothing: the text has an edge, like the other marks.
        #[default]
        Outlined => ("Outlined", "outlined"),
        /// A white rounded box.
        Bubble => ("Bubble", "bubble"),
        /// A box in the label's colour.
        Tag => ("Tag", "tag"),
        /// A dark plate.
        Badge => ("Badge", "badge"),
    }
}

impl Plate {
    /// The plate's colour and the text's on it, for a label in `colour`;
    /// `None` for no plate. The text turns black or white wherever its own
    /// colour wouldn't read.
    pub fn colours(self, colour: Color4f) -> Option<(Color4f, Color4f)> {
        const WHITE: Color4f = Color4f::new(1.0, 1.0, 1.0, 1.0);
        const INK: Color4f = Color4f::new(0.08, 0.085, 0.1, 1.0);
        const DARK: Color4f = Color4f::new(0.11, 0.118, 0.137, 1.0);
        let l = luminance(colour);
        let (light, dark) = (l > 0.6, l < 0.2);
        match self {
            Plate::Outlined => None,
            Plate::Bubble => Some((WHITE, if light { INK } else { colour })),
            Plate::Tag => Some((colour, if light { INK } else { WHITE })),
            Plate::Badge => Some((DARK, if dark { WHITE } else { colour })),
        }
    }

    /// Corner radius for a plate `height` tall, from text `size`.
    fn radius(self, height: f32, size: f32) -> f32 {
        match self {
            Plate::Bubble => height * 0.35,
            Plate::Tag => size * 0.18,
            Plate::Outlined | Plate::Badge => size * 0.1,
        }
    }
}

/// Roughly how light `c` looks, 0 to 1.
fn luminance(c: Color4f) -> f32 {
    0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b
}

choices::choices! {
    /// The fonts labels can use, all bundled (SIL Open Font License, see
    /// `fonts/OFL-*.txt`) so labels look the same on every platform.
    pub enum LabelFont {
        /// Nunito Black: heavy and rounded, like Skitch's labels.
        #[default]
        Rounded => ("Nunito", "rounded"),
        /// Pixelify Sans Bold: a scalable pixel-art font.
        Pixel => ("Pixelify Sans", "pixel"),
        /// Tiny5: a minimal pixel font on a five-pixel grid.
        Tiny => ("Tiny5", "tiny"),
        /// Anton: tall and condensed, like Impact, for memes.
        Impact => ("Anton", "impact"),
    }
}

impl LabelFont {
    /// The typeface, made once per thread.
    pub fn typeface(self) -> Option<Typeface> {
        match self {
            LabelFont::Rounded => bundled(NUNITO, 900.0),
            LabelFont::Pixel => bundled(PIXELIFY, 700.0),
            // Not a variable font: the weight is ignored.
            LabelFont::Tiny => bundled(TINY5, 400.0),
            LabelFont::Impact => bundled(ANTON, 400.0),
        }
    }
}

/// Nunito, a variable font: labels use its heaviest weight and the chrome
/// a bold one.
const NUNITO: &[u8] = include_bytes!("../fonts/Nunito.ttf");
/// Pixelify Sans, a variable pixel-art font; labels use it bold.
const PIXELIFY: &[u8] = include_bytes!("../fonts/PixelifySans.ttf");
/// Tiny5, a pixel font with one weight.
const TINY5: &[u8] = include_bytes!("../fonts/Tiny5.ttf");
/// Anton, an Impact lookalike with one weight.
const ANTON: &[u8] = include_bytes!("../fonts/Anton.ttf");

/// The bundled variable font `data` at weight `weight`, or the bold system
/// sans should it fail to load. Made once per thread, font and weight.
fn bundled(data: &'static [u8], weight: f32) -> Option<Typeface> {
    /// Typefaces made so far, by font data address and weight.
    type Faces = Vec<((usize, u32), Option<Typeface>)>;
    thread_local! {
        static FACES: RefCell<Faces> = const { RefCell::new(Vec::new()) };
    }
    let key = (data.as_ptr() as usize, weight.to_bits());
    FACES.with(|faces| {
        if let Some((_, tf)) = faces.borrow().iter().find(|(k, _)| *k == key) {
            return tf.clone();
        }
        let mgr = FontMgr::default();
        let coords = [skia_safe::font_arguments::variation_position::Coordinate {
            axis: skia_safe::FourByteTag::from_chars('w', 'g', 'h', 't'),
            value: weight,
        }];
        let args = skia_safe::FontArguments::new().set_variation_design_position(
            skia_safe::font_arguments::VariationPosition {
                coordinates: &coords,
            },
        );
        let tf = mgr
            .new_from_data(skia_safe::Data::new_copy(data), None)
            .and_then(|base| base.clone_with_arguments(&args))
            .or_else(|| mgr.legacy_make_typeface(None, FontStyle::bold()));
        faces.borrow_mut().push((key, tf.clone()));
        tf
    })
}

/// A system font with a glyph for `c`, for text a label's font can't draw
/// (scripts it lacks, symbols, emoji). Cached per character.
fn fallback(c: char) -> Option<Typeface> {
    thread_local! {
        static FALLBACKS: RefCell<HashMap<char, Option<Typeface>>> = RefCell::new(HashMap::new());
    }
    FALLBACKS.with(|f| {
        f.borrow_mut()
            .entry(c)
            .or_insert_with(|| {
                FontMgr::default().match_family_style_character(
                    "",
                    FontStyle::bold(),
                    &[],
                    c as i32,
                )
            })
            .clone()
    })
}

/// `text` split into runs that `typeface` can draw and runs that need a
/// fallback, each with the typeface to draw it in.
fn runs(text: &str, typeface: &Typeface) -> Vec<(Typeface, String)> {
    let mut out: Vec<(Typeface, String)> = Vec::new();
    for c in text.chars() {
        let tf = if c.is_whitespace() || typeface.unichar_to_glyph(c as i32) != 0 {
            typeface.clone()
        } else {
            fallback(c).unwrap_or_else(|| typeface.clone())
        };
        match out.last_mut() {
            Some((last, run)) if last.unique_id() == tf.unique_id() => run.push(c),
            _ => out.push((tf, c.to_string())),
        }
    }
    out
}

/// Laid-out label text: one path for everything with outlines, plus runs
/// of colour glyphs (emoji), which have no outline and are drawn as they
/// are.
struct TextLayout {
    path: Path,
    /// Each colour run with where it goes: its baseline's left end at the
    /// origin, moved (and on a curve, turned) by the matrix.
    colour: Vec<(Font, String, Matrix)>,
}

/// One character of a label: its line, how far along the line its left
/// edge is, how wide it is, and its font.
struct Letter {
    line: usize,
    x: f32,
    advance: f32,
    font: Font,
    ch: char,
}

/// A label's lines are this many times its size apart.
pub(crate) const LINE_HEIGHT: f32 = 1.2;

/// `text` starting at `origin`, at `size` pixels, falling back to system
/// fonts for characters `typeface` lacks. Each line (split at `\n`) sits
/// `LINE_HEIGHT` below the one before; `advance` is the widest.
/// `layout_text`'s advance, without building glyph outlines: hit-testing
/// and caret placement run this on every pointer move.
fn text_advance(text: &str, size: f32, typeface: &Typeface) -> f32 {
    text.split('\n')
        .map(|line| {
            runs(line, typeface)
                .into_iter()
                .map(|(tf, run)| Font::from_typeface(tf, size).measure_str(&run, None).0)
                .sum::<f32>()
        })
        .fold(0.0, f32::max)
}

fn layout_text(text: &str, origin: Point, size: f32, typeface: &Typeface) -> TextLayout {
    let mut pb = PathBuilder::new();
    let mut colour = Vec::new();
    for (i, line) in text.split('\n').enumerate() {
        let y = origin.y + i as f32 * size * LINE_HEIGHT;
        let mut x = origin.x;
        for (tf, run) in runs(line, typeface) {
            let font = Font::from_typeface(tf, size);
            let at = Point::new(x, y);
            let path = Path::from_str(&run, at, &font);
            if path.is_empty() && !run.trim().is_empty() {
                colour.push((font.clone(), run.clone(), Matrix::translate(at)));
            } else {
                pb.add_path(&path, None);
            }
            x += font.measure_str(&run, None).0;
        }
    }
    TextLayout {
        path: pb.detach(),
        colour,
    }
}

impl Label {
    /// The standard labels' typeface: Nunito Black. Made once per thread.
    pub fn typeface() -> Option<Typeface> {
        LabelFont::Rounded.typeface()
    }

    /// The chrome's typeface: Nunito Bold.
    pub fn ui_typeface() -> Option<Typeface> {
        bundled(NUNITO, 700.0)
    }

    /// The label's own typeface, or `fallback` should it be missing.
    fn face(&self, fallback: &Typeface) -> Typeface {
        self.font.typeface().unwrap_or_else(|| fallback.clone())
    }

    /// Each line's first and one-past-last character, as counts of
    /// characters (the `\n` between lines belongs to neither).
    pub fn lines(&self) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let mut start = 0;
        for line in self.text.split('\n') {
            let end = start + line.chars().count();
            out.push((start, end));
            start = end + 1;
        }
        out
    }

    /// Which line character boundary `chars` is on.
    pub fn line_of(&self, chars: usize) -> usize {
        self.lines()
            .iter()
            .position(|&(_, end)| chars <= end)
            .unwrap_or(0)
    }

    /// The baseline of line `line`.
    pub fn baseline(&self, line: usize) -> f32 {
        self.origin.y + line as f32 * self.size * LINE_HEIGHT
    }

    /// Every character but the line breaks, laid out straight.
    fn letters(&self, typeface: &Typeface) -> Vec<Letter> {
        let mut out = Vec::new();
        for (line, text) in self.text.split('\n').enumerate() {
            let mut x = 0.0;
            for (tf, run) in runs(text, typeface) {
                let font = Font::from_typeface(tf, self.size);
                for ch in run.chars() {
                    let advance = font.measure_str(ch.encode_utf8(&mut [0; 4]), None).0;
                    out.push(Letter {
                        line,
                        x,
                        advance,
                        font: font.clone(),
                        ch,
                    });
                    x += advance;
                }
            }
        }
        out
    }

    /// Each line's width, straight.
    fn line_widths(&self, typeface: &Typeface) -> Vec<f32> {
        let mut widths = vec![0.0; self.lines().len()];
        for l in self.letters(typeface) {
            widths[l.line] = l.x + l.advance;
        }
        widths
    }

    /// The middle of the first line's baseline, which stays put as the
    /// label bends.
    pub fn middle(&self, typeface: &Typeface) -> Point {
        let typeface = &self.face(typeface);
        let widest = self.line_widths(typeface).into_iter().fold(0.0, f32::max);
        Point::new(self.origin.x + widest / 2.0, self.origin.y)
    }

    /// How long the bend is measured over: the widest line.
    pub fn bend_width(&self, typeface: &Typeface) -> f32 {
        let typeface = &self.face(typeface);
        self.line_widths(typeface).into_iter().fold(1.0, f32::max)
    }

    /// Where a point `t` along line `line` from its middle sits on the
    /// curve, and which way the line runs there (radians, clockwise).
    fn on_curve(&self, middle: Point, line: usize, t: f32) -> (Point, f32) {
        let sign = self.bend.signum();
        let radius = 1.0 / self.bend.abs();
        let centre = middle + Point::new(0.0, -sign * radius);
        let r = (radius + sign * line as f32 * self.size * LINE_HEIGHT).max(self.size * 0.5);
        let a = t / r;
        (
            centre + Point::new(r * a.sin(), sign * r * a.cos()),
            -sign * a,
        )
    }

    /// The curved layout: each letter upright on the curve, turned to run
    /// along it, centred on its line's middle.
    fn curved(&self, typeface: &Typeface) -> TextLayout {
        let widths = self.line_widths(typeface);
        let middle = self.middle(typeface);
        let mut pb = PathBuilder::new();
        let mut colour = Vec::new();
        for l in self.letters(typeface) {
            if l.ch.is_whitespace() {
                continue;
            }
            let t = l.x + l.advance / 2.0 - widths[l.line] / 2.0;
            let (at, turn) = self.on_curve(middle, l.line, t);
            let m = Matrix::translate(at)
                * Matrix::rotate_rad(turn)
                * Matrix::translate((-l.advance / 2.0, 0.0));
            let glyph = l.font.str_to_glyphs_vec(l.ch.encode_utf8(&mut [0; 4]));
            match glyph.first().and_then(|g| l.font.get_path(*g)) {
                Some(path) if !path.is_empty() => {
                    pb.add_path(&path.make_transform(&m), None);
                }
                _ => colour.push((l.font.clone(), l.ch.to_string(), m)),
            }
        }
        TextLayout {
            path: pb.detach(),
            colour,
        }
    }

    /// The label laid out, straight or on its curve.
    fn layout(&self, typeface: &Typeface) -> TextLayout {
        if self.bend == 0.0 {
            layout_text(&self.text, self.origin, self.size, typeface)
        } else {
            self.curved(typeface)
        }
    }

    /// Where character boundary `chars` sits on its baseline, and which way
    /// the line runs there: the caret's foot.
    fn boundary(&self, typeface: &Typeface, chars: usize) -> (Point, f32) {
        let line = self.line_of(chars);
        let x = self.advance_to(typeface, chars);
        if self.bend == 0.0 {
            return (Point::new(self.origin.x + x, self.baseline(line)), 0.0);
        }
        let face = &self.face(typeface);
        let width = self.line_widths(face)[line];
        self.on_curve(self.middle(typeface), line, x - width / 2.0)
    }

    /// What the label turns about: the middle of its box when straight, so
    /// bending it doesn't move it.
    pub fn pivot(&self, typeface: &Typeface) -> Point {
        Label {
            bend: 0.0,
            ..self.clone()
        }
        .bounds(typeface)
        .center()
    }

    /// How far from `origin.x` character boundary `chars` sits, on its
    /// line.
    pub fn advance_to(&self, typeface: &Typeface, chars: usize) -> f32 {
        let typeface = &self.face(typeface);
        let (start, _) = self.lines()[self.line_of(chars)];
        let byte = |i: usize| {
            self.text
                .char_indices()
                .nth(i)
                .map_or(self.text.len(), |(b, _)| b)
        };
        text_advance(&self.text[byte(start)..byte(chars)], self.size, typeface)
    }

    /// The character boundary nearest `p`: on the line nearest its height,
    /// the closest one across (on a curve, the nearest caret).
    pub fn index_at(&self, typeface: &Typeface, p: Point) -> usize {
        if self.bend != 0.0 {
            let n = self.text.chars().count();
            let caret_mid = |i: usize| {
                let (foot, turn) = self.boundary(typeface, i);
                foot + Point::new(turn.sin(), -turn.cos()) * (self.size * 0.35)
            };
            return (0..=n)
                .min_by(|&a, &b| {
                    (caret_mid(a) - p)
                        .length()
                        .total_cmp(&(caret_mid(b) - p).length())
                })
                .unwrap_or(n);
        }
        let lines = self.lines();
        let line = ((p.y - self.origin.y) / (self.size * LINE_HEIGHT) + 0.5)
            .floor()
            .clamp(0.0, (lines.len() - 1) as f32) as usize;
        let (start, end) = lines[line];
        (start..=end)
            .min_by(|&a, &b| {
                let da = (self.origin.x + self.advance_to(typeface, a) - p.x).abs();
                let db = (self.origin.x + self.advance_to(typeface, b) - p.x).abs();
                da.total_cmp(&db)
            })
            .unwrap_or(end)
    }

    /// The plate drawn, if any: straight labels only.
    fn shown_plate(&self) -> Option<Plate> {
        (self.bend == 0.0 && self.plate != Plate::Outlined).then_some(self.plate)
    }

    /// The box the label occupies: its widest line, from the first line's
    /// cap height to just below the last baseline, and its plate round
    /// that. On a curve, the box round the letters.
    pub fn bounds(&self, typeface: &Typeface) -> Rect {
        let text = self.text_bounds(typeface);
        match self.shown_plate() {
            Some(_) => text.with_outset((self.size * 0.3, self.size * 0.12)),
            None => text,
        }
    }

    fn text_bounds(&self, typeface: &Typeface) -> Rect {
        let typeface = &self.face(typeface);
        if self.bend != 0.0 {
            let laid = self.curved(typeface);
            let mut b = laid.path.compute_tight_bounds();
            for (_, _, m) in &laid.colour {
                let glyph = Rect::from_ltrb(0.0, -self.size * 0.9, self.size, self.size * 0.2);
                b = if b.is_empty() {
                    m.map_rect(glyph).0
                } else {
                    Rect::join2(b, m.map_rect(glyph).0)
                };
            }
            if !b.is_empty() {
                return b.with_outset((self.size * 0.05, self.size * 0.05));
            }
        }
        let advance = text_advance(&self.text, self.size, typeface);
        let last = self.baseline(self.lines().len() - 1);
        Rect::from_ltrb(
            self.origin.x,
            self.origin.y - self.size * 0.8,
            self.origin.x + advance,
            last + self.size * 0.25,
        )
    }

    /// Draw the label, with its caret and any selected text while it is
    /// being typed.
    pub fn draw(&self, canvas: &Canvas, typeface: &Typeface, caret: Option<Caret>) {
        let typeface = &self.face(typeface);
        let s = self.size;
        // The plate first, under any selection.
        let plated = self
            .shown_plate()
            .and_then(|p| Some((p, p.colours(self.color)?)));
        if let Some((plate, (behind, _))) = plated {
            let r = self.bounds(typeface);
            let radius = plate.radius(r.height(), s);
            let mut pb = PathBuilder::new();
            pb.add_rrect(RRect::new_rect_xy(r, radius, radius), None, None);
            draw_filled(
                canvas,
                &pb.detach(),
                behind,
                None,
                0.0,
                (s * 0.12, s * 0.06),
            );
        }
        // Text as a path, so outline, shadow and fill line up exactly.
        let text = self.layout(typeface);
        let mut path = text.path;
        if let Some(c) = caret
            && c.anchor != c.at
            && self.bend != 0.0
        {
            // On a curve, a band behind each selected letter.
            let (a, b) = (c.at.min(c.anchor), c.at.max(c.anchor));
            let sel = fill(document::with_alpha(document::SELECTION, 0.35));
            let lines = self.lines();
            for i in a..b {
                let (foot, turn) = self.boundary(typeface, i);
                let next = if lines.iter().any(|&(_, end)| end == i) {
                    s * 0.25
                } else {
                    (self.boundary(typeface, i + 1).0 - foot).length()
                };
                let m = Matrix::translate(foot) * Matrix::rotate_rad(turn);
                canvas.draw_rect(
                    m.map_rect(Rect::from_ltrb(0.0, -s * 0.85, next, s * 0.25))
                        .0,
                    &sel,
                );
            }
        } else if let Some(c) = caret
            && c.anchor != c.at
        {
            let (a, b) = (c.at.min(c.anchor), c.at.max(c.anchor));
            let sel = fill(document::with_alpha(document::SELECTION, 0.35));
            // A band on each line the selection reaches; a line break it
            // takes in shows as a little extra at the line's end.
            for (i, (start, end)) in self.lines().into_iter().enumerate() {
                if b < start || a > end {
                    continue;
                }
                let (from, to) = (a.max(start), b.min(end));
                let tail = if b > end { s * 0.25 } else { 0.0 };
                let y = self.baseline(i);
                canvas.draw_rect(
                    Rect::from_ltrb(
                        self.origin.x + self.advance_to(typeface, from),
                        y - s * 0.85,
                        self.origin.x + self.advance_to(typeface, to) + tail,
                        y + s * 0.25,
                    ),
                    &sel,
                );
            }
        }
        if let Some(c) = caret {
            // Just past the text at a line's end; between letters elsewhere.
            let line = self.line_of(c.at);
            let at_end = c.at >= self.lines()[line].1;
            let nudge = if at_end { s * 0.06 } else { -s * 0.04 };
            let (foot, turn) = self.boundary(typeface, c.at);
            let mut pb = PathBuilder::new();
            pb.add_rect(
                Rect::from_ltrb(nudge, -s * 0.8, nudge + (s * 0.08).max(2.0), s * 0.15),
                None,
                None,
            );
            let bar = pb
                .detach()
                .make_transform(&(Matrix::translate(foot) * Matrix::rotate_rad(turn)));
            path = path.op(&bar, skia_safe::PathOp::Union).unwrap_or(path);
        }
        if path.is_empty() && text.colour.is_empty() {
            return;
        }
        let ink = Paint::new(Color4f::new(0.0, 0.0, 0.0, 1.0), None);
        for (font, run, m) in &text.colour {
            canvas.save();
            canvas.concat(m);
            canvas.draw_str(run, (0, 0), font, &ink);
            canvas.restore();
        }

        // On a plate the text needs no edge or shadow.
        if let Some((_, (_, on))) = plated {
            canvas.draw_path(&path, &fill(on));
            return;
        }
        // Text's counters fill in fast, so thick edges grow half as
        // quickly on labels.
        let k = self.look.edge_width;
        let k = if k > 1.0 { 1.0 + (k - 1.0) * 0.5 } else { k };
        let edge = (s * 0.16).max(2.0) * k;
        draw_filled(
            canvas,
            &path,
            self.color,
            self.look.edge,
            edge,
            (s * 0.08, s * 0.05),
        );
    }
}

/// The app icon at `size` pixels square, for the tray: a Skitch-red
/// rounded square with a white arrow. Returns unpremultiplied BGRA rows
/// (alpha straight, as Windows icons and X11 want).
pub fn app_icon(size: u32) -> Vec<u8> {
    let n = size.max(8) as i32;
    let s = n as f32;
    draw_bgra((n, n), AlphaType::Unpremul, |canvas| {
        let r = s * 0.22;
        canvas.draw_rrect(
            RRect::new_rect_xy(Rect::from_wh(s, s).with_inset((s * 0.04, s * 0.04)), r, r),
            &fill(Color4f::new(0.93, 0.16, 0.16, 1.0)),
        );
        let arrow = Arrow {
            from: Point::new(s * 0.22, s * 0.78),
            to: Point::new(s * 0.80, s * 0.20),
            color: Color4f::new(1.0, 1.0, 1.0, 1.0),
            width: s * 0.11,
            look: Look::default(),
            bend: 0.0,
        };
        canvas.draw_path(&arrow.path(), &fill(arrow.color));
    })
}

/// The countdown badge's side, in screen pixels at 100%.
pub const COUNTDOWN_PX: u32 = 120;

/// The countdown shown before a delayed capture: `n` on a dark rounded
/// square, `size` pixels across. Premultiplied BGRA rows; on an opaque
/// window, the corners show `behind`.
pub fn countdown_badge(n: u32, size: u32, behind: Option<Color4f>) -> Vec<u8> {
    let s = size.max(16) as i32;
    let f = s as f32;
    draw_bgra((s, s), AlphaType::Premul, |canvas| {
        if let Some(c) = behind {
            canvas.clear(c);
        }
        let r = f * 0.24;
        canvas.draw_rrect(
            RRect::new_rect_xy(Rect::from_wh(f, f), r, r),
            &fill(Color4f::new(0.08, 0.085, 0.1, 0.92)),
        );
        if let Some(tf) = Label::ui_typeface() {
            let font = Font::from_typeface(tf, f * 0.6);
            let text = n.to_string();
            let (w, bounds) = font.measure_str(&text, None);
            canvas.draw_str(
                &text,
                ((f - w) / 2.0, document::ink_baseline(&bounds, f / 2.0)),
                &font,
                &fill(Color4f::new(1.0, 1.0, 1.0, 1.0)),
            );
        }
    })
}

/// BGRA pixels of `size`, with `alpha`.
pub(crate) fn bgra_info(size: (i32, i32), alpha: AlphaType) -> ImageInfo {
    ImageInfo::new(size, skia_safe::ColorType::BGRA8888, alpha, None)
}

/// `draw` on a clear surface of `size`, read back as tightly packed BGRA
/// with `alpha`. All zeros if Skia can't make the surface.
pub(crate) fn draw_bgra(size: (i32, i32), alpha: AlphaType, draw: impl FnOnce(&Canvas)) -> Vec<u8> {
    let mut out = vec![0u8; (size.0 * size.1 * 4) as usize];
    if let Some(mut surface) = surfaces::raster(&bgra_info(size, AlphaType::Premul), None, None) {
        let canvas = surface.canvas();
        canvas.clear(Color4f::new(0.0, 0.0, 0.0, 0.0));
        draw(canvas);
        surface.read_pixels(
            &bgra_info(size, alpha),
            &mut out,
            size.0 as usize * 4,
            (0, 0),
        );
    }
    out
}

/// JPEG from tightly packed opaque BGRA rows, at quality 0-100.
pub fn encode_jpeg(width: u32, height: u32, bgra: &[u8], quality: u32) -> Option<Vec<u8>> {
    // JPEG has no transparency: anything see-through (a frame's shadow and
    // corners) goes onto white. The pixels are straight, not premultiplied.
    let opaque = bgra.as_chunks::<4>().0.iter().all(|px| px[3] == 255);
    let info = skia_safe::ImageInfo::new(
        (width as i32, height as i32),
        skia_safe::ColorType::BGRA8888,
        if opaque {
            skia_safe::AlphaType::Opaque
        } else {
            skia_safe::AlphaType::Unpremul
        },
        None,
    );
    let mut image = skia_safe::images::raster_from_data(
        &info,
        skia_safe::Data::new_copy(bgra),
        width as usize * 4,
    )?;
    if !opaque {
        let mut surface = surfaces::raster_n32_premul((width as i32, height as i32))?;
        surface.canvas().clear(Color4f::new(1.0, 1.0, 1.0, 1.0));
        surface.canvas().draw_image(&image, (0, 0), None);
        image = surface.image_snapshot();
    }
    let data = image.encode(None, skia_safe::EncodedImageFormat::JPEG, quality)?;
    Some(data.as_bytes().to_vec())
}

/// Load what the editor needs ahead of the first capture (call on the UI
/// thread at startup), so opening the editor doesn't wait on font matching.
pub fn warm_up() {
    for f in LabelFont::ALL {
        let _ = f.typeface();
    }
    let _ = Label::ui_typeface();
}

/// A Skitch-style box: a thick rounded outline in the annotation color, with
/// a thin white edge and a soft shadow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Boxed {
    pub rect: Rect,
    pub color: Color4f,
    /// The arrow width it is drawn next to; the stroke derives from it.
    pub width: f32,
    pub look: Look,
    /// Corner radius, or `None` for the standard slight rounding that
    /// follows the stroke.
    pub radius: Option<f32>,
    /// Rotation about its middle, in radians, clockwise.
    pub angle: f32,
}

impl Boxed {
    /// The corner radius drawn: the one set, or the standard one, never
    /// more than half the shorter side.
    pub fn corner_radius(&self) -> f32 {
        let stroke = stroke_of(self.width);
        let max = self.rect.width().min(self.rect.height()).abs() / 2.0;
        self.radius.unwrap_or(stroke * 1.2).clamp(0.0, max)
    }

    pub fn draw(&self, canvas: &Canvas) {
        let r = self.corner_radius();
        let mut pb = PathBuilder::new();
        pb.add_rrect(RRect::new_rect_xy(self.rect, r, r), None, None);
        stroke_outlined(
            canvas,
            &pb.detach(),
            self.color,
            self.look,
            self.width,
            false,
        );
    }
}

/// A stroked mark's line thickness for its arrow width `width`.
pub(crate) fn stroke_of(width: f32) -> f32 {
    (width * 0.6).max(2.0)
}

/// The thickness of the edge round a stroked mark, before its look's edge
/// width.
pub(crate) fn edge_of(width: f32) -> f32 {
    (width * 0.15).max(1.5)
}

/// The soft drop shadow every mark casts, blurred by `blur`.
fn shadow_paint(blur: f32) -> Paint {
    let mut p = fill(Color4f::new(0.0, 0.0, 0.0, 0.35));
    p.set_mask_filter(MaskFilter::blur(BlurStyle::Normal, blur, false));
    p
}

/// Stroke `path` the Skitch way for arrow width `width`: a soft shadow, an
/// edge (white unless the look says otherwise), then the colour. Lines and
/// strokes are `round` at their ends and joins; a box keeps its corners.
fn stroke_outlined(
    canvas: &Canvas,
    path: &Path,
    color: Color4f,
    look: Look,
    width: f32,
    round: bool,
) {
    let thick = stroke_of(width);
    let edge = edge_of(width) * look.edge_width;
    let paint = |c: Color4f, w: f32| {
        let mut p = stroke(c, w);
        if round {
            p.set_stroke_cap(skia_safe::PaintCap::Round);
            p.set_stroke_join(PaintJoin::Round);
        }
        p
    };
    let mut shadow = shadow_paint(width * 0.4);
    shadow.set_style(PaintStyle::Stroke);
    shadow.set_stroke_width(thick + 2.0 * edge);
    if round {
        shadow.set_stroke_cap(skia_safe::PaintCap::Round);
        shadow.set_stroke_join(PaintJoin::Round);
    }
    canvas.save();
    canvas.translate((0.0, width * 0.25));
    canvas.draw_path(path, &shadow);
    canvas.restore();
    if let Some(e) = look.edge {
        canvas.draw_path(path, &paint(e, thick + 2.0 * edge));
    }
    canvas.draw_path(path, &paint(color, thick));
}

/// A straight line with round ends, outlined like the arrow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Line {
    pub from: Point,
    pub to: Point,
    pub color: Color4f,
    /// The arrow width it is drawn next to; the stroke derives from it.
    pub width: f32,
    pub look: Look,
    /// How far the middle bows out, as for `Arrow`. 0 is straight.
    pub bend: f32,
}

impl Line {
    pub fn draw(&self, canvas: &Canvas) {
        stroke_outlined(
            canvas,
            &curve_path(self.from, self.to, self.bend),
            self.color,
            self.look,
            self.width,
            true,
        );
    }
}

/// An ellipse outline in the box style.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Oval {
    pub rect: Rect,
    pub color: Color4f,
    pub width: f32,
    pub look: Look,
    /// Rotation about its middle, in radians, clockwise.
    pub angle: f32,
}

impl Oval {
    pub fn draw(&self, canvas: &Canvas) {
        let mut pb = PathBuilder::new();
        pb.add_oval(self.rect, None, None);
        stroke_outlined(
            canvas,
            &pb.detach(),
            self.color,
            self.look,
            self.width,
            true,
        );
    }
}

/// A freehand pen stroke, smoothed through the midpoints of its samples.
#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    pub points: Vec<Point>,
    pub color: Color4f,
    pub width: f32,
    pub look: Look,
}

impl Stroke {
    pub fn path(&self) -> Path {
        let mut pb = PathBuilder::new();
        let Some(&first) = self.points.first() else {
            return pb.detach();
        };
        pb.move_to(first);
        for pair in self.points.windows(2) {
            let mid = Point::new((pair[0].x + pair[1].x) / 2.0, (pair[0].y + pair[1].y) / 2.0);
            pb.quad_to(pair[0], mid);
        }
        if let Some(&last) = self.points.last() {
            pb.line_to(last);
        }
        pb.detach()
    }

    pub fn draw(&self, canvas: &Canvas) {
        stroke_outlined(
            canvas,
            &self.path(),
            self.color,
            self.look,
            self.width,
            true,
        );
    }

    /// The box the samples span, without the stroke's width.
    pub fn bounds(&self) -> Rect {
        let mut r = Rect::new_empty();
        r.set_bounds(&self.points);
        r
    }
}

/// A numbered step: a filled circle with its number in white, outlined
/// and shadowed like the other marks. The number comes from the step's
/// place among the document's steps, so deleting one renumbers the rest.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Step {
    pub center: Point,
    pub color: Color4f,
    pub radius: f32,
    pub look: Look,
}

impl Step {
    pub fn draw(&self, canvas: &Canvas, typeface: Option<&Typeface>, number: u32) {
        let r = self.radius;
        let edge = (r * 0.12).max(1.5) * self.look.edge_width;
        let shadow = shadow_paint(r * 0.15);
        canvas.draw_circle(self.center + Point::new(0.0, r * 0.1), r + edge, &shadow);
        if let Some(e) = self.look.edge {
            let ring = fill(e);
            canvas.draw_circle(self.center, r + edge, &ring);
        }
        let disc = fill(self.color);
        canvas.draw_circle(self.center, r, &disc);
        let Some(tf) = typeface else { return };
        let text = number.to_string();
        let font = Font::from_typeface(tf.clone(), r * 1.2);
        let (advance, _) = font.measure_str(&text, None);
        // Digits sit on the baseline and are about 0.7 em tall.
        let origin = Point::new(
            self.center.x - advance / 2.0,
            self.center.y + r * 1.2 * 0.36,
        );
        let path = Path::from_str(&text, origin, &font);
        // White digits, or near-black on a light step.
        let c = self.color;
        let digits = if takes_dark_digits(c) {
            Color4f::new(0.07, 0.07, 0.07, 1.0)
        } else {
            Color4f::new(1.0, 1.0, 1.0, 1.0)
        };
        let paint = fill(digits);
        canvas.draw_path(&path, &paint);
    }
}

choices::choices! {
    /// How a highlight shows what it's over.
    pub enum HighlightLook {
        /// Everything else dims and goes grey; the area stays as it was,
        /// lifted with a soft shadow and a ring in the colour.
        #[default]
        Spotlight => ("Spotlight", "spotlight"),
        /// A highlighter pen dragged across: see-through ink, chisel-cut
        /// ends, streaks, and ink pooled where it landed and lifted.
        Marker => ("Marker", "marker"),
        /// A bold band of colour behind the lower half of the words.
        Band => ("Band", "band"),
        /// A glowing rounded outline, faintly tinted inside.
        Neon => ("Neon", "neon"),
    }
}

/// A highlight over a rectangle, in one of the `HighlightLook`s.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Highlight {
    pub rect: Rect,
    pub color: Color4f,
    pub look: HighlightLook,
}

/// Rec. 709 luma weights: the one measure of how light a colour is.
pub(crate) const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

/// How light `c` looks, 0 to 1.
pub(crate) fn luma(c: Color4f) -> f32 {
    LUMA[0] * c.r + LUMA[1] * c.g + LUMA[2] * c.b
}

/// Whether a step of colour `c` gets near-black digits rather than white.
/// 0.72 sits just below Soft's orange, the darkest preset that takes them.
fn takes_dark_digits(c: Color4f) -> bool {
    luma(c) > 0.72
}

/// Turns colours grey, for what's outside a spotlight.
const GREY: [f32; 20] = {
    let [r, g, b] = LUMA;
    [
        r, g, b, 0.0, 0.0, r, g, b, 0.0, 0.0, r, g, b, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0,
    ]
};

/// Grey and darken `picture` (drawn at `at`) within `area`, all but the
/// spotlights' `holes`. Several spotlights share one dimming.
pub(crate) fn dim_around(canvas: &Canvas, picture: &Image, at: Point, area: Rect, holes: &[RRect]) {
    canvas.save();
    canvas.clip_rect(area, None, true);
    for h in holes {
        canvas.clip_rrect(*h, skia_safe::ClipOp::Difference, true);
    }
    let mut grey = Paint::default();
    grey.set_color_filter(skia_safe::color_filters::matrix_row_major(&GREY, None));
    canvas.draw_image(picture, at, Some(&grey));
    canvas.draw_rect(area, &fill(Color4f::new(0.05, 0.06, 0.09, 0.55)));
    canvas.restore();
}

impl Highlight {
    /// A spotlight's opening: the rectangle, rounded.
    pub fn hole(&self) -> RRect {
        let r = self.rect.sorted();
        let radius = (r.width().min(r.height()) * 0.3).min(10.0);
        RRect::new_rect_xy(r, radius, radius)
    }

    /// The look's own part, over a light background. A spotlight's
    /// dimming of everything else is `dim_around`, drawn first; this adds
    /// its shadow and ring.
    pub fn draw(&self, canvas: &Canvas) {
        self.draw_over(canvas, false);
    }

    /// The look's own part, over a `dark` background or not. A marker or
    /// band multiplies its ink in over light backgrounds, so the words stay
    /// dark; over dark ones that would vanish, so it screens a brighter ink
    /// in, which lifts the dark pixels and keeps light words light.
    pub fn draw_over(&self, canvas: &Canvas, dark: bool) {
        let r = self.rect.sorted();
        let c = self.color;
        match self.look {
            HighlightLook::Spotlight => {
                let hole = self.hole();
                let mut shadow = stroke(Color4f::new(0.0, 0.0, 0.0, 0.45), 6.0);
                shadow.set_mask_filter(MaskFilter::blur(BlurStyle::Normal, 6.0, false));
                canvas.draw_rrect(hole.with_outset((3.0, 3.0)), &shadow);
                canvas.draw_rrect(hole, &stroke(c, 2.5));
            }
            HighlightLook::Marker => draw_marker(canvas, r, c, dark),
            HighlightLook::Band => {
                let b = Rect::from_ltrb(r.left, r.center_y(), r.right, r.bottom);
                let lean = (b.height() * 0.15).min(3.0);
                let band = Path::polygon(
                    &[
                        Point::new(b.left + lean, b.top),
                        Point::new(b.right, b.top - lean * 0.3),
                        Point::new(b.right - lean, b.bottom),
                        Point::new(b.left, b.bottom + lean * 0.3),
                    ],
                    true,
                    None,
                    None,
                );
                canvas.draw_path(&band, &ink(c, if dark { 0.85 } else { 0.95 }, dark));
            }
            HighlightLook::Neon => {
                let rr = self.hole();
                let mut tint = fill(Color4f::new(c.r, c.g, c.b, 0.16));
                tint.set_blend_mode(BlendMode::Multiply);
                canvas.draw_rrect(rr, &tint);
                let mut glow = stroke(Color4f::new(c.r, c.g, c.b, 0.9), 6.0);
                glow.set_image_filter(image_filters::blur((6.0, 6.0), None, None, None));
                canvas.draw_rrect(rr, &glow);
                canvas.draw_rrect(rr, &stroke(c, 2.5));
            }
        }
    }
}

/// Highlighter ink in `c` at `alpha`: multiplied over light backgrounds,
/// screened over `dark` ones.
fn ink(c: Color4f, alpha: f32, dark: bool) -> Paint {
    let mut p = fill(Color4f::new(c.r, c.g, c.b, alpha));
    p.set_blend_mode(if dark {
        BlendMode::Screen
    } else {
        BlendMode::Multiply
    });
    p
}

/// A highlighter pen's swipe over `r` in `c`: cut slanted at both ends,
/// streaked, and deeper where the ink pooled (brighter over `dark`
/// backgrounds, where the ink lightens).
fn draw_marker(canvas: &Canvas, r: Rect, c: Color4f, dark: bool) {
    let (top, bottom) = (r.top, r.bottom);
    let slant = ((bottom - top) * 0.35).min(r.width() * 0.25);
    let mut pb = PathBuilder::new();
    pb.move_to((r.left + slant, top));
    pb.line_to((r.right, top + 1.0));
    pb.line_to((r.right - slant, bottom));
    pb.line_to((r.left, bottom - 1.0));
    pb.close();
    let swipe = pb.detach();
    canvas.draw_path(&swipe, &ink(c, if dark { 0.72 } else { 0.85 }, dark));
    // A deeper shade of the ink, for streaks and pools: darker where it
    // multiplies, brighter where it screens.
    let deep = if dark {
        Color4f::new(c.r * 0.5 + 0.5, c.g * 0.5 + 0.5, c.b * 0.5 + 0.4, 1.0)
    } else {
        Color4f::new(c.r * 0.95, c.g * 0.75, c.b * 0.5, 1.0)
    };
    canvas.save();
    canvas.clip_path(&swipe, None, true);
    let streak = ink(deep, 0.18, dark);
    for k in [0.18, 0.55, 0.82] {
        let y = top + (bottom - top) * k;
        canvas.draw_rect(Rect::from_ltrb(r.left, y, r.right, y + 1.5), &streak);
    }
    let mut pool = ink(deep, 0.35, dark);
    pool.set_mask_filter(MaskFilter::blur(BlurStyle::Normal, 3.0, false));
    canvas.draw_rect(
        Rect::from_ltrb(r.left, top, r.left + slant + 8.0, bottom),
        &pool,
    );
    canvas.draw_rect(
        Rect::from_ltrb(r.right - slant - 8.0, top, r.right, bottom),
        &pool,
    );
    canvas.restore();
}

/// Gaussian-blur `rect` of `image`, drawn in place on `canvas`. The image is
/// assumed to be drawn at the canvas origin, as in the editor.
pub fn blur_region(canvas: &Canvas, image: &Image, rect: Rect, sigma: f32) {
    let Some(filter) = blur_filter(sigma) else {
        return;
    };
    let mut paint = Paint::default();
    paint.set_image_filter(filter);
    canvas.save();
    canvas.clip_rect(rect, None, false);
    // Blur only the pixels that can reach the region, not the whole image.
    let src = rect.with_outset((sigma * 3.0, sigma * 3.0));
    canvas.save_layer(
        &skia_safe::canvas::SaveLayerRec::default()
            .bounds(&src)
            .paint(&paint),
    );
    canvas.draw_image(image, (0, 0), None);
    canvas.restore();
    canvas.restore();
}

fn blur_filter(sigma: f32) -> Option<ImageFilter> {
    image_filters::blur((sigma, sigma), TileMode::Clamp, None, None)
}

/// Pixelate `rect` of `image` into square blocks of `block` pixels, drawn
/// in place on `canvas`. Unlike blur this is not reversible by sharpening,
/// so it is the safer default for redacting text.
pub fn pixelate_region(canvas: &Canvas, image: &Image, rect: Rect, block: f32) {
    let block = block.max(1.0);
    let small_w = (rect.width() / block).ceil().max(1.0) as i32;
    let small_h = (rect.height() / block).ceil().max(1.0) as i32;
    let Some(mut small) = surfaces::raster_n32_premul((small_w, small_h)) else {
        return;
    };
    // Downsample with averaging, then scale back up with hard edges.
    small.canvas().draw_image_rect_with_sampling_options(
        image,
        Some((&rect, skia_safe::canvas::SrcRectConstraint::Fast)),
        Rect::from_wh(small_w as f32, small_h as f32),
        SamplingOptions::new(skia_safe::FilterMode::Linear, skia_safe::MipmapMode::Linear),
        &Paint::default(),
    );
    let small = small.image_snapshot();
    canvas.save();
    canvas.clip_rect(rect, None, false);
    canvas.draw_image_rect_with_sampling_options(
        &small,
        None,
        Rect::from_xywh(
            rect.left,
            rect.top,
            small_w as f32 * block,
            small_h as f32 * block,
        ),
        SamplingOptions::new(skia_safe::FilterMode::Nearest, skia_safe::MipmapMode::None),
        &Paint::default(),
    );
    canvas.restore();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_presets_get_dark_step_digits() {
        let dark_digits: Vec<String> = PALETTES
            .iter()
            .flat_map(|p| p.colours)
            .filter(|c| takes_dark_digits(*c))
            .map(|c| {
                format!(
                    "{:02x}{:02x}{:02x}",
                    (c.r * 255.0).round() as u8,
                    (c.g * 255.0).round() as u8,
                    (c.b * 255.0).round() as u8
                )
            })
            .collect();
        assert_eq!(
            dark_digits,
            [
                "facc08", "ffffff", "fcb17e", "f6e193", "98df9a", "ffffff", "facc08", "d4d7dc",
                "ffffff"
            ]
        );
    }

    fn arrow(len: f32, width: f32) -> Arrow {
        Arrow {
            from: Point::new(100.0, 100.0),
            to: Point::new(100.0 + len, 100.0),
            color: Color4f::new(0.9, 0.1, 0.1, 1.0),
            width,
            look: Look::default(),
            bend: 0.0,
        }
    }

    #[test]
    fn arrow_spans_from_tail_to_tip() {
        let b = arrow(300.0, 10.0).path().bounds().to_owned();
        assert!((b.left - 100.0).abs() < 0.01);
        assert!((b.right - 400.0).abs() < 0.01);
        // The head is the widest part: 2 × width either side.
        assert!((b.height() - 40.0).abs() < 0.01);
    }

    #[test]
    fn short_arrow_keeps_a_shaft() {
        let a = arrow(20.0, 10.0);
        let b = a.path().bounds().to_owned();
        assert!((b.width() - 20.0).abs() < 0.01);
        // Head capped at half the length, so the shaft is still there.
        assert!(b.height() < 40.0);
    }

    #[test]
    fn app_icon_is_red_with_a_white_arrow_and_clear_corners() {
        let n = 32usize;
        let px = app_icon(n as u32);
        assert_eq!(px.len(), n * n * 4);
        let at = |x: usize, y: usize| &px[(y * n + x) * 4..][..4];
        // The very corner is outside the rounded square.
        assert_eq!(at(0, 0)[3], 0);
        // Near the edge is solid red (BGRA).
        let red = at(4, 16);
        assert!(red[2] > 200 && red[1] < 80 && red[3] == 255, "{red:?}");
        // The arrow's middle is white.
        let mid = at(16, 16);
        assert!(mid[..3].iter().all(|&v| v > 200), "{mid:?}");
    }

    #[test]
    fn degenerate_arrow_is_empty() {
        assert!(arrow(0.5, 10.0).path().is_empty());
    }

    #[test]
    fn drawing_changes_pixels() {
        let mut surface = surfaces::raster_n32_premul((500, 200)).unwrap();
        surface.canvas().clear(Color4f::new(1.0, 1.0, 1.0, 1.0));
        arrow(300.0, 10.0).draw(surface.canvas());
        let image = surface.image_snapshot();
        let mut px = [0u8; 4];
        let info = skia_safe::ImageInfo::new_n32_premul((1, 1), None);
        assert!(image.read_pixels(
            &info,
            &mut px,
            4,
            (250, 100),
            skia_safe::image::CachingHint::Allow
        ));
        // Mid-shaft is the arrow's red, not the white background.
        assert!(px[2] > 150 && px[1] < 100, "pixel was {px:?}");
    }

    #[test]
    fn labels_use_bundled_nunito_black() {
        let black = Label::typeface().expect("Nunito loads");
        assert_eq!(black.family_name(), "Nunito");
        // The weight axis took: Black is wider than Bold.
        let bold = Label::ui_typeface().expect("Nunito loads");
        let width = |tf: &Typeface| {
            Font::from_typeface(tf.clone(), 40.0)
                .measure_str("Spooch", None)
                .0
        };
        assert!(
            width(&black) > width(&bold),
            "{} vs {}",
            width(&black),
            width(&bold)
        );
        // Text Nunito has stays in one run.
        assert_eq!(runs("Hello, world!", &black).len(), 1);
    }
}
