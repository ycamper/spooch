//! The loupe while an area is picked: the pixels round the pointer with
//! its position and colour, or mid-drag the selection's size. Shells pass
//! `selecting_loupe` a way to read their frozen pixels.

use crate::document::{fill, stroke};
use skia_safe::{Color4f, Font, IRect, Paint, PaintStyle, RRect, Rect};

use crate::{Key, Label, Panel};

/// How much of the frozen screen's brightness shows outside the
/// selection, in percent. Low enough that the selection stands out, high
/// enough to still read what's around it.
pub const OUTSIDE_BRIGHTNESS: u16 = 45;

/// How many pixels the loupe shows each side of the pointer's.
pub(crate) const LOUPE_REACH: i32 = 7;
/// The loupe's side, in desktop pixels.
const LOUPE_SIDE: i32 = 2 * LOUPE_REACH + 1;
/// Screen pixels per loupe pixel.
const ZOOM: f32 = 8.0;
/// How far the loupe sits from the pointer, in screen pixels.
const OFFSET: f32 = 24.0;
/// The readout line under the pixels.
const READOUT: f32 = 22.0;
/// Behind the loupe and the hint.
const BACKING: Color4f = Color4f::new(0.08, 0.085, 0.1, 0.94);
/// Their edge.
const EDGE: Color4f = Color4f::new(1.0, 1.0, 1.0, 0.35);
/// What picking takes, until the drag starts.
const HINT: &str = "Drag an area, or click for the whole screen \u{b7} Esc cancels";

/// While the area is being picked: the loupe round `pointer` on `screen`
/// (its monitor), for a shell to blend. `read` gives the frozen pixels of
/// a rectangle of `desktop`, as tightly packed top-down BGRA rows; the
/// loupe is black past the desktop. `anchor` is where the drag started, if
/// one has, and the readout is then the selection's size. `scale` is the
/// display's pixels per screen pixel.
pub fn selecting_loupe(
    pointer: (i32, i32),
    anchor: Option<(i32, i32)>,
    desktop: IRect,
    screen: IRect,
    scale: f32,
    read: impl FnOnce(IRect) -> Option<Vec<u8>>,
) -> Option<Panel> {
    let area = loupe_area(pointer);
    let on = IRect::intersect(&area, &desktop)?;
    let got = read(on)?;
    let side = area.width();
    let row = on.width() as usize * 4;
    if got.len() < row * on.height() as usize {
        return None;
    }
    let mut pixels = vec![0u8; (side * side * 4) as usize];
    for (y, from) in got.chunks_exact(row).take(on.height() as usize).enumerate() {
        let to = (((on.top - area.top + y as i32) * side + on.left - area.left) * 4) as usize;
        pixels[to..to + row].copy_from_slice(from);
    }
    let readout = match anchor {
        Some(a) => Readout::Size((pointer.0 - a.0).abs(), (pointer.1 - a.1).abs()),
        None => Readout::Here,
    };
    loupe(pointer, readout, &pixels, screen, scale)
}

/// Where an arrow `key` moves the pointer while the area is being picked:
/// a pixel, or ten with Shift, kept on `desktop`.
pub fn nudge_pointer(
    key: Key,
    shift: bool,
    pointer: (i32, i32),
    desktop: IRect,
) -> Option<(i32, i32)> {
    let (dx, dy) = key.nudge(shift)?;
    Some((
        (pointer.0 + dx).clamp(desktop.left, desktop.right - 1),
        (pointer.1 + dy).clamp(desktop.top, desktop.bottom - 1),
    ))
}

