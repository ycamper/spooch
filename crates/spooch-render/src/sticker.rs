//! Stickers: things stamped on a capture. Most are Microsoft's Fluent Emoji
//! in their 3D style (MIT; `stickers/LICENSE-fluentui-emoji.txt`), built
//! into the app. Rubber stamps, meme captions and mouse pointers are drawn
//! as shapes, so they stay crisp at any size.

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
    /// A meme caption: white Impact-style text, outlined in black, a line
    /// or two.
    Caption(&'static str),
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
        /// emoji, then stamps, meme captions and pointers. Files keep
        /// stickers by name, so new ones can go anywhere.
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
    "hot_beverage",
    "moai",
    "person_facepalming",
    "person_shrugging",
    "popcorn",
    "clown_face",
    "nail_polish",
    "saluting_face",
    "smirking_face",
    "unamused_face",
    "face_with_spiral_eyes",
    "upside_down_face",
    "face_holding_back_tears",
    "dotted_line_face",
    "brain",
    "money_with_wings",
    "goat",
    "chicken",
    "dog_face",
    "frog",
    "pile_of_poo",
    "ghost";
    ("stamp_approved", Art::Stamp("APPROVED", 0x1f9d55)),
    ("stamp_nope", Art::Stamp("NOPE", 0xd92d2d)),
    ("stamp_bug", Art::Stamp("BUG", 0xd92d2d)),
    ("stamp_fixed", Art::Stamp("FIXED", 0x1f9d55)),
    ("stamp_plus_one", Art::Stamp("+1", 0x2563eb)),
    ("stamp_wip", Art::Stamp("WIP", 0xd97706)),
    ("stamp_ship_it", Art::Stamp("SHIP IT", 0x7c3aed)),
    ("stamp_todo", Art::Stamp("TODO", 0xd97706)),
    ("meme_this_is_fine", Art::Caption("THIS IS FINE")),
    ("meme_works_on_my_machine", Art::Caption("WORKS ON\nMY MACHINE")),
    ("meme_its_a_feature", Art::Caption("IT'S A\nFEATURE")),
    ("meme_y_u_no", Art::Caption("Y U NO")),
    ("meme_challenge_accepted", Art::Caption("CHALLENGE\nACCEPTED")),
    ("meme_me_gusta", Art::Caption("ME GUSTA")),
    ("meme_fffuuuu", Art::Caption("FFFUUUU")),
    ("meme_forever_alone", Art::Caption("FOREVER\nALONE")),
    ("meme_much_wow", Art::Caption("MUCH WOW")),
    ("meme_wat", Art::Caption("WAT")),
    ("meme_ermahgerd", Art::Caption("ERMAHGERD")),
    ("meme_stonks", Art::Caption("STONKS")),
    ("meme_not_stonks", Art::Caption("NOT STONKS")),
    ("meme_plot_twist", Art::Caption("PLOT TWIST")),
    ("meme_big_if_true", Art::Caption("BIG IF TRUE")),
    ("meme_epic_fail", Art::Caption("EPIC FAIL")),
    ("meme_hello_there", Art::Caption("HELLO THERE")),
    ("meme_do_a_barrel_roll", Art::Caption("DO A\nBARREL ROLL")),
    ("meme_67", Art::Caption("67")),
    ("meme_chicken_jockey", Art::Caption("CHICKEN\nJOCKEY")),
    ("meme_plus_1000_aura", Art::Caption("+1000 AURA")),
    ("meme_minus_1000_aura", Art::Caption("-1000 AURA")),
    ("meme_but_you_cant_prove_it", Art::Caption("BUT YOU CAN'T\nPROVE IT")),
    ("meme_skill_issue", Art::Caption("SKILL ISSUE")),
    ("meme_let_him_cook", Art::Caption("LET HIM COOK")),
    ("meme_emotional_damage", Art::Caption("EMOTIONAL\nDAMAGE")),
    ("meme_touch_grass", Art::Caption("TOUCH GRASS")),
    ("pointer_arrow", Art::Pointer(PointerShape::Arrow)),
    ("pointer_click", Art::Pointer(PointerShape::Click)),
    ("pointer_hand", Art::Pointer(PointerShape::Hand)),
    ("pointer_text", Art::Pointer(PointerShape::Text)),
];

/// A new stamp's angle: leaning about 6 degrees anticlockwise.
pub(crate) const STAMP_TILT: f32 = -0.1;

/// The angle sticker `name` is placed at.
pub(crate) fn placed_angle(name: &str) -> f32 {
    match art_of(name) {
        Some(Art::Stamp(..)) => STAMP_TILT,
        _ => 0.0,
    }
}

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
        Some(Art::Caption(text)) => text.replace('\n', " "),
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
        Some(art @ (Art::Stamp(..) | Art::Caption(_))) => art.text_box().map_or(1.0, |b| b.w / b.h),
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
pub(crate) const STICKER_TABS: [&str; 4] = ["Emoji", "Stamps", "Memes", "Pointers"];

impl Art {
    /// Its tab in the sticker grid, into `STICKER_TABS`.
    pub(crate) fn tab(self) -> usize {
        match self {
            Art::Emoji(_) => 0,
            Art::Stamp(..) => 1,
            Art::Caption(_) => 2,
            Art::Pointer(_) => 3,
        }
    }

