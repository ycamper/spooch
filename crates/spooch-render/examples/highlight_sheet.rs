//! Every highlight look on the same made-up page, light and dark, each
//! drawn through the editor the way a person would.
//!
//!     cargo run --release -p spooch-render --example highlight_sheet -- highlight-sheet.png

use skia_safe::{
    AlphaType, Canvas, Color4f, ColorType, Data, EncodedImageFormat, Font, FontMgr, FontStyle,
    ImageInfo, Paint, Point, Rect, Typeface, images, surfaces,
};
use spooch_render::{Document, HighlightLook, Tool};

const PAGE: (i32, i32) = (620, 300);

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "highlight-sheet.png".into());
    let (cols, gap) = (4, 24.0);
    let (pw, ph) = (PAGE.0 as f32, PAGE.1 as f32);
    let w = (pw + gap) * cols as f32 + gap;
    let h = (ph + 50.0 + gap) * 2.0 + gap;
    let mut s = surfaces::raster_n32_premul((w as i32, h as i32)).unwrap();
    let c = s.canvas();
    c.clear(Color4f::new(0.11, 0.12, 0.14, 1.0));
    let title = Font::from_typeface(face(true), 20.0);
    for (i, (dark, look)) in [false, true]
        .into_iter()
        .flat_map(|dark| HighlightLook::ALL.map(|look| (dark, look)))
        .enumerate()
    {
        let page = page_pixels(dark);
        let mut d = Document::from_bgra(
            PAGE.0 as u32,
            PAGE.1 as u32,
            PAGE.0 as usize * 4,
            &page,
            1.0,
        )
        .expect("a document");
        d.set_tool(Tool::Highlight);
        for _ in 0..i % HighlightLook::ALL.len() {
            d.cycle_look();
        }
        // From the right, clear of the canvas edge's crop handle.
        drag(&mut d, (366.0, 162.0), (36.0, 128.0));
        d.set_tool(Tool::Arrow);
        let e = d.export_image();
        let info = ImageInfo::new(
            (e.width as i32, e.height as i32),
            ColorType::BGRA8888,
            AlphaType::Unpremul,
            None,
        );
        let img =
            images::raster_from_data(&info, Data::new_copy(&e.bgra), e.width as usize * 4).unwrap();
        let x = gap + (i % cols) as f32 * (pw + gap);
        let y = gap + (i / cols) as f32 * (ph + 50.0 + gap);
        let ink = Paint::new(Color4f::new(0.9, 0.9, 0.92, 1.0), None);
        c.draw_str(look.name(), (x, y + 22.0), &title, &ink);
        c.draw_image(&img, (x, y + 40.0), None);
    }
    let png = s
        .image_snapshot()
        .encode(None, EncodedImageFormat::PNG, 100)
        .unwrap();
    std::fs::write(&out, png.as_bytes()).unwrap();
    println!("wrote {out}");
}

fn drag(d: &mut Document, from: (f32, f32), to: (f32, f32)) {
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

fn face(bold: bool) -> Typeface {
    let style = if bold {
        FontStyle::bold()
    } else {
        FontStyle::normal()
    };
    FontMgr::new()
        .match_family_style("Segoe UI", style)
        .or_else(|| FontMgr::new().legacy_make_typeface(None, style))
        .unwrap()
}

/// A plain settings page: a title, some text and rules.
fn page_pixels(dark: bool) -> Vec<u8> {
    let mut s = surfaces::raster_n32_premul(PAGE).unwrap();
    draw_page(s.canvas(), dark);
    let info = ImageInfo::new(PAGE, ColorType::BGRA8888, AlphaType::Premul, None);
    let mut px = vec![0u8; (PAGE.0 * PAGE.1 * 4) as usize];
    assert!(s.read_pixels(&info, &mut px, PAGE.0 as usize * 4, (0, 0)));
    px
}

/// The page, in a light or a dark theme.
fn draw_page(c: &Canvas, dark: bool) {
    let paint = |r: f32, g: f32, b: f32| {
        let mut p = Paint::new(Color4f::new(r, g, b, 1.0), None);
        p.set_anti_alias(true);
        p
    };
    let (w, _) = (PAGE.0 as f32, PAGE.1 as f32);
    let flip = |v: f32| if dark { 1.0 - v * 0.9 } else { v };
    let paint = |r: f32, g: f32, b: f32| paint(flip(r), flip(g), flip(b));
    c.clear(Color4f::new(flip(0.98), flip(0.98), flip(0.99), 1.0));
    c.draw_rect(Rect::from_wh(w, 48.0), &paint(0.16, 0.30, 0.55));
    let bold = Font::from_typeface(face(true), 20.0);
    let body = Font::from_typeface(face(false), 17.0);
    c.draw_str("Billing settings", (24, 31), &bold, &paint(1.0, 1.0, 1.0));
    let (ink, grey) = (paint(0.18, 0.2, 0.24), paint(0.45, 0.47, 0.52));
    c.draw_str(
        "Your plan renews on the first of each month.",
        (40, 92),
        &body,
        &grey,
    );
    c.draw_str(
        "Spending limit reached: jobs paused",
        (40, 152),
        &body,
        &ink,
    );
    c.draw_str("Payment method   Visa ending 4242", (40, 206), &body, &grey);
    c.draw_str(
        "Invoices are emailed to the account owner.",
        (40, 246),
        &body,
        &grey,
    );
    for y in [176.0, 222.0, 262.0] {
        c.draw_rect(
            Rect::from_xywh(24.0, y, w - 48.0, 1.0),
            &paint(0.88, 0.89, 0.91),
        );
    }
}
