//! The marks themselves: what each kind is, how it draws, where it sits
//! and how close the pointer is to it.

use std::borrow::Cow;

use super::*;

const WHITE_EDGE: Color4f = Color4f::new(1.0, 1.0, 1.0, 1.0);
const BLACK_EDGE: Color4f = Color4f::new(0.0, 0.0, 0.0, 1.0);

/// One finished annotation. The list keeps them in the order they were
/// made; drawing order puts redactions first, then highlights, then the
/// rest.
#[derive(Clone, PartialEq)]
pub(super) enum Mark {
    Arrow(Arrow),
    Line(Line),
    Box(Boxed),
    Oval(Oval),
    Highlight(Highlight),
    Label(Label),
    Stroke(Stroke),
    Step(Step),
    Sticker(crate::Sticker),
    /// Blur, pixelate or black out the captured pixels under `rect`.
    Redact(Rect, Redaction),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Redaction {
    /// Blur and pixelate carry their strength step (0 to 4).
    Blur(u8),
    Pixelate(u8),
    /// Solid black. Blur and pixelate are cosmetic and can sometimes be
    /// reversed; this removes the pixels from anything exported.
    Solid,
}

/// The captured pixels the document holds, and where they sit.
pub(super) struct Base {
    pub(super) image: Image,
    /// Desktop position of the image's top-left pixel.
    pub(super) at: Point,
}

impl Mark {
    /// Draw onto `canvas`. Redactions read the original capture, `base`, so
    /// overlapping ones never blur already-blurred pixels. `number` is a
    /// step's number; `scale` the display scale, for redaction strengths.
    pub(super) fn draw(
        &self,
        canvas: &Canvas,
        base: &Base,
        typeface: Option<&Typeface>,
        scale: f32,
        number: u32,
    ) {
        self.faded(canvas, |c| {
            self.draw_solid(c, base, typeface, scale, number)
        });
    }

    /// `draw`, as pixel art on `grid` when given. Highlights blend with
    /// what's under them, so they stay smooth.
    pub(super) fn draw_in(
        &self,
        canvas: &Canvas,
        grid: Option<&Grid>,
        base: &Base,
        typeface: Option<&Typeface>,
        scale: f32,
        number: u32,
    ) {
        let pixel = grid
            .filter(|_| !matches!(self, Mark::Highlight(_)))
            .zip(self.reach(typeface));
        let Some((grid, area)) = pixel else {
            return self.draw(canvas, base, typeface, scale, number);
        };
        let mut ink = self.with_edge_picked(base, typeface).into_owned();
        let finish = ink.look_mut().map(pixel::finish_of).unwrap_or_default();
        self.faded(canvas, |c| {
            grid.draw(c, area, finish, |c| {
                ink.draw_solid(c, base, typeface, scale, number)
            })
        });
    }

    /// Run `draw` so that a see-through mark fades as a whole, its edge and
    /// shadow not showing through its fill.
    fn faded(&self, canvas: &Canvas, draw: impl FnOnce(&Canvas)) {
        let alpha = self.look().map_or(1.0, |l| l.opacity);
        if alpha < 1.0 {
            canvas.save_layer_alpha_f(None, alpha);
        }
        draw(canvas);
        if alpha < 1.0 {
            canvas.restore();
        }
    }

    /// The mark at full opacity, its auto edge picked and turned as it's
    /// turned.
    fn draw_solid(
        &self,
        canvas: &Canvas,
        base: &Base,
        typeface: Option<&Typeface>,
        scale: f32,
        number: u32,
    ) {
        let m = self.with_edge_picked(base, typeface);
        m.turned(canvas, typeface, || {
            m.draw_opaque(canvas, base, typeface, scale, number)
        });
    }

    /// The mark with an auto edge replaced by the colour that reads over
    /// what's under it.
    fn with_edge_picked(&self, base: &Base, typeface: Option<&Typeface>) -> Cow<'_, Mark> {
        if !self.look().is_some_and(|l| l.auto_edge) {
            return Cow::Borrowed(self);
        }
        let edge = self.auto_edge(base, typeface);
        let mut picked = self.clone();
        if let Some(l) = picked.look_mut() {
            (l.edge, l.auto_edge) = (Some(edge), false);
        }
        Cow::Owned(picked)
    }

