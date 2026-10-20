//! Stickers: pictures stamped on a capture. The set is Microsoft's Fluent
//! Emoji in their 3D style (MIT; `stickers/LICENSE-fluentui-emoji.txt`),
//! built into the app.

use std::cell::RefCell;
use std::collections::HashMap;

use skia_safe::{
    Canvas, Color4f, CubicResampler, Data, Image, Paint, Rect, SamplingOptions, image_filters,
};

macro_rules! stickers {
    ($($name:literal),* $(,)?) => {
        /// Every sticker by name, with its PNG, in the order the picker
        /// shows them.
        pub const STICKERS: &[(&str, &[u8])] = &[
            $(($name, include_bytes!(concat!("../stickers/", $name, ".png")))),*
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
];

/// The sticker called `name`, as the set has it, if there is one.
pub fn sticker_named(name: &str) -> Option<&'static str> {
    STICKERS.iter().map(|(n, _)| *n).find(|n| *n == name)
}

/// A sticker's name for people: "thumbs_up" is "Thumbs up".
pub(crate) fn sticker_title(name: &str) -> String {
    let words = name.replace('_', " ");
    let mut chars = words.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

thread_local! {
    static IMAGES: RefCell<HashMap<&'static str, Image>> = RefCell::new(HashMap::new());
}

/// Sticker `name`'s picture, decoded once per thread.
pub fn sticker_image(name: &'static str) -> Option<Image> {
    IMAGES.with(|cache| {
        if let Some(img) = cache.borrow().get(name) {
            return Some(img.clone());
        }
        let (_, png) = STICKERS.iter().find(|(n, _)| *n == name)?;
        let img = Image::from_encoded(Data::new_copy(png))?;
        cache.borrow_mut().insert(name, img.clone());
        Some(img)
    })
}

/// Draw sticker `name` into `rect`, smoothly scaled.
pub fn draw_sticker_image(canvas: &Canvas, name: &'static str, rect: Rect, paint: &Paint) {
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

/// A sticker stamped on the capture: square, any size, turned about its
/// middle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sticker {
    pub rect: Rect,
    pub name: &'static str,
    /// Rotation about its middle, in radians, clockwise.
    pub angle: f32,
}

impl Sticker {
    /// Draw it with a soft shadow under it, so it sits on the capture.
    pub fn draw(&self, canvas: &Canvas) {
        let side = self.rect.width().min(self.rect.height()).abs();
        let mut paint = Paint::default();
        paint.set_image_filter(image_filters::drop_shadow(
            (0.0, side * 0.03),
            (side * 0.04, side * 0.04),
            Color4f::new(0.0, 0.0, 0.0, 0.35),
            None,
            None,
            None,
        ));
        draw_sticker_image(canvas, self.name, self.rect.sorted(), &paint);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sticker_decodes() {
        for (name, _) in STICKERS {
            let img = sticker_image(name).unwrap_or_else(|| panic!("{name}"));
            assert_eq!((img.width(), img.height()), (256, 256), "{name}");
        }
        assert_eq!(sticker_title("thumbs_up"), "Thumbs up");
        assert_eq!(sticker_named("fire"), Some("fire"));
        assert_eq!(sticker_named("unicorn"), None);
    }
}
