//! Drawing a frame from its spec: where the capture goes, the window with
//! its rows of items, or an image frame stretched round the capture.

use std::cell::OnceCell;

use crate::document::theme::cross;
use crate::document::{fill, stroke};
use skia_safe::{
    BlurStyle, Canvas, ClipOp, Color4f, Data, FilterMode, Font, FontMgr, FontStyle, IRect, Image,
    MaskFilter, Paint, PaintCap, PathBuilder, PathEffect, Point, RRect, Rect, Typeface, images,
};

use super::Framed;
use super::registry::Loaded;
use super::spec::{Align, Fill, Item, LineStyle, MenuStyle, Outline, Typeface as Face};

/// A drop shadow's softness (blur sigma), darkness and drop, in screen
/// pixels. `SHADOWS` lists the choices; the bar steps through them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    pub blur: f32,
    pub alpha: f32,
    pub drop: f32,
}

/// The shadow choices, by name, in the order the bar steps through them.
/// The first, Auto, is the standard shadow under a frame and none without
/// one; the second is none.
pub const SHADOWS: [(&str, Shadow); 5] = [
    ("Auto shadow", STANDARD),
    ("No shadow", NO_SHADOW),
    (
        "Soft shadow",
        Shadow {
            blur: 8.0,
            alpha: 0.22,
            drop: 4.0,
        },
    ),
    ("Shadow", STANDARD),
    (
        "Deep shadow",
        Shadow {
            blur: 20.0,
            alpha: 0.5,
            drop: 12.0,
        },
    ),
];

const NO_SHADOW: Shadow = Shadow {
    blur: 0.0,
    alpha: 0.0,
    drop: 0.0,
};

const STANDARD: Shadow = Shadow {
    blur: 12.0,
    alpha: 0.35,
    drop: 6.0,
};

impl Shadow {
    /// The shadow at choice `level`, or none if `frame` turns shadows off.
    /// Auto is one when the capture is `framed` (by a frame or a
    /// background).
    pub(super) fn of(level: u8, framed: bool, frame: Option<&Loaded>) -> Shadow {
        match frame {
            Some(f) if !f.spec.shadow => NO_SHADOW,
            _ if level == 0 && !framed => NO_SHADOW,
            _ => SHADOWS[usize::from(level).min(SHADOWS.len() - 1)].1,
        }
    }

    pub(super) fn is_none(self) -> bool {
        self.alpha <= 0.0
    }

    /// Room round the window for the shadow to fade out in.
    fn margin(self) -> f32 {
        if self.is_none() {
            0.0
        } else {
            self.blur * 8.0 / 3.0
        }
    }
}

/// Where the capture sits in the framed result, and the result's size.
pub(super) struct Geometry {
    pub content: IRect,
    pub width: i32,
    pub height: i32,
    /// Room round the frame: the padding, or the shadow's.
    pub margin: i32,
    /// Extra room under it for the shadow's drop.
    pub drop: i32,
}

/// The framed result's geometry for content `w` by `h`; `k` is pixels per
/// screen pixel. No frame, shadow or background is the capture as it is.
pub(super) fn geometry(f: &Framed, w: i32, h: i32, k: f32) -> Geometry {
    let px = |v: f32| (v * k).round() as i32;
    let sh = f.shadow;
    // A background's padding holds the shadow too.
    let (m, drop) = match f.backdrop {
        Some((_, pad)) => (px(pad), 0),
        None if sh.is_none() => (0, 0),
        None => (px(sh.margin()), px(sh.drop)),
    };
    let [l, t, r, b] = match f.frame.as_deref() {
        None => [0; 4],
        Some(f) => match &f.spec.image {
            Some(img) => img.content.map(px),
            None => {
                let edge = px(f.spec.border);
                [edge, px(f.spec.top_height()), edge, edge]
            }
        },
    };
    Geometry {
        content: IRect::from_xywh(m + l, m + t, w, h),
        width: w + l + r + 2 * m,
        height: h + t + b + 2 * m + drop,
        margin: m,
        drop,
    }
}

/// The window top's colour with swatch `colour` (0 is the frame's own).
pub(super) fn top_colour(frame: &Loaded, colour: u8) -> Color4f {
    super::swatch(colour).unwrap_or_else(|| frame.spec.default_colour())
}