    /// Run `draw` with the canvas turned the way the mark is turned.
    pub(super) fn turned(&self, canvas: &Canvas, typeface: Option<&Typeface>, draw: impl FnOnce()) {
        let spin = self.spin(typeface);
        if let Some((a, c)) = spin {
            canvas.save();
            canvas.rotate(a.to_degrees(), Some(c));
        }
        draw();
        if spin.is_some() {
            canvas.restore();
        }
    }

    fn draw_opaque(
        &self,
        canvas: &Canvas,
        base: &Base,
        typeface: Option<&Typeface>,
        scale: f32,
        number: u32,
    ) {
        match self {
            Mark::Redact(r, Redaction::Solid) => {
                let black = Paint::new(Color4f::new(0.0, 0.0, 0.0, 1.0), None);
                canvas.draw_rect(*r, &black);
            }
            Mark::Redact(r, kind) => {
                // The redaction helpers expect the image at the origin.
                canvas.save();
                canvas.translate(base.at);
                let r = r.with_offset(-base.at);
                let strength = |level: &u8| style::STRENGTH[usize::from(*level).min(4)] * scale;
                match kind {
                    Redaction::Blur(level) => blur_region(canvas, &base.image, r, strength(level)),
                    Redaction::Pixelate(level) => {
                        pixelate_region(canvas, &base.image, r, strength(level))
                    }
                    Redaction::Solid => {}
                }
                canvas.restore();
            }
            Mark::Arrow(a) => a.draw(canvas),
            Mark::Line(l) => l.draw(canvas),
            Mark::Box(b) => b.draw(canvas),
            Mark::Oval(o) => o.draw(canvas),
            // Over a dark background, ink that lightens instead.
            Mark::Highlight(h) => {
                let dark = brightness_under(base, h.rect.sorted()).is_some_and(|b| b < 0.5);
                h.draw_over(canvas, dark)
            }
            Mark::Stroke(s) => s.draw(canvas),
            Mark::Step(s) => s.draw(canvas, typeface, number),
            Mark::Sticker(s) => s.draw(canvas),
            Mark::Label(l) => {
                if let Some(tf) = typeface {
                    l.draw(canvas, tf, None);
                }
            }
        }
    }

    /// The mark's edge and opacity, for marks that have them.
    pub(super) fn look(&self) -> Option<crate::Look> {
        Some(match self {
            Mark::Arrow(a) => a.look,
            Mark::Line(l) => l.look,
            Mark::Box(b) => b.look,
            Mark::Oval(o) => o.look,
            Mark::Stroke(p) => p.look,
            Mark::Label(l) => l.look,
            Mark::Step(p) => p.look,
            Mark::Highlight(_) | Mark::Sticker(_) | Mark::Redact(..) => return None,
        })
    }

    pub(super) fn look_mut(&mut self) -> Option<&mut crate::Look> {
        Some(match self {
            Mark::Arrow(a) => &mut a.look,
            Mark::Line(l) => &mut l.look,
            Mark::Box(b) => &mut b.look,
            Mark::Oval(o) => &mut o.look,
            Mark::Stroke(p) => &mut p.look,
            Mark::Label(l) => &mut l.look,
            Mark::Step(p) => &mut p.look,
            Mark::Highlight(_) | Mark::Sticker(_) | Mark::Redact(..) => return None,
        })
    }

    /// The edge an auto-edged mark gets: white over a dark background,
    /// black over a light one, from the average brightness of the captured
    /// pixels under it and a margin round it.
    pub(super) fn auto_edge(&self, base: &Base, typeface: Option<&Typeface>) -> Color4f {
        let Some(r) = self.bounds(typeface) else {
            return WHITE_EDGE;
        };
        let margin = r.width().max(r.height()) * 0.15 + 12.0;
        match brightness_under(base, r.with_outset((margin, margin))) {
            Some(b) if b >= 0.5 => BLACK_EDGE,
            _ => WHITE_EDGE,
        }
    }