    /// A stamp's or caption's text, laid out.
    fn text_box(self) -> Option<TextBox> {
        match self {
            Art::Stamp(text, _) => text_box(text, STAMP_PAD),
            Art::Caption(text) => text_box(text, CAPTION_PAD),
            _ => None,
        }
    }

    /// How many grid cells it takes across.
    pub(crate) fn cells(self) -> usize {
        match self {
            Art::Stamp(..) | Art::Caption(_) => 3,
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

/// A stamp's or caption's text laid out at a font size of 100: its box,
/// and each line's baseline origin in it.
struct TextBox {
    font: Font,
    w: f32,
    h: f32,
    lines: Vec<(&'static str, Point)>,
}

/// Padding around a stamp's text, at a font size of 100.
const STAMP_PAD: (f32, f32) = (32.0, 26.0);
/// Padding around a caption, room for its outline.
const CAPTION_PAD: (f32, f32) = (10.0, 10.0);
/// The caption's outline, at a font size of 100.
const CAPTION_OUTLINE: f32 = 10.0;

/// `text`'s lines in Impact, centred, with `pad` round them.
fn text_box(text: &'static str, pad: (f32, f32)) -> Option<TextBox> {
    let font = Font::from_typeface(LabelFont::Impact.typeface()?, 100.0);
    let cap = font.metrics().1.cap_height;
    let leading = cap * 0.35;
    let widths: Vec<(&str, f32)> = text
        .lines()
        .map(|l| (l, font.measure_str(l, None).0))
        .collect();
    let widest = widths.iter().map(|(_, w)| *w).fold(0.0, f32::max);
    let n = widths.len() as f32;
    let w = widest + 2.0 * pad.0;
    let h = n * cap + (n - 1.0).max(0.0) * leading + 2.0 * pad.1;
    let lines = widths
        .into_iter()
        .enumerate()
        .map(|(i, (l, lw))| {
            let y = pad.1 + cap + i as f32 * (cap + leading);
            (l, Point::new((w - lw) / 2.0, y))
        })
        .collect();
    Some(TextBox { font, w, h, lines })
}

/// A noise seed of its own for each stamp's wear.
fn seed(text: &str) -> f32 {
    text.bytes().map(f32::from).sum()
}

/// Draw stamp `text` in `colour` at a font size of 100, its box at the
/// origin: worn ink in a double border.
fn draw_stamp(canvas: &Canvas, b: &TextBox, text: &str, colour: u32) {
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
    for (line, at) in &b.lines {
        canvas.draw_str(line, *at, &b.font, &fill);
    }
    // Wear: noise, cut to specks, takes ink out where it's high.
    if let Some(noise) = shaders::fractal_noise((0.09, 0.09), 3, seed(text), None) {
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

/// Draw a caption at a font size of 100, its box at the origin: white,
/// outlined in black.
fn draw_caption(canvas: &Canvas, b: &TextBox) {
    let mut outline = Paint::new(Color4f::new(0.0, 0.0, 0.0, 1.0), None);
    outline.set_anti_alias(true);
    outline.set_style(PaintStyle::Stroke);
    outline.set_stroke_join(PaintJoin::Round);
    outline.set_stroke_width(CAPTION_OUTLINE);
    let mut white = Paint::new(Color4f::new(1.0, 1.0, 1.0, 1.0), None);
    white.set_anti_alias(true);
    for paint in [&outline, &white] {
        for (line, at) in &b.lines {
            canvas.draw_str(line, *at, &b.font, paint);
        }
    }
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
    // Shapes are drawn in their own units, scaled to fit.
    let fit = |view: Rect, draw: &dyn Fn(&Canvas)| {
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
        draw(canvas);
        canvas.restore();
    };
    let art = art_of(name);
    match art {
        Some(Art::Emoji(_)) => {
            if let Some(img) = sticker_image(name) {
                canvas.draw_image_rect_with_sampling_options(
                    &img,
                    None,
                    rect,
                    SamplingOptions::from(CubicResampler::mitchell()),
                    paint,
                );
            }
        }
        Some(Art::Stamp(text, colour)) => {
            if let Some(b) = art.and_then(Art::text_box) {
                fit(Rect::from_wh(b.w, b.h), &|c| {
                    draw_stamp(c, &b, text, colour)
                });
            }
        }
        Some(Art::Caption(_)) => {
            if let Some(b) = art.and_then(Art::text_box) {
                fit(Rect::from_wh(b.w, b.h), &|c| draw_caption(c, &b));
            }
        }
        Some(Art::Pointer(p)) => fit(p.view(), &|c| p.draw(c)),
        None => {}
    }
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
    /// they sit on the capture, and stamps and captions flat.
    pub fn draw(&self, canvas: &Canvas) {
        let rect = self.rect.sorted();
        let mut paint = Paint::default();
        if !matches!(art_of(self.name), Some(Art::Stamp(..) | Art::Caption(_))) {
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
        assert_eq!(
            sticker_title("meme_works_on_my_machine"),
            "WORKS ON MY MACHINE"
        );
        // A long caption takes two lines, so it's no wider than a short one.
        assert!(sticker_aspect("meme_works_on_my_machine") < sticker_aspect("meme_skill_issue"));
        assert_eq!(sticker_named("fire"), Some("fire"));
        assert_eq!(sticker_named("unicorn"), None);
        // Stamps are wide; the same area as a square.
        assert!(sticker_aspect("stamp_approved") > 2.0);
        let (w, h) = sticker_size("stamp_approved", 100.0);
        assert!((w * h - 10_000.0).abs() < 1.0);
    }
}
