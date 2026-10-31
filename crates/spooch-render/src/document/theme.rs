//! The editor chrome's look, in one place: the palette every panel uses
//! (rail, style row, bar, menus, the window's top row, tooltips) and the
//! small drawing helpers they share, so they all match.

use skia_safe::{Canvas, Color4f, Font, IRect, Paint, PaintCap, PaintStyle, Point, RRect, Rect};

/// A colour from `0xRRGGBB`.
pub(crate) const fn hex(rgb: u32) -> Color4f {
    Color4f::new(
        ((rgb >> 16) & 0xff) as f32 / 255.0,
        ((rgb >> 8) & 0xff) as f32 / 255.0,
        (rgb & 0xff) as f32 / 255.0,
        1.0,
    )
}

/// Panels and the window's docked strips.
pub(crate) const PANEL: Color4f = Color4f::new(0.110, 0.114, 0.129, 1.0);
/// A panel's hairline edge.
pub(crate) const PANEL_EDGE: Color4f = Color4f::new(0.227, 0.235, 0.259, 1.0);
/// Behind the capture in the editor window: darker than the panels.
pub(crate) const BACKDROP: Color4f = Color4f::new(0.078, 0.082, 0.094, 1.0);
/// Lines between the window's docked strips.
pub(crate) const DIVIDER: Color4f = Color4f::new(0.18, 0.188, 0.208, 1.0);
/// Text fields and tooltips: darker than the panel they sit on.
pub(crate) const FIELD: Color4f = Color4f::new(0.07, 0.075, 0.082, 1.0);
/// A button at rest.
pub(crate) const BUTTON: Color4f = Color4f::new(0.165, 0.173, 0.192, 1.0);
/// What the pointer is over.
pub(crate) const HOVER: Color4f = Color4f::new(0.18, 0.188, 0.212, 1.0);
/// What's picked: the current row of a list, a pressed button.
pub(crate) const SELECTED: Color4f = Color4f::new(0.25, 0.26, 0.29, 1.0);
/// A pill's outline.
pub(crate) const OUTLINE: Color4f = Color4f::new(0.30, 0.31, 0.34, 1.0);
/// Icons and glyphs.
pub(crate) const ICON: Color4f = Color4f::new(0.84, 0.843, 0.859, 1.0);
/// Text.
pub(crate) const TEXT: Color4f = Color4f::new(0.91, 0.914, 0.925, 1.0);
/// Quiet text: names, placeholders, the rest of a list.
pub(crate) const MUTED: Color4f = Color4f::new(0.6, 0.61, 0.64, 1.0);
/// The tool in use.
pub(crate) const ACTIVE: Color4f = Color4f::new(0.91, 0.25, 0.18, 1.0);
/// The window's one accent (Copy).
pub(crate) const ACCENT: Color4f = Color4f::new(0.22, 0.54, 0.87, 1.0);
pub(crate) const ACCENT_HOVER: Color4f = Color4f::new(0.29, 0.60, 0.92, 1.0);
/// The close button under the pointer, as Windows has it.
pub(crate) const CLOSE_HOVER: Color4f = Color4f::new(0.77, 0.17, 0.11, 1.0);
/// Selection outlines and handles on the canvas.
pub(crate) const SELECTION: Color4f = Color4f::new(0.15, 0.5, 1.0, 1.0);
pub(crate) const WHITE: Color4f = Color4f::new(1.0, 1.0, 1.0, 1.0);

/// `c` with alpha `a`.
pub(crate) const fn with_alpha(c: Color4f, a: f32) -> Color4f {
    Color4f::new(c.r, c.g, c.b, a)
}

/// Paint that fills with `c`, anti-aliased.
pub(crate) fn fill(c: Color4f) -> Paint {
    let mut p = Paint::new(c, None);
    p.set_anti_alias(true);
    p
}

/// Paint that strokes `width` wide in `c`, anti-aliased.
pub(crate) fn stroke(c: Color4f, width: f32) -> Paint {
    let mut p = fill(c);
    p.set_style(PaintStyle::Stroke);
    p.set_stroke_width(width);
    p
}

/// A rounded rectangle filled with `c`.
pub(crate) fn fill_rrect(canvas: &Canvas, r: Rect, radius: f32, c: Color4f) {
    canvas.draw_rrect(RRect::new_rect_xy(r, radius, radius), &fill(c));
}

/// A pill with a hairline edge, filled while `hovered`.
pub(crate) fn outlined_pill(canvas: &Canvas, r: Rect, hovered: bool, scale: f32) {
    let radius = r.height() / 2.0;
    if hovered {
        fill_rrect(canvas, r, radius, HOVER);
    }
    let edge = RRect::new_rect_xy(r.with_inset((0.5 * scale, 0.5 * scale)), radius, radius);
    canvas.draw_rrect(edge, &stroke(OUTLINE, scale));
}

/// An X centred on `c`, `k` from the middle to each corner.
pub(crate) fn cross(canvas: &Canvas, c: Point, k: f32, paint: &Paint) {
    canvas.draw_line((c.x - k, c.y - k), (c.x + k, c.y + k), paint);
    canvas.draw_line((c.x - k, c.y + k), (c.x + k, c.y - k), paint);
}

/// A panel's rounded background and hairline edge.
pub(crate) fn rounded_panel(canvas: &Canvas, bounds: Rect, radius: f32, scale: f32) {
    let inner = RRect::new_rect_xy(
        bounds.with_inset((0.5 * scale, 0.5 * scale)),
        radius,
        radius,
    );
    canvas.draw_rrect(inner, &fill(PANEL));
    canvas.draw_rrect(inner, &stroke(PANEL_EDGE, scale.max(1.0)));
}