    /// Drawing layer: redactions under highlights under everything else.
    pub(super) fn layer(&self) -> u8 {
        match self {
            Mark::Redact(..) => 0,
            Mark::Highlight(_) => 1,
            _ => 2,
        }
    }

    pub(super) fn rect(&self) -> Option<Rect> {
        match self {
            Mark::Box(b) => Some(b.rect),
            Mark::Oval(o) => Some(o.rect),
            Mark::Highlight(h) => Some(h.rect),
            Mark::Sticker(s) => Some(s.rect),
            Mark::Redact(r, _) => Some(*r),
            Mark::Arrow(_) | Mark::Line(_) | Mark::Label(_) | Mark::Stroke(_) | Mark::Step(_) => {
                None
            }
        }
    }

    fn set_rect(&mut self, rect: Rect) {
        match self {
            Mark::Box(b) => b.rect = rect,
            Mark::Oval(o) => o.rect = rect,
            Mark::Highlight(h) => h.rect = rect,
            Mark::Sticker(s) => s.rect = rect,
            Mark::Redact(r, _) => *r = rect,
            Mark::Arrow(_) | Mark::Line(_) | Mark::Label(_) | Mark::Stroke(_) | Mark::Step(_) => {}
        }
    }

    /// Whether a click at `p` picks this mark. Boxes are picked by their
    /// outline, so the inside stays free for clicking what is boxed.
    pub(super) fn hit(&self, p: Point, slop: f32, typeface: Option<&Typeface>) -> bool {
        // A rotated mark is hit in its own, unrotated, frame.
        let p = match self.spin(typeface) {
            Some((a, c)) => rotate_about(p, -a, c),
            None => p,
        };
        match self {
            Mark::Arrow(a) => curve_distance(p, a.from, a.to, a.bend) <= a.width + slop,
            Mark::Line(l) => curve_distance(p, l.from, l.to, l.bend) <= stroke_half(l.width) + slop,
            Mark::Box(b) => outline_distance(p, b.rect) <= stroke_half(b.width) + slop,
            Mark::Oval(o) => oval_distance(p, o.rect) <= stroke_half(o.width) + slop,
            Mark::Stroke(s) => {
                let reach = stroke_half(s.width) + slop;
                match s.points.as_slice() {
                    [only] => (p - *only).length() <= reach,
                    points => points
                        .windows(2)
                        .any(|w| segment_distance(p, w[0], w[1]) <= reach),
                }
            }
            Mark::Step(s) => (p - s.center).length() <= s.radius + slop,
            Mark::Highlight(h) => contains(h.rect.with_outset((slop, slop)), p),
            Mark::Sticker(s) => contains(s.rect.sorted().with_outset((slop, slop)), p),
            Mark::Redact(r, _) => contains(r.with_outset((slop, slop)), p),
            Mark::Label(l) => {
                typeface.is_some_and(|tf| contains(l.bounds(tf).with_outset((slop, slop)), p))
            }
        }
    }

    /// The handles a selected mark shows, with where they sit.
    pub(super) fn grips(&self) -> Vec<(Grip, Point)> {
        if let Mark::Arrow(Arrow { from, to, bend, .. }) | Mark::Line(Line { from, to, bend, .. }) =
            self
        {
            return vec![
                (Grip::End(false), *from),
                (Grip::End(true), *to),
                (Grip::Bend, crate::bend_point(*from, *to, *bend)),
            ];
        }
        let Some(r) = self.rect() else {
            return Vec::new();
        };
        let xs = [r.left, r.center_x(), r.right];
        let ys = [r.top, r.center_y(), r.bottom];
        let mut grips = Vec::with_capacity(8);
        // A sticker keeps its shape, so only its corners resize it.
        let corners_only = matches!(self, Mark::Sticker(_));
        for y in 0..3u8 {
            for x in 0..3u8 {
                if (x, y) != (1, 1) && !(corners_only && (x == 1 || y == 1)) {
                    grips.push((
                        Grip::Edge { x, y },
                        Point::new(xs[x as usize], ys[y as usize]),
                    ));
                }
            }
        }
        // A box big enough for it gets a rounding handle just inside its
        // top-left corner, further in the rounder it is.
        if let Mark::Box(b) = self
            && b.rect.width().min(b.rect.height()) >= ROUND_MIN_SIDE
        {
            let d = ROUND_INSET + b.corner_radius();
            grips.push((Grip::Round, Point::new(r.left + d, r.top + d)));
        }
        grips
    }

