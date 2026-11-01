//! Helpers the examples share. Each example takes what it needs, so the
//! rest is dead code to it.
#![allow(dead_code)]

use skia_safe::{
    AlphaType, Canvas, ColorType, Data, EncodedImageFormat, IRect, Image, ImageInfo, Point, images,
    surfaces,
};
use spooch_render::{Document, Exported};

/// The output path: the first argument, else `default`.
pub fn out(default: &str) -> String {
    std::env::args().nth(1).unwrap_or_else(|| default.into())
}

/// Drag from `from` to `to` in eight steps, as a hand would.
pub fn drag(d: &mut Document, from: (f32, f32), to: (f32, f32)) {
    d.begin_drag(Point::new(from.0, from.1));
    for k in 1..=8 {
        let t = k as f32 / 8.0;
        d.drag_to(Point::new(
            from.0 + (to.0 - from.0) * t,
            from.1 + (to.1 - from.1) * t,
        ));
    }
    d.end_drag();
}

/// Tightly packed BGRA as an image.
pub fn image(size: (i32, i32), bgra: &[u8], alpha: AlphaType) -> Image {
    let info = ImageInfo::new(size, ColorType::BGRA8888, alpha, None);
    images::raster_from_data(&info, Data::new_copy(bgra), size.0 as usize * 4).expect("an image")
}

/// An export, which is straight alpha, as an image.
pub fn exported(e: &Exported) -> Image {
    image(
        (e.width as i32, e.height as i32),
        &e.bgra,
        AlphaType::Unpremul,
    )
}

/// Draw on a fresh surface of `size` and return its pixels as tightly
/// packed premultiplied BGRA.
pub fn pixels(size: (i32, i32), draw: impl FnOnce(&Canvas)) -> Vec<u8> {
    let mut s = surfaces::raster_n32_premul(size).expect("a surface");
    draw(s.canvas());
    let info = ImageInfo::new(size, ColorType::BGRA8888, AlphaType::Premul, None);
    let mut px = vec![0u8; (size.0 * size.1 * 4) as usize];
    assert!(s.read_pixels(&info, &mut px, size.0 as usize * 4, (0, 0)));
    px
}

/// `area` of BGRA rows `stride` bytes apart, tightly packed.
pub fn crop(px: &[u8], stride: usize, area: IRect) -> Vec<u8> {
    let width = area.width() as usize * 4;
    let mut out = Vec::with_capacity(width * area.height() as usize);
    for y in area.top..area.bottom {
        let at = y as usize * stride + area.left as usize * 4;
        out.extend_from_slice(&px[at..at + width]);
    }
    out
}

/// `img` as a PNG.
pub fn png(img: &Image) -> Data {
    img.encode(None, EncodedImageFormat::PNG, 100)
        .expect("a PNG")
}

/// Write `img` to `path` as a PNG and say so.
pub fn save(path: &str, img: &Image) {
    std::fs::write(path, png(img).as_bytes()).expect("written");
    println!("wrote {path}");
}
