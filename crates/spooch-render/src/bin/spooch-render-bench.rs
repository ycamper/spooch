//! Times Skia drawing marks over a full-resolution screenshot.
//!
//! Usage: `spooch-render-bench [screenshot.png] [--out bench.png]`
//!
//! Without a screenshot it draws a synthetic 2560x1440 desktop. It times a
//! full editor frame (screenshot + 10 arrows + one blurred region), plus
//! single arrows, blur at several radii and region sizes, and pixelate. It
//! then writes a PNG for checking the arrow by eye and reports its own size.

use std::time::{Duration, Instant};

use spooch_render::skia_safe::{
    Color4f, Data, EncodedImageFormat, Font, FontMgr, FontStyle, Image, Paint, Point, Rect,
    Surface, surfaces,
};
use spooch_render::{Arrow, blur_region, pixelate_region};

const W: i32 = 2560;
const H: i32 = 1440;
/// Target for one interactive frame at 60 Hz.
const FRAME_BUDGET_MS: f64 = 16.7;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = args
        .iter()
        .position(|a| a == "--out")
        .and_then(|i| args.get(i + 1).cloned())
        .unwrap_or_else(|| "bench.png".into());
    let input = args
        .iter()
        .enumerate()
        .find(|(i, a)| !a.starts_with("--") && (*i == 0 || args[i - 1] != "--out"))
        .map(|(_, a)| a.clone());

    let screenshot = match &input {
        Some(path) => {
            let bytes = std::fs::read(path).expect("read screenshot");
            Image::from_encoded(Data::new_copy(&bytes)).expect("decode screenshot")
        }
        None => synthetic_desktop(),
    };
    let (w, h) = (screenshot.width(), screenshot.height());
    println!(
        "Spooch render bench: Skia {}x{} {}",
        w,
        h,
        input.as_deref().unwrap_or("(synthetic desktop)")
    );

    let mut surface = surfaces::raster_n32_premul((w, h)).expect("surface");
    let arrows = sample_arrows(w as f32, h as f32);
    // Over window text, so the blur has something to blur.
    let region = Rect::from_xywh(w as f32 * 0.52, h as f32 * 0.2, 800.0, 450.0);

    // Warm up caches and lazily built Skia state before timing anything.
    for _ in 0..3 {
        editor_frame(&mut surface, &screenshot, &arrows, region);
    }

    let frame = time(60, || {
        editor_frame(&mut surface, &screenshot, &arrows, region);
    });
    report(
        "editor frame (screenshot + 10 arrows + 800x450 blur)",
        &frame,
        Some(FRAME_BUDGET_MS),
    );

    // What the editor will actually do while you drag an arrow: the blur
    // only changes when its region changes, so it is rendered once and
    // reused as an image.
    let cached_blur = {
        editor_frame(&mut surface, &screenshot, &[], region);
        surface
            .image_snapshot_with_bounds(region.round())
            .expect("blur snapshot")
    };
    let cached = time(60, || {
        let canvas = surface.canvas();
        canvas.draw_image(&screenshot, (0, 0), None);
        canvas.draw_image(&cached_blur, (region.left, region.top), None);
        for a in &arrows {
            a.draw(canvas);
        }
    });
    report(
        "editor frame, blur cached (screenshot + 10 arrows)",
        &cached,
        Some(FRAME_BUDGET_MS),
    );

    let base = time(60, || {
        surface.canvas().draw_image(&screenshot, (0, 0), None);
    });
    report("  screenshot blit alone", &base, None);

    let one = time(200, || arrows[0].draw(surface.canvas()));
    report("  one arrow (width 10)", &one, None);

    for sigma in [4.0, 12.0, 32.0] {
        let t = time(30, || {
            blur_region(surface.canvas(), &screenshot, region, sigma)
        });
        report(&format!("  blur 800x450, sigma {sigma}"), &t, None);
    }
    let full = Rect::from_wh(w as f32, h as f32);
    let t = time(10, || {
        blur_region(surface.canvas(), &screenshot, full, 12.0)
    });
    report(&format!("  blur full {w}x{h}, sigma 12"), &t, None);
    let t = time(30, || {
        pixelate_region(surface.canvas(), &screenshot, region, 12.0)
    });
    report("  pixelate 800x450, 12 px blocks", &t, None);

    // The image to check by eye: every arrow size, a blur and a pixelate.
    editor_frame(&mut surface, &screenshot, &arrows, region);
    let redact = Rect::from_xywh(w as f32 * 0.08, h as f32 * 0.72, 600.0, 160.0);
    pixelate_region(surface.canvas(), &screenshot, redact, 12.0);
    let png = surface
        .image_snapshot()
        .encode(None, EncodedImageFormat::PNG, None)
        .expect("encode PNG");
    std::fs::write(&out, png.as_bytes()).expect("write PNG");
    println!("wrote {out}");

    if let Ok(meta) = std::env::current_exe().and_then(std::fs::metadata) {
        println!(
            "binary size: {:.1} MB (this benchmark, Skia linked statically)",
            meta.len() as f64 / (1024.0 * 1024.0)
        );
    }
}