    /// Move the whole mark by `d`.
    pub(super) fn offset(&mut self, d: Point) {
        match self {
            Mark::Arrow(Arrow { from, to, .. }) | Mark::Line(Line { from, to, .. }) => {
                *from += d;
                *to += d;
            }
            Mark::Label(l) => l.origin += d,
            Mark::Stroke(s) => s.points.iter_mut().for_each(|p| *p += d),
            Mark::Step(s) => s.center += d,
            _ => {
                if let Some(r) = self.rect() {
                    self.set_rect(r.with_offset(d));
                }
            }
        }
    }

    /// Put `grip` at `p`. Dragging an edge past the opposite one flips the
    /// rectangle rather than turning it inside out.
    pub(super) fn regrip(&mut self, grip: Grip, p: Point, keep_aspect: bool) {
        match grip {
            Grip::End(tip) => {
                if let Mark::Arrow(Arrow { from, to, .. }) | Mark::Line(Line { from, to, .. }) =
                    self
                {
                    if tip {
                        *to = p;
                    } else {
                        *from = p;
                    }
                }
            }
            Grip::Round => {
                if let Mark::Box(b) = self {
                    // How far in from the corner, less the handle's inset;
                    // near the corner it snaps square.
                    let d = ((p.x - b.rect.left) + (p.y - b.rect.top)) / 2.0 - ROUND_INSET;
                    let max = b.rect.width().min(b.rect.height()) / 2.0;
                    b.radius = Some(if d < ROUND_SNAP { 0.0 } else { d.min(max) });
                }
            }
            Grip::Bend => {
                if let Mark::Arrow(Arrow { from, to, bend, .. })
                | Mark::Line(Line { from, to, bend, .. }) = self
                {
                    // Near the straight line, it snaps straight.
                    let b = crate::bend_toward(*from, *to, p);
                    *bend = if b.abs() < BEND_SNAP { 0.0 } else { b };
                }
            }
            Grip::Edge { x, y } => {
                let Some(r) = self.rect() else { return };
                let (mut l, mut t, mut rt, mut b) = (r.left, r.top, r.right, r.bottom);
                match x {
                    0 => l = p.x,
                    2 => rt = p.x,
                    _ => {}
                }
                match y {
                    0 => t = p.y,
                    2 => b = p.y,
                    _ => {}
                }
                // Shift on a corner keeps the proportions; a sticker's
                // always do.
                let keep_aspect = keep_aspect || matches!(self, Mark::Sticker(_));
                if keep_aspect && x != 1 && y != 1 && r.width() > 0.0 && r.height() > 0.0 {
                    let s = ((rt - l).abs() / r.width()).max((b - t).abs() / r.height());
                    let (w, h) = (r.width() * s, r.height() * s);
                    if x == 0 {
                        l = rt - w;
                    } else {
                        rt = l + w;
                    }
                    if y == 0 {
                        t = b - h;
                    } else {
                        b = t + h;
                    }
                }
                self.set_rect(Rect::from_ltrb(l.min(rt), t.min(b), l.max(rt), t.max(b)));
            }
            // Marks are moved by dragging them, and turned by `reshape`.
            Grip::Whole | Grip::Rotate => {}
        }
    }

