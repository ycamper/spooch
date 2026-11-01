//! What the framed capture sits on: a gradient, a solid colour, a blurred
//! copy of the capture, or nothing, with padding round it.

use crate::document::fill;
use crate::document::theme::hex;
use skia_safe::{
    Canvas, Color4f, Image, Paint, Point, Rect, SamplingOptions, Shader, TileMode, gradient,
    image_filters,
};

/// A background.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Backdrop {
    /// No background and no padding: the frame as it is.
    None,
    /// Padding left see-through.
    Clear,
    /// The capture itself, blurred and darkened, so it always matches.
    Glass,
    Solid(Color4f),
    /// From the top-left corner to the bottom-right.
    Gradient(Color4f, Color4f),
}

/// The backgrounds, by name, in the order the bar lists them. The
/// gradients pair colours of like lightness and chroma in OKLCH, so none
/// goes muddy in the middle.
pub const BACKGROUNDS: [(&str, Backdrop); 17] = [
    ("No background", Backdrop::None),
    ("Transparent", Backdrop::Clear),
    ("Glass", Backdrop::Glass),
    ("Paper", Backdrop::Solid(hex(0xf3eee3))),
    ("Charcoal", Backdrop::Solid(hex(0x24262b))),
    ("Peach", gradient(0xf2b772, 0xf47b74)),
    ("Rose", gradient(0xfa969f, 0xd861aa)),
    ("Orchid", gradient(0xe486c6, 0x9769dc)),
    ("Lavender", gradient(0xc1a9ee, 0x6985e7)),
    ("Sky", gradient(0x84d9f3, 0x5894e0)),
    ("Lagoon", gradient(0x6cd9d8, 0x3c96c5)),
    ("Mint", gradient(0x94eebf, 0x1dbcb5)),
    ("Meadow", gradient(0xcce576, 0x47b777)),
    ("Honey", gradient(0xf9dd73, 0xf59740)),
    ("Dusk", gradient(0x573c7f, 0x0d2e55)),
    ("Night", gradient(0x21344c, 0x0d1c27)),
    ("Ember", gradient(0x8c3019, 0x511130)),
];

const fn gradient(from: u32, to: u32) -> Backdrop {
    Backdrop::Gradient(hex(from), hex(to))
}

/// Padding round the framed capture, in screen pixels.
pub const PADDINGS: [u16; 5] = [0, 16, 32, 64, 128];

/// The padding a background starts with: 64 px, as ray.so has.
pub const DEFAULT_PADDING: u8 = 3;

/// The index into `PADDINGS` of `px` pixels, if it's one of them.
pub fn padding_index(px: u16) -> Option<u8> {
    PADDINGS.iter().position(|p| *p == px).map(|i| i as u8)
}

impl Backdrop {
    /// Background `index` into `BACKGROUNDS`.
    pub(crate) fn of(index: u8) -> Backdrop {
        BACKGROUNDS
            .get(usize::from(index))
            .map_or(Backdrop::None, |(_, b)| *b)
    }

    /// The name saved documents and `styles.json` use.
    pub(crate) fn key(index: u8) -> Option<String> {
        BACKGROUNDS
            .get(usize::from(index))
            .map(|(name, _)| name.to_lowercase())
    }

    /// The index of the background saved as `key`.
    pub(crate) fn from_key(key: &str) -> Option<u8> {
        BACKGROUNDS
            .iter()
            .position(|(name, _)| name.eq_ignore_ascii_case(key))
            .map(|i| i as u8)
    }

    /// Fill `area` with it; `shot` is the capture, for Glass.
    pub(crate) fn draw(self, canvas: &Canvas, area: Rect, shot: &Image, k: f32) {
        match self {
            Backdrop::None | Backdrop::Clear => {}
            Backdrop::Solid(c) => {
                canvas.draw_rect(area, &fill(c));
            }
            Backdrop::Gradient(..) => {
                let mut p = Paint::default();
                p.set_shader(self.shader(area));
                canvas.draw_rect(area, &p);
            }
            Backdrop::Glass => {
                // The capture scaled to cover the area, blurred well past
                // reading, and darkened so the capture stands out on it.
                let (w, h) = (shot.width() as f32, shot.height() as f32);
                let zoom = (area.width() / w).max(area.height() / h);
                let cover = Rect::from_xywh(
                    area.center_x() - w * zoom / 2.0,
                    area.center_y() - h * zoom / 2.0,
                    w * zoom,
                    h * zoom,
                );
                let blur = 30.0 * k;
                let mut p = Paint::default();
                p.set_image_filter(image_filters::blur(
                    (blur, blur),
                    TileMode::Clamp,
                    None,
                    None,
                ));
                canvas.save();
                canvas.clip_rect(area, None, true);
                canvas.draw_image_rect_with_sampling_options(
                    shot,
                    None,
                    cover,
                    SamplingOptions::default(),
                    &p,
                );
                canvas.draw_rect(area, &fill(Color4f::new(0.0, 0.0, 0.0, 0.25)));
                canvas.restore();
            }
        }
    }

    /// A gradient's shader across `area`; `None` for anything else.
    fn shader(self, area: Rect) -> Option<Shader> {
        let Backdrop::Gradient(a, b) = self else {
            return None;
        };
        // Mixed in OKLab, which keeps the middle as bright as the ends.
        let stops = [a, b];
        let colours = gradient::Colors::new_evenly_spaced(&stops, TileMode::Clamp, None);
        let mix = gradient::Interpolation {
            color_space: gradient::interpolation::ColorSpace::OKLab,
            ..Default::default()
        };
        gradient::shaders::linear_gradient(
            (
                Point::new(area.left, area.top),
                Point::new(area.right, area.bottom),
            ),
            &gradient::Gradient::new(colours, mix),
            None,
        )
    }

    /// A small round sample of it for the bar's list, centred on `c`.
    pub(crate) fn draw_sample(self, canvas: &Canvas, c: Point, r: f32, s: f32) {
        let area = Rect::from_xywh(c.x - r, c.y - r, 2.0 * r, 2.0 * r);
        let mut p = Paint::default();
        p.set_anti_alias(true);
        match self {
            Backdrop::None => {}
            Backdrop::Clear => {
                // A checkerboard, as image editors show see-through.
                canvas.save();
                canvas.clip_rrect(
                    skia_safe::RRect::new_oval(area),
                    skia_safe::ClipOp::Intersect,
                    true,
                );
                canvas.draw_rect(area, &fill(hex(0xffffff)));
                let q = r / 2.0;
                for (i, j) in (0..4).flat_map(|i| (0..4).map(move |j| (i, j))) {
                    if (i + j) % 2 == 1 {
                        let sq = Rect::from_xywh(
                            area.left + i as f32 * q,
                            area.top + j as f32 * q,
                            q,
                            q,
                        );
                        canvas.draw_rect(sq, &fill(hex(0xc8c8c8)));
                    }
                }
                canvas.restore();
            }
            // A frosted grey, light at the top.
            Backdrop::Glass => {
                p.set_shader(Backdrop::Gradient(hex(0xb8bcc6), hex(0x4a4e58)).shader(area));
                canvas.draw_circle(c, r, &p);
            }
            Backdrop::Solid(colour) => {
                canvas.draw_circle(c, r, &fill(colour));
            }
            Backdrop::Gradient(..) => {
                p.set_shader(self.shader(area));
                canvas.draw_circle(c, r, &p);
            }
        }
        if self != Backdrop::None {
            canvas.draw_circle(
                c,
                r,
                &crate::document::theme::stroke(super::super::theme::MUTED, s),
            );
        }
    }
}
