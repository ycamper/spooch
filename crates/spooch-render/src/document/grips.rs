//! Handles on the selected marks and the canvas, and what dragging one
//! does to the shape.

use super::*;
use skia_safe::Contains;

/// A handle on a selected mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Grip {
    /// An arrow's or line's start (`false`) or end (`true`).
    End(bool),
    /// A corner or edge midpoint of a rectangle: `x` and `y` are 0 for
    /// left or top, 1 for the middle and 2 for right or bottom.
    Edge { x: u8, y: u8 },
    /// The canvas's move tab: the whole rectangle moves.
    Whole,
    /// The middle of an arrow or line, or under a label's middle, which
    /// bends it.
    Bend,
    /// Inside a box's corner, which rounds it.
    Round,
    /// On a stick above a box, ellipse or label, which turns it.
    Rotate,
}

impl Grip {
    pub(super) fn cursor(self) -> Cursor {
        match self {
            Grip::End(_) | Grip::Whole => Cursor::Move,
            Grip::Bend | Grip::Round | Grip::Rotate => Cursor::Hand,
            Grip::Edge { x, y } => edge_cursor(x, y),
        }
    }
}

/// The resize pointer for an edge or corner: `x` and `y` are 0 for left or
/// top, 1 for neither, 2 for right or bottom. Shared by the canvas's and
/// marks' handles and the editor window's borders.
pub(super) fn edge_cursor(x: u8, y: u8) -> Cursor {
    match (x, y) {
        (1, _) => Cursor::ResizeVertical,
        (_, 1) => Cursor::ResizeHorizontal,
        (x, y) if x == y => Cursor::ResizeDiagonalDown,
        _ => Cursor::ResizeDiagonalUp,
    }
}

/// The pixel the loupe looks at while `grip` of `canvas` moves: the edge
/// pixel (inside the canvas) along a dragged edge, the corner for a
/// dragged corner, and for a moved canvas the corner nearest `pointer`.
pub(super) fn loupe_point(canvas: IRect, grip: Grip, pointer: Point) -> (i32, i32) {
    let (px, py) = (pointer.x as i32, pointer.y as i32);
    let (mx, my) = (
        (canvas.left + canvas.right) / 2,
        (canvas.top + canvas.bottom) / 2,
    );
    let (x, y) = match grip {
        Grip::Edge { x, y } => (x, y),
        _ => (if px < mx { 0 } else { 2 }, if py < my { 0 } else { 2 }),
    };
    let at = |side: u8, lo: i32, hi: i32, p: i32| match side {
        0 => lo,
        2 => hi - 1,
        _ => p.clamp(lo, hi - 1),
    };
    (
        at(x, canvas.left, canvas.right, px),
        at(y, canvas.top, canvas.bottom, py),
    )
}

/// `rect` with the edges named by `grip` moved by `d`, kept inside `bounds`
/// and at least `MIN_CANVAS_PX` each way. `None` if that can't be done.
pub(super) fn resized(rect: IRect, grip: Grip, d: Point, bounds: IRect) -> Option<IRect> {
    if grip == Grip::Whole {
        // Moved as it is, kept inside `bounds`.
        let (w, h) = (rect.width(), rect.height());
        if w > bounds.width() || h > bounds.height() {
            return None;
        }
        let l = (rect.left + d.x.round() as i32).clamp(bounds.left, bounds.right - w);
        let t = (rect.top + d.y.round() as i32).clamp(bounds.top, bounds.bottom - h);
        return Some(IRect::from_xywh(l, t, w, h));
    }
    let Grip::Edge { x, y } = grip else {
        return None;
    };
    let (dx, dy) = (d.x.round() as i32, d.y.round() as i32);
    let (mut l, mut t, mut r, mut b) = (rect.left, rect.top, rect.right, rect.bottom);
    match x {
        0 => l = (l + dx).max(bounds.left).min(r - MIN_CANVAS_PX),
        2 => r = (r + dx).min(bounds.right).max(l + MIN_CANVAS_PX),
        _ => {}
    }
    match y {
        0 => t = (t + dy).max(bounds.top).min(b - MIN_CANVAS_PX),
        2 => b = (b + dy).min(bounds.bottom).max(t + MIN_CANVAS_PX),
        _ => {}
    }
    let out = IRect::from_ltrb(l, t, r, b);
    (bounds.contains(out) && out.width() >= MIN_CANVAS_PX && out.height() >= MIN_CANVAS_PX)
        .then_some(out)
}