    /// The mark's rotation, for the kinds that turn.
    pub(super) fn angle(&self) -> Option<f32> {
        match self {
            Mark::Box(b) => Some(b.angle),
            Mark::Oval(o) => Some(o.angle),
            Mark::Label(l) => Some(l.angle),
            Mark::Sticker(s) => Some(s.angle),
            _ => None,
        }
    }

    pub(super) fn set_angle(&mut self, a: f32) {
        match self {
            Mark::Box(b) => b.angle = a,
            Mark::Oval(o) => o.angle = a,
            Mark::Label(l) => l.angle = a,
            Mark::Sticker(s) => s.angle = a,
            _ => {}
        }
    }

    /// What a mark turns about: the middle of its box.
    pub(super) fn pivot(&self, typeface: Option<&Typeface>) -> Option<Point> {
        match self {
            Mark::Box(_) | Mark::Oval(_) | Mark::Sticker(_) => self.rect().map(|r| r.center()),
            Mark::Label(l) => typeface.map(|tf| l.pivot(tf)),
            _ => None,
        }
    }

    /// The angle and pivot of a mark that's turned, or `None` when it's
    /// upright.
    pub(super) fn spin(&self, typeface: Option<&Typeface>) -> Option<(f32, Point)> {
        let a = self.angle().filter(|a| *a != 0.0)?;
        Some((a, self.pivot(typeface)?))
    }

    /// Everything the mark draws on, generously: its shape grown by its
    /// stroke, edge or arrow head, turned as it's turned.
    pub(super) fn reach(&self, typeface: Option<&Typeface>) -> Option<Rect> {
        let curve = |from, to, bend| *crate::curve_path(from, to, bend).bounds();
        let (r, pad) = match self {
            Mark::Arrow(a) => (curve(a.from, a.to, a.bend), a.width * 3.0),
            Mark::Line(l) => (curve(l.from, l.to, l.bend), l.width * 2.0),
            Mark::Stroke(s) => (s.bounds(), s.width * 2.0),
            Mark::Box(b) => (b.rect.sorted(), b.width * 2.0),
            Mark::Oval(o) => (o.rect.sorted(), o.width * 2.0),
            Mark::Step(s) => (self.outline(typeface)?, s.radius * 0.5),
            Mark::Label(l) => (self.outline(typeface)?, l.size * 0.3),
            Mark::Sticker(s) => (s.rect.sorted(), 0.0),
            Mark::Highlight(h) => (h.rect.sorted(), 0.0),
            Mark::Redact(r, _) => (r.sorted(), 0.0),
        };
        let r = r.with_outset((pad, pad));
        Some(match self.spin(typeface) {
            Some((a, c)) => {
                skia_safe::Matrix::rotate_deg_pivot(a.to_degrees(), c)
                    .map_rect(r)
                    .0
            }
            None => r,
        })
    }

    /// The area the mark covers, roughly: its outline, or for arrows and
    /// lines the box round their ends.
    pub(super) fn bounds(&self, typeface: Option<&Typeface>) -> Option<Rect> {
        match self {
            Mark::Arrow(Arrow { from, to, .. }) | Mark::Line(Line { from, to, .. }) => {
                Some(Rect::from_ltrb(from.x, from.y, to.x, to.y).sorted())
            }
            _ => self.outline(typeface),
        }
    }

    /// The outline drawn around the mark while it is selected.
    pub(super) fn outline(&self, typeface: Option<&Typeface>) -> Option<Rect> {
        match self {
            Mark::Label(l) => typeface.map(|tf| l.bounds(tf)),
            Mark::Stroke(s) => Some(s.bounds()),
            Mark::Step(s) => Some(Rect::from_xywh(
                s.center.x - s.radius,
                s.center.y - s.radius,
                s.radius * 2.0,
                s.radius * 2.0,
            )),
            _ => self.rect(),
        }
    }
}

