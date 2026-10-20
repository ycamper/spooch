//! Every built-in frame round a plain capture, on one sheet, for checking
//! how they look:
//!
//!     cargo run -p spooch-render --example frame_sheet -- sheet.png

use skia_safe::{
    AlphaType, Color4f, Data, EncodedImageFormat, Font, ImageInfo, Paint, images, surfaces,
};
use spooch_render::{Document, Frame, Label};

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "frame-sheet.png".into());
    let (w, h, cols) = (360, 200, 4);
    let blank = vec![250u8; w * h * 4];
    let frames: Vec<(Frame, String)> = Frame::all().into_iter().skip(1).collect();
    let (cell_w, cell_h) = (w as i32 + 120, h as i32 + 200);
    let rows = frames.len().div_ceil(cols) as i32;
    let mut sheet =
        surfaces::raster_n32_premul((cell_w * cols as i32, cell_h * rows)).expect("a surface");
    let canvas = sheet.canvas();
    canvas.clear(Color4f::new(1.0, 1.0, 1.0, 1.0));
    let font = Label::ui_typeface().map(|tf| Font::from_typeface(tf, 14.0));
    for (i, (frame, name)) in frames.iter().enumerate() {
        let mut d = Document::from_bgra(w as u32, h as u32, w * 4, &blank, 1.0).expect("a doc");
        d.set_frame(*frame);
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
            let label = format!("{}. {name}", i + 1);
            let paint = Paint::new(Color4f::new(0.4, 0.4, 0.45, 1.0), None);
            canvas.draw_str(
                &label,
                (x as f32 + 60.0, (y + cell_h - 20) as f32),
                f,
                &paint,
            );
        }
    }
    let png = sheet
        .image_snapshot()
        .encode(None, EncodedImageFormat::PNG, 100)
        .expect("a PNG");
    std::fs::write(&out, png.as_bytes()).expect("written");
    println!("{out}");
}