/// Whether text and shapes on `c` should be dark.
pub(super) fn is_light(c: Color4f) -> bool {
    crate::luma(c) > 0.6
}

/// Draw `f` round opaque BGRA `content` onto `canvas`, which is the
/// framed result's size.
pub(super) fn draw(
    canvas: &Canvas,
    f: &Framed,
    colour: u8,
    title: &str,
    content: &[u8],
    (w, h): (i32, i32),
    k: f32,
) {
    let info = crate::bgra_info((w, h), skia_safe::AlphaType::Opaque);
    let Some(shot) = images::raster_from_data(&info, Data::new_copy(content), w as usize * 4)
    else {
        return;
    };
    let g = geometry(f, w, h, k);
    if let Some((backdrop, _)) = f.backdrop {
        let all = Rect::from_wh(g.width as f32, g.height as f32);
        backdrop.draw(canvas, all, &shot, k);
    }
    let sh = f.shadow;
    let Some(f) = f.frame.as_deref() else {
        // No frame: the capture, square, with its shadow if any.
        let at = g.content;
        shadow(canvas, RRect::new_rect(Rect::from(at)), sh, k);
        canvas.draw_image(&shot, (at.left, at.top), None);
        return;
    };
    if f.spec.image.is_some() {
        draw_image_frame(canvas, f, &shot, &g, sh, k);
        return;
    }
    let (bar, edge) = (f.spec.top_height() * k, f.spec.border * k);
    let c = g.content;
    let window = Rect::from_ltrb(
        c.left as f32 - edge,
        c.top as f32 - bar,
        c.right as f32 + edge,
        c.bottom as f32 + edge,
    );
    let r = f.spec.radius * k;
    let rr = RRect::new_rect_xy(window, r, r);
    shadow(canvas, rr, sh, k);
    let top = top_colour(f, colour);
    let own_ink = if colour == 0 { f.spec.own_ink() } else { None };
    let tones = Tones::of(top, own_ink);
    // The window: its colour, the rows' backgrounds and the capture,
    // inside the rounded corners.
    canvas.save();
    canvas.clip_rrect(rr, ClipOp::Intersect, true);
    canvas.draw_rect(window, &Paint::new(top, None));
    let mut y = window.top;
    for row in &f.spec.rows {
        let fill = tones.fill(row.background);
        let h = row.height * k;
        canvas.draw_rect(
            Rect::from_ltrb(window.left, y, window.right, y + h),
            &Paint::new(fill, None),
        );
        y += h;
    }
    canvas.draw_image(&shot, (c.left, c.top), None);
    canvas.restore();
    outline(canvas, f, rr, &tones, k);
    // Each row's items.
    let mut y = window.top;
    for row in &f.spec.rows {
        let h = row.height * k;
        let area = Rect::from_ltrb(window.left, y, window.right, y + h);
        draw_row(canvas, f, &row.items, area, &tones, title, k);
        y += h;
    }
}

/// The line round the window, so a light page doesn't melt into a light
/// background, in the style the frame asks for.
fn outline(canvas: &Canvas, f: &Loaded, rr: RRect, t: &Tones, k: f32) {
    let grey = |e: f32| Color4f::new(e, e, e, 0.6);
    let colour = match f.spec.outline {
        Outline::None => return,
        Outline::Light => grey(0.78),
        Outline::Dark => grey(0.28),
        Outline::Auto => grey(if t.light { 0.78 } else { 0.28 }),
        Outline::Colour(c) => c,
    };
    let mut line = stroke(colour, k.max(1.0));
    match f.spec.outline_style {
        LineStyle::Solid => canvas.draw_rrect(rr, &line),
        LineStyle::Double => {
            canvas.draw_rrect(rr, &line);
            let inner = rr.with_inset((3.0 * k, 3.0 * k));
            canvas.draw_rrect(inner, &line)
        }
        LineStyle::Dashed | LineStyle::Ascii => {
            line.set_path_effect(PathEffect::dash(&[5.0 * k, 4.0 * k], 0.0));
            canvas.draw_rrect(rr, &line);
            if f.spec.outline_style == LineStyle::Ascii {
                let plus = stroke(colour, k.max(1.0));
                let w = rr.rect();
                for (x, y) in [
                    (w.left, w.top),
                    (w.right, w.top),
                    (w.left, w.bottom),
                    (w.right, w.bottom),
                ] {
                    cross_plus(canvas, Point::new(x, y), 3.5 * k, &plus);
                }
            }
            canvas
        }
    };
}