/// The mean brightness (0 black to 1 white) of the captured pixels in `r`
/// (desktop pixels), or `None` where there are none to read.
fn brightness_under(base: &Base, r: Rect) -> Option<f32> {
    let pixels = base.image.peek_pixels()?;
    let area = r.with_offset(-base.at).round();
    let area = IRect::intersect(&area, &IRect::from_wh(pixels.width(), pixels.height()))?;
    average_brightness(&pixels, area)
}

/// The mean brightness (0 black to 1 white) of `area` of `pixels`, from
/// about a thousand samples. `None` for pixels it can't read.
fn average_brightness(pixels: &skia_safe::Pixmap, area: IRect) -> Option<f32> {
    let bgra = match pixels.color_type() {
        skia_safe::ColorType::BGRA8888 => true,
        skia_safe::ColorType::RGBA8888 => false,
        _ => return None,
    };
    let bytes = pixels.bytes()?;
    let row = pixels.row_bytes();
    let (w, h) = (area.width().max(1), area.height().max(1));
    let step = ((w as f32 * h as f32 / 1024.0).sqrt().floor() as usize).max(1);
    let (mut sum, mut n) = (0.0f32, 0u32);
    for y in (area.top as usize..area.bottom as usize).step_by(step) {
        for x in (area.left as usize..area.right as usize).step_by(step) {
            let p = bytes.get(y * row + x * 4..y * row + x * 4 + 3)?;
            let (r, g, b) = if bgra {
                (p[2], p[1], p[0])
            } else {
                (p[0], p[1], p[2])
            };
            let [lr, lg, lb] = crate::LUMA;
            sum += (lr * f32::from(r) + lg * f32::from(g) + lb * f32::from(b)) / 255.0;
            n += 1;
        }
    }
    (n > 0).then(|| sum / n as f32)
}

pub(super) fn contains(r: Rect, p: Point) -> bool {
    p.x >= r.left && p.x <= r.right && p.y >= r.top && p.y <= r.bottom
}

/// Distance from `p` to a straight or bent arrow or line.
fn curve_distance(p: Point, from: Point, to: Point, bend: f32) -> f32 {
    if bend == 0.0 {
        return segment_distance(p, from, to);
    }
    crate::curve_points(from, to, bend, 32)
        .windows(2)
        .map(|w| segment_distance(p, w[0], w[1]))
        .fold(f32::MAX, f32::min)
}

/// Distance from `p` to the segment `a`-`b`.
fn segment_distance(p: Point, a: Point, b: Point) -> f32 {
    let ab = b - a;
    let ap = p - a;
    let len2 = ab.x * ab.x + ab.y * ab.y;
    let t = if len2 > 0.0 {
        ((ap.x * ab.x + ap.y * ab.y) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p - Point::new(a.x + ab.x * t, a.y + ab.y * t)).length()
}

/// Half the visible stroke of a line, box, ellipse or pen mark drawn next
/// to an arrow of `width`: the coloured stroke plus its white edge.
fn stroke_half(width: f32) -> f32 {
    crate::stroke_of(width) / 2.0 + crate::edge_of(width)
}

/// Roughly how far `p` is from the outline of the ellipse inscribed in
/// `r`: exact on the axes, close enough elsewhere for picking.
fn oval_distance(p: Point, r: Rect) -> f32 {
    let (rx, ry) = (r.width() / 2.0, r.height() / 2.0);
    if rx <= 0.0 || ry <= 0.0 {
        return outline_distance(p, r);
    }
    let (dx, dy) = ((p.x - r.center_x()) / rx, (p.y - r.center_y()) / ry);
    ((dx * dx + dy * dy).sqrt() - 1.0).abs() * rx.min(ry)
}

/// Distance from `p` to the outline of `r`, from inside or out.
fn outline_distance(p: Point, r: Rect) -> f32 {
    if contains(r, p) {
        (p.x - r.left)
            .min(r.right - p.x)
            .min(p.y - r.top)
            .min(r.bottom - p.y)
    } else {
        let dx = (r.left - p.x).max(p.x - r.right).max(0.0);
        let dy = (r.top - p.y).max(p.y - r.bottom).max(0.0);
        dx.hypot(dy)
    }
}