fn editor_frame(surface: &mut Surface, screenshot: &Image, arrows: &[Arrow], region: Rect) {
    let canvas = surface.canvas();
    canvas.draw_image(screenshot, (0, 0), None);
    blur_region(canvas, screenshot, region, 12.0);
    for a in arrows {
        a.draw(canvas);
    }
}

/// Ten arrows in the sizes and directions people actually draw.
fn sample_arrows(w: f32, h: f32) -> Vec<Arrow> {
    let red = Color4f::new(0.93, 0.16, 0.16, 1.0);
    let blue = Color4f::new(0.13, 0.45, 0.95, 1.0);
    let yellow = Color4f::new(1.0, 0.82, 0.1, 1.0);
    let specs: [(f32, f32, f32, f32, f32, Color4f); 10] = [
        (0.10, 0.10, 0.30, 0.25, 10.0, red),
        (0.45, 0.12, 0.35, 0.30, 6.0, red),
        (0.60, 0.40, 0.80, 0.20, 14.0, blue),
        (0.90, 0.50, 0.70, 0.45, 10.0, red),
        (0.20, 0.60, 0.25, 0.45, 8.0, yellow),
        (0.40, 0.50, 0.42, 0.52, 10.0, red),
        (0.05, 0.40, 0.18, 0.40, 4.0, blue),
        (0.70, 0.90, 0.60, 0.70, 18.0, red),
        (0.30, 0.95, 0.45, 0.80, 10.0, blue),
        (0.95, 0.05, 0.85, 0.15, 10.0, yellow),
    ];
    specs
        .iter()
        .map(|&(x0, y0, x1, y1, width, color)| Arrow {
            from: Point::new(x0 * w, y0 * h),
            to: Point::new(x1 * w, y1 * h),
            color,
            width,
            look: spooch_render::Look::default(),
            bend: 0.0,
        })
        .collect()
}

/// A stand-in screenshot with gradients, windows and text, so blur and
/// arrows have realistic content to work against.
fn synthetic_desktop() -> Image {
    let mut surface = surfaces::raster_n32_premul((W, H)).expect("surface");
    let canvas = surface.canvas();
    // A banded diagonal-ish gradient: enough tonal variation to make blur
    // visibly blur, without depending on gradient shader APIs.
    for band in 0..64 {
        let t = band as f32 / 63.0;
        let color = Color4f::new(0.15 + 0.4 * t, 0.25 + 0.05 * t, 0.45 + 0.05 * t, 1.0);
        let x = W as f32 * band as f32 / 64.0;
        canvas.draw_rect(
            Rect::from_xywh(x, 0.0, W as f32 / 64.0 + 1.0, H as f32),
            &Paint::new(color, None),
        );
    }

    let typeface = FontMgr::default().legacy_make_typeface(None, FontStyle::normal());
    let font = typeface.map(|t| Font::from_typeface(t, 15.0));
    let window = Paint::new(Color4f::new(0.97, 0.97, 0.97, 1.0), None);
    let title = Paint::new(Color4f::new(0.85, 0.86, 0.88, 1.0), None);
    let ink = Paint::new(Color4f::new(0.12, 0.12, 0.14, 1.0), None);
    for (i, (x, y, ww, wh)) in [
        (80.0, 70.0, 1100.0, 700.0),
        (1250.0, 120.0, 1200.0, 600.0),
        (300.0, 820.0, 1500.0, 560.0),
    ]
    .into_iter()
    .enumerate()
    {
        canvas.draw_rect(Rect::from_xywh(x, y, ww, wh), &window);
        canvas.draw_rect(Rect::from_xywh(x, y, ww, 32.0), &title);
        let mut line_y = y + 60.0;
        let mut n = 0;
        while line_y < y + wh - 20.0 {
            match &font {
                Some(font) => {
                    let text = format!(
                        "Window {i} line {n}: the quick brown fox jumps over the lazy dog 0123456789"
                    );
                    canvas.draw_str(text, (x + 20.0, line_y), font, &ink);
                }
                // No system fonts (headless Linux): draw text-like bars.
                None => {
                    let len = 300.0 + ((n * 97) % 500) as f32;
                    canvas.draw_rect(Rect::from_xywh(x + 20.0, line_y - 11.0, len, 11.0), &ink);
                }
            }
            line_y += 22.0;
            n += 1;
        }
    }
    surface.image_snapshot()
}

fn time(runs: usize, mut f: impl FnMut()) -> Vec<Duration> {
    (0..runs)
        .map(|_| {
            let t = Instant::now();
            f();
            t.elapsed()
        })
        .collect()
}

fn report(label: &str, samples: &[Duration], budget_ms: Option<f64>) {
    let mut ms: Vec<f64> = samples.iter().map(|d| d.as_secs_f64() * 1000.0).collect();
    ms.sort_by(|a, b| a.total_cmp(b));
    let median = ms[ms.len() / 2];
    let p95 = ms[((ms.len() as f64 * 0.95) as usize).min(ms.len() - 1)];
    let verdict = match budget_ms {
        Some(b) if p95 <= b => format!(" (budget {b} ms) PASS"),
        Some(b) => format!(" (budget {b} ms) SLOW"),
        None => String::new(),
    };
    println!("{label}: median {median:.2} ms, p95 {p95:.2} ms{verdict}");
}