/// How far a label's selection box (and its corner handles) sits outside
/// its text, in screen pixels.
pub(super) const LABEL_PAD: f32 = 3.0;

/// A label's selection box: its text's bounds with `LABEL_PAD` round them.
pub(super) fn label_box(l: &Label, tf: &Typeface, scale: f32) -> Rect {
    let pad = LABEL_PAD * scale;
    l.bounds(tf).with_outset((pad, pad))
}

/// A mark's handles. Labels have one on each corner of their box, which
/// needs the typeface to find.
pub(super) fn grips_of(m: &Mark, typeface: Option<&Typeface>, scale: f32) -> Vec<(Grip, Point)> {
    let mut grips = match (m, typeface) {
        (Mark::Label(l), Some(tf)) => {
            let b = label_box(l, tf, scale);
            vec![
                (Grip::Edge { x: 0, y: 0 }, Point::new(b.left, b.top)),
                (Grip::Edge { x: 2, y: 0 }, Point::new(b.right, b.top)),
                (Grip::Edge { x: 0, y: 2 }, Point::new(b.left, b.bottom)),
                (Grip::Edge { x: 2, y: 2 }, Point::new(b.right, b.bottom)),
                (Grip::Bend, label_bend_handle(l, tf)),
            ]
        }
        _ => m.grips(),
    };
    // Marks that turn get a rotate handle on a stick above their middle.
    if m.angle().is_some()
        && let Some(top) = rotate_stick(m, typeface, scale)
    {
        grips.push((Grip::Rotate, top.1));
    }
    if let Some((a, c)) = m.spin(typeface) {
        for (_, p) in &mut grips {
            *p = rotate_about(*p, a, c);
        }
    }
    grips
}

/// Where a mark's rotate handle's stick starts (the middle of its top
/// edge) and ends (the handle), upright.
pub(super) fn rotate_stick(
    m: &Mark,
    typeface: Option<&Typeface>,
    scale: f32,
) -> Option<(Point, Point)> {
    let r = match (m, typeface) {
        (Mark::Label(l), Some(tf)) => label_box(l, tf, scale),
        (Mark::Label(_), None) => return None,
        _ => m.rect()?,
    };
    let top = Point::new(r.center_x(), r.top.min(r.bottom));
    Some((top, Point::new(top.x, top.y - ROTATE_STICK * scale)))
}

/// `p` turned by `a` radians (clockwise on screen) about `c`.
pub(super) fn rotate_about(p: Point, a: f32, c: Point) -> Point {
    let (s, co) = a.sin_cos();
    let d = p - c;
    Point::new(c.x + d.x * co - d.y * s, c.y + d.x * s + d.y * co)
}

