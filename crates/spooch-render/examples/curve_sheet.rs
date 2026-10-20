//! Curved labels, bent the way a person would: dragging a label's bend
//! handle down and up, and bending one over a circle until it wraps.
//!
//!     cargo run --release -p spooch-render --example curve_sheet -- curve-sheet.png

use skia_safe::{
    AlphaType, Color4f, ColorType, Data, EncodedImageFormat, ImageInfo, Paint, Point, Rect, images,
    surfaces,
};
use spooch_render::{Document, Label, Tool};

const W: i32 = 1400;
const H: i32 = 560;

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "curve-sheet.png".into());
    let mut d = Document::from_bgra(W as u32, H as u32, W as usize * 4, &backdrop(), 1.0)
        .expect("a document");
    let face = Label::typeface().expect("the label font");

    // A smile, a rainbow and a full circle, each by its handle.
    for (at, text, pull) in [
        ((80.0, 120.0), "SMILE ALONG", 60.0),
        ((80.0, 330.0), "OVER THE TOP", -70.0),
        ((520.0, 90.0), "ROUND AND ROUND WE GO ", 400.0),
    ] {
        label(&mut d, at, text);
        bend(&mut d, &face, at, text, pull);
    }

    // A circle, a label above it bent until it wraps, and one below.
    d.set_tool(Tool::Ellipse);
    drag(&mut d, (980.0, 180.0), (1220.0, 420.0));
    for (at, text, pull) in [
        ((1010.0, 150.0), "TEXT ON A CIRCLE", -60.0),
        ((1020.0, 470.0), "AND UNDER IT", 40.0),
    ] {
        label(&mut d, at, text);
        bend(&mut d, &face, at, text, pull);
    }
    d.set_tool(Tool::Pen);
    d.set_tool(Tool::Arrow);

    let e = d.export_image();
    let info = ImageInfo::new(
        (e.width as i32, e.height as i32),
        ColorType::BGRA8888,
        AlphaType::Unpremul,
        None,
    );
    let img = images::raster_from_data(&info, Data::new_copy(&e.bgra), e.width as usize * 4)
        .expect("the export");
    let png = img
        .encode(None, EncodedImageFormat::PNG, 100)
        .expect("a PNG");
    std::fs::write(&out, png.as_bytes()).unwrap();
    println!("wrote {out}");
}

fn label(d: &mut Document, at: (f32, f32), text: &str) {
    d.set_tool(Tool::Text);
    d.begin_text(Point::new(at.0, at.1));
    d.type_text(text);
    d.commit_text();
}

/// Select the label typed at `at` and drag its bend handle down by `pull`.
fn bend(d: &mut Document, face: &skia_safe::Typeface, at: (f32, f32), text: &str, pull: f32) {
    // The label as `begin_text` makes it at the standard size.
    let size = 28.0;
    let l = Label {
        origin: Point::new(at.0, at.1 + size * 0.35),
        text: text.into(),
        color: Color4f::new(1.0, 0.0, 0.0, 1.0),
        size,
        look: Default::default(),
        angle: 0.0,
        font: Default::default(),
        bend: 0.0,
    };
    d.set_tool(Tool::Arrow);
    let middle = l.middle(face);
    let mid_text = Point::new(middle.x, middle.y - l.size * 0.3);
    d.begin_drag(mid_text);
    d.end_drag();
    let handle = middle + Point::new(0.0, l.size * 0.55);
    d.begin_drag(handle);
    for k in 1..=10 {
        d.drag_to(handle + Point::new(0.0, pull * k as f32 / 10.0));
    }
    d.end_drag();
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

/// A dark page with a light panel, so both edges show.
fn backdrop() -> Vec<u8> {
    let mut s = surfaces::raster_n32_premul((W, H)).unwrap();
    let c = s.canvas();
    c.clear(Color4f::new(0.13, 0.14, 0.17, 1.0));
    let mut p = Paint::default();
    p.set_color4f(Color4f::new(0.93, 0.94, 0.95, 1.0), None);
    c.draw_rect(Rect::from_xywh(900.0, 0.0, 500.0, H as f32), &p);
    let info = ImageInfo::new((W, H), ColorType::BGRA8888, AlphaType::Premul, None);
    let mut px = vec![0u8; (W * H * 4) as usize];
    assert!(s.read_pixels(&info, &mut px, W as usize * 4, (0, 0)));
    px
}