/// The selection's outline and, once editing, its crop handles, as solid
/// rectangles (`0xRRGGBB`) for the shell to fill in order: a white line
/// just outside `sel`, then a dark square with a white one inside it on
/// each corner and edge middle.
pub fn selection_frame(sel: IRect, handles: bool) -> Vec<(IRect, u32)> {
    const WHITE: u32 = 0xffffff;
    const DARK: u32 = 0x202020;
    let (l, t, r, b) = (sel.left, sel.top, sel.right, sel.bottom);
    let mut out = vec![
        (IRect::from_ltrb(l - 1, t - 1, r + 1, t), WHITE),
        (IRect::from_ltrb(l - 1, b, r + 1, b + 1), WHITE),
        (IRect::from_ltrb(l - 1, t, l, b), WHITE),
        (IRect::from_ltrb(r, t, r + 1, b), WHITE),
    ];
    if handles {
        let (mx, my) = ((l + r) / 2, (t + b) / 2);
        for y in [t, my, b] {
            for x in [l, mx, r] {
                if (x, y) == (mx, my) {
                    continue;
                }
                let outer = IRect::from_ltrb(x - 4, y - 4, x + 5, y + 5);
                out.push((outer, DARK));
                out.push((outer.with_inset((1, 1)), WHITE));
            }
        }
    }
    out
}

/// The desktop pixels the loupe shows round `pointer`, `loupe`'s `pixels`.
pub(crate) fn loupe_area(pointer: (i32, i32)) -> IRect {
    let side = LOUPE_SIDE;
    IRect::from_xywh(pointer.0 - LOUPE_REACH, pointer.1 - LOUPE_REACH, side, side)
}

/// What the line under the loupe's pixels says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Readout {
    /// Where the pointer is on its monitor, and the colour there.
    Here,
    /// The selection's size.
    Size(i32, i32),
}

/// Where the loupe for `pointer` goes on `screen`, for repainting.
pub(crate) fn loupe_rect(pointer: (i32, i32), screen: IRect, scale: f32) -> IRect {
    let s = scale.max(1.0);
    let cell = (ZOOM * s).round();
    let w = (cell * (LOUPE_SIDE) as f32) as i32;
    let h = w + (READOUT * s).round() as i32;
    place(pointer, (w, h), screen, OFFSET * s)
}

/// The loupe for `pointer` on `screen` (the monitor it's on), as a panel to
/// blend over the overlay. `pixels` are `loupe_area(pointer)`'s. `scale`
/// is the display's pixels per screen pixel.
pub(crate) fn loupe(
    pointer: (i32, i32),
    readout: Readout,
    pixels: &[u8],
    screen: IRect,
    scale: f32,
) -> Option<Panel> {
    let side = LOUPE_SIDE;
    if pixels.len() < (side * side * 4) as usize {
        return None;
    }
    let s = scale.max(1.0);
    let cell = (ZOOM * s).round();
    let rect = loupe_rect(pointer, screen, scale);
    let (w, h) = (rect.width(), rect.height());

    Some(crate::document::panel(rect, |canvas| {
        let (fw, fh) = (w as f32, h as f32);
        let radius = 6.0 * s;
        let outer = RRect::new_rect_xy(Rect::from_wh(fw, fh), radius, radius);
        canvas.draw_rrect(outer, &fill(BACKING));

        // The pixels, each a square, clipped to the rounded top.
        canvas.save();
        canvas.clip_rrect(outer, None, true);
        let mut px = Paint::default();
        for y in 0..side {
            for x in 0..side {
                let i = ((y * side + x) * 4) as usize;
                let [b, g, r] = [pixels[i], pixels[i + 1], pixels[i + 2]];
                px.set_color4f(rgb(r, g, b), None);
                canvas.draw_rect(
                    Rect::from_xywh(x as f32 * cell, y as f32 * cell, cell, cell),
                    &px,
                );
            }
        }
        canvas.restore();

        // The crosshair: faint lines along the pointer's row and column, and
        // the pointer's own pixel boxed in black and white so it shows on any
        // colour.
        let mid = LOUPE_REACH as f32 * cell;
        let mut line = Paint::new(Color4f::new(1.0, 1.0, 1.0, 0.18), None);
        line.set_anti_alias(false);
        // Up to the pointer's pixel and on from it, which keeps its true colour.
        let far = mid + cell;
        for r in [
            Rect::from_ltrb(0.0, mid, mid, far),
            Rect::from_ltrb(far, mid, fw, far),
            Rect::from_ltrb(mid, 0.0, far, mid),
            Rect::from_ltrb(mid, far, far, fw),
        ] {
            canvas.draw_rect(r, &line);
        }
        let mut outline = Paint::new(Color4f::new(0.0, 0.0, 0.0, 1.0), None);
        outline.set_style(PaintStyle::Stroke);
        outline.set_stroke_width(s);
        let boxed = Rect::from_xywh(mid, mid, cell, cell);
        canvas.draw_rect(boxed.with_outset((s * 0.5, s * 0.5)), &outline);
        outline.set_color4f(Color4f::new(1.0, 1.0, 1.0, 1.0), None);
        canvas.draw_rect(boxed.with_outset((s * 1.5, s * 1.5)), &outline);

        // The readout: the selection's size mid-drag, or else the position
        // and the colour under the pointer.
        let text = match readout {
            Readout::Size(w, h) => format!("{w} \u{00d7} {h}"),
            Readout::Here => {
                let i = ((LOUPE_REACH * side + LOUPE_REACH) * 4) as usize;
                format!(
                    "{}, {}   #{:02x}{:02x}{:02x}",
                    pointer.0 - screen.left,
                    pointer.1 - screen.top,
                    pixels[i + 2],
                    pixels[i + 1],
                    pixels[i]
                )
            }
        };
        if let Some(tf) = Label::ui_typeface() {
            let font = Font::from_typeface(tf, 12.0 * s);
            let (tw, bounds) = font.measure_str(&text, None);
            let fg = fill(Color4f::new(1.0, 1.0, 1.0, 1.0));
            let band = fh - fw;
            canvas.draw_str(
                &text,
                (
                    ((fw - tw) / 2.0).max(4.0 * s),
                    crate::document::ink_baseline(&bounds, fw + band / 2.0),
                ),
                &font,
                &fg,
            );
        }
        canvas.draw_rrect(outer.with_inset((s * 0.5, s * 0.5)), &stroke(EDGE, s));
    }))
}

