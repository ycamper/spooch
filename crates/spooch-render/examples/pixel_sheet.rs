//! Every kind of mark in clean mode, then the same document in pixel mode,
//! with arrows in a fan of directions, for reviewing pixel art.
//!
//!     cargo run --release -p spooch-render --example pixel_sheet -- pixel-sheet.png

use skia_safe::{
    AlphaType, ColorType, Data, EncodedImageFormat, IRect, ImageInfo, Point, images, surfaces,
};
use spooch_render::{Document, Mode, Tool};

const W: i32 = 900;
const H: i32 = 420;

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "pixel-sheet.png".into());
    let backdrop: Vec<u8> = (0..W * H)
        .flat_map(|i| {
            let (x, y) = (i % W, i / W);
            let v = if (x / 40 + y / 40) % 2 == 0 { 236 } else { 200 };
            [v, v, v, 255]
        })
        .collect();
    let mut d = Document::from_bgra(W as u32, H as u32, W as usize * 4, &backdrop, 1.0)
        .expect("a document");
    // Room round the canvas, so the chrome isn't in the way of the drags,
    // and marks placed exactly.
    d.set_screens(vec![IRect::from_ltrb(-400, -400, W + 400, H + 400)]);
    d.set_modifiers(false, true);

    // A fan of arrows from one tip, every 15 degrees.
    d.set_tool(Tool::Arrow);
    let tip = Point::new(200.0, 210.0);
    for k in 0..24 {
        let a = (k as f32 * 15.0).to_radians();
        let tail = tip + Point::new(a.cos(), a.sin()) * 170.0;
        drag(&mut d, tail, tip + Point::new(a.cos(), a.sin()) * 30.0);
    }
    d.set_tool(Tool::Box);
    drag(&mut d, (430.0, 40.0).into(), (600.0, 150.0).into());
    d.set_tool(Tool::Ellipse);
    drag(&mut d, (640.0, 40.0).into(), (860.0, 150.0).into());
    d.set_tool(Tool::Line);
    drag(&mut d, (430.0, 200.0).into(), (860.0, 260.0).into());
    d.set_tool(Tool::Pen);
    d.begin_drag(Point::new(430.0, 330.0));
    for i in 0..60 {
        let x = 430.0 + i as f32 * 5.0;
        d.drag_to(Point::new(x, 330.0 + (i as f32 * 0.3).sin() * 30.0));
    }
    d.end_drag();
    d.set_tool(Tool::Steps);
    drag(&mut d, (800.0, 330.0).into(), (800.0, 330.0).into());
    d.begin_text(Point::new(440.0, 390.0));
    d.type_text("Pixel text");
    d.commit_text();

    let clean = d.export_image();
    d.set_mode(Mode::Pixel);
    let pixel = d.export_image();

    // The two exports stacked, clean on top.
    let mut sheet = surfaces::raster_n32_premul((W, H * 2)).expect("a surface");
    for (row, e) in [clean, pixel].iter().enumerate() {
        let info = ImageInfo::new(
            (e.width as i32, e.height as i32),
            ColorType::BGRA8888,
            AlphaType::Unpremul,
            None,
        );
        let img = images::raster_from_data(&info, Data::new_copy(&e.bgra), e.width as usize * 4)
            .expect("an export");
        sheet
            .canvas()
            .draw_image(&img, (0.0, (row as i32 * H) as f32), None);
    }
    let png = sheet
        .image_snapshot()
        .encode(None, EncodedImageFormat::PNG, 100)
        .expect("a PNG");
    std::fs::write(&out, png.as_bytes()).unwrap();
    println!("wrote {out}");
}

fn drag(d: &mut Document, from: Point, to: Point) {
    d.begin_drag(from);
    d.drag_to(to);
    d.end_drag();
}
