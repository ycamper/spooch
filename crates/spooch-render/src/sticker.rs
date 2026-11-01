//! Stickers: things stamped on a capture. Most are Microsoft's Fluent Emoji
//! in their 3D style (MIT; `stickers/LICENSE-fluentui-emoji.txt`), built
//! into the app. Rubber stamps and mouse pointers are drawn as shapes, so
//! they stay crisp at any size.

use std::cell::RefCell;
use std::collections::HashMap;

use skia_safe::{
    BlendMode, Canvas, Color4f, CubicResampler, Data, Font, Image, Paint, PaintJoin, PaintStyle,
    Path, PathBuilder, PathOp, Point, RRect, Rect, SamplingOptions, color_filters, image_filters,
    shaders,
};

use crate::LabelFont;

/// What a sticker is drawn from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Art {
    /// A Fluent Emoji picture, as PNG.
    Emoji(&'static [u8]),
    /// Rubber-stamp text in a double border, in its colour (0xRRGGBB).
    Stamp(&'static str, u32),
    /// A mouse pointer, for tutorials.
    Pointer(PointerShape),
}

/// The mouse pointers there are stickers of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerShape {
    Arrow,
    /// The arrow clicking: rings around its tip.
    Click,
    Hand,
    /// The I-beam over text.
    Text,
}

macro_rules! stickers {
    ($($name:literal),* $(,)?; $($extra:expr),* $(,)?) => {
        /// Every sticker by name, in the order the picker shows them: the
        /// emoji, then stamps, then pointers. New ones go at the end, since
        /// the remembered recent stickers are indexes into this.
        pub const STICKERS: &[(&str, Art)] = &[
            $(($name, Art::Emoji(include_bytes!(concat!("../stickers/", $name, ".png"))))),*,
            $($extra),*
        ];
    };
}

stickers![
    "thumbs_up",
    "thumbs_down",
    "clapping_hands",
    "raising_hands",
    "ok_hand",
    "folded_hands",
    "flexed_biceps",
    "backhand_index_pointing_right",
    "backhand_index_pointing_left",
    "backhand_index_pointing_up",
    "backhand_index_pointing_down",
    "eyes",
    "face_with_tears_of_joy",
    "rolling_on_the_floor_laughing",
    "smiling_face_with_sunglasses",
    "thinking_face",
    "face_screaming_in_fear",
    "exploding_head",
    "melting_face",
    "skull",
    "face_with_rolling_eyes",
    "pleading_face",
    "zany_face",
    "nerd_face",
    "check_mark_button",
    "cross_mark",
    "red_question_mark",
    "red_exclamation_mark",
    "warning",
    "no_entry",
    "prohibited",
    "hundred_points",
    "fire",
    "sparkles",
    "star",
    "red_heart",
    "light_bulb",
    "bug",
    "rocket",
    "party_popper",
    "trophy",
    "crown",
    "bomb",
    "collision",
    "police_car_light",
    "hourglass_done",
    "locked",
    "key",
    "pushpin",
    "bullseye",
    "chart_increasing",
    "chart_decreasing",
    "hot_beverage";
    ("stamp_approved", Art::Stamp("APPROVED", 0x1f9d55)),
    ("stamp_nope", Art::Stamp("NOPE", 0xd92d2d)),
    ("stamp_bug", Art::Stamp("BUG", 0xd92d2d)),
    ("stamp_fixed", Art::Stamp("FIXED", 0x1f9d55)),
    ("stamp_plus_one", Art::Stamp("+1", 0x2563eb)),
    ("stamp_wip", Art::Stamp("WIP", 0xd97706)),
    ("stamp_ship_it", Art::Stamp("SHIP IT", 0x7c3aed)),
    ("stamp_todo", Art::Stamp("TODO", 0xd97706)),
    ("pointer_arrow", Art::Pointer(PointerShape::Arrow)),
    ("pointer_click", Art::Pointer(PointerShape::Click)),
    ("pointer_hand", Art::Pointer(PointerShape::Hand)),
    ("pointer_text", Art::Pointer(PointerShape::Text)),
];

/// How far a new stamp leans, in radians, anticlockwise.
pub(crate) const STAMP_TILT: f32 = -0.1;

/// The sticker called `name`, as the set has it, if there is one.
pub fn sticker_named(name: &str) -> Option<&'static str> {
    STICKERS.iter().map(|(n, _)| *n).find(|n| *n == name)
}

/// What sticker `name` is drawn from.
pub(crate) fn art_of(name: &str) -> Option<Art> {
    STICKERS.iter().find(|(n, _)| *n == name).map(|(_, a)| *a)
}