/// The hint at the top middle of `screen`, as a panel to blend over the
/// overlay. `scale` is the display's pixels per screen pixel.
pub fn picking_hint(screen: IRect, scale: f32) -> Option<Panel> {
    let s = scale.max(1.0);
    let font = Font::from_typeface(Label::ui_typeface()?, 13.0 * s);
    let tw = font.measure_str(HINT, None).0;
    let (w, h) = ((tw + 32.0 * s).ceil() as i32, (32.0 * s).round() as i32);
    let rect = IRect::from_xywh(
        screen.left + (screen.width() - w) / 2,
        screen.top + (24.0 * s).round() as i32,
        w,
        h,
    );
    Some(crate::document::panel(rect, |canvas| {
        let (fw, fh) = (w as f32, h as f32);
        let outer = RRect::new_rect_xy(Rect::from_wh(fw, fh), fh / 2.0, fh / 2.0);
        canvas.draw_rrect(outer, &fill(BACKING));
        let y = crate::document::centred_baseline(&font, fh / 2.0);
        let white = fill(Color4f::new(1.0, 1.0, 1.0, 1.0));
        canvas.draw_str(HINT, ((fw - tw) / 2.0, y), &font, &white);
        canvas.draw_rrect(outer.with_inset((s * 0.5, s * 0.5)), &stroke(EDGE, s));
    }))
}

fn rgb(r: u8, g: u8, b: u8) -> Color4f {
    crate::document::hex(u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b))
}