/// A + centred on `c`, `s` from the middle to each end.
fn cross_plus(canvas: &Canvas, c: Point, s: f32, p: &Paint) {
    canvas.draw_line((c.x - s, c.y), (c.x + s, c.y), p);
    canvas.draw_line((c.x, c.y - s), (c.x, c.y + s), p);
}

thread_local! {
    /// A system monospace face, looked up once.
    static MONO: OnceCell<Option<Typeface>> = const { OnceCell::new() };
}

/// The titles' typeface: the chrome's own, or a monospace one where the
/// system has one.
fn typeface(face: Face) -> Option<Typeface> {
    let ui = crate::Label::ui_typeface;
    match face {
        Face::Ui => ui(),
        Face::Mono => MONO
            .with(|m| {
                m.get_or_init(|| {
                    let fonts = FontMgr::default();
                    [
                        "Cascadia Mono",
                        "Consolas",
                        "DejaVu Sans Mono",
                        "Liberation Mono",
                        "Ubuntu Mono",
                        "Menlo",
                        "Courier New",
                    ]
                    .into_iter()
                    .find_map(|name| fonts.match_family_style(name, FontStyle::normal()))
                })
                .clone()
            })
            .or_else(ui),
    }
}

/// `sh` under the shape `rr`.
fn shadow(canvas: &Canvas, rr: RRect, sh: Shadow, k: f32) {
    if sh.is_none() {
        return;
    }
    let mut p = fill(Color4f::new(0.0, 0.0, 0.0, sh.alpha));
    p.set_mask_filter(MaskFilter::blur(BlurStyle::Normal, sh.blur * k, false));
    canvas.save();
    canvas.translate((0.0, sh.drop * k));
    canvas.draw_rrect(rr, &p);
    canvas.restore();
}

/// The picture stretched round the capture, its corners kept.
fn draw_image_frame(canvas: &Canvas, f: &Loaded, shot: &Image, g: &Geometry, sh: Shadow, k: f32) {
    let Some(spec) = &f.spec.image else { return };
    let Some(picture) = f.images.get(&spec.file) else {
        return;
    };
    let (m, drop) = (g.margin as f32, g.drop as f32);
    let dst = Rect::from_ltrb(m, m, g.width as f32 - m, g.height as f32 - m - drop);
    let r = f.spec.radius * k;
    shadow(canvas, RRect::new_rect_xy(dst, r, r), sh, k);
    let (iw, ih) = (picture.width(), picture.height());
    let [l, t, r, b] = spec.slice.map(|v| v.round() as i32);
    let centre = IRect::from_ltrb(
        l.clamp(0, iw - 1),
        t.clamp(0, ih - 1),
        (iw - r).clamp(l + 1, iw),
        (ih - b).clamp(t + 1, ih),
    );
    // The slice is in the picture's pixels; scale it with the display.
    canvas.save();
    canvas.translate((dst.left, dst.top));
    canvas.scale((k, k));
    canvas.draw_image_nine(
        picture,
        centre,
        Rect::from_wh(dst.width() / k, dst.height() / k),
        FilterMode::Linear,
        None,
    );
    canvas.restore();
    canvas.draw_image(shot, (g.content.left, g.content.top), None);
}

/// A small globe: a circle, a meridian and the equator.
fn globe(canvas: &Canvas, c: Point, s: f32, ink: Color4f, k: f32) {
    let p = stroke(ink, 1.1 * k);
    canvas.draw_circle(c, s, &p);
    canvas.draw_oval(
        Rect::from_xywh(c.x - s * 0.45, c.y - s, s * 0.9, 2.0 * s),
        &p,
    );
    canvas.draw_line((c.x - s, c.y), (c.x + s, c.y), &p);
}

/// The colours everything on a top is drawn in, all following its colour
/// so any colour reads.
struct Tones {
    top: Color4f,
    light: bool,
    /// Icons and text.
    ink: Color4f,
    /// An address bar: lighter on a light top, darker on a dark one.
    field: Color4f,
    /// A quieter field, for a field on a field.
    well: Color4f,
}