/// A small down-pointing chevron centred on `c`, as dropdowns show.
pub(crate) fn chevron(canvas: &Canvas, c: Point, scale: f32, colour: Color4f) {
    let mut p = stroke(colour, 1.5 * scale);
    p.set_stroke_cap(PaintCap::Round);
    let (left, tip, right) = (
        Point::new(c.x - 3.5 * scale, c.y - 1.5 * scale),
        Point::new(c.x, c.y + 2.0 * scale),
        Point::new(c.x + 3.5 * scale, c.y - 1.5 * scale),
    );
    canvas.draw_line(left, tip, &p);
    canvas.draw_line(tip, right, &p);
}

/// A colour dot with a quiet ring, `radius` round `c`.
pub(crate) fn swatch(canvas: &Canvas, c: Point, radius: f32, colour: Color4f, scale: f32) {
    canvas.draw_circle(c, radius, &fill(colour));
    canvas.draw_circle(c, radius, &stroke(MUTED, scale));
}

/// A small tick box centred on `c`, ticked when `on`.
pub(crate) fn tick_box(canvas: &Canvas, c: Point, on: bool, scale: f32) {
    let s = scale;
    let r = Rect::from_xywh(c.x - 6.0 * s, c.y - 6.0 * s, 12.0 * s, 12.0 * s);
    if on {
        fill_rrect(canvas, r, 3.0 * s, ACCENT);
        let mut p = stroke(WHITE, 1.75 * s);
        p.set_stroke_cap(PaintCap::Round);
        let (a, b, d) = (
            Point::new(c.x - 3.0 * s, c.y),
            Point::new(c.x - 0.75 * s, c.y + 2.5 * s),
            Point::new(c.x + 3.5 * s, c.y - 2.5 * s),
        );
        canvas.draw_line(a, b, &p);
        canvas.draw_line(b, d, &p);
    } else {
        let edge = RRect::new_rect_xy(r.with_inset((0.5 * s, 0.5 * s)), 3.0 * s, 3.0 * s);
        canvas.draw_rrect(edge, &stroke(MUTED, s));
    }
}

/// A dropdown list's panel: its rows (`rows`, in the panel's own
/// coordinates once `origin` is taken off), the one in use marked and the
/// hovered one lit, each then drawn by `row` with its rectangle and
/// whether it's the one in use.
pub(crate) fn draw_list(
    canvas: &Canvas,
    list: IRect,
    rows: &[IRect],
    current: Option<usize>,
    hovered: Option<usize>,
    scale: f32,
    mut row: impl FnMut(usize, Rect, bool),
) {
    let s = scale;
    let bounds = Rect::from_wh(list.width() as f32, list.height() as f32);
    rounded_panel(canvas, bounds, 8.0 * s, s);
    let origin = Point::new(list.left as f32, list.top as f32);
    for (i, r) in rows.iter().enumerate() {
        let rect = Rect::from(*r)
            .with_offset(-origin)
            .with_inset((3.0 * s, 0.0));
        let is_current = current == Some(i);
        if is_current {
            fill_rrect(canvas, rect, 5.0 * s, SELECTED);
        } else if hovered == Some(i) {
            fill_rrect(canvas, rect, 5.0 * s, HOVER);
        }
        row(i, rect, is_current);
    }
}

/// The baseline that centres `font`'s text on `mid_y`.
pub(crate) fn centred_baseline(font: &Font, mid_y: f32) -> f32 {
    let (_, m) = font.metrics();
    mid_y - (m.ascent + m.descent) / 2.0
}

/// The baseline that centres text whose ink is `bounds` (from
/// `measure_str`) on `mid_y`: exact for digits and short words.
pub(crate) fn ink_baseline(bounds: &Rect, mid_y: f32) -> f32 {
    mid_y - bounds.height() / 2.0 - bounds.top
}

/// `text` cut to fit `max_w` with an ellipsis, or as it is if it fits.
pub(crate) fn ellipsize(font: &Font, text: &str, max_w: f32) -> String {
    let width = |t: &str| font.measure_str(t, None).0;
    if width(text) <= max_w {
        return text.to_string();
    }
    // The longest prefix that fits with the ellipsis, by binary search over
    // character boundaries.
    let starts: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
    // The first `n` characters.
    let prefix = |n: usize| starts.get(n).map_or(text, |&end| &text[..end]);
    let fits = |n: usize| width(&format!("{}\u{2026}", prefix(n))) <= max_w;
    let (mut lo, mut hi) = (0, starts.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if fits(mid) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    format!("{}\u{2026}", prefix(lo))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_reads_rgb() {
        assert_eq!(hex(0xff8000), Color4f::new(1.0, 128.0 / 255.0, 0.0, 1.0));
    }

    #[test]
    fn ellipsize_fits_and_keeps_short_text() {
        let Some(tf) = crate::Label::ui_typeface() else {
            return;
        };
        let font = Font::from_typeface(tf, 12.0);
        assert_eq!(ellipsize(&font, "short", 500.0), "short");
        let long = "Spooch 2026-10-04 at 17.51.13 with a long name";
        let cut = ellipsize(&font, long, 80.0);
        assert!(cut.ends_with('\u{2026}'));
        assert!(font.measure_str(&cut, None).0 <= 80.0);
        assert!(cut.len() < long.len());
    }
}