/// `original` with handle `grip` dragged to `p`. Turned marks are reshaped
/// in their own frame, the far side staying put on screen; the rotate
/// handle turns them, Shift snapping to 15 degrees.
pub(super) fn reshape(
    original: &Mark,
    grip: Grip,
    p: Point,
    typeface: Option<&Typeface>,
    scale: f32,
    shift: bool,
) -> Mark {
    let mut m = original.clone();
    if grip == Grip::Rotate {
        if let Some(c) = original.pivot(typeface) {
            let deg = ((p.y - c.y).atan2(p.x - c.x) + std::f32::consts::FRAC_PI_2).to_degrees();
            let deg = (deg + 180.0).rem_euclid(360.0) - 180.0;
            let deg = if shift {
                (deg / 15.0).round() * 15.0
            } else {
                // Upright and square angles catch from a few degrees out.
                let square = (deg / 90.0).round() * 90.0;
                if (deg - square).abs() < 3.0 {
                    square
                } else {
                    deg
                }
            };
            let a = (deg + 180.0).rem_euclid(360.0) - 180.0;
            m.set_angle(if a.abs() < 1e-4 { 0.0 } else { a.to_radians() });
        }
        return m;
    }
    let spun = original.spin(typeface);
    let local = match spun {
        Some((a, c)) => rotate_about(p, -a, c),
        None => p,
    };
    match (original, typeface) {
        (Mark::Label(l), Some(tf)) if grip == Grip::Bend => {
            m = Mark::Label(bent_label(l, local, tf, scale));
        }
        (Mark::Label(l), Some(tf)) => {
            m = Mark::Label(resized_label(l, grip, local, tf, scale));
        }
        _ => m.regrip(grip, local, shift),
    }
    // Keep the side opposite the handle where it was on screen.
    if let (Some((a, c_old)), Some(c_new)) = (spun, m.pivot(typeface))
        && let Grip::Edge { x, y } = grip
    {
        let r = match (original, typeface) {
            (Mark::Label(l), Some(tf)) => label_box(l, tf, scale),
            _ => original.rect().unwrap_or_default(),
        };
        let pick = |i: u8, lo: f32, hi: f32, mid: f32| match i {
            0 => hi,
            2 => lo,
            _ => mid,
        };
        let fixed = Point::new(
            pick(x, r.left, r.right, r.center_x()),
            pick(y, r.top, r.bottom, r.center_y()),
        );
        m.offset(rotate_about(fixed, a, c_old) - rotate_about(fixed, a, c_new));
    }
    m
}

/// `l` scaled by dragging corner `grip` to `p`, the opposite corner
/// staying put. Everything about a label grows with its size, so it
/// scales about that corner, lines and curve and all.
fn resized_label(l: &Label, grip: Grip, p: Point, tf: &Typeface, scale: f32) -> Label {
    let Grip::Edge { x, y } = grip else {
        return l.clone();
    };
    let pad = LABEL_PAD * scale;
    let b = l.bounds(tf);
    let corner = Point::new(
        if x == 0 { b.right } else { b.left },
        if y == 0 { b.bottom } else { b.top },
    );
    // Take whichever way the pointer asks for more.
    let grow = |reach: f32, span: f32| {
        if span > 0.0 {
            (reach.abs() - 2.0 * pad) / span
        } else {
            0.0
        }
    };
    let k = grow(p.x - corner.x, b.width()).max(grow(p.y - corner.y, b.height()));
    let size = (l.size * k).clamp(8.0 * scale, 400.0 * scale);
    let k = size / l.size;
    Label {
        origin: corner + (l.origin - corner) * k,
        size,
        bend: l.bend / k,
        ..l.clone()
    }
}

/// How far below the middle of a label's first line its bend handle sits.
fn bend_rest(l: &Label) -> f32 {
    l.size * 0.55
}

/// A label's bend handle: just under the middle of its first line, which
/// it pulls up or down.
pub(super) fn label_bend_handle(l: &Label, tf: &Typeface) -> Point {
    l.middle(tf) + Point::new(0.0, bend_rest(l))
}

/// `l` straight, the ends of its first line where they were.
pub(super) fn straightened(l: &Label, tf: &Typeface) -> Label {
    let ends = l.origin.y - crate::curve::sagitta(l.bend, l.bend_width(tf));
    Label {
        origin: Point::new(l.origin.x, ends),
        bend: 0.0,
        ..l.clone()
    }
}

/// `l` with its bend handle dragged to `p`: the middle follows the handle
/// and the ends stay at their height, like bending an arrow. Down is a
/// smile, up a rainbow, as far as a full circle; near straight it snaps
/// straight.
fn bent_label(l: &Label, p: Point, tf: &Typeface, scale: f32) -> Label {
    let w = l.bend_width(tf);
    let ends = l.origin.y - crate::curve::sagitta(l.bend, w);
    let sag = p.y - bend_rest(l) - ends;
    let bend = if sag.abs() < BEND_SNAP * scale {
        0.0
    } else {
        crate::curve::curvature_for(sag, w)
    };
    Label {
        origin: Point::new(l.origin.x, ends + crate::curve::sagitta(bend, w)),
        bend,
        ..l.clone()
    }
}