/// A sticker's name for people: "thumbs_up" is "Thumbs up".
pub(crate) fn sticker_title(name: &str) -> String {
    match art_of(name) {
        Some(Art::Stamp(text, _)) => format!("{text} stamp"),
        Some(Art::Pointer(p)) => p.title().into(),
        _ => {
            let words = name.replace('_', " ");
            let mut chars = words.chars();
            chars
                .next()
                .map(|c| c.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        }
    }
}

/// Sticker `name`'s width over its height.
pub(crate) fn sticker_aspect(name: &str) -> f32 {
    match art_of(name) {
        Some(Art::Stamp(text, _)) => stamp_box(text).map_or(1.0, |b| b.w / b.h),
        Some(Art::Pointer(p)) => p.view().width() / p.view().height(),
        _ => 1.0,
    }
}

/// The size of sticker `name` with the area of a `side` square.
pub(crate) fn sticker_size(name: &str, side: f32) -> (f32, f32) {
    let a = sticker_aspect(name).sqrt();
    (side * a, side / a)
}

/// The size a click stamps sticker `name` at, where an emoji is `side`
/// square: pointers half that, nearer a real pointer.
pub(crate) fn clicked_size(name: &str, side: f32) -> (f32, f32) {
    let k = match art_of(name) {
        Some(Art::Pointer(_)) => 0.5,
        _ => 1.0,
    };
    sticker_size(name, side * k)
}

/// The sticker grid's tabs, by `Art::tab`.
pub(crate) const STICKER_TABS: [&str; 3] = ["Emoji", "Stamps", "Pointers"];

impl Art {
    /// Its tab in the sticker grid, into `STICKER_TABS`.
    pub(crate) fn tab(self) -> usize {
        match self {
            Art::Emoji(_) => 0,
            Art::Stamp(..) => 1,
            Art::Pointer(_) => 2,
        }
    }

    /// How many grid cells it takes across.
    pub(crate) fn cells(self) -> usize {
        match self {
            Art::Stamp(..) => 3,
            _ => 1,
        }
    }
}

impl PointerShape {
    fn title(self) -> &'static str {
        match self {
            PointerShape::Arrow => "Arrow pointer",
            PointerShape::Click => "Arrow pointer, clicking",
            PointerShape::Hand => "Hand pointer",
            PointerShape::Text => "Text pointer",
        }
    }

    /// The box it's drawn in, in pointer units (the arrow is 12 by 19).
    fn view(self) -> Rect {
        match self {
            PointerShape::Arrow => Rect::from_ltrb(-1.0, -1.0, 13.4, 20.6),
            PointerShape::Click => Rect::from_ltrb(-10.0, -10.0, 13.4, 20.6),
            PointerShape::Hand => Rect::from_ltrb(-0.5, -1.0, 17.6, 22.0),
            PointerShape::Text => Rect::from_ltrb(-0.5, -1.5, 8.5, 19.5),
        }
    }

    /// Draw it in pointer units: white with a black outline.
    fn draw(self, canvas: &Canvas) {
        let mut white = Paint::new(Color4f::new(1.0, 1.0, 1.0, 1.0), None);
        white.set_anti_alias(true);
        let mut black = Paint::new(Color4f::new(0.0, 0.0, 0.0, 1.0), None);
        black.set_anti_alias(true);
        black.set_style(PaintStyle::Stroke);
        black.set_stroke_join(PaintJoin::Round);
        black.set_stroke_width(1.1);
        match self {
            PointerShape::Arrow | PointerShape::Click => {
                if self == PointerShape::Click {
                    let blue = |a: f32| Color4f::new(0.15, 0.45, 0.95, a);
                    let mut ring = Paint::new(blue(0.9), None);
                    ring.set_anti_alias(true);
                    canvas.draw_circle((0.0, 0.0), 3.2, &Paint::new(blue(0.35), None));
                    ring.set_style(PaintStyle::Stroke);
                    ring.set_stroke_width(1.6);
                    canvas.draw_circle((0.0, 0.0), 5.2, &ring);
                    ring.set_color4f(blue(0.45), None);
                    canvas.draw_circle((0.0, 0.0), 8.6, &ring);
                }
                let arrow = Path::polygon(
                    &[
                        Point::new(0.0, 0.0),
                        Point::new(0.0, 17.0),
                        Point::new(4.2, 13.2),
                        Point::new(7.0, 19.4),
                        Point::new(9.6, 18.2),
                        Point::new(6.9, 12.2),
                        Point::new(12.2, 12.2),
                    ],
                    true,
                    None,
                    None,
                );
                canvas.draw_path(&arrow, &white);
                canvas.draw_path(&arrow, &black);
            }
            PointerShape::Hand => {
                let round = |l, t, r, b, radius| {
                    Path::rrect(
                        RRect::new_rect_xy(Rect::from_ltrb(l, t, r, b), radius, radius),
                        None,
                    )
                };
                let parts = [
                    round(4.5, 0.0, 8.5, 13.0, 2.0),
                    round(8.5, 6.0, 11.3, 13.0, 1.4),
                    round(11.3, 7.0, 14.1, 13.0, 1.4),
                    round(14.1, 8.5, 16.6, 14.0, 1.25),
                    round(4.5, 10.0, 16.6, 21.0, 3.5),
                    round(0.8, 11.2, 6.0, 15.2, 2.0),
                ];
                let hand = parts[1..].iter().fold(parts[0].clone(), |acc, p| {
                    acc.op(p, PathOp::Union).unwrap_or(acc)
                });
                canvas.draw_path(&hand, &white);
                canvas.draw_path(&hand, &black);
                // The creases between the folded fingers.
                black.set_stroke_width(0.8);
                for (x, top) in [(8.5, 8.5), (11.3, 9.0), (14.1, 10.5)] {
                    canvas.draw_line((x, top), (x, 12.5), &black);
                }
            }
            PointerShape::Text => {
                let mut beam = PathBuilder::new();
                for (end, bend) in [
                    ((1.5, 0.5), (4.0, 1.0)),
                    ((6.5, 0.5), (4.0, 1.0)),
                    ((1.5, 17.5), (4.0, 17.0)),
                    ((6.5, 17.5), (4.0, 17.0)),
                ] {
                    let joint = if end.1 < 9.0 { 2.5 } else { 15.5 };
                    beam.move_to(end);
                    beam.quad_to(bend, (4.0, joint));
                }
                beam.move_to((4.0, 2.5));
                beam.line_to((4.0, 15.5));
                let beam = beam.detach();
                let mut halo = white.clone();
                halo.set_style(PaintStyle::Stroke);
                halo.set_stroke_width(3.2);
                halo.set_stroke_cap(skia_safe::PaintCap::Round);
                black.set_stroke_width(1.3);
                black.set_stroke_cap(skia_safe::PaintCap::Round);
                canvas.draw_path(&beam, &halo);
                canvas.draw_path(&beam, &black);
            }
        }
    }
}

