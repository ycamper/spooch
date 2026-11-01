//! Every background round a small capture in a rounded frame, on one
//! sheet, for checking how they look:
//!
//!     cargo run -p spooch-render --example background_sheet -- sheet.png

use skia_safe::{
    AlphaType, Color4f, Data, EncodedImageFormat, Font, ImageInfo, Paint, Rect, images, surfaces,
};
use spooch_render::{BACKGROUNDS, Document, Frame, Label};

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "background-sheet.png".into());
    let (w, h, cols) = (240usize, 140usize, 6);
    // A page with a coloured header and a few lines, so Glass has
    // something to blur.
    let shot: Vec<u8> = (0..w * h)
        .flat_map(|i| {
            let (x, y) = (i % w, i / w);
            if y < 28 {
                [200, 120, 40, 255]
            } else if (y - 28) % 22 < 8 && x > 16 && x < 16 + (y * 37 % 180) {
                [90, 90, 96, 255]
            } else {
                [250, 250, 250, 255]
            }
        })
        .collect();
    let (cell_w, cell_h) = (w as i32 + 120, h as i32 + 130);
    let rows = BACKGROUNDS.len().div_ceil(cols) as i32;
    let mut sheet =
        surfaces::raster_n32_premul((cell_w * cols as i32, cell_h * rows)).expect("a surface");
    let canvas = sheet.canvas();
    canvas.clear(Color4f::new(1.0, 1.0, 1.0, 1.0));
    // Grey squares behind, so see-through padding shows.
    let check = Paint::new(Color4f::new(0.86, 0.86, 0.88, 1.0), None);
    for (x, y) in (0..cell_w * cols as i32 / 12)
        .flat_map(|x| (0..cell_h * rows / 12).map(move |y| (x, y)))
        .filter(|(x, y)| (x + y) % 2 == 0)
    {
        canvas.draw_rect(
            Rect::from_xywh(x as f32 * 12.0, y as f32 * 12.0, 12.0, 12.0),
            &check,
        );
    }
    let font = Label::ui_typeface().map(|tf| Font::from_typeface(tf, 14.0));
    for (i, (name, _)) in BACKGROUNDS.iter().enumerate() {
        let mut d = Document::from_bgra(w as u32, h as u32, w * 4, &shot, 1.0).expect("a doc");
        d.set_frame(Frame::builtin("rounded"));
        d.set_background(i as u8);
        d.set_padding(2);
        let img = d.export_image();
        let info = ImageInfo::new(
            (img.width as i32, img.height as i32),
            skia_safe::ColorType::BGRA8888,
            AlphaType::Unpremul,
            None,
        );
        let framed =
            images::raster_from_data(&info, Data::new_copy(&img.bgra), img.width as usize * 4)
                .expect("an image");
        let (x, y) = ((i % cols) as i32 * cell_w, (i / cols) as i32 * cell_h);
        canvas.draw_image(&framed, (x as f32 + 20.0, y as f32 + 10.0), None);
        if let Some(f) = &font {
            let paint = Paint::new(Color4f::new(0.3, 0.3, 0.35, 1.0), None);
            canvas.draw_str(name, (x as f32 + 24.0, (y + cell_h - 14) as f32), f, &paint);
        }
    }
    let png = sheet
        .image_snapshot()
        .encode(None, EncodedImageFormat::PNG, 100)
        .expect("a PNG");
    std::fs::write(&out, png.as_bytes()).expect("written");
    println!("{out}");
}
