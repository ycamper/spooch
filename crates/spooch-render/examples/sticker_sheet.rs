//! The sticker tool as it looks: a few stamped on a capture, the row of
//! recent stickers, and the grid of all of them opened from it.
//!
//!     cargo run --release -p spooch-render --example sticker_sheet -- sticker-sheet.png

use skia_safe::{
    AlphaType, Color4f, ColorType, Data, EncodedImageFormat, IRect, ImageInfo, Paint, Point, Rect,
    images, surfaces,
};
use spooch_render::{Document, Tool};

const DESK: (i32, i32) = (1200, 760);

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "sticker-sheet.png".into());
    let desk = desk_pixels();
    let area = IRect::from_xywh(120, 120, 900, 520);
    let mut part = Vec::new();
    for y in area.top..area.bottom {
        let row = (y * DESK.0 + area.left) as usize * 4;
        part.extend_from_slice(&desk[row..row + area.width() as usize * 4]);
    }
    let mut d = Document::from_bgra(
        area.width() as u32,
        area.height() as u32,
        area.width() as usize * 4,
        &part,
        1.0,
    )
    .unwrap();
    d.place(area.left, area.top, IRect::from_wh(DESK.0, DESK.1));
    d.set_screens(vec![IRect::from_wh(DESK.0, DESK.1)]);

    d.set_tool(Tool::Sticker);
    let click = |d: &mut Document, x: f32, y: f32| {
        d.begin_drag(Point::new(x, y));
        d.end_drag();
    };
    let pick = |d: &mut Document, name: &str| {
        let i = spooch_render::STICKERS
            .iter()
            .position(|(n, _)| *n == name)
            .unwrap();
        d.pick_sticker(i);
    };
    click(&mut d, 240.0, 260.0);
    pick(&mut d, "rocket");
    d.begin_drag(Point::new(330.0, 200.0));
    d.drag_to(Point::new(430.0, 300.0));
    d.end_drag();
    // Stamps lean and wear; pointers are drawn, so they stay sharp.
    for (name, x, y) in [
        ("stamp_approved", 600.0, 250.0),
        ("stamp_ship_it", 860.0, 300.0),
        ("stamp_bug", 330.0, 590.0),
        ("pointer_click", 520.0, 470.0),
        ("pointer_hand", 800.0, 470.0),
        ("pointer_text", 950.0, 560.0),
        ("pointer_arrow", 640.0, 590.0),
    ] {
        pick(&mut d, name);
        click(&mut d, x, y);
    }

    // Open the grid, on the Stamps tab since a stamp is picked: its button
    // is after the ten recent stickers.
    pick(&mut d, "stamp_ship_it");
    let row = d.chrome().to_vec()[1].rect;
    click(
        &mut d,
        row.left as f32 + 8.0 + 10.0 * 26.0 + 9.0 * 3.0 + 13.0 + 11.0,
        (row.top + row.bottom) as f32 / 2.0,
    );

    let mut s = surfaces::raster_n32_premul(DESK).unwrap();
    let c = s.canvas();
    c.draw_image(
        image((DESK.0, DESK.1), &desk, AlphaType::Premul),
        (0, 0),
        None,
    );
    c.draw_rect(
        Rect::from_iwh(DESK.0, DESK.1),
        &Paint::new(Color4f::new(0.0, 0.0, 0.0, 0.35), None),
    );
    let canvas_rect = d.canvas();
    let (px, stride) = d.render();
    let mut tight = Vec::new();
    for row in px.chunks(stride).take(canvas_rect.height() as usize) {
        tight.extend_from_slice(&row[..canvas_rect.width() as usize * 4]);
    }
    c.draw_image(
        image(
            (canvas_rect.width(), canvas_rect.height()),
            &tight,
            AlphaType::Premul,
        ),
        (canvas_rect.left, canvas_rect.top),
        None,
    );
    for p in d.chrome().to_vec() {
        c.draw_image(
            image(
                (p.rect.width(), p.rect.height()),
                &p.pixels,
                AlphaType::Premul,
            ),
            (p.rect.left, p.rect.top),
            None,
        );
    }
    let png = s
        .image_snapshot()
        .encode(None, EncodedImageFormat::PNG, 100)
        .unwrap();
    std::fs::write(&out, png.as_bytes()).unwrap();
    println!("wrote {out}");
}

fn image(size: (i32, i32), bgra: &[u8], alpha: AlphaType) -> skia_safe::Image {
    let info = ImageInfo::new(size, ColorType::BGRA8888, alpha, None);
    images::raster_from_data(&info, Data::new_copy(bgra), size.0 as usize * 4).unwrap()
}

/// A plain page with a few blocks, as a desktop.
fn desk_pixels() -> Vec<u8> {
    let mut s = surfaces::raster_n32_premul(DESK).unwrap();
    let c = s.canvas();
    c.clear(Color4f::new(0.94, 0.95, 0.97, 1.0));
    let mut p = Paint::default();
    for (i, colour) in [(0.25, 0.45, 0.85), (0.95, 0.6, 0.2), (0.3, 0.7, 0.45)]
        .into_iter()
        .enumerate()
    {
        p.set_color4f(Color4f::new(colour.0, colour.1, colour.2, 1.0), None);
        c.draw_rect(
            Rect::from_xywh(180.0 + i as f32 * 280.0, 380.0, 220.0, 160.0),
            &p,
        );
    }
    let info = ImageInfo::new(DESK, ColorType::BGRA8888, AlphaType::Premul, None);
    let mut px = vec![0u8; (DESK.0 * DESK.1 * 4) as usize];
    assert!(s.read_pixels(&info, &mut px, DESK.0 as usize * 4, (0, 0)));
    px
}