impl Tones {
    /// The tones on `top`, with `ink` for text and icons if given.
    fn of(top: Color4f, ink: Option<Color4f>) -> Tones {
        let light = is_light(top);
        let shade = |f: f32| Color4f::new(top.r * f, top.g * f, top.b * f, 1.0);
        let lift = |f: f32| {
            Color4f::new(
                top.r + (1.0 - top.r) * f,
                top.g + (1.0 - top.g) * f,
                top.b + (1.0 - top.b) * f,
                1.0,
            )
        };
        Tones {
            top,
            light,
            ink: ink.unwrap_or(if light {
                Color4f::new(0.45, 0.47, 0.51, 1.0)
            } else {
                Color4f::new(0.62, 0.65, 0.70, 1.0)
            }),
            // A tab and toolbar on a dark top are lighter, as browsers do.
            field: if light {
                Color4f::new(1.0, 1.0, 1.0, 1.0)
            } else {
                lift(0.1)
            },
            well: if light {
                Color4f::new(0.945, 0.953, 0.961, 1.0)
            } else {
                shade(0.8)
            },
        }
    }

    fn fill(&self, f: Fill) -> Color4f {
        match f {
            Fill::Top => self.top,
            Fill::Field => self.field,
            Fill::Well => self.well,
        }
    }
}

/// How wide an item is, or `None` if it grows to share what's left.
fn fixed_width(f: &Loaded, item: &Item, row: &Rect, k: f32) -> Option<f32> {
    Some(match item {
        Item::Space { width } => width * k,
        Item::Back | Item::Forward | Item::Reload | Item::Plus | Item::Share => 24.0 * k,
        Item::Menu { .. } => 24.0 * k,
        Item::Buttons { size } => 72.0 * size * k,
        Item::Prompt => 20.0 * k,
        Item::Circles { count } => f32::from(*count) * 16.0 * k + 4.0 * k,
        Item::Address { width, .. } | Item::Title { width, .. } => return width.map(|w| w * k),
        Item::Tab {
            fraction,
            min_width,
            ..
        } => (row.width() * fraction).max(min_width * k),
        Item::Dots { colours } => colours.len() as f32 * 18.0 * k + 4.0 * k,
        Item::Image { file, height } => {
            let img = f.images.get(file)?;
            height * k * img.width() as f32 / img.height().max(1) as f32
        }
    })
}