/// A stamp's layout at a font size of 100.
struct StampBox {
    font: Font,
    w: f32,
    h: f32,
    text_w: f32,
    cap: f32,
}

/// Padding around a stamp's text, at a font size of 100.
const STAMP_PAD: (f32, f32) = (32.0, 26.0);

fn stamp_box(text: &str) -> Option<StampBox> {
    let font = Font::from_typeface(LabelFont::Impact.typeface()?, 100.0);
    let text_w = font.measure_str(text, None).0;
    let cap = font.metrics().1.cap_height;
    Some(StampBox {
        w: text_w + 2.0 * STAMP_PAD.0,
        h: cap + 2.0 * STAMP_PAD.1,
        font,
        text_w,
        cap,
    })
}

/// Draw stamp `text` in `colour` at a font size of 100, its box at the
/// origin: worn ink in a double border.
fn draw_stamp(canvas: &Canvas, b: &StampBox, text: &str, colour: u32) {
    let [r, g, bl] = [16, 8, 0].map(|s| ((colour >> s) & 0xff) as f32 / 255.0);
    let ink = Color4f::new(r, g, bl, 0.9);
    let bounds = Rect::from_wh(b.w, b.h);
    canvas.save_layer(&Default::default());
    let mut line = Paint::new(ink, None);
    line.set_anti_alias(true);
    line.set_style(PaintStyle::Stroke);
    line.set_stroke_width(7.0);
    canvas.draw_rrect(
        RRect::new_rect_xy(bounds.with_inset((3.5, 3.5)), 14.0, 14.0),
        &line,
    );
    line.set_stroke_width(2.5);
    canvas.draw_rrect(
        RRect::new_rect_xy(bounds.with_inset((13.0, 13.0)), 7.0, 7.0),
        &line,
    );
    let mut fill = Paint::new(ink, None);
    fill.set_anti_alias(true);
    let at = ((b.w - b.text_w) / 2.0, (b.h + b.cap) / 2.0);
    canvas.draw_str(text, at, &b.font, &fill);
    // Wear: noise, cut to specks, takes ink out where it's high.
    if let Some(noise) = shaders::fractal_noise((0.09, 0.09), 3, text.len() as f32, None) {
        let mut wear = Paint::default();
        wear.set_shader(noise);
        wear.set_color_filter(color_filters::matrix_row_major(
            &[
                0.0, 0.0, 0.0, 0.0, 0.0, //
                0.0, 0.0, 0.0, 0.0, 0.0, //
                0.0, 0.0, 0.0, 0.0, 0.0, //
                0.0, 0.0, 0.0, 7.0, -4.6,
            ],
            None,
        ));
        wear.set_blend_mode(BlendMode::DstOut);
        canvas.draw_rect(bounds, &wear);
    }
    canvas.restore();
}