/// Where a `size` panel goes: below and right of the pointer, flipped to
/// the other side where it would leave `screen`.
fn place(pointer: (i32, i32), (w, h): (i32, i32), screen: IRect, gap: f32) -> IRect {
    let gap = gap.round() as i32;
    let mut x = pointer.0 + gap;
    if x + w > screen.right {
        x = pointer.0 - gap - w;
    }
    let mut y = pointer.1 + gap;
    if y + h > screen.bottom {
        y = pointer.1 - gap - h;
    }
    let x = x.clamp(screen.left, (screen.right - w).max(screen.left));
    let y = y.clamp(screen.top, (screen.bottom - h).max(screen.top));
    IRect::from_xywh(x, y, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch() -> Vec<u8> {
        let side = LOUPE_SIDE;
        let mut px = vec![0u8; (side * side * 4) as usize];
        // The pointer's pixel is orange.
        let i = ((LOUPE_REACH * side + LOUPE_REACH) * 4) as usize;
        px[i..i + 4].copy_from_slice(&[0x00, 0x80, 0xff, 0xff]);
        px
    }

    #[test]
    fn the_area_is_centred_on_the_pointer() {
        let a = loupe_area((100, 50));
        assert_eq!((a.left + LOUPE_REACH, a.top + LOUPE_REACH), (100, 50));
        assert_eq!(a.width(), LOUPE_SIDE);
    }

    #[test]
    fn the_loupe_sits_beside_the_pointer_and_stays_on_screen() {
        let screen = IRect::from_ltrb(0, 0, 1000, 800);
        let p = loupe((100, 100), Readout::Here, &patch(), screen, 1.0).unwrap();
        assert!(p.rect.left > 100 && p.rect.top > 100, "{:?}", p.rect);
        assert_eq!(
            p.pixels.len(),
            (p.rect.width() * p.rect.height() * 4) as usize
        );
        // The pointer's pixel, magnified, is in the middle of the square.
        let w = p.rect.width();
        let c = w / 2;
        let i = ((c * w + c) * 4) as usize;
        assert_eq!(&p.pixels[i..i + 3], &[0x00, 0x80, 0xff]);
        // In the bottom right corner it flips up and left, and on a
        // monitor to the left it stays on that monitor.
        let p = loupe((990, 790), Readout::Size(980, 780), &patch(), screen, 1.0).unwrap();
        assert!(p.rect.right < 990 && p.rect.bottom < 790, "{:?}", p.rect);
        let left = IRect::from_ltrb(-2560, 717, 0, 2157);
        let p = loupe((-5, 1000), Readout::Here, &patch(), left, 1.0).unwrap();
        assert!(p.rect.right <= 0, "{:?}", p.rect);
        // Short pixels give no loupe rather than a panic.
        assert!(loupe((1, 1), Readout::Here, &[0; 8], screen, 1.0).is_none());
    }

    #[test]
    fn selecting_reads_only_the_desktop_and_reads_out_the_size() {
        let desktop = IRect::from_ltrb(0, 0, 1000, 800);
        let mut asked = None;
        let p = selecting_loupe((2, 400), Some((50, 300)), desktop, desktop, 1.0, |r| {
            asked = Some(r);
            Some(vec![255; (r.width() * r.height() * 4) as usize])
        });
        // Off the left edge: only the part on the desktop is read.
        assert_eq!(asked, Some(IRect::from_ltrb(0, 393, 10, 408)));
        assert!(p.is_some());
        // A short read gives no loupe rather than a panic.
        assert!(selecting_loupe((2, 400), None, desktop, desktop, 1.0, |_| Some(vec![])).is_none());
    }

    #[test]
    fn the_pointer_nudges_and_stays_on_the_desktop() {
        let d = IRect::from_ltrb(-2560, 0, 2560, 1440);
        assert_eq!(nudge_pointer(Key::Left, false, (10, 10), d), Some((9, 10)));
        assert_eq!(nudge_pointer(Key::Down, true, (10, 10), d), Some((10, 20)));
        assert_eq!(nudge_pointer(Key::Up, true, (10, 5), d), Some((10, 0)));
        assert_eq!(
            nudge_pointer(Key::Right, false, (2559, 5), d),
            Some((2559, 5))
        );
        assert_eq!(nudge_pointer(Key::Enter, false, (0, 0), d), None);
    }

    #[test]
    fn the_frame_is_outside_the_selection_with_eight_handles() {
        let sel = IRect::from_ltrb(10, 10, 110, 60);
        let plain = selection_frame(sel, false);
        assert_eq!(plain.len(), 4);
        for (r, _) in &plain {
            assert!(IRect::intersect(r, &sel).is_none(), "{r:?} inside");
        }
        let all = selection_frame(sel, true);
        assert_eq!(all.len(), 4 + 8 * 2);
        assert_eq!(all[4].0, IRect::from_ltrb(6, 6, 15, 15));
    }

    #[test]
    fn it_grows_with_the_display_scale() {
        let screen = IRect::from_ltrb(0, 0, 4000, 3000);
        let one = loupe((10, 10), Readout::Here, &patch(), screen, 1.0).unwrap();
        let two = loupe((10, 10), Readout::Here, &patch(), screen, 2.0).unwrap();
        assert_eq!(two.rect.width(), one.rect.width() * 2);
    }
}