/// Lay a row's items out left to right, the growing ones sharing what's
/// left, and draw them.
fn draw_row(
    canvas: &Canvas,
    f: &Loaded,
    items: &[Item],
    area: Rect,
    t: &Tones,
    title: &str,
    k: f32,
) {
    let widths: Vec<Option<f32>> = items.iter().map(|i| fixed_width(f, i, &area, k)).collect();
    let fixed: f32 = widths.iter().flatten().sum();
    let growers = widths.iter().filter(|w| w.is_none()).count().max(1) as f32;
    let share = ((area.width() - fixed) / growers).max(0.0);
    let mut x = area.left;
    for (item, w) in items.iter().zip(widths) {
        let w = w.unwrap_or(share);
        let r = Rect::from_ltrb(x, area.top, (x + w).min(area.right), area.bottom);
        if r.width() > 0.0 {
            draw_item(canvas, f, item, r, area, t, title, k);
        }
        x += w;
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_item(
    canvas: &Canvas,
    f: &Loaded,
    item: &Item,
    r: Rect,
    row: Rect,
    t: &Tones,
    title: &str,
    k: f32,
) {
    let cy = r.center_y();
    let round_stroke = |c: Color4f, w: f32| {
        let mut p = stroke(c, w);
        p.set_stroke_cap(PaintCap::Round);
        p
    };
    let pill = |l: f32, rt: f32, h: f32, c: Color4f| {
        if rt - l > h {
            let rect = Rect::from_ltrb(l, cy - h / 2.0, rt, cy + h / 2.0);
            canvas.draw_rrect(RRect::new_rect_xy(rect, h / 2.0, h / 2.0), &fill(c));
        }
    };
    let font = typeface(f.spec.font).map(|tf| Font::from_typeface(tf, 11.5 * k));
    let text_width = |s: &str| font.as_ref().map_or(0.0, |f| f.measure_str(s, None).0);
    // `text` from `x`, centred on `y`, cut off at `right`.
    let write = |text: &str, x: f32, y: f32, right: f32| {
        let Some(font) = &font else { return };
        let (_, b) = font.measure_str(text, None);
        canvas.save();
        canvas.clip_rect(
            Rect::from_ltrb(x, y - 10.0 * k, right, y + 10.0 * k),
            ClipOp::Intersect,
            true,
        );
        canvas.draw_str(
            text,
            (x, crate::document::ink_baseline(&b, y)),
            font,
            &fill(t.ink),
        );
        canvas.restore();
    };
    // The title placed in `l..rt`, or a quiet line when there's none.
    let label = |text: &str, l: f32, rt: f32, align: Align, y: f32, placeholder: bool| {
        if text.is_empty() {
            if placeholder {
                let w = (row.width() * 0.22).clamp(40.0 * k, 160.0 * k).min(rt - l);
                let left = match align {
                    Align::Left => l,
                    Align::Center => (l + rt) / 2.0 - w / 2.0,
                };
                let line = Rect::from_ltrb(left, y - 3.0 * k, left + w, y + 3.0 * k);
                canvas.draw_rrect(
                    RRect::new_rect_xy(line, 3.0 * k, 3.0 * k),
                    &fill(Color4f { a: 0.5, ..t.ink }),
                );
            }
            return;
        }
        let x = match align {
            Align::Left => l,
            Align::Center => ((l + rt) / 2.0 - text_width(text) / 2.0).max(l),
        };
        write(text, x, y, rt);
    };
    match item {
        Item::Space { .. } => {}
        Item::Back | Item::Forward => {
            let s = if matches!(item, Item::Back) {
                4.0
            } else {
                -4.0
            } * k;
            let x = r.center_x();
            let p = round_stroke(t.ink, 1.6 * k);
            canvas.draw_line((x + s, cy - 4.5 * k), (x - s, cy), &p);
            canvas.draw_line((x - s, cy), (x + s, cy + 4.5 * k), &p);
        }
        Item::Address {
            height,
            lock,
            align,
            fill: f_,
            text,
            ..
        } => {
            pill(r.left, r.right, height * k, t.fill(*f_));
            let mut left = r.left + 10.0 * k;
            if *lock && r.width() > 40.0 * k {
                // A padlock: a body and a shackle.
                let x = r.left + 14.0 * k;
                canvas.draw_rrect(
                    RRect::new_rect_xy(
                        Rect::from_xywh(x - 4.0 * k, cy - 1.0 * k, 8.0 * k, 6.0 * k),
                        k,
                        k,
                    ),
                    &fill(t.ink),
                );
                canvas.draw_arc(
                    Rect::from_xywh(x - 2.8 * k, cy - 5.5 * k, 5.6 * k, 8.0 * k),
                    180.0,
                    180.0,
                    false,
                    &round_stroke(t.ink, 1.3 * k),
                );
                left = r.left + 26.0 * k;
            }
            if *text {
                label(title, left, r.right - 10.0 * k, *align, cy, false);
            }
        }
        Item::Reload => {
            let p = round_stroke(t.ink, 1.5 * k);
            let (c, s) = (Point::new(r.center_x(), cy), 5.0 * k);
            canvas.draw_arc(
                Rect::from_xywh(c.x - s, c.y - s, 2.0 * s, 2.0 * s),
                -60.0,
                300.0,
                false,
                &p,
            );
            let tip = Point::new(c.x + s * 0.5, c.y - s * 0.87);
            canvas.draw_line(tip, (tip.x + 3.0 * k, tip.y - 2.5 * k), &p);
            canvas.draw_line(tip, (tip.x + 0.5 * k, tip.y + 3.5 * k), &p);
        }
        Item::Plus => {
            let p = round_stroke(t.ink, 1.5 * k);
            cross_plus(canvas, Point::new(r.center_x(), cy), 5.0 * k, &p);
        }
        Item::Share => {
            let p = round_stroke(t.ink, 1.4 * k);
            let x = r.center_x();
            let mut tray = PathBuilder::new();
            tray.move_to((x - 3.0 * k, cy - 2.0 * k))
                .line_to((x - 5.5 * k, cy - 2.0 * k))
                .line_to((x - 5.5 * k, cy + 6.0 * k))
                .line_to((x + 5.5 * k, cy + 6.0 * k))
                .line_to((x + 5.5 * k, cy - 2.0 * k))
                .line_to((x + 3.0 * k, cy - 2.0 * k));
            canvas.draw_path(&tray.detach(), &p);
            canvas.draw_line((x, cy + 2.0 * k), (x, cy - 7.0 * k), &p);
            canvas.draw_line((x - 3.0 * k, cy - 4.0 * k), (x, cy - 7.0 * k), &p);
            canvas.draw_line((x + 3.0 * k, cy - 4.0 * k), (x, cy - 7.0 * k), &p);
        }
        Item::Menu { style } => {
            let x = r.center_x();
            match style {
                MenuStyle::Kebab => {
                    for dy in [-5.0, 0.0, 5.0] {
                        canvas.draw_circle((x, cy + dy * k), 1.5 * k, &fill(t.ink));
                    }
                }
                MenuStyle::Hamburger => {
                    let p = round_stroke(t.ink, 1.4 * k);
                    for dy in [-4.5, 0.0, 4.5] {
                        let y = cy + dy * k;
                        canvas.draw_line((x - 6.0 * k, y), (x + 6.0 * k, y), &p);
                    }
                }
            }
        }
        Item::Buttons { size } => {
            let p = round_stroke(t.ink, 1.3 * k);
            let (s, step) = (4.0 * size * k, 24.0 * size * k);
            let x = r.right - 18.0 * size * k;
            cross(canvas, Point::new(x, cy), s, &p);
            let x = x - step;
            canvas.draw_rect(Rect::from_ltrb(x - s, cy - s, x + s, cy + s), &p);
            let x = x - step;
            canvas.draw_line((x - s, cy), (x + s, cy), &p);
        }
        Item::Circles { count } => {
            let p = stroke(t.ink, 1.2 * k);
            for i in 0..*count {
                let x = r.left + (10.0 + f32::from(i) * 16.0) * k;
                canvas.draw_circle((x, cy), 5.0 * k, &p);
            }
        }
        Item::Prompt => {
            let green = round_stroke(Color4f::new(0.44, 0.82, 0.55, 1.0), 1.6 * k);
            let x = r.left + 4.0 * k;
            canvas.draw_line((x, cy - 4.0 * k), (x + 4.5 * k, cy), &green);
            canvas.draw_line((x + 4.5 * k, cy), (x, cy + 4.0 * k), &green);
            canvas.draw_line(
                (x + 7.0 * k, cy + 4.5 * k),
                (x + 13.0 * k, cy + 4.5 * k),
                &green,
            );
        }
        Item::Title {
            default,
            align,
            placeholder,
            ..
        } => {
            let text = if title.is_empty() {
                default.as_str()
            } else {
                title
            };
            label(text, r.left, r.right, *align, cy, *placeholder);
        }
        Item::Tab { close, .. } => {
            let tab = Rect::from_ltrb(r.left, r.top + 6.0 * k, r.right, r.bottom + 1.0);
            canvas.draw_rrect(
                RRect::new_rect_radii(
                    tab,
                    &[
                        Point::new(8.0 * k, 8.0 * k),
                        Point::new(8.0 * k, 8.0 * k),
                        Point::new(0.0, 0.0),
                        Point::new(0.0, 0.0),
                    ],
                ),
                &fill(t.field),
            );
            let ty = tab.center_y();
            globe(
                canvas,
                Point::new(tab.left + 15.0 * k, ty),
                5.0 * k,
                t.ink,
                k,
            );
            let mut right = tab.right - 10.0 * k;
            if *close {
                let p = round_stroke(t.ink, 1.3 * k);
                cross(canvas, Point::new(tab.right - 14.0 * k, ty), 3.5 * k, &p);
                right = tab.right - 26.0 * k;
            }
            let shown = if title.is_empty() { "New Tab" } else { title };
            label(shown, tab.left + 27.0 * k, right, Align::Left, ty, true);
        }
        Item::Dots { colours } => {
            for (i, c) in colours.iter().enumerate() {
                let c = super::spec::parse_colour(c).unwrap_or(t.ink);
                canvas.draw_circle(
                    (r.left + (11.0 + i as f32 * 18.0) * k, cy),
                    5.5 * k,
                    &fill(c),
                );
            }
        }
        Item::Image { file, height } => {
            if let Some(img) = f.images.get(file) {
                let h = height * k;
                let dst = Rect::from_xywh(r.left, cy - h / 2.0, r.width(), h);
                canvas.draw_image_rect(img, None, dst, &Paint::default());
            }
        }
    }
}