thread_local! {
    static IMAGES: RefCell<HashMap<&'static str, Image>> = RefCell::new(HashMap::new());
}

/// Emoji sticker `name`'s picture, decoded once per thread.
pub fn sticker_image(name: &'static str) -> Option<Image> {
    IMAGES.with(|cache| {
        if let Some(img) = cache.borrow().get(name) {
            return Some(img.clone());
        }
        let Some(Art::Emoji(png)) = art_of(name) else {
            return None;
        };
        let img = Image::from_encoded(Data::new_copy(png))?;
        cache.borrow_mut().insert(name, img.clone());
        Some(img)
    })
}

/// Draw sticker `name` as big as fits in `rect`, centred, through `paint`
/// (a shadow, say).
pub fn draw_sticker(canvas: &Canvas, name: &'static str, rect: Rect, paint: &Paint) {
    let Some(art) = art_of(name) else {
        return;
    };
    if let Art::Emoji(_) = art {
        if let Some(img) = sticker_image(name) {
            canvas.draw_image_rect_with_sampling_options(
                &img,
                None,
                rect,
                SamplingOptions::from(CubicResampler::mitchell()),
                paint,
            );
        }
        return;
    }
    let view = match art {
        Art::Stamp(text, _) => match stamp_box(text) {
            Some(b) => Rect::from_wh(b.w, b.h),
            None => return,
        },
        Art::Pointer(p) => p.view(),
        Art::Emoji(_) => unreachable!("drawn above"),
    };
    let k = (rect.width() / view.width()).min(rect.height() / view.height());
    if !(k.is_finite() && k > 0.0) {
        return;
    }
    canvas.save_layer(&skia_safe::canvas::SaveLayerRec::default().paint(paint));
    canvas.translate((
        rect.center_x() - view.center_x() * k,
        rect.center_y() - view.center_y() * k,
    ));
    canvas.scale((k, k));
    match art {
        Art::Stamp(text, colour) => {
            if let Some(b) = stamp_box(text) {
                draw_stamp(canvas, &b, text, colour);
            }
        }
        Art::Pointer(p) => p.draw(canvas),
        Art::Emoji(_) => {}
    }
    canvas.restore();
}

/// A sticker stamped on the capture: any size, turned about its middle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sticker {
    pub rect: Rect,
    pub name: &'static str,
    /// Rotation about its middle, in radians, clockwise.
    pub angle: f32,
}

impl Sticker {
    /// Draw it; pictures and pointers with a soft shadow under them, so
    /// they sit on the capture, and stamps flat, as ink.
    pub fn draw(&self, canvas: &Canvas) {
        let rect = self.rect.sorted();
        let mut paint = Paint::default();
        if !matches!(art_of(self.name), Some(Art::Stamp(..))) {
            let side = rect.width().min(rect.height());
            paint.set_image_filter(image_filters::drop_shadow(
                (0.0, side * 0.03),
                (side * 0.04, side * 0.04),
                Color4f::new(0.0, 0.0, 0.0, 0.35),
                None,
                None,
                None,
            ));
        }
        draw_sticker(canvas, self.name, rect, &paint);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sticker_draws() {
        for (name, art) in STICKERS {
            match art {
                Art::Emoji(_) => {
                    let img = sticker_image(name).unwrap_or_else(|| panic!("{name}"));
                    assert_eq!((img.width(), img.height()), (256, 256), "{name}");
                }
                _ => assert!(sticker_image(name).is_none(), "{name}"),
            }
            let a = sticker_aspect(name);
            assert!(a.is_finite() && a > 0.2 && a < 6.0, "{name}: {a}");
        }
        assert!(STICKERS.len() <= usize::from(u8::MAX), "recents are u8s");
        assert_eq!(sticker_title("thumbs_up"), "Thumbs up");
        assert_eq!(sticker_title("stamp_ship_it"), "SHIP IT stamp");
        assert_eq!(sticker_named("fire"), Some("fire"));
        assert_eq!(sticker_named("unicorn"), None);
        // Stamps are wide; the same area as a square.
        assert!(sticker_aspect("stamp_approved") > 2.0);
        let (w, h) = sticker_size("stamp_approved", 100.0);
        assert!((w * h - 10_000.0).abs() < 1.0);
    }
}
